//! Lossless conversion between serde JSON and `convex::Value` (spike S9, ARCHITECTURE §19).
//!
//! - JSON → Convex uses Convex's tagged encoding: `{"$integer": …}` becomes `Int64` and
//!   `{"$bytes": …}` becomes `Bytes` (see `akatsuki_core::convex::values`); every other number is
//!   `Float64`, as in TypeScript.
//! - Convex → JSON writes `Int64` and `Bytes` back in tagged form so the serde wrappers can read
//!   them, and writes integral `Float64` values as JSON integers so Rust integer fields
//!   deserialize. Non-finite floats become `null` (no Akatsuki field may hold them).

use std::collections::BTreeMap;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::{Map, Number, Value as Json};

use crate::ConvexApiError;

/// The largest integer a `Float64` holds exactly.
const MAX_SAFE: f64 = 9_007_199_254_740_991.0;

/// Converts JSON to a Convex value.
pub fn json_to_convex(v: Json) -> Result<convex::Value, ConvexApiError> {
    convex::Value::try_from(v).map_err(|e| ConvexApiError::Encode(e.to_string()))
}

/// Converts a Convex value to JSON (see the module docs).
pub fn convex_to_json(v: convex::Value) -> Json {
    match v {
        convex::Value::Null => Json::Null,
        convex::Value::Int64(n) => {
            let mut m = Map::new();
            m.insert("$integer".into(), Json::String(STANDARD.encode(n.to_le_bytes())));
            Json::Object(m)
        }
        convex::Value::Float64(f) => {
            if f.is_finite() && f.fract() == 0.0 && f.abs() <= MAX_SAFE {
                // Exact: |f| ≤ 2^53 − 1.
                Json::Number(Number::from(f as i64))
            } else {
                Number::from_f64(f).map_or(Json::Null, Json::Number)
            }
        }
        convex::Value::Boolean(b) => Json::Bool(b),
        convex::Value::String(s) => Json::String(s),
        convex::Value::Bytes(b) => {
            let mut m = Map::new();
            m.insert("$bytes".into(), Json::String(STANDARD.encode(b)));
            Json::Object(m)
        }
        convex::Value::Array(a) => Json::Array(a.into_iter().map(convex_to_json).collect()),
        convex::Value::Object(o) => Json::Object(o.into_iter().map(|(k, v)| (k, convex_to_json(v))).collect()),
    }
}

/// Rewrites integral floats (`1.0`) as JSON integers in place, as [`convex_to_json`] does, for values
/// that arrive through Convex's HTTP API (`format: "json"`), where every `v.number()` is a float
/// (D-384). Non-integral and out-of-range numbers are left alone.
pub fn normalize_json_numbers(v: &mut Json) {
    match v {
        Json::Number(n) if n.as_i64().is_none() && n.as_u64().is_none() => {
            if let Some(f) = n.as_f64()
                && f.is_finite()
                && f.fract() == 0.0
                && f.abs() <= MAX_SAFE
            {
                *n = Number::from(f as i64);
            }
        }
        Json::Array(a) => a.iter_mut().for_each(normalize_json_numbers),
        Json::Object(o) => o.values_mut().for_each(normalize_json_numbers),
        _ => {}
    }
}

/// Serializes typed arguments to the `BTreeMap` the Convex client takes. Arguments must be an
/// object (Convex functions take a single object).
pub fn to_args<A: Serialize + ?Sized>(args: &A) -> Result<BTreeMap<String, convex::Value>, ConvexApiError> {
    let json = serde_json::to_value(args).map_err(|e| ConvexApiError::Encode(e.to_string()))?;
    let Json::Object(map) = json else {
        return Err(ConvexApiError::Encode("Convex arguments must be an object".into()));
    };
    map.into_iter().map(|(k, v)| Ok((k, json_to_convex(v)?))).collect()
}

/// Deserializes a function result.
pub fn from_json<R: DeserializeOwned>(function: &str, v: Json) -> Result<R, ConvexApiError> {
    serde_json::from_value(v)
        .map_err(|e| ConvexApiError::Decode { function: function.to_owned(), reason: e.to_string() })
}

#[cfg(test)]
mod tests {
    use super::*;
    use akatsuki_core::convex::{ConvexBytes, ConvexInt64};
    use serde_json::json;

    #[test]
    fn int64_and_bytes_survive_both_directions() {
        #[derive(serde::Serialize, serde::Deserialize, PartialEq, Debug)]
        struct S {
            v: ConvexInt64,
            b: ConvexBytes,
            n: u32,
            f: f64,
        }
        let s = S { v: ConvexInt64(9_007_199_254_740_993), b: ConvexBytes(vec![0, 255, 7]), n: 3, f: 1.5 };
        let args = to_args(&s).unwrap();
        assert_eq!(args["v"], convex::Value::Int64(9_007_199_254_740_993));
        assert_eq!(args["b"], convex::Value::Bytes(vec![0, 255, 7]));
        assert_eq!(args["n"], convex::Value::Float64(3.0));
        let back = convex_to_json(convex::Value::Object(args));
        let s2: S = from_json("t", back).unwrap();
        assert_eq!(s2, s);
    }

    #[test]
    fn http_json_numbers_normalize() {
        let mut v: Json = serde_json::from_str(r#"{"version":1.0,"n":[2.0,-0.5,1e300],"o":{"x":7}}"#).unwrap();
        normalize_json_numbers(&mut v);
        assert_eq!(v, json!({"version": 1, "n": [2, -0.5, 1e300], "o": {"x": 7}}));
        assert_eq!(serde_json::from_value::<u64>(v["version"].clone()).unwrap(), 1);
    }

    #[test]
    fn integral_floats_become_json_integers() {
        assert_eq!(convex_to_json(convex::Value::Float64(42.0)), json!(42));
        assert_eq!(convex_to_json(convex::Value::Float64(-0.5)), json!(-0.5));
        assert_eq!(convex_to_json(convex::Value::Float64(f64::NAN)), json!(null));
        assert_eq!(convex_to_json(convex::Value::Float64(1e300)), json!(1e300));
    }

    #[test]
    fn non_object_args_are_rejected() {
        assert!(to_args(&5).is_err());
    }
}
