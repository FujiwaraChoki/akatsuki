//! The one HTML sanitizer (ARCHITECTURE §14.3). Email HTML is hostile input.
//!
//! The pipeline has three stages:
//!
//! 1. **ammonia** (allow-list): parses the HTML with html5ever, keeps only formatting tags and a
//!    small set of attributes, drops `script`, `iframe`, `object`, `embed`, `form` controls,
//!    `meta`, `base`, `link`, `svg`, `math`, every `on*` attribute, comments, relative URLs and
//!    every URL scheme except `http`, `https`, `mailto`, `tel` (links) and `http`, `https`, `cid`,
//!    small `data:image/{png,gif,jpeg,webp}` (images). `style` attributes are filtered through
//!    [`crate::css`].
//! 2. **Rewrite** (our tree): the well-formed ammonia output is tokenized into a [`SafeNode`] tree,
//!    where `<style>` blocks are sanitized and scoped, tracking pixels are removed and counted,
//!    remote `<img src>` becomes `data-akatsuki-src` (through the image proxy, or kept as the
//!    original URL, or dropped, per [`RemoteImagePolicy`]), `cid:` becomes `data-akatsuki-cid`
//!    with the part id, links get `target="_blank" rel="noopener noreferrer"`, and the start of
//!    quoted history is marked.
//! 3. **Serialize**: the tree is written back as HTML inside `<div class="akatsuki-body">`. The
//!    tree is what the desktop renders natively; the HTML is what web, mobile and "Open original"
//!    render in a sandboxed frame. Both describe exactly the same content.
//!
//! Bump [`SANITIZER_VERSION`] whenever the output for some input changes, so device caches keyed by
//! `(messageId, sanitizerVersion)` are invalidated.

use std::borrow::Cow;
use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::sync::LazyLock;

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use hmac::{Hmac, Mac};
use html5ever::tendril::StrTendril;
use html5ever::tokenizer::states::RawKind;
use html5ever::tokenizer::{
    BufferQueue, CharacterTokens, EndTag, StartTag, TagToken, Token, TokenSink, TokenSinkResult, Tokenizer,
    TokenizerOpts,
};
use sha2::{Digest, Sha256};

use crate::css::{self, BODY_CLASS, Declaration};

/// Version of the sanitizer output. Part of every body cache key.
pub const SANITIZER_VERSION: u32 = 1;

/// Largest inline `data:` image kept (1 MB decoded, about 1.4 MB of base64).
pub const MAX_DATA_IMAGE_BYTES: usize = 1024 * 1024;

const ALLOWED_TAGS: &[&str] = &[
    "a",
    "abbr",
    "acronym",
    "address",
    "article",
    "aside",
    "b",
    "bdi",
    "bdo",
    "big",
    "blockquote",
    "br",
    "caption",
    "center",
    "cite",
    "code",
    "col",
    "colgroup",
    "dd",
    "del",
    "dfn",
    "div",
    "dl",
    "dt",
    "em",
    "figcaption",
    "figure",
    "font",
    "footer",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "header",
    "hgroup",
    "hr",
    "i",
    "img",
    "ins",
    "kbd",
    "li",
    "main",
    "mark",
    "nav",
    "ol",
    "p",
    "pre",
    "q",
    "rp",
    "rt",
    "ruby",
    "s",
    "samp",
    "section",
    "small",
    "span",
    "strike",
    "strong",
    "style",
    "sub",
    "summary",
    "sup",
    "table",
    "tbody",
    "td",
    "tfoot",
    "th",
    "thead",
    "time",
    "tr",
    "tt",
    "u",
    "ul",
    "var",
    "wbr",
];

/// Elements removed together with everything inside them.
const CLEAN_CONTENT_TAGS: &[&str] = &[
    "script",
    "noscript",
    "template",
    "textarea",
    "title",
    "iframe",
    "frame",
    "frameset",
    "noframes",
    "object",
    "embed",
    "applet",
    "svg",
    "math",
    "select",
    "option",
    "optgroup",
    "datalist",
    "noembed",
    "xmp",
    "plaintext",
    "audio",
    "video",
    "canvas",
    "dialog",
    "head",
];

const GENERIC_ATTRIBUTES: &[&str] = &[
    "abbr",
    "align",
    "alt",
    "bgcolor",
    "border",
    "cellpadding",
    "cellspacing",
    "class",
    "color",
    "colspan",
    "dir",
    "face",
    "headers",
    "height",
    "hspace",
    "id",
    "lang",
    "nowrap",
    "rowspan",
    "scope",
    "size",
    "span",
    "start",
    "style",
    "summary",
    "title",
    "type",
    "valign",
    "value",
    "vspace",
    "width",
];

const VOID_TAGS: &[&str] = &["br", "hr", "img", "wbr", "col"];

const BLOCK_TAGS: &[&str] = &[
    "address",
    "article",
    "aside",
    "blockquote",
    "center",
    "dd",
    "div",
    "dl",
    "dt",
    "figcaption",
    "figure",
    "footer",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "header",
    "hgroup",
    "hr",
    "li",
    "main",
    "nav",
    "ol",
    "p",
    "pre",
    "section",
    "summary",
    "table",
    "ul",
];

/// URL fragments of well-known open-tracking endpoints (lowercase).
const TRACKER_PATTERNS: &[&str] = &[
    "/track/open",
    "/wf/open",
    "/open.php",
    "/e/o/",
    "/o.gif",
    "/pixel.gif",
    "/beacon",
    "mailtrack.io",
    "yesware.com",
    "mixmax.com/api/track",
    "sidekickopen",
    "bananatag",
    "getnotify.com",
    "mailstat.us",
    "emltrk.com",
    "trk.klclick",
    "/tracking/open",
    "open.convertkit-mail",
    "email.mg.",
];

/// How remote images are written into the output.
#[derive(Clone, Debug)]
pub enum RemoteImagePolicy {
    /// `data-akatsuki-src` = a signed image-proxy URL (worker, §14.4).
    Proxy(ProxySigner),
    /// `data-akatsuki-src` = the original URL (desktop local sanitizing; loaded only on consent).
    Original,
    /// Remote images are removed (still counted in `remote_image_count`).
    Drop,
}

/// Options for [`sanitize_html`].
#[derive(Clone, Debug)]
pub struct SanitizeOptions {
    /// Remote image handling.
    pub remote_images: RemoteImagePolicy,
    /// `Content-ID` (without brackets, lowercased) → part id, from the parsed message.
    pub cid_map: HashMap<String, String>,
}

impl Default for SanitizeOptions {
    fn default() -> Self {
        Self { remote_images: RemoteImagePolicy::Original, cid_map: HashMap::new() }
    }
}

/// Signs image-proxy URLs: `GET {base}/v1/img/{u}/{sig}/{b64url}` with
/// `sig = base64url(HMAC-SHA256(IMG_PROXY_KEY, "img1|" + u + "|" + url))`.
#[derive(Clone)]
pub struct ProxySigner {
    base_url: String,
    key: Vec<u8>,
    user_hash: String,
}

impl std::fmt::Debug for ProxySigner {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProxySigner").field("base_url", &self.base_url).finish_non_exhaustive()
    }
}

type HmacSha256 = Hmac<Sha256>;

/// `hex(sha256(userId))[..16]`, the `u` path segment of proxy URLs.
pub fn user_hash(user_id: &str) -> String {
    let digest = Sha256::digest(user_id.as_bytes());
    digest.iter().take(8).map(|b| format!("{b:02x}")).collect()
}

impl ProxySigner {
    /// A signer for `user_id`'s messages.
    pub fn new(base_url: &str, key: &[u8], user_id: &str) -> Self {
        Self { base_url: base_url.trim_end_matches('/').to_owned(), key: key.to_vec(), user_hash: user_hash(user_id) }
    }

    fn mac(key: &[u8], u: &str, url: &str) -> HmacSha256 {
        let mut mac = HmacSha256::new_from_slice(key).unwrap_or_else(|_| unreachable!("HMAC accepts any key length"));
        mac.update(b"img1|");
        mac.update(u.as_bytes());
        mac.update(b"|");
        mac.update(url.as_bytes());
        mac
    }

    /// The proxy URL for `url`.
    pub fn sign(&self, url: &str) -> String {
        let sig = URL_SAFE_NO_PAD.encode(Self::mac(&self.key, &self.user_hash, url).finalize().into_bytes());
        format!("{}/v1/img/{}/{}/{}", self.base_url, self.user_hash, sig, URL_SAFE_NO_PAD.encode(url.as_bytes()))
    }

    /// Checks a proxy request and returns the original URL when the signature is valid
    /// (constant-time comparison).
    pub fn verify(key: &[u8], u: &str, sig: &str, b64url: &str) -> Option<String> {
        let url = String::from_utf8(URL_SAFE_NO_PAD.decode(b64url).ok()?).ok()?;
        let sig = URL_SAFE_NO_PAD.decode(sig).ok()?;
        Self::mac(key, u, &url).verify_slice(&sig).ok()?;
        Some(url)
    }
}

/// Where an image comes from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ImageSource {
    /// A remote image; `url` is what `data-akatsuki-src` holds (proxy or original URL).
    Remote {
        /// URL to load once the user allows remote images.
        url: String,
    },
    /// An inline part referenced by `cid:`.
    Cid {
        /// Part id (or the raw content id when the part is unknown).
        part_id: String,
    },
    /// A small inline `data:image/*` URL, safe to render directly.
    Data {
        /// The data URL.
        url: String,
    },
}

/// How a native renderer should lay an element out.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Display {
    /// Block-level (paragraphs, divs, headings, lists, quotes).
    Block,
    /// Inline (spans, links, emphasis).
    Inline,
    /// Table parts (`table` itself is a block).
    TablePart,
    /// Line break.
    LineBreak,
    /// Hidden (`display:none`), e.g. preheader text.
    Hidden,
}

/// An element of the safe tree. Tags and attributes are already allow-listed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SafeElement {
    /// Lowercase tag name from the allow-list.
    pub tag: String,
    /// Remaining presentational attributes (`align`, `width`, `class`, …), in source order.
    pub attrs: Vec<(String, String)>,
    /// Sanitized inline style.
    pub style: Vec<Declaration>,
    /// For `img`: where the image comes from.
    pub image: Option<ImageSource>,
    /// For `a`: the link target (`http`, `https`, `mailto`, `tel`).
    pub href: Option<String>,
    /// The first element of quoted history (collapse here).
    pub quote_start: bool,
    /// Children.
    pub children: Vec<SafeNode>,
}

impl SafeElement {
    /// Layout class for native rendering.
    pub fn display(&self) -> Display {
        if self.style.iter().any(|d| d.property == "display" && d.value.trim_end_matches("!important").trim() == "none")
        {
            return Display::Hidden;
        }
        match self.tag.as_str() {
            "br" => Display::LineBreak,
            "tr" | "td" | "th" | "tbody" | "thead" | "tfoot" | "caption" | "col" | "colgroup" => Display::TablePart,
            t if BLOCK_TAGS.contains(&t) => Display::Block,
            _ => Display::Inline,
        }
    }

    /// The value of a presentational attribute.
    pub fn attr(&self, name: &str) -> Option<&str> {
        self.attrs.iter().find(|(k, _)| k == name).map(|(_, v)| v.as_str())
    }

    /// The value of a style property.
    pub fn style_value(&self, property: &str) -> Option<&str> {
        self.style.iter().rev().find(|d| d.property == property).map(|d| d.value.as_str())
    }
}

/// A node of the safe tree.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SafeNode {
    /// An element.
    Element(SafeElement),
    /// Text (entities decoded).
    Text(String),
    /// A sanitized, scoped stylesheet from a `<style>` block.
    Stylesheet(String),
}

impl SafeNode {
    /// Concatenated visible text (for previews and tests).
    pub fn text_content(&self) -> String {
        let mut out = String::new();
        self.collect_text(&mut out);
        out
    }

    fn collect_text(&self, out: &mut String) {
        match self {
            SafeNode::Text(t) => out.push_str(t),
            SafeNode::Element(e) => e.children.iter().for_each(|c| c.collect_text(out)),
            SafeNode::Stylesheet(_) => {}
        }
    }
}

/// The result of [`sanitize_html`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SanitizedHtml {
    /// Sanitized HTML, wrapped in `<div class="akatsuki-body">`.
    pub html: String,
    /// The same content as a tree, for native rendering.
    pub tree: Vec<SafeNode>,
    /// Remote images found (blocked, proxied or dropped).
    pub remote_image_count: u32,
    /// Tracking pixels removed.
    pub trackers_removed: u32,
    /// Byte offset in `html` where quoted history starts.
    pub quote_html_start: Option<usize>,
    /// [`SANITIZER_VERSION`].
    pub sanitizer_version: u32,
}

fn is_safe_data_image(value: &str) -> bool {
    let lower = value.get(..32).unwrap_or(value).to_ascii_lowercase();
    let ok_type =
        ["data:image/png;base64,", "data:image/gif;base64,", "data:image/jpeg;base64,", "data:image/webp;base64,"]
            .iter()
            .any(|p| lower.starts_with(p));
    ok_type
        && value.len() <= MAX_DATA_IMAGE_BYTES / 3 * 4 + 64
        && value.split_once(',').is_some_and(|(_, b64)| {
            b64.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'+' || b == b'/' || b == b'=')
        })
}

fn scheme(value: &str) -> Option<String> {
    let (s, _) = value.split_once(':')?;
    (!s.is_empty() && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'+' || b == b'-' || b == b'.'))
        .then(|| s.to_ascii_lowercase())
}

fn filter_attribute<'u>(element: &str, attribute: &str, value: &'u str) -> Option<Cow<'u, str>> {
    match attribute {
        "style" => css::sanitize_style_attribute(value).map(Cow::Owned),
        "href" => match scheme(value).as_deref() {
            Some("http" | "https" | "mailto" | "tel") => Some(Cow::Borrowed(value)),
            _ => None,
        },
        "src" if element == "img" => match scheme(value).as_deref() {
            Some("http" | "https" | "cid") => Some(Cow::Borrowed(value)),
            Some("data") if is_safe_data_image(value) => Some(Cow::Borrowed(value)),
            _ => None,
        },
        "src" => None,
        _ => Some(Cow::Borrowed(value)),
    }
}

fn builder() -> ammonia::Builder<'static> {
    let mut b = ammonia::Builder::empty();
    b.tags(ALLOWED_TAGS.iter().copied().collect())
        .clean_content_tags(CLEAN_CONTENT_TAGS.iter().copied().collect())
        .generic_attributes(GENERIC_ATTRIBUTES.iter().copied().collect())
        .tag_attributes(HashMap::from([("a", HashSet::from(["href"])), ("img", HashSet::from(["src"]))]))
        .url_schemes(HashSet::from(["http", "https", "mailto", "tel", "cid", "data"]))
        .url_relative(ammonia::UrlRelative::Deny)
        .link_rel(None)
        .strip_comments(true)
        .attribute_filter(filter_attribute);
    b
}

static BUILDER: LazyLock<ammonia::Builder<'static>> = LazyLock::new(builder);

#[derive(Debug)]
enum RawNode {
    Element { name: String, attrs: Vec<(String, String)>, children: Vec<RawNode> },
    Text(String),
}

/// Maximum element nesting kept in the tree. Deeper start tags are flattened into their nearest
/// kept ancestor, so hostile nesting cannot exhaust the stack of the recursive passes.
pub const MAX_TREE_DEPTH: usize = 256;

#[derive(Default)]
struct Collector {
    stack: RefCell<Vec<(String, Vec<(String, String)>, Vec<RawNode>)>>,
    /// Start tags dropped because of [`MAX_TREE_DEPTH`]; their end tags are ignored.
    skipped: RefCell<Vec<String>>,
    in_raw: Cell<bool>,
}

impl Collector {
    fn push_child(&self, node: RawNode) {
        let mut stack = self.stack.borrow_mut();
        if let Some((_, _, children)) = stack.last_mut() {
            if let (RawNode::Text(new), Some(RawNode::Text(prev))) = (&node, children.last_mut()) {
                prev.push_str(new);
                return;
            }
            children.push(node);
        }
    }

    fn close(&self, name: &str) {
        let pos = self.stack.borrow().iter().rposition(|(n, _, _)| n == name);
        let Some(pos) = pos else { return };
        if pos == 0 {
            return; // never close the synthetic root
        }
        while self.stack.borrow().len() > pos {
            let (name, attrs, children) = self.stack.borrow_mut().pop().unwrap_or_default();
            self.push_child(RawNode::Element { name, attrs, children });
        }
    }
}

impl TokenSink for Collector {
    type Handle = ();

    fn process_token(&self, token: Token, _line: u64) -> TokenSinkResult<()> {
        match token {
            TagToken(tag) => {
                let name = tag.name.to_string();
                match tag.kind {
                    StartTag => {
                        let attrs = tag.attrs.iter().map(|a| (a.name.local.to_string(), a.value.to_string())).collect();
                        if VOID_TAGS.contains(&name.as_str()) || tag.self_closing {
                            self.push_child(RawNode::Element { name, attrs, children: Vec::new() });
                        } else if self.stack.borrow().len() > MAX_TREE_DEPTH && name != "style" {
                            self.skipped.borrow_mut().push(name);
                        } else {
                            let raw = name == "style";
                            self.stack.borrow_mut().push((name, attrs, Vec::new()));
                            if raw {
                                self.in_raw.set(true);
                                return TokenSinkResult::RawData(RawKind::Rawtext);
                            }
                        }
                    }
                    EndTag => {
                        self.in_raw.set(false);
                        let skipped = self.skipped.borrow().last() == Some(&name);
                        if skipped {
                            self.skipped.borrow_mut().pop();
                        } else {
                            self.close(&name);
                        }
                    }
                }
            }
            CharacterTokens(t) => self.push_child(RawNode::Text(t.to_string())),
            _ => {}
        }
        TokenSinkResult::Continue
    }
}

fn tokenize(html: &str) -> Vec<RawNode> {
    let collector = Collector::default();
    collector.stack.borrow_mut().push((String::new(), Vec::new(), Vec::new()));
    let tok = Tokenizer::new(collector, TokenizerOpts::default());
    let input = BufferQueue::default();
    input.push_back(StrTendril::from(html));
    let _ = tok.feed(&input);
    tok.end();
    let collector = tok.sink;
    // Close anything left open.
    while collector.stack.borrow().len() > 1 {
        let (name, attrs, children) = collector.stack.borrow_mut().pop().unwrap_or_default();
        collector.push_child(RawNode::Element { name, attrs, children });
    }
    collector.stack.borrow_mut().pop().map(|(_, _, c)| c).unwrap_or_default()
}

struct Rewriter<'a> {
    opts: &'a SanitizeOptions,
    remote_images: u32,
    trackers: u32,
    quote_marked: bool,
}

fn is_quote_start(name: &str, attrs: &[(String, String)]) -> bool {
    let get = |k: &str| attrs.iter().find(|(n, _)| n == k).map(|(_, v)| v.as_str());
    let classes: Vec<&str> = get("class").map(|c| c.split_whitespace().collect()).unwrap_or_default();
    let has_class = |c: &str| classes.iter().any(|x| x.eq_ignore_ascii_case(c));
    match name {
        "blockquote" => {
            get("type").is_some_and(|t| t.eq_ignore_ascii_case("cite"))
                || has_class("gmail_quote")
                || has_class("protonmail_quote")
        }
        "div" => {
            has_class("gmail_quote")
                || has_class("gmail_quote_container")
                || has_class("yahoo_quoted")
                || has_class("moz-cite-prefix")
                || has_class("protonmail_quote")
                || get("id").is_some_and(|id| id == "divRplyFwdMsg" || id == "appendonsend")
        }
        _ => false,
    }
}

fn dimension_at_most_one(attrs: &[(String, String)], style: &[Declaration], name: &str) -> bool {
    let attr = attrs.iter().find(|(k, _)| k == name).and_then(|(_, v)| css::px(v));
    let style = style.iter().rev().find(|d| d.property == name).and_then(|d| css::px(&d.value));
    style.or(attr).is_some_and(|v| v <= 1.0)
}

fn looks_like_tracker(url: &str, attrs: &[(String, String)], style: &[Declaration]) -> bool {
    let lower = url.to_ascii_lowercase();
    if TRACKER_PATTERNS.iter().any(|p| lower.contains(p)) {
        return true;
    }
    let tiny = dimension_at_most_one(attrs, style, "width") && dimension_at_most_one(attrs, style, "height");
    let hidden = style.iter().any(|d| {
        (d.property == "display" && d.value.starts_with("none"))
            || (d.property == "visibility" && d.value.starts_with("hidden"))
            || (d.property == "opacity" && css::px(&d.value) == Some(0.0))
    });
    tiny || hidden
}

impl Rewriter<'_> {
    fn nodes(&mut self, raw: Vec<RawNode>) -> Vec<SafeNode> {
        raw.into_iter().filter_map(|n| self.node(n)).collect()
    }

    fn node(&mut self, raw: RawNode) -> Option<SafeNode> {
        match raw {
            RawNode::Text(t) => Some(SafeNode::Text(t)),
            RawNode::Element { name, attrs, children } => {
                if name == "style" {
                    let text: String = children
                        .iter()
                        .map(|c| match c {
                            RawNode::Text(t) => t.as_str(),
                            RawNode::Element { .. } => "",
                        })
                        .collect();
                    let css = css::sanitize_stylesheet(&text);
                    return (!css.is_empty()).then_some(SafeNode::Stylesheet(css));
                }
                if !ALLOWED_TAGS.contains(&name.as_str()) {
                    // Defensive: ammonia already removed it; keep only its children.
                    let kids = self.nodes(children);
                    return Some(SafeNode::Element(SafeElement {
                        tag: "span".into(),
                        attrs: Vec::new(),
                        style: Vec::new(),
                        image: None,
                        href: None,
                        quote_start: false,
                        children: kids,
                    }));
                }
                let quote_start = !self.quote_marked && is_quote_start(&name, &attrs);
                if quote_start {
                    self.quote_marked = true;
                }
                let mut style = Vec::new();
                let mut href = None;
                let mut src = None;
                let mut kept = Vec::new();
                for (k, v) in attrs {
                    match k.as_str() {
                        "style" => style = css::sanitize_declarations(&v),
                        "href" if name == "a" => href = Some(v),
                        "src" if name == "img" => src = Some(v),
                        "id" | "rel" | "target" => {}
                        k if k.starts_with("on") => {}
                        _ if GENERIC_ATTRIBUTES.contains(&k.as_str()) => kept.push((k, v)),
                        _ => {}
                    }
                }
                let mut image = None;
                if name == "img" {
                    let src = src?;
                    match scheme(&src).as_deref() {
                        Some("cid") => {
                            let cid =
                                src[4..].trim().trim_start_matches('<').trim_end_matches('>').to_ascii_lowercase();
                            let part_id = self.opts.cid_map.get(&cid).cloned().unwrap_or(cid);
                            image = Some(ImageSource::Cid { part_id });
                        }
                        Some("data") if is_safe_data_image(&src) => image = Some(ImageSource::Data { url: src }),
                        Some("http" | "https") => {
                            if looks_like_tracker(&src, &kept, &style) {
                                self.trackers += 1;
                                return None;
                            }
                            self.remote_images += 1;
                            let url = match &self.opts.remote_images {
                                RemoteImagePolicy::Proxy(signer) => signer.sign(&src),
                                RemoteImagePolicy::Original => src,
                                RemoteImagePolicy::Drop => return None,
                            };
                            image = Some(ImageSource::Remote { url });
                        }
                        _ => return None,
                    }
                }
                let children = self.nodes(children);
                Some(SafeNode::Element(SafeElement {
                    tag: name,
                    attrs: kept,
                    style,
                    image,
                    href,
                    quote_start,
                    children,
                }))
            }
        }
    }
}

fn escape_text(s: &str, out: &mut String) {
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '\u{a0}' => out.push_str("&nbsp;"),
            c => out.push(c),
        }
    }
}

fn escape_attr(s: &str, out: &mut String) {
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '"' => out.push_str("&quot;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            c => out.push(c),
        }
    }
}

fn push_attr(out: &mut String, k: &str, v: &str) {
    out.push(' ');
    out.push_str(k);
    out.push_str("=\"");
    escape_attr(v, out);
    out.push('"');
}

fn serialize(nodes: &[SafeNode], out: &mut String, quote_at: &mut Option<usize>) {
    for n in nodes {
        match n {
            SafeNode::Text(t) => escape_text(t, out),
            SafeNode::Stylesheet(css) => {
                out.push_str("<style>");
                out.push_str(css);
                out.push_str("</style>");
            }
            SafeNode::Element(e) => {
                if e.quote_start && quote_at.is_none() {
                    *quote_at = Some(out.len());
                }
                out.push('<');
                out.push_str(&e.tag);
                for (k, v) in &e.attrs {
                    push_attr(out, k, v);
                }
                if !e.style.is_empty() {
                    push_attr(out, "style", &css::render_declarations(&e.style));
                }
                if let Some(href) = &e.href {
                    push_attr(out, "href", href);
                    push_attr(out, "target", "_blank");
                    push_attr(out, "rel", "noopener noreferrer");
                }
                match &e.image {
                    Some(ImageSource::Remote { url }) => push_attr(out, "data-akatsuki-src", url),
                    Some(ImageSource::Cid { part_id }) => push_attr(out, "data-akatsuki-cid", part_id),
                    Some(ImageSource::Data { url }) => push_attr(out, "src", url),
                    None => {}
                }
                if e.quote_start {
                    push_attr(out, "data-akatsuki-quote", "");
                }
                out.push('>');
                if VOID_TAGS.contains(&e.tag.as_str()) {
                    continue;
                }
                serialize(&e.children, out, quote_at);
                out.push_str("</");
                out.push_str(&e.tag);
                out.push('>');
            }
        }
    }
}

/// HTML larger than this is truncated before parsing (Gmail clips at about 100 KB; real bodies
/// stay far below this).
pub const MAX_HTML_BYTES: usize = 4 * 1024 * 1024;

/// Estimated nesting above which the HTML parser's tree construction becomes quadratic enough to
/// be a denial-of-service vector; such bodies are shown as plain text instead.
pub const MAX_PARSE_DEPTH: usize = 1024;

fn truncate_utf8(s: &str, max: usize) -> &str {
    if s.len() <= max {
        return s;
    }
    let mut end = max;
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

/// A linear upper-bound estimate of element nesting: start tags minus end tags, ignoring void
/// elements. It over-counts unclosed `<p>`/`<li>`, which only makes the guard stricter.
fn estimated_depth(html: &str) -> usize {
    let bytes = html.as_bytes();
    let (mut depth, mut max) = (0_usize, 0_usize);
    let mut i = 0;
    while i + 1 < bytes.len() {
        if bytes[i] == b'<' {
            let next = bytes[i + 1];
            if next == b'/' {
                depth = depth.saturating_sub(1);
            } else if next.is_ascii_alphabetic() {
                let name: String = bytes[i + 1..]
                    .iter()
                    .take_while(|b| b.is_ascii_alphanumeric())
                    .take(16)
                    .map(|b| b.to_ascii_lowercase() as char)
                    .collect();
                if !matches!(
                    name.as_str(),
                    "br" | "hr" | "img" | "wbr" | "col" | "meta" | "link" | "input" | "base" | "area" | "source"
                ) {
                    depth += 1;
                    max = max.max(depth);
                }
            }
        }
        i += 1;
    }
    max
}

/// Strips tags without a parser (linear time), for bodies too hostile to parse.
fn crude_text(html: &str) -> String {
    let mut out = String::with_capacity(html.len() / 2);
    let mut in_tag = false;
    for c in html.chars() {
        match (in_tag, c) {
            (false, '<') => in_tag = true,
            (true, '>') => in_tag = false,
            (false, c) => out.push(c),
            _ => {}
        }
    }
    out.replace("&nbsp;", " ").replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&amp;", "&")
}

/// Sanitizes a message's HTML body.
pub fn sanitize_html(html: &str, opts: &SanitizeOptions) -> SanitizedHtml {
    let html = truncate_utf8(html, MAX_HTML_BYTES);
    if estimated_depth(html) > MAX_PARSE_DEPTH {
        let text = crude_text(html);
        let mut out = format!("<div class=\"{BODY_CLASS}\"><div style=\"white-space:pre-wrap\">");
        escape_text(&text, &mut out);
        out.push_str("</div></div>");
        return SanitizedHtml {
            html: out,
            tree: vec![SafeNode::Element(SafeElement {
                tag: "div".into(),
                attrs: Vec::new(),
                style: vec![Declaration { property: "white-space".into(), value: "pre-wrap".into() }],
                image: None,
                href: None,
                quote_start: false,
                children: vec![SafeNode::Text(text)],
            })],
            remote_image_count: 0,
            trackers_removed: 0,
            quote_html_start: None,
            sanitizer_version: SANITIZER_VERSION,
        };
    }
    let cleaned = BUILDER.clean(html).to_string();
    let raw = tokenize(&cleaned);
    let mut rw = Rewriter { opts, remote_images: 0, trackers: 0, quote_marked: false };
    let tree = rw.nodes(raw);
    let mut out = String::with_capacity(cleaned.len() + 64);
    out.push_str("<div class=\"");
    out.push_str(BODY_CLASS);
    out.push_str("\">");
    let mut quote_at = None;
    serialize(&tree, &mut out, &mut quote_at);
    out.push_str("</div>");
    SanitizedHtml {
        html: out,
        tree,
        remote_image_count: rw.remote_images,
        trackers_removed: rw.trackers,
        quote_html_start: quote_at,
        sanitizer_version: SANITIZER_VERSION,
    }
}

/// Sanitizes user-composed HTML for sending (signatures, snippets, the composer). Same allow-list,
/// but remote images keep their `src` (the recipient's client decides) and no wrapper or data
/// attributes are added.
pub fn sanitize_outgoing_html(html: &str) -> String {
    let html = truncate_utf8(html, MAX_HTML_BYTES);
    if estimated_depth(html) > MAX_PARSE_DEPTH {
        let mut out = String::new();
        escape_text(&crude_text(html), &mut out);
        return out;
    }
    let cleaned = BUILDER.clean(html).to_string();
    let raw = tokenize(&cleaned);
    let opts = SanitizeOptions { remote_images: RemoteImagePolicy::Original, cid_map: HashMap::new() };
    let mut rw = Rewriter { opts: &opts, remote_images: 0, trackers: 0, quote_marked: true };
    let tree = rw.nodes(raw);
    let mut out = String::with_capacity(cleaned.len());
    serialize_outgoing(&tree, &mut out);
    out
}

fn serialize_outgoing(nodes: &[SafeNode], out: &mut String) {
    for n in nodes {
        match n {
            SafeNode::Text(t) => escape_text(t, out),
            SafeNode::Stylesheet(_) => {}
            SafeNode::Element(e) => {
                out.push('<');
                out.push_str(&e.tag);
                for (k, v) in &e.attrs {
                    push_attr(out, k, v);
                }
                if !e.style.is_empty() {
                    push_attr(out, "style", &css::render_declarations(&e.style));
                }
                if let Some(href) = &e.href {
                    push_attr(out, "href", href);
                }
                match &e.image {
                    Some(ImageSource::Remote { url }) => push_attr(out, "src", url),
                    Some(ImageSource::Cid { part_id }) => push_attr(out, "src", &format!("cid:{part_id}")),
                    Some(ImageSource::Data { url }) => push_attr(out, "src", url),
                    None => {}
                }
                out.push('>');
                if VOID_TAGS.contains(&e.tag.as_str()) {
                    continue;
                }
                serialize_outgoing(&e.children, out);
                out.push_str("</");
                out.push_str(&e.tag);
                out.push('>');
            }
        }
    }
}

/// Converts sanitized HTML (or any HTML) to plain text for snippets and the text alternative.
pub fn html_to_text(html: &str) -> String {
    let html = truncate_utf8(html, MAX_HTML_BYTES);
    if estimated_depth(html) > MAX_PARSE_DEPTH {
        return crude_text(html);
    }
    let cleaned = BUILDER.clean(html).to_string();
    let raw = tokenize(&cleaned);
    let mut out = String::new();
    fn walk(nodes: &[RawNode], out: &mut String) {
        for n in nodes {
            match n {
                RawNode::Text(t) => out.push_str(t),
                RawNode::Element { name, children, .. } => {
                    if name == "style" {
                        continue;
                    }
                    let block = BLOCK_TAGS.contains(&name.as_str()) || name == "tr";
                    if name == "br" {
                        out.push('\n');
                    }
                    if block && !out.ends_with('\n') && !out.is_empty() {
                        out.push('\n');
                    }
                    walk(children, out);
                    if block && !out.ends_with('\n') {
                        out.push('\n');
                    }
                }
            }
        }
    }
    walk(&raw, &mut out);
    let mut lines: Vec<&str> = out.lines().map(str::trim_end).collect();
    while lines.last().is_some_and(|l| l.is_empty()) {
        lines.pop();
    }
    lines.join("\n").replace('\u{a0}', " ")
}
