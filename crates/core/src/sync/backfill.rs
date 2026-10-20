//! The initial window, backfill and resync bookkeeping (ARCHITECTURE §9.3).
//!
//! - The **window** is everything after `windowStartAt` (connect time − 90 days).
//! - **Backfill** walks backwards from `backfillBeforeAt` (initially `windowStartAt`) to
//!   `backfillHorizonAt` (connect − 365 days by default, `0` = everything, capped at 250k messages).
//!   Its cursor is a date (`CursorUpdate::Backfill`), never a page token, so it survives restarts.
//! - **Resync** re-lists the window and decides what to re-fetch and what to delete.

use std::collections::{BTreeMap, BTreeSet};

use super::{BACKFILL_MESSAGE_CAP, DEFAULT_HORIZON_MS, WINDOW_MS};
use crate::time::UnixMs;

/// Threads per backfill page.
pub const BACKFILL_PAGE: u32 = 100;

/// The time bounds of one account's sync.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SyncWindow {
    /// `windowStartAt`.
    pub window_start_at: UnixMs,
    /// `backfillHorizonAt` (0 = everything).
    pub horizon_at: UnixMs,
}

impl SyncWindow {
    /// Bounds from the connect time with the default horizon. The worker does not receive
    /// `mailAccounts.windowStartAt`/`backfillHorizonAt`, so it derives them from the `syncState`
    /// creation time, which is the connect time (DECISIONS D-232).
    pub fn from_connect(connected_at: UnixMs) -> Self {
        Self {
            window_start_at: connected_at.saturating_sub(WINDOW_MS),
            horizon_at: connected_at.saturating_sub(DEFAULT_HORIZON_MS).max(0),
        }
    }

    /// With an explicit horizon (`0` = everything).
    pub fn with_horizon(self, horizon_at: UnixMs) -> Self {
        Self { horizon_at: horizon_at.max(0), ..self }
    }

    /// Where backfill continues: the stored date cursor, else the window start.
    pub fn backfill_before(&self, stored: Option<UnixMs>) -> UnixMs {
        stored.unwrap_or(self.window_start_at)
    }

    /// Whether backfill has nothing left to do at `before`.
    pub fn backfill_done(&self, before: UnixMs, messages_synced: u64) -> bool {
        before <= self.horizon_at || messages_synced >= BACKFILL_MESSAGE_CAP
    }
}

/// Outcome of one backfill page.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BackfillProgress {
    /// Commit `before` as the new date cursor and continue.
    Continue {
        /// The new `backfillBeforeAt`.
        before: UnixMs,
    },
    /// The horizon (or the message cap) is reached; commit `before` and mark the phase complete.
    Done {
        /// The final `backfillBeforeAt`.
        before: UnixMs,
    },
}

/// Advances the backfill cursor after a page. `oldest` is the oldest message date the page saw.
/// An empty page, or a page that reached the horizon, ends backfill. The cursor always moves
/// strictly backwards so a provider that keeps returning the same page cannot loop forever.
pub fn advance(
    window: &SyncWindow,
    before: UnixMs,
    page_threads: usize,
    oldest: Option<UnixMs>,
    messages_synced: u64,
) -> BackfillProgress {
    let next = match oldest {
        Some(o) if o < before => o,
        _ => before.saturating_sub(1),
    }
    .max(window.horizon_at);
    if page_threads == 0 || window.backfill_done(next, messages_synced) {
        BackfillProgress::Done { before: if page_threads == 0 { window.horizon_at } else { next } }
    } else {
        BackfillProgress::Continue { before: next }
    }
}

/// What a resync must do after re-listing the window (§9.3 step 6).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ResyncPlan {
    /// Threads to fetch again: missing in Convex or whose provider version moved.
    pub refetch: Vec<String>,
    /// Convex threads inside the window that the listing no longer has.
    pub delete: Vec<String>,
}

/// Compares the provider listing with what Convex holds inside the window. `known` maps provider
/// thread ids to their stored `providerVersion`; `listed` maps provider ids to the version the
/// listing reported (`None` when the provider does not report one, which forces a re-fetch).
pub fn resync_plan(known: &BTreeMap<String, Option<i64>>, listed: &BTreeMap<String, Option<i64>>) -> ResyncPlan {
    let refetch = listed
        .iter()
        .filter(|(id, v)| match (known.get(*id), v) {
            (None, _) | (_, None) => true,
            (Some(old), Some(new)) => old.is_none_or(|o| *new > o),
        })
        .map(|(id, _)| id.clone())
        .collect();
    let listed_ids: BTreeSet<&String> = listed.keys().collect();
    let delete = known.keys().filter(|id| !listed_ids.contains(id)).cloned().collect();
    ResyncPlan { refetch, delete }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::time::DAY_MS;

    #[test]
    fn window_from_connect() {
        let w = SyncWindow::from_connect(400 * DAY_MS);
        assert_eq!(w.window_start_at, 310 * DAY_MS);
        assert_eq!(w.horizon_at, 35 * DAY_MS);
        assert_eq!(w.backfill_before(None), 310 * DAY_MS);
        assert_eq!(w.backfill_before(Some(100)), 100);
        assert_eq!(SyncWindow::from_connect(10).horizon_at, 0);
        assert_eq!(w.with_horizon(0).horizon_at, 0);
    }

    #[test]
    fn backfill_moves_backwards_and_stops_at_the_horizon() {
        let w = SyncWindow { window_start_at: 1_000, horizon_at: 100 };
        assert_eq!(advance(&w, 1_000, 50, Some(600), 0), BackfillProgress::Continue { before: 600 });
        // A provider that reports nothing older still makes progress.
        assert_eq!(advance(&w, 600, 50, Some(700), 0), BackfillProgress::Continue { before: 599 });
        assert_eq!(advance(&w, 600, 50, Some(50), 0), BackfillProgress::Done { before: 100 });
        assert_eq!(advance(&w, 600, 0, None, 0), BackfillProgress::Done { before: 100 });
        assert_eq!(advance(&w, 600, 10, Some(500), BACKFILL_MESSAGE_CAP), BackfillProgress::Done { before: 500 });
    }

    #[test]
    fn resync_refetches_moved_and_missing_and_deletes_gone() {
        let known: BTreeMap<String, Option<i64>> =
            [("a".into(), Some(5)), ("b".into(), Some(5)), ("gone".into(), Some(1)), ("nov".into(), None)].into();
        let listed: BTreeMap<String, Option<i64>> =
            [("a".into(), Some(5)), ("b".into(), Some(6)), ("new".into(), Some(1)), ("nov".into(), Some(2))].into();
        let p = resync_plan(&known, &listed);
        assert_eq!(p.refetch, vec!["b".to_owned(), "new".into(), "nov".into()]);
        assert_eq!(p.delete, vec!["gone".to_owned()]);
        // Without known state (no Convex listing), everything listed is fetched and nothing deleted.
        let p = resync_plan(&BTreeMap::new(), &listed);
        assert_eq!(p.refetch.len(), 4);
        assert!(p.delete.is_empty());
    }
}
