//! Capability tokens: attachment links (`/v1/l/{token}`, `LINK_SIGNING_KEY`, 5 minutes) and
//! read-receipt pixels (`/r/{token}.gif`, `READ_RECEIPT_KEY`).

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde::{Deserialize, Serialize};

use crate::crypto::{hmac_tag, hmac_verify};
use akatsuki_core::time::{MINUTE_MS, UnixMs};

/// Attachment link lifetime.
pub const LINK_TTL_MS: i64 = 5 * MINUTE_MS;

/// What an attachment link grants.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LinkClaims {
    /// Owner.
    pub u: String,
    /// Convex message id.
    pub m: String,
    /// Part id.
    pub p: String,
    /// Expiry (ms).
    pub exp: UnixMs,
}

/// `base64url(json) "." base64url(HMAC-SHA256(key, "link1|" + json))`.
pub fn sign_link(key: &[u8], claims: &LinkClaims) -> String {
    let json = serde_json::to_vec(claims).unwrap_or_default();
    let mut msg = b"link1|".to_vec();
    msg.extend_from_slice(&json);
    format!("{}.{}", URL_SAFE_NO_PAD.encode(&json), URL_SAFE_NO_PAD.encode(hmac_tag(key, &msg)))
}

/// Verifies a link token at `now`.
pub fn verify_link(key: &[u8], token: &str, now: UnixMs) -> Option<LinkClaims> {
    if token.len() > 2048 {
        return None;
    }
    let (body, sig) = token.split_once('.')?;
    let json = URL_SAFE_NO_PAD.decode(body).ok()?;
    let sig = URL_SAFE_NO_PAD.decode(sig).ok()?;
    let mut msg = b"link1|".to_vec();
    msg.extend_from_slice(&json);
    if sig.len() != 32 || !hmac_verify(key, &msg, &sig) {
        return None;
    }
    let claims: LinkClaims = serde_json::from_slice(&json).ok()?;
    (claims.exp >= now).then_some(claims)
}

/// The read-receipt token of one outgoing message: 16 bytes (22 base64url characters) made of a
/// 10-byte id derived from `(userId, rfcMessageId)` and a 6-byte tag over it. It is stable across
/// autosaves of the same draft, and the pixel route can reject forged tokens without a lookup.
pub fn receipt_token(key: &[u8], user_id: &str, rfc_message_id: &str) -> String {
    let id_full = hmac_tag(key, format!("rr-id|{user_id}|{rfc_message_id}").as_bytes());
    let id = &id_full[..10];
    let tag = receipt_tag(key, id);
    let mut raw = id.to_vec();
    raw.extend_from_slice(&tag[..6]);
    URL_SAFE_NO_PAD.encode(raw)
}

fn receipt_tag(key: &[u8], id: &[u8]) -> [u8; 32] {
    let mut msg = b"rr-tag|".to_vec();
    msg.extend_from_slice(id);
    hmac_tag(key, &msg)
}

/// Whether a pixel token was minted with `key`.
pub fn verify_receipt(key: &[u8], token: &str) -> bool {
    if token.len() != 22 || !token.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_') {
        return false;
    }
    let Ok(raw) = URL_SAFE_NO_PAD.decode(token) else { return false };
    if raw.len() != 16 {
        return false;
    }
    let mut msg = b"rr-tag|".to_vec();
    msg.extend_from_slice(&raw[..10]);
    hmac_verify(key, &msg, &raw[10..])
}

/// `{base}/r/{token}.gif`.
pub fn receipt_url(public_base_url: &str, token: &str) -> String {
    format!("{}/r/{token}.gif", public_base_url.trim_end_matches('/'))
}

/// Opens through mail-provider image proxies (Gmail, Yahoo; Apple Mail Privacy Protection
/// prefetches) are flagged `proxied`: they say little about a human reading the message.
pub fn opened_through_proxy(user_agent: Option<&str>) -> bool {
    let ua = user_agent.unwrap_or_default();
    ["GoogleImageProxy", "ggpht.com", "YahooMailProxy", "Yahoo! Slurp", "BingPreview", "Outlook-iOS", "OutlookProxy"]
        .iter()
        .any(|p| ua.contains(p))
        || ua.is_empty()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn links_expire_and_resist_tampering() {
        let key = [1u8; 32];
        let c = LinkClaims { u: "u1".into(), m: "msg00001".into(), p: "2".into(), exp: 1_000 };
        let t = sign_link(&key, &c);
        assert_eq!(verify_link(&key, &t, 999), Some(c.clone()));
        assert_eq!(verify_link(&key, &t, 1_001), None);
        assert_eq!(verify_link(&[2u8; 32], &t, 0), None);
        let forged = sign_link(&[2u8; 32], &LinkClaims { u: "u2".into(), ..c });
        let (body, _) = forged.split_once('.').unwrap();
        let (_, sig) = t.split_once('.').unwrap();
        assert_eq!(verify_link(&key, &format!("{body}.{sig}"), 0), None);
        assert_eq!(verify_link(&key, "garbage", 0), None);
    }

    #[test]
    fn receipt_tokens_are_stable_and_verifiable() {
        let key = [3u8; 32];
        let a = receipt_token(&key, "u1", "id1@x.com");
        assert_eq!(a.len(), 22);
        assert_eq!(a, receipt_token(&key, "u1", "id1@x.com"));
        assert_ne!(a, receipt_token(&key, "u2", "id1@x.com"));
        assert!(verify_receipt(&key, &a));
        assert!(!verify_receipt(&[4u8; 32], &a));
        assert!(!verify_receipt(&key, "AAAAAAAAAAAAAAAAAAAAAA"));
        assert!(!verify_receipt(&key, "short"));
        assert_eq!(receipt_url("https://sync.akatsuki.vc/", &a), format!("https://sync.akatsuki.vc/r/{a}.gif"));
        assert!(opened_through_proxy(Some("Mozilla/5.0 (via ggpht.com GoogleImageProxy)")));
        assert!(!opened_through_proxy(Some("Mozilla/5.0 (X11; Linux x86_64) Firefox/140.0")));
    }
}
