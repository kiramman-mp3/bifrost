//! `bifrost daemon` (Heimdall start, status, stop and login autostart).

use super::*;

pub(super) async fn handle_daemon(
    action: &DaemonAction,
    explicit_config: Option<&PathBuf>,
    as_json: bool,
) -> Result<String> {
    let (_, _, _, paths) = load_effective_config_and_paths(explicit_config)?;
    let client = heimdall_client_for(&paths);

    match action {
        DaemonAction::Start => {
            let session = Session::open(explicit_config.map(PathBuf::as_path)).await?;
            let started = session
                .start_heimdall(explicit_config.map(PathBuf::as_path))
                .await?;
            let health = started.health();
            if as_json {
                return serde_json::to_string_pretty(health)
                    .map_err(|err| BifrostError::ConfigSerialize(err.to_string()));
            }
            Ok(match started {
                DaemonStart::AlreadyRunning(_) => format!(
                    "Heimdall daemon is already running (PID {}, pipe '{}').",
                    health.daemon_pid, health.pipe_name
                ),
                DaemonStart::Started(_) => format!(
                    "Started Heimdall daemon in background (PID {}, pipe '{}').",
                    health.daemon_pid, health.pipe_name
                ),
            })
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
