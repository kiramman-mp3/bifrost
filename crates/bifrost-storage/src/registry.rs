//! Midgard runtime image catalog and upgrade compatibility checks.
//!
//! Ensures Android `userdata` persists safely across compatible runtime upgrades
//! and blocks silent downgrades or incompatible architecture switches.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use bifrost_core::{BifrostError, Result};
use serde::{Deserialize, Serialize};

use crate::verify::{sha256_file, VerifiedBundle};

/// Default userdata compatibility epoch for Midgard bundles.
pub const DEFAULT_USERDATA_COMPAT_VERSION: u32 = 1;

/// Catalog record for a cryptographically verified Midgard runtime bundle.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InstalledImageRecord {
    pub id: String,
    pub name: String,
    pub android_version: String,
    pub arch: String,
    pub manifest_path: PathBuf,
    pub manifest_sha256: String,
    pub signature_verified: bool,
    pub userdata_compat_version: u32,
    pub registered_at_unix: u64,
}

/// Result of evaluating whether switching from `current` to `target` preserves `userdata`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum UpgradeCompatibility {
    Compatible,
    DowngradeBlocked {
        from_android: String,
        to_android: String,
    },
    IncompatibleUserdata {
        current_compat: u32,
        target_compat: u32,
    },
    IncompatibleArch {
        current_arch: String,
        target_arch: String,
    },
}

/// Persisted local registry of verified Midgard runtime versions (`registry.json`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImageRegistry {
    pub active_image_id: Option<String>,
    pub images: Vec<InstalledImageRecord>,
}

impl ImageRegistry {
    /// Loads `registry.json` from `images_dir` (returning an empty registry if not yet created).
    pub fn load(images_dir: &Path) -> Result<Self> {
        let path = Self::registry_file(images_dir);
        if !path.exists() {
            return Ok(Self::default());
        }
        let raw = fs::read_to_string(&path).map_err(|source| BifrostError::Io {
            context: format!("reading image registry '{}'", path.display()),
            source,
        })?;
        serde_json::from_str(&raw).map_err(|err| BifrostError::ConfigParse(err.to_string()))
    }

    /// Atomically persists `registry.json` inside `images_dir`.
    pub fn save(&self, images_dir: &Path) -> Result<()> {
        fs::create_dir_all(images_dir).map_err(|source| BifrostError::Io {
            context: format!("creating images directory '{}'", images_dir.display()),
            source,
        })?;
        let path = Self::registry_file(images_dir);
        let tmp = path.with_extension("json.tmp");
        let serialized = serde_json::to_string_pretty(self)
            .map_err(|err| BifrostError::ConfigSerialize(err.to_string()))?;
        fs::write(&tmp, serialized).map_err(|source| BifrostError::Io {
            context: format!("writing temporary registry '{}'", tmp.display()),
            source,
        })?;
        fs::rename(&tmp, &path).map_err(|source| BifrostError::Io {
            context: format!("persisting image registry '{}'", path.display()),
            source,
        })?;
        Ok(())
    }

    /// Returns the path to `registry.json`.
    #[must_use]
    pub fn registry_file(images_dir: &Path) -> PathBuf {
        images_dir.join("registry.json")
    }

    /// Registers (or updates) a verified Midgard bundle in the catalog.
    pub fn register_bundle(
        &mut self,
        bundle: &VerifiedBundle,
        signature_verified: bool,
        userdata_compat_version: u32,
    ) -> Result<InstalledImageRecord> {
        let manifest_sha256 = sha256_file(&bundle.manifest_path)?;
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);

        let record = InstalledImageRecord {
            id: bundle.id.clone(),
            name: bundle.name.clone(),
            android_version: bundle.android_version.clone(),
            arch: bundle.arch.clone(),
            manifest_path: bundle.manifest_path.clone(),
            manifest_sha256,
            signature_verified,
            userdata_compat_version,
            registered_at_unix: now,
        };

        if let Some(existing) = self.images.iter_mut().find(|i| i.id == record.id) {
            *existing = record.clone();
        } else {
            self.images.push(record.clone());
        }

        if self.active_image_id.is_none() {
            self.active_image_id = Some(record.id.clone());
        }

        Ok(record)
    }

    /// Switches the active Midgard runtime version after verifying upgrade compatibility.
    pub fn activate(&mut self, target_id: &str) -> Result<InstalledImageRecord> {
        let target = self
            .images
            .iter()
            .find(|i| i.id == target_id)
            .cloned()
            .ok_or_else(|| {
                BifrostError::Runtime(format!(
                    "image '{target_id}' is not registered in the local Midgard catalog"
                ))
            })?;

        if let Some(ref current_id) = self.active_image_id {
            if current_id != target_id {
                if let Some(current) = self.images.iter().find(|i| i.id == *current_id) {
                    match check_upgrade_compatibility(current, &target) {
                        UpgradeCompatibility::Compatible => {}
                        UpgradeCompatibility::DowngradeBlocked {
                            from_android,
                            to_android,
                        } => {
                            return Err(BifrostError::Runtime(format!(
                                "refusing Android downgrade from {from_android} to {to_android} to protect persistent userdata; back up and wipe userdata explicitly first"
                            )));
                        }
                        UpgradeCompatibility::IncompatibleUserdata {
                            current_compat,
                            target_compat,
                        } => {
                            return Err(BifrostError::Runtime(format!(
                                "incompatible userdata schema (current v{current_compat}, target v{target_compat}); back up userdata before switching"
                            )));
                        }
                        UpgradeCompatibility::IncompatibleArch {
                            current_arch,
                            target_arch,
                        } => {
                            return Err(BifrostError::Runtime(format!(
                                "incompatible architecture switch ({current_arch} -> {target_arch})"
                            )));
                        }
                    }
                }
            }
        }

        self.active_image_id = Some(target.id.clone());
        Ok(target)
    }
}

/// Evaluates whether transitioning from `current` to `target` preserves Android `userdata`.
#[must_use]
pub fn check_upgrade_compatibility(
    current: &InstalledImageRecord,
    target: &InstalledImageRecord,
) -> UpgradeCompatibility {
    if current.arch != target.arch {
        return UpgradeCompatibility::IncompatibleArch {
            current_arch: current.arch.clone(),
            target_arch: target.arch.clone(),
        };
    }
    if current.userdata_compat_version != target.userdata_compat_version {
        return UpgradeCompatibility::IncompatibleUserdata {
            current_compat: current.userdata_compat_version,
            target_compat: target.userdata_compat_version,
        };
    }

    let cur_major = parse_android_major(&current.android_version);
    let tgt_major = parse_android_major(&target.android_version);
    if let (Some(c), Some(t)) = (cur_major, tgt_major) {
        if t < c {
            return UpgradeCompatibility::DowngradeBlocked {
                from_android: current.android_version.clone(),
                to_android: target.android_version.clone(),
            };
        }
    }

    UpgradeCompatibility::Compatible
}

fn parse_android_major(version: &str) -> Option<u32> {
    let first = version.split('.').next()?.trim();
    first.parse::<u32>().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_record(id: &str, android_version: &str, compat: u32) -> InstalledImageRecord {
        InstalledImageRecord {
            id: id.to_string(),
            name: id.to_string(),
            android_version: android_version.to_string(),
            arch: "x86_64".to_string(),
            manifest_path: PathBuf::from(format!("{id}.json")),
            manifest_sha256: "0".repeat(64),
            signature_verified: true,
            userdata_compat_version: compat,
            registered_at_unix: 100,
        }
    }

    #[test]
    fn allows_compatible_forward_upgrade_and_blocks_downgrade() {
        let mut reg = ImageRegistry {
            active_image_id: Some("aosp-15".to_string()),
            images: vec![
                sample_record("aosp-15", "15", 1),
                sample_record("aosp-16", "16", 1),
                sample_record("aosp-14", "14", 1),
            ],
        };

        // Forward upgrade 15 -> 16 succeeds and preserves userdata.
        assert_eq!(reg.activate("aosp-16").unwrap().id, "aosp-16");

        // Downgrade 16 -> 14 is blocked to prevent userdata corruption.
        let err = reg.activate("aosp-14").unwrap_err();
        assert!(err.to_string().contains("downgrade"));
    }
}
