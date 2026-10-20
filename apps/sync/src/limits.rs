//! Per-user limits of the worker's HTTP API (ARCHITECTURE §13.4, DECISIONS D-397).
//!
//! All state is in memory (one worker process in v1, D-044; a restart resets it), keyed by the
//! JWT `sub`. Every refusal is `429 rate_limited` with `Retry-After`.
//!
//! | What | Limit |
//! |---|---|
//! | Body, part and prefetch requests | 30/s, burst 200 |
//! | Concurrent raw reads (each holds up to 64 MB) | 4 per user, 8 for the process |
//! | Attachment and raw-message egress | 2 GB per UTC day |
//! | Uploads | 200 MB/h; at most 50 files and 250 MB alive (24 h) per user; 2 GiB in the spool |
//! | `/v1/connect/*` (OAuth start, IMAP connect) | 10/h |
//! | IMAP discovery | 60/h |

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use akatsuki_core::sync::budget::TokenBucket;
use akatsuki_core::time::{DAY_MS, HOUR_MS, now_ms};
use akatsuki_core::wire::ErrorCode;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

use crate::http::ApiError;

/// Body, part and prefetch requests: refill per second.
pub const READS_PER_SEC: u32 = 30;
/// Body, part and prefetch requests: burst.
pub const READS_BURST: u32 = 200;
/// Concurrent raw reads per user.
pub const RAW_PER_USER: usize = 4;
/// Concurrent raw reads per process (64 MB each at most, inside the 2 GB container limit).
pub const RAW_GLOBAL: usize = 8;
/// How long a read waits for a raw slot before it is refused.
pub const RAW_WAIT: Duration = Duration::from_secs(15);
/// Attachment and raw egress per user per UTC day.
pub const EGRESS_PER_DAY: u64 = 2 * 1024 * 1024 * 1024;
/// Upload bytes per user per hour.
pub const UPLOAD_BYTES_PER_HOUR: u32 = 200 * 1024 * 1024;
/// Live uploads per user (the spool keeps a file 24 h).
pub const UPLOAD_FILES_ALIVE: usize = 50;
/// Live upload bytes per user.
pub const UPLOAD_BYTES_ALIVE: u64 = 250 * 1024 * 1024;
/// The spool as a whole (a bind mount on a shared disk).
pub const SPOOL_MAX_BYTES: u64 = 2 * 1024 * 1024 * 1024;
/// OAuth starts and IMAP connects per user per hour.
pub const CONNECTS_PER_HOUR: u32 = 10;
/// IMAP discovery lookups per user per hour.
pub const DISCOVERS_PER_HOUR: u32 = 60;
/// Keys kept per table before idle entries are dropped.
const MAX_KEYS: usize = 50_000;

fn limited(retry_after_ms: i64) -> ApiError {
    ApiError::new(ErrorCode::RateLimited).retry_after(u64::try_from(retry_after_ms.max(1_000)).unwrap_or(1_000))
}

/// Token buckets by user.
#[derive(Debug)]
struct Buckets {
    capacity: u32,
    period_ms: i64,
    map: Mutex<HashMap<String, TokenBucket>>,
}

impl Buckets {
    fn new(capacity: u32, period_ms: i64) -> Self {
        Self { capacity, period_ms, map: Mutex::new(HashMap::new()) }
    }

    fn take(&self, user: &str, cost: u32) -> Result<(), ApiError> {
        let now = now_ms();
        let mut m = self.map.lock().unwrap_or_else(|e| e.into_inner());
        if m.len() > MAX_KEYS {
            // Buckets that refilled completely carry no state worth keeping.
            let full = f64::from(self.capacity);
            m.retain(|_, b| b.available(now) < full);
        }
        m.entry(user.to_owned())
            .or_insert_with(|| TokenBucket::new(self.capacity, self.period_ms, now))
            .try_take(cost, now)
            .map_err(limited)
    }
}

/// One user's upload history inside the spool TTL.
#[derive(Debug, Default)]
struct Alive {
    files: Vec<(Instant, u64)>,
}

/// The per-user limits.
#[derive(Debug)]
pub struct Limits {
    reads: Buckets,
    connects: Buckets,
    discovers: Buckets,
    upload_bytes: Buckets,
    alive: Mutex<HashMap<String, Alive>>,
    egress: Mutex<HashMap<String, (i64, u64)>>,
    raw_users: Mutex<HashMap<String, Arc<Semaphore>>>,
    raw_global: Arc<Semaphore>,
    spool_ttl: Duration,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            // 30/s with a burst of 200: a bucket of 200 that refills in 200/30 s.
            reads: Buckets::new(READS_BURST, i64::from(READS_BURST) * 1000 / i64::from(READS_PER_SEC)),
            connects: Buckets::new(CONNECTS_PER_HOUR, HOUR_MS),
            discovers: Buckets::new(DISCOVERS_PER_HOUR, HOUR_MS),
            upload_bytes: Buckets::new(UPLOAD_BYTES_PER_HOUR, HOUR_MS),
            alive: Mutex::new(HashMap::new()),
            egress: Mutex::new(HashMap::new()),
            raw_users: Mutex::new(HashMap::new()),
            raw_global: Arc::new(Semaphore::new(RAW_GLOBAL)),
            spool_ttl: crate::spool::SPOOL_TTL,
        }
    }
}

/// Both raw-read slots; dropped when the read is done.
#[derive(Debug)]
pub struct RawSlot {
    _user: OwnedSemaphorePermit,
    _global: OwnedSemaphorePermit,
}

impl Limits {
    /// One body, part or prefetch request.
    pub fn read(&self, user: &str) -> Result<(), ApiError> {
        self.reads.take(user, 1)
    }

    /// One OAuth start or IMAP connect.
    pub fn connect(&self, user: &str) -> Result<(), ApiError> {
        self.connects.take(user, 1)
    }

    /// One IMAP discovery lookup.
    pub fn connect_discover(&self, user: &str) -> Result<(), ApiError> {
        self.discovers.take(user, 1)
    }

    /// Waits for a raw-read slot (per user and per process), at most [`RAW_WAIT`].
    pub async fn raw_slot(&self, user: &str) -> Result<RawSlot, ApiError> {
        let per_user = {
            let mut m = self.raw_users.lock().unwrap_or_else(|e| e.into_inner());
            if m.len() > MAX_KEYS {
                m.retain(|_, s| s.available_permits() < RAW_PER_USER);
            }
            m.entry(user.to_owned()).or_insert_with(|| Arc::new(Semaphore::new(RAW_PER_USER))).clone()
        };
        let busy = || limited(2_000);
        let wait = async {
            let u = per_user.acquire_owned().await.map_err(|_| busy())?;
            let g = self.raw_global.clone().acquire_owned().await.map_err(|_| busy())?;
            Ok::<_, ApiError>(RawSlot { _user: u, _global: g })
        };
        tokio::time::timeout(RAW_WAIT, wait).await.map_err(|_| busy())?
    }

    /// Refuses when today's egress is used up (checked before the provider is asked).
    pub fn egress_open(&self, user: &str) -> Result<(), ApiError> {
        let day = now_ms().div_euclid(DAY_MS);
        let m = self.egress.lock().unwrap_or_else(|e| e.into_inner());
        match m.get(user) {
            Some((d, used)) if *d == day && *used >= EGRESS_PER_DAY => Err(limited(until_next_day())),
            _ => Ok(()),
        }
    }

    /// Charges `bytes` of egress; refuses (and charges nothing) when they do not fit today.
    pub fn egress(&self, user: &str, bytes: u64) -> Result<(), ApiError> {
        let day = now_ms().div_euclid(DAY_MS);
        let mut m = self.egress.lock().unwrap_or_else(|e| e.into_inner());
        if m.len() > MAX_KEYS {
            m.retain(|_, (d, _)| *d == day);
        }
        let e = m.entry(user.to_owned()).or_insert((day, 0));
        if e.0 != day {
            *e = (day, 0);
        }
        if e.1.saturating_add(bytes) > EGRESS_PER_DAY {
            return Err(limited(until_next_day()));
        }
        e.1 += bytes;
        Ok(())
    }

    /// Admits an upload of `bytes`: the hourly byte budget, the user's live files and bytes, and
    /// the spool total (`spool_bytes`, measured by the caller).
    pub fn upload(&self, user: &str, bytes: u64, spool_bytes: u64) -> Result<(), ApiError> {
        if spool_bytes.saturating_add(bytes) > SPOOL_MAX_BYTES {
            return Err(ApiError::with_message(
                ErrorCode::RateLimited,
                "Uploads are paused on this server. Try again later.",
            )
            .retry_after(10 * 60 * 1000));
        }
        let now = Instant::now();
        {
            let mut m = self.alive.lock().unwrap_or_else(|e| e.into_inner());
            let a = m.entry(user.to_owned()).or_default();
            a.files.retain(|(at, _)| now.duration_since(*at) < self.spool_ttl);
            let live: u64 = a.files.iter().map(|(_, n)| n).sum();
            if a.files.len() >= UPLOAD_FILES_ALIVE || live.saturating_add(bytes) > UPLOAD_BYTES_ALIVE {
                let oldest = a.files.iter().map(|(at, _)| *at).min().unwrap_or(now);
                let wait = self.spool_ttl.saturating_sub(now.duration_since(oldest));
                return Err(limited(i64::try_from(wait.as_millis()).unwrap_or(i64::MAX)));
            }
        }
        self.upload_bytes.take(user, u32::try_from(bytes).unwrap_or(u32::MAX))?;
        self.alive
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .entry(user.to_owned())
            .or_default()
            .files
            .push((now, bytes));
        Ok(())
    }
}

fn until_next_day() -> i64 {
    DAY_MS - now_ms().rem_euclid(DAY_MS)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn code(r: Result<(), ApiError>) -> Option<ErrorCode> {
        r.err().map(|e| e.0.code)
    }

    #[test]
    fn reads_allow_a_burst_then_refuse() {
        let l = Limits::default();
        for _ in 0..READS_BURST {
            l.read("u1").unwrap();
        }
        assert_eq!(code(l.read("u1")), Some(ErrorCode::RateLimited));
        l.read("u2").unwrap();
    }

    #[test]
    fn connects_are_ten_an_hour() {
        let l = Limits::default();
        for _ in 0..CONNECTS_PER_HOUR {
            l.connect("u1").unwrap();
        }
        let e = l.connect("u1").unwrap_err();
        assert_eq!(e.0.code, ErrorCode::RateLimited);
        assert!(e.0.retry_after_ms.unwrap() >= 60_000, "about six minutes for the next token");
        for _ in 0..DISCOVERS_PER_HOUR {
            l.connect_discover("u1").unwrap();
        }
        assert_eq!(code(l.connect_discover("u1")), Some(ErrorCode::RateLimited));
    }

    #[test]
    fn egress_is_two_gigabytes_a_day() {
        let l = Limits::default();
        l.egress("u1", EGRESS_PER_DAY - 10).unwrap();
        l.egress_open("u1").unwrap();
        assert_eq!(code(l.egress("u1", 11)), Some(ErrorCode::RateLimited));
        l.egress("u1", 10).unwrap();
        assert_eq!(code(l.egress_open("u1")), Some(ErrorCode::RateLimited));
        l.egress_open("u2").unwrap();
    }

    #[test]
    fn uploads_are_bounded_per_hour_alive_and_in_the_spool() {
        let l = Limits::default();
        const MB: u64 = 1024 * 1024;
        // 200 MB in an hour: eight 25 MB files pass, the ninth does not.
        for _ in 0..8 {
            l.upload("u1", 25 * MB, 0).unwrap();
        }
        assert_eq!(code(l.upload("u1", 25 * MB, 0)), Some(ErrorCode::RateLimited));
        // Fifty small files alive at most.
        for _ in 0..UPLOAD_FILES_ALIVE {
            l.upload("u2", 1024, 0).unwrap();
        }
        assert_eq!(code(l.upload("u2", 1024, 0)), Some(ErrorCode::RateLimited));
        // A full spool refuses everyone.
        assert_eq!(code(l.upload("u3", MB, SPOOL_MAX_BYTES)), Some(ErrorCode::RateLimited));
        l.upload("u3", MB, SPOOL_MAX_BYTES - MB).unwrap();
    }

    #[tokio::test]
    async fn raw_slots_are_bounded_per_user_and_released() {
        let l = Limits::default();
        let mut held = Vec::new();
        for _ in 0..RAW_PER_USER {
            held.push(l.raw_slot("u1").await.unwrap());
        }
        tokio::time::pause();
        let waiting = l.raw_slot("u1");
        tokio::pin!(waiting);
        assert!(futures::poll!(&mut waiting).is_pending());
        tokio::time::advance(RAW_WAIT + Duration::from_millis(1)).await;
        assert_eq!(waiting.await.unwrap_err().0.code, ErrorCode::RateLimited);
        held.pop();
        l.raw_slot("u1").await.unwrap();
    }
}
