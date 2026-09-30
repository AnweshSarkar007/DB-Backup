//! Tracing setup (Section 18). Logs go to the console and a daily-rolling file in the app's `logs`
//! directory. Secret redaction is enforced at the *source* (errors and tool output are scrubbed
//! before they are ever logged — see [`crate::redact`]), so no log sink can leak a password.

use crate::config::AppPaths;
use tracing_subscriber::{fmt, prelude::*, EnvFilter};

/// Initialize logging. Safe to call once at process start. Returns a guard that must be kept alive
/// for the duration of the process so buffered file logs are flushed.
pub fn init(paths: &AppPaths) -> Option<tracing_appender::non_blocking::WorkerGuard> {
    let filter = EnvFilter::try_from_env("DBM_LOG").unwrap_or_else(|_| EnvFilter::new("info"));

    let file_appender = tracing_appender::rolling::daily(&paths.logs_dir, "dbm.log");
    let (nb, guard) = tracing_appender::non_blocking(file_appender);

    let console_layer = fmt::layer().with_target(false).with_ansi(true);
    let file_layer = fmt::layer()
        .with_target(true)
        .with_ansi(false)
        .with_writer(nb);

    let _ = tracing_subscriber::registry()
        .with(filter)
        .with(console_layer)
        .with(file_layer)
        .try_init();

    Some(guard)
}

/// Minimal console-only logging for tests / short-lived tools that have no data dir yet.
pub fn init_console() {
    let filter = EnvFilter::try_from_env("DBM_LOG").unwrap_or_else(|_| EnvFilter::new("info"));
    let _ = tracing_subscriber::registry()
        .with(filter)
        .with(fmt::layer().with_target(false))
        .try_init();
}
