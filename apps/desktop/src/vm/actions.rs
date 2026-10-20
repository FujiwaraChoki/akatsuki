//! Client action helpers (ARCHITECTURE §5.1 table): the only way the desktop builds a
//! `ThreadAction`. On Gmail most actions are label-key deltas; on folder providers (Graph, IMAP)
//! location changes are `move`s between exclusive folders.

use akatsuki_core::convex::rows::{LabelView, MailAccountView};
use akatsuki_core::convex::{Provider, ThreadAction, UnsubscribeMethod};
use akatsuki_core::model::{LabelRole, ListUnsubscribe};

/// What an account can do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AccountCtx {
    /// Provider.
    pub provider: Provider,
    /// Gmail-style labels (`caps.multiLabel`).
    pub multi_label: bool,
    /// Permanent delete is allowed.
    pub delete_forever: bool,
    /// The role-`archive` folder's key on folder providers.
    pub archive_key: Option<String>,
}

impl AccountCtx {
    /// Builds the context from the mirror's account and label rows.
    pub fn new(account: &MailAccountView, labels: &[LabelView]) -> Self {
        let archive_key = labels
            .iter()
            .find(|l| l.account_id == account.id && l.role == Some(LabelRole::Archive))
            .map(|l| l.label_key.clone());
        Self {
            provider: account.provider,
            multi_label: account.caps.multi_label,
            delete_forever: account.caps.delete_forever,
            archive_key,
        }
    }

    /// A Gmail account (tests, defaults).
    pub fn gmail() -> Self {
        Self { provider: Provider::Gmail, multi_label: true, delete_forever: false, archive_key: None }
    }
}

/// Why an action cannot be built.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ActionError {
    /// The folder provider has no archive folder yet.
    #[error("This account has no Archive folder yet.")]
    NoArchiveFolder,
    /// The thread has no unsubscribe option.
    #[error("This thread has no unsubscribe option.")]
    NoUnsubscribe,
}

fn modify(add: &[&str], remove: &[&str]) -> ThreadAction {
    ThreadAction::Modify {
        add: add.iter().map(|s| (*s).to_owned()).collect(),
        remove: remove.iter().map(|s| (*s).to_owned()).collect(),
    }
}

/// `archive()`.
pub fn archive(ctx: &AccountCtx) -> Result<ThreadAction, ActionError> {
    if ctx.multi_label {
        Ok(modify(&[], &["sys:inbox"]))
    } else {
        ctx.archive_key.clone().map(|to| ThreadAction::Move { to }).ok_or(ActionError::NoArchiveFolder)
    }
}

/// `moveToInbox()`.
pub fn move_to_inbox(ctx: &AccountCtx) -> ThreadAction {
    if ctx.multi_label { modify(&["sys:inbox"], &[]) } else { ThreadAction::Move { to: "sys:inbox".into() } }
}

/// `markRead()` / `markUnread()`.
pub fn set_read(read: bool) -> ThreadAction {
    if read { modify(&[], &["sys:unread"]) } else { modify(&["sys:unread"], &[]) }
}

/// `star()` / `unstar()`.
pub fn set_starred(starred: bool) -> ThreadAction {
    if starred { modify(&["sys:starred"], &[]) } else { modify(&[], &["sys:starred"]) }
}

/// `spam()`.
pub fn spam(ctx: &AccountCtx) -> ThreadAction {
    if ctx.multi_label { modify(&["sys:spam"], &["sys:inbox"]) } else { ThreadAction::Move { to: "sys:spam".into() } }
}

/// `notSpam()`.
pub fn not_spam(ctx: &AccountCtx) -> ThreadAction {
    if ctx.multi_label { modify(&["sys:inbox"], &["sys:spam"]) } else { ThreadAction::Move { to: "sys:inbox".into() } }
}

/// `trash()`.
pub fn trash() -> ThreadAction {
    ThreadAction::Trash
}

/// `untrash()`.
pub fn untrash() -> ThreadAction {
    ThreadAction::Untrash
}

/// `addLabel(id)`: labels and categories are `modify`; folders are `move`.
pub fn add_label(label: &LabelView) -> ThreadAction {
    if label.exclusive {
        ThreadAction::Move { to: label.label_key.clone() }
    } else {
        ThreadAction::Modify { add: vec![label.label_key.clone()], remove: vec![] }
    }
}

/// `removeLabel(id)`.
pub fn remove_label(label: &LabelView, ctx: &AccountCtx) -> ThreadAction {
    if label.exclusive {
        move_to_inbox(ctx)
    } else {
        ThreadAction::Modify { add: vec![], remove: vec![label.label_key.clone()] }
    }
}

/// `unsubscribe(m)`: one-click POST when offered, else mailto.
pub fn unsubscribe(options: Option<&ListUnsubscribe>) -> Result<ThreadAction, ActionError> {
    match options {
        Some(o) if o.one_click && o.http.is_some() => {
            Ok(ThreadAction::Unsubscribe { method: UnsubscribeMethod::OneClickPost })
        }
        Some(o) if o.mailto.is_some() => Ok(ThreadAction::Unsubscribe { method: UnsubscribeMethod::Mailto }),
        _ => Err(ActionError::NoUnsubscribe),
    }
}

/// Where a mailto unsubscribe goes: `(address, foreign)`, where `foreign` means the address is not
/// at the sender's domain (compared on the last two labels). Shown before anything is sent.
pub fn mailto_target(mailto: &str, sender_email: Option<&str>) -> Option<(String, bool)> {
    let rest = mailto.trim();
    let rest = rest.get(..7).filter(|p| p.eq_ignore_ascii_case("mailto:")).map(|_| &rest[7..])?;
    let addr = rest.split('?').next()?;
    let addr = percent_decode(addr).split(',').next()?.trim().to_lowercase();
    let (local, domain) = addr.split_once('@')?;
    if local.is_empty() || !domain.contains('.') || addr.contains(char::is_whitespace) {
        return None;
    }
    let base = |d: &str| {
        let parts: Vec<&str> = d.trim_end_matches('.').split('.').collect();
        parts[parts.len().saturating_sub(2)..].join(".").to_lowercase()
    };
    let foreign = sender_email.and_then(|s| s.split_once('@')).is_some_and(|(_, d)| base(d) != base(domain));
    Some((addr, foreign))
}

fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%'
            && i + 2 < b.len()
            && let Some(v) = std::str::from_utf8(&b[i + 1..i + 3]).ok().and_then(|h| u8::from_str_radix(h, 16).ok())
        {
            out.push(v);
            i += 3;
            continue;
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// The local overlay row for `mail.snooze` (the server enqueues the same delta, §10).
pub fn snooze_overlay(ctx: &AccountCtx) -> ThreadAction {
    if ctx.multi_label {
        modify(&["sys:snoozed"], &["sys:inbox"])
    } else {
        ThreadAction::Move { to: "sys:snoozed".into() }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mailto_targets_are_shown_and_foreign_ones_flagged() {
        assert_eq!(
            mailto_target("mailto:Leave@Lists.Shop.example?subject=x", Some("news@shop.example")),
            Some(("leave@lists.shop.example".into(), false))
        );
        assert_eq!(
            mailto_target("mailto:ceo%40victim.example?subject=I%20quit", Some("promo@spoofed.example")),
            Some(("ceo@victim.example".into(), true))
        );
        assert_eq!(mailto_target("https://x.example", None), None);
        assert_eq!(mailto_target("mailto:?subject=x", None), None);
    }
    use serde_json::json;

    fn folder_ctx() -> AccountCtx {
        AccountCtx {
            provider: Provider::Graph,
            multi_label: false,
            delete_forever: true,
            archive_key: Some("l:arch".into()),
        }
    }

    fn label(exclusive: bool) -> LabelView {
        serde_json::from_value(json!({"_id": "l1", "_creationTime": 1.0, "accountId": "a", "name": "Work",
            "kind": if exclusive {"folder"} else {"user"}, "exclusive": exclusive, "hidden": false,
            "pendingDelete": false, "labelKey": "l:l1"}))
        .unwrap()
    }

    #[test]
    fn gmail_uses_label_deltas() {
        let g = AccountCtx::gmail();
        assert_eq!(archive(&g).unwrap(), modify(&[], &["sys:inbox"]));
        assert_eq!(move_to_inbox(&g), modify(&["sys:inbox"], &[]));
        assert_eq!(spam(&g), modify(&["sys:spam"], &["sys:inbox"]));
        assert_eq!(not_spam(&g), modify(&["sys:inbox"], &["sys:spam"]));
        assert_eq!(snooze_overlay(&g), modify(&["sys:snoozed"], &["sys:inbox"]));
    }

    #[test]
    fn folder_providers_move() {
        let f = folder_ctx();
        assert_eq!(archive(&f).unwrap(), ThreadAction::Move { to: "l:arch".into() });
        assert_eq!(move_to_inbox(&f), ThreadAction::Move { to: "sys:inbox".into() });
        assert_eq!(spam(&f), ThreadAction::Move { to: "sys:spam".into() });
        assert_eq!(archive(&AccountCtx { archive_key: None, ..f }), Err(ActionError::NoArchiveFolder));
    }

    #[test]
    fn labels_folders_and_flags() {
        assert_eq!(add_label(&label(false)), ThreadAction::Modify { add: vec!["l:l1".into()], remove: vec![] });
        assert_eq!(add_label(&label(true)), ThreadAction::Move { to: "l:l1".into() });
        assert_eq!(remove_label(&label(true), &folder_ctx()), ThreadAction::Move { to: "sys:inbox".into() });
        assert_eq!(set_read(true), modify(&[], &["sys:unread"]));
        assert_eq!(set_starred(true), modify(&["sys:starred"], &[]));
    }

    #[test]
    fn unsubscribe_prefers_one_click() {
        let both = ListUnsubscribe { http: Some("https://x".into()), mailto: Some("mailto:x".into()), one_click: true };
        assert_eq!(
            unsubscribe(Some(&both)).unwrap(),
            ThreadAction::Unsubscribe { method: UnsubscribeMethod::OneClickPost }
        );
        let mailto =
            ListUnsubscribe { http: Some("https://x".into()), mailto: Some("mailto:x".into()), one_click: false };
        assert_eq!(
            unsubscribe(Some(&mailto)).unwrap(),
            ThreadAction::Unsubscribe { method: UnsubscribeMethod::Mailto }
        );
        assert_eq!(unsubscribe(None), Err(ActionError::NoUnsubscribe));
    }
}
