//! Small IMAP syntax helpers: argument quoting, literals, UID sets, dates.

use jiff::Timestamp;
use jiff::civil::Date;
use jiff::tz::{Offset, TimeZone};

use super::utf7;
use crate::ProviderError;

/// One piece of a command line: text, or a literal that must be sent after a continuation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Part {
    /// Raw command text (already quoted where needed).
    Text(String),
    /// A literal (`{n}` / `{n+}`).
    Literal(Vec<u8>),
}

/// A command under construction.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Cmd {
    /// Pieces in order.
    pub parts: Vec<Part>,
}

impl Cmd {
    /// Starts a command with the verb (and anything else that needs no quoting).
    pub fn new(verb: impl Into<String>) -> Self {
        Self { parts: vec![Part::Text(verb.into())] }
    }

    /// Appends raw text, separated by a space.
    pub fn raw(mut self, s: impl AsRef<str>) -> Self {
        self.push_text(" ");
        self.push_text(s.as_ref());
        self
    }

    /// Appends a string argument: quoted when it is 7-bit clean, a literal otherwise.
    pub fn string(mut self, s: &str) -> Result<Self, ProviderError> {
        self.push_text(" ");
        match quote(s) {
            Some(q) => self.push_text(&q),
            None => {
                if s.contains('\0') {
                    return Err(ProviderError::Invalid("NUL in IMAP string".into()));
                }
                self.parts.push(Part::Literal(s.as_bytes().to_vec()));
            }
        }
        Ok(self)
    }

    /// Appends a mailbox name (UTF-8), encoded as modified UTF-7 and quoted.
    pub fn mailbox(mut self, name: &str) -> Result<Self, ProviderError> {
        let enc = utf7::encode(name);
        let q = quote(&enc).ok_or_else(|| ProviderError::Invalid("mailbox name cannot be quoted".into()))?;
        self.push_text(" ");
        self.push_text(&q);
        Ok(self)
    }

    /// Appends a literal argument.
    pub fn literal(mut self, bytes: Vec<u8>) -> Self {
        self.push_text(" ");
        self.parts.push(Part::Literal(bytes));
        self
    }

    fn push_text(&mut self, s: &str) {
        if let Some(Part::Text(t)) = self.parts.last_mut() {
            t.push_str(s);
        } else {
            self.parts.push(Part::Text(s.to_owned()));
        }
    }

    /// The command for logs: the verb plus the number of literals. Never arguments (they can hold
    /// passwords, addresses or search terms).
    pub fn verb(&self) -> String {
        let Some(Part::Text(t)) = self.parts.first() else { return String::new() };
        let mut words = t.split(' ');
        let first = words.next().unwrap_or_default().to_ascii_uppercase();
        match words.next() {
            Some(second) if first == "UID" => format!("UID {}", second.to_ascii_uppercase()),
            _ => first,
        }
    }
}

/// Quotes a string if it is 7-bit and has no CR, LF or NUL.
pub fn quote(s: &str) -> Option<String> {
    if !s.is_ascii() || s.bytes().any(|b| b == b'\r' || b == b'\n' || b == 0) {
        return None;
    }
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        if c == '"' || c == '\\' {
            out.push('\\');
        }
        out.push(c);
    }
    out.push('"');
    Some(out)
}

/// Compresses UIDs into an IMAP sequence set (`1:3,7,9:10`). Input need not be sorted.
pub fn uid_set(uids: &[u32]) -> String {
    let mut v: Vec<u32> = uids.to_vec();
    v.sort_unstable();
    v.dedup();
    let mut out = String::new();
    let mut i = 0;
    while i < v.len() {
        let start = v[i];
        let mut end = start;
        while i + 1 < v.len() && v[i + 1] == end + 1 {
            i += 1;
            end = v[i];
        }
        if !out.is_empty() {
            out.push(',');
        }
        if start == end {
            out.push_str(&start.to_string());
        } else {
            out.push_str(&format!("{start}:{end}"));
        }
        i += 1;
    }
    out
}

/// Expands RFC 4315 `uid-set` members.
pub fn expand_uid_set(members: &[imap_proto::rfc4315::UidSetMember]) -> Vec<u32> {
    let mut out = Vec::new();
    for m in members {
        match m {
            imap_proto::rfc4315::UidSetMember::Uid(u) => out.push(*u),
            imap_proto::rfc4315::UidSetMember::UidRange(r) => {
                let (a, b) = (*r.start(), *r.end());
                let (lo, hi) = if a <= b { (a, b) } else { (b, a) };
                // Ranges in COPYUID/APPENDUID pair up element by element; keep them bounded.
                if hi - lo > 1_000_000 {
                    continue;
                }
                if a <= b {
                    out.extend(lo..=hi);
                } else {
                    out.extend((lo..=hi).rev());
                }
            }
        }
    }
    out
}

const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];

/// An IMAP `date` (`SEARCH SINCE/BEFORE`), in UTC: `1-Jan-2026`.
pub fn search_date(ms: i64) -> String {
    let date = Timestamp::from_millisecond(ms).unwrap_or(Timestamp::UNIX_EPOCH).to_zoned(TimeZone::UTC).date();
    let month = MONTHS[usize::try_from(date.month() - 1).unwrap_or(0)];
    format!("{}-{}-{}", date.day(), month, date.year())
}

/// Parses `INTERNALDATE` (`"17-Jul-1996 02:44:25 -0700"`, the day may be space-padded) to ms.
pub fn parse_internal_date(s: &str) -> Option<i64> {
    let mut it = s.split_whitespace();
    let date = it.next()?;
    let time = it.next()?;
    let zone = it.next().unwrap_or("+0000");
    let mut d = date.split('-');
    let day: i8 = d.next()?.parse().ok()?;
    let mon_s = d.next()?;
    let month = i8::try_from(MONTHS.iter().position(|m| m.eq_ignore_ascii_case(mon_s))? + 1).ok()?;
    let year: i16 = d.next()?.parse().ok()?;
    let mut t = time.split(':');
    let h: i8 = t.next()?.parse().ok()?;
    let mi: i8 = t.next()?.parse().ok()?;
    let sec: i8 = t.next()?.parse().ok()?;
    let (sign, digits) = match zone.as_bytes().first()? {
        b'+' => (1, &zone[1..]),
        b'-' => (-1, &zone[1..]),
        _ => (1, zone),
    };
    if digits.len() != 4 {
        return None;
    }
    let zh: i32 = digits[..2].parse().ok()?;
    let zm: i32 = digits[2..].parse().ok()?;
    let offset = Offset::from_seconds(sign * (zh * 3600 + zm * 60)).ok()?;
    let dt = Date::new(year, month, day).ok()?.at(h, mi, sec, 0);
    let ts = dt.to_zoned(TimeZone::fixed(offset)).ok()?.timestamp();
    Some(ts.as_millisecond())
}

/// Formats ms as an `INTERNALDATE` / `APPEND` date-time (UTC).
pub fn format_internal_date(ms: i64) -> String {
    let z = Timestamp::from_millisecond(ms).unwrap_or(Timestamp::UNIX_EPOCH).to_zoned(TimeZone::UTC);
    let month = MONTHS[usize::try_from(z.month() - 1).unwrap_or(0)];
    format!("{:02}-{}-{} {:02}:{:02}:{:02} +0000", z.day(), month, z.year(), z.hour(), z.minute(), z.second())
}

/// `true` for an IMAP section path made only of part numbers (`1`, `1.2.3`).
pub fn is_part_path(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 64
        && s.split('.').all(|p| !p.is_empty() && p.len() <= 4 && p.bytes().all(|b| b.is_ascii_digit()) && p != "0")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sets_and_quotes() {
        assert_eq!(uid_set(&[9, 1, 2, 3, 7, 10, 3]), "1:3,7,9:10");
        assert_eq!(uid_set(&[]), "");
        assert_eq!(quote(r#"a"b\c"#).as_deref(), Some(r#""a\"b\\c""#));
        assert_eq!(quote("é"), None);
        assert_eq!(quote("a\r\nb"), None);
        let c = Cmd::new("LOGIN").string("u").unwrap().string("pässword").unwrap();
        assert_eq!(c.parts.len(), 2);
        assert_eq!(c.verb(), "LOGIN");
        assert_eq!(Cmd::new("UID fetch").raw("1:*").verb(), "UID FETCH");
        assert!(matches!(&c.parts[1], Part::Literal(b) if b == "pässword".as_bytes()));
        let m = Cmd::new("SELECT").mailbox("Entwürfe").unwrap();
        assert_eq!(m.parts, vec![Part::Text("SELECT \"Entw&APw-rfe\"".into())]);
    }

    #[test]
    fn uid_set_members() {
        use imap_proto::rfc4315::UidSetMember as M;
        assert_eq!(expand_uid_set(&[M::Uid(4), M::UidRange(7..=9)]), vec![4, 7, 8, 9]);
    }

    #[test]
    fn dates() {
        // 2026-10-20T12:00:00Z
        let ms = 1_792_497_600_000;
        assert_eq!(search_date(ms), "20-Oct-2026");
        assert_eq!(parse_internal_date("20-Oct-2026 14:00:00 +0200"), Some(ms));
        assert_eq!(parse_internal_date(" 1-Jan-2026 00:00:00 +0000"), Some(1_767_225_600_000));
        assert_eq!(parse_internal_date("garbage"), None);
        assert_eq!(format_internal_date(ms), "20-Oct-2026 12:00:00 +0000");
        assert_eq!(parse_internal_date(&format_internal_date(ms)), Some(ms));
    }

    #[test]
    fn part_paths() {
        assert!(is_part_path("1"));
        assert!(is_part_path("1.2.10"));
        assert!(!is_part_path(""));
        assert!(!is_part_path("1..2"));
        assert!(!is_part_path("HEADER"));
        assert!(!is_part_path("1]<0.10> X"));
        assert!(!is_part_path("0"));
    }
}
