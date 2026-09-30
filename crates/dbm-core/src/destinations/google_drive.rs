//! Google Drive [`StorageDestination`] (Section 14). One verified dump is uploaded (resumably) into
//! a per-database app folder. Verification compares the remote size to the expected size (Drive does
//! not expose SHA-256; the source dump's SHA-256 is already verified before distribution).

use crate::error::{BackupError, Result};
use crate::google::drive;
use crate::google::oauth::{self, OAuthConfig};
use crate::traits::{StorageDestination, UploadOutput};
use std::path::Path;

/// A Google Drive destination bound to one account (refresh token) and one folder.
#[derive(Debug, Clone)]
pub struct GoogleDriveDestination {
    name: String,
    cfg: OAuthConfig,
    refresh_token: String,
    folder_id: String,
}

impl GoogleDriveDestination {
    pub fn new(
        name: impl Into<String>,
        client_id: impl Into<String>,
        client_secret: Option<String>,
        refresh_token: impl Into<String>,
        folder_id: impl Into<String>,
    ) -> Self {
        GoogleDriveDestination {
            name: name.into(),
            cfg: OAuthConfig { client_id: client_id.into(), client_secret },
            refresh_token: refresh_token.into(),
            folder_id: folder_id.into(),
        }
    }

    async fn token(&self) -> Result<String> {
        oauth::refresh(&self.cfg, &self.refresh_token).await.map(|t| t.access_token)
    }
}

#[async_trait::async_trait]
impl StorageDestination for GoogleDriveDestination {
    fn name(&self) -> &str {
        &self.name
    }

    async fn preflight(&self) -> Result<()> {
        // A successful token refresh proves the account is still authorized.
        self.token().await.map(|_| ())
    }

    async fn available_space(&self) -> Result<u64> {
        let token = self.token().await?;
        drive::available_space(&token).await
    }

    async fn upload(&self, local: &Path, dest_filename: &str) -> Result<UploadOutput> {
        let token = self.token().await?;
        // Replace any existing same-named file so retries are idempotent.
        if let Some(existing) = drive::find_file(&token, &self.folder_id, dest_filename).await? {
            let _ = drive::delete_file(&token, &existing).await;
        }
        let bytes = tokio::fs::metadata(local).await?.len();
        let file_id = drive::upload_resumable(&token, &self.folder_id, dest_filename, local).await?;
        Ok(UploadOutput {
            stored_path: format!("drive:{}/{}", self.folder_id, dest_filename),
            bytes_written: bytes,
            remote_id: Some(file_id),
        })
    }

    async fn exists(&self, dest_filename: &str) -> Result<bool> {
        let token = self.token().await?;
        Ok(drive::find_file(&token, &self.folder_id, dest_filename).await?.is_some())
    }

    async fn verify(&self, dest_filename: &str, _expected_sha256: &str, expected_size: u64) -> Result<()> {
        let token = self.token().await?;
        let id = drive::find_file(&token, &self.folder_id, dest_filename)
            .await?
            .ok_or_else(|| BackupError::GoogleUploadFailed {
                detail: format!("uploaded file '{dest_filename}' not found for verification"),
            })?;
        let remote = drive::file_size(&token, &id).await?;
        if remote != expected_size {
            return Err(BackupError::ChecksumMismatch {
                expected: format!("{expected_size} bytes"),
                actual: format!("{remote} bytes"),
            });
        }
        Ok(())
    }

    async fn delete(&self, dest_filename: &str) -> Result<()> {
        let token = self.token().await?;
        if let Some(id) = drive::find_file(&token, &self.folder_id, dest_filename).await? {
            drive::delete_file(&token, &id).await?;
        }
        Ok(())
    }
}
