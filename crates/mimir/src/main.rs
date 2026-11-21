//! Mimir (`bifrost`) binary entry point.

mod cli;
mod commands;

use std::process::ExitCode;

use bifrost_core::config::LoggingConfig;
use bifrost_core::logging::init_logging;
use clap::Parser;
use tracing::{debug, error};

use crate::cli::Cli;

/// Exit code reserved for explicitly unsupported (planned) commands.
const EXIT_UNSUPPORTED: u8 = 2;

fn main() -> ExitCode {
    let cli = Cli::parse();

    let logging_cfg = LoggingConfig {
        level: cli.log_level.map(Into::into).unwrap_or_default(),
        json: cli.json,
    };
    init_logging(&logging_cfg);
    debug!(command = ?cli.command, "dispatching mimir command");

    match commands::execute(&cli) {
        Ok(output) => {
            println!("{output}");
            ExitCode::SUCCESS
        }
        Err(err) if err.is_unsupported() => {
            eprintln!("error: {err}");
            ExitCode::from(EXIT_UNSUPPORTED)
        }
        Err(err) => {
            error!(error = %err, "command failed");
            eprintln!("error: {err}");
            ExitCode::FAILURE
        }
    }
}
