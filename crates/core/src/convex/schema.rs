//! Rust twins of the shared Convex validators (`convex/lib/validators.ts`, ARCHITECTURE §3.1) and
//! the enum-valued columns of `convex/schema.ts` (§3.2).
//!
//! Field names match the TypeScript schema exactly (`camelCase`); string enum values are
//! `snake_case`. Optional fields are skipped when `None` because Convex rejects `null` for
//! `v.optional(...)`.

use serde::{Deserialize, Serialize};
use serde_with::skip_serializing_none;

use crate::ids::{AccountId, DraftId, LabelId, ScheduledSendId};
use crate::time::UnixMs;

/// `vProvider`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Provider {
    /// Gmail / Google Workspace (Gmail API).
    Gmail,
    /// Outlook / Microsoft 365 (Microsoft Graph).
    Graph,
    /// Any IMAP + SMTP host.
    Imap,
}

/// `vAppError`: a safe-to-show error. `message` never contains provider text or user data.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppError {
    /// An error code from the registry (§15), e.g. `"provider_transient"`.
    pub code: String,
    /// English, safe to show.
    pub message: String,
    /// Whether retrying may succeed.
    pub retryable: bool,
}

/// How a `unsubscribe` action unsubscribes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UnsubscribeMethod {
    /// RFC 8058 one-click POST.
    OneClickPost,
    /// Send an empty message to the `mailto:` address.
    Mailto,
}

/// `vThreadAction`: the canonical thread action vocabulary (§3.1, D-004).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ThreadAction {
    /// Add and remove label keys.
    Modify {
        /// Keys to add.
        add: Vec<String>,
        /// Keys to remove.
        remove: Vec<String>,
    },
    /// Move to trash.
    Trash,
    /// Move out of trash, back to the inbox.
    Untrash,
    /// Delete permanently (only when `caps.deleteForever`).
    DeleteForever,
    /// Move to a folder (folder providers only).
    Move {
        /// Destination folder key.
        to: String,
    },
    /// Unsubscribe from the mailing list.
    Unsubscribe {
        /// One-click POST or mailto.
        method: UnsubscribeMethod,
    },
}

impl ThreadAction {
    /// The `kind` tag as a string (used for `payloadKind` and logging).
    pub fn kind(&self) -> &'static str {
        match self {
            ThreadAction::Modify { .. } => "modify",
            ThreadAction::Trash => "trash",
            ThreadAction::Untrash => "untrash",
            ThreadAction::DeleteForever => "delete_forever",
            ThreadAction::Move { .. } => "move",
            ThreadAction::Unsubscribe { .. } => "unsubscribe",
        }
    }
}

/// `vOpPayload`: what an `ops` row asks the worker to do.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum OpPayload {
    /// A thread action.
    Thread {
        /// The action.
        action: ThreadAction,
    },
    /// Send a scheduled message.
    #[serde(rename_all = "camelCase")]
    Send {
        /// The `scheduledSends` row.
        scheduled_send_id: ScheduledSendId,
    },
    /// Delete a provider draft.
    #[serde(rename_all = "camelCase")]
    DeleteDraft {
        /// The `drafts` row.
        draft_id: DraftId,
    },
    /// Create a label at the provider.
    #[serde(rename_all = "camelCase")]
    LabelCreate {
        /// The `labels` row.
        label_id: LabelId,
    },
    /// Rename a label at the provider.
    #[serde(rename_all = "camelCase")]
    LabelRename {
        /// The `labels` row.
        label_id: LabelId,
        /// New name.
        name: String,
    },
    /// Delete a label at the provider.
    #[serde(rename_all = "camelCase")]
    LabelDelete {
        /// The `labels` row.
        label_id: LabelId,
    },
}

impl OpPayload {
    /// The `kind` tag as a string (`payloadKind`).
    pub fn kind(&self) -> &'static str {
        match self {
            OpPayload::Thread { .. } => "thread",
            OpPayload::Send { .. } => "send",
            OpPayload::DeleteDraft { .. } => "delete_draft",
            OpPayload::LabelCreate { .. } => "label_create",
            OpPayload::LabelRename { .. } => "label_rename",
            OpPayload::LabelDelete { .. } => "label_delete",
        }
    }
}

/// `vOpState` (§5.3).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OpState {
    /// Waiting to be claimed.
    Queued,
    /// Claimed by a worker under a lease.
    Inflight,
    /// Applied (or moot). Terminal.
    Done,
    /// Permanently failed. Terminal.
    Failed,
    /// Cancelled by undo. Terminal.
    Cancelled,
    /// Send outcome could not be confirmed. Terminal.
    Unknown,
}

impl OpState {
    /// `done`, `failed`, `cancelled` and `unknown` are terminal.
    pub fn is_terminal(self) -> bool {
        matches!(self, OpState::Done | OpState::Failed | OpState::Cancelled | OpState::Unknown)
    }
}

/// `ops.outcome`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OpOutcome {
    /// The provider applied the change.
    Applied,
    /// Nothing needed doing (net-zero delta, or the target no longer exists).
    Moot,
}

/// `vTriageCategory`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TriageCategory {
    /// Important.
    Important,
    /// Personal.
    Personal,
    /// Updates.
    Updates,
    /// Newsletters.
    Newsletters,
    /// Promotions.
    Promotions,
    /// Receipts.
    Receipts,
    /// Calendar.
    Calendar,
    /// Notifications.
    Notifications,
    /// Anything else.
    Other,
}

impl TriageCategory {
    /// All categories in contract order.
    pub const ALL: [TriageCategory; 9] = [
        TriageCategory::Important,
        TriageCategory::Personal,
        TriageCategory::Updates,
        TriageCategory::Newsletters,
        TriageCategory::Promotions,
        TriageCategory::Receipts,
        TriageCategory::Calendar,
        TriageCategory::Notifications,
        TriageCategory::Other,
    ];

    /// The wire name.
    pub fn as_str(self) -> &'static str {
        match self {
            TriageCategory::Important => "important",
            TriageCategory::Personal => "personal",
            TriageCategory::Updates => "updates",
            TriageCategory::Newsletters => "newsletters",
            TriageCategory::Promotions => "promotions",
            TriageCategory::Receipts => "receipts",
            TriageCategory::Calendar => "calendar",
            TriageCategory::Notifications => "notifications",
            TriageCategory::Other => "other",
        }
    }

    /// Parses a wire name.
    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|c| c.as_str() == name)
    }
}

/// Who produced a triage result.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TriageSource {
    /// apps/api managed AI.
    Managed,
    /// A user client with a BYO provider.
    Client,
    /// A filter rule.
    Rule,
}

/// `vTriage`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Triage {
    /// Category.
    pub category: TriageCategory,
    /// Whether the user is expected to reply.
    pub needs_reply: bool,
    /// Producer.
    pub source: TriageSource,
    /// Resolved model id, e.g. `"anthropic/claude-haiku-5.5"`.
    pub model: String,
    /// Prompt version used.
    pub prompt_version: u32,
    /// When it was produced.
    pub at: UnixMs,
}

/// `vRuleCond`: one condition of a split or filter rule, tagged by `f`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "f", rename_all = "snake_case")]
pub enum RuleCond {
    /// Sender address equals (lowercased).
    FromEmail {
        /// Address.
        eq: String,
    },
    /// Sender domain equals or is a subdomain of.
    FromDomain {
        /// Domain, e.g. `"example.com"`.
        eq: String,
    },
    /// Any to/cc participant equals.
    ToEmail {
        /// Address.
        eq: String,
    },
    /// Thread has a label key (`sys:*` or `l:*`).
    Label {
        /// Label key.
        key: String,
    },
    /// Triage category equals.
    Triage {
        /// Category.
        category: TriageCategory,
    },
    /// Triage `needsReply` equals.
    NeedsReply {
        /// Expected value.
        eq: bool,
    },
    /// Bulk mail flag equals.
    IsBulk {
        /// Expected value.
        eq: bool,
    },
    /// `List-Id` equals.
    ListId {
        /// List id.
        eq: String,
    },
    /// Subject contains (case-insensitive).
    SubjectContains {
        /// Substring.
        text: String,
    },
    /// Sender VIP status equals.
    Vip {
        /// Expected value.
        eq: bool,
    },
    /// Thread belongs to an account.
    #[serde(rename_all = "camelCase")]
    Account {
        /// The account.
        account_id: AccountId,
    },
}

/// `vRule.match`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuleMatch {
    /// Every condition must hold.
    All,
    /// At least one condition must hold.
    Any,
}

/// `vRule`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Rule {
    /// `all` or `any`.
    pub r#match: RuleMatch,
    /// 1..20 conditions.
    pub conds: Vec<RuleCond>,
}

/// `prefs.remoteImages`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RemoteImages {
    /// Blocked unless allowed per sender (default).
    Block,
    /// Loaded through the image proxy.
    Proxy,
}

/// `prefs.readReceipts`.
#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReadReceiptPrefs {
    /// Opt-in (BRIEF decision 8).
    pub enabled: bool,
    /// When the user acknowledged the reliability warning.
    pub warning_acknowledged_at: Option<UnixMs>,
}

/// `prefs.density`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Density {
    /// Compact rows.
    Compact,
    /// Comfortable rows.
    Comfortable,
}

/// `prefs.theme`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ThemePref {
    /// Follow the OS.
    System,
    /// Light.
    Light,
    /// Dark.
    Dark,
}

/// `prefs.keymap`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Keymap {
    /// Superhuman-style keys.
    Superhuman,
    /// Gmail-style keys.
    Gmail,
}

/// `prefs.notifications`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NotificationPref {
    /// No notifications.
    Off,
    /// Notifications without sender or subject.
    Private,
    /// Notifications with sender and subject.
    Full,
}

/// `vPrefs`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Prefs {
    /// 0 | 5 | 10 | 20 | 30, default 10.
    pub undo_send_seconds: u32,
    /// Default `block`.
    pub remote_images: RemoteImages,
    /// Opt-in read receipts.
    pub read_receipts: ReadReceiptPrefs,
    /// Default true.
    pub cache_bodies_on_device: bool,
    /// List density.
    pub density: Density,
    /// Theme.
    pub theme: ThemePref,
    /// Key map.
    pub keymap: Keymap,
    /// Unified inbox across accounts.
    pub unified_inbox: bool,
    /// Push notifications.
    pub notifications: NotificationPref,
}

/// `vEntitlement.state`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EntitlementState {
    /// Never subscribed.
    None,
    /// In the 14-day trial.
    Trialing,
    /// Paying.
    Active,
    /// Past due, within the grace period.
    Grace,
    /// Subscription ended.
    Lapsed,
    /// `AKATSUKI_BILLING=off`.
    Selfhost,
}

/// `vEntitlement`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Entitlement {
    /// State.
    pub state: EntitlementState,
    /// May connect and sync mailboxes.
    pub hosted_sync: bool,
    /// May use the managed AI provider.
    pub managed_ai: bool,
    /// When this must be re-evaluated (0 = never entitled).
    pub until: UnixMs,
}

/// `vAiProviderKind`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AiProviderKind {
    /// Akatsuki-managed (Pro), through apps/api.
    Akatsuki,
    /// OpenRouter.
    Openrouter,
    /// Anthropic native API.
    Anthropic,
    /// OpenAI.
    Openai,
    /// Google.
    Google,
    /// Vercel AI Gateway.
    Gateway,
    /// Ollama (local).
    Ollama,
    /// Any OpenAI-compatible endpoint.
    OpenaiCompatible,
}

impl AiProviderKind {
    /// The wire name (also the column name in `routing.json`).
    pub fn as_str(self) -> &'static str {
        match self {
            AiProviderKind::Akatsuki => "akatsuki",
            AiProviderKind::Openrouter => "openrouter",
            AiProviderKind::Anthropic => "anthropic",
            AiProviderKind::Openai => "openai",
            AiProviderKind::Google => "google",
            AiProviderKind::Gateway => "gateway",
            AiProviderKind::Ollama => "ollama",
            AiProviderKind::OpenaiCompatible => "openai_compatible",
        }
    }
}

/// `vAiFeature`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AiFeature {
    /// Thread summary.
    Summarize,
    /// Three suggested replies.
    InstantReply,
    /// Write in my voice.
    Compose,
    /// Rewrite a passage.
    Rewrite,
    /// Extract action items.
    ExtractActions,
    /// Auto-triage.
    Triage,
    /// Natural-language search to query grammar.
    NlSearch,
    /// Build a voice profile from sent mail.
    VoiceProfile,
    /// Speech to text.
    Dictation,
}

impl AiFeature {
    /// All features in contract order.
    pub const ALL: [AiFeature; 9] = [
        AiFeature::Summarize,
        AiFeature::InstantReply,
        AiFeature::Compose,
        AiFeature::Rewrite,
        AiFeature::ExtractActions,
        AiFeature::Triage,
        AiFeature::NlSearch,
        AiFeature::VoiceProfile,
        AiFeature::Dictation,
    ];

    /// The wire name (also the directory name under `packages/prompts/features`).
    pub fn as_str(self) -> &'static str {
        match self {
            AiFeature::Summarize => "summarize",
            AiFeature::InstantReply => "instant_reply",
            AiFeature::Compose => "compose",
            AiFeature::Rewrite => "rewrite",
            AiFeature::ExtractActions => "extract_actions",
            AiFeature::Triage => "triage",
            AiFeature::NlSearch => "nl_search",
            AiFeature::VoiceProfile => "voice_profile",
            AiFeature::Dictation => "dictation",
        }
    }

    /// Parses a wire name.
    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|f| f.as_str() == name)
    }
}

/// `mailAccounts.status`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AccountStatus {
    /// Created, not yet syncing.
    Connecting,
    /// Initial sync in progress.
    InitialSync,
    /// Live.
    Live,
    /// Lease lost or repeated failures.
    Degraded,
    /// Credentials rejected.
    ReauthRequired,
    /// Not entitled.
    Paused,
    /// Being removed.
    Removing,
}

/// `mailAccounts.syncPhase`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SyncPhase {
    /// Labels, cursor and watch.
    Baseline,
    /// Inbox threads first.
    InboxFirst,
    /// Everything within the 90-day window.
    Window,
    /// Lazy backfill down to the horizon.
    Backfill,
    /// Done.
    Complete,
}

/// `mailAccounts.caps`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountCaps {
    /// Gmail true; Graph/IMAP false (exclusive folders).
    pub multi_label: bool,
    /// Categories supported.
    pub categories: bool,
    /// Provider-side search supported.
    pub server_search: bool,
    /// Permanent delete supported (false for Gmail).
    pub delete_forever: bool,
}

/// IMAP/SMTP transport security.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TlsMode {
    /// TLS from the first byte (993 / 465).
    Implicit,
    /// STARTTLS upgrade (143 / 587).
    Starttls,
}

/// `mailAccounts.imap`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImapSettings {
    /// IMAP host.
    pub host: String,
    /// IMAP port.
    pub port: u16,
    /// IMAP TLS mode.
    pub tls: TlsMode,
    /// SMTP host.
    pub smtp_host: String,
    /// SMTP port.
    pub smtp_port: u16,
    /// SMTP TLS mode.
    pub smtp_tls: TlsMode,
    /// Advertised IMAP capabilities.
    pub capabilities: Vec<String>,
    /// The server stores sent mail itself.
    pub sent_auto_saved: bool,
}

/// `mailAccounts.progress`.
#[skip_serializing_none]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncProgress {
    /// Threads synced so far.
    pub threads_done: u64,
    /// Estimated total, if known.
    pub threads_estimated: Option<u64>,
}

/// `syncState.watchKind`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WatchKind {
    /// Gmail `users.watch` through Pub/Sub.
    GmailWatch,
    /// Graph change-notification subscription.
    GraphSubscription,
    /// IMAP IDLE.
    ImapIdle,
    /// Polling.
    Poll,
}

/// `accountSecrets.kind`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SecretKind {
    /// `{refreshToken, scope, obtainedAt}`.
    Oauth,
    /// `{username, password}`.
    Imap,
}

/// `profiles.onboarding`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Onboarding {
    /// Must start the trial first.
    Checkout,
    /// Must connect a mailbox.
    Connect,
    /// Finished.
    Done,
}

/// `devices.kind`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeviceKind {
    /// Browser.
    Web,
    /// GPUI desktop app.
    Desktop,
    /// Expo app.
    Mobile,
}

/// `drafts.state`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DraftState {
    /// Being edited.
    Editing,
    /// Scheduled to send.
    Scheduled,
    /// Send in progress.
    Sending,
    /// Sent.
    Sent,
    /// Discarded.
    Discarded,
}

/// `scheduledSends.kind`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SendKind {
    /// Undo-send window.
    Undo,
    /// Send later.
    Later,
}

/// `scheduledSends.state`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SendState {
    /// Waiting for `sendAt`.
    Scheduled,
    /// Cancelled (undo).
    Cancelled,
    /// Due; a `send` op exists.
    Due,
    /// Sending.
    Sending,
    /// Sent.
    Sent,
    /// Failed.
    Failed,
    /// Outcome unknown.
    Unknown,
}

/// `timers.kind`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TimerKind {
    /// Snooze.
    Snooze,
    /// Reminder.
    Reminder,
}

/// `timers.condition`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TimerCondition {
    /// Always fire.
    Always,
    /// Fire only if nobody replied.
    IfNoReply,
}

/// `timers.state`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TimerState {
    /// Waiting.
    Active,
    /// Fired.
    Fired,
    /// Skipped (a reply arrived).
    Skipped,
    /// Cancelled.
    Cancelled,
}

/// `threads.triageState`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TriageState {
    /// Not triaged.
    None,
    /// Claimed by a client.
    Pending,
    /// Triaged.
    Done,
    /// Triage off.
    Off,
}

/// `tombstones.table`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TombstoneTable {
    /// `threads`
    Threads,
    /// `messages`
    Messages,
    /// `labels`
    Labels,
}

/// `billing.status` (Stripe's own strings).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BillingStatus {
    /// No subscription.
    None,
    /// Trialing.
    Trialing,
    /// Active.
    Active,
    /// Past due.
    PastDue,
    /// Canceled.
    Canceled,
    /// Unpaid.
    Unpaid,
    /// Incomplete.
    Incomplete,
    /// Incomplete, expired.
    IncompleteExpired,
    /// Paused.
    Paused,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn thread_action_wire_shapes() {
        let cases = [
            (
                ThreadAction::Modify { add: vec![], remove: vec!["sys:inbox".into()] },
                json!({"kind":"modify","add":[],"remove":["sys:inbox"]}),
            ),
            (ThreadAction::Trash, json!({"kind":"trash"})),
            (ThreadAction::DeleteForever, json!({"kind":"delete_forever"})),
            (ThreadAction::Move { to: "l:abc".into() }, json!({"kind":"move","to":"l:abc"})),
            (
                ThreadAction::Unsubscribe { method: UnsubscribeMethod::OneClickPost },
                json!({"kind":"unsubscribe","method":"one_click_post"}),
            ),
        ];
        for (action, wire) in cases {
            assert_eq!(serde_json::to_value(&action).unwrap(), wire);
            assert_eq!(serde_json::from_value::<ThreadAction>(wire).unwrap(), action);
        }
    }

    #[test]
    fn op_payload_wire_shapes() {
        let p = OpPayload::LabelRename { label_id: "lab1".into(), name: "X".into() };
        assert_eq!(serde_json::to_value(&p).unwrap(), json!({"kind":"label_rename","labelId":"lab1","name":"X"}));
        let p = OpPayload::Send { scheduled_send_id: "s1".into() };
        assert_eq!(serde_json::to_value(&p).unwrap(), json!({"kind":"send","scheduledSendId":"s1"}));
        let p: OpPayload = serde_json::from_value(json!({"kind":"thread","action":{"kind":"untrash"}})).unwrap();
        assert_eq!(p, OpPayload::Thread { action: ThreadAction::Untrash });
    }

    #[test]
    fn rule_wire_shape() {
        let r = Rule {
            r#match: RuleMatch::Any,
            conds: vec![RuleCond::FromDomain { eq: "x.com".into() }, RuleCond::Account { account_id: "a1".into() }],
        };
        assert_eq!(
            serde_json::to_value(&r).unwrap(),
            json!({"match":"any","conds":[{"f":"from_domain","eq":"x.com"},{"f":"account","accountId":"a1"}]})
        );
    }

    #[test]
    fn enum_values_are_snake_case() {
        assert_eq!(serde_json::to_value(AccountStatus::ReauthRequired).unwrap(), json!("reauth_required"));
        assert_eq!(serde_json::to_value(AiProviderKind::OpenaiCompatible).unwrap(), json!("openai_compatible"));
        assert_eq!(serde_json::to_value(AiFeature::InstantReply).unwrap(), json!("instant_reply"));
        assert_eq!(serde_json::to_value(SyncPhase::InboxFirst).unwrap(), json!("inbox_first"));
        assert_eq!(serde_json::to_value(BillingStatus::IncompleteExpired).unwrap(), json!("incomplete_expired"));
        assert_eq!(serde_json::to_value(TombstoneTable::Threads).unwrap(), json!("threads"));
    }

    #[test]
    fn optional_fields_are_omitted() {
        let p = ReadReceiptPrefs { enabled: false, warning_acknowledged_at: None };
        assert_eq!(serde_json::to_value(&p).unwrap(), json!({"enabled": false}));
    }
}
