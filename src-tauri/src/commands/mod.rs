//! Tauri command layer — a *thin* adapter over `dbm-core` (spec Rule 1/8). No business logic here:
//! commands manage app state, call the core, and translate results/errors for the frontend.

use dbm_core::{
    doctor, run_backup, AppPaths, AutoStore, ConnectionConfig, ConnectionRecord, CredentialStore,
    Database, DestinationKind, DestinationRecord, DestinationSpec, DockerHealth, EngineOptions,
    ErrorReport, JobSpec, PolicyRecord, PostgresSource, Progress, RetentionPolicy, RunResult,
    RunSummary, Schedule, TaskScheduler, TaskStatus, TriggerType, DRIVE_FILE_SCOPE,
    GOOGLE_CLIENT_SECRET_REF,
};
use dbm_core::google::{drive, oauth};
use dbm_core::OAuthConfig;
use dbm_core::model::TriggerTypeDefaulted;
use dbm_core::traits::{ArchiveEntry, BackupRunner, DatabaseSource};
use dbm_core::DockerPgRunner;
use serde::Serialize;
use std::path::PathBuf;
use tauri::{AppHandle, Emitter, State};
use tauri_plugin_opener::OpenerExt;

/// Path to the sibling `backup-worker.exe` (used by Task Scheduler).
fn worker_exe() -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.join("backup-worker.exe")))
        .unwrap_or_else(|| PathBuf::from("backup-worker.exe"))
}

/// Shared application state: the local DB, the OS credential store, and the active browse session.
pub struct AppState {
    pub db: Database,
    pub creds: AutoStore,
    pub paths: AppPaths,
    pub browse: tokio::sync::Mutex<Option<dbm_core::BrowseSession>>,
}

/// Summary returned by a connection test.
#[derive(Debug, Serialize)]
pub struct ConnInfo {
    pub database: String,
    pub host: String,
    pub username: String,
    pub postgres_major: u32,
    pub postgres_text: String,
    pub size_bytes: Option<u64>,
    pub engine_image: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ProgressEvent {
    pub state: String,
    pub message: String,
}

fn rep(e: &dbm_core::BackupError) -> ErrorReport {
    ErrorReport::from(e)
}

// -------------------------------------------------------------------------------------------------
// Docker
// -------------------------------------------------------------------------------------------------

#[tauri::command]
pub async fn docker_health() -> DockerHealth {
    doctor().await
}

// -------------------------------------------------------------------------------------------------
// Connections
// -------------------------------------------------------------------------------------------------

#[tauri::command]
pub fn add_connection(
    state: State<'_, AppState>,
    name: String,
    uri: String,
) -> Result<ConnectionRecord, ErrorReport> {
    state.db.add_connection_from_uri(&name, &uri, &state.creds).map_err(|e| rep(&e))
}

#[tauri::command]
pub fn list_connections(state: State<'_, AppState>) -> Result<Vec<ConnectionRecord>, ErrorReport> {
    state.db.list_connections().map_err(|e| rep(&e))
}

#[tauri::command]
pub fn delete_connection(state: State<'_, AppState>, id: i64) -> Result<(), ErrorReport> {
    state.db.delete_connection(id, &state.creds).map_err(|e| rep(&e))
}

/// Test a saved connection (by id), updating its detected version.
#[tauri::command]
pub async fn test_connection(
    state: State<'_, AppState>,
    id: i64,
) -> Result<ConnInfo, ErrorReport> {
    let rec = state.db.get_connection(id).map_err(|e| rep(&e))?;
    let password = state.creds.get(&rec.secret_reference).map_err(|e| rep(&e))?;
    let info = probe(&rec.config, &password).await?;
    let _ = state.db.set_connection_version(id, info.postgres_major);
    Ok(info)
}

/// Test an unsaved URI (used by the "Add Database" form before saving).
#[tauri::command]
pub async fn test_connection_uri(uri: String, name: String) -> Result<ConnInfo, ErrorReport> {
    let parsed = dbm_core::parse_connection_uri(&name, &uri).map_err(|e| rep(&e))?;
    let password = parsed.password.unwrap_or_default();
    probe(&parsed.config, &password).await
}

async fn probe(config: &ConnectionConfig, password: &str) -> Result<ConnInfo, ErrorReport> {
    let source = PostgresSource::new(config.clone(), password.to_string());
    source.test_connection().await.map_err(|e| rep(&e))?;
    let version = source.server_version().await.map_err(|e| rep(&e))?;
    let dbinfo = source.database_info().await.ok();
    let engine_image = dbm_core::docker::image_for_major(version.major).unwrap_or_default();
    Ok(ConnInfo {
        database: config.database.clone(),
        host: config.host.clone(),
        username: config.username.clone(),
        postgres_major: version.major,
        postgres_text: version.text,
        size_bytes: dbinfo.and_then(|i| i.size_bytes),
        engine_image,
    })
}

// -------------------------------------------------------------------------------------------------
// Destinations
// -------------------------------------------------------------------------------------------------

#[tauri::command]
pub fn add_local_destination(
    state: State<'_, AppState>,
    name: String,
    path: String,
) -> Result<DestinationRecord, ErrorReport> {
    state.db.add_local_destination(&name, &path).map_err(|e| rep(&e))
}

#[tauri::command]
pub fn list_destinations(state: State<'_, AppState>) -> Result<Vec<DestinationRecord>, ErrorReport> {
    state.db.list_destinations().map_err(|e| rep(&e))
}

#[derive(Debug, Serialize)]
pub struct GoogleOAuthImportResult { pub client_id: String, pub accounts_reconnect_required: usize, pub same_client_id: bool }

/// Read OAuth JSON in Rust so credentials never enter frontend JavaScript.
#[tauri::command]
pub fn import_google_oauth_json(state: State<'_, AppState>, path: String) -> Result<GoogleOAuthImportResult, ErrorReport> {
    let metadata = std::fs::metadata(&path).map_err(|e| rep(&dbm_core::BackupError::GoogleAuthenticationExpired { detail: format!("could not read selected JSON file: {e}") }))?;
    if metadata.len() > 1024 * 1024 { return Err(rep(&dbm_core::BackupError::GoogleAuthenticationExpired { detail: "OAuth JSON file is unexpectedly large".into() })); }
    let json = std::fs::read_to_string(&path).map_err(|e| rep(&dbm_core::BackupError::GoogleAuthenticationExpired { detail: format!("could not read selected JSON file: {e}") }))?;
    let cfg = oauth::parse_desktop_client_json(&json).map_err(|e| rep(&e))?;
    let old_id = state.db.get_setting("google:client_id").map_err(|e| rep(&e))?;
    let same_client_id = old_id.as_deref() == Some(cfg.client_id.as_str());
    let old_secret = state.creds.get(GOOGLE_CLIENT_SECRET_REF).ok();
    let new_secret = cfg.client_secret.as_deref();
    let credential_result = match new_secret { Some(s) => state.creds.set(GOOGLE_CLIENT_SECRET_REF, s), None => state.creds.delete(GOOGLE_CLIENT_SECRET_REF) };
    credential_result.map_err(|e| rep(&e))?;
    if let Err(e) = state.db.set_setting("google:client_id", &cfg.client_id) {
        match old_secret { Some(s) => { let _ = state.creds.set(GOOGLE_CLIENT_SECRET_REF, &s); }, None => { let _ = state.creds.delete(GOOGLE_CLIENT_SECRET_REF); } }
        return Err(rep(&e));
    }
    let accounts_reconnect_required = if same_client_id { 0 } else {
        match state.db.mark_google_destinations_for_reauthorization(&cfg.client_id) {
            Ok(n) => n,
            Err(e) => {
                if let Some(id) = old_id { let _ = state.db.set_setting("google:client_id", &id); }
                match old_secret { Some(s) => { let _ = state.creds.set(GOOGLE_CLIENT_SECRET_REF, &s); }, None => { let _ = state.creds.delete(GOOGLE_CLIENT_SECRET_REF); } }
                return Err(rep(&e));
            }
        }
    };
    Ok(GoogleOAuthImportResult { client_id: cfg.client_id, accounts_reconnect_required, same_client_id })
}

#[tauri::command]
pub fn google_client_configured(state: State<'_, AppState>) -> Result<bool, ErrorReport> {
    let has_id = state.db.get_setting("google:client_id").map_err(|e| rep(&e))?.is_some();
    Ok(has_id)
}

fn current_google_oauth_config(state: &AppState) -> Result<OAuthConfig, ErrorReport> {
    let client_id = state.db.get_setting("google:client_id").map_err(|e| rep(&e))?
        .ok_or_else(|| rep(&dbm_core::BackupError::GoogleAuthenticationExpired { detail: "Import a Google Cloud Console Desktop app JSON file first".into() }))?;
    let client_secret = match state.creds.get(GOOGLE_CLIENT_SECRET_REF) {
        Ok(s) => Some(s),
        Err(dbm_core::BackupError::SecretNotFound { .. }) => None,
        Err(e) => return Err(rep(&e)),
    };
    Ok(OAuthConfig { client_id, client_secret })
}

async fn google_authorize(app: &AppHandle, cfg: &OAuthConfig) -> Result<(String, String, String), ErrorReport> {
    let (tokens, _) = oauth::run_loopback_flow(cfg, DRIVE_FILE_SCOPE, |url| app.opener().open_url(url, None::<&str>).map_err(|e| e.to_string())).await.map_err(|e| rep(&e))?;
    let refresh = tokens.refresh_token.ok_or_else(|| rep(&dbm_core::BackupError::GoogleAuthenticationExpired { detail: "Google did not return a refresh token; revoke this app's prior access and try again".into() }))?;
    let email = drive::account_email(&tokens.access_token).await.map_err(|e| rep(&e))?;
    let folder = drive::ensure_folder(&tokens.access_token, "Database Backup Manager", None).await.map_err(|e| rep(&e))?;
    Ok((refresh, email, folder))
}

/// Run the Google OAuth desktop flow and save a Google Drive destination.
#[tauri::command]
pub async fn connect_google_drive(
    app: AppHandle,
    state: State<'_, AppState>,
    name: String,
) -> Result<DestinationRecord, ErrorReport> {
    let cfg = current_google_oauth_config(&state)?;
    let client_id = cfg.client_id.clone();
    let (refresh_token, email, folder_id) = google_authorize(&app, &cfg).await?;

    let refresh_ref = format!("gdrive:{}", uuid_v4());
    state.creds.set(&refresh_ref, &refresh_token).map_err(|e| rep(&e))?;

    let config_json = serde_json::json!({
        "account_email": email,
        "folder_id": folder_id,
        "client_id": client_id,
        "reauthorization_required": false,
    })
    .to_string();
    state
        .db
        .add_destination(&name, "google_drive", &config_json, Some(&refresh_ref))
        .map_err(|e| rep(&e))
}

#[tauri::command]
pub async fn reconnect_google_drive(app: AppHandle, state: State<'_, AppState>, destination_id: i64) -> Result<DestinationRecord, ErrorReport> {
    let existing = state.db.get_destination(destination_id).map_err(|e| rep(&e))?;
    if existing.kind != "google_drive" { return Err(rep(&dbm_core::BackupError::Internal { detail: "selected destination is not Google Drive".into() })); }
    let cfg = current_google_oauth_config(&state)?;
    let (refresh, email, folder) = google_authorize(&app, &cfg).await?;
    let secret_ref = existing.secret_reference.clone().ok_or_else(|| rep(&dbm_core::BackupError::Database { detail: "Google destination is missing its credential reference".into() }))?;
    let previous_secret = state.creds.get(&secret_ref).ok();
    state.creds.set(&secret_ref, &refresh).map_err(|e| rep(&e))?;
    let mut configuration: serde_json::Value = serde_json::from_str(&existing.configuration_json).map_err(|e| rep(&dbm_core::BackupError::Database { detail: format!("bad Google destination config: {e}") }))?;
    configuration["account_email"] = serde_json::Value::String(email);
    configuration["folder_id"] = serde_json::Value::String(folder);
    configuration["client_id"] = serde_json::Value::String(cfg.client_id);
    configuration["reauthorization_required"] = serde_json::Value::Bool(false);
    if let Err(e) = state.db.reconnect_google_destination(destination_id, &configuration.to_string()) {
        match previous_secret { Some(s) => { let _ = state.creds.set(&secret_ref, &s); }, None => { let _ = state.creds.delete(&secret_ref); } }
        return Err(rep(&e));
    }
    state.db.get_destination(destination_id).map_err(|e| rep(&e))
}

fn uuid_v4() -> String {
    // Small local helper to avoid importing uuid into the GUI crate.
    use std::time::{SystemTime, UNIX_EPOCH};
    let n = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
    format!("{n:x}")
}

#[tauri::command]
pub fn set_destination_enabled(
    state: State<'_, AppState>,
    id: i64,
    enabled: bool,
) -> Result<(), ErrorReport> {
    state.db.set_destination_enabled(id, enabled).map_err(|e| rep(&e))
}

#[tauri::command]
pub fn delete_destination(state: State<'_, AppState>, id: i64) -> Result<(), ErrorReport> {
    state.db.delete_destination(id).map_err(|e| rep(&e))
}

// -------------------------------------------------------------------------------------------------
// Backup (from a saved connection to selected destinations) + history
// -------------------------------------------------------------------------------------------------

#[tauri::command]
pub async fn backup_now(
    app: AppHandle,
    state: State<'_, AppState>,
    connection_id: i64,
    destination_ids: Vec<i64>,
) -> Result<RunResult, ErrorReport> {
    let conn = state.db.get_connection(connection_id).map_err(|e| rep(&e))?;

    // Build destination specs from the selected saved destinations.
    let mut destinations: Vec<DestinationSpec> = Vec::new();
    for did in &destination_ids {
        let d = state.db.get_destination(*did).map_err(|e| rep(&e))?;
        destinations.push(d.to_spec().map_err(|e| rep(&e))?);
    }
    if destinations.is_empty() {
        return Err(rep(&dbm_core::BackupError::Internal {
            detail: "no destinations selected".into(),
        }));
    }

    let job = JobSpec {
        job_id: format!("conn:{}", conn.uuid),
        connection: conn.config.clone(),
        secret_reference: conn.secret_reference.clone(),
        destinations,
        trigger: TriggerTypeDefaulted(TriggerType::Manual),
    };

    let opts = EngineOptions::default();
    let app2 = app.clone();
    let progress = move |p: Progress| {
        let _ = app2.emit(
            "backup-progress",
            ProgressEvent { state: format!("{:?}", p.state), message: p.message },
        );
    };

    let result = run_backup(&job, &state.paths, &state.creds, &opts, progress).await;
    // Record into history (best-effort; a recording failure must not lose the result).
    let _ = state.db.record_run(
        &result,
        Some(connection_id),
        None,
        TriggerType::Manual,
        &conn.config.name,
    );
    Ok(result)
}

/// Quick ad-hoc backup from a pasted URI to a chosen folder (the original DevBackup flow).
#[tauri::command]
pub async fn run_local_backup(
    app: AppHandle,
    state: State<'_, AppState>,
    uri: String,
    name: String,
    dest_name: String,
    dest_path: String,
) -> Result<RunResult, ErrorReport> {
    let parsed = dbm_core::parse_connection_uri(&name, &uri).map_err(|e| rep(&e))?;
    let password = parsed.password.clone().unwrap_or_default();
    let secret_ref = "gui:adhoc".to_string();
    let creds = dbm_core::MemoryStore::with_secret(&secret_ref, password);

    let job = JobSpec {
        job_id: format!("gui:{}", parsed.config.slug()),
        connection: parsed.config.clone(),
        secret_reference: secret_ref,
        destinations: vec![DestinationSpec {
            name: dest_name,
            enabled: true,
            kind: DestinationKind::Local { path: dest_path },
        }],
        trigger: TriggerTypeDefaulted(TriggerType::Manual),
    };

    let opts = EngineOptions::default();
    let app2 = app.clone();
    let progress = move |p: Progress| {
        let _ = app2.emit(
            "backup-progress",
            ProgressEvent { state: format!("{:?}", p.state), message: p.message },
        );
    };
    let result = run_backup(&job, &state.paths, &creds, &opts, progress).await;
    let _ = state.db.record_run(&result, None, None, TriggerType::Manual, &parsed.config.name);
    Ok(result)
}

#[tauri::command]
pub fn list_runs(
    state: State<'_, AppState>,
    connection_id: Option<i64>,
    status: Option<String>,
    limit: Option<i64>,
) -> Result<Vec<RunSummary>, ErrorReport> {
    state
        .db
        .list_runs(connection_id, status.as_deref(), limit.unwrap_or(200))
        .map_err(|e| rep(&e))
}

#[tauri::command]
pub fn get_run(state: State<'_, AppState>, uuid: String) -> Result<RunSummary, ErrorReport> {
    state.db.get_run_by_uuid(&uuid).map_err(|e| rep(&e))
}

// -------------------------------------------------------------------------------------------------
// Inspect (Phase 9) — read a dump's table of contents without touching any database
// -------------------------------------------------------------------------------------------------

/// Pick the pg image for a dump: prefer the sibling manifest's version, else a sensible default.
async fn image_for_dump(dump: &std::path::Path) -> String {
    let manifest_name = dbm_core::backup::naming::manifest_for_dump(
        &dump.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default(),
    );
    if let Some(dir) = dump.parent() {
        if let Ok(m) = dbm_core::backup::manifest::read_manifest(&dir.join(manifest_name)).await {
            if let Ok(major) = m.postgres_version.parse::<u32>() {
                if let Ok(img) = dbm_core::docker::image_for_major(major) {
                    return img;
                }
            }
        }
    }
    "postgres:17".to_string()
}

#[tauri::command]
pub async fn inspect_dump(path: String) -> Result<Vec<ArchiveEntry>, ErrorReport> {
    let p = PathBuf::from(&path);
    if !p.exists() {
        return Err(rep(&dbm_core::BackupError::InvalidPath {
            detail: format!("file not found: {path}"),
        }));
    }
    let image = image_for_dump(&p).await;
    let runner = DockerPgRunner::new(image, 300);
    runner.inspect(&p).await.map_err(|e| rep(&e))
}

// -------------------------------------------------------------------------------------------------
// Browse a historical backup (Phase 10) — temporary restore into a throwaway container
// -------------------------------------------------------------------------------------------------

#[tauri::command]
pub async fn browse_open(
    state: State<'_, AppState>,
    path: String,
) -> Result<dbm_core::BrowseOpenResult, ErrorReport> {
    let p = PathBuf::from(&path);
    if !p.exists() {
        return Err(rep(&dbm_core::BackupError::InvalidPath {
            detail: format!("file not found: {path}"),
        }));
    }
    let image = image_for_dump(&p).await;
    // Replace any previous session.
    let mut guard = state.browse.lock().await;
    if let Some(prev) = guard.take() {
        prev.close().await;
    }
    let session = dbm_core::BrowseSession::open(&p, &image).await.map_err(|e| rep(&e))?;
    let tables = session.list_tables().await.map_err(|e| rep(&e))?;
    let restore_warning = session.restore_warning().map(str::to_string);
    *guard = Some(session);
    Ok(dbm_core::BrowseOpenResult { tables, restore_warning })
}

#[tauri::command]
pub async fn browse_table(
    state: State<'_, AppState>,
    schema: String,
    table: String,
    offset: i64,
    limit: i64,
) -> Result<dbm_core::TablePage, ErrorReport> {
    let mut guard = state.browse.lock().await;
    if guard.is_none() {
        return Err(rep(&dbm_core::BackupError::Internal {
            detail: "no active browse session".into(),
        }));
    }
    // The first table click lazily initializes the temporary database; subsequent pages reuse it.
    guard
        .as_mut()
        .expect("browse session checked above")
        .table_rows(&schema, &table, offset, limit)
        .await
        .map_err(|e| rep(&e))
}

#[tauri::command]
pub async fn browse_close(state: State<'_, AppState>) -> Result<(), ErrorReport> {
    let mut guard = state.browse.lock().await;
    if let Some(session) = guard.take() {
        session.close().await;
    }
    Ok(())
}

// -------------------------------------------------------------------------------------------------
// Restore into a target database (Phase 11) — destructive, guarded by typed confirmation
// -------------------------------------------------------------------------------------------------

#[tauri::command]
pub async fn restore_backup(
    state: State<'_, AppState>,
    dump_path: String,
    target_connection_id: i64,
    confirm_text: String,
) -> Result<(), ErrorReport> {
    let target = state.db.get_connection(target_connection_id).map_err(|e| rep(&e))?;

    // Defense-in-depth: require the user to have typed the target's exact name.
    if confirm_text.trim() != target.config.name {
        return Err(rep(&dbm_core::BackupError::Internal {
            detail: "confirmation text does not match the target database name".into(),
        }));
    }

    let p = PathBuf::from(&dump_path);
    if !p.exists() {
        return Err(rep(&dbm_core::BackupError::InvalidPath {
            detail: format!("dump not found: {dump_path}"),
        }));
    }

    let password = state.creds.get(&target.secret_reference).map_err(|e| rep(&e))?;
    // Verify the target is reachable before we touch it.
    let _ = probe(&target.config, &password).await?;

    let image = image_for_dump(&p).await;
    let params = dbm_core::RestoreParams {
        target: target.config.clone(),
        password,
        image,
        clean: true,
    };
    dbm_core::restore_to_target(&p, &params).await.map_err(|e| rep(&e))
}

// -------------------------------------------------------------------------------------------------
// Schedules (Phase 6) — a policy (connection + destinations + schedule) registered with Task Scheduler
// -------------------------------------------------------------------------------------------------

/// A schedule as shown on the Schedules page: the policy, its connection, parsed schedule, and the
/// live Windows Task Scheduler status.
#[derive(Debug, Serialize)]
pub struct ScheduleInfo {
    pub policy: PolicyRecord,
    pub connection_name: String,
    pub schedule: Option<Schedule>,
    pub destination_ids: Vec<i64>,
    pub task: TaskStatus,
}

#[tauri::command]
pub fn create_schedule(
    state: State<'_, AppState>,
    connection_id: i64,
    name: String,
    destination_ids: Vec<i64>,
    schedule: Schedule,
    retention: Option<RetentionPolicy>,
) -> Result<PolicyRecord, ErrorReport> {
    schedule.validate().map_err(|e| rep(&e))?;
    let schedule_json = serde_json::to_string(&schedule).unwrap_or_default();
    let retention_json = retention
        .filter(|r| !r.is_noop())
        .map(|r| serde_json::to_string(&r).unwrap_or_default());
    let policy = state
        .db
        .add_policy(
            connection_id,
            &name,
            Some(&schedule_json),
            retention_json.as_deref(),
            &destination_ids,
        )
        .map_err(|e| rep(&e))?;

    let sched = TaskScheduler::new(worker_exe());
    if let Err(e) = sched.create_or_update(policy.id, &policy.uuid, &schedule) {
        // Roll back the policy so we don't leave an unscheduled orphan.
        let _ = state.db.delete_policy(policy.id);
        return Err(rep(&e));
    }
    Ok(policy)
}

#[tauri::command]
pub fn list_schedules(state: State<'_, AppState>) -> Result<Vec<ScheduleInfo>, ErrorReport> {
    let sched = TaskScheduler::new(worker_exe());
    let mut out = Vec::new();
    for policy in state.db.list_policies().map_err(|e| rep(&e))? {
        let connection_name = state
            .db
            .get_connection(policy.connection_id)
            .map(|c| c.config.name)
            .unwrap_or_else(|_| "(deleted)".into());
        let schedule = policy
            .schedule_json
            .as_deref()
            .and_then(|j| serde_json::from_str::<Schedule>(j).ok());
        let destination_ids = state.db.policy_destination_ids(policy.id).unwrap_or_default();
        let task = sched.status(&policy.uuid).unwrap_or_default();
        out.push(ScheduleInfo { policy, connection_name, schedule, destination_ids, task });
    }
    Ok(out)
}

#[tauri::command]
pub fn set_schedule_enabled(
    state: State<'_, AppState>,
    id: i64,
    enabled: bool,
) -> Result<(), ErrorReport> {
    let policy = state.db.get_policy(id).map_err(|e| rep(&e))?;
    let sched = TaskScheduler::new(worker_exe());
    if enabled {
        let schedule: Schedule = policy
            .schedule_json
            .as_deref()
            .and_then(|j| serde_json::from_str(j).ok())
            .ok_or_else(|| rep(&dbm_core::BackupError::SchedulerFailed {
                detail: "policy has no schedule".into(),
            }))?;
        sched.create_or_update(policy.id, &policy.uuid, &schedule).map_err(|e| rep(&e))?;
    } else {
        sched.delete(&policy.uuid).map_err(|e| rep(&e))?;
    }
    state.db.set_policy_enabled(id, enabled).map_err(|e| rep(&e))
}

#[tauri::command]
pub fn delete_schedule(state: State<'_, AppState>, id: i64) -> Result<(), ErrorReport> {
    let policy = state.db.get_policy(id).map_err(|e| rep(&e))?;
    let sched = TaskScheduler::new(worker_exe());
    let _ = sched.delete(&policy.uuid);
    state.db.delete_policy(id).map_err(|e| rep(&e))
}

/// Run a scheduled policy immediately (the "Run now" button), recording it in history.
#[tauri::command]
pub async fn run_schedule_now(
    app: AppHandle,
    state: State<'_, AppState>,
    id: i64,
) -> Result<RunResult, ErrorReport> {
    let hydrated = state.db.hydrate_policy_job(id).map_err(|e| rep(&e))?;
    let opts = EngineOptions { retention: hydrated.retention, ..Default::default() };
    let app2 = app.clone();
    let progress = move |p: Progress| {
        let _ = app2.emit(
            "backup-progress",
            ProgressEvent { state: format!("{:?}", p.state), message: p.message },
        );
    };
    let result = run_backup(&hydrated.job, &state.paths, &state.creds, &opts, progress).await;
    let _ = state.db.record_run(
        &result,
        Some(hydrated.connection_id),
        Some(hydrated.policy_id),
        TriggerType::Manual,
        &hydrated.database_label,
    );
    Ok(result)
}
