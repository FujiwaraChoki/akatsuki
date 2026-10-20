//! Microsoft Graph adapter against a wiremock Graph and identity platform. All JSON is synthetic
//! (made-up ids, addresses and subjects shaped like real Graph responses); no network, no keys.

use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use akatsuki_providers::graph::{
    GraphConfig, GraphProvider, GraphTokenSource, Limits, MicrosoftOAuth, OAuthConfig, RefreshTokenStore,
    client_state_hash, verify_client_state,
};
use akatsuki_providers::{
    AccessToken, Cursor, FetchDepth, GraphFolderCursor, LabelKind, LabelRole, LabelTarget, LimitScope, MailProvider,
    OpEffect, OutgoingMime, ProviderError, ProviderOp, RemoteDraftRef, RemoteMessageRef, RemoteThreadId,
    RemoteThreadRef, SysLabel, TokenSource, WatchKind, WatchState, WindowRequest,
};
use async_trait::async_trait;
use base64::Engine as _;
use bytes::Bytes;
use futures::StreamExt;
use serde_json::{Value, json};
use wiremock::matchers::{body_string_contains, header, method, path, query_param};
use wiremock::{Mock, MockServer, Request, Respond, ResponseTemplate};

/// 2026-10-20T00:00:00Z.
const NOW: i64 = 1_792_454_400_000;

// ---------------------------------------------------------------------------------------------
// Harness

#[derive(Default)]
struct StaticTokens {
    invalidations: Mutex<u32>,
}

#[async_trait]
impl TokenSource for StaticTokens {
    async fn access_token(&self) -> Result<AccessToken, ProviderError> {
        Ok(AccessToken { token: "test-token".into(), expires_at: NOW + 3_600_000 })
    }
    async fn invalidate(&self) {
        *self.invalidations.lock().unwrap() += 1;
    }
}

fn config(server: &MockServer) -> GraphConfig {
    GraphConfig {
        base_url: format!("{}/v1.0", server.uri()),
        self_addresses: vec!["me@example.test".into()],
        limits: Limits { max_concurrent: 4, max_rps: 1000.0, max_inline_wait: Duration::from_secs(2) },
        ..GraphConfig::default()
    }
}

fn provider_with(cfg: GraphConfig) -> (GraphProvider, Arc<StaticTokens>) {
    let tokens = Arc::new(StaticTokens::default());
    let p = GraphProvider::with_http(reqwest::Client::new(), cfg, tokens.clone()).with_clock(Arc::new(|| NOW));
    (p, tokens)
}

fn provider(server: &MockServer) -> GraphProvider {
    provider_with(config(server)).0
}

/// One `$batch` sub-request as the fake Graph saw it.
#[derive(Clone, Debug)]
struct SubReq {
    method: String,
    url: String,
    headers: Value,
    body: Option<Value>,
}

impl SubReq {
    fn path(&self) -> String {
        self.url.split('?').next().unwrap_or_default().to_owned()
    }
    fn param(&self, name: &str) -> Option<String> {
        let parsed = url::Url::parse(&format!("http://x{}", self.url)).unwrap();
        parsed.query_pairs().find(|(k, _)| k == name).map(|(_, v)| v.into_owned())
    }
}

type Route = dyn Fn(&SubReq) -> (u16, Value, Option<u64>) + Send + Sync;

/// Answers `POST /$batch` by routing every sub-request through `route` and records them.
#[derive(Clone)]
struct Batch {
    route: Arc<Route>,
    log: Arc<Mutex<Vec<SubReq>>>,
}

impl Respond for Batch {
    fn respond(&self, req: &Request) -> ResponseTemplate {
        let v: Value = serde_json::from_slice(&req.body).unwrap();
        let requests = v["requests"].as_array().cloned().unwrap_or_default();
        assert!(requests.len() <= 20, "batch larger than 20");
        let mut responses = Vec::new();
        for r in requests {
            let sub = SubReq {
                method: r["method"].as_str().unwrap().to_owned(),
                url: r["url"].as_str().unwrap().to_owned(),
                headers: r["headers"].clone(),
                body: r.get("body").cloned(),
            };
            assert!(
                sub.headers["Prefer"].as_str().unwrap_or_default().contains("IdType=\"ImmutableId\""),
                "sub-request without the immutable-id preference"
            );
            self.log.lock().unwrap().push(sub.clone());
            let (status, body, ra) = (self.route)(&sub);
            let mut headers = json!({ "Content-Type": "application/json" });
            if let Some(s) = ra {
                headers["Retry-After"] = json!(s.to_string());
            }
            responses.push(json!({ "id": r["id"], "status": status, "headers": headers, "body": body }));
        }
        ResponseTemplate::new(200).set_body_json(json!({ "responses": responses }))
    }
}

async fn mount_batch(
    server: &MockServer,
    route: impl Fn(&SubReq) -> (u16, Value, Option<u64>) + Send + Sync + 'static,
) -> Arc<Mutex<Vec<SubReq>>> {
    let log = Arc::new(Mutex::new(Vec::new()));
    Mock::given(method("POST"))
        .and(path("/v1.0/$batch"))
        .and(header("authorization", "Bearer test-token"))
        .respond_with(Batch { route: Arc::new(route), log: log.clone() })
        .mount(server)
        .await;
    log
}

fn not_found() -> (u16, Value, Option<u64>) {
    (
        404,
        json!({"error": {"code": "ErrorItemNotFound", "message": "The specified object was not found in the store."}}),
        None,
    )
}

fn ok(v: Value) -> (u16, Value, Option<u64>) {
    (200, v, None)
}

// ---------------------------------------------------------------------------------------------
// Synthetic mailbox

const INBOX: &str = "AAMkADU3Inbox";
const SENT: &str = "AAMkADU3Sent";
const DRAFTS: &str = "AAMkADU3Drafts";
const TRASH: &str = "AAMkADU3Deleted";
const JUNK: &str = "AAMkADU3Junk";
const ARCHIVE: &str = "AAMkADU3Archive";
const OUTBOX: &str = "AAMkADU3Outbox";
const SYNC_ISSUES: &str = "AAMkADU3SyncIssues";
const PROJECTS: &str = "AAMkADU3Projects";
const AKATSUKI: &str = "AAMkADU3Akatsuki";
const SNOOZED: &str = "AAMkADU3Snoozed";

fn folder(id: &str, name: &str, children: u32) -> Value {
    json!({
        "id": id, "displayName": name, "parentFolderId": "AAMkADU3Root", "childFolderCount": children,
        "unreadItemCount": 0, "totalItemCount": 12, "sizeInBytes": 40960, "isHidden": false
    })
}

/// Well-known lookups, child folders and nothing else; other sub-requests go to `rest`.
fn mailbox_routes(sub: &SubReq) -> Option<(u16, Value, Option<u64>)> {
    let p = sub.path();
    let wk: HashMap<&str, &str> = [
        ("inbox", INBOX),
        ("sentitems", SENT),
        ("drafts", DRAFTS),
        ("deleteditems", TRASH),
        ("junkemail", JUNK),
        ("archive", ARCHIVE),
        ("outbox", OUTBOX),
        ("syncissues", SYNC_ISSUES),
    ]
    .into();
    if let Some(name) = p.strip_prefix("/me/mailFolders/") {
        if !name.contains('/') {
            return Some(match wk.get(name) {
                Some(id) => ok(json!({ "id": id })),
                None => not_found(),
            });
        }
        if let Some(parent) = name.strip_suffix("/childFolders") {
            let mut child = match parent {
                INBOX => folder(PROJECTS, "Projects", 0),
                AKATSUKI => folder(SNOOZED, "Snoozed", 0),
                SYNC_ISSUES => folder("AAMkADU3Conflicts", "Conflicts", 0),
                _ => return Some(ok(json!({ "value": [] }))),
            };
            child["parentFolderId"] = json!(parent);
            return Some(ok(json!({ "value": [child] })));
        }
    }
    None
}

async fn mount_mailbox(server: &MockServer) {
    Mock::given(method("GET"))
        .and(path("/v1.0/me/mailFolders"))
        .and(query_param("$top", "100"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "@odata.context": "https://graph.microsoft.com/v1.0/$metadata#users('x')/mailFolders",
            "value": [
                folder(INBOX, "Inbox", 1), folder(SENT, "Sent Items", 0), folder(DRAFTS, "Drafts", 0),
                folder(TRASH, "Deleted Items", 0), folder(JUNK, "Junk Email", 0), folder(ARCHIVE, "Archive", 0),
                folder(OUTBOX, "Outbox", 0), folder(SYNC_ISSUES, "Sync Issues", 1), folder(AKATSUKI, "Akatsuki", 1)
            ]
        })))
        .mount(server)
        .await;
    Mock::given(method("GET"))
        .and(path("/v1.0/me/outlook/masterCategories"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "value": [
                {"id": "7b4e6d3a-0000-4000-8000-000000000001", "displayName": "Work", "color": "preset15"},
                {"id": "7b4e6d3a-0000-4000-8000-000000000002", "displayName": "Personal", "color": "preset4"}
            ]
        })))
        .mount(server)
        .await;
}

fn addr(name: &str, address: &str) -> Value {
    json!({ "emailAddress": { "name": name, "address": address } })
}

#[allow(clippy::too_many_arguments)]
fn msg(
    id: &str,
    conv: &str,
    folder: &str,
    received: &str,
    read: bool,
    flagged: bool,
    cats: &[&str],
    from: &str,
) -> Value {
    json!({
        "@odata.etag": "W/\"CQAAABYAAAB\"",
        "id": id,
        "conversationId": conv,
        "internetMessageId": format!("<{id}@mail.example.test>"),
        "subject": "Quarterly planning",
        "bodyPreview": "Hi team,\r\n\r\nAttached   is the draft plan.",
        "from": addr("Ada Example", from),
        "sender": addr("Ada Example", from),
        "toRecipients": [addr("Me", "me@example.test")],
        "ccRecipients": [],
        "bccRecipients": [],
        "replyTo": [],
        "receivedDateTime": received,
        "sentDateTime": received,
        "lastModifiedDateTime": received,
        "isRead": read,
        "isDraft": false,
        "flag": { "flagStatus": if flagged { "flagged" } else { "notFlagged" } },
        "categories": cats,
        "hasAttachments": false,
        "parentFolderId": folder,
        "internetMessageHeaders": []
    })
}

// ---------------------------------------------------------------------------------------------
// OAuth

#[derive(Default)]
struct Store {
    saved: Mutex<Vec<String>>,
}

#[async_trait]
impl RefreshTokenStore for Store {
    async fn persist(&self, refresh_token: &str, _scope: Option<&str>) -> Result<(), ProviderError> {
        self.saved.lock().unwrap().push(refresh_token.to_owned());
        Ok(())
    }
}

fn oauth(server: &MockServer) -> MicrosoftOAuth {
    let mut cfg = OAuthConfig::new("00000000-1111-2222-3333-444444444444", Some("client-secret".into()));
    cfg.login_base = server.uri();
    MicrosoftOAuth::new(reqwest::Client::new(), cfg).with_clock(Arc::new(|| NOW))
}

#[tokio::test]
async fn refresh_is_single_flight_and_persists_rotated_tokens() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/common/oauth2/v2.0/token"))
        .and(body_string_contains("grant_type=refresh_token"))
        .and(body_string_contains("refresh_token=rt-1"))
        .and(body_string_contains("client_secret=client-secret"))
        .and(body_string_contains("scope=offline_access+openid+email+User.Read+Mail.ReadWrite+Mail.Send"))
        .respond_with(ResponseTemplate::new(200).set_delay(Duration::from_millis(50)).set_body_json(json!({
            "token_type": "Bearer", "scope": "Mail.ReadWrite Mail.Send User.Read", "expires_in": 3599,
            "ext_expires_in": 3599, "access_token": "at-1", "refresh_token": "rt-2"
        })))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/common/oauth2/v2.0/token"))
        .and(body_string_contains("refresh_token=rt-2"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "token_type": "Bearer", "expires_in": 3599, "access_token": "at-2", "refresh_token": "rt-2"
        })))
        .expect(1)
        .mount(&server)
        .await;

    let store = Arc::new(Store::default());
    let src = Arc::new(GraphTokenSource::new(oauth(&server), "rt-1".into(), store.clone()));
    let calls = (0..5).map(|_| {
        let s = src.clone();
        tokio::spawn(async move { s.access_token().await })
    });
    for c in calls {
        let t = c.await.unwrap().unwrap();
        assert_eq!(t.token, "at-1");
        assert_eq!(t.expires_at, NOW + 3_599_000);
    }
    assert_eq!(*store.saved.lock().unwrap(), vec!["rt-2".to_owned()]);
    assert!(!format!("{src:?}").contains("rt-"));

    // After a 401 the token is dropped and refreshed with the rotated refresh token; an
    // unchanged refresh token is not persisted again.
    src.invalidate().await;
    assert_eq!(src.access_token().await.unwrap().token, "at-2");
    assert_eq!(store.saved.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn token_errors_are_classified() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(body_string_contains("refresh_token=revoked"))
        .respond_with(ResponseTemplate::new(400).set_body_json(json!({
            "error": "invalid_grant", "error_description": "AADSTS70000: The provided grant has expired.", "error_codes": [70000]
        })))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(body_string_contains("refresh_token=busy"))
        .respond_with(
            ResponseTemplate::new(429)
                .insert_header("Retry-After", "30")
                .set_body_json(json!({"error": "too_many_requests"})),
        )
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(body_string_contains("grant_type=authorization_code"))
        .and(body_string_contains("code_verifier=verifier-123"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "token_type": "Bearer", "expires_in": 3600, "access_token": "at", "refresh_token": "rt", "id_token": "eyJ.x.y",
            "scope": "openid email Mail.ReadWrite Mail.Send User.Read"
        })))
        .mount(&server)
        .await;
    let o = oauth(&server);
    assert_eq!(o.refresh("revoked").await.unwrap_err(), ProviderError::Auth { reauth_required: true });
    assert_eq!(
        o.refresh("busy").await.unwrap_err(),
        ProviderError::RateLimited { retry_after: Some(Duration::from_secs(30)), scope: LimitScope::Account }
    );
    let g = o
        .exchange_code("code-abc", "https://sync.example.test/oauth/microsoft/callback", "verifier-123")
        .await
        .unwrap();
    assert_eq!(g.refresh_token.as_deref(), Some("rt"));
    assert_eq!(g.id_token.as_deref(), Some("eyJ.x.y"));
    assert!(!format!("{g:?}").contains("\"at\""));
}

// ---------------------------------------------------------------------------------------------
// Labels, baseline, delta

#[tokio::test]
async fn list_labels_maps_folders_categories_and_roles() {
    let server = MockServer::start().await;
    mount_mailbox(&server).await;
    let log = mount_batch(&server, |s| mailbox_routes(s).unwrap_or_else(not_found)).await;
    let p = provider(&server);
    let labels = p.list_labels().await.unwrap();
    let got: Vec<(String, String, LabelKind, Option<LabelRole>, bool)> =
        labels.iter().map(|l| (l.provider_label_id.clone(), l.name.clone(), l.kind, l.role, l.exclusive)).collect();
    let f = |id: &str, name: &str, kind, role| (id.to_owned(), name.to_owned(), kind, role, true);
    assert_eq!(
        got,
        vec![
            f(INBOX, "Inbox", LabelKind::System, Some(LabelRole::Inbox)),
            f(SENT, "Sent Items", LabelKind::System, Some(LabelRole::Sent)),
            f(DRAFTS, "Drafts", LabelKind::System, Some(LabelRole::Drafts)),
            f(TRASH, "Deleted Items", LabelKind::System, Some(LabelRole::Trash)),
            f(JUNK, "Junk Email", LabelKind::System, Some(LabelRole::Spam)),
            f(ARCHIVE, "Archive", LabelKind::Folder, Some(LabelRole::Archive)),
            f(AKATSUKI, "Akatsuki", LabelKind::Folder, None),
            f(PROJECTS, "Inbox/Projects", LabelKind::Folder, None),
            f(SNOOZED, "Akatsuki/Snoozed", LabelKind::Folder, Some(LabelRole::Snoozed)),
            ("cat:Work".into(), "Work".into(), LabelKind::Category, None, false),
            ("cat:Personal".into(), "Personal".into(), LabelKind::Category, None, false),
        ]
    );
    assert_eq!(labels[9].color.as_deref(), Some("#c11a25"));
    // Outbox and Sync Issues (with its Conflicts child) are not mail lists.
    assert!(labels.iter().all(|l| !l.name.contains("Outbox") && !l.name.contains("Conflicts")));
    // One batch of well-known lookups, then child folders of the three parents.
    {
        let log = log.lock().unwrap();
        assert!(
            log.iter().any(|s| s.path() == "/me/mailFolders/junkemail" && s.param("$select").as_deref() == Some("id"))
        );
        assert_eq!(log.iter().filter(|s| s.path().ends_with("/childFolders")).count(), 3);
    }

    // ensure_label finds the existing roles without creating anything.
    assert_eq!(p.ensure_label(LabelRole::Snoozed, "Akatsuki/Snoozed").await.unwrap().provider_label_id, SNOOZED);
    assert_eq!(p.ensure_label(LabelRole::Archive, "Archive").await.unwrap().provider_label_id, ARCHIVE);
}

fn delta_routes(base: String) -> impl Fn(&SubReq) -> (u16, Value, Option<u64>) + Send + Sync + 'static {
    move |s: &SubReq| {
        if let Some(r) = mailbox_routes(s) {
            return r;
        }
        let p = s.path();
        let Some(folder) = p.strip_prefix("/me/mailFolders/").and_then(|r| r.strip_suffix("/messages/delta")) else {
            return not_found();
        };
        assert!(s.headers["Prefer"].as_str().unwrap().contains("odata.maxpagesize=50"));
        if s.param("$skiptoken").is_some() {
            return ok(json!({
                "value": [{"id": "m-2", "conversationId": "conv-2"}],
                "@odata.deltaLink": format!("{base}/me/mailFolders/{folder}/messages/delta?$deltatoken=final-{folder}")
            }));
        }
        assert_eq!(s.param("$select").as_deref(), Some("conversationId"));
        assert_eq!(s.param("$filter").as_deref(), Some("receivedDateTime ge 2026-07-22T00:00:00Z"));
        if folder == INBOX {
            ok(json!({
                "@odata.context": "https://graph.microsoft.com/v1.0/$metadata#Collection(message)",
                "value": [{"@odata.etag": "W/\"x\"", "id": "m-1", "conversationId": "conv-1"}],
                "@odata.nextLink": format!("{base}/me/mailFolders/{folder}/messages/delta?$skiptoken=page-2")
            }))
        } else {
            ok(json!({
                "value": [],
                "@odata.deltaLink": format!("{base}/me/mailFolders/{folder}/messages/delta?$deltatoken=final-{folder}")
            }))
        }
    }
}

#[tokio::test]
async fn baseline_drains_a_windowed_delta_per_folder() {
    let server = MockServer::start().await;
    mount_mailbox(&server).await;
    let base = format!("{}/v1.0", server.uri());
    let log = mount_batch(&server, delta_routes(base.clone())).await;
    let p = provider(&server);
    let Cursor::Graph { folders } = p.baseline().await.unwrap() else { panic!("graph cursor") };
    assert_eq!(folders.len(), 9);
    for (id, c) in &folders {
        assert_eq!(
            c.delta_link.as_deref(),
            Some(format!("{base}/me/mailFolders/{id}/messages/delta?$deltatoken=final-{id}").as_str())
        );
    }
    let deltas = log.lock().unwrap().iter().filter(|s| s.path().ends_with("/messages/delta")).count();
    assert_eq!(deltas, 10, "9 folders plus one follow-up page for the inbox");
}

#[tokio::test]
async fn changes_since_separates_moves_from_deletes() {
    let server = MockServer::start().await;
    mount_mailbox(&server).await;
    let base = format!("{}/v1.0", server.uri());
    let b = base.clone();
    let log = mount_batch(&server, move |s| {
        if let Some(r) = mailbox_routes(s) {
            return r;
        }
        let p = s.path();
        match p.as_str() {
            "/me/mailFolders/AAMkADU3Inbox/messages/delta" => {
                assert_eq!(s.param("$deltatoken").as_deref(), Some("inbox-1"));
                ok(json!({
                    "value": [
                        {"id": "m-1", "conversationId": "conv-1"},
                        {"id": "m-2", "@removed": {"reason": "deleted"}},
                        {"id": "m-3", "@removed": {"reason": "deleted"}},
                        {"id": "m-4", "@removed": {"reason": "deleted"}}
                    ],
                    "@odata.nextLink": format!("{b}/me/mailFolders/{INBOX}/messages/delta?$skiptoken=inbox-2")
                }))
            }
            "/me/mailFolders/AAMkADU3Archive/messages/delta" => ok(json!({
                "value": [{"id": "m-2", "conversationId": "conv-2"}],
                "@odata.deltaLink": format!("{b}/me/mailFolders/{ARCHIVE}/messages/delta?$deltatoken=archive-2")
            })),
            p if p.ends_with("/messages/delta") => {
                let folder = p.trim_start_matches("/me/mailFolders/").trim_end_matches("/messages/delta");
                ok(json!({ "value": [], "@odata.deltaLink": format!("{b}/me/mailFolders/{folder}/messages/delta?$deltatoken=new") }))
            }
            "/me/messages/m-3" => not_found(),
            "/me/messages/m-4" => ok(json!({"id": "m-4", "conversationId": "conv-4"})),
            other => panic!("unexpected {other}"),
        }
    })
    .await;
    let p = provider(&server);
    let mut folders = BTreeMap::new();
    folders.insert(
        INBOX.to_owned(),
        GraphFolderCursor {
            delta_link: Some(format!("{base}/me/mailFolders/{INBOX}/messages/delta?$deltatoken=inbox-1")),
        },
    );
    folders.insert(
        ARCHIVE.to_owned(),
        GraphFolderCursor {
            delta_link: Some(format!("{base}/me/mailFolders/{ARCHIVE}/messages/delta?$deltatoken=archive-1")),
        },
    );
    folders.insert(
        "AAMkADU3GoneFolder".to_owned(),
        GraphFolderCursor {
            delta_link: Some(format!("{base}/me/mailFolders/AAMkADU3GoneFolder/messages/delta?$deltatoken=z")),
        },
    );

    let page = p.changes_since(&Cursor::Graph { folders }).await.unwrap();
    let ids: Vec<&str> = page.touched_threads.iter().map(|t| t.0.as_str()).collect();
    assert_eq!(ids, vec!["conv-1", "conv-2", "conv-4"]);
    assert_eq!(page.deleted_messages, vec!["m-3".to_owned()]);
    assert!(page.more, "the inbox has another page");
    assert!(page.labels_changed, "a folder appeared and one disappeared");
    let Cursor::Graph { folders: next } = page.next else { panic!() };
    assert!(next[INBOX].delta_link.as_deref().unwrap().ends_with("$skiptoken=inbox-2"));
    assert!(next[ARCHIVE].delta_link.as_deref().unwrap().ends_with("$deltatoken=archive-2"));
    assert!(next[PROJECTS].delta_link.as_deref().unwrap().ends_with("$deltatoken=new"));
    assert!(!next.contains_key("AAMkADU3GoneFolder"));
    // m-2 reappeared in Archive, so only m-3 and m-4 were looked up.
    let lookups: Vec<String> =
        log.lock().unwrap().iter().filter(|s| s.path().starts_with("/me/messages/")).map(SubReq::path).collect();
    assert_eq!(lookups, vec!["/me/messages/m-3", "/me/messages/m-4"]);
}

#[tokio::test]
async fn expired_and_foreign_delta_links() {
    let server = MockServer::start().await;
    mount_mailbox(&server).await;
    let base = format!("{}/v1.0", server.uri());
    let b = base.clone();
    mount_batch(&server, move |s| {
        mailbox_routes(s).unwrap_or_else(|| {
            if s.param("$deltatoken").as_deref() == Some("old") {
                (410, json!({"error": {"code": "SyncStateNotFound", "message": "The sync state is not found."}}), None)
            } else {
                ok(json!({ "value": [], "@odata.deltaLink": format!("{b}{}?$deltatoken=new", s.path()) }))
            }
        })
    })
    .await;
    let p = provider(&server);
    let mut folders = BTreeMap::new();
    folders.insert(
        INBOX.to_owned(),
        GraphFolderCursor { delta_link: Some(format!("{base}/me/mailFolders/{INBOX}/messages/delta?$deltatoken=old")) },
    );
    assert_eq!(
        p.changes_since(&Cursor::Graph { folders: folders.clone() }).await.unwrap_err(),
        ProviderError::CursorExpired
    );

    // A cursor whose link points away from the API base is refused before any request carries
    // the bearer token there.
    folders.insert(
        INBOX.to_owned(),
        GraphFolderCursor { delta_link: Some("https://evil.example/steal?$deltatoken=x".into()) },
    );
    assert!(matches!(p.changes_since(&Cursor::Graph { folders }).await, Err(ProviderError::Invalid(_))));
    assert!(matches!(p.changes_since(&Cursor::Gmail { history_id: 1 }).await, Err(ProviderError::Invalid(_))));
}

// ---------------------------------------------------------------------------------------------
// Threads

#[tokio::test]
async fn fetch_threads_maps_graph_messages() {
    let server = MockServer::start().await;
    mount_batch(&server, |s| {
        assert_eq!(s.path(), "/me/messages");
        assert!(s.param("$select").unwrap().contains("internetMessageHeaders"));
        match s.param("$filter").as_deref() {
            Some("conversationId eq 'conv-1'") => {
                let mut newer = msg("m-2", "conv-1", INBOX, "2026-10-19T09:30:00Z", false, true, &["Work"], "ada@example.test");
                newer["lastModifiedDateTime"] = json!("2026-10-19T10:00:00.123Z");
                newer["hasAttachments"] = json!(true);
                newer["internetMessageHeaders"] = json!([
                    {"name": "In-Reply-To", "value": "<m-1@mail.example.test>"},
                    {"name": "References", "value": "<root@mail.example.test> <m-1@mail.example.test>"},
                    {"name": "List-Id", "value": "Planning <planning.example.test>"},
                    {"name": "List-Unsubscribe", "value": "<https://example.test/u?x=1>, <mailto:u@example.test>"},
                    {"name": "List-Unsubscribe-Post", "value": "List-Unsubscribe=One-Click"},
                    {"name": "Authentication-Results", "value": "mx.example.test; spf=pass smtp.mailfrom=example.test; dkim=pass header.d=example.test; dmarc=pass header.from=example.test"}
                ]);
                let older = msg("m-1", "conv-1", SENT, "2026-10-18T08:00:00Z", true, false, &[], "me@example.test");
                ok(json!({ "value": [newer, older] }))
            }
            Some("conversationId eq 'conv-gone'") => ok(json!({ "value": [] })),
            other => panic!("unexpected filter {other:?}"),
        }
    })
    .await;
    let p = provider(&server);
    let threads = p
        .fetch_threads(&[RemoteThreadId("conv-1".into()), RemoteThreadId("conv-gone".into())], FetchDepth::Metadata)
        .await
        .unwrap();
    assert_eq!(threads.len(), 1, "a conversation without messages is left out");
    let t = &threads[0];
    assert_eq!(t.provider_thread_id, "conv-1");
    assert_eq!(t.provider_version, Some(1_792_404_000_123));
    let ids: Vec<&str> = t.messages.iter().map(|m| m.provider_message_id.as_str()).collect();
    assert_eq!(ids, vec!["m-1", "m-2"], "oldest first");
    let m = &t.messages[1];
    assert_eq!(m.rfc_message_id.as_deref(), Some("m-2@mail.example.test"));
    assert_eq!(m.in_reply_to.as_deref(), Some("m-1@mail.example.test"));
    assert_eq!(m.references, vec!["root@mail.example.test", "m-1@mail.example.test"]);
    assert_eq!(m.from.as_ref().unwrap().email, "ada@example.test");
    assert_eq!(m.to[0].email, "me@example.test");
    assert_eq!(m.snippet, "Hi team, Attached is the draft plan.");
    assert_eq!(m.provider_label_ids, vec![INBOX.to_owned(), "cat:Work".to_owned()]);
    assert_eq!(m.sys, vec![SysLabel::Unread, SysLabel::Starred]);
    assert!(m.has_attachments && !m.is_draft);
    assert_eq!(m.list_id.as_deref(), Some("planning.example.test"));
    let lu = m.list_unsubscribe.as_ref().unwrap();
    assert!(lu.one_click && lu.http.as_deref() == Some("https://example.test/u?x=1"));
    assert_eq!(m.auth.as_ref().unwrap().dmarc, "pass");
    assert_eq!(m.date, 1_792_402_200_000);
    assert!(t.messages[0].sys.is_empty());
}

// ---------------------------------------------------------------------------------------------
// Ops

fn thread_ref(conv: &str) -> RemoteThreadRef {
    RemoteThreadRef { provider_thread_id: conv.into(), messages: Vec::new() }
}

#[tokio::test]
async fn apply_batches_patches_moves_and_label_ops() {
    let server = MockServer::start().await;
    mount_mailbox(&server).await;
    Mock::given(method("POST"))
        .and(path("/v1.0/me/outlook/masterCategories"))
        .and(wiremock::matchers::body_json(json!({"displayName": "Travel", "color": "preset15"})))
        .respond_with(
            ResponseTemplate::new(201)
                .set_body_json(json!({"id": "c-3", "displayName": "Travel", "color": "preset15"})),
        )
        .expect(1)
        .mount(&server)
        .await;
    let log = mount_batch(&server, |s| {
        if let Some(r) = mailbox_routes(s) {
            return r;
        }
        if s.method == "GET" && s.path() == "/me/messages" {
            let f = s.param("$filter").unwrap();
            let v = match f.as_str() {
                "conversationId eq 'conv-1'" => json!([
                    msg("m-1", "conv-1", INBOX, "2026-10-18T08:00:00Z", false, false, &[], "ada@example.test"),
                    msg("m-2", "conv-1", INBOX, "2026-10-18T09:00:00Z", true, false, &["Work"], "ada@example.test"),
                ]),
                "conversationId eq 'conv-2'" => json!([
                    msg("m-4", "conv-2", INBOX, "2026-10-17T08:00:00Z", true, false, &[], "ada@example.test"),
                    msg("m-5", "conv-2", SENT, "2026-10-17T09:00:00Z", true, false, &[], "me@example.test"),
                ]),
                "conversationId eq 'conv-3'" => json!([
                    msg("m-6", "conv-3", INBOX, "2026-10-16T08:00:00Z", true, false, &[], "ada@example.test"),
                    msg("m-7", "conv-3", SENT, "2026-10-16T09:00:00Z", true, false, &[], "me@example.test"),
                ]),
                "conversationId eq 'conv-4'" => json!([
                    msg("m-8", "conv-4", INBOX, "2026-10-15T08:00:00Z", true, false, &[], "ada@example.test"),
                ]),
                "conversationId eq 'conv-5'" => json!([
                    msg("m-9", "conv-5", TRASH, "2026-10-14T08:00:00Z", true, false, &[], "ada@example.test"),
                    msg("m-10", "conv-5", TRASH, "2026-10-14T09:00:00Z", true, false, &[], "me@example.test"),
                ]),
                _ => json!([]),
            };
            return ok(json!({ "value": v }));
        }
        if s.method == "POST" && s.path() == "/me/messages/m-8/move" {
            return (429, json!({"error": {"code": "ApplicationThrottled", "message": "Application is over its MailboxConcurrency limit."}}), Some(120));
        }
        if s.method == "PATCH" || s.method == "POST" {
            return ok(json!({"id": s.path().split('/').nth(3).unwrap_or_default()}));
        }
        not_found()
    })
    .await;
    let p = provider(&server);
    let cat = |n: &str| LabelTarget::Label { provider_label_id: format!("cat:{n}"), exclusive: false };
    let ops = vec![
        (
            "op-modify".to_owned(),
            ProviderOp::Modify {
                thread: thread_ref("conv-1"),
                add: vec![LabelTarget::Sys(SysLabel::Starred), cat("Personal")],
                remove: vec![LabelTarget::Sys(SysLabel::Unread), cat("Work")],
            },
        ),
        (
            "op-archive".to_owned(),
            ProviderOp::Move {
                thread: thread_ref("conv-2"),
                to: LabelTarget::Label { provider_label_id: ARCHIVE.into(), exclusive: true },
            },
        ),
        ("op-trash".to_owned(), ProviderOp::Trash { thread: thread_ref("conv-3") }),
        (
            "op-gone".to_owned(),
            ProviderOp::Modify {
                thread: thread_ref("conv-missing"),
                add: vec![LabelTarget::Sys(SysLabel::Starred)],
                remove: vec![],
            },
        ),
        ("op-label".to_owned(), ProviderOp::LabelCreate { name: "Travel".into(), color: Some("#C1272D".into()) }),
        (
            "op-throttled".to_owned(),
            ProviderOp::Move { thread: thread_ref("conv-4"), to: LabelTarget::Sys(SysLabel::Spam) },
        ),
        ("op-untrash".to_owned(), ProviderOp::Untrash { thread: thread_ref("conv-5") }),
        (
            "op-important".to_owned(),
            ProviderOp::Modify {
                thread: thread_ref("conv-1"),
                add: vec![LabelTarget::Sys(SysLabel::Important)],
                remove: vec![],
            },
        ),
        (
            "op-unfold".to_owned(),
            ProviderOp::Modify {
                thread: thread_ref("conv-1"),
                add: vec![],
                remove: vec![LabelTarget::Sys(SysLabel::Inbox)],
            },
        ),
        (
            "op-rename-cat".to_owned(),
            ProviderOp::LabelRename { provider_label_id: "cat:Work".into(), name: "Job".into() },
        ),
    ];
    let results: HashMap<String, Result<OpEffect, ProviderError>> = p.apply(&ops).await.into_iter().collect();
    assert_eq!(results.len(), ops.len());

    let applied = |id: &str| match &results[id] {
        Ok(OpEffect::Applied { post, .. }) => post.clone(),
        other => panic!("{id}: {other:?}"),
    };
    assert_eq!(applied("op-modify").unwrap().provider_thread_id, "conv-1");
    assert!(applied("op-archive").is_some());
    assert!(applied("op-trash").is_some());
    assert!(applied("op-untrash").is_some());
    assert_eq!(results["op-gone"], Err(ProviderError::NotFound));
    assert_eq!(
        results["op-throttled"],
        Err(ProviderError::RateLimited { retry_after: Some(Duration::from_secs(120)), scope: LimitScope::Account })
    );
    assert_eq!(results["op-important"], Err(ProviderError::Unsupported("graph: important")));
    assert!(matches!(results["op-unfold"], Err(ProviderError::Invalid(_))));
    assert!(matches!(results["op-rename-cat"], Err(ProviderError::Unsupported(_))));
    match &results["op-label"] {
        Ok(OpEffect::Applied { created: Some(l), .. }) => {
            assert_eq!((l.provider_label_id.as_str(), l.kind, l.exclusive), ("cat:Travel", LabelKind::Category, false));
            assert_eq!(l.color.as_deref(), Some("#c11a25"));
        }
        other => panic!("{other:?}"),
    }

    let log = log.lock().unwrap();
    let writes: BTreeMap<String, Value> = log
        .iter()
        .filter(|s| s.method != "GET")
        .map(|s| (format!("{} {}", s.method, s.path()), s.body.clone().unwrap_or(Value::Null)))
        .collect();
    // Mark read touches the unread message; the star goes on the newest message; categories are
    // replaced as a whole list on every message.
    assert_eq!(writes["PATCH /me/messages/m-1"], json!({"isRead": true, "categories": ["Personal"]}));
    assert_eq!(
        writes["PATCH /me/messages/m-2"],
        json!({"flag": {"flagStatus": "flagged"}, "categories": ["Personal"]})
    );
    // Archive moves the received message and leaves the sent one in Sent Items.
    assert_eq!(writes["POST /me/messages/m-4/move"], json!({"destinationId": ARCHIVE}));
    assert!(!writes.contains_key("POST /me/messages/m-5/move"));
    // Trash takes everything.
    assert_eq!(writes["POST /me/messages/m-6/move"], json!({"destinationId": "deleteditems"}));
    assert_eq!(writes["POST /me/messages/m-7/move"], json!({"destinationId": "deleteditems"}));
    // Untrash: received mail to the inbox, own mail back to Sent Items.
    assert_eq!(writes["POST /me/messages/m-9/move"], json!({"destinationId": "inbox"}));
    assert_eq!(writes["POST /me/messages/m-10/move"], json!({"destinationId": "sentitems"}));
    assert_eq!(writes["POST /me/messages/m-8/move"], json!({"destinationId": "junkemail"}));
    // The throttled sub-request was not retried inline (120 s is longer than the inline limit).
    assert_eq!(log.iter().filter(|s| s.path() == "/me/messages/m-8/move").count(), 1);
}

// ---------------------------------------------------------------------------------------------
// Throttling and auth retries

#[tokio::test]
async fn throttling_and_auth_retries() {
    let server = MockServer::start().await;
    // Short Retry-After: retried inline.
    Mock::given(method("GET"))
        .and(path("/v1.0/me/mailFolders/inbox/messages"))
        .respond_with(
            ResponseTemplate::new(429)
                .insert_header("Retry-After", "0")
                .set_body_json(json!({"error": {"code": "TooManyRequests"}})),
        )
        .up_to_n_times(1)
        .with_priority(1)
        .mount(&server)
        .await;
    // A 401 drops the cached token once and retries.
    Mock::given(method("GET"))
        .and(path("/v1.0/me/mailFolders/inbox/messages"))
        .respond_with(
            ResponseTemplate::new(401).set_body_json(json!({"error": {"code": "InvalidAuthenticationToken"}})),
        )
        .up_to_n_times(1)
        .with_priority(2)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/v1.0/me/mailFolders/inbox/messages"))
        .and(header("authorization", "Bearer test-token"))
        .and(header("prefer", "IdType=\"ImmutableId\""))
        .and(query_param("$orderby", "receivedDateTime desc"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "value": [
                {"id": "m-1", "conversationId": "conv-1", "receivedDateTime": "2026-10-19T09:00:00Z"},
                {"id": "m-2", "conversationId": "conv-1", "receivedDateTime": "2026-10-19T08:00:00Z"},
                {"id": "m-3", "conversationId": "conv-2", "receivedDateTime": "2026-10-18T08:00:00Z"}
            ],
            "@odata.nextLink": format!("{}/v1.0/me/mailFolders/inbox/messages?$skip=50", server.uri())
        })))
        .with_priority(3)
        .mount(&server)
        .await;
    let (p, tokens) = provider_with(config(&server));
    let page = p.list_window(&WindowRequest::Inbox { page: None }).await.unwrap();
    assert_eq!(page.threads, vec![RemoteThreadId("conv-1".into()), RemoteThreadId("conv-2".into())]);
    assert_eq!(page.oldest, Some(1_792_310_400_000));
    assert!(page.next_page.unwrap().ends_with("$skip=50"));
    assert_eq!(*tokens.invalidations.lock().unwrap(), 1);

    // Long Retry-After: surfaced so the account actor pauses.
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(
            ResponseTemplate::new(429)
                .insert_header("Retry-After", "120")
                .set_body_json(json!({"error": {"code": "ApplicationThrottled"}})),
        )
        .expect(1)
        .mount(&server)
        .await;
    let p = provider(&server);
    assert_eq!(
        p.list_window(&WindowRequest::Inbox { page: None }).await.unwrap_err(),
        ProviderError::RateLimited { retry_after: Some(Duration::from_secs(120)), scope: LimitScope::Account }
    );

    // 503 without Retry-After is transient; with it, it is a rate limit.
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(query_param("$top", "50"))
        .respond_with(ResponseTemplate::new(503).set_body_json(json!({"error": {"code": "ServiceUnavailable"}})))
        .mount(&server)
        .await;
    let p = provider(&server);
    assert!(matches!(
        p.list_window(&WindowRequest::Inbox { page: None }).await,
        Err(ProviderError::Transient(_, None))
    ));
    assert!(matches!(
        p.list_window(&WindowRequest::Inbox { page: Some("https://elsewhere.example/v1.0/me/messages".into()) }).await,
        Err(ProviderError::Invalid(_))
    ));
}

#[tokio::test]
async fn backfill_window_filters_by_date() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1.0/me/messages"))
        .and(query_param(
            "$filter",
            "receivedDateTime lt 2026-07-22T00:00:00Z and receivedDateTime ge 2025-10-20T00:00:00Z",
        ))
        .and(query_param("$top", "200"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "value": [{"id": "m-1", "conversationId": "conv-9", "receivedDateTime": "2026-07-01T00:00:00Z"}],
            "@odata.nextLink": format!("{}/v1.0/me/messages?$skip=200", server.uri())
        })))
        .mount(&server)
        .await;
    let p = provider(&server);
    let w = p
        .list_window(&WindowRequest::Before { before: 1_784_678_400_000, horizon: 1_760_918_400_000, limit: 200 })
        .await
        .unwrap();
    assert_eq!(w.threads, vec![RemoteThreadId("conv-9".into())]);
    assert_eq!(w.next_page, None, "backfill pages by date, not by token");
    assert_eq!(w.oldest, Some(1_782_864_000_000));
}

// ---------------------------------------------------------------------------------------------
// Drafts and send

const MIME: &str = "From: Me <me@example.test>\r\nTo: Ada <ada@example.test>\r\nSubject: Plan\r\n\
Message-ID: <0192f0a0-0000-7000-8000-000000000001@example.test>\r\nMIME-Version: 1.0\r\n\
Content-Type: text/plain; charset=utf-8\r\n\r\nSounds good.\r\n";

#[tokio::test]
async fn mime_drafts_replace_and_send() {
    let server = MockServer::start().await;
    let b64 = base64::engine::general_purpose::STANDARD.encode(MIME);
    Mock::given(method("POST"))
        .and(path("/v1.0/me/messages"))
        .and(header("content-type", "text/plain"))
        .and(wiremock::matchers::body_string(b64))
        .respond_with(
            ResponseTemplate::new(201)
                .set_body_json(json!({"id": "draft-2", "conversationId": "conv-7", "isDraft": true})),
        )
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("DELETE"))
        .and(path("/v1.0/me/messages/draft-1"))
        .respond_with(ResponseTemplate::new(204))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/v1.0/me/messages/draft-2"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"id": "draft-2", "conversationId": "conv-7", "isDraft": true})),
        )
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/v1.0/me/messages/draft-2/send"))
        .respond_with(ResponseTemplate::new(202))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/v1.0/me/messages/draft-gone"))
        .respond_with(ResponseTemplate::new(404).set_body_json(json!({"error": {"code": "ErrorItemNotFound"}})))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/v1.0/me/messages"))
        .and(query_param("$filter", "internetMessageId eq '<0192f0a0-0000-7000-8000-000000000001@example.test>'"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"value": [
            {"id": "draft-x", "conversationId": "conv-7", "isDraft": true, "parentFolderId": DRAFTS},
            {"id": "draft-2", "conversationId": "conv-7", "isDraft": false, "parentFolderId": SENT}
        ]})))
        .mount(&server)
        .await;
    let p = provider(&server);
    let out = OutgoingMime {
        rfc_message_id: "0192f0a0-0000-7000-8000-000000000001@example.test".into(),
        bytes: Bytes::from_static(MIME.as_bytes()),
        thread: None,
    };
    let old = RemoteDraftRef { provider_draft_id: "draft-1".into(), provider_message_id: Some("draft-1".into()) };
    let d = p.save_draft(&out, Some(&old)).await.unwrap();
    assert_eq!(d, RemoteDraftRef { provider_draft_id: "draft-2".into(), provider_message_id: Some("draft-2".into()) });
    let sent = p.send_draft(&d, &out.rfc_message_id).await.unwrap();
    assert_eq!((sent.provider_message_id.as_str(), sent.provider_thread_id.as_deref()), ("draft-2", Some("conv-7")));
    let found = p.find_sent(&out.rfc_message_id).await.unwrap().unwrap();
    assert_eq!(found.provider_message_id, "draft-2", "drafts with the same Message-ID are ignored");
    let gone = RemoteDraftRef { provider_draft_id: "draft-gone".into(), provider_message_id: None };
    assert_eq!(p.send_draft(&gone, "x@y").await.unwrap_err(), ProviderError::NotFound);
}

#[tokio::test]
async fn large_drafts_reply_through_create_reply_and_upload_sessions() {
    let server = MockServer::start().await;
    let big: Vec<u8> = (0..700 * 1024).map(|i| (i % 251) as u8).collect();
    let b64 = base64::engine::general_purpose::STANDARD.encode(&big);
    let wrapped: String =
        b64.as_bytes().chunks(76).map(|c| format!("{}\r\n", std::str::from_utf8(c).unwrap())).collect();
    let mime = format!(
        "From: Me <me@example.test>\r\nTo: Ada <ada@example.test>\r\nCc: Bob <bob@example.test>\r\nSubject: Re: Plan\r\n\
Message-ID: <reply-1@example.test>\r\nIn-Reply-To: <parent-1@mail.example.test>\r\n\
References: <parent-1@mail.example.test>\r\nMIME-Version: 1.0\r\n\
Content-Type: multipart/mixed; boundary=\"b1\"\r\n\r\n--b1\r\nContent-Type: text/html; charset=utf-8\r\n\r\n\
<p>See attached.</p>\r\n--b1\r\nContent-Type: text/plain; name=\"note.txt\"\r\nContent-Disposition: attachment; filename=\"note.txt\"\r\n\r\n\
small note\r\n--b1\r\nContent-Type: application/pdf; name=\"plan.pdf\"\r\nContent-Disposition: attachment; filename=\"plan.pdf\"\r\n\
Content-Transfer-Encoding: base64\r\n\r\n{wrapped}--b1--\r\n"
    );
    Mock::given(method("GET"))
        .and(path("/v1.0/me/messages"))
        .and(query_param("$filter", "internetMessageId eq '<parent-1@mail.example.test>'"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"value": [{"id": "parent-1", "conversationId": "conv-1", "isDraft": false}]})),
        )
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/v1.0/me/messages/parent-1/createReply"))
        .respond_with(
            ResponseTemplate::new(201)
                .set_body_json(json!({"id": "draft-9", "conversationId": "conv-1", "isDraft": true})),
        )
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("PATCH"))
        .and(path("/v1.0/me/messages/draft-9"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"id": "draft-9"})))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/v1.0/me/messages/draft-9/attachments"))
        .respond_with(ResponseTemplate::new(201).set_body_json(json!({"id": "att-1"})))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/v1.0/me/messages/draft-9/attachments/createUploadSession"))
        .respond_with(ResponseTemplate::new(201).set_body_json(json!({
            "@odata.context": "https://graph.microsoft.com/v1.0/$metadata#microsoft.graph.uploadSession",
            "uploadUrl": format!("{}/upload/session-1?authtoken=opaque", server.uri()),
            "expirationDateTime": "2026-10-20T06:00:00Z", "nextExpectedRanges": ["0-"]
        })))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("PUT"))
        .and(path("/upload/session-1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"nextExpectedRanges": []})))
        .expect(3)
        .mount(&server)
        .await;
    let cfg = GraphConfig {
        mime_draft_max_bytes: 4096,
        upload_session_min_bytes: 64 * 1024,
        upload_chunk_bytes: 320 * 1024,
        ..config(&server)
    };
    let (p, _) = provider_with(cfg);
    let out = OutgoingMime {
        rfc_message_id: "reply-1@example.test".into(),
        bytes: Bytes::from(mime),
        thread: Some("conv-1".into()),
    };
    let d = p.save_draft(&out, None).await.unwrap();
    assert_eq!(d.provider_draft_id, "draft-9");

    let reqs = server.received_requests().await.unwrap();
    let patch = reqs.iter().find(|r| r.method.as_str() == "PATCH").unwrap();
    let body: Value = serde_json::from_slice(&patch.body).unwrap();
    assert_eq!(body["subject"], "Re: Plan");
    assert_eq!(body["internetMessageId"], "<reply-1@example.test>");
    assert_eq!(body["body"]["contentType"], "HTML");
    assert_eq!(body["toRecipients"][0]["emailAddress"]["address"], "ada@example.test");
    assert_eq!(body["ccRecipients"][0]["emailAddress"]["address"], "bob@example.test");
    let small = reqs.iter().find(|r| r.url.path() == "/v1.0/me/messages/draft-9/attachments").unwrap();
    let small: Value = serde_json::from_slice(&small.body).unwrap();
    assert_eq!(small["name"], "note.txt");
    assert_eq!(small["@odata.type"], "#microsoft.graph.fileAttachment");
    let session = reqs.iter().find(|r| r.url.path().ends_with("createUploadSession")).unwrap();
    let session: Value = serde_json::from_slice(&session.body).unwrap();
    assert_eq!(session["AttachmentItem"]["name"], "plan.pdf");
    assert_eq!(session["AttachmentItem"]["size"], big.len());
    let puts: Vec<_> = reqs.iter().filter(|r| r.method.as_str() == "PUT").collect();
    let ranges: Vec<&str> = puts.iter().map(|r| r.headers.get("content-range").unwrap().to_str().unwrap()).collect();
    assert_eq!(ranges, vec!["bytes 0-327679/716800", "bytes 327680-655359/716800", "bytes 655360-716799/716800"]);
    assert!(puts.iter().all(|r| r.headers.get("authorization").is_none()), "upload URLs carry their own auth");
    let uploaded: Vec<u8> = puts.iter().flat_map(|r| r.body.clone()).collect();
    assert_eq!(uploaded, big);
}

// ---------------------------------------------------------------------------------------------
// Search, bodies, push, profile

#[tokio::test]
async fn search_sends_kql() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1.0/me/mailFolders/sentitems/messages"))
        .and(query_param("$search", "\"(from:ada@example.com AND subject:\\\"quarterly plan\\\")\""))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "value": [{"id": "a", "conversationId": "c-1"}, {"id": "b", "conversationId": "c-1"}, {"id": "c", "conversationId": "c-2"}],
            "@odata.nextLink": format!("{}/v1.0/me/mailFolders/sentitems/messages?$skiptoken=s2", server.uri())
        })))
        .expect(1)
        .mount(&server)
        .await;
    let p = provider(&server);
    let q = akatsuki_search::query::parse("in:sent from:ada@example.com subject:\"quarterly plan\"", NOW, "UTC").ast;
    let (ids, next) = p.search(&q, None).await.unwrap();
    assert_eq!(ids, vec![RemoteThreadId("c-1".into()), RemoteThreadId("c-2".into())]);
    assert!(next.unwrap().ends_with("$skiptoken=s2"));
    let unread = akatsuki_search::query::parse("is:unread", NOW, "UTC").ast;
    assert_eq!(p.search(&unread, None).await.unwrap_err(), ProviderError::Unsupported("is:"));
    assert!(matches!(p.search(&unread, Some("http://evil.example/x".into())).await, Err(ProviderError::Invalid(_))));
}

const RAW: &str = "From: Ada <ada@example.test>\r\nTo: me@example.test\r\nSubject: Files\r\nMIME-Version: 1.0\r\n\
Content-Type: multipart/mixed; boundary=\"zz\"\r\n\r\n--zz\r\nContent-Type: text/plain\r\n\r\nHello\r\n--zz\r\n\
Content-Type: application/octet-stream; name=\"a.bin\"\r\nContent-Disposition: attachment; filename=\"a.bin\"\r\n\
Content-Transfer-Encoding: base64\r\n\r\nAAECAw==\r\n--zz--\r\n";

#[tokio::test]
async fn raw_source_and_parts() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1.0/me/messages/m-1/$value"))
        .respond_with(ResponseTemplate::new(200).insert_header("content-type", "text/plain").set_body_string(RAW))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/v1.0/me/messages/m-1/attachments/AAMkAtt1/$value"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(vec![9u8, 8, 7]))
        .mount(&server)
        .await;
    let p = provider(&server);
    let m = RemoteMessageRef { provider_message_id: "m-1".into(), imap: Vec::new() };
    let collect = |s: akatsuki_providers::ByteStream| async move {
        s.map(|c| c.unwrap().to_vec()).collect::<Vec<_>>().await.concat()
    };
    assert_eq!(collect(p.fetch_raw(&m).await.unwrap()).await, RAW.as_bytes());
    assert_eq!(collect(p.fetch_part(&m, "2").await.unwrap()).await, vec![0u8, 1, 2, 3]);
    assert_eq!(collect(p.fetch_part(&m, "att:AAMkAtt1").await.unwrap()).await, vec![9u8, 8, 7]);
    assert!(matches!(p.fetch_part(&m, "9").await, Err(ProviderError::NotFound)));
    assert!(matches!(p.fetch_part(&m, "../x").await, Err(ProviderError::Invalid(_))));
}

#[tokio::test]
async fn subscriptions_create_renew_recreate_and_stop() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1.0/subscriptions"))
        .and(header("prefer", "IdType=\"ImmutableId\""))
        .respond_with(|req: &Request| {
            let b: Value = serde_json::from_slice(&req.body).unwrap();
            assert_eq!(b["resource"], "me/messages");
            assert_eq!(b["changeType"], "created,updated,deleted");
            assert_eq!(b["notificationUrl"], "https://sync.example.test/hooks/graph");
            assert_eq!(b["lifecycleNotificationUrl"], "https://sync.example.test/hooks/graph/lifecycle");
            assert_eq!(b["expirationDateTime"], "2026-10-22T22:00:00Z");
            assert_eq!(b["clientState"].as_str().unwrap().len(), 43);
            ResponseTemplate::new(201).set_body_json(json!({
                "id": "sub-1", "resource": "me/messages", "changeType": "created,updated,deleted",
                "expirationDateTime": "2026-10-22T22:00:00Z", "clientState": b["clientState"]
            }))
        })
        .expect(2)
        .mount(&server)
        .await;
    Mock::given(method("PATCH"))
        .and(path("/v1.0/subscriptions/sub-1"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"id": "sub-1", "expirationDateTime": "2026-10-22T22:00:00Z"})),
        )
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("PATCH"))
        .and(path("/v1.0/subscriptions/sub-expired"))
        .respond_with(ResponseTemplate::new(404).set_body_json(json!({"error": {"code": "ResourceNotFound"}})))
        .mount(&server)
        .await;
    Mock::given(method("DELETE"))
        .and(path("/v1.0/subscriptions/sub-1"))
        .respond_with(ResponseTemplate::new(204))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/v1.0/subscriptions/sub-1/reauthorize"))
        .respond_with(ResponseTemplate::new(204))
        .expect(1)
        .mount(&server)
        .await;
    let p = provider(&server);

    let w = p.watch(None, "https://sync.example.test/").await.unwrap().unwrap();
    assert_eq!(
        (w.kind, w.external_id.as_deref(), w.expires_at),
        (WatchKind::GraphSubscription, Some("sub-1"), NOW + 4200 * 60_000)
    );
    let cs = w.client_state.clone().unwrap();
    assert!(verify_client_state(Some(&cs), &client_state_hash(&cs)));

    // Plenty of time left: nothing is sent.
    let fresh = WatchState { expires_at: NOW + 48 * 3_600_000, client_state: None, ..w.clone() };
    assert_eq!(p.watch(Some(&fresh), "https://sync.example.test").await.unwrap(), Some(fresh));
    // Less than 24 h: renewed in place, clientState unchanged.
    let due = WatchState { expires_at: NOW + 3_600_000, client_state: None, ..w.clone() };
    let renewed = p.watch(Some(&due), "https://sync.example.test").await.unwrap().unwrap();
    assert_eq!((renewed.external_id.as_deref(), renewed.client_state.as_deref()), (Some("sub-1"), None));
    // Gone at Graph: a new subscription with a new clientState.
    let expired = WatchState { external_id: Some("sub-expired".into()), expires_at: NOW, ..w.clone() };
    let again = p.watch(Some(&expired), "https://sync.example.test").await.unwrap().unwrap();
    assert!(again.client_state.is_some() && again.client_state.as_deref() != Some(cs.as_str()));

    p.reauthorize("sub-1").await.unwrap();
    p.unwatch(&w).await.unwrap();
    p.unwatch(&WatchState { external_id: None, ..w.clone() }).await.unwrap();
    p.revoke().await.unwrap();

    let (polling, _) = provider_with(GraphConfig { push: false, ..config(&server) });
    assert_eq!(polling.watch(None, "https://sync.example.test").await.unwrap(), None);
}

#[tokio::test]
async fn profile_prefers_mail_then_upn() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1.0/me"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "@odata.context": "https://graph.microsoft.com/v1.0/$metadata#users/$entity",
            "id": "00000000-0000-0000-0001-000000000001", "displayName": "Sam Example", "mail": null,
            "userPrincipalName": "sam@example.onmicrosoft.test"
        })))
        .mount(&server)
        .await;
    let prof = provider(&server).profile().await.unwrap();
    assert_eq!(prof.email, "sam@example.onmicrosoft.test");
    assert_eq!(prof.display_name.as_deref(), Some("Sam Example"));
}
