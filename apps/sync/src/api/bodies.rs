//! Bodies and attachments (ARCHITECTURE §7.2): fetched from the provider on demand, sanitized
//! with `crates/mime` for web and mobile, raw for desktop (which sanitizes locally), never stored.

use std::sync::Arc;

use akatsuki_core::convex::worker::{ResolveThreadArgs, ResolvedMessage};
use akatsuki_core::ids::{MessageId, ThreadId};
use akatsuki_core::time::now_ms;
use akatsuki_core::wire::ErrorCode;
use akatsuki_core::wire::worker::{
    BodyFormat, BodyQuery, LinkRequest, LinkResponse, MessageBody, PrefetchRequest, RawMessageLine,
};
use akatsuki_mime::{ProxySigner, RemoteImagePolicy};
use axum::body::Body;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::{Extension, Json};
use base64::Engine as _;
use futures::StreamExt as _;

use super::{Services, convex_api_error, services};
use crate::capability::{LINK_TTL_MS, LinkClaims, sign_link, verify_link};
use crate::config::Config;
use crate::http::{ApiError, ApiJson, AppState, CurrentUser};
use crate::metrics::Metrics;

/// NDJSON content type.
pub const NDJSON: &str = "application/x-ndjson";

fn image_policy(config: &Config, user_id: &str) -> RemoteImagePolicy {
    match (&config.img_proxy_key, &config.public_base_url) {
        (Some(key), Some(base)) => RemoteImagePolicy::Proxy(ProxySigner::new(base, &key.0, user_id)),
        // Without a proxy key, remote images are removed rather than exposed.
        _ => RemoteImagePolicy::Drop,
    }
}

async fn sanitized(
    svc: &Services,
    config: &Config,
    user_id: &str,
    msg: &ResolvedMessage,
) -> Result<MessageBody, ApiError> {
    let handle = svc.handle(user_id, &msg.account_id)?;
    let raw = svc.fetch_raw(user_id, &handle, msg).await?;
    let body = akatsuki_mime::message_body_from_raw(
        &raw,
        msg.message_id.clone(),
        &msg.provider_message_id,
        image_policy(config, user_id),
    )
    .map_err(|_| ApiError::with_message(ErrorCode::ProviderInvalid, "This message could not be read."))?;
    Metrics::inc(&svc.metrics.bodies_served);
    Ok(body)
}

fn ndjson_line<T: serde::Serialize>(v: &T) -> Vec<u8> {
    let mut line = serde_json::to_vec(v).unwrap_or_default();
    line.push(b'\n');
    line
}

/// `GET /v1/messages/{messageId}?format=sanitized|raw`.
pub async fn message(
    State(state): State<AppState>,
    Extension(CurrentUser(user)): Extension<CurrentUser>,
    Path(message_id): Path<String>,
    Query(q): Query<BodyQuery>,
) -> Result<Response, ApiError> {
    let svc = services(&state.services)?;
    svc.limits.read(&user.user_id)?;
    svc.entitled(&user.user_id).await?;
    let msg = svc.resolve_message(&user.user_id, &MessageId::new(message_id)).await?;
    match q.format {
        BodyFormat::Sanitized => Ok(Json(sanitized(svc, &state.config, &user.user_id, &msg).await?).into_response()),
        BodyFormat::Raw => {
            svc.limits.egress_open(&user.user_id)?;
            let handle = svc.handle(&user.user_id, &msg.account_id)?;
            let raw = svc.fetch_raw(&user.user_id, &handle, &msg).await?;
            svc.limits.egress(&user.user_id, raw.len() as u64)?;
            Metrics::inc(&svc.metrics.bodies_served);
            let mut resp = (StatusCode::OK, raw).into_response();
            resp.headers_mut().insert(header::CONTENT_TYPE, HeaderValue::from_static("message/rfc822"));
            Ok(resp)
        }
    }
}

fn stream_response(lines: futures::stream::BoxStream<'static, Vec<u8>>) -> Response {
    let body = Body::from_stream(lines.map(Ok::<_, std::io::Error>));
    let mut resp = (StatusCode::OK, body).into_response();
    resp.headers_mut().insert(header::CONTENT_TYPE, HeaderValue::from_static(NDJSON));
    resp
}

/// `GET /v1/threads/{threadId}/bodies`: one `MessageBody` per line, newest first, streamed as
/// fetched. A message that cannot be fetched is skipped.
pub async fn thread(
    State(state): State<AppState>,
    Extension(CurrentUser(user)): Extension<CurrentUser>,
    Path(thread_id): Path<String>,
) -> Result<Response, ApiError> {
    let svc = services(&state.services)?.clone();
    svc.limits.read(&user.user_id)?;
    svc.entitled(&user.user_id).await?;
    let tid = ThreadId::new(thread_id);
    let t = svc
        .api
        .resolve_thread(&ResolveThreadArgs { user_id: user.user_id.clone(), thread_id: tid.clone() })
        .await
        .map_err(|e| convex_api_error(&e))?
        .ok_or_else(|| ApiError::new(ErrorCode::NotFound))?;
    svc.handle(&user.user_id, &t.account_id)?;
    let mut msgs = t.messages.clone();
    msgs.sort_by_key(|m| std::cmp::Reverse(m.date));
    let config = state.config.clone();
    let user_id = user.user_id.clone();
    let account_id = t.account_id.clone();
    let provider_thread_id = t.provider_thread_id.clone();
    let lines = futures::stream::iter(msgs)
        .then(move |m| {
            let svc = svc.clone();
            let config = config.clone();
            let user_id = user_id.clone();
            let resolved = ResolvedMessage {
                message_id: m.message_id,
                account_id: account_id.clone(),
                thread_id: tid.clone(),
                provider_message_id: m.provider_message_id,
                provider_thread_id: provider_thread_id.clone(),
                imap_locations: m.imap_locations,
                is_draft: m.is_draft,
            };
            async move { sanitized(&svc, &config, &user_id, &resolved).await.ok().map(|b| ndjson_line(&b)) }
        })
        .filter_map(futures::future::ready)
        .boxed();
    Ok(stream_response(lines))
}

/// `POST /v1/prefetch`: NDJSON of `MessageBody`, or `{messageId, rawBase64}` lines for raw.
pub async fn prefetch(
    State(state): State<AppState>,
    Extension(CurrentUser(user)): Extension<CurrentUser>,
    ApiJson(body): ApiJson<PrefetchRequest>,
) -> Result<Response, ApiError> {
    if body.message_ids.is_empty() || body.message_ids.len() > 100 {
        return Err(ApiError::with_message(ErrorCode::InvalidArgument, "Send 1 to 100 message ids."));
    }
    let svc = services(&state.services)?.clone();
    svc.limits.read(&user.user_id)?;
    svc.entitled(&user.user_id).await?;
    let msgs = svc.resolve_messages(&user.user_id, &body.message_ids).await?;
    let config = state.config.clone();
    let user_id = user.user_id.clone();
    let format = body.format;
    let lines = futures::stream::iter(msgs)
        .then(move |m| {
            let svc = svc.clone();
            let config = config.clone();
            let user_id = user_id.clone();
            async move {
                match format {
                    BodyFormat::Sanitized => sanitized(&svc, &config, &user_id, &m).await.ok().map(|b| ndjson_line(&b)),
                    BodyFormat::Raw => {
                        let h = svc.handle(&user_id, &m.account_id).ok()?;
                        let raw = svc.fetch_raw(&user_id, &h, &m).await.ok()?;
                        svc.limits.egress(&user_id, raw.len() as u64).ok()?;
                        Some(ndjson_line(&RawMessageLine {
                            message_id: m.message_id.clone(),
                            raw_base64: base64::engine::general_purpose::STANDARD.encode(raw),
                        }))
                    }
                }
            }
        })
        .filter_map(futures::future::ready)
        .boxed();
    Ok(stream_response(lines))
}

/// Parses a single `bytes=a-b` range against `len`. `Err(())` means unsatisfiable (416).
pub fn parse_range(h: &str, len: usize) -> Option<Result<(usize, usize), ()>> {
    let spec = h.trim().strip_prefix("bytes=")?;
    if spec.contains(',') {
        return None; // multiple ranges: serve the whole part
    }
    let (a, b) = spec.split_once('-')?;
    let (start, end) = match (a.trim(), b.trim()) {
        ("", suffix) => {
            let n: usize = suffix.parse().ok()?;
            if n == 0 {
                return Some(Err(()));
            }
            (len.saturating_sub(n), len.checked_sub(1)?)
        }
        (s, "") => (s.parse().ok()?, len.checked_sub(1)?),
        (s, e) => (s.parse().ok()?, e.parse::<usize>().ok()?.min(len.checked_sub(1)?)),
    };
    if start >= len || start > end {
        return Some(Err(()));
    }
    Some(Ok((start, end)))
}

/// `Content-Disposition: attachment; filename*=UTF-8''…`.
pub fn content_disposition(filename: &str) -> String {
    let clean: String = filename.chars().filter(|c| !c.is_control()).take(255).collect();
    let clean = if clean.trim().is_empty() { "attachment".to_owned() } else { clean };
    let enc = percent_encoding::utf8_percent_encode(&clean, percent_encoding::NON_ALPHANUMERIC).to_string();
    format!("attachment; filename*=UTF-8''{enc}")
}

async fn serve_part(
    svc: &Arc<Services>,
    user_id: &str,
    message_id: MessageId,
    part_id: &str,
    headers: &HeaderMap,
) -> Result<Response, ApiError> {
    if part_id.is_empty() || part_id.len() > 64 || !part_id.bytes().all(|b| b.is_ascii_digit() || b == b'.') {
        return Err(ApiError::new(ErrorCode::NotFound));
    }
    svc.limits.read(user_id)?;
    svc.limits.egress_open(user_id)?;
    svc.entitled(user_id).await?;
    let msg = svc.resolve_message(user_id, &message_id).await?;
    let handle = svc.handle(user_id, &msg.account_id)?;
    let raw = svc.fetch_raw(user_id, &handle, &msg).await?;
    let part = akatsuki_mime::part_content(&raw, part_id)
        .map_err(|_| ApiError::new(ErrorCode::NotFound))?
        .ok_or_else(|| ApiError::new(ErrorCode::NotFound))?;
    let ctype = if part.mime_type.to_ascii_lowercase().starts_with("image/") && !part.mime_type.contains("svg") {
        part.mime_type.clone()
    } else {
        "application/octet-stream".to_owned()
    };
    let len = part.bytes.len();
    let range = headers.get(header::RANGE).and_then(|v| v.to_str().ok()).and_then(|h| parse_range(h, len));
    let (status, bytes, content_range) = match range {
        Some(Ok((s, e))) => {
            (StatusCode::PARTIAL_CONTENT, part.bytes[s..=e].to_vec(), Some(format!("bytes {s}-{e}/{len}")))
        }
        Some(Err(())) => {
            let mut resp = StatusCode::RANGE_NOT_SATISFIABLE.into_response();
            if let Ok(v) = HeaderValue::from_str(&format!("bytes */{len}")) {
                resp.headers_mut().insert(header::CONTENT_RANGE, v);
            }
            return Ok(resp);
        }
        None => (StatusCode::OK, part.bytes, None),
    };
    // Attachment egress: 2 GB per user per UTC day (§13.4, D-397).
    svc.limits.egress(user_id, bytes.len() as u64)?;
    let mut resp = (status, bytes).into_response();
    let h = resp.headers_mut();
    if let Ok(v) = HeaderValue::from_str(&ctype) {
        h.insert(header::CONTENT_TYPE, v);
    }
    if let Ok(v) = HeaderValue::from_str(&content_disposition(part.filename.as_deref().unwrap_or("attachment"))) {
        h.insert(header::CONTENT_DISPOSITION, v);
    }
    h.insert(header::X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff"));
    h.insert(header::CONTENT_SECURITY_POLICY, HeaderValue::from_static("sandbox"));
    h.insert(header::ACCEPT_RANGES, HeaderValue::from_static("bytes"));
    if let Some(cr) = content_range.and_then(|c| HeaderValue::from_str(&c).ok()) {
        h.insert(header::CONTENT_RANGE, cr);
    }
    Ok(resp)
}

/// `GET /v1/messages/{messageId}/parts/{partId}`.
pub async fn part(
    State(state): State<AppState>,
    Extension(CurrentUser(user)): Extension<CurrentUser>,
    Path((message_id, part_id)): Path<(String, String)>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    let svc = services(&state.services)?;
    serve_part(svc, &user.user_id, MessageId::new(message_id), &part_id, &headers).await
}

/// `POST /v1/links`: a 5-minute HMAC capability for `<a download>` and share sheets.
pub async fn create_link(
    State(state): State<AppState>,
    Extension(CurrentUser(user)): Extension<CurrentUser>,
    ApiJson(body): ApiJson<LinkRequest>,
) -> Result<Response, ApiError> {
    let svc = services(&state.services)?;
    let key = state.config.link_signing_key.as_ref().ok_or_else(|| {
        ApiError::with_message(ErrorCode::ProviderUnavailable, "LINK_SIGNING_KEY is not configured on this server.")
    })?;
    svc.resolve_message(&user.user_id, &body.message_id).await?;
    let expires_at = now_ms() + LINK_TTL_MS;
    let token = sign_link(
        &key.0,
        &LinkClaims {
            u: user.user_id.clone(),
            m: body.message_id.as_str().to_owned(),
            p: body.part_id,
            exp: expires_at,
        },
    );
    Ok(Json(LinkResponse { url: format!("/v1/l/{token}"), expires_at }).into_response())
}

/// `GET /v1/l/{token}`.
pub async fn capability_link(
    State(state): State<AppState>,
    Path(token): Path<String>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    let svc = services(&state.services)?;
    let key = state.config.link_signing_key.as_ref().ok_or_else(|| ApiError::new(ErrorCode::NotFound))?;
    let claims = verify_link(&key.0, &token, now_ms()).ok_or_else(|| ApiError::new(ErrorCode::NotFound))?;
    serve_part(svc, &claims.u, MessageId::new(claims.m), &claims.p, &headers).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ranges() {
        assert_eq!(parse_range("bytes=0-9", 100), Some(Ok((0, 9))));
        assert_eq!(parse_range("bytes=90-", 100), Some(Ok((90, 99))));
        assert_eq!(parse_range("bytes=-10", 100), Some(Ok((90, 99))));
        assert_eq!(parse_range("bytes=50-500", 100), Some(Ok((50, 99))));
        assert_eq!(parse_range("bytes=100-", 100), Some(Err(())));
        assert_eq!(parse_range("bytes=0-1,5-6", 100), None);
        assert_eq!(parse_range("items=0-1", 100), None);
    }

    #[test]
    fn dispositions_are_encoded() {
        assert_eq!(
            content_disposition("Résumé 2026.pdf"),
            "attachment; filename*=UTF-8''R%C3%A9sum%C3%A9%202026%2Epdf"
        );
        assert_eq!(content_disposition("a\r\nb"), "attachment; filename*=UTF-8''ab");
        assert_eq!(content_disposition(""), "attachment; filename*=UTF-8''attachment");
    }
}
