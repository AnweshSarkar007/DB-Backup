//! `backup-worker.exe` — headless execution of a single backup job (Section 3).
//!
//! Runs independently of the GUI, so Windows Task Scheduler (Phase 6) can invoke it while
//! `DatabaseBackupManager.exe` is closed. Thin shell: all logic lives in `dbm-core`.

mod cli;
mod executor;

use clap::Parser;
use cli::{Cli, Command};
use dbm_core::AppPaths;
use std::process::ExitCode;

#[tokio::main]
async fn main() -> ExitCode {
    let args = Cli::parse();

    match args.command {
        Command::Doctor => {
            dbm_core::logging::init_console();
            if executor::doctor().await {
                ExitCode::SUCCESS
            } else {
                ExitCode::FAILURE
            }
        }

        Command::TestConnection { spec } => {
            dbm_core::logging::init_console();
            match executor::load_spec(&spec) {
                Ok(spec) => match executor::test_connection(&spec).await {
                    Ok(()) => ExitCode::SUCCESS,
                    Err(e) => {
                        eprintln!("Connection failed: {e:#}");
                        ExitCode::FAILURE
                    }
                },
                Err(e) => {
                    eprintln!("{e:#}");
                    ExitCode::FAILURE
                }
            }
        }

        Command::Run { job_id, spec, keep_temp } => {
            let paths = match AppPaths::resolve() {
                Ok(p) => p,
                Err(e) => {
                    eprintln!(
                        "Could not prepare application directories: {}",
                        e.user_message()
                    );
                    return ExitCode::FAILURE;
                }
            };
            let _guard = dbm_core::logging::init(&paths);

            let result = match (job_id, spec) {
                (Some(id), _) => match executor::run_job(id, &paths, keep_temp).await {
                    Ok(r) => r,
                    Err(e) => {
                        eprintln!("{e:#}");
                        return ExitCode::FAILURE;
                    }
                },
                (None, Some(path)) => match executor::load_spec(&path) {
                    Ok(s) => executor::run(&s, &paths, keep_temp).await,
                    Err(e) => {
                        eprintln!("{e:#}");
                        return ExitCode::FAILURE;
                    }
                },
                (None, None) => {
                    eprintln!("Provide --job-id <policy_id> or --spec <file>.");
                    return ExitCode::FAILURE;
                }
            };
            print_summary(&result);
            ExitCode::from(executor::exit_code(&result) as u8)
        }

        Command::Inspect { dump, image } => {
            dbm_core::logging::init_console();
            match executor::inspect(&dump, &image).await {
                Ok(()) => ExitCode::SUCCESS,
                Err(e) => {
                    eprintln!("Inspect failed: {e:#}");
                    ExitCode::FAILURE
                }
            }
        }
    }
}

fn print_summary(result: &dbm_core::RunResult) {
    println!("\n──────── Backup Summary ────────");
    println!("Backup ID : {}", result.backup_id);
    println!("Job       : {}", result.job_id);
    println!("State     : {:?}", result.state);
    if let Some(f) = &result.filename {
        println!("File      : {f}");
    }
    if let Some(s) = result.file_size {
        println!("Size      : {}", dbm_core::error::human_bytes(s));
    }
    if let Some(h) = &result.sha256 {
        println!("SHA-256   : {h}");
    }
    println!("Verify    : {:?}", result.verification);
    if !result.destinations.is_empty() {
        println!("Destinations:");
        for d in &result.destinations {
            match &d.error_message {
                Some(msg) if d.status != dbm_core::DestinationStatus::Completed => {
                    println!("  - {:<20} {:?} ({msg})", d.name, d.status)
                }
                _ => println!("  - {:<20} {:?}", d.name, d.status),
            }
        }
    }
    if let Some(msg) = &result.error_message {
        println!("Error     : {msg}");
    }
    println!("────────────────────────────────");
}
