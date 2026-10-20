//! # akatsuki-search
//!
//! Search for Akatsuki (ARCHITECTURE §11):
//!
//! - [`query`]: the one query grammar (`from:`, `to:`, `subject:`, `has:attachment`, `is:unread`,
//!   `label:`, `before:`, `after:`, quoted phrases, negation, `OR`, groups), the Rust twin of
//!   `packages/mail-model/src/query/` (interim: `convex/lib/query.ts`, D-126).
//! - [`index`]: the desktop SQLite FTS5 index that runs the full grammar offline.
//! - [`date`]: time-zone-aware date resolution for the grammar.
//! - [`gmail`]: translation into a Gmail `q` string (provider search, §11.2).
//! - [`fixtures`]: the runner for `packages/mail-model/fixtures/query.json`.

pub mod date;
pub mod fixtures;
pub mod gmail;
pub mod index;
pub mod query;

pub use index::{IndexDoc, IndexError, SearchContext, SearchIndex, ThreadHit};
pub use query::{ParseResult, QueryAst, normalize, parse};

/// `true` when a `label:` value is already a label key (`sys:…` or `l:…`) rather than a name.
pub(crate) fn is_label_key_like(value: &str) -> bool {
    value.starts_with("sys:") || value.starts_with("l:")
}
