//! Time conventions (ARCHITECTURE §1.3).
//!
//! An instant is a number of **milliseconds since the Unix epoch, UTC**. Durations are carried in
//! fields ending in `Ms`.

use std::time::{SystemTime, UNIX_EPOCH};

/// Milliseconds since the Unix epoch, UTC. The only instant type on the wire.
pub type UnixMs = i64;

/// One second in milliseconds.
pub const SECOND_MS: i64 = 1_000;
/// One minute in milliseconds.
pub const MINUTE_MS: i64 = 60 * SECOND_MS;
/// One hour in milliseconds.
pub const HOUR_MS: i64 = 60 * MINUTE_MS;
/// One day in milliseconds.
pub const DAY_MS: i64 = 24 * HOUR_MS;

/// The current instant. Never call this inside code that must be deterministic (Convex queries,
/// fixture-driven pure functions); pass `now` in instead.
pub fn now_ms() -> UnixMs {
    let since = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default();
    i64::try_from(since.as_millis()).unwrap_or(i64::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn now_is_after_2026() {
        // 2026-01-01T00:00:00Z
        assert!(now_ms() > 1_767_225_600_000);
    }

    #[test]
    fn constants_compose() {
        assert_eq!(DAY_MS, 86_400_000);
    }
}
