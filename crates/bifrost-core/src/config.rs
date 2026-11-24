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
    /// 2D `virtio-gpu-pci`; Android renders with ANGLE on SwiftShader (CPU). See ADR-0007.
    #[default]
    Auto,
    /// Experimental 3D device (`virtio-gpu-gl-pci` on QEMU, `gfxstream` on crosvm). On QEMU for
    /// Windows the guest still renders in software (`llvmpipe`); see ADR-0007.
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
    pub desktop: DesktopConfig,
    pub network: NetworkConfig,
    pub logging: LoggingConfig,
    pub images: ImagesConfig,
}

impl Default for BifrostConfig {
    fn default() -> Self {
        Self {
            schema_version: CURRENT_SCHEMA_VERSION,
            runtime: RuntimeConfig::default(),
            display: DisplayConfig::default(),
            desktop: DesktopConfig::default(),
            network: NetworkConfig::default(),
            logging: LoggingConfig::default(),
            images: ImagesConfig::default(),
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
    pub audio_enabled: bool,
    pub data_dir: Option<PathBuf>,
}

impl RuntimeConfig {
    /// Computes recommended `(cpu_cores, memory_mb)` allocations for a given host
    /// while preserving generous headroom for Windows 11.
    #[must_use]
    pub fn recommend_for_host(logical_cores: u32, total_ram_bytes: u64) -> (u32, u64) {
        const GIB: u64 = 1024 * 1024 * 1024;
        let cpu_cores = if logical_cores >= 12 {
            8
        } else if logical_cores >= 8 {
            6
        } else if logical_cores >= 4 {
            4
        } else {
            2
        };

        let memory_mb = if total_ram_bytes >= 28 * GIB {
            8192
        } else if total_ram_bytes >= 15 * GIB {
            6144
        } else if total_ram_bytes >= 10 * GIB {
            4096
        } else {
            2048
        };

        (cpu_cores, memory_mb)
    }
}

impl Default for RuntimeConfig {
    fn default() -> Self {
        Self {
            cpu_cores: 4,
            memory_mb: 4096,
            storage_gb: 32,
            graphics_mode: GraphicsMode::Auto,
            audio_enabled: true,
            data_dir: None,
        }
    }
}

/// Midgard runtime image trust policy.
///
/// `trusted_keys` is the local trust store of Ed25519 publisher public keys
/// (32-byte lowercase hex). `bifrost images register` only marks a bundle as signed
/// when its manifest signature verifies against one of these keys; unsigned bundles
/// require an explicit per-registration `--allow-unsigned` opt-in.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ImagesConfig {
    pub trusted_keys: Vec<String>,
}

/// Returns `true` when `key` is a 32-byte Ed25519 public key encoded as 64 hex characters.
#[must_use]
pub fn is_valid_ed25519_public_key_hex(key: &str) -> bool {
    key.len() == 64 && key.chars().all(|c| c.is_ascii_hexdigit())
}

/// Windows 11 desktop integration automation settings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct DesktopConfig {
    /// Automatically sync Desktop & Start Menu shortcuts for apps installed/removed inside Android.
    pub auto_sync_shortcuts: bool,
    /// Forward new Android notifications to Windows 11 native Toast notifications.
    pub forward_notifications: bool,
    /// Automatically sync Windows <-> Android clipboard text changes via Heimdall.
    pub auto_sync_clipboard: bool,
    /// Closing the desktop app window hides it to the system tray instead of exiting.
    pub close_to_tray: bool,
}

impl Default for DesktopConfig {
    fn default() -> Self {
        Self {
            auto_sync_shortcuts: true,
            forward_notifications: true,
            auto_sync_clipboard: true,
            close_to_tray: true,
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

    /// Replaces `[images] trusted_keys` in a config file while keeping the rest of the
    /// file (comments, ordering, formatting) intact. A missing file is created from the
    /// defaults. The edited document is validated before anything is written.
    pub fn write_trusted_keys(path: impl AsRef<Path>, keys: &[String]) -> Result<()> {
        let safe_path = validate_host_file_path(path, Some("toml"))?;
        let raw = if safe_path.exists() {
            std::fs::read_to_string(&safe_path).map_err(|source| BifrostError::Io {
                context: format!("reading config '{}'", safe_path.display()),
                source,
            })?
        } else {
            Self::default().to_toml_pretty()?
        };
        let mut doc: toml_edit::DocumentMut = raw
            .parse()
            .map_err(|err: toml_edit::TomlError| BifrostError::ConfigParse(err.to_string()))?;
        let images = doc
            .entry("images")
            .or_insert_with(|| toml_edit::Item::Table(toml_edit::Table::new()))
            .as_table_like_mut()
            .ok_or_else(|| BifrostError::ConfigValidation {
                field: "images".to_string(),
                reason: "is not a table".to_string(),
            })?;
        let mut array = toml_edit::Array::new();
        for key in keys {
            array.push(key.as_str());
        }
        images.insert("trusted_keys", toml_edit::value(array));
        let edited = doc.to_string();
        Self::from_toml_str(&edited)?;
        if let Some(parent) = safe_path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent).map_err(|source| BifrostError::Io {
                    context: format!("creating config directory '{}'", parent.display()),
                    source,
                })?;
            }
        }
        std::fs::write(&safe_path, edited).map_err(|source| BifrostError::Io {
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

        for key in &self.images.trusted_keys {
            if !is_valid_ed25519_public_key_hex(key.trim()) {
                return Err(BifrostError::ConfigValidation {
                    field: "images.trusted_keys".to_string(),
                    reason: format!(
                        "'{key}' is not a 32-byte Ed25519 public key in hex (64 hex characters)"
                    ),
                });
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn write_trusted_keys_preserves_comments_and_other_settings() {
        let dir = std::env::temp_dir().join(format!("bifrost-cfg-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("bifrost.toml");
        std::fs::write(
            &path,
            "# my notes
schema_version = 1

[runtime]
cpu_cores = 3 # keep me
",
        )
        .unwrap();
        let key = "ab".repeat(32);
        BifrostConfig::write_trusted_keys(&path, std::slice::from_ref(&key)).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("# my notes") && text.contains("cpu_cores = 3 # keep me"));
        let cfg = BifrostConfig::load_from_file(&path).unwrap();
        assert_eq!(cfg.images.trusted_keys, vec![key]);
        assert_eq!(cfg.runtime.cpu_cores, 3);

        BifrostConfig::write_trusted_keys(&path, &[]).unwrap();
        assert!(BifrostConfig::load_from_file(&path)
            .unwrap()
            .images
            .trusted_keys
            .is_empty());
        assert!(BifrostConfig::write_trusted_keys(&path, &["bad".to_string()]).is_err());
        assert!(std::fs::read_to_string(&path)
            .unwrap()
            .contains("# my notes"));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn desktop_close_to_tray_defaults_on_for_existing_files() {
        let cfg = BifrostConfig::from_toml_str(
            "schema_version = 1
[desktop]
auto_sync_clipboard = false
",
        )
        .unwrap();
        assert!(cfg.desktop.close_to_tray);
        assert!(!cfg.desktop.auto_sync_clipboard);
    }

    #[test]
    fn validates_trusted_image_keys() {
        let mut cfg = BifrostConfig::default();
        assert!(cfg.images.trusted_keys.is_empty());

        cfg.images.trusted_keys = vec!["ab".repeat(32)];
        assert!(cfg.validate().is_ok());

        cfg.images.trusted_keys = vec!["not-a-key".to_string()];
        assert!(cfg.validate().is_err());

        let parsed = BifrostConfig::from_toml_str(
            "schema_version = 1\n[images]\ntrusted_keys = [\"0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef\"]\n",
        )
        .unwrap();
        assert_eq!(parsed.images.trusted_keys.len(), 1);
    }

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

    #[test]
    fn recommends_safe_host_tuned_cpu_and_memory() {
        const GIB: u64 = 1024 * 1024 * 1024;
        assert_eq!(RuntimeConfig::recommend_for_host(16, 32 * GIB), (8, 8192));
        assert_eq!(RuntimeConfig::recommend_for_host(8, 16 * GIB), (6, 6144));
        assert_eq!(RuntimeConfig::recommend_for_host(4, 12 * GIB), (4, 4096));
        assert_eq!(RuntimeConfig::recommend_for_host(2, 8 * GIB), (2, 2048));
    }
}
