//! RFC 5322 / MIME parsing with `mail-parser` (charsets, encoded words, broken structure).
//!
//! Part ids are IMAP body-section paths (`"1"`, `"1.2"`, `"2"`), so the IMAP adapter can fetch a
//! single part with `BODY.PEEK[<partId>]` and every other code path can find the same part in the
//! raw source with [`part_content`].

use std::collections::HashMap;

use akatsuki_core::model::{Addr, AuthSummary, ListUnsubscribe};
use akatsuki_core::wire::worker::AttachmentMeta;
use mail_parser::{Address, HeaderValue, Message, MessageParser, MimeHeaders, PartType};

use crate::MimeError;

/// A parsed message: headers, the chosen bodies and the attachment list.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ParsedMessage {
    /// `Message-ID` without angle brackets.
    pub rfc_message_id: Option<String>,
    /// `In-Reply-To` (first id).
    pub in_reply_to: Option<String>,
    /// `References`, oldest first.
    pub references: Vec<String>,
    /// Sender.
    pub from: Option<Addr>,
    /// To (group members flattened).
    pub to: Vec<Addr>,
    /// Cc.
    pub cc: Vec<Addr>,
    /// Bcc.
    pub bcc: Vec<Addr>,
    /// Reply-To.
    pub reply_to: Vec<Addr>,
    /// Decoded subject.
    pub subject: String,
    /// `Date` in ms, if present and valid.
    pub date: Option<i64>,
    /// `List-Id` (the part inside angle brackets).
    pub list_id: Option<String>,
    /// `List-Unsubscribe` / `List-Unsubscribe-Post`.
    pub list_unsubscribe: Option<ListUnsubscribe>,
    /// Bulk mail: `List-Id`, `List-Unsubscribe` or `Precedence: bulk|list|junk`.
    pub is_bulk: bool,
    /// `Authentication-Results` verdicts.
    pub auth: Option<AuthSummary>,
    /// The HTML body (all `text/html` body parts, concatenated), unsanitized.
    pub html: Option<String>,
    /// The plain-text body (all `text/plain` body parts, concatenated).
    pub text: Option<String>,
    /// Attachments and inline parts.
    pub attachments: Vec<AttachmentMeta>,
    /// `Content-ID` (lowercased, no brackets) → part id.
    pub cid_map: HashMap<String, String>,
    /// Raw size in bytes.
    pub size_bytes: u64,
    /// The parser reported an encoding problem in some part (body may be partial).
    pub had_encoding_problem: bool,
}

/// A part's decoded bytes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PartContent {
    /// Decoded bytes (text parts are UTF-8).
    pub bytes: Vec<u8>,
    /// `type/subtype`.
    pub mime_type: String,
    /// File name, if any.
    pub filename: Option<String>,
}

fn addr_list(a: Option<&Address<'_>>) -> Vec<Addr> {
    let Some(a) = a else { return Vec::new() };
    a.iter().filter_map(|x| x.address().map(|e| Addr::new(x.name(), e))).filter(|x| !x.email.is_empty()).collect()
}

fn clean_id(id: &str) -> Option<String> {
    let id = id.trim().trim_start_matches('<').trim_end_matches('>').trim();
    (!id.is_empty()).then(|| id.to_owned())
}

fn ids(v: &HeaderValue<'_>) -> Vec<String> {
    match v {
        HeaderValue::Text(t) => t.split_whitespace().filter_map(clean_id).collect(),
        HeaderValue::TextList(l) => l.iter().filter_map(|t| clean_id(t)).collect(),
        _ => Vec::new(),
    }
}

/// Assigns IMAP section paths to every part reachable from the root.
fn part_ids(msg: &Message<'_>) -> HashMap<u32, String> {
    let mut out = HashMap::new();
    let Some(root) = msg.parts.first() else { return out };
    match &root.body {
        PartType::Multipart(children) => walk(msg, children, "", &mut out, 0),
        _ => {
            out.insert(0, "1".to_owned());
        }
    }
    out
}

fn walk(msg: &Message<'_>, children: &[u32], prefix: &str, out: &mut HashMap<u32, String>, depth: u32) {
    if depth > 32 {
        return;
    }
    for (i, &child) in children.iter().enumerate() {
        let path = if prefix.is_empty() { format!("{}", i + 1) } else { format!("{prefix}.{}", i + 1) };
        if out.contains_key(&child) {
            continue; // malformed self-reference
        }
        out.insert(child, path.clone());
        if let Some(PartType::Multipart(grand)) = msg.parts.get(child as usize).map(|p| &p.body) {
            walk(msg, grand, &path, out, depth + 1);
        }
    }
}

fn header_raw<'a>(msg: &'a Message<'_>, name: &'a str) -> Option<&'a str> {
    msg.header_raw(name).map(str::trim).filter(|s| !s.is_empty())
}

fn parse_list_id(raw: &str) -> Option<String> {
    match (raw.rfind('<'), raw.rfind('>')) {
        (Some(a), Some(b)) if b > a + 1 => Some(raw[a + 1..b].trim().to_lowercase()),
        _ => Some(raw.trim().to_lowercase()).filter(|s| !s.is_empty()),
    }
}

/// Parses `List-Unsubscribe` (`<https://…>, <mailto:…>`) and `List-Unsubscribe-Post`.
pub fn parse_list_unsubscribe(header: &str, post: Option<&str>) -> Option<ListUnsubscribe> {
    let mut http = None;
    let mut mailto = None;
    let mut rest = header;
    while let Some(start) = rest.find('<') {
        let Some(len) = rest[start + 1..].find('>') else { break };
        let url: String = rest[start + 1..start + 1 + len].chars().filter(|c| !c.is_whitespace()).collect();
        let lower = url.to_ascii_lowercase();
        if lower.starts_with("https://") && http.is_none() {
            http = Some(url);
        } else if lower.starts_with("mailto:") && mailto.is_none() {
            mailto = Some(url);
        }
        rest = &rest[start + 1 + len + 1..];
    }
    let one_click = http.is_some()
        && post.is_some_and(|p| p.to_ascii_lowercase().replace(' ', "").contains("list-unsubscribe=one-click"));
    (http.is_some() || mailto.is_some()).then_some(ListUnsubscribe { http, mailto, one_click })
}

/// Extracts `spf=`, `dkim=` and `dmarc=` verdicts from `Authentication-Results`.
pub fn parse_auth_results(header: &str) -> Option<AuthSummary> {
    let verdict = |key: &str| -> Option<String> {
        let lower = header.to_ascii_lowercase();
        let mut from = 0;
        while let Some(pos) = lower[from..].find(key) {
            let at = from + pos;
            let boundary = at == 0 || !lower.as_bytes()[at - 1].is_ascii_alphanumeric();
            let after = &lower[at + key.len()..];
            if boundary && let Some(v) = after.strip_prefix('=') {
                let v: String = v.chars().take_while(|c| c.is_ascii_alphanumeric()).collect();
                if !v.is_empty() {
                    return Some(v);
                }
            }
            from = at + key.len();
        }
        None
    };
    let (spf, dkim, dmarc) = (verdict("spf"), verdict("dkim"), verdict("dmarc"));
    if spf.is_none() && dkim.is_none() && dmarc.is_none() {
        return None;
    }
    let none = || "none".to_owned();
    Some(AuthSummary {
        spf: spf.unwrap_or_else(none),
        dkim: dkim.unwrap_or_else(none),
        dmarc: dmarc.unwrap_or_else(none),
    })
}

fn mime_type_of(part: &mail_parser::MessagePart<'_>) -> String {
    match part.content_type() {
        Some(ct) => match ct.subtype() {
            Some(sub) => format!("{}/{}", ct.ctype(), sub).to_ascii_lowercase(),
            None => ct.ctype().to_ascii_lowercase(),
        },
        None => match &part.body {
            PartType::Text(_) => "text/plain".into(),
            PartType::Html(_) => "text/html".into(),
            PartType::Message(_) => "message/rfc822".into(),
            _ => "application/octet-stream".into(),
        },
    }
}

fn sanitize_filename(name: &str) -> String {
    let cleaned: String =
        name.chars()
            .map(|c| {
                if c.is_control() || matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|') {
                    '_'
                } else {
                    c
                }
            })
            .collect();
    let cleaned = cleaned.trim().trim_start_matches('.').trim().to_owned();
    if cleaned.is_empty() { "attachment".into() } else { cleaned.chars().take(255).collect() }
}

/// Parses a raw RFC 5322 message.
pub fn parse_message(raw: &[u8]) -> Result<ParsedMessage, MimeError> {
    let msg = MessageParser::default().parse(raw).ok_or(MimeError::Unparseable)?;
    let ids_by_part = part_ids(&msg);

    let html_parts: Vec<String> = msg
        .html_body
        .iter()
        .filter_map(|&i| match msg.parts.get(i as usize).map(|p| &p.body) {
            Some(PartType::Html(h)) => Some(h.trim_start_matches('\u{feff}').to_owned()),
            _ => None,
        })
        .collect();
    let text_parts: Vec<String> = msg
        .text_body
        .iter()
        .filter_map(|&i| match msg.parts.get(i as usize).map(|p| &p.body) {
            Some(PartType::Text(t)) => Some(t.trim_start_matches('\u{feff}').to_owned()),
            _ => None,
        })
        .collect();

    let mut attachments = Vec::new();
    let mut cid_map = HashMap::new();
    for &i in &msg.attachments {
        let Some(part) = msg.parts.get(i as usize) else { continue };
        let Some(part_id) = ids_by_part.get(&i).cloned() else { continue };
        let content_id = part.content_id().and_then(clean_id);
        let disposition_inline = part.content_disposition().is_some_and(|d| d.is_inline());
        let disposition_attachment = part.content_disposition().is_some_and(|d| d.is_attachment());
        let inline = (content_id.is_some() && !disposition_attachment) || disposition_inline;
        let mime_type = mime_type_of(part);
        let filename = part
            .attachment_name()
            .map(sanitize_filename)
            .or_else(|| match &part.body {
                PartType::Message(m) => m.subject().map(|s| sanitize_filename(&format!("{s}.eml"))),
                _ => None,
            })
            .unwrap_or_else(|| format!("part-{part_id}"));
        if let Some(cid) = &content_id {
            cid_map.insert(cid.to_lowercase(), part_id.clone());
        }
        attachments.push(AttachmentMeta { part_id, filename, mime_type, size: part.len() as u64, inline, content_id });
    }

    let list_id = header_raw(&msg, "List-Id").and_then(parse_list_id);
    let list_unsubscribe = header_raw(&msg, "List-Unsubscribe")
        .and_then(|h| parse_list_unsubscribe(h, header_raw(&msg, "List-Unsubscribe-Post")));
    let precedence = header_raw(&msg, "Precedence").map(str::to_ascii_lowercase);
    let is_bulk = list_id.is_some()
        || list_unsubscribe.is_some()
        || precedence.as_deref().is_some_and(|p| matches!(p, "bulk" | "list" | "junk"));

    Ok(ParsedMessage {
        rfc_message_id: msg.message_id().and_then(clean_id),
        in_reply_to: ids(msg.in_reply_to()).into_iter().next(),
        references: ids(msg.references()),
        from: addr_list(msg.from()).into_iter().next(),
        to: addr_list(msg.to()),
        cc: addr_list(msg.cc()),
        bcc: addr_list(msg.bcc()),
        reply_to: addr_list(msg.reply_to()),
        subject: msg.subject().unwrap_or_default().trim().to_owned(),
        date: msg.date().map(|d| d.to_timestamp().saturating_mul(1000)),
        list_id,
        list_unsubscribe,
        is_bulk,
        auth: header_raw(&msg, "Authentication-Results").and_then(parse_auth_results),
        html: (!html_parts.is_empty()).then(|| html_parts.join("\n")),
        text: (!text_parts.is_empty()).then(|| text_parts.join("\n")),
        attachments,
        cid_map,
        size_bytes: raw.len() as u64,
        had_encoding_problem: msg.parts.iter().any(|p| p.is_encoding_problem),
    })
}

/// Returns the decoded content of one part, by IMAP section path.
pub fn part_content(raw: &[u8], part_id: &str) -> Result<Option<PartContent>, MimeError> {
    let msg = MessageParser::default().parse(raw).ok_or(MimeError::Unparseable)?;
    let ids_by_part = part_ids(&msg);
    let Some((&index, _)) = ids_by_part.iter().find(|(_, id)| id.as_str() == part_id) else { return Ok(None) };
    let Some(part) = msg.parts.get(index as usize) else { return Ok(None) };
    let bytes = match &part.body {
        PartType::Text(t) | PartType::Html(t) => t.as_bytes().to_vec(),
        PartType::Binary(b) | PartType::InlineBinary(b) => b.to_vec(),
        PartType::Message(m) => m.raw_message().to_vec(),
        PartType::Multipart(_) => return Ok(None),
    };
    Ok(Some(PartContent {
        bytes,
        mime_type: mime_type_of(part),
        filename: part.attachment_name().map(sanitize_filename),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn list_unsubscribe() {
        let l = parse_list_unsubscribe(
            "<mailto:u@x.com?subject=unsub>, <https://x.com/u?id=1>",
            Some("List-Unsubscribe=One-Click"),
        )
        .unwrap();
        assert_eq!(l.http.as_deref(), Some("https://x.com/u?id=1"));
        assert_eq!(l.mailto.as_deref(), Some("mailto:u@x.com?subject=unsub"));
        assert!(l.one_click);
        let l = parse_list_unsubscribe("<http://insecure/u>, <mailto:a@b>", None).unwrap();
        assert_eq!(l.http, None, "plain http is not offered for one-click");
        assert!(!l.one_click);
        assert_eq!(parse_list_unsubscribe("garbage", None), None);
    }

    #[test]
    fn auth_results() {
        let a = parse_auth_results(
            "mx.google.com; dkim=pass header.i=@x.com; spf=softfail smtp.mailfrom=y; dmarc=fail (p=NONE)",
        )
        .unwrap();
        assert_eq!((a.spf.as_str(), a.dkim.as_str(), a.dmarc.as_str()), ("softfail", "pass", "fail"));
        assert_eq!(parse_auth_results("mx; none"), None);
        let a = parse_auth_results("mx; arc=pass; dkim=pass").unwrap();
        assert_eq!(a.spf, "none");
    }

    #[test]
    fn filenames_are_safe() {
        assert_eq!(sanitize_filename("../../etc/passwd"), "_.._etc_passwd");
        assert_eq!(sanitize_filename("a\r\nb.txt"), "a__b.txt");
        assert_eq!(sanitize_filename("..."), "attachment");
    }
}
