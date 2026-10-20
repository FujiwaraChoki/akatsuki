//! Google OAuth 2.0 for Gmail (§6.7, §9.7): authorization URLs with PKCE (S256), the code
//! exchange, refresh, revocation, and [`GoogleTokenSource`], the per-account [`TokenSource`].
//!
//! Two client types are supported (D-202):
//!
//! - **Web application** clients (the hosted worker, `GOOGLE_MAIL_CLIENT_ID/SECRET`): the secret
//!   is required and sent in the form body (`client_secret_post`), and the redirect URI is the
//!   worker's `/oauth/google/callback`.
//! - **Installed application** clients ("Desktop app" type, used by self-hosters and the desktop
//!   standalone mode): PKCE is mandatory, the redirect is a loopback URI, and the client secret
//!   is optional (Google issues one to desktop clients but does not treat it as confidential;
//!   it is sent when configured).
//!
//! Tokens never appear in `Debug` output, logs or errors.

use std::sync::Arc;

use async_trait::async_trait;
use base64::Engine as _;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use tokio::sync::Mutex;

use super::Clock;
use crate::{AccessToken, LimitScope, ProviderError, TokenSource, UnixMs};

/// Google's authorization endpoint.
pub const GOOGLE_AUTH_URL: &str = "https://accounts.google.com/o/oauth2/v2/auth";
/// Google's token endpoint.
pub const GOOGLE_TOKEN_URL: &str = "https://oauth2.googleapis.com/token";
/// Google's revocation endpoint.
pub const GOOGLE_REVOKE_URL: &str = "https://oauth2.googleapis.com/revoke";
/// Scopes for a mailbox connection (§6.7; `gmail.modify`, D-039).
pub const GMAIL_SCOPES: [&str; 3] = ["openid", "email", "https://www.googleapis.com/auth/gmail.modify"];
/// Refresh this long before expiry (§9.1).
pub const REFRESH_SKEW_MS: i64 = 5 * 60 * 1000;

/// The OAuth client type.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClientKind {
    /// Web application client (confidential).
    Web,
    /// Installed (desktop) application client.
    Installed,
}

/// An OAuth client. `Debug` hides the secret.
#[derive(Clone)]
pub struct OAuthClient {
    /// Client type.
    pub kind: ClientKind,
    /// Client id.
    pub client_id: String,
    client_secret: Option<String>,
    /// Authorization endpoint.
    pub auth_url: String,
    /// Token endpoint.
    pub token_url: String,
    /// Revocation endpoint.
    pub revoke_url: String,
}

impl std::fmt::Debug for OAuthClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OAuthClient")
            .field("kind", &self.kind)
            .field("client_id", &self.client_id)
            .field("has_secret", &self.client_secret.is_some())
            .finish_non_exhaustive()
    }
}

/// Parameters of an authorization URL.
#[derive(Clone, Debug)]
pub struct AuthorizeRequest<'a> {
    /// Registered redirect URI (worker callback, or a loopback URI for installed clients).
    pub redirect_uri: &'a str,
    /// Opaque state (the worker's sealed state, §6.7).
    pub state: &'a str,
    /// `pkce_challenge(verifier)`.
    pub code_challenge: &'a str,
    /// `login_hint` (the address being reconnected).
    pub login_hint: Option<&'a str>,
    /// Scopes; [`GMAIL_SCOPES`] when empty.
    pub scopes: &'a [&'a str],
}

/// A token endpoint response. `Debug` hides the tokens.
#[derive(Clone, Deserialize)]
pub struct TokenResponse {
    /// Access token.
    pub access_token: String,
    /// Lifetime in seconds.
    #[serde(default)]
    pub expires_in: Option<i64>,
    /// Refresh token (code exchange; rarely on refresh).
    #[serde(default)]
    pub refresh_token: Option<String>,
    /// Granted scopes, space-separated.
    #[serde(default)]
    pub scope: Option<String>,
    /// OpenID id token (code exchange with `openid`).
    #[serde(default)]
    pub id_token: Option<String>,
    /// `Bearer`.
    #[serde(default)]
    pub token_type: Option<String>,
}

impl std::fmt::Debug for TokenResponse {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TokenResponse")
            .field("expires_in", &self.expires_in)
            .field("has_refresh_token", &self.refresh_token.is_some())
            .field("scope", &self.scope)
            .finish_non_exhaustive()
    }
}

impl TokenResponse {
    /// `true` when `gmail.modify` was granted (users can untick scopes on the consent screen).
    pub fn has_gmail_scope(&self) -> bool {
        self.scope.as_deref().is_some_and(|s| s.split_whitespace().any(|x| x == GMAIL_SCOPES[2]))
    }

    /// The access token with its expiry, from `now`.
    pub fn access_token_at(&self, now: UnixMs) -> AccessToken {
        AccessToken {
            token: self.access_token.clone(),
            expires_at: now.saturating_add(self.expires_in.unwrap_or(3600).saturating_mul(1000)),
        }
    }
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct OAuthError {
    error: String,
}

/// A new PKCE verifier: 64 characters from two UUIDv4s (244 random bits), within RFC 7636's
/// 43–128 unreserved characters.
pub fn pkce_verifier() -> String {
    format!("{}{}", uuid::Uuid::new_v4().simple(), uuid::Uuid::new_v4().simple())
}

/// The S256 challenge for a verifier.
pub fn pkce_challenge(verifier: &str) -> String {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()))
}

fn form(pairs: &[(&str, &str)]) -> String {
    let mut s = url::form_urlencoded::Serializer::new(String::new());
    for (k, v) in pairs {
        s.append_pair(k, v);
    }
    s.finish()
}

pub(crate) fn transport_error(e: &reqwest::Error) -> ProviderError {
    // Never include the URL: it can carry tokens or ids. The kind is enough.
    let kind = if e.is_timeout() {
        "timeout"
    } else if e.is_connect() {
        "connect"
    } else if e.is_body() || e.is_decode() {
        "body"
    } else {
        "request"
    };
    ProviderError::Transient(format!("network: {kind}"), None)
}

impl OAuthClient {
    /// A web application client.
    pub fn web(client_id: impl Into<String>, client_secret: impl Into<String>) -> Self {
        Self::new(ClientKind::Web, client_id.into(), Some(client_secret.into()))
    }

    /// An installed (desktop) application client.
    pub fn installed(client_id: impl Into<String>, client_secret: Option<String>) -> Self {
        Self::new(ClientKind::Installed, client_id.into(), client_secret)
    }

    fn new(kind: ClientKind, client_id: String, client_secret: Option<String>) -> Self {
        Self {
            kind,
            client_id,
            client_secret: client_secret.filter(|s| !s.is_empty()),
            auth_url: GOOGLE_AUTH_URL.into(),
            token_url: GOOGLE_TOKEN_URL.into(),
            revoke_url: GOOGLE_REVOKE_URL.into(),
        }
    }

    /// Points the client at other endpoints (tests, emulators).
    pub fn with_endpoints(mut self, auth_url: &str, token_url: &str, revoke_url: &str) -> Self {
        auth_url.clone_into(&mut self.auth_url);
        token_url.clone_into(&mut self.token_url);
        revoke_url.clone_into(&mut self.revoke_url);
        self
    }

    /// The consent URL. Always asks for offline access and forces the consent screen, so Google
    /// returns a refresh token on reconnects too.
    pub fn authorize_url(&self, req: &AuthorizeRequest<'_>) -> Result<String, ProviderError> {
        let mut url = url::Url::parse(&self.auth_url).map_err(|_| ProviderError::Invalid("bad auth url".into()))?;
        let scopes = if req.scopes.is_empty() { GMAIL_SCOPES.join(" ") } else { req.scopes.join(" ") };
        {
            let mut q = url.query_pairs_mut();
            q.append_pair("client_id", &self.client_id)
                .append_pair("redirect_uri", req.redirect_uri)
                .append_pair("response_type", "code")
                .append_pair("scope", &scopes)
                .append_pair("access_type", "offline")
                .append_pair("prompt", "consent")
                .append_pair("include_granted_scopes", "true")
                .append_pair("state", req.state)
                .append_pair("code_challenge", req.code_challenge)
                .append_pair("code_challenge_method", "S256");
            if let Some(h) = req.login_hint {
                q.append_pair("login_hint", h);
            }
        }
        Ok(url.into())
    }

    async fn token_request(
        &self,
        http: &reqwest::Client,
        mut pairs: Vec<(&str, &str)>,
    ) -> Result<TokenResponse, ProviderError> {
        pairs.push(("client_id", &self.client_id));
        if let Some(s) = &self.client_secret {
            pairs.push(("client_secret", s));
        } else if self.kind == ClientKind::Web {
            return Err(ProviderError::Invalid("web OAuth client without a secret".into()));
        }
        let resp = http
            .post(&self.token_url)
            .header(reqwest::header::CONTENT_TYPE, "application/x-www-form-urlencoded")
            .body(form(&pairs))
            .send()
            .await
            .map_err(|e| transport_error(&e))?;
        let status = resp.status().as_u16();
        let retry_after = super::client::retry_after(resp.headers());
        let body = resp.bytes().await.map_err(|e| transport_error(&e))?;
        if (200..300).contains(&status) {
            return serde_json::from_slice(&body).map_err(|_| ProviderError::Transient("token response".into(), None));
        }
        let err: OAuthError = serde_json::from_slice(&body).unwrap_or_default();
        Err(match (status, err.error.as_str()) {
            (_, "invalid_grant") => ProviderError::Auth { reauth_required: true },
            (_, "invalid_client" | "unauthorized_client") => {
                ProviderError::Invalid(format!("OAuth client rejected: {}", err.error))
            }
            (429, _) => ProviderError::RateLimited { retry_after, scope: LimitScope::Project },
            (s, _) if s >= 500 => ProviderError::Transient(format!("token endpoint {s}"), retry_after),
            (s, e) => ProviderError::Invalid(format!("token endpoint {s} {e}")),
        })
    }

    /// Exchanges an authorization code (PKCE).
    pub async fn exchange_code(
        &self,
        http: &reqwest::Client,
        code: &str,
        redirect_uri: &str,
        code_verifier: &str,
    ) -> Result<TokenResponse, ProviderError> {
        self.token_request(
            http,
            vec![
                ("grant_type", "authorization_code"),
                ("code", code),
                ("redirect_uri", redirect_uri),
                ("code_verifier", code_verifier),
            ],
        )
        .await
    }

    /// Refreshes an access token. `invalid_grant` (revoked, expired, password change) is
    /// `Auth { reauth_required: true }`.
    pub async fn refresh(&self, http: &reqwest::Client, refresh_token: &str) -> Result<TokenResponse, ProviderError> {
        self.token_request(http, vec![("grant_type", "refresh_token"), ("refresh_token", refresh_token)]).await
    }

    /// Revokes a token (access or refresh; revoking either revokes the grant). An already
    /// revoked or unknown token counts as success.
    pub async fn revoke(&self, http: &reqwest::Client, token: &str) -> Result<(), ProviderError> {
        revoke_token(http, &self.revoke_url, token).await
    }
}

/// `POST {revoke_url}` with `token=…`. A 400 (`invalid_token`: already revoked) is success.
pub async fn revoke_token(http: &reqwest::Client, revoke_url: &str, token: &str) -> Result<(), ProviderError> {
    let resp = http
        .post(revoke_url)
        .header(reqwest::header::CONTENT_TYPE, "application/x-www-form-urlencoded")
        .body(form(&[("token", token)]))
        .send()
        .await
        .map_err(|e| transport_error(&e))?;
    match resp.status().as_u16() {
        200..=299 | 400 => Ok(()),
        s if s >= 500 => Err(ProviderError::Transient(format!("revoke {s}"), None)),
        s => Err(ProviderError::Invalid(format!("revoke {s}"))),
    }
}

/// Where a rotated refresh token goes (`worker.putSecret` with compare-and-set, §9.7). Google
/// rarely rotates, but the token endpoint may return a new refresh token and it must be kept.
#[async_trait]
pub trait RefreshTokenStore: Send + Sync {
    /// Persists the new refresh token. An error (CAS lost: another worker owns the account) is
    /// returned to the caller of `access_token`.
    async fn rotated(&self, refresh_token: &str) -> Result<(), ProviderError>;
}

struct Inner {
    refresh_token: String,
    cached: Option<AccessToken>,
}

/// The per-account token source: caches the access token, refreshes it single-flight at
/// `exp − 5 min`, persists rotated refresh tokens.
pub struct GoogleTokenSource {
    client: OAuthClient,
    http: reqwest::Client,
    inner: Mutex<Inner>,
    store: Option<Arc<dyn RefreshTokenStore>>,
    clock: Clock,
}

impl std::fmt::Debug for GoogleTokenSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GoogleTokenSource").field("client", &self.client).finish_non_exhaustive()
    }
}

impl GoogleTokenSource {
    /// A token source for one account's refresh token.
    pub fn new(
        client: OAuthClient,
        http: reqwest::Client,
        refresh_token: String,
        store: Option<Arc<dyn RefreshTokenStore>>,
        clock: Clock,
    ) -> Self {
        Self { client, http, inner: Mutex::new(Inner { refresh_token, cached: None }), store, clock }
    }

    /// Revokes the grant (account removal).
    pub async fn revoke(&self) -> Result<(), ProviderError> {
        let token = self.inner.lock().await.refresh_token.clone();
        self.client.revoke(&self.http, &token).await
    }
}

#[async_trait]
impl TokenSource for GoogleTokenSource {
    async fn access_token(&self) -> Result<AccessToken, ProviderError> {
        // Holding the lock across the refresh makes it single-flight: concurrent callers wait
        // here and then find the fresh token in the cache.
        let mut inner = self.inner.lock().await;
        let now = (self.clock)();
        if let Some(t) = &inner.cached
            && t.expires_at - REFRESH_SKEW_MS > now
        {
            return Ok(t.clone());
        }
        let resp = self.client.refresh(&self.http, &inner.refresh_token).await?;
        if let Some(rt) = resp.refresh_token.as_deref().filter(|rt| *rt != inner.refresh_token) {
            if let Some(store) = &self.store {
                store.rotated(rt).await?;
            }
            inner.refresh_token = rt.to_owned();
        }
        let token = resp.access_token_at((self.clock)());
        inner.cached = Some(token.clone());
        Ok(token)
    }

    async fn invalidate(&self) {
        self.inner.lock().await.cached = None;
    }
}

/// A fixed token (tests and tools). Never refreshes.
#[derive(Clone)]
pub struct StaticTokenSource(pub AccessToken);

#[async_trait]
impl TokenSource for StaticTokenSource {
    async fn access_token(&self) -> Result<AccessToken, ProviderError> {
        Ok(self.0.clone())
    }
    async fn invalidate(&self) {}
}
