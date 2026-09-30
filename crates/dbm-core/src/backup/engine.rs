//! The backup engine: the state machine that turns a [`JobSpec`] into a verified dump fanned out to
//! destinations (Sections 9–12, 15–16, 26–27).
//!
//! Design notes:
//! - The engine returns a [`RunResult`] rather than a bare `Result`: *every* run — success, partial,
//!   or hard failure — produces a record, which is what history wants. The worker maps the terminal
//!   state to an exit code.
//! - The dump is produced and **verified once**, then fanned out. A single failing destination
//!   yields `PartialSuccess`, never a lost backup (Section 16).
//! - Secrets are resolved from the [`CredentialStore`] at the last moment and never logged.

use crate::backup::{hash, manifest, naming};
use crate::config::AppPaths;
use crate::database::PostgresSource;
use crate::docker::{self, DockerPgRunner};
use crate::error::BackupError;
use crate::model::*;
use crate::traits::{
    BackupRunner, CredentialStore, DatabaseSource, DumpParams, StorageDestination,
};
use std::path::Path;

/// Default overall timeout for a single pg_dump (seconds). Overridable per call.
pub const DEFAULT_DUMP_TIMEOUT_SECS: u64 = 3600;

/// A progress update emitted as the run advances (Section 34 — stage-based progress).
#[derive(Debug, Clone)]
pub struct Progress {
    pub state: BackupState,
    pub message: String,
}

/// Options controlling engine behavior (a subset of future Settings — Section 40).
#[derive(Debug, Clone)]
pub struct EngineOptions {
    pub dump_timeout_secs: u64,
    /// Keep the temporary dump after a fully successful run (default: false → clean up).
    pub keep_temp_on_success: bool,
    /// Optional docker network (tests attach the pg tooling to a sibling server container).
    pub network: Option<String>,
    /// Optional retention policy applied to each local destination after a successful run (Section 25).
    pub retention: Option<crate::retention::RetentionPolicy>,
}

impl Default for EngineOptions {
    fn default() -> Self {
        EngineOptions {
            dump_timeout_secs: DEFAULT_DUMP_TIMEOUT_SECS,
            keep_temp_on_success: false,
            network: None,
            retention: None,
        }
    }
}

/// Run a full backup. Never panics; hard failures are captured into the returned [`RunResult`].
pub async fn run_backup<C, P>(
    job: &JobSpec,
    paths: &AppPaths,
    creds: &C,
    opts: &EngineOptions,
    progress: P,
) -> RunResult
where
    C: CredentialStore,
    P: Fn(Progress),
{
    let backup_id = uuid::Uuid::new_v4().to_string();
    let started_at = crate::util::now_rfc3339();
    let mut result = RunResult::pending(backup_id.clone(), job.job_id.clone());
    result.started_at = Some(started_at.clone());

    match run_inner(job, paths, creds, opts, &backup_id, &progress).await {
        Ok(mut r) => {
            r.started_at = Some(started_at);
            r.completed_at = Some(crate::util::now_rfc3339());
            r
        }
        Err(e) => {
            fail(&mut result, &e, &progress);
            result.completed_at = Some(crate::util::now_rfc3339());
            result
        }
    }
}

/// The fallible core; `run_backup` wraps this and converts an early error into a Failed result.
async fn run_inner<C, P>(
    job: &JobSpec,
    paths: &AppPaths,
    creds: &C,
    opts: &EngineOptions,
    backup_id: &str,
    progress: &P,
) -> crate::error::Result<RunResult>
where
    C: CredentialStore,
    P: Fn(Progress),
{
    use crate::backup::lock::JobLock;

    let mut result = RunResult::pending(backup_id.to_string(), job.job_id.clone());

    // --- Lock (Section 10): prevent concurrent duplicate backups. ---
    let _lock = JobLock::acquire(paths, &job.job_id)?;

    // --- Connect + detect version (Sections 6/7). ---
    emit(progress, BackupState::Connecting, "Connecting to database…");
    let password = creds.get(&job.secret_reference)?;
    let source = PostgresSource::new(job.connection.clone(), password.clone());
    source.test_connection().await?;

    let version = source.server_version().await?;
    result.postgres_major_version = Some(version.major);
    let info = source.database_info().await.ok();
    emit(
        progress,
        BackupState::Connecting,
        &format!("Connected. PostgreSQL {} detected.", version.major),
    );

    // --- Docker readiness + matching image (Sections 7/8). ---
    emit(progress, BackupState::Preparing, "Checking Docker…");
    docker::require_ready().await?;
    let image = docker::image_for_major(version.major)?;
    emit(
        progress,
        BackupState::Preparing,
        &format!("Ensuring backup engine image {image}…"),
    );
    docker::ensure_image(&image).await?;

    // --- Working directory (Section 27) + disk-space check (Section 26). ---
    let run_dir = paths.create_run_dir(backup_id)?;
    if let Some(size) = info.as_ref().and_then(|i| i.size_bytes) {
        // Best-effort: a custom-format dump is usually smaller than the live DB, but we still ensure
        // there is at least room for the raw size as a safety margin.
        if let Ok(avail) = tokio::task::spawn_blocking({
            let d = run_dir.clone();
            move || fs2::available_space(&d)
        })
        .await
        .unwrap_or(Ok(u64::MAX))
        {
            if avail < size {
                return Err(BackupError::InsufficientDiskSpace {
                    required_bytes: size,
                    available_bytes: avail,
                });
            }
        }
    }

    // --- Dump (Section 9). ---
    let now = chrono::Local::now();
    let dump_name = naming::dump_filename(&job.connection.name, now);
    let dump_path = run_dir.join(&dump_name);
    result.filename = Some(dump_name.clone());

    emit(progress, BackupState::Dumping, "Running pg_dump…");
    let runner = DockerPgRunner::new(image.clone(), opts.dump_timeout_secs);
    let dump_params = DumpParams {
        connection: job.connection.clone(),
        password: password.clone(),
        image: image.clone(),
        output_path: dump_path.clone(),
        network: opts.network.clone(),
        timeout_secs: opts.dump_timeout_secs,
    };
    let dump = runner.dump(&dump_params).await?;
    result.file_size = Some(dump.size_bytes);

    // --- Verify (Section 12): a file existing is NOT success. ---
    emit(
        progress,
        BackupState::Verifying,
        "Verifying archive (pg_restore -l)…",
    );
    match runner.verify(&dump_path).await {
        Ok(()) => result.verification = VerificationStatus::Passed,
        Err(e) => {
            result.verification = VerificationStatus::Failed;
            // Do not distribute a corrupt dump.
            return Err(e);
        }
    }

    // --- Hash (Section 14). ---
    emit(progress, BackupState::Hashing, "Calculating SHA-256…");
    let sha = hash::sha256_file(&dump_path).await?;
    result.sha256 = Some(sha.clone());

    // --- Distribute (Sections 15/16): one verified dump → many destinations. ---
    emit(
        progress,
        BackupState::Distributing,
        "Distributing to destinations…",
    );
    let manifest_name = naming::manifest_filename(&job.connection.name, now);
    let manifest_path = run_dir.join(&manifest_name);

    let dest_results = distribute(
        job,
        &dump_path,
        &dump_name,
        &manifest_path,
        &manifest_name,
        &sha,
        dump.size_bytes,
        creds,
        progress,
    )
    .await;
    result.destinations = dest_results;

    // --- Manifest (Section 11), written with final destination statuses. ---
    let man = Manifest {
        backup_id: backup_id.to_string(),
        database: job.connection.name.clone(),
        created_at: now.to_rfc3339(),
        postgres_version: version.major.to_string(),
        format: BackupFormat::PostgresCustom,
        size_bytes: dump.size_bytes,
        sha256: sha.clone(),
        verification: result.verification,
        engine: "pg_dump".to_string(),
        destinations: result
            .destinations
            .iter()
            .map(|d| ManifestDestination {
                name: d.name.clone(),
                status: d.status,
            })
            .collect(),
    };
    manifest::write_manifest(&manifest_path, &man).await?;
    // Best-effort: also place the final manifest next to the dump at each completed destination.
    upload_manifest_to_completed(job, &manifest_path, &manifest_name, &result.destinations, creds)
        .await;

    // --- Retention (Section 25): applied per local destination, only where the copy succeeded. ---
    if let Some(policy) = opts.retention {
        if !policy.is_noop() {
            apply_retention(job, &result.destinations, &policy, progress).await;
        }
    }

    // --- Terminal state + cleanup (Sections 16/27). ---
    result.state = compute_final_state(&result.destinations);
    match result.state {
        BackupState::Completed => {
            emit(
                progress,
                BackupState::Completed,
                "Backup completed successfully.",
            );
            if !opts.keep_temp_on_success {
                let _ = tokio::fs::remove_dir_all(&run_dir).await;
            }
        }
        BackupState::PartialSuccess => {
            emit(
                progress,
                BackupState::PartialSuccess,
                "Backup completed with some destinations failing. Temp copy kept for retry.",
            );
        }
        _ => {
            emit(
                progress,
                BackupState::Failed,
                "Backup failed: no destination received the dump.",
            );
        }
    }

    Ok(result)
}

/// Preflight, upload and verify the dump at each enabled destination, returning per-destination
/// results. Never short-circuits — every destination is attempted (Section 16).
#[allow(clippy::too_many_arguments)]
async fn distribute<P>(
    job: &JobSpec,
    dump_path: &Path,
    dump_name: &str,
    _manifest_path: &Path,
    _manifest_name: &str,
    sha: &str,
    size: u64,
    creds: &dyn crate::traits::CredentialStore,
    progress: &P,
) -> Vec<DestinationResult>
where
    P: Fn(Progress),
{
    let mut results = Vec::new();

    for spec in &job.destinations {
        if !spec.enabled {
            results.push(skipped(&spec.name, "destination disabled"));
            continue;
        }

        let dest: Box<dyn StorageDestination> = match crate::destinations::build(spec, creds) {
            Ok(d) => d,
            Err(e) => {
                results.push(failed(&spec.name, &e));
                continue;
            }
        };

        emit(
            progress,
            BackupState::Distributing,
            &format!("→ {}", dest.name()),
        );

        // Preflight (drive connected / writable). Failure = per-destination failure, not fatal.
        if let Err(e) = dest.preflight().await {
            results.push(failed(dest.name(), &e));
            continue;
        }
        // Space check (best-effort).
        if let Ok(avail) = dest.available_space().await {
            if avail < size {
                results.push(failed(
                    dest.name(),
                    &BackupError::InsufficientDiskSpace {
                        required_bytes: size,
                        available_bytes: avail,
                    },
                ));
                continue;
            }
        }
        // Upload the dump.
        let upload = match dest.upload(dump_path, dump_name).await {
            Ok(u) => u,
            Err(e) => {
                results.push(failed(dest.name(), &e));
                continue;
            }
        };
        // Verify the copy (size + SHA-256).
        if let Err(e) = dest.verify(dump_name, sha, size).await {
            // Remove the bad copy so a retry starts clean.
            let _ = dest.delete(dump_name).await;
            results.push(failed(dest.name(), &e));
            continue;
        }

        results.push(DestinationResult {
            name: dest.name().to_string(),
            status: DestinationStatus::Completed,
            stored_path: Some(upload.stored_path),
            bytes_written: Some(upload.bytes_written),
            error_code: None,
            error_message: None,
        });
    }

    results
}

/// Apply retention to each local destination that received this backup (Section 25). Best-effort:
/// a retention failure at one destination never affects the run's success or other destinations.
async fn apply_retention<P: Fn(Progress)>(
    job: &JobSpec,
    results: &[DestinationResult],
    policy: &crate::retention::RetentionPolicy,
    progress: &P,
) {
    let slug = job.connection.slug();
    let now = chrono::Local::now().naive_local();
    for spec in &job.destinations {
        let completed = results
            .iter()
            .any(|r| r.name == spec.name && r.status == DestinationStatus::Completed);
        if !completed {
            continue;
        }
        // Retention currently applies to local destinations; cloud backends handle their own
        // retention in a later phase. `match` (not `if let`) so new kinds must be handled explicitly.
        #[allow(clippy::single_match)]
        match &spec.kind {
            crate::model::DestinationKind::Local { path } => {
                let dest = match crate::destinations::LocalDestination::new(spec.name.clone(), path) {
                    Ok(d) => d,
                    Err(_) => continue,
                };
                let dir = std::path::Path::new(path);
                match crate::retention::apply_local(&dest, dir, &slug, policy, now).await {
                    Ok(deleted) if !deleted.is_empty() => emit(
                        progress,
                        BackupState::Distributing,
                        &format!(
                            "Retention removed {} old backup(s) from {}",
                            deleted.len(),
                            spec.name
                        ),
                    ),
                    Ok(_) => {}
                    Err(e) => {
                        tracing::warn!(dest = %spec.name, error = %e.technical_detail(), "retention failed")
                    }
                }
            }
            // Cloud destinations manage their own retention in a later phase.
            _ => {}
        }
    }
}

/// Best-effort copy of the final manifest to each completed destination.
async fn upload_manifest_to_completed(
    job: &JobSpec,
    manifest_path: &Path,
    manifest_name: &str,
    results: &[DestinationResult],
    creds: &dyn crate::traits::CredentialStore,
) {
    for spec in &job.destinations {
        let completed = results
            .iter()
            .any(|r| r.name == spec.name && r.status == DestinationStatus::Completed);
        if !completed {
            continue;
        }
        if let Ok(dest) = crate::destinations::build(spec, creds) {
            let _ = dest.upload(manifest_path, manifest_name).await;
        }
    }
}

fn emit<P: Fn(Progress)>(progress: &P, state: BackupState, message: &str) {
    tracing::info!(?state, "{message}");
    progress(Progress {
        state,
        message: message.to_string(),
    });
}

fn fail<P: Fn(Progress)>(result: &mut RunResult, e: &BackupError, progress: &P) {
    result.state = if e.code() == "Cancelled" {
        BackupState::Cancelled
    } else {
        BackupState::Failed
    };
    result.error_code = Some(e.code().to_string());
    result.error_message = Some(e.user_message());
    tracing::error!(code = e.code(), detail = %e.technical_detail(), "{}", e.user_message());
    progress(Progress {
        state: result.state,
        message: e.user_message(),
    });
}

fn failed(name: &str, e: &BackupError) -> DestinationResult {
    DestinationResult {
        name: name.to_string(),
        status: DestinationStatus::Failed,
        stored_path: None,
        bytes_written: None,
        error_code: Some(e.code().to_string()),
        error_message: Some(e.user_message()),
    }
}

fn skipped(name: &str, why: &str) -> DestinationResult {
    DestinationResult {
        name: name.to_string(),
        status: DestinationStatus::Skipped,
        stored_path: None,
        bytes_written: None,
        error_code: None,
        error_message: Some(why.to_string()),
    }
}
