//! PostgreSQL Docker image management (Section 7): map a detected major version to an image tag,
//! check whether it is present, and pull it on demand.

use crate::docker::client::run_docker;
use crate::error::{BackupError, Result};
use std::time::Duration;

/// Lowest PostgreSQL major version we support backing up. Older servers are rejected with a clear
/// error rather than silently using a mismatched client.
pub const MIN_SUPPORTED_MAJOR: u32 = 12;

/// The image tag for a given PostgreSQL major version, e.g. 17 -> "postgres:17".
///
/// Returns an error for versions below [`MIN_SUPPORTED_MAJOR`].
pub fn image_for_major(major: u32) -> Result<String> {
    if major < MIN_SUPPORTED_MAJOR {
        return Err(BackupError::UnsupportedPostgresVersion {
            version: major.to_string(),
        });
    }
    Ok(format!("postgres:{major}"))
}

/// Does the image already exist locally? (`docker image inspect`)
pub async fn image_exists(image: &str) -> Result<bool> {
    let out = run_docker(
        &["image".into(), "inspect".into(), image.to_string()],
        Duration::from_secs(20),
        None,
    )
    .await?;
    Ok(out.success())
}

/// Pull an image (`docker pull`). Large images can take a while, hence the generous timeout.
pub async fn pull_image(image: &str) -> Result<()> {
    let out = run_docker(
        &["pull".into(), image.to_string()],
        Duration::from_secs(600),
        None,
    )
    .await?;
    if out.success() {
        Ok(())
    } else {
        Err(BackupError::Internal {
            detail: format!("docker pull {image} failed: {}", out.stderr.trim()),
        })
    }
}

/// Ensure the image is present, pulling it if missing (Section 7 — auto-pull).
pub async fn ensure_image(image: &str) -> Result<()> {
    if image_exists(image).await? {
        return Ok(());
    }
    tracing::info!(image, "required postgres image missing; pulling");
    pull_image(image).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_major_to_tag() {
        assert_eq!(image_for_major(17).unwrap(), "postgres:17");
        assert_eq!(image_for_major(15).unwrap(), "postgres:15");
    }

    #[test]
    fn rejects_ancient_versions() {
        let e = image_for_major(9).unwrap_err();
        assert_eq!(e.code(), "UnsupportedPostgresVersion");
    }
}
