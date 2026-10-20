//! Microsoft Graph adapter (Outlook.com, Microsoft 365): ARCHITECTURE §9.1, §9.4, §9.5 "Graph",
//! §9.6, §9.7, §10.1, §10.2 and §11.2.
//!
//! | Concern | How |
//! |---|---|
//! | Auth | [`MicrosoftOAuth`] (identity platform v2, `common` tenant: personal and work accounts) and [`GraphTokenSource`] (single-flight refresh, rotated refresh tokens persisted through [`RefreshTokenStore`]) |
//! | Ids | `Prefer: IdType="ImmutableId"` on every request and sub-request; thread id = `conversationId` |
//! | Labels | well-known folders → `sys:*` roles, `archive` → role `archive`, other folders → exclusive `folder` labels with `/` paths, `Akatsuki/Snoozed` → role `snoozed`, master categories → non-exclusive `category` labels (`cat:<name>`) |
//! | Sync | per-folder `messages/delta` (`$select=conversationId`, `odata.maxpagesize=50`, window filter), batched 20 folders per `$batch`; removals are checked with a `GET` so moves are not deletes |
//! | Ops | `PATCH` (`isRead`, `flag`, `categories`) and `POST …/move`, 20 per `$batch`; the thread is re-read after the change |
//! | Drafts and send | MIME drafts (`POST /me/messages`, `text/plain` base64) up to ~2.8 MB; larger drafts as JSON with attachments (upload sessions over 3 MB, replies through `createReply`); `POST /messages/{id}/send` |
//! | Throttling | 4 concurrent requests and 16 req/s per mailbox, `Retry-After` honoured on 429/503 |
//! | Push | `/subscriptions` on `me/messages` with lifecycle notifications, renewed when < 24 h remain; polling every 2 min when push is off |
//! | Search | [`kql`] |
//!
//! Decisions: D-210 … D-219 in `docs/DECISIONS.md`.

mod apply;
pub mod auth;
mod client;
mod drafts;
pub mod kql;
mod map;
mod model;
pub mod webhook;

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use bytes::Bytes;
use futures::StreamExt;
use reqwest::Method;
use serde_json::{Value, json};

use crate::labels::SNOOZED_LABEL_NAME;
use crate::{
    ByteStream, ChangePage, Cursor, FetchDepth, FetchedThread, GraphFolderCursor, LabelKind, LabelRole, MailProvider,
    OpEffect, OpId, OutgoingMime, ProviderCaps, ProviderError, ProviderKind, ProviderOp, PushKind, RemoteDraftRef,
    RemoteLabel, RemoteMessageRef, RemoteThreadId, SentRef, TokenSource, UnixMs, WatchKind, WatchState, WindowPage,
    WindowRequest,
};

pub use auth::{
    COMMON_TENANT, GRAPH_SCOPES, GraphTokenSource, LOGIN_BASE, MicrosoftOAuth, OAuthConfig, RefreshTokenStore,
    TokenGrant,
};
pub use client::Limits;
pub use kql::{KqlError, KqlQuery, to_kql};
pub use map::{CATEGORY_PREFIX, category_label_id, category_name};
pub use webhook::{
    GraphNotification, LifecycleAction, LifecycleEvent, client_state_hash, lifecycle_action, new_client_state,
    parse_notifications, verify_client_state,
};

use client::{Body, GraphHttp, Sub, enc, odata_str, query};
use map::{SKIPPED_WELL_KNOWN, SYNCED_WELL_KNOWN, format_time, parse_time};
use model::{Category, Folder, Message, Page, Subscription, User};

/// A source of "now" in ms (injectable for tests).
pub type Clock = Arc<dyn Fn() -> UnixMs + Send + Sync>;

/// The real clock.
pub fn system_clock() -> Clock {
    Arc::new(akatsuki_core::time::now_ms)
}

/// Graph v1.0.
pub const GRAPH_BASE: &str = "https://graph.microsoft.com/v1.0";
/// Delta page size (§9.5).
pub const DELTA_PAGE_SIZE: u32 = 50;
/// Subscription lifetime at creation and renewal (§9.6; Graph allows up to 10,080 min).
pub const SUBSCRIPTION_MINUTES: i64 = 4200;
/// Renew when less than this remains (§9.6).
pub const RENEW_BEFORE_MS: i64 = 24 * 60 * 60 * 1000;

/// Header fields read for every message (bodies are never part of metadata).
pub const MESSAGE_SELECT: &str = "id,conversationId,internetMessageId,subject,bodyPreview,from,sender,toRecipients,\
ccRecipients,bccRecipients,replyTo,receivedDateTime,sentDateTime,lastModifiedDateTime,isRead,isDraft,flag,categories,\
hasAttachments,parentFolderId,internetMessageHeaders";
/// What delta pages carry: only the thread id, because touched threads are re-read (D-211).
pub const DELTA_SELECT: &str = "conversationId";
const FOLDER_SELECT: &str = "id,displayName,parentFolderId,childFolderCount,isHidden";
const MAX_FOLDERS: usize = 1000;
const MAX_FOLDER_DEPTH: u32 = 10;
const MAX_PAGES: usize = 20;
const MAX_BASELINE_ROUNDS: usize = 5000;

/// Adapter settings for one account.
#[derive(Clone, Debug)]
pub struct GraphConfig {
    /// API base, [`GRAPH_BASE`] unless testing.
    pub base_url: String,
    /// Use change notifications (`SYNC_PUSH` on and a public `PUBLIC_BASE_URL`); otherwise poll.
    pub push: bool,
    /// Poll interval without push (§9.6: 2 min).
    pub poll_every: Duration,
    /// The initial-sync window; delta tracking covers messages received inside it (D-211).
    pub window_ms: i64,
    /// The account's own addresses (lowercase), for `untrash` of sent mail.
    pub self_addresses: Vec<String>,
    /// Request budget (§9.4).
    pub limits: Limits,
    /// How long the folder list is reused by `changes_since` before it is re-read.
    pub folder_cache_ms: i64,
    /// Drafts up to this many MIME bytes are created from MIME; larger ones as JSON (D-213).
    pub mime_draft_max_bytes: usize,
    /// Attachments of at least this size go through an upload session.
    pub upload_session_min_bytes: usize,
    /// Upload-session chunk size (a multiple of 320 KiB).
    pub upload_chunk_bytes: usize,
}

impl Default for GraphConfig {
    fn default() -> Self {
        Self {
            base_url: GRAPH_BASE.to_owned(),
            push: true,
            poll_every: Duration::from_secs(120),
            window_ms: akatsuki_core::sync::WINDOW_MS,
            self_addresses: Vec::new(),
            limits: Limits::default(),
            folder_cache_ms: 10 * 60 * 1000,
            mime_draft_max_bytes: 2_800_000,
            upload_session_min_bytes: 3 * 1024 * 1024,
            upload_chunk_bytes: 10 * 320 * 1024,
        }
    }
}

/// The signed-in mailbox (`GET /me`), read at connect time.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GraphProfile {
    /// Directory or MSA object id.
    pub id: String,
    /// Display name.
    pub display_name: Option<String>,
    /// `mail`, else `userPrincipalName`: the `providerAccountId` (as given, D-127).
    pub email: String,
}

/// The folder and category set of the mailbox, cached for `folder_cache_ms`.
#[derive(Debug)]
pub(crate) struct FolderSet {
    pub at: UnixMs,
    pub labels: Vec<RemoteLabel>,
    /// Synced folder ids (every non-category label).
    pub folders: Vec<String>,
    /// Well-known name → folder id.
    pub well_known: HashMap<String, String>,
    pub snoozed: Option<String>,
}

impl FolderSet {
    pub fn wk(&self, name: &str) -> Option<&str> {
        self.well_known.get(name).map(String::as_str)
    }
}

/// One Microsoft mailbox.
pub struct GraphProvider {
    pub(crate) http: GraphHttp,
    pub(crate) cfg: GraphConfig,
    caps: ProviderCaps,
    pub(crate) clock: Clock,
    folders: tokio::sync::Mutex<Option<Arc<FolderSet>>>,
}

impl std::fmt::Debug for GraphProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GraphProvider").field("http", &self.http).field("caps", &self.caps).finish_non_exhaustive()
    }
}

/// An IMAP-style body-section path (`1`, `1.2`), the part ids `crates/mime` produces (D-095).
fn is_section_path(part: &str) -> bool {
    !part.is_empty() && part.split('.').all(|s| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit()))
}

impl GraphProvider {
    /// An adapter with its own HTTP client (10 s connect timeout, 60 s read timeout).
    pub fn new(cfg: GraphConfig, tokens: Arc<dyn TokenSource>) -> Self {
        let http = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .read_timeout(Duration::from_secs(60))
            .user_agent(concat!("akatsuki-sync/", env!("CARGO_PKG_VERSION")))
            .build()
            .unwrap_or_default();
        Self::with_http(http, cfg, tokens)
    }

    /// An adapter over an existing client.
    pub fn with_http(http: reqwest::Client, cfg: GraphConfig, tokens: Arc<dyn TokenSource>) -> Self {
        let mut caps = ProviderCaps::graph();
        if !cfg.push {
            caps.push = PushKind::Poll { every: cfg.poll_every };
        }
        Self {
            http: GraphHttp::new(http, &cfg.base_url, tokens, cfg.limits),
            cfg,
            caps,
            clock: system_clock(),
            folders: tokio::sync::Mutex::new(None),
        }
    }

    /// Replaces the clock (tests).
    pub fn with_clock(mut self, clock: Clock) -> Self {
        self.clock = clock;
        self
    }

    pub(crate) fn now(&self) -> UnixMs {
        (self.clock)()
    }

    /// The signed-in mailbox (§6.7 step 3.3).
    pub async fn profile(&self) -> Result<GraphProfile, ProviderError> {
        let u: User = self
            .http
            .get_json(
                &self.http.url(&format!("/me?{}", query(&[("$select", "id,displayName,mail,userPrincipalName")]))),
                None,
            )
            .await?;
        let email = u
            .mail
            .filter(|m| !m.trim().is_empty())
            .or(u.user_principal_name)
            .map(|e| e.trim().to_owned())
            .ok_or_else(|| ProviderError::Invalid("graph: mailbox has no address".into()))?;
        Ok(GraphProfile { id: u.id, display_name: u.display_name, email })
    }

    /// Re-authorizes a subscription after a `reauthorizationRequired` lifecycle event (§9.6).
    pub async fn reauthorize(&self, subscription_id: &str) -> Result<(), ProviderError> {
        let url = self.http.url(&format!("/subscriptions/{}/reauthorize", enc(subscription_id)));
        self.http.send(Method::POST, &url, None, Body::Json(json!({}))).await.map(|_| ())
    }

    /// Drops the cached folder list (after creating or deleting folders and categories).
    pub(crate) async fn invalidate_folders(&self) {
        *self.folders.lock().await = None;
    }

    /// The folder set, re-read when `force` or when the cache is older than `folder_cache_ms`.
    pub(crate) async fn folder_set(&self, force: bool) -> Result<Arc<FolderSet>, ProviderError> {
        let mut guard = self.folders.lock().await;
        if let Some(fs) = guard.as_ref()
            && !force
            && self.now() - fs.at < self.cfg.folder_cache_ms
        {
            return Ok(fs.clone());
        }
        let fs = Arc::new(self.load_folders().await?);
        *guard = Some(fs.clone());
        Ok(fs)
    }

    async fn folder_page_all(&self, first: Page<Folder>, out: &mut Vec<Folder>) -> Result<Vec<String>, ProviderError> {
        let mut parents = Vec::new();
        let mut page = first;
        for _ in 0..MAX_PAGES {
            for f in page.value.drain(..) {
                if f.child_folder_count.unwrap_or(0) > 0 {
                    parents.push(f.id.clone());
                }
                out.push(f);
            }
            match page.next_link.take() {
                Some(link) if out.len() < MAX_FOLDERS => {
                    self.http.check_link(&link)?;
                    page = self.http.get_json(&link, None).await?;
                }
                _ => break,
            }
        }
        Ok(parents)
    }

    async fn load_folders(&self) -> Result<FolderSet, ProviderError> {
        // Well-known folder ids (one batch).
        let names: Vec<&str> =
            SYNCED_WELL_KNOWN.iter().map(|(n, _)| *n).chain(SKIPPED_WELL_KNOWN.iter().copied()).collect();
        let subs: Vec<Sub> =
            names.iter().map(|n| Sub::get(format!("/me/mailFolders/{n}?{}", query(&[("$select", "id")])))).collect();
        let mut well_known = HashMap::new();
        for (name, r) in names.iter().zip(self.http.batch(&subs).await?) {
            match r.into_result() {
                Ok(v) => {
                    if let Some(id) = v.get("id").and_then(Value::as_str) {
                        well_known.insert((*name).to_owned(), id.to_owned());
                    }
                }
                Err(ProviderError::NotFound | ProviderError::Invalid(_)) => {}
                Err(e) => return Err(e),
            }
        }

        // The folder tree: top level, then children breadth-first, 20 parents per batch.
        let q = query(&[("$top", "100"), ("$select", FOLDER_SELECT)]);
        let mut all = Vec::new();
        let top: Page<Folder> = self.http.get_json(&self.http.url(&format!("/me/mailFolders?{q}")), None).await?;
        let mut frontier = self.folder_page_all(top, &mut all).await?;
        let mut depth = 0;
        while !frontier.is_empty() && depth < MAX_FOLDER_DEPTH && all.len() < MAX_FOLDERS {
            let subs: Vec<Sub> =
                frontier.iter().map(|id| Sub::get(format!("/me/mailFolders/{}/childFolders?{q}", enc(id)))).collect();
            let mut next = Vec::new();
            for r in self.http.batch(&subs).await? {
                let page: Page<Folder> = match r.into_result() {
                    Ok(v) => serde_json::from_value(v)
                        .map_err(|_| ProviderError::Transient("graph: malformed folder page".into(), None))?,
                    Err(ProviderError::NotFound) => continue, // deleted meanwhile
                    Err(e) => return Err(e),
                };
                next.extend(self.folder_page_all(page, &mut all).await?);
            }
            frontier = next;
            depth += 1;
        }

        let mut labels: Vec<RemoteLabel> = map::folder_labels(&all, &well_known).into_iter().map(|(l, _)| l).collect();
        let folders: Vec<String> = labels.iter().map(|l| l.provider_label_id.clone()).collect();
        let snoozed = labels.iter().find(|l| l.role == Some(LabelRole::Snoozed)).map(|l| l.provider_label_id.clone());
        match self.categories().await {
            Ok(cats) => labels.extend(cats.iter().map(map::category_label)),
            // Some mailboxes (shared or legacy) have no master category list.
            Err(ProviderError::NotFound | ProviderError::Invalid(_)) => {}
            Err(e) => return Err(e),
        }
        Ok(FolderSet { at: self.now(), labels, folders, well_known, snoozed })
    }

    pub(crate) async fn categories(&self) -> Result<Vec<Category>, ProviderError> {
        let page: Page<Category> = self.http.get_json(&self.http.url("/me/outlook/masterCategories"), None).await?;
        Ok(page.value)
    }

    fn initial_delta_url(&self, folder_id: &str) -> String {
        let since = self.now() - self.cfg.window_ms;
        self.http.url(&format!(
            "/me/mailFolders/{}/messages/delta?{}",
            enc(folder_id),
            query(&[("$select", DELTA_SELECT), ("$filter", &format!("receivedDateTime ge {}", format_time(since)))])
        ))
    }

    fn delta_sub(&self, url: &str) -> Result<Sub, ProviderError> {
        Ok(Sub::get(self.http.relative(url)?).with_prefer(&format!("odata.maxpagesize={DELTA_PAGE_SIZE}")))
    }

    /// Every message of the given conversations (header fields), batched 20 per request.
    pub(crate) async fn fetch_conversations(&self, ids: &[String]) -> Result<Vec<Message>, ProviderError> {
        let subs: Vec<Sub> = ids
            .iter()
            .map(|c| {
                Sub::get(format!(
                    "/me/messages?{}",
                    query(&[
                        ("$filter", &format!("conversationId eq {}", odata_str(c))),
                        ("$select", MESSAGE_SELECT),
                        ("$top", "100"),
                    ])
                ))
            })
            .collect();
        let mut out = Vec::new();
        for r in self.http.batch(&subs).await? {
            let mut page: Page<Message> = serde_json::from_value(r.into_result()?)
                .map_err(|_| ProviderError::Transient("graph: malformed message page".into(), None))?;
            for _ in 0..MAX_PAGES {
                out.append(&mut page.value);
                match page.next_link.take() {
                    Some(link) => {
                        self.http.check_link(&link)?;
                        page = self.http.get_json(&link, None).await?;
                    }
                    None => break,
                }
            }
        }
        Ok(out)
    }

    /// The id of `Akatsuki/Snoozed`, creating it when missing.
    pub(crate) async fn snoozed_folder(&self) -> Result<String, ProviderError> {
        if let Some(id) = self.folder_set(false).await?.snoozed.clone() {
            return Ok(id);
        }
        Ok(self.ensure_label(LabelRole::Snoozed, SNOOZED_LABEL_NAME).await?.provider_label_id)
    }

    /// Finds a folder by display name among `parent`'s children (or the top level), creating it
    /// if missing. A 409 from a concurrent create is resolved by listing again.
    async fn find_or_create_folder(&self, parent: Option<&str>, name: &str) -> Result<String, ProviderError> {
        let list_url = match parent {
            Some(p) => self.http.url(&format!(
                "/me/mailFolders/{}/childFolders?{}",
                enc(p),
                query(&[("$top", "100"), ("$select", FOLDER_SELECT)])
            )),
            None => {
                self.http.url(&format!("/me/mailFolders?{}", query(&[("$top", "100"), ("$select", FOLDER_SELECT)])))
            }
        };
        let find = |page: &Page<Folder>| {
            page.value
                .iter()
                .find(|f| f.display_name.as_deref().is_some_and(|n| n.eq_ignore_ascii_case(name)))
                .map(|f| f.id.clone())
        };
        let page: Page<Folder> = self.http.get_json(&list_url, None).await?;
        if let Some(id) = find(&page) {
            return Ok(id);
        }
        let create_url = match parent {
            Some(p) => self.http.url(&format!("/me/mailFolders/{}/childFolders", enc(p))),
            None => self.http.url("/me/mailFolders"),
        };
        match self.http.send(Method::POST, &create_url, None, Body::Json(json!({ "displayName": name }))).await {
            Ok(resp) => resp.json::<Folder>().map(|f| f.id),
            Err(ProviderError::Conflict(_)) => {
                let page: Page<Folder> = self.http.get_json(&list_url, None).await?;
                find(&page).ok_or_else(|| ProviderError::Conflict("graph: folder exists but was not listed".into()))
            }
            Err(e) => Err(e),
        }
    }
}

#[async_trait]
impl MailProvider for GraphProvider {
    fn kind(&self) -> ProviderKind {
        ProviderKind::Graph
    }

    fn caps(&self) -> &ProviderCaps {
        &self.caps
    }

    async fn list_labels(&self) -> Result<Vec<RemoteLabel>, ProviderError> {
        Ok(self.folder_set(true).await?.labels.clone())
    }

    async fn ensure_label(&self, role: LabelRole, name: &str) -> Result<RemoteLabel, ProviderError> {
        let fs = self.folder_set(false).await?;
        if let Some(l) = fs.labels.iter().find(|l| l.role == Some(role)) {
            return Ok(l.clone());
        }
        let label = |id: String, name: &str| RemoteLabel {
            provider_label_id: id,
            name: name.to_owned(),
            kind: LabelKind::Folder,
            role: Some(role),
            exclusive: true,
            color: None,
            hidden: false,
        };
        let created = match role {
            LabelRole::Snoozed => {
                let (parent, child) = SNOOZED_LABEL_NAME.split_once('/').unwrap_or(("Akatsuki", "Snoozed"));
                let pid = self.find_or_create_folder(None, parent).await?;
                label(self.find_or_create_folder(Some(&pid), child).await?, SNOOZED_LABEL_NAME)
            }
            LabelRole::Archive => label(self.find_or_create_folder(None, name).await?, name),
            _ => return Err(ProviderError::Invalid("graph: this folder role cannot be created".into())),
        };
        self.invalidate_folders().await;
        Ok(created)
    }

    async fn baseline(&self) -> Result<Cursor, ProviderError> {
        let fs = self.folder_set(true).await?;
        let mut pending: Vec<(String, String)> =
            fs.folders.iter().map(|id| (id.clone(), self.initial_delta_url(id))).collect();
        let mut done = BTreeMap::new();
        for _ in 0..MAX_BASELINE_ROUNDS {
            if pending.is_empty() {
                break;
            }
            let subs = pending.iter().map(|(_, u)| self.delta_sub(u)).collect::<Result<Vec<_>, _>>()?;
            let mut next = Vec::new();
            for ((folder, _), r) in pending.into_iter().zip(self.http.batch(&subs).await?) {
                let page: Page<Message> = match r.into_result() {
                    Ok(v) => serde_json::from_value(v)
                        .map_err(|_| ProviderError::Transient("graph: malformed delta page".into(), None))?,
                    Err(ProviderError::NotFound) => continue, // folder deleted meanwhile
                    Err(e) => return Err(e),
                };
                match (page.delta_link, page.next_link) {
                    (Some(d), _) => {
                        done.insert(folder, GraphFolderCursor { delta_link: Some(d) });
                    }
                    (None, Some(n)) => next.push((folder, n)),
                    (None, None) => {
                        return Err(ProviderError::Transient("graph: delta page without a link".into(), None));
                    }
                }
            }
            pending = next;
        }
        if !pending.is_empty() {
            return Err(ProviderError::Transient("graph: baseline did not finish".into(), None));
        }
        Ok(Cursor::Graph { folders: done })
    }

    async fn changes_since(&self, cursor: &Cursor) -> Result<ChangePage, ProviderError> {
        let Cursor::Graph { folders } = cursor else {
            return Err(ProviderError::Invalid("graph: not a Graph cursor".into()));
        };
        let fs = self.folder_set(false).await?;
        let tracked: BTreeSet<&str> = fs.folders.iter().map(String::as_str).collect();
        let known: BTreeSet<&str> = folders.keys().map(String::as_str).collect();
        let mut labels_changed = tracked != known;

        let mut plan = Vec::new();
        for id in &tracked {
            let url = match folders.get(*id).and_then(|c| c.delta_link.clone()) {
                Some(link) => link,
                None => self.initial_delta_url(id),
            };
            plan.push(((*id).to_owned(), url));
        }
        let subs = plan.iter().map(|(_, u)| self.delta_sub(u)).collect::<Result<Vec<_>, _>>()?;
        let responses = self.http.batch(&subs).await?;

        let mut next = BTreeMap::new();
        let mut touched = BTreeSet::new();
        let mut present = HashSet::new();
        let mut removed = Vec::new();
        let mut more = false;
        for ((folder, _), r) in plan.into_iter().zip(responses) {
            let page: Page<Message> = match r.into_result() {
                Ok(v) => serde_json::from_value(v)
                    .map_err(|_| ProviderError::Transient("graph: malformed delta page".into(), None))?,
                Err(ProviderError::NotFound) => {
                    // The folder was deleted since the folder list was read.
                    labels_changed = true;
                    self.invalidate_folders().await;
                    continue;
                }
                Err(e) => return Err(e),
            };
            for m in page.value {
                if m.removed.is_some() {
                    removed.push(m.id);
                } else if let Some(c) = m.conversation_id.filter(|c| !c.is_empty()) {
                    present.insert(m.id);
                    touched.insert(c);
                } else {
                    removed.push(m.id); // no thread id in the page: look it up below
                }
            }
            let link = match (page.delta_link, page.next_link) {
                (Some(d), _) => d,
                (None, Some(n)) => {
                    more = true;
                    n
                }
                (None, None) => return Err(ProviderError::Transient("graph: delta page without a link".into(), None)),
            };
            self.http.check_link(&link)?;
            next.insert(folder, GraphFolderCursor { delta_link: Some(link) });
        }

        // A message that left a folder is either somewhere else (a move) or gone.
        let mut check: Vec<String> = removed.into_iter().filter(|id| !present.contains(id)).collect();
        check.sort();
        check.dedup();
        let subs: Vec<Sub> = check
            .iter()
            .map(|id| Sub::get(format!("/me/messages/{}?{}", enc(id), query(&[("$select", "id,conversationId")]))))
            .collect();
        let mut deleted_messages = Vec::new();
        for (id, r) in check.into_iter().zip(self.http.batch(&subs).await?) {
            match r.into_result() {
                Ok(v) => {
                    if let Some(c) = v.get("conversationId").and_then(Value::as_str) {
                        touched.insert(c.to_owned());
                    }
                }
                Err(ProviderError::NotFound) => deleted_messages.push(id),
                Err(e) => return Err(e),
            }
        }

        Ok(ChangePage {
            touched_threads: touched.into_iter().map(RemoteThreadId).collect(),
            deleted_threads: Vec::new(),
            deleted_messages,
            labels_changed,
            next: Cursor::Graph { folders: next },
            more,
        })
    }

    async fn list_window(&self, req: &WindowRequest) -> Result<WindowPage, ProviderError> {
        let select = "conversationId,receivedDateTime";
        let url = match req {
            WindowRequest::Inbox { page: Some(link) } | WindowRequest::Since { page: Some(link), .. } => {
                self.http.check_link(link)?;
                link.clone()
            }
            WindowRequest::Inbox { page: None } => self.http.url(&format!(
                "/me/mailFolders/inbox/messages?{}",
                query(&[("$select", select), ("$orderby", "receivedDateTime desc"), ("$top", "50")])
            )),
            WindowRequest::Since { after, page: None } => self.http.url(&format!(
                "/me/messages?{}",
                query(&[
                    ("$filter", &format!("receivedDateTime ge {}", format_time(*after))),
                    ("$orderby", "receivedDateTime desc"),
                    ("$select", select),
                    ("$top", "100"),
                ])
            )),
            WindowRequest::Before { before, horizon, limit } => self.http.url(&format!(
                "/me/messages?{}",
                query(&[
                    (
                        "$filter",
                        &format!(
                            "receivedDateTime lt {} and receivedDateTime ge {}",
                            format_time(*before),
                            format_time(*horizon)
                        ),
                    ),
                    ("$orderby", "receivedDateTime desc"),
                    ("$select", select),
                    ("$top", &(*limit).clamp(1, 1000).to_string()),
                ])
            )),
        };
        let page: Page<Message> = self.http.get_json(&url, None).await?;
        let mut seen = HashSet::new();
        let mut threads = Vec::new();
        let mut oldest: Option<UnixMs> = None;
        for m in &page.value {
            if let Some(t) = m.received_date_time.as_deref().and_then(parse_time) {
                oldest = Some(oldest.map_or(t, |o| o.min(t)));
            }
            if let Some(c) = m.conversation_id.as_deref().filter(|c| !c.is_empty())
                && seen.insert(c.to_owned())
            {
                threads.push(RemoteThreadId(c.to_owned()));
            }
        }
        let next_page = match req {
            // Backfill pages by date: the engine moves `before` to `oldest`.
            WindowRequest::Before { .. } => None,
            _ => page.next_link,
        };
        if let Some(n) = &next_page {
            self.http.check_link(n)?;
        }
        Ok(WindowPage { threads, next_page, oldest })
    }

    async fn fetch_threads(
        &self,
        ids: &[RemoteThreadId],
        _depth: FetchDepth,
    ) -> Result<Vec<FetchedThread>, ProviderError> {
        let order: Vec<String> = ids.iter().map(|t| t.0.clone()).collect();
        let msgs = self.fetch_conversations(&order).await?;
        Ok(map::threads(&order, msgs))
    }

    async fn fetch_raw(&self, msg: &RemoteMessageRef) -> Result<ByteStream, ProviderError> {
        self.http.stream(&self.http.url(&format!("/me/messages/{}/$value", enc(&msg.provider_message_id)))).await
    }

    async fn fetch_part(&self, msg: &RemoteMessageRef, part_id: &str) -> Result<ByteStream, ProviderError> {
        if let Some(att) = part_id.strip_prefix("att:") {
            let url = self.http.url(&format!(
                "/me/messages/{}/attachments/{}/$value",
                enc(&msg.provider_message_id),
                enc(att)
            ));
            return self.http.stream(&url).await;
        }
        if !is_section_path(part_id) {
            return Err(ProviderError::Invalid("graph: unknown part id".into()));
        }
        let url = self.http.url(&format!("/me/messages/{}/$value", enc(&msg.provider_message_id)));
        let raw = self.http.send(Method::GET, &url, None, Body::Empty).await?.body;
        let part = akatsuki_mime::part_content(&raw, part_id)
            .map_err(|_| ProviderError::Invalid("graph: message source is not parseable".into()))?
            .ok_or(ProviderError::NotFound)?;
        let bytes = Bytes::from(part.bytes);
        Ok(futures::stream::once(async move { Ok(bytes) }).boxed())
    }

    async fn apply(&self, ops: &[(OpId, ProviderOp)]) -> Vec<(OpId, Result<OpEffect, ProviderError>)> {
        apply::apply(self, ops).await
    }

    async fn save_draft(
        &self,
        mime: &OutgoingMime,
        existing: Option<&RemoteDraftRef>,
    ) -> Result<RemoteDraftRef, ProviderError> {
        drafts::save_draft(self, mime, existing).await
    }

    async fn delete_draft(&self, d: &RemoteDraftRef) -> Result<(), ProviderError> {
        drafts::delete_message(self, &d.provider_draft_id).await
    }

    async fn send_draft(&self, d: &RemoteDraftRef, rfc_message_id: &str) -> Result<SentRef, ProviderError> {
        drafts::send_draft(self, d, rfc_message_id).await
    }

    async fn find_sent(&self, rfc_message_id: &str) -> Result<Option<SentRef>, ProviderError> {
        drafts::find_sent(self, rfc_message_id).await
    }

    async fn search(
        &self,
        q: &akatsuki_search::query::Ast,
        page: Option<String>,
    ) -> Result<(Vec<RemoteThreadId>, Option<String>), ProviderError> {
        let url = match page {
            Some(link) => {
                self.http.check_link(&link)?;
                link
            }
            None => {
                let k =
                    to_kql(q, self.now()).map_err(|KqlError::Unsupported(what)| ProviderError::Unsupported(what))?;
                let path = match k.folder {
                    Some(f) => format!("/me/mailFolders/{f}/messages"),
                    None => "/me/messages".to_owned(),
                };
                let select = "conversationId,receivedDateTime";
                let qs = match &k.search {
                    Some(s) => query(&[("$search", &kql::search_param(s)), ("$select", select), ("$top", "50")]),
                    None => query(&[("$orderby", "receivedDateTime desc"), ("$select", select), ("$top", "50")]),
                };
                self.http.url(&format!("{path}?{qs}"))
            }
        };
        let page: Page<Message> = self.http.get_json(&url, None).await?;
        let mut seen = HashSet::new();
        let ids = page
            .value
            .iter()
            .filter_map(|m| m.conversation_id.clone())
            .filter(|c| seen.insert(c.clone()))
            .map(RemoteThreadId)
            .collect();
        if let Some(n) = &page.next_link {
            self.http.check_link(n)?;
        }
        Ok((ids, page.next_link))
    }

    async fn watch(&self, current: Option<&WatchState>, hook_base: &str) -> Result<Option<WatchState>, ProviderError> {
        if !self.cfg.push {
            return Ok(None);
        }
        let now = self.now();
        let expires = now + SUBSCRIPTION_MINUTES * 60_000;
        if let Some(w) = current.filter(|w| w.kind == WatchKind::GraphSubscription)
            && let Some(id) = &w.external_id
        {
            if w.expires_at - now > RENEW_BEFORE_MS {
                return Ok(Some(w.clone()));
            }
            let url = self.http.url(&format!("/subscriptions/{}", enc(id)));
            let body = json!({ "expirationDateTime": format_time(expires) });
            match self.http.send(Method::PATCH, &url, None, Body::Json(body)).await {
                Ok(resp) => {
                    let s: Subscription = resp.json()?;
                    return Ok(Some(WatchState {
                        kind: WatchKind::GraphSubscription,
                        external_id: Some(id.clone()),
                        expires_at: s.expiration_date_time.as_deref().and_then(parse_time).unwrap_or(expires),
                        // Unchanged at Graph; `None` means "keep the stored hash".
                        client_state: w.client_state.clone(),
                    }));
                }
                Err(ProviderError::NotFound) => {} // expired or removed: create a new one
                Err(e) => return Err(e),
            }
        }
        let base = hook_base.trim_end_matches('/');
        let client_state = new_client_state();
        let body = json!({
            "changeType": "created,updated,deleted",
            "notificationUrl": format!("{base}/hooks/graph"),
            "lifecycleNotificationUrl": format!("{base}/hooks/graph/lifecycle"),
            "resource": "me/messages",
            "expirationDateTime": format_time(expires),
            "clientState": client_state,
        });
        let s: Subscription =
            self.http.send(Method::POST, &self.http.url("/subscriptions"), None, Body::Json(body)).await?.json()?;
        Ok(Some(WatchState {
            kind: WatchKind::GraphSubscription,
            external_id: Some(s.id),
            expires_at: s.expiration_date_time.as_deref().and_then(parse_time).unwrap_or(expires),
            client_state: Some(client_state),
        }))
    }

    async fn unwatch(&self, w: &WatchState) -> Result<(), ProviderError> {
        let Some(id) = &w.external_id else { return Ok(()) };
        match self
            .http
            .send(Method::DELETE, &self.http.url(&format!("/subscriptions/{}", enc(id))), None, Body::Empty)
            .await
        {
            Ok(_) | Err(ProviderError::NotFound) => Ok(()),
            Err(e) => Err(e),
        }
    }

    /// Microsoft has no endpoint that revokes one application's refresh token for a mailbox
    /// (`revokeSignInSessions` would sign the user out everywhere), so removal drops the sealed
    /// token and the subscription; the user removes the app's consent in their account (D-218).
    async fn revoke(&self) -> Result<(), ProviderError> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn section_paths() {
        assert!(is_section_path("1"));
        assert!(is_section_path("1.2.10"));
        assert!(!is_section_path(""));
        assert!(!is_section_path("1..2"));
        assert!(!is_section_path("att:x"));
    }

    #[test]
    fn caps_follow_push_setting() {
        struct NoTokens;
        #[async_trait]
        impl TokenSource for NoTokens {
            async fn access_token(&self) -> Result<crate::AccessToken, ProviderError> {
                Err(ProviderError::Auth { reauth_required: true })
            }
            async fn invalidate(&self) {}
        }
        let p = GraphProvider::new(GraphConfig { push: false, ..GraphConfig::default() }, Arc::new(NoTokens));
        assert_eq!(p.caps().push, PushKind::Poll { every: Duration::from_secs(120) });
        assert_eq!(p.kind(), ProviderKind::Graph);
        assert!(!p.caps().multi_label && p.caps().delete_forever && p.caps().max_batch_modify == 20);
        let p = GraphProvider::new(GraphConfig::default(), Arc::new(NoTokens));
        assert_eq!(p.caps().push, PushKind::GraphWebhook);
    }
}
