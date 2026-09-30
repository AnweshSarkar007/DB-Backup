//! Local / removable filesystem [`StorageDestination`] (Section 13).
//!
//! Robustness: preflight validates the path, detects a disconnected removable drive, and ensures
//! the folder is writable. A single unavailable destination must not crash the whole run — the
//! engine treats a preflight/upload failure here as a per-destination failure (→ partial success).

use crate::backup::hash::sha256_file;
use crate::config::validate_destination_path;
use crate::error::{BackupError, Result};
use crate::traits::{StorageDestination, UploadOutput};
use std::path::{Path, PathBuf};

/// Stores dumps in a filesystem folder (fixed disk, external HDD, or NAS mount).
#[derive(Debug, Clone)]
pub struct LocalDestination {
    name: String,
    base_path: PathBuf,
}

impl LocalDestination {
    /// Construct from a display name and a folder path. The path is validated (absolute, no `..`).
    pub fn new(name: impl Into<String>, path: &str) -> Result<Self> {
        let base_path = validate_destination_path(path)?;
        Ok(LocalDestination {
            name: name.into(),
            base_path,
        })
    }

    fn target(&self, dest_filename: &str) -> Result<PathBuf> {
        // Guard against a filename that tries to escape the base directory.
        if dest_filename.contains('/')
            || dest_filename.contains('\\')
            || dest_filename.contains("..")
        {
            return Err(BackupError::InvalidPath {
                detail: format!("unsafe destination filename: {dest_filename}"),
            });
        }
        Ok(self.base_path.join(dest_filename))
    }

    /// The root that must exist for the drive to be considered "connected" (e.g. `F:\`).
    fn drive_root(&self) -> PathBuf {
        self.base_path
            .ancestors()
            .last()
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| self.base_path.clone())
    }
}

#[async_trait::async_trait]
impl StorageDestination for LocalDestination {
    fn name(&self) -> &str {
        &self.name
    }

    async fn preflight(&self) -> Result<()> {
        // If the drive root does not exist, the removable drive is likely disconnected.
        let root = self.drive_root();
        if !root.exists() {
            return Err(BackupError::DestinationUnavailable {
                name: self.name.clone(),
                detail: format!("drive not connected: {}", root.display()),
            });
        }
        // Ensure the target folder exists (create it if the drive is present).
        if !self.base_path.exists() {
            tokio::fs::create_dir_all(&self.base_path)
                .await
                .map_err(|e| BackupError::DestinationUnavailable {
                    name: self.name.clone(),
                    detail: format!("cannot create folder {}: {e}", self.base_path.display()),
                })?;
        }
        // Probe writability.
        let probe = self.base_path.join(".dbm-write-test");
        tokio::fs::write(&probe, b"1")
            .await
            .map_err(|e| BackupError::DestinationUnavailable {
                name: self.name.clone(),
                detail: format!("folder not writable: {e}"),
            })?;
        let _ = tokio::fs::remove_file(&probe).await;
        Ok(())
    }

    async fn available_space(&self) -> Result<u64> {
        let path = self.base_path.clone();
        tokio::task::spawn_blocking(move || fs2::available_space(&path))
            .await
            .map_err(|e| BackupError::Internal {
                detail: format!("space check join: {e}"),
            })?
            .map_err(|e| BackupError::DestinationUnavailable {
                name: self.name.clone(),
                detail: format!("cannot read free space: {e}"),
            })
    }

    async fn upload(&self, local: &Path, dest_filename: &str) -> Result<UploadOutput> {
        let target = self.target(dest_filename)?;
        let bytes = tokio::fs::copy(local, &target).await.map_err(|e| {
            BackupError::DestinationUnavailable {
                name: self.name.clone(),
                detail: format!("copy failed: {e}"),
            }
        })?;
        Ok(UploadOutput {
            stored_path: target.to_string_lossy().to_string(),
            bytes_written: bytes,
            remote_id: None,
        })
    }

    async fn exists(&self, dest_filename: &str) -> Result<bool> {
        Ok(self.target(dest_filename)?.exists())
    }

    async fn verify(
        &self,
        dest_filename: &str,
        expected_sha256: &str,
        expected_size: u64,
    ) -> Result<()> {
        let target = self.target(dest_filename)?;
        let meta = tokio::fs::metadata(&target).await?;
        if meta.len() != expected_size {
            return Err(BackupError::ChecksumMismatch {
                expected: format!("{expected_size} bytes"),
                actual: format!("{} bytes", meta.len()),
            });
        }
        let actual = sha256_file(&target).await?;
        if actual != expected_sha256 {
            return Err(BackupError::ChecksumMismatch {
                expected: expected_sha256.to_string(),
                actual,
            });
        }
        Ok(())
    }

    async fn delete(&self, dest_filename: &str) -> Result<()> {
        let target = self.target(dest_filename)?;
        if target.exists() {
            tokio::fs::remove_file(&target).await?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn upload_then_verify_roundtrip() {
        let dir = std::env::temp_dir().join(format!("dbm-dest-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let dest = LocalDestination::new("Local HDD", &dir.to_string_lossy()).unwrap();

        // Source file.
        let src = dir.join("source.dump");
        let data = b"hello backup world";
        std::fs::write(&src, data).unwrap();
        let sha = crate::backup::hash::sha256_bytes(data);

        dest.preflight().await.unwrap();
        let out = dest.upload(&src, "copy.dump").await.unwrap();
        assert_eq!(out.bytes_written, data.len() as u64);
        assert!(dest.exists("copy.dump").await.unwrap());
        dest.verify("copy.dump", &sha, data.len() as u64)
            .await
            .unwrap();

        // Tampered expectation fails.
        let bad = dest
            .verify("copy.dump", "deadbeef", data.len() as u64)
            .await;
        assert_eq!(bad.unwrap_err().code(), "ChecksumMismatch");

        dest.delete("copy.dump").await.unwrap();
        assert!(!dest.exists("copy.dump").await.unwrap());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn disconnected_drive_is_unavailable_not_panic() {
        // A drive letter that almost certainly does not exist.
        #[cfg(windows)]
        let path = r"Z:\dbm-nonexistent\backups";
        #[cfg(not(windows))]
        let path = "/nonexistent-root-xyz/dbm/backups";
        let dest = LocalDestination::new("External", path).unwrap();
        let err = dest.preflight().await.unwrap_err();
        assert_eq!(err.code(), "DestinationUnavailable");
    }

    #[test]
    fn rejects_unsafe_filename() {
        let dir = std::env::temp_dir();
        let dest = LocalDestination::new("x", &dir.to_string_lossy()).unwrap();
        assert!(dest.target("../evil.dump").is_err());
        assert!(dest.target("sub/evil.dump").is_err());
    }
}
