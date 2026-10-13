//! Repository methods over [`Database`](super::Database): CRUD for connections, destinations and
//! policies, plus recording and querying backup history (Section 17/20).
//!
//! Secrets never land here — connections/destinations store a `secret_reference` and the actual
//! secret is put in the OS credential store by these helpers when a URI is imported.

use super::Database;
use crate::config::{parse_connection_uri, ConnectionConfig, SslMode};
use crate::error::{BackupError, Result};
use crate::model::{
    DestinationKind, DestinationSpec, DestinationStatus, JobSpec, RunResult, TriggerType,
    TriggerTypeDefaulted,
};
use crate::traits::CredentialStore;
use crate::util::{new_uuid, now_rfc3339};
use rusqlite::{params, OptionalExtension, Row};
use serde::Serialize;

/// A stored database connection (sanitized config + secret reference).
#[derive(Debug, Clone, Serialize)]
pub struct ConnectionRecord {
    pub id: i64,
    pub uuid: String,
    pub config: ConnectionConfig,
    pub secret_reference: String,
    pub created_at: String,
    pub updated_at: String,
}

/// A stored destination.
#[derive(Debug, Clone, Serialize)]
pub struct DestinationRecord {
    pub id: i64,
    pub uuid: String,
    pub name: String,
    pub kind: String,
    pub configuration_json: String,
    pub secret_reference: Option<String>,
    pub enabled: bool,
    pub created_at: String,
    pub updated_at: String,
}

impl DestinationRecord {
    /// Convert to the engine's [`DestinationSpec`]. Errors for unknown/unsupported kinds.
    pub fn to_spec(&self) -> Result<DestinationSpec> {
        let kind = match self.kind.as_str() {
            "local" => {
                let cfg: serde_json::Value = serde_json::from_str(&self.configuration_json)
                    .map_err(|e| BackupError::Database {
                        detail: format!("bad dest config: {e}"),
                    })?;
                let path = cfg
                    .get("path")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| BackupError::Database {
                        detail: "local dest missing path".into(),
                    })?
                    .to_string();
                DestinationKind::Local { path }
            }
            "google_drive" => {
                let cfg: serde_json::Value = serde_json::from_str(&self.configuration_json)
                    .map_err(|e| BackupError::Database {
                        detail: format!("bad dest config: {e}"),
                    })?;
                let s = |k: &str| {
                    cfg.get(k)
                        .and_then(|v| v.as_str())
                        .unwrap_or_default()
                        .to_string()
                };
                DestinationKind::GoogleDrive {
                    account_email: s("account_email"),
                    folder_id: s("folder_id"),
                    client_id: s("client_id"),
                    refresh_token_ref: self.secret_reference.clone().unwrap_or_default(),
                }
            }
            other => {
                return Err(BackupError::Database {
                    detail: format!("unsupported destination kind '{other}'"),
                })
            }
        };
        Ok(DestinationSpec {
            name: self.name.clone(),
            enabled: self.enabled,
            kind,
        })
    }
}

/// A stored backup policy (a schedule + destinations for a connection).
#[derive(Debug, Clone, Serialize)]
pub struct PolicyRecord {
    pub id: i64,
    pub uuid: String,
    pub connection_id: i64,
    pub name: String,
    pub schedule_json: Option<String>,
    pub retention_policy_json: Option<String>,
    pub enabled: bool,
    pub created_at: String,
    pub updated_at: String,
}

/// A fully-hydrated job ready to run, plus the ids needed to record its history.
#[derive(Debug, Clone)]
pub struct HydratedJob {
    pub job: JobSpec,
    pub connection_id: i64,
    pub policy_id: i64,
    pub database_label: String,
    pub retention: Option<crate::retention::RetentionPolicy>,
}

/// A history row for the History list (Section 20).
#[derive(Debug, Clone, Serialize)]
pub struct RunSummary {
    pub id: i64,
    pub uuid: String,
    pub connection_id: Option<i64>,
    pub policy_id: Option<i64>,
    pub trigger_type: String,
    pub database_label: Option<String>,
    pub started_at: Option<String>,
    pub completed_at: Option<String>,
    pub status: String,
    pub filename: Option<String>,
    pub file_size: Option<i64>,
    pub sha256: Option<String>,
    pub verification_status: Option<String>,
    pub error_code: Option<String>,
    pub error_message: Option<String>,
}

fn enum_str<T: Serialize>(v: &T) -> String {
    serde_json::to_value(v)
        .ok()
        .and_then(|x| x.as_str().map(String::from))
        .unwrap_or_default()
}

/// Repository operations. Implemented directly on [`Database`].
impl Database {
    // ---- Connections ---------------------------------------------------------------------

    /// Import a connection from a URI: parse it, store the password in `creds` under a generated
    /// reference, and persist only sanitized metadata (Section 6/28).
    pub fn add_connection_from_uri(
        &self,
        name: &str,
        uri: &str,
        creds: &dyn CredentialStore,
    ) -> Result<ConnectionRecord> {
        let parsed = parse_connection_uri(name, uri)?;
        let uuid = new_uuid();
        let secret_reference = format!("conn:{uuid}");
        if let Some(pw) = parsed.password {
            creds.set(&secret_reference, &pw)?;
        }
        self.insert_connection(&uuid, &parsed.config, &secret_reference)
    }

    fn insert_connection(
        &self,
        uuid: &str,
        config: &ConnectionConfig,
        secret_reference: &str,
    ) -> Result<ConnectionRecord> {
        let now = now_rfc3339();
        self.with_conn(|c| {
            c.execute(
                "INSERT INTO connections
                 (uuid,name,host,port,database_name,username,ssl_mode,secret_reference,postgres_major_version,created_at,updated_at)
                 VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?10)",
                params![
                    uuid,
                    config.name,
                    config.host,
                    config.port,
                    config.database,
                    config.username,
                    config.ssl_mode.as_libpq(),
                    secret_reference,
                    config.postgres_major_version,
                    now,
                ],
            )?;
            Ok(())
        })?;
        self.get_connection_by_uuid(uuid)
    }

    pub fn list_connections(&self) -> Result<Vec<ConnectionRecord>> {
        self.with_conn(|c| {
            let mut stmt = c.prepare("SELECT * FROM connections ORDER BY name")?;
            let rows = stmt
                .query_map([], row_to_connection)?
                .collect::<std::result::Result<Vec<_>, _>>()?;
            Ok(rows)
        })
    }

    pub fn get_connection(&self, id: i64) -> Result<ConnectionRecord> {
        self.with_conn(|c| {
            c.query_row(
                "SELECT * FROM connections WHERE id=?1",
                [id],
                row_to_connection,
            )
            .map_err(Into::into)
        })
    }

    pub fn get_connection_by_uuid(&self, uuid: &str) -> Result<ConnectionRecord> {
        self.with_conn(|c| {
            c.query_row(
                "SELECT * FROM connections WHERE uuid=?1",
                [uuid],
                row_to_connection,
            )
            .map_err(Into::into)
        })
    }

    /// Update the detected postgres major version after a successful connection test.
    pub fn set_connection_version(&self, id: i64, major: u32) -> Result<()> {
        self.with_conn(|c| {
            c.execute(
                "UPDATE connections SET postgres_major_version=?1, updated_at=?2 WHERE id=?3",
                params![major, now_rfc3339(), id],
            )?;
            Ok(())
        })
    }

    /// Delete a connection and its secret from the credential store.
    pub fn delete_connection(&self, id: i64, creds: &dyn CredentialStore) -> Result<()> {
        let rec = self.get_connection(id)?;
        let _ = creds.delete(&rec.secret_reference);
        self.with_conn(|c| {
            c.execute("DELETE FROM connections WHERE id=?1", [id])?;
            Ok(())
        })
    }

    // ---- Destinations --------------------------------------------------------------------

    pub fn add_local_destination(&self, name: &str, path: &str) -> Result<DestinationRecord> {
        let uuid = new_uuid();
        let now = now_rfc3339();
        let config = serde_json::json!({ "path": path }).to_string();
        self.with_conn(|c| {
            c.execute(
                "INSERT INTO destinations (uuid,name,type,configuration_json,secret_reference,enabled,created_at,updated_at)
                 VALUES (?1,?2,'local',?3,NULL,1,?4,?4)",
                params![uuid, name, config, now],
            )?;
            Ok(())
        })?;
        self.get_destination_by_uuid(&uuid)
    }

    /// Insert an arbitrary destination (used by Phase 7 for google_drive, etc.).
    pub fn add_destination(
        &self,
        name: &str,
        kind: &str,
        configuration_json: &str,
        secret_reference: Option<&str>,
    ) -> Result<DestinationRecord> {
        let uuid = new_uuid();
        let now = now_rfc3339();
        self.with_conn(|c| {
            c.execute(
                "INSERT INTO destinations (uuid,name,type,configuration_json,secret_reference,enabled,created_at,updated_at)
                 VALUES (?1,?2,?3,?4,?5,1,?6,?6)",
                params![uuid, name, kind, configuration_json, secret_reference, now],
            )?;
            Ok(())
        })?;
        self.get_destination_by_uuid(&uuid)
    }

    pub fn list_destinations(&self) -> Result<Vec<DestinationRecord>> {
        self.with_conn(|c| {
            let mut stmt = c.prepare("SELECT * FROM destinations ORDER BY name")?;
            let rows = stmt
                .query_map([], row_to_destination)?
                .collect::<std::result::Result<Vec<_>, _>>()?;
            Ok(rows)
        })
    }

    pub fn get_destination(&self, id: i64) -> Result<DestinationRecord> {
        self.with_conn(|c| {
            c.query_row(
                "SELECT * FROM destinations WHERE id=?1",
                [id],
                row_to_destination,
            )
            .map_err(Into::into)
        })
    }

    pub fn get_destination_by_uuid(&self, uuid: &str) -> Result<DestinationRecord> {
        self.with_conn(|c| {
            c.query_row(
                "SELECT * FROM destinations WHERE uuid=?1",
                [uuid],
                row_to_destination,
            )
            .map_err(Into::into)
        })
    }

    pub fn set_destination_enabled(&self, id: i64, enabled: bool) -> Result<()> {
        self.with_conn(|c| {
            if enabled {
                let rec = c.query_row(
                    "SELECT type, configuration_json FROM destinations WHERE id=?1",
                    [id],
                    |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)),
                )?;
                if rec.0 == "google_drive"
                    && serde_json::from_str::<serde_json::Value>(&rec.1)
                        .ok()
                        .and_then(|v| v.get("reauthorization_required").and_then(|x| x.as_bool()))
                        .unwrap_or(false)
                {
                    return Err(BackupError::GoogleAuthenticationExpired {
                        detail: "Reconnect this Google Drive account before enabling it".into(),
                    });
                }
            }
            c.execute(
                "UPDATE destinations SET enabled=?1, updated_at=?2 WHERE id=?3",
                params![enabled as i64, now_rfc3339(), id],
            )?;
            Ok(())
        })
    }

    /// On OAuth client replacement, preserve existing destination identities and tokens while
    /// disabling those that must authorize against the new client.
    pub fn mark_google_destinations_for_reauthorization(
        &self,
        new_client_id: &str,
    ) -> Result<usize> {
        self.with_conn(|c| {
            let mut stmt = c.prepare("SELECT id, configuration_json FROM destinations WHERE type='google_drive'")?;
            let rows = stmt.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))?
                .collect::<std::result::Result<Vec<_>, _>>()?;
            drop(stmt);
            let mut changed = 0;
            for (id, raw) in rows {
                let mut cfg: serde_json::Value = serde_json::from_str(&raw).map_err(|e| BackupError::Database { detail: format!("bad Google destination config: {e}") })?;
                if cfg.get("client_id").and_then(|v| v.as_str()) != Some(new_client_id) {
                    cfg["reauthorization_required"] = serde_json::Value::Bool(true);
                    c.execute("UPDATE destinations SET configuration_json=?1, enabled=0, updated_at=?2 WHERE id=?3", params![cfg.to_string(), now_rfc3339(), id])?;
                    changed += 1;
                }
            }
            Ok(changed)
        })
    }

    /// Update an existing Google destination after successful authorization, preserving its id/name.
    pub fn reconnect_google_destination(&self, id: i64, configuration_json: &str) -> Result<()> {
        self.with_conn(|c| {
            let changed = c.execute("UPDATE destinations SET configuration_json=?1, enabled=1, updated_at=?2 WHERE id=?3 AND type='google_drive'", params![configuration_json, now_rfc3339(), id])?;
            if changed == 0 { return Err(BackupError::NotFound { what: format!("Google Drive destination {id}") }); }
            Ok(())
        })
    }

    pub fn delete_destination(&self, id: i64) -> Result<()> {
        self.with_conn(|c| {
            c.execute("DELETE FROM destinations WHERE id=?1", [id])?;
            Ok(())
        })
    }

    // ---- Policies ------------------------------------------------------------------------

    pub fn add_policy(
        &self,
        connection_id: i64,
        name: &str,
        schedule_json: Option<&str>,
        retention_policy_json: Option<&str>,
        destination_ids: &[i64],
    ) -> Result<PolicyRecord> {
        let uuid = new_uuid();
        let now = now_rfc3339();
        self.with_conn(|c| {
            c.execute(
                "INSERT INTO backup_policies (uuid,connection_id,name,schedule_json,retention_policy_json,enabled,created_at,updated_at)
                 VALUES (?1,?2,?3,?4,?5,1,?6,?6)",
                params![uuid, connection_id, name, schedule_json, retention_policy_json, now],
            )?;
            let pid = c.last_insert_rowid();
            for did in destination_ids {
                c.execute(
                    "INSERT OR IGNORE INTO policy_destinations (policy_id,destination_id) VALUES (?1,?2)",
                    params![pid, did],
                )?;
            }
            Ok(())
        })?;
        self.get_policy_by_uuid(&uuid)
    }

    pub fn list_policies(&self) -> Result<Vec<PolicyRecord>> {
        self.with_conn(|c| {
            let mut stmt = c.prepare("SELECT * FROM backup_policies ORDER BY name")?;
            let rows = stmt
                .query_map([], row_to_policy)?
                .collect::<std::result::Result<Vec<_>, _>>()?;
            Ok(rows)
        })
    }

    pub fn get_policy(&self, id: i64) -> Result<PolicyRecord> {
        self.with_conn(|c| {
            c.query_row(
                "SELECT * FROM backup_policies WHERE id=?1",
                [id],
                row_to_policy,
            )
            .map_err(Into::into)
        })
    }

    pub fn get_policy_by_uuid(&self, uuid: &str) -> Result<PolicyRecord> {
        self.with_conn(|c| {
            c.query_row(
                "SELECT * FROM backup_policies WHERE uuid=?1",
                [uuid],
                row_to_policy,
            )
            .map_err(Into::into)
        })
    }

    pub fn set_policy_enabled(&self, id: i64, enabled: bool) -> Result<()> {
        self.with_conn(|c| {
            c.execute(
                "UPDATE backup_policies SET enabled=?1, updated_at=?2 WHERE id=?3",
                params![enabled as i64, now_rfc3339(), id],
            )?;
            Ok(())
        })
    }

    pub fn set_policy_schedule(&self, id: i64, schedule_json: Option<&str>) -> Result<()> {
        self.with_conn(|c| {
            c.execute(
                "UPDATE backup_policies SET schedule_json=?1, updated_at=?2 WHERE id=?3",
                params![schedule_json, now_rfc3339(), id],
            )?;
            Ok(())
        })
    }

    pub fn delete_policy(&self, id: i64) -> Result<()> {
        self.with_conn(|c| {
            c.execute("DELETE FROM backup_policies WHERE id=?1", [id])?;
            Ok(())
        })
    }

    pub fn policy_destination_ids(&self, policy_id: i64) -> Result<Vec<i64>> {
        self.with_conn(|c| {
            let mut stmt =
                c.prepare("SELECT destination_id FROM policy_destinations WHERE policy_id=?1")?;
            let ids = stmt
                .query_map([policy_id], |r| r.get::<_, i64>(0))?
                .collect::<std::result::Result<Vec<_>, _>>()?;
            Ok(ids)
        })
    }

    // ---- Job hydration (worker --job-id) -------------------------------------------------

    /// Assemble a runnable [`JobSpec`] from a saved policy (Section 3/5). The worker calls this for
    /// `--job-id <policy_id>`, so a scheduled run uses the exact same engine path as a manual one.
    pub fn hydrate_policy_job(&self, policy_id: i64) -> Result<HydratedJob> {
        let policy = self.get_policy(policy_id)?;
        let conn = self.get_connection(policy.connection_id)?;
        let mut destinations = Vec::new();
        for did in self.policy_destination_ids(policy_id)? {
            let d = self.get_destination(did)?;
            if d.enabled {
                destinations.push(d.to_spec()?);
            }
        }
        let job = JobSpec {
            job_id: format!("policy:{}", policy.uuid),
            connection: conn.config.clone(),
            secret_reference: conn.secret_reference.clone(),
            destinations,
            trigger: TriggerTypeDefaulted(TriggerType::Scheduled),
        };
        let retention = policy
            .retention_policy_json
            .as_deref()
            .and_then(|j| serde_json::from_str::<crate::retention::RetentionPolicy>(j).ok());
        Ok(HydratedJob {
            job,
            connection_id: conn.id,
            policy_id,
            database_label: conn.config.name.clone(),
            retention,
        })
    }

    // ---- Backup history ------------------------------------------------------------------

    /// Record a completed run (and its per-destination outcomes) into history (Section 17/20).
    /// Returns the new `backup_runs.id`.
    pub fn record_run(
        &self,
        result: &RunResult,
        connection_id: Option<i64>,
        policy_id: Option<i64>,
        trigger: TriggerType,
        database_label: &str,
    ) -> Result<i64> {
        self.with_conn(|c| {
            c.execute(
                "INSERT INTO backup_runs
                 (uuid,policy_id,connection_id,trigger_type,database_label,started_at,completed_at,status,filename,local_temp_path,file_size,sha256,postgres_major_version,verification_status,error_code,error_message)
                 VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16)",
                params![
                    result.backup_id,
                    policy_id,
                    connection_id,
                    enum_str(&trigger),
                    database_label,
                    result.started_at,
                    result.completed_at,
                    enum_str(&result.state),
                    result.filename,
                    result.local_temp_path,
                    result.file_size.map(|v| v as i64),
                    result.sha256,
                    result.postgres_major_version,
                    enum_str(&result.verification),
                    result.error_code,
                    result.error_message,
                ],
            )?;
            let run_id = c.last_insert_rowid();
            for d in &result.destinations {
                c.execute(
                    "INSERT INTO backup_destination_runs
                     (backup_run_id,destination_id,destination_name,started_at,completed_at,status,stored_path,remote_file_id,bytes_written,sha256,error_code,error_message)
                     VALUES (?1,NULL,?2,?3,?3,?4,?5,NULL,?6,?7,?8,?9)",
                    params![
                        run_id,
                        d.name,
                        result.completed_at,
                        dest_status_str(d.status),
                        d.stored_path,
                        d.bytes_written.map(|v| v as i64),
                        result.sha256,
                        d.error_code,
                        d.error_message,
                    ],
                )?;
            }
            Ok(run_id)
        })
    }

    /// List history rows, newest first, optionally filtered by connection and/or status.
    pub fn list_runs(
        &self,
        connection_id: Option<i64>,
        status: Option<&str>,
        limit: i64,
    ) -> Result<Vec<RunSummary>> {
        self.with_conn(|c| {
            // Static SQL with NULL-guards so callers can omit filters without dynamic query building.
            let mut stmt = c.prepare(
                "SELECT * FROM backup_runs
                 WHERE (:conn IS NULL OR connection_id = :conn)
                   AND (:status IS NULL OR status = :status)
                 ORDER BY COALESCE(started_at,'') DESC
                 LIMIT :limit",
            )?;
            let rows = stmt
                .query_map(
                    rusqlite::named_params! {
                        ":conn": connection_id,
                        ":status": status,
                        ":limit": limit,
                    },
                    row_to_run_summary,
                )?
                .collect::<std::result::Result<Vec<_>, _>>()?;
            Ok(rows)
        })
    }

    pub fn get_run_by_uuid(&self, uuid: &str) -> Result<RunSummary> {
        self.with_conn(|c| {
            c.query_row(
                "SELECT * FROM backup_runs WHERE uuid=?1",
                [uuid],
                row_to_run_summary,
            )
            .map_err(Into::into)
        })
    }

    // ---- App settings --------------------------------------------------------------------

    pub fn get_setting(&self, key: &str) -> Result<Option<String>> {
        self.with_conn(|c| {
            c.query_row("SELECT value FROM app_settings WHERE key=?1", [key], |r| {
                r.get(0)
            })
            .optional()
            .map_err(Into::into)
        })
    }

    pub fn set_setting(&self, key: &str, value: &str) -> Result<()> {
        self.with_conn(|c| {
            c.execute(
                "INSERT INTO app_settings (key,value) VALUES (?1,?2)
                 ON CONFLICT(key) DO UPDATE SET value=excluded.value",
                params![key, value],
            )?;
            Ok(())
        })
    }

    pub fn delete_setting(&self, key: &str) -> Result<()> {
        self.with_conn(|c| {
            c.execute("DELETE FROM app_settings WHERE key=?1", [key])?;
            Ok(())
        })
    }
}

fn dest_status_str(s: DestinationStatus) -> &'static str {
    match s {
        DestinationStatus::Completed => "completed",
        DestinationStatus::Failed => "failed",
        DestinationStatus::Skipped => "skipped",
    }
}

fn row_to_connection(r: &Row) -> rusqlite::Result<ConnectionRecord> {
    let ssl: String = r.get("ssl_mode")?;
    Ok(ConnectionRecord {
        id: r.get("id")?,
        uuid: r.get("uuid")?,
        config: ConnectionConfig {
            name: r.get("name")?,
            host: r.get("host")?,
            port: r.get::<_, i64>("port")? as u16,
            database: r.get("database_name")?,
            username: r.get("username")?,
            ssl_mode: SslMode::parse(&ssl).unwrap_or_default(),
            postgres_major_version: r
                .get::<_, Option<i64>>("postgres_major_version")?
                .map(|v| v as u32),
        },
        secret_reference: r.get("secret_reference")?,
        created_at: r.get("created_at")?,
        updated_at: r.get("updated_at")?,
    })
}

fn row_to_destination(r: &Row) -> rusqlite::Result<DestinationRecord> {
    Ok(DestinationRecord {
        id: r.get("id")?,
        uuid: r.get("uuid")?,
        name: r.get("name")?,
        kind: r.get("type")?,
        configuration_json: r.get("configuration_json")?,
        secret_reference: r.get("secret_reference")?,
        enabled: r.get::<_, i64>("enabled")? != 0,
        created_at: r.get("created_at")?,
        updated_at: r.get("updated_at")?,
    })
}

fn row_to_policy(r: &Row) -> rusqlite::Result<PolicyRecord> {
    Ok(PolicyRecord {
        id: r.get("id")?,
        uuid: r.get("uuid")?,
        connection_id: r.get("connection_id")?,
        name: r.get("name")?,
        schedule_json: r.get("schedule_json")?,
        retention_policy_json: r.get("retention_policy_json")?,
        enabled: r.get::<_, i64>("enabled")? != 0,
        created_at: r.get("created_at")?,
        updated_at: r.get("updated_at")?,
    })
}

fn row_to_run_summary(r: &Row) -> rusqlite::Result<RunSummary> {
    Ok(RunSummary {
        id: r.get("id")?,
        uuid: r.get("uuid")?,
        connection_id: r.get("connection_id")?,
        policy_id: r.get("policy_id")?,
        trigger_type: r.get("trigger_type")?,
        database_label: r.get("database_label")?,
        started_at: r.get("started_at")?,
        completed_at: r.get("completed_at")?,
        status: r.get("status")?,
        filename: r.get("filename")?,
        file_size: r.get("file_size")?,
        sha256: r.get("sha256")?,
        verification_status: r.get("verification_status")?,
        error_code: r.get("error_code")?,
        error_message: r.get("error_message")?,
    })
}

/// A thin marker trait alias kept for readability at call sites and future generic code.
pub trait Repository {}
impl Repository for Database {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::credentials::MemoryStore;
    use crate::model::{BackupState, VerificationStatus};

    #[test]
    fn connection_import_keeps_password_out_of_sqlite() {
        let db = Database::open_in_memory().unwrap();
        let creds = MemoryStore::new();
        let rec = db
            .add_connection_from_uri(
                "Prod",
                "postgresql://postgres:sup3rsecret@db.example.com:5432/postgres",
                &creds,
            )
            .unwrap();

        assert_eq!(rec.config.host, "db.example.com");
        assert_eq!(rec.secret_reference, format!("conn:{}", rec.uuid));
        // The password is retrievable from the credential store...
        assert_eq!(creds.get(&rec.secret_reference).unwrap(), "sup3rsecret");

        // ...but appears nowhere in the SQLite row.
        db.with_conn(|c| {
            let dump: String = c
                .query_row("SELECT group_concat(name||':'||host||':'||username||':'||secret_reference) FROM connections", [], |r| r.get(0))
                .unwrap();
            assert!(!dump.contains("sup3rsecret"), "password leaked into SQLite: {dump}");
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn destination_roundtrip_and_spec() {
        let db = Database::open_in_memory().unwrap();
        let d = db
            .add_local_destination("Local HDD", r"D:\Backups")
            .unwrap();
        assert_eq!(db.list_destinations().unwrap().len(), 1);
        let spec = d.to_spec().unwrap();
        assert_eq!(spec.name, "Local HDD");
        match spec.kind {
            DestinationKind::Local { path } => assert_eq!(path, r"D:\Backups"),
            _ => panic!("expected a local destination"),
        }
    }

    #[test]
    fn replacing_google_client_requires_in_place_reconnect() {
        let db = Database::open_in_memory().unwrap();
        let destination = db.add_destination("Drive", "google_drive", &serde_json::json!({
            "account_email": "user@example.com", "folder_id": "folder", "client_id": "old-client"
        }).to_string(), Some("gdrive:existing")).unwrap();
        assert_eq!(
            db.mark_google_destinations_for_reauthorization("new-client")
                .unwrap(),
            1
        );
        let disabled = db.get_destination(destination.id).unwrap();
        assert!(!disabled.enabled);
        assert!(db.set_destination_enabled(destination.id, true).is_err());
        db.reconnect_google_destination(destination.id, &serde_json::json!({
            "account_email": "user@example.com", "folder_id": "folder", "client_id": "new-client", "reauthorization_required": false
        }).to_string()).unwrap();
        let connected = db.get_destination(destination.id).unwrap();
        assert_eq!(connected.uuid, destination.uuid);
        assert!(connected.enabled);
        assert_eq!(
            connected.secret_reference.as_deref(),
            Some("gdrive:existing")
        );
    }

    #[test]
    fn record_and_list_runs() {
        let db = Database::open_in_memory().unwrap();
        let mut result = RunResult::pending("bid-1", "job-1");
        result.state = BackupState::Completed;
        result.verification = VerificationStatus::Passed;
        result.file_size = Some(1234);
        result.sha256 = Some("abc".into());
        result.started_at = Some(now_rfc3339());
        result.completed_at = Some(now_rfc3339());
        result.destinations.push(crate::model::DestinationResult {
            name: "Local".into(),
            status: DestinationStatus::Completed,
            stored_path: Some(r"D:\Backups\x.dump".into()),
            bytes_written: Some(1234),
            error_code: None,
            error_message: None,
        });

        db.record_run(&result, None, None, TriggerType::Manual, "My DB")
            .unwrap();
        let runs = db.list_runs(None, None, 10).unwrap();
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].status, "COMPLETED");
        assert_eq!(runs[0].database_label.as_deref(), Some("My DB"));

        let completed = db.list_runs(None, Some("COMPLETED"), 10).unwrap();
        assert_eq!(completed.len(), 1);
        let failed = db.list_runs(None, Some("FAILED"), 10).unwrap();
        assert_eq!(failed.len(), 0);
    }

    #[test]
    fn settings_upsert() {
        let db = Database::open_in_memory().unwrap();
        assert!(db.get_setting("theme").unwrap().is_none());
        db.set_setting("theme", "dark").unwrap();
        db.set_setting("theme", "light").unwrap();
        assert_eq!(db.get_setting("theme").unwrap().as_deref(), Some("light"));
    }
}
