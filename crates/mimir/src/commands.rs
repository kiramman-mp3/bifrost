//! Command handlers for Mimir (`bifrost`).
//!
//! Unimplemented commands always return `BifrostError::Unsupported` and never
//! simulate successful runtime operations.

use std::path::PathBuf;

use bifrost_bridge::{
    default_pipe_name, list_active_notifications, read_host_clipboard_text,
    sync_clipboard_to_android, AdbClient, AdbDevice, DaemonHealthReport, HeimdallClient,
    IpcRequest, SharedDirectoryBridge,
};
use bifrost_core::{
    validate_host_file_path, BifrostConfig, BifrostError, BifrostPaths, Result, RuntimeState,
    VersionInfo,
};
use bifrost_diagnostics::{render_human_report, run_doctor, OverallReadiness};
use bifrost_display::{AppWindowSpec, DisplayController};
use bifrost_network::NetworkManager;
use bifrost_process::ProcessManager;
use bifrost_runtime::{MidgardRuntime, RuntimeLogs, RuntimeRecord, StartOptions};
use bifrost_storage::{
    backup_userdata, evaluate_manifest_trust, restore_userdata, verify_manifest_file,
    wipe_userdata_disk, ImageRegistry, ImageTrust,
};
use serde::Serialize;
use valhalla::manager::{InstallOutcome, LaunchOutcome};
use valhalla::{inspect_apk, AbiCompatibility, InstalledPackage, PackageDetails, ValhallaManager};

use crate::cli::{
    AppsAction, Cli, ClipboardAction, Command, ConfigAction, DaemonAction, DesktopAction,
    ImagesAction, NetworkAction, NotificationsAction, ProcessesAction, RecoveryAction,
    RuntimeAction, ShareAction,
};

/// Structured status output returned by `bifrost status`.
#[derive(Debug, Serialize)]
pub struct StatusReport {
    pub runtime_state: RuntimeState,
    pub heimdall_connected: bool,
    pub heimdall_health: Option<DaemonHealthReport>,
    pub config_path: PathBuf,
    pub config_exists: bool,
    pub runtime: RuntimeRecord,
    pub note: String,
}

/// Structured output returned by a Mimir CLI command.
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

fn handle_desktop(action: &DesktopAction, as_json: bool) -> Result<String> {
    match action {
        DesktopAction::RegisterApkHandler { context_menu_only } => {
            let status = valhalla::register_apk_handler(!*context_menu_only)?;
            if as_json {
                return serde_json::to_string_pretty(&status)
                    .map_err(|err| BifrostError::ConfigSerialize(err.to_string()));
            }
            Ok(format!(
                "Registered Windows .apk/.apkm handler (ProgID: {}, context_menu: {}, default_app: {}).\nCommand: {}",
                status.prog_id,
                status.context_menu_registered,
                status.default_handler_registered,
                status.command
            ))
        }
        DesktopAction::UnregisterApkHandler => {
            valhalla::unregister_apk_handler()?;
            if as_json {
                return serde_json::to_string_pretty(&serde_json::json!({
                    "unregistered": true,
                    "prog_id": valhalla::BIFROST_APK_PROG_ID,
                }))
                .map_err(|err| BifrostError::ConfigSerialize(err.to_string()));
            }
            Ok(
                "Removed BIFRÖST .apk/.apkm file association and Explorer context menu from HKCU."
                    .to_string(),
            )
        }
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
    let config_path = resolve_config_path(explicit_config)?;
    let config_exists = config_path.exists();
    let config = if config_exists {
        BifrostConfig::load_from_file(&config_path)?
    } else {
        BifrostConfig::default()
    };

    let mut paths = BifrostPaths::discover()?;
    if let Some(ref custom_data_dir) = config.runtime.data_dir {
        paths.data_dir = custom_data_dir.clone();
        paths.images_dir = custom_data_dir.join("images");
        paths.logs_dir = custom_data_dir.join("logs");
    }

    Ok((config_path, config_exists, config, paths))
}

fn heimdall_client_for(paths: &BifrostPaths) -> HeimdallClient {
    HeimdallClient::new(default_pipe_name(), &paths.data_dir)
}

fn locate_heimdall_binary() -> Result<PathBuf> {
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            for name in ["heimdall.exe", "heimdall"] {
                let sibling = dir.join(name);
                if sibling.is_file() {
                    return Ok(sibling);
                }
            }
        }
    }
    if let Ok(local) = std::env::var("LOCALAPPDATA") {
        let installed = PathBuf::from(local)
            .join("Programs")
            .join("BIFRÖST")
            .join("heimdall.exe");
        if installed.is_file() {
            return Ok(installed);
        }
    }
    Err(BifrostError::Runtime(
        "heimdall executable not found next to bifrost.exe or in %LOCALAPPDATA%\\Programs\\BIFRÖST"
            .to_string(),
    ))
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

async fn handle_status(explicit_config: Option<&PathBuf>, as_json: bool) -> Result<String> {
    let (config_path, config_exists, config, paths) =
        load_effective_config_and_paths(explicit_config)?;
    let client = heimdall_client_for(&paths);
    let heimdall_health = client.health().await.ok();
    let heimdall_connected = heimdall_health.is_some();

    let mut record = if heimdall_connected {
        client
            .send::<RuntimeRecord>(IpcRequest::RuntimeStatus)
            .await?
    } else {
        let runtime_mgr = MidgardRuntime::with_default_backend(config.clone(), paths.clone());
        runtime_mgr.status()?
    };

    // Reconcile `Booting` -> `Running` directly via loopback ADB when Heimdall is offline.
    if !heimdall_connected && record.state == RuntimeState::Booting {
        if let Some(ref endpoint) = record.adb_endpoint {
            let adb = AdbClient::new();
            if adb.ensure_connected(endpoint).unwrap_or(false)
                && adb
                    .probe_guest_health(endpoint)
                    .is_ok_and(|g| g.boot_completed)
            {
                let runtime_mgr = MidgardRuntime::with_default_backend(config, paths);
                let _ = runtime_mgr.mark_running();
                record.state = RuntimeState::Running;
            }
        }
    }

    let runtime_state = heimdall_health
        .as_ref()
        .map(|h| h.runtime_state)
        .unwrap_or(record.state);

    let note = if heimdall_connected {
        "Connected to Heimdall supervisor over authenticated Windows Named Pipe IPC."
    } else if record.state.is_active() {
        "Midgard VM is active; Heimdall daemon is not running (start with `bifrost daemon start`)."
    } else {
        "Heimdall daemon is offline. Start `bifrost daemon start` for background supervision or launch Midgard directly."
    };

    let report = StatusReport {
        runtime_state,
        heimdall_connected,
        heimdall_health,
        config_path,
        config_exists,
        runtime: record,
        note: note.to_string(),
    };

    if as_json {
        return serde_json::to_string_pretty(&report)
            .map_err(|err| BifrostError::ConfigSerialize(err.to_string()));
    }

    let daemon_summary = if let Some(ref health) = report.heimdall_health {
        format!(
            "connected (PID {}, uptime {}s, checks {}, ADB devices {})",
            health.daemon_pid,
            health.uptime_secs,
            health.health_checks_completed,
            health.adb_device_count
        )
    } else {
        "disconnected".to_string()
    };

    let mut out = format!(
        "Runtime State:      {}\nHeimdall Daemon:    {}\nConfig Path:        {} ({})",
        report.runtime_state,
        daemon_summary,
        report.config_path.display(),
        if report.config_exists {
            "found"
        } else {
            "not initialized; using built-in defaults"
        }
    );

    if let Some(pid) = report.runtime.pid {
        out.push_str(&format!("\nVM PID:             {pid}"));
    }
    if let Some(ref backend) = report.runtime.backend {
        out.push_str(&format!("\nHypervisor Backend: {backend}"));
    }
    if let Some(ref image_id) = report.runtime.image_id {
        out.push_str(&format!("\nImage ID:           {image_id}"));
    }
    if let Some(ref adb) = report.runtime.adb_endpoint {
        out.push_str(&format!("\nADB Endpoint:       {adb} (loopback)"));
    }
    if let Some(ref serial) = report.runtime.serial_log_path {
        out.push_str(&format!("\nSerial Log:         {}", serial.display()));
    }
    out.push_str(&format!("\nNote:               {}", report.note));
    Ok(out)
}

async fn handle_devices(explicit_config: Option<&PathBuf>, as_json: bool) -> Result<String> {
    let (_, _, _, paths) = load_effective_config_and_paths(explicit_config)?;
    let client = heimdall_client_for(&paths);

    let (via_heimdall, devices) = match client.send::<Vec<AdbDevice>>(IpcRequest::DevicesList).await
    {
        Ok(list) => (true, list),
        Err(_) => (false, AdbClient::new().list_devices()?),
    };

    if as_json {
        let payload = serde_json::json!({
            "via_heimdall": via_heimdall,
            "count": devices.len(),
            "devices": devices,
        });
        return serde_json::to_string_pretty(&payload)
            .map_err(|err| BifrostError::ConfigSerialize(err.to_string()));
    }

    let source = if via_heimdall {
        "Heimdall Bridge IPC"
    } else {
        "local ADB client"
    };
    if devices.is_empty() {
        return Ok(format!("No Android devices detected (via {source})."));
    }

    let mut out = format!("Connected Android Devices (via {source}):\n");
    for d in &devices {
        let model = d.model.as_deref().unwrap_or("unknown");
        let product = d.product.as_deref().unwrap_or("unknown");
        out.push_str(&format!(
            "  - {:<22} [{}] model={} product={}\n",
            d.serial, d.state, model, product
        ));
    }
    Ok(out.trim_end().to_string())
}

async fn handle_daemon(
    action: &DaemonAction,
    explicit_config: Option<&PathBuf>,
    as_json: bool,
) -> Result<String> {
    let (_, _, _, paths) = load_effective_config_and_paths(explicit_config)?;
    let client = heimdall_client_for(&paths);

    match action {
        DaemonAction::Start => {
            if let Ok(health) = client.health().await {
                if as_json {
                    return serde_json::to_string_pretty(&health)
                        .map_err(|err| BifrostError::ConfigSerialize(err.to_string()));
                }
                return Ok(format!(
                    "Heimdall daemon is already running (PID {}, pipe '{}').",
                    health.daemon_pid, health.pipe_name
                ));
            }

            let heimdall_bin = locate_heimdall_binary()?;
            let mut cmd = std::process::Command::new(&heimdall_bin);
            cmd.arg("run")
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null());
            if let Some(cfg_path) = explicit_config {
                cmd.arg("--config").arg(cfg_path);
            }
            #[cfg(windows)]
            {
                use std::os::windows::process::CommandExt;
                const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
                const CREATE_NO_WINDOW: u32 = 0x0800_0000;
                cmd.creation_flags(CREATE_NEW_PROCESS_GROUP | CREATE_NO_WINDOW);
            }
            let _child = cmd.spawn().map_err(|source| BifrostError::Io {
                context: format!("spawning Heimdall daemon '{}'", heimdall_bin.display()),
                source,
            })?;

            // Poll until Heimdall's authenticated Named Pipe is online.
            let mut started_health = None;
            for _ in 0..20 {
                tokio::time::sleep(std::time::Duration::from_millis(250)).await;
                if let Ok(h) = client.health().await {
                    started_health = Some(h);
                    break;
                }
            }
            let health = started_health.ok_or_else(|| {
                BifrostError::Ipc(
                    "Heimdall daemon was spawned but did not become reachable over IPC within 5s"
                        .to_string(),
                )
            })?;
            if as_json {
                return serde_json::to_string_pretty(&health)
                    .map_err(|err| BifrostError::ConfigSerialize(err.to_string()));
            }
            Ok(format!(
                "Started Heimdall daemon in background (PID {}, pipe '{}').",
                health.daemon_pid, health.pipe_name
            ))
        }
        DaemonAction::Status => {
            let health = client.health().await?;
            if as_json {
                return serde_json::to_string_pretty(&health)
                    .map_err(|err| BifrostError::ConfigSerialize(err.to_string()));
            }
            Ok(format!(
                "Heimdall Daemon:    running (PID {})\nPipe Endpoint:      {}\nUptime:             {}s\nRuntime State:      {}\nADB Available:      {} ({} device(s))\nHealth Checks Run:  {}",
                health.daemon_pid,
                health.pipe_name,
                health.uptime_secs,
                health.runtime_state,
                health.adb_available,
                health.adb_device_count,
                health.health_checks_completed
            ))
        }
        DaemonAction::Stop => {
            let resp: serde_json::Value = client.send(IpcRequest::DaemonShutdown).await?;
            if as_json {
                return serde_json::to_string_pretty(&resp)
                    .map_err(|err| BifrostError::ConfigSerialize(err.to_string()));
            }
            Ok("Requested graceful Heimdall daemon shutdown.".to_string())
        }
        DaemonAction::Autostart { enable, disable } => {
            let status = if *enable {
                valhalla::set_heimdall_autostart(true)?
            } else if *disable {
                valhalla::set_heimdall_autostart(false)?
            } else {
                valhalla::query_heimdall_autostart()?
            };
            if as_json {
                return serde_json::to_string_pretty(&status)
                    .map_err(|err| BifrostError::ConfigSerialize(err.to_string()));
            }
            if status.enabled {
                Ok(format!(
                    "Heimdall Windows login autostart: ENABLED (HKCU\\...\\Run\\{})\nCommand: {}",
                    status.value_name,
                    status.command.as_deref().unwrap_or("n/a")
                ))
            } else {
                Ok(format!(
                    "Heimdall Windows login autostart: DISABLED (HKCU\\...\\Run\\{})",
                    status.value_name
                ))
            }
        }
    }
}

async fn handle_runtime(
    action: &RuntimeAction,
    explicit_config: Option<&PathBuf>,
    as_json: bool,
) -> Result<String> {
    let (_, _, config, paths) = load_effective_config_and_paths(explicit_config)?;
    let client = heimdall_client_for(&paths);
    let heimdall_up = client.health().await.is_ok();
    let runtime = MidgardRuntime::with_default_backend(config, paths);

    match action {
        RuntimeAction::Start {
            image,
            headless,
            console,
        } => {
            let record = if heimdall_up && !*console {
                client
                    .send::<RuntimeRecord>(IpcRequest::RuntimeStart {
                        manifest_path: image.clone(),
                        headless: *headless,
                    })
                    .await?
            } else {
                runtime.start(
                    image,
                    StartOptions {
                        headless: *headless,
                        interactive_console: *console,
                    },
                )?
            };
            if as_json {
                return serde_json::to_string_pretty(&record)
                    .map_err(|err| BifrostError::ConfigSerialize(err.to_string()));
            }
            Ok(format!(
                "Started Midgard VM (state: {}, PID: {}, backend: {}, image: {}).\nADB Endpoint: {}\nSerial Log:   {}",
                record.state,
                record.pid.unwrap_or(0),
                record.backend.as_deref().unwrap_or("unknown"),
                record.image_id.as_deref().unwrap_or("unknown"),
                record.adb_endpoint.as_deref().unwrap_or("n/a"),
                record
                    .serial_log_path
                    .as_ref()
                    .map(|p| p.display().to_string())
                    .unwrap_or_else(|| "stdio".to_string())
            ))
        }
        RuntimeAction::Stop { force } => {
            let record = if heimdall_up {
                client
                    .send::<RuntimeRecord>(IpcRequest::RuntimeStop { force: *force })
                    .await?
            } else {
                runtime.stop(*force)?
            };
            if as_json {
                return serde_json::to_string_pretty(&record)
                    .map_err(|err| BifrostError::ConfigSerialize(err.to_string()));
            }
            Ok(format!(
                "Midgard VM stopped (state: {}). {}",
                record.state,
                record.note.as_deref().unwrap_or("")
            ))
        }
        RuntimeAction::Restart { image, headless } => {
            let record = if heimdall_up {
                client
                    .send::<RuntimeRecord>(IpcRequest::RuntimeRestart {
                        manifest_path: image.clone(),
                        headless: *headless,
                    })
                    .await?
            } else {
                runtime.restart(
                    image.as_deref(),
                    StartOptions {
                        headless: *headless,
                        interactive_console: false,
                    },
                )?
            };
            if as_json {
                return serde_json::to_string_pretty(&record)
                    .map_err(|err| BifrostError::ConfigSerialize(err.to_string()));
            }
            Ok(format!(
                "Restarted Midgard VM (state: {}, PID: {}, image: {}).",
                record.state,
                record.pid.unwrap_or(0),
                record.image_id.as_deref().unwrap_or("unknown")
            ))
        }
        RuntimeAction::Verify { image } => {
            let bundle = runtime.verify_image(image)?;
            if as_json {
                return serde_json::to_string_pretty(&bundle)
                    .map_err(|err| BifrostError::ConfigSerialize(err.to_string()));
            }
            Ok(format!(
                "Verified Midgard bundle '{}' (Android {}, arch: {}) — all SHA-256 digests match.",
                bundle.id, bundle.android_version, bundle.arch
            ))
        }
    }
}

async fn handle_logs(
    explicit_config: Option<&PathBuf>,
    tail: usize,
    as_json: bool,
) -> Result<String> {
    let (_, _, config, paths) = load_effective_config_and_paths(explicit_config)?;
    let client = heimdall_client_for(&paths);
    let logs = match client
        .send::<RuntimeLogs>(IpcRequest::RuntimeLogs { tail })
        .await
    {
        Ok(l) => l,
        Err(_) => {
            let runtime = MidgardRuntime::with_default_backend(config, paths);
            runtime.read_logs(tail)?
        }
    };

    if as_json {
        return serde_json::to_string_pretty(&logs)
            .map_err(|err| BifrostError::ConfigSerialize(err.to_string()));
    }

    let mut out = format!(
        "=== Serial Console Log ({}) ===\n",
        logs.serial_log_path.display()
    );
    if logs.serial_lines.is_empty() {
        out.push_str("(no serial log output recorded yet)\n");
    } else {
        for line in &logs.serial_lines {
            out.push_str(line);
            out.push('\n');
        }
    }

    if !logs.vmm_stderr_lines.is_empty() {
        out.push_str(&format!(
            "\n=== VMM Stderr Log ({}) ===\n",
            logs.vmm_stderr_path.display()
        ));
        for line in &logs.vmm_stderr_lines {
            out.push_str(line);
            out.push('\n');
        }
    }

    Ok(out.trim_end().to_string())
}

fn handle_config(
    action: &ConfigAction,
    explicit_config: Option<&PathBuf>,
    as_json: bool,
) -> Result<String> {
    match action {
        ConfigAction::Init { output, force } => {
            let target = resolve_config_path(output.as_ref().or(explicit_config))?;
            let config = BifrostConfig::default();
            config.save_to_file(&target, *force)?;
            if as_json {
                let payload = serde_json::json!({
                    "initialized": true,
                    "path": target,
                });
                return serde_json::to_string_pretty(&payload)
                    .map_err(|err| BifrostError::ConfigSerialize(err.to_string()));
            }
            Ok(format!(
                "Initialized default BIFRÖST configuration at '{}'.",
                target.display()
            ))
        }
        ConfigAction::Show => {
            let config_path = resolve_config_path(explicit_config)?;
            let config = if config_path.exists() {
                BifrostConfig::load_from_file(&config_path)?
            } else {
                BifrostConfig::default()
            };

            if as_json {
                return serde_json::to_string_pretty(&config)
                    .map_err(|err| BifrostError::ConfigSerialize(err.to_string()));
            }
            config.to_toml_pretty()
        }
        ConfigAction::Validate => {
            let config_path = resolve_config_path(explicit_config)?;
            let config = BifrostConfig::load_from_file(&config_path)?;
            config.validate()?;
            if as_json {
                let payload = serde_json::json!({
                    "valid": true,
                    "path": config_path,
                    "schema_version": config.schema_version,
                });
                return serde_json::to_string_pretty(&payload)
                    .map_err(|err| BifrostError::ConfigSerialize(err.to_string()));
            }
            Ok(format!(
                "Configuration '{}' is valid (schema v{}).",
                config_path.display(),
                config.schema_version
            ))
        }
        ConfigAction::Autotune => {
            let (config_path, _, mut config, paths) =
                load_effective_config_and_paths(explicit_config)?;
            let doctor = run_doctor(&config, &paths.data_dir);
            let (cpu_cores, memory_mb) = bifrost_core::config::RuntimeConfig::recommend_for_host(
                doctor.snapshot.cpu.logical_cores,
                doctor.snapshot.memory.total_bytes,
            );
            config.runtime.cpu_cores = cpu_cores;
            config.runtime.memory_mb = memory_mb;
            config.save_to_file(&config_path, true)?;
            if as_json {
                return serde_json::to_string_pretty(&serde_json::json!({
                    "tuned": true,
                    "path": config_path,
                    "cpu_cores": cpu_cores,
                    "memory_mb": memory_mb,
                }))
                .map_err(|err| BifrostError::ConfigSerialize(err.to_string()));
            }
            Ok(format!(
                "Auto-tuned BIFRÖST for this host ({} logical cores, {:.1} GB RAM): set cpu_cores={cpu_cores}, memory_mb={memory_mb} in '{}'. Restart Midgard to apply.",
                doctor.snapshot.cpu.logical_cores,
                doctor.snapshot.memory.total_bytes as f64 / (1024.0 * 1024.0 * 1024.0),
                config_path.display()
            ))
        }
    }
}

async fn handle_apps(
    action: &AppsAction,
    explicit_config: Option<&PathBuf>,
    as_json: bool,
) -> Result<String> {
    let (_, _, _, paths) = load_effective_config_and_paths(explicit_config)?;
    let client = heimdall_client_for(&paths);
    let heimdall_up = client.health().await.is_ok();
    let valhalla = ValhallaManager::new();

    match action {
        AppsAction::List {
            third_party,
            serial,
        } => {
            let packages = if heimdall_up {
                client
                    .send::<Vec<InstalledPackage>>(IpcRequest::AppsList {
                        serial: serial.clone(),
                        third_party_only: *third_party,
                    })
                    .await?
            } else {
                valhalla.list_packages(serial.as_deref(), *third_party)?
            };

            if as_json {
                return serde_json::to_string_pretty(&packages)
                    .map_err(|err| BifrostError::ConfigSerialize(err.to_string()));
            }
            if packages.is_empty() {
                return Ok("No installed Android packages found.".to_string());
            }
            let mut out = format!("Installed Android Packages ({}):\n", packages.len());
            for pkg in &packages {
                let kind = if pkg.is_system { "system" } else { "user" };
                out.push_str(&format!("  - {:<40} [{}]\n", pkg.package_name, kind));
            }
            Ok(out.trim_end().to_string())
        }
        AppsAction::Info { package, serial } => {
            valhalla::validate_package_name(package)?;
            let details = if heimdall_up {
                client
                    .send::<PackageDetails>(IpcRequest::AppInfo {
                        serial: serial.clone(),
                        package: package.clone(),
                    })
                    .await?
            } else {
                valhalla.package_info(serial.as_deref(), package)?
            };

            if as_json {
                return serde_json::to_string_pretty(&details)
                    .map_err(|err| BifrostError::ConfigSerialize(err.to_string()));
            }
            Ok(format!(
                "Package:          {}\nVersion Name:     {}\nVersion Code:     {}\nPrimary CPU ABI:  {}\nLauncher:         {}\nCode Path:        {}",
                details.package_name,
                details.version_name.as_deref().unwrap_or("unknown"),
                details.version_code.as_deref().unwrap_or("unknown"),
                details.primary_cpu_abi.as_deref().unwrap_or("pure ART / none"),
                details.launcher_component.as_deref().unwrap_or("none"),
                details.code_path.as_deref().unwrap_or("unknown")
            ))
        }
        AppsAction::Launch {
            package,
            serial,
            window,
            width,
            height,
            dpi,
            no_autostart,
        } => {
            valhalla::validate_package_name(package)?;
            if *window {
                let ctrl = DisplayController::new();
                let target = match valhalla.resolve_target_serial(serial.as_deref()) {
                    Ok(t) => t,
                    Err(_) if !*no_autostart && serial.is_none() => {
                        let (_, _, config, paths) =
                            load_effective_config_and_paths(explicit_config)?;
                        let adb_endpoint = format!(
                            "{}:{}",
                            config.network.adb_bind_address, config.network.adb_port
                        );
                        let rt = MidgardRuntime::with_default_backend(config, paths.clone());
                        if !rt.status()?.state.is_active() {
                            let manifest = rt.resolve_default_manifest()?;
                            if heimdall_up {
                                let _: RuntimeRecord = client
                                    .send(IpcRequest::RuntimeStart {
                                        manifest_path: manifest,
                                        headless: true,
                                    })
                                    .await?;
                            } else {
                                let _ = rt.ensure_started_headless()?;
                            }
                        }
                        ctrl.wait_for_device_ready(
                            &adb_endpoint,
                            std::time::Duration::from_secs(90),
                        )?
                    }
                    Err(err) => return Err(err),
                };
                let handle = ctrl.launch_in_independent_window(
                    &target,
                    package,
                    AppWindowSpec {
                        width: *width,
                        height: *height,
                        dpi: *dpi,
                    },
                )?;
                if as_json {
                    return serde_json::to_string_pretty(&handle)
                        .map_err(|err| BifrostError::ConfigSerialize(err.to_string()));
                }
                return Ok(format!(
                    "Launched '{}' in independent Windows 11 window '{}' (PID {}, {}x{}@{}dpi).",
                    handle.package_name,
                    handle.window_title,
                    handle.pid,
                    handle.spec.width,
                    handle.spec.height,
                    handle.spec.dpi
                ));
            }

            let outcome = if heimdall_up {
                client
                    .send::<LaunchOutcome>(IpcRequest::AppLaunch {
                        serial: serial.clone(),
                        package: package.clone(),
                    })
                    .await?
            } else {
                valhalla.launch_app(serial.as_deref(), package)?
            };

            if as_json {
                return serde_json::to_string_pretty(&outcome)
                    .map_err(|err| BifrostError::ConfigSerialize(err.to_string()));
            }
            Ok(format!(
                "Launched '{}' on '{}' (component: {}).",
                outcome.package_name,
                outcome.serial,
                outcome.component.as_deref().unwrap_or("launcher intent")
            ))
        }
        AppsAction::Stop { package, serial } => {
            valhalla::validate_package_name(package)?;
            if heimdall_up {
                let _: serde_json::Value = client
                    .send(IpcRequest::AppStop {
                        serial: serial.clone(),
                        package: package.clone(),
                    })
                    .await?;
            } else {
                valhalla.force_stop(serial.as_deref(), package)?;
            }

            if as_json {
                let payload = serde_json::json!({ "package": package, "stopped": true });
                return serde_json::to_string_pretty(&payload)
                    .map_err(|err| BifrostError::ConfigSerialize(err.to_string()));
            }
            Ok(format!("Force-stopped Android package '{package}'."))
        }
        AppsAction::Shortcut {
            package,
            name,
            apk,
            remove,
        } => {
            let mgr = valhalla::ShortcutManager::new(paths.data_dir.clone());
            if *remove {
                let removed = mgr.remove_for_app(package)?;
                if as_json {
                    return serde_json::to_string_pretty(&serde_json::json!({
                        "package": package,
                        "removed": removed.is_some(),
                    }))
                    .map_err(|err| BifrostError::ConfigSerialize(err.to_string()));
                }
                return Ok(if removed.is_some() {
                    format!("Removed Windows Desktop and Start Menu shortcuts for '{package}'.")
                } else {
                    format!("No recorded Windows shortcuts found for '{package}'.")
                });
            }

            let record = mgr.create_for_app(package, name.as_deref(), apk.as_deref())?;
            if as_json {
                return serde_json::to_string_pretty(&record)
                    .map_err(|err| BifrostError::ConfigSerialize(err.to_string()));
            }
            Ok(format!(
                "Created Windows shortcuts for '{}' ({}):\n  Desktop:    {}\n  Start Menu: {}",
                record.display_name,
                record.package_name,
                record.desktop_shortcut.display(),
                record.start_menu_shortcut.display()
            ))
        }
    }
}

async fn handle_install(
    apk: &PathBuf,
    serial: Option<&str>,
    inspect_only: bool,
    explicit_config: Option<&PathBuf>,
    as_json: bool,
) -> Result<String> {
    // Reject unsafe paths (traversal / wrong extension) locally before touching IPC or ADB.
    let safe_apk = valhalla::validate_package_file_path(apk)?;

    if inspect_only {
        let info = inspect_apk(&safe_apk)?;
        if as_json {
            return serde_json::to_string_pretty(&info)
                .map_err(|err| BifrostError::ConfigSerialize(err.to_string()));
        }
        let abi_summary = match &info.abi_compatibility {
            AbiCompatibility::PureArt => "SUPPORTED (Pure ART / DEX — no native .so libraries)".to_string(),
            AbiCompatibility::NativeX86_64 { abis } => {
                format!("SUPPORTED (x86_64 native libs present; all ABIs: {})", abis.join(", "))
            }
            AbiCompatibility::UnsupportedNativeAbi { abis } => format!(
                "UNSUPPORTED (only [{}] native libs; ARM64 translation is unsupported per ADR-0004)",
                abis.join(", ")
            ),
        };
        let bundle_line = if info.is_split_bundle {
            format!(
                "\nSplit Bundle:      yes ({} split APKs)",
                info.split_apk_count
            )
        } else {
            String::new()
        };
        return Ok(format!(
            "Package Path:      {}\nPackage Name:      {}{}\nSize:              {} bytes\nSHA-256:           {}\nZIP Entries:       {}\nAndroidManifest:   {}\nDEX Bytecode:      {}\nABI Compatibility: {}",
            info.path.display(),
            info.package_name.as_deref().unwrap_or("unknown"),
            bundle_line,
            info.size_bytes,
            info.sha256,
            info.entry_count,
            info.has_android_manifest,
            info.has_dex,
            abi_summary
        ));
    }

    let (_, _, config, paths) = load_effective_config_and_paths(explicit_config)?;
    let client = heimdall_client_for(&paths);
    let heimdall_up = client.health().await.is_ok();

    // Ensure Midgard is booted headlessly if installing via double-click while stopped.
    if serial.is_none() && ValhallaManager::new().resolve_target_serial(None).is_err() {
        let inspection = inspect_apk(&safe_apk)?;
        if !inspection.abi_compatibility.is_supported() {
            return Err(BifrostError::UnsupportedAbi {
                path: inspection.path,
                detected_abis: inspection.detected_abis.join(", "),
            });
        }
        let rt = MidgardRuntime::with_default_backend(config.clone(), paths.clone());
        if let Ok(manifest) = rt.resolve_default_manifest() {
            if !rt.status()?.state.is_active() {
                if heimdall_up {
                    let _: RuntimeRecord = client
                        .send(IpcRequest::RuntimeStart {
                            manifest_path: manifest,
                            headless: true,
                        })
                        .await?;
                } else {
                    let _ = rt.ensure_started_headless()?;
                }
            }
            let endpoint = format!(
                "{}:{}",
                config.network.adb_bind_address, config.network.adb_port
            );
            let _ = DisplayController::new()
                .wait_for_device_ready(&endpoint, std::time::Duration::from_secs(90))?;
        }
    }

    let outcome = if heimdall_up {
        client
            .send::<InstallOutcome>(IpcRequest::AppInstall {
                serial: serial.map(ToOwned::to_owned),
                apk_path: safe_apk.clone(),
            })
            .await?
    } else {
        ValhallaManager::new().install_apk(serial, &safe_apk)?
    };

    if as_json {
        return serde_json::to_string_pretty(&outcome)
            .map_err(|err| BifrostError::ConfigSerialize(err.to_string()));
    }
    let mut msg = format!(
        "Installed '{}' onto '{}' (SHA-256: {}).",
        outcome.inspection.path.display(),
        outcome.serial,
        outcome.inspection.sha256
    );
    if let Some(ref sc) = outcome.shortcuts {
        msg.push_str(&format!(
            "\nDesktop Shortcut:    {}\nStart Menu Shortcut: {}",
            sc.desktop_shortcut.display(),
            sc.start_menu_shortcut.display()
        ));
    }
    Ok(msg)
}

async fn handle_uninstall(
    package: &str,
    serial: Option<&str>,
    keep_data: bool,
    explicit_config: Option<&PathBuf>,
    as_json: bool,
) -> Result<String> {
    // Validate Android package identifier locally before touching IPC or ADB.
    valhalla::validate_package_name(package)?;
    let (_, _, _, paths) = load_effective_config_and_paths(explicit_config)?;
    let client = heimdall_client_for(&paths);
    let output = if client.health().await.is_ok() {
        let resp: serde_json::Value = client
            .send(IpcRequest::AppUninstall {
                serial: serial.map(ToOwned::to_owned),
                package: package.to_string(),
                keep_data,
            })
            .await?;
        resp["output"].as_str().unwrap_or("Success").to_string()
    } else {
        ValhallaManager::new().uninstall_package(serial, package, keep_data)?
    };

    if as_json {
        let payload = serde_json::json!({
            "package": package,
            "keep_data": keep_data,
            "output": output,
        });
        return serde_json::to_string_pretty(&payload)
            .map_err(|err| BifrostError::ConfigSerialize(err.to_string()));
    }
    Ok(format!("Uninstalled '{package}' ({output})."))
}

fn handle_share(
    action: &ShareAction,
    explicit_config: Option<&PathBuf>,
    as_json: bool,
) -> Result<String> {
    let (_, _, _, paths) = load_effective_config_and_paths(explicit_config)?;
    let share_root = paths.data_dir.join("shared");
    let adb = AdbClient::new();
    let bridge = SharedDirectoryBridge::new(&share_root, adb);

    match action {
        ShareAction::List => {
            let files = bridge.list_host_files()?;
            if as_json {
                let payload = serde_json::json!({
                    "host_root": share_root,
                    "entries": files,
                });
                return serde_json::to_string_pretty(&payload)
                    .map_err(|err| BifrostError::ConfigSerialize(err.to_string()));
            }
            if files.is_empty() {
                return Ok(format!(
                    "Host shared directory '{}' is empty.",
                    share_root.display()
                ));
            }
            let mut out = format!("Shared Files ({}):\n", share_root.display());
            for entry in &files {
                let kind = if entry.is_dir { "dir" } else { "file" };
                out.push_str(&format!(
                    "  - {:<36} [{}] {} bytes\n",
                    entry.relative_path, kind, entry.size_bytes
                ));
            }
            Ok(out.trim_end().to_string())
        }
        ShareAction::Push { path, serial } => {
            let target = ValhallaManager::new().resolve_target_serial(serial.as_deref())?;
            let out = bridge.push_to_guest(&target, path)?;
            if as_json {
                return serde_json::to_string_pretty(&serde_json::json!({
                    "pushed": path,
                    "serial": target,
                    "output": out.trim(),
                }))
                .map_err(|err| BifrostError::ConfigSerialize(err.to_string()));
            }
            Ok(format!(
                "Pushed '{path}' to '{target}' (/sdcard/BifrostShare)."
            ))
        }
        ShareAction::Pull { path, serial } => {
            let target = ValhallaManager::new().resolve_target_serial(serial.as_deref())?;
            let dest = bridge.pull_from_guest(&target, path)?;
            if as_json {
                return serde_json::to_string_pretty(&serde_json::json!({
                    "pulled": path,
                    "serial": target,
                    "host_path": dest,
                }))
                .map_err(|err| BifrostError::ConfigSerialize(err.to_string()));
            }
            Ok(format!(
                "Pulled '{path}' from '{target}' into '{}'.",
                dest.display()
            ))
        }
    }
}

fn handle_clipboard(action: &ClipboardAction, as_json: bool) -> Result<String> {
    match action {
        ClipboardAction::Get => {
            let text = read_host_clipboard_text()?;
            if as_json {
                return serde_json::to_string_pretty(&serde_json::json!({
                    "has_text": text.is_some(),
                    "length": text.as_ref().map_or(0, String::len),
                    "text": text,
                }))
                .map_err(|err| BifrostError::ConfigSerialize(err.to_string()));
            }
            match text {
                Some(t) => Ok(t),
                None => Ok("(Windows clipboard does not currently contain text)".to_string()),
            }
        }
        ClipboardAction::SyncToAndroid { serial } => {
            let target = ValhallaManager::new().resolve_target_serial(serial.as_deref())?;
            let bytes = sync_clipboard_to_android(&AdbClient::new(), &target)?;
            if as_json {
                return serde_json::to_string_pretty(&serde_json::json!({
                    "synced": true,
                    "serial": target,
                    "bytes": bytes,
                }))
                .map_err(|err| BifrostError::ConfigSerialize(err.to_string()));
            }
            Ok(format!(
                "Synced {bytes} bytes of clipboard text to '{target}'."
            ))
        }
    }
}

fn handle_notifications(action: &NotificationsAction, as_json: bool) -> Result<String> {
    match action {
        NotificationsAction::List { serial } => {
            let target = ValhallaManager::new().resolve_target_serial(serial.as_deref())?;
            let list = list_active_notifications(&AdbClient::new(), &target)?;
            if as_json {
                return serde_json::to_string_pretty(&list)
                    .map_err(|err| BifrostError::ConfigSerialize(err.to_string()));
            }
            if list.is_empty() {
                return Ok(format!("No active notifications on '{target}'."));
            }
            let mut out = format!(
                "Active Android Notifications on '{target}' ({}):\n",
                list.len()
            );
            for n in &list {
                let title = n.title.as_deref().unwrap_or("(no title)");
                let text = n.text.as_deref().unwrap_or("");
                out.push_str(&format!("  - [{}] {}: {}\n", n.package_name, title, text));
            }
            Ok(out.trim_end().to_string())
        }
    }
}

fn handle_processes(action: &ProcessesAction, as_json: bool) -> Result<String> {
    let mgr = ProcessManager::new();
    match action {
        ProcessesAction::List { filter, serial } => {
            let target = mgr.resolve_serial(serial.as_deref())?;
            let list = mgr.list_processes(&target, filter.as_deref())?;
            if as_json {
                return serde_json::to_string_pretty(&list)
                    .map_err(|err| BifrostError::ConfigSerialize(err.to_string()));
            }
            if list.is_empty() {
                return Ok(format!("No matching processes on '{target}'."));
            }
            let mut out = format!("Android Processes on '{target}' ({}):\n", list.len());
            for p in list.iter().take(50) {
                out.push_str(&format!(
                    "  PID {:<7} {:<12} {:>8} KB  {}\n",
                    p.pid, p.user, p.rss_kb, p.name
                ));
            }
            Ok(out.trim_end().to_string())
        }
    }
}

fn handle_network(action: &NetworkAction, as_json: bool) -> Result<String> {
    let mgr = NetworkManager::new();
    match action {
        NetworkAction::Forwards { serial } => {
            let rules = mgr.list_forwards(serial.as_deref())?;
            if as_json {
                return serde_json::to_string_pretty(&rules)
                    .map_err(|err| BifrostError::ConfigSerialize(err.to_string()));
            }
            if rules.is_empty() {
                return Ok("No active ADB loopback TCP port forwards.".to_string());
            }
            let mut out = format!("Active Loopback TCP Port Forwards ({}):\n", rules.len());
            for r in &rules {
                out.push_str(&format!(
                    "  - {} : {} -> {}\n",
                    r.serial, r.local_spec, r.remote_spec
                ));
            }
            Ok(out.trim_end().to_string())
        }
        NetworkAction::Forward {
            local_port,
            guest_port,
            serial,
        } => {
            let target = mgr.resolve_serial(serial.as_deref())?;
            let rule = mgr.add_forward(&target, *local_port, *guest_port)?;
            if as_json {
                return serde_json::to_string_pretty(&rule)
                    .map_err(|err| BifrostError::ConfigSerialize(err.to_string()));
            }
            Ok(format!(
                "Forwarded 127.0.0.1:{} -> {}:{} on '{}'.",
                local_port, rule.remote_spec, guest_port, rule.serial
            ))
        }
        NetworkAction::Unforward { local_port, serial } => {
            let target = mgr.resolve_serial(serial.as_deref())?;
            mgr.remove_forward(&target, *local_port)?;
            if as_json {
                return serde_json::to_string_pretty(&serde_json::json!({
                    "removed": true,
                    "serial": target,
                    "local_port": local_port,
                }))
                .map_err(|err| BifrostError::ConfigSerialize(err.to_string()));
            }
            Ok(format!(
                "Removed loopback forward on port {local_port} for '{target}'."
            ))
        }
    }
}

fn handle_images(
    action: &ImagesAction,
    explicit_config: Option<&PathBuf>,
    as_json: bool,
) -> Result<String> {
    let (_, _, config, paths) = load_effective_config_and_paths(explicit_config)?;
    let mut registry = ImageRegistry::load(&paths.images_dir)?;

    match action {
        ImagesAction::List => {
            if as_json {
                return serde_json::to_string_pretty(&registry)
                    .map_err(|err| BifrostError::ConfigSerialize(err.to_string()));
            }
            if registry.images.is_empty() {
                return Ok("No Midgard runtime images registered yet.".to_string());
            }
            let mut out = format!(
                "Registered Midgard Runtime Images ({}):\n",
                registry.images.len()
            );
            for img in &registry.images {
                let marker = if registry.active_image_id.as_deref() == Some(img.id.as_str()) {
                    "* "
                } else {
                    "  "
                };
                let sig = if img.signature_verified {
                    "signed:ed25519"
                } else {
                    "sha256-only"
                };
                out.push_str(&format!(
                    "{marker}{:<24} Android {:<6} [{} | compat v{}] {}\n",
                    img.id,
                    img.android_version,
                    sig,
                    img.userdata_compat_version,
                    img.manifest_path.display()
                ));
            }
            Ok(out.trim_end().to_string())
        }
        ImagesAction::Register {
            manifest,
            signature,
            allow_unsigned,
            userdata_compat,
        } => {
            let trust = evaluate_manifest_trust(
                manifest,
                signature.as_deref(),
                &config.images.trusted_keys,
                *allow_unsigned,
            )?;
            let signature_verified = trust.is_signed();
            if let ImageTrust::UnsignedAllowed = trust {
                tracing::warn!(
                    manifest = %manifest.display(),
                    "registering UNSIGNED Midgard image (sha256-only) by explicit --allow-unsigned"
                );
            }

            let bundle = verify_manifest_file(manifest)?;
            let record = registry.register_bundle(&bundle, signature_verified, *userdata_compat)?;
            registry.save(&paths.images_dir)?;

            if as_json {
                return serde_json::to_string_pretty(&record)
                    .map_err(|err| BifrostError::ConfigSerialize(err.to_string()));
            }
            Ok(format!(
                "Registered verified Midgard image '{}' (Android {}, signature_verified={}).",
                record.id, record.android_version, record.signature_verified
            ))
        }
        ImagesAction::Use { image_id } => {
            let activated = registry.activate(image_id)?;
            registry.save(&paths.images_dir)?;

            if as_json {
                return serde_json::to_string_pretty(&activated)
                    .map_err(|err| BifrostError::ConfigSerialize(err.to_string()));
            }
            Ok(format!(
                "Active Midgard runtime set to '{}' (Android {}; persistent userdata preserved).",
                activated.id, activated.android_version
            ))
        }
    }
}

async fn handle_recovery(
    action: &RecoveryAction,
    explicit_config: Option<&PathBuf>,
    as_json: bool,
) -> Result<String> {
    let (config_path, config_exists, config, paths) =
        load_effective_config_and_paths(explicit_config)?;
    let userdata_path = paths
        .data_dir
        .join("instances")
        .join("default")
        .join("userdata.img");

    match action {
        RecoveryAction::Backup { output } => {
            let meta = backup_userdata(&userdata_path, output)?;
            if as_json {
                return serde_json::to_string_pretty(&meta)
                    .map_err(|err| BifrostError::ConfigSerialize(err.to_string()));
            }
            Ok(format!(
                "Backed up userdata ({} bytes, sha256: {}) to '{}'.",
                meta.size_bytes,
                meta.sha256,
                output.display()
            ))
        }
        RecoveryAction::Restore { from, confirm } => {
            let meta = restore_userdata(from, &userdata_path, *confirm)?;
            if as_json {
                return serde_json::to_string_pretty(&meta)
                    .map_err(|err| BifrostError::ConfigSerialize(err.to_string()));
            }
            Ok(format!(
                "Restored verified userdata (sha256: {}) into '{}'.",
                meta.sha256,
                userdata_path.display()
            ))
        }
        RecoveryAction::WipeUserdata { confirm } => {
            wipe_userdata_disk(&userdata_path, *confirm)?;
            if as_json {
                return serde_json::to_string_pretty(&serde_json::json!({
                    "wiped": true,
                    "path": userdata_path,
                }))
                .map_err(|err| BifrostError::ConfigSerialize(err.to_string()));
            }
            Ok(format!(
                "Wiped persistent userdata disk '{}'.",
                userdata_path.display()
            ))
        }
        RecoveryAction::ExportDiagnostics { output } => {
            let safe_out = validate_host_file_path(output, Some("json"))?;
            let doctor = run_doctor(&config, &paths.data_dir);
            let runtime_mgr = MidgardRuntime::with_default_backend(config.clone(), paths.clone());
            let runtime_status = runtime_mgr.status()?;
            let logs = runtime_mgr.read_logs(100)?;
            let registry = ImageRegistry::load(&paths.images_dir)?;
            let devices = AdbClient::new().list_devices().unwrap_or_default();
            let heimdall_health = heimdall_client_for(&paths).health().await.ok();

            let bundle = serde_json::json!({
                "schema_version": 1,
                "version": VersionInfo::current(),
                "config_path": config_path,
                "config_exists": config_exists,
                "config": config,
                "doctor": doctor,
                "heimdall_health": heimdall_health,
                "runtime_status": runtime_status,
                "image_registry": registry,
                "adb_devices": devices,
                "recent_logs": logs,
            });

            if let Some(parent) = safe_out.parent() {
                if !parent.as_os_str().is_empty() {
                    std::fs::create_dir_all(parent).map_err(|source| BifrostError::Io {
                        context: format!("creating diagnostics output dir '{}'", parent.display()),
                        source,
                    })?;
                }
            }
            let serialized = serde_json::to_string_pretty(&bundle)
                .map_err(|err| BifrostError::ConfigSerialize(err.to_string()))?;
            std::fs::write(&safe_out, serialized).map_err(|source| BifrostError::Io {
                context: format!("writing diagnostic bundle '{}'", safe_out.display()),
                source,
            })?;

            if as_json {
                return serde_json::to_string_pretty(&serde_json::json!({
                    "exported": true,
                    "path": safe_out,
                }))
                .map_err(|err| BifrostError::ConfigSerialize(err.to_string()));
            }
            Ok(format!(
                "Exported sanitized BIFRÖST diagnostic bundle to '{}'.",
                safe_out.display()
            ))
        }
    }
}

fn handle_doctor(
    explicit_config: Option<&PathBuf>,
    as_json: bool,
    strict: bool,
) -> Result<CommandOutput> {
    let (_, _, config, paths) = load_effective_config_and_paths(explicit_config)?;
    let report = run_doctor(&config, &paths.data_dir);
    let exit_code = if strict && report.overall == OverallReadiness::NotReady {
        1
    } else {
        0
    };

    let stdout = if as_json {
        serde_json::to_string_pretty(&report)
            .map_err(|err| BifrostError::ConfigSerialize(err.to_string()))?
    } else {
        render_human_report(&report)
    };

    Ok(CommandOutput { stdout, exit_code })
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
