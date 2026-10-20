//! The Gmail adapter (ARCHITECTURE §9.1, §9.4–§9.7, §10.2, §11.2): [`GmailProvider`] implements
//! [`MailProvider`] over the Gmail REST API v1 with the `gmail.modify` scope (D-039).
//!
//! - **Auth**: [`auth::GoogleTokenSource`] refreshes access tokens single-flight at `exp − 5 min`
//!   for web and installed OAuth clients; a 401 is retried once with a fresh token, then becomes
//!   `Auth { reauth_required: true }`.
//! - **Sync**: `baseline` reads `users.getProfile.historyId`; `changes_since` pages through
//!   `history.list` and returns touched threads (re-fetched with `threads.get format=metadata`
//!   in HTTP batches of 50); a 404 on `startHistoryId` is `CursorExpired` (full resync, §9.3).
//! - **Push**: `users.watch` to `GMAIL_PUBSUB_TOPIC` when configured. Without a topic, or when
//!   Gmail refuses the watch (Pub/Sub not set up, billing not linked), `watch` returns `None` and
//!   the engine polls `changes_since` every [`GmailProvider::poll_interval`] (60 s). Polling is a
//!   first-class mode, not an error (D-205).
//! - **Ops**: deltas touching `sys:trash` use `threads.trash`/`untrash`; other deltas use
//!   `threads.modify`, or `messages.batchModify` when at least
//!   [`GmailConfig::batch_modify_min_threads`] consecutive ops carry the same delta. Every applied
//!   op returns the re-read thread as `post`. `delete_forever` and `move` are `Unsupported`.
//! - **Drafts and sends**: `drafts.create/update/delete/send` and `messages.send` with `raw`
//!   (large messages through the multipart upload endpoint). [`MailProvider::send_draft`] and
//!   [`GmailProvider::send_raw`] look for the message in Sent first (`rfc822msgid:`), so a retry
//!   never sends twice (§5.6, §10.2).
//! - **Budget**: every call is charged to [`quota::AccountQuota`] first (§9.4, S7).
//!
//! Bodies are never parsed or logged here: `fetch_raw`/`fetch_part` stream bytes to the caller.

pub mod api;
pub mod auth;
pub mod batch;
mod client;
pub mod convert;
pub mod push;
pub mod quota;

#[cfg(test)]
mod tests;

use std::collections::HashSet;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use bytes::Bytes;
use futures::StreamExt as _;
use serde_json::json;

use self::api::{
    Draft, Label, ListHistory, ListLabels, ListMessages, ListSendAs, ListThreads, Message, Profile, SendAs, Thread,
};
use self::client::{Body, Call, Client};
use self::convert::{
    METADATA_HEADERS, decode_b64, encode_b64, fetched_thread, part_by_section, remote_label, system_label_id,
};
use self::quota::{AccountQuota, Lane, Method};
use crate::labels::{SNOOZED_LABEL_NAME, gmail_system_label};
use crate::{
    ByteStream, ChangePage, Cursor, FetchDepth, FetchedThread, LabelRole, LabelTarget, MailProvider, OpEffect, OpId,
    OutgoingMime, ProviderCaps, ProviderError, ProviderKind, ProviderOp, PushKind, RemoteDraftRef, RemoteLabel,
    RemoteMessageRef, RemoteThreadId, RemoteThreadRef, SentRef, SysLabel, TokenSource, UnixMs, WatchKind, WatchState,
    WindowPage, WindowRequest,
};

pub use akatsuki_search::gmail::{GmailQuery, TranslateError};

/// Wall clock in ms (injectable for tests).
pub type Clock = Arc<dyn Fn() -> UnixMs + Send + Sync>;

/// The system clock.
pub fn system_clock() -> Clock {
    Arc::new(|| {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
            .unwrap_or(0)
    })
}

/// The Gmail API host.
pub const GMAIL_API_BASE: &str = "https://gmail.googleapis.com";
/// Labels the watch reports (§9.6).
pub const WATCH_LABEL_IDS: [&str; 6] = ["INBOX", "SENT", "DRAFT", "TRASH", "SPAM", "STARRED"];
/// A watch lasts 7 days.
pub const WATCH_TTL_MS: i64 = 7 * 24 * 60 * 60 * 1000;

/// Adapter settings. `Default` is production.
#[derive(Clone, Debug)]
pub struct GmailConfig {
    /// API host (a mock server in tests).
    pub api_base: String,
    /// Google's revocation endpoint.
    pub revoke_url: String,
    /// `me` (the token's mailbox).
    pub user_id: String,
    /// `GMAIL_PUBSUB_TOPIC` (`projects/<p>/topics/<t>`); `None` (or `SYNC_PUSH=off`) means polling.
    pub pubsub_topic: Option<String>,
    /// Poll interval without push (§9.6: 60 s).
    pub poll_every: Duration,
    /// Calls per HTTP batch (§9.3: 50; Google allows 100).
    pub batch_size: usize,
    /// `maxResults` for thread and message lists (≤ 500).
    pub list_page_size: u32,
    /// `maxResults` for `history.list` (≤ 500).
    pub history_page_size: u32,
    /// `history.list` pages followed within one `changes_since` call.
    pub history_pages_per_call: u32,
    /// Consecutive identical deltas from which `messages.batchModify` is used.
    pub batch_modify_min_threads: usize,
    /// Messages larger than this go through the `/upload` multipart endpoint.
    pub upload_threshold: usize,
}

impl Default for GmailConfig {
    fn default() -> Self {
        Self {
            api_base: GMAIL_API_BASE.into(),
            revoke_url: auth::GOOGLE_REVOKE_URL.into(),
            user_id: "me".into(),
            pubsub_topic: None,
            poll_every: Duration::from_secs(60),
            batch_size: 50,
            list_page_size: 100,
            history_page_size: 500,
            history_pages_per_call: 10,
            batch_modify_min_threads: 5,
            upload_threshold: 4 * 1024 * 1024,
        }
    }
}

#[derive(Default)]
struct LabelCache {
    /// Every label id from the last listing (None before the first one).
    known: Option<HashSet<String>>,
    /// All labels from the last listing.
    labels: Vec<RemoteLabel>,
    /// `Akatsuki/Snoozed`.
    snoozed_id: Option<String>,
}

/// One Gmail account.
pub struct GmailProvider {
    cfg: GmailConfig,
    caps: ProviderCaps,
    client: Client,
    clock: Clock,
    labels: Mutex<LabelCache>,
}

impl std::fmt::Debug for GmailProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GmailProvider").field("api_base", &self.cfg.api_base).finish_non_exhaustive()
    }
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn secs_floor(ms: UnixMs) -> i64 {
    ms.div_euclid(1000)
}

fn secs_ceil(ms: UnixMs) -> i64 {
    -((-ms).div_euclid(1000))
}

/// `true` when a send failed in a way that leaves it unknown whether Gmail accepted the message
/// (timeout, reset, 5xx after the upload began). The engine then sets `verifyOnly` (§5.6). A
/// connection that never opened, a 4xx or a rate limit means Gmail certainly did not send it.
pub fn is_ambiguous_send_error(e: &ProviderError) -> bool {
    matches!(e, ProviderError::Transient(m, _) if !m.starts_with("network: connect"))
}

fn one_chunk(bytes: Vec<u8>) -> ByteStream {
    futures::stream::once(async move { Ok(Bytes::from(bytes)) }).boxed()
}

fn clean_message_id(id: &str) -> Result<String, ProviderError> {
    let id = id.trim().trim_start_matches('<').trim_end_matches('>').trim();
    if id.is_empty()
        || id.chars().any(|c| c.is_whitespace() || c.is_control() || matches!(c, '"' | '(' | ')' | '{' | '}'))
    {
        return Err(ProviderError::Invalid("bad Message-ID".into()));
    }
    Ok(id.to_owned())
}

/// A `multipart/related` body for the `/upload` endpoint: JSON metadata, then the message.
fn related_body(meta: &serde_json::Value, raw: &[u8]) -> Body {
    let boundary = format!("akatsuki_{}", uuid::Uuid::new_v4().simple());
    let mut b = Vec::with_capacity(raw.len() + 256);
    b.extend_from_slice(format!("--{boundary}\r\nContent-Type: application/json; charset=UTF-8\r\n\r\n{meta}\r\n--{boundary}\r\nContent-Type: message/rfc822\r\n\r\n").as_bytes());
    b.extend_from_slice(raw);
    b.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
    Body::Bytes { content_type: format!("multipart/related; boundary={boundary}"), bytes: Bytes::from(b) }
}

impl GmailProvider {
    /// An adapter for one account.
    pub fn new(
        cfg: GmailConfig,
        tokens: Arc<dyn TokenSource>,
        quota: Arc<AccountQuota>,
        http: reqwest::Client,
    ) -> Self {
        let mut caps = ProviderCaps::gmail();
        if cfg.pubsub_topic.is_none() {
            caps.push = PushKind::Poll { every: cfg.poll_every };
        }
        let client = Client {
            http,
            base: cfg.api_base.trim_end_matches('/').to_owned(),
            user: cfg.user_id.clone(),
            tokens,
            quota,
        };
        Self { cfg, caps, client, clock: system_clock(), labels: Mutex::new(LabelCache::default()) }
    }

    /// Replaces the clock (tests).
    #[must_use]
    pub fn with_clock(mut self, clock: Clock) -> Self {
        self.clock = clock;
        self
    }

    /// The poll interval to use when `watch` returned `None`.
    pub fn poll_interval(&self) -> Duration {
        self.cfg.poll_every
    }

    /// The account budget.
    pub fn quota(&self) -> &Arc<AccountQuota> {
        &self.client.quota
    }

    // ---------------------------------------------------------------------------------------
    // Account

    /// `users.getProfile` (address and current history id).
    pub async fn profile(&self) -> Result<Profile, ProviderError> {
        self.client.json(&Call::get(&["profile"], Method::GetProfile, Lane::Sync)).await
    }

    /// `settings.sendAs.list` (identities, §6.7).
    pub async fn send_as(&self) -> Result<Vec<SendAs>, ProviderError> {
        let r: ListSendAs =
            self.client.json(&Call::get(&["settings", "sendAs"], Method::SendAsList, Lane::Sync)).await?;
        Ok(r.send_as)
    }

    // ---------------------------------------------------------------------------------------
    // Labels

    fn remember_labels(&self, labels: &[RemoteLabel], complete: bool) {
        let mut c = lock(&self.labels);
        if complete {
            c.known = Some(labels.iter().map(|l| l.provider_label_id.clone()).collect());
            c.labels = labels.to_vec();
            c.snoozed_id = None;
        } else {
            let known = c.known.get_or_insert_with(HashSet::new);
            for l in labels {
                known.insert(l.provider_label_id.clone());
            }
            for l in labels {
                c.labels.retain(|x| x.provider_label_id != l.provider_label_id);
                c.labels.push(l.clone());
            }
        }
        if let Some(s) = labels.iter().find(|l| l.role == Some(LabelRole::Snoozed)) {
            c.snoozed_id = Some(s.provider_label_id.clone());
        }
    }

    fn forget_label(&self, id: &str) {
        let mut c = lock(&self.labels);
        if let Some(k) = c.known.as_mut() {
            k.remove(id);
        }
        c.labels.retain(|l| l.provider_label_id != id);
        if c.snoozed_id.as_deref() == Some(id) {
            c.snoozed_id = None;
        }
    }

    fn cached_snoozed_id(&self) -> Option<String> {
        lock(&self.labels).snoozed_id.clone()
    }

    /// `labels.create`. A name that already exists is returned as is (an op replayed after a
    /// crash must not fail). A colour Gmail rejects (it only accepts its palette) is dropped.
    pub async fn create_label(&self, name: &str, color: Option<&str>) -> Result<RemoteLabel, ProviderError> {
        let mut body = json!({"name": name, "labelListVisibility": "labelShow", "messageListVisibility": "show"});
        if let Some(c) = color {
            body["color"] = json!({"backgroundColor": c, "textColor": "#ffffff"});
        }
        let call = Call::post(&["labels"], Method::LabelsCreate, Lane::Sync);
        let created = match self.client.json::<Label>(&call.clone().json(body.clone())).await {
            Err(ProviderError::Invalid(_)) if color.is_some() => {
                body.as_object_mut().map(|o| o.remove("color"));
                self.client.json::<Label>(&call.json(body)).await
            }
            r => r,
        };
        let label = match created {
            Ok(l) => remote_label(&l),
            Err(ProviderError::Conflict(msg)) => {
                let all = self.list_labels().await?;
                all.into_iter().find(|l| l.name.eq_ignore_ascii_case(name)).ok_or(ProviderError::Conflict(msg))?
            }
            Err(e) => return Err(e),
        };
        self.remember_labels(std::slice::from_ref(&label), false);
        Ok(label)
    }

    /// `labels.patch` (rename and/or recolour).
    pub async fn update_label(
        &self,
        id: &str,
        name: Option<&str>,
        color: Option<&str>,
    ) -> Result<RemoteLabel, ProviderError> {
        let mut body = json!({});
        if let Some(n) = name {
            body["name"] = json!(n);
        }
        if let Some(c) = color {
            body["color"] = json!({"backgroundColor": c, "textColor": "#ffffff"});
        }
        let call = Call::new(reqwest::Method::PATCH, &["labels", id], Method::LabelsUpdate, Lane::Sync).json(body);
        let l = remote_label(&self.client.json::<Label>(&call).await?);
        self.remember_labels(std::slice::from_ref(&l), false);
        Ok(l)
    }

    /// `labels.delete`.
    pub async fn delete_label(&self, id: &str) -> Result<(), ProviderError> {
        self.client
            .empty(&Call::new(reqwest::Method::DELETE, &["labels", id], Method::LabelsDelete, Lane::Sync))
            .await?;
        self.forget_label(id);
        Ok(())
    }

    async fn snoozed_label_id(&self) -> Result<String, ProviderError> {
        if let Some(id) = self.cached_snoozed_id() {
            return Ok(id);
        }
        Ok(self.ensure_label(LabelRole::Snoozed, SNOOZED_LABEL_NAME).await?.provider_label_id)
    }

    async fn label_id(&self, t: &LabelTarget) -> Result<String, ProviderError> {
        match t {
            LabelTarget::Sys(SysLabel::Snoozed) => self.snoozed_label_id().await,
            LabelTarget::Sys(s) => Ok(system_label_id(*s).unwrap_or_default().to_owned()),
            LabelTarget::Label { provider_label_id, .. } => Ok(provider_label_id.clone()),
        }
    }

    fn is_unknown_label(&self, id: &str) -> bool {
        if gmail_system_label(id).is_some() || id.starts_with("CATEGORY_") || id == "CHAT" {
            return false;
        }
        match &lock(&self.labels).known {
            Some(k) => !k.contains(id),
            None => true,
        }
    }

    // ---------------------------------------------------------------------------------------
    // Threads and messages

    fn thread_call(&self, id: &str, depth: FetchDepth, lane: Lane) -> Call {
        match depth {
            FetchDepth::Metadata => {
                let mut c =
                    Call::get(&["threads", id], Method::ThreadsGet { full: false }, lane).q("format", "metadata");
                for h in METADATA_HEADERS {
                    c = c.q("metadataHeaders", h);
                }
                c
            }
            FetchDepth::Full => {
                Call::get(&["threads", id], Method::ThreadsGet { full: true }, lane).q("format", "full")
            }
        }
    }

    async fn fetch_threads_lane(
        &self,
        ids: &[RemoteThreadId],
        depth: FetchDepth,
        lane: Lane,
    ) -> Result<Vec<FetchedThread>, ProviderError> {
        let snoozed = self.cached_snoozed_id();
        let mut out = Vec::with_capacity(ids.len());
        let mut seen = HashSet::new();
        let ids: Vec<&RemoteThreadId> = ids.iter().filter(|i| seen.insert(i.0.clone())).collect();
        for chunk in ids.chunks(self.cfg.batch_size.clamp(1, 100)) {
            let calls: Vec<Call> = chunk.iter().map(|id| self.thread_call(&id.0, depth, lane)).collect();
            for r in self.client.batch_get(&calls).await? {
                match r {
                    Ok(bytes) => {
                        let t: Thread = serde_json::from_slice(&bytes)
                            .map_err(|e| ProviderError::Transient(format!("gmail thread: {e}"), None))?;
                        out.push(fetched_thread(&t, snoozed.as_deref()));
                    }
                    Err(ProviderError::NotFound) => {} // deleted: the caller treats missing ids as gone
                    Err(e) => return Err(e),
                }
            }
        }
        Ok(out)
    }

    async fn fetch_one(&self, thread_id: &str) -> Result<FetchedThread, ProviderError> {
        self.fetch_threads_lane(&[RemoteThreadId(thread_id.to_owned())], FetchDepth::Metadata, Lane::Sync)
            .await?
            .pop()
            .ok_or(ProviderError::NotFound)
    }

    async fn raw_bytes(&self, message_id: &str) -> Result<Vec<u8>, ProviderError> {
        let call = Call::get(&["messages", message_id], Method::MessagesGet { full: true }, Lane::Interactive)
            .q("format", "raw");
        let m: Message = self.client.json(&call).await?;
        decode_b64(m.raw.as_deref().unwrap_or_default())
            .map_err(|_| ProviderError::Transient("gmail raw is not base64".into(), None))
    }

    /// `messages.attachments.get`, decoded.
    pub async fn get_attachment(&self, message_id: &str, attachment_id: &str) -> Result<Vec<u8>, ProviderError> {
        let call = Call::get(
            &["messages", message_id, "attachments", attachment_id],
            Method::AttachmentsGet,
            Lane::Interactive,
        );
        let a: api::Attachment = self.client.json(&call).await?;
        decode_b64(&a.data).map_err(|_| ProviderError::Transient("gmail attachment is not base64".into(), None))
    }

    /// Message ids with attachments after `after` (the §9.5 `hasAttachments` sweep during the
    /// window phase; `multipart/mixed` alone misses some).
    pub async fn attachment_sweep(
        &self,
        after: UnixMs,
        page: Option<String>,
    ) -> Result<(Vec<String>, Option<String>), ProviderError> {
        let call = Call::get(&["messages"], Method::MessagesList, Lane::Sync)
            .q("q", format!("has:attachment after:{}", secs_floor(after)))
            .q("maxResults", self.cfg.list_page_size.min(500).to_string())
            .q("includeSpamTrash", "true")
            .q_opt("pageToken", page);
        let r: ListMessages = self.client.json(&call).await?;
        Ok((r.messages.into_iter().map(|m| m.id).collect(), r.next_page_token))
    }

    /// `messages.batchModify` (at most 1,000 ids per call; longer lists are split).
    pub async fn batch_modify(
        &self,
        message_ids: &[String],
        add: &[String],
        remove: &[String],
    ) -> Result<(), ProviderError> {
        for chunk in message_ids.chunks(1000) {
            let call = Call::post(&["messages", "batchModify"], Method::MessagesBatchModify, Lane::Sync)
                .json(json!({"ids": chunk, "addLabelIds": add, "removeLabelIds": remove}));
            self.client.empty(&call).await?;
        }
        Ok(())
    }

    /// `threads.modify`.
    pub async fn modify_thread(&self, thread_id: &str, add: &[String], remove: &[String]) -> Result<(), ProviderError> {
        let call = Call::post(&["threads", thread_id, "modify"], Method::ThreadsModify, Lane::Sync)
            .json(json!({"addLabelIds": add, "removeLabelIds": remove}));
        self.client.empty(&call).await
    }

    /// `threads.trash`.
    pub async fn trash_thread(&self, thread_id: &str) -> Result<(), ProviderError> {
        self.client.empty(&Call::post(&["threads", thread_id, "trash"], Method::ThreadsTrash, Lane::Sync)).await
    }

    /// `threads.untrash`.
    pub async fn untrash_thread(&self, thread_id: &str) -> Result<(), ProviderError> {
        self.client.empty(&Call::post(&["threads", thread_id, "untrash"], Method::ThreadsUntrash, Lane::Sync)).await
    }

    /// `threads.delete` (permanent). Needs the full `https://mail.google.com/` scope, which
    /// Akatsuki does not request (D-039); with `gmail.modify` Gmail answers 403. Not used by `apply`.
    pub async fn delete_thread_permanently(&self, thread_id: &str) -> Result<(), ProviderError> {
        self.client
            .empty(&Call::new(reqwest::Method::DELETE, &["threads", thread_id], Method::ThreadsDelete, Lane::Sync))
            .await
    }

    // ---------------------------------------------------------------------------------------
    // Ops

    async fn post_state(&self, thread_id: &str) -> Result<OpEffect, ProviderError> {
        let post = self.fetch_one(thread_id).await?;
        Ok(OpEffect::Applied { post: Some(post), created: None })
    }

    async fn resolve(&self, targets: &[LabelTarget]) -> Result<(Vec<String>, bool), ProviderError> {
        let mut ids = Vec::new();
        let mut trash = false;
        for t in targets {
            if matches!(t, LabelTarget::Sys(SysLabel::Trash)) {
                trash = true;
            } else {
                let id = self.label_id(t).await?;
                if !ids.contains(&id) {
                    ids.push(id);
                }
            }
        }
        Ok((ids, trash))
    }

    async fn apply_modify(
        &self,
        thread: &RemoteThreadRef,
        add: &[LabelTarget],
        remove: &[LabelTarget],
    ) -> Result<OpEffect, ProviderError> {
        let (add_ids, trash) = self.resolve(add).await?;
        let (remove_ids, untrash) = self.resolve(remove).await?;
        let id = &thread.provider_thread_id;
        if !trash && !untrash && add_ids.is_empty() && remove_ids.is_empty() {
            return Ok(OpEffect::Moot);
        }
        if trash && !untrash {
            self.trash_thread(id).await?;
        } else if untrash && !trash {
            self.untrash_thread(id).await?;
        }
        if !add_ids.is_empty() || !remove_ids.is_empty() {
            self.modify_thread(id, &add_ids, &remove_ids).await?;
        }
        self.post_state(id).await
    }

    async fn apply_one(&self, op: &ProviderOp) -> Result<OpEffect, ProviderError> {
        match op {
            ProviderOp::Modify { thread, add, remove } => self.apply_modify(thread, add, remove).await,
            ProviderOp::Trash { thread } => self.apply_modify(thread, &[LabelTarget::Sys(SysLabel::Trash)], &[]).await,
            ProviderOp::Untrash { thread } => {
                self.apply_modify(thread, &[LabelTarget::Sys(SysLabel::Inbox)], &[LabelTarget::Sys(SysLabel::Trash)])
                    .await
            }
            ProviderOp::DeleteForever { .. } => Err(ProviderError::Unsupported("delete_forever")),
            ProviderOp::Move { .. } => Err(ProviderError::Unsupported("move")),
            ProviderOp::LabelCreate { name, color } => {
                let created = self.create_label(name, color.as_deref()).await?;
                Ok(OpEffect::Applied { post: None, created: Some(created) })
            }
            ProviderOp::LabelRename { provider_label_id, name } => {
                self.update_label(provider_label_id, Some(name), None).await?;
                Ok(OpEffect::Applied { post: None, created: None })
            }
            ProviderOp::LabelDelete { provider_label_id } => match self.delete_label(provider_label_id).await {
                Ok(()) => Ok(OpEffect::Applied { post: None, created: None }),
                Err(ProviderError::NotFound) => Ok(OpEffect::Moot),
                Err(e) => Err(e),
            },
            ProviderOp::DeleteDraft { draft } => match self.delete_draft_strict(draft).await {
                Ok(()) => Ok(OpEffect::Applied { post: None, created: None }),
                Err(ProviderError::NotFound) => Ok(OpEffect::Moot),
                Err(e) => Err(e),
            },
        }
    }

    /// A plain label delta (no trash change) eligible for `messages.batchModify`.
    fn batchable(op: &ProviderOp) -> Option<(&RemoteThreadRef, &[LabelTarget], &[LabelTarget])> {
        match op {
            ProviderOp::Modify { thread, add, remove }
                if !thread.messages.is_empty()
                    && !add.iter().chain(remove.iter()).any(|t| matches!(t, LabelTarget::Sys(SysLabel::Trash)))
                    && !(add.is_empty() && remove.is_empty()) =>
            {
                Some((thread, add, remove))
            }
            _ => None,
        }
    }

    async fn apply_batch(&self, group: &[(OpId, ProviderOp)]) -> Vec<(OpId, Result<OpEffect, ProviderError>)> {
        let fail = |e: ProviderError| group.iter().map(|(id, _)| (id.clone(), Err(e.clone()))).collect::<Vec<_>>();
        let Some((_, add, remove)) = Self::batchable(&group[0].1) else {
            return fail(ProviderError::Invalid("not batchable".into()));
        };
        let (add_ids, remove_ids) = match (self.resolve(add).await, self.resolve(remove).await) {
            (Ok((a, _)), Ok((r, _))) => (a, r),
            (Err(e), _) | (_, Err(e)) => return fail(e),
        };
        let mut message_ids = Vec::new();
        let mut thread_ids = Vec::new();
        for (_, op) in group {
            if let Some((t, _, _)) = Self::batchable(op) {
                message_ids.extend(t.messages.iter().map(|m| m.provider_message_id.clone()));
                thread_ids.push(RemoteThreadId(t.provider_thread_id.clone()));
            }
        }
        if let Err(e) = self.batch_modify(&message_ids, &add_ids, &remove_ids).await {
            return fail(e);
        }
        let posts = match self.fetch_threads_lane(&thread_ids, FetchDepth::Metadata, Lane::Sync).await {
            Ok(p) => p,
            Err(e) => return fail(e),
        };
        group
            .iter()
            .zip(thread_ids)
            .map(|((op_id, _), tid)| {
                let post = posts.iter().find(|p| p.provider_thread_id == tid.0).cloned();
                (
                    op_id.clone(),
                    post.map(|p| OpEffect::Applied { post: Some(p), created: None }).ok_or(ProviderError::NotFound),
                )
            })
            .collect()
    }

    // ---------------------------------------------------------------------------------------
    // Drafts and sending

    fn message_body(&self, mime: &OutgoingMime) -> serde_json::Value {
        let mut m = json!({"raw": encode_b64(&mime.bytes)});
        if let Some(t) = &mime.thread {
            m["threadId"] = json!(t);
        }
        m
    }

    fn draft_call(&self, mime: &OutgoingMime, existing: Option<&str>) -> Call {
        let (method, segs, cost) = match existing {
            Some(id) => (reqwest::Method::PUT, vec!["drafts", id], Method::DraftsUpdate),
            None => (reqwest::Method::POST, vec!["drafts"], Method::DraftsCreate),
        };
        let mut call = Call::new(method, &segs, cost, Lane::Interactive);
        if mime.bytes.len() > self.cfg.upload_threshold {
            let mut meta = json!({"message": {}});
            if let Some(id) = existing {
                meta["id"] = json!(id);
            }
            if let Some(t) = &mime.thread {
                meta["message"]["threadId"] = json!(t);
            }
            call.upload = true;
            call.body = related_body(&meta, &mime.bytes);
            call.q("uploadType", "multipart")
        } else {
            let mut body = json!({"message": self.message_body(mime)});
            if let Some(id) = existing {
                body["id"] = json!(id);
            }
            call.json(body)
        }
    }

    async fn delete_draft_strict(&self, d: &RemoteDraftRef) -> Result<(), ProviderError> {
        self.client
            .empty(&Call::new(
                reqwest::Method::DELETE,
                &["drafts", &d.provider_draft_id],
                Method::DraftsDelete,
                Lane::Sync,
            ))
            .await
    }

    /// `drafts.get format=raw`, decoded (opening a provider-native draft, `adopt`).
    pub async fn get_draft_raw(&self, draft_id: &str) -> Result<(RemoteDraftRef, Vec<u8>), ProviderError> {
        let call =
            Call::get(&["drafts", draft_id], Method::DraftsGet { full: true }, Lane::Interactive).q("format", "raw");
        let d: Draft = self.client.json(&call).await?;
        let m = d.message.unwrap_or_default();
        let raw = decode_b64(m.raw.as_deref().unwrap_or_default())
            .map_err(|_| ProviderError::Transient("gmail raw is not base64".into(), None))?;
        Ok((RemoteDraftRef { provider_draft_id: d.id, provider_message_id: Some(m.id).filter(|s| !s.is_empty()) }, raw))
    }

    /// `messages.send` with `raw` (no draft). Looks the `Message-ID` up in Sent first, so a
    /// retried send returns the earlier result instead of sending twice.
    pub async fn send_raw(&self, mime: &OutgoingMime) -> Result<SentRef, ProviderError> {
        if let Some(s) = self.find_sent(&mime.rfc_message_id).await? {
            return Ok(s);
        }
        let mut call = Call::post(&["messages", "send"], Method::MessagesSend, Lane::Sync);
        if mime.bytes.len() > self.cfg.upload_threshold {
            let mut meta = json!({});
            if let Some(t) = &mime.thread {
                meta["threadId"] = json!(t);
            }
            call.upload = true;
            call.body = related_body(&meta, &mime.bytes);
            call = call.q("uploadType", "multipart");
        } else {
            call = call.json(self.message_body(mime));
        }
        let m: Message = self.client.json(&call).await?;
        Ok(SentRef { provider_message_id: m.id, provider_thread_id: Some(m.thread_id).filter(|s| !s.is_empty()) })
    }

    // ---------------------------------------------------------------------------------------
    // Search

    /// Translates a query AST into Gmail `q`, resolving `label:` names against the cached
    /// labels. `l:<id>` keys must be rewritten to label names by the caller, which knows the
    /// Convex ids (D-203).
    pub fn translate_query(&self, ast: &akatsuki_search::query::Ast) -> Result<GmailQuery, TranslateError> {
        let labels = lock(&self.labels).labels.clone();
        let resolve = move |v: &str| -> Option<String> {
            if v.starts_with("l:") {
                return None;
            }
            labels.iter().find(|l| l.name.eq_ignore_ascii_case(v)).map(|l| l.name.clone())
        };
        akatsuki_search::gmail::to_gmail_q(ast, &resolve)
    }
}

#[async_trait]
impl MailProvider for GmailProvider {
    fn kind(&self) -> ProviderKind {
        ProviderKind::Gmail
    }

    fn caps(&self) -> &ProviderCaps {
        &self.caps
    }

    async fn list_labels(&self) -> Result<Vec<RemoteLabel>, ProviderError> {
        let r: ListLabels = self.client.json(&Call::get(&["labels"], Method::LabelsList, Lane::Sync)).await?;
        let labels: Vec<RemoteLabel> = r.labels.iter().filter(|l| !l.id.is_empty()).map(remote_label).collect();
        self.remember_labels(&labels, true);
        Ok(labels)
    }

    async fn ensure_label(&self, role: LabelRole, name: &str) -> Result<RemoteLabel, ProviderError> {
        if role == LabelRole::Archive {
            return Err(ProviderError::Unsupported("archive folder (Gmail archives by removing INBOX)"));
        }
        let all = self.list_labels().await?;
        let mut label = match all.into_iter().find(|l| l.name.eq_ignore_ascii_case(name)) {
            Some(l) => l,
            None => self.create_label(name, None).await?,
        };
        if role == LabelRole::Snoozed {
            label.role = Some(LabelRole::Snoozed);
            lock(&self.labels).snoozed_id = Some(label.provider_label_id.clone());
        }
        Ok(label)
    }

    async fn baseline(&self) -> Result<Cursor, ProviderError> {
        let p = self.profile().await?;
        let history_id =
            p.history_id.ok_or_else(|| ProviderError::Transient("profile without historyId".into(), None))?;
        Ok(Cursor::Gmail { history_id })
    }

    async fn changes_since(&self, cursor: &Cursor) -> Result<ChangePage, ProviderError> {
        let Cursor::Gmail { history_id: start } = cursor else {
            return Err(ProviderError::Invalid("not a Gmail cursor".into()));
        };
        let start = *start;
        let mut touched: Vec<RemoteThreadId> = Vec::new();
        let mut seen = HashSet::new();
        let mut deleted_messages = Vec::new();
        let mut labels_changed = false;
        let mut page: Option<String> = None;
        let mut last_record = None;
        let mut head = None;
        let mut more = false;
        for n in 0..self.cfg.history_pages_per_call.max(1) {
            let mut call = Call::get(&["history"], Method::HistoryList, Lane::Sync)
                .q("startHistoryId", start.to_string())
                .q("maxResults", self.cfg.history_page_size.clamp(1, 500).to_string());
            for t in ["messageAdded", "messageDeleted", "labelAdded", "labelRemoved"] {
                call = call.q("historyTypes", t);
            }
            let call = call.q_opt("pageToken", page.take());
            let r: ListHistory = match self.client.json(&call).await {
                Err(ProviderError::NotFound) => return Err(ProviderError::CursorExpired),
                r => r?,
            };
            let mut touch = |thread_id: &str| {
                if !thread_id.is_empty() && seen.insert(thread_id.to_owned()) {
                    touched.push(RemoteThreadId(thread_id.to_owned()));
                }
            };
            for h in &r.history {
                if h.id.is_some() {
                    last_record = h.id;
                }
                for m in &h.messages_added {
                    touch(&m.message.thread_id);
                    labels_changed |= m.message.label_ids.iter().any(|l| self.is_unknown_label(l));
                }
                for m in h.labels_added.iter().chain(&h.labels_removed) {
                    touch(&m.message.thread_id);
                    labels_changed |= m.label_ids.iter().any(|l| self.is_unknown_label(l));
                }
                for m in &h.messages_deleted {
                    touch(&m.message.thread_id);
                    if !m.message.id.is_empty() {
                        deleted_messages.push(m.message.id.clone());
                    }
                }
            }
            head = r.history_id.or(head);
            match r.next_page_token {
                Some(t) if n + 1 < self.cfg.history_pages_per_call.max(1) => page = Some(t),
                Some(_) => {
                    more = true;
                    break;
                }
                None => break,
            }
        }
        // A finished listing ends at the mailbox head; an interrupted one resumes after the last
        // record seen (re-reading one record is harmless: touched threads are re-fetched).
        let next = if more { last_record.unwrap_or(start) } else { head.unwrap_or(start) }.max(start);
        if more && next == start {
            // No progress possible through the cursor alone; report what we have and stop paging.
            more = false;
        }
        Ok(ChangePage {
            touched_threads: touched,
            deleted_threads: Vec::new(),
            deleted_messages,
            labels_changed,
            next: Cursor::Gmail { history_id: next },
            more,
        })
    }

    async fn list_window(&self, req: &WindowRequest) -> Result<WindowPage, ProviderError> {
        let size = self.cfg.list_page_size.clamp(1, 500).to_string();
        match req {
            WindowRequest::Inbox { page } => {
                let call = Call::get(&["threads"], Method::ThreadsList, Lane::Sync)
                    .q("labelIds", "INBOX")
                    .q("maxResults", size)
                    .q_opt("pageToken", page.clone());
                let r: ListThreads = self.client.json(&call).await?;
                Ok(WindowPage {
                    threads: r.threads.into_iter().map(|t| RemoteThreadId(t.id)).collect(),
                    next_page: r.next_page_token,
                    oldest: None,
                })
            }
            WindowRequest::Since { after, page } => {
                let call = Call::get(&["threads"], Method::ThreadsList, Lane::Sync)
                    .q("q", format!("after:{}", secs_floor(*after)))
                    .q("includeSpamTrash", "true")
                    .q("maxResults", size)
                    .q_opt("pageToken", page.clone());
                let r: ListThreads = self.client.json(&call).await?;
                Ok(WindowPage {
                    threads: r.threads.into_iter().map(|t| RemoteThreadId(t.id)).collect(),
                    next_page: r.next_page_token,
                    oldest: None,
                })
            }
            WindowRequest::Before { before, horizon, limit } => {
                // A date cursor, never a page token (§9.3). `before` rounds up so the message at
                // the cursor is listed again rather than skipped; the upsert is idempotent.
                let call = Call::get(&["messages"], Method::MessagesList, Lane::Sync)
                    .q("q", format!("before:{} after:{}", secs_ceil(*before), secs_floor(*horizon)))
                    .q("includeSpamTrash", "true")
                    .q("maxResults", (*limit).clamp(1, 500).to_string());
                let r: ListMessages = self.client.json(&call).await?;
                let mut seen = HashSet::new();
                let threads: Vec<RemoteThreadId> = r
                    .messages
                    .iter()
                    .filter(|m| seen.insert(m.thread_id.clone()))
                    .map(|m| RemoteThreadId(m.thread_id.clone()))
                    .collect();
                let oldest = match (r.next_page_token.as_ref(), r.messages.last()) {
                    // Exhausted: the range down to the horizon is covered.
                    (None, _) | (_, None) => Some(*horizon),
                    (Some(_), Some(last)) => {
                        let call = Call::get(&["messages", &last.id], Method::MessagesGet { full: false }, Lane::Sync)
                            .q("format", "minimal");
                        let m: Message = self.client.json(&call).await?;
                        Some(m.internal_date.unwrap_or(*horizon).max(*horizon))
                    }
                };
                Ok(WindowPage { threads, next_page: None, oldest })
            }
        }
    }

    async fn fetch_threads(
        &self,
        ids: &[RemoteThreadId],
        depth: FetchDepth,
    ) -> Result<Vec<FetchedThread>, ProviderError> {
        self.fetch_threads_lane(ids, depth, Lane::Sync).await
    }

    async fn fetch_raw(&self, msg: &RemoteMessageRef) -> Result<ByteStream, ProviderError> {
        Ok(one_chunk(self.raw_bytes(&msg.provider_message_id).await?))
    }

    async fn fetch_part(&self, msg: &RemoteMessageRef, part_id: &str) -> Result<ByteStream, ProviderError> {
        let id = &msg.provider_message_id;
        let call =
            Call::get(&["messages", id], Method::MessagesGet { full: true }, Lane::Interactive).q("format", "full");
        let m: Message = self.client.json(&call).await?;
        if let Some(part) = m.payload.as_ref().and_then(|p| part_by_section(p, part_id)) {
            let body = part.body.clone().unwrap_or_default();
            if let Some(data) = body.data.as_deref().filter(|d| !d.is_empty()) {
                let bytes =
                    decode_b64(data).map_err(|_| ProviderError::Transient("gmail part is not base64".into(), None))?;
                return Ok(one_chunk(bytes));
            }
            if let Some(att) = body.attachment_id.as_deref() {
                return Ok(one_chunk(self.get_attachment(id, att).await?));
            }
            if body.size == Some(0) {
                return Ok(one_chunk(Vec::new()));
            }
        }
        // The structure did not name the part (message/rfc822 children, odd trees): find it in
        // the raw source with the same parser that assigned the part ids (D-095).
        let raw = self.raw_bytes(id).await?;
        match akatsuki_mime::part_content(&raw, part_id) {
            Ok(Some(p)) => Ok(one_chunk(p.bytes)),
            Ok(None) => Err(ProviderError::NotFound),
            Err(_) => Err(ProviderError::Invalid("message could not be parsed".into())),
        }
    }

    async fn apply(&self, ops: &[(OpId, ProviderOp)]) -> Vec<(OpId, Result<OpEffect, ProviderError>)> {
        let mut out = Vec::with_capacity(ops.len());
        let mut i = 0;
        while i < ops.len() {
            let mut j = i + 1;
            if let Some((_, add, remove)) = Self::batchable(&ops[i].1) {
                while j < ops.len() && Self::batchable(&ops[j].1).is_some_and(|(_, a, r)| a == add && r == remove) {
                    j += 1;
                }
            }
            if j - i >= self.cfg.batch_modify_min_threads.max(2) {
                out.extend(self.apply_batch(&ops[i..j]).await);
            } else {
                for (id, op) in &ops[i..j] {
                    out.push((id.clone(), self.apply_one(op).await));
                }
            }
            i = j;
        }
        out
    }

    async fn save_draft(
        &self,
        mime: &OutgoingMime,
        existing: Option<&RemoteDraftRef>,
    ) -> Result<RemoteDraftRef, ProviderError> {
        let saved = match existing {
            Some(d) => match self.client.json::<Draft>(&self.draft_call(mime, Some(&d.provider_draft_id))).await {
                // Deleted elsewhere: autosave keeps the text by creating a new draft.
                Err(ProviderError::NotFound) => self.client.json::<Draft>(&self.draft_call(mime, None)).await?,
                r => r?,
            },
            None => self.client.json::<Draft>(&self.draft_call(mime, None)).await?,
        };
        Ok(RemoteDraftRef {
            provider_draft_id: saved.id,
            provider_message_id: saved.message.map(|m| m.id).filter(|s| !s.is_empty()),
        })
    }

    async fn delete_draft(&self, d: &RemoteDraftRef) -> Result<(), ProviderError> {
        match self.delete_draft_strict(d).await {
            Err(ProviderError::NotFound) => Ok(()),
            r => r,
        }
    }

    async fn send_draft(&self, d: &RemoteDraftRef, rfc_message_id: &str) -> Result<SentRef, ProviderError> {
        if let Some(s) = self.find_sent(rfc_message_id).await? {
            return Ok(s);
        }
        let call =
            Call::post(&["drafts", "send"], Method::DraftsSend, Lane::Sync).json(json!({"id": d.provider_draft_id}));
        let m: Message = self.client.json(&call).await?;
        Ok(SentRef { provider_message_id: m.id, provider_thread_id: Some(m.thread_id).filter(|s| !s.is_empty()) })
    }

    async fn find_sent(&self, rfc_message_id: &str) -> Result<Option<SentRef>, ProviderError> {
        let id = clean_message_id(rfc_message_id)?;
        // `in:sent` keeps the draft (same Message-ID, DRAFT label) from matching.
        let call = Call::get(&["messages"], Method::MessagesList, Lane::Sync)
            .q("q", format!("rfc822msgid:{id} in:sent"))
            .q("includeSpamTrash", "true")
            .q("maxResults", "5");
        let r: ListMessages = self.client.json(&call).await?;
        Ok(r.messages.into_iter().next().map(|m| SentRef {
            provider_message_id: m.id,
            provider_thread_id: Some(m.thread_id).filter(|s| !s.is_empty()),
        }))
    }

    async fn search(
        &self,
        q: &akatsuki_search::query::Ast,
        page: Option<String>,
    ) -> Result<(Vec<RemoteThreadId>, Option<String>), ProviderError> {
        if lock(&self.labels).known.is_none() {
            self.list_labels().await?;
        }
        let translated = self.translate_query(q).map_err(|e| match e {
            TranslateError::Unsupported(_) => ProviderError::Unsupported("search term without a Gmail equivalent"),
            TranslateError::UnknownLabel(l) => ProviderError::Invalid(format!("unknown label {l}")),
        })?;
        let call = Call::get(&["threads"], Method::ThreadsList, Lane::Interactive)
            .q("maxResults", self.cfg.list_page_size.clamp(1, 500).to_string())
            .q_opt("q", Some(translated.q).filter(|s| !s.is_empty()))
            .q_opt("pageToken", page);
        let r: ListThreads = self.client.json(&call).await?;
        Ok((r.threads.into_iter().map(|t| RemoteThreadId(t.id)).collect(), r.next_page_token))
    }

    async fn watch(
        &self,
        _current: Option<&WatchState>,
        _hook_base: &str,
    ) -> Result<Option<WatchState>, ProviderError> {
        // The push endpoint is the Pub/Sub subscription's, configured in Google Cloud; Gmail
        // only needs the topic. `hook_base` is for Graph.
        let Some(topic) = &self.cfg.pubsub_topic else { return Ok(None) };
        let call = Call::post(&["watch"], Method::Watch, Lane::Sync).json(json!({
            "topicName": topic,
            "labelIds": WATCH_LABEL_IDS,
            "labelFilterBehavior": "include",
        }));
        match self.client.json::<api::WatchResponse>(&call).await {
            Ok(w) => Ok(Some(WatchState {
                kind: WatchKind::GmailWatch,
                external_id: None,
                expires_at: w.expiration.unwrap_or_else(|| (self.clock)() + WATCH_TTL_MS),
                client_state: None,
            })),
            // The topic is missing, or Gmail's service account may not publish to it (billing
            // not linked, permissions not granted): fall back to polling.
            Err(ProviderError::Invalid(_) | ProviderError::NotFound | ProviderError::Conflict(_)) => {
                tracing::warn!(target: "akatsuki::gmail", "gmail watch unavailable, polling instead");
                Ok(None)
            }
            Err(e) => Err(e),
        }
    }

    async fn unwatch(&self, _w: &WatchState) -> Result<(), ProviderError> {
        match self.client.empty(&Call::post(&["stop"], Method::Stop, Lane::Sync)).await {
            Err(ProviderError::NotFound) => Ok(()),
            r => r,
        }
    }

    async fn revoke(&self) -> Result<(), ProviderError> {
        let token = match self.client.tokens.access_token().await {
            // The grant is already gone (revoked by the user, invalid_grant): nothing to revoke.
            Err(ProviderError::Auth { .. }) => return Ok(()),
            r => r?,
        };
        auth::revoke_token(&self.client.http, &self.cfg.revoke_url, &token.token).await
    }
}
