//! In-memory [`CredentialStore`] — used by the GUI's ad-hoc "backup now from a pasted URI" flow
//! (Phase 1, before SQLite/Credential-Manager persistence lands) and by tests. Secrets live only
//! for the lifetime of the process and are never written to disk.

use crate::error::{BackupError, Result};
use crate::traits::CredentialStore;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

#[derive(Debug, Default, Clone)]
pub struct MemoryStore {
    inner: Arc<Mutex<HashMap<String, String>>>,
}

impl MemoryStore {
    pub fn new() -> Self {
        MemoryStore::default()
    }

    /// Convenience constructor seeding a single reference → secret mapping.
    pub fn with_secret(reference: impl Into<String>, secret: impl Into<String>) -> Self {
        let store = MemoryStore::new();
        store
            .inner
            .lock()
            .unwrap()
            .insert(reference.into(), secret.into());
        store
    }
}

impl CredentialStore for MemoryStore {
    fn get(&self, reference: &str) -> Result<String> {
        self.inner
            .lock()
            .unwrap()
            .get(reference)
            .cloned()
            .ok_or_else(|| BackupError::SecretNotFound {
                reference: reference.to_string(),
            })
    }

    fn set(&self, reference: &str, secret: &str) -> Result<()> {
        self.inner
            .lock()
            .unwrap()
            .insert(reference.to_string(), secret.to_string());
        Ok(())
    }

    fn delete(&self, reference: &str) -> Result<()> {
        self.inner.lock().unwrap().remove(reference);
        Ok(())
    }
}
