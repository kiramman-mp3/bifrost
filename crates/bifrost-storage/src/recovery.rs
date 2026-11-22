//! Cryptographically verified backup and confirmation-guarded restore for `userdata.img`.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use bifrost_core::{validate_host_file_path, BifrostError, Result};
use serde::{Deserialize, Serialize};
use tracing::info;

use crate::verify::sha256_file;

const BACKUP_DATA_FILENAME: &str = "userdata.img";
const BACKUP_META_FILENAME: &str = "backup-metadata.json";

/// Integrity metadata stored alongside a backed-up `userdata.img`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BackupMetadata {
    pub schema_version: u32,
    pub created_at_unix: u64,
    pub source_path: PathBuf,
    pub size_bytes: u64,
    pub sha256: String,
}

/// Creates a SHA-256-verified backup of `userdata_path` inside `backup_dir`.
pub fn backup_userdata(
    userdata_path: impl AsRef<Path>,
    backup_dir: impl AsRef<Path>,
) -> Result<BackupMetadata> {
    let src = validate_host_file_path(userdata_path, Some("img"))?;
    let dest_dir = validate_host_file_path(backup_dir, None)?;

    if !src.is_file() {
        return Err(BifrostError::Runtime(format!(
            "cannot back up userdata: '{}' does not exist",
            src.display()
        )));
    }

    fs::create_dir_all(&dest_dir).map_err(|source| BifrostError::Io {
        context: format!("creating backup directory '{}'", dest_dir.display()),
        source,
    })?;

    let dest_img = dest_dir.join(BACKUP_DATA_FILENAME);
    fs::copy(&src, &dest_img).map_err(|source| BifrostError::Io {
        context: format!(
            "copying userdata '{}' to '{}'",
            src.display(),
            dest_img.display()
        ),
        source,
    })?;

    let size_bytes = fs::metadata(&dest_img)
        .map_err(|source| BifrostError::Io {
            context: format!("inspecting backed-up userdata '{}'", dest_img.display()),
            source,
        })?
        .len();
    let sha256 = sha256_file(&dest_img)?;
    let created_at_unix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    let meta = BackupMetadata {
        schema_version: 1,
        created_at_unix,
        source_path: src,
        size_bytes,
        sha256,
    };

    let meta_path = dest_dir.join(BACKUP_META_FILENAME);
    let serialized = serde_json::to_string_pretty(&meta)
        .map_err(|err| BifrostError::ConfigSerialize(err.to_string()))?;
    fs::write(&meta_path, serialized).map_err(|source| BifrostError::Io {
        context: format!("writing backup metadata '{}'", meta_path.display()),
        source,
    })?;

    info!(dir = %dest_dir.display(), sha256 = %meta.sha256, "completed verified userdata backup");
    Ok(meta)
}

/// Verifies a backup's SHA-256 digest and restores it to `target_userdata_path`.
///
/// Requires explicit `confirmed == true` because restoration overwrites live user data.
pub fn restore_userdata(
    backup_dir: impl AsRef<Path>,
    target_userdata_path: impl AsRef<Path>,
    confirmed: bool,
) -> Result<BackupMetadata> {
    let src_dir = validate_host_file_path(backup_dir, None)?;
    let target = validate_host_file_path(target_userdata_path, Some("img"))?;

    if !confirmed {
        return Err(BifrostError::ConfirmationRequired {
            operation: format!("restore userdata backup into '{}'", target.display()),
        });
    }

    let meta_path = src_dir.join(BACKUP_META_FILENAME);
    let raw = fs::read_to_string(&meta_path).map_err(|source| BifrostError::Io {
        context: format!("reading backup metadata '{}'", meta_path.display()),
        source,
    })?;
    let meta: BackupMetadata =
        serde_json::from_str(&raw).map_err(|err| BifrostError::ConfigParse(err.to_string()))?;

    let backup_img = src_dir.join(BACKUP_DATA_FILENAME);
    let actual_sha = sha256_file(&backup_img)?;
    if actual_sha != meta.sha256 {
        return Err(BifrostError::IntegrityMismatch {
            path: backup_img,
            expected: meta.sha256,
            actual: actual_sha,
        });
    }

    if let Some(parent) = target.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent).map_err(|source| BifrostError::Io {
                context: format!("creating userdata parent '{}'", parent.display()),
                source,
            })?;
        }
    }

    fs::copy(&backup_img, &target).map_err(|source| BifrostError::Io {
        context: format!("restoring verified userdata into '{}'", target.display()),
        source,
    })?;

    info!(target = %target.display(), sha256 = %meta.sha256, "restored verified userdata backup");
    Ok(meta)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn backs_up_verifies_and_guards_userdata_restore() {
        let tmp = TempDir::new().unwrap();
        let live_disk = tmp.path().join("live").join("userdata.img");
        fs::create_dir_all(live_disk.parent().unwrap()).unwrap();
        fs::write(&live_disk, b"android-userdata-v1").unwrap();

        let backup_dir = tmp.path().join("backup-1");
        let meta = backup_userdata(&live_disk, &backup_dir).unwrap();
        assert_eq!(meta.size_bytes, 19);

        // Overwrite live disk and confirm restore requires explicit confirmation.
        fs::write(&live_disk, b"corrupted").unwrap();
        assert!(matches!(
            restore_userdata(&backup_dir, &live_disk, false).unwrap_err(),
            BifrostError::ConfirmationRequired { .. }
        ));

        // Confirmed restore brings back original verified bytes.
        restore_userdata(&backup_dir, &live_disk, true).unwrap();
        assert_eq!(fs::read(&live_disk).unwrap(), b"android-userdata-v1");

        // Tampered backup must be rejected by SHA-256 check.
        fs::write(backup_dir.join(BACKUP_DATA_FILENAME), b"tampered-backup").unwrap();
        assert!(matches!(
            restore_userdata(&backup_dir, &live_disk, true).unwrap_err(),
            BifrostError::IntegrityMismatch { .. }
        ));
    }
}
