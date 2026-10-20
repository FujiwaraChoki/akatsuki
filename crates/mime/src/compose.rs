//! Building outgoing messages (ARCHITECTURE §7.3). `compose` is the only MIME builder in
//! Akatsuki: the worker builds every draft and send from `DraftContent` with it.
//!
//! It handles RFC 2047 encoding of non-ASCII headers, `In-Reply-To` / `References`, the fixed
//! `Message-ID`, `multipart/alternative` text + HTML, `multipart/mixed` attachments, inline `cid:`
//! parts, the identity's signature and (only when the user opted in) a read-receipt pixel. The
//! HTML is sanitized again before it is sent.

use akatsuki_core::model::Addr;
use mail_builder::MessageBuilder;
use mail_builder::headers::address::Address;
use mail_builder::headers::raw::Raw;

use crate::MimeError;
use crate::sanitize::sanitize_outgoing_html;

/// Total attachment cap for an outgoing message (§1.6).
pub const MAX_ATTACHMENTS_BYTES: usize = 25 * 1024 * 1024;

/// One attachment of an outgoing message.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ComposeAttachment {
    /// File name.
    pub filename: String,
    /// `type/subtype`.
    pub mime_type: String,
    /// Bytes.
    pub content: Vec<u8>,
    /// Set for inline parts referenced as `cid:<content_id>` from the HTML.
    pub inline_content_id: Option<String>,
}

/// Everything `compose` needs.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ComposeInput {
    /// The identity sending.
    pub from: Addr,
    /// To.
    pub to: Vec<Addr>,
    /// Cc.
    pub cc: Vec<Addr>,
    /// Bcc.
    pub bcc: Vec<Addr>,
    /// Reply-To.
    pub reply_to: Vec<Addr>,
    /// Subject.
    pub subject: String,
    /// Plain-text body (always present).
    pub text: String,
    /// HTML body, if the composer produced one (sanitized again here).
    pub html: Option<String>,
    /// Fixed `Message-ID` without brackets (`drafts.rfcMessageId`).
    pub message_id: String,
    /// `In-Reply-To`, without brackets.
    pub in_reply_to: Option<String>,
    /// `References`, oldest first, without brackets.
    pub references: Vec<String>,
    /// `Date` (ms).
    pub date_ms: i64,
    /// Signature HTML (sanitized here).
    pub signature_html: Option<String>,
    /// Signature text.
    pub signature_text: Option<String>,
    /// Read-receipt pixel URL (`https://sync.akatsuki.vc/r/<token>.gif`), only when opted in.
    pub read_receipt_url: Option<String>,
    /// Attachments.
    pub attachments: Vec<ComposeAttachment>,
    /// Include a `Bcc:` header (Gmail/Graph drafts need it; SMTP submission must not have it).
    pub include_bcc_header: bool,
}

fn check_addr(a: &Addr) -> Result<(), MimeError> {
    let e = &a.email;
    let ok = e.len() <= 254
        && e.matches('@').count() == 1
        && !e.starts_with('@')
        && !e.ends_with('@')
        && !e
            .chars()
            .any(|c| c.is_whitespace() || c.is_control() || matches!(c, '<' | '>' | ',' | ';' | '"' | '(' | ')'));
    if !ok {
        return Err(MimeError::InvalidAddress(e.clone()));
    }
    if a.name.as_deref().is_some_and(|n| n.chars().any(char::is_control)) {
        return Err(MimeError::InvalidHeader("display name".into()));
    }
    Ok(())
}

fn check_id(id: &str) -> Result<(), MimeError> {
    if id.is_empty()
        || id.len() > 250
        || id.chars().any(|c| c.is_whitespace() || c.is_control() || c == '<' || c == '>')
        || !id.contains('@')
    {
        return Err(MimeError::InvalidHeader("message id".into()));
    }
    Ok(())
}

fn address(a: &Addr) -> Address<'static> {
    Address::new_address(a.name.clone(), a.email.clone())
}

fn address_list(list: &[Addr]) -> Address<'static> {
    Address::new_list(list.iter().map(address).collect())
}

/// Builds `<uuidv7@domain>` (returned without brackets) for a new draft.
pub fn mint_message_id(domain: &str) -> String {
    format!("{}@{}", uuid::Uuid::now_v7(), domain.trim().trim_start_matches('@'))
}

fn escape_html_attr(s: &str) -> String {
    s.replace('&', "&amp;").replace('"', "&quot;").replace('<', "&lt;").replace('>', "&gt;")
}

/// Builds the RFC 5322 bytes of a message.
pub fn compose(input: &ComposeInput) -> Result<Vec<u8>, MimeError> {
    check_addr(&input.from)?;
    for a in input.to.iter().chain(&input.cc).chain(&input.bcc).chain(&input.reply_to) {
        check_addr(a)?;
    }
    if input.to.is_empty() && input.cc.is_empty() && input.bcc.is_empty() {
        return Err(MimeError::InvalidHeader("no recipients".into()));
    }
    check_id(&input.message_id)?;
    if let Some(id) = &input.in_reply_to {
        check_id(id)?;
    }
    for id in &input.references {
        check_id(id)?;
    }
    if input.subject.chars().any(|c| c == '\r' || c == '\n') {
        return Err(MimeError::InvalidHeader("subject".into()));
    }
    let total: usize = input.attachments.iter().map(|a| a.content.len()).sum();
    if total > MAX_ATTACHMENTS_BYTES {
        return Err(MimeError::TooLarge);
    }

    let mut text = input.text.clone();
    if let Some(sig) = input.signature_text.as_deref().filter(|s| !s.trim().is_empty()) {
        text.push_str("\n\n-- \n");
        text.push_str(sig);
    }

    let html = input.html.as_deref().map(|h| {
        let mut html = sanitize_outgoing_html(h);
        if let Some(sig) = input.signature_html.as_deref().filter(|s| !s.trim().is_empty()) {
            html.push_str("<div class=\"akatsuki-signature\">");
            html.push_str(&sanitize_outgoing_html(sig));
            html.push_str("</div>");
        }
        if let Some(url) = &input.read_receipt_url {
            html.push_str(&format!("<img src=\"{}\" width=\"1\" height=\"1\" alt=\"\">", escape_html_attr(url)));
        }
        html
    });

    let mut b = MessageBuilder::new()
        .from(address(&input.from))
        .subject(input.subject.clone())
        .message_id(input.message_id.clone())
        .date(input.date_ms / 1000)
        .header("MIME-Version", Raw::new("1.0"))
        .text_body(text);
    if !input.to.is_empty() {
        b = b.to(address_list(&input.to));
    }
    if !input.cc.is_empty() {
        b = b.cc(address_list(&input.cc));
    }
    if input.include_bcc_header && !input.bcc.is_empty() {
        b = b.bcc(address_list(&input.bcc));
    }
    if !input.reply_to.is_empty() {
        b = b.reply_to(address_list(&input.reply_to));
    }
    if let Some(id) = &input.in_reply_to {
        b = b.in_reply_to(id.clone());
    }
    if !input.references.is_empty() {
        b = b.references(input.references.clone());
    }
    if let Some(html) = html {
        b = b.html_body(html);
    }
    for a in &input.attachments {
        let content = a.content.clone();
        b = match &a.inline_content_id {
            Some(cid) => b.inline(a.mime_type.clone(), cid.clone(), content),
            None => b.attachment(a.mime_type.clone(), a.filename.clone(), content),
        };
    }
    b.write_to_vec().map_err(|e| MimeError::Build(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse::parse_message;

    fn input() -> ComposeInput {
        ComposeInput {
            from: Addr::new(Some("Sami Hindi"), "me@akatsuki.vc"),
            to: vec![Addr::new(Some("Zoë Ünïcode"), "zoe@example.com")],
            cc: vec![Addr::new(None, "cc@example.com")],
            bcc: vec![Addr::new(None, "hidden@example.com")],
            subject: "Grüße – テスト".into(),
            text: "Hello".into(),
            html: Some("<p onclick=\"x()\">Hello <script>alert(1)</script><a href=\"javascript:x\">x</a></p>".into()),
            message_id: "0192f0a0-0000-7000-8000-000000000000@akatsuki.vc".into(),
            in_reply_to: Some("parent@example.com".into()),
            references: vec!["root@example.com".into(), "parent@example.com".into()],
            date_ms: 1_792_540_800_000,
            signature_html: Some("<b>Sami</b><img src=x onerror=alert(1)>".into()),
            signature_text: Some("Sami".into()),
            ..Default::default()
        }
    }

    #[test]
    fn round_trips_through_the_parser() {
        let mut i = input();
        i.attachments.push(ComposeAttachment {
            filename: "report.pdf".into(),
            mime_type: "application/pdf".into(),
            content: b"%PDF-1.7 fake".to_vec(),
            inline_content_id: None,
        });
        let raw = compose(&i).unwrap();
        let s = String::from_utf8_lossy(&raw);
        assert!(s.contains("Message-ID: <0192f0a0-0000-7000-8000-000000000000@akatsuki.vc>"));
        assert!(s.contains("In-Reply-To: <parent@example.com>"));
        assert!(!s.contains("hidden@example.com"), "no Bcc header unless asked");
        assert!(!s.contains("Grüße"), "non-ASCII subject must be RFC 2047 encoded");

        let p = parse_message(&raw).unwrap();
        assert_eq!(p.subject, "Grüße – テスト");
        assert_eq!(p.to[0].name.as_deref(), Some("Zoë Ünïcode"));
        assert_eq!(p.references, vec!["root@example.com", "parent@example.com"]);
        assert_eq!(p.rfc_message_id.as_deref(), Some("0192f0a0-0000-7000-8000-000000000000@akatsuki.vc"));
        assert!(p.text.unwrap().contains("-- \nSami"));
        let html = p.html.unwrap();
        assert!(
            !html.contains("script")
                && !html.contains("onclick")
                && !html.contains("javascript")
                && !html.contains("onerror")
        );
        assert!(html.contains("<b>Sami</b>"));
        assert_eq!(p.attachments.len(), 1);
        assert_eq!(p.attachments[0].filename, "report.pdf");
        assert_eq!(p.date, Some(1_792_540_800_000));
    }

    #[test]
    fn bcc_header_on_request_and_read_receipt() {
        let mut i = input();
        i.include_bcc_header = true;
        i.read_receipt_url = Some("https://sync.akatsuki.vc/r/abc.gif".into());
        let raw = compose(&i).unwrap();
        let p = parse_message(&raw).unwrap();
        assert_eq!(p.bcc[0].email, "hidden@example.com");
        assert!(p.html.unwrap().contains("https://sync.akatsuki.vc/r/abc.gif"));
    }

    #[test]
    fn rejects_header_injection() {
        let mut i = input();
        i.subject = "Hi\r\nBcc: victim@example.com".into();
        assert!(matches!(compose(&i), Err(MimeError::InvalidHeader(_))));
        let mut i = input();
        i.to = vec![Addr { name: None, email: "a@b.com\r\nBcc: x@y.z".into() }];
        assert!(matches!(compose(&i), Err(MimeError::InvalidAddress(_))));
        let mut i = input();
        i.message_id = "x> <evil@y".into();
        assert!(compose(&i).is_err());
        let mut i = input();
        i.to.clear();
        i.cc.clear();
        i.bcc.clear();
        assert!(compose(&i).is_err());
    }

    #[test]
    fn size_cap() {
        let mut i = input();
        i.attachments.push(ComposeAttachment {
            filename: "big.bin".into(),
            mime_type: "application/octet-stream".into(),
            content: vec![0; MAX_ATTACHMENTS_BYTES + 1],
            inline_content_id: None,
        });
        assert!(matches!(compose(&i), Err(MimeError::TooLarge)));
    }

    #[test]
    fn message_ids() {
        let id = mint_message_id("akatsuki.vc");
        assert!(id.ends_with("@akatsuki.vc"));
        assert!(check_id(&id).is_ok());
    }
}
