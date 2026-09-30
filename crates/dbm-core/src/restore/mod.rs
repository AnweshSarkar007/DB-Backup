//! Restore a backup into a target PostgreSQL database (Section 24).
//!
//! This is the only destructive operation, so the *caller* (UI) must gate it behind an explicit
//! typed confirmation and a target-connection test. This module performs the mechanical restore via
//! `pg_restore` in Docker, connecting to the target with the target's own SSL settings.

use crate::config::ConnectionConfig;
use crate::docker::{self, container_runner, container_runner::PgConnOpts, image_manager};
use crate::error::Result;
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
    docker::require_ready().await?;
    image_manager::ensure_image(&params.image).await?;

    let opts = PgConnOpts {
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
    container_runner::run_pg_restore(&opts, &name, dump, false, params.clean).await
}
