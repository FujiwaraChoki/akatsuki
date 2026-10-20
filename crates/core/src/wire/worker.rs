//! Sync worker HTTP API types (`packages/wire/src/worker.ts`, ARCHITECTURE §7.3).
//!
//! Fixtures: `packages/wire/fixtures/worker/*.json` (round-tripped by `wire::shared_fixtures`; the
//! TypeScript side runs them once `packages/wire` exists, D-126).

use serde::{Deserialize, Serialize};
use serde_with::skip_serializing_none;

use crate::convex::TlsMode;
use crate::convex::worker::ScheduleOutcome;
use crate::ids::{AccountId, DraftId, MessageId, ScheduledSendId, ThreadId};
use crate::model::{Addr, AuthSummary, ListUnsubscribe};
use crate::time::UnixMs;

/// `POST /v1/connect/{google|microsoft}/start` request.
#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectStart {
    /// Where to send the user afterwards (allow-listed).
    pub return_to: String,
    /// Provider login hint.
    pub login_hint: Option<String>,
    /// Set for a reauth.
    pub account_id: Option<AccountId>,
}

/// `POST /v1/connect/{provider}/start` response.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectStartResponse {
    /// Provider authorization URL.
    pub authorize_url: String,
}

/// `POST /v1/connect/complete` request: the grant handle the OAuth callback put in `returnTo`
/// (`?status=pending&grant=…`, DECISIONS D-395).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectComplete {
    /// Grant handle (single use, 10 minutes).
    pub grant: String,
}

/// `POST /v1/connect/complete` 201 response.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectCompleteResponse {
    /// The connected account.
    pub account_id: AccountId,
    /// The same user reconnected an existing mailbox (a reauth).
    pub reconnected: bool,
}

/// Where discovered IMAP settings came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ImapDiscoverySource {
    /// The built-in table.
    WellKnown,
    /// The domain's own autoconfig file.
    Autoconfig,
    /// The Thunderbird ISPDB.
    Ispdb,
}

/// `GET /v1/connect/imap/discover?email=` response (DECISIONS D-396).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImapDiscovery {
    /// IMAP endpoint.
    pub imap: Endpoint,
    /// SMTP submission endpoint.
    pub smtp: Endpoint,
    /// The login name to pre-fill.
    pub username: String,
    /// Where it came from.
    pub source: ImapDiscoverySource,
    /// The provider requires an app-specific password.
    pub app_password_required: bool,
}

/// `GET /v1/connect/imap/discover` query.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImapDiscoverQuery {
    /// The address being connected.
    pub email: String,
}

/// One IMAP or SMTP endpoint.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Endpoint {
    /// Host name.
    pub host: String,
    /// Port.
    pub port: u16,
    /// TLS mode.
    pub tls: TlsMode,
}

/// `POST /v1/connect/imap` request. The password crosses TLS once and is never echoed or logged.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImapConnect {
    /// Address.
    pub email: String,
    /// Login name.
    pub username: String,
    /// Password or app password.
    pub password: String,
    /// IMAP endpoint.
    pub imap: Endpoint,
    /// SMTP endpoint.
    pub smtp: Endpoint,
}

impl std::fmt::Debug for ImapConnect {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ImapConnect")
            .field("imap", &self.imap)
            .field("smtp", &self.smtp)
            .field("password", &"<redacted>")
            .finish_non_exhaustive()
    }
}

/// `POST /v1/connect/imap` 201 response.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImapConnectResponse {
    /// New account.
    pub account_id: AccountId,
}

/// `AttachmentMeta`.
#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AttachmentMeta {
    /// Part id (opaque to clients; IMAP-style section path from `crates/mime`).
    pub part_id: String,
    /// File name.
    pub filename: String,
    /// MIME type.
    pub mime_type: String,
    /// Decoded size in bytes.
    pub size: u64,
    /// Inline (referenced by `cid:`).
    pub inline: bool,
    /// `Content-ID` without angle brackets.
    pub content_id: Option<String>,
}

/// Quote-collapse offsets in the sanitized HTML and the text.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QuotedOffsets {
    /// Byte offset in `html` where quoted history starts.
    pub html_start: Option<u64>,
    /// Byte offset in `text` where quoted history starts.
    pub text_start: Option<u64>,
}

/// Full header lists that Convex caps (`MessageBody.headers`).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MessageBodyHeaders {
    /// To.
    pub to: Vec<Addr>,
    /// Cc.
    pub cc: Vec<Addr>,
    /// Reply-To.
    pub reply_to: Vec<Addr>,
    /// Unsubscribe options, or `null`.
    pub list_unsubscribe: Option<ListUnsubscribe>,
    /// Authentication summary, or `null`.
    pub auth: Option<AuthSummary>,
}

/// `MessageBody`: a sanitized message body. `null` fields are sent as `null`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MessageBody {
    /// Message.
    pub message_id: MessageId,
    /// Provider message id.
    pub provider_message_id: String,
    /// Bumping it invalidates device caches.
    pub sanitizer_version: u32,
    /// Sanitized HTML (§14.3): remote images as `data-akatsuki-src`, cid as `data-akatsuki-cid`.
    pub html: Option<String>,
    /// Plain text.
    pub text: Option<String>,
    /// Quote-collapse offsets.
    pub quoted: QuotedOffsets,
    /// Attachments.
    pub attachments: Vec<AttachmentMeta>,
    /// Remote images found (blocked or proxied).
    pub remote_image_count: u32,
    /// Tracking pixels removed.
    pub trackers_removed: u32,
    /// Full header lists.
    pub headers: MessageBodyHeaders,
}

/// `GET /v1/messages/{id}?format=`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BodyFormat {
    /// Sanitized `MessageBody` (default).
    #[default]
    Sanitized,
    /// `message/rfc822` (desktop only).
    Raw,
}

/// Query of `GET /v1/messages/{messageId}`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct BodyQuery {
    /// Format.
    #[serde(default)]
    pub format: BodyFormat,
}

/// `POST /v1/prefetch` request.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PrefetchRequest {
    /// Messages (≤ 100).
    pub message_ids: Vec<MessageId>,
    /// Format.
    pub format: BodyFormat,
}

/// One NDJSON line of a raw prefetch.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RawMessageLine {
    /// Message.
    pub message_id: MessageId,
    /// The RFC 822 bytes, base64.
    pub raw_base64: String,
}

/// `POST /v1/links` request.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LinkRequest {
    /// Message.
    pub message_id: MessageId,
    /// Part.
    pub part_id: String,
}

/// `POST /v1/links` response.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LinkResponse {
    /// `/v1/l/<token>`.
    pub url: String,
    /// Expiry (5 minutes).
    pub expires_at: UnixMs,
}

/// `POST /v1/uploads` 201 response.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UploadResponse {
    /// Upload id.
    pub upload_id: String,
    /// Size in bytes.
    pub size: u64,
    /// Hex sha256.
    pub sha256: String,
}

/// An attachment of a draft: an upload or a part of an existing message.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum DraftAttachment {
    /// From `POST /v1/uploads`.
    #[serde(rename_all = "camelCase")]
    Upload {
        /// Upload id.
        upload_id: String,
    },
    /// Forwarded from a message.
    #[serde(rename_all = "camelCase")]
    FromMessage {
        /// Message.
        from_message_id: MessageId,
        /// Part.
        part_id: String,
    },
}

/// `DraftContent`.
#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DraftContent {
    /// Account.
    pub account_id: AccountId,
    /// One of the account's identities.
    pub identity_email: String,
    /// To.
    pub to: Vec<Addr>,
    /// Cc.
    pub cc: Vec<Addr>,
    /// Bcc.
    pub bcc: Vec<Addr>,
    /// Subject.
    pub subject: String,
    /// Composed HTML (`null` for plain text); the worker re-sanitizes.
    #[serialize_always]
    pub html: Option<String>,
    /// Plain-text alternative, always present.
    pub text: String,
    /// Sets In-Reply-To/References/threading.
    pub in_reply_to_message_id: Option<MessageId>,
    /// Forwarded message.
    pub forward_of_message_id: Option<MessageId>,
    /// Attachments.
    pub attachments: Vec<DraftAttachment>,
    /// Honoured only if `prefs.readReceipts.enabled`.
    pub read_receipt: Option<bool>,
}

/// `PUT /v1/drafts/{clientDraftId}` request.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DraftPut {
    /// 0 = new.
    pub base_version: u64,
    /// Content.
    pub draft: DraftContent,
    /// Device.
    pub device_id: String,
}

/// `PUT /v1/drafts/{clientDraftId}` 200 response.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DraftSaved {
    /// Draft row.
    pub draft_id: DraftId,
    /// New version.
    pub version: u64,
    /// Provider draft id.
    pub provider_draft_id: String,
    /// Fixed `Message-ID`.
    pub rfc_message_id: String,
}

/// `POST /v1/drafts/{clientDraftId}/adopt` request.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AdoptRequest {
    /// The provider-native draft message.
    pub message_id: MessageId,
}

/// `POST /v1/drafts/{clientDraftId}/adopt` response.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AdoptResponse {
    /// Content.
    pub draft: DraftContent,
    /// Version.
    pub version: u64,
}

/// `POST /v1/send` request.
#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SendRequest {
    /// Idempotency key.
    pub client_send_id: String,
    /// Draft.
    pub client_draft_id: String,
    /// CAS base.
    pub base_version: u64,
    /// Final content; saved before scheduling.
    pub draft: Option<DraftContent>,
    /// Absent = now + undoSeconds.
    pub send_at: Option<UnixMs>,
    /// `prefs.undoSendSeconds`.
    pub undo_seconds: u32,
    /// Device.
    pub device_id: String,
}

/// `POST /v1/send` 202 response.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SendAccepted {
    /// Scheduled send row.
    pub send_id: ScheduledSendId,
    /// When it will send.
    pub send_at: UnixMs,
    /// State.
    pub state: ScheduleOutcome,
}

/// Query of `GET /v1/search`.
#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchQuery {
    /// Account.
    pub account_id: AccountId,
    /// Query in the search grammar (§11.1).
    pub q: String,
    /// Opaque cursor.
    pub cursor: Option<String>,
}

/// `GET /v1/search` response.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchResponse {
    /// Matching threads (upserted into Convex first).
    pub thread_ids: Vec<ThreadId>,
    /// Next cursor, or `null`.
    pub next_cursor: Option<String>,
}

/// Query of `GET /oauth/{provider}/callback`.
#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct OAuthCallbackQuery {
    /// Authorization code.
    pub code: Option<String>,
    /// Sealed state.
    pub state: Option<String>,
    /// Provider error.
    pub error: Option<String>,
}

/// `POST /hooks/gmail`: Pub/Sub push envelope.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PubSubPush {
    /// The message.
    pub message: PubSubMessage,
    /// Subscription name.
    #[serde(default)]
    pub subscription: String,
}

/// A Pub/Sub message.
#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PubSubMessage {
    /// Base64 of [`GmailPushData`] JSON.
    pub data: String,
    /// Message id.
    pub message_id: Option<String>,
    /// Publish time.
    pub publish_time: Option<String>,
}

/// Decoded Gmail push data.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GmailPushData {
    /// Mailbox address.
    pub email_address: String,
    /// New history id (a JSON number from Google; may exceed 2^53, kept as u64).
    pub history_id: u64,
}

/// Query of `POST /hooks/graph` during subscription validation.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GraphValidationQuery {
    /// Echo this as `text/plain` within 10 s.
    pub validation_token: Option<String>,
}

/// A Graph change or lifecycle notification batch.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct GraphNotificationBatch {
    /// Notifications.
    pub value: Vec<GraphNotification>,
}

/// One Graph notification.
#[skip_serializing_none]
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GraphNotification {
    /// Subscription id.
    pub subscription_id: String,
    /// The secret sent at subscription time.
    pub client_state: Option<String>,
    /// `created`, `updated`, `deleted`.
    pub change_type: Option<String>,
    /// Resource path.
    pub resource: Option<String>,
    /// `reauthorizationRequired`, `subscriptionRemoved`, `missed`.
    pub lifecycle_event: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn message_body_sends_nulls() {
        let body = MessageBody {
            message_id: "m1".into(),
            provider_message_id: "p1".into(),
            sanitizer_version: 1,
            html: None,
            text: Some("hi".into()),
            quoted: QuotedOffsets::default(),
            attachments: vec![],
            remote_image_count: 0,
            trackers_removed: 0,
            headers: MessageBodyHeaders::default(),
        };
        let v = serde_json::to_value(&body).unwrap();
        assert_eq!(v["html"], json!(null));
        assert_eq!(v["quoted"], json!({"htmlStart": null, "textStart": null}));
        assert_eq!(v["headers"]["listUnsubscribe"], json!(null));
    }

    #[test]
    fn draft_content_html_null_is_explicit() {
        let wire = json!({
            "accountId": "a", "identityEmail": "me@x.com", "to": [{"email": "y@x.com"}], "cc": [], "bcc": [],
            "subject": "S", "html": null, "text": "T",
            "attachments": [{"uploadId": "u1"}, {"fromMessageId": "m1", "partId": "2"}]
        });
        let d: DraftContent = serde_json::from_value(wire.clone()).unwrap();
        assert_eq!(
            d.attachments[1],
            DraftAttachment::FromMessage { from_message_id: "m1".into(), part_id: "2".into() }
        );
        assert_eq!(serde_json::to_value(&d).unwrap(), wire);
    }

    #[test]
    fn imap_connect_debug_redacts_password() {
        let c = ImapConnect {
            email: "a@b.c".into(),
            username: "a".into(),
            password: "hunter2".into(),
            imap: Endpoint { host: "imap.b.c".into(), port: 993, tls: TlsMode::Implicit },
            smtp: Endpoint { host: "smtp.b.c".into(), port: 465, tls: TlsMode::Implicit },
        };
        assert!(!format!("{c:?}").contains("hunter2"));
    }

    #[test]
    fn send_accepted_shape() {
        let s = SendAccepted { send_id: "s".into(), send_at: 5, state: ScheduleOutcome::Scheduled };
        assert_eq!(serde_json::to_value(&s).unwrap(), json!({"sendId": "s", "sendAt": 5, "state": "scheduled"}));
    }
}
