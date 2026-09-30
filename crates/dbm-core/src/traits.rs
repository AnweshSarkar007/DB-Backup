//! The core abstractions (Section 5). These are the seams that let later phases add cloud
//! destinations, schedulers, and restore flows *without touching the backup engine*.
//!
//! Phase 1 implements: `DatabaseSource` (PostgreSQL), `BackupRunner` (Docker pg tooling),
//! `StorageDestination` (local filesystem), and `CredentialStore` (Windows Credential Manager +
//! an env-var store for tests). The remaining traits are defined now and implemented later.

use crate::config::ConnectionConfig;
use crate::error::Result;
use std::path::{Path, PathBuf};

/// A detected PostgreSQL server version.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerVersion {
    /// Raw `server_version_num`, e.g. 170002.
    pub num: u32,
    /// Major version, e.g. 17.
    pub major: u32,
    /// Human string, e.g. "17.0".
    pub text: String,
}

/// Basic information about the target database.
#[derive(Debug, Clone)]
pub struct DatabaseInfo {
    pub name: String,
    pub size_bytes: Option<u64>,
}

/// A source database that can be tested and interrogated (Section 6/7).
#[allow(async_fn_in_trait)]
pub trait DatabaseSource {
    /// Verify connectivity + auth (a cheap `SELECT 1`).
    async fn test_connection(&self) -> Result<()>;
    /// Detect the server version via `SHOW server_version_num`.
    async fn server_version(&self) -> Result<ServerVersion>;
    /// Fetch the current database name and (best-effort) size.
    async fn database_info(&self) -> Result<DatabaseInfo>;
}

/// Parameters for producing a dump.
#[derive(Debug, Clone)]
pub struct DumpParams {
    pub connection: ConnectionConfig,
    /// The password, held transiently and passed to the container via a restricted env-file.
    pub password: String,
    /// The Docker image to run, e.g. "postgres:17".
    pub image: String,
    /// Absolute host path of the `.dump` file to create.
    pub output_path: PathBuf,
    /// Optional docker network to attach to (tests reach a sibling container by name).
    pub network: Option<String>,
    /// Overall timeout for the dump operation.
    pub timeout_secs: u64,
}

/// Output of a successful dump.
#[derive(Debug, Clone)]
pub struct DumpOutput {
    pub size_bytes: u64,
}

/// One entry from a dump's table of contents (`pg_restore -l`).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct ArchiveEntry {
    /// Object type as reported by pg_restore, e.g. TABLE, SEQUENCE, INDEX, CONSTRAINT, FUNCTION.
    pub object_type: String,
    pub schema: String,
    pub name: String,
}

/// Runs pg tooling to create, verify, inspect and (later) restore dumps (Section 8/12).
#[allow(async_fn_in_trait)]
pub trait BackupRunner {
    /// Create a `pg_dump -Fc` custom-format archive.
    async fn dump(&self, params: &DumpParams) -> Result<DumpOutput>;
    /// Verify a dump is a readable archive (`pg_restore -l` succeeds). A file existing is NOT
    /// success (Section 12).
    async fn verify(&self, dump_path: &Path) -> Result<()>;
    /// Parse the archive's table of contents without modifying any database (Section 22).
    async fn inspect(&self, dump_path: &Path) -> Result<Vec<ArchiveEntry>>;
}

/// Output of writing a dump to a destination.
#[derive(Debug, Clone)]
pub struct UploadOutput {
    pub stored_path: String,
    pub bytes_written: u64,
    /// Remote object id, for cloud destinations (None for local).
    pub remote_id: Option<String>,
}

/// A place a verified dump can be stored (Section 5/13). One dump is fanned out to many of these.
///
/// Uses `#[async_trait]` (not native async-fn-in-trait) so the engine can hold a
/// `Vec<Box<dyn StorageDestination>>` and fan out over heterogeneous backends.
#[async_trait::async_trait]
pub trait StorageDestination: Send + Sync {
    fn name(&self) -> &str;
    /// Check the destination is reachable/writable before we start (existence, connectivity).
    async fn preflight(&self) -> Result<()>;
    /// Free space in bytes, when determinable.
    async fn available_space(&self) -> Result<u64>;
    /// Store the local file under `dest_filename` at the destination.
    async fn upload(&self, local: &Path, dest_filename: &str) -> Result<UploadOutput>;
    async fn exists(&self, dest_filename: &str) -> Result<bool>;
    /// Verify the stored copy matches the expected size and SHA-256.
    async fn verify(
        &self,
        dest_filename: &str,
        expected_sha256: &str,
        expected_size: u64,
    ) -> Result<()>;
    async fn delete(&self, dest_filename: &str) -> Result<()>;
}

/// Secure secret storage (Section 28). Synchronous — the OS credential store is fast and blocking.
///
/// `Send + Sync` so a `&dyn CredentialStore` can cross `.await` points in async engine code without
/// making the resulting futures non-`Send` (Tauri async commands require `Send` futures).
pub trait CredentialStore: Send + Sync {
    /// Fetch a secret by reference. Returns [`crate::error::BackupError::SecretNotFound`] if absent.
    fn get(&self, reference: &str) -> Result<String>;
    fn set(&self, reference: &str, secret: &str) -> Result<()>;
    fn delete(&self, reference: &str) -> Result<()>;
}

// ---------------------------------------------------------------------------------------------
// Traits defined now, implemented in later phases. Kept here so the engine and callers can be
// written against stable seams.
// ---------------------------------------------------------------------------------------------

/// Persistence of runs/history (Section 17). Phase 2 provides the SQLite implementation; Phase 1
/// uses [`NoopRepository`].
pub trait BackupRepository {
    fn record_result(&self, result: &crate::model::RunResult) -> Result<()>;
}

/// A repository that persists nothing (Phase 1 default).
#[derive(Debug, Default, Clone, Copy)]
pub struct NoopRepository;

impl BackupRepository for NoopRepository {
    fn record_result(&self, _result: &crate::model::RunResult) -> Result<()> {
        Ok(())
    }
}

/// Registers/queries OS schedules (Section 19). Implemented in Phase 6 (Windows Task Scheduler).
pub trait Scheduler {
    fn register(&self, job_id: &str, cron_like: &str) -> Result<()>;
    fn unregister(&self, job_id: &str) -> Result<()>;
}

/// Restore/inspect service (Sections 22–24). Implemented in Phases 10–11.
#[allow(async_fn_in_trait)]
pub trait RestoreService {
    async fn inspect(&self, dump_path: &Path) -> Result<Vec<ArchiveEntry>>;
}

/// Transforms an artifact on its way to storage — compression, then encryption (Section 29).
/// Phase 1 ships only [`IdentityProcessor`]; the seam exists so encryption slots in later without
/// touching upload logic.
#[allow(async_fn_in_trait)]
pub trait ArtifactProcessor {
    /// Filename suffix this processor appends (e.g. "" for identity, ".enc" for encryption).
    fn suffix(&self) -> &str;
    /// Transform `input` into `output`. Identity copies/renames.
    async fn process(&self, input: &Path, output: &Path) -> Result<()>;
}

/// No-op processor: the artifact passes through unchanged.
#[derive(Debug, Default, Clone, Copy)]
pub struct IdentityProcessor;

impl ArtifactProcessor for IdentityProcessor {
    fn suffix(&self) -> &str {
        ""
    }
    async fn process(&self, input: &Path, output: &Path) -> Result<()> {
        if input != output {
            tokio::fs::copy(input, output).await?;
        }
        Ok(())
    }
}
