//! Shared renderers for prompt variables, the Rust twins of `packages/ai/src/transcript.ts`. The
//! spec is `packages/prompts/shared/transcript.md`; both implementations must give byte-identical
//! output for `packages/prompts/shared/fixtures/transcript/*.json` and
//! `shared/fixtures/triage-items/*.json`.
//!
//! "Character" means a Unicode scalar value (`chars()` here, `Array.from` in TypeScript).

use akatsuki_core::model::Addr;
use jiff::Timestamp;
use jiff::tz::TimeZone;
use serde::{Deserialize, Serialize};

/// Per-message text cap.
pub const MESSAGE_TEXT_MAX: usize = 8_000;
/// Default `maxInputChars`.
pub const DEFAULT_MAX_INPUT_CHARS: usize = 120_000;
/// Appended where text is cut.
pub const CUT_MARKER: &str = "\n[…]";
/// Most triage items per batch.
pub const TRIAGE_BATCH_MAX: usize = 25;

/// An address as the renderers take it (`name` may be missing or blank).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RenderAddr {
    /// Display name.
    #[serde(default)]
    pub name: Option<String>,
    /// Address.
    pub email: String,
}

impl From<&Addr> for RenderAddr {
    fn from(a: &Addr) -> Self {
        Self { name: a.name.clone(), email: a.email.clone() }
    }
}

/// One message of `ThreadForTranscript`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TranscriptMessage {
    /// Sender.
    #[serde(default)]
    pub from: Option<RenderAddr>,
    /// To.
    pub to: Vec<RenderAddr>,
    /// Cc.
    #[serde(default)]
    pub cc: Vec<RenderAddr>,
    /// ms since the epoch.
    pub date: i64,
    /// Plain text with quoted history already stripped (see [`strip_quoted`]).
    pub text: String,
}

/// `ThreadForTranscript`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadForTranscript {
    /// IANA zone (`profiles.timezone`).
    pub timezone: String,
    /// Cap; defaults to [`DEFAULT_MAX_INPUT_CHARS`].
    #[serde(default)]
    pub max_input_chars: Option<usize>,
    /// Messages, any order.
    pub messages: Vec<TranscriptMessage>,
}

/// The one whitespace set of both renderers (here and `packages/ai/src/transcript.ts`): Unicode
/// `White_Space` ([`char::is_whitespace`]) plus U+FEFF. JavaScript's `trim` and `\s` include U+FEFF
/// but not U+0085 and Rust's `trim` the reverse, so neither side uses its built-in set (D-342).
pub fn is_render_whitespace(c: char) -> bool {
    c.is_whitespace() || c == '\u{FEFF}'
}

fn trim_ws(s: &str) -> &str {
    s.trim_matches(is_render_whitespace)
}

fn char_len(s: &str) -> usize {
    s.chars().count()
}

/// The first `max` characters of `s`.
pub fn cut_chars(s: &str, max: usize) -> &str {
    match s.char_indices().nth(max) {
        Some((i, _)) => &s[..i],
        None => s,
    }
}

/// `Name <email>` when the trimmed name is non-empty, otherwise `email`; missing → empty.
pub fn format_addr(a: Option<&RenderAddr>) -> String {
    let Some(a) = a else { return String::new() };
    match a.name.as_deref().map(trim_ws).filter(|n| !n.is_empty()) {
        Some(name) => format!("{name} <{}>", a.email),
        None => a.email.clone(),
    }
}

/// `YYYY-MM-DDTHH:mm:ss±HH:MM` in `tz` (never `Z`); UTC when the zone is unknown.
pub fn format_instant(ms: i64, tz: &str) -> String {
    let zone = TimeZone::get(tz).unwrap_or(TimeZone::UTC);
    match Timestamp::from_millisecond(ms) {
        Ok(ts) => ts.to_zoned(zone).strftime("%Y-%m-%dT%H:%M:%S%:z").to_string(),
        Err(_) => String::new(),
    }
}

fn normalize_text(text: &str) -> String {
    let t = text.replace("\r\n", "\n").replace('\r', "\n");
    let t = t.trim_end_matches(is_render_whitespace);
    if char_len(t) > MESSAGE_TEXT_MAX {
        format!("{}{CUT_MARKER}", cut_chars(t, MESSAGE_TEXT_MAX))
    } else {
        t.to_owned()
    }
}

fn render_message(m: &TranscriptMessage, tz: &str) -> String {
    let join = |list: &[RenderAddr]| list.iter().map(|a| format_addr(Some(a))).collect::<Vec<_>>().join(", ");
    let mut lines = vec![
        format!("From: {}", format_addr(m.from.as_ref())),
        format!("Date: {}", format_instant(m.date, tz)),
        format!("To: {}", join(&m.to)),
    ];
    if !m.cc.is_empty() {
        lines.push(format!("Cc: {}", join(&m.cc)));
    }
    format!("{}\n\n{}", lines.join("\n"), normalize_text(&m.text))
}

/// Renders `{{transcript}}`: oldest first (stable), `\n---\n` between messages, the oldest dropped
/// until it fits `maxInputChars`; a single message that still does not fit is cut and marked.
pub fn render_transcript(thread: &ThreadForTranscript) -> String {
    let max = thread.max_input_chars.unwrap_or(DEFAULT_MAX_INPUT_CHARS);
    let mut sorted: Vec<&TranscriptMessage> = thread.messages.iter().collect();
    sorted.sort_by_key(|m| m.date); // stable
    let rendered: Vec<String> = sorted.iter().map(|m| render_message(m, &thread.timezone)).collect();
    let sizes: Vec<usize> = rendered.iter().map(|r| char_len(r)).collect();
    let mut start = 0;
    let mut total: usize = sizes.iter().sum::<usize>() + rendered.len().saturating_sub(1) * 5;
    while total > max && start + 1 < rendered.len() {
        total -= sizes[start] + 5;
        start += 1;
    }
    let out = rendered[start.min(rendered.len())..].join("\n---\n");
    if char_len(&out) <= max {
        return out;
    }
    format!("{}{CUT_MARKER}", cut_chars(&out, max.saturating_sub(char_len(CUT_MARKER))))
}

/// Head-room kept below `maxInputChars` when budgeting a transcript (`TRANSCRIPT_BUDGET_MARGIN` in
/// `packages/ai`); it also covers the 4-character cut marker on a tiny budget.
pub const TRANSCRIPT_BUDGET_MARGIN: usize = 200;

/// Characters left for `{{transcript}}`: `max_input_chars` minus the length of every other string var
/// minus [`TRANSCRIPT_BUDGET_MARGIN`], never below 0 (`transcriptBudget` in `packages/ai`). The cap is on
/// the sum of all string vars, so a transcript rendered at the full cap would never fit next to them.
/// The `transcript` key itself is ignored.
pub fn transcript_budget(max_input_chars: usize, vars: &serde_json::Map<String, serde_json::Value>) -> usize {
    let used: usize =
        vars.iter().filter(|(k, _)| k.as_str() != "transcript").filter_map(|(_, v)| v.as_str()).map(char_len).sum();
    max_input_chars.saturating_sub(used).saturating_sub(TRANSCRIPT_BUDGET_MARGIN)
}

/// Strips quoted history from a plain-text body before it goes into a transcript.
pub fn strip_quoted(text: &str) -> &str {
    match akatsuki_mime::quote::text_quote_start(text) {
        Some(at) => &text[..at],
        None => text,
    }
}

/// One triage item (`TriageInput`: headers and snippet only, never body text).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TriageItem {
    /// Convex thread id.
    pub thread_id: String,
    /// Sender.
    #[serde(default)]
    pub from: Option<RenderAddr>,
    /// Recipients.
    pub to: Vec<RenderAddr>,
    /// Subject.
    pub subject: String,
    /// Snippet.
    pub snippet: String,
    /// `List-Id`.
    #[serde(default)]
    pub list_id: Option<String>,
    /// Bulk.
    pub is_bulk: bool,
    /// Attachments.
    pub has_attachments: bool,
}

/// Collapses every run of the shared whitespace set to one space and trims (`oneLine`).
fn one_line(s: &str) -> String {
    s.split(is_render_whitespace).filter(|p| !p.is_empty()).collect::<Vec<_>>().join(" ")
}

/// Renders `{{items}}` for triage. Keys are `"1"`, `"2"`, …; returns the text and key → thread id.
pub fn render_triage_items(items: &[TriageItem]) -> Result<(String, Vec<(String, String)>), crate::AiError> {
    if items.len() > TRIAGE_BATCH_MAX {
        return Err(crate::AiError::InvalidVars(format!("at most {TRIAGE_BATCH_MAX} triage items")));
    }
    let mut keys = Vec::new();
    let blocks: Vec<String> = items
        .iter()
        .enumerate()
        .map(|(i, it)| {
            let key = (i + 1).to_string();
            keys.push((key.clone(), it.thread_id.clone()));
            let to = it.to.iter().map(|a| format_addr(Some(a))).collect::<Vec<_>>().join(", ");
            let mut lines = vec![
                format!("key: {key}"),
                format!("from: {}", one_line(&format_addr(it.from.as_ref()))),
                format!("to: {}", one_line(&to)),
                format!("subject: {}", one_line(&it.subject)),
            ];
            if let Some(l) = it.list_id.as_deref().filter(|l| !l.is_empty()) {
                lines.push(format!("list-id: {}", one_line(l)));
            }
            lines.push(format!("bulk: {}", if it.is_bulk { "yes" } else { "no" }));
            lines.push(format!("attachments: {}", if it.has_attachments { "yes" } else { "no" }));
            lines.push(format!("snippet: {}", one_line(&it.snippet)));
            lines.join("\n")
        })
        .collect();
    Ok((blocks.join("\n\n"), keys))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture_dir(sub: &str) -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../packages/prompts/shared/fixtures").join(sub)
    }

    fn cases(sub: &str) -> Vec<(String, serde_json::Value)> {
        let entries = std::fs::read_dir(fixture_dir(sub)).unwrap_or_else(|e| panic!("fixtures/{sub}: {e}"));
        let mut out: Vec<_> = entries
            .flatten()
            .filter(|e| e.path().extension().is_some_and(|x| x == "json"))
            .map(|e| {
                (
                    e.path().display().to_string(),
                    serde_json::from_str(&std::fs::read_to_string(e.path()).unwrap()).unwrap(),
                )
            })
            .collect();
        out.sort_by(|a, b| a.0.cmp(&b.0));
        assert!(!out.is_empty(), "no shared fixtures found");
        out
    }

    #[test]
    fn shared_transcript_fixtures() {
        for (name, case) in cases("transcript") {
            let thread: ThreadForTranscript = serde_json::from_value(case["input"].clone()).unwrap();
            assert_eq!(render_transcript(&thread), case["expected"].as_str().unwrap(), "{name}");
        }
    }

    #[test]
    fn shared_triage_fixtures() {
        for (name, case) in cases("triage-items") {
            let items: Vec<TriageItem> = serde_json::from_value(case["input"].clone()).unwrap();
            let (text, keys) = render_triage_items(&items).unwrap();
            assert_eq!(text, case["expected"]["text"].as_str().unwrap(), "{name}");
            for (k, id) in keys {
                assert_eq!(case["expected"]["keys"][&k], id.as_str(), "{name}");
            }
        }
    }

    #[test]
    fn shared_whitespace_set() {
        // packages/prompts/shared/fixtures/whitespace.json; the same check runs in packages/ai.
        let path = fixture_dir("whitespace.json");
        let fx: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
        let want: Vec<u32> = fx["codePoints"]
            .as_array()
            .unwrap()
            .iter()
            .map(|c| u32::from_str_radix(c.as_str().unwrap().trim_start_matches("U+"), 16).unwrap())
            .collect();
        let got: Vec<u32> =
            (0..=0x10_FFFF).filter_map(char::from_u32).filter(|c| is_render_whitespace(*c)).map(u32::from).collect();
        assert_eq!(got, want);
        assert_eq!(one_line("\u{FEFF}a\u{85}b\u{3000} "), "a b");
    }

    #[test]
    fn format_rules() {
        let thread = ThreadForTranscript {
            timezone: "UTC".into(),
            max_input_chars: None,
            messages: vec![TranscriptMessage {
                from: Some(RenderAddr { name: Some("  ".into()), email: "a@x.com".into() }),
                to: vec![],
                cc: vec![],
                date: 0,
                text: "x\r\ny\r\n\r\n".into(),
            }],
        };
        assert_eq!(render_transcript(&thread), "From: a@x.com\nDate: 1970-01-01T00:00:00+00:00\nTo: \n\nx\ny");
        assert_eq!(cut_chars("a😀b", 2), "a😀");
        assert_eq!(strip_quoted("Yes\n\nOn Mon, A <a@x> wrote:\n> q\n"), "Yes\n\n");
    }
}
