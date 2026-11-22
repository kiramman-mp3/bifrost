//! Versioned manifest schema for Midgard Android `x86_64` image bundles.

use std::path::{Path, PathBuf};

use bifrost_core::{validate_host_file_path, validate_relative_safe_path, BifrostError, Result};
use serde::{Deserialize, Serialize};

/// Supported Midgard image manifest schema version.
pub const CURRENT_MANIFEST_SCHEMA: u32 = 1;

/// Disk image container format supported by Midgard VMM backends.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DiskFormat {
    Raw,
    Qcow2,
}

impl DiskFormat {
    /// Returns the QEMU `-drive format=...` string representation.
    #[must_use]
    pub const fn as_qemu_format(self) -> &'static str {
        match self {
            Self::Raw => "raw",
            Self::Qcow2 => "qcow2",
        }
    }
}

/// Media attachment type for bootable single-image bundles.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DiskMedia {
    Disk,
    Cdrom,
}

/// A single verifiable file inside an image bundle directory.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArtifactSpec {
    /// Relative path inside the manifest directory (must not contain `..` or absolute prefixes).
    pub relative_path: PathBuf,
    /// Lowercase 64-character SHA-256 hex digest verified prior to VM boot.
    pub sha256: String,
    /// Optional expected file size in bytes.
    pub size_bytes: Option<u64>,
}

impl ArtifactSpec {
    /// Validates path safety and SHA-256 hex format without touching the filesystem.
    pub fn validate(&self, field: &str) -> Result<()> {
        validate_relative_safe_path(&self.relative_path)?;
        if self.sha256.len() != 64 || !self.sha256.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(BifrostError::ConfigValidation {
                field: field.to_string(),
                reason: "sha256 must be a 64-character hexadecimal digest".to_string(),
            });
        }
        Ok(())
    }
}

/// A VirtIO block disk attached to Midgard in direct-kernel boot mode.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiskArtifactSpec {
    pub id: String,
    pub artifact: ArtifactSpec,
    pub format: DiskFormat,
    #[serde(default = "default_true")]
    pub read_only: bool,
}

const fn default_true() -> bool {
    true
}

/// Supported Android `x86_64` boot topologies.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum BootMode {
    /// Canonical AOSP x86_64 VirtIO boot (`bzImage`/`kernel` + `ramdisk.img` + `super.img`/partitions).
    DirectKernel {
        kernel: ArtifactSpec,
        initrd: Option<ArtifactSpec>,
        cmdline: String,
        disks: Vec<DiskArtifactSpec>,
    },
    /// Bootable x86_64 disk or ISO image (e.g., open-source Android-x86 / Bliss OS prototype image).
    BootableImage {
        image: ArtifactSpec,
        format: DiskFormat,
        media: DiskMedia,
    },
    /// UEFI (`pflash` EDK2) VirtIO disk topology (e.g., LineageOS `virtio_x86_64` with `vda.qcow2` + `vdb.qcow2` + `efi_vars.fd`).
    UefiDisks {
        /// Optional bundled UEFI code firmware (if omitted, auto-discovers QEMU's `edk2-x86_64-code.fd`).
        efi_code: Option<ArtifactSpec>,
        /// Optional UEFI NVRAM variables template (`efi_vars.fd`), staged per-instance so the bundle stays immutable.
        efi_vars: Option<ArtifactSpec>,
        /// Ordered VirtIO block disks (`bootindex=0`, `bootindex=1`, ...).
        disks: Vec<DiskArtifactSpec>,
        /// If true, skips attaching a separate raw `userdata.img` because the bundle ships its own VirtIO userdata disk.
        #[serde(default)]
        bundle_includes_userdata: bool,
    },
}

/// Versioned manifest describing a cryptographically verifiable Midgard image bundle.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MidgardImageManifest {
    pub schema_version: u32,
    pub id: String,
    pub name: String,
    pub android_version: String,
    pub arch: String,
    pub boot: BootMode,
}

impl MidgardImageManifest {
    /// Loads and validates a JSON manifest from disk.
    pub fn load_from_file(path: impl AsRef<Path>) -> Result<Self> {
        let safe_path = validate_host_file_path(path, Some("json"))?;
        let raw = std::fs::read_to_string(&safe_path).map_err(|source| BifrostError::Io {
            context: format!("reading Midgard manifest '{}'", safe_path.display()),
            source,
        })?;
        Self::from_json_str(&raw)
    }

    /// Parses and validates a JSON manifest string (tolerating an optional Windows UTF-8 BOM).
    pub fn from_json_str(raw: &str) -> Result<Self> {
        let trimmed = raw.strip_prefix('\u{feff}').unwrap_or(raw);
        let manifest: Self = serde_json::from_str(trimmed)
            .map_err(|err| BifrostError::ConfigParse(err.to_string()))?;
        manifest.validate()?;
        Ok(manifest)
    }

    /// Enforces schema version, `x86_64` architecture, and path/digest invariants.
    pub fn validate(&self) -> Result<()> {
        if self.schema_version != CURRENT_MANIFEST_SCHEMA {
            return Err(BifrostError::ConfigValidation {
                field: "schema_version".to_string(),
                reason: format!(
                    "unsupported manifest schema {} (expected {})",
                    self.schema_version, CURRENT_MANIFEST_SCHEMA
                ),
            });
        }

        if self.id.trim().is_empty()
            || !self
                .id
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
        {
            return Err(BifrostError::ConfigValidation {
                field: "id".to_string(),
                reason: "must be non-empty ASCII alphanumeric with '-', '_', or '.'".to_string(),
            });
        }

        if self.arch != "x86_64" {
            return Err(BifrostError::ConfigValidation {
                field: "arch".to_string(),
                reason: format!(
                    "unsupported image architecture '{}' (BIFRÖST currently supports x86_64 only)",
                    self.arch
                ),
            });
        }

        match &self.boot {
            BootMode::DirectKernel {
                kernel,
                initrd,
                cmdline,
                disks,
            } => {
                kernel.validate("boot.kernel")?;
                if let Some(ref initrd_spec) = initrd {
                    initrd_spec.validate("boot.initrd")?;
                }
                if cmdline.trim().is_empty() {
                    return Err(BifrostError::ConfigValidation {
                        field: "boot.cmdline".to_string(),
                        reason: "kernel command line must not be empty".to_string(),
                    });
                }
                for (idx, disk) in disks.iter().enumerate() {
                    if disk.id.trim().is_empty()
                        || !disk
                            .id
                            .chars()
                            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
                    {
                        return Err(BifrostError::ConfigValidation {
                            field: format!("boot.disks[{idx}].id"),
                            reason: "disk id must be non-empty ASCII alphanumeric".to_string(),
                        });
                    }
                    disk.artifact.validate(&format!("boot.disks[{idx}]"))?;
                }
            }
            BootMode::BootableImage { image, .. } => {
                image.validate("boot.image")?;
            }
            BootMode::UefiDisks {
                efi_code,
                efi_vars,
                disks,
                ..
            } => {
                if let Some(ref code) = efi_code {
                    code.validate("boot.efi_code")?;
                }
                if let Some(ref vars) = efi_vars {
                    vars.validate("boot.efi_vars")?;
                }
                if disks.is_empty() {
                    return Err(BifrostError::ConfigValidation {
                        field: "boot.disks".to_string(),
                        reason: "UEFI disk boot requires at least one VirtIO disk".to_string(),
                    });
                }
                for (idx, disk) in disks.iter().enumerate() {
                    if disk.id.trim().is_empty()
                        || !disk
                            .id
                            .chars()
                            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
                    {
                        return Err(BifrostError::ConfigValidation {
                            field: format!("boot.disks[{idx}].id"),
                            reason: "disk id must be non-empty ASCII alphanumeric".to_string(),
                        });
                    }
                    disk.artifact.validate(&format!("boot.disks[{idx}]"))?;
                }
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const VALID_SHA: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    #[test]
    fn validates_direct_kernel_manifest() {
        let json = format!(
            r#"{{
                "schema_version": 1,
                "id": "aosp-16-x86_64",
                "name": "AOSP 16 Midgard",
                "android_version": "16",
                "arch": "x86_64",
                "boot": {{
                    "type": "direct_kernel",
                    "kernel": {{ "relative_path": "bzImage", "sha256": "{VALID_SHA}" }},
                    "initrd": {{ "relative_path": "ramdisk.img", "sha256": "{VALID_SHA}" }},
                    "cmdline": "console=ttyS0 androidboot.console=ttyS0",
                    "disks": [
                        {{
                            "id": "super",
                            "artifact": {{ "relative_path": "super.img", "sha256": "{VALID_SHA}" }},
                            "format": "raw",
                            "read_only": true
                        }}
                    ]
                }}
            }}"#
        );
        let manifest = MidgardImageManifest::from_json_str(&json).unwrap();
        assert_eq!(manifest.id, "aosp-16-x86_64");

        // Windows PowerShell 5.1 UTF-8 BOM must also parse cleanly.
        let with_bom = format!("\u{feff}{json}");
        assert_eq!(
            MidgardImageManifest::from_json_str(&with_bom).unwrap().id,
            "aosp-16-x86_64"
        );
    }

    #[test]
    fn rejects_arm64_and_path_traversal_in_manifest() {
        let arm_json = format!(
            r#"{{
                "schema_version": 1,
                "id": "bad-arm",
                "name": "ARM Image",
                "android_version": "15",
                "arch": "arm64",
                "boot": {{
                    "type": "bootable_image",
                    "image": {{ "relative_path": "android.iso", "sha256": "{VALID_SHA}" }},
                    "format": "raw",
                    "media": "cdrom"
                }}
            }}"#
        );
        assert!(MidgardImageManifest::from_json_str(&arm_json).is_err());

        let traversal_json = format!(
            r#"{{
                "schema_version": 1,
                "id": "bad-path",
                "name": "Traversal",
                "android_version": "15",
                "arch": "x86_64",
                "boot": {{
                    "type": "bootable_image",
                    "image": {{ "relative_path": "../escape.iso", "sha256": "{VALID_SHA}" }},
                    "format": "raw",
                    "media": "cdrom"
                }}
            }}"#
        );
        assert!(MidgardImageManifest::from_json_str(&traversal_json).is_err());
    }
}
