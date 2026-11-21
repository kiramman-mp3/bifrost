//! Persistent Android `userdata` virtual disk management.

use std::fs::{self, OpenOptions};
use std::path::{Path, PathBuf};

use bifrost_core::{validate_host_file_path, BifrostError, Result};
use serde::{Deserialize, Serialize};
use tracing::info;

use crate::manifest::DiskFormat;

const BYTES_PER_GIB: u64 = 1024 * 1024 * 1024;

/// Metadata about a provisioned persistent `userdata` virtual disk.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UserdataDiskInfo {
    pub path: PathBuf,
    pub format: DiskFormat,
    pub size_bytes: u64,
    pub created_now: bool,
}

/// Ensures a persistent raw `userdata` disk exists at `disk_path`.
///
/// If the disk already exists, its contents are preserved untouched across
/// reboots and compatible Midgard runtime upgrades.
pub fn ensure_userdata_disk(disk_path: impl AsRef<Path>, size_gb: u64) -> Result<UserdataDiskInfo> {
    let safe_path = validate_host_file_path(disk_path, Some("img"))?;
    if !(1..=1024).contains(&size_gb) {
        return Err(BifrostError::ConfigValidation {
            field: "storage_gb".to_string(),
            reason: "userdata disk size must be between 1 and 1024 GB".to_string(),
        });
    }

    if safe_path.exists() {
        let meta = fs::metadata(&safe_path).map_err(|source| BifrostError::Io {
            context: format!(
                "inspecting existing userdata disk '{}'",
                safe_path.display()
            ),
            source,
        })?;
        return Ok(UserdataDiskInfo {
            path: safe_path,
            format: DiskFormat::Raw,
            size_bytes: meta.len(),
            created_now: false,
        });
    }

    if let Some(parent) = safe_path.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent).map_err(|source| BifrostError::Io {
                context: format!("creating userdata directory '{}'", parent.display()),
                source,
            })?;
        }
    }

    let target_bytes = size_gb * BYTES_PER_GIB;
    let file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&safe_path)
        .map_err(|source| BifrostError::Io {
            context: format!("creating userdata disk '{}'", safe_path.display()),
            source,
        })?;

    file.set_len(target_bytes)
        .map_err(|source| BifrostError::Io {
            context: format!(
                "allocating sparse {} GB userdata disk '{}'",
                size_gb,
                safe_path.display()
            ),
            source,
        })?;

    info!(
        path = %safe_path.display(),
        size_gb,
        "provisioned new persistent userdata disk"
    );

    Ok(UserdataDiskInfo {
        path: safe_path,
        format: DiskFormat::Raw,
        size_bytes: target_bytes,
        created_now: true,
    })
}

/// Deletes a persistent `userdata` disk ONLY when explicitly confirmed by the user.
pub fn wipe_userdata_disk(disk_path: impl AsRef<Path>, confirmed: bool) -> Result<()> {
    let safe_path = validate_host_file_path(disk_path, Some("img"))?;
    if !confirmed {
        return Err(BifrostError::ConfirmationRequired {
            operation: format!("wipe persistent userdata disk '{}'", safe_path.display()),
        });
    }

    if safe_path.exists() {
        fs::remove_file(&safe_path).map_err(|source| BifrostError::Io {
            context: format!("removing userdata disk '{}'", safe_path.display()),
            source,
        })?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn creates_preserves_and_guards_wipe_of_userdata_disk() {
        let tmp = TempDir::new().unwrap();
        let disk = tmp
            .path()
            .join("instances")
            .join("default")
            .join("userdata.img");

        let first = ensure_userdata_disk(&disk, 1).unwrap();
        assert!(first.created_now);
        assert_eq!(first.size_bytes, BYTES_PER_GIB);

        // Subsequent calls must preserve the existing disk without recreating it.
        let second = ensure_userdata_disk(&disk, 1).unwrap();
        assert!(!second.created_now);

        // Wipe without explicit confirmation must fail.
        let err = wipe_userdata_disk(&disk, false).unwrap_err();
        assert!(matches!(err, BifrostError::ConfirmationRequired { .. }));
        assert!(disk.exists());

        // Wipe with explicit confirmation removes the disk.
        wipe_userdata_disk(&disk, true).unwrap();
        assert!(!disk.exists());
    }
}
