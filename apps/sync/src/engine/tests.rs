//! Engine tests: the account actor against `FakeProvider` and an in-memory `WorkerApi` that
//! follows the ops-outbox rules of `convex/worker.ts` closely enough to check claim/complete
//! ordering, retries, poison and the at-most-once send protocol.

use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use akatsuki_convex_api::{ConvexApiError, Subscription, WorkerApi};
use akatsuki_core::convex::rows::SyncStateDoc;
use akatsuki_core::convex::schema::{AppError, OpPayload, OpState, ThreadAction};
use akatsuki_core::convex::worker::*;
use akatsuki_core::convex::{AccountCaps, AccountStatus, CursorUpdate, Provider, SendKind, SyncPhase, WatchKind};
use akatsuki_core::ids::ThreadId;
use akatsuki_core::sync::{AccountPhase, ActorCmd, PhaseEvent, WorkKind};
use akatsuki_providers::{ChangePage, Cursor, FetchedThread, ProviderError, ProviderOp, RemoteThreadId, SysLabel};
use async_trait::async_trait;
use serde_json::json;
use tokio::sync::mpsc;

use super::actor::{AccountActor, ActorParts, ClaimState, ExitReason};
use super::fake::{FakeProvider, message};
use super::sink::ConvexSink;
use super::{ActorMsg, EngineConfig, ProjectGate};
use crate::fetch::SafeFetcher;
use crate::metrics::Metrics;

// ───────────── an in-memory WorkerApi ─────────────

#[derive(Clone, Debug)]
struct FakeOp {
    claimed: ClaimedOp,
    state: OpState,
    available_at: i64,
    attempt: u32,
    max_attempts: u32,
    verify_only: bool,
    send_started: bool,
    send_started_at: Option<i64>,
    last_error: Option<AppError>,
}

#[derive(Default)]
struct ConvexState {
    now: i64,
    ops: Vec<FakeOp>,
    upserts: Vec<UpsertThreadsArgs>,
    cursors: Vec<CursorUpdate>,
    statuses: Vec<SetAccountStatusArgs>,
    completes: Vec<CompleteOpsArgs>,
    labels_calls: Vec<UpsertLabelsArgs>,
    watches: Vec<SetWatchArgs>,
    marks: Vec<String>,
    released: u32,
    lease_lost: bool,
    mark_lease_lost: bool,
    thread_ids: HashMap<String, ThreadId>,
    acked_removed: bool,
    /// `upsertThreads` reports every written thread as new inbound inbox mail.
    new_inbound_inbox: bool,
    triage_claims: Vec<WorkerClaimTriageArgs>,
    triage_applied: Vec<ApplyTriageArgs>,
    /// What `getSecret` answers.
    secret: Option<SecretRecord>,
}

#[derive(Default, Clone)]
struct FakeConvex {
    st: Arc<Mutex<ConvexState>>,
}

fn lease_lost(f: &str) -> ConvexApiError {
    ConvexApiError::from_app_error(f, "x".into(), json!({"code": "lease_lost", "message": "Lease lost"}))
}

impl FakeConvex {
    fn st(&self) -> std::sync::MutexGuard<'_, ConvexState> {
        self.st.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn enqueue(&self, op: ClaimedOp) {
        let max_attempts = akatsuki_core::sync::max_attempts(op.op.payload.kind());
        self.st().ops.push(FakeOp {
            claimed: op,
            state: OpState::Queued,
            available_at: 0,
            attempt: 0,
            max_attempts,
            verify_only: false,
            send_started: false,
            send_started_at: None,
            last_error: None,
        });
    }

    fn op(&self, id: &str) -> FakeOp {
        self.st().ops.iter().find(|o| o.claimed.op.id.as_str() == id).cloned().expect("op exists")
    }

    /// `ops.reapLeases`: inflight ops go back to queued; a send that started becomes verifyOnly.
    fn reap(&self) {
        let mut st = self.st();
        let now = st.now;
        for o in st.ops.iter_mut().filter(|o| o.state == OpState::Inflight) {
            o.state = OpState::Queued;
            o.available_at = now;
            if o.send_started {
                o.verify_only = true;
            }
        }
    }

    fn advance(&self, ms: i64) {
        self.st().now += ms;
    }
}

fn thread_key(o: &ClaimedOp) -> Option<String> {
    o.thread.as_ref().map(|t| t.provider_thread_id.clone())
}

#[async_trait]
impl WorkerApi for FakeConvex {
    async fn assigned_accounts(&self, _shard: u32) -> Result<Subscription<Vec<AssignedAccount>>, ConvexApiError> {
        Ok(Box::pin(futures::stream::pending()))
    }
    async fn claim_account(&self, _a: &ClaimAccountArgs) -> Result<ClaimAccountResult, ConvexApiError> {
        Ok(ClaimAccountResult::Claimed { ok: True, sync_state: sync_state(None), folder_cursors: vec![] })
    }
    async fn heartbeat(&self, _a: &HeartbeatArgs) -> Result<HeartbeatResult, ConvexApiError> {
        Ok(HeartbeatResult { lost: vec![] })
    }
    async fn release_account(&self, _a: &AccountWorkerArgs) -> Result<(), ConvexApiError> {
        self.st().released += 1;
        Ok(())
    }
    async fn get_secret(&self, _a: &GetSecretArgs) -> Result<Option<SecretRecord>, ConvexApiError> {
        Ok(self.st().secret.clone())
    }
    async fn put_secret(&self, _a: &PutSecretArgs) -> Result<PutSecretResult, ConvexApiError> {
        Ok(PutSecretResult { version: 2 })
    }
    async fn create_account(&self, _a: &CreateAccountArgs) -> Result<CreateAccountResult, ConvexApiError> {
        Ok(CreateAccountResult { account_id: "acc00001".into(), reconnected: false })
    }
    async fn set_account_status(&self, a: &SetAccountStatusArgs) -> Result<(), ConvexApiError> {
        let mut st = self.st();
        if st.lease_lost {
            return Err(lease_lost("worker:setAccountStatus"));
        }
        st.statuses.push(a.clone());
        Ok(())
    }
    async fn upsert_identities(&self, _a: &UpsertIdentitiesArgs) -> Result<(), ConvexApiError> {
        Ok(())
    }
    async fn upsert_labels(&self, a: &UpsertLabelsArgs) -> Result<UpsertLabelsResult, ConvexApiError> {
        let mut st = self.st();
        st.labels_calls.push(a.clone());
        let map = a
            .labels
            .iter()
            .map(|l| {
                let key = match l.role.and_then(|r| r.sys()) {
                    Some(s) => s.key(),
                    None => format!("l:{:0>8}", l.provider_label_id.to_lowercase().replace(['_', '/'], "")),
                };
                (l.provider_label_id.clone(), key)
            })
            .collect();
        Ok(UpsertLabelsResult { label_key_by_provider_id: map })
    }
    async fn upsert_threads(&self, a: &UpsertThreadsArgs) -> Result<UpsertThreadsResult, ConvexApiError> {
        let mut st = self.st();
        if st.lease_lost {
            return Err(lease_lost("worker:upsertThreads"));
        }
        if let Some(c) = &a.cursor {
            st.cursors.push(c.clone());
        }
        for (i, t) in a.threads.iter().enumerate() {
            let n = st.thread_ids.len();
            st.thread_ids.entry(t.provider_thread_id.clone()).or_insert_with(|| format!("thr{n:05}{i}").into());
        }
        st.upserts.push(a.clone());
        let new_inbound = if st.new_inbound_inbox {
            a.threads
                .iter()
                .map(|t| NewInbound { thread_id: st.thread_ids[&t.provider_thread_id].clone(), is_inbox: true })
                .collect()
        } else {
            vec![]
        };
        Ok(UpsertThreadsResult { written: a.threads.len() as u32, skipped_stale: 0, new_inbound })
    }
    async fn set_cursor(&self, a: &SetCursorArgs) -> Result<(), ConvexApiError> {
        self.st().cursors.push(a.cursor.clone());
        Ok(())
    }
    async fn set_watch(&self, a: &SetWatchArgs) -> Result<(), ConvexApiError> {
        self.st().watches.push(a.clone());
        Ok(())
    }
    async fn accounts_for_push(&self, _a: &AccountsForPushArgs) -> Result<Vec<PushAccount>, ConvexApiError> {
        Ok(vec![])
    }
    async fn pending_op_accounts(&self, _shard: u32) -> Result<Subscription<Vec<PendingOpAccount>>, ConvexApiError> {
        Ok(Box::pin(futures::stream::pending()))
    }
    async fn claim_ops(&self, a: &ClaimOpsArgs) -> Result<Vec<ClaimedOp>, ConvexApiError> {
        let mut st = self.st();
        let now = st.now;
        // FIFO per thread: an inflight op, or a skipped older one, blocks later ops on its thread.
        let mut blocked: std::collections::HashSet<String> =
            st.ops.iter().filter(|o| o.state == OpState::Inflight).filter_map(|o| thread_key(&o.claimed)).collect();
        let mut out = Vec::new();
        for o in st.ops.iter_mut().filter(|o| o.state == OpState::Queued) {
            if out.len() >= a.limit as usize {
                break;
            }
            let t = thread_key(&o.claimed);
            let skip = o.available_at > now || t.as_ref().is_some_and(|t| blocked.contains(t));
            if skip {
                if let Some(t) = t {
                    blocked.insert(t);
                }
                continue;
            }
            o.state = OpState::Inflight;
            o.attempt += 1;
            let mut c = o.claimed.clone();
            c.op.attempt = o.attempt;
            c.op.verify_only = o.verify_only;
            c.op.send_elapsed_ms = o.send_started_at.map(|t| now - t);
            out.push(c);
        }
        Ok(out)
    }
    async fn mark_send_started(&self, a: &MarkSendStartedArgs) -> Result<(), ConvexApiError> {
        let mut st = self.st();
        if st.mark_lease_lost {
            return Err(lease_lost("worker:markSendStarted"));
        }
        st.marks.push(a.op_id.as_str().to_owned());
        let now = st.now;
        if let Some(o) = st.ops.iter_mut().find(|o| o.claimed.op.id == a.op_id) {
            o.send_started = true;
            o.send_started_at = Some(now);
        }
        Ok(())
    }
    async fn complete_ops(&self, a: &CompleteOpsArgs) -> Result<(), ConvexApiError> {
        let mut st = self.st();
        let now = st.now;
        st.completes.push(a.clone());
        for r in &a.results {
            match r {
                OpResult::Ok { op_id, .. } => {
                    if let Some(o) =
                        st.ops.iter_mut().find(|o| o.claimed.op.id == *op_id && o.state == OpState::Inflight)
                    {
                        o.state = OpState::Done;
                    }
                }
                OpResult::Err { op_id, error, retry_after_ms, count_attempt, .. } => {
                    let Some(o) = st.ops.iter_mut().find(|o| o.claimed.op.id == *op_id && o.state == OpState::Inflight)
                    else {
                        continue;
                    };
                    let attempt = if *count_attempt { o.attempt } else { o.attempt.saturating_sub(1) };
                    o.attempt = attempt;
                    o.last_error = Some(error.clone());
                    let exhausted = *count_attempt && attempt >= o.max_attempts;
                    if (error.retryable || !*count_attempt) && !exhausted {
                        o.state = OpState::Queued;
                        o.available_at = now + retry_after_ms.unwrap_or(0);
                    } else {
                        let send = matches!(o.claimed.op.payload, OpPayload::Send { .. });
                        o.state = if send && o.verify_only { OpState::Unknown } else { OpState::Failed };
                    }
                }
            }
        }
        Ok(())
    }
    async fn record_draft(&self, _a: &RecordDraftArgs) -> Result<RecordDraftResult, ConvexApiError> {
        Ok(RecordDraftResult::Saved { draft_id: "drf00001".into(), version: 1 })
    }
    async fn schedule_send(&self, _a: &ScheduleSendArgs) -> Result<ScheduleSendResult, ConvexApiError> {
        Ok(ScheduleSendResult { send_id: "snd00001".into(), state: ScheduleOutcome::Scheduled })
    }
    async fn resolve_messages(&self, _a: &ResolveMessagesArgs) -> Result<Vec<ResolvedMessage>, ConvexApiError> {
        Ok(vec![])
    }
    async fn resolve_thread(&self, _a: &ResolveThreadArgs) -> Result<Option<ResolvedThread>, ConvexApiError> {
        Ok(None)
    }
    async fn user_context(&self, _a: &UserContextArgs) -> Result<UserContext, ConvexApiError> {
        Err(ConvexApiError::Transport("unused".into()))
    }
    async fn claim_managed_triage(
        &self,
        a: &WorkerClaimTriageArgs,
    ) -> Result<Vec<akatsuki_core::convex::rows::TriageInput>, ConvexApiError> {
        let mut st = self.st();
        st.triage_claims.push(a.clone());
        let ids = a.thread_ids.clone().unwrap_or_default();
        Ok(ids
            .into_iter()
            .take(a.limit as usize)
            .map(|thread_id| akatsuki_core::convex::rows::TriageInput {
                thread_id,
                from: None,
                to: vec![],
                subject: "Subject".into(),
                snippet: "Snippet".into(),
                list_id: None,
                is_bulk: false,
                has_attachments: false,
            })
            .collect())
    }
    async fn apply_triage(&self, a: &ApplyTriageArgs) -> Result<(), ConvexApiError> {
        self.st().triage_applied.push(a.clone());
        Ok(())
    }
    async fn record_read_receipt(&self, _a: &RecordReadReceiptArgs) -> Result<(), ConvexApiError> {
        Ok(())
    }
    async fn ack_account_removed(&self, _a: &AccountWorkerArgs) -> Result<(), ConvexApiError> {
        self.st().acked_removed = true;
        Ok(())
    }
    async fn thread_ids_by_provider(
        &self,
        a: &ThreadIdsByProviderArgs,
    ) -> Result<Vec<ProviderThreadIdRow>, ConvexApiError> {
        let st = self.st();
        Ok(a.provider_thread_ids
            .iter()
            .filter_map(|p| {
                st.thread_ids
                    .get(p)
                    .map(|t| ProviderThreadIdRow { provider_thread_id: p.clone(), thread_id: t.clone() })
            })
            .collect())
    }
}

// ───────────── fixtures ─────────────

fn account(status: AccountStatus) -> AssignedAccount {
    AssignedAccount {
        account_id: "acc00001".into(),
        user_id: "u1".into(),
        provider: Provider::Gmail,
        provider_account_id: "me@example.com".into(),
        email: "me@example.com".into(),
        status,
        entitled: true,
        server_triage: false,
        imap: None,
        caps: AccountCaps { multi_label: true, categories: true, server_search: true, delete_forever: false },
    }
}

fn sync_state(history: Option<&str>) -> SyncStateDoc {
    SyncStateDoc {
        id: "sst00001".into(),
        creation_time: 1_800_000_000_000.0,
        account_id: "acc00001".into(),
        shard: 0,
        gmail_history_id: history.map(str::to_owned),
        lease_owner: Some("w1".into()),
        lease_expires_at: None,
        watch_kind: None,
        watch_external_id: None,
        watch_expires_at: None,
        watch_client_state_hash: None,
        backfill_before_at: None,
        consecutive_failures: 0,
        last_error: None,
        updated_at: 0,
    }
}

struct Harness {
    actor: AccountActor,
    convex: FakeConvex,
    provider: Arc<FakeProvider>,
    tx: mpsc::UnboundedSender<ActorMsg>,
}

fn harness(status: AccountStatus, history: Option<&str>, provider: FakeProvider) -> Harness {
    let convex = FakeConvex::default();
    let api: Arc<dyn WorkerApi> = Arc::new(convex.clone());
    let provider = Arc::new(provider);
    let (tx, rx) = mpsc::unbounded_channel();
    let sink = Arc::new(ConvexSink::new(api.clone(), "acc00001".into(), "w1".into(), vec!["me@example.com".into()]));
    let actor = AccountActor::new(ActorParts {
        account: account(status),
        api,
        config: Arc::new(EngineConfig::new("w1")),
        provider: provider.clone(),
        push: None,
        sink,
        claim: ClaimState { sync_state: sync_state(history), folder_cursors: vec![] },
        rx,
        self_tx: tx.clone(),
        fetcher: SafeFetcher::new(),
        metrics: Arc::new(Metrics::default()),
        gate: Arc::new(ProjectGate::default()),
        lost: None,
        last_interactive_at: None,
        graph_subs: None,
    });
    Harness { actor, convex, provider, tx }
}

fn thread(id: &str, date: i64, labels: &[&str]) -> FetchedThread {
    FetchedThread {
        provider_thread_id: id.into(),
        provider_version: Some(date),
        messages: vec![message(id, &format!("{id}-m1"), date, labels, &[SysLabel::Unread])],
        partial: false,
    }
}

fn claimed(id: &str, thread: Option<&str>, payload: OpPayload) -> ClaimedOp {
    ClaimedOp {
        op: ClaimedOpCore {
            id: id.into(),
            client_op_id: format!("c-{id}"),
            payload,
            attempt: 0,
            verify_only: false,
            send_elapsed_ms: None,
        },
        thread: thread.map(|t| ClaimedThread {
            provider_thread_id: t.into(),
            synced_label_keys: vec!["sys:inbox".into()],
            messages: vec![ClaimedMessage {
                provider_message_id: format!("{t}-m1"),
                label_keys: vec!["sys:inbox".into()],
                imap_locations: None,
            }],
        }),
        labels: BTreeMap::new(),
        send: None,
        draft: None,
    }
}

fn modify(add: &[&str], remove: &[&str]) -> OpPayload {
    OpPayload::Thread {
        action: ThreadAction::Modify {
            add: add.iter().map(|s| (*s).to_owned()).collect(),
            remove: remove.iter().map(|s| (*s).to_owned()).collect(),
        },
    }
}

fn send_op(id: &str) -> ClaimedOp {
    let mut c = claimed(id, None, OpPayload::Send { scheduled_send_id: "snd00001".into() });
    c.send = Some(ClaimedSend {
        draft: ClaimedDraft {
            provider_draft_id: "d1".into(),
            provider_message_id: Some("d1-msg".into()),
            rfc_message_id: "0192f0c4-7b51-7cc3-a3c5-6a8b1f2d3e4f@example.com".into(),
        },
        kind: SendKind::Undo,
        send_at: 0,
    });
    c
}

const RFC: &str = "0192f0c4-7b51-7cc3-a3c5-6a8b1f2d3e4f@example.com";

fn result_codes(c: &CompleteOpsArgs) -> Vec<(String, String)> {
    c.results
        .iter()
        .map(|r| match r {
            OpResult::Ok { op_id, outcome, .. } => (op_id.to_string(), format!("{outcome:?}").to_lowercase()),
            OpResult::Err { op_id, error, .. } => (op_id.to_string(), error.code.clone()),
        })
        .collect()
}

// ───────────── actor state machine ─────────────

#[tokio::test]
async fn initial_sync_runs_baseline_inbox_first_window_then_live() {
    let p = FakeProvider::gmail();
    for i in 0..30 {
        p.add_thread(thread(&format!("in{i:02}"), 1_800_000_000_000 - i, &["INBOX"]), true);
    }
    for i in 0..10 {
        p.add_thread(thread(&format!("w{i:02}"), 1_799_000_000_000 - i, &[]), false);
    }
    p.st().page_size = 100;
    let mut h = harness(AccountStatus::Connecting, None, p);
    assert_eq!(h.actor.phase(), &AccountPhase::Baseline);

    // Ops wait for the baseline; the first step is the baseline.
    assert_eq!(h.actor.queue.next(h.actor.phase()), Some(WorkKind::InitialStep));
    for _ in 0..10 {
        if matches!(h.actor.phase(), AccountPhase::Live) {
            break;
        }
        let kind = h.actor.queue.next(h.actor.phase()).unwrap_or_else(|| panic!("idle in {:?}", h.actor.phase()));
        let r = h.actor.step(kind).await;
        assert!(h.actor.after_step(r).await.is_none());
    }
    assert_eq!(h.actor.phase(), &AccountPhase::Live);
    let st = h.convex.st();
    // Labels went up complete before anything else, the cursor was written at the baseline.
    assert!(st.labels_calls[0].complete);
    assert_eq!(st.cursors[0], CursorUpdate::Gmail { history_id: "100".into() });
    // Push is off in tests: the watch is recorded as polling.
    assert_eq!(st.watches[0].watch_kind, WatchKind::Poll);
    // First paint: 25 inbox threads in the first mutation, the rest after.
    assert_eq!(st.upserts[0].threads.len(), 25);
    assert_eq!(st.upserts[1].threads.len(), 5);
    let total: usize = st.upserts.iter().map(|u| u.threads.len()).sum();
    assert_eq!(total, 30 + 40, "inbox pass plus window pass (idempotent upserts)");
    let statuses: Vec<_> = st.statuses.iter().map(|s| (s.status, s.sync_phase)).collect();
    assert!(statuses.contains(&(Some(AccountStatus::InitialSync), Some(SyncPhase::InboxFirst))));
    assert!(statuses.contains(&(Some(AccountStatus::Live), Some(SyncPhase::Backfill))));
}

#[tokio::test]
async fn delta_commits_cursor_with_data_and_drops_stale_hints() {
    let p = FakeProvider::gmail();
    p.add_thread(thread("t1", 5, &["INBOX"]), true);
    p.st().changes.push_back(Ok(ChangePage {
        touched_threads: vec![RemoteThreadId("t1".into())],
        deleted_threads: vec![RemoteThreadId("gone".into())],
        deleted_messages: vec![],
        labels_changed: false,
        next: Cursor::Gmail { history_id: 105 },
        more: false,
    }));
    let mut h = harness(AccountStatus::Live, Some("100"), p);
    assert_eq!(h.actor.phase(), &AccountPhase::Live);
    assert!(h.actor.queue.delta_due, "a restarted actor runs one delta at once");
    h.actor.queue.ops_due = false;
    let r = h.actor.step(WorkKind::Delta).await;
    assert!(h.actor.after_step(r).await.is_none());
    {
        let st = h.convex.st();
        let last = st.upserts.last().unwrap();
        assert_eq!(last.threads[0].provider_thread_id, "t1");
        assert_eq!(last.deletes.as_deref(), Some(&["gone".to_owned()][..]));
        assert_eq!(last.cursor, Some(CursorUpdate::Gmail { history_id: "105".into() }), "cursor in the same mutation");
    }
    assert!(!h.actor.queue.delta_due);
    // A Gmail push at or below the cursor is dropped; a newer one schedules a delta.
    h.tx.send(ActorMsg::Cmd(ActorCmd::PushHint { history_id: Some(104) })).unwrap();
    h.actor.drain_for_tests();
    assert!(!h.actor.queue.delta_due);
    h.tx.send(ActorMsg::Cmd(ActorCmd::PushHint { history_id: Some(200) })).unwrap();
    h.actor.drain_for_tests();
    assert!(h.actor.queue.delta_due);
}

#[tokio::test]
async fn expired_cursor_resyncs_the_window_and_commits_a_new_baseline() {
    let p = FakeProvider::gmail();
    p.add_thread(thread("t1", 5, &["INBOX"]), true);
    p.add_thread(thread("t2", 4, &[]), false);
    p.st().changes.push_back(Err(ProviderError::CursorExpired));
    p.st().history = 900;
    let mut h = harness(AccountStatus::Live, Some("100"), p);
    let r = h.actor.step(WorkKind::Delta).await;
    assert!(h.actor.after_step(r).await.is_none());
    assert!(matches!(h.actor.phase(), AccountPhase::Resync { .. }));
    // Resync preempts other work as a delta step until it is done.
    for _ in 0..4 {
        if matches!(h.actor.phase(), AccountPhase::Live) {
            break;
        }
        let r = h.actor.step(WorkKind::Delta).await;
        assert!(h.actor.after_step(r).await.is_none());
    }
    assert_eq!(h.actor.phase(), &AccountPhase::Live);
    let st = h.convex.st();
    assert_eq!(st.cursors.last(), Some(&CursorUpdate::Gmail { history_id: "900".into() }));
    let refetched: Vec<_> =
        st.upserts.iter().flat_map(|u| u.threads.iter().map(|t| t.provider_thread_id.clone())).collect();
    assert_eq!(refetched, vec!["t1", "t2"]);
}

#[tokio::test]
async fn auth_errors_park_the_account_until_reconnect() {
    let p = FakeProvider::gmail();
    p.st().changes.push_back(Err(ProviderError::Auth { reauth_required: true }));
    let mut h = harness(AccountStatus::Live, Some("100"), p);
    let r = h.actor.step(WorkKind::Delta).await;
    assert!(h.actor.after_step(r).await.is_none());
    assert_eq!(h.actor.phase(), &AccountPhase::ReauthRequired);
    assert_eq!(h.actor.queue.next(h.actor.phase()), None, "nothing runs while waiting for reauth");
    let st_status = h.convex.st().statuses.last().cloned().unwrap();
    assert_eq!(st_status.status, Some(AccountStatus::ReauthRequired));
    // Reconnecting (status back to connecting) restarts the actor with fresh credentials.
    h.tx.send(ActorMsg::Account(account(AccountStatus::Connecting))).unwrap();
    assert_eq!(h.actor.run().await, ExitReason::Restart);
}

#[tokio::test]
async fn losing_entitlement_pauses_and_stops_the_watch() {
    let mut a = account(AccountStatus::Live);
    a.entitled = false;
    let h = harness(AccountStatus::Live, Some("100"), FakeProvider::gmail());
    let mut actor = h.actor;
    actor.watch = Some(akatsuki_providers::WatchState {
        kind: WatchKind::GmailWatch,
        external_id: None,
        expires_at: i64::MAX,
        client_state: None,
    });
    h.tx.send(ActorMsg::Account(a)).unwrap();
    let provider = h.provider.clone();
    let tx = h.tx.clone();
    let task = tokio::spawn(actor.run());
    for _ in 0..100 {
        if provider.st().calls.contains(&"unwatch") {
            break;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    assert!(provider.st().calls.contains(&"unwatch"));
    // Entitled again: a fresh actor takes over.
    tx.send(ActorMsg::Account(account(AccountStatus::Live))).unwrap();
    let reason = tokio::time::timeout(Duration::from_secs(5), task).await.unwrap().unwrap();
    assert_eq!(reason, ExitReason::Restart);
}

#[tokio::test]
async fn transient_failures_back_off_then_degrade() {
    let p = FakeProvider::gmail();
    for _ in 0..5 {
        p.st().changes.push_back(Err(ProviderError::Transient("503".into(), None)));
    }
    let mut h = harness(AccountStatus::Live, Some("100"), p);
    for n in 1..=5 {
        let r = h.actor.step(WorkKind::Delta).await;
        assert!(h.actor.after_step(r).await.is_none());
        assert!(matches!(h.actor.phase(), AccountPhase::Backoff { .. }), "attempt {n}");
        h.actor.phase = h.actor.phase.clone().next(PhaseEvent::BackoffElapsed);
    }
    let st = h.convex.st();
    assert!(st.statuses.iter().any(
        |s| s.status == Some(AccountStatus::Degraded) && s.status_detail == Some(Some("provider_transient".into()))
    ));
}

#[tokio::test]
async fn rate_limits_pause_the_actor() {
    let p = FakeProvider::gmail();
    p.st().changes.push_back(Err(ProviderError::RateLimited {
        retry_after: Some(Duration::from_secs(30)),
        scope: akatsuki_providers::LimitScope::Project,
    }));
    let mut h = harness(AccountStatus::Live, Some("100"), p);
    let r = h.actor.step(WorkKind::Delta).await;
    assert!(h.actor.after_step(r).await.is_none());
    assert!(h.actor.pause_until.is_some());
    assert!(h.actor.gate.paused_until().is_some(), "project scope pauses every Gmail actor");
}

#[tokio::test]
async fn full_loop_runs_to_live_and_shutdown_releases_the_lease() {
    let p = FakeProvider::gmail();
    p.add_thread(thread("t1", 5, &["INBOX"]), true);
    let h = harness(AccountStatus::Connecting, None, p);
    let convex = h.convex.clone();
    let tx = h.tx.clone();
    let task = tokio::spawn(h.actor.run());
    for _ in 0..200 {
        if convex.st().statuses.iter().any(|s| s.status == Some(AccountStatus::Live)) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert!(convex.st().statuses.iter().any(|s| s.status == Some(AccountStatus::Live)));
    tx.send(ActorMsg::Cmd(ActorCmd::Shutdown)).unwrap();
    let reason = tokio::time::timeout(Duration::from_secs(5), task).await.unwrap().unwrap();
    assert_eq!(reason, ExitReason::Released);
    assert_eq!(convex.st().released, 1);
}

#[tokio::test]
async fn lease_loss_stops_the_actor_without_releasing() {
    let h = harness(AccountStatus::Live, Some("100"), FakeProvider::gmail());
    h.tx.send(ActorMsg::LeaseLost).unwrap();
    let convex = h.convex.clone();
    assert_eq!(h.actor.run().await, ExitReason::LeaseLost);
    assert_eq!(convex.st().released, 0);

    let h = harness(AccountStatus::Live, Some("100"), FakeProvider::gmail());
    h.convex.st().lease_lost = true;
    let convex = h.convex.clone();
    let reason = tokio::time::timeout(Duration::from_secs(5), h.actor.run()).await.unwrap();
    assert_eq!(reason, ExitReason::LeaseLost, "a lease_lost from a mutation stops the actor at once");
    assert_eq!(convex.st().released, 0);
}

#[tokio::test]
async fn removal_stops_the_watch_revokes_and_acknowledges() {
    let h = harness(AccountStatus::Live, Some("100"), FakeProvider::gmail());
    h.tx.send(ActorMsg::Account(account(AccountStatus::Removing))).unwrap();
    let convex = h.convex.clone();
    let provider = h.provider.clone();
    assert_eq!(h.actor.run().await, ExitReason::Removed);
    assert!(convex.st().acked_removed);
    assert!(provider.st().calls.contains(&"revoke"));
}

#[tokio::test]
async fn backfill_walks_back_by_date_cursor_to_the_horizon() {
    let p = FakeProvider::gmail();
    let window_start = 1_800_000_000_000 - 90 * akatsuki_core::time::DAY_MS;
    for i in 0..3 {
        p.st().older.push((format!("old{i}"), window_start - (i + 1) * akatsuki_core::time::DAY_MS));
    }
    let mut h = harness(AccountStatus::Live, Some("100"), p);
    h.actor.queue = Default::default();
    assert_eq!(h.actor.backfill_before, window_start);
    let r = h.actor.step(WorkKind::BackfillStep).await;
    assert!(h.actor.after_step(r).await.is_none());
    let oldest = window_start - 3 * akatsuki_core::time::DAY_MS;
    assert_eq!(h.actor.backfill_before, oldest);
    let r = h.actor.step(WorkKind::BackfillStep).await;
    assert!(h.actor.after_step(r).await.is_none());
    let st = h.convex.st();
    assert_eq!(st.cursors[0], CursorUpdate::Backfill { before_at: oldest });
    assert!(matches!(st.cursors[1], CursorUpdate::Backfill { .. }));
    assert!(st.statuses.iter().any(|s| s.sync_phase == Some(SyncPhase::Complete)));
}

#[tokio::test]
async fn search_fetch_upserts_hits_and_answers_with_convex_ids() {
    let p = FakeProvider::gmail();
    p.add_thread(thread("s1", 5, &[]), false);
    let mut h = harness(AccountStatus::Live, Some("100"), p);
    let (msg, rx) = ActorMsg::search(vec![RemoteThreadId("s1".into()), RemoteThreadId("missing".into())]);
    h.tx.send(msg).unwrap();
    h.actor.drain_for_tests();
    assert_eq!(h.actor.queue.search_fetches, 1);
    h.actor.queue.ops_due = false;
    h.actor.queue.delta_due = false;
    assert_eq!(h.actor.queue.next(h.actor.phase()), Some(WorkKind::SearchFetch));
    let r = h.actor.step(WorkKind::SearchFetch).await;
    assert!(h.actor.after_step(r).await.is_none());
    let ids = rx.await.unwrap();
    assert_eq!(ids.len(), 2, "both hits were upserted and mapped");
}

// ───────────── ops: claim/complete ordering, retry, poison ─────────────

#[tokio::test]
async fn coalesced_moot_and_applied_ops_complete_in_one_call_in_claim_order() {
    let p = FakeProvider::gmail();
    p.add_thread(thread("t2", 5, &["INBOX"]), true);
    let mut h = harness(AccountStatus::Live, Some("100"), p);
    h.convex.enqueue(claimed("op00001", Some("t1"), modify(&[], &["sys:inbox"])));
    h.convex.enqueue(claimed("op00002", Some("t1"), modify(&["sys:inbox"], &[])));
    h.convex.enqueue(claimed("op00003", Some("t2"), OpPayload::Thread { action: ThreadAction::Trash }));
    h.actor.ops_step().await.unwrap();
    let st = h.convex.st();
    assert_eq!(st.completes.len(), 1);
    assert_eq!(
        result_codes(&st.completes[0]),
        vec![
            ("op00001".into(), "moot".into()),
            ("op00002".into(), "moot".into()),
            ("op00003".into(), "applied".into())
        ]
    );
    // Only the trash reached the provider; its post-op thread state rides along.
    let applied = &h.provider.st().applied;
    assert_eq!(applied.len(), 1);
    assert!(matches!(applied[0].1, ProviderOp::Trash { .. }));
    assert_eq!(st.completes[0].threads.as_ref().unwrap()[0].provider_thread_id, "t2");
    assert!(st.completes[0].threads.as_ref().unwrap()[0].complete);
}

#[tokio::test]
async fn fifo_per_thread_holds_across_claims() {
    let p = FakeProvider::gmail();
    let mut h = harness(AccountStatus::Live, Some("100"), p);
    h.convex.enqueue(claimed("op00001", Some("t1"), OpPayload::Thread { action: ThreadAction::Trash }));
    h.convex.enqueue(claimed("op00002", Some("t1"), OpPayload::Thread { action: ThreadAction::Untrash }));
    h.convex.enqueue(claimed("op00003", Some("t9"), modify(&["sys:starred"], &[])));
    h.provider.st().apply_results.insert("op00001".into(), Err(ProviderError::Transient("503".into(), None)));
    h.actor.ops_step().await.unwrap();
    // op1 failed transiently and is queued with backoff; op2 (same thread, same claim) is not
    // sent ahead of it and goes back behind it; op3 on another thread was not held back.
    assert_eq!(h.convex.op("op00001").state, OpState::Queued);
    assert_eq!(h.convex.op("op00002").state, OpState::Queued);
    assert_eq!(h.convex.op("op00002").attempt, 0, "deferral does not consume an attempt");
    assert_eq!(h.convex.op("op00003").state, OpState::Done);
    let order: Vec<String> = h.provider.st().applied.iter().map(|(id, _)| id.clone()).collect();
    assert_eq!(order, vec!["op00001", "op00003"]);
    // While op1 waits for its backoff, nothing else on t1 is claimable.
    h.convex.enqueue(claimed("op00004", Some("t1"), modify(&["sys:starred"], &[])));
    h.actor.ops_step().await.unwrap();
    assert_eq!(h.convex.op("op00004").state, OpState::Queued);
}

#[tokio::test]
async fn error_dispositions_follow_the_retry_table() {
    let p = FakeProvider::gmail();
    p.add_thread(thread("t3", 5, &["INBOX"]), true);
    let mut h = harness(AccountStatus::Live, Some("100"), p);
    for (id, t) in [("op00001", "t1"), ("op00002", "t2"), ("op00003", "t3"), ("op00004", "t4")] {
        h.convex.enqueue(claimed(id, Some(t), OpPayload::Thread { action: ThreadAction::Trash }));
    }
    {
        let mut st = h.provider.st();
        st.apply_results.insert("op00001".into(), Err(ProviderError::Transient("reset".into(), None)));
        st.apply_results.insert(
            "op00002".into(),
            Err(ProviderError::RateLimited {
                retry_after: Some(Duration::from_secs(40)),
                scope: akatsuki_providers::LimitScope::Account,
            }),
        );
        st.apply_results.insert("op00003".into(), Err(ProviderError::Invalid("bad".into())));
        st.apply_results.insert("op00004".into(), Err(ProviderError::NotFound));
    }
    h.actor.ops_step().await.unwrap();
    let st = h.convex.st();
    let c = &st.completes[0];
    let by_id: HashMap<String, &OpResult> = c
        .results
        .iter()
        .map(|r| match r {
            OpResult::Ok { op_id, .. } | OpResult::Err { op_id, .. } => (op_id.to_string(), r),
        })
        .collect();
    assert!(
        matches!(by_id["op00001"], OpResult::Err { error, count_attempt: true, retry_after_ms: Some(ms), .. } if error.retryable && error.code == "provider_transient" && *ms >= 1000)
    );
    assert!(
        matches!(by_id["op00002"], OpResult::Err { error, count_attempt: false, retry_after_ms: Some(40_000), .. } if error.code == "provider_rate_limited")
    );
    assert!(
        matches!(by_id["op00003"], OpResult::Err { error, .. } if !error.retryable && error.code == "provider_invalid")
    );
    assert!(matches!(by_id["op00004"], OpResult::Ok { outcome: akatsuki_core::convex::OpOutcome::Moot, .. }));
    // A permanent failure sends fresh provider truth for the thread in the same call.
    assert!(c.threads.as_ref().unwrap().iter().any(|t| t.provider_thread_id == "t3"));
    // NotFound deletes the thread.
    assert!(st.upserts.iter().any(|u| u.deletes.as_deref() == Some(&["t4".to_owned()][..])));
    drop(st);
    assert!(h.actor.pause_until.is_some(), "an account rate limit pauses the actor");
}

#[tokio::test]
async fn local_failures_twice_poison_the_op() {
    let mut h = harness(AccountStatus::Live, Some("100"), FakeProvider::gmail());
    // A user label without a provider id cannot be mapped: a non-provider error.
    let bad = || claimed("op00001", Some("t1"), modify(&["l:abcdefgh"], &[]));
    h.convex.enqueue(bad());
    h.actor.ops_step().await.unwrap();
    let first = h.convex.op("op00001");
    assert_eq!(first.state, OpState::Queued);
    assert_eq!(first.last_error.as_ref().unwrap().code, "internal");
    h.convex.advance(60_000);
    h.actor.ops_step().await.unwrap();
    let second = h.convex.op("op00001");
    assert_eq!(second.state, OpState::Failed);
    assert_eq!(second.last_error.unwrap().code, "internal");
    assert!(h.provider.st().applied.is_empty());
}

#[tokio::test]
async fn snoozed_label_is_created_lazily() {
    let p = FakeProvider::gmail();
    p.add_thread(thread("t1", 5, &["INBOX"]), true);
    let mut h = harness(AccountStatus::Live, Some("100"), p);
    h.convex.enqueue(claimed("op00001", Some("t1"), modify(&["sys:snoozed"], &["sys:inbox"])));
    h.actor.ops_step().await.unwrap();
    assert_eq!(h.provider.st().ensured, vec![akatsuki_providers::LabelRole::Snoozed]);
    let c = h.convex.st().completes[0].clone();
    assert_eq!(c.labels.unwrap()[0].name, "Akatsuki/Snoozed");
    let applied = h.provider.st().applied.clone();
    assert!(
        matches!(&applied[0].1, ProviderOp::Modify { add, .. } if matches!(&add[0], akatsuki_providers::LabelTarget::Label { provider_label_id, .. } if provider_label_id == "Label_Akatsuki_Snoozed"))
    );
}

#[tokio::test]
async fn label_create_reports_the_provider_id() {
    let mut h = harness(AccountStatus::Live, Some("100"), FakeProvider::gmail());
    let mut op = claimed("op00001", None, OpPayload::LabelCreate { label_id: "lab00001".into() });
    op.labels.insert(
        "l:lab00001".into(),
        ClaimedLabel { provider_label_id: None, name: "Receipts".into(), exclusive: false, role: None },
    );
    h.convex.enqueue(op);
    h.actor.ops_step().await.unwrap();
    let c = h.convex.st().completes[0].clone();
    assert!(matches!(&c.results[0], OpResult::Ok { provider_label_id: Some(id), .. } if id == "Label_Receipts"));
}

#[tokio::test]
async fn archive_placeholder_label_create_ensures_the_archive_folder() {
    let mut h = harness(AccountStatus::Live, Some("100"), FakeProvider::gmail());
    let mut op = claimed("op00001", None, OpPayload::LabelCreate { label_id: "lab00002".into() });
    op.labels.insert(
        "l:lab00002".into(),
        ClaimedLabel { provider_label_id: None, name: "Archive".into(), exclusive: true, role: Some("archive".into()) },
    );
    h.convex.enqueue(op);
    h.actor.ops_step().await.unwrap();
    assert_eq!(h.provider.st().ensured, vec![akatsuki_providers::LabelRole::Archive]);
    assert!(h.provider.st().applied.is_empty(), "no generic label create (a category or keyword)");
    let c = h.convex.st().completes[0].clone();
    assert!(matches!(&c.results[0], OpResult::Ok { provider_label_id: Some(id), .. } if id == "Label_Archive"));
    assert_eq!(c.labels.unwrap()[0].role, Some(akatsuki_providers::LabelRole::Archive));
}

// ───────────── sends: at most once ─────────────

#[tokio::test]
async fn send_checks_sent_first_marks_started_then_sends_once() {
    let mut h = harness(AccountStatus::Live, Some("100"), FakeProvider::gmail());
    h.convex.enqueue(send_op("op00001"));
    h.actor.ops_step().await.unwrap();
    let p = h.provider.st();
    assert_eq!(
        p.calls.iter().filter(|c| **c == "find_sent" || **c == "send_draft").copied().collect::<Vec<_>>(),
        vec!["find_sent", "send_draft"]
    );
    assert_eq!(h.convex.st().marks, vec!["op00001"]);
    let c = h.convex.st().completes[0].clone();
    assert!(matches!(&c.results[0], OpResult::Ok { sent_provider_message_id: Some(id), .. } if id == "sent-d1"));
}

#[tokio::test]
async fn already_sent_message_is_never_sent_again() {
    let mut h = harness(AccountStatus::Live, Some("100"), FakeProvider::gmail());
    h.provider.st().sent.insert(
        RFC.into(),
        akatsuki_providers::SentRef { provider_message_id: "pm1".into(), provider_thread_id: None },
    );
    h.convex.enqueue(send_op("op00001"));
    h.actor.ops_step().await.unwrap();
    assert_eq!(h.provider.st().send_calls, 0);
    assert!(h.convex.st().marks.is_empty());
    assert_eq!(h.convex.op("op00001").state, OpState::Done);
}

#[tokio::test]
async fn ambiguous_send_is_verified_not_resent() {
    let mut h = harness(AccountStatus::Live, Some("100"), FakeProvider::gmail());
    // The provider takes the message but the connection resets before we hear back.
    h.provider.st().send_errors.push_back((ProviderError::Transient("reset".into(), None), true));
    h.convex.enqueue(send_op("op00001"));
    h.actor.ops_step().await.unwrap();
    // Not reported: the op stays inflight until its lease expires.
    assert!(h.convex.st().completes.is_empty() || h.convex.st().completes[0].results.is_empty());
    assert_eq!(h.convex.op("op00001").state, OpState::Inflight);
    // The reaper sees sendStartedAt and requeues it as verifyOnly.
    h.convex.reap();
    assert!(h.convex.op("op00001").verify_only);
    h.actor.ops_step().await.unwrap();
    assert_eq!(h.convex.op("op00001").state, OpState::Done);
    assert_eq!(h.provider.st().send_calls, 1, "never sent twice");
}

#[tokio::test]
async fn unconfirmed_verify_only_send_ends_unknown() {
    let mut h = harness(AccountStatus::Live, Some("100"), FakeProvider::gmail());
    h.provider.st().send_errors.push_back((ProviderError::Transient("timeout".into(), None), false));
    h.convex.enqueue(send_op("op00001"));
    h.actor.ops_step().await.unwrap();
    h.convex.reap();
    let mut delays = vec![];
    for _ in 0..6 {
        h.actor.ops_step().await.unwrap();
        let op = h.convex.op("op00001");
        if op.state != OpState::Queued {
            break;
        }
        let now = h.convex.st().now;
        delays.push(op.available_at - now);
        h.convex.advance(op.available_at - now);
    }
    // Checks at +30 s, +2 min and +10 min after the send began (timed by Convex, not a worker counter).
    assert_eq!(delays, vec![30_000, 90_000, 480_000]);
    let op = h.convex.op("op00001");
    assert_eq!(op.state, OpState::Unknown);
    // The send and the final verdict count; the misses in between consumed no attempts.
    assert_eq!(op.attempt, 2);
    assert_eq!(op.last_error.unwrap().code, "send_unconfirmed");
    assert_eq!(h.provider.st().send_calls, 1);
}

#[tokio::test]
async fn send_without_lease_never_reaches_the_provider() {
    let mut h = harness(AccountStatus::Live, Some("100"), FakeProvider::gmail());
    h.convex.st().mark_lease_lost = true;
    h.convex.enqueue(send_op("op00001"));
    h.actor.ops_step().await.unwrap();
    assert_eq!(h.provider.st().send_calls, 0);
}

#[tokio::test]
async fn deleted_draft_fails_the_send() {
    let mut h = harness(AccountStatus::Live, Some("100"), FakeProvider::gmail());
    h.provider.st().send_errors.push_back((ProviderError::NotFound, false));
    h.convex.enqueue(send_op("op00001"));
    h.actor.ops_step().await.unwrap();
    let op = h.convex.op("op00001");
    assert_eq!(op.state, OpState::Failed);
    assert_eq!(op.last_error.unwrap().code, "draft_deleted");
}

// ───────────── timers: ops become due ─────────────

#[tokio::test(start_paused = true)]
async fn pending_ops_wake_the_actor_at_available_at() {
    use super::supervisor::{Supervisor, SupervisorDeps};
    let convex = FakeConvex::default();
    let registry = Arc::new(super::Registry::default());
    let (tx, mut rx) = mpsc::unbounded_channel();
    let provider: Arc<dyn akatsuki_providers::MailProvider> = Arc::new(FakeProvider::gmail());
    registry.insert(
        "acc00001".into(),
        Arc::new(super::AccountHandle {
            account: std::sync::RwLock::new(account(AccountStatus::Live)),
            provider,
            tx,
            last_manual_sync: Mutex::new(None),
            last_interactive_at: Mutex::new(None),
        }),
    );
    let sup = Supervisor::new(SupervisorDeps {
        api: Arc::new(convex),
        config: Arc::new(EngineConfig::new("w1")),
        factory: Arc::new(crate::providers::AdapterFactory::default()),
        keyring: Arc::new(crate::crypto::SecretKeyring::new(vec![])),
        oauth: HashMap::new(),
        registry,
        metrics: Arc::new(Metrics::default()),
        gate: Arc::new(ProjectGate::default()),
        http: reqwest::Client::new(),
        fetcher: SafeFetcher::new(),
        gmail_topic: None,
        live: Arc::new(std::sync::atomic::AtomicBool::new(false)),
    });
    let due_in = 2_000;
    sup.on_pending_for_tests(vec![PendingOpAccount {
        account_id: "acc00001".into(),
        min_available_at: akatsuki_core::time::now_ms() + due_in,
    }]);
    tokio::task::yield_now().await;
    assert!(rx.try_recv().is_err(), "not due yet");
    tokio::time::advance(Duration::from_millis(2_100)).await;
    tokio::task::yield_now().await;
    assert!(matches!(rx.try_recv(), Ok(ActorMsg::Cmd(ActorCmd::ApplyOps))));
    sup.on_pending_for_tests(vec![PendingOpAccount { account_id: "acc00001".into(), min_available_at: 0 }]);
    assert!(matches!(rx.try_recv(), Ok(ActorMsg::Cmd(ActorCmd::ApplyOps))));
}

// ───────────── removal without a provider (D-385) ─────────────

#[tokio::test]
async fn removing_account_without_a_usable_secret_is_still_acknowledged() {
    use super::supervisor::{SpawnError, Supervisor, SupervisorDeps};
    let convex = FakeConvex::default();
    let deps = |api: FakeConvex| SupervisorDeps {
        api: Arc::new(api),
        config: Arc::new(EngineConfig::new("w1")),
        factory: Arc::new(crate::providers::AdapterFactory::default()),
        keyring: Arc::new(crate::crypto::SecretKeyring::new(vec![])),
        oauth: HashMap::new(),
        registry: Arc::new(super::Registry::default()),
        metrics: Arc::new(Metrics::default()),
        gate: Arc::new(ProjectGate::default()),
        http: reqwest::Client::new(),
        fetcher: SafeFetcher::new(),
        gmail_topic: None,
        live: Arc::new(std::sync::atomic::AtomicBool::new(false)),
    };
    // The fake has no secret row: a live account parks for reauth, a removing one is acknowledged.
    let mut sup = Supervisor::new(deps(convex.clone()));
    assert!(matches!(sup.spawn(account(AccountStatus::Removing)).await, Err(SpawnError::Removed)));
    assert!(convex.st().acked_removed, "ackAccountRemoved called");
    assert!(convex.st().statuses.is_empty(), "a removing account is never re-marked degraded or reauth");
    assert_eq!(convex.st().released, 1);

    let live = FakeConvex::default();
    let mut sup = Supervisor::new(deps(live.clone()));
    assert!(matches!(sup.spawn(account(AccountStatus::Live)).await, Err(SpawnError::Later(_))));
    assert!(!live.st().acked_removed);
    assert_eq!(live.st().statuses.len(), 1);
}

// ───────────── IMAP endpoints sealed with the password (D-414) ─────────────

#[tokio::test]
async fn imap_settings_changed_in_convex_never_receive_the_password() {
    use akatsuki_core::convex::values::ConvexBytes;
    use akatsuki_core::convex::{ImapSettings, SecretKind, TlsMode};

    use super::supervisor::{SpawnError, Supervisor, SupervisorDeps};
    use crate::config::Key32;
    use crate::crypto::SecretKeyring;
    use crate::oauth::{ImapEndpoints, ImapSecret};

    let keyring = Arc::new(SecretKeyring::new(vec![("k1".into(), Key32([9; 32]))]));
    let settings = ImapSettings {
        host: "imap.example.com".into(),
        port: 993,
        tls: TlsMode::Implicit,
        smtp_host: "smtp.example.com".into(),
        smtp_port: 465,
        smtp_tls: TlsMode::Implicit,
        capabilities: vec![],
        sent_auto_saved: false,
    };
    let mut acc = account(AccountStatus::Live);
    acc.provider = Provider::Imap;
    acc.provider_account_id = "me@imap.example.com".into();
    // Someone with Convex write access points SMTP at their own server.
    acc.imap = Some(ImapSettings { smtp_host: "mail.attacker.example".into(), ..settings.clone() });
    let run = |endpoints: Option<ImapEndpoints>, acc: AssignedAccount| {
        let keyring = keyring.clone();
        async move {
            let secret = ImapSecret { username: "me".into(), password: "not-real".into(), endpoints };
            let sealed =
                keyring.seal(&acc.secret_aad(SecretKind::Imap), &serde_json::to_vec(&secret).unwrap()).unwrap();
            let convex = FakeConvex::default();
            convex.st().secret = Some(SecretRecord {
                key_id: sealed.key_id,
                nonce: ConvexBytes(sealed.nonce),
                ciphertext: ConvexBytes(sealed.ciphertext),
                version: 1,
            });
            let mut sup = Supervisor::new(SupervisorDeps {
                api: Arc::new(convex.clone()),
                config: Arc::new(EngineConfig::new("w1")),
                // Building a provider would try to connect: the check must stop before it.
                factory: Arc::new(crate::providers::AdapterFactory::default()),
                keyring,
                oauth: HashMap::new(),
                registry: Arc::new(super::Registry::default()),
                metrics: Arc::new(Metrics::default()),
                gate: Arc::new(ProjectGate::default()),
                http: reqwest::Client::new(),
                fetcher: SafeFetcher::new(),
                gmail_topic: None,
                live: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            });
            let r = sup.spawn(acc).await;
            (r, convex)
        }
    };

    let (r, convex) = run(Some(ImapEndpoints::of(&settings)), acc.clone()).await;
    assert!(matches!(r, Err(SpawnError::Later(_))));
    {
        let st = convex.st();
        assert_eq!(st.statuses.last().unwrap().status, Some(AccountStatus::ReauthRequired));
        assert_eq!(st.released, 1, "the lease is handed back");
    }

    // A secret sealed before D-414 (no endpoints) is not trusted with Convex's hosts either.
    acc.imap = Some(settings);
    let (r, convex) = run(None, acc).await;
    assert!(matches!(r, Err(SpawnError::Later(_))));
    assert_eq!(convex.st().statuses.last().unwrap().status, Some(AccountStatus::ReauthRequired));
}

// ───────────── managed triage (§12.5, D-390) ─────────────

#[derive(Debug, Default)]
struct FakeTriage {
    calls: Mutex<Vec<(String, Vec<akatsuki_core::convex::rows::TriageInput>)>>,
    fail: std::sync::atomic::AtomicBool,
}

#[async_trait]
impl crate::triage::TriageApi for FakeTriage {
    async fn triage(
        &self,
        user_id: &str,
        items: &[akatsuki_core::convex::rows::TriageInput],
    ) -> Result<Vec<TriageResult>, crate::triage::TriageError> {
        self.calls.lock().unwrap().push((user_id.to_owned(), items.to_vec()));
        if self.fail.load(std::sync::atomic::Ordering::Relaxed) {
            return Err(crate::triage::TriageError::Failed);
        }
        let mut out: Vec<TriageResult> = items
            .iter()
            .map(|i| TriageResult {
                thread_id: i.thread_id.clone(),
                category: akatsuki_core::convex::schema::TriageCategory::Important,
                needs_reply: true,
                model: "anthropic/claude-haiku-5.5".into(),
                prompt_version: 1,
            })
            .collect();
        // An answer for a thread this batch did not claim is never written.
        out.push(TriageResult {
            thread_id: "thrforeign".into(),
            category: akatsuki_core::convex::schema::TriageCategory::Updates,
            needs_reply: false,
            model: "m".into(),
            prompt_version: 1,
        });
        Ok(out)
    }
}

async fn wait_for(mut f: impl FnMut() -> bool) {
    tokio::time::timeout(Duration::from_secs(5), async {
        while !f() {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("condition reached");
}

fn triage_harness(server_triage: bool) -> (Harness, Arc<FakeTriage>) {
    let p = FakeProvider::gmail();
    for i in 0..30 {
        p.add_thread(thread(&format!("in{i:02}"), 1_800_000_000_000 - i, &["INBOX"]), true);
    }
    p.st().page_size = 100;
    let mut h = harness(AccountStatus::Connecting, None, p);
    h.convex.st().new_inbound_inbox = true;
    let fake = Arc::new(FakeTriage::default());
    let mut cfg = EngineConfig::new("w1");
    cfg.triage = Some(fake.clone());
    h.actor.config = Arc::new(cfg);
    h.actor.account.server_triage = server_triage;
    h.actor.start_triage();
    (h, fake)
}

#[tokio::test]
async fn managed_triage_sends_new_inbox_threads_to_apps_api_in_batches_of_25() {
    let (mut h, fake) = triage_harness(true);
    while !matches!(h.actor.phase(), AccountPhase::Live) {
        let kind = h.actor.queue.next(h.actor.phase()).expect("work");
        let r = h.actor.step(kind).await;
        assert!(h.actor.after_step(r).await.is_none());
    }
    wait_for(|| h.convex.st().triage_applied.iter().map(|a| a.results.len()).sum::<usize>() == 30).await;
    let calls = fake.calls.lock().unwrap().clone();
    assert!(calls.iter().all(|(u, items)| u == "u1" && items.len() <= 25), "batches of at most 25 for the owner");
    assert_eq!(calls.iter().map(|(_, i)| i.len()).sum::<usize>(), 30);
    let st = h.convex.st();
    assert!(st.triage_claims.iter().all(|c| c.account_id.as_str() == "acc00001" && c.worker_id == "w1"));
    assert!(
        st.triage_applied.iter().flat_map(|a| &a.results).all(|r| r.thread_id.as_str() != "thrforeign"),
        "only claimed threads are written"
    );
}

#[tokio::test]
async fn managed_triage_is_off_without_server_triage_and_failures_write_nothing() {
    let (mut h, fake) = triage_harness(false);
    while !matches!(h.actor.phase(), AccountPhase::Live) {
        let kind = h.actor.queue.next(h.actor.phase()).expect("work");
        let r = h.actor.step(kind).await;
        assert!(h.actor.after_step(r).await.is_none());
    }
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert!(fake.calls.lock().unwrap().is_empty());
    assert!(h.convex.st().triage_claims.is_empty());

    // Turned on, apps/api failing: claims happen, nothing is applied (the sweep retries after the lease).
    fake.fail.store(true, std::sync::atomic::Ordering::Relaxed);
    let mut a = account(AccountStatus::Live);
    a.server_triage = true;
    h.tx.send(ActorMsg::Account(a)).unwrap();
    h.actor.drain_for_tests();
    h.provider.add_thread(thread("fresh", 1_800_000_000_100, &["INBOX"]), true);
    let next = h.actor.cursor.clone().expect("cursor");
    h.provider.st().changes.push_back(Ok(ChangePage {
        touched_threads: vec![RemoteThreadId("fresh".into())],
        deleted_threads: vec![],
        deleted_messages: vec![],
        labels_changed: false,
        next,
        more: false,
    }));
    h.actor.queue.delta_due = true;
    let r = h.actor.step(WorkKind::Delta).await;
    assert!(h.actor.after_step(r).await.is_none());
    wait_for(|| !fake.calls.lock().unwrap().is_empty()).await;
    tokio::time::sleep(Duration::from_millis(20)).await;
    assert!(h.convex.st().triage_applied.is_empty());
}

#[tokio::test]
async fn http_triage_posts_a_worker_token_and_maps_budget_errors() {
    use wiremock::matchers::{header_exists, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/internal/v1/triage"))
        .and(header_exists("authorization"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"results": [
            {"threadId": "thr1", "category": "receipts", "needsReply": false, "model": "anthropic/claude-haiku-5.5", "promptVersion": 1}
        ]})))
        .up_to_n_times(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/internal/v1/triage"))
        .respond_with(ResponseTemplate::new(429).insert_header("retry-after", "120"))
        .mount(&server)
        .await;
    let pem = {
        use p256::pkcs8::EncodePrivateKey;
        let signing = p256::ecdsa::SigningKey::random(&mut rand08::rngs::OsRng);
        signing.to_pkcs8_pem(p256::pkcs8::LineEnding::LF).unwrap().to_string()
    };
    let minter =
        akatsuki_core::auth::ServiceTokenMinter::from_pem(&pem, "k1", "https://sync.test", "worker:w1").unwrap();
    let client = crate::triage::HttpTriage::new(reqwest::Client::new(), &server.uri(), Arc::new(minter));
    let item = akatsuki_core::convex::rows::TriageInput {
        thread_id: "thr1".into(),
        from: None,
        to: vec![],
        subject: "Receipt".into(),
        snippet: "Thanks".into(),
        list_id: None,
        is_bulk: false,
        has_attachments: false,
    };
    use crate::triage::TriageApi as _;
    let r = client.triage("u1", std::slice::from_ref(&item)).await.unwrap();
    assert_eq!(r[0].thread_id.as_str(), "thr1");
    assert_eq!(
        client.triage("u1", &[item]).await.unwrap_err(),
        crate::triage::TriageError::Paused(Duration::from_secs(120))
    );
    let reqs = server.received_requests().await.unwrap();
    let body: serde_json::Value = serde_json::from_slice(&reqs[0].body).unwrap();
    assert_eq!(body["userId"], "u1");
    assert_eq!(body["items"][0]["threadId"], "thr1");
    let auth = reqs[0].headers.get("authorization").unwrap().to_str().unwrap();
    let token = auth.strip_prefix("Bearer ").unwrap();
    let claims: serde_json::Value = {
        use base64::Engine as _;
        let part = token.split('.').nth(1).unwrap();
        serde_json::from_slice(&base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(part).unwrap()).unwrap()
    };
    assert_eq!(claims["aud"], "akatsuki-api");
    assert_eq!(claims["sub"], "worker:w1");
}

#[test]
fn the_actor_publishes_its_graph_subscription_for_the_push_hook() {
    // D-405: the hook checks notifications against what the running actors hold, without Convex.
    let convex = FakeConvex::default();
    let api: Arc<dyn WorkerApi> = Arc::new(convex.clone());
    let (tx, rx) = mpsc::unbounded_channel();
    let index = Arc::new(super::GraphSubscriptions::default());
    let mut state = sync_state(None);
    state.watch_kind = Some(WatchKind::GraphSubscription);
    state.watch_external_id = Some("sub-from-claim".into());
    state.watch_expires_at = Some(1_900_000_000_000);
    state.watch_client_state_hash = Some("hash-from-claim".into());
    let mut actor = AccountActor::new(ActorParts {
        account: account(AccountStatus::Live),
        api: api.clone(),
        config: Arc::new(EngineConfig::new("w1")),
        provider: Arc::new(FakeProvider::gmail()),
        push: None,
        sink: Arc::new(ConvexSink::new(api, "acc00001".into(), "w1".into(), vec![])),
        claim: ClaimState { sync_state: state, folder_cursors: vec![] },
        rx,
        self_tx: tx,
        fetcher: SafeFetcher::new(),
        metrics: Arc::new(Metrics::default()),
        gate: Arc::new(ProjectGate::default()),
        lost: None,
        last_interactive_at: None,
        graph_subs: Some(index.clone()),
    });
    actor.publish_graph_subscription();
    assert_eq!(index.get("sub-from-claim"), Some(("acc00001".into(), "hash-from-claim".into())));
    // The watch went away (unwatch, or a switch to polling): the entry goes too.
    actor.watch = None;
    actor.publish_graph_subscription();
    assert_eq!(index.get("sub-from-claim"), None);
}

// ───────────── IMAP: partial threads (D-223, D-410) ─────────────

mod imap_partial {
    use std::time::Duration;

    use akatsuki_providers::imap::fake::imap::{FakeImap, FakeImapOptions};
    use akatsuki_providers::imap::{
        Credentials, ImapAccountConfig, ImapProvider, Security, ServerEndpoint, SyncTuning, Timeouts,
    };
    use akatsuki_providers::{Cursor, MailProvider};

    use super::*;

    fn mail(mid: &str, subject: &str, refs: &[&str], date: i64) -> Vec<u8> {
        let when = jiff::Timestamp::from_millisecond(date).unwrap().strftime("%a, %d %b %Y %H:%M:%S +0000");
        let mut s = format!(
            "From: Ada <ada@example.com>\r\nTo: Jane <jane@example.org>\r\nSubject: {subject}\r\nDate: {when}\r\nMessage-ID: <{mid}>\r\n"
        );
        if let Some(last) = refs.last() {
            let all: Vec<String> = refs.iter().map(|r| format!("<{r}>")).collect();
            s.push_str(&format!("In-Reply-To: <{last}>\r\nReferences: {}\r\n", all.join(" ")));
        }
        s.push_str("MIME-Version: 1.0\r\nContent-Type: text/plain; charset=utf-8\r\n\r\nhello\r\n");
        s.into_bytes()
    }

    fn config(server: &FakeImap) -> ImapAccountConfig {
        let mut cfg = ImapAccountConfig::new(
            "acc00001",
            "jane@example.org",
            ServerEndpoint::new("127.0.0.1", server.addr.port(), Security::Plaintext),
            ServerEndpoint::new("127.0.0.1", 9, Security::Plaintext),
            Credentials::Password { username: "jane".into(), password: "secret".into() },
        );
        cfg.tls.allow_private_hosts = true;
        cfg.timeouts = Timeouts {
            connect: Duration::from_secs(5),
            command: Duration::from_secs(5),
            smtp_data_end: Duration::from_secs(5),
            idle_reissue: Duration::from_secs(2),
        };
        cfg.tuning = SyncTuning {
            flag_scan_every: Duration::ZERO,
            scan_window: Duration::from_secs(90 * 24 * 3600),
            status_sweep_every: Duration::from_millis(300),
            poll_every: Duration::from_millis(200),
            max_new_per_page: 500,
        };
        cfg.sent_auto_saved = Some(false);
        cfg
    }

    fn imap_actor(provider: Arc<dyn MailProvider>, cursor: Cursor) -> (AccountActor, FakeConvex) {
        let convex = FakeConvex::default();
        let api: Arc<dyn WorkerApi> = Arc::new(convex.clone());
        let (tx, rx) = mpsc::unbounded_channel();
        let sink =
            Arc::new(ConvexSink::new(api.clone(), "acc00001".into(), "w1".into(), vec!["jane@example.org".into()]));
        let mut acc = account(AccountStatus::Live);
        acc.provider = Provider::Imap;
        acc.provider_account_id = "jane@127.0.0.1".into();
        acc.email = "jane@example.org".into();
        let mut actor = AccountActor::new(ActorParts {
            account: acc,
            api,
            config: Arc::new(EngineConfig::new("w1")),
            provider,
            push: None,
            sink,
            claim: ClaimState { sync_state: sync_state(None), folder_cursors: vec![] },
            rx,
            self_tx: tx,
            fetcher: SafeFetcher::new(),
            metrics: Arc::new(Metrics::default()),
            gate: Arc::new(ProjectGate::default()),
            lost: None,
            last_interactive_at: None,
            graph_subs: None,
        });
        // As claimed from Convex with the folder cursors of the previous process.
        actor.cursor = Some(cursor);
        actor.phase = AccountPhase::Live;
        (actor, convex)
    }

    #[tokio::test]
    async fn a_delta_after_a_restart_keeps_older_messages_and_names_expunged_ones() {
        let server = FakeImap::start(FakeImapOptions::full()).await;
        let t0 = akatsuki_core::time::now_ms() - 3_600_000;
        {
            let mut w = server.world();
            w.deliver("INBOX", &mail("root@x", "Plan", &[], t0), &[], t0);
            w.deliver("INBOX", &mail("r1@x", "Re: Plan", &["root@x"], t0 + 1_000), &[], t0 + 1_000);
        }
        // The previous worker process synced the thread and committed its folder cursors.
        let before = ImapProvider::connect(config(&server)).await.unwrap();
        let cursor = before.baseline().await.unwrap();
        drop(before);
        // A reply arrives; the restarted worker's thread index is empty.
        server.world().deliver("INBOX", &mail("r2@x", "Re: Plan", &["root@x", "r1@x"], t0 + 2_000), &[], t0 + 2_000);
        let restarted: Arc<dyn MailProvider> = Arc::new(ImapProvider::connect(config(&server)).await.unwrap());
        let (mut actor, convex) = imap_actor(restarted, cursor);
        let r = actor.step(WorkKind::Delta).await;
        assert!(actor.after_step(r).await.is_none());
        {
            let st = convex.st();
            let ups: Vec<&ThreadUpsert> = st.upserts.iter().flat_map(|u| u.threads.iter()).collect();
            assert_eq!(ups.len(), 1, "the touched thread");
            assert_eq!(ups[0].messages.len(), 1, "only the new reply is known to this process");
            assert!(!ups[0].complete, "a partial IMAP thread must not delete the older messages in Convex");
        }

        // An expunge reaches Convex by message id: no thread is touched, nothing is re-fetched.
        let v = server.world().mbox("INBOX").validity;
        server.world().expunge("INBOX", 1);
        actor.queue.delta_due = true;
        let r = actor.step(WorkKind::Delta).await;
        assert!(actor.after_step(r).await.is_none());
        let st = convex.st();
        let last = st.upserts.last().unwrap();
        assert_eq!(last.deleted_messages, Some(vec![format!("INBOX:{v}:1")]));
        assert!(last.cursor.is_some(), "the cursor commits with the deletion");
    }
}
