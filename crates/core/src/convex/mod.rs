//! Rust twins of the Convex shapes (ARCHITECTURE §3 and §4).
//!
//! - [`schema`]: shared validators and enum columns (`convex/lib/validators.ts`, `convex/schema.ts`).
//! - [`rows`]: row projections returned to clients (`convex/lib/rows.ts`) and AI settings.
//! - [`worker`]: worker service function arguments/results and the ops protocol (§4.3, §5.4).
//! - [`client`]: client-facing functions the desktop app calls (§4.2).
//! - [`values`]: `v.int64()` and `v.bytes()` wrappers that survive the JSON bridge.
//!
//! These are hand-written to the contract until `cargo xtask convex-gen` generates
//! `crates/convex-api` from `npx convex function-spec` (§18); the generated bindings must keep
//! these names and shapes.

pub mod client;
pub mod rows;
pub mod schema;
pub mod values;
pub mod worker;

pub use rows::*;
pub use schema::*;
pub use values::{ConvexBytes, ConvexInt64};
pub use worker::{ClaimedOp, CursorUpdate, MessageUpsert, OpResult, ThreadUpsert, TriageResult, UpsertThreadsResult};
