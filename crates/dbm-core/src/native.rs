//! Native PostgreSQL client-tool execution and native-first runner selection.
//!
//! The bundled tools are resolved from the installed executable's resource layout, never from the
//! working directory or PATH. Docker is an explicit fallback selected before an operation starts;
//! a running operation never changes runner.

use crate::docker::DockerPgRunner;
use crate::error::{BackupError, Result};
use crate::traits::{ArchiveEntry, BackupRunner, DumpOutput, DumpParams};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;
use tokio::io::AsyncReadExt;
use tokio::process::Command;

const RESOURCE_RELATIVE_BIN: &str = "resources/postgresql/17/bin";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NativePgToolPaths {
    pub bin_dir: PathBuf,
}

impl NativePgToolPaths {
    pub fn from_bin_dir(bin_dir: PathBuf) -> Self {
        Self { bin_dir }
    }

    pub fn discover_from_executable(executable: PathBuf) -> Self {
        let install_dir = executable.parent().unwrap_or_else(|| Path::new("."));
        Self::from_bin_dir(install_dir.join(RESOURCE_RELATIVE_BIN))
    }

    pub fn discover() -> Self {
        std::env::var_os("DBM_PG_TOOLS_DIR")
            .map(PathBuf::from)
            .map(Self::from_bin_dir)
            .unwrap_or_else(|| {
                std::env::current_exe()
                    .map(Self::discover_from_executable)
                    .unwrap_or_else(|_| Self::from_bin_dir(PathBuf::from(RESOURCE_RELATIVE_BIN)))
            })
    }

    fn tool(&self, name: &str) -> PathBuf {
        let exe = self.bin_dir.join(format!("{name}.exe"));
        if exe.is_file() {
            exe
        } else {
            self.bin_dir.join(name)
        }
    }

    pub fn is_available(&self) -> bool {
        ["pg_dump", "pg_restore", "psql"]
            .iter()
            .all(|tool| self.tool(tool).is_file())
    }

    pub fn pg_dump(&self) -> PathBuf {
        self.tool("pg_dump")
    }
    pub fn pg_restore(&self) -> PathBuf {
        self.tool("pg_restore")
    }
    pub fn psql(&self) -> PathBuf {
        self.tool("psql")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunnerKind {
    Native,
    Docker,
}

pub fn select_runner(paths: &NativePgToolPaths, _server_major: u32) -> RunnerKind {
    if paths.is_available() {
        RunnerKind::Native
    } else {
        RunnerKind::Docker
    }
}

#[derive(Debug, Clone)]
pub enum SelectedRunner {
    Native(NativePgRunner),
    Docker(DockerPgRunner),
}

impl SelectedRunner {
    pub fn select(server_major: u32, timeout_secs: u64) -> Self {
        let paths = NativePgToolPaths::discover();
        match select_runner(&paths, server_major) {
            RunnerKind::Native => Self::Native(NativePgRunner::new(paths, timeout_secs)),
            RunnerKind::Docker => Self::Docker(DockerPgRunner::new(
                format!("postgres:{server_major}"),
                timeout_secs,
            )),
        }
    }

    pub fn kind(&self) -> RunnerKind {
        match self {
            Self::Native(_) => RunnerKind::Native,
            Self::Docker(_) => RunnerKind::Docker,
        }
    }

    pub async fn restore(&self, dump: &Path, params: &crate::restore::RestoreParams) -> Result<()> {
        match self {
            Self::Native(r) => r.restore(dump, params).await,
            Self::Docker(_) => {
                let opts = crate::docker::container_runner::PgConnOpts {
                    image: params.image.clone(),
                    host: params.target.host.clone(),
                    port: params.target.port,
                    user: params.target.username.clone(),
                    database: params.target.database.clone(),
                    ssl_mode: params.target.ssl_mode,
                    password: params.password.clone(),
                    network: None,
                    timeout_secs: 3600,
                };
                let name = format!("dbm-restore-{}", uuid::Uuid::new_v4());
                crate::docker::container_runner::run_pg_restore(
                    &opts,
                    &name,
                    dump,
                    false,
                    params.clean,
                )
                .await
            }
        }
    }
}

impl BackupRunner for SelectedRunner {
    async fn dump(&self, params: &DumpParams) -> Result<DumpOutput> {
        match self {
            Self::Native(r) => r.dump(params).await,
            Self::Docker(r) => r.dump(params).await,
        }
    }
    async fn verify(&self, path: &Path) -> Result<()> {
        match self {
            Self::Native(r) => r.verify(path).await,
            Self::Docker(r) => r.verify(path).await,
        }
    }
    async fn inspect(&self, path: &Path) -> Result<Vec<ArchiveEntry>> {
        match self {
            Self::Native(r) => r.inspect(path).await,
            Self::Docker(r) => r.inspect(path).await,
        }
    }
}

#[derive(Debug, Clone)]
pub struct NativePgRunner {
    paths: NativePgToolPaths,
    timeout_secs: u64,
}

impl NativePgRunner {
    pub fn new(paths: NativePgToolPaths, timeout_secs: u64) -> Self {
        Self {
            paths,
            timeout_secs,
        }
    }

    async fn run(&self, tool: &Path, args: &[String], password: Option<&str>) -> Result<CmdOutput> {
        if !tool.is_absolute() {
            return Err(BackupError::InvalidPath {
                detail: "native PostgreSQL tool path must be absolute".into(),
            });
        }
        let mut cmd = Command::new(tool);
        #[cfg(windows)]
        cmd.creation_flags(0x0800_0000);
        cmd.args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        if let Some(password) = password {
            cmd.env("PGPASSWORD", password);
        }
        let mut child = cmd.spawn().map_err(|e| BackupError::Io {
            detail: format!("failed to spawn {}: {e}", tool.display()),
        })?;
        let mut stdout_pipe = child.stdout.take();
        let mut stderr_pipe = child.stderr.take();
        let mut stdout = String::new();
        let mut stderr = String::new();
        let wait = async {
            let out = async {
                if let Some(pipe) = stdout_pipe.as_mut() {
                    let _ = pipe.read_to_string(&mut stdout).await;
                }
            };
            let err = async {
                if let Some(pipe) = stderr_pipe.as_mut() {
                    let _ = pipe.read_to_string(&mut stderr).await;
                }
            };
            tokio::join!(child.wait(), out, err).0
        };
        let status = tokio::time::timeout(Duration::from_secs(self.timeout_secs), wait)
            .await
            .map_err(|_| BackupError::Timeout {
                seconds: self.timeout_secs,
            })?
            .map_err(|e| BackupError::Io {
                detail: format!("native PostgreSQL tool wait: {e}"),
            })?;
        let scrub = |s: String| match password {
            Some(secret) => crate::redact::redact_secret(&s, secret),
            None => crate::redact::redact_env_assignments(&s),
        };
        Ok(CmdOutput {
            code: status.code(),
            stdout: scrub(stdout),
            stderr: scrub(stderr),
        })
    }
}

#[derive(Debug, Clone)]
struct CmdOutput {
    code: Option<i32>,
    stdout: String,
    stderr: String,
}

impl BackupRunner for NativePgRunner {
    async fn dump(&self, params: &DumpParams) -> Result<DumpOutput> {
        let output = absolute_path(&params.output_path)?;
        let parent = output.parent().ok_or_else(|| BackupError::InvalidPath {
            detail: "dump output has no parent dir".into(),
        })?;
        tokio::fs::create_dir_all(parent).await?;
        let args = vec![
            "-Fc".into(),
            "-h".into(),
            params.connection.host.clone(),
            "-p".into(),
            params.connection.port.to_string(),
            "-U".into(),
            params.connection.username.clone(),
            "-d".into(),
            params.connection.database.clone(),
            "-w".into(),
            "-f".into(),
            output.to_string_lossy().into_owned(),
        ];
        let out = self
            .run(&self.paths.pg_dump(), &args, Some(&params.password))
            .await?;
        if !out.code.is_some_and(|code| code == 0) {
            return Err(BackupError::PgDumpFailed { detail: out.stderr });
        }
        let size = tokio::fs::metadata(output).await?.len();
        if size == 0 {
            return Err(BackupError::PgDumpFailed {
                detail: "dump file is empty".into(),
            });
        }
        Ok(DumpOutput { size_bytes: size })
    }

    async fn verify(&self, dump_path: &Path) -> Result<()> {
        let path = absolute_path(dump_path)?;
        let out = self
            .run(
                &self.paths.pg_restore(),
                &["-l".into(), path.to_string_lossy().into_owned()],
                None,
            )
            .await?;
        if !out.code.is_some_and(|code| code == 0) || out.stdout.trim().is_empty() {
            return Err(BackupError::BackupVerificationFailed { detail: out.stderr });
        }
        Ok(())
    }

    async fn inspect(&self, dump_path: &Path) -> Result<Vec<ArchiveEntry>> {
        let path = absolute_path(dump_path)?;
        let out = self
            .run(
                &self.paths.pg_restore(),
                &["-l".into(), path.to_string_lossy().into_owned()],
                None,
            )
            .await?;
        if !out.code.is_some_and(|code| code == 0) {
            return Err(BackupError::BackupVerificationFailed { detail: out.stderr });
        }
        Ok(crate::docker::runner::parse_toc(&out.stdout))
    }
}

impl NativePgRunner {
    async fn restore(
        &self,
        dump_path: &Path,
        params: &crate::restore::RestoreParams,
    ) -> Result<()> {
        let path = absolute_path(dump_path)?;
        let mut args = vec![
            "-h".into(),
            params.target.host.clone(),
            "-p".into(),
            params.target.port.to_string(),
            "-U".into(),
            params.target.username.clone(),
            "-d".into(),
            params.target.database.clone(),
            "-w".into(),
            "--no-owner".into(),
            "--no-privileges".into(),
        ];
        if params.clean {
            args.extend(["--clean".into(), "--if-exists".into()]);
        }
        args.push(path.to_string_lossy().into_owned());
        let out = self
            .run(&self.paths.pg_restore(), &args, Some(&params.password))
            .await?;
        if !out.code.is_some_and(|code| code == 0) {
            return Err(BackupError::PgRestoreFailed { detail: out.stderr });
        }
        Ok(())
    }
}

fn absolute_path(path: &Path) -> Result<PathBuf> {
    if !path.is_absolute() {
        return Err(BackupError::InvalidPath {
            detail: format!("path must be absolute: {}", path.display()),
        });
    }
    Ok(path.to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bundled_layout_is_relative_to_executable() {
        let p = NativePgToolPaths::discover_from_executable(PathBuf::from(
            r"C:\App\DatabaseBackupManager.exe",
        ));
        assert!(p
            .bin_dir
            .ends_with(Path::new(r"resources\postgresql\17\bin")));
    }
}
