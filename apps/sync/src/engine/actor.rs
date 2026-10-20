//! The account actor (ARCHITECTURE §9.2–§9.4, §9.6): one serialized task per mail account.
//!
//! Every loop iteration picks the highest-priority pending work with
//! `akatsuki_core::sync::WorkQueue` (shutdown, ops, push/delta, search fetch, initial step,
//! backfill step) and runs exactly one bounded step: one provider page plus one Convex mutation.
//! Priorities therefore preempt between steps, and a stale fetch can never interleave with an op.

use std::collections::{BTreeMap, HashMap, VecDeque};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use akatsuki_convex_api::{ConvexApiError, WorkerApi};
use akatsuki_core::convex::rows::{FolderCursor, SyncStateDoc};
use akatsuki_core::convex::worker::{AccountWorkerArgs, AssignedAccount, SetWatchArgs, ThreadIdsByProviderArgs};
use akatsuki_core::convex::{AccountStatus, CursorUpdate, Provider, SyncPhase, SyncProgress, WatchKind};
use akatsuki_core::ids::ThreadId;
use akatsuki_core::sync::backfill::{BACKFILL_PAGE, BackfillProgress, SyncWindow, advance};
use akatsuki_core::sync::budget::{AccountBudget, GRAPH_MAX_PER_SEC, WorkClass, gmail_delta_cost, gmail_page_cost};
use akatsuki_core::sync::{
    AccountPhase, ActorCmd, FIRST_PAINT_BATCH, GMAIL_POLL_MS, GRAPH_POLL_MS, PROGRESS_EVERY_MS, PhaseEvent,
    ResyncReason, SAFETY_POLL_MS, UPSERT_BATCH, WorkKind, WorkQueue, backoff,
};
use akatsuki_core::time::{DAY_MS, HOUR_MS, UnixMs, now_ms};
use akatsuki_core::wire::ErrorCode;
use akatsuki_providers::{
    AccountStatusUpdate, Cursor, FetchDepth, GraphFolderCursor, ImapFolderCursor, LimitScope, MailProvider, MetaSink,
    ProviderError, PushEvent, PushKind, PushSource, RemoteThreadId, SinkError, WatchState, WindowRequest,
};
use sha2::{Digest, Sha256};
use tokio::sync::{mpsc, oneshot};

use super::sink::ConvexSink;
use super::{ActorMsg, EngineConfig, Lifecycle, ProjectGate};
use crate::config::PushMode;
use crate::fetch::SafeFetcher;
use crate::metrics::Metrics;

/// Threads fetched per `fetch_threads` call.
pub const FETCH_CHUNK: usize = 100;
/// Consecutive step failures before the account is reported `degraded`.
pub const DEGRADED_AFTER: u32 = 5;
/// Gmail watch renewal period (expires in 7 days, §9.6).
pub const GMAIL_WATCH_RENEW_MS: i64 = 24 * HOUR_MS;
/// Graph subscriptions are renewed when less than this remains.
pub const GRAPH_RENEW_BEFORE_MS: i64 = 24 * HOUR_MS;
/// IMAP folder `STATUS` poll (§9.6) when IDLE is not available or for other folders.
pub const IMAP_POLL_MS: i64 = 5 * 60 * 1000;

/// Why an actor stopped.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ExitReason {
    /// Shutdown: lease released.
    Released,
    /// Another worker owns the account now.
    LeaseLost,
    /// Credentials or entitlement changed: the supervisor starts a fresh actor.
    Restart,
    /// The account was removed and acknowledged.
    Removed,
}

/// A step failure.
#[derive(Debug)]
pub enum StepError {
    /// From the provider.
    Provider(ProviderError),
    /// From Convex.
    Sink(SinkError),
}

impl From<ProviderError> for StepError {
    fn from(e: ProviderError) -> Self {
        StepError::Provider(e)
    }
}

impl From<SinkError> for StepError {
    fn from(e: SinkError) -> Self {
        StepError::Sink(e)
    }
}

/// Maps a Convex call error into a step error.
pub fn convex_step_error(e: &ConvexApiError) -> StepError {
    StepError::Sink(super::sink::sink_error(e))
}

/// What a claimed account starts from.
#[derive(Clone, Debug)]
pub struct ClaimState {
    /// The `syncState` row.
    pub sync_state: SyncStateDoc,
    /// Folder cursors (Graph, IMAP).
    pub folder_cursors: Vec<FolderCursor>,
}

/// Rebuilds the provider cursor from what `worker.claimAccount` returned.
pub fn cursor_from_claim(provider: Provider, st: &SyncStateDoc, folders: &[FolderCursor]) -> Option<Cursor> {
    match provider {
        Provider::Gmail => {
            st.gmail_history_id.as_deref().and_then(|h| h.parse::<u64>().ok()).map(|h| Cursor::Gmail { history_id: h })
        }
        Provider::Graph => {
            let folders: BTreeMap<String, GraphFolderCursor> = folders
                .iter()
                .map(|f| (f.remote_folder_id.clone(), GraphFolderCursor { delta_link: f.delta_link.clone() }))
                .collect();
            (!folders.is_empty()).then_some(Cursor::Graph { folders })
        }
        Provider::Imap => {
            let folders: BTreeMap<String, ImapFolderCursor> = folders
                .iter()
                .filter_map(|f| {
                    Some((
                        f.remote_folder_id.clone(),
                        ImapFolderCursor {
                            uid_validity: f.uid_validity?,
                            uid_next: f.uid_next?,
                            highest_modseq: f.highest_mod_seq.as_deref().and_then(|m| m.parse().ok()),
                        },
                    ))
                })
                .collect();
            (!folders.is_empty()).then_some(Cursor::Imap { folders })
        }
    }
}

/// Everything an actor runs with.
pub struct ActorParts {
    /// The account.
    pub account: AssignedAccount,
    /// Convex.
    pub api: Arc<dyn WorkerApi>,
    /// Engine settings.
    pub config: Arc<EngineConfig>,
    /// The adapter.
    pub provider: Arc<dyn MailProvider>,
    /// IMAP IDLE stream.
    pub push: Option<Box<dyn PushSource>>,
    /// Metadata sink.
    pub sink: Arc<ConvexSink>,
    /// Claim result.
    pub claim: ClaimState,
    /// Mailbox.
    pub rx: mpsc::UnboundedReceiver<ActorMsg>,
    /// A sender into the own mailbox (push forwarding).
    pub self_tx: mpsc::UnboundedSender<ActorMsg>,
    /// SSRF-guarded client (one-click unsubscribe).
    pub fetcher: SafeFetcher,
    /// Counters.
    pub metrics: Arc<Metrics>,
    /// Shared Gmail project budget.
    pub gate: Arc<ProjectGate>,
    /// Set by the token source when the secret moved to another worker.
    pub lost: Option<Arc<AtomicBool>>,
    /// Interactive use reported by the HTTP layer.
    pub last_interactive_at: Option<Arc<std::sync::Mutex<Option<UnixMs>>>>,
    /// Where the actor publishes its live Graph subscription for the push hook (D-405).
    pub graph_subs: Option<Arc<super::GraphSubscriptions>>,
}

impl std::fmt::Debug for ActorParts {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ActorParts").field("account_id", &self.account.account_id).finish_non_exhaustive()
    }
}

/// Resync bookkeeping (§9.3 step 6).
#[derive(Clone, Debug)]
pub(super) struct ResyncState {
    pub(super) new_cursor: Option<Cursor>,
    pub(super) page: Option<String>,
}

/// The actor.
pub struct AccountActor {
    pub(super) account: AssignedAccount,
    pub(super) api: Arc<dyn WorkerApi>,
    pub(super) config: Arc<EngineConfig>,
    pub(super) provider: Arc<dyn MailProvider>,
    pub(super) sink: Arc<ConvexSink>,
    pub(super) fetcher: SafeFetcher,
    pub(super) metrics: Arc<Metrics>,
    pub(super) gate: Arc<ProjectGate>,
    rx: mpsc::UnboundedReceiver<ActorMsg>,
    self_tx: mpsc::UnboundedSender<ActorMsg>,
    // A `Mutex` only to make the actor `Sync` (`PushSource` is `Send`); it is taken once.
    push: std::sync::Mutex<Option<Box<dyn PushSource>>>,
    lost: Option<Arc<AtomicBool>>,
    last_interactive_at: Option<Arc<std::sync::Mutex<Option<UnixMs>>>>,
    graph_subs: Option<Arc<super::GraphSubscriptions>>,
    /// Hash of the current Graph subscription's clientState (from the claim, or set when one is created).
    client_state_hash: Option<String>,

    pub(super) phase: AccountPhase,
    pub(super) queue: WorkQueue,
    pub(super) cursor: Option<Cursor>,
    page: Option<String>,
    pub(super) window: SyncWindow,
    pub(super) backfill_before: UnixMs,
    backfill_messages: u64,
    backfill_wait_until: Option<Instant>,
    pub(super) resync: Option<ResyncState>,
    pub(super) watch: Option<WatchState>,
    watch_renew_at: Option<Instant>,
    next_poll_at: Instant,
    searches: VecDeque<(Vec<RemoteThreadId>, oneshot::Sender<Vec<ThreadId>>)>,
    pub(super) budget: AccountBudget,
    pub(super) poison: HashMap<String, u32>,
    pub(super) pause_until: Option<Instant>,
    consecutive_failures: u32,
    threads_done: u64,
    last_progress_at: Option<Instant>,
    last_status: Option<(Option<AccountStatus>, Option<SyncPhase>)>,
    labels_loaded: bool,
    unwatch_pending: bool,
    exit: Option<ExitReason>,
    /// Managed triage task (started in `run`, §12.5).
    pub(super) triage: Option<crate::triage::TriageHandle>,
}

impl std::fmt::Debug for AccountActor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AccountActor")
            .field("account_id", &self.account.account_id)
            .field("phase", &self.phase)
            .finish_non_exhaustive()
    }
}

fn jitter() -> f64 {
    let b = crate::crypto::random_bytes::<2>().unwrap_or([128, 0]);
    f64::from(u16::from_le_bytes(b)) / 65_536.0
}

/// `hex(sha256(clientState))`, what Convex stores for Graph subscriptions.
pub fn client_state_hash(client_state: &str) -> String {
    hex::encode(Sha256::digest(client_state.as_bytes()))
}

fn duration_ms(ms: i64) -> Duration {
    Duration::from_millis(u64::try_from(ms.max(0)).unwrap_or(0))
}

impl AccountActor {
    /// Builds an actor from a claimed account.
    pub fn new(parts: ActorParts) -> Self {
        let ActorParts {
            account,
            api,
            config,
            provider,
            push,
            sink,
            claim,
            rx,
            self_tx,
            fetcher,
            metrics,
            gate,
            lost,
            last_interactive_at,
            graph_subs,
        } = parts;
        let cursor = cursor_from_claim(account.provider, &claim.sync_state, &claim.folder_cursors);
        let connected_at = claim.sync_state.creation_time as i64;
        let window = SyncWindow::from_connect(if connected_at > 0 { connected_at } else { now_ms() });
        let backfill_before = window.backfill_before(claim.sync_state.backfill_before_at);
        let watch = claim.sync_state.watch_kind.and_then(|kind| {
            matches!(kind, WatchKind::GmailWatch | WatchKind::GraphSubscription).then(|| WatchState {
                kind,
                external_id: claim.sync_state.watch_external_id.clone(),
                expires_at: claim.sync_state.watch_expires_at.unwrap_or(0),
                client_state: None,
            })
        });
        let phase = match AccountPhase::resume(account.status, cursor.is_some()) {
            AccountPhase::Authenticating => AccountPhase::Baseline,
            p => p,
        };
        let phase = if account.entitled { phase } else { AccountPhase::Paused };
        let budget = match account.provider {
            Provider::Gmail => AccountBudget::gmail(now_ms()),
            Provider::Graph => AccountBudget::requests_per_second(GRAPH_MAX_PER_SEC, now_ms()),
            Provider::Imap => AccountBudget::requests_per_second(50, now_ms()),
        };
        let backfill_remaining = !window.backfill_done(backfill_before, 0);
        let queue = WorkQueue {
            // A restarted worker runs one delta pass at once to cover pushes it missed (§9.2).
            delta_due: cursor.is_some(),
            ops_due: true,
            backfill_remaining: backfill_remaining && matches!(phase, AccountPhase::Live),
            ..Default::default()
        };
        Self {
            account,
            api,
            config,
            provider,
            sink,
            fetcher,
            metrics,
            gate,
            rx,
            self_tx,
            push: std::sync::Mutex::new(push),
            lost,
            last_interactive_at,
            graph_subs,
            client_state_hash: claim.sync_state.watch_client_state_hash.clone(),
            phase,
            queue,
            cursor,
            page: None,
            window,
            backfill_before,
            backfill_messages: 0,
            backfill_wait_until: None,
            resync: None,
            watch,
            watch_renew_at: None,
            next_poll_at: Instant::now(),
            searches: VecDeque::new(),
            budget,
            poison: HashMap::new(),
            pause_until: None,
            consecutive_failures: 0,
            threads_done: 0,
            last_progress_at: None,
            last_status: None,
            labels_loaded: false,
            unwatch_pending: false,
            exit: None,
            triage: None,
        }
    }

    /// The current phase (tests).
    pub fn phase(&self) -> &AccountPhase {
        &self.phase
    }

    pub(super) fn worker_id(&self) -> &str {
        &self.config.worker_id
    }

    pub(super) fn account_worker(&self) -> AccountWorkerArgs {
        AccountWorkerArgs { account_id: self.account.account_id.clone(), worker_id: self.config.worker_id.clone() }
    }

    fn push_active(&self) -> bool {
        if self.config.push == PushMode::Off || self.config.public_base_url.is_none() {
            return false;
        }
        match self.provider.caps().push {
            PushKind::GmailPubSub => self.config.gmail_topic,
            PushKind::GraphWebhook => true,
            PushKind::ImapIdle => true,
            PushKind::Poll { .. } => false,
        }
    }

    fn poll_interval(&self) -> Duration {
        let ms = if self.push_active() {
            SAFETY_POLL_MS
        } else {
            match self.provider.caps().push {
                PushKind::GmailPubSub => GMAIL_POLL_MS,
                PushKind::GraphWebhook => GRAPH_POLL_MS,
                PushKind::ImapIdle => IMAP_POLL_MS,
                PushKind::Poll { every } => i64::try_from(every.as_millis()).unwrap_or(IMAP_POLL_MS),
            }
        };
        duration_ms(ms)
    }

    fn on_msg(&mut self, msg: ActorMsg) {
        match msg {
            ActorMsg::Cmd(ActorCmd::SearchFetch(ids, reply)) => {
                self.queue.search_fetches += 1;
                self.searches.push_back((ids, reply));
            }
            ActorMsg::Cmd(ActorCmd::PushHint { history_id: Some(h) }) if matches!(self.cursor, Some(Cursor::Gmail { history_id }) if h <= history_id) =>
            {
                // A hint at or below the committed cursor is dropped (§9.6).
            }
            ActorMsg::Cmd(ActorCmd::RenewWatch) => {
                self.watch_renew_at = Some(Instant::now());
                self.queue.delta_due = true;
            }
            ActorMsg::Cmd(cmd) => self.queue.note(&cmd),
            ActorMsg::LeaseLost => self.exit = Some(ExitReason::LeaseLost),
            ActorMsg::Lifecycle(l) => {
                if matches!(l, Lifecycle::Reauthorize | Lifecycle::Removed) {
                    if l == Lifecycle::Removed {
                        self.watch = None;
                    }
                    self.watch_renew_at = Some(Instant::now());
                }
                self.queue.delta_due = true;
            }
            ActorMsg::Account(a) => self.on_account(a),
        }
    }

    fn on_account(&mut self, a: AssignedAccount) {
        let was_reauth = matches!(self.phase, AccountPhase::ReauthRequired);
        let was_paused = matches!(self.phase, AccountPhase::Paused);
        if a.status == AccountStatus::Removing {
            self.phase = self.phase.clone().next(PhaseEvent::RemoveRequested);
        } else if !a.entitled {
            if !was_paused {
                // Not entitled: stop provider push until the owner is entitled again (§13.3).
                self.phase = self.phase.clone().next(PhaseEvent::EntitlementLost);
                self.unwatch_pending = true;
            }
        } else if was_paused || (was_reauth && a.status != AccountStatus::ReauthRequired) {
            // Fresh credentials or entitlement: start over with a new actor.
            self.exit = Some(ExitReason::Restart);
        } else if a.status == AccountStatus::ReauthRequired && !was_reauth {
            self.phase = self.phase.clone().next(PhaseEvent::AuthRejected);
        }
        if let Some(t) = &self.triage {
            t.set_enabled(a.server_triage);
        }
        self.account = a;
    }

    fn drain(&mut self) {
        while let Ok(m) = self.rx.try_recv() {
            self.on_msg(m);
        }
    }

    /// Applies queued messages without running a step (tests).
    #[cfg(test)]
    pub(super) fn drain_for_tests(&mut self) {
        self.drain();
    }

    fn spawn_push(&mut self) -> Option<tokio::task::JoinHandle<()>> {
        let mut push = self.push.lock().unwrap_or_else(|e| e.into_inner()).take()?;
        let tx = self.self_tx.clone();
        Some(tokio::spawn(async move {
            let mut failures = 0u32;
            loop {
                let msg = match push.next().await {
                    Ok(PushEvent::FolderChanged { remote_folder_id }) => {
                        failures = 0;
                        ActorMsg::Cmd(ActorCmd::FolderChanged(remote_folder_id))
                    }
                    Ok(PushEvent::Reconnected) => {
                        failures = 0;
                        ActorMsg::Cmd(ActorCmd::PushHint { history_id: None })
                    }
                    Err(_) => {
                        failures = failures.saturating_add(1);
                        tokio::time::sleep(backoff(failures, jitter())).await;
                        continue;
                    }
                };
                if tx.send(msg).is_err() {
                    return;
                }
            }
        }))
    }

    /// Runs until shutdown, lease loss, removal or a restart request.
    pub async fn run(mut self) -> ExitReason {
        let push_task = self.spawn_push();
        self.start_triage();
        self.publish_graph_subscription();
        let reason = self.run_loop().await;
        if let Some(t) = push_task {
            t.abort();
        }
        if let Some(index) = &self.graph_subs {
            index.clear(&self.account.account_id);
        }
        if matches!(reason, ExitReason::Released | ExitReason::Restart) {
            let _ = self.api.release_account(&self.account_worker()).await;
        }
        reason
    }

    /// Starts the managed-triage task when apps/api is configured (§12.5, D-390).
    pub(super) fn start_triage(&mut self) {
        if let Some(client) = self.config.triage.clone() {
            self.triage = Some(crate::triage::Triager::spawn(
                self.api.clone(),
                client,
                self.account.account_id.clone(),
                self.config.worker_id.clone(),
                self.account.user_id.clone(),
                self.account.server_triage,
            ));
        }
    }

    async fn run_loop(&mut self) -> ExitReason {
        loop {
            self.drain();
            if let Some(r) = self.exit.take() {
                return r;
            }
            if self.lost.as_ref().is_some_and(|l| l.load(Ordering::Relaxed)) {
                return ExitReason::Released;
            }
            if matches!(self.phase, AccountPhase::Removing) {
                return self.remove().await;
            }
            if self.unwatch_pending {
                self.unwatch_pending = false;
                if let Some(w) =
                    self.watch.take().filter(|w| matches!(w.kind, WatchKind::GmailWatch | WatchKind::GraphSubscription))
                {
                    let _ = self.provider.unwatch(&w).await;
                }
                self.publish_graph_subscription();
            }
            let now = Instant::now();
            if let AccountPhase::Backoff { until, .. } = &self.phase
                && now >= *until
            {
                self.phase = self.phase.clone().next(PhaseEvent::BackoffElapsed);
            }
            if self.cursor.is_some()
                && matches!(self.phase, AccountPhase::Live | AccountPhase::InboxFirst { .. } | AccountPhase::Window)
                && now >= self.next_poll_at
            {
                self.queue.delta_due = true;
                self.next_poll_at = now + self.poll_interval();
            }
            if let Some(at) =
                self.last_interactive_at.as_ref().and_then(|m| *m.lock().unwrap_or_else(|e| e.into_inner()))
            {
                self.budget.note_user_activity(at);
            }
            let backfill_ready = self.backfill_wait_until.is_none_or(|u| now >= u);
            let backfill_open = !self.window.backfill_done(self.backfill_before, self.backfill_messages);
            self.queue.backfill_remaining = backfill_open && backfill_ready;

            let paused = self.pause_until.into_iter().chain(self.gmail_project_pause()).max().filter(|u| *u > now);
            let work = if self.queue.shutdown {
                Some(WorkKind::Shutdown)
            } else if paused.is_some() {
                None
            } else {
                self.queue.next(&self.phase)
            };
            let renew_due = self.watch_renew_at.is_some_and(|t| now >= t) && self.phase_is_synced();
            match work {
                Some(WorkKind::Shutdown) => return ExitReason::Released,
                Some(kind) => {
                    let r = self.step(kind).await;
                    if let Some(reason) = self.after_step(r).await {
                        return reason;
                    }
                }
                None if renew_due => {
                    let r = self.ensure_watch().await;
                    if let Some(reason) = self.after_step(r).await {
                        return reason;
                    }
                }
                None => {
                    let mut wake = self.next_poll_at;
                    for t in [
                        paused,
                        self.watch_renew_at,
                        self.backfill_wait_until.filter(|_| backfill_open),
                        match &self.phase {
                            AccountPhase::Backoff { until, .. } => Some(*until),
                            _ => None,
                        },
                    ]
                    .into_iter()
                    .flatten()
                    {
                        wake = wake.min(t);
                    }
                    let wake = wake.max(now + Duration::from_millis(5));
                    tokio::select! {
                        m = self.rx.recv() => match m {
                            Some(m) => self.on_msg(m),
                            None => return ExitReason::Released,
                        },
                        () = tokio::time::sleep_until(tokio::time::Instant::from_std(wake)) => {}
                    }
                }
            }
        }
    }

    fn phase_is_synced(&self) -> bool {
        matches!(
            self.phase,
            AccountPhase::Live | AccountPhase::InboxFirst { .. } | AccountPhase::Window | AccountPhase::Resync { .. }
        )
    }

    fn gmail_project_pause(&self) -> Option<Instant> {
        (self.account.provider == Provider::Gmail).then(|| self.gate.paused_until()).flatten()
    }

    pub(super) async fn step(&mut self, kind: WorkKind) -> Result<(), StepError> {
        match kind {
            WorkKind::Shutdown => Ok(()),
            WorkKind::ApplyOps => self.ops_step().await,
            WorkKind::Delta => {
                if matches!(self.phase, AccountPhase::Resync { .. }) {
                    self.resync_step().await
                } else {
                    self.delta_step().await
                }
            }
            WorkKind::SearchFetch => self.search_step().await,
            WorkKind::InitialStep => match self.phase.clone() {
                AccountPhase::Authenticating => {
                    self.phase = self.phase.clone().next(PhaseEvent::Authenticated);
                    Ok(())
                }
                AccountPhase::Baseline => self.baseline_step().await,
                AccountPhase::InboxFirst { done } => self.window_step(true, done).await,
                AccountPhase::Window => self.window_step(false, 0).await,
                _ => Ok(()),
            },
            WorkKind::BackfillStep => self.backfill_step().await,
        }
    }

    /// Handles a step result; returns an exit reason when the actor must stop.
    pub(super) async fn after_step(&mut self, r: Result<(), StepError>) -> Option<ExitReason> {
        match r {
            Ok(()) => {
                self.consecutive_failures = 0;
                self.report_status().await
            }
            Err(StepError::Sink(SinkError::LeaseLost)) => Some(ExitReason::LeaseLost),
            Err(StepError::Sink(SinkError::NotEntitled)) => {
                self.phase = self.phase.clone().next(PhaseEvent::EntitlementLost);
                None
            }
            Err(StepError::Provider(ProviderError::Auth { .. })) => {
                self.phase = self.phase.clone().next(PhaseEvent::AuthRejected);
                let _ = self
                    .sink
                    .set_status(AccountStatusUpdate {
                        status: Some(AccountStatus::ReauthRequired),
                        status_detail: Some(Some(ErrorCode::ReauthRequired)),
                        ..Default::default()
                    })
                    .await;
                None
            }
            Err(StepError::Provider(ProviderError::CursorExpired)) => {
                self.metrics.provider_errors.fetch_add(1, Ordering::Relaxed);
                self.resync = None;
                self.phase = self.phase.clone().next(PhaseEvent::CursorExpired(ResyncReason::CursorExpired));
                None
            }
            Err(StepError::Provider(ProviderError::RateLimited { retry_after, scope })) => {
                self.metrics.provider_errors.fetch_add(1, Ordering::Relaxed);
                let wait = retry_after.unwrap_or(Duration::from_secs(30)).max(Duration::from_secs(1));
                let until = Instant::now() + wait;
                self.pause_until = Some(until);
                self.budget.exhausted(now_ms());
                if scope == LimitScope::Project && self.account.provider == Provider::Gmail {
                    self.gate.pause_until(until);
                }
                None
            }
            Err(e) => {
                if matches!(e, StepError::Provider(_)) {
                    self.metrics.provider_errors.fetch_add(1, Ordering::Relaxed);
                }
                self.consecutive_failures = self.consecutive_failures.saturating_add(1);
                let until = Instant::now() + backoff(self.consecutive_failures, jitter());
                self.phase = self.phase.clone().next(PhaseEvent::Transient { until });
                if self.consecutive_failures == DEGRADED_AFTER {
                    let code = match &e {
                        StepError::Provider(p) => p.code(),
                        StepError::Sink(_) => ErrorCode::Internal,
                    };
                    let _ = self
                        .sink
                        .set_status(AccountStatusUpdate {
                            status: Some(AccountStatus::Degraded),
                            status_detail: Some(Some(code)),
                            ..Default::default()
                        })
                        .await;
                    self.last_status = None;
                }
                tracing::warn!(
                    account = %self.account.account_id,
                    failures = self.consecutive_failures,
                    kind = match &e { StepError::Provider(p) => p.code().as_str(), StepError::Sink(_) => "sink" },
                    "sync step failed"
                );
                None
            }
        }
    }

    /// Writes `status`/`syncPhase` when the phase changed what Convex should show, and progress
    /// at most every 5 s.
    async fn report_status(&mut self) -> Option<ExitReason> {
        let status = self.phase.status();
        let sync_phase = self.phase.sync_phase();
        let changed = self.last_status != Some((status, sync_phase));
        let progress_due = self.last_progress_at.is_none_or(|t| t.elapsed() >= duration_ms(PROGRESS_EVERY_MS));
        if !changed && !(progress_due && self.threads_done > 0 && sync_phase.is_some()) {
            return None;
        }
        let update = AccountStatusUpdate {
            status: if changed { status } else { None },
            status_detail: (changed && status.is_some()).then_some(None),
            sync_phase: if changed { sync_phase } else { None },
            progress: progress_due.then_some(SyncProgress { threads_done: self.threads_done, threads_estimated: None }),
            last_sync_at: matches!(self.phase, AccountPhase::Live).then(now_ms),
            ..Default::default()
        };
        match self.sink.set_status(update).await {
            Ok(()) => {
                self.last_status = Some((status, sync_phase));
                if progress_due {
                    self.last_progress_at = Some(Instant::now());
                }
                None
            }
            Err(SinkError::LeaseLost) => Some(ExitReason::LeaseLost),
            Err(_) => None,
        }
    }

    pub(super) async fn refresh_labels(&mut self) -> Result<(), StepError> {
        let labels = self.provider.list_labels().await?;
        self.sink.upsert_labels(labels, true).await?;
        self.labels_loaded = true;
        Ok(())
    }

    async fn ensure_labels(&mut self) -> Result<(), StepError> {
        if !self.labels_loaded {
            self.refresh_labels().await?;
        }
        Ok(())
    }

    fn cursor_updates(&self, cursor: &Cursor) -> Vec<CursorUpdate> {
        let sink = self.sink.clone();
        cursor.to_updates(|id| sink.label_key(id))
    }

    /// Commits `cursor` together with `threads`: the first update rides on the last upsert batch,
    /// the remaining folder cursors follow with `worker.setCursor`.
    async fn commit(
        &mut self,
        threads: Vec<akatsuki_providers::FetchedThread>,
        deletes: Vec<String>,
        deleted_messages: Vec<String>,
        cursor: Option<&Cursor>,
        first_batch: usize,
    ) -> Result<u32, StepError> {
        let mut updates = cursor.map(|c| self.cursor_updates(c)).unwrap_or_default().into_iter();
        let first = updates.next();
        let upserts = self.sink.to_upserts(&threads);
        let n = upserts.len() as u64;
        let ack = self.sink.write_changes(upserts, deletes, deleted_messages, first, first_batch).await?;
        for u in updates {
            self.sink.set_cursor(u).await?;
        }
        self.threads_done += n;
        self.metrics.threads_written.fetch_add(u64::from(ack.written), Ordering::Relaxed);
        if let Some(t) = &self.triage {
            t.submit(ack.new_inbound.into_iter().filter(|n| n.is_inbox).map(|n| n.thread_id).collect());
        }
        Ok(ack.written)
    }

    async fn fetch(&self, ids: &[RemoteThreadId]) -> Result<Vec<akatsuki_providers::FetchedThread>, StepError> {
        let mut out = Vec::with_capacity(ids.len());
        for chunk in ids.chunks(FETCH_CHUNK) {
            out.extend(self.provider.fetch_threads(chunk, FetchDepth::Metadata).await?);
        }
        Ok(out)
    }

    /// Step-level budget. The Gmail adapter charges exact per-call quota itself (account, project
    /// and interactive lanes), so the actor only paces backfill against the 30% share while the
    /// user is active (§9.4); sync steps are not charged twice.
    fn charge(&mut self, class: WorkClass, units: u32) -> Result<(), i64> {
        if class != WorkClass::Backfill {
            return Ok(());
        }
        let units = if self.account.provider == Provider::Gmail { units } else { 1 };
        self.budget.try_spend(class, units, now_ms())
    }

    async fn baseline_step(&mut self) -> Result<(), StepError> {
        self.refresh_labels().await?;
        let cursor = self.provider.baseline().await?;
        for u in self.cursor_updates(&cursor) {
            self.sink.set_cursor(u).await?;
        }
        self.cursor = Some(cursor);
        self.ensure_watch().await?;
        self.phase = self.phase.clone().next(PhaseEvent::BaselineDone);
        self.queue.delta_due = false;
        self.page = None;
        Ok(())
    }

    /// One InboxFirst (`inbox`) or Window page.
    async fn window_step(&mut self, inbox: bool, done: u32) -> Result<(), StepError> {
        self.ensure_labels().await?;
        let req = if inbox {
            WindowRequest::Inbox { page: self.page.clone() }
        } else {
            WindowRequest::Since { after: self.window.window_start_at, page: self.page.clone() }
        };
        let page = self.provider.list_window(&req).await?;
        if let Err(wait) = self.charge(WorkClass::Sync, gmail_page_cost(page.threads.len())) {
            // Over budget: the listing is cheap to redo; wait and retry this page.
            self.pause_until = Some(Instant::now() + duration_ms(wait));
            return Ok(());
        }
        let threads = self.fetch(&page.threads).await?;
        let first = if inbox && done == 0 { FIRST_PAINT_BATCH } else { UPSERT_BATCH };
        let n = u32::try_from(threads.len()).unwrap_or(u32::MAX);
        self.commit(threads, vec![], vec![], None, first).await?;
        let more = page.next_page.is_some();
        self.page = page.next_page;
        if inbox {
            self.phase = self.phase.clone().next(PhaseEvent::InboxPage { threads: n, more });
        } else if !more {
            self.phase = self.phase.clone().next(PhaseEvent::WindowDone);
            let complete = self.window.backfill_done(self.backfill_before, 0);
            self.sink
                .set_status(AccountStatusUpdate {
                    status: Some(AccountStatus::Live),
                    status_detail: Some(None),
                    sync_phase: Some(if complete { SyncPhase::Complete } else { SyncPhase::Backfill }),
                    oldest_synced_at: Some(self.window.window_start_at),
                    last_sync_at: Some(now_ms()),
                    ..Default::default()
                })
                .await?;
            self.last_status = Some((self.phase.status(), self.phase.sync_phase()));
        }
        Ok(())
    }

    async fn backfill_step(&mut self) -> Result<(), StepError> {
        self.ensure_labels().await?;
        if let Err(wait) = self.charge(WorkClass::Backfill, gmail_page_cost(BACKFILL_PAGE as usize)) {
            self.backfill_wait_until = Some(Instant::now() + duration_ms(wait));
            return Ok(());
        }
        let req = WindowRequest::Before {
            before: self.backfill_before,
            horizon: self.window.horizon_at,
            limit: BACKFILL_PAGE,
        };
        let page = self.provider.list_window(&req).await?;
        let threads = self.fetch(&page.threads).await?;
        let messages: u64 = threads.iter().map(|t| t.messages.len() as u64).sum();
        let progress = advance(
            &self.window,
            self.backfill_before,
            page.threads.len(),
            page.oldest,
            self.backfill_messages + messages,
        );
        let before = match progress {
            BackfillProgress::Continue { before } | BackfillProgress::Done { before } => before,
        };
        let upserts = self.sink.to_upserts(&threads);
        self.threads_done += upserts.len() as u64;
        self.sink
            .write_upserts(upserts, vec![], Some(CursorUpdate::Backfill { before_at: before }), UPSERT_BATCH)
            .await?;
        self.backfill_before = before;
        self.backfill_messages += messages;
        if let BackfillProgress::Done { .. } = progress {
            self.queue.backfill_remaining = false;
            self.sink
                .set_status(AccountStatusUpdate {
                    sync_phase: Some(SyncPhase::Complete),
                    oldest_synced_at: Some(before),
                    ..Default::default()
                })
                .await?;
        }
        Ok(())
    }

    async fn delta_step(&mut self) -> Result<(), StepError> {
        let Some(cursor) = self.cursor.clone() else {
            self.queue.delta_due = false;
            return Ok(());
        };
        self.ensure_labels().await?;
        let page = self.provider.changes_since(&cursor).await?;
        if let Err(wait) = self.charge(WorkClass::Sync, gmail_delta_cost(page.touched_threads.len())) {
            self.pause_until = Some(Instant::now() + duration_ms(wait));
            return Ok(());
        }
        if page.labels_changed {
            self.refresh_labels().await?;
        }
        let threads = self.fetch(&page.touched_threads).await?;
        let deletes: Vec<String> = page.deleted_threads.iter().map(|t| t.0.clone()).collect();
        // Partial (IMAP) threads never drop unlisted messages, so expunged and moved-away messages
        // must be named (D-410).
        self.commit(threads, deletes, page.deleted_messages, Some(&page.next), UPSERT_BATCH).await?;
        self.cursor = Some(page.next);
        self.queue.delta_due = page.more;
        self.metrics.deltas.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }

    /// §9.3 step 6: new baseline H′, re-list the window, re-fetch, then commit H′. Convex offers
    /// no listing of its threads inside the window yet, so every listed thread is re-fetched
    /// (version-guarded, so unchanged threads are skipped as stale) and nothing is deleted by the
    /// diff; deletions still arrive through later deltas (DECISIONS D-233).
    async fn resync_step(&mut self) -> Result<(), StepError> {
        let mut state = self.resync.take().unwrap_or(ResyncState { new_cursor: None, page: None });
        if state.new_cursor.is_none() {
            self.refresh_labels().await?;
            state.new_cursor = Some(self.provider.baseline().await?);
            self.resync = Some(state);
            return Ok(());
        }
        let page = self
            .provider
            .list_window(&WindowRequest::Since { after: self.window.window_start_at, page: state.page.clone() })
            .await?;
        let threads = self.fetch(&page.threads).await?;
        self.commit(threads, vec![], vec![], None, UPSERT_BATCH).await?;
        state.page = page.next_page;
        if state.page.is_none() {
            let cursor = state.new_cursor.take();
            if let Some(c) = &cursor {
                for u in self.cursor_updates(c) {
                    self.sink.set_cursor(u).await?;
                }
            }
            self.cursor = cursor;
            self.phase = self.phase.clone().next(PhaseEvent::ResyncDone);
            self.queue.delta_due = false;
            self.resync = None;
        } else {
            self.resync = Some(state);
        }
        Ok(())
    }

    async fn search_step(&mut self) -> Result<(), StepError> {
        let Some((ids, reply)) = self.searches.pop_front() else {
            self.queue.search_fetches = 0;
            return Ok(());
        };
        self.queue.search_fetches = self.searches.len();
        self.ensure_labels().await?;
        let ids: Vec<RemoteThreadId> = ids.into_iter().take(FETCH_CHUNK).collect();
        let threads = self.fetch(&ids).await?;
        self.commit(threads, vec![], vec![], None, UPSERT_BATCH).await?;
        let rows = self
            .api
            .thread_ids_by_provider(&ThreadIdsByProviderArgs {
                account_id: self.account.account_id.clone(),
                provider_thread_ids: ids.iter().map(|i| i.0.clone()).collect(),
            })
            .await
            .map_err(|e| convex_step_error(&e))?;
        let by_provider: HashMap<String, ThreadId> =
            rows.into_iter().map(|r| (r.provider_thread_id, r.thread_id)).collect();
        let ordered: Vec<ThreadId> = ids.iter().filter_map(|i| by_provider.get(&i.0).cloned()).collect();
        let _ = reply.send(ordered);
        Ok(())
    }

    /// Creates or renews the provider watch and records it (§9.6).
    async fn ensure_watch(&mut self) -> Result<(), StepError> {
        let now = now_ms();
        if !self.push_active() {
            let kind = if self.provider.caps().push == PushKind::ImapIdle && self.config.push == PushMode::On {
                WatchKind::ImapIdle
            } else {
                WatchKind::Poll
            };
            if self.watch.as_ref().is_none_or(|w| w.kind != kind) {
                self.set_watch(kind, None, None, None).await?;
                self.watch = Some(WatchState { kind, external_id: None, expires_at: 0, client_state: None });
                self.client_state_hash = None;
            }
            self.watch_renew_at = None;
            self.publish_graph_subscription();
            return Ok(());
        }
        let hook_base = self.config.public_base_url.clone().unwrap_or_default();
        let current = self.watch.clone().filter(|w| w.expires_at > now);
        match self.provider.watch(current.as_ref(), &hook_base).await? {
            Some(w) => {
                // A renewal keeps the stored hash; only a new subscription carries a new secret.
                let hash = w.client_state.as_deref().map(client_state_hash);
                if hash.is_some()
                    || current.as_ref().is_none_or(|c| c.external_id != w.external_id || c.expires_at != w.expires_at)
                {
                    self.set_watch(w.kind, w.external_id.clone(), Some(w.expires_at), hash.clone()).await?;
                }
                if hash.is_some() {
                    self.client_state_hash = hash;
                }
                let renew_in = match w.kind {
                    WatchKind::GmailWatch => GMAIL_WATCH_RENEW_MS,
                    _ => (w.expires_at - now - GRAPH_RENEW_BEFORE_MS).max(HOUR_MS),
                };
                self.watch_renew_at = Some(Instant::now() + duration_ms(renew_in.min(7 * DAY_MS)));
                self.watch = Some(w);
            }
            None => {
                self.watch_renew_at = None;
            }
        }
        self.publish_graph_subscription();
        Ok(())
    }

    /// Tells the push hook which Graph subscription (and clientState hash) this account holds, or that it
    /// holds none (D-405).
    pub(super) fn publish_graph_subscription(&self) {
        let Some(index) = &self.graph_subs else { return };
        let live = self.watch.as_ref().filter(|w| w.kind == WatchKind::GraphSubscription);
        match (live.and_then(|w| w.external_id.as_deref()), self.client_state_hash.as_deref()) {
            (Some(id), Some(hash)) => index.set(&self.account.account_id, id, hash),
            _ => index.clear(&self.account.account_id),
        }
    }

    async fn set_watch(
        &self,
        kind: WatchKind,
        external_id: Option<String>,
        expires_at: Option<UnixMs>,
        client_state_hash: Option<String>,
    ) -> Result<(), StepError> {
        self.api
            .set_watch(&SetWatchArgs {
                account_id: self.account.account_id.clone(),
                worker_id: self.worker_id().to_owned(),
                watch_kind: kind,
                external_id,
                expires_at,
                client_state_hash,
            })
            .await
            .map_err(|e| convex_step_error(&e))
    }

    /// Account removal: stop the watch, revoke the grant (best effort), acknowledge so Convex
    /// purges, and release.
    async fn remove(&mut self) -> ExitReason {
        if let Some(w) =
            self.watch.take().filter(|w| matches!(w.kind, WatchKind::GmailWatch | WatchKind::GraphSubscription))
        {
            let _ = self.provider.unwatch(&w).await;
        }
        let _ = self.provider.revoke().await;
        match self.api.ack_account_removed(&self.account_worker()).await {
            Ok(()) => {
                let _ = self.api.release_account(&self.account_worker()).await;
                ExitReason::Removed
            }
            Err(e) if e.is_lease_lost() => ExitReason::LeaseLost,
            Err(_) => {
                let _ = self.api.release_account(&self.account_worker()).await;
                ExitReason::Restart
            }
        }
    }
}
