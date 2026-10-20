//! Arguments and results of the client-facing Convex functions that Rust (the desktop app) calls
//! (ARCHITECTURE §4.2). The web and mobile clients use the generated TypeScript API instead.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_with::skip_serializing_none;

use super::rows::{AiConsent, AiProfile, AiRoute, LabelView, MessageMeta, ThreadMirror};
use super::schema::{DeviceKind, Rule, ThreadAction, TimerCondition, TombstoneTable};
use super::worker::TriageResult;
use crate::ids::{AccountId, OpId, SplitId, ThreadId, TimerId};
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
