//! Wiremock tests for the Gmail adapter. Every fixture is synthetic (no real mail, no real
//! accounts); no test touches the network beyond the local mock server.

mod auth;
mod drafts;
mod ops;
mod sync;

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use async_trait::async_trait;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use super::quota::AccountQuota;
use super::{GmailConfig, GmailProvider};
use crate::{AccessToken, ProviderError, TokenSource};

pub(super) const LABELS: &str = include_str!("../fixtures/labels.json");
pub(super) const THREAD: &str = include_str!("../fixtures/thread_metadata.json");
pub(super) const THREAD_SMALL: &str = include_str!("../fixtures/thread_small.json");
pub(super) const HISTORY_1: &str = include_str!("../fixtures/history_page1.json");
pub(super) const HISTORY_2: &str = include_str!("../fixtures/history_page2.json");
pub(super) const MESSAGE_FULL: &str = include_str!("../fixtures/message_full.json");

pub(super) const USER: &str = "/gmail/v1/users/me";

/// A token source that hands out `tok-1`, `tok-2`, … and counts invalidations.
#[derive(Default)]
pub(super) struct CountingTokens {
    pub issued: AtomicUsize,
    pub invalidated: AtomicUsize,
}

#[async_trait]
impl TokenSource for CountingTokens {
    async fn access_token(&self) -> Result<AccessToken, ProviderError> {
        let n = self.invalidated.load(Ordering::SeqCst) + 1;
        self.issued.fetch_add(1, Ordering::SeqCst);
        Ok(AccessToken { token: format!("tok-{n}"), expires_at: i64::MAX })
    }
    async fn invalidate(&self) {
        self.invalidated.fetch_add(1, Ordering::SeqCst);
    }
}

pub(super) fn config(server: &MockServer) -> GmailConfig {
    GmailConfig {
        api_base: server.uri(),
        revoke_url: format!("{}/revoke", server.uri()),
        pubsub_topic: Some("projects/akatsuki-test/topics/gmail-push".into()),
        ..GmailConfig::default()
    }
}

pub(super) async fn setup() -> (MockServer, GmailProvider, Arc<CountingTokens>) {
    let server = MockServer::start().await;
    let tokens = Arc::new(CountingTokens::default());
    let p = GmailProvider::new(
        config(&server),
        tokens.clone(),
        Arc::new(AccountQuota::standalone()),
        reqwest::Client::new(),
    )
    .with_clock(Arc::new(|| 1_790_000_000_000));
    (server, p, tokens)
}

pub(super) fn json(body: &str) -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_raw(body.as_bytes().to_vec(), "application/json; charset=UTF-8")
}

pub(super) fn gmail_error(status: u16, reason: &str, message: &str) -> ResponseTemplate {
    let body = serde_json::json!({
        "error": {"code": status, "message": message, "errors": [{"message": message, "domain": "global", "reason": reason}], "status": "X"}
    });
    ResponseTemplate::new(status).set_body_raw(body.to_string().into_bytes(), "application/json; charset=UTF-8")
}

/// A batch response with one part per `(status, json)`, keyed `response-item{n}`.
pub(super) fn batch_response(parts: &[(u16, &str)]) -> ResponseTemplate {
    let mut b = String::new();
    for (i, (status, body)) in parts.iter().enumerate() {
        b.push_str(&format!(
            "--batch_test\r\nContent-Type: application/http\r\nContent-ID: <response-item{}>\r\n\r\nHTTP/1.1 {status} X\r\nContent-Type: application/json; charset=UTF-8\r\n\r\n{body}\r\n",
            i + 1
        ));
    }
    b.push_str("--batch_test--\r\n");
    ResponseTemplate::new(200).set_body_raw(b.into_bytes(), "multipart/mixed; boundary=batch_test")
}

pub(super) async fn mount_labels(server: &MockServer) {
    Mock::given(method("GET")).and(path(format!("{USER}/labels"))).respond_with(json(LABELS)).mount(server).await;
}
