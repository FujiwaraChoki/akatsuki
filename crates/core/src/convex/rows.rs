//! Row projections returned by Convex functions (`convex/lib/rows.ts`, ARCHITECTURE §4.4) and the
//! AI settings document (§3.2 `aiSettings`, §12.2).
//!
//! Each view includes `_id` and `_creationTime` unless the contract says otherwise.

use serde::{Deserialize, Serialize};
use serde_with::skip_serializing_none;

use super::schema::{
    AccountCaps, AccountStatus, AiFeature, AiProviderKind, AppError, OpState, Provider, SendKind, SendState, SyncPhase,
    SyncProgress, TimerCondition, TimerKind, TimerState, TlsMode, Triage, TriageCategory, TriageState, WatchKind,
};
use super::values::ConvexInt64;
use crate::ids::{
    AccountId, DraftId, FolderCursorId, LabelId, MessageId, OpId, ScheduledSendId, SyncStateId, ThreadId, TimerId,
};
use crate::model::{Addr, LabelKind, LabelRole, ListUnsubscribe};
use crate::time::UnixMs;

/// Triage as shown on a list row (`vThreadRow.triage`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TriageBrief {
    /// Category.
    pub category: TriageCategory,
    /// Needs a reply.
    pub needs_reply: bool,
}

/// `vThreadRow`: one row of a thread list (about 700 bytes). Has `_id` only.
#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadRow {
    /// Thread id.
    #[serde(rename = "_id")]
    pub id: ThreadId,
    /// Account.
    pub account_id: AccountId,
    /// Subject (≤ 998 chars).
    pub subject: String,
    /// Snippet (≤ 200 chars).
    pub snippet: String,
    /// Up to 10 participants, most recent first.
    pub participants: Vec<Addr>,
    /// Latest sender.
    pub last_from: Option<Addr>,
    /// Latest message time.
    pub last_message_at: UnixMs,
    /// List sort key.
    pub sort_at: UnixMs,
    /// Messages in the thread.
    pub message_count: u32,
    /// Unread messages.
    pub unread_count: u32,
    /// Drafts in the thread.
    pub draft_count: u32,
    /// Any attachment (best effort).
    pub has_attachments: bool,
    /// Bulk mail.
    pub is_bulk: bool,
    /// Visible label keys.
    pub label_keys: Vec<String>,
    /// Split key for inbox threads.
    pub split: Option<String>,
    /// Pending ops on the thread.
    pub pending_op_count: u32,
    /// Snoozed until.
    pub snoozed_until: Option<UnixMs>,
    /// Returned from snooze/reminder at.
    pub returned_at: Option<UnixMs>,
    /// Reminder at.
    pub reminder_at: Option<UnixMs>,
    /// Triage summary.
    pub triage: Option<TriageBrief>,
}

/// `vMessageMeta`: the `messages` document minus `userId`, `rev`, `imapLocations`.
#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MessageMeta {
    /// Message id.
    #[serde(rename = "_id")]
    pub id: MessageId,
    /// Creation time.
    #[serde(rename = "_creationTime")]
    pub creation_time: f64,
    /// Account.
    pub account_id: AccountId,
    /// Thread.
    pub thread_id: ThreadId,
    /// Provider message id.
    pub provider_message_id: String,
    /// RFC 5322 `Message-ID` without angle brackets.
    pub rfc_message_id: Option<String>,
    /// `In-Reply-To`.
    pub in_reply_to: Option<String>,
    /// Last 20 `References`.
    pub references: Vec<String>,
    /// Sender.
    pub from: Option<Addr>,
    /// To (≤ 100).
    pub to: Vec<Addr>,
    /// Cc (≤ 100).
    pub cc: Vec<Addr>,
    /// Bcc (≤ 100).
    pub bcc: Vec<Addr>,
    /// Reply-To.
    pub reply_to: Vec<Addr>,
    /// Subject.
    pub subject: String,
    /// Snippet.
    pub snippet: String,
    /// Date.
    pub date: UnixMs,
    /// Provider label keys (`sys:*` and `l:*`).
    pub label_keys: Vec<String>,
    /// Unread.
    pub is_unread: bool,
    /// Draft.
    pub is_draft: bool,
    /// Sent from one of the account's identities.
    pub is_outbound: bool,
    /// Has attachments (best effort).
    pub has_attachments: bool,
    /// Size in bytes.
    pub size_bytes: Option<u64>,
    /// `List-Id`.
    pub list_id: Option<String>,
    /// Authentication summary.
    pub auth_summary: Option<crate::model::AuthSummary>,
}

/// `vOpSummary`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OpSummary {
    /// Op id.
    #[serde(rename = "_id")]
    pub id: OpId,
    /// Gesture id.
    pub client_op_id: String,
    /// State.
    pub state: OpState,
    /// Payload kind.
    pub payload_kind: String,
    /// Created at.
    pub created_at: UnixMs,
}

/// Terminal failure states shown to users.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FailureState {
    /// Failed.
    Failed,
    /// Unknown (sends).
    Unknown,
}

/// `vOpFailure`.
#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OpFailure {
    /// Op id.
    #[serde(rename = "_id")]
    pub id: OpId,
    /// Thread, for thread ops.
    pub thread_id: Option<ThreadId>,
    /// Thread subject.
    pub subject: Option<String>,
    /// Account address.
    pub account_email: String,
    /// Payload kind.
    pub payload_kind: String,
    /// Error.
    pub error: AppError,
    /// State.
    pub state: FailureState,
    /// When it failed.
    pub at: UnixMs,
}

/// `vTriageInput`: headers and snippet only, never body text.
#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TriageInput {
    /// Thread.
    pub thread_id: ThreadId,
    /// Sender.
    pub from: Option<Addr>,
    /// Recipients.
    pub to: Vec<Addr>,
    /// Subject.
    pub subject: String,
    /// Snippet.
    pub snippet: String,
    /// `List-Id`.
    pub list_id: Option<String>,
    /// Bulk mail.
    pub is_bulk: bool,
    /// Has attachments.
    pub has_attachments: bool,
}

/// `LabelView`: `labels` minus `userId`, `rev`, plus `labelKey`.
#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LabelView {
    /// Label id.
    #[serde(rename = "_id")]
    pub id: LabelId,
    /// Creation time.
    #[serde(rename = "_creationTime")]
    pub creation_time: f64,
    /// Account.
    pub account_id: AccountId,
    /// Provider id (absent until `label_create` completes).
    pub provider_label_id: Option<String>,
    /// Name.
    pub name: String,
    /// Kind.
    pub kind: LabelKind,
    /// Role.
    pub role: Option<LabelRole>,
    /// Folder semantics.
    pub exclusive: bool,
    /// Colour.
    pub color: Option<String>,
    /// Hidden in the UI.
    pub hidden: bool,
    /// A `label_delete` op is pending.
    pub pending_delete: bool,
    /// `sys:<role>` or `l:<_id>`.
    pub label_key: String,
}

/// `mailAccounts.imap` minus `capabilities` (`MailAccountView`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImapSettingsView {
    /// IMAP host.
    pub host: String,
    /// IMAP port.
    pub port: u16,
    /// IMAP TLS.
    pub tls: TlsMode,
    /// SMTP host.
    pub smtp_host: String,
    /// SMTP port.
    pub smtp_port: u16,
    /// SMTP TLS.
    pub smtp_tls: TlsMode,
    /// Server saves sent mail.
    pub sent_auto_saved: bool,
}

/// `MailAccountView`: `mailAccounts` minus `userId`, `shard`, `imap.capabilities`.
#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MailAccountView {
    /// Account id.
    #[serde(rename = "_id")]
    pub id: AccountId,
    /// Creation time.
    #[serde(rename = "_creationTime")]
    pub creation_time: f64,
    /// Provider.
    pub provider: Provider,
    /// Provider account id.
    pub provider_account_id: String,
    /// Address.
    pub email: String,
    /// Display name.
    pub display_name: Option<String>,
    /// Colour.
    pub color: Option<String>,
    /// Sort order.
    pub sort_order: f64,
    /// Status.
    pub status: AccountStatus,
    /// Error code, never provider text.
    pub status_detail: Option<String>,
    /// Sync phase.
    pub sync_phase: SyncPhase,
    /// Progress.
    pub progress: SyncProgress,
    /// Connect time − 90 days.
    pub window_start_at: UnixMs,
    /// Backfill horizon (0 = everything).
    pub backfill_horizon_at: UnixMs,
    /// Oldest synced message.
    pub oldest_synced_at: Option<UnixMs>,
    /// Capabilities.
    pub caps: AccountCaps,
    /// IMAP settings.
    pub imap: Option<ImapSettingsView>,
    /// Created at.
    pub created_at: UnixMs,
    /// Last successful sync.
    pub last_sync_at: Option<UnixMs>,
}

/// `ThreadMirror`: `threads` minus `userId`, `triageLeaseUntil` (desktop feed).
#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadMirror {
    /// Thread id.
    #[serde(rename = "_id")]
    pub id: ThreadId,
    /// Creation time.
    #[serde(rename = "_creationTime")]
    pub creation_time: f64,
    /// Account.
    pub account_id: AccountId,
    /// Provider thread id.
    pub provider_thread_id: String,
    /// Subject.
    pub subject: String,
    /// Snippet.
    pub snippet: String,
    /// Participants.
    pub participants: Vec<Addr>,
    /// Latest sender.
    pub last_from: Option<Addr>,
    /// Bulk.
    pub is_bulk: bool,
    /// `List-Id`.
    pub list_id: Option<String>,
    /// Unsubscribe options.
    pub list_unsubscribe: Option<ListUnsubscribe>,
    /// Latest message.
    pub last_message_at: UnixMs,
    /// Latest inbound.
    pub last_inbound_at: Option<UnixMs>,
    /// Latest outbound.
    pub last_outbound_at: Option<UnixMs>,
    /// Sort key.
    pub sort_at: UnixMs,
    /// Messages.
    pub message_count: u32,
    /// Unread.
    pub unread_count: u32,
    /// Drafts.
    pub draft_count: u32,
    /// Attachments.
    pub has_attachments: bool,
    /// Provider-confirmed keys.
    pub synced_label_keys: Vec<String>,
    /// Visible keys.
    pub label_keys: Vec<String>,
    /// Split.
    pub split: Option<String>,
    /// Pending ops.
    pub pending_op_count: u32,
    /// Hidden by a pending delete.
    pub deleted_pending: bool,
    /// Provider version (int64).
    pub provider_version: Option<ConvexInt64>,
    /// Snoozed until.
    pub snoozed_until: Option<UnixMs>,
    /// Returned at.
    pub returned_at: Option<UnixMs>,
    /// Reminder at.
    pub reminder_at: Option<UnixMs>,
    /// Triage.
    pub triage: Option<Triage>,
    /// Triage state.
    pub triage_state: TriageState,
    /// Per-account revision.
    pub rev: u64,
    /// Updated at.
    pub updated_at: UnixMs,
}

/// `FolderCursor`: `folderCursors` minus `_creationTime`.
#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FolderCursor {
    /// Row id.
    #[serde(rename = "_id")]
    pub id: FolderCursorId,
    /// Account.
    pub account_id: AccountId,
    /// Graph folder id or IMAP mailbox name.
    pub remote_folder_id: String,
    /// The label key of the folder.
    pub label_key: String,
    /// IMAP `UIDVALIDITY`.
    pub uid_validity: Option<u32>,
    /// IMAP `UIDNEXT`.
    pub uid_next: Option<u32>,
    /// IMAP `HIGHESTMODSEQ` (decimal u64 string).
    pub highest_mod_seq: Option<String>,
    /// Graph delta link.
    pub delta_link: Option<String>,
    /// Last synced.
    pub last_synced_at: UnixMs,
}

/// The `syncState` document (worker-only).
#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncStateDoc {
    /// Row id.
    #[serde(rename = "_id")]
    pub id: SyncStateId,
    /// Creation time.
    #[serde(rename = "_creationTime")]
    pub creation_time: f64,
    /// Account.
    pub account_id: AccountId,
    /// Shard.
    pub shard: u32,
    /// Gmail history id (decimal u64 string).
    pub gmail_history_id: Option<String>,
    /// Lease owner (workerId).
    pub lease_owner: Option<String>,
    /// Lease expiry.
    pub lease_expires_at: Option<UnixMs>,
    /// Watch kind.
    pub watch_kind: Option<WatchKind>,
    /// Graph subscription id.
    pub watch_external_id: Option<String>,
    /// Watch expiry.
    pub watch_expires_at: Option<UnixMs>,
    /// Hex sha256 of the Graph clientState.
    pub watch_client_state_hash: Option<String>,
    /// Backfill date cursor.
    pub backfill_before_at: Option<UnixMs>,
    /// Consecutive failures.
    pub consecutive_failures: u32,
    /// Last error.
    pub last_error: Option<AppError>,
    /// Updated at.
    pub updated_at: UnixMs,
}

/// `TimerView`: `timers` minus `userId`, `scheduledFnId`.
#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TimerView {
    /// Timer id.
    #[serde(rename = "_id")]
    pub id: TimerId,
    /// Creation time.
    #[serde(rename = "_creationTime")]
    pub creation_time: f64,
    /// Account.
    pub account_id: AccountId,
    /// Thread.
    pub thread_id: ThreadId,
    /// Kind.
    pub kind: TimerKind,
    /// Fire time.
    pub fire_at: UnixMs,
    /// Condition.
    pub condition: TimerCondition,
    /// Set time.
    pub anchor_at: UnixMs,
    /// Return unread.
    pub return_unread: bool,
    /// User note (≤ 500 chars).
    pub note: Option<String>,
    /// State.
    pub state: TimerState,
    /// Created at.
    pub created_at: UnixMs,
}

/// Preview of a scheduled send.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SendPreview {
    /// Subject.
    pub subject: String,
    /// Recipients.
    pub to: Vec<Addr>,
}

/// `ScheduledSendView`: `scheduledSends` minus `userId`, `scheduledFnId`.
#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScheduledSendView {
    /// Row id.
    #[serde(rename = "_id")]
    pub id: ScheduledSendId,
    /// Creation time.
    #[serde(rename = "_creationTime")]
    pub creation_time: f64,
    /// Account (absent on a cancel tombstone).
    pub account_id: Option<AccountId>,
    /// Client idempotency key.
    pub client_send_id: String,
    /// Draft (absent on a cancel tombstone).
    pub draft_id: Option<DraftId>,
    /// Kind.
    pub kind: SendKind,
    /// When to send.
    pub send_at: UnixMs,
    /// State.
    pub state: SendState,
    /// The send op.
    pub op_id: Option<OpId>,
    /// Preview.
    pub preview: SendPreview,
    /// Sent provider message id.
    pub sent_provider_message_id: Option<String>,
    /// Error.
    pub error: Option<AppError>,
    /// Created at.
    pub created_at: UnixMs,
}

/// Per-tier model overrides of an AI profile.
#[skip_serializing_none]
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AiModels {
    /// Fast tier.
    pub fast: Option<String>,
    /// Default tier.
    pub default: Option<String>,
    /// Heavy tier.
    pub heavy: Option<String>,
    /// Transcription tier.
    pub transcription: Option<String>,
}

/// One AI provider profile (`aiSettings.profiles[]`). Never holds a key.
#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AiProfile {
    /// UUIDv4.
    pub profile_id: String,
    /// User label.
    pub label: String,
    /// Provider kind.
    pub kind: AiProviderKind,
    /// `https://…` or `http://localhost`/LAN; no userinfo, no query.
    pub base_url: Option<String>,
    /// Per-tier models.
    pub models: AiModels,
    /// Device ids whose keychain holds a key for this profile.
    pub key_on_devices: Vec<String>,
}

/// The feature a route applies to: a feature or `"default"`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(untagged)]
pub enum AiRouteFeature {
    /// A specific feature.
    Feature(AiFeature),
    /// The fallback route.
    Default(DefaultRoute),
}

/// The literal `"default"`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DefaultRoute {
    /// `"default"`
    Default,
}

impl AiRouteFeature {
    /// The `"default"` route.
    pub const DEFAULT: Self = AiRouteFeature::Default(DefaultRoute::Default);
}

/// `aiSettings.routes[]`.
#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AiRoute {
    /// Feature or `"default"`.
    pub feature: AiRouteFeature,
    /// Profile to use.
    pub profile_id: String,
    /// Model override.
    pub model: Option<String>,
    /// Enabled.
    pub enabled: bool,
}

/// `aiSettings.consent`.
#[skip_serializing_none]
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AiConsent {
    /// When consent was granted. No AI runs without it.
    pub granted_at: Option<UnixMs>,
    /// Consent text version shown.
    pub version: Option<u32>,
    /// Auto-triage toggle.
    pub auto_triage: bool,
    /// Sync the voice profile to Convex.
    pub sync_voice_profile: bool,
}

/// `AiSettingsView`: `aiSettings` minus `userId`. The same model as
/// `packages/ai/schema/ai-settings.schema.json` (system fields are optional so settings files
/// without them parse too).
#[skip_serializing_none]
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AiSettings {
    /// Row id.
    #[serde(rename = "_id")]
    pub id: Option<String>,
    /// Creation time.
    #[serde(rename = "_creationTime")]
    pub creation_time: Option<f64>,
    /// Provider profiles (≤ 10).
    pub profiles: Vec<AiProfile>,
    /// Routes.
    pub routes: Vec<AiRoute>,
    /// Consent.
    pub consent: AiConsent,
    /// Voice profile (≤ 2048 chars), only with `consent.syncVoiceProfile`.
    pub voice_profile: Option<String>,
    /// Updated at.
    #[serde(default)]
    pub updated_at: UnixMs,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn thread_row_round_trip() {
        let wire = json!({
            "_id": "t1", "accountId": "a1", "subject": "Hi", "snippet": "…", "participants": [{"email": "a@b.c"}],
            "lastMessageAt": 1, "sortAt": 1, "messageCount": 1, "unreadCount": 0, "draftCount": 0,
            "hasAttachments": false, "isBulk": false, "labelKeys": ["sys:inbox"], "pendingOpCount": 0,
            "triage": {"category": "personal", "needsReply": true}
        });
        let row: ThreadRow = serde_json::from_value(wire.clone()).unwrap();
        assert_eq!(serde_json::to_value(&row).unwrap(), wire);
    }

    #[test]
    fn route_feature_accepts_default_and_features() {
        let r: AiRoute =
            serde_json::from_value(json!({"feature": "default", "profileId": "p", "enabled": true})).unwrap();
        assert_eq!(r.feature, AiRouteFeature::DEFAULT);
        let r: AiRoute =
            serde_json::from_value(json!({"feature": "nl_search", "profileId": "p", "enabled": true})).unwrap();
        assert_eq!(r.feature, AiRouteFeature::Feature(AiFeature::NlSearch));
        assert_eq!(serde_json::to_value(AiRouteFeature::DEFAULT).unwrap(), json!("default"));
    }

    #[test]
    fn mirror_reads_int64_provider_version() {
        let mut wire = json!({
            "_id": "t1", "_creationTime": 1.5, "accountId": "a1", "providerThreadId": "p", "subject": "", "snippet": "",
            "participants": [], "isBulk": false, "lastMessageAt": 1, "sortAt": 1, "messageCount": 1, "unreadCount": 0,
            "draftCount": 0, "hasAttachments": false, "syncedLabelKeys": [], "labelKeys": [], "pendingOpCount": 0,
            "deletedPending": false, "triageState": "none", "rev": 7, "updatedAt": 1
        });
        wire["providerVersion"] = serde_json::to_value(ConvexInt64(123_456_789_012)).unwrap();
        let m: ThreadMirror = serde_json::from_value(wire).unwrap();
        assert_eq!(m.provider_version, Some(ConvexInt64(123_456_789_012)));
    }
}
