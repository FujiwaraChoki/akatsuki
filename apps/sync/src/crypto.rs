//! Worker cryptography (ARCHITECTURE §14.1, §6.7, §10.4).
//!
//! - [`SecretKeyring`]: `accountSecrets` sealing with `TOKEN_ENC_KEYS` (XChaCha20-Poly1305, 24-byte
//!   random nonce, AAD from `akatsuki_core::convex::worker::secret_aad`, D-127).
//! - [`seal`]/[`open`]: the same AEAD for other keys (OAuth state, spool).
//! - [`hkdf32`]: per-file spool keys, `HKDF-SHA256(SPOOL_KEY, info)`.
//! - [`hmac_tag`]/[`hmac_verify`]: capability links and read-receipt tokens.
//!
//! No function here logs, and errors never carry key material or plaintext.

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{XChaCha20Poly1305, XNonce};
use hmac::{Hmac, Mac};
use sha2::Sha256;

use crate::config::Key32;

/// XChaCha20 nonce length.
pub const NONCE_LEN: usize = 24;

/// A crypto failure. Deliberately uninformative.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum CryptoError {
    /// No key is configured.
    #[error("no key configured")]
    NoKey,
    /// The key id is not in `TOKEN_ENC_KEYS`.
    #[error("unknown key id")]
    UnknownKey,
    /// Decryption failed (wrong key, wrong AAD, tampered ciphertext).
    #[error("could not decrypt")]
    Open,
    /// Encryption failed.
    #[error("could not encrypt")]
    Seal,
    /// The OS random source failed.
    #[error("no randomness")]
    Random,
}

/// Fills a buffer from the OS random source.
pub fn random_bytes<const N: usize>() -> Result<[u8; N], CryptoError> {
    let mut b = [0u8; N];
    getrandom::fill(&mut b).map_err(|_| CryptoError::Random)?;
    Ok(b)
}

/// `n` random bytes as base64url without padding.
pub fn random_token<const N: usize>() -> Result<String, CryptoError> {
    Ok(URL_SAFE_NO_PAD.encode(random_bytes::<N>()?))
}

/// Encrypts `plaintext` under `key` with a fresh random nonce. Returns `(nonce, ciphertext)`.
pub fn seal(key: &[u8; 32], aad: &[u8], plaintext: &[u8]) -> Result<([u8; NONCE_LEN], Vec<u8>), CryptoError> {
    let cipher = XChaCha20Poly1305::new_from_slice(key).map_err(|_| CryptoError::Seal)?;
    let nonce = random_bytes::<NONCE_LEN>()?;
    let ct = cipher.encrypt(&XNonce::from(nonce), Payload { msg: plaintext, aad }).map_err(|_| CryptoError::Seal)?;
    Ok((nonce, ct))
}

/// Decrypts what [`seal`] produced.
pub fn open(key: &[u8; 32], aad: &[u8], nonce: &[u8], ciphertext: &[u8]) -> Result<Vec<u8>, CryptoError> {
    let nonce: [u8; NONCE_LEN] = nonce.try_into().map_err(|_| CryptoError::Open)?;
    let cipher = XChaCha20Poly1305::new_from_slice(key).map_err(|_| CryptoError::Open)?;
    cipher.decrypt(&XNonce::from(nonce), Payload { msg: ciphertext, aad }).map_err(|_| CryptoError::Open)
}

/// `base64url(nonce ‖ ciphertext)`: a self-contained sealed token (OAuth state).
pub fn seal_token(key: &[u8; 32], aad: &[u8], plaintext: &[u8]) -> Result<String, CryptoError> {
    let (nonce, ct) = seal(key, aad, plaintext)?;
    let mut out = Vec::with_capacity(NONCE_LEN + ct.len());
    out.extend_from_slice(&nonce);
    out.extend_from_slice(&ct);
    Ok(URL_SAFE_NO_PAD.encode(out))
}

/// Opens a [`seal_token`] token.
pub fn open_token(key: &[u8; 32], aad: &[u8], token: &str) -> Result<Vec<u8>, CryptoError> {
    let raw = URL_SAFE_NO_PAD.decode(token.as_bytes()).map_err(|_| CryptoError::Open)?;
    if raw.len() < NONCE_LEN + 16 {
        return Err(CryptoError::Open);
    }
    let (nonce, ct) = raw.split_at(NONCE_LEN);
    open(key, aad, nonce, ct)
}

/// `HKDF-SHA256(ikm = key, salt = none, info)` → 32 bytes.
pub fn hkdf32(key: &[u8; 32], info: &[u8]) -> [u8; 32] {
    let hk = hkdf::Hkdf::<Sha256>::new(None, key);
    let mut out = [0u8; 32];
    // 32 bytes is far below HKDF-SHA256's 8160-byte output limit.
    let _ = hk.expand(info, &mut out);
    out
}

type HmacSha256 = Hmac<Sha256>;

/// `HMAC-SHA256(key, msg)`.
pub fn hmac_tag(key: &[u8], msg: &[u8]) -> [u8; 32] {
    let mut mac = <HmacSha256 as Mac>::new_from_slice(key).unwrap_or_else(|_| unreachable!("HMAC takes any key"));
    mac.update(msg);
    mac.finalize().into_bytes().into()
}

/// Constant-time check of a (possibly truncated) HMAC tag.
pub fn hmac_verify(key: &[u8], msg: &[u8], tag: &[u8]) -> bool {
    if tag.is_empty() || tag.len() > 32 {
        return false;
    }
    let mut mac = <HmacSha256 as Mac>::new_from_slice(key).unwrap_or_else(|_| unreachable!("HMAC takes any key"));
    mac.update(msg);
    mac.verify_truncated_left(tag).is_ok()
}

/// Constant-time equality for short secrets (hash comparisons).
pub fn ct_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// A sealed `accountSecrets` value.
#[derive(Clone, PartialEq, Eq)]
pub struct Sealed {
    /// Which `TOKEN_ENC_KEYS` entry sealed it.
    pub key_id: String,
    /// 24-byte nonce.
    pub nonce: Vec<u8>,
    /// Ciphertext with tag.
    pub ciphertext: Vec<u8>,
}

impl std::fmt::Debug for Sealed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Sealed").field("key_id", &self.key_id).finish_non_exhaustive()
    }
}

/// `TOKEN_ENC_KEYS`: the first key seals, every key opens.
#[derive(Clone)]
pub struct SecretKeyring {
    keys: Vec<(String, Key32)>,
}

impl std::fmt::Debug for SecretKeyring {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SecretKeyring").field("keys", &self.keys.len()).finish_non_exhaustive()
    }
}

impl SecretKeyring {
    /// A keyring from parsed `TOKEN_ENC_KEYS`.
    pub fn new(keys: Vec<(String, Key32)>) -> Self {
        Self { keys }
    }

    /// The active key id.
    pub fn active_id(&self) -> Option<&str> {
        self.keys.first().map(|(id, _)| id.as_str())
    }

    /// Seals with the active key.
    pub fn seal(&self, aad: &[u8], plaintext: &[u8]) -> Result<Sealed, CryptoError> {
        let (id, key) = self.keys.first().ok_or(CryptoError::NoKey)?;
        let (nonce, ciphertext) = seal(&key.0, aad, plaintext)?;
        Ok(Sealed { key_id: id.clone(), nonce: nonce.to_vec(), ciphertext })
    }

    /// Opens with the key the value names.
    pub fn open(&self, key_id: &str, nonce: &[u8], ciphertext: &[u8], aad: &[u8]) -> Result<Vec<u8>, CryptoError> {
        let (_, key) = self.keys.iter().find(|(id, _)| id == key_id).ok_or(CryptoError::UnknownKey)?;
        open(&key.0, aad, nonce, ciphertext)
    }

    /// Whether a value sealed under `key_id` should be re-sealed (it is not the active key).
    pub fn needs_rotation(&self, key_id: &str) -> bool {
        self.active_id().is_some_and(|a| a != key_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use akatsuki_core::convex::worker::secret_aad;
    use akatsuki_core::convex::{Provider, SecretKind};

    fn key(b: u8) -> Key32 {
        Key32([b; 32])
    }

    #[test]
    fn keyring_seals_with_active_and_opens_with_any() {
        let old = SecretKeyring::new(vec![("k1".into(), key(1))]);
        let aad = secret_aad("u1", Provider::Gmail, "ada@example.com", SecretKind::Oauth);
        let sealed = old.seal(&aad, b"{\"refreshToken\":\"r\"}").unwrap();
        assert_eq!(sealed.key_id, "k1");
        assert_eq!(sealed.nonce.len(), NONCE_LEN);
        assert!(!format!("{sealed:?}").contains("refresh"));

        let rotated = SecretKeyring::new(vec![("k2".into(), key(2)), ("k1".into(), key(1))]);
        assert!(rotated.needs_rotation(&sealed.key_id));
        let pt = rotated.open(&sealed.key_id, &sealed.nonce, &sealed.ciphertext, &aad).unwrap();
        assert_eq!(pt, b"{\"refreshToken\":\"r\"}");
        let resealed = rotated.seal(&aad, &pt).unwrap();
        assert_eq!(resealed.key_id, "k2");
        assert!(!rotated.needs_rotation("k2"));

        // Removing the old key makes old ciphertext unreadable.
        let new_only = SecretKeyring::new(vec![("k2".into(), key(2))]);
        assert_eq!(new_only.open("k1", &sealed.nonce, &sealed.ciphertext, &aad), Err(CryptoError::UnknownKey));
    }

    #[test]
    fn aad_binds_the_ciphertext_to_its_mailbox() {
        let ring = SecretKeyring::new(vec![("k1".into(), key(7))]);
        let aad = secret_aad("u1", Provider::Graph, "obj-1", SecretKind::Oauth);
        let s = ring.seal(&aad, b"secret").unwrap();
        for other in [
            secret_aad("u2", Provider::Graph, "obj-1", SecretKind::Oauth),
            secret_aad("u1", Provider::Graph, "obj-2", SecretKind::Oauth),
            secret_aad("u1", Provider::Gmail, "obj-1", SecretKind::Oauth),
            secret_aad("u1", Provider::Graph, "obj-1", SecretKind::Imap),
        ] {
            assert_eq!(ring.open("k1", &s.nonce, &s.ciphertext, &other), Err(CryptoError::Open));
        }
        let mut tampered = s.ciphertext.clone();
        tampered[0] ^= 1;
        assert_eq!(ring.open("k1", &s.nonce, &tampered, &aad), Err(CryptoError::Open));
        assert!(SecretKeyring::new(vec![]).seal(&aad, b"x").is_err());
    }

    #[test]
    fn nonces_are_fresh() {
        let k = [3u8; 32];
        let (n1, c1) = seal(&k, b"a", b"same").unwrap();
        let (n2, c2) = seal(&k, b"a", b"same").unwrap();
        assert_ne!(n1, n2);
        assert_ne!(c1, c2);
    }

    #[test]
    fn tokens_round_trip_and_reject_tampering() {
        let k = [9u8; 32];
        let t = seal_token(&k, b"state", b"payload").unwrap();
        assert_eq!(open_token(&k, b"state", &t).unwrap(), b"payload");
        assert!(open_token(&k, b"other", &t).is_err());
        assert!(open_token(&[8u8; 32], b"state", &t).is_err());
        assert!(open_token(&k, b"state", "short").is_err());
        assert!(open_token(&k, b"state", "!!!").is_err());
    }

    #[test]
    fn hkdf_and_hmac() {
        let k = [1u8; 32];
        assert_ne!(hkdf32(&k, b"a"), hkdf32(&k, b"b"));
        assert_eq!(hkdf32(&k, b"a"), hkdf32(&k, b"a"));
        let tag = hmac_tag(&k, b"msg");
        assert!(hmac_verify(&k, b"msg", &tag[..8]));
        assert!(!hmac_verify(&k, b"msg2", &tag[..8]));
        assert!(!hmac_verify(&k, b"msg", &[]));
        assert!(ct_eq(b"abc", b"abc") && !ct_eq(b"abc", b"abd") && !ct_eq(b"a", b"ab"));
    }
}
