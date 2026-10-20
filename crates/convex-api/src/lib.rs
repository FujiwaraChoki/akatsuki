//! # akatsuki-convex-api
//!
//! Typed bindings to the Convex functions Rust calls (ARCHITECTURE §4), on top of the official
//! `convex` crate.
//!
//! - [`WorkerApi`]: the worker service functions (`convex/worker.ts`, §4.3), used by the sync
//!   worker under its service identity.
//! - [`ClientApi`]: the client functions the desktop app calls (§4.2), under the user's JWT.
//! - Both are traits, implemented for any [`ConvexTransport`] through [`Convex`], so callers can be
//!   tested against [`MockTransport`] (or their own trait implementation) without a deployment.
//!
//! The argument and result types live in `akatsuki_core::convex`. §18 asks for these bindings to be
//! generated from `npx convex function-spec` by `cargo xtask convex-gen`; until `convex/` exists
//! they are written by hand to the contract, with the same names (DECISIONS D-092).

pub mod transport;
pub mod value;

use async_trait::async_trait;
use futures::StreamExt;
use futures::stream::BoxStream;
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value as Json;

use akatsuki_core::convex::client::{
    self as cf, AckFailureArgs, ClaimTriageArgs, DraftDiscardArgs, DraftDiscardResult, LabelCreateArgs,
    LabelCreateResult, LabelOpResult, LabelRemoveArgs, LabelRenameArgs, MailApplyArgs, MailApplyResult,
    MailSetReminderArgs, MailSetReminderResult, MailSnoozeArgs, MailSnoozeResult, MailUndoArgs, MailUndoResult, MeView,
    OpRetryArgs, OpRetryResult, RegisterDeviceArgs, SaveAiSettingsArgs, SearchThreadsArgs, SearchThreadsResult,
    SendsCancelArgs, SendsCancelResult, SendsListArgs, SendsRescheduleArgs, SendsResolveArgs, SetTriageArgs,
    SnippetIdArgs, SnippetSaveArgs, SnippetSaveResult, SnippetView, SplitView, SyncChangesArgs, SyncChangesResult,
    UpdatePrefsArgs,
};
use akatsuki_core::convex::rows::{AiSettings, LabelView, MailAccountView, OpFailure, ScheduledSendView, TriageInput};
use akatsuki_core::convex::worker::{
    self as wf, AccountWorkerArgs, AccountsForPushArgs, ApplyTriageArgs, AssignedAccount, ClaimAccountArgs,
    ClaimAccountResult, ClaimOpsArgs, ClaimedOp, CompleteOpsArgs, CreateAccountArgs, CreateAccountResult,
    GetSecretArgs, HeartbeatArgs, HeartbeatResult, MarkSendStartedArgs, PendingOpAccount, ProviderThreadIdRow,
    PushAccount, PutSecretArgs, PutSecretResult, RecordDraftArgs, RecordDraftResult, RecordReadReceiptArgs,
    ResolveMessagesArgs, ResolveThreadArgs, ResolvedMessage, ResolvedThread, ScheduleSendArgs, ScheduleSendResult,
    SecretRecord, SetAccountStatusArgs, SetCursorArgs, SetWatchArgs, ShardArgs, ThreadIdsByProviderArgs,
    UpsertIdentitiesArgs, UpsertLabelsArgs, UpsertLabelsResult, UpsertThreadsArgs, UpsertThreadsResult, UserContext,
    UserContextArgs, WorkerClaimTriageArgs,
};
use akatsuki_core::wire::ErrorCode;

pub use transport::{ConvexClientTransport, ConvexTransport, JsonStream, MockTransport, RecordedCall};

/// Errors from Convex calls.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum ConvexApiError {
    /// The connection or the client failed.
    #[error("convex transport: {0}")]
    Transport(String),
    /// The function threw a non-application error (a bug or a validator failure).
    #[error("{function} failed: {message}")]
    Function {
        /// Function name.
        function: String,
        /// Server message (redacted in production deployments).
        message: String,
    },
    /// The function threw `ConvexError({code, message, …})`.
    #[error("{function} returned {code:?}: {message}")]
    App {
        /// Function name.
        function: String,
        /// The registry code, when the data carried a known one.
        code: Option<ErrorCode>,
        /// Message.
        message: String,
        /// The full error data.
        data: Json,
    },
    /// Arguments could not be encoded.
    #[error("could not encode arguments: {0}")]
    Encode(String),
    /// The result did not match the expected type.
    #[error("unexpected result from {function}: {reason}")]
    Decode {
        /// Function name.
        function: String,
        /// Serde message.
        reason: String,
    },
}

impl ConvexApiError {
    /// Builds an [`ConvexApiError::App`] from `ConvexError` data.
    pub fn from_app_error(function: &str, message: String, data: Json) -> Self {
        let code = data.get("code").and_then(Json::as_str).and_then(ErrorCode::from_name);
        let message = data.get("message").and_then(Json::as_str).map(str::to_owned).unwrap_or(message);
        ConvexApiError::App { function: function.to_owned(), code, message, data }
    }

    /// The registry code, if this is an application error with one.
    pub fn code(&self) -> Option<ErrorCode> {
        match self {
            ConvexApiError::App { code, .. } => *code,
            _ => None,
        }
    }

    /// `true` when another worker owns the account (`lease_lost`); the actor must stop.
    pub fn is_lease_lost(&self) -> bool {
        self.code() == Some(ErrorCode::LeaseLost)
    }
}

/// A typed live query.
pub type Subscription<T> = BoxStream<'static, Result<T, ConvexApiError>>;

/// Typed calls over a transport.
#[derive(Debug, Clone)]
pub struct Convex<T: ConvexTransport> {
    transport: T,
}

impl<T: ConvexTransport> Convex<T> {
    /// Wraps a transport.
    pub fn new(transport: T) -> Self {
        Self { transport }
    }

    /// The transport.
    pub fn transport(&self) -> &T {
        &self.transport
    }

    /// Sets the JWT (user JWT for clients, service JWT for the worker).
    pub async fn set_auth(&self, token: Option<String>) {
        self.transport.set_auth(token).await;
    }

    fn encode<A: Serialize + ?Sized>(args: &A) -> Result<Json, ConvexApiError> {
        serde_json::to_value(args).map_err(|e| ConvexApiError::Encode(e.to_string()))
    }

    async fn query<A: Serialize + Sync + ?Sized, R: DeserializeOwned>(
        &self,
        f: &str,
        args: &A,
    ) -> Result<R, ConvexApiError> {
        let v = self.transport.query(f, Self::encode(args)?).await?;
        value::from_json(f, v)
    }

    async fn mutation<A: Serialize + Sync + ?Sized, R: DeserializeOwned>(
        &self,
        f: &str,
        args: &A,
    ) -> Result<R, ConvexApiError> {
        let v = self.transport.mutation(f, Self::encode(args)?).await?;
        value::from_json(f, v)
    }

    async fn subscribe<A: Serialize + Sync + ?Sized, R: DeserializeOwned + Send + 'static>(
        &self,
        f: &str,
        args: &A,
    ) -> Result<Subscription<R>, ConvexApiError> {
        let stream = self.transport.subscribe(f, Self::encode(args)?).await?;
        let name = f.to_owned();
        Ok(stream.map(move |r| r.and_then(|v| value::from_json(&name, v))).boxed())
    }
}

/// `null` results.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize)]
struct Unit;

fn unit(r: Result<Option<Unit>, ConvexApiError>) -> Result<(), ConvexApiError> {
    r.map(|_| ())
}

/// The worker service functions (§4.3).
#[async_trait]
pub trait WorkerApi: Send + Sync {
    /// `worker.assignedAccounts` (subscribed).
    async fn assigned_accounts(&self, shard: u32) -> Result<Subscription<Vec<AssignedAccount>>, ConvexApiError>;
    /// `worker.claimAccount`.
    async fn claim_account(&self, args: &ClaimAccountArgs) -> Result<ClaimAccountResult, ConvexApiError>;
    /// `worker.heartbeat`.
    async fn heartbeat(&self, args: &HeartbeatArgs) -> Result<HeartbeatResult, ConvexApiError>;
    /// `worker.releaseAccount`.
    async fn release_account(&self, args: &AccountWorkerArgs) -> Result<(), ConvexApiError>;
    /// `worker.getSecret`.
    async fn get_secret(&self, args: &GetSecretArgs) -> Result<Option<SecretRecord>, ConvexApiError>;
    /// `worker.putSecret`.
    async fn put_secret(&self, args: &PutSecretArgs) -> Result<PutSecretResult, ConvexApiError>;
    /// `worker.createAccount`.
    async fn create_account(&self, args: &CreateAccountArgs) -> Result<CreateAccountResult, ConvexApiError>;
    /// `worker.setAccountStatus`.
    async fn set_account_status(&self, args: &SetAccountStatusArgs) -> Result<(), ConvexApiError>;
    /// `worker.upsertIdentities`.
    async fn upsert_identities(&self, args: &UpsertIdentitiesArgs) -> Result<(), ConvexApiError>;
    /// `worker.upsertLabels`.
    async fn upsert_labels(&self, args: &UpsertLabelsArgs) -> Result<UpsertLabelsResult, ConvexApiError>;
    /// `worker.upsertThreads`.
    async fn upsert_threads(&self, args: &UpsertThreadsArgs) -> Result<UpsertThreadsResult, ConvexApiError>;
    /// `worker.setCursor`.
    async fn set_cursor(&self, args: &SetCursorArgs) -> Result<(), ConvexApiError>;
    /// `worker.setWatch`.
    async fn set_watch(&self, args: &SetWatchArgs) -> Result<(), ConvexApiError>;
    /// `worker.accountsForPush`.
    async fn accounts_for_push(&self, args: &AccountsForPushArgs) -> Result<Vec<PushAccount>, ConvexApiError>;
    /// `worker.pendingOpAccounts` (subscribed).
    async fn pending_op_accounts(&self, shard: u32) -> Result<Subscription<Vec<PendingOpAccount>>, ConvexApiError>;
    /// `worker.claimOps`.
    async fn claim_ops(&self, args: &ClaimOpsArgs) -> Result<Vec<ClaimedOp>, ConvexApiError>;
    /// `worker.markSendStarted`.
    async fn mark_send_started(&self, args: &MarkSendStartedArgs) -> Result<(), ConvexApiError>;
    /// `worker.completeOps`.
    async fn complete_ops(&self, args: &CompleteOpsArgs) -> Result<(), ConvexApiError>;
    /// `worker.recordDraft`.
    async fn record_draft(&self, args: &RecordDraftArgs) -> Result<RecordDraftResult, ConvexApiError>;
    /// `worker.scheduleSend`.
    async fn schedule_send(&self, args: &ScheduleSendArgs) -> Result<ScheduleSendResult, ConvexApiError>;
    /// `worker.resolveMessages`.
    async fn resolve_messages(&self, args: &ResolveMessagesArgs) -> Result<Vec<ResolvedMessage>, ConvexApiError>;
    /// `worker.resolveThread`.
    async fn resolve_thread(&self, args: &ResolveThreadArgs) -> Result<Option<ResolvedThread>, ConvexApiError>;
    /// `worker.userContext`.
    async fn user_context(&self, args: &UserContextArgs) -> Result<UserContext, ConvexApiError>;
    /// `worker.claimTriage` (managed triage, DECISIONS D-390).
    async fn claim_managed_triage(&self, args: &WorkerClaimTriageArgs) -> Result<Vec<TriageInput>, ConvexApiError>;
    /// `worker.applyTriage`.
    async fn apply_triage(&self, args: &ApplyTriageArgs) -> Result<(), ConvexApiError>;
    /// `worker.recordReadReceipt`.
    async fn record_read_receipt(&self, args: &RecordReadReceiptArgs) -> Result<(), ConvexApiError>;
    /// `worker.ackAccountRemoved`.
    async fn ack_account_removed(&self, args: &AccountWorkerArgs) -> Result<(), ConvexApiError>;
    /// `worker.threadIdsByProvider` (DECISIONS D-236).
    async fn thread_ids_by_provider(
        &self,
        args: &ThreadIdsByProviderArgs,
    ) -> Result<Vec<ProviderThreadIdRow>, ConvexApiError>;
}

#[async_trait]
impl<T: ConvexTransport> WorkerApi for Convex<T> {
    async fn assigned_accounts(&self, shard: u32) -> Result<Subscription<Vec<AssignedAccount>>, ConvexApiError> {
        self.subscribe(wf::functions::ASSIGNED_ACCOUNTS, &ShardArgs { shard }).await
    }
    async fn claim_account(&self, args: &ClaimAccountArgs) -> Result<ClaimAccountResult, ConvexApiError> {
        self.mutation(wf::functions::CLAIM_ACCOUNT, args).await
    }
    async fn heartbeat(&self, args: &HeartbeatArgs) -> Result<HeartbeatResult, ConvexApiError> {
        self.mutation(wf::functions::HEARTBEAT, args).await
    }
    async fn release_account(&self, args: &AccountWorkerArgs) -> Result<(), ConvexApiError> {
        unit(self.mutation(wf::functions::RELEASE_ACCOUNT, args).await)
    }
    async fn get_secret(&self, args: &GetSecretArgs) -> Result<Option<SecretRecord>, ConvexApiError> {
        self.query(wf::functions::GET_SECRET, args).await
    }
    async fn put_secret(&self, args: &PutSecretArgs) -> Result<PutSecretResult, ConvexApiError> {
        self.mutation(wf::functions::PUT_SECRET, args).await
    }
    async fn create_account(&self, args: &CreateAccountArgs) -> Result<CreateAccountResult, ConvexApiError> {
        self.mutation(wf::functions::CREATE_ACCOUNT, args).await
    }
    async fn set_account_status(&self, args: &SetAccountStatusArgs) -> Result<(), ConvexApiError> {
        unit(self.mutation(wf::functions::SET_ACCOUNT_STATUS, args).await)
    }
    async fn upsert_identities(&self, args: &UpsertIdentitiesArgs) -> Result<(), ConvexApiError> {
        unit(self.mutation(wf::functions::UPSERT_IDENTITIES, args).await)
    }
    async fn upsert_labels(&self, args: &UpsertLabelsArgs) -> Result<UpsertLabelsResult, ConvexApiError> {
        self.mutation(wf::functions::UPSERT_LABELS, args).await
    }
    async fn upsert_threads(&self, args: &UpsertThreadsArgs) -> Result<UpsertThreadsResult, ConvexApiError> {
        self.mutation(wf::functions::UPSERT_THREADS, args).await
    }
    async fn set_cursor(&self, args: &SetCursorArgs) -> Result<(), ConvexApiError> {
        unit(self.mutation(wf::functions::SET_CURSOR, args).await)
    }
    async fn set_watch(&self, args: &SetWatchArgs) -> Result<(), ConvexApiError> {
        unit(self.mutation(wf::functions::SET_WATCH, args).await)
    }
    async fn accounts_for_push(&self, args: &AccountsForPushArgs) -> Result<Vec<PushAccount>, ConvexApiError> {
        self.query(wf::functions::ACCOUNTS_FOR_PUSH, args).await
    }
    async fn pending_op_accounts(&self, shard: u32) -> Result<Subscription<Vec<PendingOpAccount>>, ConvexApiError> {
        self.subscribe(wf::functions::PENDING_OP_ACCOUNTS, &ShardArgs { shard }).await
    }
    async fn claim_ops(&self, args: &ClaimOpsArgs) -> Result<Vec<ClaimedOp>, ConvexApiError> {
        self.mutation(wf::functions::CLAIM_OPS, args).await
    }
    async fn mark_send_started(&self, args: &MarkSendStartedArgs) -> Result<(), ConvexApiError> {
        unit(self.mutation(wf::functions::MARK_SEND_STARTED, args).await)
    }
    async fn complete_ops(&self, args: &CompleteOpsArgs) -> Result<(), ConvexApiError> {
        unit(self.mutation(wf::functions::COMPLETE_OPS, args).await)
    }
    async fn record_draft(&self, args: &RecordDraftArgs) -> Result<RecordDraftResult, ConvexApiError> {
        self.mutation(wf::functions::RECORD_DRAFT, args).await
    }
    async fn schedule_send(&self, args: &ScheduleSendArgs) -> Result<ScheduleSendResult, ConvexApiError> {
        self.mutation(wf::functions::SCHEDULE_SEND, args).await
    }
    async fn resolve_messages(&self, args: &ResolveMessagesArgs) -> Result<Vec<ResolvedMessage>, ConvexApiError> {
        self.query(wf::functions::RESOLVE_MESSAGES, args).await
    }
    async fn resolve_thread(&self, args: &ResolveThreadArgs) -> Result<Option<ResolvedThread>, ConvexApiError> {
        self.query(wf::functions::RESOLVE_THREAD, args).await
    }
    async fn user_context(&self, args: &UserContextArgs) -> Result<UserContext, ConvexApiError> {
        self.query(wf::functions::USER_CONTEXT, args).await
    }
    async fn claim_managed_triage(&self, args: &WorkerClaimTriageArgs) -> Result<Vec<TriageInput>, ConvexApiError> {
        self.mutation(wf::functions::CLAIM_TRIAGE, args).await
    }
    async fn apply_triage(&self, args: &ApplyTriageArgs) -> Result<(), ConvexApiError> {
        unit(self.mutation(wf::functions::APPLY_TRIAGE, args).await)
    }
    async fn record_read_receipt(&self, args: &RecordReadReceiptArgs) -> Result<(), ConvexApiError> {
        unit(self.mutation(wf::functions::RECORD_READ_RECEIPT, args).await)
    }
    async fn ack_account_removed(&self, args: &AccountWorkerArgs) -> Result<(), ConvexApiError> {
        unit(self.mutation(wf::functions::ACK_ACCOUNT_REMOVED, args).await)
    }
    async fn thread_ids_by_provider(
        &self,
        args: &ThreadIdsByProviderArgs,
    ) -> Result<Vec<ProviderThreadIdRow>, ConvexApiError> {
        self.query(wf::functions::THREAD_IDS_BY_PROVIDER, args).await
    }
}

/// Empty arguments.
#[derive(Debug, Clone, Copy, Serialize)]
struct NoArgs {}

/// The client functions the desktop app calls (§4.2).
#[async_trait]
pub trait ClientApi: Send + Sync {
    /// `devices.register`.
    async fn register_device(&self, args: &RegisterDeviceArgs) -> Result<(), ConvexApiError>;
    /// `accounts.list`.
    async fn list_accounts(&self) -> Result<Vec<MailAccountView>, ConvexApiError>;
    /// `labels.list` (all accounts).
    async fn list_labels(&self) -> Result<Vec<LabelView>, ConvexApiError>;
    /// `mail.apply`.
    async fn mail_apply(&self, args: &MailApplyArgs) -> Result<MailApplyResult, ConvexApiError>;
    /// `mail.undo`.
    async fn mail_undo(&self, args: &MailUndoArgs) -> Result<MailUndoResult, ConvexApiError>;
    /// `ops.failures` (subscribed).
    async fn op_failures(&self) -> Result<Subscription<Vec<OpFailure>>, ConvexApiError>;
    /// `ops.ackFailure`.
    async fn ack_failures(&self, args: &AckFailureArgs) -> Result<(), ConvexApiError>;
    /// `ai.getSettings`: `None` until the user's `aiSettings` row exists (the function returns
    /// `null`, §4.2 and D-057).
    async fn ai_settings(&self) -> Result<Option<AiSettings>, ConvexApiError>;
    /// `ai.saveSettings`.
    async fn save_ai_settings(&self, args: &SaveAiSettingsArgs) -> Result<(), ConvexApiError>;
    /// `splits.list`.
    async fn list_splits(&self) -> Result<Vec<SplitView>, ConvexApiError>;
    /// `mail.snooze`.
    async fn mail_snooze(&self, args: &MailSnoozeArgs) -> Result<MailSnoozeResult, ConvexApiError>;
    /// `sends.cancel` (undo send, §10.3).
    async fn sends_cancel(&self, args: &SendsCancelArgs) -> Result<SendsCancelResult, ConvexApiError>;
    /// `mail.setReminder`.
    async fn set_reminder(&self, args: &MailSetReminderArgs) -> Result<MailSetReminderResult, ConvexApiError>;
    /// `ai.claimTriage`.
    async fn claim_triage(&self, args: &ClaimTriageArgs) -> Result<Vec<TriageInput>, ConvexApiError>;
    /// `ai.setTriage`.
    async fn set_triage(&self, args: &SetTriageArgs) -> Result<(), ConvexApiError>;
    /// `sync.changes` (subscribed; re-subscribe with `upToRev`).
    async fn sync_changes(&self, args: &SyncChangesArgs) -> Result<Subscription<SyncChangesResult>, ConvexApiError>;
    /// `me.get`: `None` until the user's rows exist.
    async fn me(&self) -> Result<Option<MeView>, ConvexApiError>;
    /// `me.updatePrefs`.
    async fn update_prefs(&self, args: &UpdatePrefsArgs) -> Result<(), ConvexApiError>;
    /// `labels.create`.
    async fn create_label(&self, args: &LabelCreateArgs) -> Result<LabelCreateResult, ConvexApiError>;
    /// `labels.rename`.
    async fn rename_label(&self, args: &LabelRenameArgs) -> Result<LabelOpResult, ConvexApiError>;
    /// `labels.remove` (sets `pendingDelete`).
    async fn remove_label(&self, args: &LabelRemoveArgs) -> Result<LabelOpResult, ConvexApiError>;
    /// `snippets.list`.
    async fn list_snippets(&self) -> Result<Vec<SnippetView>, ConvexApiError>;
    /// `snippets.save`.
    async fn save_snippet(&self, args: &SnippetSaveArgs) -> Result<SnippetSaveResult, ConvexApiError>;
    /// `snippets.remove`.
    async fn remove_snippet(&self, args: &SnippetIdArgs) -> Result<(), ConvexApiError>;
    /// `snippets.markUsed`.
    async fn mark_snippet_used(&self, args: &SnippetIdArgs) -> Result<(), ConvexApiError>;
    /// `ops.retry`.
    async fn retry_op(&self, args: &OpRetryArgs) -> Result<OpRetryResult, ConvexApiError>;
    /// `search.threads`.
    async fn search_threads(&self, args: &SearchThreadsArgs) -> Result<SearchThreadsResult, ConvexApiError>;
    /// `sends.list`.
    async fn list_sends(&self, args: &SendsListArgs) -> Result<Vec<ScheduledSendView>, ConvexApiError>;
    /// `sends.reschedule`.
    async fn reschedule_send(&self, args: &SendsRescheduleArgs) -> Result<(), ConvexApiError>;
    /// `sends.resolveUnknown`.
    async fn resolve_unknown_send(&self, args: &SendsResolveArgs) -> Result<(), ConvexApiError>;
    /// `drafts.discard`.
    async fn discard_draft(&self, args: &DraftDiscardArgs) -> Result<DraftDiscardResult, ConvexApiError>;
}

#[async_trait]
impl<T: ConvexTransport> ClientApi for Convex<T> {
    async fn register_device(&self, args: &RegisterDeviceArgs) -> Result<(), ConvexApiError> {
        unit(self.mutation(cf::functions::DEVICES_REGISTER, args).await)
    }
    async fn list_accounts(&self) -> Result<Vec<MailAccountView>, ConvexApiError> {
        self.query(cf::functions::ACCOUNTS_LIST, &NoArgs {}).await
    }
    async fn list_labels(&self) -> Result<Vec<LabelView>, ConvexApiError> {
        self.query(cf::functions::LABELS_LIST, &NoArgs {}).await
    }
    async fn mail_apply(&self, args: &MailApplyArgs) -> Result<MailApplyResult, ConvexApiError> {
        self.mutation(cf::functions::MAIL_APPLY, args).await
    }
    async fn mail_undo(&self, args: &MailUndoArgs) -> Result<MailUndoResult, ConvexApiError> {
        self.mutation(cf::functions::MAIL_UNDO, args).await
    }
    async fn op_failures(&self) -> Result<Subscription<Vec<OpFailure>>, ConvexApiError> {
        self.subscribe(cf::functions::OPS_FAILURES, &NoArgs {}).await
    }
    async fn ack_failures(&self, args: &AckFailureArgs) -> Result<(), ConvexApiError> {
        unit(self.mutation(cf::functions::OPS_ACK_FAILURE, args).await)
    }
    async fn ai_settings(&self) -> Result<Option<AiSettings>, ConvexApiError> {
        self.query(cf::functions::AI_GET_SETTINGS, &NoArgs {}).await
    }
    async fn save_ai_settings(&self, args: &SaveAiSettingsArgs) -> Result<(), ConvexApiError> {
        unit(self.mutation(cf::functions::AI_SAVE_SETTINGS, args).await)
    }
    async fn list_splits(&self) -> Result<Vec<SplitView>, ConvexApiError> {
        self.query(cf::functions::SPLITS_LIST, &NoArgs {}).await
    }
    async fn mail_snooze(&self, args: &MailSnoozeArgs) -> Result<MailSnoozeResult, ConvexApiError> {
        self.mutation(cf::functions::MAIL_SNOOZE, args).await
    }
    async fn sends_cancel(&self, args: &SendsCancelArgs) -> Result<SendsCancelResult, ConvexApiError> {
        self.mutation(cf::functions::SENDS_CANCEL, args).await
    }
    async fn set_reminder(&self, args: &MailSetReminderArgs) -> Result<MailSetReminderResult, ConvexApiError> {
        self.mutation(cf::functions::MAIL_SET_REMINDER, args).await
    }
    async fn claim_triage(&self, args: &ClaimTriageArgs) -> Result<Vec<TriageInput>, ConvexApiError> {
        self.mutation(cf::functions::AI_CLAIM_TRIAGE, args).await
    }
    async fn set_triage(&self, args: &SetTriageArgs) -> Result<(), ConvexApiError> {
        unit(self.mutation(cf::functions::AI_SET_TRIAGE, args).await)
    }
    async fn sync_changes(&self, args: &SyncChangesArgs) -> Result<Subscription<SyncChangesResult>, ConvexApiError> {
        self.subscribe(cf::functions::SYNC_CHANGES, args).await
    }
    async fn me(&self) -> Result<Option<MeView>, ConvexApiError> {
        self.query(cf::functions::ME_GET, &NoArgs {}).await
    }
    async fn update_prefs(&self, args: &UpdatePrefsArgs) -> Result<(), ConvexApiError> {
        unit(self.mutation(cf::functions::ME_UPDATE_PREFS, args).await)
    }
    async fn create_label(&self, args: &LabelCreateArgs) -> Result<LabelCreateResult, ConvexApiError> {
        self.mutation(cf::functions::LABELS_CREATE, args).await
    }
    async fn rename_label(&self, args: &LabelRenameArgs) -> Result<LabelOpResult, ConvexApiError> {
        self.mutation(cf::functions::LABELS_RENAME, args).await
    }
    async fn remove_label(&self, args: &LabelRemoveArgs) -> Result<LabelOpResult, ConvexApiError> {
        self.mutation(cf::functions::LABELS_REMOVE, args).await
    }
    async fn list_snippets(&self) -> Result<Vec<SnippetView>, ConvexApiError> {
        self.query(cf::functions::SNIPPETS_LIST, &NoArgs {}).await
    }
    async fn save_snippet(&self, args: &SnippetSaveArgs) -> Result<SnippetSaveResult, ConvexApiError> {
        self.mutation(cf::functions::SNIPPETS_SAVE, args).await
    }
    async fn remove_snippet(&self, args: &SnippetIdArgs) -> Result<(), ConvexApiError> {
        unit(self.mutation(cf::functions::SNIPPETS_REMOVE, args).await)
    }
    async fn mark_snippet_used(&self, args: &SnippetIdArgs) -> Result<(), ConvexApiError> {
        unit(self.mutation(cf::functions::SNIPPETS_MARK_USED, args).await)
    }
    async fn retry_op(&self, args: &OpRetryArgs) -> Result<OpRetryResult, ConvexApiError> {
        self.mutation(cf::functions::OPS_RETRY, args).await
    }
    async fn search_threads(&self, args: &SearchThreadsArgs) -> Result<SearchThreadsResult, ConvexApiError> {
        self.query(cf::functions::SEARCH_THREADS, args).await
    }
    async fn list_sends(&self, args: &SendsListArgs) -> Result<Vec<ScheduledSendView>, ConvexApiError> {
        self.query(cf::functions::SENDS_LIST, args).await
    }
    async fn reschedule_send(&self, args: &SendsRescheduleArgs) -> Result<(), ConvexApiError> {
        unit(self.mutation(cf::functions::SENDS_RESCHEDULE, args).await)
    }
    async fn resolve_unknown_send(&self, args: &SendsResolveArgs) -> Result<(), ConvexApiError> {
        unit(self.mutation(cf::functions::SENDS_RESOLVE_UNKNOWN, args).await)
    }
    async fn discard_draft(&self, args: &DraftDiscardArgs) -> Result<DraftDiscardResult, ConvexApiError> {
        self.mutation(cf::functions::DRAFTS_DISCARD, args).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use akatsuki_core::convex::values::ConvexInt64;
    use akatsuki_core::convex::worker::{False, MessageUpsert, ThreadUpsert};
    use serde_json::json;

    fn api() -> Convex<MockTransport> {
        Convex::new(MockTransport::new())
    }

    #[tokio::test]
    async fn worker_calls_use_contract_names_and_shapes() {
        let c = api();
        c.transport().respond("worker:claimAccount", Ok(json!({"ok": false, "owner": "w2"})));
        let r = c
            .claim_account(&ClaimAccountArgs { account_id: "a1".into(), worker_id: "w1".into(), lease_ms: 60_000 })
            .await
            .unwrap();
        assert_eq!(r, ClaimAccountResult::Taken { ok: False, owner: "w2".into() });

        c.transport().respond(
            "worker:upsertThreads",
            Ok(json!({"written": 1, "skippedStale": 0, "newInbound": [{"threadId": "t1", "isInbox": true}]})),
        );
        let r = c
            .upsert_threads(&UpsertThreadsArgs {
                account_id: "a1".into(),
                worker_id: "w1".into(),
                threads: vec![ThreadUpsert {
                    provider_thread_id: "pt".into(),
                    provider_version: Some(ConvexInt64(1 << 60)),
                    complete: true,
                    messages: vec![MessageUpsert { provider_message_id: "pm".into(), ..Default::default() }],
                    removed_provider_message_ids: None,
                }],
                deletes: None,
                deleted_messages: None,
                cursor: None,
            })
            .await
            .unwrap();
        assert_eq!(r.written, 1);
        assert!(r.new_inbound[0].is_inbox);

        c.transport().respond("worker:releaseAccount", Ok(Json::Null));
        c.release_account(&AccountWorkerArgs { account_id: "a1".into(), worker_id: "w1".into() }).await.unwrap();

        let calls = c.transport().calls();
        assert_eq!(calls[0].function, "worker:claimAccount");
        assert_eq!(calls[0].kind, "mutation");
        assert_eq!(calls[0].args, json!({"accountId": "a1", "workerId": "w1", "leaseMs": 60000}));
        assert_eq!(calls[1].args["threads"][0]["providerVersion"], json!({"$integer": "AAAAAAAAABA="}));
        assert!(calls[1].args.get("deletes").is_none(), "optional args are omitted, not null");
    }

    #[tokio::test]
    async fn app_errors_carry_registry_codes() {
        let c = api();
        c.transport().respond(
            "worker:heartbeat",
            Err(ConvexApiError::from_app_error(
                "worker:heartbeat",
                "redacted".into(),
                json!({"code": "lease_lost", "message": "Lease lost"}),
            )),
        );
        let err =
            c.heartbeat(&HeartbeatArgs { worker_id: "w".into(), account_ids: vec![], lease_ms: 1 }).await.unwrap_err();
        assert!(err.is_lease_lost());
        assert_eq!(err.code(), Some(ErrorCode::LeaseLost));
    }

    #[tokio::test]
    async fn decode_errors_name_the_function() {
        let c = api();
        c.transport().respond("worker:userContext", Ok(json!({"unexpected": true})));
        let err = c.user_context(&UserContextArgs { user_id: "u".into() }).await.unwrap_err();
        assert!(matches!(err, ConvexApiError::Decode { ref function, .. } if function == "worker:userContext"));
    }

    #[tokio::test]
    async fn subscriptions_yield_typed_items() {
        let c = api();
        c.transport().respond("worker:pendingOpAccounts", Ok(json!([{"accountId": "a1", "minAvailableAt": 5}])));
        c.transport().respond("worker:pendingOpAccounts", Ok(json!([])));
        let mut sub = c.pending_op_accounts(0).await.unwrap();
        assert_eq!(sub.next().await.unwrap().unwrap()[0].min_available_at, 5);
        assert!(sub.next().await.unwrap().unwrap().is_empty());
        assert!(sub.next().await.is_none());
        assert_eq!(c.transport().calls()[0].args, json!({"shard": 0}));
    }

    #[tokio::test]
    async fn client_calls() {
        let c = api();
        c.set_auth(Some("jwt".into())).await;
        assert_eq!(c.transport().auth().as_deref(), Some("jwt"));
        c.transport().respond("mail:apply", Ok(json!({"opIds": ["o1"], "rev": {"a1": 7}})));
        let r = c
            .mail_apply(&MailApplyArgs {
                client_op_id: "c".into(),
                thread_ids: vec!["t1".into()],
                action: akatsuki_core::convex::ThreadAction::Trash,
            })
            .await
            .unwrap();
        assert_eq!(r.rev["a1"], 7);
        assert_eq!(c.transport().calls()[0].args["action"], json!({"kind": "trash"}));
        c.transport().respond("accounts:list", Ok(json!([])));
        assert!(c.list_accounts().await.unwrap().is_empty());
        assert_eq!(c.transport().calls()[1].args, json!({}));
    }

    #[tokio::test]
    async fn ai_settings_decodes_null_as_none() {
        let c = api();
        c.transport().respond("ai:getSettings", Ok(Json::Null));
        assert_eq!(c.ai_settings().await.unwrap(), None);
        c.transport().respond(
            "ai:getSettings",
            Ok(json!({"_id": "s1", "_creationTime": 1.0, "profiles": [], "routes": [],
                "consent": {"autoTriage": false, "syncVoiceProfile": false}, "updatedAt": 5})),
        );
        let s = c.ai_settings().await.unwrap().unwrap();
        assert_eq!(s.updated_at, 5);
    }

    #[tokio::test]
    async fn desktop_extras_use_contract_names() {
        let c = api();
        c.transport().respond("sends:cancel", Ok(json!({"result": "too_late"})));
        let r = c.sends_cancel(&SendsCancelArgs { client_send_id: "s".into() }).await.unwrap();
        assert_eq!(r.result, akatsuki_core::convex::client::CancelOutcome::TooLate);
        c.transport().respond("ai:saveSettings", Ok(Json::Null));
        c.save_ai_settings(&SaveAiSettingsArgs::default()).await.unwrap();
        assert_eq!(c.transport().calls()[1].args, json!({}), "absent fields are omitted");
        c.transport().respond("mail:snooze", Ok(json!({"timerIds": ["tm1"]})));
        let r = c
            .mail_snooze(&MailSnoozeArgs {
                client_op_id: "c".into(),
                thread_ids: vec!["t1".into()],
                until: 9,
                return_unread: false,
            })
            .await
            .unwrap();
        assert_eq!(r.timer_ids.len(), 1);
        c.transport().respond("splits:list", Ok(json!([])));
        assert!(c.list_splits().await.unwrap().is_empty());
        c.transport().respond("mail:setReminder", Ok(json!({"timerId": "tm2"})));
        let r = c
            .set_reminder(&MailSetReminderArgs {
                thread_id: "t1".into(),
                at: 10,
                condition: akatsuki_core::convex::TimerCondition::IfNoReply,
                note: None,
            })
            .await
            .unwrap();
        assert_eq!(r.timer_id.as_str(), "tm2");
        let last = c.transport().calls().pop().unwrap();
        assert_eq!(last.args, json!({"threadId": "t1", "at": 10, "condition": "if_no_reply"}));
    }

    #[tokio::test]
    async fn desktop_feature_calls_use_contract_names_and_shapes() {
        use akatsuki_core::convex::client::{
            DraftDiscardArgs, LabelCreateArgs, LabelRenameArgs, OpRetryArgs, PaginationOpts, PrefsPatch,
            SearchThreadsArgs, SendsListArgs, SendsRescheduleArgs, SnippetIdArgs, SnippetSaveArgs, UpdatePrefsArgs,
        };
        let c = api();
        let last = |c: &Convex<MockTransport>| c.transport().calls().pop().unwrap();

        c.transport().respond("me:get", Ok(Json::Null));
        assert_eq!(c.me().await.unwrap(), None, "null before the user's rows exist");
        c.transport().respond(
            "me:get",
            Ok(json!({"userId": "u1", "email": "a@b.example", "profile": {"_id": "p1", "_creationTime": 1.0,
                "email": "a@b.example", "timezone": "Europe/Zurich", "onboarding": "done", "createdAt": 1, "updatedAt": 2,
                "prefs": {"undoSendSeconds": 5, "remoteImages": "block", "readReceipts": {"enabled": false},
                    "cacheBodiesOnDevice": true, "density": "compact", "theme": "dark", "keymap": "gmail",
                    "unifiedInbox": true, "notifications": "private"}},
                "entitlement": {"state": "active", "hostedSync": true, "managedAi": true, "until": 9},
                "billing": {"status": "active", "cancelAtPeriodEnd": false}})),
        );
        let me = c.me().await.unwrap().unwrap();
        assert_eq!(me.profile.timezone, "Europe/Zurich");
        assert_eq!(me.profile.prefs.undo_send_seconds, 5);
        assert!(me.entitlement.managed_ai);

        c.transport().respond("me:updatePrefs", Ok(Json::Null));
        let patch = PrefsPatch { theme: Some(akatsuki_core::convex::ThemePref::Dark), ..PrefsPatch::default() };
        c.update_prefs(&UpdatePrefsArgs { patch }).await.unwrap();
        assert_eq!(last(&c).args, json!({"patch": {"theme": "dark"}}), "only changed fields");

        c.transport().respond("labels:create", Ok(json!({"labelId": "l1", "labelKey": "l:l1", "opId": "o1"})));
        let r = c
            .create_label(&LabelCreateArgs {
                client_op_id: "c1".into(),
                account_id: "a1".into(),
                name: "Taxes".into(),
                color: None,
            })
            .await
            .unwrap();
        assert_eq!(r.label_key, "l:l1");
        assert_eq!(last(&c).args, json!({"clientOpId": "c1", "accountId": "a1", "name": "Taxes"}));
        c.transport().respond("labels:rename", Ok(json!({"opId": "o2"})));
        c.rename_label(&LabelRenameArgs { client_op_id: "c2".into(), label_id: "l1".into(), name: "Tax".into() })
            .await
            .unwrap();
        assert_eq!(last(&c).function, "labels:rename");

        c.transport().respond(
            "snippets:list",
            Ok(json!([{"_id": "s1", "_creationTime": 1.0, "name": "Thanks", "shortcut": "thx",
                "bodyHtml": "<p>Thanks, {{first_name}}!</p>", "variables": ["first_name"], "useCount": 3, "updatedAt": 4}])),
        );
        let s = c.list_snippets().await.unwrap();
        assert_eq!(s[0].shortcut.as_deref(), Some("thx"));
        c.transport().respond("snippets:save", Ok(json!({"snippetId": "s2"})));
        c.save_snippet(&SnippetSaveArgs {
            snippet_id: None,
            name: "Hi".into(),
            shortcut: None,
            subject: None,
            body_html: "<p>Hi</p>".into(),
            variables: vec![],
        })
        .await
        .unwrap();
        assert_eq!(last(&c).args, json!({"name": "Hi", "bodyHtml": "<p>Hi</p>", "variables": []}));
        c.transport().respond("snippets:markUsed", Ok(Json::Null));
        c.mark_snippet_used(&SnippetIdArgs { snippet_id: "s1".into() }).await.unwrap();

        c.transport().respond("ops:retry", Ok(json!({"opIds": ["o3"]})));
        let r = c.retry_op(&OpRetryArgs { op_id: "o1".into(), client_op_id: "c3".into() }).await.unwrap();
        assert_eq!(r.op_ids.len(), 1);

        c.transport().respond(
            "search:threads",
            Ok(json!({"page": [], "isDone": true, "continueCursor": "", "unsupported": ["filename:"], "ast": {"and": []}})),
        );
        let r = c
            .search_threads(&SearchThreadsArgs {
                q: "filename:plan".into(),
                now_at: 60_000,
                account_id: None,
                pagination_opts: PaginationOpts { num_items: 50, cursor: None },
            })
            .await
            .unwrap();
        assert_eq!(r.unsupported, vec!["filename:"]);
        assert_eq!(
            last(&c).args,
            json!({"q": "filename:plan", "nowAt": 60_000, "paginationOpts": {"numItems": 50, "cursor": null}})
        );

        c.transport().respond("sends:list", Ok(json!([])));
        assert!(c.list_sends(&SendsListArgs::default()).await.unwrap().is_empty());
        c.transport().respond("sends:reschedule", Ok(Json::Null));
        c.reschedule_send(&SendsRescheduleArgs { client_send_id: "cs".into(), send_at: 99 }).await.unwrap();
        assert_eq!(last(&c).args, json!({"clientSendId": "cs", "sendAt": 99}));
        c.transport().respond("drafts:discard", Ok(json!({"opId": "o4"})));
        c.discard_draft(&DraftDiscardArgs { client_op_id: "c4".into(), client_draft_id: "d1".into() }).await.unwrap();
        assert_eq!(last(&c).args, json!({"clientOpId": "c4", "clientDraftId": "d1"}));
    }
}

/// The bindings are hand-written (DECISIONS D-092, D-396): every function name they call must be an
/// exported Convex function, so a rename or removal in `convex/` fails here instead of at run time.
#[cfg(test)]
mod name_check {
    use std::path::Path;

    #[test]
    fn every_bound_function_is_exported_by_convex() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let mut names = Vec::new();
        for file in ["crates/core/src/convex/worker.rs", "crates/core/src/convex/client.rs"] {
            let src = std::fs::read_to_string(root.join(file)).unwrap();
            for line in src.lines().filter(|l| l.trim_start().starts_with("pub const ") && l.contains(": &str = \"")) {
                let lit = line.split('"').nth(1).unwrap();
                if let Some((module, name)) = lit.split_once(':') {
                    names.push((module.to_owned(), name.to_owned()));
                }
            }
        }
        assert!(names.len() >= 40, "function constants found: {}", names.len());
        for (module, name) in names {
            let path = root.join("convex").join(format!("{module}.ts"));
            let src = std::fs::read_to_string(&path).unwrap_or_else(|_| panic!("convex/{module}.ts is missing"));
            assert!(
                src.contains(&format!("export const {name} = ")),
                "{module}:{name} is not exported by convex/{module}.ts"
            );
        }
    }
}
