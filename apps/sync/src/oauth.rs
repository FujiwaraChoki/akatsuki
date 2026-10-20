//! Mailbox OAuth (ARCHITECTURE §6.7, §9.7): sealed `state`, PKCE (S256), the authorization URL,
//! code exchange, refresh, profile reads, and the [`OAuthTokenSource`] the adapters use.
//!
//! These are the mailbox clients (`GOOGLE_MAIL_CLIENT_*`, `MICROSOFT_MAIL_CLIENT_*`), held only by
//! the worker; login clients live in Convex. Tokens, codes and verifiers never reach a log line or
//! an error message.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use akatsuki_convex_api::WorkerApi;
use akatsuki_core::convex::values::ConvexBytes;
use akatsuki_core::convex::worker::PutSecretArgs;
use akatsuki_core::convex::{ImapSettings, Provider, SecretKind, TlsMode};
use akatsuki_core::ids::AccountId;
use akatsuki_core::time::{MINUTE_MS, UnixMs, now_ms};
use akatsuki_core::wire::ErrorCode;
use akatsuki_providers::{AccessToken, ProviderError, TokenSource};
use async_trait::async_trait;
use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::crypto::{self, SecretKeyring};

/// `state` lifetime.
pub const STATE_TTL_MS: i64 = 10 * MINUTE_MS;
/// Refresh this long before expiry.
pub const REFRESH_SKEW_MS: i64 = 5 * MINUTE_MS;
const STATE_AAD: &[u8] = b"akatsuki:oauth-state:v1";

/// Gmail mailbox scopes (§6.7).
pub const GOOGLE_SCOPES: &str = "openid email https://www.googleapis.com/auth/gmail.modify";
/// Graph mailbox scopes (§6.7).
pub const MICROSOFT_SCOPES: &str = "offline_access openid email User.Read Mail.ReadWrite Mail.Send";

/// Which OAuth provider.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OAuthKind {
    /// Google (Gmail).
    Google,
    /// Microsoft (Graph).
    Microsoft,
}

impl OAuthKind {
    /// From the route segment.
    pub fn from_path(s: &str) -> Option<Self> {
        match s {
            "google" => Some(Self::Google),
            "microsoft" => Some(Self::Microsoft),
            _ => None,
        }
    }

    /// The route segment.
    pub fn as_path(self) -> &'static str {
        match self {
            Self::Google => "google",
            Self::Microsoft => "microsoft",
        }
    }

    /// The Convex provider.
    pub fn provider(self) -> Provider {
        match self {
            Self::Google => Provider::Gmail,
            Self::Microsoft => Provider::Graph,
        }
    }

    /// The OAuth kind of a Convex provider.
    pub fn of(provider: Provider) -> Option<Self> {
        match provider {
            Provider::Gmail => Some(Self::Google),
            Provider::Graph => Some(Self::Microsoft),
            Provider::Imap => None,
        }
    }
}

/// Provider endpoints (overridable for tests).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Endpoints {
    /// Google authorization endpoint.
    pub google_auth: String,
    /// Google token endpoint.
    pub google_token: String,
    /// Gmail API base (`…/gmail/v1`).
    pub gmail_api: String,
    /// Google revoke endpoint.
    pub google_revoke: String,
    /// Microsoft authorization endpoint.
    pub microsoft_auth: String,
    /// Microsoft token endpoint.
    pub microsoft_token: String,
    /// Graph base (`…/v1.0`).
    pub graph_api: String,
}

impl Default for Endpoints {
    fn default() -> Self {
        Self {
            google_auth: "https://accounts.google.com/o/oauth2/v2/auth".into(),
            google_token: "https://oauth2.googleapis.com/token".into(),
            gmail_api: "https://gmail.googleapis.com/gmail/v1".into(),
            google_revoke: "https://oauth2.googleapis.com/revoke".into(),
            microsoft_auth: "https://login.microsoftonline.com/common/oauth2/v2.0/authorize".into(),
            microsoft_token: "https://login.microsoftonline.com/common/oauth2/v2.0/token".into(),
            graph_api: "https://graph.microsoft.com/v1.0".into(),
        }
    }
}

/// One OAuth client.
#[derive(Clone)]
pub struct OAuthClient {
    /// Provider.
    pub kind: OAuthKind,
    /// Client id.
    pub client_id: String,
    client_secret: String,
    /// `{PUBLIC_BASE_URL}/oauth/{provider}/callback`.
    pub redirect_uri: String,
    /// Endpoints.
    pub endpoints: Endpoints,
}

impl std::fmt::Debug for OAuthClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OAuthClient")
            .field("kind", &self.kind)
            .field("client_id", &self.client_id)
            .finish_non_exhaustive()
    }
}

impl OAuthClient {
    /// A client.
    pub fn new(
        kind: OAuthKind,
        client_id: String,
        client_secret: String,
        public_base_url: &str,
        endpoints: Endpoints,
    ) -> Self {
        let redirect_uri = format!("{}/oauth/{}/callback", public_base_url.trim_end_matches('/'), kind.as_path());
        Self { kind, client_id, client_secret, redirect_uri, endpoints }
    }

    fn token_url(&self) -> &str {
        match self.kind {
            OAuthKind::Google => &self.endpoints.google_token,
            OAuthKind::Microsoft => &self.endpoints.microsoft_token,
        }
    }

    /// The authorization URL for a sealed `state` and PKCE challenge.
    pub fn authorize_url(&self, state: &str, challenge: &str, login_hint: Option<&str>) -> String {
        let (base, scopes) = match self.kind {
            OAuthKind::Google => (&self.endpoints.google_auth, GOOGLE_SCOPES),
            OAuthKind::Microsoft => (&self.endpoints.microsoft_auth, MICROSOFT_SCOPES),
        };
        let mut url = url::Url::parse(base).unwrap_or_else(|_| unreachable!("endpoint constants are valid URLs"));
        {
            let mut q = url.query_pairs_mut();
            q.append_pair("client_id", &self.client_id)
                .append_pair("redirect_uri", &self.redirect_uri)
                .append_pair("response_type", "code")
                .append_pair("scope", scopes)
                .append_pair("state", state)
                .append_pair("code_challenge", challenge)
                .append_pair("code_challenge_method", "S256");
            match self.kind {
                OAuthKind::Google => {
                    q.append_pair("access_type", "offline").append_pair("prompt", "consent");
                }
                OAuthKind::Microsoft => {
                    q.append_pair("response_mode", "query").append_pair("prompt", "select_account");
                }
            }
            if let Some(hint) = login_hint.filter(|h| !h.is_empty() && h.len() <= 320) {
                q.append_pair("login_hint", hint);
            }
        }
        url.into()
    }

    async fn token_request(&self, http: &reqwest::Client, form: &[(&str, &str)]) -> Result<TokenResponse, OAuthError> {
        let mut full: Vec<(&str, &str)> =
            vec![("client_id", self.client_id.as_str()), ("client_secret", self.client_secret.as_str())];
        full.extend_from_slice(form);
        if self.kind == OAuthKind::Microsoft {
            full.push(("scope", MICROSOFT_SCOPES));
        }
        let body = url::form_urlencoded::Serializer::new(String::new()).extend_pairs(full.iter()).finish();
        let resp = http
            .post(self.token_url())
            .header(reqwest::header::CONTENT_TYPE, "application/x-www-form-urlencoded")
            .body(body)
            .timeout(Duration::from_secs(15))
            .send()
            .await
            .map_err(|_| OAuthError::Unavailable)?;
        let status = resp.status();
        let body: serde_json::Value = resp.json().await.map_err(|_| OAuthError::Unavailable)?;
        if status.is_success() {
            return serde_json::from_value(body).map_err(|_| OAuthError::Unavailable);
        }
        let error = body.get("error").and_then(|e| e.as_str()).unwrap_or_default();
        if error == "invalid_grant" || status == reqwest::StatusCode::UNAUTHORIZED {
            return Err(OAuthError::InvalidGrant);
        }
        if status.is_server_error() || status == reqwest::StatusCode::TOO_MANY_REQUESTS {
            return Err(OAuthError::Unavailable);
        }
        Err(OAuthError::Rejected)
    }

    /// Exchanges an authorization code with its PKCE verifier.
    pub async fn exchange_code(
        &self,
        http: &reqwest::Client,
        code: &str,
        verifier: &str,
    ) -> Result<TokenResponse, OAuthError> {
        self.token_request(
            http,
            &[
                ("grant_type", "authorization_code"),
                ("code", code),
                ("code_verifier", verifier),
                ("redirect_uri", self.redirect_uri.as_str()),
            ],
        )
        .await
    }

    /// Refreshes an access token.
    pub async fn refresh(&self, http: &reqwest::Client, refresh_token: &str) -> Result<TokenResponse, OAuthError> {
        self.token_request(http, &[("grant_type", "refresh_token"), ("refresh_token", refresh_token)]).await
    }
}

/// OAuth failures. Never carries provider text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum OAuthError {
    /// The grant is revoked or expired.
    #[error("invalid grant")]
    InvalidGrant,
    /// The provider refused the request.
    #[error("rejected")]
    Rejected,
    /// Network or provider outage.
    #[error("unavailable")]
    Unavailable,
}

/// A token endpoint response.
#[derive(Clone, PartialEq, Eq, Deserialize)]
pub struct TokenResponse {
    /// Access token.
    pub access_token: String,
    /// Lifetime in seconds.
    #[serde(default)]
    pub expires_in: Option<i64>,
    /// Refresh token (always on first consent; Microsoft rotates it on every refresh).
    #[serde(default)]
    pub refresh_token: Option<String>,
    /// Granted scopes.
    #[serde(default)]
    pub scope: Option<String>,
}

impl std::fmt::Debug for TokenResponse {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TokenResponse").field("expires_in", &self.expires_in).finish_non_exhaustive()
    }
}

impl TokenResponse {
    /// As an [`AccessToken`] at `now`.
    pub fn access(&self, now: UnixMs) -> AccessToken {
        AccessToken {
            token: self.access_token.clone(),
            expires_at: now + self.expires_in.unwrap_or(3600).clamp(60, 86_400) * 1000,
        }
    }
}

/// What the sealed `state` carries (§6.7, DECISIONS D-395).
///
/// The PKCE verifier is not in it: it stays in the worker, keyed by `nonce`, so the state and the code
/// that travel together in the redirect URL are not enough to redeem the code, and a state is used once.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OAuthState {
    /// Caller.
    pub user_id: String,
    /// Provider.
    pub provider: OAuthKind,
    /// Single-use flow id: the key of the worker's pending flow (PKCE verifier, caller).
    pub nonce: String,
    /// Allow-listed return URL.
    pub return_to: String,
    /// Set for a reauth.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account_id: Option<AccountId>,
    /// Expiry (ms).
    pub exp: UnixMs,
}

impl std::fmt::Debug for OAuthState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OAuthState").field("provider", &self.provider).field("exp", &self.exp).finish_non_exhaustive()
    }
}

/// Why a `state` was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum StateError {
    /// Not sealed by us, or tampered.
    #[error("invalid state")]
    Invalid,
    /// Older than 10 minutes.
    #[error("expired state")]
    Expired,
    /// Sealed for the other provider.
    #[error("wrong provider")]
    WrongProvider,
}

/// `base64url(XChaCha20-Poly1305(STATE_KEY, state))`.
pub fn seal_state(key: &[u8; 32], state: &OAuthState) -> Result<String, crypto::CryptoError> {
    let json = serde_json::to_vec(state).map_err(|_| crypto::CryptoError::Seal)?;
    crypto::seal_token(key, STATE_AAD, &json)
}

/// Opens and validates a `state` for `provider` at `now`.
pub fn open_state(key: &[u8; 32], token: &str, provider: OAuthKind, now: UnixMs) -> Result<OAuthState, StateError> {
    if token.len() > 4096 {
        return Err(StateError::Invalid);
    }
    let json = crypto::open_token(key, STATE_AAD, token).map_err(|_| StateError::Invalid)?;
    let state: OAuthState = serde_json::from_slice(&json).map_err(|_| StateError::Invalid)?;
    if state.exp < now {
        return Err(StateError::Expired);
    }
    if state.provider != provider {
        return Err(StateError::WrongProvider);
    }
    Ok(state)
}

/// A PKCE pair: `(verifier, S256 challenge)`. The verifier is 43 base64url characters.
pub fn pkce_pair() -> Result<(String, String), crypto::CryptoError> {
    let verifier = crypto::random_token::<32>()?;
    Ok((verifier.clone(), pkce_challenge(&verifier)))
}

/// `base64url(sha256(verifier))`.
pub fn pkce_challenge(verifier: &str) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()))
}

/// The plaintext of an OAuth `accountSecrets` row (§14.1).
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OAuthSecret {
    /// Refresh token.
    pub refresh_token: String,
    /// Granted scopes.
    #[serde(default)]
    pub scope: String,
    /// When it was obtained (ms).
    pub obtained_at: UnixMs,
}

impl std::fmt::Debug for OAuthSecret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OAuthSecret").field("obtained_at", &self.obtained_at).finish_non_exhaustive()
    }
}

/// The plaintext of an IMAP `accountSecrets` row (§14.1).
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImapSecret {
    /// Login name.
    pub username: String,
    /// Password.
    pub password: String,
    /// The servers the password was entered for, sealed with it (D-414). The worker connects only
    /// when `mailAccounts.imap` still names exactly these, so a Convex write cannot send the
    /// password to another host. Rows sealed before D-414 have none and need a reconnect.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub endpoints: Option<ImapEndpoints>,
}

/// IMAP and SMTP endpoints bound to an IMAP password (D-414).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImapEndpoints {
    /// IMAP host (lowercased, no trailing dot).
    pub imap_host: String,
    /// IMAP port.
    pub imap_port: u16,
    /// IMAP TLS mode.
    pub imap_tls: TlsMode,
    /// SMTP host (lowercased, no trailing dot).
    pub smtp_host: String,
    /// SMTP port.
    pub smtp_port: u16,
    /// SMTP TLS mode.
    pub smtp_tls: TlsMode,
}

fn norm_host(h: &str) -> String {
    h.trim().trim_end_matches('.').to_ascii_lowercase()
}

impl ImapEndpoints {
    /// The endpoints of stored IMAP settings.
    pub fn of(s: &ImapSettings) -> Self {
        Self {
            imap_host: norm_host(&s.host),
            imap_port: s.port,
            imap_tls: s.tls,
            smtp_host: norm_host(&s.smtp_host),
            smtp_port: s.smtp_port,
            smtp_tls: s.smtp_tls,
        }
    }

    /// Whether `s` names exactly these endpoints.
    pub fn matches(&self, s: &ImapSettings) -> bool {
        *self == Self::of(s)
    }
}

impl ImapSecret {
    /// The sealed endpoints, when `settings` (from Convex) still names them; `None` for a secret
    /// sealed without endpoints or settings that differ (D-414).
    pub fn bound_endpoints(&self, settings: &ImapSettings) -> Option<&ImapEndpoints> {
        self.endpoints.as_ref().filter(|e| e.matches(settings))
    }
}

impl std::fmt::Debug for ImapSecret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ImapSecret").finish_non_exhaustive()
    }
}

/// A mailbox profile read right after the code exchange.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MailboxProfile {
    /// Normalised provider account id (Gmail: lowercased address; Graph: object id).
    pub provider_account_id: String,
    /// Primary address.
    pub email: String,
    /// Display name.
    pub display_name: Option<String>,
    /// Send-as identities (the primary first).
    pub identities: Vec<akatsuki_core::convex::worker::IdentityIn>,
}

/// Reads the Gmail profile and send-as aliases.
pub async fn google_profile(
    http: &reqwest::Client,
    ep: &Endpoints,
    access: &str,
) -> Result<MailboxProfile, OAuthError> {
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Profile {
        email_address: String,
    }
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct SendAs {
        send_as_email: String,
        #[serde(default)]
        display_name: Option<String>,
        #[serde(default)]
        reply_to_address: Option<String>,
        #[serde(default)]
        is_default: bool,
        #[serde(default)]
        verification_status: Option<String>,
    }
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct SendAsList {
        #[serde(default)]
        send_as: Vec<SendAs>,
    }
    let get = |path: &str| {
        http.get(format!("{}/users/me/{path}", ep.gmail_api)).bearer_auth(access).timeout(Duration::from_secs(15))
    };
    let p: Profile = get("profile")
        .send()
        .await
        .and_then(reqwest::Response::error_for_status)
        .map_err(|_| OAuthError::Unavailable)?
        .json()
        .await
        .map_err(|_| OAuthError::Unavailable)?;
    let email = p.email_address.trim().to_lowercase();
    // Aliases are best effort: a failure here still connects the primary address.
    let send_as: Vec<SendAs> = match get("settings/sendAs").send().await.and_then(reqwest::Response::error_for_status) {
        Ok(r) => r.json::<SendAsList>().await.map(|l| l.send_as).unwrap_or_default(),
        Err(_) => vec![],
    };
    let mut identities: Vec<akatsuki_core::convex::worker::IdentityIn> = send_as
        .into_iter()
        .filter(|s| s.verification_status.as_deref().is_none_or(|v| v == "accepted"))
        .map(|s| akatsuki_core::convex::worker::IdentityIn {
            email: s.send_as_email.trim().to_lowercase(),
            name: s.display_name.filter(|n| !n.is_empty()),
            reply_to: s.reply_to_address.filter(|r| !r.is_empty()),
            is_default: s.is_default,
        })
        .collect();
    if !identities.iter().any(|i| i.email == email) {
        identities.insert(
            0,
            akatsuki_core::convex::worker::IdentityIn {
                email: email.clone(),
                name: None,
                reply_to: None,
                is_default: identities.iter().all(|i| !i.is_default),
            },
        );
    }
    let display_name = identities.iter().find(|i| i.email == email).and_then(|i| i.name.clone());
    Ok(MailboxProfile { provider_account_id: email.clone(), email, display_name, identities })
}

/// Reads the Graph `/me` profile.
pub async fn graph_profile(http: &reqwest::Client, ep: &Endpoints, access: &str) -> Result<MailboxProfile, OAuthError> {
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Me {
        id: String,
        #[serde(default)]
        mail: Option<String>,
        #[serde(default)]
        user_principal_name: Option<String>,
        #[serde(default)]
        display_name: Option<String>,
    }
    let me: Me = http
        .get(format!("{}/me?$select=id,mail,userPrincipalName,displayName", ep.graph_api))
        .bearer_auth(access)
        .timeout(Duration::from_secs(15))
        .send()
        .await
        .and_then(reqwest::Response::error_for_status)
        .map_err(|_| OAuthError::Unavailable)?
        .json()
        .await
        .map_err(|_| OAuthError::Unavailable)?;
    let email = me.mail.or(me.user_principal_name).ok_or(OAuthError::Rejected)?.trim().to_lowercase();
    let display_name = me.display_name.filter(|n| !n.is_empty());
    Ok(MailboxProfile {
        provider_account_id: me.id,
        email: email.clone(),
        display_name: display_name.clone(),
        identities: vec![akatsuki_core::convex::worker::IdentityIn {
            email,
            name: display_name,
            reply_to: None,
            is_default: true,
        }],
    })
}

/// Where the token source writes rotated refresh tokens.
#[derive(Clone, Debug)]
pub struct SecretSlot {
    /// Account.
    pub account_id: AccountId,
    /// This worker.
    pub worker_id: String,
    /// The AAD of §14.1.
    pub aad: Vec<u8>,
}

struct TokenState {
    secret: OAuthSecret,
    version: u64,
    cached: Option<AccessToken>,
}

/// Access tokens for one account: single-flight refresh at `exp − 5 min`; Microsoft's rotated
/// refresh tokens are re-sealed and written with `worker.putSecret(expectedVersion)` (§9.7). A
/// failed compare-and-set means another worker owns the account: [`OAuthTokenSource::lost`] turns
/// true and the actor releases it.
pub struct OAuthTokenSource {
    client: Arc<OAuthClient>,
    http: reqwest::Client,
    api: Arc<dyn WorkerApi>,
    keyring: Arc<SecretKeyring>,
    slot: SecretSlot,
    state: tokio::sync::Mutex<TokenState>,
    lost: Arc<AtomicBool>,
}

impl std::fmt::Debug for OAuthTokenSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OAuthTokenSource").field("account_id", &self.slot.account_id).finish_non_exhaustive()
    }
}

impl OAuthTokenSource {
    /// A token source for a decrypted secret at `version`.
    pub fn new(
        client: Arc<OAuthClient>,
        http: reqwest::Client,
        api: Arc<dyn WorkerApi>,
        keyring: Arc<SecretKeyring>,
        slot: SecretSlot,
        secret: OAuthSecret,
        version: u64,
    ) -> Self {
        Self {
            client,
            http,
            api,
            keyring,
            slot,
            state: tokio::sync::Mutex::new(TokenState { secret, version, cached: None }),
            lost: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Set when a rotated refresh token could not be stored because the secret moved on.
    pub fn lost(&self) -> Arc<AtomicBool> {
        self.lost.clone()
    }

    /// Seeds the cache (right after a code exchange).
    pub async fn seed(&self, token: AccessToken) {
        self.state.lock().await.cached = Some(token);
    }

    /// The current refresh token (for revocation on account removal).
    pub async fn refresh_token(&self) -> String {
        self.state.lock().await.secret.refresh_token.clone()
    }
}

#[async_trait]
impl TokenSource for OAuthTokenSource {
    async fn access_token(&self) -> Result<AccessToken, ProviderError> {
        if self.lost.load(Ordering::Relaxed) {
            return Err(ProviderError::Transient("lease moved".into(), None));
        }
        // Holding the lock across the refresh makes it single-flight.
        let mut st = self.state.lock().await;
        let now = now_ms();
        if let Some(t) = &st.cached
            && t.expires_at - REFRESH_SKEW_MS > now
        {
            return Ok(t.clone());
        }
        let resp = match self.client.refresh(&self.http, &st.secret.refresh_token).await {
            Ok(r) => r,
            Err(OAuthError::InvalidGrant) => return Err(ProviderError::Auth { reauth_required: true }),
            Err(OAuthError::Rejected) => return Err(ProviderError::Auth { reauth_required: true }),
            Err(OAuthError::Unavailable) => return Err(ProviderError::Transient("token endpoint".into(), None)),
        };
        if let Some(rotated) = resp.refresh_token.clone().filter(|r| *r != st.secret.refresh_token) {
            let next = OAuthSecret {
                refresh_token: rotated,
                scope: resp.scope.clone().unwrap_or_else(|| st.secret.scope.clone()),
                obtained_at: now,
            };
            let plain =
                serde_json::to_vec(&next).map_err(|_| ProviderError::Transient("encode secret".into(), None))?;
            let sealed = self
                .keyring
                .seal(&self.slot.aad, &plain)
                .map_err(|_| ProviderError::Transient("seal secret".into(), None))?;
            let put = self
                .api
                .put_secret(&PutSecretArgs {
                    account_id: self.slot.account_id.clone(),
                    worker_id: self.slot.worker_id.clone(),
                    kind: SecretKind::Oauth,
                    key_id: sealed.key_id,
                    nonce: ConvexBytes(sealed.nonce),
                    ciphertext: ConvexBytes(sealed.ciphertext),
                    expected_version: st.version,
                })
                .await;
            match put {
                Ok(r) => {
                    st.version = r.version;
                    st.secret = next;
                }
                Err(e) if matches!(e.code(), Some(ErrorCode::VersionConflict | ErrorCode::LeaseLost)) => {
                    self.lost.store(true, Ordering::Relaxed);
                    return Err(ProviderError::Transient("lease moved".into(), None));
                }
                Err(_) => return Err(ProviderError::Transient("store secret".into(), None)),
            }
        }
        let token = resp.access(now);
        st.cached = Some(token.clone());
        Ok(token)
    }

    async fn invalidate(&self) {
        self.state.lock().await.cached = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use akatsuki_convex_api::{Convex, MockTransport};
    use serde_json::json;
    use wiremock::matchers::{body_string_contains, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn state(exp: UnixMs) -> OAuthState {
        OAuthState {
            user_id: "user-sealed-marker".into(),
            provider: OAuthKind::Google,
            nonce: "v".repeat(43),
            return_to: "https://app.akatsuki.vc/settings".into(),
            account_id: None,
            exp,
        }
    }

    #[test]
    fn state_is_sealed_bound_and_expires() {
        let key = [4u8; 32];
        let sealed = seal_state(&key, &state(10_000)).unwrap();
        // Markers long enough that random base64url ciphertext cannot contain them by chance.
        assert!(!sealed.contains("user-sealed-marker") && !sealed.contains("vvvvvvvv"));
        let back = open_state(&key, &sealed, OAuthKind::Google, 9_000).unwrap();
        assert_eq!(back.user_id, "user-sealed-marker");
        assert_eq!(open_state(&key, &sealed, OAuthKind::Google, 10_001), Err(StateError::Expired));
        assert_eq!(open_state(&key, &sealed, OAuthKind::Microsoft, 0), Err(StateError::WrongProvider));
        assert_eq!(open_state(&[5u8; 32], &sealed, OAuthKind::Google, 0), Err(StateError::Invalid));
        let mut chars: Vec<char> = sealed.chars().collect();
        let mid = chars.len() / 2;
        chars[mid] = if chars[mid] == 'A' { 'B' } else { 'A' };
        let tampered: String = chars.into_iter().collect();
        assert_eq!(open_state(&key, &tampered, OAuthKind::Google, 0), Err(StateError::Invalid));
        assert!(!format!("{back:?}").contains("user-sealed-marker"));
    }

    #[test]
    fn pkce_matches_rfc7636_example() {
        // RFC 7636 appendix B.
        assert_eq!(
            pkce_challenge("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"),
            "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
        );
        let (v, c) = pkce_pair().unwrap();
        assert_eq!(v.len(), 43);
        assert_eq!(pkce_challenge(&v), c);
    }

    #[test]
    fn authorize_urls_carry_pkce_and_scopes() {
        let g = OAuthClient::new(
            OAuthKind::Google,
            "cid".into(),
            "secret".into(),
            "https://sync.akatsuki.vc/",
            Endpoints::default(),
        );
        let u = url::Url::parse(&g.authorize_url("ST", "CH", Some("ada@example.com"))).unwrap();
        let q: std::collections::HashMap<_, _> = u.query_pairs().into_owned().collect();
        assert_eq!(q["redirect_uri"], "https://sync.akatsuki.vc/oauth/google/callback");
        assert_eq!(q["code_challenge_method"], "S256");
        assert_eq!(q["code_challenge"], "CH");
        assert_eq!(q["scope"], GOOGLE_SCOPES);
        assert_eq!(q["access_type"], "offline");
        assert_eq!(q["login_hint"], "ada@example.com");
        assert!(!u.as_str().contains("secret"));
        let m = OAuthClient::new(OAuthKind::Microsoft, "cid".into(), "s".into(), "https://x", Endpoints::default());
        assert!(m.authorize_url("s", "c", None).contains("Mail.Send"));
        assert!(!format!("{m:?}").contains("\"s\""));
    }

    fn endpoints(server: &MockServer) -> Endpoints {
        Endpoints {
            google_token: format!("{}/token", server.uri()),
            microsoft_token: format!("{}/mstoken", server.uri()),
            gmail_api: format!("{}/gmail/v1", server.uri()),
            graph_api: format!("{}/v1.0", server.uri()),
            ..Endpoints::default()
        }
    }

    #[tokio::test]
    async fn exchange_and_profiles() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/token"))
            .and(body_string_contains("code_verifier=VER"))
            .and(body_string_contains("grant_type=authorization_code"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "access_token": "at", "expires_in": 3599, "refresh_token": "rt", "scope": GOOGLE_SCOPES
            })))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/gmail/v1/users/me/profile"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"emailAddress": "Ada@Example.com"})))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/gmail/v1/users/me/settings/sendAs"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"sendAs": [
                {"sendAsEmail": "ada@example.com", "displayName": "Ada", "isDefault": true, "isPrimary": true},
                {"sendAsEmail": "alias@example.com", "verificationStatus": "accepted"},
                {"sendAsEmail": "pending@example.com", "verificationStatus": "pending"}
            ]})))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/v1.0/me"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "id": "obj-1", "mail": null, "userPrincipalName": "Bob@Contoso.com", "displayName": "Bob"
            })))
            .mount(&server)
            .await;
        let ep = endpoints(&server);
        let http = reqwest::Client::new();
        let g = OAuthClient::new(OAuthKind::Google, "cid".into(), "cs".into(), "https://sync.x", ep.clone());
        let t = g.exchange_code(&http, "CODE", "VER").await.unwrap();
        assert_eq!(t.refresh_token.as_deref(), Some("rt"));
        assert!(!format!("{t:?}").contains("rt"));
        let p = google_profile(&http, &ep, "at").await.unwrap();
        assert_eq!(p.provider_account_id, "ada@example.com");
        assert_eq!(p.display_name.as_deref(), Some("Ada"));
        assert_eq!(p.identities.len(), 2);
        let me = graph_profile(&http, &ep, "at").await.unwrap();
        assert_eq!(me.provider_account_id, "obj-1");
        assert_eq!(me.email, "bob@contoso.com");
    }

    #[tokio::test]
    async fn token_source_refreshes_once_and_persists_rotation() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/mstoken"))
            .and(body_string_contains("refresh_token=old"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "access_token": "at1", "expires_in": 3600, "refresh_token": "new"
            })))
            .expect(1)
            .mount(&server)
            .await;
        let client = Arc::new(OAuthClient::new(
            OAuthKind::Microsoft,
            "cid".into(),
            "cs".into(),
            "https://sync.x",
            endpoints(&server),
        ));
        let mock = Convex::new(MockTransport::new());
        mock.transport().respond("worker:putSecret", Ok(json!({"version": 2})));
        let api: Arc<dyn WorkerApi> = Arc::new(mock);
        let keyring = Arc::new(SecretKeyring::new(vec![("k1".into(), crate::config::Key32([1; 32]))]));
        let aad = akatsuki_core::convex::worker::secret_aad("u1", Provider::Graph, "obj", SecretKind::Oauth);
        let ts = Arc::new(OAuthTokenSource::new(
            client,
            reqwest::Client::new(),
            api.clone(),
            keyring.clone(),
            SecretSlot { account_id: "a1".into(), worker_id: "w1".into(), aad: aad.clone() },
            OAuthSecret { refresh_token: "old".into(), scope: String::new(), obtained_at: 0 },
            1,
        ));
        // Concurrent callers share one refresh.
        let (a, b) = tokio::join!(ts.access_token(), ts.access_token());
        assert_eq!(a.unwrap().token, "at1");
        assert_eq!(b.unwrap().token, "at1");
        assert_eq!(ts.refresh_token().await, "new");
        assert!(!ts.lost().load(Ordering::Relaxed));
        server.verify().await;
    }

    #[tokio::test]
    async fn invalid_grant_means_reauth_and_cas_failure_releases() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/token"))
            .respond_with(ResponseTemplate::new(400).set_body_json(json!({"error": "invalid_grant"})))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/mstoken"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "access_token": "at1", "expires_in": 3600, "refresh_token": "rotated"
            })))
            .mount(&server)
            .await;
        let keyring = Arc::new(SecretKeyring::new(vec![("k1".into(), crate::config::Key32([1; 32]))]));
        let mk = |kind, api: Arc<dyn WorkerApi>| {
            OAuthTokenSource::new(
                Arc::new(OAuthClient::new(kind, "c".into(), "s".into(), "https://x", endpoints(&server))),
                reqwest::Client::new(),
                api,
                keyring.clone(),
                SecretSlot { account_id: "a1".into(), worker_id: "w1".into(), aad: b"aad".to_vec() },
                OAuthSecret { refresh_token: "old".into(), scope: String::new(), obtained_at: 0 },
                3,
            )
        };
        let api: Arc<dyn WorkerApi> = Arc::new(Convex::new(MockTransport::new()));
        let g = mk(OAuthKind::Google, api);
        assert_eq!(g.access_token().await.unwrap_err(), ProviderError::Auth { reauth_required: true });

        let mock = Convex::new(MockTransport::new());
        mock.transport().respond(
            "worker:putSecret",
            Err(akatsuki_convex_api::ConvexApiError::from_app_error(
                "worker:putSecret",
                "x".into(),
                json!({"code": "version_conflict", "message": "m"}),
            )),
        );
        let m = mk(OAuthKind::Microsoft, Arc::new(mock));
        assert!(matches!(m.access_token().await, Err(ProviderError::Transient(..))));
        assert!(m.lost().load(Ordering::Relaxed));
    }
}
