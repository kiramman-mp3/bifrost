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

    /// Inspect Windows 11 compatibility, virtualization, WHPX, RAM, storage, and GPU readiness.
    Doctor {
        /// Exit with non-zero status if any compatibility check fails.
        #[arg(long)]
        strict: bool,
    },

    /// Manage the Midgard Android runtime lifecycle (start, stop, restart, verify).
    Runtime {
        #[command(subcommand)]
        action: RuntimeAction,
    },

    /// List connected Android devices over Bridge/ADB.
    Devices,

    /// Inspect or stop the Heimdall background supervisor daemon over authenticated IPC.
    Daemon {
        #[command(subcommand)]
        action: DaemonAction,
    },

    /// Manage installed Android applications via Valhalla.
    Apps {
        #[command(subcommand)]
        action: AppsAction,
    },

    /// Inspect and install an Android `.apk` or `.apkm` split-APK bundle into Midgard via Valhalla.
    Install {
        /// Path to the `.apk` or `.apkm` file to inspect or install.
        apk: PathBuf,

        /// Target Android device serial (optional when a single device is online).
        #[arg(long)]
        serial: Option<String>,

        /// Only inspect APK structure, SHA-256, and x86_64 ABI compatibility without installing.
        #[arg(long)]
        inspect_only: bool,
    },

    /// Uninstall an Android package by package name via Valhalla.
    Uninstall {
        /// Android application package identifier (e.g., org.fdroid.fdroid).
        package: String,

        /// Target Android device serial (optional when a single device is online).
        #[arg(long)]
        serial: Option<String>,

        /// Retain application data and cache directories (`pm uninstall -k`).
        #[arg(long)]
        keep_data: bool,
    },

    /// Inspect Midgard serial console and VMM runtime logs.
    Logs {
        /// Number of trailing lines to show (0 = all lines).
        #[arg(long, default_value_t = 100)]
        tail: usize,
    },

    /// Manage scoped Windows <-> Android shared directories.
    Share {
        #[command(subcommand)]
        action: ShareAction,
    },

    /// Read the Windows clipboard or sync text to Android.
    Clipboard {
        #[command(subcommand)]
        action: ClipboardAction,
    },

    /// Inspect active Android notifications and render Windows toast payloads.
    Notifications {
        #[command(subcommand)]
        action: NotificationsAction,
    },

    /// Inspect running Android guest processes (`ps -A`).
    Processes {
        #[command(subcommand)]
        action: ProcessesAction,
    },

    /// Manage loopback-safe Windows <-> Android TCP port forwards.
    Network {
        #[command(subcommand)]
        action: NetworkAction,
    },

    /// Manage verified Midgard Android runtime image versions and upgrade compatibility.
    Images {
        #[command(subcommand)]
        action: ImagesAction,
    },

    /// Back up, restore, reset userdata safely, or export diagnostic bundles.
    Recovery {
        #[command(subcommand)]
        action: RecoveryAction,
    },

    /// Manage Windows 11 desktop integration (.apk file association & Explorer context menu).
    Desktop {
        #[command(subcommand)]
        action: DesktopAction,
    },
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

    /// Auto-tune CPU cores and RAM allocation in `bifrost.toml` for this Windows 11 host.
    Autotune,
}

#[derive(Debug, Subcommand)]
pub enum RuntimeAction {
    /// Verify image bundle integrity and start the Midgard Android VM over WHPX.
    Start {
        /// Path to the Midgard image bundle `manifest.json`.
        #[arg(long)]
        image: PathBuf,

        /// Run without a graphical display window (`-display none`).
        #[arg(long)]
        headless: bool,

        /// Attach serial console directly to current terminal stdio.
        #[arg(long)]
        console: bool,
    },
    /// Stop the running Midgard Android VM gracefully via ACPI/QMP.
    Stop {
        /// Forcefully terminate the VM process immediately.
        #[arg(long)]
        force: bool,
    },
    /// Restart the Midgard Android VM.
    Restart {
        /// Optional replacement manifest path (reuses previous manifest if omitted).
        #[arg(long)]
        image: Option<PathBuf>,

        /// Run without a graphical display window.
        #[arg(long)]
        headless: bool,
    },
    /// Cryptographically verify a Midgard image bundle manifest and SHA-256 digests.
    Verify {
        /// Path to the Midgard image bundle `manifest.json`.
        #[arg(long)]
        image: PathBuf,
    },
}

#[derive(Debug, Subcommand)]
pub enum AppsAction {
    /// List installed Android applications (`pm list packages`).
    List {
        /// Show only third-party (non-system) installed packages.
        #[arg(long)]
        third_party: bool,

        /// Target Android device serial.
        #[arg(long)]
        serial: Option<String>,
    },
    /// Show detailed metadata for an installed Android package (`dumpsys package`).
    Info {
        /// Android package identifier.
        package: String,

        /// Target Android device serial.
        #[arg(long)]
        serial: Option<String>,
    },
    /// Launch an installed Android application (optionally in an independent Windows 11 window).
    Launch {
        /// Android package identifier.
        package: String,

        /// Target Android device serial.
        #[arg(long)]
        serial: Option<String>,

        /// Launch inside an independent per-app Windows 11 VirtualDisplay window (ADR-0003).
        #[arg(long)]
        window: bool,

        /// Window width in pixels when `--window` is set.
        #[arg(long, default_value_t = 1280)]
        width: u32,

        /// Window height in pixels when `--window` is set.
        #[arg(long, default_value_t = 720)]
        height: u32,

        /// Display DPI when `--window` is set.
        #[arg(long, default_value_t = 240)]
        dpi: u32,

        /// Do not auto-start Midgard headlessly if no Android device is currently online.
        #[arg(long)]
        no_autostart: bool,
    },
    /// Force-stop a running Android application (`am force-stop`).
    Stop {
        /// Android package identifier.
        package: String,

        /// Target Android device serial.
        #[arg(long)]
        serial: Option<String>,
    },
    /// Create or remove Windows Desktop and Start Menu shortcuts for an installed app.
    Shortcut {
        /// Android package identifier (e.g., org.fdroid.fdroid).
        package: String,

        /// Optional display name for the Desktop & Start Menu shortcuts.
        #[arg(long)]
        name: Option<String>,

        /// Optional local `.apk` path to extract the app's native icon from.
        #[arg(long)]
        apk: Option<PathBuf>,

        /// Remove existing Desktop and Start Menu shortcuts instead of creating them.
        #[arg(long)]
        remove: bool,
    },
}

#[derive(Debug, Subcommand)]
pub enum DaemonAction {
    /// Start the Heimdall background supervisor daemon cleanly detached in the background.
    Start,
    /// Query live Heimdall daemon health and IPC status.
    Status,
    /// Request a graceful Heimdall daemon shutdown over authenticated IPC.
    Stop,
    /// Query or toggle unprivileged Windows login autostart for Heimdall (`HKCU\...\Run`).
    Autostart {
        /// Enable silent Heimdall startup at Windows user login.
        #[arg(long, conflicts_with = "disable")]
        enable: bool,
        /// Disable Heimdall startup at Windows user login.
        #[arg(long, conflicts_with = "enable")]
        disable: bool,
    },
}

#[derive(Debug, Subcommand)]
pub enum ShareAction {
    /// List files in the host BIFRÖST shared directory.
    List,
    /// Push a relative file from the host shared directory into Android `/sdcard/BifrostShare`.
    Push {
        /// Relative path inside the host shared directory.
        path: String,
        #[arg(long)]
        serial: Option<String>,
    },
    /// Pull a relative file from Android `/sdcard/BifrostShare` into the host shared directory.
    Pull {
        /// Relative path inside Android `/sdcard/BifrostShare`.
        path: String,
        #[arg(long)]
        serial: Option<String>,
    },
}

#[derive(Debug, Subcommand)]
pub enum ClipboardAction {
    /// Read current Unicode text from the Windows host clipboard.
    Get,
    /// Sync Windows host clipboard text into the connected Android device.
    SyncToAndroid {
        #[arg(long)]
        serial: Option<String>,
    },
}

#[derive(Debug, Subcommand)]
pub enum NotificationsAction {
    /// List active Android notifications over Bridge.
    List {
        #[arg(long)]
        serial: Option<String>,
    },
}

#[derive(Debug, Subcommand)]
pub enum ProcessesAction {
    /// List running Android guest processes sorted by resident memory (RSS).
    List {
        /// Optional substring filter matched against process names.
        #[arg(long)]
        filter: Option<String>,
        #[arg(long)]
        serial: Option<String>,
    },
}

#[derive(Debug, Subcommand)]
pub enum NetworkAction {
    /// List active ADB TCP port-forwarding rules.
    Forwards {
        #[arg(long)]
        serial: Option<String>,
    },
    /// Add an unprivileged loopback TCP port forward (`127.0.0.1:<local_port>` -> guest).
    Forward {
        local_port: u16,
        guest_port: u16,
        #[arg(long)]
        serial: Option<String>,
    },
    /// Remove a host loopback TCP port forward.
    Unforward {
        local_port: u16,
        #[arg(long)]
        serial: Option<String>,
    },
}

#[derive(Debug, Subcommand)]
pub enum ImagesAction {
    /// List registered Midgard runtime image bundles and highlight the active version.
    List,
    /// Verify SHA-256 (and optional Ed25519 signature) and register a Midgard image bundle.
    Register {
        /// Path to `manifest.json`.
        #[arg(long)]
        manifest: PathBuf,
        /// Optional 64-byte hex Ed25519 signature over `manifest.json`.
        #[arg(long)]
        signature: Option<String>,
        /// Optional 32-byte hex Ed25519 public key (required when `--signature` is set).
        #[arg(long)]
        pubkey: Option<String>,
        /// Userdata compatibility schema epoch (defaults to 1).
        #[arg(long, default_value_t = 1)]
        userdata_compat: u32,
    },
    /// Switch active Midgard runtime version after verifying upgrade compatibility.
    Use {
        /// Registered image ID to activate.
        image_id: String,
    },
}

#[derive(Debug, Subcommand)]
pub enum RecoveryAction {
    /// Create a SHA-256-verified backup of persistent Android `userdata.img`.
    Backup {
        /// Destination directory for the verified backup.
        #[arg(long)]
        output: PathBuf,
    },
    /// Verify backup SHA-256 and restore `userdata.img` (requires `--confirm`).
    Restore {
        /// Source backup directory containing `backup-metadata.json` and `userdata.img`.
        #[arg(long)]
        from: PathBuf,
        /// Explicit confirmation required before overwriting live userdata.
        #[arg(long)]
        confirm: bool,
    },
    /// Permanently delete persistent `userdata.img` (requires `--confirm`).
    WipeUserdata {
        /// Explicit confirmation required for destructive userdata wipe.
        #[arg(long)]
        confirm: bool,
    },
    /// Export a sanitized JSON diagnostic report (excluding auth tokens).
    ExportDiagnostics {
        /// Output `.json` file path.
        #[arg(long)]
        output: PathBuf,
    },
}

#[derive(Debug, Subcommand)]
pub enum DesktopAction {
    /// Register Windows Explorer "Install with BIFRÖST" context menu and default `.apk` / `.apkm` handler (HKCU, no admin).
    RegisterApkHandler {
        /// Only register the right-click context menu without setting BIFRÖST as the default `.apk`/`.apkm` double-click app.
        #[arg(long)]
        context_menu_only: bool,
    },
    /// Remove BIFRÖST's user-space `.apk` / `.apkm` file association and context menu keys from HKCU.
    UnregisterApkHandler,
}
