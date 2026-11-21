//! Security-hardened filesystem path validation and standard BIFRÖST directory layout.

use std::path::{Component, Path, PathBuf};

use directories::ProjectDirs;
use serde::{Deserialize, Serialize};

use crate::error::{BifrostError, Result};
use crate::TECHNICAL_ID;

/// Standard filesystem locations used by BIFRÖST on the host.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BifrostPaths {
    pub config_dir: PathBuf,
    pub data_dir: PathBuf,
    pub images_dir: PathBuf,
    pub logs_dir: PathBuf,
}

impl BifrostPaths {
    /// Resolves OS-appropriate per-user BIFRÖST directories (`%LOCALAPPDATA%\bifrost` on Windows).
    pub fn discover() -> Result<Self> {
        let dirs = ProjectDirs::from("org", TECHNICAL_ID, TECHNICAL_ID).ok_or_else(|| {
            BifrostError::ConfigValidation {
                field: "paths".to_string(),
                reason: "unable to determine user profile directories".to_string(),
            }
        })?;

        let config_dir = dirs.config_local_dir().to_path_buf();
        let data_dir = dirs.data_local_dir().to_path_buf();
        let images_dir = data_dir.join("images");
        let logs_dir = data_dir.join("logs");

        Ok(Self {
            config_dir,
            data_dir,
            images_dir,
            logs_dir,
        })
    }

    /// Creates a deterministic layout rooted at `base_dir` (useful for tests or portable setups).
    #[must_use]
    pub fn from_root(base_dir: impl AsRef<Path>) -> Self {
        let root = base_dir.as_ref();
        Self {
            config_dir: root.join("config"),
            data_dir: root.join("data"),
            images_dir: root.join("data").join("images"),
            logs_dir: root.join("data").join("logs"),
        }
    }

    /// Default path to the primary TOML configuration file.
    #[must_use]
    pub fn default_config_file(&self) -> PathBuf {
        self.config_dir.join("bifrost.toml")
    }
}

/// Validates that `relative` is strictly relative and cannot traverse outside a root directory.
pub fn validate_relative_safe_path(relative: impl AsRef<Path>) -> Result<PathBuf> {
    let path = relative.as_ref();
    let raw = path.as_os_str().to_string_lossy();

    if raw.is_empty() {
        return Err(BifrostError::UnsafePath {
            path: path.to_path_buf(),
            reason: "path must not be empty".to_string(),
        });
    }

    if raw.contains('\0') {
        return Err(BifrostError::UnsafePath {
            path: path.to_path_buf(),
            reason: "path contains null byte".to_string(),
        });
    }

    let mut clean = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Normal(segment) => clean.push(segment),
            Component::CurDir => {}
            Component::ParentDir => {
                return Err(BifrostError::UnsafePath {
                    path: path.to_path_buf(),
                    reason: "parent directory traversal ('..') is forbidden".to_string(),
                });
            }
            Component::RootDir | Component::Prefix(_) => {
                return Err(BifrostError::UnsafePath {
                    path: path.to_path_buf(),
                    reason: "absolute or prefixed paths are not allowed here".to_string(),
                });
            }
        }
    }

    if clean.as_os_str().is_empty() {
        return Err(BifrostError::UnsafePath {
            path: path.to_path_buf(),
            reason: "path resolves to empty relative target".to_string(),
        });
    }

    Ok(clean)
}

/// Joins a validated relative path inside `root` and guarantees containment.
///
/// If both `root` and the target exist on disk, canonicalization is also verified
/// so symlinks or Windows junctions cannot escape `root`.
pub fn resolve_within_root(root: impl AsRef<Path>, relative: impl AsRef<Path>) -> Result<PathBuf> {
    let root_path = root.as_ref();
    let clean_relative = validate_relative_safe_path(relative)?;
    let joined = root_path.join(&clean_relative);

    if root_path.exists() && joined.exists() {
        let canonical_root = root_path
            .canonicalize()
            .map_err(|source| BifrostError::Io {
                context: format!("canonicalizing root '{}'", root_path.display()),
                source,
            })?;
        let canonical_target = joined.canonicalize().map_err(|source| BifrostError::Io {
            context: format!("canonicalizing target '{}'", joined.display()),
            source,
        })?;

        if !canonical_target.starts_with(&canonical_root) {
            return Err(BifrostError::UnsafePath {
                path: joined,
                reason: "resolved path escapes allowed root directory".to_string(),
            });
        }
        return Ok(canonical_target);
    }

    Ok(joined)
}

/// Validates a user-supplied host file path (such as a config or APK path),
/// rejecting empty values, null bytes, and parent-directory traversal components.
pub fn validate_host_file_path(
    path: impl AsRef<Path>,
    expected_extension: Option<&str>,
) -> Result<PathBuf> {
    let candidate = path.as_ref();
    let raw = candidate.as_os_str().to_string_lossy();

    if raw.trim().is_empty() {
        return Err(BifrostError::UnsafePath {
            path: candidate.to_path_buf(),
            reason: "path must not be empty".to_string(),
        });
    }

    if raw.contains('\0') {
        return Err(BifrostError::UnsafePath {
            path: candidate.to_path_buf(),
            reason: "path contains null byte".to_string(),
        });
    }

    for component in candidate.components() {
        if matches!(component, Component::ParentDir) {
            return Err(BifrostError::UnsafePath {
                path: candidate.to_path_buf(),
                reason: "parent directory traversal ('..') is forbidden".to_string(),
            });
        }
    }

    if let Some(ext) = expected_extension {
        let actual_ext = candidate
            .extension()
            .and_then(|e| e.to_str())
            .map(str::to_ascii_lowercase);
        if actual_ext.as_deref() != Some(&ext.to_ascii_lowercase()) {
            return Err(BifrostError::UnsafePath {
                path: candidate.to_path_buf(),
                reason: format!("expected '.{ext}' file extension"),
            });
        }
    }

    Ok(candidate.to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn rejects_directory_traversal_in_relative_paths() {
        assert!(validate_relative_safe_path("../secret.txt").is_err());
        assert!(validate_relative_safe_path("subdir/../../Windows/System32").is_err());
        assert!(validate_relative_safe_path("").is_err());
        assert!(validate_relative_safe_path(".").is_err());
    }

    #[test]
    fn rejects_absolute_paths_as_relative_paths() {
        assert!(validate_relative_safe_path("/etc/passwd").is_err());
        #[cfg(windows)]
        assert!(validate_relative_safe_path(r"C:\Windows\System32").is_err());
    }

    #[test]
    fn resolves_valid_relative_path_inside_root() {
        let tmp = TempDir::new().unwrap();
        let nested = tmp.path().join("images").join("midgard");
        std::fs::create_dir_all(&nested).unwrap();
        let file = nested.join("manifest.json");
        std::fs::write(&file, "{}").unwrap();

        let resolved = resolve_within_root(tmp.path(), "images/midgard/manifest.json").unwrap();
        assert!(resolved.ends_with("manifest.json"));
    }

    #[test]
    fn host_file_path_validates_extension_and_traversal() {
        assert!(validate_host_file_path("configs/bifrost.toml", Some("toml")).is_ok());
        assert!(validate_host_file_path("../bifrost.toml", Some("toml")).is_err());
        assert!(validate_host_file_path("configs/bifrost.yaml", Some("toml")).is_err());
    }
}
