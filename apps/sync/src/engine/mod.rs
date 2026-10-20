//! The sync engine driver (ARCHITECTURE §9.2–§9.6, §5.4–§5.6): one serialized account actor per
//! mail account, the supervisor that claims accounts and wakes actors from the Convex
//! subscriptions, and `ConvexSink`.
//!
//! The pure decisions (phases, priorities, budgets, coalescing, the send table, backfill
//! bookkeeping) live in `akatsuki_core::sync`; this module performs them against a
//! `MailProvider` and Convex (DECISIONS D-090, D-230).

pub mod actor;
pub mod ops;
pub mod sink;
pub mod supervisor;

#[cfg(test)]
pub mod fake;
#[cfg(test)]
mod tests;

use std::collections::HashMap;
use std::sync::{Arc, Mutex, RwLock};
use std::time::Instant;

use akatsuki_core::convex::worker::AssignedAccount;
use akatsuki_core::ids::{AccountId, ThreadId};
use akatsuki_core::model::RemoteThreadId;
use akatsuki_core::sync::ActorCmd;
use akatsuki_core::sync::budget::{GMAIL_PROJECT_UNITS_PER_MIN, TokenBucket};
use akatsuki_core::time::{UnixMs, now_ms};
use akatsuki_providers::MailProvider;
use tokio::sync::{mpsc, oneshot};

use crate::config::PushMode;

/// Engine settings shared by every actor.
#[derive(Clone, Debug)]
pub struct EngineConfig {
    /// This worker instance (`workerId`, also the service JWT `sub` suffix).
    pub worker_id: String,
    /// `PUBLIC_BASE_URL`; push hooks live under it.
    pub public_base_url: Option<String>,
    /// `SYNC_PUSH`.
    pub push: PushMode,
    /// `GMAIL_PUBSUB_TOPIC` is set.
    pub gmail_topic: bool,
    /// Shards this process serves (0..SHARD_COUNT).
    pub shard_count: u32,
    /// apps/api's internal triage endpoint (`API_INTERNAL_URL`), for managed triage (§12.5, D-390).
    pub triage: Option<Arc<dyn crate::triage::TriageApi>>,
}

impl EngineConfig {
    /// Settings for tests and for a worker without public URL.
    pub fn new(worker_id: impl Into<String>) -> Self {
        Self {
            worker_id: worker_id.into(),
            public_base_url: None,
            push: PushMode::Off,
            gmail_topic: false,
            shard_count: 1,
            triage: None,
        }
    }
}

/// A Graph lifecycle event (§9.6).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Lifecycle {
    /// `reauthorizationRequired`: renew the subscription.
    Reauthorize,
    /// `subscriptionRemoved`: recreate it and run a delta on all folders.
    Removed,
    /// `missed`: run a delta on all folders.
    Missed,
}

/// What the supervisor, HTTP handlers and push sources send to an actor.
#[derive(Debug)]
pub enum ActorMsg {
    /// A §9.2 command.
    Cmd(ActorCmd),
    /// The account row changed (`worker.assignedAccounts`).
    Account(AssignedAccount),
    /// The supervisor's heartbeat says the lease is gone: stop at once.
    LeaseLost,
    /// A Graph lifecycle notification.
    Lifecycle(Lifecycle),
}

impl ActorMsg {
    /// `SearchFetch` with its reply channel.
    pub fn search(ids: Vec<RemoteThreadId>) -> (Self, oneshot::Receiver<Vec<ThreadId>>) {
        let (tx, rx) = oneshot::channel();
        (ActorMsg::Cmd(ActorCmd::SearchFetch(ids, tx)), rx)
    }
}

/// A running actor as seen from outside: its provider (body and part reads bypass the actor,
/// §9.2) and its mailbox.
pub struct AccountHandle {
    /// The account.
    pub account: RwLock<AssignedAccount>,
    /// The provider.
    pub provider: Arc<dyn MailProvider>,
    /// The actor's mailbox.
    pub tx: mpsc::UnboundedSender<ActorMsg>,
    /// Manual-sync debounce (`POST /v1/accounts/{id}/sync`, 10 s).
    pub last_manual_sync: Mutex<Option<Instant>>,
    /// Last interactive use (bodies, parts, search), for the backfill budget.
    pub last_interactive_at: Mutex<Option<UnixMs>>,
}

impl std::fmt::Debug for AccountHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AccountHandle").finish_non_exhaustive()
    }
}

impl AccountHandle {
    /// Sends a message; `false` when the actor is gone.
    pub fn send(&self, msg: ActorMsg) -> bool {
        self.tx.send(msg).is_ok()
    }

    /// Records interactive use.
    pub fn touch(&self) {
        *self.last_interactive_at.lock().unwrap_or_else(|e| e.into_inner()) = Some(now_ms());
    }

    /// The owner.
    pub fn user_id(&self) -> String {
        self.account.read().unwrap_or_else(|e| e.into_inner()).user_id.clone()
    }

    /// A snapshot of the account row.
    pub fn snapshot(&self) -> AssignedAccount {
        self.account.read().unwrap_or_else(|e| e.into_inner()).clone()
    }
}

/// Running actors by account.
#[derive(Debug, Default)]
pub struct Registry {
    handles: RwLock<HashMap<AccountId, Arc<AccountHandle>>>,
    /// Live Graph subscriptions of the running actors, for the push hook (D-405).
    pub graph: Arc<GraphSubscriptions>,
}

/// Graph subscription id → (account, hex sha256 of its clientState), published by the actors that hold the
/// subscriptions. `POST /hooks/graph` checks a notification against it before anything else, so a forged batch
/// costs a map lookup instead of a Convex query per item (D-405).
#[derive(Debug, Default)]
pub struct GraphSubscriptions {
    by_id: RwLock<HashMap<String, (AccountId, String)>>,
}

impl GraphSubscriptions {
    /// Records the account's current subscription (replacing an earlier one of the same account).
    pub fn set(&self, account_id: &AccountId, subscription_id: &str, client_state_hash: &str) {
        let mut m = self.by_id.write().unwrap_or_else(|e| e.into_inner());
        m.retain(|_, (a, _)| a != account_id);
        m.insert(subscription_id.to_owned(), (account_id.clone(), client_state_hash.to_owned()));
    }

    /// Forgets the account's subscription (watch gone, actor stopped).
    pub fn clear(&self, account_id: &AccountId) {
        self.by_id.write().unwrap_or_else(|e| e.into_inner()).retain(|_, (a, _)| a != account_id);
    }

    /// The account and clientState hash of a live subscription.
    pub fn get(&self, subscription_id: &str) -> Option<(AccountId, String)> {
        self.by_id.read().unwrap_or_else(|e| e.into_inner()).get(subscription_id).cloned()
    }
}

impl Registry {
    /// The handle of a running actor.
    pub fn get(&self, account_id: &AccountId) -> Option<Arc<AccountHandle>> {
        self.handles.read().unwrap_or_else(|e| e.into_inner()).get(account_id).cloned()
    }

    /// Registers a handle.
    pub fn insert(&self, account_id: AccountId, handle: Arc<AccountHandle>) {
        self.handles.write().unwrap_or_else(|e| e.into_inner()).insert(account_id, handle);
    }

    /// Removes a handle (and its Graph subscription entry).
    pub fn remove(&self, account_id: &AccountId) -> Option<Arc<AccountHandle>> {
        self.graph.clear(account_id);
        self.handles.write().unwrap_or_else(|e| e.into_inner()).remove(account_id)
    }

    /// Accounts with a running actor.
    pub fn ids(&self) -> Vec<AccountId> {
        self.handles.read().unwrap_or_else(|e| e.into_inner()).keys().cloned().collect()
    }

    /// Sends to one actor; `false` when none runs.
    pub fn send(&self, account_id: &AccountId, msg: ActorMsg) -> bool {
        self.get(account_id).is_some_and(|h| h.send(msg))
    }

    /// Number of running actors.
    pub fn len(&self) -> usize {
        self.handles.read().unwrap_or_else(|e| e.into_inner()).len()
    }

    /// Whether no actor runs.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// The process-wide Gmail budget and project-scope pause (§9.4, §5.5 `scope: Project`).
#[derive(Debug)]
pub struct ProjectGate {
    inner: Mutex<(TokenBucket, Option<Instant>)>,
}

impl Default for ProjectGate {
    fn default() -> Self {
        Self { inner: Mutex::new((TokenBucket::per_minute(GMAIL_PROJECT_UNITS_PER_MIN, now_ms()), None)) }
    }
}

impl ProjectGate {
    /// Pauses every Gmail actor until `until`.
    pub fn pause_until(&self, until: Instant) {
        let mut g = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        if g.1.is_none_or(|u| u < until) {
            g.1 = Some(until);
        }
    }

    /// When the project pause ends, if one is active.
    pub fn paused_until(&self) -> Option<Instant> {
        let g = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        g.1.filter(|u| *u > Instant::now())
    }

    /// Charges project units; returns the wait in ms when the project budget is spent.
    pub fn try_spend(&self, units: u32) -> Result<(), i64> {
        let mut g = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        g.0.try_take(units, now_ms())
    }
}
