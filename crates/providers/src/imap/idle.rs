//! IMAP push (§9.6): IDLE on INBOX on a dedicated connection, re-issued every 25 minutes, plus a
//! `STATUS (UIDNEXT HIGHESTMODSEQ MESSAGES)` sweep of the other folders every 5 minutes. Servers
//! without IDLE are polled. NOTIFY (RFC 5465) is not used yet.

use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use tokio::time::Instant;

use super::config::ImapAccountConfig;
use super::folders::{Folder, folders_from_list};
use super::session::{ImapSession, MailboxStatus, SelectMode};
use crate::{LabelRole, ProviderError, PushEvent, PushSource};

/// The IDLE connection of one account.
pub struct ImapIdle {
    cfg: Arc<ImapAccountConfig>,
    session: Option<ImapSession>,
    inbox: String,
    folders: Vec<Folder>,
    last_status: HashMap<String, MailboxStatus>,
    next_sweep: Instant,
    pending: VecDeque<PushEvent>,
    failures: u32,
    connected_once: bool,
}

impl std::fmt::Debug for ImapIdle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ImapIdle").field("connected", &self.session.is_some()).finish_non_exhaustive()
    }
}

impl ImapIdle {
    /// A source that connects on the first [`PushSource::next`].
    pub fn new(cfg: Arc<ImapAccountConfig>) -> Self {
        let sweep = cfg.tuning.status_sweep_every;
        Self {
            cfg,
            session: None,
            inbox: "INBOX".into(),
            folders: Vec::new(),
            last_status: HashMap::new(),
            next_sweep: Instant::now() + sweep,
            pending: VecDeque::new(),
            failures: 0,
            connected_once: false,
        }
    }

    async fn connect(&mut self) -> Result<(), ProviderError> {
        if self.failures > 0 {
            // Exponential backoff between reconnects, capped at 5 minutes.
            let ms = akatsuki_core::sync::backoff_ms(self.failures.min(10), 1.0).min(300_000);
            tokio::time::sleep(Duration::from_millis(ms.unsigned_abs())).await;
        }
        let mut s = ImapSession::connect(&self.cfg).await.map_err(super::session::ConnectError::into_provider)?;
        let list = s.list().await.map_err(|e| e.into_provider())?;
        self.folders = folders_from_list(list);
        if let Some(i) = self.folders.iter().find(|f| f.role == Some(LabelRole::Inbox)) {
            self.inbox = i.name.clone();
        }
        let inbox = self.inbox.clone();
        s.select(&inbox, SelectMode::Plain, true).await.map_err(|e| e.into_provider())?;
        self.last_status.clear();
        self.sweep_with(&mut s, true).await?;
        self.session = Some(s);
        self.failures = 0;
        if self.connected_once {
            self.pending.push_back(PushEvent::Reconnected);
        }
        self.connected_once = true;
        Ok(())
    }

    async fn sweep_with(&mut self, s: &mut ImapSession, record_only: bool) -> Result<(), ProviderError> {
        let condstore = s.caps.imap.condstore;
        let poll_inbox = !s.caps.imap.idle;
        let names: Vec<String> = self
            .folders
            .iter()
            .filter(|f| f.synced() && (poll_inbox || f.name != self.inbox))
            .map(|f| f.name.clone())
            .collect();
        for name in names {
            let st = match s.status(&name, condstore).await {
                Ok(st) => st,
                Err(e) if e.is_fatal() => return Err(e.into_provider()),
                Err(_) => continue,
            };
            let changed = self.last_status.get(&name).is_some_and(|old| *old != st);
            self.last_status.insert(name.clone(), st);
            if changed && !record_only {
                self.pending.push_back(PushEvent::FolderChanged { remote_folder_id: name });
            }
        }
        Ok(())
    }

    async fn step(&mut self) -> Result<(), ProviderError> {
        let Some(mut s) = self.session.take() else {
            let r = self.connect().await;
            if r.is_err() {
                self.failures += 1;
            }
            return r;
        };
        let idle = s.caps.imap.idle;
        let every = if idle { self.cfg.tuning.status_sweep_every } else { self.cfg.tuning.poll_every };
        let now = Instant::now();
        if now >= self.next_sweep {
            self.next_sweep = now + every;
            let r = self.sweep_with(&mut s, false).await;
            self.session = Some(s);
            return r;
        }
        let until_sweep = self.next_sweep.saturating_duration_since(now);
        if !idle {
            self.session = Some(s);
            tokio::time::sleep(until_sweep).await;
            return Ok(());
        }
        let wait = self.cfg.timeouts.idle_reissue.min(until_sweep).max(Duration::from_millis(10));
        match s.idle(wait).await {
            Ok((s, changed)) => {
                self.session = Some(s);
                if changed {
                    self.pending.push_back(PushEvent::FolderChanged { remote_folder_id: self.inbox.clone() });
                }
                Ok(())
            }
            Err(e) => {
                self.failures += 1;
                Err(e.into_provider())
            }
        }
    }
}

#[async_trait]
impl PushSource for ImapIdle {
    async fn next(&mut self) -> Result<PushEvent, ProviderError> {
        loop {
            if let Some(e) = self.pending.pop_front() {
                return Ok(e);
            }
            if let Err(e) = self.step().await {
                // Drop the connection; the next call reconnects (and reports `Reconnected`).
                self.session = None;
                return Err(e);
            }
        }
    }
}
