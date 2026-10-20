//! Provider label tables (ARCHITECTURE §9.5). Pure lookups shared by the adapters.

use crate::{LabelKind, LabelRole, SysLabel};

/// The provider-side label or folder that backs snooze (`sys:snoozed`, D-037).
pub const SNOOZED_LABEL_NAME: &str = "Akatsuki/Snoozed";

/// Gmail system label ids that map to `sys:*` keys.
pub fn gmail_system_label(id: &str) -> Option<SysLabel> {
    Some(match id {
        "INBOX" => SysLabel::Inbox,
        "SENT" => SysLabel::Sent,
        "DRAFT" => SysLabel::Drafts,
        "TRASH" => SysLabel::Trash,
        "SPAM" => SysLabel::Spam,
        "STARRED" => SysLabel::Starred,
        "IMPORTANT" => SysLabel::Important,
        "UNREAD" => SysLabel::Unread,
        _ => return None,
    })
}

/// The kind and role of a Gmail label, from its id, `type` (`system`/`user`) and name.
pub fn gmail_label(id: &str, label_type: &str, name: &str) -> (LabelKind, Option<LabelRole>) {
    if let Some(sys) = gmail_system_label(id) {
        return (LabelKind::System, Some(role_of(sys)));
    }
    if id.starts_with("CATEGORY_") {
        return (LabelKind::Category, None);
    }
    if name == SNOOZED_LABEL_NAME {
        return (LabelKind::User, Some(LabelRole::Snoozed));
    }
    if label_type == "system" { (LabelKind::System, None) } else { (LabelKind::User, None) }
}

/// Graph well-known folder names (`wellKnownName`) and the role they carry.
pub fn graph_well_known_folder(name: &str) -> Option<LabelRole> {
    Some(match name.to_ascii_lowercase().as_str() {
        "inbox" => LabelRole::Inbox,
        "sentitems" => LabelRole::Sent,
        "drafts" => LabelRole::Drafts,
        "deleteditems" => LabelRole::Trash,
        "junkemail" => LabelRole::Spam,
        "archive" => LabelRole::Archive,
        _ => return None,
    })
}

/// IMAP SPECIAL-USE attributes (RFC 6154) and the role they carry. `INBOX` is matched by name.
pub fn imap_special_use(mailbox: &str, attributes: &[&str]) -> Option<LabelRole> {
    if mailbox.eq_ignore_ascii_case("INBOX") {
        return Some(LabelRole::Inbox);
    }
    if mailbox == SNOOZED_LABEL_NAME {
        return Some(LabelRole::Snoozed);
    }
    for a in attributes {
        let role = match a.to_ascii_lowercase().as_str() {
            "\\sent" => LabelRole::Sent,
            "\\drafts" => LabelRole::Drafts,
            "\\trash" => LabelRole::Trash,
            "\\junk" => LabelRole::Spam,
            "\\archive" => LabelRole::Archive,
            "\\all" => LabelRole::All,
            "\\flagged" => LabelRole::Starred,
            _ => continue,
        };
        return Some(role);
    }
    None
}

/// The role of a system label.
pub fn role_of(sys: SysLabel) -> LabelRole {
    match sys {
        SysLabel::Inbox => LabelRole::Inbox,
        SysLabel::Sent => LabelRole::Sent,
        SysLabel::Drafts => LabelRole::Drafts,
        SysLabel::Trash => LabelRole::Trash,
        SysLabel::Spam => LabelRole::Spam,
        SysLabel::Starred => LabelRole::Starred,
        SysLabel::Important => LabelRole::Important,
        SysLabel::Unread => LabelRole::Unread,
        SysLabel::Snoozed => LabelRole::Snoozed,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gmail() {
        assert_eq!(gmail_label("INBOX", "system", "INBOX"), (LabelKind::System, Some(LabelRole::Inbox)));
        assert_eq!(gmail_label("CATEGORY_PROMOTIONS", "system", "CATEGORY_PROMOTIONS"), (LabelKind::Category, None));
        assert_eq!(gmail_label("Label_9", "user", SNOOZED_LABEL_NAME), (LabelKind::User, Some(LabelRole::Snoozed)));
        assert_eq!(gmail_label("Label_1", "user", "Work"), (LabelKind::User, None));
        assert_eq!(gmail_system_label("CHAT"), None);
    }

    #[test]
    fn graph_and_imap() {
        assert_eq!(graph_well_known_folder("SentItems"), Some(LabelRole::Sent));
        assert_eq!(graph_well_known_folder("archive"), Some(LabelRole::Archive));
        assert_eq!(imap_special_use("inbox", &[]), Some(LabelRole::Inbox));
        assert_eq!(imap_special_use("[Gmail]/Spam", &["\\HasNoChildren", "\\Junk"]), Some(LabelRole::Spam));
        assert_eq!(imap_special_use("Projects", &["\\HasChildren"]), None);
    }
}
