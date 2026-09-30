//! Running pg tooling inside ephemeral Postgres containers (Section 8/12).
//!
//! Security choices (Section 28):
//! - Arguments are passed as a structured vector — never a shell string (#7/#8).
//! - The password is written to a temporary **env-file** (restricted permissions) and passed with
//!   `--env-file`, then deleted immediately after the run — never placed in argv (#15).
//! - Containers run `--rm`, publish **no ports**, and use a deterministic `--name` so an abandoned
//!   one can be force-removed (#12/#13).

use crate::config::SslMode;
use crate::docker::client::{run_docker, CmdOutput};
use crate::error::{BackupError, Result};
use std::path::{Path, PathBuf};
use std::time::Duration;

/// Connection + runtime options shared by dump/restore/psql.
#[derive(Debug, Clone)]
pub struct PgConnOpts {
    pub image: String,
    pub host: String,
    pub port: u16,
    pub user: String,
    pub database: String,
    pub ssl_mode: SslMode,
    pub password: String,
    /// Optional docker network to attach to (used in tests to reach a sibling container by name).
    pub network: Option<String>,
    pub timeout_secs: u64,
}

/// A temporary env-file holding PG* secrets, removed on drop.
struct EnvFile {
    path: PathBuf,
}

impl EnvFile {
    fn write(dir: &Path, opts: &PgConnOpts) -> Result<EnvFile> {
        let path = dir.join(format!(".pgenv-{}", uuid::Uuid::new_v4()));
        let contents = format!(
            "PGPASSWORD={}\nPGSSLMODE={}\nPGCONNECT_TIMEOUT=15\n",
            opts.password,
            opts.ssl_mode.as_libpq()
        );
        std::fs::write(&path, contents)?;
        restrict_file(&path);
        Ok(EnvFile { path })
    }
}

impl Drop for EnvFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

#[cfg(windows)]
fn restrict_file(_path: &Path) {
    // The containing run dir already has a restricted ACL (see config::paths); the file inherits it.
}

#[cfg(not(windows))]
fn restrict_file(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    if let Ok(meta) = std::fs::metadata(path) {
        let mut perms = meta.permissions();
        perms.set_mode(0o600);
        let _ = std::fs::set_permissions(path, perms);
    }
}

/// Format a host path as a Docker bind-mount source (Docker Desktop accepts forward slashes).
fn mount_source(dir: &Path) -> String {
    dir.to_string_lossy().replace('\\', "/")
}

/// Rewrite loopback hosts so a container can reach a server running on the Windows host.
fn resolve_host(host: &str, network: &Option<String>) -> String {
    if network.is_none() && (host == "localhost" || host == "127.0.0.1" || host == "::1") {
        "host.docker.internal".to_string()
    } else {
        host.to_string()
    }
}

fn base_run_args(
    name: &str,
    envfile: &Path,
    mount: Option<(&Path,)>,
    opts: &PgConnOpts,
) -> Vec<String> {
    let mut args: Vec<String> = vec![
        "run".into(),
        "--rm".into(),
        "--name".into(),
        name.into(),
        "--env-file".into(),
        envfile.to_string_lossy().to_string(),
    ];
    if let Some(net) = &opts.network {
        args.push("--network".into());
        args.push(net.clone());
    } else {
        // Allow reaching the host loopback when needed.
        args.push("--add-host".into());
        args.push("host.docker.internal:host-gateway".into());
    }
    if let Some((dir,)) = mount {
        args.push("-v".into());
        args.push(format!("{}:/out", mount_source(dir)));
    }
    args.push(opts.image.clone());
    args
}

/// Force-remove a container by name (best-effort cleanup of an abandoned run).
pub async fn remove_container(name: &str) {
    let _ = run_docker(
        &["rm".into(), "-f".into(), name.into()],
        Duration::from_secs(30),
        None,
    )
    .await;
}

/// Run `pg_dump -Fc` producing `output_host_path`.
pub async fn run_pg_dump(
    opts: &PgConnOpts,
    container_name: &str,
    output_host_path: &Path,
) -> Result<()> {
    let dir = output_host_path
        .parent()
        .ok_or_else(|| BackupError::InvalidPath {
            detail: "dump output has no parent dir".into(),
        })?;
    let filename = output_host_path
        .file_name()
        .ok_or_else(|| BackupError::InvalidPath {
            detail: "dump output has no filename".into(),
        })?
        .to_string_lossy()
        .to_string();

    let envfile = EnvFile::write(dir, opts)?;
    let host = resolve_host(&opts.host, &opts.network);

    let mut args = base_run_args(container_name, &envfile.path, Some((dir,)), opts);
    args.extend([
        "pg_dump".into(),
        "-Fc".into(),
        "-h".into(),
        host,
        "-p".into(),
        opts.port.to_string(),
        "-U".into(),
        opts.user.clone(),
        "-d".into(),
        opts.database.clone(),
        "-w".into(), // never prompt for a password
        "-f".into(),
        format!("/out/{filename}"),
    ]);

    let out = run_docker(
        &args,
        Duration::from_secs(opts.timeout_secs),
        Some(&opts.password),
    )
    .await;
    // Ensure the container is gone even if the run errored/timed out.
    remove_container(container_name).await;
    let out = out?;

    if !out.success() {
        return Err(map_pg_failure(&out, "pg_dump"));
    }
    Ok(())
}

/// Run `pg_restore` of `dump_host_path` into `opts.database`. `create` uses `-C` to recreate the DB;
/// `clean` uses `--clean --if-exists` to drop objects before recreating them (for overwriting a
/// populated target).
pub async fn run_pg_restore(
    opts: &PgConnOpts,
    container_name: &str,
    dump_host_path: &Path,
    create: bool,
    clean: bool,
) -> Result<()> {
    run_pg_restore_impl(opts, container_name, dump_host_path, create, clean, false, None, None)
        .await
        .map(|_| ())
}

/// Restore an archive for the read-only backup browser. PostgreSQL's `pg_restore` continues after
/// individual SQL errors and exits with status 1; accept that only when its final summary confirms
/// it ignored errors, so the browser can still expose tables that were restored successfully.
pub async fn run_pg_restore_for_browse(
    opts: &PgConnOpts,
    container_name: &str,
    dump_host_path: &Path,
    schema: Option<&str>,
    table: Option<&str>,
) -> Result<Option<String>> {
    run_pg_restore_impl(
        opts,
        container_name,
        dump_host_path,
        false,
        false,
        true,
        schema,
        table,
    )
    .await
}

async fn run_pg_restore_impl(
    opts: &PgConnOpts,
    container_name: &str,
    dump_host_path: &Path,
    create: bool,
    clean: bool,
    allow_ignored_errors: bool,
    schema: Option<&str>,
    table: Option<&str>,
) -> Result<Option<String>> {
    let dir = dump_host_path
        .parent()
        .ok_or_else(|| BackupError::InvalidPath {
            detail: "dump has no parent dir".into(),
        })?;
    let filename = dump_host_path
        .file_name()
        .ok_or_else(|| BackupError::InvalidPath {
            detail: "dump has no filename".into(),
        })?
        .to_string_lossy()
        .to_string();

    let envfile = EnvFile::write(dir, opts)?;
    let host = resolve_host(&opts.host, &opts.network);

    let mut args = base_run_args(container_name, &envfile.path, Some((dir,)), opts);
    args.extend([
        "pg_restore".into(),
        "-h".into(),
        host,
        "-p".into(),
        opts.port.to_string(),
        "-U".into(),
        opts.user.clone(),
        "-d".into(),
        opts.database.clone(),
        "-w".into(),
        "--no-owner".into(),
        "--no-privileges".into(),
    ]);
    if create {
        args.push("-C".into());
    }
    if clean {
        args.push("--clean".into());
        args.push("--if-exists".into());
    }
    if let (Some(schema), Some(table)) = (schema, table) {
        args.push("--data-only".into());
        args.push("--disable-triggers".into());
        args.push("--schema".into());
        args.push(schema.to_string());
        args.push("--table".into());
        args.push(table.to_string());
    } else if schema == Some("pre-data") {
        args.push("--section=pre-data".into());
    }
    args.push(format!("/out/{filename}"));

    let out = run_docker(
        &args,
        Duration::from_secs(opts.timeout_secs),
        Some(&opts.password),
    )
    .await;
    remove_container(container_name).await;
    let out = out?;

    if !out.success() && allow_ignored_errors && ignored_restore_error_summary(&out.stderr) {
        return Ok(Some(out.stderr.trim().to_string()));
    }
    if !out.success() {
        return Err(BackupError::PgRestoreFailed {
            detail: out.stderr.trim().to_string(),
        });
    }
    Ok(None)
}

fn ignored_restore_error_summary(stderr: &str) -> bool {
    stderr.lines().any(|line| {
        let Some((_, count)) = line.split_once("errors ignored on restore:") else {
            return false;
        };
        count.trim().parse::<u32>().is_ok_and(|n| n > 0)
    })
}

/// List a dump's table of contents (`pg_restore -l`). No server or database is touched — this reads
/// the archive file only, so no connection options are needed.
pub async fn list_archive(image: &str, dump_host_path: &Path, timeout_secs: u64) -> Result<String> {
    let dir = dump_host_path
        .parent()
        .ok_or_else(|| BackupError::InvalidPath {
            detail: "dump has no parent dir".into(),
        })?;
    let filename = dump_host_path
        .file_name()
        .ok_or_else(|| BackupError::InvalidPath {
            detail: "dump has no filename".into(),
        })?
        .to_string_lossy()
        .to_string();

    let name = format!("dbm-inspect-{}", uuid::Uuid::new_v4());
    let args: Vec<String> = vec![
        "run".into(),
        "--rm".into(),
        "--name".into(),
        name.clone(),
        "-v".into(),
        format!("{}:/out", mount_source(dir)),
        image.to_string(),
        "pg_restore".into(),
        "-l".into(),
        format!("/out/{filename}"),
    ];
    let out = run_docker(&args, Duration::from_secs(timeout_secs), None).await;
    remove_container(&name).await;
    let out = out?;
    if !out.success() {
        return Err(BackupError::BackupVerificationFailed {
            detail: out.stderr.trim().to_string(),
        });
    }
    Ok(out.stdout)
}

/// Run a single SQL statement via `psql -tAc` and return trimmed stdout. Used by integration tests
/// (seeding, row counts, creating a target database).
pub async fn run_psql(opts: &PgConnOpts, container_name: &str, sql: &str) -> Result<String> {
    // psql needs a writable dir only for the env-file; reuse the system temp dir.
    let tmp = std::env::temp_dir();
    let envfile = EnvFile::write(&tmp, opts)?;
    let host = resolve_host(&opts.host, &opts.network);

    let mut args = base_run_args(container_name, &envfile.path, None, opts);
    args.extend([
        "psql".into(),
        "-h".into(),
        host,
        "-p".into(),
        opts.port.to_string(),
        "-U".into(),
        opts.user.clone(),
        "-d".into(),
        opts.database.clone(),
        "-w".into(),
        "-tAc".into(),
        sql.to_string(),
    ]);

    let out = run_docker(
        &args,
        Duration::from_secs(opts.timeout_secs),
        Some(&opts.password),
    )
    .await;
    remove_container(container_name).await;
    let out = out?;
    if !out.success() {
        return Err(BackupError::Internal {
            detail: format!("psql failed: {}", out.stderr.trim()),
        });
    }
    Ok(out.stdout.trim().to_string())
}

/// Map a failed pg_dump/pg_restore output to a typed error, inspecting stderr for auth vs. network.
fn map_pg_failure(out: &CmdOutput, tool: &str) -> BackupError {
    let stderr = out.stderr.to_ascii_lowercase();
    if stderr.contains("password authentication failed")
        || stderr.contains("role")
        || stderr.contains("authentication")
    {
        BackupError::DatabaseAuthenticationFailed {
            detail: out.stderr.trim().to_string(),
        }
    } else if stderr.contains("could not connect")
        || stderr.contains("could not translate host")
        || stderr.contains("timeout")
        || stderr.contains("no route")
        || stderr.contains("connection refused")
    {
        BackupError::DatabaseUnavailable {
            detail: out.stderr.trim().to_string(),
        }
    } else if tool == "pg_dump" {
        BackupError::PgDumpFailed {
            detail: out.stderr.trim().to_string(),
        }
    } else {
        BackupError::PgRestoreFailed {
            detail: out.stderr.trim().to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loopback_hosts_are_rewritten_without_network() {
        assert_eq!(resolve_host("localhost", &None), "host.docker.internal");
        assert_eq!(resolve_host("127.0.0.1", &None), "host.docker.internal");
        assert_eq!(resolve_host("db.example.com", &None), "db.example.com");
    }

    #[test]
    fn loopback_kept_when_on_a_network() {
        assert_eq!(resolve_host("localhost", &Some("net".into())), "localhost");
    }

    #[test]
    fn mount_source_uses_forward_slashes() {
        let p = Path::new(r"C:\ProgramData\DatabaseBackupManager\work\abc");
        assert_eq!(
            mount_source(p),
            "C:/ProgramData/DatabaseBackupManager/work/abc"
        );
    }
}
