//! Secret storage. Windows Credential Manager for real use; an env-var store for CI/tests.

pub mod env;
pub mod memory;
pub mod wincred;

pub use env::EnvStore;
pub use memory::MemoryStore;
pub use wincred::{WinCredStore, SERVICE};

use crate::error::Result;
use crate::traits::CredentialStore;

/// Dispatches by reference prefix: `env:*` uses the environment, everything else uses the OS
/// credential store. This lets one [`JobSpec`](crate::model::JobSpec) run both on a developer's
/// machine (Credential Manager) and in CI (`env:` reference) without code changes.
#[derive(Debug, Default, Clone)]
pub struct AutoStore {
    wincred: WinCredStore,
    env: EnvStore,
}

impl AutoStore {
    pub fn new() -> Self {
        AutoStore::default()
    }

    fn is_env(reference: &str) -> bool {
        reference.starts_with("env:")
    }
}

impl CredentialStore for AutoStore {
    fn get(&self, reference: &str) -> Result<String> {
        if Self::is_env(reference) {
            self.env.get(reference)
        } else {
            self.wincred.get(reference)
        }
    }

    fn set(&self, reference: &str, secret: &str) -> Result<()> {
        if Self::is_env(reference) {
            self.env.set(reference, secret)
        } else {
            self.wincred.set(reference, secret)
        }
    }

    fn delete(&self, reference: &str) -> Result<()> {
        if Self::is_env(reference) {
            self.env.delete(reference)
        } else {
            self.wincred.delete(reference)
        }
    }
}
