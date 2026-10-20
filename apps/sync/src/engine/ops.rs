//! The ops claim → apply → complete loop (ARCHITECTURE §5.3–§5.6).
//!
//! One step claims up to 100 ops (`worker.claimOps`), plans them (`akatsuki_core::sync::plan`:
//! coalescing, moot detection), runs thread, label and draft ops in one `MailProvider::apply`
//! call, runs sends and unsubscribes one by one, and reports every op individually in one
//! `worker.completeOps` together with the post-op thread state. Errors follow the §5.5 table
//! (`ProviderError::disposition`); a non-provider failure twice in a row poisons the op.

use std::collections::{HashMap, VecDeque};
use std::sync::atomic::Ordering;

use akatsuki_core::convex::schema::{AppError, OpOutcome, OpPayload, ThreadAction, UnsubscribeMethod};
use akatsuki_core::convex::worker::{
    ClaimedOp, ClaimedThread, CompleteOpsArgs, False, MarkSendStartedArgs, OpResult, ThreadUpsert, True,
};
use akatsuki_core::model::{LabelKey, SysLabel, parse_label_key};
use akatsuki_core::sync::plan::PlannedStep;
use akatsuki_core::sync::send::{SendErrorClass, SendStep, after_send_error, before_send, verify_due};
use akatsuki_core::sync::{CLAIM_LIMIT, OP_LEASE_MS, backoff_ms, max_attempts};
use akatsuki_core::wire::ErrorCode;
use akatsuki_providers::labels::SNOOZED_LABEL_NAME;
use akatsuki_providers::{
    FetchDepth, LabelRole, LabelTarget, OpDisposition, OpEffect, OutgoingMime, ProviderError, ProviderKind, ProviderOp,
    RemoteDraftRef, RemoteLabel, RemoteMessageRef, RemoteThreadId, RemoteThreadRef,
};
use futures::FutureExt as _;
use futures::StreamExt as _;

use super::actor::{AccountActor, StepError, convex_step_error};
use crate::fetch::FetchError;
use crate::metrics::Metrics;

/// Largest raw message read for an unsubscribe header lookup.
const UNSUB_RAW_MAX: usize = 8 * 1024 * 1024;
/// An unsubscribe is never tried more than this many times (§9.1).
const UNSUB_MAX_ATTEMPTS: u32 = 3;

/// A non-provider failure while preparing an op (bad refs, missing label ids).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct LocalError(pub(super) &'static str);

/// What one claim batch produced.
#[derive(Debug, Default)]
pub(super) struct BatchOutcome {
    pub(super) results: Vec<OpResult>,
    pub(super) threads: Vec<ThreadUpsert>,
    pub(super) labels: Vec<RemoteLabel>,
    pub(super) deletes: Vec<String>,
}

fn ok(op: &ClaimedOp, outcome: OpOutcome) -> OpResult {
    OpResult::Ok { op_id: op.op.id.clone(), ok: True, outcome, provider_label_id: None, sent_provider_message_id: None }
}

fn err(op: &ClaimedOp, code: ErrorCode, retryable: bool, retry_after_ms: Option<i64>, count_attempt: bool) -> OpResult {
    OpResult::Err {
        op_id: op.op.id.clone(),
        ok: False,
        error: AppError { code: code.as_str().to_owned(), message: code.default_message().to_owned(), retryable },
        retry_after_ms,
        count_attempt,
    }
}

fn thread_ref(t: &ClaimedThread) -> RemoteThreadRef {
    RemoteThreadRef {
        provider_thread_id: t.provider_thread_id.clone(),
        messages: t
            .messages
            .iter()
            .map(|m| RemoteMessageRef {
                provider_message_id: m.provider_message_id.clone(),
                imap: m.imap_locations.clone().unwrap_or_default(),
            })
            .collect(),
    }
}

fn jitter() -> f64 {
    let b = crate::crypto::random_bytes::<2>().unwrap_or([0, 128]);
    f64::from(u16::from_le_bytes(b)) / 65_536.0
}

/// Maps a provider error from `send_draft` onto the §5.6 classes.
///
/// The IMAP adapter marks transport failures after the SMTP upload began with the
/// `send_ambiguous` prefix, so its other transient errors (connection refused, failures before
/// `DATA`) certainly did not send. Gmail and Graph cannot tell: every transient failure of their
/// send call counts as ambiguous.
pub fn classify_send_error(kind: ProviderKind, e: &ProviderError) -> SendErrorClass {
    match e {
        e if akatsuki_providers::imap::is_ambiguous_send(e) => SendErrorClass::Ambiguous,
        ProviderError::Transient(..) if kind == ProviderKind::Imap => SendErrorClass::NotSent,
        ProviderError::Transient(..) | ProviderError::CursorExpired => SendErrorClass::Ambiguous,
        ProviderError::RateLimited { retry_after, .. } => SendErrorClass::RateLimited {
            retry_after_ms: retry_after.map(|d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX)),
        },
        ProviderError::Auth { .. } => SendErrorClass::Auth,
        ProviderError::NotFound => SendErrorClass::DraftGone,
        ProviderError::Invalid(_) | ProviderError::Conflict(_) | ProviderError::Unsupported(_) => {
            SendErrorClass::Rejected
        }
        ProviderError::Quota(_) => SendErrorClass::Quota,
    }
}

fn send_step_result(op: &ClaimedOp, step: SendStep) -> Option<OpResult> {
    Some(match step {
        SendStep::Sent { provider_message_id } => OpResult::Ok {
            op_id: op.op.id.clone(),
            ok: True,
            outcome: OpOutcome::Applied,
            provider_label_id: None,
            sent_provider_message_id: Some(provider_message_id),
        },
        // A verification miss is not a failed attempt: the schedule (`verify_due`) bounds it (§5.6).
        SendStep::VerifyLater { after_ms } => err(op, ErrorCode::SendUnconfirmed, true, Some(after_ms), false),
        SendStep::GiveUp => err(op, ErrorCode::SendUnconfirmed, false, None, true),
        SendStep::Requeue { code, retry_after_ms, count_attempt } => {
            err(op, code, true, Some(retry_after_ms), count_attempt)
        }
        SendStep::Fail { code } => err(op, code, false, None, true),
        SendStep::LeaveInflight | SendStep::Send => return None,
    })
}

/// Longest `subject` a mailto unsubscribe keeps (list servers use short tokens).
pub const UNSUB_SUBJECT_MAX: usize = 64;

/// The subject a mailto unsubscribe sends: the list's own subject only when it is a short token of
/// letters, digits and `-_.:+=@ ` (some list servers put a token there); anything else, including
/// free text a sender could use to speak through the user's mailbox, becomes `unsubscribe`.
pub fn unsubscribe_subject(requested: &str) -> String {
    let s = requested.trim();
    let token_like = !s.is_empty()
        && s.len() <= UNSUB_SUBJECT_MAX
        && s.chars().filter(|c| c.is_whitespace()).count() <= 2
        && s.chars().all(|c| c.is_ascii_alphanumeric() || " -_.:+=@".contains(c));
    if token_like { s.to_owned() } else { "unsubscribe".to_owned() }
}

/// Parses `mailto:addr?subject=…` into `(address, subject)`; the subject passes [`unsubscribe_subject`].
pub fn parse_mailto(mailto: &str) -> Option<(String, String)> {
    let url = url::Url::parse(mailto).ok()?;
    if url.scheme() != "mailto" {
        return None;
    }
    let addr = percent_encoding::percent_decode_str(url.path()).decode_utf8().ok()?.trim().to_lowercase();
    let addr = addr.split(',').next()?.trim().to_owned();
    if addr.is_empty() || !addr.contains('@') {
        return None;
    }
    let subject = url
        .query_pairs()
        .find(|(k, _)| k.eq_ignore_ascii_case("subject"))
        .map(|(_, v)| v.into_owned())
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| "unsubscribe".to_owned());
    Some((addr, unsubscribe_subject(&subject)))
}

impl AccountActor {
    /// One ops step.
    pub(super) async fn ops_step(&mut self) -> Result<(), StepError> {
        let claimed = self
            .api
            .claim_ops(&akatsuki_core::convex::worker::ClaimOpsArgs {
                account_id: self.account.account_id.clone(),
                worker_id: self.worker_id().to_owned(),
                limit: CLAIM_LIMIT,
                lease_ms: OP_LEASE_MS,
            })
            .await
            .map_err(|e| convex_step_error(&e))?;
        if claimed.is_empty() {
            self.queue.ops_due = false;
            return Ok(());
        }
        self.budget.note_user_activity(akatsuki_core::time::now_ms());
        let outcome = match std::panic::AssertUnwindSafe(self.run_claimed(&claimed)).catch_unwind().await {
            Ok(o) => o,
            Err(_) => {
                tracing::error!(account = %self.account.account_id, "op processing panicked");
                let mut o = BatchOutcome::default();
                for c in &claimed {
                    o.results.push(self.poisoned(c, LocalError("panic")));
                }
                o
            }
        };
        for r in &outcome.results {
            match r {
                OpResult::Ok { outcome: OpOutcome::Applied, .. } => Metrics::inc(&self.metrics.ops_applied),
                OpResult::Ok { outcome: OpOutcome::Moot, .. } => Metrics::inc(&self.metrics.ops_moot),
                OpResult::Err { error, .. } if error.retryable => Metrics::inc(&self.metrics.ops_retried),
                OpResult::Err { .. } => Metrics::inc(&self.metrics.ops_failed),
            }
        }
        if !outcome.results.is_empty() {
            self.api
                .complete_ops(&CompleteOpsArgs {
                    account_id: self.account.account_id.clone(),
                    worker_id: self.worker_id().to_owned(),
                    results: outcome.results,
                    threads: (!outcome.threads.is_empty()).then_some(outcome.threads),
                    labels: (!outcome.labels.is_empty()).then_some(outcome.labels.clone()),
                })
                .await
                .map_err(|e| convex_step_error(&e))?;
        }
        if !outcome.labels.is_empty() {
            // Keep the local map in step for later conversions; Convex already has the rows.
            self.refresh_labels().await?;
        }
        if !outcome.deletes.is_empty() {
            self.sink.write_upserts(vec![], outcome.deletes, None, 100).await?;
        }
        // Claim again until the queue is empty; ops waiting for `availableAt` come back through
        // `worker.pendingOpAccounts`.
        Ok(())
    }

    pub(super) fn poisoned(&mut self, op: &ClaimedOp, e: LocalError) -> OpResult {
        let n = self.poison.entry(op.op.id.as_str().to_owned()).or_insert(0);
        *n += 1;
        tracing::warn!(account = %self.account.account_id, op = %op.op.id, reason = e.0, count = *n, "op failed locally");
        if *n >= 2 {
            self.poison.remove(op.op.id.as_str());
            err(op, ErrorCode::Internal, false, None, true)
        } else {
            err(op, ErrorCode::Internal, true, Some(backoff_ms(op.op.attempt, jitter())), true)
        }
    }

    fn provider_error(&mut self, op: &ClaimedOp, e: &ProviderError, out: &mut BatchOutcome) -> Option<OpResult> {
        let kind = op.op.payload.kind();
        match e.disposition(op.op.attempt, max_attempts(kind), jitter()) {
            OpDisposition::Requeue { retry_after_ms, count_attempt, pause, reauth } => {
                if let Some(scope) = pause {
                    let until = std::time::Instant::now()
                        + std::time::Duration::from_millis(u64::try_from(retry_after_ms.max(1_000)).unwrap_or(1_000));
                    self.pause_until = Some(self.pause_until.map_or(until, |u| u.max(until)));
                    if scope == akatsuki_providers::LimitScope::Project {
                        self.gate.pause_until(until);
                    }
                }
                if reauth {
                    self.phase = self.phase.clone().next(akatsuki_core::sync::PhaseEvent::AuthRejected);
                }
                Some(err(op, e.code(), true, Some(retry_after_ms), count_attempt))
            }
            OpDisposition::Moot => {
                if let (OpPayload::Thread { .. }, Some(t)) = (&op.op.payload, &op.thread) {
                    out.deletes.push(t.provider_thread_id.clone());
                }
                Some(ok(op, OpOutcome::Moot))
            }
            OpDisposition::Fail => Some(err(op, e.code(), false, None, true)),
        }
    }

    /// The provider target of a label key; lazily creates `Akatsuki/Snoozed` (§9.3).
    async fn target(&mut self, op: &ClaimedOp, key: &str, out: &mut BatchOutcome) -> Result<LabelTarget, LocalError> {
        let parsed = parse_label_key(key).map_err(|_| LocalError("bad label key"))?;
        let exclusive_default = !self.provider.caps().multi_label;
        match parsed {
            LabelKey::Sys(SysLabel::Snoozed) => {
                if let Some(id) = op.labels.get(key).and_then(|l| l.provider_label_id.clone()) {
                    return Ok(LabelTarget::Label { provider_label_id: id, exclusive: exclusive_default });
                }
                if let Some(created) = out.labels.iter().find(|l| l.role == Some(LabelRole::Snoozed)) {
                    return Ok(LabelTarget::Label {
                        provider_label_id: created.provider_label_id.clone(),
                        exclusive: created.exclusive,
                    });
                }
                match self.provider.ensure_label(LabelRole::Snoozed, SNOOZED_LABEL_NAME).await {
                    Ok(l) => {
                        let t = LabelTarget::Label {
                            provider_label_id: l.provider_label_id.clone(),
                            exclusive: l.exclusive,
                        };
                        out.labels.push(l);
                        Ok(t)
                    }
                    Err(_) => Err(LocalError("snoozed label unavailable")),
                }
            }
            LabelKey::Sys(s) => Ok(LabelTarget::Sys(s)),
            LabelKey::User(_) => {
                let l = op.labels.get(key).ok_or(LocalError("label not in claim"))?;
                let id = l.provider_label_id.clone().ok_or(LocalError("label has no provider id"))?;
                Ok(LabelTarget::Label { provider_label_id: id, exclusive: l.exclusive })
            }
            LabelKey::Split(_) => Err(LocalError("split key in an op")),
        }
    }

    async fn targets(
        &mut self,
        op: &ClaimedOp,
        keys: &[String],
        out: &mut BatchOutcome,
    ) -> Result<Vec<LabelTarget>, LocalError> {
        let mut v = Vec::with_capacity(keys.len());
        for k in keys {
            v.push(self.target(op, k, out).await?);
        }
        Ok(v)
    }

    /// Builds the provider op of a single claimed op; `Ok(None)` means nothing to do (moot).
    async fn build_single(&mut self, op: &ClaimedOp, out: &mut BatchOutcome) -> Result<Option<ProviderOp>, LocalError> {
        Ok(Some(match &op.op.payload {
            OpPayload::Thread { action } => {
                let Some(t) = &op.thread else { return Ok(None) };
                let thread = thread_ref(t);
                match action {
                    ThreadAction::Trash => ProviderOp::Trash { thread },
                    ThreadAction::Untrash => ProviderOp::Untrash { thread },
                    ThreadAction::DeleteForever => ProviderOp::DeleteForever { thread },
                    ThreadAction::Move { to } => ProviderOp::Move { thread, to: self.target(op, to, out).await? },
                    ThreadAction::Modify { add, remove } => ProviderOp::Modify {
                        thread,
                        add: self.targets(op, add, out).await?,
                        remove: self.targets(op, remove, out).await?,
                    },
                    ThreadAction::Unsubscribe { .. } => return Err(LocalError("unsubscribe is not a provider op")),
                }
            }
            OpPayload::DeleteDraft { .. } => {
                let Some(id) = op.draft.as_ref().and_then(|d| d.provider_draft_id.clone()) else { return Ok(None) };
                ProviderOp::DeleteDraft {
                    draft: RemoteDraftRef {
                        provider_draft_id: id,
                        provider_message_id: op.draft.as_ref().and_then(|d| d.provider_message_id.clone()),
                    },
                }
            }
            OpPayload::LabelCreate { label_id } => {
                let l = op.labels.get(&format!("l:{label_id}")).ok_or(LocalError("label row missing"))?;
                if let Some(existing) = &l.provider_label_id {
                    // Already created (a replay after a lost completeOps).
                    let _ = existing;
                    return Ok(None);
                }
                ProviderOp::LabelCreate { name: l.name.clone(), color: None }
            }
            OpPayload::LabelRename { label_id, name } => {
                let Some(id) = op.labels.get(&format!("l:{label_id}")).and_then(|l| l.provider_label_id.clone()) else {
                    return Ok(None);
                };
                ProviderOp::LabelRename { provider_label_id: id, name: name.clone() }
            }
            OpPayload::LabelDelete { label_id } => {
                let Some(id) = op.labels.get(&format!("l:{label_id}")).and_then(|l| l.provider_label_id.clone()) else {
                    return Ok(None);
                };
                ProviderOp::LabelDelete { provider_label_id: id }
            }
            OpPayload::Send { .. } => return Err(LocalError("send is not a provider op")),
        }))
    }

    /// `label_create` of the Archive placeholder (DECISIONS D-392): finds the server's archive
    /// folder or creates one (`ensure_label`), never a category or keyword.
    async fn run_ensure_archive(&mut self, op: &ClaimedOp, label_id: &str, out: &mut BatchOutcome) -> OpResult {
        let Some(l) = op.labels.get(&format!("l:{label_id}")) else {
            return self.poisoned(op, LocalError("label row missing"));
        };
        if let Some(id) = &l.provider_label_id {
            let mut r = ok(op, OpOutcome::Moot);
            if let OpResult::Ok { provider_label_id, .. } = &mut r {
                *provider_label_id = Some(id.clone());
            }
            return r;
        }
        match self.provider.ensure_label(LabelRole::Archive, &l.name).await {
            Ok(created) => {
                let mut r = ok(op, OpOutcome::Applied);
                if let OpResult::Ok { provider_label_id, .. } = &mut r {
                    *provider_label_id = Some(created.provider_label_id.clone());
                }
                out.labels.push(created);
                r
            }
            Err(e) => {
                let mut sink = BatchOutcome::default();
                self.provider_error(op, &e, &mut sink).unwrap_or_else(|| err(op, e.code(), true, None, true))
            }
        }
    }

    /// Processes one claim batch.
    ///
    /// Provider ops run in rounds: each round sends at most one op per thread (or label) in one
    /// `apply` call. When an op fails, later ops of the same thread in this batch are not sent
    /// and are requeued behind it, so FIFO per thread holds even inside one claim (§5.4).
    pub(super) async fn run_claimed(&mut self, claimed: &[ClaimedOp]) -> BatchOutcome {
        let mut out = BatchOutcome::default();
        let mut results: Vec<Option<OpResult>> = vec![None; claimed.len()];
        let mut left_inflight = vec![false; claimed.len()];
        // Lanes in first-seen order: (lane key, queue of (op key, provider op, claimed indices)).
        let mut lanes: Vec<(String, VecDeque<(String, ProviderOp, Vec<usize>)>)> = Vec::new();
        let mut blocked: HashMap<String, i64> = HashMap::new();
        let lane_of = |op: &ClaimedOp| -> String {
            match (&op.thread, &op.op.payload) {
                (Some(t), _) => format!("t:{}", t.provider_thread_id),
                (
                    None,
                    OpPayload::LabelCreate { label_id }
                    | OpPayload::LabelRename { label_id, .. }
                    | OpPayload::LabelDelete { label_id },
                ) => {
                    format!("l:{label_id}")
                }
                _ => format!("o:{}", op.op.id),
            }
        };
        let mut push =
            |lane: String, key: String, p: ProviderOp, ix: Vec<usize>| match lanes.iter_mut().find(|(l, _)| *l == lane)
            {
                Some((_, q)) => q.push_back((key, p, ix)),
                None => lanes.push((lane, VecDeque::from([(key, p, ix)]))),
            };

        for step in akatsuki_core::sync::plan::plan(claimed) {
            match step {
                PlannedStep::Moot(ix) => {
                    for i in ix {
                        results[i] = Some(ok(&claimed[i], OpOutcome::Moot));
                    }
                }
                PlannedStep::Modify { ops, add, remove } => {
                    let first = &claimed[ops[0]];
                    let add = self.targets(first, &add, &mut out).await;
                    let remove = self.targets(first, &remove, &mut out).await;
                    match (add, remove, &first.thread) {
                        (Ok(add), Ok(remove), Some(t)) => push(
                            lane_of(first),
                            first.op.id.as_str().to_owned(),
                            ProviderOp::Modify { thread: thread_ref(t), add, remove },
                            ops,
                        ),
                        (Ok(_), Ok(_), None) => {
                            for i in ops {
                                results[i] = Some(ok(&claimed[i], OpOutcome::Moot));
                            }
                        }
                        (Err(e), _, _) | (_, Err(e), _) => {
                            for &i in &ops {
                                results[i] = Some(self.poisoned(&claimed[i], e.clone()));
                            }
                            blocked.insert(lane_of(first), 1_000);
                        }
                    }
                }
                PlannedStep::Single(i) => {
                    let op = &claimed[i];
                    match &op.op.payload {
                        OpPayload::Send { .. } => {
                            results[i] = self.run_send(op).await;
                            left_inflight[i] = results[i].is_none();
                        }
                        OpPayload::Thread { action: ThreadAction::Unsubscribe { method } } => {
                            results[i] = Some(self.run_unsubscribe(op, *method).await);
                        }
                        OpPayload::LabelCreate { label_id }
                            if op
                                .labels
                                .get(&format!("l:{label_id}"))
                                .is_some_and(|l| l.role.as_deref() == Some("archive")) =>
                        {
                            results[i] = Some(self.run_ensure_archive(op, label_id.as_str(), &mut out).await);
                        }
                        _ => match self.build_single(op, &mut out).await {
                            Ok(Some(p)) => push(lane_of(op), op.op.id.as_str().to_owned(), p, vec![i]),
                            Ok(None) => results[i] = Some(ok(op, OpOutcome::Moot)),
                            Err(e) => {
                                results[i] = Some(self.poisoned(op, e));
                                blocked.insert(lane_of(op), 1_000);
                            }
                        },
                    }
                }
            }
        }

        let mut refetch: Vec<RemoteThreadId> = Vec::new();
        loop {
            // Lanes blocked by a failed predecessor give up their remaining ops for this batch.
            for (lane, q) in &mut lanes {
                if let Some(wait) = blocked.get(lane) {
                    for (_, _, ix) in q.drain(..) {
                        for i in ix {
                            results[i] = Some(err(&claimed[i], ErrorCode::ProviderTransient, true, Some(*wait), false));
                        }
                    }
                }
            }
            let round: Vec<(String, String, ProviderOp, Vec<usize>)> = lanes
                .iter_mut()
                .filter_map(|(lane, q)| q.pop_front().map(|(k, p, ix)| (lane.clone(), k, p, ix)))
                .collect();
            if round.is_empty() {
                break;
            }
            let batch: Vec<(String, ProviderOp)> = round.iter().map(|(_, k, p, _)| (k.clone(), p.clone())).collect();
            let applied = self.provider.apply(&batch).await;
            let mut by_id: HashMap<String, Result<OpEffect, ProviderError>> = applied.into_iter().collect();
            for (lane, key, _, ix) in round {
                match by_id.remove(&key) {
                    Some(Ok(OpEffect::Applied { post, created })) => {
                        if let Some(t) = post {
                            out.threads.extend(self.sink.to_upserts(std::slice::from_ref(&t)));
                        }
                        let created_id = created.as_ref().map(|l| l.provider_label_id.clone());
                        if let Some(l) = created {
                            out.labels.push(l);
                        }
                        for &i in &ix {
                            let mut r = ok(&claimed[i], OpOutcome::Applied);
                            if let (OpResult::Ok { provider_label_id, .. }, OpPayload::LabelCreate { .. }) =
                                (&mut r, &claimed[i].op.payload)
                            {
                                provider_label_id.clone_from(&created_id);
                            }
                            results[i] = Some(r);
                        }
                    }
                    Some(Ok(OpEffect::Moot)) => {
                        for &i in &ix {
                            results[i] = Some(ok(&claimed[i], OpOutcome::Moot));
                        }
                    }
                    Some(Err(e)) => {
                        for &i in &ix {
                            let r = self.provider_error(&claimed[i], &e, &mut out);
                            match &r {
                                Some(OpResult::Err { error, retry_after_ms, .. }) => {
                                    if !error.retryable
                                        && let Some(t) = &claimed[i].thread
                                    {
                                        // Fresh truth after a permanent failure (§5.3).
                                        refetch.push(RemoteThreadId(t.provider_thread_id.clone()));
                                    }
                                    blocked.insert(lane.clone(), retry_after_ms.unwrap_or(1_000).max(1_000));
                                }
                                Some(OpResult::Ok { .. }) | None => {}
                            }
                            results[i] = r;
                        }
                    }
                    None => {
                        for &i in &ix {
                            results[i] = Some(self.poisoned(&claimed[i], LocalError("adapter returned no result")));
                        }
                        blocked.insert(lane, 1_000);
                    }
                }
            }
        }
        refetch.sort();
        refetch.dedup();
        if !refetch.is_empty()
            && let Ok(threads) = self.provider.fetch_threads(&refetch, FetchDepth::Metadata).await
        {
            out.threads.extend(self.sink.to_upserts(&threads));
        }

        for (i, r) in results.into_iter().enumerate() {
            match r {
                Some(r) => out.results.push(r),
                None if left_inflight[i] => {} // ambiguous send: left inflight for the reaper (§5.6)
                None => out.results.push(self.poisoned(&claimed[i], LocalError("op not processed"))),
            }
        }
        out
    }

    /// A `send` op (§5.6). `None` leaves it inflight (ambiguous outcome).
    async fn run_send(&mut self, op: &ClaimedOp) -> Option<OpResult> {
        let Some(send) = &op.send else {
            return Some(err(op, ErrorCode::DraftDeleted, false, None, true));
        };
        let rfc = send.draft.rfc_message_id.trim_matches(|c| c == '<' || c == '>').to_owned();
        let found = match self.provider.find_sent(&rfc).await {
            Ok(f) => f.map(|s| s.provider_message_id),
            Err(e) => {
                let mut scratch = BatchOutcome::default();
                return self.provider_error(op, &e, &mut scratch);
            }
        };
        let step = if op.op.verify_only { verify_due(found, op.op.send_elapsed_ms) } else { before_send(found) };
        if !matches!(step, SendStep::Send) {
            if matches!(step, SendStep::Sent { .. }) {
                Metrics::inc(&self.metrics.sends_sent);
            }
            return send_step_result(op, step);
        }
        // Right before the provider call (§5.6 step 2).
        match self
            .api
            .mark_send_started(&MarkSendStartedArgs { op_id: op.op.id.clone(), worker_id: self.worker_id().to_owned() })
            .await
        {
            Ok(()) => {}
            Err(e) if e.is_lease_lost() => return None,
            Err(_) => return Some(err(op, ErrorCode::ProviderTransient, true, Some(5_000), false)),
        }
        let draft = RemoteDraftRef {
            provider_draft_id: send.draft.provider_draft_id.clone(),
            provider_message_id: send.draft.provider_message_id.clone(),
        };
        match self.provider.send_draft(&draft, &rfc).await {
            Ok(sent) => {
                Metrics::inc(&self.metrics.sends_sent);
                send_step_result(op, SendStep::Sent { provider_message_id: sent.provider_message_id })
            }
            Err(e) => {
                let class = classify_send_error(self.provider.kind(), &e);
                let step =
                    after_send_error(class, op.op.attempt, max_attempts("send"), backoff_ms(op.op.attempt, jitter()));
                if step == SendStep::LeaveInflight {
                    self.metrics.sends_ambiguous.fetch_add(1, Ordering::Relaxed);
                    tracing::warn!(account = %self.account.account_id, op = %op.op.id, "send outcome ambiguous; verifying");
                }
                send_step_result(op, step)
            }
        }
    }

    /// `unsubscribe` (§9.1): RFC 8058 one-click POST through the SSRF-guarded client, or a mail
    /// to the `mailto:` address. Never tried more than 3 times.
    async fn run_unsubscribe(&mut self, op: &ClaimedOp, method: UnsubscribeMethod) -> OpResult {
        let Some(t) = &op.thread else { return ok(op, OpOutcome::Moot) };
        let Some(newest) = t.messages.last() else { return ok(op, OpOutcome::Moot) };
        let msg = RemoteMessageRef {
            provider_message_id: newest.provider_message_id.clone(),
            imap: newest.imap_locations.clone().unwrap_or_default(),
        };
        let retry_or_fail = |op: &ClaimedOp| {
            if op.op.attempt >= UNSUB_MAX_ATTEMPTS {
                err(op, ErrorCode::ProviderInvalid, false, None, true)
            } else {
                err(op, ErrorCode::ProviderTransient, true, Some(backoff_ms(op.op.attempt, jitter())), true)
            }
        };
        let raw = match self.provider.fetch_raw(&msg).await {
            Ok(mut stream) => {
                let mut buf = Vec::new();
                let mut too_big = false;
                while let Some(chunk) = stream.next().await {
                    match chunk {
                        Ok(b) if buf.len() + b.len() <= UNSUB_RAW_MAX => buf.extend_from_slice(&b),
                        Ok(_) => {
                            too_big = true;
                            break;
                        }
                        Err(_) => return retry_or_fail(op),
                    }
                }
                if too_big && buf.is_empty() {
                    return err(op, ErrorCode::ProviderInvalid, false, None, true);
                }
                buf
            }
            Err(ProviderError::NotFound) => return ok(op, OpOutcome::Moot),
            Err(_) => return retry_or_fail(op),
        };
        let lu = akatsuki_mime::parse_message(&raw).ok().and_then(|p| p.list_unsubscribe);
        let Some(lu) = lu else { return err(op, ErrorCode::ProviderInvalid, false, None, true) };
        match method {
            UnsubscribeMethod::OneClickPost => {
                let Some(url) = lu.http.filter(|u| u.starts_with("https://") && lu.one_click) else {
                    return err(op, ErrorCode::ProviderInvalid, false, None, true);
                };
                match self.fetcher.post_form(&url, "List-Unsubscribe=One-Click", 64 * 1024).await {
                    Ok(f) if (200..300).contains(&f.status) => ok(op, OpOutcome::Applied),
                    Ok(f) if f.status >= 500 => retry_or_fail(op),
                    Ok(_) => err(op, ErrorCode::ProviderInvalid, false, None, true),
                    Err(FetchError::Timeout | FetchError::Network) => retry_or_fail(op),
                    Err(_) => err(op, ErrorCode::ProviderInvalid, false, None, true),
                }
            }
            UnsubscribeMethod::Mailto => {
                let Some((to, subject)) = lu.mailto.as_deref().and_then(parse_mailto) else {
                    return err(op, ErrorCode::ProviderInvalid, false, None, true);
                };
                let from = akatsuki_core::model::Addr::new(None, &self.account.email);
                let domain = from.domain().unwrap_or("akatsuki.vc").to_owned();
                // Stable per op, so a retry finds an earlier send instead of sending twice.
                let digest = crate::crypto::hmac_tag(b"akatsuki:unsubscribe:v1", op.op.id.as_str().as_bytes());
                let rfc = format!("unsub.{}@{domain}", hex::encode(&digest[..16]));
                if let Ok(Some(_)) = self.provider.find_sent(&rfc).await {
                    return ok(op, OpOutcome::Applied);
                }
                let input = akatsuki_mime::ComposeInput {
                    from,
                    to: vec![akatsuki_core::model::Addr::new(None, &to)],
                    subject,
                    text: "unsubscribe".into(),
                    message_id: rfc.clone(),
                    date_ms: akatsuki_core::time::now_ms(),
                    include_bcc_header: false,
                    ..Default::default()
                };
                let Ok(bytes) = akatsuki_mime::compose(&input) else {
                    return err(op, ErrorCode::ProviderInvalid, false, None, true);
                };
                let mime = OutgoingMime { rfc_message_id: rfc.clone(), bytes: bytes.into(), thread: None };
                let draft = match self.provider.save_draft(&mime, None).await {
                    Ok(d) => d,
                    Err(_) => return retry_or_fail(op),
                };
                match self.provider.send_draft(&draft, &rfc).await {
                    Ok(_) => ok(op, OpOutcome::Applied),
                    Err(e) if classify_send_error(self.provider.kind(), &e) == SendErrorClass::Ambiguous => {
                        retry_or_fail(op)
                    }
                    Err(_) => err(op, ErrorCode::ProviderInvalid, false, None, true),
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mailto_parsing() {
        assert_eq!(
            parse_mailto("mailto:Leave@List.Example.com?subject=remove%20me"),
            Some(("leave@list.example.com".into(), "remove me".into()))
        );
        assert_eq!(parse_mailto("mailto:a@b.c"), Some(("a@b.c".into(), "unsubscribe".into())));
        assert_eq!(
            parse_mailto("mailto:u@l.example?subject=unsubscribe%20abc-123"),
            Some(("u@l.example".into(), "unsubscribe abc-123".into()))
        );
        // Free text a sender chose never goes out under the user's name.
        for hostile in [
            "I%20resign%2C%20effective%20today.%20Wire%20the%20money%20to%20the%20new%20account.",
            "<b>hi</b>",
            "Ja%20bitte%20%C3%BCberweisen",
        ] {
            let (_, subject) = parse_mailto(&format!("mailto:x@y.example?subject={hostile}")).unwrap();
            assert_eq!(subject, "unsubscribe", "{hostile}");
        }
        assert_eq!(parse_mailto("https://x"), None);
        assert_eq!(parse_mailto("mailto:?subject=x"), None);
    }

    #[test]
    fn send_errors_classify_conservatively() {
        let g = ProviderKind::Gmail;
        let reset = ProviderError::Transient("reset".into(), None);
        assert_eq!(classify_send_error(g, &reset), SendErrorClass::Ambiguous);
        assert_eq!(classify_send_error(ProviderKind::Graph, &reset), SendErrorClass::Ambiguous);
        // IMAP: only failures after the SMTP upload began are ambiguous.
        assert_eq!(classify_send_error(ProviderKind::Imap, &reset), SendErrorClass::NotSent);
        let upload = ProviderError::Transient(format!("{}: reset", akatsuki_providers::imap::AMBIGUOUS_SEND), None);
        assert_eq!(classify_send_error(ProviderKind::Imap, &upload), SendErrorClass::Ambiguous);
        assert_eq!(classify_send_error(g, &ProviderError::NotFound), SendErrorClass::DraftGone);
        assert_eq!(classify_send_error(g, &ProviderError::Quota("daily".into())), SendErrorClass::Quota);
        assert_eq!(classify_send_error(g, &ProviderError::Auth { reauth_required: true }), SendErrorClass::Auth);
    }
}
