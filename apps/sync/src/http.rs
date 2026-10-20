//! The worker's HTTP API (ARCHITECTURE §7): the full route table and the common rules of §7.1
//! (bearer auth, exact-origin CORS, the error envelope, no-store caching, content-free logging).
//! Handlers live in `crate::api`; they need [`AppState::services`] (Convex, the actor registry,
//! keys), and answer `503 provider_unavailable` when the worker runs without them.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};

use akatsuki_core::auth::{AuthError, JwksSource, UserVerifier, VerifiedUser, bearer_token};
use akatsuki_core::wire::{ErrorBody, ErrorCode, ErrorPayload, Health};
use axum::body::Body;
use axum::extract::rejection::JsonRejection;
use axum::extract::{DefaultBodyLimit, FromRequest, MatchedPath, Request, State};
use axum::http::{HeaderMap, HeaderName, HeaderValue, Method, StatusCode, header};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post, put};
use axum::{Json, Router};
use serde::de::DeserializeOwned;
use sha2::{Digest, Sha256};
use tower_http::catch_panic::CatchPanicLayer;
use tower_http::cors::{AllowOrigin, CorsLayer};
use tower_http::set_header::SetResponseHeaderLayer;

use crate::api::{self, Services};
use crate::config::Config;
use crate::metrics::Metrics;

/// Largest JSON body accepted outside uploads.
pub const JSON_BODY_LIMIT: usize = 2 * 1024 * 1024;
/// Largest upload (`POST /v1/uploads`).
pub const UPLOAD_LIMIT: usize = 25 * 1024 * 1024;

/// A 1×1 transparent GIF (read-receipt pixel).
pub const PIXEL_GIF: &[u8] = &[
    0x47, 0x49, 0x46, 0x38, 0x39, 0x61, 0x01, 0x00, 0x01, 0x00, 0x80, 0x00, 0x00, 0x00, 0x00, 0x00, 0xff, 0xff, 0xff,
    0x21, 0xf9, 0x04, 0x01, 0x00, 0x00, 0x00, 0x00, 0x2c, 0x00, 0x00, 0x00, 0x00, 0x01, 0x00, 0x01, 0x00, 0x00, 0x02,
    0x02, 0x44, 0x01, 0x00, 0x3b,
];

/// Verifies user JWTs. Implemented by `UserVerifier`; tests inject their own.
pub trait VerifyUser: Send + Sync {
    /// Verifies a bearer token.
    fn verify<'a>(
        &'a self,
        token: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<VerifiedUser, AuthError>> + Send + 'a>>;
}

impl<S: JwksSource + 'static> VerifyUser for UserVerifier<S> {
    fn verify<'a>(
        &'a self,
        token: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<VerifiedUser, AuthError>> + Send + 'a>> {
        Box::pin(UserVerifier::verify(self, token))
    }
}

/// Shared state of the HTTP layer.
#[derive(Clone)]
pub struct AppState {
    /// Configuration.
    pub config: Arc<Config>,
    /// User JWT verifier; `None` when `CONVEX_SITE_URL` is unset (every `/v1` call is then 401).
    pub verifier: Option<Arc<dyn VerifyUser>>,
    /// Set once the Convex subscription is live (`/readyz`).
    pub convex_live: Arc<AtomicBool>,
    /// Requests served (`/metrics`).
    pub requests: Arc<AtomicU64>,
    /// Process start.
    pub started: Instant,
    /// Convex, actors and keys; `None` when the worker runs without Convex (routes answer 503).
    pub services: Option<Arc<Services>>,
    /// Worker counters (`/metrics`).
    pub metrics: Arc<Metrics>,
}

impl AppState {
    /// State for a configuration.
    pub fn new(config: Config, verifier: Option<Arc<dyn VerifyUser>>) -> Self {
        Self {
            config: Arc::new(config),
            verifier,
            convex_live: Arc::new(AtomicBool::new(false)),
            requests: Arc::new(AtomicU64::new(0)),
            started: Instant::now(),
            services: None,
            metrics: Arc::new(Metrics::default()),
        }
    }

    /// With services (production and handler tests).
    pub fn with_services(mut self, services: Arc<Services>) -> Self {
        self.metrics = services.metrics.clone();
        self.services = Some(services);
        self
    }
}

/// An API error rendered as the §15 envelope.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApiError(pub ErrorPayload);

impl ApiError {
    /// An error with the code's default message.
    pub fn new(code: ErrorCode) -> Self {
        Self(ErrorPayload::new(code))
    }

    /// An error with a specific safe message.
    pub fn with_message(code: ErrorCode, message: &str) -> Self {
        Self(ErrorPayload::new(code).with_message(message))
    }

    /// Adds `retryAfterMs` (and the `Retry-After` header).
    pub fn retry_after(self, ms: u64) -> Self {
        Self(self.0.with_retry_after_ms(ms))
    }
}

impl From<ErrorCode> for ApiError {
    fn from(code: ErrorCode) -> Self {
        Self::new(code)
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let status = StatusCode::from_u16(self.0.code.http_status()).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
        let retry = self.0.retry_after_ms;
        let mut resp = (status, Json(ErrorBody { error: self.0 })).into_response();
        if let Some(ms) = retry
            && let Ok(v) = HeaderValue::from_str(&ms.div_ceil(1000).to_string())
        {
            resp.headers_mut().insert(header::RETRY_AFTER, v);
        }
        resp
    }
}

/// JSON body extractor whose rejection is the §15 envelope (`400 invalid_argument`, `413`).
pub struct ApiJson<T>(pub T);

impl<S: Send + Sync, T: DeserializeOwned> FromRequest<S> for ApiJson<T> {
    type Rejection = ApiError;

    async fn from_request(req: Request, state: &S) -> Result<Self, Self::Rejection> {
        match Json::<T>::from_request(req, state).await {
            Ok(Json(v)) => Ok(ApiJson(v)),
            Err(JsonRejection::BytesRejection(_)) => Err(ApiError::new(ErrorCode::PayloadTooLarge)),
            Err(JsonRejection::MissingJsonContentType(_)) => {
                Err(ApiError::with_message(ErrorCode::InvalidArgument, "Send the body as application/json."))
            }
            Err(_) => Err(ApiError::with_message(ErrorCode::InvalidArgument, "The request body is not valid.")),
        }
    }
}

/// `hex(sha256(userId))[..12]`, the only user identifier that may appear in logs.
pub fn log_user_hash(user_id: &str) -> String {
    Sha256::digest(user_id.as_bytes()).iter().take(6).map(|b| format!("{b:02x}")).collect()
}

/// The verified caller, inserted by [`require_user`].
#[derive(Clone, Debug)]
pub struct CurrentUser(pub VerifiedUser);

/// The caller's own JWT, for client functions the worker calls as the user (D-235). Never logged.
#[derive(Clone)]
pub struct UserToken(pub String);

impl std::fmt::Debug for UserToken {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("UserToken(<redacted>)")
    }
}

#[derive(Clone, Debug)]
struct LogUser(String);

/// Requires `Authorization: Bearer <user JWT>` (§6.5).
async fn require_user(State(state): State<AppState>, mut req: Request, next: Next) -> Response {
    let token = req
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(bearer_token)
        .map(str::to_owned);
    let Some(token) = token else {
        return ApiError::new(ErrorCode::Unauthenticated).into_response();
    };
    let Some(verifier) = state.verifier.clone() else {
        return ApiError::with_message(ErrorCode::Unauthenticated, "Sign-in is not configured on this server.")
            .into_response();
    };
    match verifier.verify(&token).await {
        Ok(user) => {
            let hash = log_user_hash(&user.user_id);
            req.extensions_mut().insert(CurrentUser(user));
            req.extensions_mut().insert(UserToken(token));
            let mut resp = next.run(req).await;
            resp.extensions_mut().insert(LogUser(hash));
            resp
        }
        Err(AuthError::JwksUnavailable(_)) => {
            ApiError::with_message(ErrorCode::Internal, "Sign-in could not be checked. Try again.").into_response()
        }
        Err(_) => ApiError::new(ErrorCode::Unauthenticated).into_response(),
    }
}

/// Logs method, route template, status, latency and the user hash. Never bodies, query strings,
/// addresses, subjects or tokens (§7.1).
async fn log_requests(State(state): State<AppState>, req: Request, next: Next) -> Response {
    let method = req.method().clone();
    let route =
        req.extensions().get::<MatchedPath>().map(|p| p.as_str().to_owned()).unwrap_or_else(|| "<unmatched>".into());
    let start = Instant::now();
    let resp = next.run(req).await;
    state.requests.fetch_add(1, Ordering::Relaxed);
    let user = resp.extensions().get::<LogUser>().map(|u| u.0.as_str()).unwrap_or("-");
    tracing::info!(
        method = %method,
        route = %route,
        status = resp.status().as_u16(),
        latency_ms = start.elapsed().as_millis() as u64,
        user = %user,
        "request"
    );
    resp
}

/// `returnTo` allow-list (§6.7): `https://app.akatsuki.vc/*`, `akatsuki://*`, `http://127.0.0.1:*/*`,
/// plus the configured CORS (dev) origins.
pub fn return_to_allowed(return_to: &str, extra_origins: &[String]) -> bool {
    let Ok(url) = url::Url::parse(return_to) else { return false };
    if !url.username().is_empty() || url.password().is_some() {
        return false;
    }
    match url.scheme() {
        "akatsuki" => true,
        "https" if url.host_str() == Some("app.akatsuki.vc") && url.port().is_none() => true,
        "http" if url.host_str() == Some("127.0.0.1") && url.port().is_some() => true,
        _ => {
            let origin = url.origin().ascii_serialization();
            extra_origins.contains(&origin)
        }
    }
}

async fn healthz() -> Json<Health> {
    Json(Health { ok: true })
}

async fn readyz(State(state): State<AppState>) -> Response {
    if state.convex_live.load(Ordering::Relaxed) {
        (StatusCode::OK, Json(Health { ok: true })).into_response()
    } else {
        (StatusCode::SERVICE_UNAVAILABLE, Json(Health { ok: false })).into_response()
    }
}

async fn metrics(State(state): State<AppState>, headers: HeaderMap) -> Response {
    // Internal network only: anything that came through the host proxy carries forwarding headers.
    if ["x-forwarded-for", "x-real-ip", "forwarded"].iter().any(|h| headers.contains_key(*h)) {
        return ApiError::new(ErrorCode::NotFound).into_response();
    }
    let mut body = format!(
        "# HELP akatsuki_sync_up Whether the worker is running.\n# TYPE akatsuki_sync_up gauge\nakatsuki_sync_up 1\n\
         # HELP akatsuki_sync_convex_live Whether the Convex subscription is live.\n# TYPE akatsuki_sync_convex_live gauge\nakatsuki_sync_convex_live {}\n\
         # HELP akatsuki_sync_requests_total HTTP requests served.\n# TYPE akatsuki_sync_requests_total counter\nakatsuki_sync_requests_total {}\n\
         # HELP akatsuki_sync_uptime_seconds Seconds since start.\n# TYPE akatsuki_sync_uptime_seconds gauge\nakatsuki_sync_uptime_seconds {}\n",
        u8::from(state.convex_live.load(Ordering::Relaxed)),
        state.requests.load(Ordering::Relaxed),
        state.started.elapsed().as_secs(),
    );
    body.push_str(&state.metrics.render());
    let mut resp = (StatusCode::OK, body).into_response();
    resp.headers_mut().insert(header::CONTENT_TYPE, HeaderValue::from_static("text/plain; version=0.0.4"));
    resp
}

async fn not_found() -> ApiError {
    ApiError::new(ErrorCode::NotFound)
}

fn panic_response(_: Box<dyn std::any::Any + Send + 'static>) -> Response<Body> {
    tracing::error!("handler panicked");
    ApiError::new(ErrorCode::Internal).into_response()
}

fn cors(config: &Config) -> CorsLayer {
    let origins: Vec<HeaderValue> = config.cors_origins.iter().filter_map(|o| HeaderValue::from_str(o).ok()).collect();
    CorsLayer::new()
        .allow_origin(AllowOrigin::list(origins))
        .allow_methods([Method::GET, Method::POST, Method::PUT, Method::DELETE])
        .allow_headers([
            header::AUTHORIZATION,
            header::CONTENT_TYPE,
            HeaderName::from_static("x-filename"),
            header::IF_NONE_MATCH,
        ])
        .allow_credentials(false)
        .max_age(Duration::from_secs(600))
}

/// Builds the router.
pub fn router(state: AppState) -> Router {
    let authed = Router::new()
        .route("/v1/connect/{provider}/start", post(api::connect::start))
        .route("/v1/connect/imap", post(api::connect::imap))
        .route("/v1/connect/complete", post(api::connect::complete))
        .route("/v1/connect/imap/discover", get(api::connect::discover))
        .route("/v1/accounts/{account_id}/sync", post(api::search::account_sync))
        .route("/v1/messages/{message_id}", get(api::bodies::message))
        .route("/v1/threads/{thread_id}/bodies", get(api::bodies::thread))
        .route("/v1/prefetch", post(api::bodies::prefetch))
        .route("/v1/messages/{message_id}/parts/{part_id}", get(api::bodies::part))
        .route("/v1/links", post(api::bodies::create_link))
        .route("/v1/uploads", post(api::compose::upload).layer(DefaultBodyLimit::max(UPLOAD_LIMIT)))
        .route("/v1/drafts/{client_draft_id}", put(api::compose::put).delete(api::compose::delete))
        .route("/v1/drafts/{client_draft_id}/adopt", post(api::compose::adopt))
        .route("/v1/send", post(api::compose::send))
        .route("/v1/search", get(api::search::search))
        .route_layer(middleware::from_fn_with_state(state.clone(), require_user));

    let no_store =
        SetResponseHeaderLayer::if_not_present(header::CACHE_CONTROL, HeaderValue::from_static("private, no-store"));

    let public = Router::new()
        .route("/healthz", get(healthz))
        .route("/readyz", get(readyz))
        .route("/oauth/{provider}/callback", get(api::connect::callback))
        .route("/v1/l/{token}", get(api::bodies::capability_link))
        .route("/v1/img/{u}/{sig}/{b64url}", get(api::media::image))
        .route("/hooks/gmail", post(api::hooks::gmail))
        .route("/hooks/graph", post(api::hooks::graph))
        .route("/hooks/graph/lifecycle", post(api::hooks::graph_lifecycle))
        .route("/r/{file}", get(api::media::read_receipt))
        .route("/metrics", get(metrics));

    authed
        .merge(public)
        .fallback(not_found)
        .route_layer(middleware::from_fn_with_state(state.clone(), log_requests))
        .layer(no_store)
        .layer(SetResponseHeaderLayer::overriding(header::X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff")))
        .layer(DefaultBodyLimit::max(JSON_BODY_LIMIT))
        .layer(cors(&state.config))
        .layer(CatchPanicLayer::custom(panic_response))
        .with_state(state)
}
