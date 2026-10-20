//! The Graph HTTP core: bearer auth with one retry after a 401, `Prefer: IdType="ImmutableId"` on
//! every request, the §9.4 budget (4 concurrent requests and 16 requests per second per
//! mailbox), short `Retry-After` waits handled inline, JSON `$batch` in groups of 20, and the
//! status → [`ProviderError`] table of §5.5.
//!
//! Errors never carry response text: only the HTTP status and Graph's `error.code`, which are
//! fixed identifiers such as `ErrorItemNotFound` (ARCHITECTURE §14.2, logs carry no content).

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use bytes::Bytes;
use futures::StreamExt;
use reqwest::Method;
use reqwest::header::{CONTENT_TYPE, HeaderMap, RETRY_AFTER};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::sync::Semaphore;

use crate::{ByteStream, LimitScope, ProviderError, TokenSource};

/// Sent with every request so message ids survive moves (§1.2, §9.5).
pub(crate) const PREFER_IMMUTABLE: &str = "IdType=\"ImmutableId\"";
/// Graph's `$batch` limit (§5.4).
pub(crate) const BATCH_MAX: usize = 20;
/// How many times a throttled request is retried inline before the error is surfaced.
const MAX_INLINE_RETRIES: u32 = 2;

/// Request body.
#[derive(Clone, Debug)]
pub(crate) enum Body {
    /// No body.
    Empty,
    /// `application/json`.
    Json(Value),
    /// Raw bytes with a content type (MIME drafts are `text/plain` base64).
    Raw { content_type: &'static str, bytes: Bytes },
}

/// A successful response.
#[derive(Debug)]
pub(crate) struct Resp {
    pub body: Bytes,
}

impl Resp {
    pub fn json<T: DeserializeOwned>(&self) -> Result<T, ProviderError> {
        serde_json::from_slice(&self.body).map_err(|_| ProviderError::Transient("graph: malformed JSON".into(), None))
    }
}

/// One `$batch` sub-request. `url` is relative to the API version (`/me/messages/{id}`).
#[derive(Clone, Debug)]
pub(crate) struct Sub {
    pub method: Method,
    pub url: String,
    pub body: Option<Value>,
    /// Extra `Prefer` values (joined after the immutable-id preference).
    pub prefer: Option<String>,
}

impl Sub {
    pub fn get(url: String) -> Self {
        Self { method: Method::GET, url, body: None, prefer: None }
    }
    pub fn with_prefer(mut self, prefer: &str) -> Self {
        self.prefer = Some(prefer.to_owned());
        self
    }
    pub fn post(url: String, body: Value) -> Self {
        Self { method: Method::POST, url, body: Some(body), prefer: None }
    }
    pub fn patch(url: String, body: Value) -> Self {
        Self { method: Method::PATCH, url, body: Some(body), prefer: None }
    }
    pub fn delete(url: String) -> Self {
        Self { method: Method::DELETE, url, body: None, prefer: None }
    }
}

/// One `$batch` sub-response.
#[derive(Clone, Debug)]
pub(crate) struct SubResp {
    pub status: u16,
    pub retry_after: Option<Duration>,
    pub body: Value,
}

impl SubResp {
    pub fn ok(&self) -> bool {
        (200..300).contains(&self.status)
    }

    /// The error for a non-2xx sub-response.
    pub fn error(&self) -> ProviderError {
        map_status(self.status, self.retry_after, &serde_json::to_vec(&self.body).unwrap_or_default())
    }

    /// `Ok(body)` for 2xx, the mapped error otherwise.
    pub fn into_result(self) -> Result<Value, ProviderError> {
        if self.ok() { Ok(self.body) } else { Err(self.error()) }
    }
}

#[derive(Serialize)]
struct BatchRequest<'a> {
    id: String,
    method: &'a str,
    url: &'a str,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    headers: BTreeMap<&'static str, String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    body: Option<&'a Value>,
}

#[derive(Deserialize)]
struct BatchResponse {
    id: String,
    status: u16,
    #[serde(default)]
    headers: BTreeMap<String, String>,
    #[serde(default)]
    body: Option<Value>,
}

#[derive(Deserialize)]
struct BatchEnvelope {
    #[serde(default)]
    responses: Vec<BatchResponse>,
}

/// Throughput limits for one mailbox (§9.4).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Limits {
    /// Concurrent requests (Graph allows 4 per mailbox).
    pub max_concurrent: usize,
    /// Requests per second (a `$batch` counts once per sub-request).
    pub max_rps: f64,
    /// A throttled request whose `Retry-After` is at most this long is retried inline; longer
    /// waits surface as [`ProviderError::RateLimited`] so the account actor pauses.
    pub max_inline_wait: Duration,
}

impl Default for Limits {
    fn default() -> Self {
        Self { max_concurrent: 4, max_rps: 16.0, max_inline_wait: Duration::from_secs(5) }
    }
}

struct Bucket {
    tokens: f64,
    last: Instant,
}

/// The HTTP core shared by every Graph call of one account.
pub(crate) struct GraphHttp {
    pub http: reqwest::Client,
    base: String,
    tokens: Arc<dyn TokenSource>,
    sem: Semaphore,
    bucket: std::sync::Mutex<Bucket>,
    limits: Limits,
}

impl std::fmt::Debug for GraphHttp {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GraphHttp").field("base", &self.base).field("limits", &self.limits).finish_non_exhaustive()
    }
}

/// `Retry-After` in seconds (the form Graph uses).
pub(crate) fn retry_after(headers: &HeaderMap) -> Option<Duration> {
    headers.get(RETRY_AFTER).and_then(|v| v.to_str().ok()).and_then(parse_retry_after)
}

fn parse_retry_after(v: &str) -> Option<Duration> {
    v.trim().parse::<u64>().ok().map(Duration::from_secs)
}

/// Graph's `error.code`, if the body is an OData error.
fn error_code(body: &[u8]) -> String {
    serde_json::from_slice::<Value>(body)
        .ok()
        .and_then(|v| v.get("error").and_then(|e| e.get("code")).and_then(Value::as_str).map(str::to_owned))
        .filter(|c| c.len() <= 80 && c.chars().all(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '.'))
        .unwrap_or_default()
}

/// The §5.5 classification of a Graph error response.
pub(crate) fn map_status(status: u16, retry_after: Option<Duration>, body: &[u8]) -> ProviderError {
    let code = error_code(body);
    let lc = code.to_ascii_lowercase();
    let label = if code.is_empty() { format!("graph {status}") } else { format!("graph {status} {code}") };
    if lc.contains("syncstate") {
        return ProviderError::CursorExpired;
    }
    if lc.contains("quota") || lc.contains("mailboxfull") || lc.contains("submissionquota") {
        return ProviderError::Quota(label);
    }
    match status {
        400 | 405 | 406 | 411 | 412 | 413 | 415 | 416 | 422 => ProviderError::Invalid(label),
        401 => ProviderError::Auth { reauth_required: true },
        403 if matches!(lc.as_str(), "erroraccessdenied" | "accessdenied" | "authorization_requestdenied") => {
            ProviderError::Auth { reauth_required: true }
        }
        403 => ProviderError::Invalid(label),
        404 => ProviderError::NotFound,
        409 => ProviderError::Conflict(label),
        410 => ProviderError::CursorExpired,
        429 => ProviderError::RateLimited { retry_after, scope: LimitScope::Account },
        503 if retry_after.is_some() => ProviderError::RateLimited { retry_after, scope: LimitScope::Account },
        507 => ProviderError::Quota(label),
        500..=599 => ProviderError::Transient(label, retry_after),
        _ => ProviderError::Invalid(label),
    }
}

/// A transport failure. The reqwest error text may contain the URL, so it is never surfaced.
pub(crate) fn transport(e: &reqwest::Error) -> ProviderError {
    let what = if e.is_timeout() {
        "graph: timeout"
    } else if e.is_connect() {
        "graph: connect failed"
    } else {
        "graph: connection error"
    };
    ProviderError::Transient(what.into(), None)
}

/// Percent-encodes one query value or path segment.
pub(crate) fn enc(v: &str) -> String {
    url::form_urlencoded::byte_serialize(v.as_bytes()).collect()
}

/// `k=v&k=v` with OData keys kept literal (`$select`) and values percent-encoded.
pub(crate) fn query(pairs: &[(&str, &str)]) -> String {
    pairs.iter().map(|(k, v)| format!("{k}={}", enc(v))).collect::<Vec<_>>().join("&")
}

/// An OData string literal (`'it''s'`).
pub(crate) fn odata_str(v: &str) -> String {
    format!("'{}'", v.replace('\'', "''"))
}

impl GraphHttp {
    pub fn new(http: reqwest::Client, base: &str, tokens: Arc<dyn TokenSource>, limits: Limits) -> Self {
        let limits = Limits {
            max_concurrent: limits.max_concurrent.max(1),
            max_rps: if limits.max_rps > 0.0 { limits.max_rps } else { 16.0 },
            max_inline_wait: limits.max_inline_wait,
        };
        Self {
            http,
            base: base.trim_end_matches('/').to_owned(),
            tokens,
            sem: Semaphore::new(limits.max_concurrent),
            bucket: std::sync::Mutex::new(Bucket { tokens: limits.max_rps, last: Instant::now() }),
            limits,
        }
    }

    /// The API base (`https://graph.microsoft.com/v1.0`).
    pub fn base(&self) -> &str {
        &self.base
    }

    /// Absolute URL of a relative path (`/me/messages`).
    pub fn url(&self, rel: &str) -> String {
        format!("{}{rel}", self.base)
    }

    /// Paging and delta links come back from Graph and are stored in Convex. They are only ever
    /// followed when they point at the configured API base, so a tampered cursor cannot send the
    /// bearer token anywhere else.
    pub fn check_link(&self, link: &str) -> Result<(), ProviderError> {
        let ok = link.strip_prefix(&self.base).is_some_and(|rest| rest.starts_with('/'));
        if ok { Ok(()) } else { Err(ProviderError::Invalid("graph: link outside the API base".into())) }
    }

    /// The batch-relative form of an absolute link.
    pub fn relative(&self, link: &str) -> Result<String, ProviderError> {
        self.check_link(link)?;
        Ok(link[self.base.len()..].to_owned())
    }

    /// Waits for `cost` request tokens (token bucket, refilled at `max_rps`).
    pub async fn throttle(&self, cost: usize) {
        let need = (cost.max(1) as f64).min(self.limits.max_rps);
        loop {
            let wait = {
                let mut b = self.bucket.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
                let now = Instant::now();
                let refill = now.duration_since(b.last).as_secs_f64() * self.limits.max_rps;
                b.tokens = (b.tokens + refill).min(self.limits.max_rps);
                b.last = now;
                if b.tokens >= need {
                    b.tokens -= need;
                    return;
                }
                Duration::from_secs_f64((need - b.tokens) / self.limits.max_rps)
            };
            tokio::time::sleep(wait).await;
        }
    }

    /// Sends one request with auth, budget and inline retries. Returns the 2xx response.
    async fn execute(
        &self,
        method: &Method,
        url: &str,
        prefer: Option<&str>,
        body: &Body,
        cost: usize,
    ) -> Result<reqwest::Response, ProviderError> {
        let mut auth_retried = false;
        let mut waits = 0;
        loop {
            self.throttle(cost).await;
            let permit =
                self.sem.acquire().await.map_err(|_| ProviderError::Transient("graph: closed".into(), None))?;
            let token = self.tokens.access_token().await?;
            let prefer_value = match prefer {
                Some(p) => format!("{PREFER_IMMUTABLE}, {p}"),
                None => PREFER_IMMUTABLE.to_owned(),
            };
            let mut rb =
                self.http.request(method.clone(), url).bearer_auth(&token.token).header("Prefer", prefer_value);
            rb = match body {
                Body::Empty => rb,
                Body::Json(v) => {
                    rb.header(CONTENT_TYPE, "application/json").body(serde_json::to_vec(v).unwrap_or_default())
                }
                Body::Raw { content_type, bytes } => rb.header(CONTENT_TYPE, *content_type).body(bytes.clone()),
            };
            let resp = rb.send().await.map_err(|e| transport(&e))?;
            let status = resp.status().as_u16();
            if (200..300).contains(&status) {
                drop(permit);
                return Ok(resp);
            }
            let ra = retry_after(resp.headers());
            let body_bytes = resp.bytes().await.unwrap_or_default();
            drop(permit);
            if status == 401 && !auth_retried {
                auth_retried = true;
                self.tokens.invalidate().await;
                continue;
            }
            if matches!(status, 429 | 503)
                && waits < MAX_INLINE_RETRIES
                && let Some(wait) = ra.filter(|w| *w <= self.limits.max_inline_wait)
            {
                waits += 1;
                tokio::time::sleep(wait).await;
                continue;
            }
            return Err(map_status(status, ra, &body_bytes));
        }
    }

    /// A request whose response body is read whole.
    pub async fn send(
        &self,
        method: Method,
        url: &str,
        prefer: Option<&str>,
        body: Body,
    ) -> Result<Resp, ProviderError> {
        let resp = self.execute(&method, url, prefer, &body, 1).await?;
        let body = resp.bytes().await.map_err(|e| transport(&e))?;
        Ok(Resp { body })
    }

    /// `GET` + JSON.
    pub async fn get_json<T: DeserializeOwned>(&self, url: &str, prefer: Option<&str>) -> Result<T, ProviderError> {
        self.send(Method::GET, url, prefer, Body::Empty).await?.json()
    }

    /// `GET` streamed (message source, attachment bytes).
    pub async fn stream(&self, url: &str) -> Result<ByteStream, ProviderError> {
        let resp = self.execute(&Method::GET, url, None, &Body::Empty, 1).await?;
        Ok(resp
            .bytes_stream()
            .map(|r| r.map_err(|_| ProviderError::Transient("graph: stream interrupted".into(), None)))
            .boxed())
    }

    /// Runs sub-requests through `$batch` in groups of 20 and returns one response per input, in
    /// order. Throttled sub-requests with a short `Retry-After` are re-sent (at most twice);
    /// everything else is returned for the caller to classify. An error of the outer request
    /// (auth, throttling of the whole batch, transport) fails the call.
    pub async fn batch(&self, subs: &[Sub]) -> Result<Vec<SubResp>, ProviderError> {
        let mut out: Vec<Option<SubResp>> = vec![None; subs.len()];
        let mut pending: Vec<usize> = (0..subs.len()).collect();
        let mut round = 0;
        while !pending.is_empty() {
            for chunk in pending.chunks(BATCH_MAX) {
                let results = self.batch_once(subs, chunk).await?;
                for (i, r) in chunk.iter().zip(results) {
                    out[*i] = Some(r);
                }
            }
            if round >= MAX_INLINE_RETRIES {
                break;
            }
            round += 1;
            let retry: Vec<usize> = pending
                .iter()
                .copied()
                .filter(|i| {
                    out[*i].as_ref().is_some_and(|r| {
                        matches!(r.status, 429 | 503) && r.retry_after.is_some_and(|w| w <= self.limits.max_inline_wait)
                    })
                })
                .collect();
            if let Some(wait) = retry.iter().filter_map(|i| out[*i].as_ref().and_then(|r| r.retry_after)).max() {
                tokio::time::sleep(wait).await;
            }
            pending = retry;
        }
        Ok(out
            .into_iter()
            .map(|r| r.unwrap_or(SubResp { status: 500, retry_after: None, body: Value::Null }))
            .collect())
    }

    async fn batch_once(&self, subs: &[Sub], idx: &[usize]) -> Result<Vec<SubResp>, ProviderError> {
        let requests: Vec<BatchRequest<'_>> = idx
            .iter()
            .map(|&i| {
                let s = &subs[i];
                let mut headers = BTreeMap::new();
                headers.insert(
                    "Prefer",
                    match &s.prefer {
                        Some(p) => format!("{PREFER_IMMUTABLE}, {p}"),
                        None => PREFER_IMMUTABLE.to_owned(),
                    },
                );
                if s.body.is_some() {
                    headers.insert("Content-Type", "application/json".to_owned());
                }
                BatchRequest {
                    id: i.to_string(),
                    method: s.method.as_str(),
                    url: &s.url,
                    headers,
                    body: s.body.as_ref(),
                }
            })
            .collect();
        let body = serde_json::json!({ "requests": requests });
        let resp = self.execute(&Method::POST, &self.url("/$batch"), None, &Body::Json(body), idx.len()).await?;
        let bytes = resp.bytes().await.map_err(|e| transport(&e))?;
        let env: BatchEnvelope = serde_json::from_slice(&bytes)
            .map_err(|_| ProviderError::Transient("graph: malformed batch".into(), None))?;
        let mut by_id: BTreeMap<String, BatchResponse> = env.responses.into_iter().map(|r| (r.id.clone(), r)).collect();
        Ok(idx
            .iter()
            .map(|i| match by_id.remove(&i.to_string()) {
                Some(r) => {
                    let ra = r
                        .headers
                        .iter()
                        .find(|(k, _)| k.eq_ignore_ascii_case("retry-after"))
                        .and_then(|(_, v)| parse_retry_after(v));
                    SubResp { status: r.status, retry_after: ra, body: r.body.unwrap_or(Value::Null) }
                }
                None => SubResp { status: 502, retry_after: None, body: Value::Null },
            })
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_table() {
        let b = |code: &str| {
            serde_json::to_vec(&serde_json::json!({"error": {"code": code, "message": "secret subject"}})).unwrap()
        };
        assert_eq!(map_status(404, None, &b("ErrorItemNotFound")), ProviderError::NotFound);
        assert_eq!(map_status(410, None, &[]), ProviderError::CursorExpired);
        assert_eq!(map_status(400, None, &b("SyncStateNotFound")), ProviderError::CursorExpired);
        assert_eq!(map_status(401, None, &[]), ProviderError::Auth { reauth_required: true });
        assert_eq!(map_status(403, None, &b("ErrorAccessDenied")), ProviderError::Auth { reauth_required: true });
        assert!(matches!(map_status(403, None, &b("MailboxNotEnabledForRESTAPI")), ProviderError::Invalid(_)));
        assert!(matches!(map_status(409, None, &b("ErrorFolderExists")), ProviderError::Conflict(_)));
        assert!(matches!(map_status(400, None, &b("ErrorQuotaExceeded")), ProviderError::Quota(_)));
        assert!(matches!(map_status(507, None, &[]), ProviderError::Quota(_)));
        let ra = Some(Duration::from_secs(7));
        assert_eq!(
            map_status(429, ra, &[]),
            ProviderError::RateLimited { retry_after: ra, scope: LimitScope::Account }
        );
        assert_eq!(
            map_status(503, ra, &[]),
            ProviderError::RateLimited { retry_after: ra, scope: LimitScope::Account }
        );
        assert!(matches!(map_status(503, None, &[]), ProviderError::Transient(_, None)));
        assert!(matches!(map_status(504, None, &[]), ProviderError::Transient(_, None)));
        // Error text from Graph never reaches the error value, only the code.
        let e = map_status(400, None, &b("ErrorInvalidRecipients"));
        assert_eq!(e, ProviderError::Invalid("graph 400 ErrorInvalidRecipients".into()));
        // A code that is not an identifier is dropped.
        assert_eq!(map_status(400, None, &b("has spaces <b>")), ProviderError::Invalid("graph 400".into()));
    }

    #[test]
    fn encoding() {
        assert_eq!(
            query(&[("$select", "id,subject"), ("$filter", "a eq 'b+c'")]),
            "$select=id%2Csubject&$filter=a+eq+%27b%2Bc%27"
        );
        assert_eq!(odata_str("it's"), "'it''s'");
        assert_eq!(parse_retry_after(" 12 "), Some(Duration::from_secs(12)));
        assert_eq!(parse_retry_after("Wed, 21 Oct 2026 07:28:00 GMT"), None);
    }
}
