//! Storage destinations. Phase 1: local filesystem. Google Drive et al. arrive in Phase 7+.

pub mod google_drive;
pub mod local;

pub use google_drive::GoogleDriveDestination;
pub use local::LocalDestination;

use crate::error::Result;
use crate::model::{DestinationKind, DestinationSpec};
use crate::traits::{CredentialStore, StorageDestination};

/// Credential key under which the shared Google OAuth client secret is stored.
pub const GOOGLE_CLIENT_SECRET_REF: &str = "google:client_secret";

/// Build a boxed [`StorageDestination`] from a [`DestinationSpec`], resolving any secrets from the
/// credential store. This is the one place that maps spec → concrete implementation, so adding a
/// cloud backend is a single new match arm.
pub fn build(
    spec: &DestinationSpec,
    creds: &dyn CredentialStore,
) -> Result<Box<dyn StorageDestination>> {
    match &spec.kind {
        DestinationKind::Local { path } => {
            Ok(Box::new(LocalDestination::new(spec.name.clone(), path)?))
        }
        DestinationKind::GoogleDrive { folder_id, client_id, refresh_token_ref, .. } => {
            let client_secret = match creds.get(GOOGLE_CLIENT_SECRET_REF) {
                Ok(secret) => Some(secret),
                Err(crate::error::BackupError::SecretNotFound { .. }) => None,
                Err(error) => return Err(error),
            };
            let refresh_token = creds.get(refresh_token_ref)?;
            Ok(Box::new(GoogleDriveDestination::new(
                spec.name.clone(),
                client_id.clone(),
                client_secret,
                refresh_token,
                folder_id.clone(),
            )))
        }
    }
}
