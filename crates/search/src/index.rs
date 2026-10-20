//! The desktop full-text index (ARCHITECTURE §11.2, backend 3): SQLite FTS5 over subjects,
//! addresses, bodies and attachment names, plus a metadata table so the full grammar works offline.
//!
//! ```sql
//! messages_fts(subject, from_, to_, body, filenames)
//!   content='', contentless_delete=1, tokenize='unicode61 remove_diacritics 2', prefix='2 3'
//! ```
//!
//! bm25 weights are subject 5, from 3, to 2, body 1, filenames 2. Results are grouped by thread so
//! they can be merged with Convex results by `threadId`.

use std::collections::HashSet;
use std::path::Path;

use rusqlite::types::Value as SqlValue;
use rusqlite::{Connection, OptionalExtension, params, params_from_iter};

use crate::query::{AgeField, DateField, Field, QueryAst};

/// Errors from the index.
#[derive(Debug, thiserror::Error)]
pub enum IndexError {
    /// SQLite failed.
    #[error("sqlite: {0}")]
    Sqlite(#[from] rusqlite::Error),
    /// The bundled SQLite lacks a needed feature.
    #[error("sqlite feature missing: {0}")]
    Unsupported(String),
}

/// One message to index. Bodies are indexed only on the device; they never leave it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct IndexDoc {
    /// Convex message id.
    pub message_id: String,
    /// Convex thread id.
    pub thread_id: String,
    /// Convex account id.
    pub account_id: String,
    /// Account address (for `account:`).
    pub account_email: String,
    /// Message date (ms).
    pub date: i64,
    /// Subject.
    pub subject: String,
    /// Sender name and address.
    pub from: String,
    /// To and Cc names and addresses.
    pub to: String,
    /// Plain-text body (quoted history may be included).
    pub body: String,
    /// Attachment file names, space-separated.
    pub filenames: String,
    /// Label keys of the thread (`sys:*`, `l:*`).
    pub label_keys: Vec<String>,
    /// Label display names of the thread (for `label:`).
    pub label_names: Vec<String>,
    /// Split name of the thread (for `in:<split>`).
    pub split_name: Option<String>,
    /// Unread.
    pub is_unread: bool,
    /// Has attachments.
    pub has_attachments: bool,
    /// Sent by the user.
    pub is_outbound: bool,
    /// Triage category.
    pub triage_category: Option<String>,
    /// Triage needs-reply.
    pub needs_reply: bool,
    /// `List-Id`.
    pub list_id: Option<String>,
}

/// A thread-level hit.
#[derive(Clone, Debug, PartialEq)]
pub struct ThreadHit {
    /// Convex thread id.
    pub thread_id: String,
    /// The best-matching message.
    pub message_id: String,
    /// That message's date.
    pub date: i64,
    /// bm25 score of the best message (lower is better), when the query had free text.
    pub score: Option<f64>,
}

/// Inputs a query needs besides the AST.
#[derive(Clone, Copy, Debug)]
pub struct SearchContext {
    /// "Now" for `older_than` / `newer_than` (ms).
    pub now_at: i64,
    /// Maximum threads returned.
    pub limit: usize,
}

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS search_docs (
    rowid INTEGER PRIMARY KEY,
    message_id TEXT NOT NULL UNIQUE,
    thread_id TEXT NOT NULL,
    account_id TEXT NOT NULL,
    account_email TEXT NOT NULL,
    date INTEGER NOT NULL,
    label_keys TEXT NOT NULL,
    label_names TEXT NOT NULL,
    split_name TEXT,
    is_unread INTEGER NOT NULL,
    has_attachments INTEGER NOT NULL,
    is_outbound INTEGER NOT NULL,
    triage_category TEXT,
    needs_reply INTEGER NOT NULL,
    list_id TEXT
);
CREATE INDEX IF NOT EXISTS search_docs_thread ON search_docs(thread_id);
CREATE INDEX IF NOT EXISTS search_docs_date ON search_docs(date);
CREATE VIRTUAL TABLE IF NOT EXISTS messages_fts USING fts5(
    subject, from_, to_, body, filenames,
    content='', contentless_delete=1,
    tokenize='unicode61 remove_diacritics 2',
    prefix='2 3'
);
"#;

/// Label separator in `label_names` (never appears in names typed by users).
const SEP: char = '\u{1f}';

/// The FTS5 index.
pub struct SearchIndex {
    conn: Connection,
}

impl std::fmt::Debug for SearchIndex {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SearchIndex").finish_non_exhaustive()
    }
}

impl SearchIndex {
    /// Opens (or creates) an index file.
    pub fn open(path: &Path) -> Result<Self, IndexError> {
        Self::init(Connection::open(path)?)
    }

    /// An in-memory index (tests, ephemeral sessions).
    pub fn open_in_memory() -> Result<Self, IndexError> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(conn: Connection) -> Result<Self, IndexError> {
        conn.pragma_update(None, "journal_mode", "WAL").ok();
        conn.execute_batch(SCHEMA).map_err(|e| {
            if e.to_string().contains("contentless_delete") {
                IndexError::Unsupported("FTS5 contentless_delete (SQLite >= 3.43)".into())
            } else {
                IndexError::Sqlite(e)
            }
        })?;
        Ok(Self { conn })
    }

    /// Inserts or replaces a message.
    pub fn upsert(&mut self, doc: &IndexDoc) -> Result<(), IndexError> {
        let tx = self.conn.transaction()?;
        let existing: Option<i64> = tx
            .query_row("SELECT rowid FROM search_docs WHERE message_id = ?1", [&doc.message_id], |r| r.get(0))
            .optional()?;
        if let Some(rowid) = existing {
            tx.execute("DELETE FROM messages_fts WHERE rowid = ?1", [rowid])?;
            tx.execute("DELETE FROM search_docs WHERE rowid = ?1", [rowid])?;
        }
        let label_keys = format!(" {} ", doc.label_keys.join(" "));
        let label_names = {
            let mut s = String::from(SEP);
            for n in &doc.label_names {
                s.push_str(&n.to_lowercase());
                s.push(SEP);
            }
            s
        };
        tx.execute(
            "INSERT INTO search_docs (message_id, thread_id, account_id, account_email, date, label_keys, label_names,
                split_name, is_unread, has_attachments, is_outbound, triage_category, needs_reply, list_id)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
            params![
                doc.message_id,
                doc.thread_id,
                doc.account_id,
                doc.account_email.to_lowercase(),
                doc.date,
                label_keys,
                label_names,
                doc.split_name.as_deref().map(str::to_lowercase),
                doc.is_unread,
                doc.has_attachments,
                doc.is_outbound,
                doc.triage_category,
                doc.needs_reply,
                doc.list_id.as_deref().map(str::to_lowercase),
            ],
        )?;
        let rowid = tx.last_insert_rowid();
        tx.execute(
            "INSERT INTO messages_fts (rowid, subject, from_, to_, body, filenames) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![rowid, doc.subject, doc.from, doc.to, doc.body, doc.filenames],
        )?;
        tx.commit()?;
        Ok(())
    }

    /// Removes a message. Returns `true` if it was indexed.
    pub fn remove(&mut self, message_id: &str) -> Result<bool, IndexError> {
        let tx = self.conn.transaction()?;
        let rowid: Option<i64> = tx
            .query_row("SELECT rowid FROM search_docs WHERE message_id = ?1", [message_id], |r| r.get(0))
            .optional()?;
        if let Some(rowid) = rowid {
            tx.execute("DELETE FROM messages_fts WHERE rowid = ?1", [rowid])?;
            tx.execute("DELETE FROM search_docs WHERE rowid = ?1", [rowid])?;
        }
        tx.commit()?;
        Ok(rowid.is_some())
    }

    /// Removes every message of an account (sign-out, account removal).
    pub fn remove_account(&mut self, account_id: &str) -> Result<usize, IndexError> {
        let tx = self.conn.transaction()?;
        tx.execute(
            "DELETE FROM messages_fts WHERE rowid IN (SELECT rowid FROM search_docs WHERE account_id = ?1)",
            [account_id],
        )?;
        let n = tx.execute("DELETE FROM search_docs WHERE account_id = ?1", [account_id])?;
        tx.commit()?;
        Ok(n)
    }

    /// Number of indexed messages.
    pub fn len(&self) -> Result<usize, IndexError> {
        let n: i64 = self.conn.query_row("SELECT count(*) FROM search_docs", [], |r| r.get(0))?;
        Ok(usize::try_from(n).unwrap_or(0))
    }

    /// `true` when nothing is indexed.
    pub fn is_empty(&self) -> Result<bool, IndexError> {
        Ok(self.len()? == 0)
    }

    /// Runs a query and returns matching threads, best first (bm25 when the query has free text,
    /// otherwise newest first).
    pub fn search(&self, ast: &QueryAst, ctx: SearchContext) -> Result<Vec<ThreadHit>, IndexError> {
        let mut params: Vec<SqlValue> = Vec::new();
        let where_sql = compile(ast, ctx.now_at, &mut params);
        let mut positive = Vec::new();
        collect_positive_text(ast, false, &mut positive);

        let sql = if positive.is_empty() {
            format!(
                "SELECT d.thread_id, d.message_id, d.date, NULL FROM search_docs d WHERE {where_sql} ORDER BY d.date DESC"
            )
        } else {
            let rank_param = params.len() + 1;
            params.push(SqlValue::Text(positive.join(" OR ")));
            format!(
                "SELECT d.thread_id, d.message_id, d.date, r.score FROM search_docs d
                 LEFT JOIN (SELECT rowid, bm25(messages_fts, 5.0, 3.0, 2.0, 1.0, 2.0) AS score
                            FROM messages_fts WHERE messages_fts MATCH ?{rank_param}) r ON r.rowid = d.rowid
                 WHERE {where_sql}
                 ORDER BY (r.score IS NULL), r.score, d.date DESC"
            )
        };
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map(params_from_iter(params.iter()), |r| {
            Ok(ThreadHit { thread_id: r.get(0)?, message_id: r.get(1)?, date: r.get(2)?, score: r.get(3)? })
        })?;
        let mut seen = HashSet::new();
        let mut out = Vec::new();
        for hit in rows {
            let hit = hit?;
            if seen.insert(hit.thread_id.clone()) {
                out.push(hit);
                if out.len() >= ctx.limit {
                    break;
                }
            }
        }
        Ok(out)
    }
}

/// Quotes a string as an FTS5 string literal.
fn fts_string(s: &str) -> String {
    format!("\"{}\"", s.replace('"', "\"\""))
}

/// FTS5 expression for a text leaf.
fn text_match(text: &str, phrase: bool) -> Option<String> {
    // Text without any letters or digits has no tokens; it is ignored rather than sent to FTS5.
    if !text.chars().any(char::is_alphanumeric) {
        return None;
    }
    if !phrase
        && let Some(prefix) = text.strip_suffix('*')
        && !prefix.is_empty()
    {
        return Some(format!("{} *", fts_string(prefix)));
    }
    Some(fts_string(text))
}

fn column_match(column: &str, value: &str) -> Option<String> {
    text_match(value, true).map(|m| format!("{{{column}}} : {m}"))
}

fn fts_subquery(expr: String, params: &mut Vec<SqlValue>) -> String {
    params.push(SqlValue::Text(expr));
    format!("d.rowid IN (SELECT rowid FROM messages_fts WHERE messages_fts MATCH ?{})", params.len())
}

fn param(params: &mut Vec<SqlValue>, v: impl Into<SqlValue>) -> String {
    params.push(v.into());
    format!("?{}", params.len())
}

fn like_escape(s: &str) -> String {
    s.replace('\\', "\\\\").replace('%', "\\%").replace('_', "\\_")
}

fn has_key(key: &str, params: &mut Vec<SqlValue>) -> String {
    let p = param(params, format!("% {} %", like_escape(key)));
    format!("d.label_keys LIKE {p} ESCAPE '\\'")
}

const TRUE: &str = "1";
const FALSE: &str = "0";

fn compile(ast: &QueryAst, now_at: i64, params: &mut Vec<SqlValue>) -> String {
    match ast {
        QueryAst::And(items) if items.is_empty() => TRUE.to_owned(),
        QueryAst::Or(items) if items.is_empty() => FALSE.to_owned(),
        QueryAst::And(items) => {
            format!("({})", items.iter().map(|i| compile(i, now_at, params)).collect::<Vec<_>>().join(" AND "))
        }
        QueryAst::Or(items) => {
            format!("({})", items.iter().map(|i| compile(i, now_at, params)).collect::<Vec<_>>().join(" OR "))
        }
        QueryAst::Not(inner) => format!("(NOT {})", compile(inner, now_at, params)),
        QueryAst::Text { text, phrase } => match text_match(text, *phrase) {
            Some(m) => fts_subquery(m, params),
            None => TRUE.to_owned(),
        },
        QueryAst::Date { field, at } => {
            let p = param(params, *at);
            match field {
                DateField::Before => format!("d.date < {p}"),
                DateField::After => format!("d.date >= {p}"),
            }
        }
        QueryAst::Age { field, days } => {
            let cutoff = now_at - i64::from(*days) * 86_400_000;
            let p = param(params, cutoff);
            match field {
                AgeField::OlderThan => format!("d.date < {p}"),
                AgeField::NewerThan => format!("d.date >= {p}"),
            }
        }
        QueryAst::Field { field, value } => compile_field(*field, value, params),
    }
}

fn compile_field(field: Field, value: &str, params: &mut Vec<SqlValue>) -> String {
    let lower = value.to_lowercase();
    match field {
        Field::From => column_match("from_", value).map_or(TRUE.to_owned(), |m| fts_subquery(m, params)),
        Field::To | Field::Cc => column_match("to_", value).map_or(TRUE.to_owned(), |m| fts_subquery(m, params)),
        Field::Subject => column_match("subject", value).map_or(TRUE.to_owned(), |m| fts_subquery(m, params)),
        Field::Filename => column_match("filenames", value).map_or(TRUE.to_owned(), |m| fts_subquery(m, params)),
        Field::Label => {
            if crate::is_label_key_like(value) {
                has_key(value, params)
            } else {
                let p = param(params, format!("%{SEP}{}{SEP}%", like_escape(&lower)));
                format!("d.label_names LIKE {p} ESCAPE '\\'")
            }
        }
        Field::In => match lower.as_str() {
            "inbox" | "sent" | "drafts" | "trash" | "spam" | "snoozed" => has_key(&format!("sys:{lower}"), params),
            "anywhere" => TRUE.to_owned(),
            "archive" => {
                let a = has_key("sys:inbox", params);
                let b = has_key("sys:trash", params);
                let c = has_key("sys:spam", params);
                format!("(NOT {a} AND NOT {b} AND NOT {c})")
            }
            _ => {
                let p = param(params, lower);
                format!("d.split_name = {p}")
            }
        },
        Field::Is => match lower.as_str() {
            "unread" => "d.is_unread = 1".to_owned(),
            "read" => "d.is_unread = 0".to_owned(),
            "starred" | "important" | "snoozed" => has_key(&format!("sys:{lower}"), params),
            "needs_reply" => "d.needs_reply = 1".to_owned(),
            _ => FALSE.to_owned(),
        },
        Field::Has => match lower.as_str() {
            "attachment" => "d.has_attachments = 1".to_owned(),
            _ => FALSE.to_owned(),
        },
        Field::Account => {
            let a = param(params, lower);
            let b = param(params, value.to_owned());
            format!("(d.account_email = {a} OR d.account_id = {b})")
        }
        Field::List => {
            let p = param(params, format!("%{}%", like_escape(&lower)));
            format!("d.list_id LIKE {p} ESCAPE '\\'")
        }
        Field::Category => {
            let p = param(params, lower);
            format!("d.triage_category = {p}")
        }
    }
}

/// Free-text and text-field leaves that are not under a negation, for ranking.
fn collect_positive_text(ast: &QueryAst, negated: bool, out: &mut Vec<String>) {
    match ast {
        QueryAst::And(items) | QueryAst::Or(items) => {
            for i in items {
                collect_positive_text(i, negated, out);
            }
        }
        QueryAst::Not(inner) => collect_positive_text(inner, !negated, out),
        QueryAst::Text { text, phrase } if !negated => out.extend(text_match(text, *phrase)),
        QueryAst::Field { field, value } if !negated => {
            let col = match field {
                Field::From => "from_",
                Field::To | Field::Cc => "to_",
                Field::Subject => "subject",
                Field::Filename => "filenames",
                _ => return,
            };
            out.extend(column_match(col, value));
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::query::parse;

    const NOW: i64 = 1_792_540_800_000; // 2026-10-20T00:00:00Z
    const DAY: i64 = 86_400_000;

    fn doc(id: &str, thread: &str, subject: &str, from: &str, body: &str, days_ago: i64) -> IndexDoc {
        IndexDoc {
            message_id: id.into(),
            thread_id: thread.into(),
            account_id: "acc1".into(),
            account_email: "me@example.com".into(),
            date: NOW - days_ago * DAY,
            subject: subject.into(),
            from: from.into(),
            to: "Me <me@example.com>".into(),
            body: body.into(),
            label_keys: vec!["sys:inbox".into()],
            ..Default::default()
        }
    }

    fn index() -> SearchIndex {
        let mut idx = SearchIndex::open_in_memory().unwrap();
        idx.upsert(&doc(
            "m1",
            "t1",
            "Quarterly report",
            "Ada Lovelace <ada@example.com>",
            "Numbers attached for Q3.",
            1,
        ))
        .unwrap();
        let mut m2 = doc("m2", "t2", "Lunch?", "Bob <bob@lunch.io>", "Café at noon, then the report review", 10);
        m2.is_unread = true;
        m2.label_names = vec!["Work".into()];
        m2.label_keys.push("sys:starred".into());
        idx.upsert(&m2).unwrap();
        let mut m3 = doc("m3", "t1", "Re: Quarterly report", "Me <me@example.com>", "Thanks!", 0);
        m3.is_outbound = true;
        m3.has_attachments = true;
        m3.filenames = "q3-numbers.xlsx".into();
        m3.label_keys = vec!["sys:sent".into()];
        idx.upsert(&m3).unwrap();
        let mut m4 = doc("m4", "t3", "Newsletter", "News <news@list.example>", "Weekly digest", 40);
        m4.list_id = Some("weekly.list.example".into());
        m4.triage_category = Some("newsletters".into());
        m4.label_keys = vec![];
        idx.upsert(&m4).unwrap();
        idx
    }

    fn threads(idx: &SearchIndex, q: &str) -> Vec<String> {
        let ast = parse(q, NOW, "UTC").ast;
        idx.search(&ast, SearchContext { now_at: NOW, limit: 50 }).unwrap().into_iter().map(|h| h.thread_id).collect()
    }

    #[test]
    fn free_text_ranks_subject_matches_first() {
        let idx = index();
        let got = threads(&idx, "report");
        assert_eq!(got, vec!["t1", "t2"], "subject hit (weight 5) before body hit");
    }

    #[test]
    fn diacritics_and_prefixes() {
        let idx = index();
        assert_eq!(threads(&idx, "cafe"), vec!["t2"]);
        assert_eq!(threads(&idx, "quart*"), vec!["t1"]);
    }

    #[test]
    fn field_operators() {
        let idx = index();
        assert_eq!(threads(&idx, "from:ada@example.com"), vec!["t1"]);
        assert_eq!(threads(&idx, "from:bob"), vec!["t2"]);
        assert_eq!(threads(&idx, "subject:lunch"), vec!["t2"]);
        assert_eq!(threads(&idx, "is:unread"), vec!["t2"]);
        assert_eq!(threads(&idx, "is:starred"), vec!["t2"]);
        assert_eq!(threads(&idx, "label:work"), vec!["t2"]);
        assert_eq!(threads(&idx, "has:attachment"), vec!["t1"]);
        assert_eq!(threads(&idx, "filename:xlsx"), vec!["t1"]);
        assert_eq!(threads(&idx, "in:sent"), vec!["t1"]);
        assert_eq!(threads(&idx, "list:weekly"), vec!["t3"]);
        assert_eq!(threads(&idx, "category:newsletters"), vec!["t3"]);
        assert_eq!(threads(&idx, "account:me@example.com").len(), 3);
        assert_eq!(threads(&idx, "in:archive"), vec!["t1", "t3"]);
    }

    #[test]
    fn dates_negation_and_boolean_logic() {
        let idx = index();
        assert_eq!(threads(&idx, "older_than:30d"), vec!["t3"]);
        assert_eq!(threads(&idx, "newer_than:2d"), vec!["t1"]);
        assert_eq!(threads(&idx, "after:2026-10-15"), vec!["t1"]);
        assert_eq!(threads(&idx, "before:2026-10-15"), vec!["t2", "t3"]);
        assert_eq!(threads(&idx, "report -lunch"), vec!["t1"]);
        assert_eq!(threads(&idx, "-report"), vec!["t3"]);
    }

    #[test]
    fn or_and_phrases() {
        let idx = index();
        let mut got = threads(&idx, "lunch OR newsletter");
        got.sort();
        assert_eq!(got, vec!["t2", "t3"]);
        assert_eq!(threads(&idx, "\"report review\""), vec!["t2"]);
        assert!(threads(&idx, "\"review report\"").is_empty());
    }

    #[test]
    fn hostile_input_is_inert() {
        let idx = index();
        for q in [
            "\" OR 1=1 --",
            "a\"b",
            "NEAR(a b)",
            "subject:\"x\" OR \"",
            "*",
            "'; DROP TABLE search_docs; --",
            "{body}:x",
        ] {
            let ast = parse(q, NOW, "UTC").ast;
            idx.search(&ast, SearchContext { now_at: NOW, limit: 10 }).unwrap();
        }
        assert_eq!(idx.len().unwrap(), 4);
    }

    #[test]
    fn upsert_replaces_and_remove_deletes() {
        let mut idx = index();
        let mut m2 = doc("m2", "t2", "Dinner?", "Bob <bob@example.org>", "Changed plans", 10);
        m2.is_unread = false;
        idx.upsert(&m2).unwrap();
        assert!(threads(&idx, "lunch").is_empty());
        assert_eq!(threads(&idx, "dinner"), vec!["t2"]);
        assert!(idx.remove("m2").unwrap());
        assert!(!idx.remove("m2").unwrap());
        assert!(threads(&idx, "dinner").is_empty());
        assert_eq!(idx.remove_account("acc1").unwrap(), 3);
        assert!(idx.is_empty().unwrap());
    }

    #[test]
    fn persists_to_disk() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("search.sqlite");
        {
            let mut idx = SearchIndex::open(&path).unwrap();
            idx.upsert(&doc("m1", "t1", "Persisted", "a@b.c", "body", 0)).unwrap();
        }
        let idx = SearchIndex::open(&path).unwrap();
        assert_eq!(threads(&idx, "persisted"), vec!["t1"]);
    }
}
