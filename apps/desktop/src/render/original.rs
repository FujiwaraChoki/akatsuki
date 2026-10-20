//! "Open original" (§14.3): the sanitized HTML of a message, written to a private file and shown in
//! the system browser (D-317). The page carries the same CSP as the web client's sandboxed frame,
//! so even a sanitizer bug could not run script or fetch anything. Remote images stay blocked; "Show
//! images" uses the worker's sanitized copy, whose image URLs point at the worker's image proxy
//! (§14.4), and the page may then load images from that proxy only. A sender's own URL never
//! becomes a `src`, so senders never see the user's address or browser (DECISIONS D-399).

use std::io::Write;
use std::path::{Path, PathBuf};

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use sha2::{Digest, Sha256};

use super::message::Rendered;

/// Which images the page shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Images<'a> {
    /// Inline (`cid:`) images only.
    Blocked,
    /// The worker's sanitized HTML (`html`), with remote images loaded through its proxy: only URLs
    /// that start with `proxy_prefix` (`{sync}/v1/img/`) become `src`.
    Proxied {
        /// The worker's `MessageBody.html`.
        html: &'a str,
        /// `{sync base}/v1/img/`.
        proxy_prefix: &'a str,
    },
}

/// The origin (`scheme://host[:port]`) of a URL, or `None`.
fn origin(url: &str) -> Option<String> {
    let u = url::Url::parse(url).ok()?;
    if !matches!(u.scheme(), "https" | "http") {
        return None;
    }
    let o = u.origin().ascii_serialization();
    (o != "null").then_some(o)
}

/// The CSP of the original page: images from `data:` and, when showing images, the proxy origin only.
pub fn csp(images: Images<'_>) -> String {
    let img = match images {
        Images::Proxied { proxy_prefix, .. } => match origin(proxy_prefix) {
            Some(o) => format!("img-src data: {o}"),
            None => "img-src data:".to_owned(),
        },
        Images::Blocked => "img-src data:".to_owned(),
    };
    format!("default-src 'none'; {img}; style-src 'unsafe-inline'; font-src data:; form-action 'none'; base-uri 'none'")
}

fn escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

/// Turns `data-akatsuki-src="<proxy URL>"` into `src="…"`; any other deferred URL stays deferred.
fn load_proxied(html: &str, prefix: &str) -> String {
    const ATTR: &str = "data-akatsuki-src=\"";
    let mut out = String::with_capacity(html.len());
    let mut rest = html;
    while let Some(i) = rest.find(ATTR) {
        out.push_str(&rest[..i]);
        let after = &rest[i + ATTR.len()..];
        let Some(end) = after.find('"') else {
            out.push_str(&rest[i..]);
            return out;
        };
        let value = &after[..end];
        if !prefix.is_empty() && value.starts_with(prefix) && !value.contains(['<', '>', '\'']) {
            out.push_str("src=\"");
        } else {
            out.push_str(ATTR);
        }
        out.push_str(value);
        out.push('"');
        rest = &after[end + 1..];
    }
    out.push_str(rest);
    out
}

/// The HTML document for a rendered message.
pub fn document(subject: &str, rendered: &Rendered, images: Images<'_>) -> String {
    let html = match images {
        Images::Proxied { html, .. } => Some(html.to_owned()),
        Images::Blocked => rendered.html.clone(),
    };
    let mut body = match html {
        Some(h) => h,
        None => format!("<pre style=\"white-space:pre-wrap\">{}</pre>", escape(rendered.text.as_deref().unwrap_or(""))),
    };
    // Inline images become data URLs; the sanitizer left `data-akatsuki-cid="<partId>"` and no src.
    for (part, (mime, bytes)) in &rendered.inline_images {
        let needle = format!("data-akatsuki-cid=\"{}\"", escape(part));
        let src = format!("src=\"data:{};base64,{}\"", escape(mime), STANDARD.encode(bytes));
        body = body.replace(&needle, &src);
    }
    if let Images::Proxied { proxy_prefix, .. } = images {
        body = load_proxied(&body, proxy_prefix);
    }
    format!(
        "<!doctype html><html><head><meta charset=\"utf-8\">\
         <meta http-equiv=\"Content-Security-Policy\" content=\"{}\">\
         <meta name=\"referrer\" content=\"no-referrer\"><title>{}</title>\
         <style>body{{max-width:768px;margin:24px auto;padding:0 24px;font:15px/1.6 system-ui,sans-serif}}</style>\
         </head><body>{body}</body></html>",
        csp(images),
        escape(subject)
    )
}

/// Writes the original for `message_id` into `dir` (created with owner-only permissions) and
/// returns its path. The file name is a hash, never message data.
pub fn write_original(
    dir: &Path,
    message_id: &str,
    subject: &str,
    rendered: &Rendered,
    images: Images<'_>,
) -> std::io::Result<PathBuf> {
    std::fs::create_dir_all(dir)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?;
    }
    let shown = matches!(images, Images::Proxied { .. });
    let name = hex::encode(&Sha256::digest(format!("{message_id}|{shown}").as_bytes())[..12]);
    let path = dir.join(format!("{name}.html"));
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    let mut f = opts.open(&path)?;
    f.write_all(document(subject, rendered, images).as_bytes())?;
    Ok(path)
}

/// Deletes every written original (start-up and sign-out).
pub fn clear_originals(dir: &Path) {
    let _ = std::fs::remove_dir_all(dir);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render::message::render_raw;

    const RAW: &[u8] = b"From: a@b.c\r\nSubject: s\r\nMIME-Version: 1.0\r\nContent-Type: text/html; charset=utf-8\r\n\r\n<p>Hi</p><img src=\"https://img.example/x.png\"><a href=\"javascript:alert(1)\">x</a>\r\n";

    #[test]
    fn originals_carry_a_strict_csp_and_block_images_by_default() {
        let r = render_raw(RAW).unwrap();
        let doc = document("<Subject>", &r, Images::Blocked);
        assert!(doc.contains("default-src 'none'; img-src data:;"));
        assert!(doc.contains("&lt;Subject&gt;"));
        assert!(!doc.replace("data-akatsuki-src", "").contains("src=\"https://img.example"), "no live src");
        assert!(!doc.contains("javascript:"));
    }

    #[test]
    fn shown_images_load_through_the_worker_proxy_only() {
        let r = render_raw(RAW).unwrap();
        let proxied = "<p>Hi</p><img data-akatsuki-src=\"https://sync.akatsuki.vc/v1/img/u/sig/aHR0cHM\">\
                       <img data-akatsuki-src=\"https://img.example/x.png\">";
        let doc =
            document("s", &r, Images::Proxied { html: proxied, proxy_prefix: "https://sync.akatsuki.vc/v1/img/" });
        assert!(doc.contains("default-src 'none'; img-src data: https://sync.akatsuki.vc;"), "{doc}");
        assert!(doc.contains("src=\"https://sync.akatsuki.vc/v1/img/u/sig/aHR0cHM\""));
        // A sender URL (whatever produced it) is never loaded, and the CSP would refuse it anyway.
        assert!(!doc.contains(" src=\"https://img.example"));
        assert!(doc.contains("data-akatsuki-src=\"https://img.example/x.png\""));
        assert!(!doc.contains("img-src data: https:;"), "never any https image");
    }

    #[test]
    fn files_are_private_and_named_by_hash() {
        let dir = std::env::temp_dir().join(format!("akatsuki-originals-{}", uuid::Uuid::new_v4()));
        let r = render_raw(RAW).unwrap();
        let path = write_original(&dir, "m/../1", "s", &r, Images::Blocked).unwrap();
        assert_eq!(path.parent(), Some(dir.as_path()));
        assert!(!path.file_name().unwrap().to_string_lossy().contains(".."));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
        }
        clear_originals(&dir);
        assert!(!dir.exists());
    }
}
