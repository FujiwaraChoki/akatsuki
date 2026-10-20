//! Snippets in the composer (D-295): user templates from `snippets.list`, inserted at the caret
//! with Ctrl/⌘ ; (picker) or by typing `;shortcut` then Tab. Built-in variables are
//! `{{first_name}}`, `{{name}}`, `{{email}}` (first recipient) and `{{day}}`, `{{date}}` (today in
//! the user's zone); any other `{{variable}}` stays in the text and the composer says which to
//! fill. Snippet HTML is reduced to text for the plain-text editor and stored as simple HTML, the
//! same rules as `apps/web/src/compose/text.ts` (D-280).

use std::collections::BTreeMap;

use akatsuki_core::Addr;
use akatsuki_core::convex::client::SnippetView;
use jiff::Timestamp;
use jiff::tz::TimeZone;

/// The longest `;shortcut` (Convex: `[a-z0-9][a-z0-9_-]{0,31}`).
pub const SHORTCUT_MAX: usize = 32;

fn is_var_start(c: char) -> bool {
    c.is_ascii_alphabetic()
}

fn is_var_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_'
}

/// One `{{ name }}` occurrence: byte range and the lowercased name.
fn variables(template: &str) -> Vec<(std::ops::Range<usize>, String)> {
    let mut out = Vec::new();
    let mut from = 0;
    while let Some(open) = template[from..].find("{{").map(|i| from + i) {
        let Some(close) = template[open + 2..].find("}}").map(|i| open + 2 + i) else { break };
        let inner = template[open + 2..close].trim();
        let ok = inner.chars().next().is_some_and(is_var_start)
            && inner.chars().all(is_var_char)
            && inner.chars().count() <= 40;
        if ok {
            out.push((open..close + 2, inner.to_ascii_lowercase()));
            from = close + 2;
        } else {
            from = open + 2;
        }
    }
    out
}

/// Variable names a template uses, in order of first use.
pub fn variables_in(template: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for (_, name) in variables(template) {
        if !out.contains(&name) {
            out.push(name);
        }
    }
    out
}

/// Fills `{{var}}`; unknown or empty variables stay as typed and are listed in `missing`.
pub fn expand_variables(template: &str, vars: &BTreeMap<String, String>) -> (String, Vec<String>) {
    let mut text = String::with_capacity(template.len());
    let mut missing: Vec<String> = Vec::new();
    let mut last = 0;
    for (range, name) in variables(template) {
        text.push_str(&template[last..range.start]);
        match vars.get(&name).filter(|v| !v.is_empty()) {
            Some(v) => text.push_str(v),
            None => {
                text.push_str(&template[range.clone()]);
                if !missing.contains(&name) {
                    missing.push(name);
                }
            }
        }
        last = range.end;
    }
    text.push_str(&template[last..]);
    (text, missing)
}

/// The first name of an address: `Okafor, Mira` → `Mira`, `Mira Okafor` → `Mira`, else the
/// mailbox (`jonas.weber@…` → `Jonas`).
pub fn first_name(a: Option<&Addr>) -> String {
    let Some(a) = a else { return String::new() };
    if let Some(n) = a.name.as_deref().map(str::trim).filter(|n| !n.is_empty()) {
        return match n.split_once(',') {
            Some((_, first)) if !first.trim().is_empty() => first.trim().to_owned(),
            _ => n.split_whitespace().next().unwrap_or(n).to_owned(),
        };
    }
    let local = a.email.split('@').next().unwrap_or("");
    let part = local.split(['.', '_', '-']).next().unwrap_or("");
    let mut chars = part.chars();
    match chars.next() {
        Some(c) => c.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

/// Built-in variables from the first recipient and the clock, in the user's zone.
pub fn builtin_variables(to: &[Addr], now_ms: i64, tz: &TimeZone) -> BTreeMap<String, String> {
    let first = to.first();
    let mut vars = BTreeMap::new();
    vars.insert("first_name".to_owned(), first_name(first));
    vars.insert("name".to_owned(), first.and_then(|a| a.name.clone()).unwrap_or_else(|| first_name(first)));
    vars.insert("email".to_owned(), first.map(|a| a.email.clone()).unwrap_or_default());
    if let Ok(t) = Timestamp::from_millisecond(now_ms) {
        let z = t.to_zoned(tz.clone());
        vars.insert("day".to_owned(), z.strftime("%A").to_string());
        vars.insert("date".to_owned(), z.strftime("%B %-d").to_string());
    }
    vars
}

/// The `;shortcut` typed right before the caret, as `(start of ';', shortcut)`. The `;` must start
/// the text or follow whitespace, so `a;b` in prose is not a trigger.
pub fn trigger_before(text: &str, caret: usize) -> Option<(usize, String)> {
    let caret = caret.min(text.len());
    if !text.is_char_boundary(caret) {
        return None;
    }
    let before = &text[..caret];
    let semi = before.rfind(';')?;
    let word = &before[semi + 1..];
    let valid = !word.is_empty()
        && word.len() <= SHORTCUT_MAX
        && word.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-');
    let boundary = before[..semi].chars().next_back().is_none_or(char::is_whitespace);
    (valid && boundary).then(|| (semi, word.to_ascii_lowercase()))
}

/// The snippet whose shortcut matches (case-insensitive).
pub fn by_shortcut<'a>(snippets: &'a [SnippetView], shortcut: &str) -> Option<&'a SnippetView> {
    snippets.iter().find(|s| s.shortcut.as_deref().is_some_and(|x| x.eq_ignore_ascii_case(shortcut)))
}

/// Picker order: most used first, then recently used, then by name.
pub fn ordered(snippets: &[SnippetView]) -> Vec<SnippetView> {
    let mut v = snippets.to_vec();
    v.sort_by(|a, b| {
        b.use_count
            .total_cmp(&a.use_count)
            .then(b.last_used_at.unwrap_or(0.0).total_cmp(&a.last_used_at.unwrap_or(0.0)))
            .then(a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    v
}

/// What inserting a snippet does to the draft.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Expansion {
    /// Text to insert at the caret.
    pub text: String,
    /// Subject to set (only when the draft has none).
    pub subject: Option<String>,
    /// Variables left for the user to fill.
    pub missing: Vec<String>,
}

/// Expands a snippet for a draft with recipients `to` and subject `current_subject`.
pub fn expand(snippet: &SnippetView, to: &[Addr], current_subject: &str, now_ms: i64, tz: &TimeZone) -> Expansion {
    let vars = builtin_variables(to, now_ms, tz);
    let (text, missing) = expand_variables(&html_to_text(&snippet.body_html), &vars);
    let subject = snippet
        .subject
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty() && current_subject.trim().is_empty())
        .map(|s| expand_variables(s, &vars).0);
    Expansion { text, subject, missing }
}

/// `{{a}}, {{b}}` for the "fill in" notice.
pub fn missing_notice(missing: &[String]) -> Option<String> {
    (!missing.is_empty())
        .then(|| format!("Fill in {}.", missing.iter().map(|m| format!("{{{{{m}}}}}")).collect::<Vec<_>>().join(", ")))
}

fn escape_html(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(c),
        }
    }
    out
}

/// Plain text → simple HTML (paragraphs on blank lines, `<br>` for single newlines, `>` lines as a
/// blockquote), everything escaped; the web's `textToHtml` without autolinking.
pub fn text_to_html(text: &str) -> String {
    let normalized = text.replace("\r\n", "\n").replace('\r', "\n");
    let mut out: Vec<String> = Vec::new();
    for block in normalized.split("\n\n") {
        if block.trim().is_empty() {
            continue;
        }
        let mut run: Vec<&str> = Vec::new();
        let mut quoted = false;
        let flush = |run: &mut Vec<&str>, quoted: bool, out: &mut Vec<String>| {
            if run.is_empty() {
                return;
            }
            if quoted {
                let inner: Vec<String> =
                    run.iter().map(|l| l.trim_start_matches('>').trim_start().to_owned()).collect();
                out.push(format!("<blockquote>{}</blockquote>", text_to_html(&inner.join("\n"))));
            } else {
                out.push(format!("<p>{}</p>", run.iter().map(|l| escape_html(l)).collect::<Vec<_>>().join("<br>")));
            }
            run.clear();
        };
        for line in block.split('\n') {
            let q = line.starts_with('>');
            if q != quoted {
                flush(&mut run, quoted, &mut out);
                quoted = q;
            }
            run.push(line);
        }
        flush(&mut run, quoted, &mut out);
    }
    out.join("\n")
}

/// Snippet or signature HTML → text for the editor: `<br>` is a newline, block ends are blank
/// lines, tags are dropped and the common entities decoded (the web's non-DOM `htmlToText`).
pub fn html_to_text(html: &str) -> String {
    let mut s = html.to_owned();
    for tag in ["script", "style"] {
        loop {
            let lower = s.to_ascii_lowercase();
            let Some(start) = lower.find(&format!("<{tag}")) else { break };
            let end = lower[start..].find(&format!("</{tag}>")).map_or(s.len(), |e| start + e + tag.len() + 3);
            s.replace_range(start..end, "");
        }
    }
    let mut out = String::with_capacity(s.len());
    let mut rest = s.as_str();
    while let Some(lt) = rest.find('<') {
        out.push_str(&rest[..lt]);
        let Some(gt) = rest[lt..].find('>') else {
            rest = "";
            break;
        };
        let tag = rest[lt + 1..lt + gt].trim().to_ascii_lowercase();
        let name: String = tag.trim_start_matches('/').chars().take_while(char::is_ascii_alphanumeric).collect();
        if name == "br" {
            out.push('\n');
        } else if tag.starts_with('/')
            && matches!(name.as_str(), "p" | "div" | "li" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6" | "blockquote")
        {
            out.push_str("\n\n");
        }
        rest = &rest[lt + gt + 1..];
    }
    out.push_str(rest);
    let decoded = out
        .replace("&nbsp;", " ")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&amp;", "&");
    let mut collapsed = String::with_capacity(decoded.len());
    let mut newlines = 0;
    for c in decoded.chars() {
        if c == '\n' {
            newlines += 1;
            if newlines > 2 {
                continue;
            }
        } else {
            newlines = 0;
        }
        collapsed.push(c);
    }
    collapsed.trim().to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn snippet(v: serde_json::Value) -> SnippetView {
        serde_json::from_value(v).unwrap()
    }

    #[test]
    fn variables_are_found_expanded_and_reported() {
        let t = "Hi {{ First_Name }}, see {{calendar_link}} by {{day}}. {{first_name}} {{ 9bad }} {{";
        assert_eq!(variables_in(t), vec!["first_name", "calendar_link", "day"]);
        let mut vars = BTreeMap::new();
        vars.insert("first_name".to_owned(), "Mira".to_owned());
        vars.insert("day".to_owned(), String::new());
        let (text, missing) = expand_variables(t, &vars);
        assert_eq!(text, "Hi Mira, see {{calendar_link}} by {{day}}. Mira {{ 9bad }} {{");
        assert_eq!(missing, vec!["calendar_link", "day"], "empty values count as missing");
        assert_eq!(missing_notice(&missing).as_deref(), Some("Fill in {{calendar_link}}, {{day}}."));
        assert_eq!(missing_notice(&[]), None);
    }

    #[test]
    fn first_names_come_from_the_name_or_the_mailbox() {
        assert_eq!(first_name(Some(&Addr::new(Some("Mira Okafor"), "m@x.example"))), "Mira");
        assert_eq!(first_name(Some(&Addr::new(Some("Okafor, Mira"), "m@x.example"))), "Mira");
        assert_eq!(first_name(Some(&Addr::new(None, "jonas.weber@x.example"))), "Jonas");
        assert_eq!(first_name(None), "");
    }

    #[test]
    fn builtins_resolve_in_the_users_zone() {
        let tz = TimeZone::get("Asia/Tokyo").unwrap();
        // 2026-10-20 23:30 UTC is Wednesday the 21st in Tokyo.
        let now = "2026-10-20T23:30:00Z".parse::<Timestamp>().unwrap().as_millisecond();
        let v = builtin_variables(&[Addr::new(Some("Hana Ito"), "hana@kitsune-mail.example")], now, &tz);
        assert_eq!(v["day"], "Wednesday");
        assert_eq!(v["date"], "October 21");
        assert_eq!(v["name"], "Hana Ito");
        assert_eq!(v["email"], "hana@kitsune-mail.example");
        let none = builtin_variables(&[], now, &tz);
        assert_eq!(none["first_name"], "");
    }

    #[test]
    fn shortcuts_trigger_only_at_a_word_start() {
        assert_eq!(trigger_before("Thanks ;thx", 11), Some((7, "thx".into())));
        assert_eq!(trigger_before(";Intro", 6), Some((0, "intro".into())));
        assert_eq!(trigger_before("a;thx", 5), None, "inside a word");
        assert_eq!(trigger_before("see ; thx", 9), None, "space after the semicolon");
        assert_eq!(trigger_before("x ;thx more", 6), Some((2, "thx".into())), "caret right after the shortcut");
        assert_eq!(trigger_before("x ;", 3), None);
        assert_eq!(trigger_before("é ;thx", 99), Some((3, "thx".into())), "caret clamps to the end");
    }

    #[test]
    fn snippets_expand_with_subject_and_order_by_use() {
        let tz = TimeZone::UTC;
        let now = "2026-10-20T08:00:00Z".parse::<Timestamp>().unwrap().as_millisecond();
        let s = snippet(json!({"_id": "s1", "_creationTime": 1.0, "name": "Intro", "shortcut": "intro",
            "subject": "Intro for {{first_name}}", "bodyHtml": "<p>Hi {{first_name}},</p><p>Call {{day}}? {{link}}</p>",
            "variables": ["first_name", "day", "link"], "useCount": 1, "updatedAt": 1}));
        let to = [Addr::new(Some("Mira Okafor"), "mira@northwind.example")];
        let e = expand(&s, &to, "", now, &tz);
        assert_eq!(e.text, "Hi Mira,\n\nCall Tuesday? {{link}}");
        assert_eq!(e.subject.as_deref(), Some("Intro for Mira"));
        assert_eq!(e.missing, vec!["link"]);
        assert_eq!(expand(&s, &to, "Existing", now, &tz).subject, None, "a typed subject is kept");

        let a = snippet(json!({"_id": "a", "_creationTime": 1.0, "name": "Alpha", "bodyHtml": "", "variables": [],
            "useCount": 0, "updatedAt": 1}));
        let b = snippet(json!({"_id": "b", "_creationTime": 1.0, "name": "beta", "shortcut": "B", "bodyHtml": "",
            "variables": [], "useCount": 5, "updatedAt": 1}));
        let names: Vec<String> = ordered(&[a.clone(), s.clone(), b.clone()]).into_iter().map(|x| x.name).collect();
        assert_eq!(names, vec!["beta", "Intro", "Alpha"]);
        let list = [a, b];
        assert_eq!(by_shortcut(&list, "b").map(|x| x.name.as_str()), Some("beta"));
        assert!(by_shortcut(&list, "zzz").is_none());
    }

    #[test]
    fn html_and_text_convert_like_the_web() {
        assert_eq!(
            html_to_text("<p>Hi&nbsp;there,</p><p>Line<br>two &amp; <b>three</b></p><script>x()</script>"),
            "Hi there,\n\nLine\ntwo & three"
        );
        assert_eq!(html_to_text("<div>a</div><div></div><div></div><div>b</div>"), "a\n\nb");
        assert_eq!(html_to_text("<style>p{}</style>plain"), "plain");
        assert_eq!(
            text_to_html("Hi <you>,\nline two\n\n> quoted\n> more\nafter"),
            "<p>Hi &lt;you&gt;,<br>line two</p>\n<blockquote><p>quoted<br>more</p></blockquote>\n<p>after</p>"
        );
        assert_eq!(html_to_text(&text_to_html("One\n\nTwo\nThree")), "One\n\nTwo\nThree", "round trip");
    }
}
