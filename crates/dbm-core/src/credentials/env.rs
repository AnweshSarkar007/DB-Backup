//! Environment-variable [`CredentialStore`] for CI / tests / headless use.
//!
//! A reference of `env:DBM_PG_PASSWORD` (or bare `DBM_PG_PASSWORD`) reads that environment variable.
//! This keeps passwords out of files and argv while allowing automated runs where the OS credential
//! store is unavailable.

use crate::error::{BackupError, Result};
use crate::traits::CredentialStore;

#[derive(Debug, Default, Clone, Copy)]
pub struct EnvStore;

/// Strip an optional `env:` prefix from a reference to get the variable name.
pub fn var_name(reference: &str) -> &str {
    reference.strip_prefix("env:").unwrap_or(reference)
}

impl CredentialStore for EnvStore {
    fn get(&self, reference: &str) -> Result<String> {
        let name = var_name(reference);
        std::env::var(name).map_err(|_| BackupError::SecretNotFound {
            reference: reference.to_string(),
        })
    }

    fn set(&self, _reference: &str, _secret: &str) -> Result<()> {
        Err(BackupError::Internal {
            detail: "EnvStore is read-only".into(),
        })
    }

    fn delete(&self, _reference: &str) -> Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_env_var_with_and_without_prefix() {
        let key = format!("DBM_TEST_SECRET_{}", uuid::Uuid::new_v4().simple());
        std::env::set_var(&key, "s3cret");
        assert_eq!(EnvStore.get(&key).unwrap(), "s3cret");
        assert_eq!(EnvStore.get(&format!("env:{key}")).unwrap(), "s3cret");
        std::env::remove_var(&key);
        assert_eq!(EnvStore.get(&key).unwrap_err().code(), "SecretNotFound");
    }
}
