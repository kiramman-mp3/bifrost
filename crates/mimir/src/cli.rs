//! Command-line argument definitions for Mimir (`bifrost`).

use std::path::PathBuf;

use bifrost_core::LogLevel;
use clap::{Parser, Subcommand, ValueEnum};

/// CLI log level selector mapped cleanly to `bifrost_core::LogLevel`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum CliLogLevel {
    Error,
    Warn,
    Info,
    Debug,
    Trace,
}

impl From<CliLogLevel> for LogLevel {
    fn from(value: CliLogLevel) -> Self {
        match value {
            CliLogLevel::Error => Self::Error,
            CliLogLevel::Warn => Self::Warn,
            CliLogLevel::Info => Self::Info,
            CliLogLevel::Debug => Self::Debug,
            CliLogLevel::Trace => Self::Trace,
        }
    }
}

/// BIFRÖST — Administrative CLI (Mimir).
#[derive(Debug, Parser)]
#[command(
    name = "bifrost",
    version,
    about = "BIFRÖST — An open-source Android subsystem for Windows, powered by Rust.",
    long_about = "Mimir is the administrative command-line interface for BIFRÖST.\n\nCommands that are not yet implemented explicitly report their unsupported status and target milestone."
)]
pub struct Cli {
    /// Emit machine-readable JSON output where supported.
    #[arg(long, global = true)]
    pub json: bool,

    /// Override logging verbosity for this invocation.
    #[arg(long, value_enum, global = true)]
    pub log_level: Option<CliLogLevel>,

    /// Path to a BIFRÖST TOML configuration file.
    #[arg(long, global = true)]
    pub config: Option<PathBuf>,

    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Show BIFRÖST version, target platform, and component maturity status.
    Version,

    /// Show current runtime and Heimdall daemon status.
    Status,

    /// Inspect, initialize, and validate BIFRÖST configuration.
    Config {
        #[command(subcommand)]
        action: ConfigAction,
    },

    /// Inspect Windows 11 compatibility and virtualization readiness (Planned: Milestone 2).
    Doctor,

    /// Manage the Midgard Android runtime lifecycle (Planned: Milestones 3-4).
    Runtime {
        #[command(subcommand)]
        action: RuntimeAction,
    },

    /// List connected Android devices over Bridge/ADB (Planned: Milestone 4).
    Devices,

    /// Manage installed Android applications via Valhalla (Planned: Milestone 5).
    Apps {
        #[command(subcommand)]
        action: AppsAction,
    },

    /// Install an Android APK package into Midgard (Planned: Milestone 5).
    Install {
        /// Path to the `.apk` file to install.
        apk: PathBuf,
    },

    /// Uninstall an Android package by package name (Planned: Milestone 5).
    Uninstall {
        /// Android application package identifier (e.g., com.example.app).
        package: String,
    },

    /// Stream or inspect Heimdall and Midgard runtime logs (Planned: Milestone 4).
    Logs,
}

#[derive(Debug, Subcommand)]
pub enum ConfigAction {
    /// Write a default validated `bifrost.toml` configuration file.
    Init {
        /// Destination path (defaults to per-user BIFRÖST config directory).
        #[arg(long)]
        output: Option<PathBuf>,

        /// Explicitly allow overwriting an existing configuration file.
        #[arg(long)]
        force: bool,
    },

    /// Print the effective validated configuration (from file or built-in defaults).
    Show,

    /// Validate a configuration file against schema, bounds, and security rules.
    Validate,
}

#[derive(Debug, Subcommand)]
pub enum RuntimeAction {
    /// Start the Midgard Android runtime.
    Start,
    /// Stop the Midgard Android runtime gracefully.
    Stop,
    /// Restart the Midgard Android runtime.
    Restart,
}

#[derive(Debug, Subcommand)]
pub enum AppsAction {
    /// List installed Android applications.
    List,
}
