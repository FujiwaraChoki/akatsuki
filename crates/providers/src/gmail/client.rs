//! The Gmail HTTP client: URLs, bearer tokens (one retry after a 401 with a fresh token), quota
//! charging, HTTP batch, and the classification of Gmail errors into [`ProviderError`] (§5.5).

use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use reqwest::header::{AUTHORIZATION, CONTENT_TYPE, HeaderMap, RETRY_AFTER};
use serde::de::DeserializeOwned;

use super::api::ErrorEnvelope;
use super::auth::transport_error;
use super::batch::{self, BatchItem};
use super::quota::{AccountQuota, Lane, Method};
use crate::{LimitScope, ProviderError, TokenSource};

/// A request body.
#[derive(Clone, Debug)]
pub(crate) enum Body {
    /// No body.
    None,
    /// JSON.
    Json(serde_json::Value),
    /// Raw bytes with a content type (media and multipart uploads).
    Bytes {
        /// `Content-Type`.
        content_type: String,
        /// Bytes.
        bytes: Bytes,
    },
}

/// One API call.
#[derive(Clone, Debug)]
pub(crate) struct Call {
    pub method: reqwest::Method,
    /// Path segments after `users/{userId}/`.
    pub segs: Vec<String>,
    pub query: Vec<(&'static str, String)>,
    pub body: Body,
    pub cost: Method,
    pub lane: Lane,
    /// Use the `/upload/gmail/v1` prefix.
    pub upload: bool,
}

impl Call {
    pub fn new(method: reqwest::Method, segs: &[&str], cost: Method, lane: Lane) -> Self {
        Self {
            method,
            segs: segs.iter().map(|s| (*s).to_owned()).collect(),
            query: Vec::new(),
            body: Body::None,
            cost,
            lane,
            upload: false,
        }
    }
    pub fn get(segs: &[&str], cost: Method, lane: Lane) -> Self {
        Self::new(reqwest::Method::GET, segs, cost, lane)
    }
    pub fn post(segs: &[&str], cost: Method, lane: Lane) -> Self {
        Self::new(reqwest::Method::POST, segs, cost, lane)
    }
    pub fn q(mut self, k: &'static str, v: impl Into<String>) -> Self {
        self.query.push((k, v.into()));
        self
    }
    pub fn q_opt(self, k: &'static str, v: Option<impl Into<String>>) -> Self {
        match v {
            Some(v) => self.q(k, v),
            None => self,
        }
    }
    pub fn json(mut self, v: serde_json::Value) -> Self {
        self.body = Body::Json(v);
        self
    }
}

/// The `Retry-After` header in seconds.
pub(crate) fn retry_after(h: &HeaderMap) -> Option<Duration> {
    h.get(RETRY_AFTER)?.to_str().ok()?.trim().parse::<u64>().ok().map(Duration::from_secs)
}

fn short(s: &str) -> String {
    let s: String = s.chars().filter(|c| !c.is_control()).take(200).collect();
    s
}

/// Classifies a non-2xx Gmail response.
pub(crate) fn classify(status: u16, headers: &HeaderMap, body: &[u8]) -> ProviderError {
    let env: ErrorEnvelope = serde_json::from_slice(body).unwrap_or_default();
    let reason = env.error.errors.first().map(|e| e.reason.clone()).unwrap_or_default();
    let message = short(&env.error.message);
    let detail = if reason.is_empty() {
        format!("gmail {status}: {message}")
    } else {
        format!("gmail {status} {reason}: {message}")
    };
    let ra = retry_after(headers);
    let rate_scope = || {
        let m = message.to_ascii_lowercase();
        if m.contains("per user") || reason == "userRateLimitExceeded" {
            LimitScope::Account
        } else if m.contains("project") || m.contains("consumer") {
            LimitScope::Project
        } else {
            LimitScope::Account
        }
    };
    match status {
        401 => ProviderError::Auth { reauth_required: true },
        403 => match reason.as_str() {
            "rateLimitExceeded" | "userRateLimitExceeded" => {
                ProviderError::RateLimited { retry_after: ra, scope: rate_scope() }
            }
            "dailyLimitExceeded" | "quotaExceeded" => ProviderError::Quota(detail),
            "insufficientPermissions" | "authError" => ProviderError::Auth { reauth_required: true },
            _ if env.error.message.contains("insufficient authentication scopes")
                || env.error.message.contains("Delegation denied") =>
            {
                ProviderError::Auth { reauth_required: true }
            }
            _ => ProviderError::Invalid(detail),
        },
        404 => ProviderError::NotFound,
        409 => ProviderError::Conflict(detail),
        429 => ProviderError::RateLimited { retry_after: ra, scope: rate_scope() },
        500..=599 => ProviderError::Transient(detail, ra),
        _ => ProviderError::Invalid(detail),
    }
}

/// The Gmail client for one account.
pub(crate) struct Client {
    pub http: reqwest::Client,
    /// `https://gmail.googleapis.com` (no trailing slash).
    pub base: String,
    /// `me`, or the mailbox address.
    pub user: String,
    pub tokens: Arc<dyn TokenSource>,
    pub quota: Arc<AccountQuota>,
}

impl Client {
    fn url(&self, call: &Call) -> Result<url::Url, ProviderError> {
        let mut url = url::Url::parse(&self.base).map_err(|_| ProviderError::Invalid("bad Gmail API base".into()))?;
        {
            let mut p = url.path_segments_mut().map_err(|()| ProviderError::Invalid("bad Gmail API base".into()))?;
            p.pop_if_empty();
            if call.upload {
                p.push("upload");
            }
            p.extend(["gmail", "v1", "users", self.user.as_str()]);
            p.extend(call.segs.iter());
        }
        if !call.query.is_empty() {
            let mut q = url.query_pairs_mut();
            for (k, v) in &call.query {
                q.append_pair(k, v);
            }
        }
        Ok(url)
    }

    /// The path-and-query of a call, for a batch part.
    fn batch_path(&self, call: &Call) -> Result<String, ProviderError> {
        let url = self.url(call)?;
        Ok(match url.query() {
            Some(q) => format!("{}?{q}", url.path()),
            None => url.path().to_owned(),
        })
    }

    async fn send_once(
        &self,
        method: &reqwest::Method,
        url: &url::Url,
        body: &Body,
    ) -> Result<(u16, HeaderMap, Bytes), ProviderError> {
        let token = self.tokens.access_token().await?;
        let mut req =
            self.http.request(method.clone(), url.clone()).header(AUTHORIZATION, format!("Bearer {}", token.token));
        req = match body {
            Body::None => {
                if *method == reqwest::Method::POST {
                    req.header(reqwest::header::CONTENT_LENGTH, "0")
                } else {
                    req
                }
            }
            Body::Json(v) => req.header(CONTENT_TYPE, "application/json").body(v.to_string()),
            Body::Bytes { content_type, bytes } => req.header(CONTENT_TYPE, content_type.as_str()).body(bytes.clone()),
        };
        let resp = req.send().await.map_err(|e| transport_error(&e))?;
        let status = resp.status().as_u16();
        let headers = resp.headers().clone();
        let bytes = resp.bytes().await.map_err(|e| transport_error(&e))?;
        Ok((status, headers, bytes))
    }

    /// Sends with one retry after a 401 (fresh token), charging quota once.
    async fn send_raw(
        &self,
        method: &reqwest::Method,
        url: &url::Url,
        body: &Body,
    ) -> Result<(u16, HeaderMap, Bytes), ProviderError> {
        let (status, headers, bytes) = self.send_once(method, url, body).await?;
        if status != 401 {
            return Ok((status, headers, bytes));
        }
        self.tokens.invalidate().await;
        self.send_once(method, url, body).await
    }

    async fn checked(&self, status: u16, headers: &HeaderMap, bytes: &Bytes) -> Result<(), ProviderError> {
        if (200..300).contains(&status) {
            return Ok(());
        }
        let err = classify(status, headers, bytes);
        if matches!(err, ProviderError::RateLimited { scope: LimitScope::Account, .. }) {
            self.quota.penalize().await;
        }
        Err(err)
    }

    /// Runs a call and returns the body bytes.
    pub async fn bytes(&self, call: &Call) -> Result<Bytes, ProviderError> {
        self.quota.charge_calls(call.lane, call.cost, 1).await;
        let url = self.url(call)?;
        let (status, headers, bytes) = self.send_raw(&call.method, &url, &call.body).await?;
        self.checked(status, &headers, &bytes).await?;
        Ok(bytes)
    }

    /// Runs a call and parses the JSON response.
    pub async fn json<T: DeserializeOwned>(&self, call: &Call) -> Result<T, ProviderError> {
        let bytes = self.bytes(call).await?;
        let body: &[u8] = if bytes.is_empty() { b"{}" } else { &bytes };
        serde_json::from_slice(body).map_err(|e| ProviderError::Transient(format!("gmail response: {e}"), None))
    }

    /// Runs a call whose response body is ignored.
    pub async fn empty(&self, call: &Call) -> Result<(), ProviderError> {
        self.bytes(call).await.map(|_| ())
    }

    /// Runs GET calls as one HTTP batch. One result per call, in order; a 404 is `NotFound`.
    pub async fn batch_get(&self, calls: &[Call]) -> Result<Vec<Result<Bytes, ProviderError>>, ProviderError> {
        if calls.is_empty() {
            return Ok(Vec::new());
        }
        if calls.len() == 1 {
            return Ok(vec![self.bytes(&calls[0]).await]);
        }
        for c in calls {
            self.quota.charge_calls(c.lane, c.cost, 1).await;
        }
        let items: Vec<BatchItem> =
            calls.iter().map(|c| self.batch_path(c).map(|path| BatchItem { path })).collect::<Result<_, _>>()?;
        let boundary = format!("akatsuki_{}", uuid::Uuid::new_v4().simple());
        let body = Body::Bytes {
            content_type: format!("multipart/mixed; boundary={boundary}"),
            bytes: Bytes::from(batch::encode(&items, &boundary)),
        };
        let mut url = url::Url::parse(&self.base).map_err(|_| ProviderError::Invalid("bad Gmail API base".into()))?;
        url.path_segments_mut()
            .map_err(|()| ProviderError::Invalid("bad Gmail API base".into()))?
            .pop_if_empty()
            .extend(["batch", "gmail", "v1"]);
        let (status, headers, bytes) = self.send_raw(&reqwest::Method::POST, &url, &body).await?;
        self.checked(status, &headers, &bytes).await?;
        let ct = headers.get(CONTENT_TYPE).and_then(|v| v.to_str().ok()).unwrap_or_default();
        let resp_boundary = batch::boundary_of(ct)
            .ok_or_else(|| ProviderError::Transient("batch response without boundary".into(), None))?;
        let parts =
            batch::decode(&bytes, &resp_boundary, calls.len()).map_err(|e| ProviderError::Transient(e, None))?;
        let mut out = Vec::with_capacity(parts.len());
        for p in parts {
            if (200..300).contains(&p.status) {
                out.push(Ok(Bytes::from(p.body)));
            } else if p.status == 0 {
                out.push(Err(ProviderError::Transient("batch part missing".into(), None)));
            } else {
                let mut h = HeaderMap::new();
                for (k, v) in &p.headers {
                    if let (Ok(k), Ok(v)) = (
                        reqwest::header::HeaderName::try_from(k.as_str()),
                        reqwest::header::HeaderValue::try_from(v.as_str()),
                    ) {
                        h.insert(k, v);
                    }
                }
                let err = classify(p.status, &h, &p.body);
                if matches!(err, ProviderError::RateLimited { scope: LimitScope::Account, .. }) {
                    self.quota.penalize().await;
                }
                out.push(Err(err));
            }
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn body(code: u16, reason: &str, message: &str) -> Vec<u8> {
        serde_json::json!({"error": {"code": code, "message": message, "errors": [{"reason": reason, "domain": "usageLimits", "message": message}]}})
            .to_string()
            .into_bytes()
    }

    #[test]
    fn classification_table() {
        let h = HeaderMap::new();
        assert_eq!(classify(401, &h, b""), ProviderError::Auth { reauth_required: true });
        assert!(matches!(
            classify(403, &h, &body(403, "userRateLimitExceeded", "User-rate limit exceeded")),
            ProviderError::RateLimited { scope: LimitScope::Account, .. }
        ));
        assert!(matches!(
            classify(
                403,
                &h,
                &body(
                    403,
                    "rateLimitExceeded",
                    "Quota exceeded for quota metric 'Queries' and limit 'Queries per minute' of service 'gmail.googleapis.com' for consumer 'project_number:1'"
                )
            ),
            ProviderError::RateLimited { scope: LimitScope::Project, .. }
        ));
        assert!(matches!(
            classify(
                403,
                &h,
                &body(
                    403,
                    "rateLimitExceeded",
                    "Quota exceeded for quota metric 'Queries' and limit 'Queries per minute per user'"
                )
            ),
            ProviderError::RateLimited { scope: LimitScope::Account, .. }
        ));
        assert!(matches!(
            classify(403, &h, &body(403, "dailyLimitExceeded", "Daily sending quota exceeded")),
            ProviderError::Quota(_)
        ));
        assert!(matches!(
            classify(403, &h, &body(403, "insufficientPermissions", "Request had insufficient authentication scopes.")),
            ProviderError::Auth { reauth_required: true }
        ));
        assert_eq!(
            classify(404, &h, &body(404, "notFound", "Requested entity was not found.")),
            ProviderError::NotFound
        );
        assert!(matches!(
            classify(409, &h, &body(409, "aborted", "Label name exists or conflicts")),
            ProviderError::Conflict(_)
        ));
        let mut h429 = HeaderMap::new();
        h429.insert(RETRY_AFTER, "7".parse().unwrap());
        assert_eq!(
            classify(429, &h429, &body(429, "rateLimitExceeded", "Too many concurrent requests for user")),
            ProviderError::RateLimited { retry_after: Some(Duration::from_secs(7)), scope: LimitScope::Account }
        );
        assert!(matches!(classify(503, &h, b"<html>"), ProviderError::Transient(_, None)));
        assert!(matches!(
            classify(400, &h, &body(400, "failedPrecondition", "Mail service not enabled")),
            ProviderError::Invalid(_)
        ));
    }
}
