//! Connection configuration + URI parsing/sanitization (Section 6, Section 28).
//!
//! Core invariant: a [`ConnectionConfig`] never holds a password. When a user pastes a full
//! `postgresql://user:pass@host/db` URI, [`parse_connection_uri`] splits it into a *sanitized*
//! config and the extracted secret, which the caller then hands to a `CredentialStore`. The raw
//! URI-with-password is never persisted or logged.

use crate::error::{BackupError, Result};
use serde::{Deserialize, Serialize};

/// PostgreSQL SSL mode. Supabase requires SSL, so we default to `Require`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SslMode {
    Disable,
    Prefer,
    /// Default: Supabase requires SSL.
    #[default]
    Require,
    VerifyCa,
    VerifyFull,
}

impl SslMode {
    /// The value libpq expects in `PGSSLMODE` / `sslmode=`.
    pub fn as_libpq(&self) -> &'static str {
        match self {
            SslMode::Disable => "disable",
            SslMode::Prefer => "prefer",
            SslMode::Require => "require",
            SslMode::VerifyCa => "verify-ca",
            SslMode::VerifyFull => "verify-full",
        }
    }

    pub fn parse(s: &str) -> Option<SslMode> {
        match s.trim().to_ascii_lowercase().as_str() {
            "disable" => Some(SslMode::Disable),
            "prefer" | "allow" => Some(SslMode::Prefer),
            "require" => Some(SslMode::Require),
            "verify-ca" => Some(SslMode::VerifyCa),
            "verify-full" => Some(SslMode::VerifyFull),
            _ => None,
        }
    }
}

/// Sanitized connection metadata — safe to persist in SQLite and to log. Never contains a password.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConnectionConfig {
    /// Friendly display name, e.g. "Glasafe Production".
    pub name: String,
    pub host: String,
    pub port: u16,
    pub database: String,
    pub username: String,
    #[serde(default)]
    pub ssl_mode: SslMode,
    /// Discovered PostgreSQL major version (e.g. 17), when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub postgres_major_version: Option<u32>,
}

impl ConnectionConfig {
    /// A filesystem-safe slug of the display name, used in backup filenames (Section 11).
    pub fn slug(&self) -> String {
        slugify(&self.name)
    }

    /// A sanitized `postgresql://` URI **without** password — safe to display/log.
    pub fn sanitized_uri(&self) -> String {
        format!(
            "postgresql://{user}@{host}:{port}/{db}?sslmode={ssl}",
            user = self.username,
            host = self.host,
            port = self.port,
            db = self.database,
            ssl = self.ssl_mode.as_libpq(),
        )
    }
}

/// The result of parsing a user-supplied connection URI: sanitized config + extracted password.
#[derive(Debug, Clone)]
pub struct ParsedConnection {
    pub config: ConnectionConfig,
    /// The password extracted from the URI, if any. The caller must store this securely and drop it.
    pub password: Option<String>,
}

/// Parse a `postgresql://[user[:password]@]host[:port]/database[?sslmode=...]` URI.
///
/// Returns a sanitized [`ConnectionConfig`] plus the extracted password (never retained here).
/// `name` is the caller-provided display name (the URI does not carry one).
pub fn parse_connection_uri(name: &str, uri: &str) -> Result<ParsedConnection> {
    let uri = uri.trim();
    let parsed = url::Url::parse(uri).map_err(|e| BackupError::InvalidConnectionString {
        detail: e.to_string(),
    })?;

    match parsed.scheme() {
        "postgres" | "postgresql" => {}
        other => {
            return Err(BackupError::InvalidConnectionString {
                detail: format!("unsupported scheme '{other}' (expected postgres/postgresql)"),
            });
        }
    }

    let host = parsed
        .host_str()
        .ok_or_else(|| BackupError::InvalidConnectionString {
            detail: "missing host".into(),
        })?
        .to_string();

    let port = parsed.port().unwrap_or(5432);

    let username = {
        let u = parsed.username();
        if u.is_empty() {
            "postgres".to_string()
        } else {
            percent_decode(u)
        }
    };

    let password = parsed
        .password()
        .map(percent_decode)
        .filter(|p| !p.is_empty());

    // database = first path segment.
    let database = parsed
        .path()
        .trim_start_matches('/')
        .split('/')
        .next()
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .unwrap_or_else(|| "postgres".to_string());

    // sslmode from query, else default (Require).
    let ssl_mode = parsed
        .query_pairs()
        .find(|(k, _)| k.eq_ignore_ascii_case("sslmode"))
        .and_then(|(_, v)| SslMode::parse(&v))
        .unwrap_or_default();

    let name = if name.trim().is_empty() {
        database.clone()
    } else {
        name.trim().to_string()
    };

    Ok(ParsedConnection {
        config: ConnectionConfig {
            name,
            host,
            port,
            database,
            username,
            ssl_mode,
            postgres_major_version: None,
        },
        password,
    })
}

/// Minimal percent-decoding for userinfo (the `url` crate leaves userinfo percent-encoded).
fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hi = (bytes[i + 1] as char).to_digit(16);
            let lo = (bytes[i + 2] as char).to_digit(16);
            if let (Some(h), Some(l)) = (hi, lo) {
                out.push((h * 16 + l) as u8);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Produce a filesystem/URL-safe lowercase slug: alphanumerics kept, runs of others become `-`.
pub fn slugify(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut prev_dash = false;
    for c in input.trim().chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
            prev_dash = false;
        } else if !prev_dash {
            out.push('-');
            prev_dash = true;
        }
    }
    let trimmed = out.trim_matches('-').to_string();
    if trimmed.is_empty() {
        "database".to_string()
    } else {
        trimmed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_full_uri_and_extracts_password() {
        let p = parse_connection_uri(
            "Glasafe Production",
            "postgresql://postgres:s3cret@db.abc.supabase.co:5432/postgres?sslmode=require",
        )
        .unwrap();
        assert_eq!(p.config.name, "Glasafe Production");
        assert_eq!(p.config.host, "db.abc.supabase.co");
        assert_eq!(p.config.port, 5432);
        assert_eq!(p.config.database, "postgres");
        assert_eq!(p.config.username, "postgres");
        assert_eq!(p.config.ssl_mode, SslMode::Require);
        assert_eq!(p.password.as_deref(), Some("s3cret"));
    }

    #[test]
    fn sanitized_config_never_contains_password() {
        let p = parse_connection_uri("x", "postgres://u:verysecret@h:6543/d").unwrap();
        // The config serialized must not contain the password.
        let json = serde_json::to_string(&p.config).unwrap();
        assert!(
            !json.contains("verysecret"),
            "config leaked password: {json}"
        );
        assert!(!p.config.sanitized_uri().contains("verysecret"));
    }

    #[test]
    fn defaults_port_user_db_and_sslmode() {
        let p = parse_connection_uri("", "postgresql://host.example.com").unwrap();
        assert_eq!(p.config.port, 5432);
        assert_eq!(p.config.username, "postgres");
        assert_eq!(p.config.database, "postgres");
        assert_eq!(p.config.ssl_mode, SslMode::Require);
        // name falls back to database when blank.
        assert_eq!(p.config.name, "postgres");
        assert!(p.password.is_none());
    }

    #[test]
    fn decodes_percent_encoded_password() {
        let p = parse_connection_uri("x", "postgres://user:p%40ss%3Aword@h/d").unwrap();
        assert_eq!(p.password.as_deref(), Some("p@ss:word"));
    }

    #[test]
    fn rejects_non_postgres_scheme() {
        let e = parse_connection_uri("x", "mysql://u:p@h/d").unwrap_err();
        assert_eq!(e.code(), "InvalidConnectionString");
    }

    #[test]
    fn slug_is_filesystem_safe() {
        assert_eq!(slugify("Glasafe Production"), "glasafe-production");
        assert_eq!(slugify("  CRM / Sales!! "), "crm-sales");
        assert_eq!(slugify("///"), "database");
    }

    #[test]
    fn pooler_port_6543_parsed() {
        let p = parse_connection_uri(
            "pool",
            "postgres://u:pw@aws.pooler.supabase.com:6543/postgres",
        )
        .unwrap();
        assert_eq!(p.config.port, 6543);
    }
}
