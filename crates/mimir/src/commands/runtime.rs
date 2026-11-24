//! `bifrost status`, `devices`, `runtime`, `logs` and `doctor`.

use super::*;

/// Structured status output returned by `bifrost status`.
#[derive(Debug, Serialize)]
pub struct StatusReport {
    pub runtime_state: RuntimeState,
    pub heimdall_connected: bool,
    pub heimdall_health: Option<DaemonHealthReport>,
    pub config_path: PathBuf,
    pub config_exists: bool,
    pub runtime: RuntimeRecord,
    /// ADB access to the active guest (`ready`, `unauthorized`, `trade_in_mode`, `offline`).
    pub adb_access: Option<AdbEndpointState>,
    pub note: String,
}

pub(super) async fn handle_status(
    explicit_config: Option<&PathBuf>,
    as_json: bool,
) -> Result<String> {
    let session = Session::open(explicit_config.map(PathBuf::as_path)).await?;
    let heimdall_connected = session.heimdall_connected();
    let (config_path, config_exists) = (
        session.loaded().config_path.clone(),
        session.loaded().config_exists,
    );
    let StatusSnapshot {
        record,
        runtime_state,
        heimdall_health,
        adb_access,
    } = session.status().await?;

    let adb_hint = adb_access.and_then(AdbEndpointState::remediation);
    let note = if let Some(hint) = adb_hint {
        hint
    } else if heimdall_connected {
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
        adb_access,
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

pub(super) async fn handle_devices(
    explicit_config: Option<&PathBuf>,
    as_json: bool,
) -> Result<String> {
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

pub(super) async fn handle_runtime(
    action: &RuntimeAction,
    explicit_config: Option<&PathBuf>,
    as_json: bool,
) -> Result<String> {
    let session = Session::open(explicit_config.map(PathBuf::as_path)).await?;

    match action {
        RuntimeAction::Start {
            image,
            headless,
            console,
        } => {
            let record = session
                .start_runtime(image.clone(), *headless, *console)
                .await?;
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
            let record = session.stop_runtime(*force).await?;
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
            let record = session.restart_runtime(image.clone(), *headless).await?;
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
            let runtime = MidgardRuntime::with_default_backend(
                session.config().clone(),
                session.paths().clone(),
            );
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

pub(super) async fn handle_logs(
    explicit_config: Option<&PathBuf>,
    tail: usize,
    as_json: bool,
) -> Result<String> {
    let session = Session::open(explicit_config.map(PathBuf::as_path)).await?;
    let logs = session.logs(tail).await?;

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

pub(super) fn handle_doctor(
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
