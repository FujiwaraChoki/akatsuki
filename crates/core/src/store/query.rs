//! List and thread reads. These run on the UI thread for every frame that shows a list, so they
//! only touch indexed columns and decode at most one page of documents.

use std::collections::BTreeMap;

use rusqlite::{OptionalExtension, params_from_iter, types::Value as SqlValue};

use super::{Result, Store};
use crate::convex::rows::{MessageMeta, ThreadMirror};
use crate::model::Addr;
use crate::time::UnixMs;

/// Which threads a list shows (`mail.listThreads`' `view`).
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct ListQuery {
    /// A label key (`sys:inbox`, `l:<id>`, `split:<id>`), or `None` for all mail (everything
    /// except trash and spam).
    pub label_key: Option<String>,
    /// One account, or every account (unified).
    pub account_id: Option<String>,
    /// Only unread threads.
    pub unread_only: bool,
}

/// What a list row shows: the server row with this device's pending actions applied.
#[derive(Clone, Debug, PartialEq)]
pub struct ThreadSummary {
    /// Thread id.
    pub id: String,
    /// Account.
    pub account_id: String,
    /// Subject.
    pub subject: String,
    /// Snippet.
    pub snippet: String,
    /// Latest sender.
    pub last_from: Option<Addr>,
    /// Participants, most recent first.
    pub participants: Vec<Addr>,
    /// Sort key.
    pub sort_at: UnixMs,
    /// Messages in the thread.
    pub message_count: u32,
    /// Drafts in the thread.
    pub draft_count: u32,
    /// Visible keys.
    pub label_keys: Vec<String>,
    /// Unread (visible).
    pub unread: bool,
    /// Starred (visible).
    pub starred: bool,
    /// Has attachments.
    pub has_attachments: bool,
    /// Actions not yet confirmed by the provider (server count or local overlay).
    pub pending: bool,
    /// Snoozed until.
    pub snoozed_until: Option<UnixMs>,
    /// Triage category wire name.
    pub triage: Option<String>,
    /// Triage says a reply is expected.
    pub needs_reply: bool,
}

impl ThreadSummary {
    fn from_row(doc: &str, visible: &str, overlay: i64) -> Result<Self> {
        let t: ThreadMirror = serde_json::from_str(doc)?;
        let keys: Vec<String> = serde_json::from_str(visible)?;
        Ok(Self {
            unread: keys.iter().any(|k| k == "sys:unread"),
            starred: keys.iter().any(|k| k == "sys:starred"),
            pending: overlay > 0 || t.pending_op_count > 0,
            triage: t.triage.as_ref().map(|tr| tr.category.as_str().to_owned()),
            needs_reply: t.triage.as_ref().is_some_and(|tr| tr.needs_reply),
            id: t.id.into_string(),
            account_id: t.account_id.into_string(),
            subject: t.subject,
            snippet: t.snippet,
            last_from: t.last_from,
            participants: t.participants,
            sort_at: t.sort_at,
            message_count: t.message_count,
            draft_count: t.draft_count,
            label_keys: keys,
            has_attachments: t.has_attachments,
            snoozed_until: t.snoozed_until,
        })
    }
}

impl Store {
    /// The ordered thread ids of a list (newest first). Cheap enough for 100k threads; the UI
    /// keeps this vector and fetches summaries for the visible range only.
    pub fn list_ids(&self, q: &ListQuery) -> Result<Vec<String>> {
        let mut args: Vec<SqlValue> = Vec::new();
        let sql = match &q.label_key {
            Some(key) => {
                args.push(SqlValue::Text(key.clone()));
                let mut sql = String::from("SELECT thread_id FROM thread_labels WHERE label_key = ?1");
                if let Some(a) = &q.account_id {
                    args.push(SqlValue::Text(a.clone()));
                    sql.push_str(" AND account_id = ?2");
                }
                if q.unread_only {
                    sql.push_str(" AND is_unread = 1");
                }
                sql.push_str(" ORDER BY sort_at DESC, thread_id DESC");
                sql
            }
            None => {
                let mut sql = String::from(
                    "SELECT id FROM threads WHERE visible_deleted = 0
                     AND instr(visible_keys, '\"sys:trash\"') = 0 AND instr(visible_keys, '\"sys:spam\"') = 0",
                );
                if let Some(a) = &q.account_id {
                    args.push(SqlValue::Text(a.clone()));
                    sql.push_str(" AND account_id = ?1");
                }
                if q.unread_only {
                    sql.push_str(" AND instr(visible_keys, '\"sys:unread\"') > 0");
                }
                sql.push_str(" ORDER BY sort_at DESC, id DESC");
                sql
            }
        };
        let mut stmt = self.conn.prepare_cached(&sql)?;
        let ids = stmt.query_map(params_from_iter(args), |r| r.get(0))?.collect::<rusqlite::Result<_>>()?;
        Ok(ids)
    }

    /// Threads with a reminder set, soonest first (the "Reminders" list).
    pub fn reminder_ids(&self, account_id: Option<&str>) -> Result<Vec<String>> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT id FROM threads WHERE visible_deleted = 0 AND json_extract(doc, '$.reminderAt') IS NOT NULL
             AND (?1 IS NULL OR account_id = ?1) ORDER BY json_extract(doc, '$.reminderAt'), id",
        )?;
        let ids = stmt.query_map([account_id], |r| r.get(0))?.collect::<rusqlite::Result<_>>()?;
        Ok(ids)
    }

    /// Summaries of the given threads, in the given order; unknown ids are skipped.
    pub fn summaries(&self, ids: &[String]) -> Result<Vec<ThreadSummary>> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT doc, visible_keys, (SELECT count(*) FROM pending_overlay o WHERE o.thread_id = t.id)
             FROM threads t WHERE id = ?1",
        )?;
        let mut out = Vec::with_capacity(ids.len());
        for id in ids {
            let row: Option<(String, String, i64)> =
                stmt.query_row([id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?))).optional()?;
            if let Some((doc, visible, overlay)) = row {
                out.push(ThreadSummary::from_row(&doc, &visible, overlay)?);
            }
        }
        Ok(out)
    }

    /// One thread's summary.
    pub fn summary(&self, id: &str) -> Result<Option<ThreadSummary>> {
        Ok(self.summaries(&[id.to_owned()])?.pop())
    }

    /// The server row of a thread.
    pub fn thread(&self, id: &str) -> Result<Option<ThreadMirror>> {
        super::overlay::load_thread(&self.conn, id)
    }

    /// Message headers of a thread, oldest first.
    pub fn messages(&self, thread_id: &str) -> Result<Vec<MessageMeta>> {
        let mut stmt = self.conn.prepare_cached("SELECT doc FROM messages WHERE thread_id = ?1 ORDER BY date, id")?;
        let docs: Vec<String> = stmt.query_map([thread_id], |r| r.get(0))?.collect::<rusqlite::Result<_>>()?;
        docs.iter().map(|d| Ok(serde_json::from_str(d)?)).collect()
    }

    /// One message's headers.
    pub fn message(&self, message_id: &str) -> Result<Option<MessageMeta>> {
        let doc: Option<String> =
            self.conn.query_row("SELECT doc FROM messages WHERE id = ?1", [message_id], |r| r.get(0)).optional()?;
        Ok(match doc {
            Some(d) => Some(serde_json::from_str(&d)?),
            None => None,
        })
    }

    /// Unread threads per label key, each capped at `cap` (the UI shows "99+" past 99).
    pub fn unread_counts(
        &self,
        keys: &[String],
        account_id: Option<&str>,
        cap: usize,
    ) -> Result<BTreeMap<String, usize>> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT count(*) FROM (SELECT 1 FROM thread_labels
              WHERE label_key = ?1 AND is_unread = 1 AND (?2 IS NULL OR account_id = ?2) LIMIT ?3)",
        )?;
        let mut out = BTreeMap::new();
        for key in keys {
            let n: i64 = stmt.query_row(rusqlite::params![key, account_id, cap as i64], |r| r.get(0))?;
            out.insert(key.clone(), usize::try_from(n).unwrap_or(0));
        }
        Ok(out)
    }

    /// Number of threads in the mirror (all accounts).
    pub fn thread_count(&self) -> Result<usize> {
        let n: i64 = self.conn.query_row("SELECT count(*) FROM threads", [], |r| r.get(0))?;
        Ok(usize::try_from(n).unwrap_or(0))
    }
}
