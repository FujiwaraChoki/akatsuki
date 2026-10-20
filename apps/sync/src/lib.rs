//! # akatsuki-sync (library)
//!
//! The worker: configuration, the HTTP API (§7), the sync engine (§9: account actors, the
//! supervisor, `ConvexSink`), crypto for secrets, OAuth state and the upload spool (§14.1, §10.4),
//! the SSRF-guarded fetcher behind the image proxy (§14.4), OAuth connect flows (§6.7) and push
//! hooks (§9.6). The binary (`main.rs`) wires them together.

pub mod api;
pub mod app;
pub mod capability;
pub mod config;
pub mod crypto;
pub mod engine;
pub mod fetch;
pub mod http;
pub mod limits;
pub mod metrics;
pub mod oauth;
pub mod providers;
pub mod push_auth;
pub mod rotate;
pub mod spool;
pub mod triage;
pub mod user_convex;

#[cfg(test)]
mod tests;
#[cfg(test)]
mod tests_handlers;
