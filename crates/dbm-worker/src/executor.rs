//! The shared execution logic. Both the worker CLI and (in Phase 2) the `--job-id` scheduled path
//! call these, so a scheduled backup is identical to a manual one.

use anyhow::{Context, Result};
use dbm_core::{
    backup::engine::Progress, docker, native::SelectedRunner, AppPaths, AutoStore, BackupRunner,
    BackupState, CredentialStore, Database, DatabaseSource, EngineOptions, JobSpec, PostgresSource,
    RunResult, TriggerType,
};
use std::path::Path;

/// Open the local database at the resolved data dir.
pub fn open_db(paths: &AppPaths) -> Result<Database> {
    Database::open(&paths.data_root.join("dbm.sqlite")).context("opening local database")
}

/// Load and parse a job spec JSON file.
pub fn load_spec(path: &Path) -> Result<JobSpec> {
    let bytes =
        std::fs::read(path).with_context(|| format!("reading job spec {}", path.display()))?;
    let spec: JobSpec = serde_json::from_slice(&bytes)
        .with_context(|| format!("parsing job spec {}", path.display()))?;
    Ok(spec)
}

/// Print Docker health and return whether it is ready.
pub async fn doctor() -> bool {
    let h = docker::doctor().await;
    println!("Docker Desktop");
    println!("  Installed: {}", yn(h.installed));
    println!("  Running:   {}", yn(h.running));
    match &h.version {
        Some(v) => println!("  Version:   {v}"),
        None => println!("  Version:   (unavailable)"),
    }
    h.installed && h.running
}

/// Test a connection described by a spec, resolving the password from the credential store.
pub async fn test_connection(spec: &JobSpec) -> Result<()> {
    let creds = AutoStore::new();
    let password = creds
        .get(&spec.secret_reference)
        .map_err(|e| anyhow::anyhow!("{}: {}", e.user_message(), e.technical_detail()))?;
    let source = PostgresSource::new(spec.connection.clone(), password);

    source
        .test_connection()
        .await
        .map_err(|e| anyhow::anyhow!("{}", e.user_message()))?;
    let version = source
        .server_version()
        .await
        .map_err(|e| anyhow::anyhow!("{}", e.user_message()))?;
    let info = source.database_info().await.ok();

    println!("Connection OK");
    println!("  Database: {}", spec.connection.name);
    println!("  Host:     {}", spec.connection.host);
    println!(
        "  Server:   PostgreSQL {} ({})",
        version.major, version.text
    );
    if let Some(i) = info {
        if let Some(size) = i.size_bytes {
            println!("  Size:     {}", dbm_core::error::human_bytes(size));
        }
    }
    Ok(())
}

/// Run a full backup from a standalone spec (not recorded in history — no DB context).
pub async fn run(spec: &JobSpec, paths: &AppPaths, keep_temp: bool) -> RunResult {
    let creds = AutoStore::new();
    let opts = EngineOptions {
        keep_temp_on_success: keep_temp,
        ..Default::default()
    };
    let progress = |p: Progress| println!("[{:?}] {}", p.state, p.message);
    dbm_core::run_backup(spec, paths, &creds, &opts, progress).await
}

/// Run a scheduled/manual backup for a saved policy (`--job-id`), recording it in history. This is
/// the path Windows Task Scheduler invokes; it uses the same engine as a manual GUI backup.
pub async fn run_job(policy_id: i64, paths: &AppPaths, keep_temp: bool) -> Result<RunResult> {
    let db = open_db(paths)?;
    let hydrated = db
        .hydrate_policy_job(policy_id)
        .map_err(|e| anyhow::anyhow!("{}: {}", e.user_message(), e.technical_detail()))?;

    let creds = AutoStore::new();
    let opts = EngineOptions {
        keep_temp_on_success: keep_temp,
        retention: hydrated.retention,
        ..Default::default()
    };
    let progress = |p: Progress| println!("[{:?}] {}", p.state, p.message);

    let result = dbm_core::run_backup(&hydrated.job, paths, &creds, &opts, progress).await;
    let _ = db.record_run(
        &result,
        Some(hydrated.connection_id),
        Some(hydrated.policy_id),
        TriggerType::Scheduled,
        &hydrated.database_label,
    );
    Ok(result)
}

/// Inspect an archive's table of contents.
pub async fn inspect(dump: &Path, _image: &str) -> Result<()> {
    if !dump.exists() {
        anyhow::bail!("dump not found: {}", dump.display());
    }
    let runner = SelectedRunner::select(17, 300);
    let entries = runner
        .inspect(dump)
        .await
        .map_err(|e| anyhow::anyhow!("{}: {}", e.user_message(), e.technical_detail()))?;

    println!("Archive contains {} objects:", entries.len());
    for e in &entries {
        println!("  {:<16} {}.{}", e.object_type, e.schema, e.name);
    }
    Ok(())
}

/// Map a terminal [`RunResult`] to a process exit code:
/// `0` success, `2` partial success, `1` failure/cancelled.
pub fn exit_code(result: &RunResult) -> i32 {
    match result.state {
        BackupState::Completed => 0,
        BackupState::PartialSuccess => 2,
        _ => 1,
    }
}

fn yn(b: bool) -> &'static str {
    if b {
        "yes"
    } else {
        "no"
    }
}
