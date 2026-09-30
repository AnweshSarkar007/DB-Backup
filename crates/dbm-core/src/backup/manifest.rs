//! Writing/reading the per-backup manifest (Section 11).

use crate::error::{BackupError, Result};
use crate::model::Manifest;
use std::path::Path;

/// Serialize a manifest to pretty JSON and write it to `path`.
pub async fn write_manifest(path: &Path, manifest: &Manifest) -> Result<()> {
    let json = serde_json::to_string_pretty(manifest).map_err(|e| BackupError::Internal {
        detail: format!("manifest serialize: {e}"),
    })?;
    tokio::fs::write(path, json).await?;
    Ok(())
}

/// Read and parse a manifest from `path`.
pub async fn read_manifest(path: &Path) -> Result<Manifest> {
    let bytes = tokio::fs::read(path).await?;
    serde_json::from_slice(&bytes).map_err(|e| BackupError::Internal {
        detail: format!("manifest parse: {e}"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{BackupFormat, DestinationStatus, ManifestDestination, VerificationStatus};

    #[tokio::test]
    async fn manifest_roundtrips_and_matches_schema() {
        let m = Manifest {
            backup_id: "11111111-1111-1111-1111-111111111111".into(),
            database: "Glasafe Production".into(),
            created_at: "2026-09-30T11:35:24+05:30".into(),
            postgres_version: "17".into(),
            format: BackupFormat::PostgresCustom,
            size_bytes: 178_234_899,
            sha256: "deadbeef".into(),
            verification: VerificationStatus::Passed,
            engine: "pg_dump".into(),
            destinations: vec![
                ManifestDestination {
                    name: "Local HDD".into(),
                    status: DestinationStatus::Completed,
                },
                ManifestDestination {
                    name: "Company Google Drive".into(),
                    status: DestinationStatus::Completed,
                },
            ],
        };

        // The serialized form must use the spec's field names / values.
        let json = serde_json::to_string(&m).unwrap();
        assert!(json.contains("\"format\":\"postgres-custom\""));
        assert!(json.contains("\"verification\":\"passed\""));
        assert!(json.contains("\"status\":\"completed\""));

        let dir = std::env::temp_dir().join(format!("dbm-manifest-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("m.json");
        write_manifest(&path, &m).await.unwrap();
        let back = read_manifest(&path).await.unwrap();
        assert_eq!(back.backup_id, m.backup_id);
        assert_eq!(back.size_bytes, m.size_bytes);
        assert_eq!(back.destinations.len(), 2);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
