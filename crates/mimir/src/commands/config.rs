//! `bifrost config` (init, show, validate, autotune).

use super::*;

pub(super) fn handle_config(
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
