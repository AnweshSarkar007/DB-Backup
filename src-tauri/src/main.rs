//! Database Backup Manager — desktop GUI entry point (Tauri 2).
//!
//! This process is the UI shell only. All backup logic lives in `dbm-core`; scheduled backups run
//! in a separate `backup-worker.exe` so they work with this window closed (Section 3).

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod commands;

use commands::AppState;
use dbm_core::{AppPaths, AutoStore, Database};

fn main() {
    // Resolve app dirs + logging first so early failures are captured.
    let paths = AppPaths::resolve().expect("failed to resolve application directories");
    if let Some(guard) = dbm_core::logging::init(&paths) {
        // Keep the log-flushing worker alive for the whole process.
        std::mem::forget(guard);
    }

    let db_path = paths.data_root.join("dbm.sqlite");
    let db = Database::open(&db_path).expect("failed to open local database");

    let state = AppState {
        db,
        creds: AutoStore::new(),
        paths,
        browse: tokio::sync::Mutex::new(None),
    };

    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .manage(state)
        .invoke_handler(tauri::generate_handler![
            commands::docker_health,
            commands::add_connection,
            commands::list_connections,
            commands::delete_connection,
            commands::test_connection,
            commands::test_connection_uri,
            commands::add_local_destination,
            commands::import_google_oauth_json,
            commands::set_google_client,
            commands::google_client_configured,
            commands::connect_google_drive,
            commands::reconnect_google_drive,
            commands::list_destinations,
            commands::set_destination_enabled,
            commands::delete_destination,
            commands::backup_now,
            commands::run_local_backup,
            commands::list_runs,
            commands::get_run,
            commands::create_schedule,
            commands::list_schedules,
            commands::set_schedule_enabled,
            commands::delete_schedule,
            commands::run_schedule_now,
            commands::inspect_dump,
            commands::browse_open,
            commands::browse_table,
            commands::browse_close,
            commands::restore_backup,
        ])
        .run(tauri::generate_context!())
        .expect("error while running Database Backup Manager");
}
