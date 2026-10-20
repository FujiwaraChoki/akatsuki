//! The desktop metadata mirror (ARCHITECTURE §16.3, D-025): a SQLite copy of every thread,
//! message header and label of the user's accounts, fed by the per-account `rev` feed
//! (`sync.changes`), with a local overlay of pending actions on top.
//!
//! - **Feed** ([`Store::apply_changes`]): one `sync.changes` batch is applied in one SQLite
//!   transaction. Upserts are idempotent (a thread whose stored `rev` is newer is kept), tombstones
//!   delete, and the account's `rev` moves to `upToRev`. A batch that starts past the local rev is
//!   a gap and is refused; `resetRequired` wipes the account so the caller re-pulls from 0.
//! - **Visible state**: `apply_ops(server labelKeys, local overlay)` with the Rust twin of
//!   `applyOps` (§5.1). The server's `labelKeys` already include every op the server knows about;
//!   the overlay holds this device's actions until the mirror has caught up with the revision the
//!   mutation returned (`min_rev`), then the row is dropped (D-312).
//! - **Lists** read the materialized `thread_labels` table (one row per visible key, plus the
//!   split row of inbox threads), ordered by `sort_at`.
//! - **Device caches**: the durable outbox (§16.1), bodies (zstd, LRU, §16.1 2 GB cap) and the AI
//!   output cache. [`Store::wipe`] clears everything on sign-out.
//!
//! Bodies and AI outputs never leave the device; nothing here talks to the network.

mod cache;
mod feed;
mod overlay;
mod query;
pub mod schema;
#[cfg(test)]
mod tests;

use std::path::Path;

use rusqlite::{Connection, OptionalExtension, params};

use crate::convex::rows::{LabelView, MailAccountView};

pub use cache::{BODY_CACHE_MAX_BYTES, CachedBody, OutboxEntry};
pub use feed::FeedOutcome;
pub use overlay::OverlayRow;
pub use query::{ListQuery, ThreadSummary};

/// Errors from the mirror.
#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    /// SQLite failed.
    #[error("sqlite: {0}")]
    Sqlite(#[from] rusqlite::Error),
    /// A stored document did not decode.
    #[error("stored document: {0}")]
    Json(#[from] serde_json::Error),
    /// Compression failed.
    #[error("compression: {0}")]
    Io(#[from] std::io::Error),
}

/// Result alias.
pub type Result<T> = std::result::Result<T, StoreError>;

/// The mirror. One connection; callers share it behind a mutex.
pub struct Store {
    conn: Connection,
}

impl std::fmt::Debug for Store {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Store").finish_non_exhaustive()
    }
}

impl Store {
    /// Opens (or creates) the mirror file.
    pub fn open(path: &Path) -> Result<Self> {
        let conn = Connection::open(path)?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        Self::init(conn)
    }

    /// An in-memory mirror (tests, demo mode).
    pub fn open_in_memory() -> Result<Self> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(conn: Connection) -> Result<Self> {
        conn.busy_timeout(std::time::Duration::from_secs(5))?;
        conn.pragma_update(None, "foreign_keys", "OFF")?;
        conn.execute_batch("CREATE TABLE IF NOT EXISTS meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);")?;
        let version: Option<i64> = conn
            .query_row("SELECT value FROM meta WHERE key = 'schema_version'", [], |r| r.get::<_, String>(0))
            .optional()?
            .and_then(|v| v.parse().ok());
        if version.is_some_and(|v| v != schema::SCHEMA_VERSION) {
            // An older layout: drop the mirror; the feed re-pulls everything from rev 0.
            for table in schema::ALL_TABLES {
                conn.execute_batch(&format!("DROP TABLE IF EXISTS {table};"))?;
            }
        }
        conn.execute_batch(schema::SCHEMA)?;
        conn.execute(
            "INSERT OR REPLACE INTO meta (key, value) VALUES ('schema_version', ?1)",
            [schema::SCHEMA_VERSION.to_string()],
        )?;
        Ok(Self { conn })
    }

    /// Replaces the account list (`accounts.list`). Accounts that disappeared lose their mirrored
    /// data, overlay and revision.
    pub fn set_accounts(&mut self, accounts: &[MailAccountView]) -> Result<()> {
        let tx = self.conn.transaction()?;
        let existing: Vec<String> = {
            let mut stmt = tx.prepare("SELECT id FROM accounts")?;
            stmt.query_map([], |r| r.get(0))?.collect::<rusqlite::Result<_>>()?
        };
        for id in existing {
            if !accounts.iter().any(|a| a.id.as_str() == id) {
                feed::wipe_account(&tx, &id)?;
                tx.execute("DELETE FROM accounts WHERE id = ?1", [&id])?;
                tx.execute("DELETE FROM sync_state WHERE account_id = ?1", [&id])?;
                tx.execute("DELETE FROM pending_overlay WHERE account_id = ?1", [&id])?;
            }
        }
        for a in accounts {
            let provider = serde_json::to_value(a.provider)?;
            tx.execute(
                "INSERT OR REPLACE INTO accounts (id, email, provider, sort_order, doc) VALUES (?1, ?2, ?3, ?4, ?5)",
                params![
                    a.id.as_str(),
                    a.email,
                    provider.as_str().unwrap_or("gmail"),
                    a.sort_order,
                    serde_json::to_string(a)?
                ],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Accounts, by `sortOrder`.
    pub fn accounts(&self) -> Result<Vec<MailAccountView>> {
        let mut stmt = self.conn.prepare("SELECT doc FROM accounts ORDER BY sort_order, email")?;
        let docs: Vec<String> = stmt.query_map([], |r| r.get(0))?.collect::<rusqlite::Result<_>>()?;
        docs.iter().map(|d| Ok(serde_json::from_str(d)?)).collect()
    }

    /// Labels of one account, or of every account.
    pub fn labels(&self, account_id: Option<&str>) -> Result<Vec<LabelView>> {
        let mut stmt = self
            .conn
            .prepare("SELECT doc FROM labels WHERE (?1 IS NULL OR account_id = ?1) ORDER BY name COLLATE NOCASE")?;
        let docs: Vec<String> = stmt.query_map([account_id], |r| r.get(0))?.collect::<rusqlite::Result<_>>()?;
        docs.iter().map(|d| Ok(serde_json::from_str(d)?)).collect()
    }

    /// The revision the mirror holds for an account (0 when it holds nothing).
    pub fn account_rev(&self, account_id: &str) -> Result<u64> {
        let rev: Option<i64> = self
            .conn
            .query_row("SELECT rev FROM sync_state WHERE account_id = ?1", [account_id], |r| r.get(0))
            .optional()?;
        Ok(rev.map_or(0, |r| u64::try_from(r).unwrap_or(0)))
    }

    /// Deletes everything (sign-out, §6.3 step 8 and §16.1).
    pub fn wipe(&mut self) -> Result<()> {
        let tx = self.conn.transaction()?;
        for table in schema::ALL_TABLES {
            tx.execute(&format!("DELETE FROM {table}"), [])?;
        }
        tx.commit()?;
        Ok(())
    }
}

fn to_i64(v: u64) -> i64 {
    i64::try_from(v).unwrap_or(i64::MAX)
}

fn to_u64(v: i64) -> u64 {
    u64::try_from(v).unwrap_or(0)
}
