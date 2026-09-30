//! Aggregate Docker health, surfaced to the CLI (`doctor`) and later the UI first-run check
//! (Section 39). We never try to install Docker — we report clearly and let the user act.

use crate::docker::client;
use crate::error::{BackupError, Result};
use serde::Serialize;

/// A snapshot of Docker availability.
#[derive(Debug, Clone, Serialize)]
pub struct DockerHealth {
    pub installed: bool,
    pub running: bool,
    pub version: Option<String>,
}

/// Gather Docker health without failing.
pub async fn doctor() -> DockerHealth {
    let installed = client::is_docker_installed().await;
    if !installed {
        return DockerHealth {
            installed: false,
            running: false,
            version: None,
        };
    }
    let running = client::is_docker_running().await;
    let version = if running {
        client::get_docker_version().await
    } else {
        None
    };
    DockerHealth {
        installed,
        running,
        version,
    }
}

/// Require a healthy Docker, returning the precise typed error otherwise (used by the engine before
/// a backup starts).
pub async fn require_ready() -> Result<()> {
    if !client::is_docker_installed().await {
        return Err(BackupError::DockerNotInstalled);
    }
    if !client::is_docker_running().await {
        return Err(BackupError::DockerNotRunning);
    }
    Ok(())
}
