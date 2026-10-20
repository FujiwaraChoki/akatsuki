//! SQLite schema of the desktop metadata mirror (ARCHITECTURE §16.3).
//!
//! Every row keeps the Convex document as JSON (`doc`) next to the few columns lists sort and
//! filter on, so the mirror never loses a field the server sends and needs no migration when a
//! view gains one. `messages_fts` lives in `crates/search`'s index file (D-311).

/// Bumped when the layout changes; an older file is dropped and re-pulled from rev 0.
pub const SCHEMA_VERSION: i64 = 1;

/// The tables. Statements are idempotent.
pub const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS meta (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS accounts (
    id TEXT PRIMARY KEY,
    email TEXT NOT NULL,
    provider TEXT NOT NULL,
    sort_order REAL NOT NULL,
    doc TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS labels (
    id TEXT PRIMARY KEY,
    account_id TEXT NOT NULL,
    label_key TEXT NOT NULL,
    name TEXT NOT NULL,
    exclusive INTEGER NOT NULL,
    hidden INTEGER NOT NULL,
    doc TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS labels_account ON labels(account_id);
CREATE TABLE IF NOT EXISTS threads (
    id TEXT PRIMARY KEY,
    account_id TEXT NOT NULL,
    rev INTEGER NOT NULL,
    sort_at INTEGER NOT NULL,
    visible_keys TEXT NOT NULL,
    visible_deleted INTEGER NOT NULL,
    doc TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS threads_account ON threads(account_id);
CREATE INDEX IF NOT EXISTS threads_sort ON threads(sort_at DESC);
CREATE TABLE IF NOT EXISTS thread_labels (
    thread_id TEXT NOT NULL,
    account_id TEXT NOT NULL,
    label_key TEXT NOT NULL,
    sort_at INTEGER NOT NULL,
    is_unread INTEGER NOT NULL,
    PRIMARY KEY (thread_id, label_key)
);
CREATE INDEX IF NOT EXISTS thread_labels_list ON thread_labels(label_key, sort_at DESC);
CREATE INDEX IF NOT EXISTS thread_labels_account ON thread_labels(account_id, label_key, sort_at DESC);
CREATE TABLE IF NOT EXISTS messages (
    id TEXT PRIMARY KEY,
    account_id TEXT NOT NULL,
    thread_id TEXT NOT NULL,
    date INTEGER NOT NULL,
    doc TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS messages_thread ON messages(thread_id, date);
CREATE INDEX IF NOT EXISTS messages_account ON messages(account_id);
CREATE TABLE IF NOT EXISTS sync_state (
    account_id TEXT PRIMARY KEY,
    rev INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS pending_overlay (
    seq INTEGER PRIMARY KEY AUTOINCREMENT,
    client_op_id TEXT NOT NULL,
    thread_id TEXT NOT NULL,
    account_id TEXT NOT NULL,
    action_json TEXT NOT NULL,
    min_rev INTEGER,
    acked_at INTEGER,
    created_at INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS overlay_thread ON pending_overlay(thread_id, seq);
CREATE INDEX IF NOT EXISTS overlay_op ON pending_overlay(client_op_id);
CREATE TABLE IF NOT EXISTS op_effects (
    client_op_id TEXT NOT NULL,
    thread_id TEXT NOT NULL,
    account_id TEXT NOT NULL,
    added_json TEXT NOT NULL,
    removed_json TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    PRIMARY KEY (client_op_id, thread_id)
);
CREATE TABLE IF NOT EXISTS outbox (
    seq INTEGER PRIMARY KEY AUTOINCREMENT,
    client_op_id TEXT NOT NULL UNIQUE,
    fn TEXT NOT NULL,
    args_json TEXT NOT NULL,
    attempts INTEGER NOT NULL DEFAULT 0,
    created_at INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS bodies (
    message_id TEXT PRIMARY KEY,
    sanitizer_version INTEGER NOT NULL,
    html_zst BLOB,
    text_zst BLOB,
    raw_zst BLOB,
    size INTEGER NOT NULL,
    fetched_at INTEGER NOT NULL,
    last_access INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS bodies_lru ON bodies(last_access);
CREATE TABLE IF NOT EXISTS ai_cache (
    key TEXT PRIMARY KEY,
    output_json TEXT NOT NULL,
    created_at INTEGER NOT NULL
);
"#;

/// Tables wiped on sign-out (§16.1), in dependency order.
pub const ALL_TABLES: &[&str] = &[
    "accounts",
    "labels",
    "threads",
    "thread_labels",
    "messages",
    "sync_state",
    "pending_overlay",
    "op_effects",
    "outbox",
    "bodies",
    "ai_cache",
];
