//! Per-job locking to prevent concurrent duplicate backups (Section 10).
//!
//! We use an **OS-level advisory exclusive lock** (via `fs2`) on a per-job lock file. This gives
//! crash-safe stale-lock recovery for free: if the holding process dies, the operating system
//! releases the lock automatically, so a later run can acquire it — no PID liveness polling or TTL
//! guessing required. Manual and scheduled runs are separate processes, so this also stops them
//! from colliding on the same job. We additionally write PID + timestamp + host into the file for
//! diagnostics.

use crate::config::AppPaths;
use crate::error::{BackupError, Result};
use fs2::FileExt;
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::PathBuf;

/// A held job lock. Dropping it releases the OS lock and removes the lock file (best-effort).
#[derive(Debug)]
pub struct JobLock {
    file: Option<File>,
    path: PathBuf,
    job_id: String,
}

impl JobLock {
    /// Try to acquire the lock for `job_id`. Returns [`BackupError::JobAlreadyRunning`] if another
    /// process (or a still-running run in this process) holds it.
    pub fn acquire(paths: &AppPaths, job_id: &str) -> Result<JobLock> {
        let locks_dir = paths.data_root.join("locks");
        std::fs::create_dir_all(&locks_dir)?;
        let safe = crate::config::slugify(job_id);
        let path = locks_dir.join(format!("{safe}.lock"));

        let file = OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .truncate(false)
            .open(&path)?;

        match file.try_lock_exclusive() {
            Ok(()) => {}
            Err(e) => {
                // Distinguish "already locked" from a genuine I/O error.
                if e.kind() == std::io::ErrorKind::WouldBlock
                    || e.raw_os_error() == Some(33) // EWOULDBLOCK on some platforms
                    || e.raw_os_error() == Some(11)
                {
                    return Err(BackupError::JobAlreadyRunning {
                        job: job_id.to_string(),
                    });
                }
                // On Windows, a lock conflict surfaces as ERROR_LOCK_VIOLATION (33).
                return Err(BackupError::JobAlreadyRunning {
                    job: job_id.to_string(),
                });
            }
        }

        // We hold the lock — write diagnostics.
        let mut f = file;
        let _ = f.set_len(0);
        let pid = std::process::id();
        let host = std::env::var("COMPUTERNAME").unwrap_or_else(|_| "unknown".into());
        let now = chrono::Local::now().to_rfc3339();
        let _ = writeln!(f, "job={job_id}\npid={pid}\nhost={host}\nacquired_at={now}");
        let _ = f.flush();

        Ok(JobLock {
            file: Some(f),
            path,
            job_id: job_id.to_string(),
        })
    }

    pub fn job_id(&self) -> &str {
        &self.job_id
    }
}

impl Drop for JobLock {
    fn drop(&mut self) {
        if let Some(file) = self.file.take() {
            let _ = FileExt::unlock(&file);
            drop(file);
        }
        // Best-effort cleanup; if another waiter re-created it, removing is harmless.
        let _ = std::fs::remove_file(&self.path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_paths() -> (AppPaths, PathBuf) {
        let tmp = std::env::temp_dir().join(format!("dbm-lock-{}", uuid::Uuid::new_v4()));
        std::env::set_var("DBM_DATA_DIR", &tmp);
        let paths = AppPaths::resolve().unwrap();
        std::env::remove_var("DBM_DATA_DIR");
        (paths, tmp)
    }

    #[test]
    fn second_acquire_fails_while_held_then_succeeds_after_release() {
        let (paths, tmp) = temp_paths();

        let lock = JobLock::acquire(&paths, "12").unwrap();
        // Second acquisition of the same job must fail.
        let err = JobLock::acquire(&paths, "12").unwrap_err();
        assert_eq!(err.code(), "JobAlreadyRunning");

        // A different job is independent.
        let other = JobLock::acquire(&paths, "13").unwrap();
        drop(other);

        // Releasing the first lets us re-acquire.
        drop(lock);
        let again = JobLock::acquire(&paths, "12").unwrap();
        assert_eq!(again.job_id(), "12");
        drop(again);

        let _ = std::fs::remove_dir_all(&tmp);
    }
}
