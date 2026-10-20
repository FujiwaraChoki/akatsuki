//! Arguments and results of the worker service functions (`convex/worker.ts`, ARCHITECTURE §4.3),
//! including the ops-outbox claim/complete protocol (§5.4) and the `ThreadUpsert` family (§4.4).
//!
//! Every function that takes an `accountId` also takes `workerId`. Function names are listed in
//! [`functions`] so the Convex bindings and tests share one spelling.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_with::skip_serializing_none;

use super::rows::{FolderCursor, SendPreview, SyncStateDoc};
use super::schema::{
    AccountCaps, AccountStatus, AppError, Entitlement, ImapSettings, OpOutcome, OpPayload, Provider, RemoteImages,
    SecretKind, SendKind, SyncPhase, SyncProgress, TriageCategory, WatchKind,
};
use super::values::{ConvexBytes, ConvexInt64};
use crate::ids::{AccountId, DraftId, MessageId, OpId, ScheduledSendId, ThreadId};
use crate::model::{Addr, AuthSummary, ImapLocation, ListUnsubscribe, RemoteLabel};
use crate::time::UnixMs;

/// Fully qualified names of the worker functions.
pub mod functions {
    /// `worker.assignedAccounts` (query, subscribed)
    pub const ASSIGNED_ACCOUNTS: &str = "worker:assignedAccounts";
    /// `worker.claimAccount`
    pub const CLAIM_ACCOUNT: &str = "worker:claimAccount";
    /// `worker.heartbeat`
    pub const HEARTBEAT: &str = "worker:heartbeat";
    /// `worker.releaseAccount`
    pub const RELEASE_ACCOUNT: &str = "worker:releaseAccount";
    /// `worker.getSecret` (query)
    pub const GET_SECRET: &str = "worker:getSecret";
    /// `worker.putSecret`
    pub const PUT_SECRET: &str = "worker:putSecret";
    /// `worker.createAccount`
    pub const CREATE_ACCOUNT: &str = "worker:createAccount";
    /// `worker.setAccountStatus`
    pub const SET_ACCOUNT_STATUS: &str = "worker:setAccountStatus";
    /// `worker.upsertIdentities`
    pub const UPSERT_IDENTITIES: &str = "worker:upsertIdentities";
    /// `worker.upsertLabels`
    pub const UPSERT_LABELS: &str = "worker:upsertLabels";
    /// `worker.upsertThreads`
    pub const UPSERT_THREADS: &str = "worker:upsertThreads";
    /// `worker.setCursor`
    pub const SET_CURSOR: &str = "worker:setCursor";
    /// `worker.setWatch`
    pub const SET_WATCH: &str = "worker:setWatch";
    /// `worker.accountsForPush` (query)
    pub const ACCOUNTS_FOR_PUSH: &str = "worker:accountsForPush";
    /// `worker.pendingOpAccounts` (query, subscribed)
    pub const PENDING_OP_ACCOUNTS: &str = "worker:pendingOpAccounts";
    /// `worker.claimOps`
    pub const CLAIM_OPS: &str = "worker:claimOps";
    /// `worker.markSendStarted`
    pub const MARK_SEND_STARTED: &str = "worker:markSendStarted";
    /// `worker.completeOps`
    pub const COMPLETE_OPS: &str = "worker:completeOps";
    /// `worker.recordDraft`
    pub const RECORD_DRAFT: &str = "worker:recordDraft";
    /// `worker.scheduleSend`
    pub const SCHEDULE_SEND: &str = "worker:scheduleSend";
    /// `worker.resolveMessages` (query)
    pub const RESOLVE_MESSAGES: &str = "worker:resolveMessages";
    /// `worker.resolveThread` (query)
    pub const RESOLVE_THREAD: &str = "worker:resolveThread";
    /// `worker.userContext` (query)
    pub const USER_CONTEXT: &str = "worker:userContext";
    /// `worker.claimTriage` (managed triage, DECISIONS D-390)
    pub const CLAIM_TRIAGE: &str = "worker:claimTriage";
    /// `worker.applyTriage`
    pub const APPLY_TRIAGE: &str = "worker:applyTriage";
    /// `worker.recordReadReceipt`
    pub const RECORD_READ_RECEIPT: &str = "worker:recordReadReceipt";
    /// `worker.ackAccountRemoved`
    pub const ACK_ACCOUNT_REMOVED: &str = "worker:ackAccountRemoved";
    /// `worker.threadIdsByProvider` (query; DECISIONS D-236)
    pub const THREAD_IDS_BY_PROVIDER: &str = "worker:threadIdsByProvider";
}

/// Arguments that only carry the shard.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ShardArgs {
    /// Shard number (0 in v1).
    pub shard: u32,
}

/// One row of `worker.assignedAccounts`.
#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AssignedAccount {
    /// Account.
    pub account_id: AccountId,
    /// Owner.
    pub user_id: String,
    /// Provider.
    pub provider: Provider,
    /// Provider account id.
    pub provider_account_id: String,
    /// Address.
    pub email: String,
    /// Status.
    pub status: AccountStatus,
    /// Owner has `hostedSync`.
    pub entitled: bool,
    /// Owner's triage route is `akatsuki`.
    pub server_triage: bool,
    /// IMAP settings.
    pub imap: Option<ImapSettings>,
    /// Capabilities.
    pub caps: AccountCaps,
}

/// `worker.claimAccount` arguments.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClaimAccountArgs {
    /// Account.
    pub account_id: AccountId,
    /// This worker instance.
    pub worker_id: String,
    /// Lease length (≤ 120000).
    pub lease_ms: i64,
}

/// `worker.claimAccount` result.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ClaimAccountResult {
    /// Claimed.
    #[serde(rename_all = "camelCase")]
    Claimed {
        /// Always `true`.
        ok: True,
        /// The sync state row.
        sync_state: SyncStateDoc,
        /// Folder cursors (Graph, IMAP).
        folder_cursors: Vec<FolderCursor>,
    },
    /// Another worker holds the lease.
    Taken {
        /// Always `false`.
        ok: False,
        /// The current owner.
        owner: String,
    },
}

/// The JSON literal `true` (for untagged unions discriminated by a boolean).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct True;

/// The JSON literal `false`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct False;

macro_rules! bool_literal {
    ($ty:ident, $value:literal) => {
        impl Serialize for $ty {
            fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
                s.serialize_bool($value)
            }
        }
        impl<'de> Deserialize<'de> for $ty {
            fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
                if bool::deserialize(d)? == $value {
                    Ok($ty)
                } else {
                    Err(serde::de::Error::custom(concat!("expected ", stringify!($value))))
                }
            }
        }
    };
}
bool_literal!(True, true);
bool_literal!(False, false);

/// `worker.heartbeat` arguments.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HeartbeatArgs {
    /// This worker instance.
    pub worker_id: String,
    /// Accounts whose leases to renew.
    pub account_ids: Vec<AccountId>,
    /// Lease length.
    pub lease_ms: i64,
}

/// `worker.heartbeat` result.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HeartbeatResult {
    /// Accounts whose lease was lost; their actors stop at once.
    pub lost: Vec<AccountId>,
}

/// Arguments that carry an account and the worker id (`releaseAccount`, `ackAccountRemoved`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountWorkerArgs {
    /// Account.
    pub account_id: AccountId,
    /// This worker instance.
    pub worker_id: String,
}

/// `worker.getSecret` arguments.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GetSecretArgs {
    /// Account.
    pub account_id: AccountId,
    /// Secret kind.
    pub kind: SecretKind,
}

/// `worker.getSecret` result (when present).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SecretRecord {
    /// Which `TOKEN_ENC_KEYS` entry sealed it.
    pub key_id: String,
    /// 24-byte XChaCha20 nonce.
    pub nonce: ConvexBytes,
    /// Ciphertext.
    pub ciphertext: ConvexBytes,
    /// CAS counter.
    pub version: u64,
}

/// `worker.putSecret` arguments.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PutSecretArgs {
    /// Account.
    pub account_id: AccountId,
    /// This worker instance.
    pub worker_id: String,
    /// Secret kind.
    pub kind: SecretKind,
    /// Key id.
    pub key_id: String,
    /// Nonce.
    pub nonce: ConvexBytes,
    /// Ciphertext.
    pub ciphertext: ConvexBytes,
    /// Expected current version (throws `version_conflict` otherwise).
    pub expected_version: u64,
}

/// `worker.putSecret` result.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PutSecretResult {
    /// New version.
    pub version: u64,
}

/// A send-as identity reported by the provider.
#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IdentityIn {
    /// Address.
    pub email: String,
    /// Display name.
    pub name: Option<String>,
    /// Reply-To.
    pub reply_to: Option<String>,
    /// Default identity.
    pub is_default: bool,
}

/// Normalises a `providerAccountId` exactly as `worker.createAccount` stores it: Gmail ids (the address) are
/// trimmed and lowercased, Graph object ids and IMAP `user@host` ids are kept as given.
pub fn normalize_provider_account_id(provider: Provider, raw: &str) -> String {
    match provider {
        Provider::Gmail => raw.trim().to_lowercase(),
        Provider::Graph | Provider::Imap => raw.to_owned(),
    }
}

/// The XChaCha20-Poly1305 associated data that binds an `accountSecrets` ciphertext to its mailbox
/// (ARCHITECTURE §14.1, D-127):
///
/// `"akatsuki:secret:v1:" + userId + ":" + provider + ":" + kind + ":" + providerAccountId`
///
/// Every input is known before `worker.createAccount` inserts the `mailAccounts` row (the OAuth `state`
/// carries `userId`; the profile gives `providerAccountId`), and none of them changes for the life of the
/// row, so the same AAD seals on first connect, `putSecret` (refresh-token rotation) and `rotate-secrets`.
/// `providerAccountId` is normalised first ([`normalize_provider_account_id`]) and comes last: the other
/// fields never contain `:`, so the encoding is unambiguous.
pub fn secret_aad(user_id: &str, provider: Provider, provider_account_id: &str, kind: SecretKind) -> Vec<u8> {
    let provider_name = match provider {
        Provider::Gmail => "gmail",
        Provider::Graph => "graph",
        Provider::Imap => "imap",
    };
    let kind_name = match kind {
        SecretKind::Oauth => "oauth",
        SecretKind::Imap => "imap",
    };
    let id = normalize_provider_account_id(provider, provider_account_id);
    format!("akatsuki:secret:v1:{user_id}:{provider_name}:{kind_name}:{id}").into_bytes()
}

impl AssignedAccount {
    /// [`secret_aad`] for this account's secret of `kind`.
    pub fn secret_aad(&self, kind: SecretKind) -> Vec<u8> {
        secret_aad(&self.user_id, self.provider, &self.provider_account_id, kind)
    }
}

impl CreateAccountArgs {
    /// [`secret_aad`] that `secret` must have been sealed with.
    pub fn secret_aad(&self) -> Vec<u8> {
        secret_aad(&self.user_id, self.provider, &self.provider_account_id, self.secret.kind)
    }
}

/// A sealed secret for `worker.createAccount`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SecretIn {
    /// Kind.
    pub kind: SecretKind,
    /// Key id.
    pub key_id: String,
    /// Nonce.
    pub nonce: ConvexBytes,
    /// Ciphertext.
    pub ciphertext: ConvexBytes,
}

/// `worker.createAccount` arguments.
#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateAccountArgs {
    /// Owner.
    pub user_id: String,
    /// Provider.
    pub provider: Provider,
    /// Provider account id.
    pub provider_account_id: String,
    /// Address.
    pub email: String,
    /// Display name.
    pub display_name: Option<String>,
    /// IMAP settings.
    pub imap: Option<ImapSettings>,
    /// Capabilities.
    pub caps: AccountCaps,
    /// Identities.
    pub identities: Vec<IdentityIn>,
    /// Sealed credentials.
    pub secret: SecretIn,
    /// Set for a reauth: Convex refuses a grant for a different mailbox (DECISIONS D-395).
    pub reauth_account_id: Option<AccountId>,
}

/// `worker.createAccount` result.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateAccountResult {
    /// Account.
    pub account_id: AccountId,
    /// The same user reconnected an existing mailbox.
    pub reconnected: bool,
}

/// `worker.setAccountStatus` arguments. `statusDetail: null` clears the detail.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SetAccountStatusArgs {
    /// Account.
    pub account_id: AccountId,
    /// This worker instance.
    pub worker_id: String,
    /// New status.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<AccountStatus>,
    /// `Some(None)` sends `null` (clear), `None` leaves it unchanged.
    #[serde(default, skip_serializing_if = "Option::is_none", with = "double_option")]
    pub status_detail: Option<Option<String>>,
    /// Sync phase.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sync_phase: Option<SyncPhase>,
    /// Progress (written at most every 5 s).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub progress: Option<SyncProgress>,
    /// Oldest synced message.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub oldest_synced_at: Option<UnixMs>,
    /// Last successful sync.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_sync_at: Option<UnixMs>,
}

/// Serde helper for `Option<Option<T>>`: absent / `null` / value.
pub mod double_option {
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    /// Serializes `Some(None)` as `null` and `Some(Some(v))` as `v`. Pair with
    /// `skip_serializing_if = "Option::is_none"` so `None` is omitted.
    pub fn serialize<T: Serialize, S: Serializer>(value: &Option<Option<T>>, s: S) -> Result<S::Ok, S::Error> {
        match value {
            Some(inner) => inner.serialize(s),
            None => s.serialize_none(),
        }
    }

    /// A present field (even `null`) becomes `Some(..)`.
    pub fn deserialize<'de, T: Deserialize<'de>, D: Deserializer<'de>>(d: D) -> Result<Option<Option<T>>, D::Error> {
        Option::<T>::deserialize(d).map(Some)
    }
}

/// `worker.upsertIdentities` arguments.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpsertIdentitiesArgs {
    /// Account.
    pub account_id: AccountId,
    /// This worker instance.
    pub worker_id: String,
    /// Identities.
    pub identities: Vec<IdentityIn>,
}

/// `worker.upsertLabels` arguments.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpsertLabelsArgs {
    /// Account.
    pub account_id: AccountId,
    /// This worker instance.
    pub worker_id: String,
    /// Labels (`RemoteLabelIn[]`).
    pub labels: Vec<RemoteLabel>,
    /// The list is complete: absent labels are deleted.
    pub complete: bool,
}

/// `worker.upsertLabels` result.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpsertLabelsResult {
    /// Provider label id → label key.
    pub label_key_by_provider_id: BTreeMap<String, String>,
}

/// `MessageUpsert` (worker → Convex). Labels are already label keys.
#[skip_serializing_none]
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MessageUpsert {
    /// Provider message id.
    pub provider_message_id: String,
    /// `Message-ID` without brackets.
    pub rfc_message_id: Option<String>,
    /// `In-Reply-To`.
    pub in_reply_to: Option<String>,
    /// `References` (last 20).
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
    /// Label keys.
    pub label_keys: Vec<String>,
    /// Unread.
    pub is_unread: bool,
    /// Draft.
    pub is_draft: bool,
    /// Outbound.
    pub is_outbound: bool,
    /// Attachments (best effort).
    pub has_attachments: bool,
    /// Size.
    pub size_bytes: Option<u64>,
    /// `List-Id`.
    pub list_id: Option<String>,
    /// Unsubscribe options.
    pub list_unsubscribe: Option<ListUnsubscribe>,
    /// Authentication summary.
    pub auth_summary: Option<AuthSummary>,
    /// IMAP locations.
    pub imap_locations: Option<Vec<ImapLocation>>,
}

/// `ThreadUpsert` (worker → Convex).
#[skip_serializing_none]
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadUpsert {
    /// Provider thread id.
    pub provider_thread_id: String,
    /// Version guard (`v.int64()`).
    pub provider_version: Option<ConvexInt64>,
    /// `messages` is the full thread: messages not listed are deleted.
    pub complete: bool,
    /// Messages (≤ 400 per call in total).
    pub messages: Vec<MessageUpsert>,
    /// Messages removed at the provider.
    pub removed_provider_message_ids: Option<Vec<String>>,
}

/// `CursorUpdate`, committed in the same mutation as the data it covers.
#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CursorUpdate {
    /// Gmail history cursor.
    #[serde(rename_all = "camelCase")]
    Gmail {
        /// Decimal u64.
        history_id: String,
    },
    /// Graph or IMAP folder cursor.
    #[serde(rename_all = "camelCase")]
    Folder {
        /// Folder id / mailbox name.
        remote_folder_id: String,
        /// The folder's label key.
        label_key: String,
        /// IMAP `UIDVALIDITY`.
        uid_validity: Option<u32>,
        /// IMAP `UIDNEXT`.
        uid_next: Option<u32>,
        /// IMAP `HIGHESTMODSEQ` (decimal u64).
        highest_mod_seq: Option<String>,
        /// Graph delta link.
        delta_link: Option<String>,
    },
    /// Backfill date cursor.
    #[serde(rename_all = "camelCase")]
    Backfill {
        /// Fetch messages before this instant next.
        before_at: UnixMs,
    },
    /// Sync phase change.
    #[serde(rename_all = "camelCase")]
    Phase {
        /// New phase.
        sync_phase: SyncPhase,
    },
}

/// `worker.upsertThreads` arguments.
#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpsertThreadsArgs {
    /// Account.
    pub account_id: AccountId,
    /// This worker instance.
    pub worker_id: String,
    /// Threads (≤ 100).
    pub threads: Vec<ThreadUpsert>,
    /// Provider thread ids deleted at the provider.
    pub deletes: Option<Vec<String>>,
    /// Provider message ids deleted (or moved away) at the provider, ≤ 400 (D-410). Convex finds
    /// their threads, so this works for partial (IMAP) threads the worker did not re-fetch.
    pub deleted_messages: Option<Vec<String>>,
    /// Cursor to commit with the data.
    pub cursor: Option<CursorUpdate>,
}

/// A newly arrived inbound thread (`upsertThreads` result).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NewInbound {
    /// Thread.
    pub thread_id: ThreadId,
    /// In the inbox.
    pub is_inbox: bool,
}

/// `worker.upsertThreads` result.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpsertThreadsResult {
    /// Threads written.
    pub written: u32,
    /// Threads skipped because their version was stale.
    pub skipped_stale: u32,
    /// New inbound threads (for managed triage and notifications).
    pub new_inbound: Vec<NewInbound>,
}

/// `worker.setCursor` arguments.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SetCursorArgs {
    /// Account.
    pub account_id: AccountId,
    /// This worker instance.
    pub worker_id: String,
    /// Cursor.
    pub cursor: CursorUpdate,
}

/// `worker.setWatch` arguments.
#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SetWatchArgs {
    /// Account.
    pub account_id: AccountId,
    /// This worker instance.
    pub worker_id: String,
    /// Watch kind.
    pub watch_kind: WatchKind,
    /// Graph subscription id.
    pub external_id: Option<String>,
    /// Expiry.
    pub expires_at: Option<UnixMs>,
    /// Hex sha256 of the Graph clientState.
    pub client_state_hash: Option<String>,
}

/// `worker.accountsForPush` arguments.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "by", rename_all = "snake_case")]
pub enum AccountsForPushArgs {
    /// Gmail Pub/Sub fan-in by address.
    GmailEmail {
        /// Address from the push payload.
        email: String,
    },
    /// Graph notifications by subscription id.
    #[serde(rename_all = "camelCase")]
    GraphSubscription {
        /// Subscription id.
        subscription_id: String,
    },
}

/// One row of `worker.accountsForPush`.
#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PushAccount {
    /// Account.
    pub account_id: AccountId,
    /// Hex sha256 of the Graph clientState.
    pub client_state_hash: Option<String>,
}

/// One row of `worker.pendingOpAccounts`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PendingOpAccount {
    /// Account.
    pub account_id: AccountId,
    /// Earliest `availableAt` of its queued ops.
    pub min_available_at: UnixMs,
}

/// `worker.claimOps` arguments.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClaimOpsArgs {
    /// Account.
    pub account_id: AccountId,
    /// This worker instance.
    pub worker_id: String,
    /// At most 100.
    pub limit: u32,
    /// Lease length (≤ 120000).
    pub lease_ms: i64,
}

/// The op part of a [`ClaimedOp`].
#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClaimedOpCore {
    /// Op id.
    #[serde(rename = "_id")]
    pub id: OpId,
    /// Gesture id.
    pub client_op_id: String,
    /// Payload.
    pub payload: OpPayload,
    /// Attempt number (already incremented by the claim).
    pub attempt: u32,
    /// Send outcome is ambiguous: verify only, never re-send.
    pub verify_only: bool,
    /// Milliseconds since the provider send began (`markSendStarted`), on Convex's clock; times
    /// the verification schedule of an ambiguous send.
    pub send_elapsed_ms: Option<i64>,
}

/// A message of the claimed thread.
#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClaimedMessage {
    /// Provider message id.
    pub provider_message_id: String,
    /// Label keys.
    pub label_keys: Vec<String>,
    /// IMAP locations.
    pub imap_locations: Option<Vec<ImapLocation>>,
}

/// The thread a claimed op targets.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClaimedThread {
    /// Provider thread id.
    pub provider_thread_id: String,
    /// Provider-confirmed keys.
    pub synced_label_keys: Vec<String>,
    /// Messages.
    pub messages: Vec<ClaimedMessage>,
}

/// Label metadata a claimed op needs.
#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClaimedLabel {
    /// Provider id, once known.
    pub provider_label_id: Option<String>,
    /// Name.
    pub name: String,
    /// Folder semantics.
    pub exclusive: bool,
    /// Role.
    pub role: Option<String>,
}

/// The provider draft a claimed send uses.
#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClaimedDraft {
    /// Provider draft id.
    pub provider_draft_id: String,
    /// Provider message id.
    pub provider_message_id: Option<String>,
    /// Fixed `Message-ID`, the send dedupe key.
    pub rfc_message_id: String,
}

/// Send details of a claimed `send` op.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClaimedSend {
    /// The draft.
    pub draft: ClaimedDraft,
    /// Undo window or send-later.
    pub kind: SendKind,
    /// Scheduled time.
    pub send_at: UnixMs,
}

/// `ClaimedOp` (§5.4).
#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClaimedOp {
    /// The op.
    pub op: ClaimedOpCore,
    /// The thread, for thread ops.
    pub thread: Option<ClaimedThread>,
    /// Labels referenced by the op, by label key.
    pub labels: BTreeMap<String, ClaimedLabel>,
    /// Send details, for send ops.
    pub send: Option<ClaimedSend>,
    /// The provider draft, for `delete_draft` ops (DECISIONS D-057).
    pub draft: Option<ClaimedDraftRef>,
}

/// The provider draft a `delete_draft` op targets (DECISIONS D-057).
#[skip_serializing_none]
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClaimedDraftRef {
    /// Provider draft id, if one was saved.
    pub provider_draft_id: Option<String>,
    /// Provider message id, if known.
    pub provider_message_id: Option<String>,
}

/// `worker.markSendStarted` arguments.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MarkSendStartedArgs {
    /// Op.
    pub op_id: OpId,
    /// This worker instance.
    pub worker_id: String,
}

/// `OpResult` (§5.4).
#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum OpResult {
    /// Success.
    #[serde(rename_all = "camelCase")]
    Ok {
        /// Op.
        op_id: OpId,
        /// Always `true`.
        ok: True,
        /// Applied or moot.
        outcome: OpOutcome,
        /// For `label_create`.
        provider_label_id: Option<String>,
        /// For `send`.
        sent_provider_message_id: Option<String>,
    },
    /// Failure.
    #[serde(rename_all = "camelCase")]
    Err {
        /// Op.
        op_id: OpId,
        /// Always `false`.
        ok: False,
        /// Error.
        error: AppError,
        /// Earliest retry delay.
        retry_after_ms: Option<i64>,
        /// `false` for auth and rate-limit errors.
        count_attempt: bool,
    },
}

/// `worker.completeOps` arguments.
#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CompleteOpsArgs {
    /// Account.
    pub account_id: AccountId,
    /// This worker instance.
    pub worker_id: String,
    /// Results (≤ 100).
    pub results: Vec<OpResult>,
    /// Post-op thread state (`complete: true`).
    pub threads: Option<Vec<ThreadUpsert>>,
    /// Labels created or changed.
    pub labels: Option<Vec<RemoteLabel>>,
}

/// `worker.recordDraft` arguments.
#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecordDraftArgs {
    /// Owner.
    pub user_id: String,
    /// Account.
    pub account_id: AccountId,
    /// Client idempotency key.
    pub client_draft_id: String,
    /// CAS base.
    pub base_version: u64,
    /// Provider draft id.
    pub provider_draft_id: String,
    /// Provider message id.
    pub provider_message_id: String,
    /// Fixed `Message-ID`.
    pub rfc_message_id: String,
    /// Thread.
    pub thread_id: Option<ThreadId>,
    /// Message being replied to.
    pub reply_to_message_id: Option<MessageId>,
    /// Subject.
    pub subject: String,
    /// To.
    pub to: Vec<Addr>,
    /// Cc.
    pub cc: Vec<Addr>,
    /// Bcc.
    pub bcc: Vec<Addr>,
    /// Device that saved.
    pub device_id: Option<String>,
    /// The read-receipt token when the composed draft carries the pixel (D-239, D-412).
    pub read_receipt_token: Option<String>,
}

/// `worker.recordDraft` result.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum RecordDraftResult {
    /// Version conflict.
    Conflict {
        /// Always `true`.
        conflict: True,
        /// Current version.
        version: u64,
    },
    /// Saved.
    #[serde(rename_all = "camelCase")]
    Saved {
        /// Draft row.
        draft_id: DraftId,
        /// New version.
        version: u64,
    },
}

/// `worker.scheduleSend` arguments.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScheduleSendArgs {
    /// Owner.
    pub user_id: String,
    /// Account.
    pub account_id: AccountId,
    /// Client idempotency key.
    pub client_send_id: String,
    /// Draft.
    pub draft_id: DraftId,
    /// Kind.
    pub kind: SendKind,
    /// When to send.
    pub send_at: UnixMs,
    /// Preview.
    pub preview: SendPreview,
}

/// Initial state of a scheduled send.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScheduleOutcome {
    /// Waiting for `sendAt`.
    Scheduled,
    /// Due now (`undoSeconds == 0`).
    Due,
    /// A cancel tombstone already existed.
    Cancelled,
}

/// `worker.scheduleSend` result.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScheduleSendResult {
    /// Row.
    pub send_id: ScheduledSendId,
    /// State.
    pub state: ScheduleOutcome,
}

/// `worker.resolveMessages` arguments.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResolveMessagesArgs {
    /// Caller (JWT `sub`).
    pub user_id: String,
    /// Ids (≤ 100).
    pub message_ids: Vec<MessageId>,
}

/// One row of `worker.resolveMessages`. Ids the user does not own are omitted.
#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResolvedMessage {
    /// Message.
    pub message_id: MessageId,
    /// Account.
    pub account_id: AccountId,
    /// Thread.
    pub thread_id: ThreadId,
    /// Provider message id.
    pub provider_message_id: String,
    /// Provider thread id.
    pub provider_thread_id: String,
    /// IMAP locations.
    pub imap_locations: Option<Vec<ImapLocation>>,
    /// Draft.
    pub is_draft: bool,
}

/// `worker.resolveThread` arguments.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResolveThreadArgs {
    /// Caller.
    pub user_id: String,
    /// Thread.
    pub thread_id: ThreadId,
}

/// A message of a resolved thread.
#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResolvedThreadMessage {
    /// Message.
    pub message_id: MessageId,
    /// Provider message id.
    pub provider_message_id: String,
    /// IMAP locations.
    pub imap_locations: Option<Vec<ImapLocation>>,
    /// Date.
    pub date: UnixMs,
    /// Draft.
    pub is_draft: bool,
}

/// `worker.resolveThread` result (when owned).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResolvedThread {
    /// Account.
    pub account_id: AccountId,
    /// Provider thread id.
    pub provider_thread_id: String,
    /// Messages.
    pub messages: Vec<ResolvedThreadMessage>,
}

/// `worker.userContext` arguments.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UserContextArgs {
    /// Caller.
    pub user_id: String,
}

/// The prefs subset the worker needs.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkerPrefs {
    /// Remote image policy.
    pub remote_images: RemoteImages,
    /// Read receipt opt-in.
    pub read_receipts: super::schema::ReadReceiptPrefs,
}

/// `worker.userContext` result.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UserContext {
    /// Entitlement.
    pub entitlement: Entitlement,
    /// Prefs subset.
    pub prefs: WorkerPrefs,
    /// The user's accounts.
    pub account_ids: Vec<AccountId>,
    /// `profiles.timezone` (IANA), for search dates (§11.1, D-404). `"UTC"` when absent.
    #[serde(default = "utc_zone")]
    pub timezone: String,
}

fn utc_zone() -> String {
    "UTC".to_owned()
}

/// One triage result (`worker.applyTriage`, `ai.setTriage`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TriageResult {
    /// Thread.
    pub thread_id: ThreadId,
    /// Category.
    pub category: TriageCategory,
    /// Needs reply.
    pub needs_reply: bool,
    /// Resolved model id.
    pub model: String,
    /// Prompt version.
    pub prompt_version: u32,
}

/// `worker.claimTriage` arguments (managed triage, §12.5, DECISIONS D-390).
#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkerClaimTriageArgs {
    /// Account (the worker must hold its lease).
    pub account_id: AccountId,
    /// This worker instance.
    pub worker_id: String,
    /// The `newInbound` threads to claim (≤ 100); `None` sweeps untriaged and lease-expired threads.
    pub thread_ids: Option<Vec<ThreadId>>,
    /// At most this many (1–25).
    pub limit: u32,
}

/// `worker.applyTriage` arguments.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApplyTriageArgs {
    /// Results (≤ 50).
    pub results: Vec<TriageResult>,
}

/// `worker.recordReadReceipt` arguments.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecordReadReceiptArgs {
    /// 22-char base64url token.
    pub token: String,
    /// When.
    pub at: UnixMs,
    /// Opened through a proxy (e.g. Apple MPP).
    pub proxied: bool,
}

/// `worker.threadIdsByProvider` arguments (DECISIONS D-236).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadIdsByProviderArgs {
    /// Account.
    pub account_id: AccountId,
    /// Provider thread ids (≤ 100).
    pub provider_thread_ids: Vec<String>,
}

/// One row of `worker.threadIdsByProvider`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderThreadIdRow {
    /// Provider thread id.
    pub provider_thread_id: String,
    /// Convex thread id.
    pub thread_id: ThreadId,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::convex::ThreadAction;
    use serde_json::json;

    #[test]
    fn secret_aad_binds_user_provider_kind_and_mailbox() {
        assert_eq!(
            secret_aad("u1", Provider::Gmail, "  Ada@Example.COM ", SecretKind::Oauth),
            b"akatsuki:secret:v1:u1:gmail:oauth:ada@example.com".to_vec()
        );
        assert_eq!(
            secret_aad("u1", Provider::Imap, "Ada@mail.example.com", SecretKind::Imap),
            b"akatsuki:secret:v1:u1:imap:imap:Ada@mail.example.com".to_vec()
        );
        assert_eq!(
            secret_aad("u1", Provider::Graph, "00000000-0000-0000-0000-00000000ABCD", SecretKind::Oauth),
            b"akatsuki:secret:v1:u1:graph:oauth:00000000-0000-0000-0000-00000000ABCD".to_vec()
        );
        // Another user, mailbox, provider or kind gives different associated data.
        let base = secret_aad("u1", Provider::Gmail, "ada@example.com", SecretKind::Oauth);
        assert_ne!(base, secret_aad("u2", Provider::Gmail, "ada@example.com", SecretKind::Oauth));
        assert_ne!(base, secret_aad("u1", Provider::Gmail, "bob@example.com", SecretKind::Oauth));
        assert_ne!(base, secret_aad("u1", Provider::Imap, "ada@example.com", SecretKind::Oauth));
        assert_ne!(base, secret_aad("u1", Provider::Gmail, "ada@example.com", SecretKind::Imap));
    }

    #[test]
    fn claim_account_result_variants() {
        let taken: ClaimAccountResult = serde_json::from_value(json!({"ok": false, "owner": "w2"})).unwrap();
        assert_eq!(taken, ClaimAccountResult::Taken { ok: False, owner: "w2".into() });
    }

    #[test]
    fn op_result_wire_shapes() {
        let ok = OpResult::Ok {
            op_id: "o1".into(),
            ok: True,
            outcome: OpOutcome::Moot,
            provider_label_id: None,
            sent_provider_message_id: None,
        };
        assert_eq!(serde_json::to_value(&ok).unwrap(), json!({"opId": "o1", "ok": true, "outcome": "moot"}));
        let err = OpResult::Err {
            op_id: "o2".into(),
            ok: False,
            error: AppError { code: "provider_transient".into(), message: "Try again".into(), retryable: true },
            retry_after_ms: Some(2000),
            count_attempt: true,
        };
        let wire = json!({"opId": "o2", "ok": false, "error": {"code": "provider_transient", "message": "Try again", "retryable": true}, "retryAfterMs": 2000, "countAttempt": true});
        assert_eq!(serde_json::to_value(&err).unwrap(), wire);
        assert_eq!(serde_json::from_value::<OpResult>(wire).unwrap(), err);
    }

    #[test]
    fn claimed_op_parses() {
        let wire = json!({
            "op": {"_id": "o1", "clientOpId": "c1", "payload": {"kind": "thread", "action": {"kind": "modify", "add": [], "remove": ["sys:inbox"]}}, "attempt": 1, "verifyOnly": false},
            "thread": {"providerThreadId": "t", "syncedLabelKeys": ["sys:inbox"], "messages": [{"providerMessageId": "m", "labelKeys": ["sys:inbox"]}]},
            "labels": {}
        });
        let op: ClaimedOp = serde_json::from_value(wire).unwrap();
        assert_eq!(
            op.op.payload,
            OpPayload::Thread { action: ThreadAction::Modify { add: vec![], remove: vec!["sys:inbox".into()] } }
        );
    }

    #[test]
    fn cursor_update_shapes() {
        let c = CursorUpdate::Folder {
            remote_folder_id: "INBOX".into(),
            label_key: "sys:inbox".into(),
            uid_validity: Some(7),
            uid_next: None,
            highest_mod_seq: Some("9007199254740993".into()),
            delta_link: None,
        };
        assert_eq!(
            serde_json::to_value(&c).unwrap(),
            json!({"kind": "folder", "remoteFolderId": "INBOX", "labelKey": "sys:inbox", "uidValidity": 7, "highestModSeq": "9007199254740993"})
        );
        assert_eq!(
            serde_json::to_value(CursorUpdate::Phase { sync_phase: SyncPhase::Window }).unwrap(),
            json!({"kind": "phase", "syncPhase": "window"})
        );
    }

    #[test]
    fn status_detail_null_clears() {
        let args = SetAccountStatusArgs {
            account_id: "a".into(),
            worker_id: "w".into(),
            status_detail: Some(None),
            ..Default::default()
        };
        assert_eq!(
            serde_json::to_value(&args).unwrap(),
            json!({"accountId": "a", "workerId": "w", "statusDetail": null})
        );
        let args = SetAccountStatusArgs { account_id: "a".into(), worker_id: "w".into(), ..Default::default() };
        assert_eq!(serde_json::to_value(&args).unwrap(), json!({"accountId": "a", "workerId": "w"}));
    }

    #[test]
    fn record_draft_result_variants() {
        let r: RecordDraftResult = serde_json::from_value(json!({"conflict": true, "version": 3})).unwrap();
        assert_eq!(r, RecordDraftResult::Conflict { conflict: True, version: 3 });
        let r: RecordDraftResult = serde_json::from_value(json!({"draftId": "d", "version": 4})).unwrap();
        assert_eq!(r, RecordDraftResult::Saved { draft_id: "d".into(), version: 4 });
    }

    #[test]
    fn push_lookup_shapes() {
        assert_eq!(
            serde_json::to_value(AccountsForPushArgs::GraphSubscription { subscription_id: "s".into() }).unwrap(),
            json!({"by": "graph_subscription", "subscriptionId": "s"})
        );
    }
}
