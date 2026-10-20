//! Handler tests with services: Convex is a `MockTransport`, the provider a `FakeProvider`, the
//! user-scoped Convex calls a fake, OAuth and remote images a local `wiremock` server.

use std::collections::{BTreeMap, HashMap};
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};

use akatsuki_convex_api::{Convex, MockTransport, WorkerApi};
use akatsuki_core::auth::{AuthError, VerifiedUser};
use akatsuki_core::convex::worker::AssignedAccount;
use akatsuki_core::convex::worker::secret_aad;
use akatsuki_core::convex::{AccountCaps, AccountStatus, DraftState, Provider, SecretKind};
use akatsuki_core::ids::AccountId;
use akatsuki_core::sync::ActorCmd;
use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use base64::Engine as _;
use http_body_util::BodyExt;
use serde_json::{Value, json};
use tokio::sync::mpsc;
use tower::ServiceExt;

use crate::api::Services;
use crate::config::{Config, Key32};
use crate::crypto::SecretKeyring;
use crate::engine::fake::FakeProvider;
use crate::engine::{AccountHandle, ActorMsg, Registry};
use crate::fetch::SafeFetcher;
use crate::http::{AppState, VerifyUser, router};
use crate::metrics::Metrics;
use crate::oauth::{Endpoints, OAuthClient, OAuthKind};
use crate::providers::AdapterImapProbe;
use crate::push_auth::PushVerifier;
use crate::spool::Spool;
use crate::user_convex::{DraftMeta, IdentityView, UserCallError, UserConvex};

struct Verifier;

impl VerifyUser for Verifier {
    fn verify<'a>(
        &'a self,
        token: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<VerifiedUser, AuthError>> + Send + 'a>> {
        Box::pin(async move {
            match token {
                "good" => Ok(VerifiedUser { user_id: "user_1".into(), session_id: None }),
                "other" => Ok(VerifiedUser { user_id: "user_2".into(), session_id: None }),
                _ => Err(AuthError::Malformed),
            }
        })
    }
}

#[derive(Default)]
struct FakeUser {
    draft: Mutex<Option<DraftMeta>>,
    discards: Mutex<Vec<String>>,
}

#[async_trait::async_trait]
impl UserConvex for FakeUser {
    async fn draft_get(&self, jwt: &str, _id: &str) -> Result<Option<DraftMeta>, UserCallError> {
        assert_eq!(jwt, "good", "user calls carry the caller's own JWT");
        Ok(self.draft.lock().unwrap().clone())
    }
    async fn draft_discard(&self, _jwt: &str, _op: &str, id: &str) -> Result<(), UserCallError> {
        self.discards.lock().unwrap().push(id.to_owned());
        Ok(())
    }
    async fn identities(&self, _jwt: &str, account_id: &AccountId) -> Result<Vec<IdentityView>, UserCallError> {
        Ok(vec![IdentityView {
            account_id: account_id.clone(),
            email: "me@example.com".into(),
            name: Some("Me".into()),
            reply_to: None,
            signature_html: Some("<p>Cheers, Me</p>".into()),
            is_default: true,
        }])
    }
}

const KEY: &str = "AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8="; // bytes 0..32, test only

fn key() -> [u8; 32] {
    std::array::from_fn(|i| i as u8)
}

struct Env {
    state: AppState,
    mock: Arc<Convex<MockTransport>>,
    provider: Arc<FakeProvider>,
    rx: mpsc::UnboundedReceiver<ActorMsg>,
    user: Arc<FakeUser>,
    keyring: Arc<SecretKeyring>,
    _spool: tempfile::TempDir,
}

fn account() -> AssignedAccount {
    AssignedAccount {
        account_id: "acc00001".into(),
        user_id: "user_1".into(),
        provider: Provider::Gmail,
        provider_account_id: "me@example.com".into(),
        email: "me@example.com".into(),
        status: AccountStatus::Live,
        entitled: true,
        server_triage: false,
        imap: None,
        caps: AccountCaps { multi_label: true, categories: true, server_search: true, delete_forever: false },
    }
}

fn env_with(oauth: HashMap<OAuthKind, Arc<OAuthClient>>, push: Option<Arc<PushVerifier>>) -> Env {
    let vars: BTreeMap<String, String> = [
        ("CORS_ORIGINS", "https://app.akatsuki.vc"),
        ("IMG_PROXY_KEY", KEY),
        ("LINK_SIGNING_KEY", KEY),
        ("STATE_KEY", KEY),
        ("READ_RECEIPT_KEY", KEY),
        ("SPOOL_KEY", KEY),
        ("PUBLIC_BASE_URL", "https://sync.akatsuki.vc"),
    ]
    .iter()
    .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
    .collect();
    let config = Config::from_map(&vars).unwrap();
    let mock = Arc::new(Convex::new(MockTransport::new()));
    let provider = Arc::new(FakeProvider::gmail());
    let registry = Arc::new(Registry::default());
    let (tx, rx) = mpsc::unbounded_channel();
    registry.insert(
        "acc00001".into(),
        Arc::new(AccountHandle {
            account: std::sync::RwLock::new(account()),
            provider: provider.clone(),
            tx,
            last_manual_sync: Mutex::new(None),
            last_interactive_at: Mutex::new(None),
        }),
    );
    let spool_dir = tempfile::tempdir().unwrap();
    let user = Arc::new(FakeUser::default());
    let keyring = Arc::new(SecretKeyring::new(vec![("k1".into(), Key32([7; 32]))]));
    let services = Arc::new(Services::new(
        mock.clone() as Arc<dyn WorkerApi>,
        registry,
        keyring.clone(),
        oauth,
        reqwest::Client::new(),
        SafeFetcher::allowing_loopback(),
        Some(Spool::new(spool_dir.path(), key())),
        user.clone(),
        Arc::new(AdapterImapProbe::default()),
        push,
        Arc::new(Metrics::default()),
    ));
    let state = AppState::new(config, Some(Arc::new(Verifier))).with_services(services);
    Env { state, mock, provider, rx, user, keyring, _spool: spool_dir }
}

fn env() -> Env {
    env_with(HashMap::new(), None)
}

impl Env {
    fn respond(&self, f: &str, v: Value) {
        self.mock.transport().respond(f, Ok(v));
    }

    fn entitled(&self, receipts: bool) {
        self.respond(
            "worker:userContext",
            json!({"entitlement": {"state": "active", "hostedSync": true, "managedAi": true, "until": 0},
                   "prefs": {"remoteImages": "block", "readReceipts": {"enabled": receipts}},
                   "accountIds": ["acc00001"]}),
        );
    }

    fn resolves(&self, message: &str, is_draft: bool) {
        self.respond(
            "worker:resolveMessages",
            json!([{"messageId": message, "accountId": "acc00001", "threadId": "thr00001",
                    "providerMessageId": format!("p-{message}"), "providerThreadId": "pt1", "isDraft": is_draft}]),
        );
    }

    async fn call(&self, req: Request<Body>) -> (StatusCode, axum::http::HeaderMap, Vec<u8>) {
        let resp = router(self.state.clone()).oneshot(req).await.unwrap();
        let status = resp.status();
        let headers = resp.headers().clone();
        (status, headers, resp.into_body().collect().await.unwrap().to_bytes().to_vec())
    }
}

fn authed(method: &str, uri: &str, body: Option<Value>) -> Request<Body> {
    let mut b = Request::builder().method(method).uri(uri).header(header::AUTHORIZATION, "Bearer good");
    match body {
        Some(v) => {
            b = b.header(header::CONTENT_TYPE, "application/json");
            b.body(Body::from(v.to_string())).unwrap()
        }
        None => b.body(Body::empty()).unwrap(),
    }
}

fn js(b: &[u8]) -> Value {
    serde_json::from_slice(b).unwrap_or_else(|_| panic!("not json: {}", String::from_utf8_lossy(b)))
}

const RAW: &str = "From: Ada <ada@example.com>\r\n\
To: me@example.com\r\n\
Subject: Hello\r\n\
Message-ID: <orig1@example.com>\r\n\
References: <r0@example.com>\r\n\
List-Unsubscribe: <https://list.example.com/u/1>\r\n\
List-Unsubscribe-Post: List-Unsubscribe=One-Click\r\n\
MIME-Version: 1.0\r\n\
Content-Type: multipart/mixed; boundary=\"b1\"\r\n\
\r\n\
--b1\r\n\
Content-Type: text/html; charset=utf-8\r\n\
\r\n\
<p>Hi<script>alert(1)</script><img src=\"https://cdn.example.com/a.png\" width=\"300\" height=\"200\"></p>\r\n\
--b1\r\n\
Content-Type: application/pdf; name=\"report.pdf\"\r\n\
Content-Disposition: attachment; filename=\"report.pdf\"\r\n\
Content-Transfer-Encoding: base64\r\n\
\r\n\
JVBERi0xLjcKMTIzNDU2Nzg5MA==\r\n\
--b1--\r\n";

// ───────────── bodies and parts ─────────────

#[tokio::test]
async fn sanitized_body_strips_scripts_and_proxies_images() {
    let e = env();
    e.entitled(false);
    e.resolves("msg00001", false);
    e.provider.st().raw.insert("p-msg00001".into(), RAW.as_bytes().to_vec());
    let (s, h, b) = e.call(authed("GET", "/v1/messages/msg00001", None)).await;
    assert_eq!(s, StatusCode::OK, "{}", String::from_utf8_lossy(&b));
    assert_eq!(h.get(header::CACHE_CONTROL).unwrap(), "private, no-store");
    let body = js(&b);
    let html = body["html"].as_str().unwrap();
    assert!(!html.contains("<script") && !html.contains("alert(1)"));
    assert!(html.contains("data-akatsuki-src=\"https://sync.akatsuki.vc/v1/img/"));
    assert!(!html.contains(" src=\"https://cdn.example.com"));
    assert_eq!(body["attachments"][0]["filename"], "report.pdf");
    assert_eq!(body["headers"]["listUnsubscribe"]["oneClick"], true);

    // Raw for desktop, same ownership check (cached).
    let (s, h, b) = e.call(authed("GET", "/v1/messages/msg00001?format=raw", None)).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(h.get(header::CONTENT_TYPE).unwrap(), "message/rfc822");
    assert_eq!(b, RAW.as_bytes());
}

#[tokio::test]
async fn messages_of_other_users_do_not_exist() {
    let e = env();
    e.entitled(false);
    e.respond("worker:resolveMessages", json!([]));
    let (s, _, b) = e.call(authed("GET", "/v1/messages/msg00009", None)).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    assert_eq!(js(&b)["error"]["code"], "not_found");
}

#[tokio::test]
async fn lapsed_users_get_402() {
    let e = env();
    e.respond(
        "worker:userContext",
        json!({"entitlement": {"state": "lapsed", "hostedSync": false, "managedAi": false, "until": 0},
               "prefs": {"remoteImages": "block", "readReceipts": {"enabled": false}}, "accountIds": []}),
    );
    let (s, _, b) = e.call(authed("GET", "/v1/messages/msg00001", None)).await;
    assert_eq!(s, StatusCode::PAYMENT_REQUIRED);
    assert_eq!(js(&b)["error"]["code"], "not_entitled");
}

#[tokio::test]
async fn parts_serve_ranges_with_safe_headers_and_links_work_once_signed() {
    let e = env();
    e.entitled(false);
    e.resolves("msg00001", false);
    e.provider.st().raw.insert("p-msg00001".into(), RAW.as_bytes().to_vec());
    let mut req = authed("GET", "/v1/messages/msg00001/parts/2", None);
    req.headers_mut().insert(header::RANGE, "bytes=0-3".parse().unwrap());
    let (s, h, b) = e.call(req).await;
    assert_eq!(s, StatusCode::PARTIAL_CONTENT, "{}", String::from_utf8_lossy(&b));
    assert_eq!(b, b"%PDF");
    assert_eq!(h.get(header::CONTENT_TYPE).unwrap(), "application/octet-stream");
    assert_eq!(h.get(header::CONTENT_DISPOSITION).unwrap(), "attachment; filename*=UTF-8''report%2Epdf");
    assert_eq!(h.get(header::CONTENT_SECURITY_POLICY).unwrap(), "sandbox");
    assert_eq!(h.get(header::X_CONTENT_TYPE_OPTIONS).unwrap(), "nosniff");
    assert_eq!(h.get(header::CONTENT_RANGE).unwrap(), "bytes 0-3/19");

    let (s, _, b) = e.call(authed("POST", "/v1/links", Some(json!({"messageId": "msg00001", "partId": "2"})))).await;
    assert_eq!(s, StatusCode::OK);
    let url = js(&b)["url"].as_str().unwrap().to_owned();
    assert!(url.starts_with("/v1/l/"));
    let (s, _, b) = e.call(Request::get(&url).body(Body::empty()).unwrap()).await;
    assert_eq!(s, StatusCode::OK);
    assert!(b.starts_with(b"%PDF-1.7"));
    let (s, _, _) = e.call(Request::get(format!("{url}x")).body(Body::empty()).unwrap()).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn thread_bodies_stream_ndjson_newest_first() {
    let e = env();
    e.entitled(false);
    e.respond(
        "worker:resolveThread",
        json!({"accountId": "acc00001", "providerThreadId": "pt1", "messages": [
            {"messageId": "msg00001", "providerMessageId": "p-old", "date": 1, "isDraft": false},
            {"messageId": "msg00002", "providerMessageId": "p-new", "date": 2, "isDraft": false}
        ]}),
    );
    e.provider.st().raw.insert("p-old".into(), RAW.as_bytes().to_vec());
    e.provider.st().raw.insert("p-new".into(), RAW.replace("Hello", "Newer").into_bytes());
    let (s, h, b) = e.call(authed("GET", "/v1/threads/thr00001/bodies", None)).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(h.get(header::CONTENT_TYPE).unwrap(), "application/x-ndjson");
    let lines: Vec<Value> = String::from_utf8(b).unwrap().lines().map(|l| serde_json::from_str(l).unwrap()).collect();
    assert_eq!(lines.len(), 2);
    assert_eq!(lines[0]["messageId"], "msg00002");
    assert_eq!(lines[1]["messageId"], "msg00001");
}

// ───────────── uploads, drafts, send ─────────────

#[tokio::test]
async fn uploads_are_spooled_and_attached_drafts_compose_with_fixed_message_id() {
    let mut e = env();
    e.entitled(true);
    let req = Request::post("/v1/uploads")
        .header(header::AUTHORIZATION, "Bearer good")
        .header(header::CONTENT_TYPE, "text/plain")
        .header("x-filename", "notes%20v2.txt")
        .body(Body::from("attached text"))
        .unwrap();
    let (s, _, b) = e.call(req).await;
    assert_eq!(s, StatusCode::CREATED, "{}", String::from_utf8_lossy(&b));
    let up = js(&b);
    let upload_id = up["uploadId"].as_str().unwrap().to_owned();
    assert_eq!(up["size"], 13);

    e.respond("worker:recordDraft", json!({"draftId": "drf00001", "version": 1}));
    let draft = json!({
        "accountId": "acc00001", "identityEmail": "Me@Example.com", "to": [{"email": "bob@example.com"}],
        "cc": [], "bcc": [], "subject": "Plan", "html": "<p>Hi <script>x()</script>Bob</p>", "text": "Hi Bob",
        "attachments": [{"uploadId": upload_id}], "readReceipt": true
    });
    let (s, _, b) = e
        .call(authed(
            "PUT",
            "/v1/drafts/0192f0c4-7b51-7cc3-a3c5-6a8b1f2d3e4f",
            Some(json!({"baseVersion": 0, "draft": draft, "deviceId": "dev1"})),
        ))
        .await;
    assert_eq!(s, StatusCode::OK, "{}", String::from_utf8_lossy(&b));
    let saved = js(&b);
    assert_eq!(saved["version"], 1);
    let rfc = saved["rfcMessageId"].as_str().unwrap().to_owned();
    assert!(rfc.ends_with("@example.com"));
    let (draft_id, mime_rfc, bytes) = e.provider.st().drafts[0].clone();
    assert_eq!(saved["providerDraftId"], draft_id);
    assert_eq!(mime_rfc, rfc);
    let text = String::from_utf8_lossy(&bytes);
    assert!(text.contains(&format!("Message-ID: <{rfc}>")));
    assert!(!text.contains("<script"), "outgoing HTML is sanitized again");
    assert!(text.contains("notes v2.txt"));
    assert!(text.contains("https://sync.akatsuki.vc/r/"), "opted-in read receipt pixel");
    let record = e.mock.transport().calls().into_iter().find(|c| c.function == "worker:recordDraft").unwrap();
    assert_eq!(record.args["baseVersion"], 0);
    assert_eq!(record.args["rfcMessageId"], rfc);
    // The opted-in pixel's token goes to Convex with the draft (D-412).
    assert_eq!(record.args["readReceiptToken"].as_str().map(str::len), Some(22));

    // The upload stays for the next save of the same draft (an autosave, the send, undo-send, D-416).
    assert_eq!(std::fs::read_dir(e._spool.path()).unwrap().count(), 1);
    *e.user.draft.lock().unwrap() = Some(DraftMeta {
        id: "drf00001".into(),
        account_id: "acc00001".into(),
        client_draft_id: "0192f0c4-7b51-7cc3-a3c5-6a8b1f2d3e4f".into(),
        thread_id: None,
        reply_to_message_id: None,
        provider_draft_id: Some(draft_id.clone()),
        provider_message_id: None,
        rfc_message_id: rfc.clone(),
        version: 1,
        subject: "Plan".into(),
        to: vec![],
        state: DraftState::Editing,
    });
    e.respond("worker:recordDraft", json!({"draftId": "drf00001", "version": 2}));
    let (s, _, b) = e
        .call(authed(
            "PUT",
            "/v1/drafts/0192f0c4-7b51-7cc3-a3c5-6a8b1f2d3e4f",
            Some(json!({"baseVersion": 1, "draft": draft, "deviceId": "dev1"})),
        ))
        .await;
    assert_eq!(s, StatusCode::OK, "a second save attaches the upload again: {}", String::from_utf8_lossy(&b));
    assert!(String::from_utf8_lossy(&e.provider.st().drafts.last().unwrap().2).contains("notes v2.txt"));

    // A stale baseVersion is a 409 that carries the current version, before any provider write.
    *e.user.draft.lock().unwrap() = Some(DraftMeta {
        id: "drf00001".into(),
        account_id: "acc00001".into(),
        client_draft_id: "0192f0c4-7b51-7cc3-a3c5-6a8b1f2d3e4f".into(),
        thread_id: None,
        reply_to_message_id: None,
        provider_draft_id: Some(draft_id),
        provider_message_id: None,
        rfc_message_id: rfc.clone(),
        version: 4,
        subject: "Plan".into(),
        to: vec![],
        state: DraftState::Editing,
    });
    let (s, _, b) = e
        .call(authed(
            "PUT",
            "/v1/drafts/0192f0c4-7b51-7cc3-a3c5-6a8b1f2d3e4f",
            Some(json!({"baseVersion": 3, "draft": {"accountId": "acc00001", "identityEmail": "me@example.com", "to": [{"email": "bob@example.com"}], "cc": [], "bcc": [], "subject": "Plan", "html": null, "text": "x", "attachments": []}, "deviceId": "d"})),
        ))
        .await;
    assert_eq!(s, StatusCode::CONFLICT);
    assert_eq!(js(&b)["error"]["code"], "draft_version_conflict");
    assert_eq!(js(&b)["error"]["version"], 4);
    assert_eq!(e.provider.st().drafts.len(), 1);
    let _ = &mut e.rx;
}

#[tokio::test]
async fn drafts_must_send_from_an_identity_of_the_account() {
    let e = env();
    e.entitled(false);
    let draft = json!({"accountId": "acc00001", "identityEmail": "ceo@other.example", "to": [{"email": "b@x.com"}],
        "cc": [], "bcc": [], "subject": "S", "html": null, "text": "t", "attachments": []});
    let (s, _, b) = e
        .call(authed(
            "PUT",
            "/v1/drafts/0192f0c4-7b51-7cc3-a3c5-6a8b1f2d3e4f",
            Some(json!({"baseVersion": 0, "draft": draft, "deviceId": "d"})),
        ))
        .await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    assert_eq!(js(&b)["error"]["code"], "invalid_argument");
    assert!(e.provider.st().drafts.is_empty());
}

#[tokio::test]
async fn send_saves_then_schedules_the_undo_window() {
    let e = env();
    e.entitled(false);
    e.respond("worker:recordDraft", json!({"draftId": "drf00001", "version": 1}));
    e.respond("worker:scheduleSend", json!({"sendId": "snd00001", "state": "scheduled"}));
    let before = akatsuki_core::time::now_ms();
    let draft = json!({"accountId": "acc00001", "identityEmail": "me@example.com", "to": [{"email": "b@x.com"}],
        "cc": [], "bcc": [], "subject": "S", "html": null, "text": "t", "attachments": []});
    let (s, _, b) = e
        .call(authed(
            "POST",
            "/v1/send",
            Some(json!({"clientSendId": "0192f0c4-0000-7000-8000-000000000001", "clientDraftId": "0192f0c4-7b51-7cc3-a3c5-6a8b1f2d3e4f",
                "baseVersion": 0, "draft": draft, "undoSeconds": 10, "deviceId": "d"})),
        ))
        .await;
    assert_eq!(s, StatusCode::ACCEPTED, "{}", String::from_utf8_lossy(&b));
    assert_eq!(js(&b)["state"], "scheduled");
    let calls = e.mock.transport().calls();
    let order: Vec<&str> =
        calls.iter().map(|c| c.function.as_str()).filter(|f| f.contains("Draft") || f.contains("Send")).collect();
    assert_eq!(order, vec!["worker:recordDraft", "worker:scheduleSend"], "the draft is durable before scheduling");
    let sched = calls.iter().find(|c| c.function == "worker:scheduleSend").unwrap();
    assert_eq!(sched.args["kind"], "undo");
    let at = sched.args["sendAt"].as_i64().unwrap();
    assert!(at >= before + 10_000 && at < before + 12_000);
    assert_eq!(e.provider.st().drafts.len(), 1);
}

#[tokio::test]
async fn deleting_a_draft_goes_through_convex() {
    let e = env();
    e.entitled(false);
    let (s, _, _) = e.call(authed("DELETE", "/v1/drafts/0192f0c4-7b51-7cc3-a3c5-6a8b1f2d3e4f", None)).await;
    assert_eq!(s, StatusCode::ACCEPTED);
    assert_eq!(e.user.discards.lock().unwrap().as_slice(), ["0192f0c4-7b51-7cc3-a3c5-6a8b1f2d3e4f"]);
}

// ───────────── connect (OAuth) ─────────────

#[tokio::test]
async fn oauth_connect_start_and_callback_seal_the_refresh_token() {
    use wiremock::matchers::{body_string_contains, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/token"))
        .and(body_string_contains("grant_type=authorization_code"))
        .respond_with(ResponseTemplate::new(200).set_body_json(
            json!({"access_token": "at", "expires_in": 3600, "refresh_token": "rt-secret", "scope": "s"}),
        ))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/gmail/v1/users/me/profile"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"emailAddress": "Me@Example.com"})))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/gmail/v1/users/me/settings/sendAs"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"sendAs": []})))
        .mount(&server)
        .await;
    let ep = Endpoints {
        google_token: format!("{}/token", server.uri()),
        gmail_api: format!("{}/gmail/v1", server.uri()),
        ..Endpoints::default()
    };
    let client =
        Arc::new(OAuthClient::new(OAuthKind::Google, "cid".into(), "csecret".into(), "https://sync.akatsuki.vc", ep));
    let e = env_with(HashMap::from([(OAuthKind::Google, client)]), None);
    e.entitled(false);
    let (s, _, b) = e
        .call(authed("POST", "/v1/connect/google/start", Some(json!({"returnTo": "https://app.akatsuki.vc/settings"}))))
        .await;
    assert_eq!(s, StatusCode::OK, "{}", String::from_utf8_lossy(&b));
    let authorize = url::Url::parse(js(&b)["authorizeUrl"].as_str().unwrap()).unwrap();
    let q: HashMap<String, String> = authorize.query_pairs().into_owned().collect();
    assert_eq!(q["code_challenge_method"], "S256");
    let st = q["state"].clone();

    assert!(!st.contains("vvvv") && !authorize.as_str().contains("code_verifier"), "the verifier stays in the worker");

    let (s, h, _) =
        e.call(Request::get(format!("/oauth/google/callback?code=abc&state={st}")).body(Body::empty()).unwrap()).await;
    assert_eq!(s, StatusCode::FOUND);
    // The callback creates nothing: it hands the browser a grant handle (D-395).
    let location = url::Url::parse(h.get(header::LOCATION).unwrap().to_str().unwrap()).unwrap();
    assert_eq!(location.path(), "/settings");
    let back: HashMap<String, String> = location.query_pairs().into_owned().collect();
    assert_eq!(back["status"], "pending");
    let grant = back["grant"].clone();
    assert!(!e.mock.transport().calls().iter().any(|c| c.function == "worker:createAccount"));

    // The state is single use: the same callback again is refused.
    let (s, _, _) =
        e.call(Request::get(format!("/oauth/google/callback?code=abc&state={st}")).body(Body::empty()).unwrap()).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);

    e.respond("worker:createAccount", json!({"accountId": "acc00077", "reconnected": false}));
    let (s, _, b) = e.call(authed("POST", "/v1/connect/complete", Some(json!({"grant": grant})))).await;
    assert_eq!(s, StatusCode::CREATED, "{}", String::from_utf8_lossy(&b));
    assert_eq!(js(&b), json!({"accountId": "acc00077", "reconnected": false}));
    let create = e.mock.transport().calls().into_iter().find(|c| c.function == "worker:createAccount").unwrap();
    assert_eq!(create.args["userId"], "user_1");
    assert_eq!(create.args["providerAccountId"], "me@example.com");
    assert!(create.args.get("reauthAccountId").is_none());
    assert!(!create.args.to_string().contains("rt-secret"), "only ciphertext reaches Convex");
    let b64 = |v: &Value| base64::engine::general_purpose::STANDARD.decode(v["$bytes"].as_str().unwrap()).unwrap();
    let aad = secret_aad("user_1", Provider::Gmail, "me@example.com", SecretKind::Oauth);
    let plain = e
        .keyring
        .open("k1", &b64(&create.args["secret"]["nonce"]), &b64(&create.args["secret"]["ciphertext"]), &aad)
        .unwrap();
    assert!(String::from_utf8(plain).unwrap().contains("rt-secret"));
    // A grant is used once.
    let (s, _, _) = e.call(authed("POST", "/v1/connect/complete", Some(json!({"grant": grant})))).await;
    assert_eq!(s, StatusCode::NOT_FOUND);

    // A forged or replayed-for-another-provider state is refused without a redirect.
    let (s, _, _) =
        e.call(Request::get("/oauth/google/callback?code=abc&state=forged").body(Body::empty()).unwrap()).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    let (s, _, _) = e
        .call(Request::get(format!("/oauth/microsoft/callback?code=abc&state={st}")).body(Body::empty()).unwrap())
        .await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
}

/// The mock Google endpoints for one code exchange and profile read.
async fn google_mock() -> (wiremock::MockServer, Arc<OAuthClient>) {
    use wiremock::matchers::{body_string_contains, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/token"))
        .and(body_string_contains("grant_type=authorization_code"))
        .respond_with(ResponseTemplate::new(200).set_body_json(
            json!({"access_token": "at", "expires_in": 3600, "refresh_token": "rt-victim", "scope": "s"}),
        ))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/gmail/v1/users/me/profile"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"emailAddress": "victim@example.com"})))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/gmail/v1/users/me/settings/sendAs"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"sendAs": []})))
        .mount(&server)
        .await;
    let ep = Endpoints {
        google_token: format!("{}/token", server.uri()),
        gmail_api: format!("{}/gmail/v1", server.uri()),
        ..Endpoints::default()
    };
    let client =
        Arc::new(OAuthClient::new(OAuthKind::Google, "cid".into(), "csecret".into(), "https://sync.akatsuki.vc", ep));
    (server, client)
}

#[tokio::test]
async fn a_flow_started_by_one_user_cannot_be_finished_by_another() {
    // Attacker (user_1) starts a connect and sends the consent link to a victim. The victim approves;
    // their browser lands on the app with the grant, signed in as user_2.
    let (_server, client) = google_mock().await;
    let e = env_with(HashMap::from([(OAuthKind::Google, client)]), None);
    e.entitled(false);
    let (_, _, b) = e
        .call(authed(
            "POST",
            "/v1/connect/google/start",
            Some(json!({"returnTo": "https://app.akatsuki.vc/onboarding"})),
        ))
        .await;
    let authorize = url::Url::parse(js(&b)["authorizeUrl"].as_str().unwrap()).unwrap();
    let st = authorize.query_pairs().find(|(k, _)| k == "state").unwrap().1.into_owned();
    let (_, h, _) =
        e.call(Request::get(format!("/oauth/google/callback?code=abc&state={st}")).body(Body::empty()).unwrap()).await;
    let location = url::Url::parse(h.get(header::LOCATION).unwrap().to_str().unwrap()).unwrap();
    let grant = location.query_pairs().find(|(k, _)| k == "grant").unwrap().1.into_owned();

    let mut victim = authed("POST", "/v1/connect/complete", Some(json!({"grant": grant})));
    victim.headers_mut().insert(header::AUTHORIZATION, "Bearer other".parse().unwrap());
    let (s, _, b) = e.call(victim).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{}", String::from_utf8_lossy(&b));
    assert_eq!(js(&b)["error"]["code"], "invalid_argument");
    assert!(!e.mock.transport().calls().iter().any(|c| c.function == "worker:createAccount"));
    // The refused attempt burned the grant: the starter cannot use it either.
    assert!(!e.state.services.as_ref().unwrap().grant_pending(&grant));
    let (s, _, _) = e.call(authed("POST", "/v1/connect/complete", Some(json!({"grant": grant})))).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    assert!(!e.mock.transport().calls().iter().any(|c| c.function == "worker:createAccount"));
}

#[tokio::test]
async fn a_reauth_grant_names_the_account_it_reconnects() {
    let (_server, client) = google_mock().await;
    let e = env_with(HashMap::from([(OAuthKind::Google, client)]), None);
    e.entitled(false);
    let (s, _, b) = e
        .call(authed(
            "POST",
            "/v1/connect/google/start",
            Some(json!({"returnTo": "https://app.akatsuki.vc/onboarding", "accountId": "acc00001"})),
        ))
        .await;
    assert_eq!(s, StatusCode::OK);
    let authorize = url::Url::parse(js(&b)["authorizeUrl"].as_str().unwrap()).unwrap();
    let st = authorize.query_pairs().find(|(k, _)| k == "state").unwrap().1.into_owned();
    let (_, h, _) =
        e.call(Request::get(format!("/oauth/google/callback?code=abc&state={st}")).body(Body::empty()).unwrap()).await;
    let location = url::Url::parse(h.get(header::LOCATION).unwrap().to_str().unwrap()).unwrap();
    let grant = location.query_pairs().find(|(k, _)| k == "grant").unwrap().1.into_owned();
    e.respond("worker:createAccount", json!({"accountId": "acc00001", "reconnected": true}));
    let (s, _, b) = e.call(authed("POST", "/v1/connect/complete", Some(json!({"grant": grant})))).await;
    assert_eq!(s, StatusCode::CREATED);
    assert_eq!(js(&b)["reconnected"], true);
    let create = e.mock.transport().calls().into_iter().find(|c| c.function == "worker:createAccount").unwrap();
    assert_eq!(create.args["reauthAccountId"], "acc00001", "Convex refuses a grant for another mailbox");
}

#[tokio::test]
async fn connect_start_requires_entitlement_and_configuration() {
    let e = env();
    let (s, _, _) =
        e.call(authed("POST", "/v1/connect/google/start", Some(json!({"returnTo": "https://app.akatsuki.vc/"})))).await;
    assert_eq!(s, StatusCode::SERVICE_UNAVAILABLE, "no OAuth client configured");
}

// ───────────── hooks ─────────────

#[tokio::test]
async fn gmail_push_is_verified_and_hints_the_actor() {
    use crate::push_auth::tests::{StaticKeys, push_claims, rsa_key, sign};
    let (pem, set) = rsa_key("g1");
    let verifier = Arc::new(PushVerifier::new(Arc::new(StaticKeys(set)), "aud-1".into(), "push@sa.example".into()));
    let mut e = env_with(HashMap::new(), Some(verifier));
    e.respond("worker:accountsForPush", json!([{"accountId": "acc00001"}]));
    let data =
        base64::engine::general_purpose::STANDARD.encode(br#"{"emailAddress":"me@example.com","historyId":12345}"#);
    let body = json!({"message": {"data": data, "messageId": "1"}, "subscription": "projects/x/subscriptions/y"});
    let token = sign(&pem, "g1", push_claims("aud-1", "push@sa.example", true));
    let req = Request::post("/hooks/gmail")
        .header(header::AUTHORIZATION, format!("Bearer {token}"))
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(body.to_string()))
        .unwrap();
    let (s, _, _) = e.call(req).await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    assert!(matches!(e.rx.try_recv(), Ok(ActorMsg::Cmd(ActorCmd::PushHint { history_id: Some(12345) }))));

    let bad = sign(&pem, "g1", push_claims("aud-other", "push@sa.example", true));
    let req = Request::post("/hooks/gmail")
        .header(header::AUTHORIZATION, format!("Bearer {bad}"))
        .body(Body::from(body.to_string()))
        .unwrap();
    let (s, _, _) = e.call(req).await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);
    assert!(e.rx.try_recv().is_err());
}

const SUB: &str = "0f8fad5b-d9cb-469f-a165-70867728950e";

fn graph_batch(path: &str, items: Value) -> Request<Body> {
    Request::post(path)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(json!({ "value": items }).to_string()))
        .unwrap()
}

#[tokio::test]
async fn graph_notifications_need_the_client_state() {
    let mut e = env();
    let svc = crate::api::services(&e.state.services).unwrap().clone();
    svc.registry.graph.set(&"acc00001".into(), SUB, &crate::engine::actor::client_state_hash("cs-secret"));
    let one = |cs: &str| json!([{"subscriptionId": SUB, "clientState": cs, "changeType": "created"}]);
    let (s, _, _) = e.call(graph_batch("/hooks/graph", one("cs-secret"))).await;
    assert_eq!(s, StatusCode::ACCEPTED);
    assert!(matches!(e.rx.try_recv(), Ok(ActorMsg::Cmd(ActorCmd::PushHint { history_id: None }))));
    let (s, _, _) = e.call(graph_batch("/hooks/graph", one("guess"))).await;
    assert_eq!(s, StatusCode::ACCEPTED);
    assert!(e.rx.try_recv().is_err(), "a wrong clientState is ignored");
    let lifecycle =
        json!([{"subscriptionId": SUB, "clientState": "cs-secret", "lifecycleEvent": "reauthorizationRequired"}]);
    let (s, _, _) = e.call(graph_batch("/hooks/graph/lifecycle", lifecycle)).await;
    assert_eq!(s, StatusCode::ACCEPTED);
    assert!(matches!(e.rx.try_recv(), Ok(ActorMsg::Lifecycle(crate::engine::Lifecycle::Reauthorize))));
    // Several notifications for one subscription in a batch: one delta hint.
    let (s, _, _) = e
        .call(graph_batch(
            "/hooks/graph",
            json!([
                {"subscriptionId": SUB, "clientState": "cs-secret"}, {"subscriptionId": SUB, "clientState": "cs-secret"}
            ]),
        ))
        .await;
    assert_eq!(s, StatusCode::ACCEPTED);
    assert!(e.rx.try_recv().is_ok());
    assert!(e.rx.try_recv().is_err());
    // The actor stopped: its subscription is forgotten.
    svc.registry.remove(&"acc00001".into());
    assert!(svc.registry.graph.get(SUB).is_none());
}

#[tokio::test]
async fn forged_graph_batches_never_reach_convex() {
    let mut e = env();
    let svc = crate::api::services(&e.state.services).unwrap().clone();
    svc.registry.graph.set(&"acc00001".into(), SUB, &crate::engine::actor::client_state_hash("cs-secret"));
    // Unknown GUIDs, non-GUID ids and missing clientState: answered without a single Convex call
    // (the mock transport has no scripted answers and would fail the request if one were made).
    let items: Vec<Value> = (0..50)
        .map(|i| match i % 3 {
            0 => json!({"subscriptionId": format!("00000000-0000-4000-8000-{i:012}"), "clientState": "x"}),
            1 => json!({"subscriptionId": format!("x{i}"), "clientState": "x"}),
            _ => json!({"subscriptionId": SUB}),
        })
        .collect();
    let (s, _, _) = e.call(graph_batch("/hooks/graph", Value::Array(items))).await;
    assert_eq!(s, StatusCode::ACCEPTED);
    assert!(e.rx.try_recv().is_err());
    assert_eq!(e.mock.transport().calls().len(), 0, "no Convex call for a forged batch");
    // More than GRAPH_BATCH_MAX notifications: refused before any work.
    let big: Vec<Value> = (0..51).map(|_| json!({"subscriptionId": SUB, "clientState": "cs-secret"})).collect();
    let (s, _, _) = e.call(graph_batch("/hooks/graph", Value::Array(big))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    assert!(e.rx.try_recv().is_err());
}

// ───────────── image proxy and read receipts ─────────────

#[tokio::test]
async fn image_proxy_fetches_checks_magic_bytes_and_caches() {
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};
    let server = MockServer::start().await;
    let png = [0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a, 0, 0, 0, 0];
    Mock::given(method("GET"))
        .and(path("/a.png"))
        .respond_with(
            ResponseTemplate::new(200).insert_header("content-type", "text/html").set_body_bytes(png.to_vec()),
        )
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/b.svg"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "image/svg+xml")
                .set_body_string("<svg onload=alert(1)>"),
        )
        .mount(&server)
        .await;
    let e = env();
    let signer = akatsuki_mime::ProxySigner::new("https://sync.akatsuki.vc", &key(), "user_1");
    let path_of = |u: &str| signer.sign(u).strip_prefix("https://sync.akatsuki.vc").unwrap().to_owned();
    let (s, h, b) =
        e.call(Request::get(path_of(&format!("{}/a.png", server.uri()))).body(Body::empty()).unwrap()).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(b, png);
    assert_eq!(h.get(header::CONTENT_TYPE).unwrap(), "image/png", "type from magic bytes, not the header");
    assert_eq!(h.get(header::CACHE_CONTROL).unwrap(), "private, max-age=86400");
    assert_eq!(h.get(header::CONTENT_SECURITY_POLICY).unwrap(), "default-src 'none'; sandbox");
    let (s, _, _) =
        e.call(Request::get(path_of(&format!("{}/b.svg", server.uri()))).body(Body::empty()).unwrap()).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn read_receipt_pixel_records_valid_tokens_only() {
    let e = env();
    e.respond("worker:recordReadReceipt", Value::Null);
    let token = crate::capability::receipt_token(&key(), "user_1", "id@example.com");
    let req = Request::get(format!("/r/{token}.gif"))
        .header(header::USER_AGENT, "Mozilla/5.0 (via ggpht.com GoogleImageProxy)")
        .body(Body::empty())
        .unwrap();
    let (s, h, _) = e.call(req).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(h.get(header::CACHE_CONTROL).unwrap(), "no-store");
    for _ in 0..50 {
        if !e.mock.transport().calls().is_empty() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    let call = e.mock.transport().calls().into_iter().find(|c| c.function == "worker:recordReadReceipt").unwrap();
    assert_eq!(call.args["token"], token);
    assert_eq!(call.args["proxied"], true);
    let (s, _, _) = e.call(Request::get("/r/AAAAAAAAAAAAAAAAAAAAAA.gif").body(Body::empty()).unwrap()).await;
    assert_eq!(s, StatusCode::OK, "forged tokens still get a pixel");
    assert_eq!(e.mock.transport().calls().iter().filter(|c| c.function == "worker:recordReadReceipt").count(), 1);
}

// ───────────── search and manual sync ─────────────

#[tokio::test]
async fn search_upserts_hits_through_the_actor_and_binds_the_cursor() {
    let mut e = env();
    e.entitled(false);
    e.provider.st().window = vec!["pt1".into(), "pt2".into()];
    let mut rx = std::mem::replace(&mut e.rx, mpsc::unbounded_channel().1);
    tokio::spawn(async move {
        while let Some(m) = rx.recv().await {
            if let ActorMsg::Cmd(ActorCmd::SearchFetch(ids, reply)) = m {
                let _ = reply.send(ids.iter().map(|i| format!("thr-{}", i.0).into()).collect());
            }
        }
    });
    let (s, _, b) = e.call(authed("GET", "/v1/search?accountId=acc00001&q=from%3Aada", None)).await;
    assert_eq!(s, StatusCode::OK, "{}", String::from_utf8_lossy(&b));
    let r = js(&b);
    assert_eq!(r["threadIds"], json!(["thr-pt1", "thr-pt2"]));
    let cursor = r["nextCursor"].as_str().unwrap().to_owned();
    let (s, _, _) =
        e.call(authed("GET", &format!("/v1/search?accountId=acc00001&q=other&cursor={cursor}"), None)).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "a cursor is bound to its query");
}

#[tokio::test]
async fn provider_search_resolves_dates_in_the_users_time_zone() {
    let mut e = env();
    e.respond(
        "worker:userContext",
        json!({"entitlement": {"state": "active", "hostedSync": true, "managedAi": true, "until": 0},
               "prefs": {"remoteImages": "block", "readReceipts": {"enabled": false}},
               "accountIds": ["acc00001"], "timezone": "Asia/Tokyo"}),
    );
    e.provider.st().window = vec![];
    let _rx = std::mem::replace(&mut e.rx, mpsc::unbounded_channel().1);
    let (s, _, b) = e.call(authed("GET", "/v1/search?accountId=acc00001&q=after%3A2026-01-15", None)).await;
    assert_eq!(s, StatusCode::OK, "{}", String::from_utf8_lossy(&b));
    // Midnight in Tokyo (UTC+9), not UTC midnight: the same boundary as Convex search.threads (D-404).
    let searched = e.provider.st().searched.clone();
    assert_eq!(
        searched,
        vec![akatsuki_search::query::QueryAst::Date {
            field: akatsuki_search::query::DateField::After,
            at: 1_768_402_800_000
        }]
    );
}

#[tokio::test]
async fn manual_sync_is_debounced() {
    let mut e = env();
    e.entitled(false);
    for _ in 0..2 {
        let (s, _, _) = e.call(authed("POST", "/v1/accounts/acc00001/sync", None)).await;
        assert_eq!(s, StatusCode::ACCEPTED);
    }
    assert!(matches!(e.rx.try_recv(), Ok(ActorMsg::Cmd(ActorCmd::PushHint { history_id: None }))));
    assert!(e.rx.try_recv().is_err(), "second request within 10 s is debounced");
    // An unknown account re-reads the context once before answering 404 (D-383).
    e.entitled(false);
    let (s, _, _) = e.call(authed("POST", "/v1/accounts/acc00099/sync", None)).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn accounts_of_other_users_are_not_reachable_through_the_registry() {
    let e = env();
    e.respond(
        "worker:userContext",
        json!({"entitlement": {"state": "active", "hostedSync": true, "managedAi": true, "until": 0},
               "prefs": {"remoteImages": "block", "readReceipts": {"enabled": false}}, "accountIds": []}),
    );
    let req = Request::get("/v1/search?accountId=acc00001&q=x")
        .header(header::AUTHORIZATION, "Bearer other")
        .body(Body::empty())
        .unwrap();
    let (s, _, _) = e.call(req).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn an_account_connected_within_the_cache_ttl_is_found() {
    // D-383: the user context is cached for 60 s; an account created after it was cached (connect, then
    // compose right away) must not answer 404.
    let e = env();
    let ctx = |ids: Value| {
        json!({"entitlement": {"state": "active", "hostedSync": true, "managedAi": true, "until": 0},
               "prefs": {"remoteImages": "block", "readReceipts": {"enabled": false}}, "accountIds": ids})
    };
    // The mock answers each call once, in order: the cached read and the one re-read both see no account.
    e.respond("worker:userContext", ctx(json!([])));
    e.respond("worker:userContext", ctx(json!([])));
    let (s, _, _) = e.call(authed("POST", "/v1/accounts/acc00001/sync", None)).await;
    assert_eq!(s, StatusCode::NOT_FOUND, "not the user's account yet");
    e.respond("worker:userContext", ctx(json!(["acc00001"])));
    let (s, _, _) = e.call(authed("POST", "/v1/accounts/acc00001/sync", None)).await;
    assert_eq!(s, StatusCode::ACCEPTED, "the stale cached context is re-read once");
}

// ───────────── per-user limits (§13.4, D-397) and IMAP discovery (D-396) ─────────────

#[tokio::test]
async fn connect_starts_are_limited_to_ten_an_hour_per_user() {
    let (_server, client) = google_mock().await;
    let e = env_with(HashMap::from([(OAuthKind::Google, client)]), None);
    e.entitled(false);
    let start = || authed("POST", "/v1/connect/google/start", Some(json!({"returnTo": "https://app.akatsuki.vc/"})));
    for _ in 0..crate::limits::CONNECTS_PER_HOUR {
        assert_eq!(e.call(start()).await.0, StatusCode::OK);
    }
    let (s, h, b) = e.call(start()).await;
    assert_eq!(s, StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(js(&b)["error"]["code"], "rate_limited");
    assert!(h.get(header::RETRY_AFTER).is_some());
}

#[tokio::test]
async fn imap_discovery_answers_from_the_table_and_checks_the_address() {
    let e = env();
    let (s, _, b) = e.call(authed("GET", "/v1/connect/imap/discover?email=sam%40fastmail.com", None)).await;
    assert_eq!(s, StatusCode::OK, "{}", String::from_utf8_lossy(&b));
    assert_eq!(
        js(&b),
        json!({"imap": {"host": "imap.fastmail.com", "port": 993, "tls": "implicit"},
               "smtp": {"host": "smtp.fastmail.com", "port": 465, "tls": "implicit"},
               "username": "sam@fastmail.com", "source": "well_known", "appPasswordRequired": true})
    );
    for bad in ["nobody", "a%40localhost", "a%40192.168.1.1", "a%40printer.local"] {
        let (s, _, _) = e.call(authed("GET", &format!("/v1/connect/imap/discover?email={bad}"), None)).await;
        assert_eq!(s, StatusCode::BAD_REQUEST, "{bad}");
    }
    let (s, _, _) =
        e.call(Request::get("/v1/connect/imap/discover?email=sam%40fastmail.com").body(Body::empty()).unwrap()).await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn uploads_stop_at_the_live_file_cap() {
    let e = env();
    e.entitled(false);
    let up = || {
        Request::builder()
            .method("POST")
            .uri("/v1/uploads")
            .header(header::AUTHORIZATION, "Bearer good")
            .header(header::CONTENT_TYPE, "application/pdf")
            .header("x-filename", "a.pdf")
            .body(Body::from(vec![7u8; 1024]))
            .unwrap()
    };
    for _ in 0..crate::limits::UPLOAD_FILES_ALIVE {
        assert_eq!(e.call(up()).await.0, StatusCode::CREATED);
    }
    let (s, _, b) = e.call(up()).await;
    assert_eq!(s, StatusCode::TOO_MANY_REQUESTS, "{}", String::from_utf8_lossy(&b));
}

#[tokio::test]
async fn imap_connect_refuses_odd_ports_and_private_addresses() {
    let e = env();
    e.entitled(false);
    let body = |imap_host: &str, imap_port: u16, smtp_port: u16| {
        json!({"email": "me@example.com", "username": "me", "password": "not-real",
               "imap": {"host": imap_host, "port": imap_port, "tls": "implicit"},
               "smtp": {"host": "smtp.example.com", "port": smtp_port, "tls": "implicit"}})
    };
    // Port scanning through the probe: only mail ports.
    let (s, _, b) = e.call(authed("POST", "/v1/connect/imap", Some(body("imap.example.com", 6379, 465)))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{}", String::from_utf8_lossy(&b));
    let (s, _, _) = e.call(authed("POST", "/v1/connect/imap", Some(body("imap.example.com", 993, 8080)))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    // Internal addresses look exactly like a host that does not answer.
    for host in ["127.0.0.1", "172.17.0.1", "10.1.2.3", "169.254.169.254"] {
        let (s, _, b) = e.call(authed("POST", "/v1/connect/imap", Some(body(host, 993, 465)))).await;
        assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY, "{host}");
        assert_eq!(js(&b)["error"]["code"], "unreachable", "{host}");
    }
}
