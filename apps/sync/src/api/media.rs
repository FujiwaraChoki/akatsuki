//! The image proxy (ARCHITECTURE §14.4) and the read-receipt pixel (§7.2, BRIEF decision 8).

use akatsuki_core::convex::worker::RecordReadReceiptArgs;
use akatsuki_core::time::now_ms;
use akatsuki_core::wire::ErrorCode;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};

use super::services;
use crate::capability::{opened_through_proxy, verify_receipt};
use crate::fetch::sniff_image;
use crate::http::{ApiError, AppState, PIXEL_GIF};
use crate::metrics::Metrics;

/// Largest proxied image.
pub const IMG_MAX_BYTES: usize = 10 * 1024 * 1024;

/// `GET /v1/img/{u}/{sig}/{b64url}`: only URLs the worker signed inside a body work.
pub async fn image(
    State(state): State<AppState>,
    Path((u, sig, b64)): Path<(String, String, String)>,
) -> Result<Response, ApiError> {
    let Some(key) = &state.config.img_proxy_key else { return Err(ApiError::new(ErrorCode::NotFound)) };
    let Some(url) = akatsuki_mime::ProxySigner::verify(&key.0, &u, &sig, &b64) else {
        return Err(ApiError::new(ErrorCode::NotFound));
    };
    let svc = services(&state.services)?;
    if !svc.img_allowed(&u) {
        return Err(ApiError::new(ErrorCode::RateLimited).retry_after(10_000));
    }
    let refused = || {
        Metrics::inc(&svc.metrics.images_refused);
        ApiError::new(ErrorCode::NotFound)
    };
    let fetched = svc.fetcher.get(&url, "image/*", IMG_MAX_BYTES).await.map_err(|_| refused())?;
    if fetched.status != 200 {
        return Err(refused());
    }
    // The type comes from the bytes, never from the remote header (SVG and HTML are refused).
    let ctype = sniff_image(&fetched.body).ok_or_else(refused)?;
    Metrics::inc(&svc.metrics.images_proxied);
    let mut resp = (StatusCode::OK, fetched.body).into_response();
    let h = resp.headers_mut();
    h.insert(header::CONTENT_TYPE, HeaderValue::from_static(ctype));
    h.insert(header::CACHE_CONTROL, HeaderValue::from_static("private, max-age=86400"));
    h.insert(header::CONTENT_SECURITY_POLICY, HeaderValue::from_static("default-src 'none'; sandbox"));
    h.insert(header::X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff"));
    Ok(resp)
}

/// `GET /r/{token}.gif`: always the 1×1 GIF; a token minted with `READ_RECEIPT_KEY` records an
/// open (in the background, so the pixel never waits on Convex).
pub async fn read_receipt(State(state): State<AppState>, Path(file): Path<String>, headers: HeaderMap) -> Response {
    let Some(token) = file.strip_suffix(".gif") else { return ApiError::new(ErrorCode::NotFound).into_response() };
    if let (Some(key), Some(svc)) = (&state.config.read_receipt_key, &state.services)
        && verify_receipt(&key.0, token)
    {
        let proxied = opened_through_proxy(headers.get(header::USER_AGENT).and_then(|v| v.to_str().ok()));
        let api = svc.api.clone();
        let args = RecordReadReceiptArgs { token: token.to_owned(), at: now_ms(), proxied };
        Metrics::inc(&svc.metrics.read_receipts);
        tokio::spawn(async move {
            let _ = api.record_read_receipt(&args).await;
        });
    }
    let mut resp = (StatusCode::OK, PIXEL_GIF).into_response();
    let h = resp.headers_mut();
    h.insert(header::CONTENT_TYPE, HeaderValue::from_static("image/gif"));
    h.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    resp
}
