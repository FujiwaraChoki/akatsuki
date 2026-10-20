//! IMAP + SMTP adapter (ARCHITECTURE §9.1, §9.4–§9.7, §10.2, §5.5).
//!
//! - **Connection**: implicit TLS or STARTTLS over rustls, `LOGIN` / `AUTHENTICATE PLAIN` /
//!   `XOAUTH2`, capability detection ([`ImapCaps`]: CONDSTORE, QRESYNC, IDLE, MOVE, UIDPLUS,
//!   SPECIAL-USE, X-GM-EXT-1), `ENABLE QRESYNC`. One command connection per account (§9.4) plus the
//!   IDLE connection owned by [`ImapIdle`].
//! - **Ids**: message `"<folder>:<uidValidity>:<uid>"`; thread `X-GM-THRID` (hex, as the Gmail API
//!   writes it) or the computed id of [`crate::imap_thread`].
//! - **Delta** ([`MailProvider::changes_since`]): `SELECT (QRESYNC …)` with `VANISHED (EARLIER)`;
//!   with CONDSTORE only, `UID FETCH 1:* (FLAGS) (CHANGEDSINCE m)` plus a `UID SEARCH` expunge diff
//!   every 15 minutes; with neither, new UIDs from `UIDNEXT` plus a flag scan every 15 minutes. A
//!   changed `UIDVALIDITY` returns [`ProviderError::CursorExpired`].
//! - **Ops**: flags via `UID STORE`, folders via `UID MOVE` or `UID COPY` + `\Deleted` +
//!   `UID EXPUNGE`/`EXPUNGE`; `COPYUID` rewrites message ids.
//! - **Drafts and sending**: `APPEND Drafts (\Draft)`, SMTP submission ([`smtp`]), `APPEND Sent`
//!   unless the server files sent mail itself, then the draft is removed.
//! - **Discovery**: [`autoconfig`] for the connect form (well-known hosts, Mozilla ISPDB).

pub mod autoconfig;
pub mod cache;
pub mod config;
pub mod folders;
pub mod idle;
pub mod message;
mod ops;
pub mod proto;
pub mod search;
pub(crate) mod session;
pub mod smtp;
mod sync;
pub mod tls;
pub mod utf7;

/// In-process fake IMAP and SMTP servers: the adapter tests, and the local end-to-end harness
/// through the `fake-servers` feature (D-382). Not part of the adapter.
#[cfg(any(test, feature = "fake-servers"))]
#[doc(hidden)]
#[allow(missing_docs, clippy::all)]
pub mod fake;
#[cfg(test)]
mod tests;

use std::collections::{BTreeSet, HashMap};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use async_trait::async_trait;
use futures::StreamExt as _;

pub use config::{
    Credentials, ImapAccountConfig, Security, ServerEndpoint, SyncTuning, Timeouts, TlsOptions, is_loopback_host,
    sent_auto_saved_for,
};
pub use idle::ImapIdle;
pub use session::ConnectError;
pub use smtp::{AMBIGUOUS_SEND, SmtpError, is_ambiguous_send};

use self::cache::ThreadCache;
use self::folders::{Folder, folders_from_list};
use self::message::{FolderCtx, SNIPPET_BYTES, build_message, metadata_items, snippet_from_partial};
use self::session::{ImapError, ImapSession, SelectMode, Selected};
use crate::{
    ByteStream, ChangePage, Cursor, FetchDepth, FetchedThread, ImapCaps, LabelRole, MailProvider, OpEffect, OpId,
    OutgoingMime, ProviderCaps, ProviderError, ProviderKind, ProviderOp, RemoteDraftRef, RemoteLabel, RemoteMessage,
    RemoteMessageRef, RemoteThreadId, SentRef, WatchState, WindowPage, WindowRequest,
};

/// What a successful connect test learned (`mailAccounts.imap.capabilities`, `sentAutoSaved`, caps).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConnectReport {
    /// Advertised capabilities (uppercased).
    pub capabilities: Vec<String>,
    /// Parsed extensions.
    pub imap: ImapCaps,
    /// The provider files sent mail itself.
    pub sent_auto_saved: bool,
}

/// A failed connect test, with the §7.2 error code to answer.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("{code:?}")]
pub struct ConnectFailure {
    /// `imap_auth_failed`, `smtp_auth_failed`, `tls_failed` or `unreachable`.
    pub code: akatsuki_core::wire::ErrorCode,
}

/// `POST /v1/connect/imap` (§6.7): a live IMAP login and an SMTP AUTH without DATA.
pub async fn test_connection(cfg: &ImapAccountConfig) -> Result<ConnectReport, ConnectFailure> {
    let s = ImapSession::connect(cfg).await.map_err(|e| ConnectFailure { code: e.code(false) })?;
    let report = ConnectReport {
        capabilities: s.caps.raw.clone(),
        imap: s.caps.imap.clone(),
        sent_auto_saved: cfg
            .sent_auto_saved
            .unwrap_or_else(|| sent_auto_saved_for(&cfg.imap.host, s.caps.imap.x_gm_ext)),
    };
    s.logout().await;
    smtp::check_login(cfg).await.map_err(|e| {
        use akatsuki_core::wire::ErrorCode as C;
        let code = if e.is_auth() {
            C::SmtpAuthFailed
        } else if e.is_tls() {
            C::TlsFailed
        } else {
            C::Unreachable
        };
        ConnectFailure { code }
    })?;
    Ok(report)
}

/// Per-folder scan memory for servers without QRESYNC (expunge diff, flag scan).
#[derive(Clone, Debug, Default)]
pub(crate) struct FolderScan {
    pub uid_validity: u32,
    /// UID → (internal date, flags fingerprint).
    pub known: HashMap<u32, (i64, u64)>,
    pub last_scan: Option<Instant>,
}

#[derive(Debug, Default)]
pub(crate) struct State {
    pub folders: Vec<Folder>,
    pub keywords: BTreeSet<String>,
    pub keywords_allowed: bool,
    pub cache: ThreadCache,
    pub scans: HashMap<String, FolderScan>,
}

/// One IMAP account.
pub struct ImapProvider {
    pub(crate) cfg: Arc<ImapAccountConfig>,
    caps: ProviderCaps,
    capabilities: Vec<String>,
    conn: tokio::sync::Mutex<Option<ImapSession>>,
    pub(crate) state: Mutex<State>,
}

impl std::fmt::Debug for ImapProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ImapProvider").field("caps", &self.caps).finish_non_exhaustive()
    }
}

/// A locked command connection that reconnects on demand and is dropped after a broken read.
pub(crate) struct Conn<'a> {
    guard: tokio::sync::MutexGuard<'a, Option<ImapSession>>,
}

impl Conn<'_> {
    pub fn s(&mut self) -> &mut ImapSession {
        self.guard.as_mut().unwrap_or_else(|| unreachable!("Conn always holds a session"))
    }

    /// Maps an error, dropping the connection when it is broken.
    pub fn check<T>(&mut self, r: Result<T, ImapError>) -> Result<T, ProviderError> {
        match r {
            Ok(v) => Ok(v),
            Err(e) => {
                if e.is_fatal() {
                    *self.guard = None;
                }
                Err(e.into_provider())
            }
        }
    }

    /// The connection still works.
    pub fn alive(&self) -> bool {
        self.guard.is_some()
    }
}

pub(crate) fn lock_state(m: &Mutex<State>) -> std::sync::MutexGuard<'_, State> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

impl ImapProvider {
    /// Connects, detects capabilities and lists folders.
    pub async fn connect(cfg: ImapAccountConfig) -> Result<Self, ProviderError> {
        let s = ImapSession::connect(&cfg).await.map_err(ConnectError::into_provider)?;
        let imap = s.caps.imap.clone();
        let sent_auto_saved = cfg.sent_auto_saved.unwrap_or_else(|| sent_auto_saved_for(&cfg.imap.host, imap.x_gm_ext));
        let mut caps = ProviderCaps::imap(imap, sent_auto_saved);
        if !caps.imap.as_ref().is_some_and(|c| c.idle) {
            caps.push = crate::PushKind::Poll { every: cfg.tuning.poll_every };
        }
        let p = ImapProvider {
            capabilities: s.caps.raw.clone(),
            cfg: Arc::new(cfg),
            caps,
            conn: tokio::sync::Mutex::new(Some(s)),
            state: Mutex::new(State::default()),
        };
        let mut c = p.conn().await?;
        p.refresh_folders(&mut c).await?;
        drop(c);
        Ok(p)
    }

    /// Advertised capabilities, for `mailAccounts.imap.capabilities`.
    pub fn capabilities(&self) -> &[String] {
        &self.capabilities
    }

    /// The account configuration.
    pub fn config(&self) -> &ImapAccountConfig {
        &self.cfg
    }

    /// An IDLE stream on a second connection (§9.4, §9.6).
    pub fn idle_source(&self) -> ImapIdle {
        ImapIdle::new(self.cfg.clone())
    }

    fn imap_caps(&self) -> ImapCaps {
        self.caps.imap.clone().unwrap_or_default()
    }

    pub(crate) async fn conn(&self) -> Result<Conn<'_>, ProviderError> {
        let mut guard = self.conn.lock().await;
        if guard.is_none() {
            let s = ImapSession::connect(&self.cfg).await.map_err(ConnectError::into_provider)?;
            *guard = Some(s);
        }
        Ok(Conn { guard })
    }

    pub(crate) async fn refresh_folders(&self, c: &mut Conn<'_>) -> Result<Vec<Folder>, ProviderError> {
        let r = c.s().list().await;
        let list = c.check(r)?;
        let folders = folders_from_list(list);
        lock_state(&self.state).folders = folders.clone();
        Ok(folders)
    }

    pub(crate) fn folders(&self) -> Vec<Folder> {
        lock_state(&self.state).folders.clone()
    }

    pub(crate) fn folder(&self, name: &str) -> Option<Folder> {
        lock_state(&self.state).folders.iter().find(|f| f.name == name).cloned()
    }

    pub(crate) fn folder_by_role(&self, role: LabelRole) -> Option<Folder> {
        lock_state(&self.state).folders.iter().find(|f| f.role == Some(role)).cloned()
    }

    /// Selects `name` unless it is already selected.
    pub(crate) async fn ensure_selected(&self, c: &mut Conn<'_>, name: &str) -> Result<Selected, ProviderError> {
        if let Some(sel) = c.s().selected.as_ref().filter(|s| s.name == name) {
            return Ok(sel.clone());
        }
        self.select(c, name, SelectMode::Plain).await
    }

    pub(crate) async fn select(
        &self,
        c: &mut Conn<'_>,
        name: &str,
        mode: SelectMode,
    ) -> Result<Selected, ProviderError> {
        let r = c.s().select(name, mode, false).await.cloned();
        let sel = c.check(r)?;
        let mut st = lock_state(&self.state);
        // After a UIDVALIDITY change the index must not hand out ids of the old numbering.
        st.cache.retain_folder_validity(name, sel.uid_validity);
        if name.eq_ignore_ascii_case("INBOX") {
            st.keywords_allowed = sel.keywords_allowed();
        }
        for f in &sel.flags {
            if !f.starts_with('\\') && !folders::is_ignored_keyword(f) {
                st.keywords.insert(f.clone());
            }
        }
        Ok(sel)
    }

    /// Fetches header-level messages for `uids` of the selected folder (two passes: metadata, then
    /// the first 1 KB of each text part for the snippet) and records them in the thread index.
    pub(crate) async fn fetch_messages(
        &self,
        c: &mut Conn<'_>,
        folder: &Folder,
        sel: &Selected,
        uids: &[u32],
    ) -> Result<Vec<RemoteMessage>, ProviderError> {
        if uids.is_empty() {
            return Ok(Vec::new());
        }
        let caps = self.imap_caps();
        let items = metadata_items(caps.x_gm_ext, caps.condstore && sel.highest_modseq.is_some());
        let r = c.s().uid_fetch(uids, &items, None).await;
        let fetched = c.check(r)?;
        self.note_scan(&folder.name, sel.uid_validity, &fetched);
        // Second pass: group by text part path so one FETCH covers many messages.
        let mut by_path: HashMap<String, Vec<u32>> = HashMap::new();
        for f in &fetched {
            if let (Some(uid), Some(tp)) = (f.uid, f.structure.as_ref().and_then(|s| s.text_part.as_ref())) {
                by_path.entry(tp.path.clone()).or_default().push(uid);
            }
        }
        let mut partials: HashMap<u32, Vec<u8>> = HashMap::new();
        for (path, uids) in by_path {
            let items = format!("(UID BODY.PEEK[{path}]<0.{SNIPPET_BYTES}>)");
            let r = c.s().uid_fetch(&uids, &items, None).await;
            for f in c.check(r)? {
                if let (Some(uid), Some(sec)) = (f.uid, f.sections.into_iter().find(|s| s.key == path)) {
                    partials.insert(uid, sec.data);
                }
            }
        }
        let keywords_allowed = lock_state(&self.state).keywords_allowed || sel.keywords_allowed();
        let ctx = FolderCtx { name: &folder.name, uid_validity: sel.uid_validity, role: folder.role, keywords_allowed };
        let mut out = Vec::with_capacity(fetched.len());
        for f in &fetched {
            let snippet =
                match (f.uid.and_then(|u| partials.get(&u)), f.structure.as_ref().and_then(|s| s.text_part.as_ref())) {
                    (Some(data), Some(tp)) => snippet_from_partial(tp, data),
                    _ => String::new(),
                };
            if let Some(m) = build_message(&ctx, f, &self.cfg.account_id, snippet) {
                out.push(m);
            }
        }
        out.sort_by_key(|m| m.imap.first().map(|l| l.uid).unwrap_or_default());
        let mut st = lock_state(&self.state);
        for m in &out {
            st.cache.insert(m.clone());
        }
        Ok(out)
    }

    /// Re-reads messages at the given locations (post-op state, search hits).
    pub(crate) async fn fetch_locations(
        &self,
        c: &mut Conn<'_>,
        locations: &[crate::ImapLocation],
    ) -> Result<Vec<RemoteMessage>, ProviderError> {
        let mut by_folder: Vec<(String, Vec<&crate::ImapLocation>)> = Vec::new();
        for l in locations {
            match by_folder.iter_mut().find(|(f, _)| *f == l.remote_folder_id) {
                Some((_, v)) => v.push(l),
                None => by_folder.push((l.remote_folder_id.clone(), vec![l])),
            }
        }
        let mut out = Vec::new();
        for (name, locs) in by_folder {
            let Some(folder) = self.folder(&name) else { continue };
            let sel = match self.ensure_selected(c, &name).await {
                Ok(s) => s,
                Err(ProviderError::NotFound) => continue,
                Err(e) => return Err(e),
            };
            let uids: Vec<u32> = locs.iter().filter(|l| l.uid_validity == sel.uid_validity).map(|l| l.uid).collect();
            out.extend(self.fetch_messages(c, &folder, &sel, &uids).await?);
        }
        Ok(out)
    }

    /// Locations of a message: its `imap` list, else parsed from the provider id.
    pub(crate) fn locations_of(m: &RemoteMessageRef) -> Vec<crate::ImapLocation> {
        if !m.imap.is_empty() {
            return m.imap.clone();
        }
        message::parse_message_id(&m.provider_message_id).into_iter().collect()
    }

    /// The raw source of the first location that still exists.
    pub(crate) async fn raw_message(&self, c: &mut Conn<'_>, m: &RemoteMessageRef) -> Result<Vec<u8>, ProviderError> {
        for loc in Self::locations_of(m) {
            if self.folder(&loc.remote_folder_id).is_none() {
                continue;
            }
            let sel = match self.ensure_selected(c, &loc.remote_folder_id).await {
                Ok(s) => s,
                Err(ProviderError::NotFound) => continue,
                Err(e) => return Err(e),
            };
            if sel.uid_validity != loc.uid_validity {
                continue;
            }
            let r = c.s().uid_fetch(&[loc.uid], "(UID BODY.PEEK[])", None).await;
            if let Some(f) = c.check(r)?.into_iter().next()
                && let Some(data) = f.section("")
            {
                return Ok(data.to_vec());
            }
        }
        Err(ProviderError::NotFound)
    }
}

fn once_stream(bytes: Vec<u8>) -> ByteStream {
    futures::stream::once(async move { Ok::<_, ProviderError>(bytes::Bytes::from(bytes)) }).boxed()
}

#[async_trait]
impl MailProvider for ImapProvider {
    fn kind(&self) -> ProviderKind {
        ProviderKind::Imap
    }

    fn caps(&self) -> &ProviderCaps {
        &self.caps
    }

    async fn list_labels(&self) -> Result<Vec<RemoteLabel>, ProviderError> {
        let mut c = self.conn().await?;
        let folders = self.refresh_folders(&mut c).await?;
        if let Some(inbox) = folders.iter().find(|f| f.role == Some(LabelRole::Inbox)) {
            // FLAGS / PERMANENTFLAGS of INBOX tell whether keywords are labels and which exist.
            self.select(&mut c, &inbox.name.clone(), SelectMode::Plain).await?;
        }
        let st = lock_state(&self.state);
        let mut labels: Vec<RemoteLabel> = folders.iter().filter(|f| f.synced()).map(Folder::label).collect();
        if st.keywords_allowed {
            labels.extend(st.keywords.iter().map(|k| folders::keyword_label(k)));
        }
        Ok(labels)
    }

    async fn ensure_label(&self, role: LabelRole, name: &str) -> Result<RemoteLabel, ProviderError> {
        self.ensure_role_folder(role, name).await.map(|f| f.label())
    }

    async fn baseline(&self) -> Result<Cursor, ProviderError> {
        self.baseline_impl().await
    }

    async fn changes_since(&self, cursor: &Cursor) -> Result<ChangePage, ProviderError> {
        self.changes_since_impl(cursor).await
    }

    async fn list_window(&self, req: &WindowRequest) -> Result<WindowPage, ProviderError> {
        self.list_window_impl(req).await
    }

    async fn fetch_threads(
        &self,
        ids: &[RemoteThreadId],
        depth: FetchDepth,
    ) -> Result<Vec<FetchedThread>, ProviderError> {
        self.fetch_threads_impl(ids, depth).await
    }

    async fn fetch_raw(&self, msg: &RemoteMessageRef) -> Result<ByteStream, ProviderError> {
        let mut c = self.conn().await?;
        let raw = self.raw_message(&mut c, msg).await?;
        Ok(once_stream(raw))
    }

    async fn fetch_part(&self, msg: &RemoteMessageRef, part_id: &str) -> Result<ByteStream, ProviderError> {
        if !proto::is_part_path(part_id) {
            return Err(ProviderError::Invalid("part id must be an IMAP section path".into()));
        }
        let mut c = self.conn().await?;
        let bytes = self.part_bytes(&mut c, msg, part_id).await?;
        Ok(once_stream(bytes))
    }

    async fn apply(&self, ops: &[(OpId, ProviderOp)]) -> Vec<(OpId, Result<OpEffect, ProviderError>)> {
        self.apply_impl(ops).await
    }

    async fn save_draft(
        &self,
        mime: &OutgoingMime,
        existing: Option<&RemoteDraftRef>,
    ) -> Result<RemoteDraftRef, ProviderError> {
        self.save_draft_impl(mime, existing).await
    }

    async fn delete_draft(&self, d: &RemoteDraftRef) -> Result<(), ProviderError> {
        self.delete_draft_impl(d).await
    }

    async fn send_draft(&self, d: &RemoteDraftRef, rfc_message_id: &str) -> Result<SentRef, ProviderError> {
        self.send_draft_impl(d, rfc_message_id).await
    }

    async fn find_sent(&self, rfc_message_id: &str) -> Result<Option<SentRef>, ProviderError> {
        self.find_sent_impl(rfc_message_id).await
    }

    async fn search(
        &self,
        q: &akatsuki_search::query::Ast,
        page: Option<String>,
    ) -> Result<(Vec<RemoteThreadId>, Option<String>), ProviderError> {
        self.search_impl(q, page).await
    }

    async fn watch(
        &self,
        _current: Option<&WatchState>,
        _hook_base: &str,
    ) -> Result<Option<WatchState>, ProviderError> {
        // IMAP pushes through IDLE (`idle_source`), not a provider-side subscription.
        Ok(None)
    }

    async fn unwatch(&self, _w: &WatchState) -> Result<(), ProviderError> {
        Ok(())
    }

    async fn revoke(&self) -> Result<(), ProviderError> {
        // Nothing to revoke for a password login; close the connection.
        let mut guard = self.conn.lock().await;
        if let Some(s) = guard.take() {
            s.logout().await;
        }
        Ok(())
    }
}
