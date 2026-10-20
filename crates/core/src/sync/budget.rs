//! Provider budgets (ARCHITECTURE §9.4).
//!
//! - **Gmail**: each account gets a token bucket of 5,000 of the 6,000 quota units a user may spend
//!   per minute for sync work, with the remaining 1,000 kept for the interactive body/part path. A
//!   process-wide bucket of 1,000,000 units per minute keeps all accounts under the 1.2 M
//!   per-project limit. While the user is active, backfill may use at most 30% of the account's
//!   sync budget.
//! - **Graph**: at most 4 concurrent requests and 16 requests per second per mailbox.
//! - **IMAP**: one command connection and one IDLE connection per account (enforced by the adapter).
//!
//! Everything here is pure and takes `now` explicitly, so tests are deterministic. Unit costs are
//! from the published Gmail quota table and still need the live check of spike S7.

use crate::time::{MINUTE_MS, SECOND_MS, UnixMs};

/// Gmail quota units a user may spend per minute.
pub const GMAIL_USER_UNITS_PER_MIN: u32 = 6_000;
/// Units per minute an account's sync work may spend.
pub const GMAIL_SYNC_UNITS_PER_MIN: u32 = 5_000;
/// Units per minute kept for interactive body and part reads.
pub const GMAIL_INTERACTIVE_UNITS_PER_MIN: u32 = 1_000;
/// Process-wide Gmail units per minute (the project limit is 1.2 M).
pub const GMAIL_PROJECT_UNITS_PER_MIN: u32 = 1_000_000;
/// Share of the sync budget backfill may use while the user is active.
pub const BACKFILL_ACTIVE_SHARE: f64 = 0.30;
/// The user counts as active for this long after an interactive request or an op.
pub const USER_ACTIVE_WINDOW_MS: i64 = 5 * MINUTE_MS;
/// Graph: concurrent requests per mailbox.
pub const GRAPH_MAX_CONCURRENT: usize = 4;
/// Graph: requests per second per mailbox.
pub const GRAPH_MAX_PER_SEC: u32 = 16;

/// Gmail API calls and their quota cost in units (published table; spike S7).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum GmailCall {
    /// `users.threads.get`.
    ThreadsGet,
    /// `users.threads.list`.
    ThreadsList,
    /// `users.threads.modify`, `trash`, `untrash`.
    ThreadsModify,
    /// `users.messages.get` (raw, attachments).
    MessagesGet,
    /// `users.messages.list`.
    MessagesList,
    /// `users.messages.batchModify`.
    MessagesBatchModify,
    /// `users.history.list`.
    HistoryList,
    /// `users.labels.list` and `get`.
    LabelsRead,
    /// `users.labels.create`, `update`, `delete`.
    LabelsWrite,
    /// `users.drafts.create` and `update`.
    DraftsWrite,
    /// `users.drafts.delete`.
    DraftsDelete,
    /// `users.drafts.send`.
    DraftsSend,
    /// `users.watch`.
    Watch,
    /// `users.getProfile`.
    GetProfile,
}

impl GmailCall {
    /// Quota units of one call.
    pub fn units(self) -> u32 {
        match self {
            GmailCall::ThreadsGet | GmailCall::ThreadsList | GmailCall::ThreadsModify => 10,
            GmailCall::MessagesGet | GmailCall::MessagesList => 5,
            GmailCall::MessagesBatchModify => 50,
            GmailCall::HistoryList => 2,
            GmailCall::LabelsRead | GmailCall::GetProfile => 1,
            GmailCall::LabelsWrite => 5,
            GmailCall::DraftsWrite => 10,
            GmailCall::DraftsDelete => 10,
            GmailCall::DraftsSend => 100,
            GmailCall::Watch => 100,
        }
    }
}

/// A token bucket that refills continuously.
#[derive(Clone, Debug, PartialEq)]
pub struct TokenBucket {
    capacity: f64,
    tokens: f64,
    refill_per_ms: f64,
    last: UnixMs,
}

impl TokenBucket {
    /// A full bucket that refills `capacity` tokens every `period_ms`.
    pub fn new(capacity: u32, period_ms: i64, now: UnixMs) -> Self {
        let capacity = f64::from(capacity);
        let period = period_ms.max(1) as f64;
        Self { capacity, tokens: capacity, refill_per_ms: capacity / period, last: now }
    }

    /// A full bucket that refills `capacity` tokens per minute.
    pub fn per_minute(capacity: u32, now: UnixMs) -> Self {
        Self::new(capacity, MINUTE_MS, now)
    }

    fn refill(&mut self, now: UnixMs) {
        if now > self.last {
            let elapsed = (now - self.last) as f64;
            self.tokens = (self.tokens + elapsed * self.refill_per_ms).min(self.capacity);
            self.last = now;
        }
    }

    /// Tokens available at `now`.
    pub fn available(&mut self, now: UnixMs) -> f64 {
        self.refill(now);
        self.tokens
    }

    /// Milliseconds until `cost` tokens are available (0 when they are now). A cost above the
    /// capacity is clamped to the capacity so it can still run once the bucket is full.
    pub fn wait_ms(&mut self, cost: u32, now: UnixMs) -> i64 {
        self.refill(now);
        let cost = f64::from(cost).min(self.capacity);
        if self.tokens >= cost {
            return 0;
        }
        ((cost - self.tokens) / self.refill_per_ms).ceil() as i64
    }

    /// Takes `cost` tokens, or says how long to wait.
    pub fn try_take(&mut self, cost: u32, now: UnixMs) -> Result<(), i64> {
        match self.wait_ms(cost, now) {
            0 => {
                self.tokens -= f64::from(cost).min(self.capacity);
                Ok(())
            }
            wait => Err(wait),
        }
    }

    /// Empties the bucket (after the provider said we are over the limit).
    pub fn drain(&mut self, now: UnixMs) {
        self.refill(now);
        self.tokens = 0.0;
    }
}

/// What a unit of sync work is for; decides which bucket pays for it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WorkClass {
    /// Body and part reads for a waiting user.
    Interactive,
    /// Ops, deltas, initial sync.
    Sync,
    /// Backfill below the window.
    Backfill,
}

/// The per-account budget.
#[derive(Clone, Debug)]
pub struct AccountBudget {
    sync: TokenBucket,
    interactive: TokenBucket,
    backfill_active: TokenBucket,
    last_user_activity: Option<UnixMs>,
}

impl AccountBudget {
    /// Gmail budgets (§9.4).
    pub fn gmail(now: UnixMs) -> Self {
        let backfill = (f64::from(GMAIL_SYNC_UNITS_PER_MIN) * BACKFILL_ACTIVE_SHARE) as u32;
        Self {
            sync: TokenBucket::per_minute(GMAIL_SYNC_UNITS_PER_MIN, now),
            interactive: TokenBucket::per_minute(GMAIL_INTERACTIVE_UNITS_PER_MIN, now),
            backfill_active: TokenBucket::per_minute(backfill, now),
            last_user_activity: None,
        }
    }

    /// Request-count budgets for Graph (16/s) and IMAP (generous; the connection limit is the
    /// real bound). Costs are counted in requests.
    pub fn requests_per_second(per_sec: u32, now: UnixMs) -> Self {
        let per_min = per_sec.saturating_mul(60);
        let backfill = (f64::from(per_min) * BACKFILL_ACTIVE_SHARE) as u32;
        Self {
            sync: TokenBucket::per_minute(per_min, now),
            interactive: TokenBucket::new(per_sec, SECOND_MS, now),
            backfill_active: TokenBucket::per_minute(backfill.max(1), now),
            last_user_activity: None,
        }
    }

    /// Records user activity (a body fetch, an op, a search).
    pub fn note_user_activity(&mut self, now: UnixMs) {
        self.last_user_activity = Some(now);
    }

    /// Whether the user counts as active.
    pub fn user_active(&self, now: UnixMs) -> bool {
        self.last_user_activity.is_some_and(|t| now - t < USER_ACTIVE_WINDOW_MS)
    }

    /// Charges `cost` units to the bucket for `class`, or says how long to wait. Backfill pays
    /// from the sync bucket and, while the user is active, also from the 30% backfill bucket.
    pub fn try_spend(&mut self, class: WorkClass, cost: u32, now: UnixMs) -> Result<(), i64> {
        match class {
            WorkClass::Interactive => self.interactive.try_take(cost, now),
            WorkClass::Sync => self.sync.try_take(cost, now),
            WorkClass::Backfill => {
                let active = self.user_active(now);
                let wait_share = if active { self.backfill_active.wait_ms(cost, now) } else { 0 };
                let wait_sync = self.sync.wait_ms(cost, now);
                let wait = wait_share.max(wait_sync);
                if wait > 0 {
                    return Err(wait);
                }
                if active {
                    let _ = self.backfill_active.try_take(cost, now);
                }
                self.sync.try_take(cost, now)
            }
        }
    }

    /// The provider rate-limited us: stop spending sync units for a while.
    pub fn exhausted(&mut self, now: UnixMs) {
        self.sync.drain(now);
        self.backfill_active.drain(now);
    }
}

/// Estimated cost of one sync step that lists one page and fetches `threads` threads.
pub fn gmail_page_cost(threads: usize) -> u32 {
    let n = u32::try_from(threads).unwrap_or(u32::MAX);
    GmailCall::ThreadsList.units().saturating_add(n.saturating_mul(GmailCall::ThreadsGet.units()))
}

/// Estimated cost of one delta step that touched `threads` threads.
pub fn gmail_delta_cost(threads: usize) -> u32 {
    let n = u32::try_from(threads).unwrap_or(u32::MAX);
    GmailCall::HistoryList.units().saturating_add(n.saturating_mul(GmailCall::ThreadsGet.units()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bucket_refills_continuously() {
        let mut b = TokenBucket::per_minute(6_000, 0);
        assert!(b.try_take(6_000, 0).is_ok());
        assert_eq!(b.try_take(100, 0), Err(1_000));
        // 100 units per second.
        assert!(b.try_take(100, 1_000).is_ok());
        assert_eq!(b.available(61_000).round() as i64, 6_000);
        // A cost above capacity waits for a full bucket, then runs.
        let mut small = TokenBucket::per_minute(10, 0);
        assert!(small.try_take(50, 0).is_ok());
    }

    #[test]
    fn gmail_budget_splits_interactive_from_sync() {
        let mut b = AccountBudget::gmail(0);
        assert!(b.try_spend(WorkClass::Sync, 5_000, 0).is_ok());
        assert!(b.try_spend(WorkClass::Sync, 10, 0).is_err());
        // Interactive reads still have their reserved 1,000 units.
        assert!(b.try_spend(WorkClass::Interactive, 1_000, 0).is_ok());
        assert!(b.try_spend(WorkClass::Interactive, 5, 0).is_err());
    }

    #[test]
    fn backfill_is_capped_at_30_percent_while_active() {
        let mut b = AccountBudget::gmail(0);
        b.note_user_activity(0);
        assert!(b.user_active(1));
        assert!(b.try_spend(WorkClass::Backfill, 1_500, 1).is_ok());
        let wait = b.try_spend(WorkClass::Backfill, 100, 1).unwrap_err();
        assert!(wait > 0);
        // Ops still run from what is left of the sync bucket.
        assert!(b.try_spend(WorkClass::Sync, 3_000, 1).is_ok());
        // Idle user: backfill may use the whole sync bucket.
        let mut idle = AccountBudget::gmail(0);
        assert!(!idle.user_active(USER_ACTIVE_WINDOW_MS));
        assert!(idle.try_spend(WorkClass::Backfill, 5_000, 0).is_ok());
    }

    #[test]
    fn exhausted_drains_sync() {
        let mut b = AccountBudget::requests_per_second(GRAPH_MAX_PER_SEC, 0);
        assert!(b.try_spend(WorkClass::Sync, 1, 0).is_ok());
        b.exhausted(0);
        assert!(b.try_spend(WorkClass::Sync, 1, 0).is_err());
        assert!(b.try_spend(WorkClass::Interactive, 1, 0).is_ok());
    }

    #[test]
    fn costs() {
        assert_eq!(gmail_page_cost(100), 1_010);
        assert_eq!(gmail_delta_cost(3), 32);
        assert_eq!(GmailCall::DraftsSend.units(), 100);
    }
}
