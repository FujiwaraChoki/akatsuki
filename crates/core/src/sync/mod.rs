//! The sync engine state machine (ARCHITECTURE §9.2–§9.3, §5.5).
//!
//! The worker runs one serialized task (the *account actor*) per mail account. This module holds
//! the pure parts of that actor so they can be tested without a provider or Convex:
//!
//! - [`AccountPhase`] and [`PhaseEvent`]: where an account is in its life and how it moves on.
//! - [`ActorCmd`]: messages the supervisor and HTTP handlers send to an actor.
//! - [`WorkQueue`]: the strict priority order in which an actor picks its next bounded step.
//! - [`backoff_ms`]: the op retry backoff.
//! - [`budget`]: provider quota buckets (§9.4).
//! - [`plan`]: coalescing and moot detection for one claim batch (§5.4).
//! - [`send`]: the at-most-once send decision table (§5.6).
//! - [`backfill`]: window, backfill cursor and resync bookkeeping (§9.3).
//!
//! The driver loop that owns a `MailProvider` and a `MetaSink` lives in the worker
//! (`apps/sync::engine`, crate dependency direction, DECISIONS D-090 and D-230); it only calls
//! into these functions to decide what to do next.

pub mod backfill;
pub mod budget;
pub mod plan;
pub mod send;

use std::time::{Duration, Instant};

use crate::convex::{AccountStatus, SyncPhase};
use crate::ids::ThreadId;
use crate::model::RemoteThreadId;
use crate::time::{DAY_MS, MINUTE_MS, SECOND_MS};

/// Account lease length (`worker.claimAccount`).
pub const LEASE_MS: i64 = 60 * SECOND_MS;
/// Lease renewal period (`worker.heartbeat`).
pub const HEARTBEAT_EVERY_MS: i64 = 20 * SECOND_MS;
/// Op lease length (`worker.claimOps`).
pub const OP_LEASE_MS: i64 = 60 * SECOND_MS;
/// Ops claimed per batch.
pub const CLAIM_LIMIT: u32 = 100;
/// Threads committed in the first InboxFirst mutation, so the first paint comes within seconds.
pub const FIRST_PAINT_BATCH: usize = 25;
/// Threads per `worker.upsertThreads` call.
pub const UPSERT_BATCH: usize = 100;
/// The initial sync window.
pub const WINDOW_MS: i64 = 90 * DAY_MS;
/// The default backfill horizon.
pub const DEFAULT_HORIZON_MS: i64 = 365 * DAY_MS;
/// Safety poll for push-driven accounts.
pub const SAFETY_POLL_MS: i64 = 5 * MINUTE_MS;
/// Polling interval for Gmail without Pub/Sub.
pub const GMAIL_POLL_MS: i64 = 60 * SECOND_MS;
/// Polling interval for Graph without webhooks.
pub const GRAPH_POLL_MS: i64 = 2 * MINUTE_MS;
/// IMAP IDLE re-issue period (RFC 2177 allows 29 minutes).
pub const IMAP_IDLE_REISSUE_MS: i64 = 25 * MINUTE_MS;
/// Progress is written to Convex at most this often.
pub const PROGRESS_EVERY_MS: i64 = 5 * SECOND_MS;
/// Maximum messages per account for "everything" backfill.
pub const BACKFILL_MESSAGE_CAP: u64 = 250_000;

/// Why a resync runs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ResyncReason {
    /// Gmail 404 on `startHistoryId`, Graph 410.
    CursorExpired,
    /// IMAP `UIDVALIDITY` changed for a folder.
    UidValidityChanged {
        /// The folder.
        remote_folder_id: String,
    },
}

/// The phases of an account actor (§9.2).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AccountPhase {
    /// Waiting for `worker.claimAccount`.
    Claiming,
    /// Loading and refreshing credentials.
    Authenticating,
    /// Labels, cursor and watch.
    Baseline,
    /// All inbox threads, newest first.
    InboxFirst {
        /// Threads committed so far.
        done: u32,
    },
    /// Everything since `windowStartAt`.
    Window,
    /// Push/poll plus ops; backfill runs as low-priority steps.
    Live,
    /// Cursor expired or `UIDVALIDITY` changed.
    Resync {
        /// Why.
        reason: ResyncReason,
    },
    /// Waiting after a transient failure, then resuming `from`.
    Backoff {
        /// When to resume.
        until: Instant,
        /// Phase to resume.
        from: Box<AccountPhase>,
    },
    /// Credentials rejected; waits for the user to reconnect.
    ReauthRequired,
    /// Owner not entitled; watch stopped.
    Paused,
    /// The account is being removed.
    Removing,
    /// Lease released or lost; the actor exits.
    Released,
}

/// Something that happened to an actor.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PhaseEvent {
    /// `worker.claimAccount` succeeded.
    Claimed,
    /// Credentials are usable.
    Authenticated,
    /// Labels upserted, cursor written, watch started.
    BaselineDone,
    /// One InboxFirst page committed.
    InboxPage {
        /// Threads in the page.
        threads: u32,
        /// More inbox pages remain.
        more: bool,
    },
    /// The window listing is exhausted.
    WindowDone,
    /// The provider said the cursor is no longer valid.
    CursorExpired(ResyncReason),
    /// Resync committed a new baseline.
    ResyncDone,
    /// A transient failure; retry after the delay.
    Transient {
        /// When to retry.
        until: Instant,
    },
    /// The backoff delay elapsed.
    BackoffElapsed,
    /// Credentials were rejected.
    AuthRejected,
    /// The user reconnected the account.
    Reconnected,
    /// The owner is no longer entitled.
    EntitlementLost,
    /// The owner is entitled again.
    EntitlementRegained,
    /// The account status became `removing`.
    RemoveRequested,
    /// The lease was lost or released, or the worker is shutting down.
    LeaseEnded,
}

impl AccountPhase {
    /// The phase after `event`. Events that do not apply leave the phase unchanged.
    #[must_use]
    pub fn next(self, event: PhaseEvent) -> AccountPhase {
        use AccountPhase as P;
        use PhaseEvent as E;
        match (self, event) {
            (_, E::LeaseEnded) => P::Released,
            (P::Released, _) => P::Released,
            (_, E::RemoveRequested) => P::Removing,
            (P::Removing, _) => P::Removing,
            (_, E::EntitlementLost) => P::Paused,
            (P::Paused, E::EntitlementRegained) => P::Authenticating,
            (P::Paused, _) => P::Paused,
            (_, E::AuthRejected) => P::ReauthRequired,
            (P::ReauthRequired, E::Reconnected) => P::Authenticating,
            (P::ReauthRequired, _) => P::ReauthRequired,
            (P::Claiming, E::Claimed) => P::Authenticating,
            (P::Authenticating, E::Authenticated) => P::Baseline,
            (P::Baseline, E::BaselineDone) => P::InboxFirst { done: 0 },
            (P::InboxFirst { done }, E::InboxPage { threads, more }) => {
                let done = done.saturating_add(threads);
                if more { P::InboxFirst { done } } else { P::Window }
            }
            (P::Window, E::WindowDone) => P::Live,
            (P::Resync { .. }, E::ResyncDone) => P::Live,
            (_, E::CursorExpired(reason)) => P::Resync { reason },
            (P::Backoff { from, .. }, E::BackoffElapsed) => *from,
            (P::Backoff { from, .. }, E::Transient { until }) => P::Backoff { until, from },
            (phase, E::Transient { until }) => P::Backoff { until, from: Box::new(phase) },
            (phase, _) => phase,
        }
    }

    /// The `mailAccounts.status` this phase reports, if it changes it.
    pub fn status(&self) -> Option<AccountStatus> {
        Some(match self {
            AccountPhase::Claiming | AccountPhase::Authenticating => return None,
            AccountPhase::Baseline | AccountPhase::InboxFirst { .. } | AccountPhase::Window => {
                AccountStatus::InitialSync
            }
            AccountPhase::Live | AccountPhase::Resync { .. } => AccountStatus::Live,
            AccountPhase::Backoff { from, .. } => return from.status(),
            AccountPhase::ReauthRequired => AccountStatus::ReauthRequired,
            AccountPhase::Paused => AccountStatus::Paused,
            AccountPhase::Removing => AccountStatus::Removing,
            AccountPhase::Released => return None,
        })
    }

    /// The `mailAccounts.syncPhase` this phase reports, if it changes it.
    pub fn sync_phase(&self) -> Option<SyncPhase> {
        Some(match self {
            AccountPhase::Baseline => SyncPhase::Baseline,
            AccountPhase::InboxFirst { .. } => SyncPhase::InboxFirst,
            AccountPhase::Window => SyncPhase::Window,
            AccountPhase::Backoff { from, .. } => return from.sync_phase(),
            _ => return None,
        })
    }

    /// Phases that resume where a restarted worker left off, from the stored `syncPhase`.
    pub fn resume_from(phase: SyncPhase) -> AccountPhase {
        match phase {
            SyncPhase::Baseline => AccountPhase::Baseline,
            SyncPhase::InboxFirst => AccountPhase::InboxFirst { done: 0 },
            SyncPhase::Window => AccountPhase::Window,
            SyncPhase::Backfill | SyncPhase::Complete => AccountPhase::Live,
        }
    }

    /// Where a freshly claimed actor starts. The worker sees the account's `status` and whether a
    /// sync cursor was ever committed, not `syncPhase` (DECISIONS D-232): without a cursor the
    /// baseline runs; a `live`/`degraded` account with a cursor resumes `Live` (a delta pass runs at
    /// once, §9.2); anything else re-runs the idempotent InboxFirst and Window phases.
    pub fn resume(status: AccountStatus, has_cursor: bool) -> AccountPhase {
        match status {
            AccountStatus::ReauthRequired => AccountPhase::ReauthRequired,
            AccountStatus::Paused => AccountPhase::Paused,
            AccountStatus::Removing => AccountPhase::Removing,
            _ if !has_cursor => AccountPhase::Authenticating,
            AccountStatus::Live | AccountStatus::Degraded => AccountPhase::Live,
            AccountStatus::Connecting | AccountStatus::InitialSync => AccountPhase::InboxFirst { done: 0 },
        }
    }

    /// `true` while the actor may call the provider (not paused, waiting for reauth or released).
    pub fn is_active(&self) -> bool {
        !matches!(
            self,
            AccountPhase::Claiming
                | AccountPhase::ReauthRequired
                | AccountPhase::Paused
                | AccountPhase::Released
                | AccountPhase::Backoff { .. }
        )
    }
}

/// Commands sent to an account actor.
#[derive(Debug)]
pub enum ActorCmd {
    /// `worker.pendingOpAccounts` says work is due.
    ApplyOps,
    /// A push arrived (Gmail carries a history id).
    PushHint {
        /// Gmail history id from the push, if any.
        history_id: Option<u64>,
    },
    /// An IMAP folder changed.
    FolderChanged(String),
    /// Fetch and upsert provider search hits, then reply with their Convex ids.
    SearchFetch(Vec<RemoteThreadId>, tokio::sync::oneshot::Sender<Vec<ThreadId>>),
    /// Renew the provider watch.
    RenewWatch,
    /// Finish the current step, release the lease and exit.
    Shutdown,
}

/// Kinds of work an actor can do, in priority order (lower value runs first, §9.2).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum WorkKind {
    /// 1. Shutdown.
    Shutdown = 1,
    /// 2. Apply ops.
    ApplyOps = 2,
    /// 3. Push and delta processing.
    Delta = 3,
    /// 4. Search fetch.
    SearchFetch = 4,
    /// 5. An InboxFirst or Window step.
    InitialStep = 5,
    /// 6. A backfill step.
    BackfillStep = 6,
}

/// Pending work flags of one actor. Every unit of work is one bounded step (one provider page
/// plus one Convex mutation), so priorities preempt between steps.
#[derive(Clone, Debug, Default)]
pub struct WorkQueue {
    /// Shutdown requested.
    pub shutdown: bool,
    /// Ops are due.
    pub ops_due: bool,
    /// A delta pass is due (push, poll, safety poll, folder change).
    pub delta_due: bool,
    /// Search fetches waiting.
    pub search_fetches: usize,
    /// Backfill has not reached the horizon.
    pub backfill_remaining: bool,
}

impl WorkQueue {
    /// Records a command.
    pub fn note(&mut self, cmd: &ActorCmd) {
        match cmd {
            ActorCmd::Shutdown => self.shutdown = true,
            ActorCmd::ApplyOps => self.ops_due = true,
            ActorCmd::PushHint { .. } | ActorCmd::FolderChanged(_) | ActorCmd::RenewWatch => self.delta_due = true,
            ActorCmd::SearchFetch(..) => self.search_fetches += 1,
        }
    }

    /// The highest-priority step to run now in `phase`, or `None` to wait for a command or timer.
    pub fn next(&self, phase: &AccountPhase) -> Option<WorkKind> {
        if self.shutdown {
            return Some(WorkKind::Shutdown);
        }
        if !phase.is_active() {
            return None;
        }
        let initial = matches!(phase, AccountPhase::InboxFirst { .. } | AccountPhase::Window);
        let synced_baseline = !matches!(phase, AccountPhase::Authenticating | AccountPhase::Baseline);
        if self.ops_due && synced_baseline {
            return Some(WorkKind::ApplyOps);
        }
        if (self.delta_due && synced_baseline) || matches!(phase, AccountPhase::Resync { .. }) {
            return Some(WorkKind::Delta);
        }
        if self.search_fetches > 0 && synced_baseline {
            return Some(WorkKind::SearchFetch);
        }
        if initial || matches!(phase, AccountPhase::Authenticating | AccountPhase::Baseline) {
            return Some(WorkKind::InitialStep);
        }
        if self.backfill_remaining && matches!(phase, AccountPhase::Live) {
            return Some(WorkKind::BackfillStep);
        }
        None
    }
}

/// Op retry backoff (§5.5): `min(15 min, 2 s · 2^(attempt−1)) · uniform(0.5, 1.0)`.
///
/// `jitter` is a sample from `[0, 1)`, mapped onto the `[0.5, 1.0)` factor; passing it in keeps the
/// function deterministic for tests.
pub fn backoff_ms(attempt: u32, jitter: f64) -> i64 {
    let attempt = attempt.max(1);
    let base = 2 * SECOND_MS;
    let cap = 15 * MINUTE_MS;
    let exp = attempt.saturating_sub(1).min(20);
    let raw = base.saturating_mul(1_i64 << exp).min(cap);
    let factor = 0.5 + 0.5 * jitter.clamp(0.0, 1.0);
    // The product is at most 900_000, well inside f64's exact integer range.
    (raw as f64 * factor).round() as i64
}

/// The backoff as a `Duration`.
pub fn backoff(attempt: u32, jitter: f64) -> Duration {
    Duration::from_millis(u64::try_from(backoff_ms(attempt, jitter)).unwrap_or(0))
}

/// `maxAttempts` per payload kind (§5.5).
pub fn max_attempts(payload_kind: &str) -> u32 {
    if payload_kind == "send" { 5 } else { 10 }
}

/// Verification schedule for an ambiguous send (§5.6): about +30 s, +2 min and +10 min.
pub const SEND_VERIFY_DELAYS_MS: [i64; 3] = [30 * SECOND_MS, 2 * MINUTE_MS, 10 * MINUTE_MS];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn happy_path_reaches_live() {
        let mut p = AccountPhase::Claiming;
        for e in [
            PhaseEvent::Claimed,
            PhaseEvent::Authenticated,
            PhaseEvent::BaselineDone,
            PhaseEvent::InboxPage { threads: 25, more: true },
            PhaseEvent::InboxPage { threads: 100, more: false },
            PhaseEvent::WindowDone,
        ] {
            p = p.next(e);
        }
        assert_eq!(p, AccountPhase::Live);
    }

    #[test]
    fn inbox_first_counts() {
        let p = AccountPhase::InboxFirst { done: 0 }.next(PhaseEvent::InboxPage { threads: 25, more: true });
        assert_eq!(p, AccountPhase::InboxFirst { done: 25 });
        assert_eq!(p.status(), Some(AccountStatus::InitialSync));
        assert_eq!(p.sync_phase(), Some(SyncPhase::InboxFirst));
    }

    #[test]
    fn backoff_resumes_previous_phase() {
        let until = Instant::now();
        let p = AccountPhase::Window.next(PhaseEvent::Transient { until });
        assert!(matches!(p, AccountPhase::Backoff { .. }));
        assert_eq!(p.sync_phase(), Some(SyncPhase::Window));
        assert!(!p.is_active());
        assert_eq!(p.next(PhaseEvent::BackoffElapsed), AccountPhase::Window);
    }

    #[test]
    fn terminal_and_sticky_states() {
        assert_eq!(AccountPhase::Live.next(PhaseEvent::AuthRejected), AccountPhase::ReauthRequired);
        assert_eq!(AccountPhase::ReauthRequired.next(PhaseEvent::WindowDone), AccountPhase::ReauthRequired);
        assert_eq!(AccountPhase::ReauthRequired.next(PhaseEvent::Reconnected), AccountPhase::Authenticating);
        assert_eq!(AccountPhase::Live.next(PhaseEvent::EntitlementLost), AccountPhase::Paused);
        assert_eq!(AccountPhase::Paused.next(PhaseEvent::EntitlementRegained), AccountPhase::Authenticating);
        assert_eq!(AccountPhase::Live.next(PhaseEvent::LeaseEnded), AccountPhase::Released);
        assert_eq!(AccountPhase::Released.next(PhaseEvent::Claimed), AccountPhase::Released);
        let r = AccountPhase::Live.next(PhaseEvent::CursorExpired(ResyncReason::CursorExpired));
        assert_eq!(r.next(PhaseEvent::ResyncDone), AccountPhase::Live);
    }

    #[test]
    fn priority_order() {
        let mut q = WorkQueue { backfill_remaining: true, ..Default::default() };
        let live = AccountPhase::Live;
        assert_eq!(q.next(&live), Some(WorkKind::BackfillStep));
        q.note(&ActorCmd::FolderChanged("INBOX".into()));
        assert_eq!(q.next(&live), Some(WorkKind::Delta));
        q.note(&ActorCmd::ApplyOps);
        assert_eq!(q.next(&live), Some(WorkKind::ApplyOps));
        q.note(&ActorCmd::Shutdown);
        assert_eq!(q.next(&live), Some(WorkKind::Shutdown));
    }

    #[test]
    fn ops_wait_for_baseline_and_initial_sync_yields_to_ops() {
        let q = WorkQueue { ops_due: true, ..Default::default() };
        assert_eq!(q.next(&AccountPhase::Baseline), Some(WorkKind::InitialStep));
        assert_eq!(q.next(&AccountPhase::InboxFirst { done: 3 }), Some(WorkKind::ApplyOps));
        assert_eq!(WorkQueue::default().next(&AccountPhase::Window), Some(WorkKind::InitialStep));
        assert_eq!(WorkQueue::default().next(&AccountPhase::Paused), None);
    }

    #[test]
    fn resume_from_status_and_cursor() {
        assert_eq!(AccountPhase::resume(AccountStatus::Connecting, false), AccountPhase::Authenticating);
        assert_eq!(AccountPhase::resume(AccountStatus::Live, true), AccountPhase::Live);
        assert_eq!(AccountPhase::resume(AccountStatus::Degraded, true), AccountPhase::Live);
        assert_eq!(AccountPhase::resume(AccountStatus::InitialSync, true), AccountPhase::InboxFirst { done: 0 });
        assert_eq!(AccountPhase::resume(AccountStatus::ReauthRequired, true), AccountPhase::ReauthRequired);
        assert_eq!(AccountPhase::resume(AccountStatus::Removing, false), AccountPhase::Removing);
    }

    #[test]
    fn backoff_follows_formula() {
        assert_eq!(backoff_ms(1, 1.0), 2_000);
        assert_eq!(backoff_ms(1, 0.0), 1_000);
        assert_eq!(backoff_ms(3, 1.0), 8_000);
        assert_eq!(backoff_ms(30, 1.0), 900_000);
        assert_eq!(backoff_ms(30, 0.0), 450_000);
        assert_eq!(max_attempts("send"), 5);
        assert_eq!(max_attempts("thread"), 10);
    }
}
