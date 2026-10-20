//! Gmail quota budgeting (ARCHITECTURE §9.4, spike S7).
//!
//! Every call is charged its unit cost before it is sent:
//!
//! - one **account** budget split into two lanes: `sync` (the account actor: deltas, windows,
//!   ops, sends; 5,000 units/min) and `interactive` (bodies, parts, drafts autosave, search;
//!   1,000 units/min), together the 6,000 units per user per minute Google allows;
//! - one **process** budget shared by every Gmail account (1,000,000 units/min, under the
//!   1,200,000 per-project limit).
//!
//! The unit costs are those of the current Gmail quota table
//! (<https://developers.google.com/workspace/gmail/api/reference/quota>, last updated
//! 2026-10-16, read 2026-10-20). They changed since the contract was written (`threads.get` is 40,
//! `messages.get` 20, both 3× for full/raw formats), so they are configurable: a deployment can
//! load [`QuotaCosts`] from JSON when Google changes them again (D-201).

use std::sync::Arc;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;
use tokio::time::Instant;

/// Which lane of the account budget a call uses.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Lane {
    /// The account actor (sync, ops, sends).
    Sync,
    /// The interactive body/part/draft/search path.
    Interactive,
}

/// A Gmail API method, for costing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Method {
    /// `users.getProfile`.
    GetProfile,
    /// `users.watch`.
    Watch,
    /// `users.stop`.
    Stop,
    /// `history.list`.
    HistoryList,
    /// `labels.list`.
    LabelsList,
    /// `labels.create`.
    LabelsCreate,
    /// `labels.update` / `labels.patch`.
    LabelsUpdate,
    /// `labels.delete`.
    LabelsDelete,
    /// `threads.list`.
    ThreadsList,
    /// `threads.get` (`full`: format `full`).
    ThreadsGet {
        /// Format `full`.
        full: bool,
    },
    /// `threads.modify`.
    ThreadsModify,
    /// `threads.trash`.
    ThreadsTrash,
    /// `threads.untrash`.
    ThreadsUntrash,
    /// `threads.delete`.
    ThreadsDelete,
    /// `messages.list`.
    MessagesList,
    /// `messages.get` (`full`: format `full`, `raw` or `html`).
    MessagesGet {
        /// Format `full`, `raw` or `html`.
        full: bool,
    },
    /// `messages.batchModify`.
    MessagesBatchModify,
    /// `messages.send`.
    MessagesSend,
    /// `messages.attachments.get`.
    AttachmentsGet,
    /// `drafts.create`.
    DraftsCreate,
    /// `drafts.update`.
    DraftsUpdate,
    /// `drafts.delete`.
    DraftsDelete,
    /// `drafts.send`.
    DraftsSend,
    /// `drafts.get` (`full`: format `full`, `raw` or `html`).
    DraftsGet {
        /// Format `full`, `raw` or `html`.
        full: bool,
    },
    /// `settings.sendAs.list`.
    SendAsList,
}

/// Unit cost per method. Defaults from the Gmail quota table of 2026-10-16.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct QuotaCosts {
    /// `getProfile`.
    pub get_profile: u32,
    /// `watch`.
    pub watch: u32,
    /// `stop`.
    pub stop: u32,
    /// `history.list`.
    pub history_list: u32,
    /// `labels.list`.
    pub labels_list: u32,
    /// `labels.create`.
    pub labels_create: u32,
    /// `labels.update`.
    pub labels_update: u32,
    /// `labels.delete`.
    pub labels_delete: u32,
    /// `threads.list`.
    pub threads_list: u32,
    /// `threads.get` (metadata/minimal).
    pub threads_get: u32,
    /// `threads.get` format `full`.
    pub threads_get_full: u32,
    /// `threads.modify`.
    pub threads_modify: u32,
    /// `threads.trash`.
    pub threads_trash: u32,
    /// `threads.untrash`.
    pub threads_untrash: u32,
    /// `threads.delete`.
    pub threads_delete: u32,
    /// `messages.list`.
    pub messages_list: u32,
    /// `messages.get` (metadata/minimal).
    pub messages_get: u32,
    /// `messages.get` format `full`/`raw`/`html`.
    pub messages_get_full: u32,
    /// `messages.batchModify`.
    pub messages_batch_modify: u32,
    /// `messages.send`.
    pub messages_send: u32,
    /// `messages.attachments.get`.
    pub attachments_get: u32,
    /// `drafts.create`.
    pub drafts_create: u32,
    /// `drafts.update`.
    pub drafts_update: u32,
    /// `drafts.delete`.
    pub drafts_delete: u32,
    /// `drafts.send`.
    pub drafts_send: u32,
    /// `drafts.get` (metadata/minimal).
    pub drafts_get: u32,
    /// `drafts.get` format `full`/`raw`/`html`.
    pub drafts_get_full: u32,
    /// `settings.sendAs.list`.
    pub send_as_list: u32,
}

impl Default for QuotaCosts {
    fn default() -> Self {
        Self {
            get_profile: 1,
            watch: 100,
            stop: 50,
            history_list: 2,
            labels_list: 1,
            labels_create: 5,
            labels_update: 5,
            labels_delete: 5,
            threads_list: 10,
            threads_get: 40,
            threads_get_full: 120,
            threads_modify: 10,
            threads_trash: 20,
            threads_untrash: 10,
            threads_delete: 20,
            messages_list: 5,
            messages_get: 20,
            messages_get_full: 60,
            messages_batch_modify: 50,
            messages_send: 100,
            attachments_get: 20,
            drafts_create: 10,
            drafts_update: 15,
            drafts_delete: 10,
            drafts_send: 100,
            drafts_get: 20,
            drafts_get_full: 60,
            send_as_list: 1,
        }
    }
}

impl QuotaCosts {
    /// The cost of one call.
    pub fn cost(&self, m: Method) -> u32 {
        match m {
            Method::GetProfile => self.get_profile,
            Method::Watch => self.watch,
            Method::Stop => self.stop,
            Method::HistoryList => self.history_list,
            Method::LabelsList => self.labels_list,
            Method::LabelsCreate => self.labels_create,
            Method::LabelsUpdate => self.labels_update,
            Method::LabelsDelete => self.labels_delete,
            Method::ThreadsList => self.threads_list,
            Method::ThreadsGet { full: false } => self.threads_get,
            Method::ThreadsGet { full: true } => self.threads_get_full,
            Method::ThreadsModify => self.threads_modify,
            Method::ThreadsTrash => self.threads_trash,
            Method::ThreadsUntrash => self.threads_untrash,
            Method::ThreadsDelete => self.threads_delete,
            Method::MessagesList => self.messages_list,
            Method::MessagesGet { full: false } => self.messages_get,
            Method::MessagesGet { full: true } => self.messages_get_full,
            Method::MessagesBatchModify => self.messages_batch_modify,
            Method::MessagesSend => self.messages_send,
            Method::AttachmentsGet => self.attachments_get,
            Method::DraftsCreate => self.drafts_create,
            Method::DraftsUpdate => self.drafts_update,
            Method::DraftsDelete => self.drafts_delete,
            Method::DraftsSend => self.drafts_send,
            Method::DraftsGet { full: false } => self.drafts_get,
            Method::DraftsGet { full: true } => self.drafts_get_full,
            Method::SendAsList => self.send_as_list,
        }
    }
}

/// Per-minute budgets.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct QuotaLimits {
    /// Account actor lane (units per minute).
    pub account_sync_per_minute: u32,
    /// Interactive lane (units per minute).
    pub account_interactive_per_minute: u32,
    /// Process-wide budget for all Gmail accounts (units per minute).
    pub process_per_minute: u32,
}

impl Default for QuotaLimits {
    fn default() -> Self {
        // Google: 6,000 units per user per minute; 1,200,000 per project per minute.
        Self { account_sync_per_minute: 5_000, account_interactive_per_minute: 1_000, process_per_minute: 1_000_000 }
    }
}

/// A token bucket refilled continuously at `capacity` per minute.
#[derive(Debug)]
pub struct Bucket {
    capacity: f64,
    state: Mutex<BucketState>,
}

#[derive(Debug)]
struct BucketState {
    tokens: f64,
    at: Instant,
}

impl Bucket {
    /// A full bucket of `per_minute` units.
    pub fn per_minute(per_minute: u32) -> Self {
        let capacity = f64::from(per_minute.max(1));
        Self { capacity, state: Mutex::new(BucketState { tokens: capacity, at: Instant::now() }) }
    }

    fn refill(&self, s: &mut BucketState, now: Instant) {
        let elapsed = now.saturating_duration_since(s.at).as_secs_f64();
        s.tokens = (s.tokens + elapsed * self.capacity / 60.0).min(self.capacity);
        s.at = now;
    }

    /// Takes `units` now if they are available, else returns how long to wait.
    pub async fn try_take(&self, units: u32) -> Result<(), Duration> {
        let units = f64::from(units).min(self.capacity);
        let mut s = self.state.lock().await;
        self.refill(&mut s, Instant::now());
        if s.tokens >= units {
            s.tokens -= units;
            Ok(())
        } else {
            let missing = units - s.tokens;
            Err(Duration::from_secs_f64(missing * 60.0 / self.capacity).max(Duration::from_millis(1)))
        }
    }

    /// Waits until `units` are available and takes them. Costs above the capacity are clamped.
    pub async fn take(&self, units: u32) {
        while let Err(wait) = self.try_take(units).await {
            tokio::time::sleep(wait).await;
        }
    }

    /// Empties the bucket (after Gmail answered 429), so the next calls wait for a refill.
    pub async fn drain(&self) {
        let mut s = self.state.lock().await;
        self.refill(&mut s, Instant::now());
        s.tokens = 0.0;
    }

    /// Units available now (for metrics and tests).
    pub async fn available(&self) -> f64 {
        let mut s = self.state.lock().await;
        self.refill(&mut s, Instant::now());
        s.tokens
    }
}

/// The process-wide Gmail budget, shared by every account (`Arc`).
#[derive(Debug)]
pub struct ProcessQuota {
    bucket: Bucket,
}

impl ProcessQuota {
    /// A process budget from `limits.process_per_minute`.
    pub fn new(limits: &QuotaLimits) -> Arc<Self> {
        Arc::new(Self { bucket: Bucket::per_minute(limits.process_per_minute) })
    }
}

/// One account's budget plus the shared process budget.
#[derive(Debug)]
pub struct AccountQuota {
    costs: QuotaCosts,
    sync: Bucket,
    interactive: Bucket,
    process: Arc<ProcessQuota>,
}

impl AccountQuota {
    /// A budget for one account.
    pub fn new(costs: QuotaCosts, limits: &QuotaLimits, process: Arc<ProcessQuota>) -> Self {
        Self {
            costs,
            sync: Bucket::per_minute(limits.account_sync_per_minute),
            interactive: Bucket::per_minute(limits.account_interactive_per_minute),
            process,
        }
    }

    /// Default costs and limits with a private process budget (tests, single-account tools).
    pub fn standalone() -> Self {
        let limits = QuotaLimits::default();
        Self::new(QuotaCosts::default(), &limits, ProcessQuota::new(&limits))
    }

    /// The cost table.
    pub fn costs(&self) -> &QuotaCosts {
        &self.costs
    }

    fn lane(&self, lane: Lane) -> &Bucket {
        match lane {
            Lane::Sync => &self.sync,
            Lane::Interactive => &self.interactive,
        }
    }

    /// Charges `units` to the lane and the process budget, waiting as needed.
    pub async fn charge(&self, lane: Lane, units: u32) {
        if units == 0 {
            return;
        }
        self.lane(lane).take(units).await;
        self.process.bucket.take(units).await;
    }

    /// Charges the cost of `count` calls of `method`.
    pub async fn charge_calls(&self, lane: Lane, method: Method, count: usize) {
        let units = self.costs.cost(method).saturating_mul(u32::try_from(count).unwrap_or(u32::MAX));
        self.charge(lane, units).await;
    }

    /// Gmail said 429 for this account: empty both lanes.
    pub async fn penalize(&self) {
        self.sync.drain().await;
        self.interactive.drain().await;
    }

    /// Units left in a lane.
    pub async fn available(&self, lane: Lane) -> f64 {
        self.lane(lane).available().await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_costs_match_the_2026_table() {
        let c = QuotaCosts::default();
        assert_eq!(c.cost(Method::ThreadsGet { full: false }), 40);
        assert_eq!(c.cost(Method::ThreadsGet { full: true }), 120);
        assert_eq!(c.cost(Method::MessagesGet { full: true }), 60);
        assert_eq!(c.cost(Method::HistoryList), 2);
        assert_eq!(c.cost(Method::DraftsSend), 100);
        assert_eq!(c.cost(Method::MessagesBatchModify), 50);
        let l = QuotaLimits::default();
        assert_eq!(l.account_sync_per_minute + l.account_interactive_per_minute, 6_000);
    }

    #[test]
    fn costs_are_configurable_from_json() {
        let c: QuotaCosts = serde_json::from_str(r#"{"threadsGet": 10, "historyList": 1}"#).unwrap();
        assert_eq!(c.threads_get, 10);
        assert_eq!(c.history_list, 1);
        assert_eq!(c.drafts_send, 100, "unspecified costs keep their defaults");
    }

    #[tokio::test(start_paused = true)]
    async fn bucket_waits_for_refill() {
        let limits = QuotaLimits {
            account_sync_per_minute: 600,
            account_interactive_per_minute: 60,
            process_per_minute: 10_000,
        };
        let q = AccountQuota::new(QuotaCosts::default(), &limits, ProcessQuota::new(&limits));
        let start = Instant::now();
        q.charge(Lane::Sync, 600).await;
        assert!(q.available(Lane::Sync).await < 1.0);
        // 600/min = 10 units/s: 50 more units take 5 s.
        q.charge(Lane::Sync, 50).await;
        let waited = start.elapsed();
        assert!(waited >= Duration::from_millis(4_900) && waited <= Duration::from_millis(5_200), "{waited:?}");
        // The interactive lane is separate.
        assert!((q.available(Lane::Interactive).await - 60.0).abs() < 1.0);
        // Over-capacity costs are clamped instead of waiting forever.
        q.charge(Lane::Interactive, 1_000).await;
        q.penalize().await;
        assert!(q.available(Lane::Interactive).await < 1.0);
    }

    #[tokio::test(start_paused = true)]
    async fn process_bucket_is_shared() {
        let limits = QuotaLimits {
            account_sync_per_minute: 6_000,
            account_interactive_per_minute: 6_000,
            process_per_minute: 120,
        };
        let p = ProcessQuota::new(&limits);
        let a = AccountQuota::new(QuotaCosts::default(), &limits, p.clone());
        let b = AccountQuota::new(QuotaCosts::default(), &limits, p);
        a.charge(Lane::Sync, 120).await;
        let start = Instant::now();
        b.charge(Lane::Sync, 20).await; // 120/min = 2 units/s → 10 s
        assert!(start.elapsed() >= Duration::from_secs(9));
    }
}
