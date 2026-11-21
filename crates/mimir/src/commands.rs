//! Command handlers for Mimir (`bifrost`).
//!
//! Unimplemented commands always return `BifrostError::Unsupported` and never
//! simulate successful runtime operations.

use std::path::PathBuf;

use bifrost_core::{
    validate_host_file_path, BifrostConfig, BifrostError, BifrostPaths, Result, RuntimeState,
    VersionInfo,
};
use serde::Serialize;

use crate::cli::{AppsAction, Cli, Command, ConfigAction, RuntimeAction};

/// Structured status output returned by `bifrost status`.
#[derive(Debug, Serialize)]
pub struct StatusReport {
    pub runtime_state: RuntimeState,
    pub heimdall_connected: bool,
    pub config_path: PathBuf,
    pub config_exists: bool,
    pub note: String,
}

/// Dispatches a parsed CLI command and returns formatted stdout output on success.
pub fn execute(cli: &Cli) -> Result<String> {
    match &cli.command {
        Command::Version => handle_version(cli.json),
        Command::Status => handle_status(cli.config.as_ref(), cli.json),
        Command::Config { action } => handle_config(action, cli.config.as_ref(), cli.json),
        Command::Doctor => Err(BifrostError::Unsupported {
            feature: "bifrost doctor".to_string(),
            planned_milestone: "Planned in Milestone 2 — Windows Compatibility".to_string(),
        }),
        Command::Runtime { action } => {
            let sub = match action {
                RuntimeAction::Start => "bifrost runtime start",
                RuntimeAction::Stop => "bifrost runtime stop",
                RuntimeAction::Restart => "bifrost runtime restart",
            };
            Err(BifrostError::Unsupported {
                feature: sub.to_string(),
                planned_milestone: "Planned in Milestones 3-4 — Midgard & Heimdall".to_string(),
            })
        }
        Command::Devices => Err(BifrostError::Unsupported {
            feature: "bifrost devices".to_string(),
            planned_milestone: "Planned in Milestone 4 — Heimdall & Bridge".to_string(),
        }),
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
            // Enforce security path validation even before reporting unsupported status.
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
        Command::Logs => Err(BifrostError::Unsupported {
            feature: "bifrost logs".to_string(),
            planned_milestone: "Planned in Milestone 4 — Heimdall & Bridge".to_string(),
        }),
    }
}

fn resolve_config_path(explicit: Option<&PathBuf>) -> Result<PathBuf> {
    if let Some(path) = explicit {
        return validate_host_file_path(path, Some("toml"));
    }
    let paths = BifrostPaths::discover()?;
    Ok(paths.default_config_file())
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

fn handle_status(explicit_config: Option<&PathBuf>, as_json: bool) -> Result<String> {
    let config_path = resolve_config_path(explicit_config)?;
    let config_exists = config_path.exists();

    let report = StatusReport {
        runtime_state: RuntimeState::Stopped,
        heimdall_connected: false,
        config_path,
        config_exists,
        note: "Heimdall daemon and Midgard runtime are not implemented yet (Planned in Milestones 3-4)."
            .to_string(),
    };

    if as_json {
        return serde_json::to_string_pretty(&report)
            .map_err(|err| BifrostError::ConfigSerialize(err.to_string()));
    }

    Ok(format!(
        "Runtime State:      {}\nHeimdall Daemon:    disconnected\nConfig Path:        {} ({})\nNote:               {}",
        report.runtime_state,
        report.config_path.display(),
        if report.config_exists {
            "found"
        } else {
            "not initialized; using built-in defaults"
        },
        report.note
    ))
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

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;
    use tempfile::TempDir;

    #[test]
    fn version_command_returns_json_and_text() {
        let cli = Cli::try_parse_from(["bifrost", "version"]).unwrap();
        let text = execute(&cli).unwrap();
        assert!(text.contains("BIFRÖST"));
        assert!(text.contains("arm64-translation"));

        let cli_json = Cli::try_parse_from(["bifrost", "--json", "version"]).unwrap();
        let json = execute(&cli_json).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["identifier"], "bifrost");
    }

    #[test]
    fn config_init_validate_and_show_work_end_to_end() {
        let tmp = TempDir::new().unwrap();
        let cfg_file = tmp.path().join("bifrost.toml");
        let cfg_str = cfg_file.to_str().unwrap();

        let init_cli =
            Cli::try_parse_from(["bifrost", "config", "init", "--output", cfg_str]).unwrap();
        assert!(execute(&init_cli).is_ok());

        let validate_cli =
            Cli::try_parse_from(["bifrost", "--config", cfg_str, "config", "validate"]).unwrap();
        let validation_msg = execute(&validate_cli).unwrap();
        assert!(validation_msg.contains("is valid"));

        let show_cli =
            Cli::try_parse_from(["bifrost", "--config", cfg_str, "config", "show"]).unwrap();
        let shown = execute(&show_cli).unwrap();
        assert!(shown.contains("adb_bind_address = \"127.0.0.1\""));
    }

    #[test]
    fn status_honestly_reports_stopped_and_disconnected() {
        let tmp = TempDir::new().unwrap();
        let cfg_file = tmp.path().join("bifrost.toml");
        let cli = Cli::try_parse_from([
            "bifrost",
            "--json",
            "--config",
            cfg_file.to_str().unwrap(),
            "status",
        ])
        .unwrap();
        let out = execute(&cli).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert_eq!(parsed["runtime_state"], "stopped");
        assert_eq!(parsed["heimdall_connected"], false);
    }

    #[test]
    fn unimplemented_commands_explicitly_return_unsupported() {
        let doctor = Cli::try_parse_from(["bifrost", "doctor"]).unwrap();
        assert!(execute(&doctor).unwrap_err().is_unsupported());

        let start = Cli::try_parse_from(["bifrost", "runtime", "start"]).unwrap();
        assert!(execute(&start).unwrap_err().is_unsupported());

        let install = Cli::try_parse_from(["bifrost", "install", "app.apk"]).unwrap();
        assert!(execute(&install).unwrap_err().is_unsupported());
    }

    #[test]
    fn install_rejects_path_traversal_before_unsupported() {
        let bad = Cli::try_parse_from(["bifrost", "install", "../evil.apk"]).unwrap();
        let err = execute(&bad).unwrap_err();
        assert!(matches!(err, BifrostError::UnsafePath { .. }));
    }
}
