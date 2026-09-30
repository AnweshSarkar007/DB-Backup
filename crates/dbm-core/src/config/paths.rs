//! Application directory resolution and temporary working directories (Section 27).
//!
//! Layout (Windows):
//! ```text
//! %ProgramData%\DatabaseBackupManager\        (data_root; shared, survives users)
//!     logs\                                    log files
//!     work\<run-uuid>\                         per-run temp dump + manifest (restricted ACL)
//! ```
//! If `%ProgramData%` cannot be created/written (locked-down machine, no elevation), we fall back
//! to `%LOCALAPPDATA%\DatabaseBackupManager\`, which is user-private by NTFS default. The location
//! can also be forced with the `DBM_DATA_DIR` environment variable (used by tests).

use crate::error::{BackupError, Result};
use std::path::{Path, PathBuf};

pub const APP_DIR_NAME: &str = "DatabaseBackupManager";

/// Resolved application paths.
#[derive(Debug, Clone)]
pub struct AppPaths {
    pub data_root: PathBuf,
    pub logs_dir: PathBuf,
    pub work_root: PathBuf,
}

impl AppPaths {
    /// Resolve and create the application directories, honoring `DBM_DATA_DIR` then falling back
    /// from `%ProgramData%` to `%LOCALAPPDATA%`.
    pub fn resolve() -> Result<AppPaths> {
        let data_root = resolve_data_root()?;
        let logs_dir = data_root.join("logs");
        let work_root = data_root.join("work");
        std::fs::create_dir_all(&logs_dir)?;
        std::fs::create_dir_all(&work_root)?;
        Ok(AppPaths {
            data_root,
            logs_dir,
            work_root,
        })
    }

    /// Create (and lock down) a fresh per-run working directory `work\<run-uuid>\`.
    pub fn create_run_dir(&self, run_uuid: &str) -> Result<PathBuf> {
        let dir = self.work_root.join(run_uuid);
        std::fs::create_dir_all(&dir)?;
        restrict_dir_permissions(&dir);
        Ok(dir)
    }
}

fn resolve_data_root() -> Result<PathBuf> {
    if let Ok(dir) = std::env::var("DBM_DATA_DIR") {
        let p = PathBuf::from(dir);
        std::fs::create_dir_all(&p)?;
        return Ok(p);
    }

    // Preferred: %ProgramData%\DatabaseBackupManager
    if let Ok(program_data) = std::env::var("ProgramData") {
        let candidate = PathBuf::from(program_data).join(APP_DIR_NAME);
        if std::fs::create_dir_all(&candidate).is_ok() && is_writable(&candidate) {
            return Ok(candidate);
        }
    }

    // Fallback: %LOCALAPPDATA%\DatabaseBackupManager (user-private).
    if let Ok(local) = std::env::var("LOCALAPPDATA") {
        let candidate = PathBuf::from(local).join(APP_DIR_NAME);
        std::fs::create_dir_all(&candidate)?;
        return Ok(candidate);
    }

    Err(BackupError::Internal {
        detail: "could not resolve a data directory (no ProgramData or LOCALAPPDATA)".into(),
    })
}

fn is_writable(dir: &Path) -> bool {
    let probe = dir.join(".dbm-write-test");
    match std::fs::write(&probe, b"1") {
        Ok(_) => {
            let _ = std::fs::remove_file(&probe);
            true
        }
        Err(_) => false,
    }
}

/// Best-effort tightening of a directory's ACL so only the current user (plus SYSTEM and
/// Administrators) can access it. Non-fatal: failures are logged, not returned, because temp dirs
/// under `%LOCALAPPDATA%` are already user-private and this is defense-in-depth (Section 28 #10).
#[cfg(windows)]
pub fn restrict_dir_permissions(dir: &Path) {
    use std::process::Command;
    // Disable inheritance and grant only the current user, SYSTEM and Administrators.
    // icacls is part of Windows; we run it non-interactively and ignore output.
    let username = std::env::var("USERNAME").unwrap_or_default();
    if username.is_empty() {
        return;
    }
    let result = Command::new("icacls")
        .arg(dir)
        .arg("/inheritance:r")
        .arg("/grant:r")
        .arg(format!("{username}:(OI)(CI)F"))
        .arg("/grant:r")
        .arg("*S-1-5-18:(OI)(CI)F") // SYSTEM
        .arg("/grant:r")
        .arg("*S-1-5-32-544:(OI)(CI)F") // BUILTIN\Administrators
        .arg("/Q")
        .output();
    if let Err(e) = result {
        tracing::debug!(error = %e, "could not restrict work dir permissions (non-fatal)");
    }
}

#[cfg(not(windows))]
pub fn restrict_dir_permissions(dir: &Path) {
    use std::os::unix::fs::PermissionsExt;
    if let Ok(meta) = std::fs::metadata(dir) {
        let mut perms = meta.permissions();
        perms.set_mode(0o700);
        let _ = std::fs::set_permissions(dir, perms);
    }
}

/// Validate that a user-provided destination path is absolute and free of traversal components
/// (Section 28 #6/#7). Returns the normalized path.
pub fn validate_destination_path(path: &str) -> Result<PathBuf> {
    let p = PathBuf::from(path.trim());
    if p.as_os_str().is_empty() {
        return Err(BackupError::InvalidPath {
            detail: "empty path".into(),
        });
    }
    if !p.is_absolute() {
        return Err(BackupError::InvalidPath {
            detail: format!("destination path must be absolute: {path}"),
        });
    }
    if p.components()
        .any(|c| matches!(c, std::path::Component::ParentDir))
    {
        return Err(BackupError::InvalidPath {
            detail: format!("destination path must not contain '..': {path}"),
        });
    }
    Ok(p)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_relative_and_traversal_paths() {
        assert!(validate_destination_path("relative/dir").is_err());
        assert!(validate_destination_path("").is_err());
        #[cfg(windows)]
        assert!(validate_destination_path(r"C:\backups\..\windows").is_err());
        #[cfg(not(windows))]
        assert!(validate_destination_path("/backups/../etc").is_err());
    }

    #[test]
    fn accepts_clean_absolute_path() {
        #[cfg(windows)]
        assert!(validate_destination_path(r"D:\Database Backups").is_ok());
        #[cfg(not(windows))]
        assert!(validate_destination_path("/data/backups").is_ok());
    }

    #[test]
    fn resolve_uses_env_override() {
        let tmp = std::env::temp_dir().join(format!("dbm-paths-test-{}", uuid::Uuid::new_v4()));
        std::env::set_var("DBM_DATA_DIR", &tmp);
        let paths = AppPaths::resolve().unwrap();
        assert!(paths.data_root.exists());
        assert!(paths.work_root.exists());
        assert!(paths.logs_dir.exists());
        std::env::remove_var("DBM_DATA_DIR");
        let _ = std::fs::remove_dir_all(&tmp);
    }
}
