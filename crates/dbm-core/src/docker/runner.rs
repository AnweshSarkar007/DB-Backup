//! [`BackupRunner`] implementation backed by pg tooling in Docker containers.

use crate::docker::container_runner::{self, PgConnOpts};
use crate::error::{BackupError, Result};
use crate::traits::{ArchiveEntry, BackupRunner, DumpOutput, DumpParams};
use std::path::Path;

/// Runs pg_dump / pg_restore / pg_restore -l inside a `postgres:<major>` container.
#[derive(Debug, Clone)]
pub struct DockerPgRunner {
    /// The image used for verify/inspect (must be >= the archive's server version).
    image: String,
    timeout_secs: u64,
}

impl DockerPgRunner {
    pub fn new(image: impl Into<String>, timeout_secs: u64) -> Self {
        DockerPgRunner {
            image: image.into(),
            timeout_secs,
        }
    }
}

impl BackupRunner for DockerPgRunner {
    async fn dump(&self, params: &DumpParams) -> Result<DumpOutput> {
        let opts = PgConnOpts {
            image: self.image.clone(),
            host: params.connection.host.clone(),
            port: params.connection.port,
            user: params.connection.username.clone(),
            database: params.connection.database.clone(),
            ssl_mode: params.connection.ssl_mode,
            password: params.password.clone(),
            network: params.network.clone(),
            timeout_secs: params.timeout_secs,
        };
        let name = format!("dbm-dump-{}", uuid::Uuid::new_v4());
        container_runner::run_pg_dump(&opts, &name, &params.output_path).await?;

        let size = tokio::fs::metadata(&params.output_path).await?.len();
        if size == 0 {
            return Err(BackupError::PgDumpFailed {
                detail: "dump file is empty".into(),
            });
        }
        Ok(DumpOutput { size_bytes: size })
    }

    async fn verify(&self, dump_path: &Path) -> Result<()> {
        let toc = container_runner::list_archive(&self.image, dump_path, self.timeout_secs).await?;
        // A readable archive lists at least one line. An unreadable/corrupt archive makes
        // `pg_restore -l` exit non-zero, which list_archive already turns into an error.
        if toc.trim().is_empty() {
            return Err(BackupError::BackupVerificationFailed {
                detail: "archive table of contents was empty".into(),
            });
        }
        Ok(())
    }

    async fn inspect(&self, dump_path: &Path) -> Result<Vec<ArchiveEntry>> {
        let toc = container_runner::list_archive(&self.image, dump_path, self.timeout_secs).await?;
        Ok(parse_toc(&toc))
    }
}

/// Parse `pg_restore -l` output into archive entries (best-effort).
///
/// Data lines look like: `216; 1259 16385 TABLE public customers postgres`
/// i.e. `<id>; <catalogId> <oid> <DESC...> <schema> <name> <owner>`. The DESC can be multiple words
/// (e.g. `FK CONSTRAINT`, `MATERIALIZED VIEW`), so we treat the last token as owner, the two before
/// it as name and schema, and everything between the oid and schema as the object type.
pub fn parse_toc(toc: &str) -> Vec<ArchiveEntry> {
    let mut entries = Vec::new();
    for line in toc.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with(';') {
            continue;
        }
        let Some((_id, rest)) = line.split_once(';') else {
            continue;
        };
        let tokens: Vec<&str> = rest.split_whitespace().collect();
        // Need at least: catalogId oid DESC schema name owner (6 tokens).
        if tokens.len() < 6 {
            continue;
        }
        let owner_idx = tokens.len() - 1;
        let name = tokens[owner_idx - 1].to_string();
        let schema = tokens[owner_idx - 2].to_string();
        // object type = tokens[2 .. owner_idx-2]
        let object_type = tokens[2..owner_idx - 2].join(" ");
        if object_type.is_empty() {
            continue;
        }
        entries.push(ArchiveEntry {
            object_type,
            schema,
            name,
        });
    }
    entries
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_typical_toc_lines() {
        let toc = "\
;
; Archive created at 2026-09-30 11:35:24
;     dbname: appdb
;
216; 1259 16385 TABLE public customers postgres
217; 1259 16390 SEQUENCE public customers_id_seq postgres
230; 2606 16400 CONSTRAINT public customers customers_pkey postgres
240; 1259 16420 INDEX public idx_orders_customer postgres
";
        let e = parse_toc(toc);
        // The customers table and sequence should be present.
        assert!(e
            .iter()
            .any(|x| x.object_type == "TABLE" && x.name == "customers"));
        assert!(e
            .iter()
            .any(|x| x.object_type == "SEQUENCE" && x.name == "customers_id_seq"));
        assert!(e.iter().any(|x| x.object_type == "INDEX"));
    }

    #[test]
    fn skips_comment_and_blank_lines() {
        let toc = ";\n; comment\n\n";
        assert!(parse_toc(toc).is_empty());
    }
}
