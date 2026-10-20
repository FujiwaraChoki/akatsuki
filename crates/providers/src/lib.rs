//! # akatsuki-providers
//!
//! The provider abstraction of ARCHITECTURE §9.1: one [`MailProvider`] trait that the Gmail,
//! Microsoft Graph and IMAP/SMTP adapters implement, the [`TokenSource`] that refreshes OAuth
//! tokens, the [`PushSource`] for IMAP IDLE, and the [`MetaSink`] through which metadata reaches
//! Convex (`ConvexSink` in the worker). Adapters never talk to Convex directly.
//!
//! The adapters are [`gmail`] (Gmail API, D-200..D-208), [`graph`] (Microsoft Graph,
//! D-210..D-219) and [`imap`] (IMAP + SMTP); the worker builds them in
//! `apps/sync/src/providers.rs`. All three are tested against mocks only (wiremock, the in-process
//! fake IMAP/SMTP servers in `imap::fake`); live mailboxes need credentials (`docs/STATUS.md`).
//! Next to them are the provider-independent pieces every adapter needs: label mapping tables
//! ([`labels`]), IMAP thread ids ([`imap_thread`]), the `FetchedThread` → `ThreadUpsert`
//! conversion ([`convert`]) and the error disposition table of §5.5
//! ([`ProviderError::disposition`]).
//!
//! Feature `fake-servers` exposes `imap::fake` outside tests, for the local end-to-end harness
//! (`akatsuki-fake-mail`, D-382).

pub mod convert;
pub mod gmail;
pub mod graph;
pub mod imap;
pub mod imap_thread;
pub mod labels;

use std::collections::BTreeMap;
use std::time::Duration;

use async_trait::async_trait;
use bytes::Bytes;
use futures::stream::BoxStream;
use serde::{Deserialize, Serialize};

pub use akatsuki_core::convex::{AccountStatus, CursorUpdate, SyncPhase, SyncProgress, UpsertThreadsResult, WatchKind};
pub use akatsuki_core::model::{
    Addr, AuthSummary, ImapLocation, LabelKind, LabelRole, ListUnsubscribe, RemoteLabel, RemoteThreadId, SysLabel,
};
pub use akatsuki_core::time::UnixMs;
use akatsuki_core::wire::ErrorCode;

/// A stream of raw bytes (message source or attachment part).
pub type ByteStream = BoxStream<'static, Result<Bytes, ProviderError>>;

/// Which provider an adapter talks to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderKind {
    /// Gmail API.
    Gmail,
    /// Microsoft Graph.
    Graph,
    /// IMAP + SMTP.
    Imap,
}

/// What a provider can do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProviderCaps {
    /// Gmail `threadId`, Graph `conversationId`; IMAP computes ids ([`imap_thread`]).
    pub native_threads: bool,
    /// Gmail true; Graph/IMAP use exclusive folders.
    pub multi_label: bool,
    /// Graph categories, Gmail `CATEGORY_*`.
    pub categories: bool,
    /// Provider-side search.
    pub server_search: bool,
    /// False for Gmail (`gmail.modify` scope).
    pub delete_forever: bool,
    /// How changes are pushed.
    pub push: PushKind,
    /// Gmail 1000, Graph 20, IMAP 1000 UIDs.
    pub max_batch_modify: usize,
    /// The provider stores sent mail itself.
    pub sent_auto_saved: bool,
    /// IMAP extensions, for IMAP accounts.
    pub imap: Option<ImapCaps>,
}

/// IMAP extensions that change how sync works.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ImapCaps {
    /// RFC 7162 CONDSTORE.
    pub condstore: bool,
    /// RFC 7162 QRESYNC.
    pub qresync: bool,
    /// RFC 6851 MOVE.
    pub uid_move: bool,
    /// RFC 4315 UIDPLUS.
    pub uidplus: bool,
    /// RFC 2177 IDLE.
    pub idle: bool,
    /// RFC 5465 NOTIFY.
    pub notify: bool,
    /// RFC 6154 SPECIAL-USE.
    pub special_use: bool,
    /// Gmail's X-GM-EXT-1.
    pub x_gm_ext: bool,
}

impl ImapCaps {
    /// Reads the extensions from a `CAPABILITY` list (case-insensitive).
    pub fn from_capabilities<I: IntoIterator<Item = S>, S: AsRef<str>>(caps: I) -> Self {
        let mut c = ImapCaps::default();
        for cap in caps {
            match cap.as_ref().to_ascii_uppercase().as_str() {
                "CONDSTORE" => c.condstore = true,
                "QRESYNC" => {
                    c.qresync = true;
                    c.condstore = true;
                }
                "MOVE" => c.uid_move = true,
                "UIDPLUS" => c.uidplus = true,
                "IDLE" => c.idle = true,
                "NOTIFY" => c.notify = true,
                "SPECIAL-USE" => c.special_use = true,
                "X-GM-EXT-1" => c.x_gm_ext = true,
                _ => {}
            }
        }
        c
    }
}

/// How a provider pushes changes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PushKind {
    /// Gmail `users.watch` through Pub/Sub.
    GmailPubSub,
    /// Graph change notifications.
    GraphWebhook,
    /// IMAP IDLE.
    ImapIdle,
    /// Polling.
    Poll {
        /// Interval.
        every: Duration,
    },
}

impl ProviderCaps {
    /// Gmail with Pub/Sub push.
    pub fn gmail() -> Self {
        Self {
            native_threads: true,
            multi_label: true,
            categories: true,
            server_search: true,
            delete_forever: false,
            push: PushKind::GmailPubSub,
            max_batch_modify: 1000,
            sent_auto_saved: true,
            imap: None,
        }
    }

    /// Microsoft Graph with webhooks.
    pub fn graph() -> Self {
        Self {
            native_threads: true,
            multi_label: false,
            categories: true,
            server_search: true,
            delete_forever: true,
            push: PushKind::GraphWebhook,
            max_batch_modify: 20,
            sent_auto_saved: true,
            imap: None,
        }
    }

    /// IMAP with the given extensions; `sent_auto_saved` is detected per host.
    pub fn imap(caps: ImapCaps, sent_auto_saved: bool) -> Self {
        Self {
            native_threads: caps.x_gm_ext,
            multi_label: false,
            categories: false,
            server_search: true,
            delete_forever: true,
            push: if caps.idle { PushKind::ImapIdle } else { PushKind::Poll { every: Duration::from_secs(120) } },
            max_batch_modify: 1000,
            sent_auto_saved,
            imap: Some(caps),
        }
    }

    /// The `mailAccounts.caps` subset stored in Convex.
    pub fn account_caps(&self) -> akatsuki_core::convex::AccountCaps {
        akatsuki_core::convex::AccountCaps {
            multi_label: self.multi_label,
            categories: self.categories,
            server_search: self.server_search,
            delete_forever: self.delete_forever,
        }
    }
}

/// Sync cursors, committed in the same Convex mutation as the data they cover.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Cursor {
    /// Gmail history id.
    Gmail {
        /// History id (u64; stored as a decimal string in Convex).
        history_id: u64,
    },
    /// Graph delta links per folder.
    Graph {
        /// Folder id → cursor.
        folders: BTreeMap<String, GraphFolderCursor>,
    },
    /// IMAP state per mailbox.
    Imap {
        /// Mailbox → cursor.
        folders: BTreeMap<String, ImapFolderCursor>,
    },
}

/// One Graph folder's delta link.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GraphFolderCursor {
    /// `@odata.deltaLink`.
    pub delta_link: Option<String>,
}

/// One IMAP mailbox's sync state.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImapFolderCursor {
    /// `UIDVALIDITY`.
    pub uid_validity: u32,
    /// `UIDNEXT`.
    pub uid_next: u32,
    /// `HIGHESTMODSEQ` with CONDSTORE.
    pub highest_modseq: Option<u64>,
}

impl Cursor {
    /// The Convex `CursorUpdate`s that persist this cursor. Folder cursors need the folder's
    /// label key, looked up by `label_key_for`.
    pub fn to_updates(&self, label_key_for: impl Fn(&str) -> Option<String>) -> Vec<CursorUpdate> {
        match self {
            Cursor::Gmail { history_id } => vec![CursorUpdate::Gmail { history_id: history_id.to_string() }],
            Cursor::Graph { folders } => folders
                .iter()
                .filter_map(|(id, c)| {
                    Some(CursorUpdate::Folder {
                        remote_folder_id: id.clone(),
                        label_key: label_key_for(id)?,
                        uid_validity: None,
                        uid_next: None,
                        highest_mod_seq: None,
                        delta_link: c.delta_link.clone(),
                    })
                })
                .collect(),
            Cursor::Imap { folders } => folders
                .iter()
                .filter_map(|(id, c)| {
                    Some(CursorUpdate::Folder {
                        remote_folder_id: id.clone(),
                        label_key: label_key_for(id)?,
                        uid_validity: Some(c.uid_validity),
                        uid_next: Some(c.uid_next),
                        highest_mod_seq: c.highest_modseq.map(|m| m.to_string()),
                        delta_link: None,
                    })
                })
                .collect(),
        }
    }
}

/// A message reference with IMAP locations.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RemoteMessageRef {
    /// Provider message id.
    pub provider_message_id: String,
    /// IMAP locations (empty for Gmail/Graph).
    pub imap: Vec<ImapLocation>,
}

/// A thread reference with its messages.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RemoteThreadRef {
    /// Provider thread id.
    pub provider_thread_id: String,
    /// Messages.
    pub messages: Vec<RemoteMessageRef>,
}

/// A provider draft.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RemoteDraftRef {
    /// Provider draft id.
    pub provider_draft_id: String,
    /// Provider message id, if known.
    pub provider_message_id: Option<String>,
}

/// A header-level message as fetched. Mirrors Convex `MessageUpsert` except labels are provider
/// ids, mapped to label keys by [`convert`].
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RemoteMessage {
    /// Provider message id.
    pub provider_message_id: String,
    /// Provider thread id.
    pub provider_thread_id: String,
    /// `Message-ID` without brackets.
    pub rfc_message_id: Option<String>,
    /// `In-Reply-To`.
    pub in_reply_to: Option<String>,
    /// `References`.
    pub references: Vec<String>,
    /// Sender.
    pub from: Option<Addr>,
    /// To.
    pub to: Vec<Addr>,
    /// Cc.
    pub cc: Vec<Addr>,
    /// Bcc.
    pub bcc: Vec<Addr>,
    /// Reply-To.
    pub reply_to: Vec<Addr>,
    /// Subject.
    pub subject: String,
    /// Snippet.
    pub snippet: String,
    /// Date.
    pub date: UnixMs,
    /// Provider label/folder/category/keyword ids.
    pub provider_label_ids: Vec<String>,
    /// System flags the adapter already resolved (unread, starred, drafts…).
    pub sys: Vec<SysLabel>,
    /// Draft.
    pub is_draft: bool,
    /// Attachments (best effort).
    pub has_attachments: bool,
    /// Size.
    pub size_bytes: Option<u64>,
    /// `List-Id`.
    pub list_id: Option<String>,
    /// Unsubscribe options.
    pub list_unsubscribe: Option<ListUnsubscribe>,
    /// Authentication summary.
    pub auth: Option<AuthSummary>,
    /// IMAP locations.
    pub imap: Vec<ImapLocation>,
}

/// A thread as fetched.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FetchedThread {
    /// Provider thread id.
    pub provider_thread_id: String,
    /// Gmail thread historyId; Graph max lastModified ms.
    pub provider_version: Option<i64>,
    /// Messages.
    pub messages: Vec<RemoteMessage>,
    /// `messages` may be only part of the thread (IMAP: the messages seen in this process, D-223),
    /// so the engine upserts it with `complete: false` and never deletes unlisted messages.
    /// `false` (the default) means the list is the whole thread.
    pub partial: bool,
}

/// One page of changes since a cursor.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChangePage {
    /// Re-fetch with `fetch_threads(Metadata)`.
    pub touched_threads: Vec<RemoteThreadId>,
    /// Threads deleted at the provider.
    pub deleted_threads: Vec<RemoteThreadId>,
    /// Messages deleted at the provider.
    pub deleted_messages: Vec<String>,
    /// Labels changed; re-list them.
    pub labels_changed: bool,
    /// Cursor to commit with the data.
    pub next: Cursor,
    /// More pages are waiting.
    pub more: bool,
}

/// What to list during initial sync and backfill.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WindowRequest {
    /// Inbox threads, newest first.
    Inbox {
        /// Provider page token.
        page: Option<String>,
    },
    /// Everything after an instant.
    Since {
        /// Lower bound.
        after: UnixMs,
        /// Provider page token.
        page: Option<String>,
    },
    /// Backfill page by date cursor.
    Before {
        /// Upper bound.
        before: UnixMs,
        /// Lower bound.
        horizon: UnixMs,
        /// Page size.
        limit: u32,
    },
}

/// One page of thread ids.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WindowPage {
    /// Threads.
    pub threads: Vec<RemoteThreadId>,
    /// Next provider page token.
    pub next_page: Option<String>,
    /// Oldest message date seen.
    pub oldest: Option<UnixMs>,
}

/// How much of a thread to fetch.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FetchDepth {
    /// Headers and snippets.
    Metadata,
    /// Including bodies (search hits, resync).
    Full,
}

/// A label or folder the op targets.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LabelTarget {
    /// A system label.
    Sys(SysLabel),
    /// A provider label or folder.
    Label {
        /// Provider id.
        provider_label_id: String,
        /// Folder semantics.
        exclusive: bool,
    },
}

/// A provider-level op, after Convex refs were resolved by the engine. `unsubscribe` is not a
/// provider op: the engine performs it (§9.1).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProviderOp {
    /// Add/remove labels.
    Modify {
        /// Thread.
        thread: RemoteThreadRef,
        /// Labels to add.
        add: Vec<LabelTarget>,
        /// Labels to remove.
        remove: Vec<LabelTarget>,
    },
    /// Trash.
    Trash {
        /// Thread.
        thread: RemoteThreadRef,
    },
    /// Untrash.
    Untrash {
        /// Thread.
        thread: RemoteThreadRef,
    },
    /// Delete permanently.
    DeleteForever {
        /// Thread.
        thread: RemoteThreadRef,
    },
    /// Move to a folder.
    Move {
        /// Thread.
        thread: RemoteThreadRef,
        /// Destination.
        to: LabelTarget,
    },
    /// Create a label.
    LabelCreate {
        /// Name.
        name: String,
        /// Colour.
        color: Option<String>,
    },
    /// Rename a label.
    LabelRename {
        /// Provider id.
        provider_label_id: String,
        /// New name.
        name: String,
    },
    /// Delete a label.
    LabelDelete {
        /// Provider id.
        provider_label_id: String,
    },
    /// Delete a draft.
    DeleteDraft {
        /// Draft.
        draft: RemoteDraftRef,
    },
}

/// Convex `Id<"ops">`.
pub type OpId = String;

/// The result of one applied op.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OpEffect {
    /// Applied; `post` is the thread re-read after the change, `created` a new label.
    Applied {
        /// Post-op thread state.
        post: Option<FetchedThread>,
        /// Created label (`label_create`).
        created: Option<RemoteLabel>,
    },
    /// Nothing to do.
    Moot,
}

/// A built outgoing message (`crates/mime::compose`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OutgoingMime {
    /// Fixed `Message-ID` (without brackets).
    pub rfc_message_id: String,
    /// RFC 5322 bytes.
    pub bytes: Bytes,
    /// Provider thread id for replies.
    pub thread: Option<String>,
}

/// A sent message.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SentRef {
    /// Provider message id.
    pub provider_message_id: String,
    /// Provider thread id.
    pub provider_thread_id: Option<String>,
}

/// Which budget a rate limit applies to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LimitScope {
    /// The account actor pauses.
    Account,
    /// Every Gmail actor pauses (shared project bucket).
    Project,
}

/// Provider errors, classified for the op state machine (§5.5).
#[derive(thiserror::Error, Debug, Clone, PartialEq, Eq)]
pub enum ProviderError {
    /// 5xx, timeout, reset, IMAP `NO [UNAVAILABLE]`.
    #[error("transient: {0}")]
    Transient(String, Option<Duration>),
    /// 429 / `rateLimitExceeded` / `[LIMIT]`.
    #[error("rate limited")]
    RateLimited {
        /// `Retry-After`.
        retry_after: Option<Duration>,
        /// Budget.
        scope: LimitScope,
    },
    /// 401 after refresh, `invalid_grant`, `AUTHENTICATIONFAILED`.
    #[error("auth")]
    Auth {
        /// The account needs reconnecting.
        reauth_required: bool,
    },
    /// 404, UID gone.
    #[error("not found")]
    NotFound,
    /// Gmail 404 `startHistoryId`, Graph 410, `UIDVALIDITY` change. Handled by the engine.
    #[error("cursor expired")]
    CursorExpired,
    /// 400.
    #[error("invalid: {0}")]
    Invalid(String),
    /// Name taken, folder missing.
    #[error("conflict: {0}")]
    Conflict(String),
    /// Mailbox full, `dailyLimitExceeded`.
    #[error("quota: {0}")]
    Quota(String),
    /// Not supported by this provider.
    #[error("unsupported: {0}")]
    Unsupported(&'static str),
}

/// What the engine does with an op after a provider error (§5.5).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OpDisposition {
    /// Requeue after `retry_after_ms`; `count_attempt` false for auth and rate limits.
    Requeue {
        /// Delay.
        retry_after_ms: i64,
        /// Whether this counts against `maxAttempts`.
        count_attempt: bool,
        /// Pause the whole account actor (rate limit) or every Gmail actor (project scope).
        pause: Option<LimitScope>,
        /// Mark the account `reauth_required`.
        reauth: bool,
    },
    /// Done with outcome `moot` (the target no longer exists).
    Moot,
    /// Failed permanently.
    Fail,
}

impl ProviderError {
    /// The registry code.
    pub fn code(&self) -> ErrorCode {
        match self {
            Self::Transient(..) => ErrorCode::ProviderTransient,
            Self::RateLimited { .. } => ErrorCode::ProviderRateLimited,
            Self::Auth { .. } => ErrorCode::ReauthRequired,
            Self::NotFound => ErrorCode::NotFound,
            Self::CursorExpired => ErrorCode::ProviderTransient, // handled by the engine, never surfaced
            Self::Invalid(_) => ErrorCode::ProviderInvalid,
            Self::Conflict(_) => ErrorCode::ProviderConflict,
            Self::Quota(_) => ErrorCode::ProviderQuota,
            Self::Unsupported(_) => ErrorCode::Unsupported,
        }
    }

    /// Whether a retry may succeed.
    pub fn retryable(&self) -> bool {
        matches!(self, Self::Transient(..) | Self::RateLimited { .. })
    }

    /// The §5.5 table. `attempt` is the attempt that just failed, `max_attempts` the op's limit,
    /// `jitter` a sample from `[0, 1)` for the backoff.
    pub fn disposition(&self, attempt: u32, max_attempts: u32, jitter: f64) -> OpDisposition {
        let backoff = akatsuki_core::sync::backoff_ms(attempt, jitter);
        let ms = |d: &Option<Duration>| d.map(|d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX));
        match self {
            Self::Transient(_, retry) => {
                if attempt >= max_attempts {
                    OpDisposition::Fail
                } else {
                    OpDisposition::Requeue {
                        retry_after_ms: ms(retry).unwrap_or(0).max(backoff),
                        count_attempt: true,
                        pause: None,
                        reauth: false,
                    }
                }
            }
            Self::CursorExpired => {
                OpDisposition::Requeue { retry_after_ms: backoff, count_attempt: true, pause: None, reauth: false }
            }
            Self::RateLimited { retry_after, scope } => OpDisposition::Requeue {
                retry_after_ms: ms(retry_after).unwrap_or(0).max(backoff),
                count_attempt: false,
                pause: Some(*scope),
                reauth: false,
            },
            Self::Auth { .. } => {
                OpDisposition::Requeue { retry_after_ms: backoff, count_attempt: false, pause: None, reauth: true }
            }
            Self::NotFound => OpDisposition::Moot,
            Self::Invalid(_) | Self::Conflict(_) | Self::Quota(_) | Self::Unsupported(_) => OpDisposition::Fail,
        }
    }
}

/// A short-lived access token. `Debug` never prints the token.
#[derive(Clone, PartialEq, Eq)]
pub struct AccessToken {
    /// The bearer token.
    pub token: String,
    /// Expiry.
    pub expires_at: UnixMs,
}

impl std::fmt::Debug for AccessToken {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AccessToken").field("expires_at", &self.expires_at).finish_non_exhaustive()
    }
}

/// A provider watch (Gmail watch or Graph subscription).
#[derive(Clone, PartialEq, Eq)]
pub struct WatchState {
    /// Kind.
    pub kind: WatchKind,
    /// Graph subscription id.
    pub external_id: Option<String>,
    /// Expiry.
    pub expires_at: UnixMs,
    /// Graph `clientState` (secret; only its sha256 is stored in Convex).
    pub client_state: Option<String>,
}

impl std::fmt::Debug for WatchState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WatchState")
            .field("kind", &self.kind)
            .field("external_id", &self.external_id)
            .field("expires_at", &self.expires_at)
            .finish_non_exhaustive()
    }
}

/// OAuth access tokens for one account.
#[async_trait]
pub trait TokenSource: Send + Sync {
    /// Single-flight refresh at exp − 5 min; rotated refresh tokens are persisted through
    /// `worker.putSecret` (CAS).
    async fn access_token(&self) -> Result<AccessToken, ProviderError>;
    /// Drops the cached token (after a 401).
    async fn invalidate(&self);
}

/// One mail provider account (§9.1).
#[async_trait]
pub trait MailProvider: Send + Sync {
    /// Kind.
    fn kind(&self) -> ProviderKind;
    /// Capabilities.
    fn caps(&self) -> &ProviderCaps;
    /// All labels/folders.
    async fn list_labels(&self) -> Result<Vec<RemoteLabel>, ProviderError>;
    /// Creates `Akatsuki/Snoozed` or the archive folder if missing.
    async fn ensure_label(&self, role: LabelRole, name: &str) -> Result<RemoteLabel, ProviderError>;
    /// Head cursor. Called before the first window page so changes during initial sync are kept.
    async fn baseline(&self) -> Result<Cursor, ProviderError>;
    /// Changes since a cursor.
    async fn changes_since(&self, cursor: &Cursor) -> Result<ChangePage, ProviderError>;
    /// Lists thread ids for initial sync and backfill.
    async fn list_window(&self, req: &WindowRequest) -> Result<WindowPage, ProviderError>;
    /// Fetches threads.
    async fn fetch_threads(
        &self,
        ids: &[RemoteThreadId],
        depth: FetchDepth,
    ) -> Result<Vec<FetchedThread>, ProviderError>;
    /// Streams a message's RFC 822 source.
    async fn fetch_raw(&self, msg: &RemoteMessageRef) -> Result<ByteStream, ProviderError>;
    /// Streams one MIME part.
    async fn fetch_part(&self, msg: &RemoteMessageRef, part_id: &str) -> Result<ByteStream, ProviderError>;
    /// Applies ops; one result per op, in order.
    async fn apply(&self, ops: &[(OpId, ProviderOp)]) -> Vec<(OpId, Result<OpEffect, ProviderError>)>;
    /// Creates or replaces a provider draft.
    async fn save_draft(
        &self,
        mime: &OutgoingMime,
        existing: Option<&RemoteDraftRef>,
    ) -> Result<RemoteDraftRef, ProviderError>;
    /// Deletes a draft.
    async fn delete_draft(&self, d: &RemoteDraftRef) -> Result<(), ProviderError>;
    /// Gmail `drafts.send`; Graph `POST /messages/{id}/send`; IMAP: SMTP DATA + APPEND Sent
    /// (unless `sent_auto_saved`) + delete draft.
    async fn send_draft(&self, d: &RemoteDraftRef, rfc_message_id: &str) -> Result<SentRef, ProviderError>;
    /// Looks a sent message up by `Message-ID` (send verification, §5.6).
    async fn find_sent(&self, rfc_message_id: &str) -> Result<Option<SentRef>, ProviderError>;
    /// Provider-side search.
    async fn search(
        &self,
        q: &akatsuki_search::query::Ast,
        page: Option<String>,
    ) -> Result<(Vec<RemoteThreadId>, Option<String>), ProviderError>;
    /// Creates or renews the Gmail watch / Graph subscription. IMAP and poll return `None`.
    async fn watch(&self, current: Option<&WatchState>, hook_base: &str) -> Result<Option<WatchState>, ProviderError>;
    /// Stops a watch.
    async fn unwatch(&self, w: &WatchState) -> Result<(), ProviderError>;
    /// Revokes the grant (account removal).
    async fn revoke(&self) -> Result<(), ProviderError>;
}

/// IMAP IDLE (or NOTIFY) stream owned by the account actor.
#[async_trait]
pub trait PushSource: Send {
    /// Waits for the next event.
    async fn next(&mut self) -> Result<PushEvent, ProviderError>;
}

/// An IDLE event.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PushEvent {
    /// A mailbox changed.
    FolderChanged {
        /// Mailbox.
        remote_folder_id: String,
    },
    /// The connection was re-established; run a delta.
    Reconnected,
}

/// Provider label id → label key (`worker.upsertLabels` result).
pub type LabelMap = BTreeMap<String, String>;

/// `worker.upsertThreads` result.
pub type UpsertAck = UpsertThreadsResult;

/// Status fields an actor writes (`worker.setAccountStatus` minus ids).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct AccountStatusUpdate {
    /// Status.
    pub status: Option<AccountStatus>,
    /// `Some(None)` clears the detail.
    pub status_detail: Option<Option<ErrorCode>>,
    /// Phase.
    pub sync_phase: Option<SyncPhase>,
    /// Progress.
    pub progress: Option<SyncProgress>,
    /// Oldest synced message.
    pub oldest_synced_at: Option<UnixMs>,
    /// Last successful sync.
    pub last_sync_at: Option<UnixMs>,
}

/// Errors from a [`MetaSink`].
#[derive(thiserror::Error, Debug, Clone, PartialEq, Eq)]
pub enum SinkError {
    /// Another worker owns the account; stop at once.
    #[error("lease lost")]
    LeaseLost,
    /// The owner is no longer entitled.
    #[error("not entitled")]
    NotEntitled,
    /// Convex rejected the arguments (a bug).
    #[error("invalid: {0}")]
    Invalid(String),
    /// Network or Convex outage; retry with backoff.
    #[error("unavailable: {0}")]
    Unavailable(String),
}

/// Where metadata goes: `ConvexSink` (worker). A `SqliteSink` (desktop standalone) may come later.
#[async_trait]
pub trait MetaSink: Send + Sync {
    /// `worker.upsertLabels`.
    async fn upsert_labels(&self, labels: Vec<RemoteLabel>, complete: bool) -> Result<LabelMap, SinkError>;
    /// `worker.upsertThreads`.
    async fn upsert_threads(
        &self,
        threads: Vec<FetchedThread>,
        deletes: Vec<String>,
        cursor: Option<CursorUpdate>,
    ) -> Result<UpsertAck, SinkError>;
    /// `worker.setAccountStatus`.
    async fn set_status(&self, s: AccountStatusUpdate) -> Result<(), SinkError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disposition_table() {
        let t = ProviderError::Transient("503".into(), None);
        assert!(matches!(
            t.disposition(1, 10, 1.0),
            OpDisposition::Requeue { retry_after_ms: 2000, count_attempt: true, .. }
        ));
        assert_eq!(t.disposition(10, 10, 0.5), OpDisposition::Fail);

        let rl = ProviderError::RateLimited { retry_after: Some(Duration::from_secs(30)), scope: LimitScope::Account };
        assert_eq!(
            rl.disposition(1, 10, 1.0),
            OpDisposition::Requeue {
                retry_after_ms: 30_000,
                count_attempt: false,
                pause: Some(LimitScope::Account),
                reauth: false
            }
        );
        assert!(matches!(
            ProviderError::Auth { reauth_required: true }.disposition(1, 10, 0.0),
            OpDisposition::Requeue { reauth: true, count_attempt: false, .. }
        ));
        assert_eq!(ProviderError::NotFound.disposition(1, 10, 0.0), OpDisposition::Moot);
        assert_eq!(ProviderError::Quota("full".into()).disposition(1, 10, 0.0), OpDisposition::Fail);
        assert_eq!(ProviderError::Unsupported("delete_forever").code(), ErrorCode::Unsupported);
        assert!(!ProviderError::NotFound.retryable());
    }

    #[test]
    fn imap_caps_from_capability_list() {
        let c = ImapCaps::from_capabilities(["IMAP4rev1", "idle", "QRESYNC", "MOVE", "X-GM-EXT-1"]);
        assert!(c.idle && c.qresync && c.condstore && c.uid_move && c.x_gm_ext && !c.notify);
        let caps = ProviderCaps::imap(c, true);
        assert!(caps.native_threads);
        assert_eq!(caps.push, PushKind::ImapIdle);
        assert!(!ProviderCaps::gmail().account_caps().delete_forever);
    }

    #[test]
    fn cursor_to_updates() {
        let mut folders = BTreeMap::new();
        folders.insert(
            "INBOX".to_owned(),
            ImapFolderCursor { uid_validity: 3, uid_next: 10, highest_modseq: Some(u64::MAX) },
        );
        folders.insert("Unknown".to_owned(), ImapFolderCursor { uid_validity: 1, uid_next: 1, highest_modseq: None });
        let ups = Cursor::Imap { folders }.to_updates(|f| (f == "INBOX").then(|| "sys:inbox".to_owned()));
        assert_eq!(ups.len(), 1);
        assert!(
            matches!(&ups[0], CursorUpdate::Folder { highest_mod_seq: Some(m), .. } if m == "18446744073709551615")
        );
        assert_eq!(
            Cursor::Gmail { history_id: 42 }.to_updates(|_| None),
            vec![CursorUpdate::Gmail { history_id: "42".into() }]
        );
    }

    #[test]
    fn secrets_are_not_debug_printed() {
        let t = AccessToken { token: "ya29.secret".into(), expires_at: 1 };
        assert!(!format!("{t:?}").contains("secret"));
        let w = WatchState {
            kind: WatchKind::GraphSubscription,
            external_id: None,
            expires_at: 0,
            client_state: Some("cs-secret".into()),
        };
        assert!(!format!("{w:?}").contains("cs-secret"));
    }
}
