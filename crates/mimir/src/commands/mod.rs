//! Command handlers for Mimir (`bifrost`).
//!
//! Unimplemented commands always return `BifrostError::Unsupported` and never
//! simulate successful runtime operations.

use std::path::PathBuf;

use bifrost_bridge::{
    default_pipe_name, list_active_notifications, read_host_clipboard_text,
    sync_clipboard_to_android, AdbClient, AdbDevice, AdbEndpointState, DaemonHealthReport,
    HeimdallClient, IpcRequest, SharedDirectoryBridge,
};
use bifrost_client::{blocking, DaemonStart, LoadedConfig, Session, StatusSnapshot};
use bifrost_core::{
    validate_host_file_path, BifrostConfig, BifrostError, BifrostPaths, Result, RuntimeState,
    VersionInfo,
};
use bifrost_diagnostics::{render_human_report, run_doctor, OverallReadiness};
use bifrost_display::{AppWindowSpec, DisplayController};
use bifrost_network::NetworkManager;
use bifrost_process::ProcessManager;
use bifrost_runtime::{MidgardRuntime, RuntimeRecord};
use bifrost_storage::{
    backup_userdata, evaluate_manifest_trust, restore_userdata, verify_manifest_file,
    wipe_userdata_disk, ImageRegistry, ImageTrust,
};
use serde::Serialize;
use valhalla::{inspect_apk, AbiCompatibility, ValhallaManager};

use crate::cli::{
    AppsAction, Cli, ClipboardAction, Command, ConfigAction, DaemonAction, DesktopAction,
    ImagesAction, NetworkAction, NotificationsAction, ProcessesAction, RecoveryAction,
    RuntimeAction, ShareAction,
};

/// Structured output returned by a Mimir CLI command.
mod apps;
mod config;
mod daemon;
mod desktop;
mod images;
mod recovery;
mod runtime;
mod setup;

use apps::{handle_apps, handle_install, handle_uninstall};
use config::handle_config;
use daemon::handle_daemon;
use desktop::{
    handle_clipboard, handle_desktop, handle_network, handle_notifications, handle_processes,
    handle_share,
};
use images::handle_images;
use recovery::handle_recovery;
use runtime::{handle_devices, handle_doctor, handle_logs, handle_runtime, handle_status};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandOutput {
    pub stdout: String,
    pub exit_code: u8,
}

impl CommandOutput {
    fn ok(stdout: String) -> Self {
        Self {
            stdout,
            exit_code: 0,
        }
    }
}

/// Dispatches a parsed CLI command and returns formatted stdout output and exit code on completion.
pub async fn execute(cli: &Cli) -> Result<CommandOutput> {
    match &cli.command {
        Command::Version => handle_version(cli.json).map(CommandOutput::ok),
        Command::Status => handle_status(cli.config.as_ref(), cli.json)
            .await
            .map(CommandOutput::ok),
        Command::Config { action } => {
            handle_config(action, cli.config.as_ref(), cli.json).map(CommandOutput::ok)
        }
        Command::Doctor { strict } => handle_doctor(cli.config.as_ref(), cli.json, *strict),
        Command::Update => setup::handle_update(cli.json).await.map(CommandOutput::ok),
        Command::Setup { install } => {
            setup::handle_setup(cli.config.as_ref(), cli.json, install.as_deref())
                .await
                .map(CommandOutput::ok)
        }
        Command::Runtime { action } => handle_runtime(action, cli.config.as_ref(), cli.json)
            .await
            .map(CommandOutput::ok),
        Command::Devices => handle_devices(cli.config.as_ref(), cli.json)
            .await
            .map(CommandOutput::ok),
        Command::Daemon { action } => handle_daemon(action, cli.config.as_ref(), cli.json)
            .await
            .map(CommandOutput::ok),
        Command::Apps { action } => handle_apps(action, cli.config.as_ref(), cli.json)
            .await
            .map(CommandOutput::ok),
        Command::Install {
            apk,
            serial,
            inspect_only,
        } => handle_install(
            apk,
            serial.as_deref(),
            *inspect_only,
            cli.config.as_ref(),
            cli.json,
        )
        .await
        .map(CommandOutput::ok),
        Command::Uninstall {
            package,
            serial,
            keep_data,
        } => handle_uninstall(
            package,
            serial.as_deref(),
            *keep_data,
            cli.config.as_ref(),
            cli.json,
        )
        .await
        .map(CommandOutput::ok),
        Command::Logs { tail } => handle_logs(cli.config.as_ref(), *tail, cli.json)
            .await
            .map(CommandOutput::ok),
        Command::Share { action } => {
            handle_share(action, cli.config.as_ref(), cli.json).map(CommandOutput::ok)
        }
        Command::Clipboard { action } => handle_clipboard(action, cli.json).map(CommandOutput::ok),
        Command::Notifications { action } => {
            handle_notifications(action, cli.json).map(CommandOutput::ok)
        }
        Command::Processes { action } => handle_processes(action, cli.json).map(CommandOutput::ok),
        Command::Network { action } => handle_network(action, cli.json).map(CommandOutput::ok),
        Command::Images { action } => {
            handle_images(action, cli.config.as_ref(), cli.json).map(CommandOutput::ok)
        }
        Command::Recovery { action } => handle_recovery(action, cli.config.as_ref(), cli.json)
            .await
            .map(CommandOutput::ok),
        Command::Desktop { action } => handle_desktop(action, cli.json).map(CommandOutput::ok),
    }
}

fn resolve_config_path(explicit: Option<&PathBuf>) -> Result<PathBuf> {
    if let Some(path) = explicit {
        return validate_host_file_path(path, Some("toml"));
    }
    let paths = BifrostPaths::discover()?;
    Ok(paths.default_config_file())
}

fn load_effective_config_and_paths(
    explicit_config: Option<&PathBuf>,
) -> Result<(PathBuf, bool, BifrostConfig, BifrostPaths)> {
    let loaded = LoadedConfig::load(explicit_config.map(PathBuf::as_path))?;
    Ok((
        loaded.config_path,
        loaded.config_exists,
        loaded.config,
        loaded.paths,
    ))
}

fn heimdall_client_for(paths: &BifrostPaths) -> HeimdallClient {
    HeimdallClient::new(default_pipe_name(), &paths.data_dir)
}

fn handle_version(as_json: bool) -> Result<String> {
    let info = VersionInfo::current();
    if as_json {
        return serde_json::to_string_pretty(&info)
            .map_err(|err| BifrostError::ConfigSerialize(err.to_string()));
    }

    let mut out = format!(
        "{} ({}) v{}\n{}\nTarget: {}-{}\nStage: {}\n\nComponent Status:\n",
        info.product,
        info.identifier,
        info.version,
        info.tagline,
        info.target_os,
        info.target_arch,
        info.current_milestone
    );
    for cap in &info.capabilities {
        out.push_str(&format!(
            "  - {:<18} [{}] {}\n",
            cap.name, cap.status, cap.summary
        ));
    }
    Ok(out.trim_end().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;
    use tempfile::TempDir;

    #[tokio::test]
    async fn version_command_returns_json_and_text() {
        let cli = Cli::try_parse_from(["bifrost", "version"]).unwrap();
        let out = execute(&cli).await.unwrap();
        assert_eq!(out.exit_code, 0);
        assert!(out.stdout.contains("BIFRÖST"));
        assert!(out.stdout.contains("arm64-translation"));

        let cli_json = Cli::try_parse_from(["bifrost", "--json", "version"]).unwrap();
        let json = execute(&cli_json).await.unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json.stdout).unwrap();
        assert_eq!(parsed["identifier"], "bifrost");
    }

    #[tokio::test]
    async fn config_init_validate_and_show_work_end_to_end() {
        let tmp = TempDir::new().unwrap();
        let cfg_file = tmp.path().join("bifrost.toml");
        let cfg_str = cfg_file.to_str().unwrap();

        let init_cli =
            Cli::try_parse_from(["bifrost", "config", "init", "--output", cfg_str]).unwrap();
        assert!(execute(&init_cli).await.is_ok());

        let validate_cli =
            Cli::try_parse_from(["bifrost", "--config", cfg_str, "config", "validate"]).unwrap();
        let validation = execute(&validate_cli).await.unwrap();
        assert!(validation.stdout.contains("is valid"));

        let show_cli =
            Cli::try_parse_from(["bifrost", "--config", cfg_str, "config", "show"]).unwrap();
        let shown = execute(&show_cli).await.unwrap();
        assert!(shown.stdout.contains("adb_bind_address = \"127.0.0.1\""));
    }

    #[tokio::test]
    async fn status_honestly_reports_stopped_and_disconnected() {
        let tmp = TempDir::new().unwrap();
        let cfg_file = tmp.path().join("bifrost.toml");
        let data_dir = tmp.path().join("data");
        let mut cfg = BifrostConfig::default();
        cfg.runtime.data_dir = Some(data_dir);
        cfg.save_to_file(&cfg_file, true).unwrap();

        let cli = Cli::try_parse_from([
            "bifrost",
            "--json",
            "--config",
            cfg_file.to_str().unwrap(),
            "status",
        ])
        .unwrap();
        let out = execute(&cli).await.unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&out.stdout).unwrap();
        assert_eq!(parsed["runtime_state"], "stopped");
        assert_eq!(parsed["heimdall_connected"], false);
    }

    #[tokio::test]
    async fn doctor_emits_structured_compatibility_report() {
        let cli = Cli::try_parse_from(["bifrost", "--json", "doctor"]).unwrap();
        let out = execute(&cli).await.unwrap();
        assert_eq!(out.exit_code, 0);
        let parsed: serde_json::Value = serde_json::from_str(&out.stdout).unwrap();
        assert_eq!(parsed["checks"].as_array().unwrap().len(), 8);
    }

    #[test]
    fn setup_install_accepts_only_known_tools() {
        assert!(Cli::try_parse_from(["bifrost", "setup", "--install", "adb"]).is_ok());
        assert!(Cli::try_parse_from(["bifrost", "setup", "--install", "Evil.Package"]).is_err());
    }

    #[tokio::test]
    async fn install_rejects_path_traversal_and_invalid_package_on_uninstall() {
        let bad = Cli::try_parse_from(["bifrost", "install", "../evil.apk"]).unwrap();
        let err = execute(&bad).await.unwrap_err();
        assert!(matches!(err, BifrostError::UnsafePath { .. }));

        let bad_pkg = Cli::try_parse_from(["bifrost", "uninstall", "com.bad;whoami"]).unwrap();
        let pkg_err = execute(&bad_pkg).await.unwrap_err();
        assert!(matches!(pkg_err, BifrostError::Package(_)));
    }

    #[tokio::test]
    async fn install_inspect_only_reports_pure_art_apk_offline() {
        use std::io::Write;
        let tmp = TempDir::new().unwrap();
        let apk_path = tmp.path().join("sample.apk");
        let file = std::fs::File::create(&apk_path).unwrap();
        let mut writer = zip::ZipWriter::new(file);
        writer
            .start_file(
                "AndroidManifest.xml",
                zip::write::SimpleFileOptions::default(),
            )
            .unwrap();
        writer.write_all(b"xml").unwrap();
        writer
            .start_file("classes.dex", zip::write::SimpleFileOptions::default())
            .unwrap();
        writer.write_all(b"dex").unwrap();
        writer.finish().unwrap();

        let cli = Cli::try_parse_from([
            "bifrost",
            "--json",
            "install",
            "--inspect-only",
            apk_path.to_str().unwrap(),
        ])
        .unwrap();
        let out = execute(&cli).await.unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&out.stdout).unwrap();
        assert_eq!(parsed["abi_compatibility"]["kind"], "pure_art");
    }
}
