//! Arguments and results of the client-facing Convex functions that Rust (the desktop app) calls
//! (ARCHITECTURE §4.2). The web and mobile clients use the generated TypeScript API instead.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_with::skip_serializing_none;

use super::rows::{AiConsent, AiProfile, AiRoute, LabelView, MessageMeta, ThreadMirror, ThreadRow};
use super::schema::{
    Density, DeviceKind, Entitlement, Keymap, NotificationPref, Onboarding, Prefs, ReadReceiptPrefs, RemoteImages,
    Rule, SendState, ThemePref, ThreadAction, TimerCondition, TombstoneTable,
};
use super::worker::TriageResult;
use crate::ids::{AccountId, LabelId, OpId, SnippetId, SplitId, ThreadId, TimerId};
use crate::time::UnixMs;

/// Fully qualified names of the client functions Rust calls.
pub mod functions {
    /// `me.get`
    pub const ME_GET: &str = "me:get";
    /// `devices.register`
    pub const DEVICES_REGISTER: &str = "devices:register";
    /// `accounts.list`
    pub const ACCOUNTS_LIST: &str = "accounts:list";
    /// `labels.list`
    pub const LABELS_LIST: &str = "labels:list";
    /// `mail.apply`
    pub const MAIL_APPLY: &str = "mail:apply";
    /// `mail.undo`
    pub const MAIL_UNDO: &str = "mail:undo";
    /// `ops.failures`
    pub const OPS_FAILURES: &str = "ops:failures";
    /// `ops.ackFailure`
    pub const OPS_ACK_FAILURE: &str = "ops:ackFailure";
    /// `ai.getSettings`
    pub const AI_GET_SETTINGS: &str = "ai:getSettings";
    /// `ai.claimTriage`
    pub const AI_CLAIM_TRIAGE: &str = "ai:claimTriage";
    /// `ai.setTriage`
    pub const AI_SET_TRIAGE: &str = "ai:setTriage";
    /// `sync.changes` (desktop mirror feed)
    pub const SYNC_CHANGES: &str = "sync:changes";
    /// `splits.list` (desktop split tabs, D-313)
    pub const SPLITS_LIST: &str = "splits:list";
    /// `mail.snooze`
    pub const MAIL_SNOOZE: &str = "mail:snooze";
    /// `sends.cancel` (undo send, §10.3)
    pub const SENDS_CANCEL: &str = "sends:cancel";
    /// `ai.saveSettings`
    pub const AI_SAVE_SETTINGS: &str = "ai:saveSettings";
    /// `mail.setReminder`
    pub const MAIL_SET_REMINDER: &str = "mail:setReminder";
    /// `me.updatePrefs` (desktop settings sync)
    pub const ME_UPDATE_PREFS: &str = "me:updatePrefs";
    /// `labels.create`
    pub const LABELS_CREATE: &str = "labels:create";
    /// `labels.rename`
    pub const LABELS_RENAME: &str = "labels:rename";
    /// `labels.remove`
    pub const LABELS_REMOVE: &str = "labels:remove";
    /// `snippets.list`
    pub const SNIPPETS_LIST: &str = "snippets:list";
    /// `snippets.save`
    pub const SNIPPETS_SAVE: &str = "snippets:save";
    /// `snippets.remove`
    pub const SNIPPETS_REMOVE: &str = "snippets:remove";
    /// `snippets.markUsed`
    pub const SNIPPETS_MARK_USED: &str = "snippets:markUsed";
    /// `ops.retry` (failure toasts, §5.7)
    pub const OPS_RETRY: &str = "ops:retry";
    /// `search.threads` (Convex search, §11.2 backend 1)
    pub const SEARCH_THREADS: &str = "search:threads";
    /// `sends.list` (scheduled messages)
    pub const SENDS_LIST: &str = "sends:list";
    /// `sends.reschedule`
    pub const SENDS_RESCHEDULE: &str = "sends:reschedule";
    /// `sends.resolveUnknown`
    pub const SENDS_RESOLVE_UNKNOWN: &str = "sends:resolveUnknown";
    /// `drafts.discard`
    pub const DRAFTS_DISCARD: &str = "drafts:discard";
}

/// `devices.register` arguments.
#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RegisterDeviceArgs {
    /// UUIDv4 minted on first run.
    pub device_id: String,
    /// Kind.
    pub kind: DeviceKind,
    /// User-editable name.
    pub name: String,
    /// App version.
    pub app_version: String,
    /// Expo push token (mobile only).
    pub expo_push_token: Option<String>,
}

/// `mail.apply` arguments.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MailApplyArgs {
    /// One per user gesture (UUIDv7).
    pub client_op_id: String,
    /// 1..500 threads.
    pub thread_ids: Vec<ThreadId>,
    /// Action.
    pub action: ThreadAction,
}

/// `mail.apply` result.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MailApplyResult {
    /// Ops created (or replayed).
    pub op_ids: Vec<OpId>,
    /// Account id → revision after the write (desktop overlay `min_rev`).
    pub rev: BTreeMap<String, u64>,
}

/// `mail.undo` arguments.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MailUndoArgs {
    /// New gesture id.
    pub client_op_id: String,
    /// Gesture to undo.
    pub undo_client_op_id: String,
}

/// How an undo was performed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UndoMode {
    /// Queued ops were cancelled.
    Cancelled,
    /// Inverse ops were enqueued.
    Inverse,
    /// Nothing to undo.
    Noop,
}

/// `mail.undo` result.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MailUndoResult {
    /// Mode.
    pub mode: UndoMode,
    /// Ops created.
    pub op_ids: Vec<OpId>,
}

/// `ops.ackFailure` arguments.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AckFailureArgs {
    /// Ops to acknowledge.
    pub op_ids: Vec<OpId>,
}

/// `ai.claimTriage` arguments.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClaimTriageArgs {
    /// This device.
    pub device_id: String,
    /// At most 25.
    pub limit: u32,
}

/// `ai.setTriage` arguments.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SetTriageArgs {
    /// Results (≤ 50).
    pub results: Vec<TriageResult>,
}

/// `sync.changes` arguments.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncChangesArgs {
    /// Account.
    pub account_id: AccountId,
    /// Last revision the mirror holds.
    pub since_rev: u64,
    /// At most 500.
    pub limit: u32,
}

/// A deletion in the mirror feed.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TombstoneRef {
    /// Table.
    pub table: TombstoneTable,
    /// Document id.
    pub doc_id: String,
}

/// `sync.changes` result.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncChangesResult {
    /// Changed threads.
    pub threads: Vec<ThreadMirror>,
    /// Changed messages.
    pub messages: Vec<MessageMeta>,
    /// Changed labels.
    pub labels: Vec<LabelView>,
    /// Deletions.
    pub tombstones: Vec<TombstoneRef>,
    /// Revision covered by this batch.
    pub up_to_rev: u64,
    /// More changes are waiting.
    pub has_more: bool,
    /// Tombstones were garbage-collected past `sinceRev`: re-pull everything.
    pub reset_required: bool,
}

/// `SplitView`: `splits` minus `userId`, plus `labelKey: "split:<id>"` (§4.4).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SplitView {
    /// Split id.
    #[serde(rename = "_id")]
    pub id: SplitId,
    /// Creation time.
    #[serde(rename = "_creationTime")]
    pub creation_time: f64,
    /// Name shown on the tab.
    pub name: String,
    /// Tab order.
    pub order: f64,
    /// Matching rule.
    pub rule: Rule,
    /// Show an unread count on the tab.
    pub show_count: bool,
    /// `split:<id>`.
    pub label_key: String,
}

/// `mail.snooze` arguments.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MailSnoozeArgs {
    /// One per user gesture.
    pub client_op_id: String,
    /// Threads (≤ 500).
    pub thread_ids: Vec<ThreadId>,
    /// Wake time.
    pub until: UnixMs,
    /// Mark unread when it comes back.
    pub return_unread: bool,
}

/// `mail.snooze` result.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MailSnoozeResult {
    /// Timers created.
    pub timer_ids: Vec<TimerId>,
}

/// `sends.cancel` arguments.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SendsCancelArgs {
    /// The send to cancel.
    pub client_send_id: String,
}

/// How a cancel ended (§10.3).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CancelOutcome {
    /// The send will not happen; the draft stays in Drafts.
    Cancelled,
    /// Already due, sending or sent.
    TooLate,
}

/// `sends.cancel` result.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SendsCancelResult {
    /// Outcome.
    pub result: CancelOutcome,
}

/// `ai.saveSettings` arguments. Absent fields keep their stored value; the desktop never sends
/// `voiceProfile`.
#[skip_serializing_none]
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SaveAiSettingsArgs {
    /// Provider profiles (≤ 10). Never keys.
    pub profiles: Option<Vec<AiProfile>>,
    /// Routes.
    pub routes: Option<Vec<AiRoute>>,
    /// Consent.
    pub consent: Option<AiConsent>,
}

/// `mail.setReminder` arguments.
#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MailSetReminderArgs {
    /// Thread.
    pub thread_id: ThreadId,
    /// When (in the future).
    pub at: UnixMs,
    /// `always` or `if_no_reply`.
    pub condition: TimerCondition,
    /// The user's own note (≤ 500 chars).
    pub note: Option<String>,
}

/// `mail.setReminder` result.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MailSetReminderResult {
    /// The timer.
    pub timer_id: TimerId,
}

/// The `profiles` row as `me.get` returns it (`vProfile`, minus `userId`).
#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProfileView {
    /// Address.
    pub email: String,
    /// Display name.
    pub name: Option<String>,
    /// IANA time zone; user-facing dates resolve in it (§1.3).
    pub timezone: String,
    /// Onboarding step.
    pub onboarding: Onboarding,
    /// Synced preferences.
    pub prefs: Prefs,
}

/// `me.get` result (`null` until the user's rows exist). Billing is not decoded.
#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MeView {
    /// Better Auth user id.
    pub user_id: String,
    /// Address.
    pub email: String,
    /// Display name.
    pub name: Option<String>,
    /// Profile and prefs.
    pub profile: ProfileView,
    /// What the user may use.
    pub entitlement: Entitlement,
}

/// A partial `vPrefs` for `me.updatePrefs`: absent fields keep their stored value.
#[skip_serializing_none]
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PrefsPatch {
    /// 0, 5, 10, 20 or 30.
    pub undo_send_seconds: Option<u32>,
    /// Remote images.
    pub remote_images: Option<RemoteImages>,
    /// Read receipts.
    pub read_receipts: Option<ReadReceiptPrefs>,
    /// Keep bodies on devices.
    pub cache_bodies_on_device: Option<bool>,
    /// Density.
    pub density: Option<Density>,
    /// Theme.
    pub theme: Option<ThemePref>,
    /// Key map.
    pub keymap: Option<Keymap>,
    /// Unified inbox.
    pub unified_inbox: Option<bool>,
    /// Notifications.
    pub notifications: Option<NotificationPref>,
}

impl PrefsPatch {
    /// Nothing to change.
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }
}

/// `me.updatePrefs` arguments.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdatePrefsArgs {
    /// Fields to change.
    pub patch: PrefsPatch,
}

/// `labels.create` arguments.
#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LabelCreateArgs {
    /// Gesture id (UUIDv7).
    pub client_op_id: String,
    /// Mailbox.
    pub account_id: AccountId,
    /// Name.
    pub name: String,
    /// `#RRGGBB`.
    pub color: Option<String>,
}

/// `labels.create` result.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LabelCreateResult {
    /// The new label.
    pub label_id: LabelId,
    /// `l:<labelId>`.
    pub label_key: String,
    /// The `label_create` op.
    pub op_id: OpId,
}

/// `labels.rename` arguments.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LabelRenameArgs {
    /// Gesture id.
    pub client_op_id: String,
    /// Label.
    pub label_id: LabelId,
    /// New name.
    pub name: String,
}

/// `labels.remove` arguments.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LabelRemoveArgs {
    /// Gesture id.
    pub client_op_id: String,
    /// Label.
    pub label_id: LabelId,
}

/// `labels.rename` / `labels.remove` result.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LabelOpResult {
    /// The op.
    pub op_id: OpId,
}

/// `SnippetView`: `snippets` minus `userId`, `searchText`.
#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SnippetView {
    /// Snippet id.
    #[serde(rename = "_id")]
    pub id: SnippetId,
    /// Creation time.
    #[serde(rename = "_creationTime")]
    pub creation_time: f64,
    /// Name.
    pub name: String,
    /// `;shortcut` trigger.
    pub shortcut: Option<String>,
    /// Subject filled into an empty draft.
    pub subject: Option<String>,
    /// Sanitized body (≤ 20 KB).
    pub body_html: String,
    /// Variables used (`first_name`, …).
    pub variables: Vec<String>,
    /// Times inserted.
    pub use_count: f64,
    /// Last insert.
    pub last_used_at: Option<f64>,
    /// Updated.
    pub updated_at: f64,
}

/// `snippets.save` arguments (`snippetId` absent creates one).
#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SnippetSaveArgs {
    /// Existing snippet.
    pub snippet_id: Option<SnippetId>,
    /// Name (1–100 chars).
    pub name: String,
    /// Trigger (`[a-z0-9][a-z0-9_-]{0,31}`).
    pub shortcut: Option<String>,
    /// Subject.
    pub subject: Option<String>,
    /// Body HTML (sanitized by Convex).
    pub body_html: String,
    /// Variables used.
    pub variables: Vec<String>,
}

/// `snippets.save` result.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SnippetSaveResult {
    /// The snippet.
    pub snippet_id: SnippetId,
}

/// `snippets.remove` / `snippets.markUsed` arguments.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SnippetIdArgs {
    /// The snippet.
    pub snippet_id: SnippetId,
}

/// `ops.retry` arguments.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OpRetryArgs {
    /// The failed op.
    pub op_id: OpId,
    /// New gesture id.
    pub client_op_id: String,
}

/// `ops.retry` result.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OpRetryResult {
    /// Ops enqueued.
    pub op_ids: Vec<OpId>,
}

/// Convex `paginationOpts`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PaginationOpts {
    /// Page size (50 on desktop, §1.5).
    pub num_items: u32,
    /// `null` for the first page.
    pub cursor: Option<String>,
}

/// `search.threads` arguments.
#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchThreadsArgs {
    /// Query in the grammar (≤ 500 chars).
    pub q: String,
    /// Rounded down to the minute by the caller (query cache).
    pub now_at: i64,
    /// One mailbox.
    pub account_id: Option<AccountId>,
    /// Page.
    pub pagination_opts: PaginationOpts,
}

/// `search.threads` result (the parsed `ast` is not decoded).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchThreadsResult {
    /// Matching rows.
    pub page: Vec<ThreadRow>,
    /// Last page.
    pub is_done: bool,
    /// Next cursor.
    pub continue_cursor: String,
    /// Grammar Convex could not apply; the UI offers the provider search.
    pub unsupported: Vec<String>,
}

/// `sends.list` arguments.
#[skip_serializing_none]
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SendsListArgs {
    /// States (default: scheduled, due, sending, failed, unknown).
    pub states: Option<Vec<SendState>>,
}

/// `sends.reschedule` arguments.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SendsRescheduleArgs {
    /// The send.
    pub client_send_id: String,
    /// New time (future).
    pub send_at: i64,
}

/// How an `unknown` send is resolved.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UnknownResolution {
    /// The user saw it arrive.
    WasSent,
    /// Drop it.
    Discard,
}

/// `sends.resolveUnknown` arguments.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SendsResolveArgs {
    /// The send.
    pub client_send_id: String,
    /// Resolution.
    pub resolution: UnknownResolution,
}

/// `drafts.discard` arguments.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DraftDiscardArgs {
    /// Gesture id.
    pub client_op_id: String,
    /// The draft.
    pub client_draft_id: String,
}

/// `drafts.discard` result.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DraftDiscardResult {
    /// The `delete_draft` op.
    pub op_id: OpId,
}
