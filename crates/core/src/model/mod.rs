//! The mail domain model shared by the worker, the desktop app and the provider adapters.
//!
//! - [`label_key`]: the one vocabulary for thread state (ARCHITECTURE §1.4).
//! - [`apply_ops`]: visible state as a pure function of synced state and pending actions (§5.1).
//! - [`rules`]: split and filter rule matching on metadata (§3.4).
//! - [`aggregates`]: thread-level label keys derived from message keys (§4.4 step 3).
//!
//! The TypeScript twin is `packages/mail-model` (interim: `convex/lib/model.ts`, D-126). Both run the
//! fixtures in `packages/mail-model/fixtures/*.json`.

pub mod aggregates;
pub mod apply_ops;
pub mod label_key;
pub mod rules;
#[cfg(test)]
mod shared_fixtures;

use serde::{Deserialize, Serialize};

pub use apply_ops::{ApplyContext, Visible, apply_action, apply_ops};
pub use label_key::{LabelKey, LabelKeyError, SplitKey, SysKey, is_label_key, parse_label_key};

/// An email address with an optional display name (`vAddr`). `email` is lowercased and trimmed
/// by whoever builds it from provider data.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Addr {
    /// Display name, if the header carried one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// The address, lowercased and trimmed.
    pub email: String,
}

impl Addr {
    /// Builds an address, normalising the email (trimmed, lowercased) and dropping an empty name.
    pub fn new(name: Option<&str>, email: &str) -> Self {
        let name = name.map(str::trim).filter(|n| !n.is_empty()).map(str::to_owned);
        Self { name, email: email.trim().to_lowercase() }
    }

    /// The domain part of the address (after the last `@`), lowercased.
    pub fn domain(&self) -> Option<&str> {
        self.email.rsplit_once('@').map(|(_, d)| d)
    }
}

/// Parsed `List-Unsubscribe` / `List-Unsubscribe-Post` headers.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListUnsubscribe {
    /// The `https:` unsubscribe URL, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub http: Option<String>,
    /// The `mailto:` unsubscribe URL, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mailto: Option<String>,
    /// RFC 8058 one-click (`List-Unsubscribe-Post: List-Unsubscribe=One-Click`) is offered.
    pub one_click: bool,
}

/// Summary of `Authentication-Results` (`spf`, `dkim`, `dmarc` verdicts such as `"pass"`).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AuthSummary {
    /// SPF verdict.
    pub spf: String,
    /// DKIM verdict.
    pub dkim: String,
    /// DMARC verdict.
    pub dmarc: String,
}

/// Where an IMAP message lives (`imapLocations[]`).
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImapLocation {
    /// Mailbox name (UTF-8).
    pub remote_folder_id: String,
    /// `UIDVALIDITY` of the mailbox when the UID was seen.
    pub uid_validity: u32,
    /// The message UID.
    pub uid: u32,
}

/// A provider thread id (Gmail `threadId`, Graph `conversationId`, IMAP computed id, §9.5).
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct RemoteThreadId(pub String);

impl From<&str> for RemoteThreadId {
    fn from(value: &str) -> Self {
        Self(value.to_owned())
    }
}

/// The nine system label roles that map to `sys:*` keys.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SysLabel {
    /// `sys:inbox`
    Inbox,
    /// `sys:sent`
    Sent,
    /// `sys:drafts`
    Drafts,
    /// `sys:trash`
    Trash,
    /// `sys:spam`
    Spam,
    /// `sys:starred`
    Starred,
    /// `sys:important`
    Important,
    /// `sys:unread`
    Unread,
    /// `sys:snoozed`
    Snoozed,
}

impl SysLabel {
    /// All system labels, in the order the contract lists them.
    pub const ALL: [SysLabel; 9] = [
        SysLabel::Inbox,
        SysLabel::Sent,
        SysLabel::Drafts,
        SysLabel::Trash,
        SysLabel::Spam,
        SysLabel::Starred,
        SysLabel::Important,
        SysLabel::Unread,
        SysLabel::Snoozed,
    ];

    /// The bare role name (`"inbox"`).
    pub fn as_str(self) -> &'static str {
        match self {
            SysLabel::Inbox => "inbox",
            SysLabel::Sent => "sent",
            SysLabel::Drafts => "drafts",
            SysLabel::Trash => "trash",
            SysLabel::Spam => "spam",
            SysLabel::Starred => "starred",
            SysLabel::Important => "important",
            SysLabel::Unread => "unread",
            SysLabel::Snoozed => "snoozed",
        }
    }

    /// The label key (`"sys:inbox"`).
    pub fn key(self) -> String {
        format!("sys:{}", self.as_str())
    }

    /// Parses a bare role name.
    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|s| s.as_str() == name)
    }
}

/// Kind of a label row (`labels.kind`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LabelKind {
    /// Provider system label or well-known folder.
    System,
    /// Gmail user label.
    User,
    /// Graph or IMAP folder (exclusive).
    Folder,
    /// Graph category or Gmail `CATEGORY_*`.
    Category,
    /// IMAP keyword.
    Keyword,
}

/// Role of a label row (`labels.role`): the nine system roles plus `archive` and `all`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LabelRole {
    /// Inbox.
    Inbox,
    /// Sent.
    Sent,
    /// Drafts.
    Drafts,
    /// Trash.
    Trash,
    /// Spam / Junk.
    Spam,
    /// Starred / flagged.
    Starred,
    /// Important.
    Important,
    /// Unread.
    Unread,
    /// `Akatsuki/Snoozed`.
    Snoozed,
    /// The archive folder on folder providers.
    Archive,
    /// "All mail".
    All,
}

impl LabelRole {
    /// The system label this role is addressed as, if it is one of the nine `sys:` roles.
    pub fn sys(self) -> Option<SysLabel> {
        Some(match self {
            LabelRole::Inbox => SysLabel::Inbox,
            LabelRole::Sent => SysLabel::Sent,
            LabelRole::Drafts => SysLabel::Drafts,
            LabelRole::Trash => SysLabel::Trash,
            LabelRole::Spam => SysLabel::Spam,
            LabelRole::Starred => SysLabel::Starred,
            LabelRole::Important => SysLabel::Important,
            LabelRole::Unread => SysLabel::Unread,
            LabelRole::Snoozed => SysLabel::Snoozed,
            LabelRole::Archive | LabelRole::All => return None,
        })
    }
}

/// A label or folder as the provider reports it (`RemoteLabelIn` in `worker.upsertLabels`, and
/// `RemoteLabel` in `crates/providers`, §9.1).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteLabel {
    /// The provider's id for the label or folder.
    pub provider_label_id: String,
    /// Display name.
    pub name: String,
    /// Kind of label.
    pub kind: LabelKind,
    /// System role, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<LabelRole>,
    /// Folder semantics: a message is in at most one exclusive label.
    pub exclusive: bool,
    /// Provider colour, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    /// Hidden in the provider's UI (Gmail `labelListVisibility: labelHide`). Required by
    /// `worker.upsertLabels` (`convex/worker.ts`), not listed in ARCHITECTURE §9.1.
    #[serde(default)]
    pub hidden: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn addr_normalises() {
        let a = Addr::new(Some("  Ada "), "  Ada@Example.COM ");
        assert_eq!(a.email, "ada@example.com");
        assert_eq!(a.name.as_deref(), Some("Ada"));
        assert_eq!(a.domain(), Some("example.com"));
        assert_eq!(serde_json::to_value(Addr::new(None, "x@y.z")).unwrap(), serde_json::json!({"email": "x@y.z"}));
    }

    #[test]
    fn sys_label_round_trip() {
        for s in SysLabel::ALL {
            assert_eq!(SysLabel::from_name(s.as_str()), Some(s));
            assert_eq!(serde_json::to_value(s).unwrap(), serde_json::json!(s.as_str()));
        }
    }

    #[test]
    fn remote_label_wire_shape() {
        let l = RemoteLabel {
            provider_label_id: "Label_1".into(),
            name: "Work".into(),
            kind: LabelKind::User,
            role: None,
            exclusive: false,
            color: None,
            hidden: false,
        };
        assert_eq!(
            serde_json::to_value(&l).unwrap(),
            serde_json::json!({"providerLabelId": "Label_1", "name": "Work", "kind": "user", "exclusive": false, "hidden": false})
        );
    }
}
