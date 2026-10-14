//! Low-level Docker CLI access (Section 8).
//!
//! Every Docker invocation goes through [`run_docker`], which uses a structured argument vector
//! (never a shell string — Section 28 #7/#8), captures stdout/stderr/exit code, enforces a
//! timeout, kills the child on timeout, and redacts a known secret from captured output.

use crate::error::{BackupError, Result};
use std::process::Stdio;
use std::time::Duration;
use tokio::io::AsyncReadExt;
use tokio::process::Command;

/// The docker executable. Overridable via `DBM_DOCKER_PATH` (Rule 4: never hard-code paths).
pub fn docker_exe() -> String {
    std::env::var("DBM_DOCKER_PATH").unwrap_or_else(|_| "docker".to_string())
}

/// Captured result of a docker command.
#[derive(Debug, Clone)]
pub struct CmdOutput {
    pub code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
}

impl CmdOutput {
    pub fn success(&self) -> bool {
        self.code == Some(0)
    }
}

/// Run `docker <args...>` with a timeout. `redact` (if set) is scrubbed from captured output so a
/// password can never leak into logs or error detail.
pub async fn run_docker(
    args: &[String],
    timeout: Duration,
    redact: Option<&str>,
) -> Result<CmdOutput> {
    let mut cmd = Command::new(docker_exe());
    // CREATE_NO_WINDOW: avoid console flashes and focus-triggered health-check loops.
    #[cfg(windows)]
    cmd.creation_flags(0x0800_0000);
    cmd.args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);

    let mut child = cmd.spawn().map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            BackupError::DockerNotInstalled
        } else {
            BackupError::Io {
                detail: format!("failed to spawn docker: {e}"),
            }
        }
    })?;

    let mut stdout_pipe = child.stdout.take();
    let mut stderr_pipe = child.stderr.take();

    let mut stdout = String::new();
    let mut stderr = String::new();

    let wait = async {
        // Read both pipes concurrently with the wait.
        let out_fut = async {
            if let Some(p) = stdout_pipe.as_mut() {
                let _ = p.read_to_string(&mut stdout).await;
            }
        };
        let err_fut = async {
            if let Some(p) = stderr_pipe.as_mut() {
                let _ = p.read_to_string(&mut stderr).await;
            }
        };
        let status = tokio::join!(child.wait(), out_fut, err_fut).0;
        status
    };

    let status = match tokio::time::timeout(timeout, wait).await {
        Ok(s) => s.map_err(|e| BackupError::Io {
            detail: format!("docker wait: {e}"),
        })?,
        Err(_) => {
            let _ = child.start_kill();
            return Err(BackupError::Timeout {
                seconds: timeout.as_secs(),
            });
        }
    };

    let scrub = |s: String| match redact {
        Some(secret) => {
            crate::redact::redact_env_assignments(&crate::redact::redact_secret(&s, secret))
        }
        None => crate::redact::redact_env_assignments(&s),
    };

    Ok(CmdOutput {
        code: status.code(),
        stdout: scrub(stdout),
        stderr: scrub(stderr),
    })
}

/// Is the docker executable present on PATH (or at `DBM_DOCKER_PATH`)?
pub async fn is_docker_installed() -> bool {
    matches!(run_docker(&["--version".into()], Duration::from_secs(10), None).await, Ok(o) if o.success())
}

/// Is the Docker daemon running and reachable? (`docker info`)
pub async fn is_docker_running() -> bool {
    match run_docker(
        &[
            "info".into(),
            "--format".into(),
            "{{.ServerVersion}}".into(),
        ],
        Duration::from_secs(20),
        None,
    )
    .await
    {
        Ok(o) => o.success() && !o.stdout.trim().is_empty(),
        Err(_) => false,
    }
}

/// Docker client version string, or None if docker is unavailable.
pub async fn get_docker_version() -> Option<String> {
    match run_docker(
        &[
            "version".into(),
            "--format".into(),
            "{{.Server.Version}}".into(),
        ],
        Duration::from_secs(20),
        None,
    )
    .await
    {
        Ok(o) if o.success() => Some(o.stdout.trim().to_string()),
        _ => None,
    }
}
