//! Device caches next to the mirror: the durable outbox (§16.1), message bodies (zstd, LRU) and
//! AI outputs. None of this ever leaves the device.

use rusqlite::{OptionalExtension, params};

use super::{Result, Store, to_u64};
use crate::time::now_ms;

/// Desktop body cache cap (§16.1).
pub const BODY_CACHE_MAX_BYTES: u64 = 2 * 1024 * 1024 * 1024;

/// One durable outbox entry: a mutation to (re)send, in order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OutboxEntry {
    /// Idempotency key (`clientOpId`); replay is safe because the server dedupes on it.
    pub client_op_id: String,
    /// Convex function, e.g. `mail:apply`.
    pub function: String,
    /// Arguments.
    pub args: serde_json::Value,
    /// Send attempts so far.
    pub attempts: u32,
}

/// A cached body.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CachedBody {
    /// Sanitized HTML.
    pub html: Option<String>,
    /// Plain text.
    pub text: Option<String>,
    /// The raw RFC 822 bytes, when kept (re-sanitizing after a sanitizer bump needs no fetch).
    pub raw: Option<Vec<u8>>,
}

fn pack(data: Option<&[u8]>) -> Result<Option<Vec<u8>>> {
    Ok(match data {
        Some(d) => Some(zstd::encode_all(d, 3)?),
        None => None,
    })
}

fn unpack(data: Option<Vec<u8>>) -> Result<Option<Vec<u8>>> {
    Ok(match data {
        Some(d) => Some(zstd::decode_all(d.as_slice())?),
        None => None,
    })
}

fn utf8(data: Option<Vec<u8>>) -> Option<String> {
    data.map(|d| String::from_utf8_lossy(&d).into_owned())
}

impl Store {
    /// Appends a mutation to the outbox before it is sent. A second push with the same
    /// `clientOpId` is ignored.
    pub fn outbox_push(&mut self, client_op_id: &str, function: &str, args: &serde_json::Value) -> Result<()> {
        self.conn.execute(
            "INSERT OR IGNORE INTO outbox (client_op_id, fn, args_json, attempts, created_at) VALUES (?1, ?2, ?3, 0, ?4)",
            params![client_op_id, function, serde_json::to_string(args)?, now_ms()],
        )?;
        Ok(())
    }

    /// Entries in send order.
    pub fn outbox(&self) -> Result<Vec<OutboxEntry>> {
        let mut stmt = self.conn.prepare("SELECT client_op_id, fn, args_json, attempts FROM outbox ORDER BY seq")?;
        let rows: Vec<(String, String, String, i64)> = stmt
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?
            .collect::<rusqlite::Result<_>>()?;
        rows.into_iter()
            .map(|(client_op_id, function, args, attempts)| {
                Ok(OutboxEntry {
                    client_op_id,
                    function,
                    args: serde_json::from_str(&args)?,
                    attempts: u32::try_from(attempts).unwrap_or(u32::MAX),
                })
            })
            .collect()
    }

    /// Counts a failed send attempt.
    pub fn outbox_attempted(&mut self, client_op_id: &str) -> Result<()> {
        self.conn.execute("UPDATE outbox SET attempts = attempts + 1 WHERE client_op_id = ?1", [client_op_id])?;
        Ok(())
    }

    /// Removes an entry once its mutation resolved (either way).
    pub fn outbox_remove(&mut self, client_op_id: &str) -> Result<()> {
        self.conn.execute("DELETE FROM outbox WHERE client_op_id = ?1", [client_op_id])?;
        Ok(())
    }

    /// Caches a body, then evicts least-recently-used bodies above `max_bytes`.
    pub fn put_body(
        &mut self,
        message_id: &str,
        sanitizer_version: u32,
        html: Option<&str>,
        text: Option<&str>,
        raw: Option<&[u8]>,
        max_bytes: u64,
    ) -> Result<()> {
        let html = pack(html.map(str::as_bytes))?;
        let text = pack(text.map(str::as_bytes))?;
        let raw = pack(raw)?;
        let size = [&html, &text, &raw].iter().map(|b| b.as_ref().map_or(0, Vec::len)).sum::<usize>();
        let now = now_ms();
        self.conn.execute(
            "INSERT OR REPLACE INTO bodies (message_id, sanitizer_version, html_zst, text_zst, raw_zst, size, fetched_at, last_access)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?7)",
            params![message_id, sanitizer_version, html, text, raw, size as i64, now],
        )?;
        self.evict_bodies(max_bytes)?;
        Ok(())
    }

    /// A cached body made by `sanitizer_version` (older versions are a miss, §16.1), marking it
    /// recently used.
    pub fn body(&mut self, message_id: &str, sanitizer_version: u32) -> Result<Option<CachedBody>> {
        type Row = (Option<Vec<u8>>, Option<Vec<u8>>, Option<Vec<u8>>);
        let row: Option<Row> = self
            .conn
            .query_row(
                "SELECT html_zst, text_zst, raw_zst FROM bodies WHERE message_id = ?1 AND sanitizer_version = ?2",
                params![message_id, sanitizer_version],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()?;
        let Some((html, text, raw)) = row else { return Ok(None) };
        self.conn.execute("UPDATE bodies SET last_access = ?2 WHERE message_id = ?1", params![message_id, now_ms()])?;
        Ok(Some(CachedBody { html: utf8(unpack(html)?), text: utf8(unpack(text)?), raw: unpack(raw)? }))
    }

    /// The raw message of any sanitizer version (to re-sanitize locally after a bump).
    pub fn raw_body(&self, message_id: &str) -> Result<Option<Vec<u8>>> {
        let raw: Option<Option<Vec<u8>>> = self
            .conn
            .query_row("SELECT raw_zst FROM bodies WHERE message_id = ?1", [message_id], |r| r.get(0))
            .optional()?;
        unpack(raw.flatten())
    }

    /// Bytes held by the body cache.
    pub fn body_cache_bytes(&self) -> Result<u64> {
        let n: i64 = self.conn.query_row("SELECT COALESCE(sum(size), 0) FROM bodies", [], |r| r.get(0))?;
        Ok(to_u64(n))
    }

    /// Evicts least-recently-used bodies until the cache fits `max_bytes`.
    pub fn evict_bodies(&mut self, max_bytes: u64) -> Result<usize> {
        let mut total = self.body_cache_bytes()?;
        if total <= max_bytes {
            return Ok(0);
        }
        let victims: Vec<(String, i64)> = {
            let mut stmt = self.conn.prepare("SELECT message_id, size FROM bodies ORDER BY last_access, message_id")?;
            stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<rusqlite::Result<_>>()?
        };
        let mut evicted = 0;
        for (id, size) in victims {
            if total <= max_bytes {
                break;
            }
            self.conn.execute("DELETE FROM bodies WHERE message_id = ?1", [&id])?;
            total = total.saturating_sub(to_u64(size));
            evicted += 1;
        }
        Ok(evicted)
    }

    /// Drops every cached body (the user turned `cacheBodiesOnDevice` off).
    pub fn clear_bodies(&mut self) -> Result<()> {
        self.conn.execute("DELETE FROM bodies", [])?;
        Ok(())
    }

    /// AI output cache key: `(feature, inputHash, promptVersion, model)` (§16.2).
    pub fn ai_cache_key(feature: &str, input_hash: &str, prompt_version: u32, model: &str) -> String {
        format!("{feature}|{input_hash}|{prompt_version}|{model}")
    }

    /// A cached AI output.
    pub fn ai_cached(&self, key: &str) -> Result<Option<serde_json::Value>> {
        let v: Option<String> =
            self.conn.query_row("SELECT output_json FROM ai_cache WHERE key = ?1", [key], |r| r.get(0)).optional()?;
        Ok(match v {
            Some(v) => Some(serde_json::from_str(&v)?),
            None => None,
        })
    }

    /// Caches an AI output.
    pub fn put_ai_cached(&mut self, key: &str, output: &serde_json::Value) -> Result<()> {
        self.conn.execute(
            "INSERT OR REPLACE INTO ai_cache (key, output_json, created_at) VALUES (?1, ?2, ?3)",
            params![key, serde_json::to_string(output)?, now_ms()],
        )?;
        Ok(())
    }
}
