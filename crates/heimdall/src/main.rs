//! Heimdall (`heimdall.exe`) background supervisor daemon entry point.

mod app_sync;
mod desktop_bridge;
mod pipe_security;
mod server;
mod supervisor;

use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;
use std::time::Duration;

use bifrost_bridge::{default_pipe_name, AdbClient};
use bifrost_core::config::LoggingConfig;
use bifrost_core::logging::init_logging;
use bifrost_core::{BifrostConfig, BifrostPaths, LogLevel};
use bifrost_runtime::MidgardRuntime;
use clap::{Parser, Subcommand, ValueEnum};
use tokio::sync::watch;
use tracing::{error, info};

use crate::server::serve_named_pipe;
use crate::supervisor::{run_health_monitor, SupervisorState};

#[derive(Debug, Clone, Copy, ValueEnum)]
enum CliLogLevel {
    Error,
    Warn,
    Info,
    Debug,
    Trace,
}

impl From<CliLogLevel> for LogLevel {
    fn from(v: CliLogLevel) -> Self {
        match v {
            CliLogLevel::Error => Self::Error,
            CliLogLevel::Warn => Self::Warn,
            CliLogLevel::Info => Self::Info,
            CliLogLevel::Debug => Self::Debug,
            CliLogLevel::Trace => Self::Trace,
        }
    }
}

#[derive(Debug, Parser)]
#[command(
    name = "heimdall",
    version,
    about = "Heimdall — Background supervisor daemon and authenticated local IPC server for BIFRÖST."
)]
struct Cli {
    /// Path to `bifrost.toml` configuration file.
    #[arg(long, global = true)]
    config: Option<PathBuf>,

    /// Override logging verbosity.
    #[arg(long, value_enum, global = true)]
    log_level: Option<CliLogLevel>,

    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Run the Heimdall daemon in the foreground (listening on authenticated Windows Named Pipe).
    Run {
        /// Override Named Pipe path (defaults to per-user `\\.\pipe\bifrost-heimdall-<user>`).
        #[arg(long)]
        pipe: Option<String>,

        /// Periodic health check interval in seconds.
        #[arg(long, default_value_t = 5)]
        health_interval_secs: u64,
    },
}

#[tokio::main]
async fn main() -> ExitCode {
    let cli = Cli::parse();
    let log_cfg = LoggingConfig {
        level: cli.log_level.map(Into::into).unwrap_or_default(),
        json: false,
    };
    init_logging(&log_cfg);

    match run(cli).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            error!(error = %err, "Heimdall daemon exited with error");
            eprintln!("error: {err}");
            ExitCode::FAILURE
        }
    }
}

async fn run(cli: Cli) -> bifrost_core::Result<()> {
    let mut paths = BifrostPaths::discover()?;
    let config_path = match cli.config {
        Some(p) => p,
        None => paths.default_config_file(),
    };
    let config = if config_path.exists() {
        BifrostConfig::load_from_file(&config_path)?
    } else {
        BifrostConfig::default()
    };

    if let Some(ref custom_data_dir) = config.runtime.data_dir {
        paths.data_dir = custom_data_dir.clone();
        paths.images_dir = custom_data_dir.join("images");
        paths.logs_dir = custom_data_dir.join("logs");
    }

    match cli.command {
        Command::Run {
            pipe,
            health_interval_secs,
        } => {
            let pipe_name = pipe.unwrap_or_else(default_pipe_name);
            let data_dir = paths.data_dir.clone();
            let desktop_cfg = config.desktop.clone();
            let runtime = MidgardRuntime::with_default_backend(config, paths);
            let adb = AdbClient::new();
            let state = Arc::new(SupervisorState::with_desktop_config(
                runtime,
                adb,
                pipe_name.clone(),
                data_dir.clone(),
                desktop_cfg,
            ));

            let (shutdown_tx, shutdown_rx) = watch::channel(false);

            let monitor_state = Arc::clone(&state);
            let monitor_rx = shutdown_rx.clone();
            let interval = Duration::from_secs(health_interval_secs.max(1));
            let monitor_handle = tokio::spawn(async move {
                run_health_monitor(monitor_state, interval, monitor_rx).await;
            });

            let ctrl_c_tx = shutdown_tx.clone();
            tokio::spawn(async move {
                if tokio::signal::ctrl_c().await.is_ok() {
                    info!("received Ctrl+C; stopping Heimdall");
                    let _ = ctrl_c_tx.send(true);
                }
            });

            serve_named_pipe(state, data_dir, shutdown_tx, shutdown_rx).await?;
            let _ = monitor_handle.await;
            Ok(())
        }
    }
}
