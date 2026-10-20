//! Router tests: every request goes through the real middleware stack.

use std::collections::BTreeMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use akatsuki_core::auth::{AuthError, VerifiedUser};
use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use tower::ServiceExt;

use crate::config::Config;
use crate::http::{AppState, PIXEL_GIF, VerifyUser, log_user_hash, return_to_allowed, router};

struct FakeVerifier;

impl VerifyUser for FakeVerifier {
    fn verify<'a>(
        &'a self,
        token: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<VerifiedUser, AuthError>> + Send + 'a>> {
        Box::pin(async move {
            if token == "good" {
                Ok(VerifiedUser { user_id: "user_1".into(), session_id: None })
            } else {
                Err(AuthError::Malformed)
            }
        })
    }
}

const IMG_KEY: &str = "AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8="; // test only

fn app() -> axum::Router {
    let vars: BTreeMap<String, String> =
        [("CORS_ORIGINS", "https://app.akatsuki.vc,http://localhost:5173"), ("IMG_PROXY_KEY", IMG_KEY)]
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect();
    let config = Config::from_map(&vars).unwrap();
    router(AppState::new(config, Some(Arc::new(FakeVerifier))))
}

async fn call(req: Request<Body>) -> (StatusCode, axum::http::HeaderMap, Vec<u8>) {
    let resp = app().oneshot(req).await.unwrap();
    let status = resp.status();
    let headers = resp.headers().clone();
    let body = resp.into_body().collect().await.unwrap().to_bytes().to_vec();
    (status, headers, body)
}

fn json_body(b: &[u8]) -> Value {
    serde_json::from_slice(b).unwrap()
}

fn get(uri: &str) -> Request<Body> {
    Request::get(uri).body(Body::empty()).unwrap()
}

fn post_json(uri: &str, token: Option<&str>, body: Value) -> Request<Body> {
    let mut b = Request::post(uri).header(header::CONTENT_TYPE, "application/json");
    if let Some(t) = token {
        b = b.header(header::AUTHORIZATION, format!("Bearer {t}"));
    }
    b.body(Body::from(body.to_string())).unwrap()
}

#[tokio::test]
async fn healthz_and_readyz() {
    let (s, h, b) = call(get("/healthz")).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(b, br#"{"ok":true}"#);
    assert_eq!(h.get(header::X_CONTENT_TYPE_OPTIONS).unwrap(), "nosniff");
    let (s, _, b) = call(get("/readyz")).await;
    assert_eq!(s, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(json_body(&b), json!({"ok": false}));
}

#[tokio::test]
async fn v1_requires_a_valid_bearer() {
    for req in [
        get("/v1/messages/m1"),
        Request::get("/v1/messages/m1").header(header::AUTHORIZATION, "Bearer bad").body(Body::empty()).unwrap(),
        Request::get("/v1/messages/m1").header(header::AUTHORIZATION, "Basic good").body(Body::empty()).unwrap(),
    ] {
        let (s, h, b) = call(req).await;
        assert_eq!(s, StatusCode::UNAUTHORIZED);
        assert_eq!(json_body(&b)["error"]["code"], "unauthenticated");
        assert_eq!(json_body(&b)["error"]["retryable"], false);
        assert_eq!(h.get(header::CACHE_CONTROL).unwrap(), "private, no-store");
    }
}

#[tokio::test]
async fn authed_routes_need_the_sync_services() {
    let cases = [
        Request::get("/v1/messages/m1?format=raw")
            .header(header::AUTHORIZATION, "Bearer good")
            .body(Body::empty())
            .unwrap(),
        Request::get("/v1/threads/t1/bodies").header(header::AUTHORIZATION, "Bearer good").body(Body::empty()).unwrap(),
        Request::get("/v1/messages/m1/parts/1.2")
            .header(header::AUTHORIZATION, "Bearer good")
            .body(Body::empty())
            .unwrap(),
        Request::post("/v1/accounts/a1/sync").header(header::AUTHORIZATION, "Bearer good").body(Body::empty()).unwrap(),
        post_json("/v1/prefetch", Some("good"), json!({"messageIds": ["m1"], "format": "sanitized"})),
        post_json("/v1/links", Some("good"), json!({"messageId": "m1", "partId": "2"})),
        post_json("/v1/connect/google/start", Some("good"), json!({"returnTo": "https://app.akatsuki.vc/settings"})),
        post_json(
            "/v1/send",
            Some("good"),
            json!({"clientSendId": "0192f0c4-0000-7000-8000-000000000001", "clientDraftId": "0192f0c4-0000-7000-8000-000000000002", "baseVersion": 1, "undoSeconds": 10, "deviceId": "dev"}),
        ),
        Request::get("/v1/search?accountId=a1&q=from%3Aada")
            .header(header::AUTHORIZATION, "Bearer good")
            .body(Body::empty())
            .unwrap(),
        Request::delete("/v1/drafts/0192f0c4-0000-7000-8000-000000000002")
            .header(header::AUTHORIZATION, "Bearer good")
            .body(Body::empty())
            .unwrap(),
        Request::post("/v1/uploads")
            .header(header::AUTHORIZATION, "Bearer good")
            .header(header::CONTENT_TYPE, "application/pdf")
            .header("x-filename", "a.pdf")
            .body(Body::from("%PDF"))
            .unwrap(),
    ];
    for req in cases {
        let uri = req.uri().to_string();
        let (s, _, b) = call(req).await;
        assert_eq!(s, StatusCode::SERVICE_UNAVAILABLE, "{uri}: {}", String::from_utf8_lossy(&b));
        assert_eq!(json_body(&b)["error"]["code"], "provider_unavailable", "{uri}");
        assert_eq!(json_body(&b)["error"]["retryable"], true, "{uri}");
    }
}

#[tokio::test]
async fn request_validation_happens_before_anything_else() {
    let (s, _, b) =
        call(post_json("/v1/connect/google/start", Some("good"), json!({"returnTo": "https://evil.example/"}))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    assert_eq!(json_body(&b)["error"]["code"], "invalid_argument");
    let (s, _, _) = call(post_json("/v1/connect/yahoo/start", Some("good"), json!({"returnTo": "akatsuki://x"}))).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    let (s, _, b) = call(post_json("/v1/prefetch", Some("good"), json!({"messageIds": [], "format": "raw"}))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{}", String::from_utf8_lossy(&b));
    let (s, _, b) = call(post_json("/v1/send", Some("good"), json!({"nope": true}))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    assert_eq!(json_body(&b)["error"]["code"], "invalid_argument");
    let req =
        Request::post("/v1/prefetch").header(header::AUTHORIZATION, "Bearer good").body(Body::from("{}")).unwrap();
    let (s, _, _) = call(req).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "missing content type");
}

#[tokio::test]
async fn imap_connect_never_echoes_the_password() {
    let body = json!({
        "email": "me@example.com", "username": "me", "password": "hunter2-not-real",
        "imap": {"host": "imap.example.com", "port": 993, "tls": "implicit"},
        "smtp": {"host": "smtp.example.com", "port": 465, "tls": "implicit"}
    });
    let (s, _, b) = call(post_json("/v1/connect/imap", Some("good"), body)).await;
    assert_eq!(s, StatusCode::SERVICE_UNAVAILABLE);
    assert!(!String::from_utf8_lossy(&b).contains("hunter2"));
}

#[tokio::test]
async fn graph_validation_echoes_token() {
    let (s, h, b) = call(Request::post("/hooks/graph?validationToken=abc%20123").body(Body::empty()).unwrap()).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(b, b"abc 123");
    assert!(h.get(header::CONTENT_TYPE).unwrap().to_str().unwrap().starts_with("text/plain"));
    let (s, _, _) = call(Request::post("/hooks/graph/lifecycle?validationToken=t").body(Body::empty()).unwrap()).await;
    assert_eq!(s, StatusCode::OK);
    let (s, _, _) = call(post_json("/hooks/graph", None, json!({"value": []}))).await;
    assert_eq!(s, StatusCode::SERVICE_UNAVAILABLE);
    let (s, _, _) = call(post_json("/hooks/gmail", None, json!({}))).await;
    assert_eq!(s, StatusCode::SERVICE_UNAVAILABLE);
}

#[tokio::test]
async fn image_proxy_only_serves_signed_urls() {
    use base64::Engine as _;
    let key = base64::engine::general_purpose::STANDARD.decode(IMG_KEY).unwrap();
    let signed = akatsuki_mime::ProxySigner::new("https://sync.akatsuki.vc", &key, "user_1")
        .sign("https://cdn.example.com/a.png");
    let path = signed.strip_prefix("https://sync.akatsuki.vc").unwrap();
    let (s, _, b) = call(get(path)).await;
    assert_eq!(s, StatusCode::SERVICE_UNAVAILABLE, "{}", String::from_utf8_lossy(&b));
    let tampered = path.replace("/v1/img/", "/v1/img/0");
    let (s, _, _) = call(get(&tampered)).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn read_receipt_pixel() {
    let (s, h, b) = call(get("/r/abcdefghijklmnopqrstuv.gif")).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(b, PIXEL_GIF);
    assert_eq!(h.get(header::CONTENT_TYPE).unwrap(), "image/gif");
    assert_eq!(h.get(header::CACHE_CONTROL).unwrap(), "no-store");
    let (s, _, _) = call(get("/r/abc.png")).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn cors_is_exact_origin() {
    let preflight = |origin: &str| {
        Request::options("/v1/messages/m1")
            .header(header::ORIGIN, origin)
            .header(header::ACCESS_CONTROL_REQUEST_METHOD, "GET")
            .header(header::ACCESS_CONTROL_REQUEST_HEADERS, "authorization")
            .body(Body::empty())
            .unwrap()
    };
    let (_, h, _) = call(preflight("https://app.akatsuki.vc")).await;
    assert_eq!(h.get(header::ACCESS_CONTROL_ALLOW_ORIGIN).unwrap(), "https://app.akatsuki.vc");
    assert_eq!(h.get(header::ACCESS_CONTROL_MAX_AGE).unwrap(), "600");
    assert!(h.get(header::ACCESS_CONTROL_ALLOW_CREDENTIALS).is_none());
    let (_, h, _) = call(preflight("https://evil.example")).await;
    assert!(h.get(header::ACCESS_CONTROL_ALLOW_ORIGIN).is_none());
}

#[tokio::test]
async fn metrics_are_internal_only() {
    let (s, _, b) = call(get("/metrics")).await;
    assert_eq!(s, StatusCode::OK);
    assert!(String::from_utf8_lossy(&b).contains("akatsuki_sync_up 1"));
    let (s, _, _) =
        call(Request::get("/metrics").header("x-forwarded-for", "203.0.113.9").body(Body::empty()).unwrap()).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn unknown_routes_use_the_envelope() {
    let (s, _, b) = call(get("/nope")).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    assert_eq!(json_body(&b)["error"]["code"], "not_found");
}

#[test]
fn return_to_allow_list() {
    let dev = vec!["http://localhost:5173".to_owned()];
    assert!(return_to_allowed("https://app.akatsuki.vc/settings/accounts", &dev));
    assert!(return_to_allowed("akatsuki://connected", &dev));
    assert!(return_to_allowed("http://127.0.0.1:53111/cb", &dev));
    assert!(return_to_allowed("http://localhost:5173/x", &dev));
    assert!(!return_to_allowed("https://app.akatsuki.vc.evil.example/", &dev));
    assert!(!return_to_allowed("https://user@app.akatsuki.vc/", &dev));
    assert!(!return_to_allowed("http://127.0.0.1/cb", &dev));
    assert!(!return_to_allowed("javascript:alert(1)", &dev));
}

#[test]
fn user_hash_is_short_and_stable() {
    assert_eq!(log_user_hash("user_1").len(), 12);
    assert_eq!(log_user_hash("user_1"), log_user_hash("user_1"));
}
