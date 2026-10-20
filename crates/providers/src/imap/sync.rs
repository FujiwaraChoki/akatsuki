//! Baseline, delta, window listing, thread fetch, search and parts (§9.3, §9.5, §11.2).

use std::collections::hash_map::DefaultHasher;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::hash::{Hash, Hasher};
use std::time::Instant;

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde::{Deserialize, Serialize};

use super::folders::{Folder, mailbox_for_path};
use super::message::message_id;
use super::proto::{self, Cmd};
use super::session::{FetchItem, SelectMode, Selected};
use super::{Conn, ImapProvider, lock_state, search};
use crate::{
    ChangePage, Cursor, FetchDepth, FetchedThread, ImapFolderCursor, LabelRole, ProviderError, RemoteMessage,
    RemoteMessageRef, RemoteThreadId, WindowPage, WindowRequest,
};

/// First InboxFirst page (§9.3: the first 25 threads in one mutation).
const FIRST_PAGE: usize = akatsuki_core::sync::FIRST_PAINT_BATCH;
/// Later pages.
const PAGE: usize = akatsuki_core::sync::UPSERT_BATCH;
/// Provider search page.
const SEARCH_PAGE: usize = 50;

#[derive(Serialize, Deserialize, Debug, PartialEq, Eq)]
struct PageToken {
    /// Folder.
    f: String,
    /// UIDVALIDITY when the listing started in this folder (0 = not started).
    v: u32,
    /// Exclusive upper UID bound.
    u: u32,
}

fn encode_token(t: &PageToken) -> String {
    URL_SAFE_NO_PAD.encode(serde_json::to_vec(t).unwrap_or_default())
}

fn decode_token(s: &str) -> Result<PageToken, ProviderError> {
    URL_SAFE_NO_PAD
        .decode(s)
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .ok_or_else(|| ProviderError::Invalid("bad page token".into()))
}

pub(crate) fn flags_fingerprint(flags: &[String]) -> u64 {
    let mut v: Vec<String> = flags.iter().map(|f| f.to_ascii_lowercase()).filter(|f| f != "\\recent").collect();
    v.sort();
    let mut h = DefaultHasher::new();
    v.hash(&mut h);
    h.finish()
}

fn now_ms() -> i64 {
    akatsuki_core::time::now_ms()
}

fn dedupe_threads(msgs: &[RemoteMessage], out: &mut Vec<RemoteThreadId>, seen: &mut BTreeSet<String>) {
    for m in msgs {
        if seen.insert(m.provider_thread_id.clone()) {
            out.push(RemoteThreadId(m.provider_thread_id.clone()));
        }
    }
}

impl ImapProvider {
    fn synced_folders(folders: Vec<Folder>) -> Vec<Folder> {
        folders.into_iter().filter(Folder::synced).collect()
    }

    pub(crate) async fn baseline_impl(&self) -> Result<Cursor, ProviderError> {
        let mut c = self.conn().await?;
        let folders = Self::synced_folders(self.refresh_folders(&mut c).await?);
        let condstore = self.imap_caps().condstore;
        let mut out = BTreeMap::new();
        for f in folders {
            let r = c.s().status(&f.name, condstore).await;
            let st = match c.check(r) {
                Ok(s) => s,
                Err(ProviderError::NotFound) => continue,
                Err(e) => return Err(e),
            };
            let (validity, next, modseq) = match (st.uid_validity, st.uid_next) {
                (Some(v), Some(n)) => (v, n, st.highest_modseq),
                _ => {
                    let sel = self.select(&mut c, &f.name, SelectMode::Plain).await?;
                    (sel.uid_validity, sel.uid_next, sel.highest_modseq)
                }
            };
            out.insert(
                f.name.clone(),
                ImapFolderCursor {
                    uid_validity: validity,
                    uid_next: next.max(1),
                    highest_modseq: if condstore { modseq } else { None },
                },
            );
        }
        Ok(Cursor::Imap { folders: out })
    }

    fn scan_due(&self, folder: &str) -> bool {
        let every = self.cfg.tuning.flag_scan_every;
        lock_state(&self.state).scans.get(folder).and_then(|s| s.last_scan).is_none_or(|t| t.elapsed() >= every)
    }

    /// The expunge diff (and flag scan without CONDSTORE) over the scan window (§9.5).
    /// Returns `(gone, flag_changed)` UIDs. The first scan of a folder only records.
    async fn scan_folder(
        &self,
        c: &mut Conn<'_>,
        folder: &Folder,
        sel: &Selected,
        flags_too: bool,
    ) -> Result<(Vec<u32>, Vec<u32>), ProviderError> {
        let window_start = now_ms() - i64::try_from(self.cfg.tuning.scan_window.as_millis()).unwrap_or(i64::MAX);
        let r = c.s().uid_search(Cmd::new("SINCE").raw(proto::search_date(window_start))).await;
        let uids = c.check(r)?;
        let r = c.s().uid_fetch(&uids, "(UID FLAGS INTERNALDATE)", None).await;
        let items = c.check(r)?;
        let current: HashMap<u32, (i64, u64)> = items
            .iter()
            .filter_map(|f| {
                Some((
                    f.uid?,
                    (f.internal_date.unwrap_or(i64::MAX), flags_fingerprint(f.flags.as_deref().unwrap_or_default())),
                ))
            })
            .collect();
        let mut st = lock_state(&self.state);
        let scan = st.scans.entry(folder.name.clone()).or_default();
        let first = scan.last_scan.is_none() || scan.uid_validity != sel.uid_validity;
        let mut gone = Vec::new();
        let mut changed = Vec::new();
        if !first {
            for (uid, (date, _)) in &scan.known {
                if *date >= window_start && !current.contains_key(uid) && *uid < sel.uid_next {
                    gone.push(*uid);
                }
            }
            if flags_too {
                for (uid, (_, fp)) in &current {
                    if scan.known.get(uid).is_some_and(|(_, k)| k != fp) {
                        changed.push(*uid);
                    }
                }
            }
        }
        scan.uid_validity = sel.uid_validity;
        scan.known = current;
        scan.last_scan = Some(Instant::now());
        gone.sort_unstable();
        changed.sort_unstable();
        Ok((gone, changed))
    }

    pub(crate) fn note_scan(&self, folder: &str, uid_validity: u32, items: &[FetchItem]) {
        let mut st = lock_state(&self.state);
        if let Some(scan) = st.scans.get_mut(folder).filter(|s| s.uid_validity == uid_validity) {
            for f in items {
                if let Some(uid) = f.uid {
                    let fp = flags_fingerprint(f.flags.as_deref().unwrap_or_default());
                    scan.known.insert(uid, (f.internal_date.unwrap_or(i64::MAX), fp));
                }
            }
        }
    }

    pub(crate) async fn changes_since_impl(&self, cursor: &Cursor) -> Result<ChangePage, ProviderError> {
        let Cursor::Imap { folders: prev } = cursor else {
            return Err(ProviderError::Invalid("not an IMAP cursor".into()));
        };
        let caps = self.imap_caps();
        let mut c = self.conn().await?;
        let synced = Self::synced_folders(self.refresh_folders(&mut c).await?);
        let names: BTreeSet<&str> = synced.iter().map(|f| f.name.as_str()).collect();
        let prev_names: BTreeSet<&str> = prev.keys().map(String::as_str).collect();
        let labels_changed = names != prev_names;

        let mut next = BTreeMap::new();
        let mut touched = Vec::new();
        let mut seen = BTreeSet::new();
        let mut deleted = Vec::new();
        let mut more = false;
        let mut budget = self.cfg.tuning.max_new_per_page.max(1);

        for f in &synced {
            let p = prev.get(&f.name);
            if budget == 0 {
                if let Some(p) = p {
                    next.insert(f.name.clone(), p.clone());
                }
                more = true;
                continue;
            }
            // STATUS on the selected mailbox is discouraged (RFC 3501 §6.3.10): re-select instead.
            let st = if c.s().selected.as_ref().is_some_and(|s| s.name == f.name) {
                super::session::MailboxStatus::default()
            } else {
                let r = c.s().status(&f.name, caps.condstore).await;
                match c.check(r) {
                    Ok(s) => s,
                    Err(ProviderError::NotFound) => continue,
                    Err(e) => return Err(e),
                }
            };
            if let (Some(p), Some(v)) = (p, st.uid_validity)
                && v != p.uid_validity
            {
                return Err(ProviderError::CursorExpired);
            }
            let scan_due = !caps.qresync && self.scan_due(&f.name);
            if let Some(p) = p {
                let same_next = st.uid_next == Some(p.uid_next);
                let same_mod = !caps.condstore || st.highest_modseq.is_none() || st.highest_modseq == p.highest_modseq;
                if same_next && same_mod && !scan_due {
                    next.insert(f.name.clone(), p.clone());
                    continue;
                }
            }

            let prev_mod = p.and_then(|p| p.highest_modseq);
            let mode = match (caps.qresync, p, prev_mod) {
                (true, Some(p), Some(m)) => SelectMode::Qresync { uid_validity: p.uid_validity, modseq: m },
                _ if caps.condstore => SelectMode::Condstore,
                _ => SelectMode::Plain,
            };
            let sel = self.select(&mut c, &f.name, mode).await?;
            if let Some(p) = p
                && sel.uid_validity != p.uid_validity
            {
                return Err(ProviderError::CursorExpired);
            }
            let from_uid = p.map_or(1, |p| p.uid_next.max(1));

            // New messages.
            let new_uids: Vec<u32> = if sel.uid_next > from_uid {
                let r = c.s().uid_search(Cmd::new("UID").raw(format!("{from_uid}:*"))).await;
                c.check(r)?.into_iter().filter(|u| *u >= from_uid).collect()
            } else {
                Vec::new()
            };
            let take = new_uids.len().min(budget);
            let (new_now, new_rest) = new_uids.split_at(take);

            // Changed and vanished messages.
            let mut changed: BTreeSet<u32> = BTreeSet::new();
            let mut vanished: Vec<u32> = Vec::new();
            match mode {
                SelectMode::Qresync { .. } => {
                    changed.extend(sel.changed.iter().filter_map(|f| f.uid).filter(|u| *u < from_uid));
                    vanished.extend(sel.vanished.iter().copied());
                }
                SelectMode::Condstore => {
                    if let Some(m) = prev_mod {
                        let r = c.s().uid_fetch_all("(UID FLAGS)", &format!("(CHANGEDSINCE {m})")).await;
                        let exec = c.check(r)?;
                        changed.extend(exec.fetches().iter().filter_map(|f| f.uid).filter(|u| *u < from_uid));
                    }
                }
                SelectMode::Plain => {}
            }
            if scan_due {
                let (gone, flag_changed) = self.scan_folder(&mut c, f, &sel, !caps.condstore).await?;
                vanished.extend(gone);
                changed.extend(flag_changed.into_iter().filter(|u| *u < from_uid));
            }
            vanished.sort_unstable();
            vanished.dedup();
            for u in &vanished {
                changed.remove(u);
            }

            let mut uids: Vec<u32> = changed.into_iter().collect();
            uids.extend_from_slice(new_now);
            let msgs = self.fetch_messages(&mut c, f, &sel, &uids).await?;
            dedupe_threads(&msgs, &mut touched, &mut seen);
            let gone_ids: Vec<String> = vanished.iter().map(|u| message_id(&f.name, sel.uid_validity, *u)).collect();
            lock_state(&self.state).cache.remove_messages(&gone_ids);
            deleted.extend(gone_ids);

            budget -= take;
            let uid_next = match new_rest.first() {
                Some(u) => {
                    more = true;
                    *u
                }
                None => sel.uid_next.max(from_uid),
            };
            next.insert(
                f.name.clone(),
                ImapFolderCursor {
                    uid_validity: sel.uid_validity,
                    uid_next,
                    highest_modseq: if caps.condstore { sel.highest_modseq } else { None },
                },
            );
        }
        Ok(ChangePage {
            touched_threads: touched,
            deleted_threads: Vec::new(),
            deleted_messages: deleted,
            labels_changed,
            next: Cursor::Imap { folders: next },
            more,
        })
    }

    pub(crate) async fn list_window_impl(&self, req: &WindowRequest) -> Result<WindowPage, ProviderError> {
        let mut c = self.conn().await?;
        let folders = Self::synced_folders(self.refresh_folders(&mut c).await?);
        match req {
            WindowRequest::Inbox { page } => {
                let inbox = folders
                    .iter()
                    .find(|f| f.role == Some(LabelRole::Inbox))
                    .cloned()
                    .ok_or(ProviderError::Conflict("no INBOX".into()))?;
                let token = page.as_deref().map(decode_token).transpose()?;
                let size = if token.is_none() { FIRST_PAGE } else { PAGE };
                self.page_in_folder(&mut c, &inbox, token, None, size, None).await
            }
            WindowRequest::Since { after, page } => {
                let token = page.as_deref().map(decode_token).transpose()?;
                let mut start = match &token {
                    None => 0,
                    Some(t) => match folders.iter().position(|f| f.name == t.f) {
                        Some(i) => i,
                        // The folder disappeared: continue with the next one by name order.
                        None => folders.iter().position(|f| f.name > t.f).unwrap_or(folders.len()),
                    },
                };
                let mut token = token.filter(|t| folders.get(start).is_some_and(|f| f.name == t.f));
                while start < folders.len() {
                    let next_folder = folders.get(start + 1).map(|f| f.name.clone());
                    let page = self
                        .page_in_folder(&mut c, &folders[start], token.take(), Some(*after), PAGE, next_folder)
                        .await?;
                    if !page.threads.is_empty() || page.next_page.is_none() {
                        return Ok(page);
                    }
                    // An empty folder: move on within this call.
                    start += 1;
                }
                Ok(WindowPage { threads: Vec::new(), next_page: None, oldest: None })
            }
            WindowRequest::Before { before, horizon, limit } => {
                self.backfill_page(&mut c, &folders, *before, *horizon, (*limit).max(1) as usize).await
            }
        }
    }

    /// One page of a folder, newest UID first, optionally `SINCE after`.
    async fn page_in_folder(
        &self,
        c: &mut Conn<'_>,
        folder: &Folder,
        token: Option<PageToken>,
        after: Option<i64>,
        size: usize,
        next_folder: Option<String>,
    ) -> Result<WindowPage, ProviderError> {
        let sel = self.select(c, &folder.name, SelectMode::Plain).await?;
        if let Some(t) = &token
            && t.v != 0
            && t.v != sel.uid_validity
        {
            return Err(ProviderError::CursorExpired);
        }
        let upper = token.as_ref().map_or(u32::MAX, |t| t.u);
        let mut crit = match after {
            Some(a) => Cmd::new("SINCE").raw(proto::search_date(a)),
            None => Cmd::new("ALL"),
        };
        if upper != u32::MAX {
            if upper <= 1 {
                return Ok(WindowPage { threads: Vec::new(), next_page: next_token(next_folder), oldest: None });
            }
            crit = crit.raw(format!("UID 1:{}", upper - 1));
        }
        let r = c.s().uid_search(crit).await;
        let mut uids: Vec<u32> = c.check(r)?.into_iter().filter(|u| *u < upper).collect();
        uids.sort_unstable_by(|a, b| b.cmp(a));
        let page: Vec<u32> = uids.iter().take(size).copied().collect();
        let rest = uids.len() > page.len();
        let msgs = self.fetch_messages(c, folder, &sel, &page).await?;
        let mut threads = Vec::new();
        let mut seen = BTreeSet::new();
        let mut ordered = msgs.clone();
        ordered.sort_by_key(|m| std::cmp::Reverse(m.date));
        dedupe_threads(&ordered, &mut threads, &mut seen);
        let oldest = msgs.iter().map(|m| m.date).min();
        let next_page = match page.last() {
            Some(min) if rest => {
                Some(encode_token(&PageToken { f: folder.name.clone(), v: sel.uid_validity, u: *min }))
            }
            _ => next_token(next_folder),
        };
        Ok(WindowPage { threads, next_page, oldest })
    }

    /// Backfill by date (§9.3 step 4): messages with `horizon ≤ INTERNALDATE < before`, newest
    /// first across folders, at most `limit`. The search window widens from a week to the horizon
    /// so a page does not read the dates of a whole year.
    async fn backfill_page(
        &self,
        c: &mut Conn<'_>,
        folders: &[Folder],
        before: i64,
        horizon: i64,
        limit: usize,
    ) -> Result<WindowPage, ProviderError> {
        const DAY: i64 = 86_400_000;
        let mut span = 7 * DAY;
        loop {
            let lower = (before - span).max(horizon);
            let mut cands: Vec<(i64, usize, u32)> = Vec::new();
            for (i, f) in folders.iter().enumerate() {
                self.select(c, &f.name, SelectMode::Plain).await?;
                // SEARCH dates are whole days; widen by a day each side, then filter exactly.
                let crit = Cmd::new("SINCE")
                    .raw(proto::search_date(lower - DAY))
                    .raw("BEFORE")
                    .raw(proto::search_date(before + DAY));
                let r = c.s().uid_search(crit).await;
                let uids = c.check(r)?;
                let r = c.s().uid_fetch(&uids, "(UID INTERNALDATE)", None).await;
                for item in c.check(r)? {
                    if let (Some(uid), Some(d)) = (item.uid, item.internal_date)
                        && d >= lower
                        && d < before
                    {
                        cands.push((d, i, uid));
                    }
                }
            }
            if cands.len() >= limit || lower <= horizon {
                cands.sort_by(|a, b| b.cmp(a));
                cands.truncate(limit);
                let mut by_folder: BTreeMap<usize, Vec<u32>> = BTreeMap::new();
                for (_, i, uid) in &cands {
                    by_folder.entry(*i).or_default().push(*uid);
                }
                let mut msgs = Vec::new();
                for (i, uids) in by_folder {
                    let sel = self.select(c, &folders[i].name, SelectMode::Plain).await?;
                    msgs.extend(self.fetch_messages(c, &folders[i], &sel, &uids).await?);
                }
                msgs.sort_by_key(|m| std::cmp::Reverse(m.date));
                let mut threads = Vec::new();
                dedupe_threads(&msgs, &mut threads, &mut BTreeSet::new());
                let oldest = cands.iter().map(|(d, ..)| *d).min();
                return Ok(WindowPage { threads, next_page: None, oldest });
            }
            span = span.saturating_mul(4);
        }
    }

    pub(crate) async fn fetch_threads_impl(
        &self,
        ids: &[RemoteThreadId],
        _depth: FetchDepth,
    ) -> Result<Vec<FetchedThread>, ProviderError> {
        let mut out = Vec::new();
        let mut missing = Vec::new();
        {
            let st = lock_state(&self.state);
            for id in ids {
                match st.cache.get(&id.0) {
                    // Only the messages this process has seen (the index starts empty after a restart
                    // and evicts old threads): never the whole thread for sure (D-223).
                    Some(messages) => out.push(FetchedThread {
                        provider_thread_id: id.0.clone(),
                        provider_version: None,
                        messages,
                        partial: true,
                    }),
                    None => missing.push(id.0.clone()),
                }
            }
        }
        if !missing.is_empty() && self.imap_caps().x_gm_ext {
            let mut c = self.conn().await?;
            let folders = Self::synced_folders(self.folders());
            for id in missing {
                let Ok(thrid) = u64::from_str_radix(&id, 16) else { continue };
                let mut messages = Vec::new();
                for f in &folders {
                    let sel = self.select(&mut c, &f.name, SelectMode::Plain).await?;
                    let r = c.s().uid_search(Cmd::new("X-GM-THRID").raw(thrid.to_string())).await;
                    let uids = c.check(r)?;
                    messages.extend(self.fetch_messages(&mut c, f, &sel, &uids).await?);
                }
                if !messages.is_empty() {
                    messages.sort_by_key(|m| m.date);
                    // Synced folders only (All Mail and others may hold more of the thread).
                    out.push(FetchedThread { provider_thread_id: id, provider_version: None, messages, partial: true });
                }
            }
        }
        Ok(out)
    }

    pub(crate) async fn search_impl(
        &self,
        q: &akatsuki_search::query::Ast,
        page: Option<String>,
    ) -> Result<(Vec<RemoteThreadId>, Option<String>), ProviderError> {
        let offset: usize = match page.as_deref() {
            None => 0,
            Some(p) => p.parse().map_err(|_| ProviderError::Invalid("bad page token".into()))?,
        };
        let mut c = self.conn().await?;
        let folders = Self::synced_folders(self.refresh_folders(&mut c).await?);
        let plan = search::plan(q, &folders, now_ms())?;
        let mut hits: Vec<(usize, u32)> = Vec::new();
        let targets: Vec<usize> = folders
            .iter()
            .enumerate()
            .filter(|(_, f)| {
                plan.folders.as_ref().map_or_else(|| search::default_folder(f), |names| names.contains(&f.name))
            })
            .map(|(i, _)| i)
            .collect();
        for i in &targets {
            self.select(&mut c, &folders[*i].name, SelectMode::Plain).await?;
            let r = c.s().uid_search(plan.criteria.clone()).await;
            let mut uids = c.check(r)?;
            uids.sort_unstable_by(|a, b| b.cmp(a));
            hits.extend(uids.into_iter().map(|u| (*i, u)));
        }
        let page_hits: Vec<(usize, u32)> = hits.iter().skip(offset).take(SEARCH_PAGE).copied().collect();
        let mut by_folder: BTreeMap<usize, Vec<u32>> = BTreeMap::new();
        for (i, u) in &page_hits {
            by_folder.entry(*i).or_default().push(*u);
        }
        let mut msgs = Vec::new();
        for (i, uids) in by_folder {
            let sel = self.select(&mut c, &folders[i].name, SelectMode::Plain).await?;
            msgs.extend(self.fetch_messages(&mut c, &folders[i], &sel, &uids).await?);
        }
        msgs.sort_by_key(|m| std::cmp::Reverse(m.date));
        let mut threads = Vec::new();
        dedupe_threads(&msgs, &mut threads, &mut BTreeSet::new());
        let next = (hits.len() > offset + page_hits.len()).then(|| (offset + page_hits.len()).to_string());
        Ok((threads, next))
    }

    /// Finds or creates the folder for a role (`Akatsuki/Snoozed`, `Archive`, `Trash`, …).
    pub(crate) async fn ensure_role_folder(&self, role: LabelRole, name: &str) -> Result<Folder, ProviderError> {
        let mut c = self.conn().await?;
        self.ensure_role_folder_with(&mut c, role, name).await
    }

    pub(crate) async fn ensure_role_folder_with(
        &self,
        c: &mut Conn<'_>,
        role: LabelRole,
        name: &str,
    ) -> Result<Folder, ProviderError> {
        if let Some(f) = self.folder_by_role(role) {
            return Ok(f);
        }
        let folders = self.refresh_folders(c).await?;
        if let Some(f) = folders.iter().find(|f| f.role == Some(role)) {
            return Ok(f.clone());
        }
        let mbox = mailbox_for_path(&folders, name);
        let r = c.s().mailbox_cmd("CREATE", &mbox).await;
        match c.check(r) {
            Ok(()) | Err(ProviderError::Conflict(_)) => {}
            Err(e) => return Err(e),
        }
        let r = c.s().mailbox_cmd("SUBSCRIBE", &mbox).await;
        let _ = c.check(r);
        let folders = self.refresh_folders(c).await?;
        let mut f = folders
            .into_iter()
            .find(|f| f.name == mbox)
            .ok_or_else(|| ProviderError::Conflict("created folder is not listed".into()))?;
        if f.role.is_none() {
            f.role = Some(role);
            let mut st = lock_state(&self.state);
            if let Some(x) = st.folders.iter_mut().find(|x| x.name == mbox) {
                x.role = Some(role);
            }
        }
        Ok(f)
    }

    /// One decoded MIME part: `BODY.PEEK[<part>.MIME]` + `BODY.PEEK[<part>]`, falling back to the
    /// whole message when the server gives no part header.
    pub(crate) async fn part_bytes(
        &self,
        c: &mut Conn<'_>,
        msg: &RemoteMessageRef,
        part_id: &str,
    ) -> Result<Vec<u8>, ProviderError> {
        for loc in Self::locations_of(msg) {
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
            let items = format!("(UID BODY.PEEK[{part_id}.MIME] BODY.PEEK[{part_id}])");
            let r = c.s().uid_fetch(&[loc.uid], &items, None).await;
            let Some(item) = c.check(r)?.into_iter().next() else { continue };
            let mime = item.section(&format!("{part_id}.MIME")).unwrap_or_default();
            let body = item.section(part_id);
            if let Some(body) = body
                && mime.windows(8).any(|w| w.eq_ignore_ascii_case(b"content-"))
            {
                let mut synthetic = mime.to_vec();
                if !synthetic.ends_with(b"\r\n\r\n") && !synthetic.ends_with(b"\n\n") {
                    synthetic.extend_from_slice(b"\r\n");
                }
                synthetic.extend_from_slice(body);
                if let Ok(Some(p)) = akatsuki_mime::part_content(&synthetic, "1") {
                    return Ok(p.bytes);
                }
            }
            // Fallback: the whole message, then the part by path.
            let raw = self
                .raw_message(
                    c,
                    &RemoteMessageRef {
                        provider_message_id: message_id(&loc.remote_folder_id, loc.uid_validity, loc.uid),
                        imap: vec![loc.clone()],
                    },
                )
                .await?;
            return match akatsuki_mime::part_content(&raw, part_id) {
                Ok(Some(p)) => Ok(p.bytes),
                _ => Err(ProviderError::NotFound),
            };
        }
        Err(ProviderError::NotFound)
    }
}

fn next_token(next_folder: Option<String>) -> Option<String> {
    next_folder.map(|f| encode_token(&PageToken { f, v: 0, u: u32::MAX }))
}

/// A location's §9.5 message id.
pub(crate) fn loc_id(l: &crate::ImapLocation) -> String {
    message_id(&l.remote_folder_id, l.uid_validity, l.uid)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_round_trip() {
        let t = PageToken { f: "INBOX".into(), v: 3, u: 99 };
        assert_eq!(decode_token(&encode_token(&t)).unwrap(), t);
        assert!(decode_token("!!").is_err());
        assert_eq!(flags_fingerprint(&["\\Seen".into(), "\\Recent".into()]), flags_fingerprint(&["\\seen".into()]));
        assert_ne!(flags_fingerprint(&["\\Seen".into()]), flags_fingerprint(&[]));
    }
}
