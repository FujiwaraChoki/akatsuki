//! Gmail resources → provider-neutral types (`RemoteLabel`, `RemoteMessage`, `FetchedThread`).
//!
//! Header values (`format=metadata`) are parsed by `crates/mime` from a synthetic header block, so
//! addresses, encoded words, `References`, `List-Unsubscribe` and `Authentication-Results` follow
//! exactly the same rules as bodies parsed from raw source.

use base64::Engine as _;

use super::api::{Label, Message, MessagePart, Thread};
use crate::labels::{gmail_label, gmail_system_label};
use crate::{FetchedThread, RemoteLabel, RemoteMessage, SysLabel};

/// Headers requested with `format=metadata` (§9.3).
pub const METADATA_HEADERS: [&str; 16] = [
    "From",
    "To",
    "Cc",
    "Bcc",
    "Reply-To",
    "Subject",
    "Date",
    "Message-ID",
    "In-Reply-To",
    "References",
    "List-Id",
    "List-Unsubscribe",
    "List-Unsubscribe-Post",
    "Precedence",
    "Content-Type",
    "Authentication-Results",
];

/// Decodes Gmail's base64url (padding optional; the standard alphabet is accepted too).
pub fn decode_b64(data: &str) -> Result<Vec<u8>, base64::DecodeError> {
    let cleaned: String = data.chars().filter(|c| !c.is_whitespace()).collect();
    base64::engine::general_purpose::URL_SAFE_PAD_INDIFFERENT
        .decode(cleaned.as_bytes())
        .or_else(|_| base64::engine::general_purpose::STANDARD_PAD_INDIFFERENT.decode(cleaned.as_bytes()))
}

/// Encodes bytes for `raw` fields (base64url, no padding).
pub fn encode_b64(bytes: &[u8]) -> String {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

/// A Gmail label as a `RemoteLabel` (`Akatsuki/Snoozed` gets the `snoozed` role).
pub fn remote_label(l: &Label) -> RemoteLabel {
    let (kind, role) = gmail_label(&l.id, l.label_type.as_deref().unwrap_or("user"), &l.name);
    RemoteLabel {
        provider_label_id: l.id.clone(),
        name: l.name.clone(),
        kind,
        role,
        exclusive: false,
        color: l.color.as_ref().and_then(|c| c.background_color.clone()),
        hidden: l.label_list_visibility.as_deref() == Some("labelHide"),
    }
}

/// Decodes the HTML entities Gmail leaves in snippets (`&amp;`, `&#39;`, `&quot;`, numeric).
pub fn decode_entities(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find('&') {
        out.push_str(&rest[..i]);
        rest = &rest[i..];
        let Some(end) = rest[..rest.len().min(12)].find(';') else {
            out.push('&');
            rest = &rest[1..];
            continue;
        };
        let ent = &rest[1..end];
        let decoded = match ent {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            "nbsp" => Some('\u{a0}'),
            _ => {
                let n = if let Some(h) = ent.strip_prefix("#x").or_else(|| ent.strip_prefix("#X")) {
                    u32::from_str_radix(h, 16).ok()
                } else if let Some(d) = ent.strip_prefix('#') {
                    d.parse().ok()
                } else {
                    None
                };
                n.and_then(char::from_u32)
            }
        };
        match decoded {
            Some(c) => {
                out.push(c);
                rest = &rest[end + 1..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

fn header_block(part: &MessagePart) -> Vec<u8> {
    let mut b = String::new();
    for h in &part.headers {
        if h.name.is_empty() || h.name.contains(':') || h.name.chars().any(|c| c.is_whitespace() || c.is_control()) {
            continue;
        }
        let value: String = h.value.chars().map(|c| if c == '\r' || c == '\n' { ' ' } else { c }).collect();
        b.push_str(&h.name);
        b.push_str(": ");
        b.push_str(&value);
        b.push_str("\r\n");
    }
    b.push_str("\r\n");
    b.into_bytes()
}

fn walk_has_attachment(p: &MessagePart, depth: u32) -> bool {
    if depth > 32 {
        return false;
    }
    let is_file = !p.filename.is_empty()
        || p.body.as_ref().is_some_and(|b| b.attachment_id.is_some() && !p.mime_type.starts_with("text/"));
    is_file || p.parts.iter().any(|c| walk_has_attachment(c, depth + 1))
}

/// Best-effort attachment flag: `multipart/mixed` at the top (metadata), or any part with a file
/// name or attachment id (full).
pub fn has_attachments(payload: Option<&MessagePart>) -> bool {
    let Some(p) = payload else { return false };
    let top = if p.mime_type.is_empty() {
        p.header("Content-Type").unwrap_or_default().to_owned()
    } else {
        p.mime_type.clone()
    };
    top.to_ascii_lowercase().starts_with("multipart/mixed") || walk_has_attachment(p, 0)
}

/// A Gmail message (format `metadata`, `full` or `minimal`) as a `RemoteMessage`.
/// `snoozed_label_id` is the id of the `Akatsuki/Snoozed` label, when known.
pub fn remote_message(m: &Message, snoozed_label_id: Option<&str>) -> RemoteMessage {
    let parsed = m
        .payload
        .as_ref()
        .filter(|p| !p.headers.is_empty())
        .and_then(|p| akatsuki_mime::parse_message(&header_block(p)).ok())
        .unwrap_or_default();
    let mut sys: Vec<SysLabel> = m.label_ids.iter().filter_map(|id| gmail_system_label(id)).collect();
    if snoozed_label_id.is_some_and(|s| m.label_ids.iter().any(|id| id == s)) {
        sys.push(SysLabel::Snoozed);
    }
    sys.sort();
    sys.dedup();
    let is_draft = m.label_ids.iter().any(|l| l == "DRAFT");
    let date = m.internal_date.or(parsed.date).unwrap_or(0);
    RemoteMessage {
        provider_message_id: m.id.clone(),
        provider_thread_id: m.thread_id.clone(),
        rfc_message_id: parsed.rfc_message_id,
        in_reply_to: parsed.in_reply_to,
        references: parsed.references,
        from: parsed.from,
        to: parsed.to,
        cc: parsed.cc,
        bcc: parsed.bcc,
        reply_to: parsed.reply_to,
        subject: parsed.subject,
        snippet: decode_entities(&m.snippet),
        date,
        provider_label_ids: m.label_ids.clone(),
        sys,
        is_draft,
        has_attachments: has_attachments(m.payload.as_ref()),
        size_bytes: m.size_estimate,
        list_id: parsed.list_id,
        list_unsubscribe: parsed.list_unsubscribe,
        auth: parsed.auth,
        imap: Vec::new(),
    }
}

/// A Gmail thread as a `FetchedThread`. The version is the thread's `historyId` (§9.5).
pub fn fetched_thread(t: &Thread, snoozed_label_id: Option<&str>) -> FetchedThread {
    let version = t
        .history_id
        .or_else(|| t.messages.iter().filter_map(|m| m.history_id).max())
        .and_then(|h| i64::try_from(h).ok());
    let thread_id = t.id.clone();
    let messages = t
        .messages
        .iter()
        .map(|m| {
            let mut r = remote_message(m, snoozed_label_id);
            if r.provider_thread_id.is_empty() {
                r.provider_thread_id.clone_from(&thread_id);
            }
            r
        })
        .collect();
    FetchedThread { provider_thread_id: thread_id, provider_version: version, messages, partial: false }
}

/// The Gmail label id for a system label. `Snoozed` has no fixed id (it is a user label).
pub fn system_label_id(s: SysLabel) -> Option<&'static str> {
    Some(match s {
        SysLabel::Inbox => "INBOX",
        SysLabel::Sent => "SENT",
        SysLabel::Drafts => "DRAFT",
        SysLabel::Trash => "TRASH",
        SysLabel::Spam => "SPAM",
        SysLabel::Starred => "STARRED",
        SysLabel::Important => "IMPORTANT",
        SysLabel::Unread => "UNREAD",
        SysLabel::Snoozed => return None,
    })
}

/// Maps an IMAP body-section path (`"1"`, `"2.1"`, D-095) to the Gmail part it names. Returns
/// `None` when the structure does not contain it (the caller falls back to the raw source).
pub fn part_by_section<'a>(root: &'a MessagePart, section: &str) -> Option<&'a MessagePart> {
    let mut indices = Vec::new();
    for s in section.split('.') {
        let n: usize = s.parse().ok()?;
        indices.push(n.checked_sub(1)?);
    }
    if root.parts.is_empty() {
        // A single-part message: its body is section "1".
        return (indices == [0]).then_some(root);
    }
    let mut cur = root;
    for i in indices {
        if !cur.mime_type.to_ascii_lowercase().starts_with("multipart/") && !cur.parts.is_empty() {
            // message/rfc822 children: IMAP numbering differs; let the raw path handle it.
            return None;
        }
        cur = cur.parts.get(i)?;
    }
    cur.parts.is_empty().then_some(cur)
}
