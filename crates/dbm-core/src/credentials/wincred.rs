//! Windows Credential Manager-backed [`CredentialStore`] (Section 28).
//!
//! Secrets (PostgreSQL passwords; later OAuth refresh tokens) live here — never in SQLite, never in
//! logs. SQLite stores only the *reference* used as the credential's account name.

use crate::error::{BackupError, Result};
use crate::traits::CredentialStore;

/// The service/target name under which all app secrets are grouped.
pub const SERVICE: &str = "DatabaseBackupManager";

/// Stores secrets in the OS credential store (Windows Credential Manager via the `keyring` crate).
#[derive(Debug, Clone)]
pub struct WinCredStore {
    service: String,
}

impl Default for WinCredStore {
    fn default() -> Self {
        WinCredStore {
            service: SERVICE.to_string(),
        }
    }
}

impl WinCredStore {
    pub fn new(service: impl Into<String>) -> Self {
        WinCredStore {
            service: service.into(),
        }
    }

    fn entry(&self, reference: &str) -> Result<keyring::Entry> {
        keyring::Entry::new(&self.service, reference).map_err(|e| BackupError::Internal {
            detail: format!("keyring entry: {e}"),
        })
    }
}

impl CredentialStore for WinCredStore {
    fn get(&self, reference: &str) -> Result<String> {
        match self.entry(reference)?.get_password() {
            Ok(pw) => Ok(pw),
            Err(keyring::Error::NoEntry) => Err(BackupError::SecretNotFound {
                reference: reference.to_string(),
            }),
            Err(e) => Err(BackupError::Internal {
                detail: format!("keyring get: {e}"),
            }),
        }
    }

    fn set(&self, reference: &str, secret: &str) -> Result<()> {
        self.entry(reference)?
            .set_password(secret)
            .map_err(|e| BackupError::Internal {
                detail: format!("keyring set: {e}"),
            })
    }

    fn delete(&self, reference: &str) -> Result<()> {
        match self.entry(reference)?.delete_credential() {
            Ok(()) => Ok(()),
            Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(BackupError::Internal {
                detail: format!("keyring delete: {e}"),
            }),
        }
    }
}
