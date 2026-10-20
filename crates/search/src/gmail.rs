//! Translation of the query grammar (§11.1) into a Gmail `q` string (§11.2 backend 2, "nearly
//! 1:1"). Used by the Gmail adapter in `crates/providers::gmail` for `GET /v1/search`.
//!
//! Rules (D-203):
//!
//! - Free text and phrases pass through; words that Gmail would read as an operator (`x:y`, a
//!   leading `-`, `OR`/`AND`/`AROUND`, brackets) are quoted. Gmail has no escape for `"` inside a
//!   phrase, so a `"` becomes a space.
//! - `from:`, `to:`, `cc:`, `subject:`, `list:`, `filename:` map to the Gmail operator of the same
//!   name. `has:attachment`, `is:unread|read|starred|important` and `in:inbox|sent|drafts|trash|
//!   spam|anywhere` too.
//! - `in:archive` is "no inbox, trash or spam" (§1.4): `(-in:inbox -in:trash -in:spam)`.
//! - `in:snoozed` / `is:snoozed` / `label:sys:snoozed` become the `Akatsuki/Snoozed` label.
//! - `label:` takes a name or a label key. `sys:*` keys map to the Gmail system operators; every
//!   other value goes through the caller's resolver (provider label name), and an unresolved `l:`
//!   key is an error because the adapter cannot guess a name from a Convex id.
//! - `before:` / `after:` use epoch seconds (Gmail accepts them; no time-zone ambiguity because
//!   the parser already resolved the instant). `older_than:` / `newer_than:` use `<days>d`.
//! - `account:` selects the account, which the worker has already done: a top-level conjunct is
//!   dropped (and reported in `dropped`); anywhere else it is unsupported.
//! - `category:` (Akatsuki triage), `is:needs_reply`, `in:<split>` and unknown values have no Gmail
//!   equivalent and are [`TranslateError::Unsupported`]; dropping them would silently widen or
//!   narrow the results.

use crate::query::{AgeField, DateField, Field, QueryAst};

/// The provider label behind `sys:snoozed` (D-037).
pub const SNOOZED_LABEL: &str = "Akatsuki/Snoozed";

/// A translated query.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GmailQuery {
    /// The `q` parameter; empty means "everything".
    pub q: String,
    /// Terms that were dropped because they do not apply at the provider (`account:`).
    pub dropped: Vec<String>,
}

/// Why a query cannot be sent to Gmail.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum TranslateError {
    /// The term has no Gmail equivalent.
    #[error("unsupported in Gmail search: {0}")]
    Unsupported(String),
    /// A `label:` value names a label key the resolver does not know.
    #[error("unknown label: {0}")]
    UnknownLabel(String),
}

/// Translates `ast`. `label_name` maps a `label:` value (a label key such as `l:<id>`, or a name
/// the user typed) to the Gmail label name; returning `None` for a plain name keeps the name.
pub fn to_gmail_q(ast: &QueryAst, label_name: &dyn Fn(&str) -> Option<String>) -> Result<GmailQuery, TranslateError> {
    let mut dropped = Vec::new();
    let top: Vec<&QueryAst> = match ast {
        QueryAst::And(items) => items.iter().collect(),
        other => vec![other],
    };
    let mut parts = Vec::new();
    for item in top {
        if let QueryAst::Field { field: Field::Account, value } = item {
            dropped.push(format!("account:{value}"));
            continue;
        }
        let s = term(item, label_name, Ctx::And)?;
        if !s.is_empty() {
            parts.push(s);
        }
    }
    Ok(GmailQuery { q: parts.join(" "), dropped })
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Ctx {
    /// A conjunct: no brackets needed.
    And,
    /// Inside `OR` or `-`: compound terms need brackets.
    Nested,
}

fn term(ast: &QueryAst, labels: &dyn Fn(&str) -> Option<String>, ctx: Ctx) -> Result<String, TranslateError> {
    Ok(match ast {
        QueryAst::And(items) => {
            let parts: Vec<String> = items.iter().map(|i| term(i, labels, Ctx::And)).collect::<Result<_, _>>()?;
            let parts: Vec<String> = parts.into_iter().filter(|p| !p.is_empty()).collect();
            if parts.len() > 1 && ctx == Ctx::Nested { format!("({})", parts.join(" ")) } else { parts.join(" ") }
        }
        QueryAst::Or(items) => {
            let parts: Vec<String> = items.iter().map(|i| term(i, labels, Ctx::Nested)).collect::<Result<_, _>>()?;
            let parts: Vec<String> = parts.into_iter().filter(|p| !p.is_empty()).collect();
            if parts.len() > 1 { format!("({})", parts.join(" OR ")) } else { parts.join("") }
        }
        QueryAst::Not(inner) => {
            let s = term(inner, labels, Ctx::Nested)?;
            if s.is_empty() { String::new() } else { format!("-{s}") }
        }
        QueryAst::Text { text, phrase } => {
            if *phrase {
                quoted(text)
            } else {
                word(text)
            }
        }
        QueryAst::Date { field, at } => {
            let secs = at.div_euclid(1000);
            match field {
                DateField::Before => format!("before:{secs}"),
                DateField::After => format!("after:{secs}"),
            }
        }
        QueryAst::Age { field, days } => match field {
            AgeField::OlderThan => format!("older_than:{days}d"),
            AgeField::NewerThan => format!("newer_than:{days}d"),
        },
        QueryAst::Field { field, value } => field_term(*field, value, labels, ctx)?,
    })
}

fn unsupported(field: Field, value: &str) -> TranslateError {
    TranslateError::Unsupported(format!("{}:{value}", field.as_str()))
}

fn field_term(
    field: Field,
    value: &str,
    labels: &dyn Fn(&str) -> Option<String>,
    ctx: Ctx,
) -> Result<String, TranslateError> {
    let archive = || {
        let s = "-in:inbox -in:trash -in:spam";
        if ctx == Ctx::Nested { format!("({s})") } else { s.to_owned() }
    };
    Ok(match field {
        Field::From | Field::To | Field::Cc | Field::Subject | Field::List | Field::Filename => {
            format!("{}:{}", field.as_str(), value_text(value))
        }
        Field::Has => match value {
            "attachment" => "has:attachment".into(),
            _ => return Err(unsupported(field, value)),
        },
        Field::Is => match value {
            "unread" | "read" | "starred" | "important" => format!("is:{value}"),
            "snoozed" => label_operator(SNOOZED_LABEL),
            _ => return Err(unsupported(field, value)),
        },
        Field::In => match value {
            "inbox" | "sent" | "drafts" | "trash" | "spam" | "anywhere" => format!("in:{value}"),
            "archive" => archive(),
            "snoozed" => label_operator(SNOOZED_LABEL),
            _ => return Err(unsupported(field, value)),
        },
        Field::Label => {
            if let Some(sys) = value.strip_prefix("sys:") {
                match sys {
                    "inbox" | "sent" | "drafts" | "trash" | "spam" => format!("in:{sys}"),
                    "unread" | "starred" | "important" => format!("is:{sys}"),
                    "snoozed" => label_operator(SNOOZED_LABEL),
                    _ => return Err(TranslateError::UnknownLabel(value.to_owned())),
                }
            } else if let Some(name) = labels(value) {
                label_operator(&name)
            } else if value.starts_with("l:") || value.starts_with("split:") {
                return Err(TranslateError::UnknownLabel(value.to_owned()));
            } else {
                label_operator(value)
            }
        }
        Field::Account | Field::Category => return Err(unsupported(field, value)),
    })
}

/// Gmail's search form of a label name, as its own UI writes it: lowercase, with spaces, `/`
/// and punctuation other than `-`, `_` and `.` replaced by `-`.
pub fn label_search_name(name: &str) -> String {
    name.chars()
        .map(|c| {
            if c.is_alphanumeric() || matches!(c, '-' | '_' | '.') { c.to_lowercase().next().unwrap_or(c) } else { '-' }
        })
        .collect()
}

fn label_operator(name: &str) -> String {
    format!("label:{}", label_search_name(name))
}

fn needs_quotes(s: &str) -> bool {
    s.is_empty()
        || s.starts_with('-')
        || s.starts_with('+')
        || matches!(s, "OR" | "AND" | "AROUND")
        || s.chars().any(|c| c.is_whitespace() || matches!(c, '(' | ')' | '{' | '}' | '"' | ':' | '[' | ']'))
}

fn quoted(s: &str) -> String {
    let inner: String = s.chars().map(|c| if c == '"' || c.is_control() { ' ' } else { c }).collect();
    let inner = inner.split_whitespace().collect::<Vec<_>>().join(" ");
    if inner.is_empty() { String::new() } else { format!("\"{inner}\"") }
}

fn word(s: &str) -> String {
    if needs_quotes(s) { quoted(s) } else { s.to_owned() }
}

fn value_text(s: &str) -> String {
    if needs_quotes(s) {
        let q = quoted(s);
        if q.is_empty() { "\"\"".into() } else { q }
    } else {
        s.to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::query::parse;

    const NOW: i64 = 1_790_000_000_000;

    fn q(input: &str) -> Result<GmailQuery, TranslateError> {
        let r = parse(input, NOW, "UTC");
        let labels = |v: &str| match v {
            "l:abc" => Some("Clients/Acme Corp".to_owned()),
            "work" | "Work" => Some("Work".to_owned()),
            _ => None,
        };
        to_gmail_q(&r.ast, &labels)
    }

    fn ok(input: &str) -> String {
        q(input).unwrap().q
    }

    #[test]
    fn fields_and_text() {
        assert_eq!(
            ok("from:alice@example.com subject:\"quarterly report\" budget"),
            "from:alice@example.com subject:\"quarterly report\" budget"
        );
        assert_eq!(
            ok("to:bob cc:carol list:news.example.com filename:pdf"),
            "to:bob cc:carol list:news.example.com filename:pdf"
        );
        assert_eq!(ok("has:attachment is:unread is:starred in:sent"), "has:attachment is:unread is:starred in:sent");
        assert_eq!(ok("\"exact words\""), "\"exact words\"");
        assert_eq!(ok(""), "");
    }

    #[test]
    fn operators_in_words_are_quoted() {
        assert_eq!(ok("foo:bar"), "\"foo:bar\"");
        assert_eq!(ok("\"OR\""), "\"OR\"");
        assert_eq!(ok("\"say \\\"hi\\\"\""), "\"say hi\"");
    }

    #[test]
    fn boolean_structure() {
        assert_eq!(ok("from:a OR from:b"), "(from:a OR from:b)");
        assert_eq!(ok("-from:a"), "-from:a");
        assert_eq!(ok("-(from:a subject:x)"), "-(from:a subject:x)");
        assert_eq!(ok("(from:a subject:x) OR to:b"), "((from:a subject:x) OR to:b)");
        assert_eq!(ok("in:archive"), "-in:inbox -in:trash -in:spam");
        assert_eq!(ok("-in:archive"), "-(-in:inbox -in:trash -in:spam)");
    }

    #[test]
    fn dates_and_ages() {
        let r = q("after:2026-01-01 before:2026/02/01 older_than:2w newer_than:1y").unwrap();
        // 2026-01-01T00:00:00Z = 1767225600, 2026-02-01T00:00:00Z = 1769904000.
        assert_eq!(r.q, "after:1767225600 before:1769904000 older_than:14d newer_than:365d");
    }

    #[test]
    fn labels() {
        assert_eq!(ok("label:l:abc"), "label:clients-acme-corp");
        assert_eq!(ok("label:Work"), "label:work");
        assert_eq!(ok("label:\"Some Name\""), "label:some-name");
        assert_eq!(ok("label:sys:inbox label:sys:unread"), "in:inbox is:unread");
        assert_eq!(ok("in:snoozed"), "label:akatsuki-snoozed");
        assert_eq!(ok("is:snoozed"), "label:akatsuki-snoozed");
        assert_eq!(q("label:l:unknown"), Err(TranslateError::UnknownLabel("l:unknown".into())));
        assert_eq!(label_search_name("Ünïcode/Sub Label"), "ünïcode-sub-label");
    }

    #[test]
    fn account_and_unsupported() {
        let r = q("account:me@example.com from:x").unwrap();
        assert_eq!(r.q, "from:x");
        assert_eq!(r.dropped, vec!["account:me@example.com"]);
        assert!(matches!(q("account:a OR from:x"), Err(TranslateError::Unsupported(_))));
        assert_eq!(q("is:needs_reply"), Err(TranslateError::Unsupported("is:needs_reply".into())));
        assert_eq!(q("category:receipts"), Err(TranslateError::Unsupported("category:receipts".into())));
        assert_eq!(q("in:Newsletters"), Err(TranslateError::Unsupported("in:Newsletters".into())));
    }
}
