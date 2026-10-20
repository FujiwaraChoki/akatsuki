//! Route handlers of the worker's HTTP API (ARCHITECTURE §7.2), on top of [`Services`].
//!
//! Every handler resolves the ids it receives against Convex for the calling user (§6.5,
//! D-049): messages through `worker.resolveMessages` (cached 60 s per user and id), threads
//! through `worker.resolveThread`, accounts through `worker.userContext`. Ids the user does not own
//! look exactly like ids that do not exist (`404 not_found`).

pub mod bodies;
pub mod compose;
pub mod connect;
pub mod hooks;
pub mod media;
pub mod search;

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use akatsuki_convex_api::{ConvexApiError, WorkerApi};
use akatsuki_core::convex::worker::{ResolveMessagesArgs, ResolvedMessage, UserContext, UserContextArgs};
use akatsuki_core::ids::{AccountId, MessageId};
use akatsuki_core::sync::budget::TokenBucket;
use akatsuki_core::time::now_ms;
use akatsuki_core::wire::ErrorCode;
use akatsuki_providers::{ProviderError, RemoteMessageRef};
use futures::StreamExt as _;

use crate::config::Config;
use crate::crypto::SecretKeyring;
use crate::engine::{AccountHandle, Registry};
use crate::fetch::SafeFetcher;
use crate::http::ApiError;
use crate::metrics::Metrics;
use crate::oauth::{OAuthClient, OAuthKind};
use crate::providers::ImapProbe;
use crate::push_auth::PushVerifier;
use crate::spool::Spool;
use crate::user_convex::UserConvex;

/// How long an ownership check is trusted (§6.5).
pub const RESOLVE_CACHE_TTL: Duration = Duration::from_secs(60);
/// Largest raw message the worker reads into memory.
pub const MAX_RAW_BYTES: usize = 64 * 1024 * 1024;
/// Image proxy requests per user hash per minute.
pub const IMG_PER_MINUTE: u32 = 300;

/// Everything the handlers need besides the configuration.
pub struct Services {
    /// Convex under the worker identity.
    pub api: Arc<dyn WorkerApi>,
    /// Running account actors.
    pub registry: Arc<Registry>,
    /// `TOKEN_ENC_KEYS`.
    pub keyring: Arc<SecretKeyring>,
    /// Mailbox OAuth clients.
    pub oauth: HashMap<OAuthKind, Arc<OAuthClient>>,
    /// Shared HTTP client (providers, OAuth).
    pub http: reqwest::Client,
    /// SSRF-guarded client (image proxy).
    pub fetcher: SafeFetcher,
    /// Upload spool (`SPOOL_KEY`).
    pub spool: Option<Spool>,
    /// Client functions called with the user's JWT.
    pub user_convex: Arc<dyn UserConvex>,
    /// IMAP/SMTP login probe.
    pub imap_probe: Arc<dyn ImapProbe>,
    /// Gmail Pub/Sub push verifier.
    pub push_verifier: Option<Arc<PushVerifier>>,
    /// Counters.
    pub metrics: Arc<Metrics>,
    resolved: Mutex<HashMap<(String, String), (ResolvedMessage, Instant)>>,
    contexts: Mutex<HashMap<String, (UserContext, Instant)>>,
    img_rate: Mutex<HashMap<String, TokenBucket>>,
    /// Pending OAuth connects (D-395).
    pub(crate) connect: connect::ConnectFlows,
    /// Per-user limits (§13.4, D-397).
    pub limits: crate::limits::Limits,
}

impl std::fmt::Debug for Services {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Services").finish_non_exhaustive()
    }
}

impl Services {
    /// Services over the given parts.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        api: Arc<dyn WorkerApi>,
        registry: Arc<Registry>,
        keyring: Arc<SecretKeyring>,
        oauth: HashMap<OAuthKind, Arc<OAuthClient>>,
        http: reqwest::Client,
        fetcher: SafeFetcher,
        spool: Option<Spool>,
        user_convex: Arc<dyn UserConvex>,
        imap_probe: Arc<dyn ImapProbe>,
        push_verifier: Option<Arc<PushVerifier>>,
        metrics: Arc<Metrics>,
    ) -> Self {
        Self {
            api,
            registry,
            keyring,
            oauth,
            http,
            fetcher,
            spool,
            user_convex,
            imap_probe,
            push_verifier,
            metrics,
            resolved: Mutex::new(HashMap::new()),
            contexts: Mutex::new(HashMap::new()),
            img_rate: Mutex::new(HashMap::new()),
            connect: connect::ConnectFlows::default(),
            limits: crate::limits::Limits::default(),
        }
    }

    /// Resolves message ids the user owns (cached 60 s); unknown ids are omitted.
    pub async fn resolve_messages(&self, user_id: &str, ids: &[MessageId]) -> Result<Vec<ResolvedMessage>, ApiError> {
        let now = Instant::now();
        let mut found = Vec::new();
        let mut missing = Vec::new();
        {
            let mut cache = self.resolved.lock().unwrap_or_else(|e| e.into_inner());
            cache.retain(|_, (_, at)| now.duration_since(*at) < RESOLVE_CACHE_TTL);
            for id in ids {
                match cache.get(&(user_id.to_owned(), id.as_str().to_owned())) {
                    Some((r, _)) => found.push(r.clone()),
                    None => missing.push(id.clone()),
                }
            }
        }
        if !missing.is_empty() {
            let rows = self
                .api
                .resolve_messages(&ResolveMessagesArgs { user_id: user_id.to_owned(), message_ids: missing })
                .await
                .map_err(|e| convex_api_error(&e))?;
            let mut cache = self.resolved.lock().unwrap_or_else(|e| e.into_inner());
            for r in rows {
                cache.insert((user_id.to_owned(), r.message_id.as_str().to_owned()), (r.clone(), now));
                found.push(r);
            }
        }
        // Keep the caller's order.
        let pos: HashMap<&str, usize> = ids.iter().enumerate().map(|(i, id)| (id.as_str(), i)).collect();
        found.sort_by_key(|r| pos.get(r.message_id.as_str()).copied().unwrap_or(usize::MAX));
        Ok(found)
    }

    /// Resolves one message or answers 404.
    pub async fn resolve_message(&self, user_id: &str, id: &MessageId) -> Result<ResolvedMessage, ApiError> {
        self.resolve_messages(user_id, std::slice::from_ref(id))
            .await?
            .into_iter()
            .next()
            .ok_or_else(|| ApiError::new(ErrorCode::NotFound))
    }

    /// `worker.userContext` (cached 60 s).
    pub async fn user_context(&self, user_id: &str) -> Result<UserContext, ApiError> {
        let now = Instant::now();
        if let Some((c, at)) = self.contexts.lock().unwrap_or_else(|e| e.into_inner()).get(user_id)
            && now.duration_since(*at) < RESOLVE_CACHE_TTL
        {
            return Ok(c.clone());
        }
        let c = self
            .api
            .user_context(&UserContextArgs { user_id: user_id.to_owned() })
            .await
            .map_err(|e| convex_api_error(&e))?;
        self.contexts.lock().unwrap_or_else(|e| e.into_inner()).insert(user_id.to_owned(), (c.clone(), now));
        Ok(c)
    }

    /// The user's context, requiring `hostedSync` (`402 not_entitled` otherwise).
    pub async fn entitled(&self, user_id: &str) -> Result<UserContext, ApiError> {
        let c = self.user_context(user_id).await?;
        if !c.entitlement.hosted_sync {
            return Err(ApiError::new(ErrorCode::NotEntitled));
        }
        Ok(c)
    }

    /// Drops the cached context of a user, after their accounts changed (connect, D-383).
    pub fn forget_user(&self, user_id: &str) {
        self.contexts.lock().unwrap_or_else(|e| e.into_inner()).remove(user_id);
    }

    /// The entitled context of a user who must own `account_id`. A cached context that does not list
    /// the account yet (connected within [`RESOLVE_CACHE_TTL`], possibly through another worker
    /// request) is re-read once before answering `404 not_found` (D-383).
    pub async fn entitled_with_account(&self, user_id: &str, account_id: &AccountId) -> Result<UserContext, ApiError> {
        let c = self.entitled(user_id).await?;
        if c.account_ids.contains(account_id) {
            return Ok(c);
        }
        self.forget_user(user_id);
        let c = self.entitled(user_id).await?;
        if c.account_ids.contains(account_id) { Ok(c) } else { Err(ApiError::new(ErrorCode::NotFound)) }
    }

    /// The running actor of an account the user owns.
    pub fn handle(&self, user_id: &str, account_id: &AccountId) -> Result<Arc<AccountHandle>, ApiError> {
        match self.registry.get(account_id) {
            Some(h) if h.user_id() == user_id => {
                h.touch();
                Ok(h)
            }
            Some(_) => Err(ApiError::new(ErrorCode::NotFound)),
            None => Err(ApiError::with_message(
                ErrorCode::ProviderUnavailable,
                "This account is not syncing right now. Try again in a moment.",
            )
            .retry_after(5_000)),
        }
    }

    /// Image-proxy rate limit per user hash.
    pub fn img_allowed(&self, u: &str) -> bool {
        let mut m = self.img_rate.lock().unwrap_or_else(|e| e.into_inner());
        if m.len() > 10_000 {
            m.clear();
        }
        m.entry(u.to_owned())
            .or_insert_with(|| TokenBucket::per_minute(IMG_PER_MINUTE, now_ms()))
            .try_take(1, now_ms())
            .is_ok()
    }
}

/// Maps a Convex error to the envelope (never with server text).
pub fn convex_api_error(e: &ConvexApiError) -> ApiError {
    match e.code() {
        Some(
            code @ (ErrorCode::NotFound
            | ErrorCode::NotEntitled
            | ErrorCode::InvalidArgument
            | ErrorCode::AlreadyConnected),
        ) => ApiError::new(code),
        _ => {
            ApiError::with_message(ErrorCode::ProviderUnavailable, "The service is busy. Try again.").retry_after(2_000)
        }
    }
}

/// Maps a provider error to the envelope.
pub fn provider_api_error(e: &ProviderError) -> ApiError {
    let mut err = ApiError::new(e.code());
    if let ProviderError::RateLimited { retry_after, .. } = e {
        err = err.retry_after(retry_after.map_or(5_000, |d| u64::try_from(d.as_millis()).unwrap_or(5_000)));
    }
    err
}

impl Services {
    /// [`fetch_raw`] inside the user's and the process's raw-read slots (§13.4, D-397): each read
    /// may hold up to [`MAX_RAW_BYTES`] in memory.
    pub async fn fetch_raw(
        &self,
        user_id: &str,
        handle: &AccountHandle,
        msg: &ResolvedMessage,
    ) -> Result<Vec<u8>, ApiError> {
        let _slot = self.limits.raw_slot(user_id).await?;
        fetch_raw(handle, msg).await
    }
}

/// Reads a message's RFC 822 source into memory (bounded). Handlers go through
/// [`Services::fetch_raw`], which bounds how many run at once.
pub async fn fetch_raw(handle: &AccountHandle, msg: &ResolvedMessage) -> Result<Vec<u8>, ApiError> {
    let r = RemoteMessageRef {
        provider_message_id: msg.provider_message_id.clone(),
        imap: msg.imap_locations.clone().unwrap_or_default(),
    };
    let mut stream = handle.provider.fetch_raw(&r).await.map_err(|e| provider_api_error(&e))?;
    let mut buf = Vec::new();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| provider_api_error(&e))?;
        if buf.len() + chunk.len() > MAX_RAW_BYTES {
            return Err(ApiError::new(ErrorCode::PayloadTooLarge));
        }
        buf.extend_from_slice(&chunk);
    }
    Ok(buf)
}

/// Requires the services (`503` when the worker runs without Convex).
pub fn services(s: &Option<Arc<Services>>) -> Result<&Arc<Services>, ApiError> {
    s.as_ref().ok_or_else(|| {
        ApiError::with_message(ErrorCode::ProviderUnavailable, "The sync service is not configured on this server.")
    })
}

/// The configured public base URL or `503`.
pub fn public_base(config: &Config) -> Result<&str, ApiError> {
    config.public_base_url.as_deref().ok_or_else(|| {
        ApiError::with_message(ErrorCode::ProviderUnavailable, "PUBLIC_BASE_URL is not configured on this server.")
    })
}
