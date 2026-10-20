//! Akatsuki query grammar (ARCHITECTURE §11.1) → Microsoft Graph `$search` KQL (§11.2 backend 2).
//!
//! Graph's message search accepts KQL over the properties `from`, `to`, `cc`, `bcc`,
//! `participants`, `recipients`, `subject`, `body`, `attachment`, `hasattachments`,
//! `importance`, `category`, `kind`, `received`, `sent` and `size`. It cannot be combined with
//! `$filter`, so state-only terms (`is:unread`, `is:starred`, snooze, triage `category:`) and
//! `list:` have no translation and are reported as unsupported, which makes the UI say the filter
//! is not available in Outlook search instead of returning wrong results (D-215).
//!
//! - Free text and phrases search everywhere (KQL default properties).
//! - `label:` → `category:` (Akatsuki labels on Outlook are categories).
//! - `filename:` → `attachment:`; `has:attachment` → `hasattachments:true`;
//!   `is:important` → `importance:high`.
//! - `before:`/`after:` → `received<D` / `received>=D`, where `D` is the calendar date the
//!   parser's start-of-day instant falls on (rounded to the nearest UTC midnight, so any zone
//!   within ±12 h keeps its date). `older_than:`/`newer_than:` use `now` the same way.
//! - A top-level `in:` (inbox, sent, drafts, trash, spam, archive) scopes the request to that
//!   folder; `in:anywhere` and `account:` at the top level are no-ops (the request is already
//!   scoped to one account). Elsewhere they are unsupported.

use akatsuki_search::query::{AgeField, DateField, Field, QueryAst};

use super::map::format_date;

const DAY_MS: i64 = 86_400_000;

/// A translated query.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KqlQuery {
    /// The KQL text for `$search` (`None` lists the folder newest first).
    pub search: Option<String>,
    /// A well-known folder to scope the request to (`inbox`, `sentitems`, …).
    pub folder: Option<&'static str>,
}

/// Why a query cannot be sent to Graph.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum KqlError {
    /// The term has no KQL equivalent.
    #[error("not supported by Outlook search: {0}")]
    Unsupported(&'static str),
}

fn folder_of(value: &str) -> Result<Option<&'static str>, KqlError> {
    Ok(Some(match value {
        "inbox" => "inbox",
        "sent" => "sentitems",
        "drafts" => "drafts",
        "trash" => "deleteditems",
        "spam" => "junkemail",
        "archive" => "archive",
        "anywhere" => return Ok(None),
        "snoozed" => return Err(KqlError::Unsupported("in:snoozed")),
        _ => return Err(KqlError::Unsupported("in:<split>")),
    }))
}

/// Quotes a value when it is not a plain token. Embedded double quotes cannot be escaped inside
/// KQL phrases, so they become spaces (the search is a match, not an exact comparison).
fn value(v: &str, force_quote: bool) -> String {
    let cleaned: String = v.chars().map(|c| if c == '"' || c.is_control() { ' ' } else { c }).collect();
    let cleaned = cleaned.split_whitespace().collect::<Vec<_>>().join(" ");
    let plain = !cleaned.is_empty()
        && cleaned.chars().all(|c| c.is_alphanumeric() || matches!(c, '@' | '.' | '-' | '_' | '+' | '\''))
        && !matches!(cleaned.as_str(), "AND" | "OR" | "NOT" | "NEAR");
    if plain && !force_quote { cleaned } else { format!("\"{cleaned}\"") }
}

fn day(at: i64) -> String {
    format_date((at + DAY_MS / 2).div_euclid(DAY_MS) * DAY_MS)
}

fn term(ast: &QueryAst, now: i64) -> Result<Option<String>, KqlError> {
    Ok(Some(match ast {
        QueryAst::And(items) | QueryAst::Or(items) => {
            let parts: Vec<String> =
                items.iter().map(|i| term(i, now)).collect::<Result<Vec<_>, _>>()?.into_iter().flatten().collect();
            match parts.len() {
                0 => return Ok(None),
                1 => parts.into_iter().next().unwrap_or_default(),
                _ => {
                    let op = if matches!(ast, QueryAst::And(_)) { " AND " } else { " OR " };
                    format!("({})", parts.join(op))
                }
            }
        }
        QueryAst::Not(inner) => match term(inner, now)? {
            Some(t) => format!("NOT {t}"),
            None => return Err(KqlError::Unsupported("negated empty term")),
        },
        QueryAst::Text { text, phrase } => value(text, *phrase),
        QueryAst::Field { field, value: v } => match field {
            Field::From => format!("from:{}", value(v, false)),
            Field::To => format!("to:{}", value(v, false)),
            Field::Cc => format!("cc:{}", value(v, false)),
            Field::Subject => format!("subject:{}", value(v, false)),
            Field::Label => format!("category:{}", value(v, false)),
            Field::Filename => format!("attachment:{}", value(v, false)),
            Field::Has if v == "attachment" => "hasattachments:true".to_owned(),
            Field::Has => return Err(KqlError::Unsupported("has:")),
            Field::Is if v == "important" => "importance:high".to_owned(),
            Field::Is => return Err(KqlError::Unsupported("is:")),
            Field::In => return Err(KqlError::Unsupported("in: inside a group")),
            Field::Account => return Err(KqlError::Unsupported("account: inside a group")),
            Field::List => return Err(KqlError::Unsupported("list:")),
            Field::Category => return Err(KqlError::Unsupported("category:")),
        },
        QueryAst::Date { field, at } => match field {
            DateField::Before => format!("received<{}", day(*at)),
            DateField::After => format!("received>={}", day(*at)),
        },
        QueryAst::Age { field, days } => {
            let at = now - i64::from(*days) * DAY_MS;
            match field {
                AgeField::OlderThan => format!("received<{}", day(at)),
                AgeField::NewerThan => format!("received>={}", day(at)),
            }
        }
    }))
}

/// Translates a parsed query. `now` resolves `older_than:`/`newer_than:`.
pub fn to_kql(ast: &QueryAst, now: i64) -> Result<KqlQuery, KqlError> {
    let top: Vec<&QueryAst> = match ast {
        QueryAst::And(items) => items.iter().collect(),
        other => vec![other],
    };
    let mut folder: Option<Option<&'static str>> = None;
    let mut rest = Vec::new();
    for t in top {
        match t {
            QueryAst::Field { field: Field::In, value } => {
                let f = folder_of(value)?;
                if folder.is_some_and(|prev| prev != f) {
                    return Err(KqlError::Unsupported("two in: folders"));
                }
                folder = Some(f);
            }
            QueryAst::Field { field: Field::Account, .. } => {}
            other => rest.push(other.clone()),
        }
    }
    let search = term(&QueryAst::And(rest), now)?;
    Ok(KqlQuery { search, folder: folder.flatten() })
}

/// The `$search` parameter value: the KQL wrapped in double quotes, with `\` and `"` escaped.
pub fn search_param(kql: &str) -> String {
    format!("\"{}\"", kql.replace('\\', "\\\\").replace('"', "\\\""))
}

#[cfg(test)]
mod tests {
    use super::*;
    use akatsuki_search::query::parse;

    const NOW: i64 = 1_790_812_800_000; // 2026-10-01T00:00:00Z

    fn kql(q: &str) -> Result<KqlQuery, KqlError> {
        let p = parse(q, NOW, "Europe/Zurich");
        to_kql(&p.ast, NOW)
    }

    #[test]
    fn fields_and_text() {
        let k = kql("from:ada@example.com subject:\"quarterly plan\" invoice").unwrap();
        assert_eq!(k.search.as_deref(), Some("(from:ada@example.com AND subject:\"quarterly plan\" AND invoice)"));
        assert_eq!(k.folder, None);
        assert_eq!(kql("\"exact words\"").unwrap().search.as_deref(), Some("\"exact words\""));
        assert_eq!(
            kql("label:Work has:attachment").unwrap().search.as_deref(),
            Some("(category:Work AND hasattachments:true)")
        );
        assert_eq!(
            kql("filename:report.pdf is:important").unwrap().search.as_deref(),
            Some("(attachment:report.pdf AND importance:high)")
        );
    }

    #[test]
    fn boolean_structure() {
        assert_eq!(kql("from:a OR from:b").unwrap().search.as_deref(), Some("(from:a OR from:b)"));
        assert_eq!(kql("-from:spam@x.com hello").unwrap().search.as_deref(), Some("(NOT from:spam@x.com AND hello)"));
        assert_eq!(kql("(a OR b) c").unwrap().search.as_deref(), Some("((a OR b) AND c)"));
        assert_eq!(kql("OR").unwrap().search.as_deref(), Some("\"OR\""));
    }

    #[test]
    fn dates_resolve_to_the_users_day() {
        // before:2026-10-01 in Zurich is 2026-09-30T22:00Z; the date stays 2026-10-01.
        assert_eq!(kql("before:2026-10-01").unwrap().search.as_deref(), Some("received<2026-10-01"));
        assert_eq!(kql("after:2026/09/15").unwrap().search.as_deref(), Some("received>=2026-09-15"));
        assert_eq!(kql("newer_than:7d").unwrap().search.as_deref(), Some("received>=2026-09-24"));
        assert_eq!(kql("older_than:1y").unwrap().search.as_deref(), Some("received<2025-10-01"));
    }

    #[test]
    fn folders_scope_the_request() {
        let k = kql("in:sent from:me@x.com").unwrap();
        assert_eq!(k.folder, Some("sentitems"));
        assert_eq!(k.search.as_deref(), Some("from:me@x.com"));
        let k = kql("in:inbox").unwrap();
        assert_eq!((k.folder, k.search), (Some("inbox"), None));
        assert_eq!(kql("in:anywhere x").unwrap().folder, None);
        assert_eq!(kql("account:work@x.com x").unwrap().search.as_deref(), Some("x"));
        assert_eq!(kql("").unwrap(), KqlQuery { search: None, folder: None });
    }

    #[test]
    fn unsupported_terms() {
        assert_eq!(kql("is:unread"), Err(KqlError::Unsupported("is:")));
        assert_eq!(kql("in:snoozed"), Err(KqlError::Unsupported("in:snoozed")));
        assert_eq!(kql("in:inbox in:sent"), Err(KqlError::Unsupported("two in: folders")));
        assert_eq!(kql("a OR in:inbox"), Err(KqlError::Unsupported("in: inside a group")));
        assert_eq!(kql("list:news.example.com"), Err(KqlError::Unsupported("list:")));
    }

    #[test]
    fn values_cannot_break_out() {
        let q = QueryAst::Field { field: Field::Subject, value: "a\" OR from:x \"b".into() };
        assert_eq!(to_kql(&q, NOW).unwrap().search.as_deref(), Some("subject:\"a OR from:x b\""));
        assert_eq!(search_param("subject:\"a b\""), "\"subject:\\\"a b\\\"\"");
    }
}
