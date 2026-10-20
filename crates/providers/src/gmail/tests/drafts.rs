//! Drafts, sends and the Sent check before every attempt (§5.6, §10.2).

use bytes::Bytes;
use pretty_assertions::assert_eq;
use wiremock::matchers::{body_json, body_string_contains, header_regex, method, path, query_param};
use wiremock::{Mock, ResponseTemplate};

use super::*;
use crate::gmail::convert::encode_b64;
use crate::gmail::is_ambiguous_send_error;
use crate::{MailProvider, OutgoingMime, RemoteDraftRef, SentRef};

const MIME: &str = "From: Ada Lovelace <ada@example.com>\r\nTo: grace@example.org\r\nSubject: Re: Q3 plan\r\nMessage-ID: <0190a1b2-c3d4-7e5f-8a9b-0c1d2e3f4a5b@example.com>\r\nMIME-Version: 1.0\r\nContent-Type: text/plain; charset=utf-8\r\n\r\nLooks good.\r\n";
const RFC_ID: &str = "0190a1b2-c3d4-7e5f-8a9b-0c1d2e3f4a5b@example.com";

fn mime() -> OutgoingMime {
    OutgoingMime {
        rfc_message_id: RFC_ID.into(),
        bytes: Bytes::from_static(MIME.as_bytes()),
        thread: Some("18c2a7f3e9b0d001".into()),
    }
}

async fn mount_sent_lookup(server: &MockServer, body: &str) {
    Mock::given(method("GET"))
        .and(path(format!("{USER}/messages")))
        .and(query_param("q", format!("rfc822msgid:{RFC_ID} in:sent")))
        .and(query_param("includeSpamTrash", "true"))
        .respond_with(json(body))
        .expect(1)
        .mount(server)
        .await;
}

#[tokio::test]
async fn save_draft_creates_then_updates() {
    let (server, p, _) = setup().await;
    let raw = encode_b64(MIME.as_bytes());
    Mock::given(method("POST"))
        .and(path(format!("{USER}/drafts")))
        .and(body_json(serde_json::json!({"message": {"raw": raw, "threadId": "18c2a7f3e9b0d001"}})))
        .respond_with(json(
            r#"{"id":"r-5513","message":{"id":"18c2c0000000a001","threadId":"18c2a7f3e9b0d001","labelIds":["DRAFT"]}}"#,
        ))
        .expect(1)
        .mount(&server)
        .await;
    let d = p.save_draft(&mime(), None).await.unwrap();
    assert_eq!(
        d,
        RemoteDraftRef { provider_draft_id: "r-5513".into(), provider_message_id: Some("18c2c0000000a001".into()) }
    );

    Mock::given(method("PUT"))
        .and(path(format!("{USER}/drafts/r-5513")))
        .and(body_json(serde_json::json!({"id": "r-5513", "message": {"raw": raw, "threadId": "18c2a7f3e9b0d001"}})))
        .respond_with(json(r#"{"id":"r-5513","message":{"id":"18c2c0000000a002","threadId":"18c2a7f3e9b0d001"}}"#))
        .expect(1)
        .mount(&server)
        .await;
    let d2 = p.save_draft(&mime(), Some(&d)).await.unwrap();
    assert_eq!(
        d2.provider_message_id.as_deref(),
        Some("18c2c0000000a002"),
        "Gmail gives an updated draft a new message id"
    );
}

#[tokio::test]
async fn save_draft_recreates_a_draft_deleted_elsewhere() {
    let (server, p, _) = setup().await;
    Mock::given(method("PUT"))
        .and(path(format!("{USER}/drafts/r-old")))
        .respond_with(gmail_error(404, "notFound", "Requested entity was not found."))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path(format!("{USER}/drafts")))
        .respond_with(json(r#"{"id":"r-new","message":{"id":"m-new"}}"#))
        .expect(1)
        .mount(&server)
        .await;
    let old = RemoteDraftRef { provider_draft_id: "r-old".into(), provider_message_id: None };
    assert_eq!(p.save_draft(&mime(), Some(&old)).await.unwrap().provider_draft_id, "r-new");
}

#[tokio::test]
async fn large_drafts_use_the_multipart_upload_endpoint() {
    let server = MockServer::start().await;
    let mut cfg = config(&server);
    cfg.upload_threshold = 16;
    let p = GmailProvider::new(
        cfg,
        std::sync::Arc::new(CountingTokens::default()),
        std::sync::Arc::new(AccountQuota::standalone()),
        reqwest::Client::new(),
    );
    Mock::given(method("POST"))
        .and(path("/upload/gmail/v1/users/me/drafts"))
        .and(query_param("uploadType", "multipart"))
        .and(header_regex("content-type", "^multipart/related; boundary=akatsuki_[0-9a-f]{32}$"))
        .and(body_string_contains(
            "Content-Type: application/json; charset=UTF-8\r\n\r\n{\"message\":{\"threadId\":\"18c2a7f3e9b0d001\"}}",
        ))
        .and(body_string_contains("Content-Type: message/rfc822\r\n\r\nFrom: Ada Lovelace"))
        .respond_with(json(r#"{"id":"r-big","message":{"id":"m-big"}}"#))
        .expect(1)
        .mount(&server)
        .await;
    assert_eq!(p.save_draft(&mime(), None).await.unwrap().provider_draft_id, "r-big");
}

#[tokio::test]
async fn send_draft_checks_sent_first() {
    let (server, p, _) = setup().await;
    mount_sent_lookup(&server, r#"{"resultSizeEstimate":0}"#).await;
    Mock::given(method("POST"))
        .and(path(format!("{USER}/drafts/send")))
        .and(body_json(serde_json::json!({"id": "r-5513"})))
        .respond_with(json(r#"{"id":"18c2d00000000001","threadId":"18c2a7f3e9b0d001","labelIds":["SENT"]}"#))
        .expect(1)
        .mount(&server)
        .await;
    let d = RemoteDraftRef { provider_draft_id: "r-5513".into(), provider_message_id: None };
    let s = p.send_draft(&d, &format!("<{RFC_ID}>")).await.unwrap();
    assert_eq!(
        s,
        SentRef { provider_message_id: "18c2d00000000001".into(), provider_thread_id: Some("18c2a7f3e9b0d001".into()) }
    );
}

#[tokio::test]
async fn a_message_already_in_sent_is_never_sent_again() {
    let (server, p, _) = setup().await;
    mount_sent_lookup(
        &server,
        r#"{"messages":[{"id":"18c2d00000000001","threadId":"18c2a7f3e9b0d001"}],"resultSizeEstimate":1}"#,
    )
    .await;
    Mock::given(method("POST"))
        .and(path(format!("{USER}/drafts/send")))
        .respond_with(json("{}"))
        .expect(0)
        .mount(&server)
        .await;
    let d = RemoteDraftRef { provider_draft_id: "r-5513".into(), provider_message_id: None };
    assert_eq!(p.send_draft(&d, RFC_ID).await.unwrap().provider_message_id, "18c2d00000000001");
}

#[tokio::test]
async fn send_raw_uses_messages_send() {
    let (server, p, _) = setup().await;
    mount_sent_lookup(&server, "{}").await;
    Mock::given(method("POST"))
        .and(path(format!("{USER}/messages/send")))
        .and(body_json(serde_json::json!({"raw": encode_b64(MIME.as_bytes()), "threadId": "18c2a7f3e9b0d001"})))
        .respond_with(json(r#"{"id":"18c2d00000000009","threadId":"18c2a7f3e9b0d001","labelIds":["SENT"]}"#))
        .expect(1)
        .mount(&server)
        .await;
    assert_eq!(p.send_raw(&mime()).await.unwrap().provider_message_id, "18c2d00000000009");
}

#[tokio::test]
async fn send_errors_are_classified() {
    let (server, p, _) = setup().await;
    Mock::given(method("GET")).and(path(format!("{USER}/messages"))).respond_with(json("{}")).mount(&server).await;
    Mock::given(method("POST"))
        .and(path(format!("{USER}/drafts/send")))
        .respond_with(gmail_error(403, "dailyLimitExceeded", "User-rate limit exceeded: daily sending quota"))
        .mount(&server)
        .await;
    let d = RemoteDraftRef { provider_draft_id: "r-1".into(), provider_message_id: None };
    let e = p.send_draft(&d, RFC_ID).await.unwrap_err();
    assert!(matches!(e, ProviderError::Quota(_)), "{e:?}");
    assert!(!is_ambiguous_send_error(&e));

    assert!(is_ambiguous_send_error(&ProviderError::Transient("gmail 502: ".into(), None)));
    assert!(is_ambiguous_send_error(&ProviderError::Transient("network: timeout".into(), None)));
    assert!(!is_ambiguous_send_error(&ProviderError::Transient("network: connect".into(), None)));
    assert!(matches!(p.find_sent("bad id with spaces").await, Err(ProviderError::Invalid(_))));
}

#[tokio::test]
async fn a_5xx_on_send_is_ambiguous() {
    let (server, p, _) = setup().await;
    Mock::given(method("GET")).and(path(format!("{USER}/messages"))).respond_with(json("{}")).mount(&server).await;
    Mock::given(method("POST"))
        .and(path(format!("{USER}/drafts/send")))
        .respond_with(ResponseTemplate::new(502))
        .mount(&server)
        .await;
    let d = RemoteDraftRef { provider_draft_id: "r-1".into(), provider_message_id: None };
    let e = p.send_draft(&d, RFC_ID).await.unwrap_err();
    assert!(is_ambiguous_send_error(&e), "{e:?}");
}

#[tokio::test]
async fn deleted_draft_send_is_not_found() {
    let (server, p, _) = setup().await;
    Mock::given(method("GET")).and(path(format!("{USER}/messages"))).respond_with(json("{}")).mount(&server).await;
    Mock::given(method("POST"))
        .and(path(format!("{USER}/drafts/send")))
        .respond_with(gmail_error(404, "notFound", "Requested entity was not found."))
        .mount(&server)
        .await;
    let d = RemoteDraftRef { provider_draft_id: "r-gone".into(), provider_message_id: None };
    assert_eq!(p.send_draft(&d, RFC_ID).await.unwrap_err(), ProviderError::NotFound);
}

#[tokio::test]
async fn delete_and_get_drafts() {
    let (server, p, _) = setup().await;
    Mock::given(method("DELETE"))
        .and(path(format!("{USER}/drafts/r-x")))
        .respond_with(gmail_error(404, "notFound", "x"))
        .mount(&server)
        .await;
    p.delete_draft(&RemoteDraftRef { provider_draft_id: "r-x".into(), provider_message_id: None }).await.unwrap();
    Mock::given(method("GET"))
        .and(path(format!("{USER}/drafts/r-y")))
        .and(query_param("format", "raw"))
        .respond_with(json(&format!(
            r#"{{"id":"r-y","message":{{"id":"m-y","raw":"{}"}}}}"#,
            encode_b64(MIME.as_bytes())
        )))
        .mount(&server)
        .await;
    let (d, raw) = p.get_draft_raw("r-y").await.unwrap();
    assert_eq!((d.provider_draft_id.as_str(), d.provider_message_id.as_deref()), ("r-y", Some("m-y")));
    assert_eq!(raw, MIME.as_bytes());
}
