//! The encrypted upload spool (ARCHITECTURE §10.4).
//!
//! `POST /v1/uploads` writes each file to the spool directory (`/var/lib/akatsuki/spool`, a Docker
//! volume), encrypted with XChaCha20-Poly1305 under a per-file key `HKDF(SPOOL_KEY, uploadId)`.
//! The file name, content type, owner and digest are encrypted inside the same envelope, so the
//! directory reveals only random ids and sizes. A file is deleted after 24 hours (not when first
//! attached: later saves of the same draft attach it again, D-416). This is the only place the
//! worker writes message content to disk.
//!
//! File layout: `AKS1` ‖ nonce (24) ‖ ciphertext. Plaintext: `u32 BE meta length` ‖ meta JSON ‖
//! content. AAD: `akatsuki:spool:v1:<uploadId>`.

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::crypto::{self, CryptoError};
use akatsuki_core::time::now_ms as time_now_ms;

/// How long an upload lives.
pub const SPOOL_TTL: Duration = Duration::from_secs(24 * 60 * 60);
/// Largest upload.
pub const MAX_UPLOAD_BYTES: usize = 25 * 1024 * 1024;
/// Default spool directory.
pub const DEFAULT_SPOOL_DIR: &str = "/var/lib/akatsuki/spool";

const MAGIC: &[u8; 4] = b"AKS1";

/// Spool failures.
#[derive(Debug, thiserror::Error)]
pub enum SpoolError {
    /// Unknown, expired, or owned by someone else (indistinguishable on purpose).
    #[error("upload not found")]
    NotFound,
    /// Too big.
    #[error("upload too large")]
    TooLarge,
    /// The filesystem failed.
    #[error("spool io: {0}")]
    Io(String),
    /// Crypto failed.
    #[error("spool crypto: {0}")]
    Crypto(#[from] CryptoError),
}

/// Metadata stored (encrypted) with each upload.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UploadMeta {
    /// Owner (JWT `sub`).
    pub user_id: String,
    /// Decoded file name.
    pub filename: String,
    /// `type/subtype`.
    pub content_type: String,
    /// Size in bytes.
    pub size: u64,
    /// Hex sha256 of the content.
    pub sha256: String,
    /// Created (ms).
    pub created_at: i64,
}

/// An upload read back from the spool.
#[derive(Clone, PartialEq, Eq)]
pub struct Upload {
    /// Metadata.
    pub meta: UploadMeta,
    /// Content.
    pub content: Vec<u8>,
}

impl std::fmt::Debug for Upload {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Upload").field("size", &self.meta.size).finish_non_exhaustive()
    }
}

/// The spool.
#[derive(Clone)]
pub struct Spool {
    dir: PathBuf,
    key: [u8; 32],
}

impl std::fmt::Debug for Spool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Spool").field("dir", &self.dir).finish_non_exhaustive()
    }
}

/// Upload ids are 32 lowercase hex characters (16 random bytes); anything else is rejected
/// before it touches the filesystem, so ids cannot traverse paths.
pub fn valid_upload_id(id: &str) -> bool {
    id.len() == 32 && id.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

fn aad(upload_id: &str) -> Vec<u8> {
    format!("akatsuki:spool:v1:{upload_id}").into_bytes()
}

impl Spool {
    /// A spool in `dir` keyed by `SPOOL_KEY`.
    pub fn new(dir: impl Into<PathBuf>, spool_key: [u8; 32]) -> Self {
        Self { dir: dir.into(), key: spool_key }
    }

    fn file_key(&self, upload_id: &str) -> [u8; 32] {
        crypto::hkdf32(&self.key, &aad(upload_id))
    }

    fn path(&self, upload_id: &str) -> PathBuf {
        self.dir.join(format!("{upload_id}.bin"))
    }

    /// Encrypts and stores an upload; returns its id and metadata.
    pub async fn put(
        &self,
        user_id: &str,
        filename: &str,
        content_type: &str,
        content: &[u8],
    ) -> Result<(String, UploadMeta), SpoolError> {
        if content.len() > MAX_UPLOAD_BYTES {
            return Err(SpoolError::TooLarge);
        }
        let upload_id = hex::encode(crypto::random_bytes::<16>()?);
        let meta = UploadMeta {
            user_id: user_id.to_owned(),
            filename: filename.to_owned(),
            content_type: content_type.to_owned(),
            size: content.len() as u64,
            sha256: hex::encode(Sha256::digest(content)),
            created_at: time_now_ms(),
        };
        let meta_json = serde_json::to_vec(&meta).map_err(|e| SpoolError::Io(e.to_string()))?;
        let meta_len = u32::try_from(meta_json.len()).map_err(|_| SpoolError::TooLarge)?;
        let mut plain = Vec::with_capacity(4 + meta_json.len() + content.len());
        plain.extend_from_slice(&meta_len.to_be_bytes());
        plain.extend_from_slice(&meta_json);
        plain.extend_from_slice(content);
        let (nonce, ct) = crypto::seal(&self.file_key(&upload_id), &aad(&upload_id), &plain)?;
        let mut file = Vec::with_capacity(4 + nonce.len() + ct.len());
        file.extend_from_slice(MAGIC);
        file.extend_from_slice(&nonce);
        file.extend_from_slice(&ct);
        tokio::fs::create_dir_all(&self.dir).await.map_err(|e| SpoolError::Io(e.kind().to_string()))?;
        let tmp = self.dir.join(format!("{upload_id}.tmp"));
        tokio::fs::write(&tmp, &file).await.map_err(|e| SpoolError::Io(e.kind().to_string()))?;
        tokio::fs::rename(&tmp, self.path(&upload_id)).await.map_err(|e| SpoolError::Io(e.kind().to_string()))?;
        Ok((upload_id, meta))
    }

    /// Reads an upload the user owns. Expired files are deleted and reported as missing.
    pub async fn get(&self, user_id: &str, upload_id: &str) -> Result<Upload, SpoolError> {
        if !valid_upload_id(upload_id) {
            return Err(SpoolError::NotFound);
        }
        let path = self.path(upload_id);
        let file = match tokio::fs::read(&path).await {
            Ok(f) => f,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Err(SpoolError::NotFound),
            Err(e) => return Err(SpoolError::Io(e.kind().to_string())),
        };
        if file.len() < 4 + crypto::NONCE_LEN || &file[..4] != MAGIC {
            return Err(SpoolError::NotFound);
        }
        let (nonce, ct) = file[4..].split_at(crypto::NONCE_LEN);
        let plain = crypto::open(&self.file_key(upload_id), &aad(upload_id), nonce, ct)?;
        let meta_len =
            u32::from_be_bytes(plain.get(..4).and_then(|b| b.try_into().ok()).ok_or(SpoolError::NotFound)?) as usize;
        let meta: UploadMeta = serde_json::from_slice(plain.get(4..4 + meta_len).ok_or(SpoolError::NotFound)?)
            .map_err(|_| SpoolError::NotFound)?;
        if meta.user_id != user_id {
            return Err(SpoolError::NotFound);
        }
        let age_ms = time_now_ms().saturating_sub(meta.created_at);
        if age_ms > i64::try_from(SPOOL_TTL.as_millis()).unwrap_or(i64::MAX) {
            let _ = tokio::fs::remove_file(&path).await;
            return Err(SpoolError::NotFound);
        }
        let content = plain[4 + meta_len..].to_vec();
        Ok(Upload { meta, content })
    }

    /// Deletes an upload.
    pub async fn remove(&self, upload_id: &str) {
        if valid_upload_id(upload_id) {
            let _ = tokio::fs::remove_file(self.path(upload_id)).await;
        }
    }

    /// Bytes currently in the spool directory (the global cap of §13.4, D-397).
    pub async fn total_bytes(&self) -> u64 {
        let Ok(mut entries) = tokio::fs::read_dir(&self.dir).await else { return 0 };
        let mut total = 0u64;
        while let Ok(Some(e)) = entries.next_entry().await {
            if let Ok(m) = e.metadata().await
                && m.is_file()
            {
                total = total.saturating_add(m.len());
            }
        }
        total
    }

    /// Deletes every spool file older than the TTL (by modification time, so nothing is
    /// decrypted). Returns how many were removed.
    pub async fn sweep(&self) -> usize {
        sweep_dir(&self.dir, SPOOL_TTL).await
    }
}

async fn sweep_dir(dir: &Path, ttl: Duration) -> usize {
    let Ok(mut entries) = tokio::fs::read_dir(dir).await else { return 0 };
    let mut removed = 0;
    let now = SystemTime::now();
    while let Ok(Some(entry)) = entries.next_entry().await {
        let Ok(meta) = entry.metadata().await else { continue };
        let old = meta.modified().ok().and_then(|m| now.duration_since(m).ok()).is_some_and(|age| age > ttl);
        if meta.is_file() && old && tokio::fs::remove_file(entry.path()).await.is_ok() {
            removed += 1;
        }
    }
    removed
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn round_trip_is_encrypted_at_rest() {
        let dir = tempfile::tempdir().unwrap();
        let spool = Spool::new(dir.path(), [5u8; 32]);
        let secret = b"%PDF-1.7 quarterly numbers nobody should read";
        let (id, meta) = spool.put("u1", "report.pdf", "application/pdf", secret).await.unwrap();
        assert!(valid_upload_id(&id));
        assert_eq!(meta.size, secret.len() as u64);
        assert_eq!(meta.sha256, hex::encode(Sha256::digest(secret)));

        // Nothing readable on disk: not the content, not the file name.
        let raw = std::fs::read(dir.path().join(format!("{id}.bin"))).unwrap();
        assert!(!raw.windows(8).any(|w| w == b"%PDF-1.7"));
        assert!(!raw.windows(6).any(|w| w == b"report"));

        let up = spool.get("u1", &id).await.unwrap();
        assert_eq!(up.content, secret);
        assert_eq!(up.meta.filename, "report.pdf");

        // Another user cannot read it, a different key cannot open it.
        assert!(matches!(spool.get("u2", &id).await, Err(SpoolError::NotFound)));
        let other = Spool::new(dir.path(), [6u8; 32]);
        assert!(other.get("u1", &id).await.is_err());

        spool.remove(&id).await;
        assert!(matches!(spool.get("u1", &id).await, Err(SpoolError::NotFound)));
    }

    #[tokio::test]
    async fn ids_cannot_escape_the_directory() {
        let dir = tempfile::tempdir().unwrap();
        let spool = Spool::new(dir.path(), [5u8; 32]);
        for bad in ["../etc/passwd", "ABCDEF0123456789ABCDEF0123456789", "", "0123"] {
            assert!(!valid_upload_id(bad));
            assert!(matches!(spool.get("u1", bad).await, Err(SpoolError::NotFound)));
        }
    }

    #[tokio::test]
    async fn tampering_and_size_limits() {
        let dir = tempfile::tempdir().unwrap();
        let spool = Spool::new(dir.path(), [5u8; 32]);
        let (id, _) = spool.put("u1", "a.txt", "text/plain", b"hello").await.unwrap();
        let p = dir.path().join(format!("{id}.bin"));
        let mut raw = std::fs::read(&p).unwrap();
        let last = raw.len() - 1;
        raw[last] ^= 0xff;
        std::fs::write(&p, raw).unwrap();
        assert!(spool.get("u1", &id).await.is_err());
        let big = vec![0u8; MAX_UPLOAD_BYTES + 1];
        assert!(matches!(spool.put("u1", "big", "application/octet-stream", &big).await, Err(SpoolError::TooLarge)));
    }

    #[tokio::test]
    async fn sweep_removes_expired_files() {
        let dir = tempfile::tempdir().unwrap();
        let spool = Spool::new(dir.path(), [5u8; 32]);
        let (id, _) = spool.put("u1", "a.txt", "text/plain", b"hello").await.unwrap();
        assert_eq!(sweep_dir(dir.path(), SPOOL_TTL).await, 0);
        tokio::time::sleep(Duration::from_millis(20)).await;
        assert_eq!(sweep_dir(dir.path(), Duration::ZERO).await, 1);
        assert!(matches!(spool.get("u1", &id).await, Err(SpoolError::NotFound)));
    }
}
