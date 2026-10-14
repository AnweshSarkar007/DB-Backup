//! Restore a backup into a target PostgreSQL database (Section 24).
//!
//! This is the only destructive operation, so the *caller* (UI) must gate it behind an explicit
//! typed confirmation and a target-connection test. This module performs the mechanical restore via
//! `pg_restore` in Docker, connecting to the target with the target's own SSL settings.

use crate::config::ConnectionConfig;
use crate::docker;
use crate::error::Result;
use crate::native::{RunnerKind, SelectedRunner};
use std::path::Path;

/// Parameters for a restore into an existing target database.
#[derive(Debug, Clone)]
pub struct RestoreParams {
    pub target: ConnectionConfig,
    pub password: String,
    /// pg image to run pg_restore with (should be >= the dump's server version).
    pub image: String,
    /// Drop objects before recreating (`--clean --if-exists`).
    pub clean: bool,
}

/// Restore `dump` into `params.target`. The target database must already exist.
pub async fn restore_to_target(dump: &Path, params: &RestoreParams) -> Result<()> {
    let runner = SelectedRunner::select(17, 3600);
    if runner.kind() == RunnerKind::Docker {
        docker::require_ready().await?;
        docker::ensure_image(&params.image).await?;
    }
    runner.restore(dump, params).await
}
