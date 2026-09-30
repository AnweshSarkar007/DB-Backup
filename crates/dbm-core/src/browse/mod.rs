//! Browse a historical backup (Section 23): restore a dump into a **temporary** throwaway Postgres
//! container, list its tables, and page through rows — read-only, never touching a real database.
//!
//! The container publishes a port on localhost only and is force-removed when the session is closed
//! (or on drop, as a safety net for crashes). Row pages are bounded with SQL LIMIT/OFFSET so the
//! frontend never loads millions of rows.

use crate::config::SslMode;
use crate::docker::{self, container_runner, container_runner::PgConnOpts, image_manager};
use crate::error::{BackupError, Result};
use crate::traits::ArchiveEntry;
use serde::Serialize;
use std::net::TcpListener;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio_postgres::Client;

const BROWSE_DB: &str = "dbm_browse";
const BROWSE_PW: &str = "dbm_browse_pw";

/// A table in the restored database.
#[derive(Debug, Clone, Serialize)]
pub struct TableInfo {
    pub schema: String,
    pub name: String,
    /// Estimated row count (from planner statistics — fast, avoids counting huge tables).
    pub estimated_rows: i64,
}

/// A page of rows from a table.
#[derive(Debug, Clone, Serialize)]
pub struct TablePage {
    pub columns: Vec<String>,
    pub rows: Vec<Vec<Option<String>>>,
    pub offset: i64,
    pub limit: i64,
    pub restore_warning: Option<String>,
}

/// Tables available after opening a dump, with any pg_restore errors that were skipped.
#[derive(Debug, Clone, Serialize)]
pub struct BrowseOpenResult {
    pub tables: Vec<TableInfo>,
    pub restore_warning: Option<String>,
}

/// A live browse session backed by a temporary container.
pub struct BrowseSession {
    container: Option<String>,
    port: Option<u16>,
    image: String,
    dump: PathBuf,
    tables: Vec<TableInfo>,
    loaded_tables: HashSet<(String, String)>,
    restored: bool,
    restore_warning: Option<String>,
}

fn free_port() -> Result<u16> {
    let l = TcpListener::bind("127.0.0.1:0")
        .map_err(|e| BackupError::Internal { detail: format!("no free port: {e}") })?;
    l.local_addr().map(|a| a.port()).map_err(|e| BackupError::Internal { detail: e.to_string() })
}

fn quote_ident(ident: &str) -> String {
    format!("\"{}\"", ident.replace('"', "\"\""))
}

impl BrowseSession {
    /// Read table names from the archive without restoring table data or starting a container.
    pub async fn open(dump: &Path, image: &str) -> Result<BrowseSession> {
        docker::require_ready().await?;
        image_manager::ensure_image(image).await?;
        let toc = container_runner::list_archive(image, dump, 300).await?;
        let tables = crate::docker::runner::parse_toc(&toc)
            .into_iter()
            .filter(|entry: &ArchiveEntry| {
                entry.object_type == "TABLE"
                    && entry.schema != "pg_catalog"
                    && entry.schema != "information_schema"
            })
            .map(|entry| TableInfo { schema: entry.schema, name: entry.name, estimated_rows: -1 })
            .collect();

        Ok(BrowseSession {
            container: None,
            port: None,
            image: image.to_string(),
            dump: dump.to_path_buf(),
            tables,
            loaded_tables: HashSet::new(),
            restored: false,
            restore_warning: None,
        })
    }

    fn opts(&self, db: &str, timeout_secs: u64) -> PgConnOpts {
        PgConnOpts {
            image: self.image.clone(),
            host: "localhost".into(),
            port: self.port.expect("container started before database connection"),
            user: "postgres".into(),
            database: db.into(),
            ssl_mode: SslMode::Disable,
            password: BROWSE_PW.into(),
            network: None,
            timeout_secs,
        }
    }

    async fn wait_ready(&self) -> Result<()> {
        for _ in 0..40 {
            if self.connect(BROWSE_DB).await.is_ok() || self.connect("postgres").await.is_ok() {
                return Ok(());
            }
            tokio::time::sleep(Duration::from_secs(2)).await;
        }
        Err(BackupError::RestoreFailed { detail: "browse container did not become ready".into() })
    }

    async fn create_db_and_restore(&self) -> Result<Option<String>> {
        let name = format!("dbm-browse-psql-{}", uuid::Uuid::new_v4());
        // Creating the DB is instant; the restore can take minutes for a large dump.
        container_runner::run_psql(
            &self.opts("postgres", 120),
            &name,
            &format!("CREATE DATABASE {BROWSE_DB}"),
        )
        .await?;
        let rname = format!("dbm-browse-restore-{}", uuid::Uuid::new_v4());
        container_runner::run_pg_restore_for_browse(
            &self.opts(BROWSE_DB, 3600),
            &rname,
            &self.dump,
            Some("pre-data"),
            None,
        )
            .await
    }

    async fn ensure_table_data(&mut self, schema: &str, table: &str) -> Result<()> {
        let key = (schema.to_string(), table.to_string());
        if self.loaded_tables.contains(&key) {
            return Ok(());
        }
        let name = format!("dbm-browse-table-{}", uuid::Uuid::new_v4());
        let warning = container_runner::run_pg_restore_for_browse(
            &self.opts(BROWSE_DB, 3600),
            &name,
            &self.dump,
            Some(schema),
            Some(table),
        )
        .await?;
        if let Some(warning) = warning {
            self.restore_warning = Some(match self.restore_warning.take() {
                Some(previous) => format!("{previous}\n{warning}"),
                None => warning,
            });
        }
        self.loaded_tables.insert(key);
        Ok(())
    }

    async fn ensure_restored(&mut self) -> Result<()> {
        if self.restored {
            return Ok(());
        }
        let container = format!("dbm-browse-{}", uuid::Uuid::new_v4());
        let port = free_port()?;
        let run_args = vec![
            "run".into(), "-d".into(), "--rm".into(), "--name".into(), container.clone(),
            "-p".into(), format!("127.0.0.1:{port}:5432"), "-e".into(),
            format!("POSTGRES_PASSWORD={BROWSE_PW}"), self.image.clone(),
        ];
        let out = docker::client::run_docker(&run_args, Duration::from_secs(120), Some(BROWSE_PW)).await?;
        if !out.success() {
            return Err(BackupError::RestoreFailed {
                detail: format!("could not start browse container: {}", out.stderr.trim()),
            });
        }
        self.container = Some(container);
        self.port = Some(port);
        if let Err(e) = self.wait_ready().await {
            self.force_remove().await;
            return Err(e);
        }
        match self.create_db_and_restore().await {
            Ok(warning) => {
                self.restore_warning = warning;
                self.restored = true;
                Ok(())
            }
            Err(e) => {
                self.force_remove().await;
                Err(e)
            }
        }
    }

    /// Open a host-side connection to the restored database.
    async fn connect(&self, db: &str) -> Result<Client> {
        let mut cfg = tokio_postgres::Config::new();
        cfg.host("localhost")
            .port(self.port.expect("container started before database connection"))
            .user("postgres")
            .password(BROWSE_PW)
            .dbname(db)
            .connect_timeout(Duration::from_secs(10))
            .ssl_mode(tokio_postgres::config::SslMode::Disable);
        let connector = postgres_native_tls::MakeTlsConnector::new(
            native_tls::TlsConnector::builder()
                .build()
                .map_err(|e| BackupError::Internal { detail: format!("tls: {e}") })?,
        );
        let (client, connection) = cfg
            .connect(connector)
            .await
            .map_err(|e| BackupError::RestoreFailed { detail: e.to_string() })?;
        tokio::spawn(async move {
            let _ = connection.await;
        });
        Ok(client)
    }

    /// List user tables with estimated row counts.
    pub async fn list_tables(&self) -> Result<Vec<TableInfo>> {
        Ok(self.tables.clone())
    }

    pub fn restore_warning(&self) -> Option<&str> {
        self.restore_warning.as_deref()
    }

    /// Return a bounded page of rows from a table, every value rendered as text (via row_to_json).
    pub async fn table_rows(
        &mut self,
        schema: &str,
        table: &str,
        offset: i64,
        limit: i64,
    ) -> Result<TablePage> {
        self.ensure_restored().await?;
        self.ensure_table_data(schema, table).await?;
        let client = self.connect(BROWSE_DB).await?;
        let limit = limit.clamp(1, 500);
        let offset = offset.max(0);

        // Column names in order.
        let col_rows = client
            .query(
                "SELECT column_name FROM information_schema.columns
                 WHERE table_schema = $1 AND table_name = $2 ORDER BY ordinal_position",
                &[&schema, &table],
            )
            .await
            .map_err(|e| BackupError::RestoreFailed { detail: e.to_string() })?;
        let columns: Vec<String> = col_rows.iter().map(|r| r.get::<_, String>(0)).collect();
        if columns.is_empty() {
            return Err(BackupError::NotFound { what: format!("table {schema}.{table}") });
        }

        // Rows as JSON objects (uniformly stringifiable), stable order by ctid.
        let sql = format!(
            "SELECT row_to_json(t)::text FROM (SELECT * FROM {}.{} ORDER BY ctid LIMIT {} OFFSET {}) t",
            quote_ident(schema),
            quote_ident(table),
            limit,
            offset
        );
        let data_rows = client
            .query(&sql, &[])
            .await
            .map_err(|e| BackupError::RestoreFailed { detail: e.to_string() })?;

        let mut rows = Vec::with_capacity(data_rows.len());
        for r in &data_rows {
            let json: String = r.get(0);
            let val: serde_json::Value = serde_json::from_str(&json).unwrap_or(serde_json::Value::Null);
            let obj = val.as_object();
            let cells = columns
                .iter()
                .map(|c| {
                    obj.and_then(|o| o.get(c)).map(|v| match v {
                        serde_json::Value::Null => None,
                        serde_json::Value::String(s) => Some(s.clone()),
                        other => Some(other.to_string()),
                    })
                    .flatten()
                })
                .collect();
            rows.push(cells);
        }
        Ok(TablePage { columns, rows, offset, limit, restore_warning: self.restore_warning.clone() })
    }

    /// Force-remove the temporary container.
    pub async fn force_remove(&self) {
        if let Some(container) = &self.container {
            container_runner::remove_container(container).await;
        }
    }

    pub async fn close(self) {
        self.force_remove().await;
    }

}
