//! Critical backup integration test (Section 47): the one that actually matters.
//!
//! It seeds a real PostgreSQL database with known rows, runs the **full engine** to produce a
//! verified `.dump`, then restores that dump into a *separate* fresh PostgreSQL container and asserts
//! the tables, row counts and known values survived the round-trip. This proves the backup is
//! restorable — not merely that a file exists.
//!
//! Requires Docker. Marked `#[ignore]` so `cargo test` stays fast; run explicitly:
//!   cargo test -p dbm-core --test roundtrip -- --ignored --nocapture

use dbm_core::docker::container_runner::{self, PgConnOpts};
use dbm_core::docker::{self, image_manager};
use dbm_core::{
    config::SslMode, run_backup, BackupState, ConnectionConfig, DestinationKind, DestinationSpec,
    EngineOptions, JobSpec, VerificationStatus,
};
use std::net::TcpListener;
use std::path::Path;
use std::time::Duration;

const PG_IMAGE: &str = "postgres:16";
const PW: &str = "test_pw_9271";

/// A running throwaway postgres container, published on a host port. Removed on drop.
struct PgContainer {
    name: String,
    port: u16,
}

impl PgContainer {
    async fn start(db: &str) -> PgContainer {
        let name = format!("dbm-test-{}", uuid::Uuid::new_v4());
        let port = free_port();
        let args: Vec<String> = vec![
            "run".into(),
            "-d".into(),
            "--rm".into(),
            "--name".into(),
            name.clone(),
            "-p".into(),
            format!("127.0.0.1:{port}:5432"),
            "-e".into(),
            format!("POSTGRES_PASSWORD={PW}"),
            "-e".into(),
            format!("POSTGRES_DB={db}"),
            PG_IMAGE.into(),
        ];
        let out = docker::client::run_docker(&args, Duration::from_secs(120), Some(PW))
            .await
            .expect("docker run");
        assert!(out.success(), "failed to start container: {}", out.stderr);
        PgContainer { name, port }
    }

    /// Connection options for the pg tooling (from a container, via host.docker.internal).
    fn opts(&self, db: &str) -> PgConnOpts {
        PgConnOpts {
            image: PG_IMAGE.to_string(),
            host: "localhost".to_string(), // rewritten to host.docker.internal by the runner
            port: self.port,
            user: "postgres".to_string(),
            database: db.to_string(),
            ssl_mode: SslMode::Disable,
            password: PW.to_string(),
            network: None,
            timeout_secs: 60,
        }
    }

    async fn wait_ready(&self, db: &str) {
        let opts = self.opts(db);
        for _ in 0..40 {
            if container_runner::run_psql(&opts, &tmp_name(), "SELECT 1")
                .await
                .is_ok()
            {
                return;
            }
            tokio::time::sleep(Duration::from_secs(2)).await;
        }
        panic!("postgres container {} did not become ready", self.name);
    }

    async fn psql(&self, db: &str, sql: &str) -> String {
        container_runner::run_psql(&self.opts(db), &tmp_name(), sql)
            .await
            .unwrap_or_else(|e| {
                panic!(
                    "psql failed: {} ({})",
                    e.user_message(),
                    e.technical_detail()
                )
            })
    }
}

impl Drop for PgContainer {
    fn drop(&mut self) {
        // Best-effort synchronous cleanup.
        let _ = std::process::Command::new(docker::client::docker_exe())
            .args(["rm", "-f", &self.name])
            .output();
    }
}

fn tmp_name() -> String {
    format!("dbm-psql-{}", uuid::Uuid::new_v4())
}

fn free_port() -> u16 {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    l.local_addr().unwrap().port()
}

#[tokio::test]
#[ignore = "requires Docker; run with --ignored"]
async fn backup_and_restore_roundtrip_preserves_data() {
    if docker::require_ready().await.is_err() {
        eprintln!("Docker not available — skipping");
        return;
    }
    image_manager::ensure_image(PG_IMAGE)
        .await
        .expect("pull postgres image");

    // --- 1. Source DB with known data. ---
    let src = PgContainer::start("appdb").await;
    src.wait_ready("appdb").await;
    src.psql(
        "appdb",
        "CREATE TABLE customers (id serial PRIMARY KEY, name text NOT NULL, email text); \
         CREATE TABLE products (id serial PRIMARY KEY, sku text, price numeric); \
         CREATE TABLE orders (id serial PRIMARY KEY, customer_id int REFERENCES customers(id), total numeric); \
         INSERT INTO customers (name, email) VALUES ('Alice','a@x.com'),('Bob','b@x.com'),('Carol','c@x.com'); \
         INSERT INTO products (sku, price) VALUES ('SKU-1', 9.99),('SKU-2', 19.50); \
         INSERT INTO orders (customer_id, total) VALUES (1, 9.99),(1, 19.50),(2, 29.49),(3, 5.00),(3, 7.25);",
    )
    .await;

    let cust_count_src = src.psql("appdb", "SELECT count(*) FROM customers").await;
    let order_count_src = src.psql("appdb", "SELECT count(*) FROM orders").await;
    assert_eq!(cust_count_src, "3");
    assert_eq!(order_count_src, "5");

    // --- 2. Full engine backup to a local destination. ---
    let data_dir = std::env::temp_dir().join(format!("dbm-rt-data-{}", uuid::Uuid::new_v4()));
    std::env::set_var("DBM_DATA_DIR", &data_dir);
    let paths = dbm_core::AppPaths::resolve().expect("paths");
    std::env::remove_var("DBM_DATA_DIR");

    let dest_dir = std::env::temp_dir().join(format!("dbm-rt-dest-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dest_dir).unwrap();

    std::env::set_var("DBM_RT_PW", PW);
    let job = JobSpec {
        job_id: format!("rt-{}", uuid::Uuid::new_v4()),
        connection: ConnectionConfig {
            name: "Roundtrip Test".into(),
            host: "localhost".into(),
            port: src.port,
            database: "appdb".into(),
            username: "postgres".into(),
            ssl_mode: SslMode::Disable,
            postgres_major_version: None,
        },
        secret_reference: "env:DBM_RT_PW".into(),
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
    let opts = EngineOptions {
        keep_temp_on_success: true,
        ..Default::default()
    };
    let result = run_backup(&job, &paths, &creds, &opts, |p| {
        eprintln!("[{:?}] {}", p.state, p.message)
    })
    .await;

    assert_eq!(
        result.state,
        BackupState::Completed,
        "backup not completed: {:?}",
        result.error_message
    );
    assert_eq!(result.verification, VerificationStatus::Passed);
    assert!(result.sha256.is_some());
    let dump_file = result.filename.clone().expect("dump filename");
    let dump_path = dest_dir.join(&dump_file);
    assert!(dump_path.exists(), "dump not written to destination");

    // --- 3. Restore into a SEPARATE fresh container and compare. ---
    let target = PgContainer::start("postgres").await;
    target.wait_ready("postgres").await;
    target.psql("postgres", "CREATE DATABASE restored").await;

    let restore_opts = target.opts("restored");
    container_runner::run_pg_restore(&restore_opts, &tmp_name(), &dump_path, false, false)
        .await
        .expect("pg_restore into target");

    let cust_count_dst = target
        .psql("restored", "SELECT count(*) FROM customers")
        .await;
    let order_count_dst = target.psql("restored", "SELECT count(*) FROM orders").await;
    let alice_email = target
        .psql("restored", "SELECT email FROM customers WHERE name='Alice'")
        .await;

    assert_eq!(
        cust_count_dst, cust_count_src,
        "customer count mismatch after restore"
    );
    assert_eq!(
        order_count_dst, order_count_src,
        "order count mismatch after restore"
    );
    assert_eq!(
        alice_email, "a@x.com",
        "known value did not survive round-trip"
    );

    // --- Cleanup temp dirs (containers drop automatically). ---
    let _ = std::fs::remove_dir_all(&dest_dir);
    let _ = std::fs::remove_dir_all(&data_dir);
    std::env::remove_var("DBM_RT_PW");
    ensure_removed(&dump_path);
}

fn ensure_removed(_p: &Path) {}
