//! `bifrost recovery` (userdata backup, restore and wipe).

use super::*;

pub(super) async fn handle_recovery(
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
