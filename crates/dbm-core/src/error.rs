//! Typed errors for the backup engine.
//!
//! Every variant carries a machine-stable `code` (for logs / history / retry logic) and can
//! produce a *human-readable* message plus *technical detail*, matching the spec's requirement
//! (Section 36) that the UI show an understandable message with optional technical context, and
//! never a bare `exit code 125`.

use std::fmt;

/// Result alias used throughout the core.
pub type Result<T> = std::result::Result<T, BackupError>;

/// Whether an error is worth retrying (Section 37).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Retryability {
    /// Transient — safe to retry with backoff (network blips, cloud 5xx).
    Retryable,
    /// Permanent — retrying will not help (auth failure, corrupt archive, bad config).
    NonRetryable,
}

/// The single error type returned by core operations.
#[derive(Debug, thiserror::Error)]
pub enum BackupError {
    #[error("Docker Desktop is not installed")]
    DockerNotInstalled,

    #[error("Docker Desktop is not running")]
    DockerNotRunning,

    #[error("authentication to the PostgreSQL server failed")]
    DatabaseAuthenticationFailed { detail: String },

    #[error("the PostgreSQL server could not be reached")]
    DatabaseUnavailable { detail: String },

    #[error("unsupported PostgreSQL version: {version}")]
    UnsupportedPostgresVersion { version: String },

    #[error("pg_dump failed")]
    PgDumpFailed { detail: String },

    #[error("pg_restore failed")]
    PgRestoreFailed { detail: String },

    #[error("backup verification failed")]
    BackupVerificationFailed { detail: String },

    #[error("insufficient disk space: need ~{required_bytes} bytes, {available_bytes} available")]
    InsufficientDiskSpace {
        required_bytes: u64,
        available_bytes: u64,
    },

    #[error("destination unavailable: {name}")]
    DestinationUnavailable { name: String, detail: String },

    #[error("checksum mismatch (expected {expected}, got {actual})")]
    ChecksumMismatch { expected: String, actual: String },

    #[error("a backup for this job is already running")]
    JobAlreadyRunning { job: String },

    #[error("the requested secret was not found: {reference}")]
    SecretNotFound { reference: String },

    #[error("invalid connection string")]
    InvalidConnectionString { detail: String },

    #[error("invalid path")]
    InvalidPath { detail: String },

    #[error("operation timed out after {seconds}s")]
    Timeout { seconds: u64 },

    #[error("the operation was cancelled")]
    Cancelled,

    #[error("I/O error")]
    Io { detail: String },

    #[error("database (local store) error")]
    Database { detail: String },

    #[error("record not found")]
    NotFound { what: String },

    #[error("scheduler error")]
    SchedulerFailed { detail: String },

    #[error("restore failed")]
    RestoreFailed { detail: String },

    #[error("authentication with the cloud provider expired")]
    GoogleAuthenticationExpired { detail: String },

    #[error("cloud upload failed")]
    GoogleUploadFailed { detail: String },

    #[error("internal error")]
    Internal { detail: String },
}

impl BackupError {
    /// A short machine-stable code, e.g. `DockerNotRunning`. Stored in history / used by callers.
    pub fn code(&self) -> &'static str {
        use BackupError::*;
        match self {
            DockerNotInstalled => "DockerNotInstalled",
            DockerNotRunning => "DockerNotRunning",
            DatabaseAuthenticationFailed { .. } => "DatabaseAuthenticationFailed",
            DatabaseUnavailable { .. } => "DatabaseUnavailable",
            UnsupportedPostgresVersion { .. } => "UnsupportedPostgresVersion",
            PgDumpFailed { .. } => "PgDumpFailed",
            PgRestoreFailed { .. } => "PgRestoreFailed",
            BackupVerificationFailed { .. } => "BackupVerificationFailed",
            InsufficientDiskSpace { .. } => "InsufficientDiskSpace",
            DestinationUnavailable { .. } => "DestinationUnavailable",
            ChecksumMismatch { .. } => "ChecksumMismatch",
            JobAlreadyRunning { .. } => "JobAlreadyRunning",
            SecretNotFound { .. } => "SecretNotFound",
            InvalidConnectionString { .. } => "InvalidConnectionString",
            InvalidPath { .. } => "InvalidPath",
            Timeout { .. } => "Timeout",
            Cancelled => "Cancelled",
            Io { .. } => "Io",
            Database { .. } => "Database",
            NotFound { .. } => "NotFound",
            SchedulerFailed { .. } => "SchedulerFailed",
            RestoreFailed { .. } => "RestoreFailed",
            GoogleAuthenticationExpired { .. } => "GoogleAuthenticationExpired",
            GoogleUploadFailed { .. } => "GoogleUploadFailed",
            Internal { .. } => "Internal",
        }
    }

    /// A friendly, non-technical message safe to show any user. Never contains secrets.
    pub fn user_message(&self) -> String {
        use BackupError::*;
        match self {
            DockerNotInstalled => {
                "Docker Desktop is required but was not found. Please install Docker Desktop.".into()
            }
            DockerNotRunning => {
                "Docker Desktop is installed but not running. Please start Docker Desktop and try again.".into()
            }
            DatabaseAuthenticationFailed { .. } => {
                "Could not sign in to the database. Please check the username and password.".into()
            }
            DatabaseUnavailable { .. } => {
                "Could not connect to the database host. Please check the host, port and network.".into()
            }
            UnsupportedPostgresVersion { version } => {
                format!("This PostgreSQL version ({version}) is not supported by the backup engine.")
            }
            PgDumpFailed { .. } => "The database backup (pg_dump) did not complete successfully.".into(),
            PgRestoreFailed { .. } => "The restore operation did not complete successfully.".into(),
            BackupVerificationFailed { .. } => {
                "The backup file could not be verified and may be corrupt. It was not distributed.".into()
            }
            InsufficientDiskSpace { required_bytes, available_bytes } => format!(
                "Not enough disk space. Estimated need: {}, available: {}.",
                human_bytes(*required_bytes),
                human_bytes(*available_bytes)
            ),
            DestinationUnavailable { name, .. } => {
                format!("The destination \"{name}\" is not available (it may be disconnected).")
            }
            ChecksumMismatch { .. } => {
                "A copied file did not match the original (checksum mismatch).".into()
            }
            JobAlreadyRunning { .. } => "A backup for this database is already running.".into(),
            SecretNotFound { .. } => {
                "The stored password could not be found. Please re-enter the database credentials.".into()
            }
            InvalidConnectionString { .. } => "The connection string is not valid.".into(),
            InvalidPath { .. } => "The provided path is not valid.".into(),
            Timeout { seconds } => format!("The operation timed out after {seconds} seconds."),
            Cancelled => "The operation was cancelled.".into(),
            Io { .. } => "A file system error occurred.".into(),
            Database { .. } => "The local application database could not be accessed.".into(),
            NotFound { what } => format!("Could not find the requested {what}."),
            SchedulerFailed { .. } => {
                "The Windows scheduled task could not be created or updated.".into()
            }
            RestoreFailed { .. } => "The restore did not complete successfully.".into(),
            GoogleAuthenticationExpired { .. } => {
                "Google authorization has expired. Please reconnect the Google Drive account.".into()
            }
            GoogleUploadFailed { .. } => "The upload to Google Drive did not complete.".into(),
            Internal { .. } => "An unexpected internal error occurred.".into(),
        }
    }

    /// Technical detail for the "View technical details" affordance / logs. May be empty.
    /// This is already redacted at the source (callers must not put secrets in `detail`).
    pub fn technical_detail(&self) -> String {
        use BackupError::*;
        match self {
            DatabaseAuthenticationFailed { detail }
            | DatabaseUnavailable { detail }
            | PgDumpFailed { detail }
            | PgRestoreFailed { detail }
            | BackupVerificationFailed { detail }
            | InvalidConnectionString { detail }
            | InvalidPath { detail }
            | Io { detail }
            | Database { detail }
            | SchedulerFailed { detail }
            | RestoreFailed { detail }
            | GoogleAuthenticationExpired { detail }
            | GoogleUploadFailed { detail }
            | Internal { detail } => detail.clone(),
            DestinationUnavailable { detail, .. } => detail.clone(),
            SecretNotFound { reference } => format!("reference={reference}"),
            NotFound { what } => what.clone(),
            ChecksumMismatch { expected, actual } => format!("expected={expected} actual={actual}"),
            _ => String::new(),
        }
    }

    /// Retry classification (Section 37). Auth errors are non-retryable by design.
    pub fn retryability(&self) -> Retryability {
        use BackupError::*;
        match self {
            DatabaseUnavailable { .. }
            | Timeout { .. }
            | Io { .. }
            | GoogleUploadFailed { .. } => Retryability::Retryable,
            _ => Retryability::NonRetryable,
        }
    }
}

impl From<std::io::Error> for BackupError {
    fn from(e: std::io::Error) -> Self {
        BackupError::Io {
            detail: e.to_string(),
        }
    }
}

/// Small helper for human-readable byte sizes in messages.
pub fn human_bytes(bytes: u64) -> String {
    const UNITS: [&str; 6] = ["B", "KB", "MB", "GB", "TB", "PB"];
    if bytes == 0 {
        return "0 B".to_string();
    }
    let mut size = bytes as f64;
    let mut unit = 0usize;
    while size >= 1024.0 && unit < UNITS.len() - 1 {
        size /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} {}", UNITS[unit])
    } else {
        format!("{size:.1} {}", UNITS[unit])
    }
}

/// A structured, serializable view of an error for crossing the Tauri boundary to the frontend.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ErrorReport {
    pub code: String,
    pub message: String,
    pub detail: String,
    pub retryable: bool,
}

impl From<&BackupError> for ErrorReport {
    fn from(e: &BackupError) -> Self {
        ErrorReport {
            code: e.code().to_string(),
            message: e.user_message(),
            detail: e.technical_detail(),
            retryable: e.retryability() == Retryability::Retryable,
        }
    }
}

impl fmt::Display for ErrorReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "[{}] {}", self.code, self.message)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_are_stable() {
        assert_eq!(BackupError::DockerNotRunning.code(), "DockerNotRunning");
        assert_eq!(
            BackupError::ChecksumMismatch {
                expected: "a".into(),
                actual: "b".into()
            }
            .code(),
            "ChecksumMismatch"
        );
    }

    #[test]
    fn auth_is_non_retryable_but_unavailable_is_retryable() {
        assert_eq!(
            BackupError::DatabaseAuthenticationFailed {
                detail: String::new()
            }
            .retryability(),
            Retryability::NonRetryable
        );
        assert_eq!(
            BackupError::DatabaseUnavailable {
                detail: String::new()
            }
            .retryability(),
            Retryability::Retryable
        );
    }

    #[test]
    fn error_report_roundtrip() {
        let e = BackupError::InsufficientDiskSpace {
            required_bytes: 12_000_000_000,
            available_bytes: 4_200_000_000,
        };
        let r = ErrorReport::from(&e);
        assert_eq!(r.code, "InsufficientDiskSpace");
        assert!(r.message.contains("11.2 GB") || r.message.contains("GB"));
        assert!(!r.retryable);
    }

    #[test]
    fn human_bytes_formats() {
        assert_eq!(human_bytes(0), "0 B");
        assert_eq!(human_bytes(512), "512 B");
        assert_eq!(human_bytes(1024), "1.0 KB");
        assert_eq!(human_bytes(178_234_899), "170.0 MB");
    }
}
