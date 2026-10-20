//! The account supervisor (ARCHITECTURE §9.2): subscribes to `worker.assignedAccounts` and
//! `worker.pendingOpAccounts`, claims each account, loads and decrypts its credentials, builds
//! its adapter, runs one actor per account, renews every lease with `worker.heartbeat`, wakes
//! actors when ops become due, and on shutdown stops claiming, lets actors finish their step and
//! release their leases.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use akatsuki_convex_api::{ConvexApiError, WorkerApi};
use akatsuki_core::convex::values::ConvexBytes;
use akatsuki_core::convex::worker::{
    AccountWorkerArgs, AssignedAccount, ClaimAccountArgs, ClaimAccountResult, GetSecretArgs, HeartbeatArgs,
    PendingOpAccount, PutSecretArgs,
};
use akatsuki_core::convex::{AccountStatus, Provider, SecretKind};
use akatsuki_core::ids::AccountId;
use akatsuki_core::sync::{ActorCmd, HEARTBEAT_EVERY_MS, LEASE_MS};
use akatsuki_core::time::now_ms;
use akatsuki_core::wire::ErrorCode;
use akatsuki_providers::{AccountStatusUpdate, MetaSink, TokenSource};
use futures::StreamExt as _;
use tokio::sync::{mpsc, watch};

use super::actor::{AccountActor, ActorParts, ClaimState, ExitReason};
use super::sink::ConvexSink;
use super::{AccountHandle, ActorMsg, EngineConfig, ProjectGate, Registry};
use crate::crypto::SecretKeyring;
use crate::fetch::SafeFetcher;
use crate::metrics::Metrics;
use crate::oauth::{ImapSecret, OAuthClient, OAuthKind, OAuthSecret, OAuthTokenSource, SecretSlot};
use crate::providers::{Credentials, FactoryError, ProviderContext, ProviderFactory};

/// How often the supervisor re-checks accounts it could not start.
pub const RECONCILE_EVERY: Duration = Duration::from_secs(15);
/// How long actors get to finish their step on shutdown (§9.2: hand over within 5 s).
pub const SHUTDOWN_GRACE: Duration = Duration::from_secs(5);

/// The secret kind of a provider.
pub fn secret_kind(provider: Provider) -> SecretKind {
    match provider {
        Provider::Gmail | Provider::Graph => SecretKind::Oauth,
        Provider::Imap => SecretKind::Imap,
    }
}

/// A decrypted secret.
#[derive(Clone, Debug)]
pub enum LoadedSecret {
    /// OAuth.
    OAuth(OAuthSecret, u64),
    /// IMAP.
    Imap(ImapSecret, u64),
}

/// Why credentials could not be loaded.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SecretError {
    /// No `accountSecrets` row.
    #[error("no secret")]
    Missing,
    /// It could not be decrypted (unknown key id, wrong AAD, tampered).
    #[error("secret unreadable")]
    Unreadable,
    /// Convex failed.
    #[error("convex: {0}")]
    Convex(String),
}

/// Loads and decrypts an account's secret. When it was sealed under an older key and `slot` is
/// given (the caller holds the lease), it is re-sealed under the active key on the way
/// (`rotate-secrets` does the same for every account).
pub async fn load_secret(
    api: &dyn WorkerApi,
    keyring: &SecretKeyring,
    account: &AssignedAccount,
    rotate_as: Option<&str>,
) -> Result<LoadedSecret, SecretError> {
    let kind = secret_kind(account.provider);
    let rec = api
        .get_secret(&GetSecretArgs { account_id: account.account_id.clone(), kind })
        .await
        .map_err(|e| SecretError::Convex(e.to_string()))?
        .ok_or(SecretError::Missing)?;
    let aad = account.secret_aad(kind);
    let plain =
        keyring.open(&rec.key_id, &rec.nonce.0, &rec.ciphertext.0, &aad).map_err(|_| SecretError::Unreadable)?;
    let mut version = rec.version;
    if let Some(worker_id) = rotate_as
        && keyring.needs_rotation(&rec.key_id)
    {
        let sealed = keyring.seal(&aad, &plain).map_err(|_| SecretError::Unreadable)?;
        let put = api
            .put_secret(&PutSecretArgs {
                account_id: account.account_id.clone(),
                worker_id: worker_id.to_owned(),
                kind,
                key_id: sealed.key_id,
                nonce: ConvexBytes(sealed.nonce),
                ciphertext: ConvexBytes(sealed.ciphertext),
                expected_version: rec.version,
            })
            .await
            .map_err(|e| SecretError::Convex(e.to_string()))?;
        version = put.version;
    }
    match kind {
        SecretKind::Oauth => {
            serde_json::from_slice(&plain).map(|s| LoadedSecret::OAuth(s, version)).map_err(|_| SecretError::Unreadable)
        }
        SecretKind::Imap => {
            serde_json::from_slice(&plain).map(|s| LoadedSecret::Imap(s, version)).map_err(|_| SecretError::Unreadable)
        }
    }
}

/// Shared services the supervisor needs.
pub struct SupervisorDeps {
    /// Convex.
    pub api: Arc<dyn WorkerApi>,
    /// Engine settings.
    pub config: Arc<EngineConfig>,
    /// Adapter factory.
    pub factory: Arc<dyn ProviderFactory>,
    /// `TOKEN_ENC_KEYS`.
    pub keyring: Arc<SecretKeyring>,
    /// Mailbox OAuth clients.
    pub oauth: HashMap<OAuthKind, Arc<OAuthClient>>,
    /// Running actors.
    pub registry: Arc<Registry>,
    /// Counters.
    pub metrics: Arc<Metrics>,
    /// Gmail project budget.
    pub gate: Arc<ProjectGate>,
    /// Shared HTTP client.
    pub http: reqwest::Client,
    /// SSRF-guarded client.
    pub fetcher: SafeFetcher,
    /// `GMAIL_PUBSUB_TOPIC`.
    pub gmail_topic: Option<String>,
    /// Set while the Convex subscription is live (`/readyz`).
    pub live: Arc<AtomicBool>,
}

impl std::fmt::Debug for SupervisorDeps {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SupervisorDeps").finish_non_exhaustive()
    }
}

/// Why an account could not be started.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SpawnError {
    /// Another worker holds the lease.
    Taken,
    /// Not startable now; retry after the delay.
    Later(Duration),
    /// Convex failed.
    Convex,
    /// The account was being removed and its removal was acknowledged without an actor.
    Removed,
}

/// Acknowledges the removal of an account that no actor can serve (secret missing or unreadable,
/// no adapter, server unreachable), so Convex still purges it (D-385). Revoking a grant needs a
/// working provider and is best effort (§9.1), so it is skipped on this path.
async fn ack_removal_without_provider(api: &dyn WorkerApi, args: &AccountWorkerArgs) -> SpawnError {
    let acked = api.ack_account_removed(args).await;
    let _ = api.release_account(args).await;
    match acked {
        Ok(()) => {
            tracing::info!(account = %args.account_id, "removal acknowledged without a provider connection");
            SpawnError::Removed
        }
        Err(_) => SpawnError::Convex,
    }
}

/// The supervisor.
pub struct Supervisor {
    deps: SupervisorDeps,
    exits_tx: mpsc::UnboundedSender<(AccountId, ExitReason)>,
    exits_rx: mpsc::UnboundedReceiver<(AccountId, ExitReason)>,
    accounts: BTreeMap<AccountId, AssignedAccount>,
    by_shard: BTreeMap<u32, Vec<AssignedAccount>>,
    retry_at: HashMap<AccountId, Instant>,
    removed: HashSet<AccountId>,
    joins: Vec<tokio::task::JoinHandle<()>>,
}

impl std::fmt::Debug for Supervisor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Supervisor").field("accounts", &self.accounts.len()).finish_non_exhaustive()
    }
}

fn wants_actor(a: &AssignedAccount) -> bool {
    // Removing accounts get one actor that acknowledges the removal; everything else syncs.
    let _ = a;
    true
}

impl Supervisor {
    /// A supervisor.
    pub fn new(deps: SupervisorDeps) -> Self {
        let (exits_tx, exits_rx) = mpsc::unbounded_channel();
        Self {
            deps,
            exits_tx,
            exits_rx,
            accounts: BTreeMap::new(),
            by_shard: BTreeMap::new(),
            retry_at: HashMap::new(),
            removed: HashSet::new(),
            joins: Vec::new(),
        }
    }

    /// Claims and starts the actor of one account.
    pub async fn spawn(&mut self, account: AssignedAccount) -> Result<(), SpawnError> {
        let d = &self.deps;
        let worker_id = d.config.worker_id.clone();
        let claim = d
            .api
            .claim_account(&ClaimAccountArgs {
                account_id: account.account_id.clone(),
                worker_id: worker_id.clone(),
                lease_ms: LEASE_MS,
            })
            .await
            .map_err(|_| SpawnError::Convex)?;
        let ClaimAccountResult::Claimed { sync_state, folder_cursors, .. } = claim else {
            return Err(SpawnError::Taken);
        };
        let sink = Arc::new(ConvexSink::new(
            d.api.clone(),
            account.account_id.clone(),
            worker_id.clone(),
            vec![account.email.clone()],
        ));
        let release = || async {
            let _ = d
                .api
                .release_account(&AccountWorkerArgs {
                    account_id: account.account_id.clone(),
                    worker_id: worker_id.clone(),
                })
                .await;
        };
        let degrade = |code: ErrorCode, status: AccountStatus| {
            let sink = sink.clone();
            async move {
                let _ = sink
                    .set_status(AccountStatusUpdate {
                        status: Some(status),
                        status_detail: Some(Some(code)),
                        ..Default::default()
                    })
                    .await;
            }
        };

        // A removing account must reach `ackAccountRemoved` even when it can no longer sync.
        let removing = account.status == AccountStatus::Removing;
        let removal_args = AccountWorkerArgs { account_id: account.account_id.clone(), worker_id: worker_id.clone() };
        let secret = load_secret(d.api.as_ref(), &d.keyring, &account, Some(&worker_id)).await;
        if removing && !matches!(secret, Ok(_) | Err(SecretError::Convex(_))) {
            return Err(ack_removal_without_provider(d.api.as_ref(), &removal_args).await);
        }
        let (credentials, lost) = match secret {
            Ok(LoadedSecret::OAuth(secret, version)) => {
                let Some(client) = OAuthKind::of(account.provider).and_then(|k| d.oauth.get(&k)).cloned() else {
                    if removing {
                        return Err(ack_removal_without_provider(d.api.as_ref(), &removal_args).await);
                    }
                    degrade(ErrorCode::ProviderUnavailable, AccountStatus::Degraded).await;
                    release().await;
                    return Err(SpawnError::Later(Duration::from_secs(600)));
                };
                let ts = Arc::new(OAuthTokenSource::new(
                    client,
                    d.http.clone(),
                    d.api.clone(),
                    d.keyring.clone(),
                    SecretSlot {
                        account_id: account.account_id.clone(),
                        worker_id: worker_id.clone(),
                        aad: account.secret_aad(SecretKind::Oauth),
                    },
                    secret,
                    version,
                ));
                let lost = ts.lost();
                (Credentials::OAuth(ts as Arc<dyn TokenSource>), Some(lost))
            }
            Ok(LoadedSecret::Imap(secret, _)) => {
                // The password goes only to the servers it was sealed with (D-414). Settings that
                // differ (a Convex write) or a secret sealed without endpoints need a reconnect.
                if account.imap.as_ref().and_then(|s| secret.bound_endpoints(s)).is_none() {
                    if removing {
                        return Err(ack_removal_without_provider(d.api.as_ref(), &removal_args).await);
                    }
                    tracing::error!(
                        account = %account.account_id,
                        "IMAP settings do not match the endpoints sealed with the password; not connecting"
                    );
                    degrade(ErrorCode::ReauthRequired, AccountStatus::ReauthRequired).await;
                    release().await;
                    return Err(SpawnError::Later(Duration::from_secs(600)));
                }
                (Credentials::Imap(secret), None)
            }
            Err(SecretError::Missing) => {
                degrade(ErrorCode::ReauthRequired, AccountStatus::ReauthRequired).await;
                release().await;
                return Err(SpawnError::Later(Duration::from_secs(300)));
            }
            Err(SecretError::Unreadable) => {
                tracing::error!(account = %account.account_id, "account secret cannot be decrypted with TOKEN_ENC_KEYS");
                degrade(ErrorCode::Internal, AccountStatus::Degraded).await;
                release().await;
                return Err(SpawnError::Later(Duration::from_secs(600)));
            }
            Err(SecretError::Convex(_)) => {
                release().await;
                return Err(SpawnError::Convex);
            }
        };
        let built = d
            .factory
            .build(ProviderContext {
                account: account.clone(),
                credentials,
                http: d.http.clone(),
                gmail_pubsub_topic: d.gmail_topic.clone(),
                push: d.config.push,
            })
            .await;
        let built = match built {
            Ok(b) => b,
            Err(_) if removing => {
                return Err(ack_removal_without_provider(d.api.as_ref(), &removal_args).await);
            }
            Err(e) => {
                if let FactoryError::AdapterMissing(p) = &e {
                    tracing::warn!(account = %account.account_id, provider = ?p, "no adapter compiled in for this provider");
                }
                degrade(ErrorCode::ProviderUnavailable, AccountStatus::Degraded).await;
                release().await;
                return Err(SpawnError::Later(Duration::from_secs(600)));
            }
        };

        let (tx, rx) = mpsc::unbounded_channel();
        let handle = Arc::new(AccountHandle {
            account: std::sync::RwLock::new(account.clone()),
            provider: built.provider.clone(),
            tx: tx.clone(),
            last_manual_sync: std::sync::Mutex::new(None),
            last_interactive_at: std::sync::Mutex::new(None),
        });
        let interactive = Arc::new(std::sync::Mutex::new(None));
        let actor = AccountActor::new(ActorParts {
            account: account.clone(),
            api: d.api.clone(),
            config: d.config.clone(),
            provider: built.provider,
            push: built.push,
            sink,
            claim: ClaimState { sync_state, folder_cursors },
            rx,
            self_tx: tx,
            fetcher: d.fetcher.clone(),
            metrics: d.metrics.clone(),
            gate: d.gate.clone(),
            lost,
            last_interactive_at: Some(interactive.clone()),
            graph_subs: Some(d.registry.graph.clone()),
        });
        d.registry.insert(account.account_id.clone(), handle.clone());
        d.metrics.actors.fetch_add(1, Ordering::Relaxed);
        let exits = self.exits_tx.clone();
        let id = account.account_id.clone();
        let metrics = d.metrics.clone();
        // Interactive use recorded on the handle flows into the actor's budget.
        let bridge = {
            let handle = handle.clone();
            let interactive = interactive.clone();
            tokio::spawn(async move {
                loop {
                    tokio::time::sleep(Duration::from_secs(5)).await;
                    let v = *handle.last_interactive_at.lock().unwrap_or_else(|e| e.into_inner());
                    *interactive.lock().unwrap_or_else(|e| e.into_inner()) = v;
                    if handle.tx.is_closed() {
                        return;
                    }
                }
            })
        };
        self.joins.push(tokio::spawn(async move {
            let reason = actor.run().await;
            bridge.abort();
            metrics.actors.fetch_sub(1, Ordering::Relaxed);
            let _ = exits.send((id, reason));
        }));
        Ok(())
    }

    async fn reconcile(&mut self) {
        let now = Instant::now();
        let wanted: Vec<AssignedAccount> = self.accounts.values().filter(|a| wants_actor(a)).cloned().collect();
        let wanted_ids: HashSet<AccountId> = wanted.iter().map(|a| a.account_id.clone()).collect();
        // Accounts no longer assigned: stop their actors.
        for id in self.deps.registry.ids() {
            if !wanted_ids.contains(&id) {
                self.deps.registry.send(&id, ActorMsg::Cmd(ActorCmd::Shutdown));
            }
        }
        for a in wanted {
            if let Some(h) = self.deps.registry.get(&a.account_id) {
                *h.account.write().unwrap_or_else(|e| e.into_inner()) = a.clone();
                h.send(ActorMsg::Account(a));
                continue;
            }
            if self.removed.contains(&a.account_id) || self.retry_at.get(&a.account_id).is_some_and(|t| *t > now) {
                continue;
            }
            match self.spawn(a.clone()).await {
                Ok(()) => {
                    self.retry_at.remove(&a.account_id);
                }
                Err(SpawnError::Taken) => {
                    self.retry_at.insert(a.account_id.clone(), now + Duration::from_secs(30));
                }
                Err(SpawnError::Later(d)) => {
                    self.retry_at.insert(a.account_id.clone(), now + d);
                }
                Err(SpawnError::Convex) => {
                    self.retry_at.insert(a.account_id.clone(), now + Duration::from_secs(10));
                }
                Err(SpawnError::Removed) => {
                    self.retry_at.remove(&a.account_id);
                    self.removed.insert(a.account_id.clone());
                }
            }
        }
    }

    fn on_exit(&mut self, id: AccountId, reason: ExitReason) {
        self.deps.registry.remove(&id);
        match reason {
            ExitReason::Restart => {
                self.retry_at.remove(&id);
            }
            ExitReason::LeaseLost => {
                self.retry_at.insert(id, Instant::now() + Duration::from_secs(60));
            }
            ExitReason::Removed => {
                self.removed.insert(id);
            }
            ExitReason::Released => {}
        }
    }

    fn on_pending(&self, rows: Vec<PendingOpAccount>) {
        let now = now_ms();
        for row in rows {
            let Some(h) = self.deps.registry.get(&row.account_id) else { continue };
            if row.min_available_at <= now {
                h.send(ActorMsg::Cmd(ActorCmd::ApplyOps));
            } else {
                let delay = Duration::from_millis(u64::try_from(row.min_available_at - now).unwrap_or(0));
                let tx = h.tx.clone();
                tokio::spawn(async move {
                    tokio::time::sleep(delay).await;
                    let _ = tx.send(ActorMsg::Cmd(ActorCmd::ApplyOps));
                });
            }
        }
    }

    /// [`Self::on_pending`] for tests.
    #[cfg(test)]
    pub(crate) fn on_pending_for_tests(&self, rows: Vec<PendingOpAccount>) {
        self.on_pending(rows);
    }

    async fn heartbeat(&self) {
        let ids = self.deps.registry.ids();
        if ids.is_empty() {
            return;
        }
        match self
            .deps
            .api
            .heartbeat(&HeartbeatArgs {
                worker_id: self.deps.config.worker_id.clone(),
                account_ids: ids,
                lease_ms: LEASE_MS,
            })
            .await
        {
            Ok(r) => {
                for id in r.lost {
                    tracing::warn!(account = %id, "lease lost");
                    self.deps.registry.send(&id, ActorMsg::LeaseLost);
                }
            }
            Err(e) => tracing::warn!(error = %redact(&e), "heartbeat failed"),
        }
    }

    /// Runs until `shutdown` turns true, then stops every actor and waits for them (≤ 5 s).
    pub async fn run(mut self, mut shutdown: watch::Receiver<bool>) {
        let mut heartbeat = tokio::time::interval(Duration::from_millis(HEARTBEAT_EVERY_MS as u64));
        let mut reconcile = tokio::time::interval(RECONCILE_EVERY);
        let mut assigned = futures::stream::select_all(Vec::new());
        let mut pending = futures::stream::select_all(Vec::new());
        let mut subscribed = false;
        let mut sub_failures = 0u32;
        loop {
            if !subscribed {
                match self.subscribe().await {
                    Ok((a, p)) => {
                        assigned = futures::stream::select_all(a);
                        pending = futures::stream::select_all(p);
                        subscribed = true;
                        sub_failures = 0;
                    }
                    Err(e) => {
                        sub_failures = sub_failures.saturating_add(1);
                        tracing::warn!(error = %redact(&e), "convex subscription failed");
                        let wait = akatsuki_core::sync::backoff(sub_failures, 0.5);
                        tokio::select! {
                            _ = tokio::time::sleep(wait) => continue,
                            () = stopped(&mut shutdown) => break,
                        }
                    }
                }
            }
            tokio::select! {
                () = stopped(&mut shutdown) => break,
                item = assigned.next() => match item {
                    Some(Ok((shard, rows))) => {
                        self.deps.live.store(true, Ordering::Relaxed);
                        self.by_shard.insert(shard, rows);
                        self.accounts = self
                            .by_shard
                            .values()
                            .flatten()
                            .map(|a| (a.account_id.clone(), a.clone()))
                            .collect();
                        self.reconcile().await;
                    }
                    _ => {
                        self.deps.live.store(false, Ordering::Relaxed);
                        subscribed = false;
                    }
                },
                item = pending.next() => match item {
                    Some(Ok(rows)) => self.on_pending(rows),
                    _ => {
                        self.deps.live.store(false, Ordering::Relaxed);
                        subscribed = false;
                    }
                },
                Some((id, reason)) = self.exits_rx.recv() => {
                    let restart = reason == ExitReason::Restart;
                    self.on_exit(id, reason);
                    if restart {
                        self.reconcile().await;
                    }
                }
                _ = heartbeat.tick() => self.heartbeat().await,
                _ = reconcile.tick() => self.reconcile().await,
            }
        }
        self.stop().await;
    }

    #[allow(clippy::result_large_err)] // `ConvexApiError` is the binding crate's error type
    async fn subscribe(&self) -> Result<(Vec<AssignedStream>, Vec<PendingStream>), ConvexApiError> {
        let mut a = Vec::new();
        let mut p = Vec::new();
        for shard in 0..self.deps.config.shard_count.max(1) {
            let s = self.deps.api.assigned_accounts(shard).await?;
            #[allow(clippy::result_large_err)]
            let tag = move |r: Result<Vec<AssignedAccount>, ConvexApiError>| r.map(|rows| (shard, rows));
            a.push(s.map(tag).boxed());
            p.push(self.deps.api.pending_op_accounts(shard).await?);
        }
        Ok((a, p))
    }

    /// Stops every actor: they finish their current step and release their leases.
    pub async fn stop(mut self) {
        self.deps.live.store(false, Ordering::Relaxed);
        for id in self.deps.registry.ids() {
            self.deps.registry.send(&id, ActorMsg::Cmd(ActorCmd::Shutdown));
        }
        let joins = std::mem::take(&mut self.joins);
        let all = futures::future::join_all(joins);
        if tokio::time::timeout(SHUTDOWN_GRACE, all).await.is_err() {
            tracing::warn!("actors did not stop within the grace period");
        }
    }
}

type AssignedStream = futures::stream::BoxStream<'static, Result<(u32, Vec<AssignedAccount>), ConvexApiError>>;
type PendingStream = akatsuki_convex_api::Subscription<Vec<PendingOpAccount>>;

async fn stopped(rx: &mut watch::Receiver<bool>) {
    // The guard `wait_for` returns is not `Send`; drop it before the caller's next await.
    let _ = rx.wait_for(|s| *s).await.is_ok();
}

/// A Convex error safe for logs (function name and code only).
pub fn redact(e: &ConvexApiError) -> String {
    match e {
        ConvexApiError::App { function, code, .. } => {
            format!("{function}: {}", code.map(|c| c.as_str()).unwrap_or("app_error"))
        }
        ConvexApiError::Function { function, .. } | ConvexApiError::Decode { function, .. } => function.clone(),
        ConvexApiError::Transport(_) => "transport".into(),
        ConvexApiError::Encode(_) => "encode".into(),
    }
}
