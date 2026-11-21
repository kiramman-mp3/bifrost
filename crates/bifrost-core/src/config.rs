//! Versioned TOML configuration model and security/bounds validation for BIFRÖST.

use std::net::IpAddr;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{BifrostError, Result};
use crate::paths::validate_host_file_path;

/// Supported configuration schema version.
pub const CURRENT_SCHEMA_VERSION: u32 = 1;

/// Graphics rendering backend preference for Midgard.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum GraphicsMode {
    /// Prefer hardware-accelerated `virtio-gpu` + `gfxstream`, falling back to SwiftShader if unavailable.
    #[default]
    Auto,
    /// Require hardware-accelerated `virtio-gpu` + `gfxstream`.
    Gfxstream,
    /// Force CPU software rendering (SwiftShader) for diagnostics or headless bring-up.
    SwiftShader,
}

/// Structured logging verbosity level.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum LogLevel {
    Error,
    Warn,
    #[default]
    Info,
    Debug,
    Trace,
}

impl LogLevel {
    /// Returns the `tracing` directive string corresponding to this log level.
    #[must_use]
    pub const fn as_directive(self) -> &'static str {
        match self {
            Self::Error => "error",
            Self::Warn => "warn",
            Self::Info => "info",
            Self::Debug => "debug",
            Self::Trace => "trace",
        }
    }
}

/// Top-level BIFRÖST configuration document (`bifrost.toml`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct BifrostConfig {
    pub schema_version: u32,
    pub runtime: RuntimeConfig,
    pub display: DisplayConfig,
    pub network: NetworkConfig,
    pub logging: LoggingConfig,
}

impl Default for BifrostConfig {
    fn default() -> Self {
        Self {
            schema_version: CURRENT_SCHEMA_VERSION,
            runtime: RuntimeConfig::default(),
            display: DisplayConfig::default(),
            network: NetworkConfig::default(),
            logging: LoggingConfig::default(),
        }
    }
}

/// Resource allocation and storage settings for the Midgard Android VM.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct RuntimeConfig {
    pub cpu_cores: u32,
    pub memory_mb: u64,
    pub storage_gb: u64,
    pub graphics_mode: GraphicsMode,
    pub data_dir: Option<PathBuf>,
}

impl Default for RuntimeConfig {
    fn default() -> Self {
        Self {
            cpu_cores: 4,
            memory_mb: 4096,
            storage_gb: 32,
            graphics_mode: GraphicsMode::Auto,
            data_dir: None,
        }
    }
}

/// Default virtual display dimensions and pixel density.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct DisplayConfig {
    pub width: u32,
    pub height: u32,
    pub dpi: u32,
}

impl Default for DisplayConfig {
    fn default() -> Self {
        Self {
            width: 1280,
            height: 720,
            dpi: 240,
        }
    }
}

/// Localhost-by-default networking and ADB transport settings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct NetworkConfig {
    pub adb_bind_address: String,
    pub adb_port: u16,
    /// Security guard: non-loopback ADB binds are rejected unless explicitly opted in.
    pub allow_non_loopback_adb: bool,
}

impl Default for NetworkConfig {
    fn default() -> Self {
        Self {
            adb_bind_address: "127.0.0.1".to_string(),
            adb_port: 5555,
            allow_non_loopback_adb: false,
        }
    }
}

/// Logging output configuration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct LoggingConfig {
    pub level: LogLevel,
    pub json: bool,
}

impl BifrostConfig {
    /// Parses and validates a configuration from a TOML string.
    pub fn from_toml_str(input: &str) -> Result<Self> {
        let cfg: Self =
            toml::from_str(input).map_err(|err| BifrostError::ConfigParse(err.to_string()))?;
        cfg.validate()?;
        Ok(cfg)
    }

    /// Serializes this configuration into a formatted TOML string after validation.
    pub fn to_toml_pretty(&self) -> Result<String> {
        self.validate()?;
        toml::to_string_pretty(self).map_err(|err| BifrostError::ConfigSerialize(err.to_string()))
    }

    /// Loads and validates a TOML configuration file from disk.
    pub fn load_from_file(path: impl AsRef<Path>) -> Result<Self> {
        let safe_path = validate_host_file_path(path, Some("toml"))?;
        let raw = std::fs::read_to_string(&safe_path).map_err(|source| BifrostError::Io {
            context: format!("reading config '{}'", safe_path.display()),
            source,
        })?;
        Self::from_toml_str(&raw)
    }

    /// Saves a validated TOML configuration file to disk, creating parent directories if needed.
    pub fn save_to_file(&self, path: impl AsRef<Path>, overwrite: bool) -> Result<()> {
        let safe_path = validate_host_file_path(path, Some("toml"))?;
        if safe_path.exists() && !overwrite {
            return Err(BifrostError::AlreadyExists { path: safe_path });
        }
        let serialized = self.to_toml_pretty()?;
        if let Some(parent) = safe_path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent).map_err(|source| BifrostError::Io {
                    context: format!("creating config directory '{}'", parent.display()),
                    source,
                })?;
            }
        }
        std::fs::write(&safe_path, serialized).map_err(|source| BifrostError::Io {
            context: format!("writing config '{}'", safe_path.display()),
            source,
        })
    }

    /// Enforces schema, resource bounds, path safety, and security invariants.
    pub fn validate(&self) -> Result<()> {
        if self.schema_version != CURRENT_SCHEMA_VERSION {
            return Err(BifrostError::ConfigValidation {
                field: "schema_version".to_string(),
                reason: format!(
                    "unsupported schema version {} (expected {})",
                    self.schema_version, CURRENT_SCHEMA_VERSION
                ),
            });
        }

        if !(1..=64).contains(&self.runtime.cpu_cores) {
            return Err(BifrostError::ConfigValidation {
                field: "runtime.cpu_cores".to_string(),
                reason: "must be between 1 and 64".to_string(),
            });
        }

        if !(1024..=65_536).contains(&self.runtime.memory_mb) {
            return Err(BifrostError::ConfigValidation {
                field: "runtime.memory_mb".to_string(),
                reason: "must be between 1024 MB and 65536 MB".to_string(),
            });
        }

        if !(8..=1024).contains(&self.runtime.storage_gb) {
            return Err(BifrostError::ConfigValidation {
                field: "runtime.storage_gb".to_string(),
                reason: "must be between 8 GB and 1024 GB".to_string(),
            });
        }

        if let Some(ref data_dir) = self.runtime.data_dir {
            validate_host_file_path(data_dir, None)?;
        }

        if !(320..=7680).contains(&self.display.width) {
            return Err(BifrostError::ConfigValidation {
                field: "display.width".to_string(),
                reason: "must be between 320 and 7680 pixels".to_string(),
            });
        }

        if !(320..=4320).contains(&self.display.height) {
            return Err(BifrostError::ConfigValidation {
                field: "display.height".to_string(),
                reason: "must be between 320 and 4320 pixels".to_string(),
            });
        }

        if !(120..=640).contains(&self.display.dpi) {
            return Err(BifrostError::ConfigValidation {
                field: "display.dpi".to_string(),
                reason: "must be between 120 and 640 DPI".to_string(),
            });
        }

        if self.network.adb_port < 1024 {
            return Err(BifrostError::ConfigValidation {
                field: "network.adb_port".to_string(),
                reason: "must be an unprivileged port (>= 1024)".to_string(),
            });
        }

        let ip: IpAddr =
            self.network
                .adb_bind_address
                .parse()
                .map_err(|_| BifrostError::ConfigValidation {
                    field: "network.adb_bind_address".to_string(),
                    reason: format!(
                        "'{}' is not a valid IP address",
                        self.network.adb_bind_address
                    ),
                })?;

        if !ip.is_loopback() && !self.network.allow_non_loopback_adb {
            return Err(BifrostError::ConfigValidation {
                field: "network.adb_bind_address".to_string(),
                reason: "non-loopback ADB binding is forbidden by default for security (use 127.0.0.1 or ::1)".to_string(),
            });
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn default_config_is_valid_and_roundtrips_toml() {
        let cfg = BifrostConfig::default();
        let toml_text = cfg.to_toml_pretty().unwrap();
        let parsed = BifrostConfig::from_toml_str(&toml_text).unwrap();
        assert_eq!(cfg, parsed);
    }

    #[test]
    fn rejects_public_adb_bind_by_default() {
        let mut cfg = BifrostConfig::default();
        cfg.network.adb_bind_address = "0.0.0.0".to_string();
        let err = cfg.validate().unwrap_err();
        assert!(matches!(
            err,
            BifrostError::ConfigValidation { ref field, .. } if field == "network.adb_bind_address"
        ));
    }

    #[test]
    fn rejects_invalid_memory_and_schema_version() {
        let mut cfg = BifrostConfig::default();
        cfg.runtime.memory_mb = 256;
        assert!(cfg.validate().is_err());

        let bad_schema = BifrostConfig {
            schema_version: 99,
            ..Default::default()
        };
        assert!(bad_schema.validate().is_err());
    }

    #[test]
    fn save_refuses_overwrite_without_explicit_flag() {
        let tmp = TempDir::new().unwrap();
        let file = tmp.path().join("bifrost.toml");
        let cfg = BifrostConfig::default();

        cfg.save_to_file(&file, false).unwrap();
        let err = cfg.save_to_file(&file, false).unwrap_err();
        assert!(matches!(err, BifrostError::AlreadyExists { .. }));

        cfg.save_to_file(&file, true).unwrap();
        let reloaded = BifrostConfig::load_from_file(&file).unwrap();
        assert_eq!(cfg, reloaded);
    }
}
