//! Connecting a mailbox (ARCHITECTURE §6.7): OAuth start, callback and completion for Google and
//! Microsoft, and `POST /v1/connect/imap`.
//!
//! The OAuth flow is bound to the user who finishes it (DECISIONS D-395). `start` keeps the PKCE
//! verifier in the worker under a single-use nonce that the sealed `state` names. The callback does
//! not create the account: it keeps the sealed grant in the worker and sends the browser back with
//! a random grant handle. The client then calls `POST /v1/connect/complete` with its own JWT, and
//! the account is created only when that user is the one who started the flow. A link to a
//! genuine consent screen that someone else started therefore cannot attach the victim's mailbox
//! to the starter's account.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use akatsuki_core::convex::values::ConvexBytes;
use akatsuki_core::convex::worker::{CreateAccountArgs, IdentityIn, SecretIn};
use akatsuki_core::convex::{ImapSettings, Provider, SecretKind};
use akatsuki_core::time::now_ms;
use akatsuki_core::wire::ErrorCode;
use akatsuki_core::wire::worker::{
    ConnectComplete, ConnectCompleteResponse, ConnectStart, ConnectStartResponse, ImapConnect, ImapConnectResponse,
    ImapDiscoverQuery, ImapDiscovery, ImapDiscoverySource, OAuthCallbackQuery,
};
use akatsuki_providers::imap::autoconfig::{self, AutoconfigSources, DiscoverySource};
use akatsuki_providers::{ImapCaps, ProviderCaps};
use axum::extract::{Path, Query, State};
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::{Extension, Json};

use super::{Services, convex_api_error, services};
use crate::http::{ApiError, ApiJson, AppState, CurrentUser, return_to_allowed};
use crate::oauth::{
    ImapEndpoints, ImapSecret, OAuthError, OAuthKind, OAuthSecret, OAuthState, STATE_TTL_MS, google_profile,
    graph_profile, open_state, pkce_pair, seal_state,
};

fn kind(provider: &str) -> Result<OAuthKind, ApiError> {
    OAuthKind::from_path(provider).ok_or_else(|| ApiError::new(ErrorCode::NotFound))
}

/// How long a started flow and an unclaimed grant live (the `state` lifetime).
pub const FLOW_TTL: Duration = Duration::from_millis(STATE_TTL_MS as u64);
/// Pending flows and grants kept at most (each map); beyond that `start` answers 429.
pub const MAX_PENDING: usize = 10_000;

/// A started OAuth flow, keyed by the nonce in its `state`.
pub(crate) struct PendingFlow {
    user_id: String,
    provider: OAuthKind,
    pkce_verifier: String,
    at: Instant,
}

/// A grant waiting for `POST /v1/connect/complete`, keyed by a random handle. `args.secret` is
/// already sealed; the refresh token never sits in memory in the clear.
pub(crate) struct PendingGrant {
    args: CreateAccountArgs,
    at: Instant,
}

/// The worker-side halves of OAuth connects (one worker process in v1, D-044).
#[derive(Default)]
pub(crate) struct ConnectFlows {
    flows: Mutex<HashMap<String, PendingFlow>>,
    grants: Mutex<HashMap<String, PendingGrant>>,
}

fn insert_bounded<V>(map: &Mutex<HashMap<String, V>>, key: String, value: V, at: impl Fn(&V) -> Instant) -> bool {
    let mut m = map.lock().unwrap_or_else(|e| e.into_inner());
    if m.len() >= MAX_PENDING {
        m.retain(|_, v| at(v).elapsed() < FLOW_TTL);
        if m.len() >= MAX_PENDING {
            return false;
        }
    }
    m.insert(key, value);
    true
}

fn take_fresh<V>(map: &Mutex<HashMap<String, V>>, key: &str, at: impl Fn(&V) -> Instant) -> Option<V> {
    let mut m = map.lock().unwrap_or_else(|e| e.into_inner());
    m.remove(key).filter(|v| at(v).elapsed() < FLOW_TTL)
}

impl ConnectFlows {
    fn start(&self, nonce: String, flow: PendingFlow) -> bool {
        insert_bounded(&self.flows, nonce, flow, |f| f.at)
    }

    /// Removes the flow (single use) when it is fresh.
    fn take_flow(&self, nonce: &str) -> Option<PendingFlow> {
        take_fresh(&self.flows, nonce, |f| f.at)
    }

    fn park(&self, handle: String, grant: PendingGrant) -> bool {
        insert_bounded(&self.grants, handle, grant, |g| g.at)
    }

    /// Removes the grant (single use, whoever asks) when it is fresh.
    fn take_grant(&self, handle: &str) -> Option<PendingGrant> {
        take_fresh(&self.grants, handle, |g| g.at)
    }
}

/// `POST /v1/connect/{google|microsoft}/start`.
pub async fn start(
    State(state): State<AppState>,
    Path(provider): Path<String>,
    Extension(CurrentUser(user)): Extension<CurrentUser>,
    ApiJson(body): ApiJson<ConnectStart>,
) -> Result<Response, ApiError> {
    let kind = kind(&provider)?;
    if !return_to_allowed(&body.return_to, &state.config.cors_origins) {
        return Err(ApiError::with_message(ErrorCode::InvalidArgument, "returnTo is not allowed."));
    }
    let svc = services(&state.services)?;
    let client = svc.oauth.get(&kind).ok_or_else(|| {
        ApiError::with_message(
            ErrorCode::ProviderUnavailable,
            "Connecting this provider is not configured on this server.",
        )
    })?;
    let key = state.config.state_key.as_ref().ok_or_else(|| {
        ApiError::with_message(ErrorCode::ProviderUnavailable, "Connecting is not configured on this server.")
    })?;
    match &body.account_id {
        Some(a) => svc.entitled_with_account(&user.user_id, a).await?,
        None => svc.entitled(&user.user_id).await?,
    };
    svc.limits.connect(&user.user_id)?;
    let (verifier, challenge) = pkce_pair().map_err(|_| ApiError::new(ErrorCode::Internal))?;
    let nonce = crate::crypto::random_token::<32>().map_err(|_| ApiError::new(ErrorCode::Internal))?;
    let flow =
        PendingFlow { user_id: user.user_id.clone(), provider: kind, pkce_verifier: verifier, at: Instant::now() };
    if !svc.connect.start(nonce.clone(), flow) {
        return Err(ApiError::new(ErrorCode::RateLimited).retry_after(60_000));
    }
    let sealed = seal_state(
        &key.0,
        &OAuthState {
            user_id: user.user_id.clone(),
            provider: kind,
            nonce,
            return_to: body.return_to.clone(),
            account_id: body.account_id.clone(),
            exp: now_ms() + STATE_TTL_MS,
        },
    )
    .map_err(|_| ApiError::new(ErrorCode::Internal))?;
    let authorize_url = client.authorize_url(&sealed, &challenge, body.login_hint.as_deref());
    Ok(Json(ConnectStartResponse { authorize_url }).into_response())
}

/// `{returnTo}?status=…` (or `&status=…` when `returnTo` already has a query).
pub fn return_url(return_to: &str, pairs: &[(&str, &str)]) -> String {
    let mut url = match url::Url::parse(return_to) {
        Ok(u) => u,
        Err(_) => return return_to.to_owned(),
    };
    {
        let mut q = url.query_pairs_mut();
        for (k, v) in pairs {
            q.append_pair(k, v);
        }
    }
    url.into()
}

fn redirect(to: &str) -> Response {
    let mut resp = StatusCode::FOUND.into_response();
    if let Ok(v) = HeaderValue::from_str(to) {
        resp.headers_mut().insert(header::LOCATION, v);
    }
    resp
}

fn redirect_error(return_to: &str, code: ErrorCode) -> Response {
    redirect(&return_url(return_to, &[("status", "error"), ("code", code.as_str())]))
}

/// `GET /oauth/{google|microsoft}/callback`.
pub async fn callback(
    State(state): State<AppState>,
    Path(provider): Path<String>,
    Query(q): Query<OAuthCallbackQuery>,
) -> Result<Response, ApiError> {
    let kind = kind(&provider)?;
    let svc = services(&state.services)?;
    let key = state.config.state_key.as_ref().ok_or_else(|| ApiError::new(ErrorCode::ProviderUnavailable))?;
    // Without a valid state there is no trusted place to send the user back to.
    let expired = || ApiError::with_message(ErrorCode::InvalidArgument, "This sign-in link expired. Start again.");
    let st = q
        .state
        .as_deref()
        .and_then(|s| open_state(&key.0, s, kind, now_ms()).ok())
        .filter(|s| return_to_allowed(&s.return_to, &state.config.cors_origins))
        .ok_or_else(expired)?;
    // Single use: a state (and its PKCE verifier) is consumed by the first callback that names it.
    let flow = svc
        .connect
        .take_flow(&st.nonce)
        .filter(|f| f.user_id == st.user_id && f.provider == kind)
        .ok_or_else(expired)?;
    if q.error.is_some() {
        return Ok(redirect_error(&st.return_to, ErrorCode::Unauthenticated));
    }
    let Some(code) = q.code.as_deref().filter(|c| !c.is_empty() && c.len() <= 4096) else {
        return Ok(redirect_error(&st.return_to, ErrorCode::InvalidArgument));
    };
    let Some(client) = svc.oauth.get(&kind) else {
        return Ok(redirect_error(&st.return_to, ErrorCode::ProviderUnavailable));
    };
    let tokens = match client.exchange_code(&svc.http, code, &flow.pkce_verifier).await {
        Ok(t) => t,
        Err(OAuthError::Unavailable) => return Ok(redirect_error(&st.return_to, ErrorCode::ProviderUnavailable)),
        Err(_) => return Ok(redirect_error(&st.return_to, ErrorCode::Unauthenticated)),
    };
    let Some(refresh_token) = tokens.refresh_token.clone() else {
        return Ok(redirect_error(&st.return_to, ErrorCode::ProviderInvalid));
    };
    let profile = match kind {
        OAuthKind::Google => google_profile(&svc.http, &client.endpoints, &tokens.access_token).await,
        OAuthKind::Microsoft => graph_profile(&svc.http, &client.endpoints, &tokens.access_token).await,
    };
    let Ok(profile) = profile else { return Ok(redirect_error(&st.return_to, ErrorCode::ProviderUnavailable)) };
    let provider = kind.provider();
    let secret = OAuthSecret { refresh_token, scope: tokens.scope.clone().unwrap_or_default(), obtained_at: now_ms() };
    let args = match seal_create(svc, &st.user_id, provider, &profile.provider_account_id, SecretKind::Oauth, &secret) {
        Ok(sealed) => CreateAccountArgs {
            user_id: st.user_id.clone(),
            provider,
            provider_account_id: profile.provider_account_id.clone(),
            email: profile.email.clone(),
            display_name: profile.display_name.clone(),
            imap: None,
            caps: match provider {
                Provider::Gmail => ProviderCaps::gmail().account_caps(),
                _ => ProviderCaps::graph().account_caps(),
            },
            identities: profile.identities.clone(),
            secret: sealed,
            reauth_account_id: st.account_id.clone(),
        },
        Err(e) => return Ok(redirect_error(&st.return_to, e.0.code)),
    };
    // The account is created by `POST /v1/connect/complete`, by the user who started this flow.
    let Ok(handle) = crate::crypto::random_token::<32>() else {
        return Ok(redirect_error(&st.return_to, ErrorCode::Internal));
    };
    if !svc.connect.park(handle.clone(), PendingGrant { args, at: Instant::now() }) {
        return Ok(redirect_error(&st.return_to, ErrorCode::RateLimited));
    }
    Ok(redirect(&return_url(&st.return_to, &[("status", "pending"), ("grant", handle.as_str())])))
}

/// `POST /v1/connect/complete`: creates the account of a finished OAuth flow for the caller, who
/// must be the user who started it (DECISIONS D-395). The grant is consumed by the first attempt,
/// whoever makes it, so a grant that reached the wrong browser is dead.
pub async fn complete(
    State(state): State<AppState>,
    Extension(CurrentUser(user)): Extension<CurrentUser>,
    ApiJson(body): ApiJson<ConnectComplete>,
) -> Result<Response, ApiError> {
    let svc = services(&state.services)?;
    if body.grant.is_empty() || body.grant.len() > 128 {
        return Err(ApiError::new(ErrorCode::InvalidArgument));
    }
    let gone = || ApiError::with_message(ErrorCode::NotFound, "This connection expired. Start again.");
    let grant = svc.connect.take_grant(&body.grant).ok_or_else(gone)?;
    if grant.args.user_id != user.user_id {
        tracing::warn!("connect grant presented by another user; refused");
        return Err(ApiError::with_message(
            ErrorCode::InvalidArgument,
            "This connection was started from another Akatsuki sign-in. Start again from your own account.",
        ));
    }
    match &grant.args.reauth_account_id {
        Some(a) => svc.entitled_with_account(&user.user_id, a).await?,
        None => svc.entitled(&user.user_id).await?,
    };
    let r = svc.api.create_account(&grant.args).await.map_err(|e| convex_api_error(&e))?;
    svc.forget_user(&user.user_id);
    Ok((StatusCode::CREATED, Json(ConnectCompleteResponse { account_id: r.account_id, reconnected: r.reconnected }))
        .into_response())
}

impl Services {
    /// Test hook: whether a grant handle is still waiting.
    #[cfg(test)]
    pub(crate) fn grant_pending(&self, handle: &str) -> bool {
        self.connect.grants.lock().unwrap_or_else(|e| e.into_inner()).contains_key(handle)
    }
}

fn seal_create<T: serde::Serialize>(
    svc: &super::Services,
    user_id: &str,
    provider: Provider,
    provider_account_id: &str,
    kind: SecretKind,
    secret: &T,
) -> Result<SecretIn, ApiError> {
    let plain = serde_json::to_vec(secret).map_err(|_| ApiError::new(ErrorCode::Internal))?;
    let aad = akatsuki_core::convex::worker::secret_aad(user_id, provider, provider_account_id, kind);
    let sealed = svc.keyring.seal(&aad, &plain).map_err(|_| {
        ApiError::with_message(ErrorCode::ProviderUnavailable, "TOKEN_ENC_KEYS is not configured on this server.")
    })?;
    Ok(SecretIn {
        kind,
        key_id: sealed.key_id,
        nonce: ConvexBytes(sealed.nonce),
        ciphertext: ConvexBytes(sealed.ciphertext),
    })
}

/// IMAP ports a hosted connect may use (D-398).
pub const IMAP_PORTS: [u16; 2] = [993, 143];
/// SMTP submission ports a hosted connect may use (D-398).
pub const SMTP_PORTS: [u16; 4] = [465, 587, 25, 2525];

/// `GET /v1/connect/imap/discover?email=` (DECISIONS D-396): IMAP and SMTP settings for an address
/// from the built-in table, the domain's autoconfig file or the Thunderbird ISPDB. The lookups go
/// through the SSRF-guarded client (§14.4): the domain is user input.
pub async fn discover(
    State(state): State<AppState>,
    Extension(CurrentUser(user)): Extension<CurrentUser>,
    Query(q): Query<ImapDiscoverQuery>,
) -> Result<Response, ApiError> {
    let svc = services(&state.services)?;
    let email = q.email.trim();
    if email.len() > 320 || autoconfig::email_domain(email).is_none() {
        return Err(ApiError::new(ErrorCode::InvalidArgument));
    }
    svc.limits.connect_discover(&user.user_id)?;
    let found = autoconfig::discover(svc.fetcher.client(), email, &AutoconfigSources::default())
        .await
        .ok_or_else(|| ApiError::with_message(ErrorCode::NotFound, "No settings are known for this address."))?;
    Ok(Json(ImapDiscovery {
        imap: found.imap,
        smtp: found.smtp,
        username: found.username,
        source: match found.source {
            DiscoverySource::WellKnown => ImapDiscoverySource::WellKnown,
            DiscoverySource::Autoconfig => ImapDiscoverySource::Autoconfig,
            DiscoverySource::Ispdb => ImapDiscoverySource::Ispdb,
        },
        app_password_required: found.app_password_required,
    })
    .into_response())
}

/// `POST /v1/connect/imap`. The password crosses TLS once and is never echoed or logged.
pub async fn imap(
    State(state): State<AppState>,
    Extension(CurrentUser(user)): Extension<CurrentUser>,
    ApiJson(body): ApiJson<ImapConnect>,
) -> Result<Response, ApiError> {
    let email = body.email.trim().to_lowercase();
    let valid_host = |h: &str| !h.trim().is_empty() && h.len() <= 253 && !h.contains(['/', '@', ' ']);
    if !email.contains('@')
        || body.username.trim().is_empty()
        || body.password.is_empty()
        || !valid_host(&body.imap.host)
        || !valid_host(&body.smtp.host)
        || body.imap.port == 0
        || body.smtp.port == 0
    {
        return Err(ApiError::new(ErrorCode::InvalidArgument));
    }
    // The hosted worker reaches public addresses on mail ports only (D-398): no port scanning of
    // other services through the probe. The local harness and LAN self-hosting lift both rules.
    if !state.config.mail_allow_private_hosts
        && (!IMAP_PORTS.contains(&body.imap.port) || !SMTP_PORTS.contains(&body.smtp.port))
    {
        return Err(ApiError::with_message(
            ErrorCode::InvalidArgument,
            "Use a standard mail port: IMAP 993 or 143, SMTP 465, 587, 25 or 2525.",
        ));
    }
    let svc = services(&state.services)?;
    svc.entitled(&user.user_id).await?;
    // 10/h per user: the probe logs in to a server the user names (§13.4, D-397).
    svc.limits.connect(&user.user_id)?;
    let probe = match svc.imap_probe.probe(&body).await {
        Ok(p) => p,
        Err(ErrorCode::NotImplemented) => {
            return Err(ApiError::with_message(ErrorCode::NotImplemented, "IMAP accounts are not available yet."));
        }
        Err(code) => return Err(ApiError::new(code)),
    };
    let host = body.imap.host.trim().trim_end_matches('.').to_lowercase();
    let provider_account_id = format!("{}@{}", body.username.trim(), host);
    let settings = ImapSettings {
        host,
        port: body.imap.port,
        tls: body.imap.tls,
        smtp_host: body.smtp.host.trim().trim_end_matches('.').to_lowercase(),
        smtp_port: body.smtp.port,
        smtp_tls: body.smtp.tls,
        capabilities: probe.capabilities.clone(),
        sent_auto_saved: probe.sent_auto_saved,
    };
    // The password is sealed together with the servers it was entered for (D-414).
    let secret = ImapSecret {
        username: body.username.clone(),
        password: body.password.clone(),
        endpoints: Some(ImapEndpoints::of(&settings)),
    };
    let sealed = seal_create(svc, &user.user_id, Provider::Imap, &provider_account_id, SecretKind::Imap, &secret)?;
    drop(secret);
    let caps = ProviderCaps::imap(ImapCaps::from_capabilities(&probe.capabilities), probe.sent_auto_saved);
    let args = CreateAccountArgs {
        user_id: user.user_id.clone(),
        provider: Provider::Imap,
        provider_account_id,
        email: email.clone(),
        display_name: None,
        imap: Some(settings),
        caps: caps.account_caps(),
        identities: vec![IdentityIn { email, name: None, reply_to: None, is_default: true }],
        secret: sealed,
        reauth_account_id: None,
    };
    let r = svc.api.create_account(&args).await.map_err(|e| convex_api_error(&e))?;
    svc.forget_user(&user.user_id);
    Ok((StatusCode::CREATED, Json(ImapConnectResponse { account_id: r.account_id })).into_response())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn return_urls_keep_existing_queries() {
        assert_eq!(
            return_url("https://app.akatsuki.vc/settings", &[("status", "connected"), ("accountId", "a1")]),
            "https://app.akatsuki.vc/settings?status=connected&accountId=a1"
        );
        assert_eq!(
            return_url("http://127.0.0.1:5000/cb?n=x", &[("status", "error"), ("code", "already_connected")]),
            "http://127.0.0.1:5000/cb?n=x&status=error&code=already_connected"
        );
        assert_eq!(
            return_url("akatsuki://connected", &[("status", "connected")]),
            "akatsuki://connected?status=connected"
        );
    }
}
