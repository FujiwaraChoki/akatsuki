//! Managed triage (ARCHITECTURE §12.5, DECISIONS D-390).
//!
//! For an account whose owner's triage route is `akatsuki` (`serverTriage`), the account actor
//! hands the inbox threads of every `worker.upsertThreads` `newInbound` to a per-account
//! [`Triager`] task. It claims them with `worker.claimTriage` (headers and snippet only, 2-minute
//! lease), sends batches of at most 25 to apps/api `POST /internal/v1/triage` with a worker
//! service JWT (`aud: "akatsuki-api"`), and writes the answers with `worker.applyTriage`. A batch
//! that fails stays leased until the lease expires; a periodic sweep (`claimTriage` without ids)
//! picks it up again. The task runs beside the actor so a slow model call never delays sync.

use std::collections::BTreeSet;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use akatsuki_convex_api::WorkerApi;
use akatsuki_core::auth::{API_INTERNAL_AUDIENCE, ServiceTokenMinter};
use akatsuki_core::convex::rows::TriageInput;
use akatsuki_core::convex::worker::{ApplyTriageArgs, TriageResult, WorkerClaimTriageArgs};
use akatsuki_core::ids::{AccountId, ThreadId};
use akatsuki_core::wire::api::{InternalTriageRequest, InternalTriageResponse};
use async_trait::async_trait;
use tokio::sync::mpsc;
use tokio::time::Instant;

/// Items per apps/api call (§8, §12.7 rule 2).
pub const TRIAGE_BATCH: usize = 25;
/// Thread ids waiting for a claim are capped; the sweep finds anything dropped here.
pub const MAX_PENDING: usize = 500;
/// Regular sweep for untriaged and lease-expired threads.
pub const SWEEP_EVERY: Duration = Duration::from_secs(15 * 60);
/// After a failure the next sweep waits for the 2-minute lease to run out.
pub const RETRY_AFTER: Duration = Duration::from_secs(150);
/// Batches per sweep.
pub const SWEEP_BATCHES: usize = 4;

/// Why a triage call failed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TriageError {
    /// Budget, entitlement or rate limit: stop asking for a while.
    Paused(Duration),
    /// Anything else (apps/api down, model error, bad answer): retry with the sweep.
    Failed,
}

/// apps/api's internal triage endpoint.
#[async_trait]
pub trait TriageApi: Send + Sync + std::fmt::Debug {
    /// Triages up to 25 items for `user_id` (whose budget is charged).
    async fn triage(&self, user_id: &str, items: &[TriageInput]) -> Result<Vec<TriageResult>, TriageError>;
}

/// `POST {API_INTERNAL_URL}/internal/v1/triage` with a worker service token.
pub struct HttpTriage {
    http: reqwest::Client,
    url: String,
    minter: Arc<ServiceTokenMinter>,
}

impl std::fmt::Debug for HttpTriage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HttpTriage").finish_non_exhaustive()
    }
}

impl HttpTriage {
    /// A client for apps/api at `base` (`API_INTERNAL_URL`).
    pub fn new(http: reqwest::Client, base: &str, minter: Arc<ServiceTokenMinter>) -> Self {
        Self { http, url: format!("{}/internal/v1/triage", base.trim_end_matches('/')), minter }
    }
}

fn retry_after(headers: &reqwest::header::HeaderMap) -> Option<Duration> {
    headers
        .get(reqwest::header::RETRY_AFTER)?
        .to_str()
        .ok()?
        .trim()
        .parse::<u64>()
        .ok()
        .map(|s| Duration::from_secs(s.clamp(1, 24 * 3600)))
}

#[async_trait]
impl TriageApi for HttpTriage {
    async fn triage(&self, user_id: &str, items: &[TriageInput]) -> Result<Vec<TriageResult>, TriageError> {
        let token = self.minter.mint(API_INTERNAL_AUDIENCE).map_err(|_| TriageError::Failed)?;
        let body = InternalTriageRequest { user_id: user_id.to_owned(), items: items.to_vec() };
        let res = self
            .http
            .post(&self.url)
            .bearer_auth(token)
            .json(&body)
            .timeout(Duration::from_secs(60))
            .send()
            .await
            .map_err(|_| TriageError::Failed)?;
        let status = res.status().as_u16();
        match status {
            200 => {
                let r: InternalTriageResponse = res.json().await.map_err(|_| TriageError::Failed)?;
                Ok(r.results)
            }
            // not_entitled: managed AI ended; the account update will turn serverTriage off.
            402 => Err(TriageError::Paused(Duration::from_secs(3600))),
            // rate_limited or ai_budget_exhausted.
            429 => Err(TriageError::Paused(retry_after(res.headers()).unwrap_or(Duration::from_secs(300)))),
            _ => Err(TriageError::Failed),
        }
    }
}

/// What the actor keeps: a sender for `newInbound` inbox threads and the `serverTriage` switch.
#[derive(Debug)]
pub struct TriageHandle {
    tx: mpsc::UnboundedSender<Vec<ThreadId>>,
    enabled: Arc<AtomicBool>,
    task: tokio::task::JoinHandle<()>,
}

impl TriageHandle {
    /// Queues threads for triage (ignored while `serverTriage` is off).
    pub fn submit(&self, ids: Vec<ThreadId>) {
        if !ids.is_empty() && self.enabled.load(Ordering::Relaxed) {
            let _ = self.tx.send(ids);
        }
    }

    /// Follows `worker.assignedAccounts.serverTriage`.
    pub fn set_enabled(&self, on: bool) {
        let was = self.enabled.swap(on, Ordering::Relaxed);
        if on && !was {
            // Turned on: sweep what arrived while it was off.
            let _ = self.tx.send(Vec::new());
        }
    }
}

impl Drop for TriageHandle {
    fn drop(&mut self) {
        self.task.abort();
    }
}

/// The per-account triage task.
pub struct Triager {
    api: Arc<dyn WorkerApi>,
    client: Arc<dyn TriageApi>,
    account_id: AccountId,
    worker_id: String,
    user_id: String,
    enabled: Arc<AtomicBool>,
}

impl std::fmt::Debug for Triager {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Triager").field("account_id", &self.account_id).finish_non_exhaustive()
    }
}

/// What one round ended with.
#[derive(Debug, PartialEq, Eq)]
enum Round {
    Done,
    Paused(Duration),
    Failed,
    /// The lease is gone (or Convex refused): the actor is about to stop.
    Stop,
}

impl Triager {
    /// Starts the task. `enabled` is the account's current `serverTriage`.
    pub fn spawn(
        api: Arc<dyn WorkerApi>,
        client: Arc<dyn TriageApi>,
        account_id: AccountId,
        worker_id: String,
        user_id: String,
        enabled: bool,
    ) -> TriageHandle {
        let (tx, rx) = mpsc::unbounded_channel();
        let enabled = Arc::new(AtomicBool::new(enabled));
        let t = Triager { api, client, account_id, worker_id, user_id, enabled: enabled.clone() };
        let task = tokio::spawn(t.run(rx));
        TriageHandle { tx, enabled, task }
    }

    async fn run(self, mut rx: mpsc::UnboundedReceiver<Vec<ThreadId>>) {
        let mut pending: BTreeSet<ThreadId> = BTreeSet::new();
        // A restarted worker sweeps once shortly after the claim (threads left pending by the last one).
        let mut sweep_at = Instant::now() + Duration::from_secs(5);
        let mut paused_until: Option<Instant> = None;
        loop {
            // Wait unless there is work that may run now.
            if pending.is_empty() || paused_until.is_some() {
                let wake = paused_until.unwrap_or(sweep_at);
                tokio::select! {
                    m = rx.recv() => match m {
                        Some(ids) => {
                            if ids.is_empty() {
                                sweep_at = Instant::now();
                            }
                            add(&mut pending, ids);
                        }
                        None => return,
                    },
                    () = tokio::time::sleep_until(wake) => {}
                }
            }
            while let Ok(more) = rx.try_recv() {
                add(&mut pending, more);
            }
            let now = Instant::now();
            if paused_until.is_some_and(|p| now < p) {
                continue;
            }
            paused_until = None;
            if !self.enabled.load(Ordering::Relaxed) {
                pending.clear();
                sweep_at = now + SWEEP_EVERY;
                continue;
            }
            let round = if pending.is_empty() {
                if now < sweep_at {
                    continue;
                }
                sweep_at = now + SWEEP_EVERY;
                self.sweep().await
            } else {
                let ids: Vec<ThreadId> = std::mem::take(&mut pending).into_iter().collect();
                self.claimed(&ids).await
            };
            match round {
                Round::Done => {}
                Round::Failed => sweep_at = sweep_at.min(now + RETRY_AFTER),
                Round::Paused(d) => {
                    paused_until = Some(now + d);
                    sweep_at = sweep_at.min(now + d);
                }
                Round::Stop => return,
            }
        }
    }

    /// Claims and triages the given `newInbound` threads.
    async fn claimed(&self, ids: &[ThreadId]) -> Round {
        for chunk in ids.chunks(TRIAGE_BATCH) {
            match self.batch(Some(chunk.to_vec())).await {
                (Round::Done, _) => {}
                (other, _) => return other,
            }
        }
        Round::Done
    }

    /// Claims untriaged or lease-expired threads of the account, a few batches at a time.
    async fn sweep(&self) -> Round {
        for _ in 0..SWEEP_BATCHES {
            match self.batch(None).await {
                (Round::Done, n) if n == TRIAGE_BATCH => {}
                (Round::Done, _) => return Round::Done,
                (other, _) => return other,
            }
        }
        Round::Done
    }

    async fn batch(&self, thread_ids: Option<Vec<ThreadId>>) -> (Round, usize) {
        let items = match self
            .api
            .claim_managed_triage(&WorkerClaimTriageArgs {
                account_id: self.account_id.clone(),
                worker_id: self.worker_id.clone(),
                thread_ids,
                limit: TRIAGE_BATCH as u32,
            })
            .await
        {
            Ok(items) => items,
            Err(e) if e.is_lease_lost() => return (Round::Stop, 0),
            Err(_) => return (Round::Failed, 0),
        };
        let n = items.len();
        if n == 0 {
            return (Round::Done, 0);
        }
        let results = match self.client.triage(&self.user_id, &items).await {
            Ok(r) => r,
            Err(TriageError::Paused(d)) => return (Round::Paused(d), n),
            Err(TriageError::Failed) => {
                tracing::warn!(account = %self.account_id, items = n, "managed triage failed; retrying later");
                return (Round::Failed, n);
            }
        };
        // Only answers for threads this batch claimed are written.
        let claimed: BTreeSet<&ThreadId> = items.iter().map(|i| &i.thread_id).collect();
        let results: Vec<TriageResult> = results
            .into_iter()
            .filter(|r| claimed.contains(&r.thread_id))
            .map(|mut r| {
                r.model.truncate(200);
                r
            })
            .collect();
        if results.is_empty() {
            return (Round::Failed, n);
        }
        match self.api.apply_triage(&ApplyTriageArgs { results }).await {
            Ok(()) => (Round::Done, n),
            Err(_) => (Round::Failed, n),
        }
    }
}

fn add(pending: &mut BTreeSet<ThreadId>, ids: Vec<ThreadId>) {
    for id in ids {
        if pending.len() >= MAX_PENDING {
            return;
        }
        pending.insert(id);
    }
}

/// The apps/api client when `API_INTERNAL_URL` and the worker service key are configured.
pub fn http_triage(
    config: &crate::config::Config,
    http: reqwest::Client,
    worker_id: &str,
) -> Option<Arc<dyn TriageApi>> {
    let base = config.api_internal_url.as_deref()?;
    let (Some(pem), Some(kid), Some(issuer)) = (
        config.worker_service_private_key.as_deref(),
        config.worker_service_kid.as_deref(),
        config.worker_issuer.as_deref(),
    ) else {
        return None;
    };
    let minter = ServiceTokenMinter::from_pem(pem, kid, issuer, format!("worker:{worker_id}")).ok()?;
    Some(Arc::new(HttpTriage::new(http, base, Arc::new(minter))))
}
