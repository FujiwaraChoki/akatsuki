//! Microsoft identity platform v2 (`login.microsoftonline.com/{tenant}/oauth2/v2.0`) for mailbox
//! access: the authorize URL with PKCE (S256), the code exchange, and refresh-token rotation
//! behind [`GraphTokenSource`] (ARCHITECTURE §6.7, §9.7).
//!
//! The `common` tenant accepts personal Microsoft accounts and work or school accounts alike.
//! Microsoft rotates the refresh token on every refresh; the new one is handed to a
//! [`RefreshTokenStore`] (the worker's `worker.putSecret` compare-and-set) before the access token
//! is used. Nothing here logs or prints a token, and every type holding one redacts `Debug`.

use std::sync::Arc;

use async_trait::async_trait;
use serde::Deserialize;

use crate::{AccessToken, LimitScope, ProviderError, TokenSource, UnixMs};

use super::Clock;
use super::client::{retry_after, transport};

/// Mailbox scopes (§6.7 step 4).
pub const GRAPH_SCOPES: &str = "offline_access openid email User.Read Mail.ReadWrite Mail.Send";
/// The public login host.
pub const LOGIN_BASE: &str = "https://login.microsoftonline.com";
/// Personal and work/school accounts.
pub const COMMON_TENANT: &str = "common";
/// Refresh this long before expiry (§9.1 `TokenSource`).
pub const REFRESH_SKEW_MS: i64 = 5 * 60 * 1000;

/// The OAuth client used for mailbox access (`MICROSOFT_MAIL_CLIENT_ID/SECRET`).
#[derive(Clone)]
pub struct OAuthConfig {
    /// Application (client) id.
    pub client_id: String,
    /// Client secret; `None` for a public client (desktop standalone, tests).
    pub client_secret: Option<String>,
    /// Login host, `https://login.microsoftonline.com` unless testing.
    pub login_base: String,
    /// Tenant, `common` by default.
    pub tenant: String,
    /// Space-separated scopes.
    pub scopes: String,
}

impl OAuthConfig {
    /// The production configuration for a client id and secret.
    pub fn new(client_id: impl Into<String>, client_secret: Option<String>) -> Self {
        Self {
            client_id: client_id.into(),
            client_secret,
            login_base: LOGIN_BASE.to_owned(),
            tenant: COMMON_TENANT.to_owned(),
            scopes: GRAPH_SCOPES.to_owned(),
        }
    }
}

impl std::fmt::Debug for OAuthConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OAuthConfig")
            .field("client_id", &self.client_id)
            .field("client_secret", &self.client_secret.as_ref().map(|_| "<redacted>"))
            .field("login_base", &self.login_base)
            .field("tenant", &self.tenant)
            .finish()
    }
}

/// A token endpoint response.
#[derive(Clone, PartialEq, Eq)]
pub struct TokenGrant {
    /// Bearer token for Graph.
    pub access_token: String,
    /// Expiry instant.
    pub expires_at: UnixMs,
    /// The new refresh token (Microsoft rotates it on every refresh).
    pub refresh_token: Option<String>,
    /// Granted scopes.
    pub scope: Option<String>,
    /// OpenID id token (code exchange only).
    pub id_token: Option<String>,
}

impl std::fmt::Debug for TokenGrant {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TokenGrant")
            .field("expires_at", &self.expires_at)
            .field("scope", &self.scope)
            .finish_non_exhaustive()
    }
}

#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
    #[serde(default)]
    expires_in: Option<i64>,
    refresh_token: Option<String>,
    scope: Option<String>,
    id_token: Option<String>,
}

#[derive(Deserialize)]
struct TokenError {
    #[serde(default)]
    error: String,
}

/// The token endpoint client.
#[derive(Clone)]
pub struct MicrosoftOAuth {
    http: reqwest::Client,
    cfg: OAuthConfig,
    clock: Clock,
}

impl std::fmt::Debug for MicrosoftOAuth {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MicrosoftOAuth").field("cfg", &self.cfg).finish_non_exhaustive()
    }
}

fn form(pairs: &[(&str, &str)]) -> String {
    let mut s = url::form_urlencoded::Serializer::new(String::new());
    for (k, v) in pairs {
        s.append_pair(k, v);
    }
    s.finish()
}

impl MicrosoftOAuth {
    /// A client over `http`.
    pub fn new(http: reqwest::Client, cfg: OAuthConfig) -> Self {
        Self { http, cfg, clock: super::system_clock() }
    }

    /// Replaces the clock (tests).
    pub fn with_clock(mut self, clock: Clock) -> Self {
        self.clock = clock;
        self
    }

    fn endpoint(&self, which: &str) -> String {
        format!("{}/{}/oauth2/v2.0/{which}", self.cfg.login_base.trim_end_matches('/'), self.cfg.tenant)
    }

    /// The authorize URL for the connect flow (§6.7 step 1): authorization code with PKCE S256.
    /// `state` is the worker's sealed state; `prompt=select_account` lets a user with several
    /// Microsoft accounts pick the mailbox.
    pub fn authorize_url(
        &self,
        redirect_uri: &str,
        state: &str,
        code_challenge: &str,
        login_hint: Option<&str>,
    ) -> String {
        let mut pairs = vec![
            ("client_id", self.cfg.client_id.as_str()),
            ("response_type", "code"),
            ("redirect_uri", redirect_uri),
            ("response_mode", "query"),
            ("scope", self.cfg.scopes.as_str()),
            ("state", state),
            ("code_challenge", code_challenge),
            ("code_challenge_method", "S256"),
            ("prompt", "select_account"),
        ];
        if let Some(h) = login_hint {
            pairs.push(("login_hint", h));
        }
        format!("{}?{}", self.endpoint("authorize"), form(&pairs))
    }

    /// Exchanges an authorization code (§6.7 step 3.2).
    pub async fn exchange_code(
        &self,
        code: &str,
        redirect_uri: &str,
        code_verifier: &str,
    ) -> Result<TokenGrant, ProviderError> {
        let mut pairs = vec![
            ("client_id", self.cfg.client_id.as_str()),
            ("grant_type", "authorization_code"),
            ("code", code),
            ("redirect_uri", redirect_uri),
            ("code_verifier", code_verifier),
            ("scope", self.cfg.scopes.as_str()),
        ];
        if let Some(s) = &self.cfg.client_secret {
            pairs.push(("client_secret", s.as_str()));
        }
        self.token_request(&form(&pairs)).await
    }

    /// Redeems a refresh token.
    pub async fn refresh(&self, refresh_token: &str) -> Result<TokenGrant, ProviderError> {
        let mut pairs = vec![
            ("client_id", self.cfg.client_id.as_str()),
            ("grant_type", "refresh_token"),
            ("refresh_token", refresh_token),
            ("scope", self.cfg.scopes.as_str()),
        ];
        if let Some(s) = &self.cfg.client_secret {
            pairs.push(("client_secret", s.as_str()));
        }
        self.token_request(&form(&pairs)).await
    }

    async fn token_request(&self, body: &str) -> Result<TokenGrant, ProviderError> {
        let resp = self
            .http
            .post(self.endpoint("token"))
            .header(reqwest::header::CONTENT_TYPE, "application/x-www-form-urlencoded")
            .body(body.to_owned())
            .send()
            .await
            .map_err(|e| transport(&e))?;
        let status = resp.status().as_u16();
        let ra = retry_after(resp.headers());
        let bytes = resp.bytes().await.map_err(|e| transport(&e))?;
        if (200..300).contains(&status) {
            let t: TokenResponse = serde_json::from_slice(&bytes)
                .map_err(|_| ProviderError::Transient("microsoft token: malformed response".into(), None))?;
            let now = (self.clock)();
            return Ok(TokenGrant {
                access_token: t.access_token,
                expires_at: now + t.expires_in.unwrap_or(3600).max(60) * 1000,
                refresh_token: t.refresh_token,
                scope: t.scope,
                id_token: t.id_token,
            });
        }
        let code = serde_json::from_slice::<TokenError>(&bytes).map(|e| e.error).unwrap_or_default();
        Err(match (status, code.as_str()) {
            (429, _) => ProviderError::RateLimited { retry_after: ra, scope: LimitScope::Account },
            (_, "temporarily_unavailable") => ProviderError::Transient("microsoft token: unavailable".into(), ra),
            (500..=599, _) => ProviderError::Transient(format!("microsoft token {status}"), ra),
            (_, "invalid_grant" | "interaction_required" | "consent_required" | "login_required") => {
                ProviderError::Auth { reauth_required: true }
            }
            (_, "invalid_client" | "unauthorized_client") => {
                ProviderError::Invalid("microsoft token: invalid_client".into())
            }
            (401 | 403, _) => ProviderError::Auth { reauth_required: true },
            _ => ProviderError::Invalid(format!("microsoft token {status}")),
        })
    }
}

/// Persists a rotated refresh token (the worker re-seals it and calls `worker.putSecret` with the
/// expected version). An error stops the refresh: when the compare-and-set fails, another worker
/// owns the account and this actor must release it (§9.7).
#[async_trait]
pub trait RefreshTokenStore: Send + Sync {
    /// Stores the new refresh token and the granted scope.
    async fn persist(&self, refresh_token: &str, scope: Option<&str>) -> Result<(), ProviderError>;
}

struct TokenState {
    refresh_token: String,
    access: Option<AccessToken>,
}

/// [`TokenSource`] for one Microsoft mailbox. Refreshes are single-flight: concurrent callers wait
/// on the same lock and reuse the token the first one obtained.
pub struct GraphTokenSource {
    oauth: MicrosoftOAuth,
    store: Arc<dyn RefreshTokenStore>,
    state: tokio::sync::Mutex<TokenState>,
}

impl std::fmt::Debug for GraphTokenSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GraphTokenSource").field("oauth", &self.oauth).finish_non_exhaustive()
    }
}

impl GraphTokenSource {
    /// A source starting from a decrypted refresh token.
    pub fn new(oauth: MicrosoftOAuth, refresh_token: String, store: Arc<dyn RefreshTokenStore>) -> Self {
        Self { oauth, store, state: tokio::sync::Mutex::new(TokenState { refresh_token, access: None }) }
    }
}

#[async_trait]
impl TokenSource for GraphTokenSource {
    async fn access_token(&self) -> Result<AccessToken, ProviderError> {
        let mut st = self.state.lock().await;
        let now = (self.oauth.clock)();
        if let Some(t) = &st.access
            && t.expires_at - REFRESH_SKEW_MS > now
        {
            return Ok(t.clone());
        }
        let grant = self.oauth.refresh(&st.refresh_token).await?;
        if let Some(rt) = grant.refresh_token.as_deref().filter(|rt| *rt != st.refresh_token) {
            self.store.persist(rt, grant.scope.as_deref()).await?;
            st.refresh_token = rt.to_owned();
        }
        let token = AccessToken { token: grant.access_token, expires_at: grant.expires_at };
        st.access = Some(token.clone());
        Ok(token)
    }

    async fn invalidate(&self) {
        self.state.lock().await.access = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn authorize_url_has_pkce_and_scopes() {
        let o = MicrosoftOAuth::new(reqwest::Client::new(), OAuthConfig::new("cid", Some("s3cret".into())));
        let u = o.authorize_url("https://sync.akatsuki.vc/oauth/microsoft/callback", "st", "chal", Some("a@b.c"));
        assert!(u.starts_with("https://login.microsoftonline.com/common/oauth2/v2.0/authorize?client_id=cid&"));
        assert!(u.contains("code_challenge_method=S256"));
        assert!(u.contains("scope=offline_access+openid+email+User.Read+Mail.ReadWrite+Mail.Send"));
        assert!(u.contains("login_hint=a%40b.c"));
        assert!(!u.contains("s3cret"));
        assert!(!format!("{o:?}").contains("s3cret"));
    }
}
