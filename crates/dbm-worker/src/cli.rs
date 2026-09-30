//! Command-line surface for `backup-worker.exe`.
//!
//! Phase 1 reads a job *spec file*. The scheduled `--job-id <n>` form (Section 3/19) is the Phase 2
//! evolution: it will hydrate the same [`JobSpec`](dbm_core::JobSpec) from SQLite and call the exact
//! same executor, so scheduled and manual runs share one code path.

use clap::{Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(
    name = "backup-worker",
    about = "Executes a Database Backup Manager job headlessly.",
    version
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand, Debug)]
pub enum Command {
    /// Report Docker availability (installed / running / version).
    Doctor,

    /// Test the database connection described by a job spec.
    TestConnection {
        /// Path to a job spec JSON file.
        #[arg(long)]
        spec: PathBuf,
    },

    /// Run a full backup: dump → verify → hash → manifest → distribute.
    ///
    /// Provide EITHER `--job-id <policy_id>` (hydrate from the local database — this is what Windows
    /// Task Scheduler invokes) OR `--spec <file>` (a standalone JSON job spec).
    Run {
        /// Policy id to run from the local database (scheduled path).
        #[arg(long, conflicts_with = "spec")]
        job_id: Option<i64>,
        /// Path to a standalone job spec JSON file.
        #[arg(long)]
        spec: Option<PathBuf>,
        /// Keep the temporary dump even on full success (debugging).
        #[arg(long, default_value_t = false)]
        keep_temp: bool,
    },

    /// Inspect a `.dump` archive's table of contents (no database is touched).
    Inspect {
        /// Path to a `.dump` file.
        #[arg(long)]
        dump: PathBuf,
        /// PostgreSQL image to run pg_restore with (must be >= the archive's server version).
        #[arg(long, default_value = "postgres:17")]
        image: String,
    },
}
