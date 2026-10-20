//! Thread ids for IMAP servers without `X-GM-THRID` (ARCHITECTURE §9.5, D-051).
//!
//! `threadId = base32(sha256(accountId ‖ root))[..26]` where `root` is the first id in
//! `References`, else `In-Reply-To`, else the message's own `Message-ID`, else
//! `sha256(from|date|subject)`. Subjects are never merged.

use sha2::{Digest, Sha256};

const ALPHABET: &[u8; 32] = b"abcdefghijklmnopqrstuvwxyz234567";

/// RFC 4648 base32, lowercase, no padding.
pub fn base32(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len().div_ceil(5) * 8);
    let mut buffer: u32 = 0;
    let mut bits = 0;
    for &b in bytes {
        buffer = (buffer << 8) | u32::from(b);
        bits += 8;
        while bits >= 5 {
            bits -= 5;
            out.push(ALPHABET[((buffer >> bits) & 31) as usize] as char);
        }
    }
    if bits > 0 {
        out.push(ALPHABET[((buffer << (5 - bits)) & 31) as usize] as char);
    }
    out
}

/// Strips angle brackets and whitespace from a message id.
pub fn clean_message_id(id: &str) -> String {
    id.trim().trim_start_matches('<').trim_end_matches('>').trim().to_owned()
}

/// The threading root of a message.
pub fn thread_root(
    references: &[String],
    in_reply_to: Option<&str>,
    message_id: Option<&str>,
    from: &str,
    date_ms: i64,
    subject: &str,
) -> String {
    let pick = |s: &str| {
        let c = clean_message_id(s);
        (!c.is_empty()).then_some(c)
    };
    references
        .iter()
        .find_map(|r| pick(r))
        .or_else(|| in_reply_to.and_then(pick))
        .or_else(|| message_id.and_then(pick))
        .unwrap_or_else(|| {
            let digest = Sha256::digest(format!("{from}|{date_ms}|{subject}").as_bytes());
            hex(&digest)
        })
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// `base32(sha256(accountId ‖ root))[..26]`.
pub fn thread_id(account_id: &str, root: &str) -> String {
    let mut h = Sha256::new();
    h.update(account_id.as_bytes());
    h.update(root.as_bytes());
    let mut id = base32(&h.finalize());
    id.truncate(26);
    id
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base32_vectors() {
        // RFC 4648 test vectors, lowercased, padding removed.
        assert_eq!(base32(b""), "");
        assert_eq!(base32(b"f"), "my");
        assert_eq!(base32(b"fo"), "mzxq");
        assert_eq!(base32(b"foo"), "mzxw6");
        assert_eq!(base32(b"foobar"), "mzxw6ytboi");
    }

    #[test]
    fn root_precedence() {
        let refs = vec!["<root@x>".to_owned(), "<mid@x>".to_owned()];
        assert_eq!(thread_root(&refs, Some("<mid@x>"), Some("<me@x>"), "", 0, ""), "root@x");
        assert_eq!(thread_root(&[], Some("<mid@x>"), Some("<me@x>"), "", 0, ""), "mid@x");
        assert_eq!(thread_root(&[], None, Some("<me@x>"), "", 0, ""), "me@x");
        let fallback = thread_root(&[], None, None, "a@b", 5, "Hi");
        assert_eq!(fallback.len(), 64);
    }

    #[test]
    fn ids_are_stable_and_scoped_by_account() {
        let a = thread_id("acc1", "root@x");
        assert_eq!(a.len(), 26);
        assert_eq!(a, thread_id("acc1", "root@x"));
        assert_ne!(a, thread_id("acc2", "root@x"));
        // A reply and its parent share the root, so they share the thread.
        let parent = thread_root(&[], None, Some("<p@x>"), "", 0, "");
        let reply = thread_root(&["<p@x>".into()], Some("<p@x>"), Some("<r@x>"), "", 0, "");
        assert_eq!(thread_id("acc1", &parent), thread_id("acc1", &reply));
    }
}
