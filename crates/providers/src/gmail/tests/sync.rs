//! Labels, baseline, history deltas, windows, thread fetches, bodies, search, watch, errors.

use std::sync::atomic::Ordering;
use std::time::Duration;

use futures::StreamExt as _;
use pretty_assertions::assert_eq;
use wiremock::matchers::{body_json, body_string_contains, header, method, path, query_param};
use wiremock::{Mock, ResponseTemplate};

use super::*;
use crate::gmail::convert::encode_b64;
use crate::gmail::quota::Lane;
use crate::{
    Addr, AuthSummary, Cursor, FetchDepth, LabelKind, LabelMap, LabelRole, LimitScope, ListUnsubscribe, MailProvider,
    PushKind, RemoteMessageRef, RemoteThreadId, SysLabel, WatchKind, WindowRequest,
};

fn ids(v: &[&str]) -> Vec<RemoteThreadId> {
    v.iter().map(|s| RemoteThreadId((*s).to_owned())).collect()
}

async fn drain(mut s: crate::ByteStream) -> Vec<u8> {
    let mut out = Vec::new();
    while let Some(c) = s.next().await {
        out.extend_from_slice(&c.unwrap());
    }
    out
}

#[tokio::test]
async fn list_labels_maps_kinds_roles_and_visibility() {
    let (server, p, _) = setup().await;
    Mock::given(method("GET"))
        .and(path(format!("{USER}/labels")))
        .and(header("authorization", "Bearer tok-1"))
        .respond_with(json(LABELS))
        .expect(1)
        .mount(&server)
        .await;
    let labels = p.list_labels().await.unwrap();
    assert_eq!(labels.len(), 17);
    let by = |id: &str| labels.iter().find(|l| l.provider_label_id == id).unwrap().clone();
    assert_eq!((by("INBOX").kind, by("INBOX").role), (LabelKind::System, Some(LabelRole::Inbox)));
    assert_eq!(by("DRAFT").role, Some(LabelRole::Drafts));
    assert!(by("DRAFT").hidden);
    assert_eq!((by("CATEGORY_PROMOTIONS").kind, by("CATEGORY_PROMOTIONS").role), (LabelKind::Category, None));
    assert_eq!((by("CHAT").kind, by("CHAT").role), (LabelKind::System, None));
    let work = by("Label_1");
    assert_eq!(
        (work.kind, work.role, work.color.as_deref(), work.exclusive),
        (LabelKind::User, None, Some("#fb4c2f"), false)
    );
    assert_eq!(by("Label_2").role, Some(LabelRole::Snoozed));
    assert!(by("Label_3").hidden);
    // The snoozed label id is cached for `sys:snoozed` resolution.
    assert_eq!(p.cached_snoozed_id().as_deref(), Some("Label_2"));
}

#[tokio::test]
async fn ensure_label_finds_creates_and_survives_conflicts() {
    let (server, p, _) = setup().await;
    mount_labels(&server).await;
    // Existing (case-insensitive) → no create.
    let l = p.ensure_label(LabelRole::Snoozed, "akatsuki/snoozed").await.unwrap();
    assert_eq!((l.provider_label_id.as_str(), l.role), ("Label_2", Some(LabelRole::Snoozed)));

    // Missing → labels.create with list visibility.
    Mock::given(method("POST"))
        .and(path(format!("{USER}/labels")))
        .and(body_json(serde_json::json!({"name": "Akatsuki/Later", "labelListVisibility": "labelShow", "messageListVisibility": "show"})))
        .respond_with(json(r#"{"id":"Label_7","name":"Akatsuki/Later","type":"user"}"#))
        .expect(1)
        .mount(&server)
        .await;
    let l = p.ensure_label(LabelRole::Snoozed, "Akatsuki/Later").await.unwrap();
    assert_eq!((l.provider_label_id.as_str(), l.role), ("Label_7", Some(LabelRole::Snoozed)));
    assert_eq!(p.cached_snoozed_id().as_deref(), Some("Label_7"));

    assert!(matches!(p.ensure_label(LabelRole::Archive, "Archive").await, Err(ProviderError::Unsupported(_))));
}

#[tokio::test]
async fn create_label_conflict_returns_the_existing_label() {
    let (server, p, _) = setup().await;
    mount_labels(&server).await;
    Mock::given(method("POST"))
        .and(path(format!("{USER}/labels")))
        .respond_with(gmail_error(409, "aborted", "Label name exists or conflicts"))
        .mount(&server)
        .await;
    let l = p.create_label("work", None).await.unwrap();
    assert_eq!(l.provider_label_id, "Label_1");
}

#[tokio::test]
async fn create_label_retries_without_a_rejected_colour() {
    let (server, p, _) = setup().await;
    Mock::given(method("POST"))
        .and(path(format!("{USER}/labels")))
        .and(body_string_contains("backgroundColor"))
        .respond_with(gmail_error(400, "invalidArgument", "Label color #123456 is not on the allowed color palette"))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path(format!("{USER}/labels")))
        .and(body_json(
            serde_json::json!({"name": "Clients", "labelListVisibility": "labelShow", "messageListVisibility": "show"}),
        ))
        .respond_with(json(r#"{"id":"Label_8","name":"Clients","type":"user"}"#))
        .expect(1)
        .mount(&server)
        .await;
    assert_eq!(p.create_label("Clients", Some("#123456")).await.unwrap().provider_label_id, "Label_8");
}

#[tokio::test]
async fn baseline_reads_the_profile_history_id() {
    let (server, p, _) = setup().await;
    Mock::given(method("GET"))
        .and(path(format!("{USER}/profile")))
        .respond_with(json(r#"{"emailAddress":"ada@example.com","messagesTotal":10432,"threadsTotal":5120,"historyId":"18446744073709551000"}"#))
        .mount(&server)
        .await;
    assert_eq!(p.baseline().await.unwrap(), Cursor::Gmail { history_id: 18_446_744_073_709_551_000 });
    assert_eq!(p.profile().await.unwrap().email_address, "ada@example.com");
}

#[tokio::test]
async fn changes_since_follows_pages_and_ends_at_the_head() {
    let (server, p, _) = setup().await;
    mount_labels(&server).await;
    p.list_labels().await.unwrap(); // known labels: Label_99 is new
    Mock::given(method("GET"))
        .and(path(format!("{USER}/history")))
        .and(query_param("startHistoryId", "4821000"))
        .and(query_param("pageToken", "page-2"))
        .respond_with(json(HISTORY_2))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path(format!("{USER}/history")))
        .and(query_param("startHistoryId", "4821000"))
        .and(query_param("historyTypes", "messageAdded"))
        .and(query_param("historyTypes", "messageDeleted"))
        .and(query_param("historyTypes", "labelAdded"))
        .and(query_param("historyTypes", "labelRemoved"))
        .and(query_param("maxResults", "500"))
        .respond_with(json(HISTORY_1))
        .expect(1)
        .mount(&server)
        .await;
    let page = p.changes_since(&Cursor::Gmail { history_id: 4_821_000 }).await.unwrap();
    assert_eq!(page.touched_threads, ids(&["t-1", "t-2", "t-3", "t-4"]));
    assert_eq!(page.deleted_messages, vec!["m-gone-3".to_owned()]);
    assert!(page.deleted_threads.is_empty());
    assert!(page.labels_changed, "Label_99 is not in the last listing");
    assert_eq!(page.next, Cursor::Gmail { history_id: 4_821_100 });
    assert!(!page.more);
}

#[tokio::test]
async fn changes_since_stops_after_the_page_budget_with_a_resumable_cursor() {
    let (server, p, _) = setup().await;
    let mut cfg = config(&server);
    cfg.history_pages_per_call = 1;
    let p2 = GmailProvider::new(
        cfg,
        std::sync::Arc::new(CountingTokens::default()),
        std::sync::Arc::new(AccountQuota::standalone()),
        reqwest::Client::new(),
    );
    drop(p);
    Mock::given(method("GET")).and(path(format!("{USER}/history"))).respond_with(json(HISTORY_1)).mount(&server).await;
    let page = p2.changes_since(&Cursor::Gmail { history_id: 4_821_000 }).await.unwrap();
    assert!(page.more);
    assert_eq!(page.next, Cursor::Gmail { history_id: 4_821_003 }, "resume after the last record seen");
    assert_eq!(page.touched_threads, ids(&["t-1", "t-2"]));
    assert!(page.labels_changed, "no listing yet: user labels count as unknown");
}

#[tokio::test]
async fn changes_since_maps_404_to_cursor_expired() {
    let (server, p, _) = setup().await;
    Mock::given(method("GET"))
        .and(path(format!("{USER}/history")))
        .respond_with(gmail_error(404, "notFound", "Requested entity was not found."))
        .mount(&server)
        .await;
    assert_eq!(p.changes_since(&Cursor::Gmail { history_id: 1 }).await.unwrap_err(), ProviderError::CursorExpired);
    let graph = Cursor::Graph { folders: Default::default() };
    assert!(matches!(p.changes_since(&graph).await, Err(ProviderError::Invalid(_))));
}

#[tokio::test]
async fn empty_history_keeps_the_head() {
    let (server, p, _) = setup().await;
    Mock::given(method("GET"))
        .and(path(format!("{USER}/history")))
        .respond_with(json(r#"{"historyId":"500"}"#))
        .mount(&server)
        .await;
    let page = p.changes_since(&Cursor::Gmail { history_id: 480 }).await.unwrap();
    assert!(page.touched_threads.is_empty() && !page.more && !page.labels_changed);
    assert_eq!(page.next, Cursor::Gmail { history_id: 500 });
}

#[tokio::test]
async fn list_window_inbox_and_since() {
    let (server, p, _) = setup().await;
    Mock::given(method("GET"))
        .and(path(format!("{USER}/threads")))
        .and(query_param("labelIds", "INBOX"))
        .and(query_param("pageToken", "p1"))
        .respond_with(json(r#"{"threads":[{"id":"t-a","snippet":"x","historyId":"9"},{"id":"t-b","snippet":"y","historyId":"8"}],"nextPageToken":"p2","resultSizeEstimate":201}"#))
        .mount(&server)
        .await;
    let w = p.list_window(&WindowRequest::Inbox { page: Some("p1".into()) }).await.unwrap();
    assert_eq!(w.threads, ids(&["t-a", "t-b"]));
    assert_eq!(w.next_page.as_deref(), Some("p2"));

    Mock::given(method("GET"))
        .and(path(format!("{USER}/threads")))
        .and(query_param("q", "after:1782224000"))
        .and(query_param("includeSpamTrash", "true"))
        .respond_with(json(r#"{"threads":[{"id":"t-c"}]}"#))
        .mount(&server)
        .await;
    let w = p.list_window(&WindowRequest::Since { after: 1_782_224_000_999, page: None }).await.unwrap();
    assert_eq!((w.threads, w.next_page, w.oldest), (ids(&["t-c"]), None, None));
}

#[tokio::test]
async fn list_window_before_uses_a_date_cursor() {
    let (server, p, _) = setup().await;
    Mock::given(method("GET"))
        .and(path(format!("{USER}/messages")))
        .and(query_param("q", "before:1782224001 after:1750000000"))
        .and(query_param("maxResults", "3"))
        .respond_with(json(r#"{"messages":[{"id":"m1","threadId":"t1"},{"id":"m2","threadId":"t1"},{"id":"m3","threadId":"t2"}],"nextPageToken":"ignored"}"#))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path(format!("{USER}/messages/m3")))
        .and(query_param("format", "minimal"))
        .respond_with(json(r#"{"id":"m3","threadId":"t2","internalDate":"1781000000000"}"#))
        .expect(1)
        .mount(&server)
        .await;
    let w = p
        .list_window(&WindowRequest::Before { before: 1_782_224_000_500, horizon: 1_750_000_000_000, limit: 3 })
        .await
        .unwrap();
    assert_eq!(w.threads, ids(&["t1", "t2"]));
    assert_eq!((w.next_page, w.oldest), (None, Some(1_781_000_000_000)));
}

#[tokio::test]
async fn list_window_before_reports_the_horizon_when_exhausted() {
    let (server, p, _) = setup().await;
    Mock::given(method("GET"))
        .and(path(format!("{USER}/messages")))
        .respond_with(json(r#"{"resultSizeEstimate":0}"#))
        .mount(&server)
        .await;
    let w = p.list_window(&WindowRequest::Before { before: 2_000, horizon: 1_000, limit: 100 }).await.unwrap();
    assert!(w.threads.is_empty());
    assert_eq!(w.oldest, Some(1_000));
}

#[tokio::test]
async fn fetch_one_thread_converts_metadata_headers() {
    let (server, p, _) = setup().await;
    mount_labels(&server).await;
    p.list_labels().await.unwrap();
    Mock::given(method("GET"))
        .and(path(format!("{USER}/threads/18c2a7f3e9b0d001")))
        .and(query_param("format", "metadata"))
        .and(query_param("metadataHeaders", "List-Unsubscribe-Post"))
        .and(query_param("metadataHeaders", "Authentication-Results"))
        .respond_with(json(THREAD))
        .expect(1)
        .mount(&server)
        .await;
    let before = p.quota().available(Lane::Sync).await;
    let t = p.fetch_threads(&ids(&["18c2a7f3e9b0d001"]), FetchDepth::Metadata).await.unwrap();
    let spent = before - p.quota().available(Lane::Sync).await;
    assert!((39.0..41.0).contains(&spent), "threads.get metadata costs 40 units, spent {spent}");
    assert_eq!(t.len(), 1);
    let t = &t[0];
    assert_eq!((t.provider_thread_id.as_str(), t.provider_version), ("18c2a7f3e9b0d001", Some(4_821_907)));
    let m = &t.messages[0];
    assert_eq!(m.from, Some(Addr::new(Some("Grace Hopper"), "grace@example.org")));
    assert_eq!(m.to, vec![Addr::new(Some("Ada Lovelace"), "ada@example.com"), Addr::new(None, "team@example.com")]);
    assert_eq!(m.cc, vec![Addr::new(Some("Jürgen Müller"), "juergen@example.net")]);
    assert_eq!(m.subject, "Q3 plan — budget");
    assert_eq!(m.snippet, "Hi Ada, here's the Q3 plan & the budget sheet we discussed. Let me know what you think.");
    assert_eq!(m.date, 1_790_000_000_000, "internalDate wins over the Date header");
    assert_eq!(m.rfc_message_id.as_deref(), Some("CAF1q3plan.0001@mail.example.org"));
    assert_eq!(m.in_reply_to.as_deref(), Some("prev.0000@mail.example.org"));
    assert_eq!(m.references, vec!["root.0000@mail.example.org".to_owned(), "prev.0000@mail.example.org".to_owned()]);
    assert_eq!(m.list_id.as_deref(), Some("planning.lists.example.org"));
    assert_eq!(
        m.list_unsubscribe,
        Some(ListUnsubscribe {
            http: Some("https://lists.example.org/u/abc123".into()),
            mailto: Some("mailto:unsub@lists.example.org?subject=unsubscribe".into()),
            one_click: true
        })
    );
    assert_eq!(m.auth, Some(AuthSummary { spf: "pass".into(), dkim: "pass".into(), dmarc: "pass".into() }));
    assert!(m.has_attachments, "multipart/mixed");
    assert_eq!(m.size_bytes, Some(48_213));
    assert_eq!(m.sys, vec![SysLabel::Inbox, SysLabel::Important, SysLabel::Unread, SysLabel::Snoozed]);
    assert!(!m.is_draft);
    let reply = &t.messages[1];
    assert_eq!(reply.sys, vec![SysLabel::Sent]);
    assert!(!reply.has_attachments);

    // Into the Convex ThreadUpsert, through the shared conversion.
    let mut map = LabelMap::new();
    for (id, key) in [
        ("INBOX", "sys:inbox"),
        ("SENT", "sys:sent"),
        ("UNREAD", "sys:unread"),
        ("IMPORTANT", "sys:important"),
        ("Label_1", "l:work"),
        ("Label_2", "sys:snoozed"),
        ("CATEGORY_PERSONAL", "l:cat_personal"),
    ] {
        map.insert(id.into(), key.into());
    }
    let up = crate::convert::thread_upsert(t, &map, &["ada@example.com".into()], true);
    assert_eq!(
        up.messages[0].label_keys,
        vec!["l:cat_personal", "l:work", "sys:important", "sys:inbox", "sys:snoozed", "sys:unread"]
    );
    assert!(up.messages[0].is_unread && !up.messages[0].is_outbound);
    assert!(up.messages[1].is_outbound);
    assert_eq!(up.provider_version.map(|v| v.0), Some(4_821_907));
}

#[tokio::test]
async fn fetch_threads_batches_and_skips_deleted_threads() {
    let (server, p, _) = setup().await;
    Mock::given(method("POST"))
        .and(path("/batch/gmail/v1"))
        .and(header("authorization", "Bearer tok-1"))
        .and(body_string_contains(
            "GET /gmail/v1/users/me/threads/18c2a7f3e9b0d001?format=metadata&metadataHeaders=From",
        ))
        .and(body_string_contains("GET /gmail/v1/users/me/threads/gone?format=metadata"))
        .and(body_string_contains("Content-ID: <item3>"))
        .respond_with(batch_response(&[
            (200, THREAD),
            (404, r#"{"error":{"code":404,"message":"Requested entity was not found.","status":"NOT_FOUND"}}"#),
            (200, THREAD_SMALL),
        ]))
        .expect(1)
        .mount(&server)
        .await;
    let t = p
        .fetch_threads(&ids(&["18c2a7f3e9b0d001", "gone", "18c2b00000000003", "gone"]), FetchDepth::Metadata)
        .await
        .unwrap();
    assert_eq!(
        t.iter().map(|t| t.provider_thread_id.as_str()).collect::<Vec<_>>(),
        vec!["18c2a7f3e9b0d001", "18c2b00000000003"]
    );
    let receipt = &t[1].messages[0];
    assert_eq!(receipt.provider_label_ids, vec!["CATEGORY_UPDATES".to_owned(), "INBOX".to_owned()]);
    assert_eq!(receipt.to, vec![Addr::new(None, "ada@example.com")]);
    assert!(!receipt.has_attachments);
}

#[tokio::test]
async fn fetch_threads_splits_into_batches_of_the_configured_size() {
    let server = MockServer::start().await;
    let mut cfg = config(&server);
    cfg.batch_size = 2;
    let p = GmailProvider::new(
        cfg,
        std::sync::Arc::new(CountingTokens::default()),
        std::sync::Arc::new(AccountQuota::standalone()),
        reqwest::Client::new(),
    );
    Mock::given(method("POST"))
        .and(path("/batch/gmail/v1"))
        .respond_with(batch_response(&[(200, THREAD_SMALL), (200, THREAD_SMALL)]))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path(format!("{USER}/threads/t5")))
        .respond_with(json(THREAD_SMALL))
        .expect(1)
        .mount(&server)
        .await;
    let t = p.fetch_threads(&ids(&["t1", "t2", "t5"]), FetchDepth::Metadata).await.unwrap();
    assert_eq!(t.len(), 3);
}

#[tokio::test]
async fn fetch_threads_surfaces_rate_limits_and_drains_the_bucket() {
    let (server, p, _) = setup().await;
    Mock::given(method("POST"))
        .and(path("/batch/gmail/v1"))
        .respond_with(batch_response(&[
            (200, THREAD_SMALL),
            (429, r#"{"error":{"code":429,"message":"Too many concurrent requests for user.","errors":[{"reason":"rateLimitExceeded"}]}}"#),
        ]))
        .mount(&server)
        .await;
    let err = p.fetch_threads(&ids(&["a", "b"]), FetchDepth::Metadata).await.unwrap_err();
    assert_eq!(err, ProviderError::RateLimited { retry_after: None, scope: LimitScope::Account });
    assert!(p.quota().available(Lane::Sync).await < 1.0);
}

#[tokio::test]
async fn full_depth_uses_format_full() {
    let (server, p, _) = setup().await;
    let thread = format!(r#"{{"id":"18c2a7f3e9b0d001","historyId":"4821850","messages":[{MESSAGE_FULL}]}}"#);
    Mock::given(method("GET"))
        .and(path(format!("{USER}/threads/18c2a7f3e9b0d001")))
        .and(query_param("format", "full"))
        .respond_with(json(&thread))
        .mount(&server)
        .await;
    let t = p.fetch_threads(&ids(&["18c2a7f3e9b0d001"]), FetchDepth::Full).await.unwrap();
    assert!(t[0].messages[0].has_attachments);
}

#[tokio::test]
async fn fetch_raw_decodes_base64url() {
    let (server, p, _) = setup().await;
    let raw = "From: a@example.com\r\nSubject: ÿ?>\r\n\r\nbody ~~~ >>> ???\r\n";
    Mock::given(method("GET"))
        .and(path(format!("{USER}/messages/m1")))
        .and(query_param("format", "raw"))
        .respond_with(json(&format!(r#"{{"id":"m1","threadId":"t1","raw":"{}"}}"#, encode_b64(raw.as_bytes()))))
        .mount(&server)
        .await;
    let before = p.quota().available(Lane::Interactive).await;
    let s = p.fetch_raw(&RemoteMessageRef { provider_message_id: "m1".into(), imap: vec![] }).await.unwrap();
    assert_eq!(drain(s).await, raw.as_bytes());
    assert!(before - p.quota().available(Lane::Interactive).await >= 59.0, "raw costs 60 interactive units");
}

#[tokio::test]
async fn fetch_part_by_section_path() {
    let (server, p, _) = setup().await;
    Mock::given(method("GET"))
        .and(path(format!("{USER}/messages/18c2a7f3e9b0d001")))
        .and(query_param("format", "full"))
        .respond_with(json(MESSAGE_FULL))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path(format!("{USER}/messages/18c2a7f3e9b0d001/attachments/ANGjdJ8_synthetic_attachment_id_0001")))
        .respond_with(json(&format!(r#"{{"size":8,"data":"{}"}}"#, encode_b64(b"%PDF-1.7"))))
        .expect(1)
        .mount(&server)
        .await;
    let m = RemoteMessageRef { provider_message_id: "18c2a7f3e9b0d001".into(), imap: vec![] };
    assert_eq!(drain(p.fetch_part(&m, "1.1").await.unwrap()).await, b"Hi, Ada! Q3 plan");
    assert_eq!(drain(p.fetch_part(&m, "1.2").await.unwrap()).await, b"<p>Hi, Ada!</p><br>");
    assert_eq!(drain(p.fetch_part(&m, "2").await.unwrap()).await, b"%PDF-1.7");
}

#[tokio::test]
async fn fetch_part_falls_back_to_the_raw_source() {
    let (server, p, _) = setup().await;
    Mock::given(method("GET"))
        .and(path(format!("{USER}/messages/18c2a7f3e9b0d001")))
        .and(query_param("format", "full"))
        .respond_with(json(MESSAGE_FULL))
        .mount(&server)
        .await;
    let raw = concat!(
        "From: a@example.com\r\nTo: b@example.com\r\nSubject: fwd\r\nMIME-Version: 1.0\r\n",
        "Content-Type: multipart/mixed; boundary=\"outer\"\r\n\r\n",
        "--outer\r\nContent-Type: text/plain\r\n\r\nsee attached\r\n",
        "--outer\r\nContent-Type: text/plain; name=\"a.txt\"\r\nContent-Disposition: attachment; filename=\"a.txt\"\r\n\r\nalpha\r\n",
        "--outer\r\nContent-Type: message/rfc822\r\nContent-Disposition: attachment; filename=\"forwarded.eml\"\r\n\r\n",
        "From: c@example.com\r\nSubject: inner\r\n\r\ninner\r\n",
        "--outer--\r\n"
    );
    Mock::given(method("GET"))
        .and(path(format!("{USER}/messages/18c2a7f3e9b0d001")))
        .and(query_param("format", "raw"))
        .respond_with(json(&format!(r#"{{"id":"18c2a7f3e9b0d001","raw":"{}"}}"#, encode_b64(raw.as_bytes()))))
        .mount(&server)
        .await;
    let m = RemoteMessageRef { provider_message_id: "18c2a7f3e9b0d001".into(), imap: vec![] };
    // Section 3 is a message/rfc822 part with children in Gmail's tree: resolved from raw.
    let got = drain(p.fetch_part(&m, "3").await.unwrap()).await;
    assert!(String::from_utf8_lossy(&got).contains("Subject: inner"));
    assert_eq!(p.fetch_part(&m, "9").await.err(), Some(ProviderError::NotFound));
}

#[tokio::test]
async fn search_translates_the_query() {
    let (server, p, _) = setup().await;
    mount_labels(&server).await;
    Mock::given(method("GET"))
        .and(path(format!("{USER}/threads")))
        .and(query_param("q", "from:grace@example.org label:work has:attachment \"budget sheet\""))
        .and(query_param("pageToken", "n1"))
        .respond_with(json(r#"{"threads":[{"id":"t-hit"}],"nextPageToken":"n2"}"#))
        .mount(&server)
        .await;
    let ast = akatsuki_search::parse("from:grace@example.org label:work has:attachment \"budget sheet\"", 0, "UTC").ast;
    let (hits, next) = p.search(&ast, Some("n1".into())).await.unwrap();
    assert_eq!(hits, ids(&["t-hit"]));
    assert_eq!(next.as_deref(), Some("n2"));
    let bad = akatsuki_search::parse("is:needs_reply", 0, "UTC").ast;
    assert!(matches!(p.search(&bad, None).await, Err(ProviderError::Unsupported(_))));
    let unknown = akatsuki_search::parse("label:l:k17abc", 0, "UTC").ast;
    assert!(matches!(p.search(&unknown, None).await, Err(ProviderError::Invalid(_))));
}

#[tokio::test]
async fn watch_with_a_topic() {
    let (server, p, _) = setup().await;
    assert_eq!(p.caps().push, PushKind::GmailPubSub);
    Mock::given(method("POST"))
        .and(path(format!("{USER}/watch")))
        .and(body_json(serde_json::json!({
            "topicName": "projects/akatsuki-test/topics/gmail-push",
            "labelIds": ["INBOX", "SENT", "DRAFT", "TRASH", "SPAM", "STARRED"],
            "labelFilterBehavior": "include"
        })))
        .respond_with(json(r#"{"historyId":"4821907","expiration":"1790604800000"}"#))
        .expect(1)
        .mount(&server)
        .await;
    let w = p.watch(None, "https://sync.akatsuki.vc").await.unwrap().unwrap();
    assert_eq!(
        (w.kind, w.expires_at, w.external_id.clone(), w.client_state.clone()),
        (WatchKind::GmailWatch, 1_790_604_800_000, None, None)
    );
    Mock::given(method("POST"))
        .and(path(format!("{USER}/stop")))
        .respond_with(ResponseTemplate::new(204))
        .expect(1)
        .mount(&server)
        .await;
    p.unwatch(&w).await.unwrap();
}

#[tokio::test]
async fn watch_falls_back_to_polling() {
    let (server, p, _) = setup().await;
    Mock::given(method("POST"))
        .and(path(format!("{USER}/watch")))
        .respond_with(gmail_error(403, "forbidden", "Error sending test message to Cloud PubSub projects/akatsuki-test/topics/gmail-push : User not authorized to perform this action."))
        .mount(&server)
        .await;
    assert_eq!(p.watch(None, "").await.unwrap(), None);

    // Without a topic no request is made and the caps say poll.
    let server2 = MockServer::start().await;
    let mut cfg = config(&server2);
    cfg.pubsub_topic = None;
    let polled = GmailProvider::new(
        cfg,
        std::sync::Arc::new(CountingTokens::default()),
        std::sync::Arc::new(AccountQuota::standalone()),
        reqwest::Client::new(),
    );
    assert_eq!(polled.caps().push, PushKind::Poll { every: Duration::from_secs(60) });
    assert_eq!(polled.poll_interval(), Duration::from_secs(60));
    assert_eq!(polled.watch(None, "").await.unwrap(), None);
    assert!(server2.received_requests().await.unwrap().is_empty());

    // Rate limits are not mistaken for "push unavailable".
    let server3 = MockServer::start().await;
    let p3 = GmailProvider::new(
        config(&server3),
        std::sync::Arc::new(CountingTokens::default()),
        std::sync::Arc::new(AccountQuota::standalone()),
        reqwest::Client::new(),
    );
    Mock::given(method("POST"))
        .and(path(format!("{USER}/watch")))
        .respond_with(gmail_error(429, "rateLimitExceeded", "slow down"))
        .mount(&server3)
        .await;
    assert!(matches!(p3.watch(None, "").await, Err(ProviderError::RateLimited { .. })));
}

#[tokio::test]
async fn a_401_is_retried_once_with_a_fresh_token() {
    let (server, p, tokens) = setup().await;
    Mock::given(method("GET"))
        .and(path(format!("{USER}/profile")))
        .and(header("authorization", "Bearer tok-1"))
        .respond_with(gmail_error(401, "authError", "Invalid Credentials"))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path(format!("{USER}/profile")))
        .and(header("authorization", "Bearer tok-2"))
        .respond_with(json(r#"{"emailAddress":"ada@example.com","historyId":"7"}"#))
        .expect(1)
        .mount(&server)
        .await;
    assert_eq!(p.baseline().await.unwrap(), Cursor::Gmail { history_id: 7 });
    assert_eq!(tokens.invalidated.load(Ordering::SeqCst), 1);

    Mock::given(method("GET"))
        .and(path(format!("{USER}/labels")))
        .respond_with(gmail_error(401, "authError", "Invalid Credentials"))
        .expect(2)
        .mount(&server)
        .await;
    assert_eq!(p.list_labels().await.unwrap_err(), ProviderError::Auth { reauth_required: true });
}

#[tokio::test]
async fn server_errors_are_transient_with_retry_after() {
    let (server, p, _) = setup().await;
    Mock::given(method("GET"))
        .and(path(format!("{USER}/profile")))
        .respond_with(gmail_error(503, "backendError", "Backend Error").insert_header("retry-after", "12"))
        .mount(&server)
        .await;
    match p.baseline().await.unwrap_err() {
        ProviderError::Transient(m, ra) => {
            assert!(m.starts_with("gmail 503 backendError"));
            assert_eq!(ra, Some(Duration::from_secs(12)));
        }
        e => panic!("{e:?}"),
    }
}

#[tokio::test]
async fn revoke_posts_the_token() {
    let (server, p, _) = setup().await;
    Mock::given(method("POST"))
        .and(path("/revoke"))
        .and(body_string_contains("token=tok-1"))
        .respond_with(ResponseTemplate::new(200))
        .expect(1)
        .mount(&server)
        .await;
    p.revoke().await.unwrap();
}

#[tokio::test]
async fn sweep_and_send_as() {
    let (server, p, _) = setup().await;
    Mock::given(method("GET"))
        .and(path(format!("{USER}/messages")))
        .and(query_param("q", "has:attachment after:1782224000"))
        .respond_with(json(r#"{"messages":[{"id":"m1","threadId":"t1"}],"nextPageToken":"x"}"#))
        .mount(&server)
        .await;
    let (m, next) = p.attachment_sweep(1_782_224_000_000, None).await.unwrap();
    assert_eq!((m, next.as_deref()), (vec!["m1".to_owned()], Some("x")));
    Mock::given(method("GET"))
        .and(path(format!("{USER}/settings/sendAs")))
        .respond_with(json(r#"{"sendAs":[{"sendAsEmail":"ada@example.com","displayName":"Ada Lovelace","isPrimary":true,"isDefault":true},{"sendAsEmail":"ada@work.example","verificationStatus":"accepted"}]}"#))
        .mount(&server)
        .await;
    let s = p.send_as().await.unwrap();
    assert!(s[0].is_primary && s[1].verification_status.as_deref() == Some("accepted"));
}
