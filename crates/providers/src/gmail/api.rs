//! Gmail API v1 resources, as JSON (only the fields Akatsuki reads or writes).
//!
//! `int64` fields (`historyId`, `internalDate`, `expiration`) arrive as decimal strings and are
//! parsed to `u64`/`i64` here; numbers are accepted too.

use serde::{Deserialize, Deserializer, Serialize};

fn de_u64_opt<'de, D: Deserializer<'de>>(d: D) -> Result<Option<u64>, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum V {
        S(String),
        N(u64),
    }
    Ok(match Option::<V>::deserialize(d)? {
        None => None,
        Some(V::N(n)) => Some(n),
        Some(V::S(s)) => Some(s.trim().parse().map_err(serde::de::Error::custom)?),
    })
}

fn de_i64_opt<'de, D: Deserializer<'de>>(d: D) -> Result<Option<i64>, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum V {
        S(String),
        N(i64),
    }
    Ok(match Option::<V>::deserialize(d)? {
        None => None,
        Some(V::N(n)) => Some(n),
        Some(V::S(s)) => Some(s.trim().parse().map_err(serde::de::Error::custom)?),
    })
}

/// `users.getProfile`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Profile {
    /// The mailbox address.
    pub email_address: String,
    /// Messages in the mailbox.
    pub messages_total: Option<u64>,
    /// Threads in the mailbox.
    pub threads_total: Option<u64>,
    /// Current history id.
    #[serde(deserialize_with = "de_u64_opt")]
    pub history_id: Option<u64>,
}

/// Label colour.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LabelColor {
    /// Text colour (`#rrggbb`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text_color: Option<String>,
    /// Background colour (`#rrggbb`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub background_color: Option<String>,
}

/// A label.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Label {
    /// Label id (`INBOX`, `Label_12`, `CATEGORY_UPDATES`).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub id: String,
    /// Display name (`/` separates nesting levels).
    #[serde(default)]
    pub name: String,
    /// `system` or `user`.
    #[serde(default, rename = "type", skip_serializing_if = "Option::is_none")]
    pub label_type: Option<String>,
    /// `show`, `hide`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message_list_visibility: Option<String>,
    /// `labelShow`, `labelShowIfUnread`, `labelHide`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label_list_visibility: Option<String>,
    /// Colour.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<LabelColor>,
}

/// `labels.list`.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub struct ListLabels {
    /// Labels.
    pub labels: Vec<Label>,
}

/// A header.
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub struct Header {
    /// Name as sent.
    pub name: String,
    /// Value.
    pub value: String,
}

/// A part body.
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct PartBody {
    /// Set when the data must be fetched with `attachments.get`.
    pub attachment_id: Option<String>,
    /// Decoded size in bytes.
    pub size: Option<u64>,
    /// base64url data (small parts only).
    pub data: Option<String>,
}

/// A MIME part (`payload`).
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct MessagePart {
    /// Gmail part id (`""`, `"0"`, `"1.0"`).
    pub part_id: Option<String>,
    /// `type/subtype`.
    pub mime_type: String,
    /// File name, empty for body parts.
    pub filename: String,
    /// Headers (with `format=metadata`, only the requested ones).
    pub headers: Vec<Header>,
    /// Body.
    pub body: Option<PartBody>,
    /// Children of a multipart.
    pub parts: Vec<MessagePart>,
}

impl MessagePart {
    /// The first header with this name (case-insensitive).
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|h| h.name.eq_ignore_ascii_case(name)).map(|h| h.value.as_str())
    }
}

/// A message.
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Message {
    /// Message id.
    pub id: String,
    /// Thread id.
    pub thread_id: String,
    /// Label ids.
    pub label_ids: Vec<String>,
    /// Snippet (HTML entities escaped).
    pub snippet: String,
    /// History id of the last change.
    #[serde(deserialize_with = "de_u64_opt")]
    pub history_id: Option<u64>,
    /// Internal date (ms).
    #[serde(deserialize_with = "de_i64_opt")]
    pub internal_date: Option<i64>,
    /// Estimated size in bytes.
    pub size_estimate: Option<u64>,
    /// Parsed structure (`metadata`, `full`).
    pub payload: Option<MessagePart>,
    /// base64url RFC 822 source (`raw`).
    pub raw: Option<String>,
}

/// A thread.
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Thread {
    /// Thread id.
    pub id: String,
    /// Snippet.
    pub snippet: String,
    /// History id of the last change (the thread's version).
    #[serde(deserialize_with = "de_u64_opt")]
    pub history_id: Option<u64>,
    /// Messages, oldest first.
    pub messages: Vec<Message>,
}

/// `threads.list`.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ListThreads {
    /// Thread stubs (`id`, `snippet`, `historyId`).
    pub threads: Vec<Thread>,
    /// Next page.
    pub next_page_token: Option<String>,
    /// Estimate.
    pub result_size_estimate: Option<u64>,
}

/// `messages.list`.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ListMessages {
    /// Message stubs (`id`, `threadId`).
    pub messages: Vec<Message>,
    /// Next page.
    pub next_page_token: Option<String>,
    /// Estimate.
    pub result_size_estimate: Option<u64>,
}

/// A message changed in a history record.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct HistoryMessage {
    /// Stub (`id`, `threadId`, `labelIds`).
    pub message: Message,
    /// Labels added or removed (`labelsAdded`/`labelsRemoved`).
    pub label_ids: Vec<String>,
}

/// A history record.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct History {
    /// Record id.
    #[serde(deserialize_with = "de_u64_opt")]
    pub id: Option<u64>,
    /// Messages touched by the record.
    pub messages: Vec<Message>,
    /// Added messages.
    pub messages_added: Vec<HistoryMessage>,
    /// Deleted messages.
    pub messages_deleted: Vec<HistoryMessage>,
    /// Labels added.
    pub labels_added: Vec<HistoryMessage>,
    /// Labels removed.
    pub labels_removed: Vec<HistoryMessage>,
}

/// `history.list`.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ListHistory {
    /// Records, oldest first.
    pub history: Vec<History>,
    /// Next page.
    pub next_page_token: Option<String>,
    /// The mailbox's current history id.
    #[serde(deserialize_with = "de_u64_opt")]
    pub history_id: Option<u64>,
}

/// A draft.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub struct Draft {
    /// Draft id.
    pub id: String,
    /// Its message.
    pub message: Option<Message>,
}

/// `users.watch`.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct WatchResponse {
    /// History id at watch start.
    #[serde(deserialize_with = "de_u64_opt")]
    pub history_id: Option<u64>,
    /// Expiry (ms).
    #[serde(deserialize_with = "de_i64_opt")]
    pub expiration: Option<i64>,
}

/// `messages.attachments.get`.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub struct Attachment {
    /// Size.
    pub size: Option<u64>,
    /// base64url data.
    pub data: String,
}

/// A send-as alias.
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct SendAs {
    /// Address.
    pub send_as_email: String,
    /// Display name.
    pub display_name: Option<String>,
    /// Reply-To.
    pub reply_to_address: Option<String>,
    /// Signature (HTML).
    pub signature: Option<String>,
    /// The account's primary address.
    pub is_primary: bool,
    /// The default From.
    pub is_default: bool,
    /// `accepted`, `pending`.
    pub verification_status: Option<String>,
}

/// `settings.sendAs.list`.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ListSendAs {
    /// Aliases.
    pub send_as: Vec<SendAs>,
}

/// The Gmail error envelope.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub struct ErrorEnvelope {
    /// Error.
    pub error: ErrorBody,
}

/// The Gmail error body.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub struct ErrorBody {
    /// HTTP status.
    pub code: u16,
    /// Message.
    pub message: String,
    /// `PERMISSION_DENIED`, `NOT_FOUND`…
    pub status: String,
    /// Reasons.
    pub errors: Vec<ErrorItem>,
}

/// One error reason.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub struct ErrorItem {
    /// `rateLimitExceeded`, `userRateLimitExceeded`, `dailyLimitExceeded`, `failedPrecondition`…
    pub reason: String,
    /// `usageLimits`, `global`…
    pub domain: String,
    /// Message.
    pub message: String,
}
