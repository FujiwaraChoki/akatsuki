//! Applying `sync.changes` batches (§16.3 "Feed").

use std::collections::BTreeSet;

use rusqlite::{Connection, OptionalExtension, params};

use super::overlay::{Contexts, prune, recompute_all};
use super::{Result, Store, to_i64};
use crate::convex::TombstoneTable;
use crate::convex::client::SyncChangesResult;

/// What happened to a batch.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FeedOutcome {
    /// Applied; the mirror is now at `rev`. With `has_more`, subscribe again from `rev` at once.
    Applied {
        /// New revision.
        rev: u64,
        /// More changes are waiting.
        has_more: bool,
        /// Threads whose visible state may have changed.
        threads: Vec<String>,
    },
    /// Nothing newer than what the mirror holds (a replayed or out-of-order batch).
    Stale {
        /// The mirror's revision.
        rev: u64,
    },
    /// The batch was computed from a revision the mirror does not hold: refused. Subscribe again
    /// from `local_rev`.
    Gap {
        /// The mirror's revision.
        local_rev: u64,
    },
    /// The server asked for a full re-pull: the account was wiped and is at rev 0.
    Reset,
}

/// Deletes an account's mirrored rows (threads, list rows, messages, labels). The overlay and the
/// outbox are kept: they hold this device's unsent intent.
pub(super) fn wipe_account(conn: &Connection, account_id: &str) -> Result<()> {
    for table in ["thread_labels", "threads", "messages", "labels"] {
        conn.execute(&format!("DELETE FROM {table} WHERE account_id = ?1"), [account_id])?;
    }
    Ok(())
}

impl Store {
    /// Applies one `sync.changes({accountId, sinceRev})` result in one transaction.
    pub fn apply_changes(
        &mut self,
        account_id: &str,
        since_rev: u64,
        batch: &SyncChangesResult,
    ) -> Result<FeedOutcome> {
        let local = self.account_rev(account_id)?;
        let tx = self.conn.transaction()?;
        if batch.reset_required {
            wipe_account(&tx, account_id)?;
            tx.execute("INSERT OR REPLACE INTO sync_state (account_id, rev) VALUES (?1, 0)", [account_id])?;
            tx.commit()?;
            return Ok(FeedOutcome::Reset);
        }
        if since_rev > local {
            return Ok(FeedOutcome::Gap { local_rev: local });
        }
        if batch.up_to_rev <= local {
            return Ok(FeedOutcome::Stale { rev: local });
        }

        let mut touched: BTreeSet<String> = BTreeSet::new();
        let mut labels_changed = false;

        for label in &batch.labels {
            if label.account_id.as_str() != account_id {
                continue;
            }
            labels_changed = true;
            tx.execute(
                "INSERT OR REPLACE INTO labels (id, account_id, label_key, name, exclusive, hidden, doc)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![
                    label.id.as_str(),
                    account_id,
                    label.label_key,
                    label.name,
                    label.exclusive,
                    label.hidden,
                    serde_json::to_string(label)?
                ],
            )?;
        }

        for thread in &batch.threads {
            if thread.account_id.as_str() != account_id {
                continue;
            }
            let stored: Option<i64> =
                tx.query_row("SELECT rev FROM threads WHERE id = ?1", [thread.id.as_str()], |r| r.get(0)).optional()?;
            if stored.is_some_and(|r| r > to_i64(thread.rev)) {
                continue;
            }
            tx.execute(
                "INSERT INTO threads (id, account_id, rev, sort_at, visible_keys, visible_deleted, doc)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
                 ON CONFLICT(id) DO UPDATE SET rev = excluded.rev, sort_at = excluded.sort_at, doc = excluded.doc",
                params![
                    thread.id.as_str(),
                    account_id,
                    to_i64(thread.rev),
                    thread.sort_at,
                    serde_json::to_string(&thread.label_keys)?,
                    thread.deleted_pending,
                    serde_json::to_string(thread)?
                ],
            )?;
            touched.insert(thread.id.as_str().to_owned());
        }

        for message in &batch.messages {
            if message.account_id.as_str() != account_id {
                continue;
            }
            tx.execute(
                "INSERT OR REPLACE INTO messages (id, account_id, thread_id, date, doc) VALUES (?1, ?2, ?3, ?4, ?5)",
                params![
                    message.id.as_str(),
                    account_id,
                    message.thread_id.as_str(),
                    message.date,
                    serde_json::to_string(message)?
                ],
            )?;
        }

        for t in &batch.tombstones {
            match t.table {
                TombstoneTable::Threads => {
                    tx.execute("DELETE FROM threads WHERE id = ?1 AND account_id = ?2", [&t.doc_id, account_id])?;
                    tx.execute("DELETE FROM thread_labels WHERE thread_id = ?1", [&t.doc_id])?;
                    tx.execute("DELETE FROM messages WHERE thread_id = ?1", [&t.doc_id])?;
                    tx.execute("DELETE FROM pending_overlay WHERE thread_id = ?1", [&t.doc_id])?;
                    touched.remove(&t.doc_id);
                }
                TombstoneTable::Messages => {
                    tx.execute("DELETE FROM messages WHERE id = ?1 AND account_id = ?2", [&t.doc_id, account_id])?;
                    tx.execute("DELETE FROM bodies WHERE message_id = ?1", [&t.doc_id])?;
                }
                TombstoneTable::Labels => {
                    labels_changed = true;
                    tx.execute("DELETE FROM labels WHERE id = ?1 AND account_id = ?2", [&t.doc_id, account_id])?;
                }
            }
        }

        tx.execute(
            "INSERT OR REPLACE INTO sync_state (account_id, rev) VALUES (?1, ?2)",
            params![account_id, to_i64(batch.up_to_rev)],
        )?;
        touched.extend(prune(&tx)?);
        if labels_changed {
            // Folder exclusivity may have changed: threads with pending local actions recompute.
            let mut stmt = tx.prepare("SELECT DISTINCT thread_id FROM pending_overlay WHERE account_id = ?1")?;
            let rows: Vec<String> = stmt.query_map([account_id], |r| r.get(0))?.collect::<rusqlite::Result<_>>()?;
            touched.extend(rows);
        }
        let mut contexts = Contexts::default();
        recompute_all(&tx, &mut contexts, &touched)?;
        tx.commit()?;
        Ok(FeedOutcome::Applied {
            rev: batch.up_to_rev,
            has_more: batch.has_more,
            threads: touched.into_iter().collect(),
        })
    }
}
