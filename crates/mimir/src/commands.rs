//! Command handlers for Mimir (`bifrost`).
//!
//! Unimplemented commands always return `BifrostError::Unsupported` and never
//! simulate successful runtime operations.

use std::path::PathBuf;

use bifrost_bridge::{
    default_pipe_name, AdbClient, AdbDevice, DaemonHealthReport, HeimdallClient, IpcRequest,
};
use bifrost_core::{
    validate_host_file_path, BifrostConfig, BifrostError, BifrostPaths, Result, RuntimeState,
    VersionInfo,
};
use bifrost_diagnostics::{render_human_report, run_doctor, OverallReadiness};
use bifrost_runtime::{MidgardRuntime, RuntimeLogs, RuntimeRecord, StartOptions};
use serde::Serialize;

use crate::cli::{AppsAction, Cli, Command, ConfigAction, DaemonAction, RuntimeAction};

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
        Command::Apps { action } => {
            let sub = match action {
                AppsAction::List => "bifrost apps list",
            };
            Err(BifrostError::Unsupported {
                feature: sub.to_string(),
                planned_milestone: "Planned in Milestone 5 — Valhalla".to_string(),
            })
        }
        Command::Install { apk } => {
            validate_host_file_path(apk, Some("apk"))?;
            Err(BifrostError::Unsupported {
                feature: "bifrost install".to_string(),
                planned_milestone: "Planned in Milestone 5 — Valhalla".to_string(),
            })
        }
        Command::Uninstall { package } => {
            if package.trim().is_empty() {
                return Err(BifrostError::ConfigValidation {
                    field: "package".to_string(),
                    reason: "package name must not be empty".to_string(),
                });
            }
            Err(BifrostError::Unsupported {
                feature: "bifrost uninstall".to_string(),
                planned_milestone: "Planned in Milestone 5 — Valhalla".to_string(),
            })
        }
        Command::Logs { tail } => handle_logs(cli.config.as_ref(), *tail, cli.json)
            .await
            .map(CommandOutput::ok),
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

    let record = if heimdall_connected {
        client
            .send::<RuntimeRecord>(IpcRequest::RuntimeStatus)
            .await?
    } else {
        let runtime_mgr = MidgardRuntime::with_default_backend(config, paths);
        runtime_mgr.status()?
    };

    let runtime_state = heimdall_health
        .as_ref()
        .map(|h| h.runtime_state)
        .unwrap_or(record.state);

    let note = if heimdall_connected {
        "Connected to Heimdall supervisor over authenticated Windows Named Pipe IPC."
    } else if record.state.is_active() {
        "Midgard VM is active; Heimdall daemon is not running (start with `heimdall run`)."
    } else {
        "Heimdall daemon is offline. Start `heimdall run` for background supervision or launch Midgard directly."
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
        assert_eq!(parsed["checks"].as_array().unwrap().len(), 7);
    }

    #[tokio::test]
    async fn unimplemented_commands_explicitly_return_unsupported() {
        let apps = Cli::try_parse_from(["bifrost", "apps", "list"]).unwrap();
        assert!(execute(&apps).await.unwrap_err().is_unsupported());

        let install = Cli::try_parse_from(["bifrost", "install", "app.apk"]).unwrap();
        assert!(execute(&install).await.unwrap_err().is_unsupported());
    }

    #[tokio::test]
    async fn install_rejects_path_traversal_before_unsupported() {
        let bad = Cli::try_parse_from(["bifrost", "install", "../evil.apk"]).unwrap();
        let err = execute(&bad).await.unwrap_err();
        assert!(matches!(err, BifrostError::UnsafePath { .. }));
    }
}
