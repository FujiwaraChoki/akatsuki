//! Assembling the worker's `MessageBody` (ARCHITECTURE §7.3) from a raw message.

use akatsuki_core::ids::MessageId;
use akatsuki_core::wire::worker::{MessageBody, MessageBodyHeaders, QuotedOffsets};

use crate::MimeError;
use crate::parse::{ParsedMessage, parse_message};
use crate::quote::text_quote_start;
use crate::sanitize::{RemoteImagePolicy, SANITIZER_VERSION, SanitizeOptions, html_to_text, sanitize_html};

/// Builds a `MessageBody` from a parsed message.
pub fn message_body(
    parsed: &ParsedMessage,
    message_id: MessageId,
    provider_message_id: &str,
    remote_images: RemoteImagePolicy,
) -> MessageBody {
    let opts = SanitizeOptions { remote_images, cid_map: parsed.cid_map.clone() };
    let sanitized = parsed.html.as_deref().map(|h| sanitize_html(h, &opts));
    let text = parsed.text.clone().or_else(|| parsed.html.as_deref().map(html_to_text));
    let quoted = QuotedOffsets {
        html_start: sanitized.as_ref().and_then(|s| s.quote_html_start).map(|o| o as u64),
        text_start: text.as_deref().and_then(text_quote_start).map(|o| o as u64),
    };
    MessageBody {
        message_id,
        provider_message_id: provider_message_id.to_owned(),
        sanitizer_version: SANITIZER_VERSION,
        remote_image_count: sanitized.as_ref().map_or(0, |s| s.remote_image_count),
        trackers_removed: sanitized.as_ref().map_or(0, |s| s.trackers_removed),
        html: sanitized.map(|s| s.html),
        text,
        quoted,
        attachments: parsed.attachments.clone(),
        headers: MessageBodyHeaders {
            to: parsed.to.clone(),
            cc: parsed.cc.clone(),
            reply_to: parsed.reply_to.clone(),
            list_unsubscribe: parsed.list_unsubscribe.clone(),
            auth: parsed.auth.clone(),
        },
    }
}

/// Parses raw bytes and builds the `MessageBody` in one step.
pub fn message_body_from_raw(
    raw: &[u8],
    message_id: MessageId,
    provider_message_id: &str,
    remote_images: RemoteImagePolicy,
) -> Result<MessageBody, MimeError> {
    Ok(message_body(&parse_message(raw)?, message_id, provider_message_id, remote_images))
}

/// A snippet (≤ 200 chars, whitespace collapsed, quoted history removed) for providers that do not
/// supply one (IMAP).
pub fn snippet(parsed: &ParsedMessage) -> String {
    let text = parsed.text.clone().or_else(|| parsed.html.as_deref().map(html_to_text)).unwrap_or_default();
    let text = match text_quote_start(&text) {
        Some(at) => text[..at].to_owned(),
        None => text,
    };
    let collapsed = text.split_whitespace().collect::<Vec<_>>().join(" ");
    collapsed.chars().take(200).collect()
}
