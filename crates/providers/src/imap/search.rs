//! Search grammar (§11.1) → IMAP `SEARCH` (§11.2 backend 2: `FROM`, `SUBJECT`, `SINCE`,
//! `BEFORE`, `TEXT`, `OR`, `NOT`).
//!
//! `in:` picks the folders to search (`in:inbox`, `in:sent`, `in:trash`, `in:spam`, `in:drafts`,
//! `in:snoozed`, `in:archive`, or a folder name); without it every synced folder except Trash and
//! Spam is searched, like Gmail. `label:` names a folder or a keyword.

use akatsuki_search::query::{AgeField, DateField, Field, QueryAst};

use super::folders::{Folder, KEYWORD_PREFIX};
use super::proto::{self, Cmd, Part};
use crate::{LabelRole, ProviderError};

/// What to run.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SearchPlan {
    /// Folder names to search; `None` = the default set ([`default_folder`]).
    pub folders: Option<Vec<String>>,
    /// The `SEARCH` criteria (with `CHARSET UTF-8` when needed).
    pub criteria: Cmd,
}

/// Searched when the query has no `in:`.
pub fn default_folder(f: &Folder) -> bool {
    !matches!(f.role, Some(LabelRole::Trash | LabelRole::Spam))
}

const DAY_MS: i64 = 86_400_000;

fn role_of_in(v: &str) -> Option<LabelRole> {
    Some(match v.to_ascii_lowercase().as_str() {
        "inbox" => LabelRole::Inbox,
        "sent" => LabelRole::Sent,
        "drafts" | "draft" => LabelRole::Drafts,
        "trash" | "bin" => LabelRole::Trash,
        "spam" | "junk" => LabelRole::Spam,
        "snoozed" => LabelRole::Snoozed,
        "archive" => LabelRole::Archive,
        _ => return None,
    })
}

fn find_folder<'a>(folders: &'a [Folder], v: &str) -> Option<&'a Folder> {
    match role_of_in(v) {
        Some(r) => folders.iter().find(|f| f.role == Some(r)),
        None => folders.iter().find(|f| f.name.eq_ignore_ascii_case(v) || f.display_name().eq_ignore_ascii_case(v)),
    }
}

/// Builds the plan.
pub fn plan(q: &QueryAst, folders: &[Folder], now_ms: i64) -> Result<SearchPlan, ProviderError> {
    // Top-level `in:` / folder `label:` terms choose folders.
    let mut chosen: Vec<String> = Vec::new();
    let mut rest: Vec<&QueryAst> = Vec::new();
    let top: Vec<&QueryAst> = match q {
        QueryAst::And(v) => v.iter().collect(),
        other => vec![other],
    };
    for t in top {
        match t {
            QueryAst::Field { field: Field::In, value } => match find_folder(folders, value) {
                Some(f) => chosen.push(f.name.clone()),
                None if value.eq_ignore_ascii_case("all") || value.eq_ignore_ascii_case("anywhere") => {
                    chosen.extend(folders.iter().map(|f| f.name.clone()));
                }
                None => return Err(ProviderError::Invalid("unknown folder".into())),
            },
            QueryAst::Field { field: Field::Label, value } if find_folder(folders, value).is_some() => {
                if let Some(f) = find_folder(folders, value) {
                    chosen.push(f.name.clone());
                }
            }
            other => rest.push(other),
        }
    }
    let mut out = Vec::new();
    for t in rest {
        out.push(term(t, now_ms)?);
    }
    let mut criteria = if out.is_empty() { Cmd::new("ALL") } else { join_and(out) };
    if criteria.parts.iter().any(|p| matches!(p, Part::Literal(_))) {
        let mut c = Cmd::new("CHARSET UTF-8 ");
        c.parts.extend(criteria.parts);
        merge_text(&mut c);
        criteria = c;
    }
    chosen.dedup();
    Ok(SearchPlan { folders: (!chosen.is_empty()).then_some(chosen), criteria })
}

fn merge_text(c: &mut Cmd) {
    let mut merged: Vec<Part> = Vec::new();
    for p in std::mem::take(&mut c.parts) {
        match (merged.last_mut(), p) {
            (Some(Part::Text(a)), Part::Text(b)) => a.push_str(&b),
            (_, p) => merged.push(p),
        }
    }
    c.parts = merged;
}

fn concat(a: Cmd, b: Cmd, sep: &str) -> Cmd {
    let mut c = a;
    c.parts.push(Part::Text(sep.to_owned()));
    c.parts.extend(b.parts);
    merge_text(&mut c);
    c
}

fn wrap(inner: Cmd, prefix: &str) -> Cmd {
    let mut c = Cmd::new(format!("{prefix}("));
    c.parts.extend(inner.parts);
    c.parts.push(Part::Text(")".into()));
    merge_text(&mut c);
    c
}

fn join_and(items: Vec<Cmd>) -> Cmd {
    let mut it = items.into_iter();
    let first = it.next().unwrap_or_else(|| Cmd::new("ALL"));
    it.fold(first, |acc, x| concat(acc, x, " "))
}

fn string_key(key: &str, value: &str) -> Result<Cmd, ProviderError> {
    Cmd::new(key).string(value)
}

fn term(q: &QueryAst, now_ms: i64) -> Result<Cmd, ProviderError> {
    Ok(match q {
        QueryAst::And(v) if v.is_empty() => Cmd::new("ALL"),
        QueryAst::And(v) => wrap(join_and(v.iter().map(|t| term(t, now_ms)).collect::<Result<_, _>>()?), ""),
        QueryAst::Or(v) => {
            let mut items: Vec<Cmd> = v.iter().map(|t| term(t, now_ms)).collect::<Result<_, _>>()?;
            match items.len() {
                0 => Cmd::new("ALL"),
                1 => items.remove(0),
                _ => {
                    // OR is binary: OR a (OR b c).
                    let last = items.pop().unwrap_or_else(|| Cmd::new("ALL"));
                    items
                        .into_iter()
                        .rev()
                        .fold(last, |acc, x| wrap(concat(concat(Cmd::new("OR"), x, " "), acc, " "), ""))
                }
            }
        }
        QueryAst::Not(inner) => concat(Cmd::new("NOT"), term(inner, now_ms)?, " "),
        QueryAst::Text { text, .. } => string_key("TEXT", text)?,
        QueryAst::Field { field, value } => match field {
            Field::From => string_key("FROM", value)?,
            Field::To => string_key("TO", value)?,
            Field::Cc => string_key("CC", value)?,
            Field::Subject => string_key("SUBJECT", value)?,
            Field::List => Cmd::new("HEADER").raw("List-Id").string(value)?,
            Field::Is => match value.to_ascii_lowercase().as_str() {
                "unread" => Cmd::new("UNSEEN"),
                "read" => Cmd::new("SEEN"),
                "starred" | "flagged" => Cmd::new("FLAGGED"),
                "unstarred" => Cmd::new("UNFLAGGED"),
                "important" => Cmd::new("KEYWORD $Important"),
                "draft" => Cmd::new("DRAFT"),
                _ => return Err(ProviderError::Unsupported("this is: filter on IMAP")),
            },
            Field::Has => match value.to_ascii_lowercase().as_str() {
                // Best effort: a multipart/mixed body almost always carries an attachment.
                "attachment" => Cmd::new("HEADER").raw("Content-Type").string("multipart/mixed")?,
                _ => return Err(ProviderError::Unsupported("this has: filter on IMAP")),
            },
            Field::Label => {
                let kw = value.strip_prefix(KEYWORD_PREFIX).unwrap_or(value);
                if kw.is_empty() || kw.bytes().any(|b| b <= b' ' || b"(){%*\"\\]".contains(&b)) || !kw.is_ascii() {
                    return Err(ProviderError::Invalid("not an IMAP keyword".into()));
                }
                Cmd::new(format!("KEYWORD {kw}"))
            }
            Field::In => return Err(ProviderError::Unsupported("nested in: on IMAP")),
            Field::Filename | Field::Account | Field::Category => {
                return Err(ProviderError::Unsupported("this operator on IMAP"));
            }
        },
        QueryAst::Date { field, at } => match field {
            DateField::After => Cmd::new("SINCE").raw(proto::search_date(*at)),
            DateField::Before => Cmd::new("BEFORE").raw(proto::search_date(*at)),
        },
        QueryAst::Age { field, days } => {
            let at = now_ms - i64::from(*days) * DAY_MS;
            match field {
                AgeField::NewerThan => Cmd::new("SINCE").raw(proto::search_date(at)),
                AgeField::OlderThan => Cmd::new("BEFORE").raw(proto::search_date(at)),
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::imap::session::ListEntry;

    fn folders() -> Vec<Folder> {
        crate::imap::folders::folders_from_list(vec![
            ListEntry { name: "INBOX".into(), delimiter: Some("/".into()), attrs: vec![] },
            ListEntry { name: "Sent".into(), delimiter: Some("/".into()), attrs: vec!["\\sent".into()] },
            ListEntry { name: "Trash".into(), delimiter: Some("/".into()), attrs: vec!["\\trash".into()] },
            ListEntry { name: "Work".into(), delimiter: Some("/".into()), attrs: vec![] },
        ])
    }

    fn text(c: &Cmd) -> String {
        c.parts
            .iter()
            .map(|p| match p {
                Part::Text(t) => t.clone(),
                Part::Literal(b) => format!("{{{}}}{}", b.len(), String::from_utf8_lossy(b)),
            })
            .collect()
    }

    fn q(s: &str) -> QueryAst {
        akatsuki_search::parse(s, 1_792_497_600_000, "UTC").ast
    }

    #[test]
    fn translates_grammar() {
        let f = folders();
        let p = plan(&q("from:alice subject:\"q3 report\" is:unread"), &f, 0).unwrap();
        assert_eq!(p.folders, None);
        assert_eq!(text(&p.criteria), "FROM \"alice\" SUBJECT \"q3 report\" UNSEEN");

        let p = plan(&q("in:sent -from:bob"), &f, 0).unwrap();
        assert_eq!(p.folders, Some(vec!["Sent".to_owned()]));
        assert_eq!(text(&p.criteria), "NOT FROM \"bob\"");

        let p = plan(&q("label:Work (foo OR bar OR baz)"), &f, 0).unwrap();
        assert_eq!(p.folders, Some(vec!["Work".to_owned()]));
        assert_eq!(text(&p.criteria), "(OR TEXT \"foo\" (OR TEXT \"bar\" TEXT \"baz\"))");

        let p = plan(&q("grüße"), &f, 0).unwrap();
        assert_eq!(text(&p.criteria), "CHARSET UTF-8 TEXT {7}grüße");

        let p = plan(&q("after:2026-10-01 before:2026-10-15"), &f, 0).unwrap();
        assert_eq!(text(&p.criteria), "SINCE 1-Oct-2026 BEFORE 15-Oct-2026");

        let p = plan(&q(""), &f, 0).unwrap();
        assert_eq!(text(&p.criteria), "ALL");
        assert!(plan(&q("filename:a.pdf"), &f, 0).is_err());
        assert!(default_folder(&f[0]) && !default_folder(f.iter().find(|x| x.name == "Trash").unwrap()));
    }
}
