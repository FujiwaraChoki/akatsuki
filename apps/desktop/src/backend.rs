//! The desktop's service layer: the session, the Convex connection, the mirror feed (§16.3),
//! optimistic actions with the durable outbox (§16.1), bodies, search indexing, compose and BYO AI.
//!
//! It runs on a tokio runtime next to GPUI. The UI reads the mirror directly (lists render from
//! SQLite) and calls the methods here, which return at once; results arrive as [`Event`]s on a
//! channel the workspace drains. The same code serves the demo mailbox, whose [`DemoApi`] stands in
//! for Convex and the worker.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;

use futures::StreamExt;
use futures::channel::mpsc::{UnboundedReceiver, UnboundedSender, unbounded};
use serde_json::{Value, json};
use tokio::runtime::Handle;
use tokio::task::AbortHandle;

use akatsuki_ai::transcript::{RenderAddr, ThreadForTranscript, TranscriptMessage, render_transcript, strip_quoted};
use akatsuki_ai::{AaspEvent, AiClient, PromptPack, ROUTING, resolve};
use akatsuki_convex_api::{ClientApi, Convex, ConvexApiError, ConvexClientTransport};
use akatsuki_core::convex::client::{
    MailApplyArgs, MailSnoozeArgs, MailUndoArgs, RegisterDeviceArgs, SaveAiSettingsArgs, SendsCancelArgs, SplitView,
    SyncChangesArgs,
};
use akatsuki_core::convex::rows::{AiProfile, AiRoute, AiRouteFeature, AiSettings};
use akatsuki_core::convex::{AiFeature, DeviceKind};
use akatsuki_core::store::{BODY_CACHE_MAX_BYTES, FeedOutcome, Store};
use akatsuki_core::time::now_ms;
use akatsuki_mime::SANITIZER_VERSION;
use akatsuki_search::{IndexDoc, SearchContext, SearchIndex};

use crate::auth::{AuthApi, AuthError, Jwt, Loopback, Session, SocialProvider};
use crate::config::{self, Endpoints, Paths};
use crate::demo::DemoApi;
use crate::remote::{JwtCell, LiveMailServer, MailServer};
use crate::render::message::{Rendered, render_raw};
use crate::secrets::SecretStore;
use crate::vm::actions::{AccountCtx, snooze_overlay};
use crate::vm::compose::{Draft, PendingSend};
use crate::vm::mail::PlannedApply;
use crate::worker::WorkerClient;

/// How long Undo stays offered after an action (§5.2).
pub const UNDO_WINDOW_MS: i64 = 10_000;
/// Rendered bodies kept in memory.
const BODY_MEMORY: usize = 64;

/// Where the session stands.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SessionState {
    /// Not signed in.
    SignedOut {
        /// Why, when it was not the user's choice.
        reason: Option<String>,
    },
    /// Working on it.
    Connecting,
    /// Signed in.
    SignedIn {
        /// The user's address (from the JWT, display only).
        email: String,
        /// The demo mailbox.
        demo: bool,
    },
}

/// Something the UI should show or reload.
#[derive(Clone, Debug)]
pub enum Event {
    /// The session changed.
    Session(SessionState),
    /// Sign-in progress or an error, for the sign-in screen.
    SignInStatus(String),
    /// Open this URL in the system browser.
    OpenUrl(String),
    /// Threads changed in the mirror: re-read lists.
    MirrorChanged,
    /// Accounts or labels changed.
    AccountsChanged,
    /// Split tabs.
    Splits(Vec<SplitView>),
    /// AI settings.
    AiSettings(Option<AiSettings>),
    /// A message body is ready (or failed).
    Body {
        /// Message.
        message_id: String,
        /// The rendered body, or a safe error.
        result: Result<Arc<Rendered>, String>,
    },
    /// A short notice.
    Toast {
        /// Text.
        text: String,
        /// Offer Undo.
        undo: bool,
    },
    /// The draft was autosaved.
    DraftSaved {
        /// Draft.
        client_draft_id: String,
        /// New version.
        version: u64,
    },
    /// A send left the undo window or failed; `reopen` carries the draft after a successful undo.
    SendDone {
        /// The send.
        client_send_id: String,
        /// The draft to reopen (undo), if any.
        reopen: Option<Draft>,
        /// Error text, if it failed.
        error: Option<String>,
    },
    /// An attachment finished uploading (`Ok(uploadId)`) or failed.
    AttachmentDone {
        /// Draft.
        client_draft_id: String,
        /// The attachment's local key.
        key: String,
        /// Upload id, or a safe error.
        result: Result<String, String>,
    },
    /// An AI stream event for request `request`.
    Ai {
        /// The request.
        request: u64,
        /// The event.
        event: AaspEvent,
    },
    /// "Test connection" finished.
    AiTest {
        /// Profile.
        profile_id: String,
        /// `Ok` or a safe error.
        result: Result<(), String>,
    },
}

/// What can be undone with `z`.
#[derive(Clone, Debug)]
enum Undoable {
    Gesture { ops: Vec<String>, at: i64 },
    Send(PendingSend),
}

struct Remote {
    api: Arc<dyn ClientApi>,
    mail: Arc<dyn MailServer>,
    live: Option<Arc<Convex<ConvexClientTransport>>>,
    demo: bool,
}

impl Clone for Remote {
    fn clone(&self) -> Self {
        Self { api: self.api.clone(), mail: self.mail.clone(), live: self.live.clone(), demo: self.demo }
    }
}

/// The service layer.
pub struct Backend {
    rt: Handle,
    /// The mirror (lists read it directly).
    pub store: Arc<Mutex<Store>>,
    /// The FTS5 index.
    pub search: Arc<Mutex<SearchIndex>>,
    events: UnboundedSender<Event>,
    remote: RwLock<Option<Remote>>,
    jwt: JwtCell,
    secrets: Arc<dyn SecretStore>,
    endpoints: Endpoints,
    paths: Paths,
    /// This device.
    pub device_id: String,
    http: reqwest::Client,
    ai: AiClient,
    prompts: Option<Arc<PromptPack>>,
    bodies: Mutex<(HashMap<String, Arc<Rendered>>, Vec<String>)>,
    inflight: Mutex<HashSet<String>>,
    tasks: Mutex<Vec<AbortHandle>>,
    feeds: Mutex<HashSet<String>>,
    undo: Mutex<Vec<Undoable>>,
    ai_settings: RwLock<Option<AiSettings>>,
    ai_seq: Mutex<u64>,
    cache_bodies: RwLock<bool>,
}

impl std::fmt::Debug for Backend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Backend").field("device_id", &self.device_id).finish_non_exhaustive()
    }
}

fn app_error(e: &ConvexApiError) -> bool {
    matches!(e, ConvexApiError::App { .. } | ConvexApiError::Function { .. } | ConvexApiError::Decode { .. })
}

fn safe(e: &ConvexApiError) -> String {
    match e {
        ConvexApiError::App { message, .. } => message.clone(),
        ConvexApiError::Transport(_) => "Offline. Changes will be sent when the connection is back.".into(),
        _ => "Something went wrong on the server.".into(),
    }
}

/// Settings read from Convex are user data that anyone holding a session could have written:
/// they pass the same validation as on save (`validate_settings`, e.g. no `baseUrl` on the
/// managed provider), and settings that fail it count as AI off (D-415).
pub fn trusted_ai_settings(settings: Option<AiSettings>) -> Option<AiSettings> {
    let s = settings?;
    let value = serde_json::to_value(&s).ok()?;
    match akatsuki_ai::settings::validate_settings(&value) {
        Ok(_) => Some(s),
        Err(_) => None,
    }
}

impl Backend {
    /// A backend over `store` and `search`; events go to the returned receiver.
    pub fn new(
        rt: Handle,
        store: Store,
        search: SearchIndex,
        secrets: Arc<dyn SecretStore>,
        endpoints: Endpoints,
        paths: Paths,
        device_id: String,
        cache_bodies: bool,
    ) -> (Arc<Self>, UnboundedReceiver<Event>) {
        let (tx, rx) = unbounded();
        // The prompt pack is embedded in `crates/ai`; `AKATSUKI_PROMPTS_DIR` points at a checkout
        // of packages/prompts instead (prompt development).
        let prompts = match std::env::var_os("AKATSUKI_PROMPTS_DIR") {
            Some(dir) => PromptPack::load(std::path::PathBuf::from(dir)).ok(),
            None => PromptPack::embedded().ok(),
        }
        .map(Arc::new);
        let http = reqwest::Client::builder()
            .user_agent(concat!("Akatsuki-Desktop/", env!("CARGO_PKG_VERSION")))
            .timeout(Duration::from_secs(60))
            .build()
            .unwrap_or_default();
        // Managed AI (and the user's JWT) goes only to this device's configured apps/api, never to an
        // origin from synced settings (D-415).
        let ai = AiClient::with_http(http.clone()).with_managed_api(endpoints.api_url.clone());
        let b = Arc::new(Self {
            rt,
            store: Arc::new(Mutex::new(store)),
            search: Arc::new(Mutex::new(search)),
            events: tx,
            remote: RwLock::new(None),
            jwt: Arc::new(RwLock::new(None)),
            secrets,
            endpoints,
            paths,
            device_id,
            http: http.clone(),
            ai,
            prompts,
            bodies: Mutex::new((HashMap::new(), Vec::new())),
            inflight: Mutex::new(HashSet::new()),
            tasks: Mutex::new(Vec::new()),
            feeds: Mutex::new(HashSet::new()),
            undo: Mutex::new(Vec::new()),
            ai_settings: RwLock::new(None),
            ai_seq: Mutex::new(0),
            cache_bodies: RwLock::new(cache_bodies),
        });
        (b, rx)
    }

    fn emit(&self, e: Event) {
        let _ = self.events.unbounded_send(e);
    }

    fn toast(&self, text: impl Into<String>, undo: bool) {
        self.emit(Event::Toast { text: text.into(), undo });
    }

    fn spawn(&self, f: impl Future<Output = ()> + Send + 'static) {
        let h = self.rt.spawn(f);
        self.tasks.lock().unwrap_or_else(|e| e.into_inner()).push(h.abort_handle());
    }

    fn remote(&self) -> Option<Remote> {
        self.remote.read().unwrap_or_else(|e| e.into_inner()).clone()
    }

    /// The endpoints in use.
    pub fn endpoints(&self) -> &Endpoints {
        &self.endpoints
    }

    /// Stores the body-cache choice.
    pub fn set_cache_bodies(&self, on: bool) {
        *self.cache_bodies.write().unwrap_or_else(|e| e.into_inner()) = on;
        if !on {
            let _ = self.store.lock().unwrap_or_else(|e| e.into_inner()).clear_bodies();
        }
    }

    // ───────────── session ─────────────

    /// Start-up: resume the keychain session, or report signed out.
    pub fn start(self: &Arc<Self>) {
        let me = self.clone();
        self.rt.spawn(async move {
            if !me.endpoints.is_configured() {
                me.emit(Event::Session(SessionState::SignedOut { reason: None }));
                return;
            }
            let session = match me.secrets.get(config::SESSION_SERVICE, config::SESSION_ACCOUNT) {
                Ok(Some(token)) => Session { token },
                Ok(None) => {
                    me.emit(Event::Session(SessionState::SignedOut { reason: None }));
                    return;
                }
                Err(e) => {
                    me.emit(Event::Session(SessionState::SignedOut { reason: Some(e.to_string()) }));
                    return;
                }
            };
            me.emit(Event::Session(SessionState::Connecting));
            me.finish_sign_in(session, false).await;
        });
    }

    /// Opens the demo mailbox (no server, nothing leaves the machine).
    pub fn start_demo(self: &Arc<Self>) {
        let demo = DemoApi::new();
        let remote = Remote { api: Arc::new(demo.clone()), mail: Arc::new(demo), live: None, demo: true };
        *self.remote.write().unwrap_or_else(|e| e.into_inner()) = Some(remote.clone());
        self.emit(Event::Session(SessionState::SignedIn { email: crate::demo::ME.into(), demo: true }));
        self.start_sync(remote);
    }

    fn auth(&self) -> Option<AuthApi> {
        self.endpoints.convex_site_url.as_deref().map(|s| AuthApi::new(self.http.clone(), s))
    }

    /// Emails a sign-in code.
    pub fn send_code(self: &Arc<Self>, email: String) {
        let me = self.clone();
        self.rt.spawn(async move {
            let Some(auth) = me.auth() else { return };
            match auth.send_otp(email.trim()).await {
                Ok(()) => me.emit(Event::SignInStatus(format!("We sent a code to {}.", email.trim()))),
                Err(e) => me.emit(Event::SignInStatus(e.to_string())),
            }
        });
    }

    /// Signs in with the emailed code.
    pub fn verify_code(self: &Arc<Self>, email: String, code: String) {
        let me = self.clone();
        self.rt.spawn(async move {
            let Some(auth) = me.auth() else { return };
            me.emit(Event::SignInStatus("Signing in…".into()));
            match auth.verify_otp(email.trim(), &code).await {
                Ok(s) => me.finish_sign_in(s, true).await,
                Err(e) => me.emit(Event::SignInStatus(e.to_string())),
            }
        });
    }

    /// Browser sign-in through the loopback (social or magic link).
    pub fn sign_in_browser(self: &Arc<Self>, provider: Option<SocialProvider>, email: Option<String>) {
        let me = self.clone();
        self.rt.spawn(async move {
            let Some(auth) = me.auth() else { return };
            let lb = match Loopback::bind().await {
                Ok(lb) => lb,
                Err(e) => {
                    me.emit(Event::SignInStatus(e.to_string()));
                    return;
                }
            };
            let cb = lb.callback_url();
            let started = match (provider, email) {
                (Some(p), _) => auth.start_social(p, &cb).await.map(|url| {
                    me.emit(Event::OpenUrl(url));
                    "Continue in your browser.".to_owned()
                }),
                (None, Some(email)) => auth
                    .send_magic_link(email.trim(), &cb)
                    .await
                    .map(|()| format!("Open the link we sent to {} on this computer.", email.trim())),
                (None, None) => return,
            };
            match started {
                Ok(msg) => me.emit(Event::SignInStatus(msg)),
                Err(e) => {
                    me.emit(Event::SignInStatus(e.to_string()));
                    return;
                }
            }
            let ott = match lb.wait_for_token(Duration::from_secs(600)).await {
                Ok(t) => t,
                Err(e) => {
                    me.emit(Event::SignInStatus(e.to_string()));
                    return;
                }
            };
            match auth.verify_one_time_token(&ott).await {
                Ok(s) => me.finish_sign_in(s, true).await,
                Err(e) => me.emit(Event::SignInStatus(e.to_string())),
            }
        });
    }

    async fn finish_sign_in(self: &Arc<Self>, session: Session, fresh: bool) {
        let (Some(auth), Some(convex_url)) = (self.auth(), self.endpoints.convex_url.clone()) else { return };
        let jwt = match auth.mint_jwt(&session).await {
            Ok(j) => j,
            Err(AuthError::Unauthenticated) => {
                let _ = self.secrets.delete(config::SESSION_SERVICE, config::SESSION_ACCOUNT);
                self.emit(Event::Session(SessionState::SignedOut {
                    reason: Some(AuthError::Unauthenticated.to_string()),
                }));
                return;
            }
            Err(e) => {
                // Offline start: show the mirror, retry later.
                self.emit(Event::SignInStatus(e.to_string()));
                self.emit(Event::Session(SessionState::SignedOut { reason: Some(e.to_string()) }));
                return;
            }
        };
        if fresh && let Err(e) = self.secrets.set(config::SESSION_SERVICE, config::SESSION_ACCOUNT, &session.token) {
            self.emit(Event::SignInStatus(format!("Signed in, but the session could not be saved: {e}")));
        }
        let transport = match ConvexClientTransport::connect(&convex_url).await {
            Ok(t) => t,
            Err(_) => {
                self.emit(Event::Session(SessionState::SignedOut {
                    reason: Some("Could not reach the server.".into()),
                }));
                return;
            }
        };
        let convex = Arc::new(Convex::new(transport));
        convex.set_auth(Some(jwt.token.clone())).await;
        let email = jwt.email.clone().unwrap_or_default();
        *self.jwt.write().unwrap_or_else(|e| e.into_inner()) = Some(jwt.clone());
        let mail = Arc::new(LiveMailServer::new(
            WorkerClient::new(self.http.clone(), self.endpoints.sync()),
            self.jwt.clone(),
        ));
        let remote = Remote { api: convex.clone(), mail, live: Some(convex.clone()), demo: false };
        *self.remote.write().unwrap_or_else(|e| e.into_inner()) = Some(remote.clone());
        let _ = remote
            .api
            .register_device(&RegisterDeviceArgs {
                device_id: self.device_id.clone(),
                kind: DeviceKind::Desktop,
                name: format!("Akatsuki on {}", std::env::consts::OS),
                app_version: config::APP_VERSION.into(),
                expo_push_token: None,
            })
            .await;
        self.emit(Event::Session(SessionState::SignedIn { email, demo: false }));
        self.start_sync(remote);
        self.spawn_refresh(auth, session, convex, jwt);
    }

    fn spawn_refresh(
        self: &Arc<Self>,
        auth: AuthApi,
        session: Session,
        convex: Arc<Convex<ConvexClientTransport>>,
        mut jwt: Jwt,
    ) {
        let me = self.clone();
        self.spawn(async move {
            loop {
                tokio::time::sleep(jwt.refresh_in(now_ms() / 1000)).await;
                match auth.mint_jwt(&session).await {
                    Ok(j) => {
                        convex.set_auth(Some(j.token.clone())).await;
                        *me.jwt.write().unwrap_or_else(|e| e.into_inner()) = Some(j.clone());
                        jwt = j;
                    }
                    Err(AuthError::Unauthenticated) => {
                        me.clone().sign_out_local(Some(AuthError::Unauthenticated.to_string()));
                        return;
                    }
                    Err(_) => tokio::time::sleep(Duration::from_secs(30)).await,
                }
            }
        });
    }

    /// Signs out: revokes the session, deletes the keychain entry and every local cache (§6.3).
    pub fn sign_out(self: &Arc<Self>) {
        let me = self.clone();
        self.rt.spawn(async move {
            if let (Some(auth), Ok(Some(token))) =
                (me.auth(), me.secrets.get(config::SESSION_SERVICE, config::SESSION_ACCOUNT))
            {
                let _ = auth.sign_out(&Session { token }).await;
            }
            me.sign_out_local(None);
        });
    }

    fn sign_out_local(self: Arc<Self>, reason: Option<String>) {
        for t in self.tasks.lock().unwrap_or_else(|e| e.into_inner()).drain(..) {
            t.abort();
        }
        self.feeds.lock().unwrap_or_else(|e| e.into_inner()).clear();
        *self.remote.write().unwrap_or_else(|e| e.into_inner()) = None;
        *self.jwt.write().unwrap_or_else(|e| e.into_inner()) = None;
        let _ = self.secrets.delete(config::SESSION_SERVICE, config::SESSION_ACCOUNT);
        let _ = self.store.lock().unwrap_or_else(|e| e.into_inner()).wipe();
        if let Ok(mut s) = self.search.lock() {
            let path = self.paths.search();
            let on_disk = path.exists();
            // Replace the index first so its file handle is closed, then delete the file.
            if let Ok(fresh) = SearchIndex::open_in_memory() {
                *s = fresh;
            }
            if on_disk {
                for suffix in ["", "-wal", "-shm"] {
                    let _ = std::fs::remove_file(format!("{}{suffix}", path.display()));
                }
                if let Ok(fresh) = SearchIndex::open(&path) {
                    *s = fresh;
                }
            }
        }
        crate::render::original::clear_originals(&self.paths.originals());
        self.bodies.lock().unwrap_or_else(|e| e.into_inner()).0.clear();
        self.undo.lock().unwrap_or_else(|e| e.into_inner()).clear();
        self.emit(Event::MirrorChanged);
        self.emit(Event::Session(SessionState::SignedOut { reason }));
    }

    // ───────────── sync ─────────────

    fn start_sync(self: &Arc<Self>, remote: Remote) {
        let me = self.clone();
        let r = remote.clone();
        self.spawn(async move {
            me.clone().replay_outbox(r.clone()).await;
            loop {
                me.refresh_accounts(&r).await;
                tokio::time::sleep(Duration::from_secs(60)).await;
            }
        });
        let me = self.clone();
        let r = remote;
        self.spawn(async move {
            let Ok(mut failures) = r.api.op_failures().await else { return };
            let mut seen: HashSet<String> = HashSet::new();
            while let Some(Ok(list)) = failures.next().await {
                let mut ack = Vec::new();
                for f in list {
                    if seen.insert(f.id.as_str().to_owned()) {
                        let what = f.subject.as_deref().map(|s| format!(" \"{s}\"")).unwrap_or_default();
                        me.toast(format!("Couldn't update{what} in {}: {}", f.account_email, f.error.message), false);
                        ack.push(f.id.clone());
                    }
                }
                if !ack.is_empty() {
                    let _ = r.api.ack_failures(&akatsuki_core::convex::client::AckFailureArgs { op_ids: ack }).await;
                }
            }
        });
    }

    async fn refresh_accounts(self: &Arc<Self>, r: &Remote) {
        let Ok(accounts) = r.api.list_accounts().await else { return };
        {
            let mut store = self.store.lock().unwrap_or_else(|e| e.into_inner());
            let _ = store.set_accounts(&accounts);
        }
        self.emit(Event::AccountsChanged);
        if let Ok(splits) = r.api.list_splits().await {
            self.emit(Event::Splits(splits));
        }
        if let Ok(ai) = r.api.ai_settings().await {
            let ai = trusted_ai_settings(ai);
            *self.ai_settings.write().unwrap_or_else(|e| e.into_inner()) = ai.clone();
            self.emit(Event::AiSettings(ai));
        }
        for a in accounts {
            let id = a.id.as_str().to_owned();
            if self.feeds.lock().unwrap_or_else(|e| e.into_inner()).insert(id.clone()) {
                let me = self.clone();
                let r = r.clone();
                self.spawn(async move { me.feed_loop(r, id).await });
            }
        }
    }

    async fn feed_loop(self: Arc<Self>, r: Remote, account: String) {
        let mut backoff = Duration::from_secs(1);
        loop {
            let since = self.store.lock().unwrap_or_else(|e| e.into_inner()).account_rev(&account).unwrap_or(0);
            let args = SyncChangesArgs { account_id: account.as_str().into(), since_rev: since, limit: 500 };
            let mut sub = match r.api.sync_changes(&args).await {
                Ok(s) => s,
                Err(_) => {
                    tokio::time::sleep(backoff).await;
                    backoff = (backoff * 2).min(Duration::from_secs(60));
                    continue;
                }
            };
            let mut progressed = false;
            while let Some(item) = sub.next().await {
                let Ok(batch) = item else { break };
                let outcome =
                    self.store.lock().unwrap_or_else(|e| e.into_inner()).apply_changes(&account, since, &batch);
                match outcome {
                    Ok(FeedOutcome::Applied { threads, .. }) => {
                        progressed = true;
                        self.emit(Event::MirrorChanged);
                        self.index_threads(&threads);
                        break;
                    }
                    Ok(FeedOutcome::Stale { .. }) => continue,
                    Ok(FeedOutcome::Gap { .. }) => break,
                    Ok(FeedOutcome::Reset) => {
                        // The mirror re-pulls from 0 (missed deletions, D-391); search entries of
                        // deleted mail must not outlive it, so the account's index is rebuilt too.
                        if let Ok(mut s) = self.search.lock() {
                            let _ = s.remove_account(&account);
                        }
                        progressed = true;
                        self.emit(Event::MirrorChanged);
                        break;
                    }
                    Err(_) => break,
                }
            }
            if progressed {
                backoff = Duration::from_secs(1);
            } else {
                tokio::time::sleep(backoff).await;
                backoff = (backoff * 2).min(Duration::from_secs(60));
            }
        }
    }

    /// Indexes the headers (and cached bodies) of threads for local search (§11.2).
    fn index_threads(&self, thread_ids: &[String]) {
        let docs: Vec<IndexDoc> = {
            let mut store = self.store.lock().unwrap_or_else(|e| e.into_inner());
            let accounts = store.accounts().unwrap_or_default();
            let labels = store.labels(None).unwrap_or_default();
            let mut docs = Vec::new();
            for t in thread_ids {
                let (Ok(Some(summary)), Ok(Some(mirror)), Ok(messages)) =
                    (store.summary(t), store.thread(t), store.messages(t))
                else {
                    continue;
                };
                let account_email = accounts
                    .iter()
                    .find(|a| a.id.as_str() == summary.account_id)
                    .map(|a| a.email.clone())
                    .unwrap_or_default();
                let label_names: Vec<String> = summary
                    .label_keys
                    .iter()
                    .filter_map(|k| labels.iter().find(|l| &l.label_key == k).map(|l| l.name.clone()))
                    .collect();
                for m in messages {
                    let body = store
                        .body(m.id.as_str(), SANITIZER_VERSION)
                        .ok()
                        .flatten()
                        .and_then(|b| b.text)
                        .unwrap_or_else(|| m.snippet.clone());
                    let fmt = |a: &akatsuki_core::Addr| format!("{} {}", a.name.clone().unwrap_or_default(), a.email);
                    docs.push(IndexDoc {
                        message_id: m.id.as_str().to_owned(),
                        thread_id: t.clone(),
                        account_id: summary.account_id.clone(),
                        account_email: account_email.clone(),
                        date: m.date,
                        subject: m.subject.clone(),
                        from: m.from.as_ref().map(fmt).unwrap_or_default(),
                        to: m.to.iter().chain(&m.cc).map(fmt).collect::<Vec<_>>().join(" "),
                        body,
                        filenames: String::new(),
                        label_keys: summary.label_keys.clone(),
                        label_names: label_names.clone(),
                        split_name: mirror.split.clone(),
                        is_unread: m.is_unread || summary.unread,
                        has_attachments: m.has_attachments,
                        is_outbound: m.is_outbound,
                        triage_category: summary.triage.clone(),
                        needs_reply: summary.needs_reply,
                        list_id: m.list_id.clone(),
                    });
                }
            }
            docs
        };
        let mut search = self.search.lock().unwrap_or_else(|e| e.into_inner());
        for d in &docs {
            let _ = search.upsert(d);
        }
    }

    /// Re-indexes every mirrored thread (after start-up with an empty index).
    pub fn reindex_all(self: &Arc<Self>) {
        let me = self.clone();
        self.rt.spawn_blocking(move || {
            let ids =
                me.store.lock().unwrap_or_else(|e| e.into_inner()).list_ids(&Default::default()).unwrap_or_default();
            for chunk in ids.chunks(100) {
                me.index_threads(chunk);
            }
        });
    }

    /// Runs a query over the local index; returns thread ids, best first.
    pub fn search(&self, q: &str, tz: &str) -> Result<Vec<String>, String> {
        let parsed = akatsuki_search::parse(q, now_ms(), tz);
        if let Some(e) = parsed.errors.first() {
            return Err(e.clone());
        }
        let search = self.search.lock().unwrap_or_else(|e| e.into_inner());
        let hits =
            search.search(&parsed.ast, SearchContext { now_at: now_ms(), limit: 500 }).map_err(|e| e.to_string())?;
        Ok(hits.into_iter().map(|h| h.thread_id).collect())
    }

    // ───────────── actions ─────────────

    /// Re-sends the outbox in order (start-up and after a transport error). Boxed: it and
    /// `send_apply` call each other.
    fn replay_outbox(self: Arc<Self>, r: Remote) -> futures::future::BoxFuture<'static, ()> {
        Box::pin(async move {
            let entries = self.store.lock().unwrap_or_else(|e| e.into_inner()).outbox().unwrap_or_default();
            for e in entries {
                match e.function.as_str() {
                    "mail:apply" => {
                        if let Ok(args) = serde_json::from_value::<MailApplyArgs>(e.args.clone()) {
                            self.send_apply(r.clone(), args).await;
                        }
                    }
                    "mail:undo" => {
                        if let Ok(args) = serde_json::from_value::<MailUndoArgs>(e.args.clone()) {
                            self.send_undo(r.clone(), args).await;
                        }
                    }
                    _ => {
                        let _ = self.store.lock().unwrap_or_else(|e| e.into_inner()).outbox_remove(&e.client_op_id);
                    }
                }
            }
        })
    }

    /// Runs a planned gesture: overlay at once (the list changes this frame), outbox, then
    /// `mail.apply` per account in the background.
    pub fn apply(self: &Arc<Self>, plans: Vec<PlannedApply>, label: Option<&str>) {
        let Some(r) = self.remote() else { return };
        let mut ops = Vec::new();
        for p in plans {
            for chunk in p.thread_ids.chunks(500) {
                let client_op_id = uuid::Uuid::now_v7().to_string();
                let args = MailApplyArgs {
                    client_op_id: client_op_id.clone(),
                    thread_ids: chunk.iter().map(|t| t.as_str().into()).collect(),
                    action: p.action.clone(),
                };
                {
                    let mut store = self.store.lock().unwrap_or_else(|e| e.into_inner());
                    let _ = store.add_overlay(&client_op_id, chunk, &p.action);
                    let _ = store.outbox_push(
                        &client_op_id,
                        "mail:apply",
                        &serde_json::to_value(&args).unwrap_or(Value::Null),
                    );
                }
                ops.push(client_op_id);
                let me = self.clone();
                let r = r.clone();
                self.rt.spawn(async move { me.send_apply(r, args).await });
            }
        }
        if !ops.is_empty() {
            self.undo.lock().unwrap_or_else(|e| e.into_inner()).push(Undoable::Gesture { ops, at: now_ms() });
            if let Some(label) = label {
                self.toast(label, true);
            }
        }
        self.emit(Event::MirrorChanged);
    }

    async fn send_apply(self: &Arc<Self>, r: Remote, args: MailApplyArgs) {
        match r.api.mail_apply(&args).await {
            Ok(res) => {
                let mut store = self.store.lock().unwrap_or_else(|e| e.into_inner());
                let _ = store.ack_overlay(&args.client_op_id, &res.rev);
                let _ = store.outbox_remove(&args.client_op_id);
            }
            Err(e) if app_error(&e) => {
                {
                    let mut store = self.store.lock().unwrap_or_else(|e| e.into_inner());
                    let _ = store.drop_overlay(&args.client_op_id);
                    let _ = store.outbox_remove(&args.client_op_id);
                }
                self.toast(safe(&e), false);
            }
            Err(e) => {
                let _ = self.store.lock().unwrap_or_else(|e| e.into_inner()).outbox_attempted(&args.client_op_id);
                self.toast(safe(&e), false);
                // Retry later; the overlay keeps the change visible meanwhile.
                let me = self.clone();
                self.rt.spawn(async move {
                    tokio::time::sleep(Duration::from_secs(15)).await;
                    me.replay_outbox(r).await;
                });
            }
        }
        self.emit(Event::MirrorChanged);
    }

    async fn send_undo(self: &Arc<Self>, r: Remote, args: MailUndoArgs) {
        let res = r.api.mail_undo(&args).await;
        {
            let mut store = self.store.lock().unwrap_or_else(|e| e.into_inner());
            match &res {
                Ok(_) => {
                    let _ = store.ack_overlay_when_converged(&args.client_op_id);
                }
                Err(e) if app_error(e) => {
                    let _ = store.drop_overlay(&args.client_op_id);
                }
                Err(_) => return,
            }
            let _ = store.outbox_remove(&args.client_op_id);
        }
        if let Err(e) = res {
            self.toast(safe(&e), false);
        }
        self.emit(Event::MirrorChanged);
    }

    /// `z`: undoes the latest action or send still in its window. Returns `false` when there is
    /// nothing to undo.
    pub fn undo_last(self: &Arc<Self>) -> bool {
        let Some(r) = self.remote() else { return false };
        let now = now_ms();
        let item = {
            let mut stack = self.undo.lock().unwrap_or_else(|e| e.into_inner());
            stack.retain(|u| match u {
                Undoable::Gesture { at, .. } => now - at <= UNDO_WINDOW_MS,
                Undoable::Send(p) => p.send_at > now,
            });
            stack.pop()
        };
        match item {
            None => false,
            Some(Undoable::Gesture { ops, .. }) => {
                for undone in ops {
                    let client_op_id = uuid::Uuid::now_v7().to_string();
                    let args = MailUndoArgs { client_op_id: client_op_id.clone(), undo_client_op_id: undone.clone() };
                    {
                        let mut store = self.store.lock().unwrap_or_else(|e| e.into_inner());
                        let _ = store.add_undo(&undone, &client_op_id);
                        let _ = store.outbox_push(
                            &client_op_id,
                            "mail:undo",
                            &serde_json::to_value(&args).unwrap_or(Value::Null),
                        );
                    }
                    let me = self.clone();
                    let r = r.clone();
                    self.rt.spawn(async move { me.send_undo(r, args).await });
                }
                self.emit(Event::MirrorChanged);
                self.toast("Undone", false);
                true
            }
            Some(Undoable::Send(p)) => {
                let me = self.clone();
                self.rt.spawn(async move {
                    let res = r.api.sends_cancel(&SendsCancelArgs { client_send_id: p.client_send_id.clone() }).await;
                    let (reopen, error) = match res {
                        Ok(c) if c.result == akatsuki_core::convex::client::CancelOutcome::Cancelled => {
                            (Some(p.draft.clone()), None)
                        }
                        Ok(_) => (None, Some("Too late: the message was already sent.".to_owned())),
                        Err(e) => (None, Some(safe(&e))),
                    };
                    me.emit(Event::SendDone { client_send_id: p.client_send_id.clone(), reopen, error });
                });
                true
            }
        }
    }

    /// Snoozes threads until `until` (`mail.snooze`), with the same overlay the server enqueues.
    pub fn snooze(self: &Arc<Self>, thread_ids: Vec<String>, until: i64) {
        let Some(r) = self.remote() else { return };
        let client_op_id = uuid::Uuid::now_v7().to_string();
        {
            let mut store = self.store.lock().unwrap_or_else(|e| e.into_inner());
            let accounts = store.accounts().unwrap_or_default();
            let labels = store.labels(None).unwrap_or_default();
            let rows = store.summaries(&thread_ids).unwrap_or_default();
            let mut by: BTreeMap<String, Vec<String>> = BTreeMap::new();
            for row in rows {
                by.entry(row.account_id).or_default().push(row.id);
            }
            for (account, ids) in by {
                let ctx = accounts
                    .iter()
                    .find(|a| a.id.as_str() == account)
                    .map(|a| AccountCtx::new(a, &labels))
                    .unwrap_or_else(AccountCtx::gmail);
                let _ = store.add_overlay(&client_op_id, &ids, &snooze_overlay(&ctx));
            }
        }
        self.emit(Event::MirrorChanged);
        self.toast("Snoozed", false);
        let me = self.clone();
        self.rt.spawn(async move {
            let args = MailSnoozeArgs {
                client_op_id: client_op_id.clone(),
                thread_ids: thread_ids.iter().map(|t| t.as_str().into()).collect(),
                until,
                return_unread: false,
            };
            let res = r.api.mail_snooze(&args).await;
            {
                let mut store = me.store.lock().unwrap_or_else(|e| e.into_inner());
                match &res {
                    Ok(_) => {
                        let _ = store.ack_overlay_when_converged(&client_op_id);
                    }
                    Err(_) => {
                        let _ = store.drop_overlay(&client_op_id);
                    }
                }
            }
            if let Err(e) = res {
                me.toast(safe(&e), false);
            }
            me.emit(Event::MirrorChanged);
        });
    }

    /// Sets a reminder (`mail.setReminder`, condition `if_no_reply`, §10).
    pub fn set_reminder(self: &Arc<Self>, thread_id: String, at: i64) {
        let Some(r) = self.remote() else { return };
        let me = self.clone();
        self.rt.spawn(async move {
            let args = akatsuki_core::convex::client::MailSetReminderArgs {
                thread_id: thread_id.as_str().into(),
                at,
                condition: akatsuki_core::convex::TimerCondition::IfNoReply,
                note: None,
            };
            match r.api.set_reminder(&args).await {
                Ok(_) => me.toast("Reminder set. It comes back if nobody replies.", false),
                Err(e) => me.toast(safe(&e), false),
            }
        });
    }

    // ───────────── bodies ─────────────

    /// A rendered body from memory, if it is there.
    pub fn cached_body(&self, message_id: &str) -> Option<Arc<Rendered>> {
        self.bodies.lock().unwrap_or_else(|e| e.into_inner()).0.get(message_id).cloned()
    }

    fn remember(&self, message_id: &str, r: Arc<Rendered>) {
        let mut b = self.bodies.lock().unwrap_or_else(|e| e.into_inner());
        if b.0.insert(message_id.to_owned(), r).is_none() {
            b.1.push(message_id.to_owned());
        }
        while b.1.len() > BODY_MEMORY {
            let old = b.1.remove(0);
            b.0.remove(&old);
        }
    }

    /// Loads a body: memory, then the device cache (re-sanitized locally), then the worker.
    pub fn load_body(self: &Arc<Self>, message_id: &str) {
        if let Some(r) = self.cached_body(message_id) {
            self.emit(Event::Body { message_id: message_id.to_owned(), result: Ok(r) });
            return;
        }
        if !self.inflight.lock().unwrap_or_else(|e| e.into_inner()).insert(message_id.to_owned()) {
            return;
        }
        let me = self.clone();
        let id = message_id.to_owned();
        self.rt.spawn(async move {
            let cached = me.store.lock().unwrap_or_else(|e| e.into_inner()).raw_body(&id).ok().flatten();
            let raw = match cached {
                Some(raw) => Ok(raw),
                None => match me.remote() {
                    Some(r) => r.mail.raw_message(&id).await.map_err(|e| e.to_string()),
                    None => Err("Sign in to load messages.".to_owned()),
                },
            };
            let result = match raw {
                Ok(raw) => {
                    let rendered = tokio::task::spawn_blocking(move || render_raw(&raw).map(|r| (r, raw))).await;
                    match rendered {
                        Ok(Ok((r, raw))) => {
                            let r = Arc::new(r);
                            if *me.cache_bodies.read().unwrap_or_else(|e| e.into_inner()) {
                                let mut store = me.store.lock().unwrap_or_else(|e| e.into_inner());
                                let _ = store.put_body(
                                    &id,
                                    r.sanitizer_version,
                                    r.html.as_deref(),
                                    r.text.as_deref(),
                                    Some(&raw),
                                    BODY_CACHE_MAX_BYTES,
                                );
                            }
                            me.remember(&id, r.clone());
                            Ok(r)
                        }
                        _ => Err("This message could not be read.".to_owned()),
                    }
                }
                Err(e) => Err(e),
            };
            me.inflight.lock().unwrap_or_else(|e| e.into_inner()).remove(&id);
            if result.is_ok() {
                let thread = me
                    .store
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .message(&id)
                    .ok()
                    .flatten()
                    .map(|m| m.thread_id.as_str().to_owned());
                if let Some(t) = thread {
                    me.index_threads(&[t]);
                }
            }
            me.emit(Event::Body { message_id: id, result });
        });
    }

    /// Writes the sanitized original with remote images blocked and returns its path ("Open original").
    pub fn write_original(&self, message_id: &str, subject: &str) -> Option<std::path::PathBuf> {
        let r = self.cached_body(message_id)?;
        crate::render::original::write_original(
            &self.paths.originals(),
            message_id,
            subject,
            &r,
            crate::render::original::Images::Blocked,
        )
        .ok()
    }

    /// "Show images": the original with remote images loaded through the worker's image proxy
    /// (§14.4, D-399). Fetches the worker's sanitized copy, writes the page and asks the UI to open
    /// it; a sender's own image URL is never contacted from this machine.
    pub fn open_original_with_images(self: &Arc<Self>, message_id: &str, subject: &str) {
        let (Some(r), Some(rendered)) = (self.remote(), self.cached_body(message_id)) else {
            self.emit(Event::Toast { text: "The message is still loading.".into(), undo: false });
            return;
        };
        let me = self.clone();
        let id = message_id.to_owned();
        let subject = subject.to_owned();
        self.rt.spawn(async move {
            let text = match r.mail.proxied_html(&id).await {
                Ok(p) => {
                    let images = match &p.html {
                        Some(html) => crate::render::original::Images::Proxied { html, proxy_prefix: &p.proxy_prefix },
                        None => crate::render::original::Images::Blocked,
                    };
                    match crate::render::original::write_original(
                        &me.paths.originals(),
                        &id,
                        &subject,
                        &rendered,
                        images,
                    ) {
                        Ok(path) => {
                            me.emit(Event::OpenUrl(format!("file://{}", path.display())));
                            return;
                        }
                        Err(_) => "The original could not be written.".to_owned(),
                    }
                }
                Err(e) => e.to_string(),
            };
            me.emit(Event::Toast { text, undo: false });
        });
    }

    // ───────────── compose ─────────────

    /// Autosaves a draft (§10.1).
    pub fn autosave(self: &Arc<Self>, draft: Draft) {
        let (Some(r), Some(content)) = (self.remote(), draft.autosave_content()) else { return };
        let me = self.clone();
        let device_id = self.device_id.clone();
        self.rt.spawn(async move {
            let put =
                akatsuki_core::wire::worker::DraftPut { base_version: draft.base_version, draft: content, device_id };
            if let Ok(saved) = r.mail.put_draft(&draft.client_draft_id, &put).await {
                me.emit(Event::DraftSaved { client_draft_id: draft.client_draft_id, version: saved.version });
            }
        });
    }

    /// Reads a file and uploads it to the worker's spool (§10.4); the result arrives as
    /// [`Event::AttachmentDone`]. Files over 25 MB never leave the machine.
    pub fn upload_attachment(self: &Arc<Self>, client_draft_id: String, key: String, path: std::path::PathBuf) {
        let me = self.clone();
        let remote = self.remote();
        self.rt.spawn(async move {
            let result = async {
                let r = remote.ok_or_else(|| "Sign in to attach files.".to_owned())?;
                let filename = path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .map(str::to_owned)
                    .ok_or_else(|| "That file has no usable name.".to_owned())?;
                let meta = tokio::fs::metadata(&path).await.map_err(|_| "That file cannot be read.".to_owned())?;
                if !meta.is_file() {
                    return Err("Attach files, not folders.".to_owned());
                }
                if meta.len() > crate::vm::compose::MAX_ATTACHMENT_BYTES {
                    return Err("Attachments are limited to 25 MB per message.".to_owned());
                }
                let bytes = tokio::fs::read(&path).await.map_err(|_| "That file cannot be read.".to_owned())?;
                if bytes.is_empty() {
                    return Err("That file is empty.".to_owned());
                }
                let ctype = crate::vm::compose::content_type_for(&filename);
                r.mail.upload(&filename, ctype, bytes).await.map(|u| u.upload_id).map_err(|e| e.to_string())
            }
            .await;
            me.emit(Event::AttachmentDone { client_draft_id, key, result });
        });
    }

    /// Sends with the undo window. Returns the pending send for the "Sending… Undo" bar.
    pub fn send(self: &Arc<Self>, draft: Draft, undo_seconds: u32) -> Result<PendingSend, String> {
        let r = self.remote().ok_or("Sign in to send.")?;
        let client_send_id = uuid::Uuid::now_v7().to_string();
        let req =
            draft.send_request(client_send_id.clone(), undo_seconds, &self.device_id).map_err(|e| e.to_string())?;
        // Reopened as it was (attachments included) if the worker refuses the send.
        let reopen = draft.clone();
        let pending = PendingSend {
            client_send_id: client_send_id.clone(),
            draft,
            send_at: now_ms() + i64::from(undo_seconds) * 1000,
        };
        self.undo.lock().unwrap_or_else(|e| e.into_inner()).push(Undoable::Send(pending.clone()));
        let me = self.clone();
        self.rt.spawn(async move {
            if let Err(e) = r.mail.send(&req).await {
                me.undo
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .retain(|u| !matches!(u, Undoable::Send(p) if p.client_send_id == client_send_id));
                me.emit(Event::SendDone { client_send_id, reopen: Some(reopen), error: Some(e.to_string()) });
                return;
            }
            tokio::time::sleep(Duration::from_millis(u64::from(undo_seconds) * 1000)).await;
            let still = me
                .undo
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .iter()
                .any(|u| matches!(u, Undoable::Send(p) if p.client_send_id == client_send_id));
            if still {
                me.emit(Event::SendDone { client_send_id, reopen: None, error: None });
            }
        });
        Ok(pending)
    }

    // ───────────── AI ─────────────

    /// The AI settings last read from Convex.
    pub fn ai_settings(&self) -> Option<AiSettings> {
        self.ai_settings.read().unwrap_or_else(|e| e.into_inner()).clone()
    }

    /// The egress line under an AI action (§12.2), when the feature resolves.
    pub fn ai_egress(&self, feature: AiFeature) -> Option<String> {
        let settings = self.ai_settings()?;
        resolve(feature, &settings, &ROUTING).ok().map(|r| r.egress_text())
    }

    /// Saves a provider profile (key to the keychain, never to Convex) and makes it the default
    /// route; with `consent`, records consent (§12.7 rule 1).
    pub fn save_ai_profile(self: &Arc<Self>, mut profile: AiProfile, key: Option<String>, consent: bool) {
        let Some(r) = self.remote() else { return };
        if let Some(k) = key.filter(|k| !k.trim().is_empty()) {
            if let Err(e) = self.secrets.set(config::AI_KEY_SERVICE, &profile.profile_id, k.trim()) {
                self.toast(e.to_string(), false);
                return;
            }
            if !profile.key_on_devices.contains(&self.device_id) {
                profile.key_on_devices.push(self.device_id.clone());
            }
        }
        let mut settings = self.ai_settings().unwrap_or_default();
        settings.profiles.retain(|p| p.profile_id != profile.profile_id);
        settings.profiles.push(profile.clone());
        settings.routes.retain(|r| r.feature != AiRouteFeature::DEFAULT);
        settings.routes.push(AiRoute {
            feature: AiRouteFeature::DEFAULT,
            profile_id: profile.profile_id.clone(),
            model: None,
            enabled: true,
        });
        if consent && settings.consent.granted_at.is_none() {
            settings.consent.granted_at = Some(now_ms());
            settings.consent.version = Some(1);
        }
        *self.ai_settings.write().unwrap_or_else(|e| e.into_inner()) = Some(settings.clone());
        self.emit(Event::AiSettings(Some(settings.clone())));
        let me = self.clone();
        self.rt.spawn(async move {
            let args = SaveAiSettingsArgs {
                profiles: Some(settings.profiles),
                routes: Some(settings.routes),
                consent: Some(settings.consent),
            };
            match r.api.save_ai_settings(&args).await {
                Ok(()) => me.toast("AI settings saved", false),
                Err(e) => me.toast(safe(&e), false),
            }
        });
    }

    /// Removes a profile and its key on this device.
    pub fn remove_ai_profile(self: &Arc<Self>, profile_id: &str) {
        let Some(r) = self.remote() else { return };
        let _ = self.secrets.delete(config::AI_KEY_SERVICE, profile_id);
        let mut settings = self.ai_settings().unwrap_or_default();
        settings.profiles.retain(|p| p.profile_id != profile_id);
        settings.routes.retain(|r| r.profile_id != profile_id);
        *self.ai_settings.write().unwrap_or_else(|e| e.into_inner()) = Some(settings.clone());
        self.emit(Event::AiSettings(Some(settings.clone())));
        self.rt.spawn(async move {
            let _ = r
                .api
                .save_ai_settings(&SaveAiSettingsArgs {
                    profiles: Some(settings.profiles),
                    routes: Some(settings.routes),
                    consent: None,
                })
                .await;
        });
    }

    /// Whether this device holds a key for a profile.
    pub fn has_ai_key(&self, profile_id: &str) -> bool {
        self.secrets.get(config::AI_KEY_SERVICE, profile_id).ok().flatten().is_some()
    }

    fn key_for(&self, profile: &AiProfile) -> Option<String> {
        if profile.kind == akatsuki_core::convex::AiProviderKind::Akatsuki {
            return self.jwt.read().unwrap_or_else(|e| e.into_inner()).as_ref().map(|j| j.token.clone());
        }
        self.secrets.get(config::AI_KEY_SERVICE, &profile.profile_id).ok().flatten()
    }

    /// "Test connection" from this device.
    pub fn test_ai_profile(self: &Arc<Self>, profile: AiProfile) {
        let me = self.clone();
        self.rt.spawn(async move {
            let result = async {
                let provider = me.ai.provider_for(&profile, me.key_for(&profile)).map_err(|e| e.to_string())?;
                let model = profile
                    .models
                    .default
                    .clone()
                    .or_else(|| ROUTING.model(akatsuki_ai::Tier::Default, profile.kind).map(str::to_owned))
                    .ok_or_else(|| "Choose a model.".to_owned())?;
                me.ai.test_connection(&profile, &provider, &model).await.map_err(|e| e.to_string())
            }
            .await;
            me.emit(Event::AiTest { profile_id: profile.profile_id.clone(), result });
        });
    }

    /// Runs `summarize` or `instant_reply` on a thread; events arrive as [`Event::Ai`]. Returns the
    /// request id.
    pub fn run_ai(
        self: &Arc<Self>,
        feature: AiFeature,
        thread_id: &str,
        user_name: &str,
        tz: &str,
    ) -> Result<u64, String> {
        let settings = self.ai_settings().ok_or("Set up an AI provider in Settings first.")?;
        let pack = self.prompts.clone().ok_or("The prompt pack is not installed.")?;
        let request = {
            let mut seq = self.ai_seq.lock().unwrap_or_else(|e| e.into_inner());
            *seq += 1;
            *seq
        };
        let messages =
            self.store.lock().unwrap_or_else(|e| e.into_inner()).messages(thread_id).map_err(|e| e.to_string())?;
        let texts: Vec<TranscriptMessage> = messages
            .iter()
            .map(|m| {
                let body =
                    self.cached_body(m.id.as_str()).and_then(|b| b.text.clone()).unwrap_or_else(|| m.snippet.clone());
                TranscriptMessage {
                    from: m.from.as_ref().map(RenderAddr::from),
                    to: m.to.iter().map(RenderAddr::from).collect(),
                    cc: m.cc.iter().map(RenderAddr::from).collect(),
                    date: m.date,
                    text: strip_quoted(&body).to_owned(),
                }
            })
            .collect();
        let today = jiff::Timestamp::now().strftime("%Y-%m-%d").to_string();
        let mut vars = serde_json::Map::new();
        vars.insert("userName".into(), json!(user_name));
        match feature {
            AiFeature::Summarize => {
                vars.insert("today".into(), json!(today));
            }
            AiFeature::InstantReply => {
                vars.insert("voice".into(), Value::Null);
            }
            _ => return Err("Not available here.".into()),
        }
        let budget = pack.transcript_budget(feature, &vars).map_err(|e| e.to_string())?;
        let transcript = render_transcript(&ThreadForTranscript {
            timezone: tz.to_owned(),
            max_input_chars: Some(budget),
            messages: texts,
        });
        vars.insert("transcript".into(), json!(transcript));
        let profile_key = resolve(feature, &settings, &ROUTING).ok().and_then(|r| self.key_for(r.profile));
        let stream = akatsuki_ai::run_feature(&self.ai, &pack, &settings, feature, vars, move |_| profile_key);
        let me = self.clone();
        self.rt.spawn(async move {
            let mut stream = stream;
            while let Some(event) = stream.next().await {
                me.emit(Event::Ai { request, event });
            }
        });
        Ok(request)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::secrets::MemoryStore;
    use crate::vm::mail::{ActionKind, plan};
    use akatsuki_core::store::ListQuery;

    #[test]
    fn synced_ai_settings_that_fail_validation_turn_ai_off() {
        use akatsuki_core::convex::AiProviderKind;
        use akatsuki_core::convex::rows::{AiConsent, AiModels};
        let managed = |base_url: Option<&str>| AiSettings {
            profiles: vec![AiProfile {
                profile_id: "6f0c9a52-3b1e-4c1d-9a8e-2f4b5c6d7e8f".into(),
                label: "Akatsuki AI".into(),
                kind: AiProviderKind::Akatsuki,
                base_url: base_url.map(str::to_owned),
                models: AiModels::default(),
                key_on_devices: vec![],
            }],
            routes: vec![AiRoute {
                feature: AiRouteFeature::DEFAULT,
                profile_id: "6f0c9a52-3b1e-4c1d-9a8e-2f4b5c6d7e8f".into(),
                model: None,
                enabled: true,
            }],
            consent: AiConsent { granted_at: Some(1), version: Some(1), auto_triage: false, sync_voice_profile: false },
            ..AiSettings::default()
        };
        assert!(trusted_ai_settings(Some(managed(None))).is_some());
        // A poisoned setting would send the JWT and the thread elsewhere: AI is off instead (D-415).
        assert!(trusted_ai_settings(Some(managed(Some("https://evil.example")))).is_none());
        assert!(trusted_ai_settings(None).is_none());
    }

    async fn next_matching(rx: &mut UnboundedReceiver<Event>, f: impl Fn(&Event) -> bool) -> Event {
        loop {
            let e = tokio::time::timeout(Duration::from_secs(5), rx.next())
                .await
                .expect("event in time")
                .expect("channel open");
            if f(&e) {
                return e;
            }
        }
    }

    fn demo_backend() -> (Arc<Backend>, UnboundedReceiver<Event>) {
        let paths = Paths {
            data_dir: std::env::temp_dir().join(format!("akatsuki-backend-{}", uuid::Uuid::new_v4())),
            config_file: std::env::temp_dir().join("unused.json"),
        };
        Backend::new(
            Handle::current(),
            Store::open_in_memory().unwrap(),
            SearchIndex::open_in_memory().unwrap(),
            Arc::new(MemoryStore::default()),
            Endpoints::default(),
            paths,
            "dev-1".into(),
            true,
        )
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn attachments_upload_and_oversized_files_stay_on_the_machine() {
        let (b, mut rx) = demo_backend();
        b.start_demo();
        let dir = std::env::temp_dir().join(format!("akatsuki-attach-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let small = dir.join("plan.pdf");
        std::fs::write(&small, b"%PDF-1.4 plan").unwrap();
        b.upload_attachment("d1".into(), "k1".into(), small);
        let e = next_matching(&mut rx, |e| matches!(e, Event::AttachmentDone { key, .. } if key == "k1")).await;
        let Event::AttachmentDone { client_draft_id, result, .. } = e else { unreachable!() };
        assert_eq!(client_draft_id, "d1");
        assert!(result.is_ok_and(|id| !id.is_empty()));

        // Over 25 MB: refused before any upload (a sparse file, nothing is read).
        let big = dir.join("film.mov");
        std::fs::File::create(&big).unwrap().set_len(crate::vm::compose::MAX_ATTACHMENT_BYTES + 1).unwrap();
        b.upload_attachment("d1".into(), "k2".into(), big);
        let e = next_matching(&mut rx, |e| matches!(e, Event::AttachmentDone { key, .. } if key == "k2")).await;
        let Event::AttachmentDone { result, .. } = e else { unreachable!() };
        assert_eq!(result, Err("Attachments are limited to 25 MB per message.".to_owned()));
        // A folder is not a file.
        b.upload_attachment("d1".into(), "k3".into(), dir.clone());
        let e = next_matching(&mut rx, |e| matches!(e, Event::AttachmentDone { key, .. } if key == "k3")).await;
        let Event::AttachmentDone { result, .. } = e else { unreachable!() };
        assert!(result.is_err());
        let _ = std::fs::remove_dir_all(dir);
    }

    fn inbox_ids(b: &Backend) -> Vec<String> {
        b.store
            .lock()
            .unwrap()
            .list_ids(&ListQuery { label_key: Some("sys:inbox".into()), ..ListQuery::default() })
            .unwrap()
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn demo_mailbox_syncs_applies_and_undoes_end_to_end() {
        let (b, mut rx) = demo_backend();
        b.start_demo();
        // The feed fills the mirror.
        for _ in 0..50 {
            if inbox_ids(&b).contains(&"t-board".to_owned()) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        assert!(inbox_ids(&b).contains(&"t-board".to_owned()));

        // Archive: gone at once (overlay), confirmed by the server, still gone.
        let plans =
            plan(ActionKind::Archive, &["t-board".into()], &crate::vm::mail::Folder::Inbox, &b.store.lock().unwrap())
                .unwrap();
        b.apply(plans, Some("Archived"));
        assert!(!inbox_ids(&b).contains(&"t-board".to_owned()), "optimistic");
        next_matching(&mut rx, |e| matches!(e, Event::Toast { undo: true, .. })).await;
        for _ in 0..100 {
            if b.store.lock().unwrap().overlay().unwrap().is_empty() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        assert!(b.store.lock().unwrap().overlay().unwrap().is_empty(), "the mirror caught up and the overlay is gone");
        assert!(!inbox_ids(&b).contains(&"t-board".to_owned()));
        assert!(b.store.lock().unwrap().outbox().unwrap().is_empty());

        // Undo brings it back at once and for good.
        assert!(b.undo_last());
        assert!(inbox_ids(&b).contains(&"t-board".to_owned()));
        for _ in 0..100 {
            if b.store.lock().unwrap().overlay().unwrap().is_empty() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        assert!(inbox_ids(&b).contains(&"t-board".to_owned()));
        assert!(!b.undo_last(), "nothing left to undo");

        // Bodies load, render and get indexed for search.
        b.load_body("t-dawn-m0");
        let Event::Body { result, .. } = next_matching(&mut rx, |e| matches!(e, Event::Body { .. })).await else {
            unreachable!()
        };
        let r = result.unwrap();
        assert!(r.remote_images >= 1 && r.trackers >= 1);
        assert!(b.cached_body("t-dawn-m0").is_some());
        let hits = b.search("teapot", "UTC").unwrap();
        assert_eq!(hits.first().map(String::as_str), Some("t-dawn"));
        assert!(b.search("from:\"Jonas Weber\"", "UTC").unwrap().contains(&"t-hiring".to_owned()));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn ai_summary_runs_on_the_device_against_the_chosen_provider() {
        use akatsuki_core::convex::AiProviderKind;
        use akatsuki_core::convex::rows::{AiConsent, AiModels};
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        let summary = json!({"tldr": "Mira needs the deck tonight.", "bullets": ["Add the hiring slide"], "asks": [], "deadlines": []});
        let content = summary.to_string();
        let body = format!(
            "data: {}\n\ndata: {}\n\ndata: [DONE]\n\n",
            json!({"choices": [{"delta": {"content": content}, "finish_reason": null}]}),
            json!({"choices": [{"delta": {}, "finish_reason": "stop"}], "usage": {"prompt_tokens": 10, "completion_tokens": 5}})
        );
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(
                ResponseTemplate::new(200).insert_header("content-type", "text/event-stream").set_body_string(body),
            )
            .expect(1)
            .mount(&server)
            .await;
        let (b, mut rx) = demo_backend();
        b.start_demo();
        for _ in 0..50 {
            if !inbox_ids(&b).is_empty() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        assert_eq!(
            b.run_ai(AiFeature::Summarize, "t-board", "me", "UTC").unwrap_err(),
            "Set up an AI provider in Settings first.",
            "nothing runs before settings and consent"
        );
        *b.ai_settings.write().unwrap() = Some(AiSettings {
            profiles: vec![AiProfile {
                profile_id: "p1".into(),
                label: "Local".into(),
                kind: AiProviderKind::OpenaiCompatible,
                base_url: Some(format!("{}/v1", server.uri())),
                models: AiModels { default: Some("test-model".into()), ..AiModels::default() },
                key_on_devices: vec![],
            }],
            routes: vec![AiRoute {
                feature: AiRouteFeature::DEFAULT,
                profile_id: "p1".into(),
                model: None,
                enabled: true,
            }],
            consent: AiConsent { granted_at: Some(1), version: Some(1), auto_triage: false, sync_voice_profile: false },
            ..AiSettings::default()
        });
        assert_eq!(b.ai_egress(AiFeature::Summarize).as_deref(), Some("Stays on this device"));
        let request = b.run_ai(AiFeature::Summarize, "t-board", "me", "UTC").unwrap();
        let e =
            next_matching(&mut rx, |e| matches!(e, Event::Ai { event: AaspEvent::Done(_) | AaspEvent::Error(_), .. }))
                .await;
        let Event::Ai { request: r, event: AaspEvent::Done(done) } = e else { panic!("expected done, got {e:?}") };
        assert_eq!(r, request);
        assert_eq!(done.object.unwrap()["tldr"], "Mira needs the deck tonight.");
        let sent = &server.received_requests().await.unwrap()[0];
        let text = String::from_utf8_lossy(&sent.body);
        assert!(
            text.contains("Board deck") || text.contains("board deck"),
            "the thread goes to the chosen provider only"
        );
    }

    /// Against the local Convex harness (`make convex-up`, `EMAIL_TRANSPORT=console`): email OTP
    /// sign-in, JWT minting, an authenticated Convex connection, the client functions the desktop
    /// calls, and sign-out. Run with
    /// `AKATSUKI_LIVE_SITE=http://127.0.0.1:3411 AKATSUKI_LIVE_CONVEX=http://127.0.0.1:3410
    ///  AKATSUKI_LIVE_OTP_LOG=<file that \`npx convex logs\` writes> cargo test -p akatsuki-desktop -- --ignored`.
    #[tokio::test(flavor = "multi_thread")]
    #[ignore = "needs the local Convex harness"]
    async fn live_local_otp_sign_in() {
        use akatsuki_convex_api::ClientApi;
        let site = std::env::var("AKATSUKI_LIVE_SITE").expect("AKATSUKI_LIVE_SITE");
        let convex_url = std::env::var("AKATSUKI_LIVE_CONVEX").expect("AKATSUKI_LIVE_CONVEX");
        let log = std::env::var("AKATSUKI_LIVE_OTP_LOG").expect("AKATSUKI_LIVE_OTP_LOG");
        let email = format!("desktop-alpha-{}@akatsuki.test", now_ms());
        let auth = AuthApi::new(reqwest::Client::new(), &site);
        auth.send_otp(&email).await.unwrap();
        let mut code = None;
        for _ in 0..100 {
            let text = std::fs::read_to_string(&log).unwrap_or_default();
            if let Some(at) = text.find(&format!("to={email}")) {
                code = text[at..].split(|c: char| !c.is_ascii_digit()).find(|w| w.len() == 6).map(str::to_owned);
                if code.is_some() {
                    break;
                }
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
        let code = code.expect("the OTP shows up in the console transport log");
        assert_eq!(auth.verify_otp(&email, "000000").await.unwrap_err(), AuthError::InvalidCode);
        let session = auth.verify_otp(&email, &code).await.unwrap();
        let jwt = auth.mint_jwt(&session).await.unwrap();
        assert_eq!(jwt.email.as_deref(), Some(email.as_str()));
        let convex = Convex::new(ConvexClientTransport::connect(&convex_url).await.unwrap());
        convex.set_auth(Some(jwt.token.clone())).await;
        convex
            .register_device(&RegisterDeviceArgs {
                device_id: uuid::Uuid::new_v4().to_string(),
                kind: DeviceKind::Desktop,
                name: "Akatsuki desktop test".into(),
                app_version: config::APP_VERSION.into(),
                expo_push_token: None,
            })
            .await
            .unwrap();
        assert!(convex.list_accounts().await.unwrap().is_empty(), "a new user has no mailboxes");
        let _settings: Option<AiSettings> = convex.ai_settings().await.unwrap();
        let splits = convex.list_splits().await.unwrap();
        assert!(splits.iter().all(|s| s.label_key.starts_with("split:")));
        auth.sign_out(&session).await.unwrap();
        assert_eq!(auth.mint_jwt(&session).await.unwrap_err(), AuthError::Unauthenticated, "the session is revoked");
    }

    /// Spike S1 mechanics against the local harness: a magic link with the loopback `callbackURL`,
    /// followed like a browser would; Better Auth's crossDomain plugin appends `ott`, the loopback
    /// checks the nonce and the one-time token becomes this device's session. Same env as above.
    #[tokio::test(flavor = "multi_thread")]
    #[ignore = "needs the local Convex harness"]
    async fn live_local_magic_link_loopback() {
        let site = std::env::var("AKATSUKI_LIVE_SITE").expect("AKATSUKI_LIVE_SITE");
        let log = std::env::var("AKATSUKI_LIVE_OTP_LOG").expect("AKATSUKI_LIVE_OTP_LOG");
        let email = format!("desktop-link-{}@akatsuki.test", now_ms());
        let auth = AuthApi::new(reqwest::Client::new(), &site);
        let lb = Loopback::bind().await.unwrap();
        let cb = lb.callback_url();
        let wait = tokio::spawn(lb.wait_for_token(Duration::from_secs(30)));
        auth.send_magic_link(&email, &cb).await.unwrap();
        let mut link = None;
        for _ in 0..100 {
            let text = std::fs::read_to_string(&log).unwrap_or_default();
            if let Some(at) = text.find(&format!("to={email}")) {
                // Log lines carry escaped newlines; the URL ends at the first backslash, quote or space.
                let tail = &text[at..];
                link = tail.find("/magic-link/verify?").and_then(|v| {
                    let start = tail[..v].rfind("http")?;
                    let end = tail[start..]
                        .find(|c: char| c == '\\' || c == '"' || c.is_whitespace())
                        .map_or(tail.len(), |e| start + e);
                    Some(tail[start..end].to_owned())
                });
                if link.is_some() {
                    break;
                }
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
        let link = link.expect("the magic link shows up in the console transport log");
        // The "browser": follows the redirects to the loopback.
        let page = reqwest::get(&link).await.unwrap();
        assert!(page.status().is_success(), "{}", page.status());
        assert!(page.text().await.unwrap().contains("You can close this tab"));
        let ott = wait.await.unwrap().unwrap();
        let session = auth.verify_one_time_token(&ott).await.unwrap();
        let jwt = auth.mint_jwt(&session).await.unwrap();
        assert_eq!(jwt.email.as_deref(), Some(email.as_str()));
        assert!(auth.verify_one_time_token(&ott).await.is_err(), "one-time tokens are single use");
        auth.sign_out(&session).await.unwrap();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn sends_wait_for_the_undo_window_and_can_be_undone() {
        let (b, mut rx) = demo_backend();
        b.start_demo();
        let mut d = Draft::new("d1".into(), crate::demo::GMAIL, crate::demo::ME);
        d.to = "hana@kitsune-mail.example".into();
        d.subject = "Hello".into();
        d.body = "Hi".into();
        let p = b.send(d.clone(), 5).unwrap();
        assert!(p.seconds_left(now_ms()) >= 4);
        assert!(b.undo_last());
        let e = next_matching(&mut rx, |e| matches!(e, Event::SendDone { .. })).await;
        let Event::SendDone { reopen, error, .. } = e else { unreachable!() };
        assert_eq!(error, None);
        assert_eq!(reopen.unwrap().subject, "Hello", "undo reopens the local copy");
        assert!(b.send(Draft::new("d2".into(), crate::demo::GMAIL, crate::demo::ME), 0).is_err(), "no recipients");
    }
}
