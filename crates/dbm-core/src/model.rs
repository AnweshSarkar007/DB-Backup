//! Core domain model: the backup state machine, the job spec the worker consumes, the manifest
//! written next to every dump, and the per-run / per-destination results.

use crate::config::ConnectionConfig;
use serde::{Deserialize, Serialize};

/// The backup lifecycle (Section 9). Not a single boolean — every stage is explicit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum BackupState {
    Queued,
    Connecting,
    Preparing,
    Dumping,
    Verifying,
    Hashing,
    Distributing,
    Completed,
    PartialSuccess,
    Failed,
    Cancelled,
}

impl BackupState {
    /// True once the run has reached a final state.
    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            BackupState::Completed
                | BackupState::PartialSuccess
                | BackupState::Failed
                | BackupState::Cancelled
        )
    }
}

/// How a run was triggered (Section 17 `trigger_type`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TriggerType {
    Manual,
    Scheduled,
}

/// A storage destination the verified dump is fanned out to. Phase 1 implements `Local`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum DestinationKind {
    /// A local/removable filesystem folder.
    Local { path: String },
    /// A Google Drive folder. Holds only identifiers/references — the refresh token is resolved from
    /// the credential store (`refresh_token_ref`) and the client secret from a fixed credential key.
    GoogleDrive {
        account_email: String,
        folder_id: String,
        client_id: String,
        refresh_token_ref: String,
    },
    // Future: S3 { .. }, OneDrive { .. }, SFTP { .. }, etc.
}

/// A named, toggleable destination in a job.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DestinationSpec {
    pub name: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(flatten)]
    pub kind: DestinationKind,
}

fn default_true() -> bool {
    true
}

/// The complete description of one backup job — what the worker reads from `job.json` in Phase 1,
/// and what the SQLite job store will hydrate in Phase 2. Contains NO password: only a reference
/// resolved via a `CredentialStore` (Section 28 #1).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JobSpec {
    /// Stable identifier for the job (used for locking + history). String so it can be a UUID or a
    /// numeric id-as-string once SQLite lands.
    pub job_id: String,
    /// Sanitized connection metadata (never a password).
    pub connection: ConnectionConfig,
    /// Reference the `CredentialStore` uses to fetch the password (e.g. a Credential Manager key
    /// or, for CI, the name of an environment variable). Never the password itself.
    pub secret_reference: String,
    /// One or more destinations to fan the verified dump out to.
    pub destinations: Vec<DestinationSpec>,
    #[serde(default)]
    pub trigger: TriggerTypeDefaulted,
}

/// Wrapper so `trigger` can default to Manual when omitted from `job.json`.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(transparent)]
pub struct TriggerTypeDefaulted(pub TriggerType);

impl Default for TriggerTypeDefaulted {
    fn default() -> Self {
        TriggerTypeDefaulted(TriggerType::Manual)
    }
}

/// The backup format. Phase 1 always uses PostgreSQL custom format (`pg_dump -Fc`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum BackupFormat {
    #[serde(rename = "postgres-custom")]
    PostgresCustom,
}

/// Per-destination outcome (Section 16 — tracked individually so partial success is representable).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DestinationStatus {
    Completed,
    Failed,
    /// Was disabled or skipped (e.g. drive disconnected before we started it).
    Skipped,
}

/// Result of writing the dump to one destination.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DestinationResult {
    pub name: String,
    pub status: DestinationStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stored_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bytes_written: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_code: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_message: Option<String>,
}

/// Verification outcome of the dump archive itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VerificationStatus {
    Passed,
    Failed,
    NotRun,
}

/// The manifest written alongside every dump (Section 11).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Manifest {
    pub backup_id: String,
    pub database: String,
    /// RFC3339 with local offset, e.g. `2026-09-30T11:35:24+05:30`.
    pub created_at: String,
    pub postgres_version: String,
    pub format: BackupFormat,
    pub size_bytes: u64,
    pub sha256: String,
    pub verification: VerificationStatus,
    pub engine: String,
    pub destinations: Vec<ManifestDestination>,
}

/// Destination entry inside the manifest (a slimmer view than [`DestinationResult`]).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ManifestDestination {
    pub name: String,
    pub status: DestinationStatus,
}

/// The overall outcome of a backup run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunResult {
    pub backup_id: String,
    pub job_id: String,
    pub state: BackupState,
    pub filename: Option<String>,
    /// Absolute path of the temporary local dump, if it was kept (for retry-failed-destinations).
    #[serde(default)]
    pub local_temp_path: Option<String>,
    pub file_size: Option<u64>,
    pub sha256: Option<String>,
    pub postgres_major_version: Option<u32>,
    pub verification: VerificationStatus,
    pub destinations: Vec<DestinationResult>,
    /// RFC3339 timestamps bounding the run.
    #[serde(default)]
    pub started_at: Option<String>,
    #[serde(default)]
    pub completed_at: Option<String>,
    pub error_code: Option<String>,
    pub error_message: Option<String>,
}

impl RunResult {
    /// A fresh result in the `Queued` state.
    pub fn pending(backup_id: impl Into<String>, job_id: impl Into<String>) -> RunResult {
        RunResult {
            backup_id: backup_id.into(),
            job_id: job_id.into(),
            state: BackupState::Queued,
            filename: None,
            local_temp_path: None,
            file_size: None,
            sha256: None,
            postgres_major_version: None,
            verification: VerificationStatus::NotRun,
            destinations: Vec::new(),
            started_at: None,
            completed_at: None,
            error_code: None,
            error_message: None,
        }
    }
}

/// Compute the terminal state from the destination results, assuming the dump was produced and
/// verified (Section 16). With no successful destinations it is a failure; with a mix it is a
/// partial success; with all successful it is complete. `Skipped` destinations do not, by
/// themselves, downgrade an otherwise fully successful run — but if *nothing* succeeded it fails.
pub fn compute_final_state(results: &[DestinationResult]) -> BackupState {
    let completed = results
        .iter()
        .filter(|r| r.status == DestinationStatus::Completed)
        .count();
    let failed = results
        .iter()
        .filter(|r| r.status == DestinationStatus::Failed)
        .count();

    if completed == 0 {
        // Nothing landed anywhere — treat as failure even if some were merely skipped.
        return BackupState::Failed;
    }
    if failed > 0 {
        return BackupState::PartialSuccess;
    }
    // At least one completed and none failed.
    let skipped = results
        .iter()
        .filter(|r| r.status == DestinationStatus::Skipped)
        .count();
    if skipped > 0 {
        // Some enabled destinations were skipped (e.g. disconnected) → not a clean full success.
        BackupState::PartialSuccess
    } else {
        BackupState::Completed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dr(name: &str, status: DestinationStatus) -> DestinationResult {
        DestinationResult {
            name: name.into(),
            status,
            stored_path: None,
            bytes_written: None,
            error_code: None,
            error_message: None,
        }
    }

    #[test]
    fn all_completed_is_completed() {
        let r = [
            dr("a", DestinationStatus::Completed),
            dr("b", DestinationStatus::Completed),
        ];
        assert_eq!(compute_final_state(&r), BackupState::Completed);
    }

    #[test]
    fn mix_is_partial() {
        let r = [
            dr("a", DestinationStatus::Completed),
            dr("b", DestinationStatus::Failed),
        ];
        assert_eq!(compute_final_state(&r), BackupState::PartialSuccess);
    }

    #[test]
    fn all_failed_is_failed() {
        let r = [
            dr("a", DestinationStatus::Failed),
            dr("b", DestinationStatus::Failed),
        ];
        assert_eq!(compute_final_state(&r), BackupState::Failed);
    }

    #[test]
    fn some_skipped_but_one_ok_is_partial() {
        let r = [
            dr("a", DestinationStatus::Completed),
            dr("b", DestinationStatus::Skipped),
        ];
        assert_eq!(compute_final_state(&r), BackupState::PartialSuccess);
    }

    #[test]
    fn empty_is_failed() {
        assert_eq!(compute_final_state(&[]), BackupState::Failed);
    }

    #[test]
    fn job_spec_deserializes_without_password() {
        let json = r#"{
            "job_id": "12",
            "connection": {
                "name": "Glasafe Production",
                "host": "db.example.com",
                "port": 5432,
                "database": "postgres",
                "username": "postgres",
                "ssl_mode": "require"
            },
            "secret_reference": "dbm:conn:12",
            "destinations": [
                { "name": "Local HDD", "type": "local", "path": "D:\\Database Backups" }
            ]
        }"#;
        let spec: JobSpec = serde_json::from_str(json).unwrap();
        assert_eq!(spec.job_id, "12");
        assert_eq!(spec.destinations.len(), 1);
        assert!(spec.destinations[0].enabled);
        assert_eq!(spec.connection.slug(), "glasafe-production");
        match &spec.destinations[0].kind {
            DestinationKind::Local { path } => assert!(path.contains("Database Backups")),
            _ => panic!("expected a local destination"),
        }
    }

    #[test]
    fn state_terminality() {
        assert!(BackupState::Completed.is_terminal());
        assert!(!BackupState::Dumping.is_terminal());
    }
}
