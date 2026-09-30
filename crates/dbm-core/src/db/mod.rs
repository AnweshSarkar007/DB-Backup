//! Local SQLite persistence (Sections 2, 17). Stores configuration, metadata, policies and backup
//! history. **Never stores secrets** — only `secret_reference` values pointing at the OS credential
//! store (see [`crate::credentials`] and docs/SECURITY.md).

pub mod migrations;
pub mod repository;

pub use repository::{
    ConnectionRecord, DestinationRecord, HydratedJob, PolicyRecord, Repository, RunSummary,
};

use crate::error::{BackupError, Result};
use rusqlite::Connection;
use std::path::Path;
use std::sync::Mutex;

impl From<rusqlite::Error> for BackupError {
    fn from(e: rusqlite::Error) -> Self {
        match e {
            rusqlite::Error::QueryReturnedNoRows => {
                BackupError::NotFound { what: "record".into() }
            }
            other => BackupError::Database { detail: other.to_string() },
        }
    }
}

/// The application's local database. Cheap to clone-wrap in an `Arc`; internally guarded by a mutex
/// (SQLite is single-writer and the desktop workload is low-concurrency).
pub struct Database {
    conn: Mutex<Connection>,
}

impl Database {
    /// Open (creating if needed) the database at `path`, set pragmas, and run migrations.
    pub fn open(path: &Path) -> Result<Database> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let conn = Connection::open(path)?;
        Self::from_conn(conn)
    }

    /// An in-memory database (tests).
    pub fn open_in_memory() -> Result<Database> {
        let conn = Connection::open_in_memory()?;
        Self::from_conn(conn)
    }

    fn from_conn(conn: Connection) -> Result<Database> {
        conn.pragma_update(None, "journal_mode", "WAL").ok();
        conn.pragma_update(None, "foreign_keys", "ON")?;
        Self::migrate(&conn)?;
        Ok(Database { conn: Mutex::new(conn) })
    }

    /// Apply any pending migrations in order, recording each in `schema_migrations`.
    fn migrate(conn: &Connection) -> Result<()> {
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS schema_migrations (version INTEGER PRIMARY KEY, applied_at TEXT NOT NULL);",
        )?;
        for (version, sql) in migrations::MIGRATIONS {
            let already: bool = conn.query_row(
                "SELECT EXISTS(SELECT 1 FROM schema_migrations WHERE version = ?1)",
                [version],
                |r| r.get(0),
            )?;
            if already {
                continue;
            }
            conn.execute_batch(sql)?;
            conn.execute(
                "INSERT OR IGNORE INTO schema_migrations (version, applied_at) VALUES (?1, ?2)",
                rusqlite::params![version, crate::util::now_rfc3339()],
            )?;
            tracing::info!(version, "applied migration");
        }
        Ok(())
    }

    /// Run a closure with the locked connection. Internal helper for the repository impl.
    pub(crate) fn with_conn<T>(&self, f: impl FnOnce(&Connection) -> Result<T>) -> Result<T> {
        let guard = self
            .conn
            .lock()
            .map_err(|_| BackupError::Database { detail: "db mutex poisoned".into() })?;
        f(&guard)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migrations_apply_and_are_idempotent() {
        let db = Database::open_in_memory().unwrap();
        // Second migrate call must be a no-op (all versions already applied).
        db.with_conn(|c| {
            let n: i64 =
                c.query_row("SELECT count(*) FROM schema_migrations", [], |r| r.get(0)).unwrap();
            assert!(n >= 1);
            // The core tables exist.
            for t in ["connections", "destinations", "backup_policies", "backup_runs"] {
                let exists: bool = c
                    .query_row(
                        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name=?1)",
                        [t],
                        |r| r.get(0),
                    )
                    .unwrap();
                assert!(exists, "table {t} missing");
            }
            Ok(())
        })
        .unwrap();
    }
}
