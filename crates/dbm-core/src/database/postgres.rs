//! PostgreSQL / Supabase [`DatabaseSource`] (Sections 6/7).
//!
//! Uses `tokio-postgres` with `native-tls` (SChannel on Windows, so the OS trust store validates
//! Supabase's certificate — no bundled root store to maintain). The password is held only in memory
//! and passed via the typed `Config` builder (no string escaping, no argv), never logged.

use crate::config::{ConnectionConfig, SslMode};
use crate::error::{BackupError, Result};
use crate::traits::{DatabaseInfo, DatabaseSource, ServerVersion};
use std::time::Duration;
use tokio_postgres::Client;

/// A live PostgreSQL source. Holds sanitized config plus the transient password.
pub struct PostgresSource {
    config: ConnectionConfig,
    password: String,
}

impl PostgresSource {
    pub fn new(config: ConnectionConfig, password: impl Into<String>) -> Self {
        PostgresSource {
            config,
            password: password.into(),
        }
    }

    /// Open a fresh connection, spawning the connection driver task. The returned client owns the
    /// session; dropping it ends the driver.
    async fn connect(&self) -> Result<Client> {
        let mut cfg = tokio_postgres::Config::new();
        cfg.host(&self.config.host)
            .port(self.config.port)
            .user(&self.config.username)
            .password(&self.password)
            .dbname(&self.config.database)
            .connect_timeout(Duration::from_secs(15))
            .ssl_mode(map_ssl(self.config.ssl_mode));

        let builder = native_tls::TlsConnector::builder();
        let connector = builder.build().map_err(|e| BackupError::Internal {
            detail: format!("tls init: {e}"),
        })?;
        let connector = postgres_native_tls::MakeTlsConnector::new(connector);

        let (client, connection) = cfg
            .connect(connector)
            .await
            .map_err(|e| self.map_connect_err(e))?;

        // Drive the connection in the background; it ends when `client` is dropped.
        tokio::spawn(async move {
            let _ = connection.await;
        });
        Ok(client)
    }

    fn map_connect_err(&self, e: tokio_postgres::Error) -> BackupError {
        // tokio-postgres's Display is only the top-level message ("error connecting to server");
        // the actionable cause (DNS failure, IPv6 unreachable, timeout, refused, TLS) lives in the
        // error's source chain, so we flatten the whole chain into the detail.
        let raw = err_chain(&e);
        let detail = crate::redact::redact_secret(&raw, &self.password);
        let lower = detail.to_ascii_lowercase();
        if lower.contains("password authentication failed")
            || lower.contains("role \"")
            || lower.contains("authentication")
        {
            BackupError::DatabaseAuthenticationFailed { detail }
        } else {
            BackupError::DatabaseUnavailable { detail }
        }
    }
}

/// Flatten an error and its `source()` chain into one string, e.g.
/// `error connecting to server: tcp connect error: No route to host (os error 10065)`.
fn err_chain(e: &dyn std::error::Error) -> String {
    let mut s = e.to_string();
    let mut src = e.source();
    while let Some(inner) = src {
        let msg = inner.to_string();
        if !s.contains(&msg) {
            s.push_str(": ");
            s.push_str(&msg);
        }
        src = inner.source();
    }
    s
}

impl DatabaseSource for PostgresSource {
    async fn test_connection(&self) -> Result<()> {
        let client = self.connect().await?;
        client
            .simple_query("SELECT 1")
            .await
            .map_err(|e| self.map_connect_err(e))?;
        Ok(())
    }

    async fn server_version(&self) -> Result<ServerVersion> {
        let client = self.connect().await?;
        let row = client
            .query_one("SHOW server_version_num", &[])
            .await
            .map_err(|e| self.map_connect_err(e))?;
        let num_str: String = row.get(0);
        let num: u32 = num_str.trim().parse().map_err(|_| BackupError::Internal {
            detail: format!("could not parse server_version_num='{num_str}'"),
        })?;

        let text = client
            .query_one("SHOW server_version", &[])
            .await
            .ok()
            .map(|r| r.get::<_, String>(0))
            .unwrap_or_default();

        Ok(ServerVersion {
            num,
            major: major_from_num(num),
            text,
        })
    }

    async fn database_info(&self) -> Result<DatabaseInfo> {
        let client = self.connect().await?;
        let name: String = client
            .query_one("SELECT current_database()", &[])
            .await
            .map_err(|e| self.map_connect_err(e))?
            .get(0);
        let size_bytes = client
            .query_one("SELECT pg_database_size(current_database())", &[])
            .await
            .ok()
            .map(|r| r.get::<_, i64>(0) as u64);
        Ok(DatabaseInfo { name, size_bytes })
    }
}

/// Map our [`SslMode`] to tokio-postgres's coarser SSL mode. verify-ca / verify-full are enforced
/// by the TLS connector (which validates by default), so they map to `Require` at the protocol level.
fn map_ssl(mode: SslMode) -> tokio_postgres::config::SslMode {
    use tokio_postgres::config::SslMode as Pg;
    match mode {
        SslMode::Disable => Pg::Disable,
        SslMode::Prefer => Pg::Prefer,
        SslMode::Require | SslMode::VerifyCa | SslMode::VerifyFull => Pg::Require,
    }
}

/// Extract the major version from `server_version_num` (Section 7).
/// PG10+: `num / 10000` (e.g. 170002 -> 17, 150005 -> 15).
/// PG<10: `num / 10000` still yields 9 for 9.x families (e.g. 90603 -> 9); acceptable — such
/// versions are rejected later by [`crate::docker::image_for_major`].
pub fn major_from_num(num: u32) -> u32 {
    num / 10000
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_major_version_from_num() {
        assert_eq!(major_from_num(170002), 17);
        assert_eq!(major_from_num(160003), 16);
        assert_eq!(major_from_num(150005), 15);
        assert_eq!(major_from_num(120018), 12);
    }
}
