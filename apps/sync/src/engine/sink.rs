//! `ConvexSink`: the worker's `MetaSink` over `WorkerApi` (ARCHITECTURE §9.1, §4.3).
//!
//! It keeps the provider-label-id → label-key map returned by `worker.upsertLabels`, converts
//! `FetchedThread`s to `ThreadUpsert`s with the §1.6 caps, and splits writes into batches of at
//! most 100 threads and 400 messages, putting the cursor on the last batch so it commits with the
//! data it covers.

use std::sync::{Arc, RwLock};

use akatsuki_convex_api::{ConvexApiError, WorkerApi};
use akatsuki_core::convex::worker::{
    SetAccountStatusArgs, SetCursorArgs, ThreadUpsert, UpsertLabelsArgs, UpsertThreadsArgs,
};
use akatsuki_core::convex::{CursorUpdate, UpsertThreadsResult};
use akatsuki_core::ids::AccountId;
use akatsuki_core::wire::ErrorCode;
use akatsuki_providers::convert::thread_upsert;
use akatsuki_providers::{AccountStatusUpdate, FetchedThread, LabelMap, MetaSink, RemoteLabel, SinkError, UpsertAck};
use async_trait::async_trait;

/// Threads per `worker.upsertThreads` call (§1.6).
pub const MAX_THREADS_PER_CALL: usize = 100;
/// Messages per `worker.upsertThreads` call (§4.4).
pub const MAX_MESSAGES_PER_CALL: usize = 400;

/// Maps a Convex error to a sink error.
pub fn sink_error(e: &ConvexApiError) -> SinkError {
    match e {
        ConvexApiError::App { code: Some(ErrorCode::LeaseLost), .. } => SinkError::LeaseLost,
        ConvexApiError::App { code: Some(ErrorCode::NotEntitled), .. } => SinkError::NotEntitled,
        ConvexApiError::App { code, .. } => SinkError::Invalid(code.map(|c| c.as_str()).unwrap_or("app").to_owned()),
        ConvexApiError::Transport(_) => SinkError::Unavailable("transport".into()),
        ConvexApiError::Function { function, .. } => SinkError::Unavailable(function.clone()),
        ConvexApiError::Encode(_) | ConvexApiError::Decode { .. } => SinkError::Invalid("codec".into()),
    }
}

/// The worker's sink for one account.
pub struct ConvexSink {
    api: Arc<dyn WorkerApi>,
    account_id: AccountId,
    worker_id: String,
    labels: RwLock<LabelMap>,
    identities: Vec<String>,
}

impl std::fmt::Debug for ConvexSink {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ConvexSink").field("account_id", &self.account_id).finish_non_exhaustive()
    }
}

impl ConvexSink {
    /// A sink for one account. `identities` are the account's own addresses (they decide
    /// `isOutbound`).
    pub fn new(api: Arc<dyn WorkerApi>, account_id: AccountId, worker_id: String, identities: Vec<String>) -> Self {
        let identities = identities.into_iter().map(|e| e.trim().to_lowercase()).collect();
        Self { api, account_id, worker_id, labels: RwLock::new(LabelMap::new()), identities }
    }

    /// The current label map.
    pub fn label_map(&self) -> LabelMap {
        self.labels.read().unwrap_or_else(|e| e.into_inner()).clone()
    }

    /// Merges entries into the label map (labels created by ops).
    pub fn merge_labels(&self, map: LabelMap) {
        self.labels.write().unwrap_or_else(|e| e.into_inner()).extend(map);
    }

    /// The label key of a provider label (folder cursors need it).
    pub fn label_key(&self, provider_label_id: &str) -> Option<String> {
        self.labels.read().unwrap_or_else(|e| e.into_inner()).get(provider_label_id).cloned()
    }

    /// Converts fetched threads with the current label map. A thread the adapter marks `partial`
    /// (IMAP, D-223) goes up with `complete: false`, so Convex keeps the messages it does not list.
    pub fn to_upserts(&self, threads: &[FetchedThread]) -> Vec<ThreadUpsert> {
        let map = self.label_map();
        threads.iter().map(|t| thread_upsert(t, &map, &self.identities, !t.partial)).collect()
    }

    /// Writes cursor updates on their own (`worker.setCursor`).
    pub async fn set_cursor(&self, cursor: CursorUpdate) -> Result<(), SinkError> {
        self.api
            .set_cursor(&SetCursorArgs {
                account_id: self.account_id.clone(),
                worker_id: self.worker_id.clone(),
                cursor,
            })
            .await
            .map_err(|e| sink_error(&e))
    }

    /// Writes thread upserts in batches; the first batch is at most `first_batch` threads (25 for
    /// the InboxFirst first paint).
    pub async fn write_upserts(
        &self,
        upserts: Vec<ThreadUpsert>,
        deletes: Vec<String>,
        cursor: Option<CursorUpdate>,
        first_batch: usize,
    ) -> Result<UpsertAck, SinkError> {
        self.write_changes(upserts, deletes, vec![], cursor, first_batch).await
    }

    /// Like [`Self::write_upserts`], plus `deleted_messages`: provider message ids that are gone at
    /// the provider (IMAP expunges and moves, D-410). They travel in chunks of at most 400 next to the
    /// thread batches; the cursor rides on the last call so it commits with everything it covers.
    pub async fn write_changes(
        &self,
        upserts: Vec<ThreadUpsert>,
        deletes: Vec<String>,
        deleted_messages: Vec<String>,
        cursor: Option<CursorUpdate>,
        first_batch: usize,
    ) -> Result<UpsertAck, SinkError> {
        let batches = batch(upserts, first_batch.clamp(1, MAX_THREADS_PER_CALL));
        let gone: Vec<Vec<String>> = deleted_messages.chunks(MAX_MESSAGES_PER_CALL).map(<[String]>::to_vec).collect();
        let mut ack = UpsertThreadsResult::default();
        let n = batches.len().max(gone.len());
        if n == 0 {
            if deletes.is_empty() {
                if let Some(c) = cursor {
                    self.set_cursor(c).await?;
                }
                return Ok(ack);
            }
            return self.call(vec![], deletes, vec![], cursor).await;
        }
        let mut batches = batches.into_iter();
        let mut gone = gone.into_iter();
        let mut deletes = Some(deletes);
        let mut cursor = cursor;
        for i in 0..n {
            let b = batches.next().unwrap_or_default();
            let g = gone.next().unwrap_or_default();
            let d = deletes.take().unwrap_or_default();
            let c = if i + 1 == n { cursor.take() } else { None };
            let r = self.call(b, d, g, c).await?;
            ack.written += r.written;
            ack.skipped_stale += r.skipped_stale;
            ack.new_inbound.extend(r.new_inbound);
        }
        Ok(ack)
    }

    async fn call(
        &self,
        threads: Vec<ThreadUpsert>,
        deletes: Vec<String>,
        deleted_messages: Vec<String>,
        cursor: Option<CursorUpdate>,
    ) -> Result<UpsertThreadsResult, SinkError> {
        self.api
            .upsert_threads(&UpsertThreadsArgs {
                account_id: self.account_id.clone(),
                worker_id: self.worker_id.clone(),
                threads,
                deletes: (!deletes.is_empty()).then_some(deletes),
                deleted_messages: (!deleted_messages.is_empty()).then_some(deleted_messages),
                cursor,
            })
            .await
            .map_err(|e| sink_error(&e))
    }
}

/// Splits upserts into batches of at most 100 threads and 400 messages (a thread with more than
/// 400 messages goes alone, trimmed to its newest 400 so the call stays inside the limit).
pub fn batch(upserts: Vec<ThreadUpsert>, first_batch: usize) -> Vec<Vec<ThreadUpsert>> {
    let mut out: Vec<Vec<ThreadUpsert>> = Vec::new();
    let mut cur: Vec<ThreadUpsert> = Vec::new();
    let mut msgs = 0usize;
    for mut t in upserts {
        if t.messages.len() > MAX_MESSAGES_PER_CALL {
            t.messages.sort_by_key(|m| std::cmp::Reverse(m.date));
            t.messages.truncate(MAX_MESSAGES_PER_CALL);
            // The list is no longer the whole thread: older messages must not be deleted.
            t.complete = false;
        }
        let limit = if out.is_empty() { first_batch } else { MAX_THREADS_PER_CALL };
        if !cur.is_empty() && (cur.len() >= limit || msgs + t.messages.len() > MAX_MESSAGES_PER_CALL) {
            out.push(std::mem::take(&mut cur));
            msgs = 0;
        }
        msgs += t.messages.len();
        cur.push(t);
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

#[async_trait]
impl MetaSink for ConvexSink {
    async fn upsert_labels(&self, labels: Vec<RemoteLabel>, complete: bool) -> Result<LabelMap, SinkError> {
        let r = self
            .api
            .upsert_labels(&UpsertLabelsArgs {
                account_id: self.account_id.clone(),
                worker_id: self.worker_id.clone(),
                labels,
                complete,
            })
            .await
            .map_err(|e| sink_error(&e))?;
        let map: LabelMap = r.label_key_by_provider_id.into_iter().collect();
        {
            let mut cur = self.labels.write().unwrap_or_else(|e| e.into_inner());
            if complete {
                cur.clone_from(&map);
            } else {
                cur.extend(map.clone());
            }
        }
        Ok(map)
    }

    async fn upsert_threads(
        &self,
        threads: Vec<FetchedThread>,
        deletes: Vec<String>,
        cursor: Option<CursorUpdate>,
    ) -> Result<UpsertAck, SinkError> {
        let upserts = self.to_upserts(&threads);
        self.write_upserts(upserts, deletes, cursor, MAX_THREADS_PER_CALL).await
    }

    async fn set_status(&self, s: AccountStatusUpdate) -> Result<(), SinkError> {
        self.api
            .set_account_status(&SetAccountStatusArgs {
                account_id: self.account_id.clone(),
                worker_id: self.worker_id.clone(),
                status: s.status,
                status_detail: s.status_detail.map(|d| d.map(|c| c.as_str().to_owned())),
                sync_phase: s.sync_phase,
                progress: s.progress,
                oldest_synced_at: s.oldest_synced_at,
                last_sync_at: s.last_sync_at,
            })
            .await
            .map_err(|e| sink_error(&e))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use akatsuki_convex_api::{Convex, MockTransport};
    use akatsuki_core::convex::MessageUpsert;
    use akatsuki_providers::{RemoteMessage, SysLabel};
    use serde_json::json;

    fn upsert(id: &str, messages: usize) -> ThreadUpsert {
        ThreadUpsert {
            provider_thread_id: id.into(),
            complete: true,
            messages: (0..messages)
                .map(|i| MessageUpsert {
                    provider_message_id: format!("{id}-{i}"),
                    date: i as i64,
                    ..Default::default()
                })
                .collect(),
            ..Default::default()
        }
    }

    #[test]
    fn batches_respect_thread_and_message_caps() {
        let b = batch((0..130).map(|i| upsert(&i.to_string(), 1)).collect(), 25);
        assert_eq!(b.iter().map(Vec::len).collect::<Vec<_>>(), vec![25, 100, 5]);
        let b = batch(vec![upsert("a", 300), upsert("b", 150), upsert("c", 10)], 100);
        assert_eq!(b.iter().map(Vec::len).collect::<Vec<_>>(), vec![1, 2]);
        let b = batch(vec![upsert("huge", 500)], 100);
        assert_eq!(b[0][0].messages.len(), 400);
        assert!(!b[0][0].complete);
        assert_eq!(b[0][0].messages[0].date, 499, "newest kept");
    }

    #[tokio::test]
    async fn sink_maps_labels_and_commits_cursor_last() {
        let mock = Convex::new(MockTransport::new());
        mock.transport().respond(
            "worker:upsertLabels",
            Ok(json!({"labelKeyByProviderId": {"INBOX": "sys:inbox", "Label_1": "l:abc"}})),
        );
        for _ in 0..2 {
            mock.transport()
                .respond("worker:upsertThreads", Ok(json!({"written": 1, "skippedStale": 0, "newInbound": []})));
        }
        mock.transport().respond(
            "worker:setAccountStatus",
            Err(ConvexApiError::from_app_error("worker:setAccountStatus", "x".into(), json!({"code": "lease_lost"}))),
        );
        let api = Arc::new(mock);
        let sink = ConvexSink::new(api.clone(), "a1".into(), "w1".into(), vec!["Me@x.com".into()]);
        let map = sink.upsert_labels(vec![], true).await.unwrap();
        assert_eq!(map["INBOX"], "sys:inbox");
        let threads: Vec<FetchedThread> = (0..101)
            .map(|i| FetchedThread {
                provider_thread_id: format!("t{i}"),
                provider_version: Some(7),
                messages: vec![RemoteMessage {
                    provider_message_id: format!("m{i}"),
                    provider_thread_id: format!("t{i}"),
                    provider_label_ids: vec!["INBOX".into(), "Label_1".into()],
                    sys: vec![SysLabel::Unread],
                    from: Some(akatsuki_providers::Addr::new(None, "me@x.com")),
                    ..Default::default()
                }],
                partial: false,
            })
            .collect();
        let ack = sink
            .upsert_threads(threads, vec!["gone".into()], Some(CursorUpdate::Gmail { history_id: "9".into() }))
            .await
            .unwrap();
        assert_eq!(ack.written, 2);
        let calls = api.transport().calls();
        let first = &calls[1].args;
        let second = &calls[2].args;
        assert_eq!(first["threads"].as_array().unwrap().len(), 100);
        assert_eq!(first["deletes"], json!(["gone"]));
        assert!(first.get("cursor").is_none(), "cursor only with the last batch");
        assert_eq!(second["cursor"], json!({"kind": "gmail", "historyId": "9"}));
        let m = &first["threads"][0]["messages"][0];
        assert_eq!(m["labelKeys"], json!(["l:abc", "sys:inbox", "sys:unread"]));
        assert_eq!(m["isOutbound"], json!(true));
        assert_eq!(sink.set_status(AccountStatusUpdate::default()).await.unwrap_err(), SinkError::LeaseLost);
    }
}
