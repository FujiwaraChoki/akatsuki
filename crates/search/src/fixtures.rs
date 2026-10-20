//! Runner for the shared grammar fixtures (`packages/mail-model/fixtures/query.json`).
//!
//! Accepted shapes: an array of cases, or `{ "nowAt", "timezone", "cases": [...] }`. Each case is
//! `{ "input", "ast", "normalized"?, "errors"?, "nowAt"?, "timezone"? }`; `errors` (when present) must
//! match exactly, messages included.

use serde_json::Value;

use crate::query::{normalize, parse};

/// Default `nowAt` when a fixture does not set one: 2026-10-20T00:00:00Z.
pub const DEFAULT_NOW_AT: i64 = 1_792_540_800_000;

/// Runs every case and returns the list of mismatches as an error.
pub fn run_query_fixtures(text: &str) -> Result<usize, String> {
    let root: Value = serde_json::from_str(text).map_err(|e| format!("query.json is not JSON: {e}"))?;
    let (cases, now_default, tz_default) = match &root {
        Value::Array(cases) => (cases.clone(), DEFAULT_NOW_AT, "UTC".to_owned()),
        Value::Object(o) => (
            o.get("cases").and_then(Value::as_array).cloned().unwrap_or_default(),
            o.get("nowAt").and_then(Value::as_i64).unwrap_or(DEFAULT_NOW_AT),
            o.get("timezone").and_then(Value::as_str).unwrap_or("UTC").to_owned(),
        ),
        _ => return Err("query.json must be an array or an object with cases".into()),
    };
    let mut failures = Vec::new();
    for case in &cases {
        let Some(input) = case.get("input").and_then(Value::as_str) else { continue };
        let now = case.get("nowAt").and_then(Value::as_i64).unwrap_or(now_default);
        let tz = case.get("timezone").and_then(Value::as_str).unwrap_or(&tz_default);
        let result = parse(input, now, tz);
        let got = serde_json::to_value(&result.ast).map_err(|e| e.to_string())?;
        if let Some(expected) = case.get("ast")
            && &got != expected
        {
            failures.push(format!("{input:?}: ast {got} != {expected}"));
        }
        if let Some(expected) = case.get("errors") {
            let got = serde_json::to_value(&result.errors).map_err(|e| e.to_string())?;
            if &got != expected {
                failures.push(format!("{input:?}: errors {got} != {expected}"));
            }
        }
        if let Some(expected) = case.get("normalized").and_then(Value::as_str) {
            let n = normalize(&result.ast, tz);
            if n != expected {
                failures.push(format!("{input:?}: normalized {n:?} != {expected:?}"));
            }
        }
    }
    if failures.is_empty() { Ok(cases.len()) } else { Err(failures.join("\n")) }
}
