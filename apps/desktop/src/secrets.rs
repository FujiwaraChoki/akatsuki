//! Secrets on this device: the Better Auth session token (service `vc.akatsuki.desktop`, account
//! `session`, §6.3) and BYO AI keys (service `vc.akatsuki.ai`, account `<profileId>`, §12.2), in
//! the OS keychain through `keyring` (Secret Service on Linux). Values are never logged.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// Keychain errors (the message never contains the secret).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SecretError {
    /// No keychain is available (no Secret Service running, locked keyring the user dismissed).
    #[error("the system keychain is not available: {0}")]
    Unavailable(String),
    /// The keychain refused the operation.
    #[error("keychain error: {0}")]
    Failed(String),
}

/// A place to keep secrets.
pub trait SecretStore: Send + Sync {
    /// Reads a secret; `Ok(None)` when there is none.
    fn get(&self, service: &str, account: &str) -> Result<Option<String>, SecretError>;
    /// Stores a secret.
    fn set(&self, service: &str, account: &str, value: &str) -> Result<(), SecretError>;
    /// Deletes a secret (no error when there was none).
    fn delete(&self, service: &str, account: &str) -> Result<(), SecretError>;
}

/// The OS keychain.
#[derive(Debug, Default, Clone, Copy)]
pub struct Keychain;

fn map_err(e: keyring::Error) -> SecretError {
    match e {
        keyring::Error::NoDefaultStore | keyring::Error::NoStorageAccess(_) | keyring::Error::PlatformFailure(_) => {
            SecretError::Unavailable(e.to_string())
        }
        other => SecretError::Failed(other.to_string()),
    }
}

impl SecretStore for Keychain {
    fn get(&self, service: &str, account: &str) -> Result<Option<String>, SecretError> {
        let entry = keyring::Entry::new(service, account).map_err(map_err)?;
        match entry.get_password() {
            Ok(v) => Ok(Some(v)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(map_err(e)),
        }
    }

    fn set(&self, service: &str, account: &str, value: &str) -> Result<(), SecretError> {
        keyring::Entry::new(service, account).map_err(map_err)?.set_password(value).map_err(map_err)
    }

    fn delete(&self, service: &str, account: &str) -> Result<(), SecretError> {
        match keyring::Entry::new(service, account).map_err(map_err)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(map_err(e)),
        }
    }
}

/// An in-memory store: tests and the demo mailbox (nothing touches the real keychain there).
#[derive(Debug, Default, Clone)]
pub struct MemoryStore {
    inner: Arc<Mutex<HashMap<(String, String), String>>>,
}

impl SecretStore for MemoryStore {
    fn get(&self, service: &str, account: &str) -> Result<Option<String>, SecretError> {
        let map = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        Ok(map.get(&(service.to_owned(), account.to_owned())).cloned())
    }

    fn set(&self, service: &str, account: &str, value: &str) -> Result<(), SecretError> {
        let mut map = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        map.insert((service.to_owned(), account.to_owned()), value.to_owned());
        Ok(())
    }

    fn delete(&self, service: &str, account: &str) -> Result<(), SecretError> {
        let mut map = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        map.remove(&(service.to_owned(), account.to_owned()));
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn memory_store_round_trip() {
        let s = MemoryStore::default();
        assert_eq!(s.get("svc", "a").unwrap(), None);
        s.set("svc", "a", "secret").unwrap();
        assert_eq!(s.get("svc", "a").unwrap().as_deref(), Some("secret"));
        assert_eq!(s.get("other", "a").unwrap(), None);
        s.delete("svc", "a").unwrap();
        s.delete("svc", "a").unwrap();
        assert_eq!(s.get("svc", "a").unwrap(), None);
    }
}
