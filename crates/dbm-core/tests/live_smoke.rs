//! Live smoke test against a real Supabase/PostgreSQL database (env-gated).
//!
//! This is how you validate against your actual database. It is **read-only** against the server:
//! it connects, detects the version, runs `pg_dump -Fc` to a local temp folder, verifies the archive
//! (`pg_restore -l`) and checks the SHA-256 / manifest. It never restores into your database.
//!
//! Set the connection URI and run explicitly (requires Docker):
//!   $env:DBM_LIVE_PG_URL = "postgresql://postgres:PASSWORD@db.xxx.supabase.co:5432/postgres"
//!   cargo test -p dbm-core --test live_smoke -- --ignored --nocapture

use dbm_core::docker;
use dbm_core::{
    parse_connection_uri, run_backup, BackupState, DestinationKind, DestinationSpec, EngineOptions,
    JobSpec, VerificationStatus,
};

#[tokio::test]
#[ignore = "requires DBM_LIVE_PG_URL and Docker; run with --ignored"]
async fn live_smoke_dump_and_verify() {
    let Ok(uri) = std::env::var("DBM_LIVE_PG_URL") else {
        eprintln!("DBM_LIVE_PG_URL not set — skipping live smoke test");
        return;
    };
    if docker::require_ready().await.is_err() {
        eprintln!("Docker not available — skipping");
        return;
    }

    // Parse the URI: sanitized config + extracted password (never persisted/logged).
    let parsed = parse_connection_uri("Live Smoke", &uri).expect("valid connection URI");
    let password = parsed
        .password
        .clone()
        .expect("URI must include a password");
    std::env::set_var("DBM_LIVE_PW", &password);

    // Local temp destination.
    let data_dir = std::env::temp_dir().join(format!("dbm-live-data-{}", uuid::Uuid::new_v4()));
    std::env::set_var("DBM_DATA_DIR", &data_dir);
    let paths = dbm_core::AppPaths::resolve().expect("paths");
    std::env::remove_var("DBM_DATA_DIR");

    let dest_dir = std::env::temp_dir().join(format!("dbm-live-dest-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dest_dir).unwrap();

    let job = JobSpec {
        job_id: "live-smoke".into(),
        connection: parsed.config,
        secret_reference: "env:DBM_LIVE_PW".into(),
        destinations: vec![DestinationSpec {
            name: "Local".into(),
            enabled: true,
            kind: DestinationKind::Local {
                path: dest_dir.to_string_lossy().to_string(),
            },
        }],
        trigger: Default::default(),
    };

    let creds = dbm_core::AutoStore::new();
    let opts = EngineOptions::default();
    let result = run_backup(&job, &paths, &creds, &opts, |p| {
        eprintln!("[{:?}] {}", p.state, p.message)
    })
    .await;

    eprintln!("Result: {:?}", result.state);
    assert_eq!(
        result.state,
        BackupState::Completed,
        "live backup did not complete: {:?}",
        result.error_message
    );
    assert_eq!(result.verification, VerificationStatus::Passed);
    assert!(result.sha256.is_some());
    assert!(result.file_size.unwrap_or(0) > 0);
    eprintln!(
        "OK — {} ({} bytes, sha256={})",
        result.filename.unwrap_or_default(),
        result.file_size.unwrap_or(0),
        result.sha256.unwrap_or_default()
    );

    let _ = std::fs::remove_dir_all(&dest_dir);
    let _ = std::fs::remove_dir_all(&data_dir);
    std::env::remove_var("DBM_LIVE_PW");
}
