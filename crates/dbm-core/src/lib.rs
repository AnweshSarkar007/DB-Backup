//! # dbm-core
//!
//! The heart of **Database Backup Manager**: all business logic for connecting to PostgreSQL/
//! Supabase, producing verified `pg_dump -Fc` archives via Docker, hashing them, writing manifests,
//! and fanning them out to storage destinations — with **no UI and no Tauri dependency** (spec
//! Rule 8), so every part is unit- and integration-testable without launching the app. Both the
//! desktop GUI (`src-tauri`) and the background worker (`backup-worker`) are thin shells over this
//! crate, which guarantees a scheduled backup behaves identically to a manual one.
//!
//! See `docs/ARCHITECTURE.md` for the module map and `docs/SECURITY.md` for the threat model.

pub mod backup;
pub mod browse;
pub mod config;
pub mod credentials;
pub mod database;
pub mod db;
pub mod destinations;
pub mod docker;
pub mod error;
pub mod google;
pub mod logging;
pub mod model;
pub mod redact;
pub mod restore;
pub mod retention;
pub mod scheduler;
pub mod traits;
pub mod util;

// Curated public surface for the shells (GUI + worker).
pub use backup::{run_backup, EngineOptions, Progress};
pub use config::{parse_connection_uri, AppPaths, ConnectionConfig, ParsedConnection, SslMode};
pub use credentials::{AutoStore, EnvStore, MemoryStore, WinCredStore};
pub use database::PostgresSource;
pub use db::{
    ConnectionRecord, Database, DestinationRecord, HydratedJob, PolicyRecord, RunSummary,
};
pub use browse::{BrowseOpenResult, BrowseSession, TableInfo, TablePage};
pub use restore::{restore_to_target, RestoreParams};
pub use retention::RetentionPolicy;
pub use scheduler::{Frequency, Schedule, TaskScheduler, TaskStatus};
pub use destinations::{GoogleDriveDestination, LocalDestination, GOOGLE_CLIENT_SECRET_REF};
pub use google::{OAuthConfig, DRIVE_FILE_SCOPE};
pub use docker::{doctor, DockerHealth, DockerPgRunner};
pub use error::{BackupError, ErrorReport, Result};
pub use model::{
    compute_final_state, BackupState, DestinationKind, DestinationResult, DestinationSpec,
    DestinationStatus, JobSpec, Manifest, RunResult, TriggerType, VerificationStatus,
};
pub use traits::{
    ArchiveEntry, BackupRunner, CredentialStore, DatabaseInfo, DatabaseSource, ServerVersion,
    StorageDestination,
};
