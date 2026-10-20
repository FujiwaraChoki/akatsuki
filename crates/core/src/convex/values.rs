//! Convex value types that JSON cannot express directly.
//!
//! The Rust `convex` client converts `serde_json::Value` to `convex::Value` using Convex's tagged
//! JSON encoding: `{"$integer": base64(i64 little-endian)}` for `v.int64()` and
//! `{"$bytes": base64}` for `v.bytes()`. These wrappers serialize to exactly those shapes, so a
//! serde struct can be turned into Convex arguments without loss (spike S9 in ARCHITECTURE §19).
//! Plain JSON numbers always become `Float64`.

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// A Convex `v.int64()` value (`bigint` in TypeScript).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ConvexInt64(pub i64);

impl Serialize for ConvexInt64 {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeMap;
        let mut map = serializer.serialize_map(Some(1))?;
        map.serialize_entry("$integer", &STANDARD.encode(self.0.to_le_bytes()))?;
        map.end()
    }
}

#[derive(Deserialize)]
#[serde(untagged)]
enum Int64Repr {
    Tagged {
        #[serde(rename = "$integer")]
        integer: String,
    },
    Number(i64),
    Text(String),
}

impl<'de> Deserialize<'de> for ConvexInt64 {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        match Int64Repr::deserialize(deserializer)? {
            Int64Repr::Tagged { integer } => {
                let bytes = STANDARD.decode(integer.as_bytes()).map_err(D::Error::custom)?;
                let arr: [u8; 8] =
                    bytes.try_into().map_err(|_| D::Error::custom("Int64 must be exactly eight bytes"))?;
                Ok(Self(i64::from_le_bytes(arr)))
            }
            Int64Repr::Number(n) => Ok(Self(n)),
            Int64Repr::Text(s) => s.parse().map(Self).map_err(D::Error::custom),
        }
    }
}

/// A Convex `v.bytes()` value (`ArrayBuffer` in TypeScript).
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct ConvexBytes(pub Vec<u8>);

impl Serialize for ConvexBytes {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeMap;
        let mut map = serializer.serialize_map(Some(1))?;
        map.serialize_entry("$bytes", &STANDARD.encode(&self.0))?;
        map.end()
    }
}

#[derive(Deserialize)]
#[serde(untagged)]
enum BytesRepr {
    Tagged {
        #[serde(rename = "$bytes")]
        bytes: String,
    },
    Text(String),
}

impl<'de> Deserialize<'de> for ConvexBytes {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let encoded = match BytesRepr::deserialize(deserializer)? {
            BytesRepr::Tagged { bytes } | BytesRepr::Text(bytes) => bytes,
        };
        STANDARD.decode(encoded.as_bytes()).map(Self).map_err(D::Error::custom)
    }
}

impl std::fmt::Display for ConvexInt64 {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn int64_matches_convex_encoding() {
        // Encoding used by the convex crate: base64(i64::to_le_bytes).
        let v = serde_json::to_value(ConvexInt64(1)).unwrap();
        assert_eq!(v, serde_json::json!({"$integer": "AQAAAAAAAAA="}));
        let back: ConvexInt64 = serde_json::from_value(v).unwrap();
        assert_eq!(back, ConvexInt64(1));
        let big = ConvexInt64(i64::MAX - 7);
        let back: ConvexInt64 = serde_json::from_value(serde_json::to_value(big).unwrap()).unwrap();
        assert_eq!(back, big);
    }

    #[test]
    fn bytes_matches_convex_encoding() {
        let v = serde_json::to_value(ConvexBytes(vec![0, 16, 131])).unwrap();
        assert_eq!(v, serde_json::json!({"$bytes": "ABCD"}));
        let back: ConvexBytes = serde_json::from_value(v).unwrap();
        assert_eq!(back.0, vec![0, 16, 131]);
    }
}
