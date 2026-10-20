//! The local overlay of pending actions and the visible-state recompute (§5.1, §16.3).
//!
//! `visible = apply_ops(thread.labelKeys, overlay actions in insertion order)`. Every change to a
//! thread's server row or to its overlay calls [`recompute`], the only writer of `visible_keys`
//! and `thread_labels`.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use rusqlite::{Connection, OptionalExtension, params};

use super::{Result, Store, to_i64};
use crate::convex::ThreadAction;
use crate::convex::rows::ThreadMirror;
use crate::model::apply_ops::Effect;
use crate::model::{ApplyContext, Visible, apply_action, apply_ops};
use crate::time::now_ms;

/// One overlay row (for inspection and tests).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OverlayRow {
    /// Gesture id.
    pub client_op_id: String,
    /// Thread.
    pub thread_id: String,
    /// Account.
    pub account_id: String,
    /// The action.
    pub action: ThreadAction,
    /// Revision the mutation returned; `None` until it resolves.
    pub min_rev: Option<u64>,
}

/// Per-account [`ApplyContext`]s, built once per transaction.
#[derive(Default)]
pub(super) struct Contexts(HashMap<String, ApplyContext>);

impl Contexts {
    pub(super) fn get(&mut self, conn: &Connection, account_id: &str) -> Result<&ApplyContext> {
        if !self.0.contains_key(account_id) {
            let ctx = apply_context(conn, account_id)?;
            self.0.insert(account_id.to_owned(), ctx);
        }
        Ok(&self.0[account_id])
    }
}

/// Gmail has no folder keys; Graph and IMAP folders are exclusive (§5.1).
fn apply_context(conn: &Connection, account_id: &str) -> Result<ApplyContext> {
    let provider: Option<String> =
        conn.query_row("SELECT provider FROM accounts WHERE id = ?1", [account_id], |r| r.get(0)).optional()?;
    if provider.as_deref().is_none_or(|p| p == "gmail") {
        return Ok(ApplyContext::gmail());
    }
    let mut stmt = conn.prepare("SELECT label_key FROM labels WHERE account_id = ?1 AND exclusive = 1")?;
    let keys: Vec<String> = stmt.query_map([account_id], |r| r.get(0))?.collect::<rusqlite::Result<_>>()?;
    Ok(ApplyContext::folders(keys.iter().filter_map(|k| k.strip_prefix("l:"))))
}

fn overlay_actions(conn: &Connection, thread_id: &str) -> Result<Vec<ThreadAction>> {
    let mut stmt = conn.prepare("SELECT action_json FROM pending_overlay WHERE thread_id = ?1 ORDER BY seq")?;
    let rows: Vec<String> = stmt.query_map([thread_id], |r| r.get(0))?.collect::<rusqlite::Result<_>>()?;
    rows.iter().map(|j| Ok(serde_json::from_str(j)?)).collect()
}

/// The visible state of a thread before any of this device's pending actions.
pub(super) fn load_thread(conn: &Connection, thread_id: &str) -> Result<Option<ThreadMirror>> {
    let doc: Option<String> =
        conn.query_row("SELECT doc FROM threads WHERE id = ?1", [thread_id], |r| r.get(0)).optional()?;
    Ok(match doc {
        Some(d) => Some(serde_json::from_str(&d)?),
        None => None,
    })
}

/// Recomputes one thread's visible keys and list membership.
pub(super) fn recompute(conn: &Connection, contexts: &mut Contexts, thread_id: &str) -> Result<()> {
    conn.execute("DELETE FROM thread_labels WHERE thread_id = ?1", [thread_id])?;
    let Some(thread) = load_thread(conn, thread_id)? else { return Ok(()) };
    let actions = overlay_actions(conn, thread_id)?;
    let ctx = contexts.get(conn, thread.account_id.as_str())?;
    let visible = apply_ops(&thread.label_keys, actions.iter(), ctx);
    let deleted = visible.deleted || thread.deleted_pending;
    conn.execute(
        "UPDATE threads SET visible_keys = ?2, visible_deleted = ?3 WHERE id = ?1",
        params![thread_id, serde_json::to_string(&visible.keys)?, deleted],
    )?;
    if deleted {
        return Ok(());
    }
    let unread = visible.keys.iter().any(|k| k == "sys:unread");
    let mut insert = conn.prepare_cached(
        "INSERT OR REPLACE INTO thread_labels (thread_id, account_id, label_key, sort_at, is_unread)
         VALUES (?1, ?2, ?3, ?4, ?5)",
    )?;
    for key in &visible.keys {
        let sort_at =
            if key == "sys:snoozed" { thread.snoozed_until.unwrap_or(thread.sort_at) } else { thread.sort_at };
        insert.execute(params![thread_id, thread.account_id.as_str(), key, sort_at, unread])?;
    }
    if visible.keys.iter().any(|k| k == "sys:inbox") {
        let split = thread.split.clone().unwrap_or_else(|| "split:other".to_owned());
        insert.execute(params![thread_id, thread.account_id.as_str(), split, thread.sort_at, unread])?;
    }
    Ok(())
}

/// Recomputes every listed thread.
pub(super) fn recompute_all<'a>(
    conn: &Connection,
    contexts: &mut Contexts,
    threads: impl IntoIterator<Item = &'a String>,
) -> Result<()> {
    for t in threads {
        recompute(conn, contexts, t)?;
    }
    Ok(())
}

/// How long a resolved row without a revision may wait for the server state to match it.
pub const CONVERGE_TIMEOUT_MS: i64 = 60_000;

/// Drops overlay rows whose mutation resolved and whose effect the mirror now shows: either the
/// mirror reached the revision the mutation returned, or (for mutations that return no revision,
/// such as `mail.undo` and `mail.snooze`) the server state already equals the row's outcome, or it
/// has waited [`CONVERGE_TIMEOUT_MS`]. Returns the threads that changed.
pub(super) fn prune(conn: &Connection) -> Result<BTreeSet<String>> {
    let mut threads = BTreeSet::new();
    let by_rev: Vec<(i64, String)> = {
        let mut stmt = conn.prepare(
            "SELECT o.seq, o.thread_id FROM pending_overlay o
             LEFT JOIN sync_state s ON s.account_id = o.account_id
             WHERE o.min_rev IS NOT NULL AND o.min_rev <= COALESCE(s.rev, 0)",
        )?;
        stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<rusqlite::Result<_>>()?
    };
    for (seq, thread) in by_rev {
        conn.execute("DELETE FROM pending_overlay WHERE seq = ?1", [seq])?;
        threads.insert(thread);
    }
    let converging: Vec<(i64, String, String, i64)> = {
        let mut stmt = conn.prepare(
            "SELECT seq, thread_id, action_json, acked_at FROM pending_overlay WHERE min_rev IS NULL AND acked_at IS NOT NULL",
        )?;
        stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?.collect::<rusqlite::Result<_>>()?
    };
    let now = now_ms();
    let mut contexts = Contexts::default();
    for (seq, thread_id, action, acked_at) in converging {
        let done = if acked_at < now - CONVERGE_TIMEOUT_MS {
            true
        } else {
            match load_thread(conn, &thread_id)? {
                None => true,
                Some(t) => {
                    let action: ThreadAction = serde_json::from_str(&action)?;
                    let ctx = contexts.get(conn, t.account_id.as_str())?;
                    let server = apply_ops(&t.label_keys, [], ctx);
                    apply_action(&server, &action, ctx) == server
                }
            }
        };
        if done {
            conn.execute("DELETE FROM pending_overlay WHERE seq = ?1", [seq])?;
            threads.insert(thread_id);
        }
    }
    Ok(threads)
}

impl Store {
    /// Records a local action on threads (one row per thread) and recomputes them, before the
    /// mutation is sent (§16.1 optimistic update). Unknown threads are skipped. Returns the
    /// effect per thread (keys added and removed), which [`Store::add_undo`] inverts.
    pub fn add_overlay(
        &mut self,
        client_op_id: &str,
        thread_ids: &[String],
        action: &ThreadAction,
    ) -> Result<BTreeMap<String, Effect>> {
        let tx = self.conn.transaction()?;
        let mut contexts = Contexts::default();
        let mut effects = BTreeMap::new();
        let now = now_ms();
        let action_json = serde_json::to_string(action)?;
        for thread_id in thread_ids {
            let Some((account_id, before)) = visible_of(&tx, thread_id)? else { continue };
            let ctx = contexts.get(&tx, &account_id)?;
            let after = apply_action(&before, action, ctx);
            let effect = Effect::between(&before, &after);
            tx.execute(
                "INSERT INTO pending_overlay (client_op_id, thread_id, account_id, action_json, min_rev, created_at)
                 VALUES (?1, ?2, ?3, ?4, NULL, ?5)",
                params![client_op_id, thread_id, account_id, action_json, now],
            )?;
            tx.execute(
                "INSERT OR REPLACE INTO op_effects (client_op_id, thread_id, account_id, added_json, removed_json, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![
                    client_op_id,
                    thread_id,
                    account_id,
                    serde_json::to_string(&effect.added)?,
                    serde_json::to_string(&effect.removed)?,
                    now
                ],
            )?;
            recompute(&tx, &mut contexts, thread_id)?;
            effects.insert(thread_id.clone(), effect);
        }
        tx.commit()?;
        Ok(effects)
    }

    /// The mutation resolved with `rev` (account id → revision after the write, `mail.apply`'s
    /// result). Rows stay until the mirror reaches that revision, so the list never flickers back.
    /// Returns the threads that changed (rows pruned at once because the mirror is already there).
    pub fn ack_overlay(&mut self, client_op_id: &str, revs: &BTreeMap<String, u64>) -> Result<Vec<String>> {
        let tx = self.conn.transaction()?;
        let accounts: Vec<String> = {
            let mut stmt = tx.prepare("SELECT DISTINCT account_id FROM pending_overlay WHERE client_op_id = ?1")?;
            stmt.query_map([client_op_id], |r| r.get(0))?.collect::<rusqlite::Result<_>>()?
        };
        for account in accounts {
            // An account the mutation did not report has nothing to wait for.
            let rev = revs.get(&account).copied().unwrap_or(0);
            tx.execute(
                "UPDATE pending_overlay SET min_rev = ?3 WHERE client_op_id = ?1 AND account_id = ?2",
                params![client_op_id, account, to_i64(rev)],
            )?;
        }
        let changed = prune(&tx)?;
        let mut contexts = Contexts::default();
        recompute_all(&tx, &mut contexts, &changed)?;
        tx.commit()?;
        Ok(changed.into_iter().collect())
    }

    /// The mutation resolved but returned no revision (`mail.undo`, `mail.snooze`): rows are dropped
    /// once the server state shows their outcome (see [`prune`]). Returns the threads that changed.
    pub fn ack_overlay_when_converged(&mut self, client_op_id: &str) -> Result<Vec<String>> {
        let tx = self.conn.transaction()?;
        tx.execute(
            "UPDATE pending_overlay SET acked_at = ?2 WHERE client_op_id = ?1 AND min_rev IS NULL",
            params![client_op_id, now_ms()],
        )?;
        let changed = prune(&tx)?;
        let mut contexts = Contexts::default();
        recompute_all(&tx, &mut contexts, &changed)?;
        tx.commit()?;
        Ok(changed.into_iter().collect())
    }

    /// The mutation failed for good: drop its rows, so the threads show the server state again
    /// (rollback by construction, §5.3). Returns the threads that changed.
    pub fn drop_overlay(&mut self, client_op_id: &str) -> Result<Vec<String>> {
        let tx = self.conn.transaction()?;
        let threads: BTreeSet<String> = {
            let mut stmt = tx.prepare("SELECT thread_id FROM pending_overlay WHERE client_op_id = ?1")?;
            stmt.query_map([client_op_id], |r| r.get(0))?.collect::<rusqlite::Result<_>>()?
        };
        tx.execute("DELETE FROM pending_overlay WHERE client_op_id = ?1", [client_op_id])?;
        tx.execute("DELETE FROM op_effects WHERE client_op_id = ?1", [client_op_id])?;
        let mut contexts = Contexts::default();
        recompute_all(&tx, &mut contexts, &threads)?;
        tx.commit()?;
        Ok(threads.into_iter().collect())
    }

    /// Undo (§5.2): records the inverse of `undone_client_op_id`'s effect under the new gesture
    /// id, so the threads come back at once whatever the server has done so far. Returns the
    /// threads touched; empty when the action changed nothing (`mode: "noop"`).
    pub fn add_undo(&mut self, undone_client_op_id: &str, client_op_id: &str) -> Result<Vec<String>> {
        let tx = self.conn.transaction()?;
        let effects: Vec<(String, String, String, String)> = {
            let mut stmt = tx.prepare(
                "SELECT thread_id, account_id, added_json, removed_json FROM op_effects WHERE client_op_id = ?1",
            )?;
            stmt.query_map([undone_client_op_id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?
                .collect::<rusqlite::Result<_>>()?
        };
        let now = now_ms();
        let mut contexts = Contexts::default();
        let mut touched = Vec::new();
        for (thread_id, account_id, added, removed) in effects {
            let effect = Effect { added: serde_json::from_str(&added)?, removed: serde_json::from_str(&removed)? };
            if effect.is_empty() {
                continue;
            }
            tx.execute(
                "INSERT INTO pending_overlay (client_op_id, thread_id, account_id, action_json, min_rev, created_at)
                 VALUES (?1, ?2, ?3, ?4, NULL, ?5)",
                params![client_op_id, thread_id, account_id, serde_json::to_string(&effect.inverse())?, now],
            )?;
            recompute(&tx, &mut contexts, &thread_id)?;
            touched.push(thread_id);
        }
        tx.execute("DELETE FROM op_effects WHERE created_at < ?1", [now - crate::time::HOUR_MS])?;
        tx.commit()?;
        Ok(touched)
    }

    /// Overlay rows, oldest first.
    pub fn overlay(&self) -> Result<Vec<OverlayRow>> {
        let mut stmt = self.conn.prepare(
            "SELECT client_op_id, thread_id, account_id, action_json, min_rev FROM pending_overlay ORDER BY seq",
        )?;
        let rows: Vec<(String, String, String, String, Option<i64>)> = stmt
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)))?
            .collect::<rusqlite::Result<_>>()?;
        rows.into_iter()
            .map(|(client_op_id, thread_id, account_id, action, min_rev)| {
                Ok(OverlayRow {
                    client_op_id,
                    thread_id,
                    account_id,
                    action: serde_json::from_str(&action)?,
                    min_rev: min_rev.map(super::to_u64),
                })
            })
            .collect()
    }
}

/// `(account, visible state)` of a stored thread.
fn visible_of(conn: &Connection, thread_id: &str) -> Result<Option<(String, Visible)>> {
    let row: Option<(String, String, bool)> = conn
        .query_row("SELECT account_id, visible_keys, visible_deleted FROM threads WHERE id = ?1", [thread_id], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?))
        })
        .optional()?;
    Ok(match row {
        Some((account, keys, deleted)) => Some((account, Visible { keys: serde_json::from_str(&keys)?, deleted })),
        None => None,
    })
}
