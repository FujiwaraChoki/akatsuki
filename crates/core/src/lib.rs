//! # akatsuki-core
//!
//! The shared Rust heart of Akatsuki: the mail domain model, the Convex document shapes, the
//! worker and apps/api wire types, the AI stream protocol (AASP), the error-code registry, the
//! sync-engine state machine and the user/service JWT helpers.
//!
//! Everything here is the Rust twin of a TypeScript definition named in
//! `docs/ARCHITECTURE.md`. Field names and enum values are exact: fields are `camelCase` on the
//! wire and string enum values are `snake_case` (§1.1). Optional Convex fields are omitted, never
//! sent as `null`, because a Convex `v.optional(...)` validator rejects `null`.
//!
//! Module map (ARCHITECTURE §18, row `crates/core`):
//!
//! | Module | Contract |
//! |---|---|
//! | [`time`] | §1.3 `UnixMs` |
//! | [`ids`] | §1.2 typed Convex document ids |
//! | [`model`] | §1.4 label keys, §5.1 `apply_ops`, §3.4 rule matching, shared mail types |
//! | [`convex`] | §3 schema shapes, §4.3/§4.4 worker function arguments and row projections, §5.4 ops |
//! | [`wire`] | §7.3 worker HTTP types, §8 apps/api types, §12.6 AASP, §15 error codes |
//! | [`sync`] | §9.2 account actor phases, commands and scheduling; §5.5 backoff |
//! | [`auth`] | §6.5 user JWT verification, §6.6 service JWT minting |
//! | [`net`] | §14.4 outbound address policy |
//! | `store` (feature `store`) | §16.3 desktop SQLite metadata mirror |

pub mod auth;
pub mod convex;
pub mod ids;
pub mod model;
pub mod net;
#[cfg(feature = "store")]
pub mod store;
pub mod sync;
pub mod time;
pub mod wire;

pub use ids::Id;
pub use model::{Addr, LabelKey};
pub use time::UnixMs;
pub use wire::ErrorCode;
