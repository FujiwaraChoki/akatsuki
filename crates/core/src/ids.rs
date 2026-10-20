//! Typed Convex document ids (ARCHITECTURE §1.2).
//!
//! On the wire a Convex id is a plain string (`Id<"threads">`). In Rust the table is carried in the
//! type, so a `ThreadId` cannot be passed where a `MessageId` is expected. The string itself is
//! opaque and never parsed.

use std::cmp::Ordering;
use std::fmt;
use std::hash::{Hash, Hasher};
use std::marker::PhantomData;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// A Convex table, used as the type parameter of [`Id`].
pub trait Table {
    /// The table name as written in `convex/schema.ts`.
    const NAME: &'static str;
}

/// A Convex document id of table `T`. Serializes as a bare string.
pub struct Id<T: Table> {
    raw: String,
    _table: PhantomData<fn() -> T>,
}

impl<T: Table> Id<T> {
    /// Wraps a raw id string. No validation happens: Convex ids are opaque.
    pub fn new(raw: impl Into<String>) -> Self {
        Self { raw: raw.into(), _table: PhantomData }
    }

    /// The raw id string.
    pub fn as_str(&self) -> &str {
        &self.raw
    }

    /// Consumes the id and returns the raw string.
    pub fn into_string(self) -> String {
        self.raw
    }

    /// The table this id belongs to.
    pub fn table(&self) -> &'static str {
        T::NAME
    }
}

impl<T: Table> Default for Id<T> {
    /// An empty id, only useful as a placeholder in builders and tests.
    fn default() -> Self {
        Self::new(String::new())
    }
}

impl<T: Table> Clone for Id<T> {
    fn clone(&self) -> Self {
        Self::new(self.raw.clone())
    }
}

impl<T: Table> PartialEq for Id<T> {
    fn eq(&self, other: &Self) -> bool {
        self.raw == other.raw
    }
}

impl<T: Table> Eq for Id<T> {}

impl<T: Table> PartialOrd for Id<T> {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl<T: Table> Ord for Id<T> {
    fn cmp(&self, other: &Self) -> Ordering {
        self.raw.cmp(&other.raw)
    }
}

impl<T: Table> Hash for Id<T> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.raw.hash(state);
    }
}

impl<T: Table> fmt::Debug for Id<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Id<{}>({})", T::NAME, self.raw)
    }
}

impl<T: Table> fmt::Display for Id<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.raw)
    }
}

impl<T: Table> From<&str> for Id<T> {
    fn from(value: &str) -> Self {
        Self::new(value)
    }
}

impl<T: Table> From<String> for Id<T> {
    fn from(value: String) -> Self {
        Self::new(value)
    }
}

impl<T: Table> Serialize for Id<T> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.raw)
    }
}

impl<'de, T: Table> Deserialize<'de> for Id<T> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        String::deserialize(deserializer).map(Self::new)
    }
}

/// Table markers. Each is an uninhabited type used only as the parameter of [`Id`].
pub mod tables {
    use super::Table;

    macro_rules! tables {
        ($($(#[$doc:meta])* $ty:ident => $name:literal),* $(,)?) => {
            $(
                $(#[$doc])*
                #[derive(Debug)]
                pub enum $ty {}
                impl Table for $ty {
                    const NAME: &'static str = $name;
                }
            )*
        };
    }

    tables! {
        /// `profiles`
        Profiles => "profiles",
        /// `devices`
        Devices => "devices",
        /// `mailAccounts`
        MailAccounts => "mailAccounts",
        /// `accountSecrets`
        AccountSecrets => "accountSecrets",
        /// `syncState`
        SyncState => "syncState",
        /// `folderCursors`
        FolderCursors => "folderCursors",
        /// `accountRevs`
        AccountRevs => "accountRevs",
        /// `tombstones`
        Tombstones => "tombstones",
        /// `identities`
        Identities => "identities",
        /// `labels`
        Labels => "labels",
        /// `threads`
        Threads => "threads",
        /// `threadLabels`
        ThreadLabels => "threadLabels",
        /// `threadSearch`
        ThreadSearch => "threadSearch",
        /// `messages`
        Messages => "messages",
        /// `contacts`
        Contacts => "contacts",
        /// `ops`
        Ops => "ops",
        /// `drafts`
        Drafts => "drafts",
        /// `scheduledSends`
        ScheduledSends => "scheduledSends",
        /// `timers`
        Timers => "timers",
        /// `splits`
        Splits => "splits",
        /// `rules`
        Rules => "rules",
        /// `snippets`
        Snippets => "snippets",
        /// `senderPrefs`
        SenderPrefs => "senderPrefs",
        /// `readReceipts`
        ReadReceipts => "readReceipts",
        /// `aiSettings`
        AiSettings => "aiSettings",
        /// `aiUsage`
        AiUsage => "aiUsage",
        /// `billing`
        Billing => "billing",
        /// `stripeEvents`
        StripeEvents => "stripeEvents",
        /// `_scheduled_functions` (Convex system table)
        ScheduledFunctions => "_scheduled_functions",
    }
}

/// `Id<"mailAccounts">`
pub type AccountId = Id<tables::MailAccounts>;
/// `Id<"threads">`
pub type ThreadId = Id<tables::Threads>;
/// `Id<"messages">`
pub type MessageId = Id<tables::Messages>;
/// `Id<"labels">`
pub type LabelId = Id<tables::Labels>;
/// `Id<"ops">`
pub type OpId = Id<tables::Ops>;
/// `Id<"drafts">`
pub type DraftId = Id<tables::Drafts>;
/// `Id<"scheduledSends">`
pub type ScheduledSendId = Id<tables::ScheduledSends>;
/// `Id<"timers">`
pub type TimerId = Id<tables::Timers>;
/// `Id<"splits">`
pub type SplitId = Id<tables::Splits>;
/// `Id<"rules">`
pub type RuleId = Id<tables::Rules>;
/// `Id<"snippets">`
pub type SnippetId = Id<tables::Snippets>;
/// `Id<"identities">`
pub type IdentityId = Id<tables::Identities>;
/// `Id<"syncState">`
pub type SyncStateId = Id<tables::SyncState>;
/// `Id<"folderCursors">`
pub type FolderCursorId = Id<tables::FolderCursors>;
/// `Id<"_scheduled_functions">`
pub type ScheduledFunctionId = Id<tables::ScheduledFunctions>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serializes_as_bare_string() {
        let id: ThreadId = Id::new("k57abc");
        assert_eq!(serde_json::to_string(&id).unwrap(), "\"k57abc\"");
        let back: ThreadId = serde_json::from_str("\"k57abc\"").unwrap();
        assert_eq!(back, id);
        assert_eq!(id.table(), "threads");
    }
}
