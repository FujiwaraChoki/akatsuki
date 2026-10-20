//! Mailboxes → labels (ARCHITECTURE §9.5 "Label map").
//!
//! - Special-use `\Inbox \Sent \Drafts \Trash \Junk` → `sys:*` (kind `system`).
//! - `\Archive` → an `l:` folder with role `archive`; `Akatsuki/Snoozed` → `sys:snoozed`.
//! - Other selectable mailboxes → `l:` folders (exclusive).
//! - Keywords → `l:` with kind `keyword` (only when `PERMANENTFLAGS` contains `\*`). Their provider
//!   label id is [`KEYWORD_PREFIX`] + the keyword, so it never collides with a mailbox name used as
//!   a folder id in practice.
//! - `\All` and `\Flagged` mailboxes are virtual views of other folders: they are neither labels
//!   nor synced, so a message is never counted twice.
//!
//! Servers without SPECIAL-USE get the same roles from well-known names (`Sent Items`,
//! `INBOX.Trash`, `Junk E-mail`, …) when no mailbox carries the attribute.

use crate::labels::{SNOOZED_LABEL_NAME, imap_special_use};
use crate::{LabelKind, LabelRole, RemoteLabel};

use super::session::ListEntry;

/// Provider label id prefix for IMAP keywords.
pub const KEYWORD_PREFIX: &str = "keyword:";

/// Keywords that record client state rather than user labels.
pub fn is_ignored_keyword(k: &str) -> bool {
    const IGNORED: [&str; 16] = [
        "$forwarded",
        "$mdnsent",
        "$junk",
        "$notjunk",
        "junk",
        "nonjunk",
        "notjunk",
        "$phishing",
        "$important",
        "$submitpending",
        "$submitted",
        "$hasattachment",
        "$hasnoattachment",
        "$label1",
        "forwarded",
        "$autojunk",
    ];
    let l = k.to_ascii_lowercase();
    IGNORED.contains(&l.as_str()) || l.starts_with("$x-") || l.starts_with("$mailflagbit")
}

/// One mailbox.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Folder {
    /// Mailbox name (UTF-8): the `remoteFolderId` and provider label id.
    pub name: String,
    /// Hierarchy delimiter.
    pub delimiter: Option<String>,
    /// Lowercased attributes.
    pub attrs: Vec<String>,
    /// Role.
    pub role: Option<LabelRole>,
}

impl Folder {
    /// Can be selected.
    pub fn selectable(&self) -> bool {
        !self.attrs.iter().any(|a| a == "\\noselect" || a == "\\nonexistent")
    }

    /// Synced and exposed as a label (selectable, not a virtual `\All`/`\Flagged` view).
    pub fn synced(&self) -> bool {
        self.selectable() && !matches!(self.role, Some(LabelRole::All | LabelRole::Starred))
    }

    /// The display path with `/` as separator (`INBOX.Work` → `INBOX/Work`).
    pub fn display_name(&self) -> String {
        match self.delimiter.as_deref() {
            Some(d) if d != "/" && !d.is_empty() => self.name.replace(d, "/"),
            _ => self.name.clone(),
        }
    }

    /// The label for `worker.upsertLabels`.
    pub fn label(&self) -> RemoteLabel {
        let kind = match self.role {
            Some(LabelRole::Inbox | LabelRole::Sent | LabelRole::Drafts | LabelRole::Trash | LabelRole::Spam) => {
                LabelKind::System
            }
            _ => LabelKind::Folder,
        };
        let name = match self.role {
            Some(LabelRole::Inbox) => "INBOX".to_owned(),
            Some(LabelRole::Snoozed) => SNOOZED_LABEL_NAME.to_owned(),
            _ => self.display_name(),
        };
        RemoteLabel {
            provider_label_id: self.name.clone(),
            name,
            kind,
            role: self.role,
            exclusive: true,
            color: None,
            hidden: false,
        }
    }
}

/// A keyword label.
pub fn keyword_label(keyword: &str) -> RemoteLabel {
    RemoteLabel {
        provider_label_id: format!("{KEYWORD_PREFIX}{keyword}"),
        name: keyword.to_owned(),
        kind: LabelKind::Keyword,
        role: None,
        exclusive: false,
        color: None,
        hidden: false,
    }
}

fn strip_personal_prefix(display: &str) -> &str {
    match display.split_once('/') {
        Some((first, rest)) if first.eq_ignore_ascii_case("INBOX") && !rest.is_empty() => rest,
        _ => display,
    }
}

fn role_by_name(display: &str) -> Option<LabelRole> {
    let leaf = strip_personal_prefix(display).to_ascii_lowercase();
    Some(match leaf.as_str() {
        "sent" | "sent items" | "sent messages" | "sent mail" | "gesendet" | "gesendete elemente" | "envoyés" => {
            LabelRole::Sent
        }
        "drafts" | "draft" | "entwürfe" | "brouillons" => LabelRole::Drafts,
        "trash" | "deleted" | "deleted items" | "deleted messages" | "bin" | "papierkorb" | "corbeille" => {
            LabelRole::Trash
        }
        "junk" | "spam" | "junk e-mail" | "junk email" | "bulk mail" => LabelRole::Spam,
        "archive" | "archives" | "archiv" => LabelRole::Archive,
        _ => return None,
    })
}

/// Builds the folder list from `LIST` and assigns roles.
pub(crate) fn folders_from_list(entries: Vec<ListEntry>) -> Vec<Folder> {
    let mut out: Vec<Folder> = entries
        .into_iter()
        .filter(|e| !e.name.is_empty())
        .map(|e| {
            let attrs: Vec<&str> = e.attrs.iter().map(String::as_str).collect();
            let mut role = imap_special_use(&e.name, &attrs);
            let f = Folder { name: e.name.clone(), delimiter: e.delimiter.clone(), attrs: e.attrs.clone(), role: None };
            if role.is_none() && f.display_name().ends_with(SNOOZED_LABEL_NAME) {
                role = Some(LabelRole::Snoozed);
            }
            Folder { role, ..f }
        })
        .collect();
    // Name heuristics for servers without SPECIAL-USE, one folder per role, never overriding an
    // attribute.
    for role in [LabelRole::Sent, LabelRole::Drafts, LabelRole::Trash, LabelRole::Spam, LabelRole::Archive] {
        if out.iter().any(|f| f.role == Some(role)) {
            continue;
        }
        if let Some(f) =
            out.iter_mut().find(|f| f.role.is_none() && f.selectable() && role_by_name(&f.display_name()) == Some(role))
        {
            f.role = Some(role);
        }
    }
    // INBOX first, then by name, so paging over folders is stable.
    out.sort_by(|a, b| {
        let ai = a.role == Some(LabelRole::Inbox);
        let bi = b.role == Some(LabelRole::Inbox);
        bi.cmp(&ai).then_with(|| a.name.cmp(&b.name))
    });
    out.dedup_by(|a, b| a.name == b.name);
    out
}

/// The mailbox name for a new folder at `/`-separated `path`, using the server's delimiter and,
/// when every other folder lives under `INBOX.`, that personal namespace prefix.
pub fn mailbox_for_path(folders: &[Folder], path: &str) -> String {
    let delim = folders.iter().find_map(|f| f.delimiter.clone()).unwrap_or_else(|| "/".to_owned());
    let others: Vec<&Folder> = folders.iter().filter(|f| !f.name.eq_ignore_ascii_case("INBOX")).collect();
    let prefix = format!("INBOX{delim}");
    let under_inbox = !others.is_empty() && others.iter().all(|f| f.name.starts_with(&prefix));
    let mut name = path.split('/').filter(|p| !p.is_empty()).collect::<Vec<_>>().join(&delim);
    if under_inbox && !name.starts_with(&prefix) {
        name = format!("{prefix}{name}");
    }
    name
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(name: &str, delim: &str, attrs: &[&str]) -> ListEntry {
        ListEntry {
            name: name.into(),
            delimiter: Some(delim.into()),
            attrs: attrs.iter().map(|a| a.to_ascii_lowercase()).collect(),
        }
    }

    #[test]
    fn special_use_and_virtual_folders() {
        let f = folders_from_list(vec![
            entry("[Gmail]/All Mail", "/", &["\\All", "\\HasNoChildren"]),
            entry("[Gmail]/Starred", "/", &["\\Flagged"]),
            entry("[Gmail]", "/", &["\\Noselect"]),
            entry("Projects", "/", &[]),
            entry("INBOX", "/", &[]),
            entry("[Gmail]/Sent Mail", "/", &["\\Sent"]),
            entry("Akatsuki/Snoozed", "/", &[]),
            entry("Sent", "/", &[]),
        ]);
        assert_eq!(f[0].name, "INBOX");
        let role = |n: &str| f.iter().find(|x| x.name == n).unwrap().role;
        assert_eq!(role("[Gmail]/Sent Mail"), Some(LabelRole::Sent));
        // A name heuristic never steals a role from a special-use folder.
        assert_eq!(role("Sent"), None);
        assert_eq!(role("Akatsuki/Snoozed"), Some(LabelRole::Snoozed));
        let synced: Vec<&str> = f.iter().filter(|x| x.synced()).map(|x| x.name.as_str()).collect();
        assert_eq!(synced, vec!["INBOX", "Akatsuki/Snoozed", "Projects", "Sent", "[Gmail]/Sent Mail"]);
        let inbox = f[0].label();
        assert_eq!((inbox.kind, inbox.role, inbox.exclusive), (LabelKind::System, Some(LabelRole::Inbox), true));
        assert_eq!(f.iter().find(|x| x.name == "Projects").unwrap().label().kind, LabelKind::Folder);
    }

    #[test]
    fn names_without_special_use() {
        let f = folders_from_list(vec![
            entry("INBOX", ".", &[]),
            entry("INBOX.Sent Items", ".", &[]),
            entry("INBOX.Deleted Messages", ".", &[]),
            entry("INBOX.Junk E-mail", ".", &[]),
            entry("INBOX.Drafts", ".", &[]),
            entry("INBOX.Archive", ".", &[]),
            entry("INBOX.Akatsuki.Snoozed", ".", &[]),
        ]);
        let role = |n: &str| f.iter().find(|x| x.name == n).unwrap().role;
        assert_eq!(role("INBOX.Sent Items"), Some(LabelRole::Sent));
        assert_eq!(role("INBOX.Deleted Messages"), Some(LabelRole::Trash));
        assert_eq!(role("INBOX.Junk E-mail"), Some(LabelRole::Spam));
        assert_eq!(role("INBOX.Drafts"), Some(LabelRole::Drafts));
        assert_eq!(role("INBOX.Archive"), Some(LabelRole::Archive));
        assert_eq!(role("INBOX.Akatsuki.Snoozed"), Some(LabelRole::Snoozed));
        let archive = f.iter().find(|x| x.name == "INBOX.Archive").unwrap().label();
        assert_eq!((archive.kind, archive.name.as_str()), (LabelKind::Folder, "INBOX/Archive"));
        assert_eq!(mailbox_for_path(&f, "Akatsuki/Snoozed"), "INBOX.Akatsuki.Snoozed");
        let flat = folders_from_list(vec![entry("INBOX", "/", &[]), entry("Sent", "/", &[])]);
        assert_eq!(mailbox_for_path(&flat, "Akatsuki/Snoozed"), "Akatsuki/Snoozed");
    }

    #[test]
    fn keywords() {
        assert!(is_ignored_keyword("$Forwarded"));
        assert!(is_ignored_keyword("NonJunk"));
        assert!(!is_ignored_keyword("ProjectX"));
        let k = keyword_label("ProjectX");
        assert_eq!(
            (k.provider_label_id.as_str(), k.kind, k.exclusive),
            ("keyword:ProjectX", LabelKind::Keyword, false)
        );
    }
}
