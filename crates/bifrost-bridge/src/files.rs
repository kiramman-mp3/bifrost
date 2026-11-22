//! Scoped host <-> Android shared directory bridge with strict path traversal guards.

use std::fs;
use std::path::{Path, PathBuf};

use bifrost_core::{resolve_within_root, BifrostError, Result};
use serde::{Deserialize, Serialize};
use tracing::info;

use crate::adb::AdbClient;

/// Fixed guest directory root for BIFRÖST shared files on Android.
pub const DEFAULT_GUEST_SHARE_ROOT: &str = "/sdcard/BifrostShare";

/// Metadata about a file stored in the host BIFRÖST shared directory.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SharedFileEntry {
    pub relative_path: String,
    pub size_bytes: u64,
    pub is_dir: bool,
}

/// Manages strictly scoped file transfers between Windows and Android.
#[derive(Debug, Clone)]
pub struct SharedDirectoryBridge {
    host_root: PathBuf,
    guest_root: String,
    adb: AdbClient,
}

impl SharedDirectoryBridge {
    /// Creates a shared directory bridge rooted at `host_root` and `/sdcard/BifrostShare`.
    #[must_use]
    pub fn new(host_root: impl Into<PathBuf>, adb: AdbClient) -> Self {
        Self {
            host_root: host_root.into(),
            guest_root: DEFAULT_GUEST_SHARE_ROOT.to_string(),
            adb,
        }
    }

    /// Returns the host-side shared root directory.
    #[must_use]
    pub fn host_root(&self) -> &Path {
        &self.host_root
    }

    /// Ensures the host shared root directory exists and lists its top-level entries.
    pub fn list_host_files(&self) -> Result<Vec<SharedFileEntry>> {
        fs::create_dir_all(&self.host_root).map_err(|source| BifrostError::Io {
            context: format!("ensuring shared directory '{}'", self.host_root.display()),
            source,
        })?;

        let mut entries = Vec::new();
        let read_dir = fs::read_dir(&self.host_root).map_err(|source| BifrostError::Io {
            context: format!("listing shared directory '{}'", self.host_root.display()),
            source,
        })?;

        for item in read_dir {
            let entry = item.map_err(|source| BifrostError::Io {
                context: "reading shared directory entry".to_string(),
                source,
            })?;
            let meta = entry.metadata().map_err(|source| BifrostError::Io {
                context: format!("reading metadata for '{}'", entry.path().display()),
                source,
            })?;
            entries.push(SharedFileEntry {
                relative_path: entry.file_name().to_string_lossy().to_string(),
                size_bytes: if meta.is_file() { meta.len() } else { 0 },
                is_dir: meta.is_dir(),
            });
        }
        entries.sort_by(|a, b| a.relative_path.cmp(&b.relative_path));
        Ok(entries)
    }

    /// Pushes a validated relative file from the host share root into `/sdcard/BifrostShare`.
    pub fn push_to_guest(&self, serial: &str, relative_path: &str) -> Result<String> {
        let host_path = resolve_within_root(&self.host_root, relative_path)?;
        if !host_path.is_file() {
            return Err(BifrostError::Bridge(format!(
                "shared file '{}' does not exist inside '{}'",
                relative_path,
                self.host_root.display()
            )));
        }

        let clean_guest_rel = validate_guest_relative_path(relative_path)?;
        let guest_dest = format!("{}/{}", self.guest_root, clean_guest_rel);

        let _ = self
            .adb
            .run_adb(&["-s", serial, "shell", "mkdir", "-p", &self.guest_root])?;

        let host_str = host_path.to_string_lossy().to_string();
        info!(
            serial,
            host = %host_path.display(),
            guest = %guest_dest,
            "pushing scoped shared file to Android"
        );
        self.adb
            .run_adb(&["-s", serial, "push", &host_str, &guest_dest])
    }

    /// Pulls a validated relative file from `/sdcard/BifrostShare` into the host share root.
    pub fn pull_from_guest(&self, serial: &str, guest_relative_path: &str) -> Result<PathBuf> {
        let clean_guest_rel = validate_guest_relative_path(guest_relative_path)?;
        fs::create_dir_all(&self.host_root).map_err(|source| BifrostError::Io {
            context: format!("creating shared directory '{}'", self.host_root.display()),
            source,
        })?;
        let host_dest = resolve_within_root(&self.host_root, &clean_guest_rel)?;
        if let Some(parent) = host_dest.parent() {
            fs::create_dir_all(parent).map_err(|source| BifrostError::Io {
                context: format!("creating destination directory '{}'", parent.display()),
                source,
            })?;
        }

        let guest_src = format!("{}/{}", self.guest_root, clean_guest_rel);
        let host_str = host_dest.to_string_lossy().to_string();
        info!(
            serial,
            guest = %guest_src,
            host = %host_dest.display(),
            "pulling scoped shared file from Android"
        );
        let _ = self
            .adb
            .run_adb(&["-s", serial, "pull", &guest_src, &host_str])?;
        Ok(host_dest)
    }
}

/// Validates a POSIX relative path targeted inside Android's shared directory,
/// rejecting traversal (`..`), leading slashes, null bytes, and shell metacharacters.
pub fn validate_guest_relative_path(input: &str) -> Result<String> {
    let normalized = input.replace('\\', "/");
    if normalized.trim().is_empty() {
        return Err(BifrostError::UnsafePath {
            path: PathBuf::from(input),
            reason: "guest shared path must not be empty".to_string(),
        });
    }
    if normalized.starts_with('/') || normalized.contains('\0') {
        return Err(BifrostError::UnsafePath {
            path: PathBuf::from(input),
            reason: "guest shared path must be relative without null bytes".to_string(),
        });
    }

    let mut clean_segments = Vec::new();
    for seg in normalized.split('/') {
        if seg.is_empty() || seg == "." {
            continue;
        }
        if seg == ".." {
            return Err(BifrostError::UnsafePath {
                path: PathBuf::from(input),
                reason: "parent directory traversal ('..') is forbidden in guest paths".to_string(),
            });
        }
        if !seg
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-' | ' '))
        {
            return Err(BifrostError::UnsafePath {
                path: PathBuf::from(input),
                reason: format!("illegal character in guest path segment '{seg}'"),
            });
        }
        clean_segments.push(seg);
    }

    if clean_segments.is_empty() {
        return Err(BifrostError::UnsafePath {
            path: PathBuf::from(input),
            reason: "guest path resolves to empty target".to_string(),
        });
    }

    Ok(clean_segments.join("/"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn validates_guest_relative_paths_and_blocks_traversal_or_shell_chars() {
        assert_eq!(
            validate_guest_relative_path("docs/report-1.pdf").unwrap(),
            "docs/report-1.pdf"
        );
        assert!(validate_guest_relative_path("../etc/passwd").is_err());
        assert!(validate_guest_relative_path("/sdcard/other").is_err());
        assert!(validate_guest_relative_path("file;rm -rf.txt").is_err());
    }

    #[test]
    fn lists_host_shared_directory_safely() {
        let tmp = TempDir::new().unwrap();
        let share_root = tmp.path().join("shared");
        fs::create_dir_all(&share_root).unwrap();
        fs::write(share_root.join("hello.txt"), b"bifrost").unwrap();

        let bridge = SharedDirectoryBridge::new(&share_root, AdbClient::new());
        let list = bridge.list_host_files().unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].relative_path, "hello.txt");
        assert_eq!(list[0].size_bytes, 7);

        // Push outside share root must fail before invoking ADB.
        let err = bridge
            .push_to_guest("127.0.0.1:5555", "../../Windows/win.ini")
            .unwrap_err();
        assert!(matches!(err, BifrostError::UnsafePath { .. }));
    }
}
