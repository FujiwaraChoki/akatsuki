//! A message body from its raw RFC 822 bytes: parsed and sanitized locally with `crates/mime`
//! (the same sanitizer version as the worker, §14.3), laid out for the native renderer, and kept
//! as sanitized HTML for "Open original".

use std::collections::HashMap;

use akatsuki_core::model::ListUnsubscribe;
use akatsuki_core::wire::worker::AttachmentMeta;
use akatsuki_mime::{
    MimeError, RemoteImagePolicy, SANITIZER_VERSION, SanitizeOptions, html_to_text, parse_message, part_content,
    sanitize_html,
};

use super::body::{Layout, layout_html, layout_text};

/// Inline (`cid:`) images up to this size are shown.
pub const MAX_INLINE_IMAGE_BYTES: u64 = 2 * 1024 * 1024;

/// A rendered body.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Rendered {
    /// Sanitized HTML, when the message has an HTML part.
    pub html: Option<String>,
    /// Plain text (the text part, or text extracted from the HTML).
    pub text: Option<String>,
    /// Native layout.
    pub layout: Layout,
    /// Attachments (not inline images).
    pub attachments: Vec<AttachmentMeta>,
    /// Remote images that are blocked.
    pub remote_images: u32,
    /// Tracking pixels removed.
    pub trackers: u32,
    /// Inline images: part id → (MIME type, bytes).
    pub inline_images: HashMap<String, (String, Vec<u8>)>,
    /// `List-Unsubscribe`.
    pub list_unsubscribe: Option<ListUnsubscribe>,
    /// [`SANITIZER_VERSION`] used.
    pub sanitizer_version: u32,
}

/// Parses, sanitizes and lays out a raw message.
pub fn render_raw(raw: &[u8]) -> Result<Rendered, MimeError> {
    let parsed = parse_message(raw)?;
    let opts = SanitizeOptions { remote_images: RemoteImagePolicy::Original, cid_map: parsed.cid_map.clone() };
    let sanitized = parsed.html.as_deref().map(|h| sanitize_html(h, &opts));
    let text = parsed.text.clone().or_else(|| parsed.html.as_deref().map(html_to_text));
    let layout = match &sanitized {
        Some(s) => layout_html(&s.tree),
        None => {
            let t = text.clone().unwrap_or_default();
            let q = akatsuki_mime::quote::text_quote_start(&t);
            layout_text(&t, q)
        }
    };
    let mut inline_images = HashMap::new();
    for a in parsed
        .attachments
        .iter()
        .filter(|a| a.inline && a.mime_type.starts_with("image/") && a.size <= MAX_INLINE_IMAGE_BYTES)
    {
        if let Ok(Some(part)) = part_content(raw, &a.part_id) {
            inline_images.insert(a.part_id.clone(), (part.mime_type, part.bytes));
        }
    }
    Ok(Rendered {
        html: sanitized.as_ref().map(|s| s.html.clone()),
        text,
        layout,
        attachments: parsed.attachments.iter().filter(|a| !a.inline).cloned().collect(),
        remote_images: sanitized.as_ref().map_or(0, |s| s.remote_image_count),
        trackers: sanitized.as_ref().map_or(0, |s| s.trackers_removed),
        inline_images,
        list_unsubscribe: parsed.list_unsubscribe,
        sanitizer_version: SANITIZER_VERSION,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render::body::Block;

    #[test]
    fn renders_html_and_plain_messages() {
        let raw = b"From: Ada <ada@example.com>\r\nSubject: Hi\r\nMIME-Version: 1.0\r\nContent-Type: multipart/alternative; boundary=b\r\n\r\n--b\r\nContent-Type: text/plain; charset=utf-8\r\n\r\nHello plain\r\n--b\r\nContent-Type: text/html; charset=utf-8\r\n\r\n<p>Hello <b>html</b></p><img src=\"https://x.example/a.png\"><script>bad()</script>\r\n--b--\r\n";
        let r = render_raw(raw).unwrap();
        assert!(r.html.as_deref().unwrap().contains("Hello"));
        assert!(!r.html.as_deref().unwrap().contains("script"));
        assert_eq!(r.text.as_deref().map(str::trim), Some("Hello plain"));
        assert_eq!(r.remote_images, 1);
        assert!(matches!(&r.layout.blocks[0], Block::Para(p) if p.text == "Hello html"));

        let r =
            render_raw(b"From: a@b.c\r\nSubject: x\r\n\r\nLine one\r\n\r\nOn Mon, Bo wrote:\r\n> quoted\r\n").unwrap();
        assert!(r.html.is_none());
        assert_eq!(r.layout.blocks.len(), 1);
        assert!(!r.layout.quoted.is_empty());
    }
}
