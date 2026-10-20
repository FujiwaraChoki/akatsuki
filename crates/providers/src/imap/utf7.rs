//! IMAP modified UTF-7 for mailbox names (RFC 3501 §5.1.3).
//!
//! `remoteFolderId` is the decoded UTF-8 name (§3.2 `folderCursors`); commands carry the encoded
//! form.

use base64::Engine as _;
use base64::alphabet::Alphabet;
use base64::engine::{DecodePaddingMode, GeneralPurpose, GeneralPurposeConfig};

fn engine() -> GeneralPurpose {
    // Modified base64: "," instead of "/", no padding.
    let alphabet =
        Alphabet::new("ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+,").expect("valid alphabet");
    GeneralPurpose::new(
        &alphabet,
        GeneralPurposeConfig::new().with_encode_padding(false).with_decode_padding_mode(DecodePaddingMode::RequireNone),
    )
}

/// UTF-8 → modified UTF-7.
pub fn encode(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    let mut pending: Vec<u16> = Vec::new();
    let flush = |pending: &mut Vec<u16>, out: &mut String| {
        if pending.is_empty() {
            return;
        }
        let bytes: Vec<u8> = pending.iter().flat_map(|u| u.to_be_bytes()).collect();
        out.push('&');
        out.push_str(&engine().encode(bytes));
        out.push('-');
        pending.clear();
    };
    for c in name.chars() {
        if (' '..='~').contains(&c) {
            flush(&mut pending, &mut out);
            if c == '&' {
                out.push_str("&-");
            } else {
                out.push(c);
            }
        } else {
            let mut buf = [0u16; 2];
            pending.extend_from_slice(c.encode_utf16(&mut buf));
        }
    }
    flush(&mut pending, &mut out);
    out
}

/// Modified UTF-7 → UTF-8. Invalid input is returned unchanged (servers that already speak UTF-8
/// names, or broken names, still get a stable id).
pub fn decode(name: &str) -> String {
    try_decode(name).unwrap_or_else(|| name.to_owned())
}

fn try_decode(name: &str) -> Option<String> {
    let mut out = String::with_capacity(name.len());
    let mut rest = name;
    while let Some(i) = rest.find('&') {
        out.push_str(&rest[..i]);
        let after = &rest[i + 1..];
        let end = after.find('-')?;
        let chunk = &after[..end];
        if chunk.is_empty() {
            out.push('&');
        } else {
            let bytes = engine().decode(chunk).ok()?;
            if bytes.len() % 2 != 0 {
                return None;
            }
            let units: Vec<u16> = bytes.as_chunks::<2>().0.iter().map(|b| u16::from_be_bytes(*b)).collect();
            out.push_str(&String::from_utf16(&units).ok()?);
        }
        rest = &after[end + 1..];
    }
    out.push_str(rest);
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rfc_examples_round_trip() {
        // RFC 3501 §5.1.3 example: "~peter/mail/台北/日本語".
        let utf8 = "~peter/mail/\u{53f0}\u{5317}/\u{65e5}\u{672c}\u{8a9e}";
        let enc = "~peter/mail/&U,BTFw-/&ZeVnLIqe-";
        assert_eq!(encode(utf8), enc);
        assert_eq!(decode(enc), utf8);
        assert_eq!(encode("Tom & Jerry"), "Tom &- Jerry");
        assert_eq!(decode("Tom &- Jerry"), "Tom & Jerry");
        assert_eq!(encode("Entwürfe"), "Entw&APw-rfe");
        assert_eq!(decode("Entw&APw-rfe"), "Entwürfe");
        assert_eq!(decode("INBOX"), "INBOX");
        // Emoji (surrogate pair) round trips.
        assert_eq!(decode(&encode("Mail \u{1f525}")), "Mail \u{1f525}");
        // Broken input stays as is.
        assert_eq!(decode("bad&xx"), "bad&xx");
    }
}
