//! `FakeProvider`: an in-memory mailbox behind `MailProvider` for engine tests. Every call is
//! recorded; errors and change pages can be scripted.

use std::collections::{BTreeMap, HashMap, VecDeque};
use std::sync::Mutex;

use akatsuki_providers::{
    ByteStream, ChangePage, Cursor, FetchDepth, FetchedThread, LabelKind, LabelRole, MailProvider, OpEffect, OpId,
    OutgoingMime, ProviderCaps, ProviderError, ProviderKind, ProviderOp, RemoteDraftRef, RemoteLabel, RemoteMessage,
    RemoteMessageRef, RemoteThreadId, SentRef, SysLabel, WatchKind, WatchState, WindowPage, WindowRequest,
};
use async_trait::async_trait;
use futures::StreamExt as _;

/// Scripted state of the fake mailbox.
#[derive(Default)]
pub struct FakeState {
    /// Labels.
    pub labels: Vec<RemoteLabel>,
    /// Threads by provider id.
    pub threads: BTreeMap<String, FetchedThread>,
    /// Inbox thread ids, newest first.
    pub inbox: Vec<String>,
    /// Window thread ids (everything since windowStartAt), newest first.
    pub window: Vec<String>,
    /// Older threads for backfill: (id, date).
    pub older: Vec<(String, i64)>,
    /// Page size for listings.
    pub page_size: usize,
    /// Head history id.
    pub history: u64,
    /// Scripted `changes_since` results (default: an empty page at the head).
    pub changes: VecDeque<Result<ChangePage, ProviderError>>,
    /// Scripted errors for the next `list_window` calls.
    pub list_errors: VecDeque<ProviderError>,
    /// Scripted `apply` results by op id (default: applied, post = the stored thread).
    pub apply_results: HashMap<String, Result<OpEffect, ProviderError>>,
    /// Every applied op.
    pub applied: Vec<(OpId, ProviderOp)>,
    /// Sent messages by rfc id.
    pub sent: HashMap<String, SentRef>,
    /// Scripted `send_draft` errors (consumed in order). `accept_then_fail` makes the provider
    /// keep the message even though the call errors (an ambiguous send).
    pub send_errors: VecDeque<(ProviderError, bool)>,
    /// `send_draft` calls.
    pub send_calls: u32,
    /// `find_sent` calls.
    pub find_sent_calls: u32,
    /// Raw sources by provider message id.
    pub raw: HashMap<String, Vec<u8>>,
    /// Drafts saved: (draft id, rfc id, bytes).
    pub drafts: Vec<(String, String, Vec<u8>)>,
    /// Deleted drafts.
    pub deleted_drafts: Vec<String>,
    /// Labels created by `ensure_label`.
    pub ensured: Vec<LabelRole>,
    /// `watch` calls.
    pub watch_calls: u32,
    /// Call log (method names).
    pub calls: Vec<&'static str>,
    /// Queries passed to `search`.
    pub searched: Vec<akatsuki_search::query::Ast>,
}

/// The fake provider.
pub struct FakeProvider {
    kind: ProviderKind,
    caps: ProviderCaps,
    /// State (inspect and script in tests).
    pub state: Mutex<FakeState>,
}

impl std::fmt::Debug for FakeProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FakeProvider").field("kind", &self.kind).finish_non_exhaustive()
    }
}

/// A message for tests.
pub fn message(thread: &str, id: &str, date: i64, labels: &[&str], sys: &[SysLabel]) -> RemoteMessage {
    RemoteMessage {
        provider_message_id: id.into(),
        provider_thread_id: thread.into(),
        rfc_message_id: Some(format!("{id}@example.com")),
        from: Some(akatsuki_providers::Addr::new(Some("Ada"), "ada@example.com")),
        to: vec![akatsuki_providers::Addr::new(None, "me@example.com")],
        subject: format!("Subject {thread}"),
        snippet: "hello".into(),
        date,
        provider_label_ids: labels.iter().map(|s| (*s).to_owned()).collect(),
        sys: sys.to_vec(),
        ..Default::default()
    }
}

/// Gmail-like system labels.
pub fn gmail_labels() -> Vec<RemoteLabel> {
    let sys = |id: &str, role: LabelRole| RemoteLabel {
        provider_label_id: id.into(),
        name: id.into(),
        kind: LabelKind::System,
        role: Some(role),
        exclusive: false,
        color: None,
        hidden: false,
    };
    vec![sys("INBOX", LabelRole::Inbox), sys("SENT", LabelRole::Sent), sys("TRASH", LabelRole::Trash)]
}

impl FakeProvider {
    /// A Gmail-like fake (multi-label, poll push).
    pub fn gmail() -> Self {
        let mut caps = ProviderCaps::gmail();
        caps.push = akatsuki_providers::PushKind::GmailPubSub;
        Self {
            kind: ProviderKind::Gmail,
            caps,
            state: Mutex::new(FakeState { labels: gmail_labels(), page_size: 50, history: 100, ..Default::default() }),
        }
    }

    /// Locks the state.
    pub fn st(&self) -> std::sync::MutexGuard<'_, FakeState> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Adds a thread (and lists it in the inbox and window when `inbox`).
    pub fn add_thread(&self, t: FetchedThread, inbox: bool) {
        let mut st = self.st();
        if inbox {
            st.inbox.push(t.provider_thread_id.clone());
        }
        st.window.push(t.provider_thread_id.clone());
        st.threads.insert(t.provider_thread_id.clone(), t);
    }

    fn page(ids: &[String], page: Option<&str>, size: usize) -> WindowPage {
        let start: usize = page.and_then(|p| p.parse().ok()).unwrap_or(0);
        let end = (start + size.max(1)).min(ids.len());
        let slice = ids.get(start..end).unwrap_or(&[]);
        WindowPage {
            threads: slice.iter().map(|i| RemoteThreadId(i.clone())).collect(),
            next_page: (end < ids.len()).then(|| end.to_string()),
            oldest: None,
        }
    }
}

#[async_trait]
impl MailProvider for FakeProvider {
    fn kind(&self) -> ProviderKind {
        self.kind
    }

    fn caps(&self) -> &ProviderCaps {
        &self.caps
    }

    async fn list_labels(&self) -> Result<Vec<RemoteLabel>, ProviderError> {
        let mut st = self.st();
        st.calls.push("list_labels");
        Ok(st.labels.clone())
    }

    async fn ensure_label(&self, role: LabelRole, name: &str) -> Result<RemoteLabel, ProviderError> {
        let mut st = self.st();
        st.calls.push("ensure_label");
        st.ensured.push(role);
        let l = RemoteLabel {
            provider_label_id: format!("Label_{}", name.replace('/', "_")),
            name: name.into(),
            kind: LabelKind::User,
            role: Some(role),
            exclusive: false,
            color: None,
            hidden: false,
        };
        st.labels.push(l.clone());
        Ok(l)
    }

    async fn baseline(&self) -> Result<Cursor, ProviderError> {
        let mut st = self.st();
        st.calls.push("baseline");
        Ok(Cursor::Gmail { history_id: st.history })
    }

    async fn changes_since(&self, cursor: &Cursor) -> Result<ChangePage, ProviderError> {
        let mut st = self.st();
        st.calls.push("changes_since");
        if let Some(r) = st.changes.pop_front() {
            return r;
        }
        Ok(ChangePage {
            touched_threads: vec![],
            deleted_threads: vec![],
            deleted_messages: vec![],
            labels_changed: false,
            next: cursor.clone(),
            more: false,
        })
    }

    async fn list_window(&self, req: &WindowRequest) -> Result<WindowPage, ProviderError> {
        let mut st = self.st();
        st.calls.push("list_window");
        if let Some(e) = st.list_errors.pop_front() {
            return Err(e);
        }
        let size = st.page_size;
        Ok(match req {
            WindowRequest::Inbox { page } => Self::page(&st.inbox, page.as_deref(), size),
            WindowRequest::Since { page, .. } => Self::page(&st.window, page.as_deref(), size),
            WindowRequest::Before { before, horizon, limit } => {
                let mut hits: Vec<&(String, i64)> =
                    st.older.iter().filter(|(_, d)| *d < *before && *d >= *horizon).collect();
                hits.sort_by_key(|(_, d)| std::cmp::Reverse(*d));
                hits.truncate(*limit as usize);
                WindowPage {
                    oldest: hits.iter().map(|(_, d)| *d).min(),
                    threads: hits.iter().map(|(i, _)| RemoteThreadId(i.clone())).collect(),
                    next_page: None,
                }
            }
        })
    }

    async fn fetch_threads(
        &self,
        ids: &[RemoteThreadId],
        _depth: FetchDepth,
    ) -> Result<Vec<FetchedThread>, ProviderError> {
        let mut st = self.st();
        st.calls.push("fetch_threads");
        Ok(ids
            .iter()
            .map(|i| {
                st.threads.get(&i.0).cloned().unwrap_or_else(|| FetchedThread {
                    provider_thread_id: i.0.clone(),
                    provider_version: Some(1),
                    messages: vec![message(&i.0, &format!("{}-m", i.0), 1, &["INBOX"], &[])],
                    partial: false,
                })
            })
            .collect())
    }

    async fn fetch_raw(&self, msg: &RemoteMessageRef) -> Result<ByteStream, ProviderError> {
        let raw = {
            let mut st = self.st();
            st.calls.push("fetch_raw");
            st.raw.get(&msg.provider_message_id).cloned().ok_or(ProviderError::NotFound)?
        };
        Ok(futures::stream::iter(raw.chunks(64).map(|c| Ok(bytes::Bytes::copy_from_slice(c))).collect::<Vec<_>>())
            .boxed())
    }

    async fn fetch_part(&self, msg: &RemoteMessageRef, _part_id: &str) -> Result<ByteStream, ProviderError> {
        self.fetch_raw(msg).await
    }

    async fn apply(&self, ops: &[(OpId, ProviderOp)]) -> Vec<(OpId, Result<OpEffect, ProviderError>)> {
        let mut st = self.st();
        st.calls.push("apply");
        let mut out = Vec::new();
        for (id, op) in ops {
            st.applied.push((id.clone(), op.clone()));
            let r = st.apply_results.remove(id).unwrap_or_else(|| {
                let post = match op {
                    ProviderOp::Modify { thread, .. }
                    | ProviderOp::Trash { thread }
                    | ProviderOp::Untrash { thread }
                    | ProviderOp::Move { thread, .. } => st.threads.get(&thread.provider_thread_id).cloned(),
                    _ => None,
                };
                let created = match op {
                    ProviderOp::LabelCreate { name, .. } => Some(RemoteLabel {
                        provider_label_id: format!("Label_{name}"),
                        name: name.clone(),
                        kind: LabelKind::User,
                        role: None,
                        exclusive: false,
                        color: None,
                        hidden: false,
                    }),
                    _ => None,
                };
                Ok(OpEffect::Applied { post, created })
            });
            out.push((id.clone(), r));
        }
        out
    }

    async fn save_draft(
        &self,
        mime: &OutgoingMime,
        existing: Option<&RemoteDraftRef>,
    ) -> Result<RemoteDraftRef, ProviderError> {
        let mut st = self.st();
        st.calls.push("save_draft");
        let id = existing.map(|d| d.provider_draft_id.clone()).unwrap_or_else(|| format!("d{}", st.drafts.len() + 1));
        st.drafts.retain(|(d, _, _)| *d != id);
        st.drafts.push((id.clone(), mime.rfc_message_id.clone(), mime.bytes.to_vec()));
        Ok(RemoteDraftRef { provider_draft_id: id.clone(), provider_message_id: Some(format!("{id}-msg")) })
    }

    async fn delete_draft(&self, d: &RemoteDraftRef) -> Result<(), ProviderError> {
        let mut st = self.st();
        st.calls.push("delete_draft");
        st.deleted_drafts.push(d.provider_draft_id.clone());
        Ok(())
    }

    async fn send_draft(&self, d: &RemoteDraftRef, rfc_message_id: &str) -> Result<SentRef, ProviderError> {
        let mut st = self.st();
        st.calls.push("send_draft");
        st.send_calls += 1;
        let sent = SentRef { provider_message_id: format!("sent-{}", d.provider_draft_id), provider_thread_id: None };
        if let Some((e, accepted)) = st.send_errors.pop_front() {
            if accepted {
                st.sent.insert(rfc_message_id.to_owned(), sent);
            }
            return Err(e);
        }
        st.sent.insert(rfc_message_id.to_owned(), sent.clone());
        Ok(sent)
    }

    async fn find_sent(&self, rfc_message_id: &str) -> Result<Option<SentRef>, ProviderError> {
        let mut st = self.st();
        st.calls.push("find_sent");
        st.find_sent_calls += 1;
        Ok(st.sent.get(rfc_message_id).cloned())
    }

    async fn search(
        &self,
        q: &akatsuki_search::query::Ast,
        _page: Option<String>,
    ) -> Result<(Vec<RemoteThreadId>, Option<String>), ProviderError> {
        let mut st = self.st();
        st.calls.push("search");
        st.searched.push(q.clone());
        Ok((st.window.iter().take(2).map(|i| RemoteThreadId(i.clone())).collect(), Some("p2".into())))
    }

    async fn watch(&self, current: Option<&WatchState>, _hook_base: &str) -> Result<Option<WatchState>, ProviderError> {
        let mut st = self.st();
        st.calls.push("watch");
        st.watch_calls += 1;
        Ok(Some(WatchState {
            kind: WatchKind::GmailWatch,
            external_id: None,
            expires_at: akatsuki_core::time::now_ms() + 7 * akatsuki_core::time::DAY_MS,
            client_state: current.and_then(|c| c.client_state.clone()),
        }))
    }

    async fn unwatch(&self, _w: &WatchState) -> Result<(), ProviderError> {
        self.st().calls.push("unwatch");
        Ok(())
    }

    async fn revoke(&self) -> Result<(), ProviderError> {
        self.st().calls.push("revoke");
        Ok(())
    }
}
