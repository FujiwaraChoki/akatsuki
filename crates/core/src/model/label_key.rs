//! Label keys: the one vocabulary for thread state and lists (ARCHITECTURE §1.4).
//!
//! ```text
//! labelKey = sysKey | userKey | splitKey
//! sysKey   = "sys:" ( inbox | sent | drafts | trash | spam | starred | important | unread | snoozed )
//! userKey  = "l:" <Id<"labels">>
//! splitKey = "split:" <Id<"splits">> | "split:other"
//! ```
//!
//! Convex ids are opaque; like `convex/lib/model.ts` the parser requires 8 to 64 lowercase ASCII
//! letters or digits after `l:` / `split:`. This module and `parseLabelKey` (`convex/lib/model.ts` until
//! `packages/mail-model/src/labelKeys.ts` exists, D-126) are the only parsers; both run
//! `packages/mail-model/fixtures/label-keys.json`.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use super::SysLabel;

/// Alias kept for readability where the contract says `sysKey`.
pub type SysKey = SysLabel;

/// A split key (`split:<id>` or `split:other`). Virtual: never sent to a provider.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum SplitKey {
    /// `split:<Id<"splits">>`
    Split(String),
    /// `split:other`
    Other,
}

/// A parsed label key.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum LabelKey {
    /// `sys:<role>`
    Sys(SysLabel),
    /// `l:<Id<"labels">>`
    User(String),
    /// `split:*`
    Split(SplitKey),
}

/// Why a string is not a label key.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum LabelKeyError {
    /// The prefix is not `sys:`, `l:` or `split:`.
    #[error("unknown label key prefix")]
    UnknownPrefix,
    /// `sys:` followed by something other than the nine roles.
    #[error("unknown system label")]
    UnknownSys,
    /// The id after `l:` or `split:` is not 8 to 64 lowercase ASCII letters or digits.
    #[error("invalid id in label key")]
    InvalidId,
}

fn valid_id(id: &str) -> bool {
    (8..=64).contains(&id.len()) && id.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
}

/// Parses a label key string.
pub fn parse_label_key(s: &str) -> Result<LabelKey, LabelKeyError> {
    if let Some(rest) = s.strip_prefix("sys:") {
        return SysLabel::from_name(rest).map(LabelKey::Sys).ok_or(LabelKeyError::UnknownSys);
    }
    if let Some(rest) = s.strip_prefix("l:") {
        return if valid_id(rest) { Ok(LabelKey::User(rest.to_owned())) } else { Err(LabelKeyError::InvalidId) };
    }
    if let Some(rest) = s.strip_prefix("split:") {
        if rest == "other" {
            return Ok(LabelKey::Split(SplitKey::Other));
        }
        return if valid_id(rest) {
            Ok(LabelKey::Split(SplitKey::Split(rest.to_owned())))
        } else {
            Err(LabelKeyError::InvalidId)
        };
    }
    Err(LabelKeyError::UnknownPrefix)
}

/// `true` when `s` is a well-formed label key.
pub fn is_label_key(s: &str) -> bool {
    parse_label_key(s).is_ok()
}

impl LabelKey {
    /// `true` for `split:*` keys, which clients never send in actions and providers never see.
    pub fn is_split(&self) -> bool {
        matches!(self, LabelKey::Split(_))
    }

    /// `true` for `sys:*` keys.
    pub fn is_sys(&self) -> bool {
        matches!(self, LabelKey::Sys(_))
    }

    /// Shorthand for a `sys:` key.
    pub fn sys(label: SysLabel) -> Self {
        LabelKey::Sys(label)
    }
}

impl fmt::Display for LabelKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LabelKey::Sys(s) => write!(f, "sys:{}", s.as_str()),
            LabelKey::User(id) => write!(f, "l:{id}"),
            LabelKey::Split(SplitKey::Split(id)) => write!(f, "split:{id}"),
            LabelKey::Split(SplitKey::Other) => f.write_str("split:other"),
        }
    }
}

impl FromStr for LabelKey {
    type Err = LabelKeyError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        parse_label_key(s)
    }
}

impl Serialize for LabelKey {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for LabelKey {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let s = String::deserialize(deserializer)?;
        parse_label_key(&s).map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_every_form() {
        assert_eq!(parse_label_key("sys:inbox"), Ok(LabelKey::Sys(SysLabel::Inbox)));
        assert_eq!(parse_label_key("sys:snoozed"), Ok(LabelKey::Sys(SysLabel::Snoozed)));
        assert_eq!(parse_label_key("l:k57abc123"), Ok(LabelKey::User("k57abc123".into())));
        assert_eq!(parse_label_key("split:other"), Ok(LabelKey::Split(SplitKey::Other)));
        assert_eq!(parse_label_key("split:j97x2k4m"), Ok(LabelKey::Split(SplitKey::Split("j97x2k4m".into()))));
    }

    #[test]
    fn rejects_malformed() {
        for bad in [
            "",
            "inbox",
            "sys:",
            "sys:archive",
            "sys:INBOX",
            "l:",
            "l:a b",
            "l:a:b",
            "l:short",
            "l:ABCDEFGHI",
            "split:",
            "x:y",
            "SYS:inbox",
        ] {
            assert!(!is_label_key(bad), "{bad:?} must be rejected");
        }
    }

    #[test]
    fn display_round_trips() {
        for s in ["sys:unread", "l:abcdefgh", "split:other", "split:zz9zz9zz"] {
            assert_eq!(parse_label_key(s).unwrap().to_string(), s);
        }
    }

    #[test]
    fn serde_round_trip() {
        let k: LabelKey = serde_json::from_str("\"sys:trash\"").unwrap();
        assert_eq!(serde_json::to_string(&k).unwrap(), "\"sys:trash\"");
        assert!(serde_json::from_str::<LabelKey>("\"nope\"").is_err());
    }
}
