//! Ops, drafts and sending (§5.1 helpers for folder providers, §5.6, §10.1, §10.2).

use std::collections::BTreeMap;

use super::folders::{Folder, KEYWORD_PREFIX, mailbox_for_path};
use super::message::parse_message_id;
use super::proto::Cmd;
use super::smtp;
use super::sync::loc_id;
use super::{Conn, ImapProvider, lock_state};
use crate::labels::SNOOZED_LABEL_NAME;
use crate::{
    FetchedThread, ImapLocation, LabelRole, LabelTarget, OpEffect, OpId, OutgoingMime, ProviderError, ProviderOp,
    RemoteDraftRef, RemoteMessageRef, RemoteThreadRef, SentRef, SysLabel, imap_thread,
};

/// What a label target means on IMAP.
#[derive(Clone, Debug)]
enum Target {
    Folder(Folder),
    /// A flag or keyword; `on_add` is the flag state when the label is added (`sys:unread` is the
    /// absence of `\Seen`).
    Flag {
        flag: String,
        on_add: bool,
    },
}

fn group_by_folder(locs: &[ImapLocation]) -> BTreeMap<String, Vec<ImapLocation>> {
    let mut m: BTreeMap<String, Vec<ImapLocation>> = BTreeMap::new();
    for l in locs {
        m.entry(l.remote_folder_id.clone()).or_default().push(l.clone());
    }
    m
}

fn thread_locations(t: &RemoteThreadRef) -> Vec<ImapLocation> {
    let mut v: Vec<ImapLocation> = t.messages.iter().flat_map(ImapProvider::locations_of).collect();
    v.sort_by(|a, b| (&a.remote_folder_id, a.uid).cmp(&(&b.remote_folder_id, b.uid)));
    v.dedup();
    v
}

impl ImapProvider {
    async fn revive(&self, c: &mut Conn<'_>) -> Result<(), ProviderError> {
        if !c.alive() {
            let s =
                super::session::ImapSession::connect(&self.cfg).await.map_err(super::ConnectError::into_provider)?;
            *c.guard = Some(s);
        }
        Ok(())
    }

    async fn role_folder(&self, c: &mut Conn<'_>, role: LabelRole) -> Result<Folder, ProviderError> {
        let default_name = match role {
            LabelRole::Inbox => "INBOX",
            LabelRole::Sent => "Sent",
            LabelRole::Drafts => "Drafts",
            LabelRole::Trash => "Trash",
            LabelRole::Spam => "Junk",
            LabelRole::Snoozed => SNOOZED_LABEL_NAME,
            LabelRole::Archive => "Archive",
            _ => return Err(ProviderError::Unsupported("no folder for this label")),
        };
        self.ensure_role_folder_with(c, role, default_name).await
    }

    async fn resolve(&self, c: &mut Conn<'_>, t: &LabelTarget) -> Result<Target, ProviderError> {
        Ok(match t {
            LabelTarget::Sys(SysLabel::Unread) => Target::Flag { flag: "\\Seen".into(), on_add: false },
            LabelTarget::Sys(SysLabel::Starred) => Target::Flag { flag: "\\Flagged".into(), on_add: true },
            LabelTarget::Sys(SysLabel::Important) => Target::Flag { flag: "$Important".into(), on_add: true },
            LabelTarget::Sys(s) => {
                let role = match s {
                    SysLabel::Inbox => LabelRole::Inbox,
                    SysLabel::Sent => LabelRole::Sent,
                    SysLabel::Drafts => LabelRole::Drafts,
                    SysLabel::Trash => LabelRole::Trash,
                    SysLabel::Spam => LabelRole::Spam,
                    _ => LabelRole::Snoozed,
                };
                Target::Folder(self.role_folder(c, role).await?)
            }
            LabelTarget::Label { provider_label_id, .. } => match provider_label_id.strip_prefix(KEYWORD_PREFIX) {
                Some(kw) => Target::Flag { flag: kw.to_owned(), on_add: true },
                None => match self.folder(provider_label_id) {
                    Some(f) => Target::Folder(f),
                    None => {
                        let folders = self.refresh_folders(c).await?;
                        Target::Folder(
                            folders
                                .into_iter()
                                .find(|f| &f.name == provider_label_id)
                                .ok_or_else(|| ProviderError::Conflict("folder does not exist".into()))?,
                        )
                    }
                },
            },
        })
    }

    /// Stores flags on every valid location. Returns the number of locations that still exist.
    async fn store_flags(
        &self,
        c: &mut Conn<'_>,
        locs: &[ImapLocation],
        add: &[String],
        remove: &[String],
    ) -> Result<usize, ProviderError> {
        let mut valid = 0;
        for (folder, ls) in group_by_folder(locs) {
            if self.folder(&folder).is_none() {
                continue;
            }
            let sel = match self.ensure_selected(c, &folder).await {
                Ok(s) => s,
                Err(ProviderError::NotFound) => continue,
                Err(e) => return Err(e),
            };
            let uids: Vec<u32> = ls.iter().filter(|l| l.uid_validity == sel.uid_validity).map(|l| l.uid).collect();
            let r = c.s().existing(&uids).await;
            let uids = c.check(r)?;
            valid += uids.len();
            let r = c.s().uid_store(&uids, "+FLAGS.SILENT", add).await;
            c.check(r)?;
            let r = c.s().uid_store(&uids, "-FLAGS.SILENT", remove).await;
            c.check(r)?;
        }
        Ok(valid)
    }

    /// Moves every location to `dest`. Returns the new locations (unchanged ones included).
    async fn move_to(
        &self,
        c: &mut Conn<'_>,
        locs: &[ImapLocation],
        dest: &Folder,
    ) -> Result<Vec<ImapLocation>, ProviderError> {
        let caps = self.imap_caps();
        let mut out = Vec::new();
        let mut gone = Vec::new();
        for (folder, ls) in group_by_folder(locs) {
            if folder == dest.name {
                out.extend(ls);
                continue;
            }
            if self.folder(&folder).is_none() {
                continue;
            }
            let sel = match self.ensure_selected(c, &folder).await {
                Ok(s) => s,
                Err(ProviderError::NotFound) => continue,
                Err(e) => return Err(e),
            };
            let uids: Vec<u32> = ls.iter().filter(|l| l.uid_validity == sel.uid_validity).map(|l| l.uid).collect();
            let r = c.s().existing(&uids).await;
            let uids = c.check(r)?;
            if uids.is_empty() {
                continue;
            }
            // Without UIDPLUS there is no COPYUID: remember Message-IDs to find the copies.
            let mut mids: Vec<String> = Vec::new();
            if !caps.uidplus {
                let r = c.s().uid_fetch(&uids, "(UID BODY.PEEK[HEADER.FIELDS (MESSAGE-ID)])", None).await;
                for f in c.check(r)? {
                    let h = f.section("HEADER").unwrap_or_default();
                    if let Some(id) = akatsuki_mime::parse_message(h).ok().and_then(|p| p.rfc_message_id) {
                        mids.push(id);
                    }
                }
            }
            let mapping = if caps.uid_move {
                let r = c.s().uid_move(&uids, &dest.name).await;
                c.check(r)?
            } else {
                let r = c.s().uid_copy(&uids, &dest.name).await;
                let m = c.check(r)?;
                let r = c.s().delete_uids(&uids).await;
                c.check(r)?;
                m
            };
            c.s().selected = None;
            gone.extend(uids.iter().map(|u| {
                loc_id(&ImapLocation { remote_folder_id: folder.clone(), uid_validity: sel.uid_validity, uid: *u })
            }));
            match mapping {
                Some((validity, pairs)) => out.extend(pairs.into_iter().map(|(_, dst)| ImapLocation {
                    remote_folder_id: dest.name.clone(),
                    uid_validity: validity,
                    uid: dst,
                })),
                None => {
                    let dsel = self.ensure_selected(c, &dest.name).await?;
                    for mid in mids {
                        let crit = Cmd::new("HEADER").raw("Message-ID").string(&mid)?;
                        let r = c.s().uid_search(crit).await;
                        if let Some(uid) = c.check(r)?.into_iter().max() {
                            out.push(ImapLocation {
                                remote_folder_id: dest.name.clone(),
                                uid_validity: dsel.uid_validity,
                                uid,
                            });
                        }
                    }
                }
            }
        }
        lock_state(&self.state).cache.remove_messages(&gone);
        Ok(out)
    }

    async fn post_state(
        &self,
        c: &mut Conn<'_>,
        thread: &RemoteThreadRef,
        locs: &[ImapLocation],
    ) -> Option<FetchedThread> {
        let mut messages = self.fetch_locations(c, locs).await.ok()?;
        for m in &mut messages {
            m.provider_thread_id = thread.provider_thread_id.clone();
        }
        messages.sort_by_key(|m| m.date);
        // Every message of the op's thread ref at its new location: the whole thread as Convex knows it
        // (D-223), so it may replace the stored message list.
        Some(FetchedThread {
            provider_thread_id: thread.provider_thread_id.clone(),
            provider_version: None,
            messages,
            partial: false,
        })
    }

    async fn apply_one(&self, c: &mut Conn<'_>, op: &ProviderOp) -> Result<OpEffect, ProviderError> {
        match op {
            ProviderOp::Modify { thread, add, remove } => {
                let mut flags_add = Vec::new();
                let mut flags_remove = Vec::new();
                let mut folder_add: Vec<Folder> = Vec::new();
                let mut folder_remove = 0usize;
                for (t, adding) in add.iter().map(|t| (t, true)).chain(remove.iter().map(|t| (t, false))) {
                    match self.resolve(c, t).await? {
                        Target::Flag { flag, on_add } => {
                            if adding == on_add {
                                flags_add.push(flag);
                            } else {
                                flags_remove.push(flag);
                            }
                        }
                        Target::Folder(f) if adding => folder_add.push(f),
                        Target::Folder(_) => folder_remove += 1,
                    }
                }
                folder_add.dedup_by(|a, b| a.name == b.name);
                if folder_add.len() > 1 {
                    return Err(ProviderError::Invalid("a message is in exactly one IMAP folder".into()));
                }
                if folder_add.is_empty() && folder_remove > 0 {
                    return Err(ProviderError::Invalid(
                        "removing an IMAP folder needs a destination (use move)".into(),
                    ));
                }
                let locs = thread_locations(thread);
                if locs.is_empty() {
                    return Err(ProviderError::NotFound);
                }
                let valid = self.store_flags(c, &locs, &flags_add, &flags_remove).await?;
                if valid == 0 {
                    return Err(ProviderError::NotFound);
                }
                let locs = match folder_add.first() {
                    Some(dest) => self.move_to(c, &locs, dest).await?,
                    None => locs,
                };
                Ok(OpEffect::Applied { post: self.post_state(c, thread, &locs).await, created: None })
            }
            ProviderOp::Trash { thread } => {
                let trash = self.role_folder(c, LabelRole::Trash).await?;
                self.move_thread(c, thread, &trash, |_| true).await
            }
            ProviderOp::Untrash { thread } => {
                let inbox = self.role_folder(c, LabelRole::Inbox).await?;
                let trash = self.folder_by_role(LabelRole::Trash).map(|f| f.name);
                self.move_thread(c, thread, &inbox, |l| trash.as_deref() == Some(l.remote_folder_id.as_str())).await
            }
            ProviderOp::Move { thread, to } => match self.resolve(c, to).await? {
                Target::Folder(dest) => self.move_thread(c, thread, &dest, |_| true).await,
                Target::Flag { .. } => Err(ProviderError::Invalid("move needs a folder".into())),
            },
            ProviderOp::DeleteForever { thread } => {
                let locs = thread_locations(thread);
                let mut valid = 0;
                let mut gone = Vec::new();
                for (folder, ls) in group_by_folder(&locs) {
                    if self.folder(&folder).is_none() {
                        continue;
                    }
                    let sel = match self.ensure_selected(c, &folder).await {
                        Ok(s) => s,
                        Err(ProviderError::NotFound) => continue,
                        Err(e) => return Err(e),
                    };
                    let uids: Vec<u32> =
                        ls.iter().filter(|l| l.uid_validity == sel.uid_validity).map(|l| l.uid).collect();
                    let r = c.s().existing(&uids).await;
                    let uids = c.check(r)?;
                    let r = c.s().existing(&uids).await;
                    let uids = c.check(r)?;
                    valid += uids.len();
                    let r = c.s().delete_uids(&uids).await;
                    c.check(r)?;
                    gone.extend(uids.iter().map(|u| {
                        loc_id(&ImapLocation {
                            remote_folder_id: folder.clone(),
                            uid_validity: sel.uid_validity,
                            uid: *u,
                        })
                    }));
                }
                lock_state(&self.state).cache.remove_messages(&gone);
                if valid == 0 {
                    Err(ProviderError::NotFound)
                } else {
                    Ok(OpEffect::Applied { post: None, created: None })
                }
            }
            ProviderOp::LabelCreate { name, .. } => {
                let folders = self.refresh_folders(c).await?;
                let mbox = mailbox_for_path(&folders, name);
                let r = c.s().mailbox_cmd("CREATE", &mbox).await;
                match c.check(r) {
                    // A replayed op finds the folder already there.
                    Ok(()) | Err(ProviderError::Conflict(_)) => {}
                    Err(e) => return Err(e),
                }
                let r = c.s().mailbox_cmd("SUBSCRIBE", &mbox).await;
                let _ = c.check(r);
                let folders = self.refresh_folders(c).await?;
                let f = folders
                    .into_iter()
                    .find(|f| f.name == mbox)
                    .ok_or(ProviderError::Conflict("folder not listed after CREATE".into()))?;
                Ok(OpEffect::Applied { post: None, created: Some(f.label()) })
            }
            ProviderOp::LabelRename { provider_label_id, name } => {
                if provider_label_id.starts_with(KEYWORD_PREFIX) {
                    return Err(ProviderError::Unsupported("renaming an IMAP keyword"));
                }
                let folders = self.refresh_folders(c).await?;
                let to = mailbox_for_path(&folders, name);
                c.s().selected = None;
                let r = c.s().rename(provider_label_id, &to).await;
                c.check(r)?;
                let folders = self.refresh_folders(c).await?;
                let f = folders
                    .into_iter()
                    .find(|f| f.name == to)
                    .ok_or(ProviderError::Conflict("folder not listed after RENAME".into()))?;
                // The provider label id of an IMAP folder is its name, so a rename hands back the
                // new label for the engine to store.
                Ok(OpEffect::Applied { post: None, created: Some(f.label()) })
            }
            ProviderOp::LabelDelete { provider_label_id } => {
                if provider_label_id.starts_with(KEYWORD_PREFIX) {
                    return Ok(OpEffect::Moot);
                }
                c.s().selected = None;
                let r = c.s().mailbox_cmd("DELETE", provider_label_id).await;
                match c.check(r) {
                    Ok(()) => {
                        self.refresh_folders(c).await?;
                        Ok(OpEffect::Applied { post: None, created: None })
                    }
                    Err(ProviderError::NotFound) => Ok(OpEffect::Moot),
                    Err(e) => Err(e),
                }
            }
            ProviderOp::DeleteDraft { draft } => match self.delete_draft_with(c, draft).await {
                Ok(()) => Ok(OpEffect::Applied { post: None, created: None }),
                Err(ProviderError::NotFound) => Ok(OpEffect::Moot),
                Err(e) => Err(e),
            },
        }
    }

    async fn move_thread(
        &self,
        c: &mut Conn<'_>,
        thread: &RemoteThreadRef,
        dest: &Folder,
        pick: impl Fn(&ImapLocation) -> bool,
    ) -> Result<OpEffect, ProviderError> {
        let all = thread_locations(thread);
        if all.is_empty() {
            return Err(ProviderError::NotFound);
        }
        let (moving, staying): (Vec<_>, Vec<_>) = all.into_iter().partition(|l| pick(l));
        if moving.is_empty() {
            return Ok(OpEffect::Moot);
        }
        let mut locs = self.move_to(c, &moving, dest).await?;
        if locs.is_empty() {
            return Err(ProviderError::NotFound);
        }
        locs.extend(staying);
        Ok(OpEffect::Applied { post: self.post_state(c, thread, &locs).await, created: None })
    }

    pub(crate) async fn apply_impl(&self, ops: &[(OpId, ProviderOp)]) -> Vec<(OpId, Result<OpEffect, ProviderError>)> {
        let mut out = Vec::with_capacity(ops.len());
        let mut c = match self.conn().await {
            Ok(c) => c,
            Err(e) => return ops.iter().map(|(id, _)| (id.clone(), Err(e.clone()))).collect(),
        };
        for (id, op) in ops {
            let r = match self.revive(&mut c).await {
                Ok(()) => self.apply_one(&mut c, op).await,
                Err(e) => Err(e),
            };
            out.push((id.clone(), r));
        }
        out
    }

    /// The newest UID in the selected folder whose `Message-ID` contains `rfc_id`.
    async fn find_by_message_id(&self, c: &mut Conn<'_>, rfc_id: &str) -> Result<Option<u32>, ProviderError> {
        let crit = Cmd::new("HEADER").raw("Message-ID").string(rfc_id)?;
        let r = c.s().uid_search(crit).await;
        Ok(c.check(r)?.into_iter().max())
    }

    pub(crate) async fn save_draft_impl(
        &self,
        mime: &OutgoingMime,
        existing: Option<&RemoteDraftRef>,
    ) -> Result<RemoteDraftRef, ProviderError> {
        let mut c = self.conn().await?;
        let drafts = self.role_folder(&mut c, LabelRole::Drafts).await?;
        let r = c.s().append(&drafts.name, &["\\Draft", "\\Seen"], mime.bytes.to_vec()).await;
        let appended = c.check(r)?;
        c.s().selected = None;
        let (validity, uid) = match appended {
            Some(x) => x,
            None => {
                let sel = self.ensure_selected(&mut c, &drafts.name).await?;
                let uid = self
                    .find_by_message_id(&mut c, &mime.rfc_message_id)
                    .await?
                    .ok_or_else(|| ProviderError::Transient("draft appended but not found".into(), None))?;
                (sel.uid_validity, uid)
            }
        };
        let id = loc_id(&ImapLocation { remote_folder_id: drafts.name.clone(), uid_validity: validity, uid });
        if let Some(old) = existing
            && old.provider_draft_id != id
        {
            // §10.1: APPEND the new version, then expunge the previous UID.
            match self.delete_draft_with(&mut c, old).await {
                Ok(()) | Err(ProviderError::NotFound) => {}
                Err(e) => tracing::warn!(error = %e, "could not remove the previous draft version"),
            }
        }
        Ok(RemoteDraftRef { provider_draft_id: id.clone(), provider_message_id: Some(id) })
    }

    async fn delete_draft_with(&self, c: &mut Conn<'_>, d: &RemoteDraftRef) -> Result<(), ProviderError> {
        let loc = parse_message_id(&d.provider_draft_id)
            .ok_or_else(|| ProviderError::Invalid("not an IMAP draft id".into()))?;
        if self.folder(&loc.remote_folder_id).is_none() {
            return Err(ProviderError::NotFound);
        }
        let sel = self.ensure_selected(c, &loc.remote_folder_id).await?;
        if sel.uid_validity != loc.uid_validity {
            return Err(ProviderError::NotFound);
        }
        let r = c.s().uid_search(Cmd::new("UID").raw(loc.uid.to_string())).await;
        if !c.check(r)?.contains(&loc.uid) {
            return Err(ProviderError::NotFound);
        }
        let r = c.s().delete_uids(&[loc.uid]).await;
        c.check(r)?;
        lock_state(&self.state).cache.remove_messages(std::slice::from_ref(&d.provider_draft_id));
        Ok(())
    }

    pub(crate) async fn delete_draft_impl(&self, d: &RemoteDraftRef) -> Result<(), ProviderError> {
        let mut c = self.conn().await?;
        self.delete_draft_with(&mut c, d).await
    }

    pub(crate) async fn send_draft_impl(
        &self,
        d: &RemoteDraftRef,
        rfc_message_id: &str,
    ) -> Result<SentRef, ProviderError> {
        let raw = {
            let mut c = self.conn().await?;
            self.raw_message(
                &mut c,
                &RemoteMessageRef { provider_message_id: d.provider_draft_id.clone(), imap: Vec::new() },
            )
            .await?
        };
        let parsed = akatsuki_mime::parse_message(&raw).map_err(|e| ProviderError::Invalid(format!("draft: {e}")))?;
        let want = imap_thread::clean_message_id(rfc_message_id);
        if parsed.rfc_message_id.as_deref().map(imap_thread::clean_message_id) != Some(want.clone()) {
            return Err(ProviderError::Conflict("the draft's Message-ID does not match".into()));
        }
        let mut rcpts: Vec<String> = Vec::new();
        for a in parsed.to.iter().chain(&parsed.cc).chain(&parsed.bcc) {
            if !rcpts.contains(&a.email) {
                rcpts.push(a.email.clone());
            }
        }
        let data = smtp::strip_bcc(&raw);
        smtp::send(&self.cfg, &self.cfg.email, &rcpts, &data).await.map_err(smtp::SmtpError::into_provider)?;

        // The message is out. Nothing below may fail the send (at-most-once, §5.6).
        let thread_id = (!self.imap_caps().x_gm_ext).then(|| {
            let from = parsed.from.as_ref().map(|a| a.email.clone()).unwrap_or_default();
            let root = imap_thread::thread_root(
                &parsed.references,
                parsed.in_reply_to.as_deref(),
                Some(&want),
                &from,
                parsed.date.unwrap_or(0),
                &parsed.subject,
            );
            imap_thread::thread_id(&self.cfg.account_id, &root)
        });
        let mut sent_id: Option<String> = None;
        if let Ok(mut c) = self.conn().await {
            if !self.caps.sent_auto_saved {
                match self.role_folder(&mut c, LabelRole::Sent).await {
                    Ok(sent) => {
                        let r = c.s().append(&sent.name, &["\\Seen"], raw.clone()).await;
                        match c.check(r) {
                            Ok(Some((v, uid))) => {
                                sent_id = Some(loc_id(&ImapLocation {
                                    remote_folder_id: sent.name.clone(),
                                    uid_validity: v,
                                    uid,
                                }));
                            }
                            Ok(None) => {}
                            Err(e) => tracing::warn!(error = %e, "sent copy could not be saved"),
                        }
                        c.s().selected = None;
                    }
                    Err(e) => tracing::warn!(error = %e, "no Sent folder for the sent copy"),
                }
            }
            if self.revive(&mut c).await.is_ok()
                && let Err(e) = self.delete_draft_with(&mut c, d).await
                && e != ProviderError::NotFound
            {
                tracing::warn!(error = %e, "sent draft could not be removed");
            }
        }
        if sent_id.is_none()
            && let Ok(Some(found)) = self.find_sent_impl(&want).await
        {
            return Ok(found);
        }
        Ok(SentRef {
            provider_message_id: sent_id.unwrap_or_else(|| format!("<{want}>")),
            provider_thread_id: thread_id,
        })
    }

    pub(crate) async fn find_sent_impl(&self, rfc_message_id: &str) -> Result<Option<SentRef>, ProviderError> {
        let want = imap_thread::clean_message_id(rfc_message_id);
        let mut c = self.conn().await?;
        let mut targets = Vec::new();
        if let Some(f) = self.folder_by_role(LabelRole::Sent) {
            targets.push(f);
        }
        if let Some(f) = self.folder_by_role(LabelRole::Inbox) {
            targets.push(f);
        }
        for f in targets {
            let sel = self.ensure_selected(&mut c, &f.name).await?;
            let Some(uid) = self.find_by_message_id(&mut c, &want).await? else { continue };
            let msgs = self.fetch_messages(&mut c, &f, &sel, &[uid]).await?;
            // SEARCH HEADER is a substring match: confirm the exact id.
            if let Some(m) = msgs
                .into_iter()
                .find(|m| m.rfc_message_id.as_deref().map(imap_thread::clean_message_id) == Some(want.clone()))
            {
                return Ok(Some(SentRef {
                    provider_message_id: m.provider_message_id,
                    provider_thread_id: Some(m.provider_thread_id),
                }));
            }
        }
        Ok(None)
    }
}
