//! Hostile-input tests for parsing and sanitizing. Every fixture is synthetic.

use akatsuki_mime::{
    Display, ImageSource, ProxySigner, RemoteImagePolicy, SafeNode, SanitizeOptions, html_to_text,
    message_body_from_raw, parse_message, part_content, sanitize_html, sanitize_outgoing_html,
};
use std::collections::HashMap;

const HOSTILE: &[u8] = include_bytes!("fixtures/hostile.eml");
const CHARSETS: &[u8] = include_bytes!("fixtures/charsets.eml");
const MALFORMED: &[u8] = include_bytes!("fixtures/malformed.eml");
const PLAIN: &[u8] = include_bytes!("fixtures/plain-reply.eml");

const FORBIDDEN: &[&str] = &[
    "<script",
    "javascript:",
    "vbscript:",
    "onload",
    "onerror",
    "onclick",
    "<iframe",
    "<object",
    "<embed",
    "<form",
    "<input",
    "<button",
    "<meta",
    "<base",
    "<link",
    "<svg",
    "<math",
    "http-equiv",
    "url(",
    "@import",
    "<title",
    "data:text",
    "data:image/svg",
    "srcset",
    "evil.example",
    "expression(",
    "<!--",
    "position",
];

fn proxy() -> RemoteImagePolicy {
    RemoteImagePolicy::Proxy(ProxySigner::new("https://sync.akatsuki.vc", b"test-proxy-key-not-secret", "user_1"))
}

fn assert_clean(html: &str) {
    let lower = html.to_ascii_lowercase();
    for f in FORBIDDEN {
        assert!(!lower.contains(f), "sanitized output contains {f:?}:\n{html}");
    }
    // No element may load anything by itself: every image is deferred.
    assert!(!lower.contains(" src=\"http"), "remote src survived:\n{html}");
}

#[test]
fn hostile_message_end_to_end() {
    let body = message_body_from_raw(HOSTILE, "m1".into(), "p1", proxy()).unwrap();
    let html = body.html.as_deref().unwrap();
    assert_clean(html);

    // Headers survive decoding.
    let parsed = parse_message(HOSTILE).unwrap();
    assert_eq!(parsed.subject, "Hello 👋 Ada");
    assert_eq!(parsed.from.as_ref().unwrap().email, "mallory@evil.example");
    assert_eq!(parsed.list_id.as_deref(), Some("news.evil.example"));
    assert!(parsed.is_bulk);
    assert!(parsed.list_unsubscribe.as_ref().unwrap().one_click);
    let auth = parsed.auth.as_ref().unwrap();
    assert_eq!((auth.spf.as_str(), auth.dkim.as_str(), auth.dmarc.as_str()), ("fail", "none", "fail"));

    // Text formatting survives; scripts and their content do not.
    assert!(html.contains("Hello <b>Ada</b>"));
    assert!(html.starts_with("<div class=\"akatsuki-body\">"));
    assert!(!html.contains("document.write"));
    assert!(!html.contains("password"));

    // Inline style keeps only safe declarations.
    assert!(html.contains("<p style=\"color:red\">"));

    // Links: dangerous schemes lose their href, the safe one is rewritten to open safely.
    assert!(html.contains("<a href=\"https://example.com/ok\" target=\"_blank\" rel=\"noopener noreferrer\">ok</a>"));
    assert_eq!(html.matches("href=").count(), 1, "{html}");

    // Images: one remote image proxied, three trackers removed, cid mapped to its part id,
    // svg data removed, png data kept.
    assert_eq!(body.remote_image_count, 1);
    assert_eq!(body.trackers_removed, 3);
    assert!(html.contains("data-akatsuki-src=\"https://sync.akatsuki.vc/v1/img/"));
    let logo = parsed.attachments.iter().find(|a| a.content_id.as_deref() == Some("logo@example.com")).unwrap();
    assert!(logo.inline);
    assert!(html.contains(&format!("data-akatsuki-cid=\"{}\"", logo.part_id)));
    assert!(html.contains("src=\"data:image/png;base64,iVBORw0KGgo=\""));

    // The proxy URL verifies with the key and decodes back to the original.
    let start = html.find("/v1/img/").unwrap() + "/v1/img/".len();
    let rest = &html[start..html[start..].find('"').unwrap() + start];
    let parts: Vec<&str> = rest.split('/').collect();
    assert_eq!(
        ProxySigner::verify(b"test-proxy-key-not-secret", parts[0], parts[1], parts[2]).as_deref(),
        Some("https://cdn.example.com/hero.png")
    );
    assert_eq!(ProxySigner::verify(b"other-key", parts[0], parts[1], parts[2]), None);

    // Stylesheet: @import and url() gone, the rest scoped.
    assert!(html.contains("<style>.akatsuki-body{color:#111}.akatsuki-body a[href^=https]{color:red}</style>"));

    // Nested quotes: the first quote container is the collapse point.
    let q = body.quoted.html_start.unwrap() as usize;
    assert!(html[q..].starts_with("<div class=\"gmail_quote\""), "{}", &html[q..]);
    assert_eq!(html.matches("data-akatsuki-quote").count(), 1);
    assert!(html.contains("nested"));
    let tq = body.quoted.text_start.unwrap() as usize;
    assert!(body.text.as_deref().unwrap()[tq..].starts_with("On Mon, Ada wrote:"));

    // Attachments: the hostile filename is defused, and the part is retrievable by id.
    let pdf = parsed.attachments.iter().find(|a| a.mime_type == "application/pdf").unwrap();
    assert!(!pdf.filename.contains('/'));
    assert!(!pdf.inline);
    let content = part_content(HOSTILE, &pdf.part_id).unwrap().unwrap();
    assert!(content.bytes.starts_with(b"%PDF-1.7"));
    assert_eq!(content.mime_type, "application/pdf");
    assert_eq!(part_content(HOSTILE, "9.9").unwrap(), None);
}

#[test]
fn safe_tree_matches_html() {
    let parsed = parse_message(HOSTILE).unwrap();
    let opts = SanitizeOptions { remote_images: RemoteImagePolicy::Original, cid_map: parsed.cid_map.clone() };
    let s = sanitize_html(parsed.html.as_deref().unwrap(), &opts);
    fn walk<'a>(nodes: &'a [SafeNode], out: &mut Vec<&'a akatsuki_mime::SafeElement>) {
        for n in nodes {
            if let SafeNode::Element(e) = n {
                out.push(e);
                walk(&e.children, out);
            }
        }
    }
    let mut all = Vec::new();
    walk(&s.tree, &mut all);
    let images: Vec<&ImageSource> = all.iter().filter_map(|e| e.image.as_ref()).collect();
    assert!(images.contains(&&ImageSource::Remote { url: "https://cdn.example.com/hero.png".into() }));
    assert!(images.iter().any(|i| matches!(i, ImageSource::Cid { .. })));
    assert!(images.iter().any(|i| matches!(i, ImageSource::Data { .. })));
    assert_eq!(images.len(), 3);
    for e in &all {
        for (k, _) in &e.attrs {
            assert!(!k.starts_with("on") && k != "src" && k != "href" && k != "srcset", "attr {k} leaked into tree");
        }
    }
    let link = all.iter().find(|e| e.href.is_some()).unwrap();
    assert_eq!(link.href.as_deref(), Some("https://example.com/ok"));
    assert_eq!(all.iter().filter(|e| e.quote_start).count(), 1);
    assert!(s.tree.iter().any(|n| matches!(n, SafeNode::Stylesheet(_))));
    // Original policy keeps the URL but still never as src.
    assert!(s.html.contains("data-akatsuki-src=\"https://cdn.example.com/hero.png\""));
}

#[test]
fn drop_policy_removes_remote_images_but_counts_them() {
    let s = sanitize_html(
        "<p>a<img src=\"https://x.example/a.png\" width=\"300\"></p>",
        &SanitizeOptions { remote_images: RemoteImagePolicy::Drop, cid_map: HashMap::new() },
    );
    assert_eq!(s.remote_image_count, 1);
    assert!(!s.html.contains("img"));
}

#[test]
fn charset_edge_cases() {
    let p = parse_message(CHARSETS).unwrap();
    assert_eq!(p.from.as_ref().unwrap().name.as_deref(), Some("André"));
    assert_eq!(p.to[0].name.as_deref(), Some("日本語"));
    assert_eq!(p.subject, "てすと café");
    let text = p.text.as_deref().unwrap();
    assert!(text.contains("Café crème brûlée soft-break"), "{text:?}");
    let html = p.html.as_deref().unwrap();
    assert!(html.contains("\u{201c}smart quotes\u{201d} \u{2014} \u{20ac}5"), "{html:?}");
    let att = p.attachments.iter().find(|a| a.mime_type == "application/octet-stream").unwrap();
    assert_eq!(att.filename, "résumé.txt");
}

#[test]
fn malformed_mime_does_not_panic() {
    let p = parse_message(MALFORMED).unwrap();
    assert_eq!(p.date, None);
    assert!(p.from.is_some());
    let body = message_body_from_raw(MALFORMED, "m".into(), "p", RemoteImagePolicy::Drop).unwrap();
    if let Some(html) = &body.html {
        assert_clean(html);
        assert!(html.ends_with("</div>"));
    }
    // Arbitrary truncations and byte noise never panic.
    for cut in (0..MALFORMED.len()).step_by(7) {
        let _ = parse_message(&MALFORMED[..cut]);
    }
    for cut in (0..HOSTILE.len()).step_by(53) {
        if let Ok(p) = parse_message(&HOSTILE[..cut])
            && let Some(h) = &p.html
        {
            assert_clean(&sanitize_html(h, &SanitizeOptions::default()).html);
        }
    }
    let noise: Vec<u8> = (0..4096u32).map(|i| (i.wrapping_mul(2_654_435_761) >> 13) as u8).collect();
    let _ = parse_message(&noise);
    assert!(parse_message(b"").is_err() || parse_message(b"").is_ok());
}

#[test]
fn plain_text_message() {
    let body = message_body_from_raw(PLAIN, "m".into(), "p", RemoteImagePolicy::Drop).unwrap();
    assert_eq!(body.html, None);
    let text = body.text.as_deref().unwrap();
    let q = body.quoted.text_start.unwrap() as usize;
    assert!(text[q..].starts_with("On Mon, 19 Oct 2026"));
    let p = parse_message(PLAIN).unwrap();
    assert_eq!(p.in_reply_to.as_deref(), Some("lunch-1@example.com"));
    assert_eq!(p.references, vec!["lunch-0@example.com", "lunch-1@example.com"]);
    assert!(!p.is_bulk);
    assert_eq!(akatsuki_mime::snippet(&p), "Sounds good, see you at noon.".trim_start_matches('\u{feff}'));
}

#[test]
fn xss_vectors() {
    let vectors = [
        "<scr<script>ipt>alert(1)</script>",
        "<img src=x onerror=alert(1)>",
        "<a href=\"&#106;&#97;&#118;&#97;&#115;&#99;&#114;&#105;&#112;&#116;:alert(1)\">x</a>",
        "<a href=\"java\nscript:alert(1)\">x</a>",
        "<div style=\"background:url(javascript:alert(1))\">x</div>",
        "<div style=\"width: expression(alert(1))\">x</div>",
        "<div style=\"-moz-binding: url(http://evil.example/x.xml#x)\">x</div>",
        "<style>@import 'https://evil.example/x.css';</style>",
        "<style>*{background:u\\72l(https://evil.example)}</style>",
        "<style></style><script>alert(1)</script>",
        "<noscript><p title=\"</noscript><img src=x onerror=alert(1)>\"></noscript>",
        "<math><mtext><table><mglyph><style><img src=x onerror=alert(1)></style></mglyph></table></mtext></math>",
        "<svg/onload=alert(1)>",
        "<details open ontoggle=alert(1)>",
        "<a href=\"https://ok.example\" style=\"position:fixed;top:0;left:0;width:100%;height:100%\">overlay</a>",
        "<table background=\"https://evil.example/bg.gif\"><tr><td>x</td></tr></table>",
        "<object data=\"data:text/html,<script>alert(1)</script>\"></object>",
        "<meta http-equiv=\"refresh\" content=\"0; url=https://evil.example\">",
        "<form><button formaction=\"https://evil.example\">go</button></form>",
        "<img src=\"https://evil.example/x.gif\" style=\"width:1px;height:1px\">",
        "<a href=\"//evil.example/protocol-relative\">x</a>",
        "<a href=\"/relative\">x</a>",
        "<img src=\"data:image/png;base64,AAA\" onload=\"x\">",
        "<iframe srcdoc=\"<script>alert(1)</script>\"></iframe>",
        "<template><script>alert(1)</script></template>",
        "<textarea><script>alert(1)</script></textarea>",
    ];
    for v in vectors {
        let s = sanitize_html(v, &SanitizeOptions::default());
        let lower = s.html.to_ascii_lowercase();
        for bad in [
            "<script",
            "javascript:",
            "onerror",
            "onload",
            "ontoggle",
            "expression(",
            "url(",
            "@import",
            "<iframe",
            "<object",
            "<meta",
            "<form",
            "<button",
            "position",
            "background=",
            "evil.example",
            "<svg",
            "<style><img",
        ] {
            assert!(!lower.contains(bad), "vector {v:?} produced {bad:?}: {}", s.html);
        }
        let out = sanitize_outgoing_html(v);
        let lower = out.to_ascii_lowercase();
        for bad in ["<script", "javascript:", "onerror", "onload", "<iframe", "<object", "<meta", "<form"] {
            assert!(!lower.contains(bad), "outgoing {v:?} produced {bad:?}: {out}");
        }
    }
}

#[test]
fn deep_nesting_is_bounded() {
    // Beyond the parse guard: shown as text, no markup survives.
    let html = "<div>".repeat(50_000) + "deep<script>x</script>" + &"</div>".repeat(50_000);
    let s = sanitize_html(&html, &SanitizeOptions::default());
    assert!(s.html.contains("deep"));
    assert!(!s.html.contains("<script"));
    // Within the guard but beyond the tree cap: flattened, content kept.
    let html = "<div>".repeat(900) + "<b>deep</b>" + &"</div>".repeat(900);
    let s = sanitize_html(&html, &SanitizeOptions::default());
    assert!(s.html.contains("deep"));
    assert!(s.html.matches("<div").count() <= 260);
    let text = html_to_text(&("<blockquote>".repeat(20_000) + "q"));
    assert!(text.contains('q'));
}

#[test]
fn hidden_preheader_and_display_classes() {
    let s = sanitize_html(
        "<span style=\"display:none\">preheader</span><p>Body<br>line</p><table><tr><td>c</td></tr></table>",
        &SanitizeOptions::default(),
    );
    let SafeNode::Element(span) = &s.tree[0] else { panic!() };
    assert_eq!(span.display(), Display::Hidden);
    let SafeNode::Element(p) = &s.tree[1] else { panic!() };
    assert_eq!(p.display(), Display::Block);
    let SafeNode::Element(br) = &p.children[1] else { panic!() };
    assert_eq!(br.display(), Display::LineBreak);
}

#[test]
fn outgoing_html_keeps_remote_images_and_links() {
    let out = sanitize_outgoing_html(
        "<p>Hi <a href=\"https://akatsuki.vc\">site</a><img src=\"https://cdn.example.com/a.png\" alt=\"a\"></p>",
    );
    assert_eq!(
        out,
        "<p>Hi <a href=\"https://akatsuki.vc\">site</a><img alt=\"a\" src=\"https://cdn.example.com/a.png\"></p>"
    );
}

#[test]
fn text_conversion() {
    assert_eq!(html_to_text("<p>One</p><p>Two<br>Three</p><script>x</script><style>p{}</style>"), "One\nTwo\nThree");
    assert_eq!(html_to_text("a&nbsp;&amp;&lt;b"), "a &<b");
}
