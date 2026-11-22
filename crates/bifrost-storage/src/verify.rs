//! Streaming SHA-256 verification of Midgard image bundle artifacts.

use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};

use bifrost_core::{resolve_within_root, BifrostError, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tracing::debug;

use crate::manifest::{ArtifactSpec, BootMode, DiskFormat, DiskMedia, MidgardImageManifest};

const HASH_BUFFER_BYTES: usize = 64 * 1024;

/// A verified VirtIO block disk ready to attach to a VM.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VerifiedDisk {
    pub id: String,
    pub path: PathBuf,
    pub format: DiskFormat,
    pub read_only: bool,
}

/// Verified boot media paths resolved safely within the bundle root.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum VerifiedBootMedia {
    DirectKernel {
        kernel: PathBuf,
        initrd: Option<PathBuf>,
        cmdline: String,
        disks: Vec<VerifiedDisk>,
    },
    BootableImage {
        image: PathBuf,
        format: DiskFormat,
        media: DiskMedia,
    },
    UefiDisks {
        efi_code: Option<PathBuf>,
        efi_vars: Option<PathBuf>,
        disks: Vec<VerifiedDisk>,
        bundle_includes_userdata: bool,
    },
}

/// Fully verified Midgard image bundle ready for hypervisor launch.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VerifiedBundle {
    pub id: String,
    pub name: String,
    pub android_version: String,
    pub arch: String,
    pub manifest_path: PathBuf,
    pub boot: VerifiedBootMedia,
}

/// Computes the lowercase hex SHA-256 digest of a file using streaming reads.
pub fn sha256_file(path: impl AsRef<Path>) -> Result<String> {
    let path_ref = path.as_ref();
    let mut file = File::open(path_ref).map_err(|source| BifrostError::Io {
        context: format!("opening '{}' for SHA-256 verification", path_ref.display()),
        source,
    })?;

    let mut hasher = Sha256::new();
    let mut buf = [0u8; HASH_BUFFER_BYTES];
    loop {
        let n = file.read(&mut buf).map_err(|source| BifrostError::Io {
            context: format!("reading '{}' for SHA-256 verification", path_ref.display()),
            source,
        })?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }

    Ok(hex::encode(hasher.finalize()))
}

/// Loads a manifest file and cryptographically verifies every referenced artifact.
pub fn verify_manifest_file(manifest_path: impl AsRef<Path>) -> Result<VerifiedBundle> {
    let manifest_ref = manifest_path.as_ref();
    let manifest = MidgardImageManifest::load_from_file(manifest_ref)?;
    let bundle_root = manifest_ref
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));

    let boot = match manifest.boot {
        BootMode::DirectKernel {
            kernel,
            initrd,
            cmdline,
            disks,
        } => {
            let kernel_path = verify_artifact(bundle_root, &kernel)?;
            let initrd_path = match initrd {
                Some(spec) => Some(verify_artifact(bundle_root, &spec)?),
                None => None,
            };
            let mut verified_disks = Vec::with_capacity(disks.len());
            for disk in disks {
                let path = verify_artifact(bundle_root, &disk.artifact)?;
                verified_disks.push(VerifiedDisk {
                    id: disk.id,
                    path,
                    format: disk.format,
                    read_only: disk.read_only,
                });
            }
            VerifiedBootMedia::DirectKernel {
                kernel: kernel_path,
                initrd: initrd_path,
                cmdline,
                disks: verified_disks,
            }
        }
        BootMode::BootableImage {
            image,
            format,
            media,
        } => {
            let image_path = verify_artifact(bundle_root, &image)?;
            VerifiedBootMedia::BootableImage {
                image: image_path,
                format,
                media,
            }
        }
        BootMode::UefiDisks {
            efi_code,
            efi_vars,
            disks,
            bundle_includes_userdata,
        } => {
            let efi_code_path = match efi_code {
                Some(spec) => Some(verify_artifact(bundle_root, &spec)?),
                None => None,
            };
            let efi_vars_path = match efi_vars {
                Some(spec) => Some(verify_artifact(bundle_root, &spec)?),
                None => None,
            };
            let mut verified_disks = Vec::with_capacity(disks.len());
            for disk in disks {
                let path = verify_artifact(bundle_root, &disk.artifact)?;
                verified_disks.push(VerifiedDisk {
                    id: disk.id,
                    path,
                    format: disk.format,
                    read_only: disk.read_only,
                });
            }
            VerifiedBootMedia::UefiDisks {
                efi_code: efi_code_path,
                efi_vars: efi_vars_path,
                disks: verified_disks,
                bundle_includes_userdata,
            }
        }
    };

    Ok(VerifiedBundle {
        id: manifest.id,
        name: manifest.name,
        android_version: manifest.android_version,
        arch: manifest.arch,
        manifest_path: manifest_ref.to_path_buf(),
        boot,
    })
}

fn verify_artifact(bundle_root: &Path, spec: &ArtifactSpec) -> Result<PathBuf> {
    let resolved = resolve_within_root(bundle_root, &spec.relative_path)?;
    let metadata = std::fs::metadata(&resolved).map_err(|source| BifrostError::Io {
        context: format!("inspecting bundle artifact '{}'", resolved.display()),
        source,
    })?;

    if let Some(expected_size) = spec.size_bytes {
        if metadata.len() != expected_size {
            return Err(BifrostError::Runtime(format!(
                "artifact '{}' size mismatch: expected {} bytes, got {} bytes",
                resolved.display(),
                expected_size,
                metadata.len()
            )));
        }
    }

    let actual_hash = sha256_file(&resolved)?;
    let expected_hash = spec.sha256.to_ascii_lowercase();
    if actual_hash != expected_hash {
        return Err(BifrostError::IntegrityMismatch {
            path: resolved,
            expected: expected_hash,
            actual: actual_hash,
        });
    }

    debug!(artifact = %resolved.display(), sha256 = %actual_hash, "verified bundle artifact");
    Ok(resolved)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn verifies_matching_sha256_and_rejects_tampered_artifact() {
        let tmp = TempDir::new().unwrap();
        let img = tmp.path().join("android.iso");
        std::fs::write(&img, b"fake-android-image-bytes").unwrap();
        let digest = sha256_file(&img).unwrap();

        let manifest_path = tmp.path().join("manifest.json");
        let json = format!(
            r#"{{
                "schema_version": 1,
                "id": "proto-x86_64",
                "name": "Prototype",
                "android_version": "15",
                "arch": "x86_64",
                "boot": {{
                    "type": "bootable_image",
                    "image": {{
                        "relative_path": "android.iso",
                        "sha256": "{digest}",
                        "size_bytes": 24
                    }},
                    "format": "raw",
                    "media": "cdrom"
                }}
            }}"#
        );
        std::fs::write(&manifest_path, json).unwrap();

        let verified = verify_manifest_file(&manifest_path).unwrap();
        assert_eq!(verified.id, "proto-x86_64");

        // Tamper with the artifact and confirm verification fails cleanly.
        std::fs::write(&img, b"tampered-image-bytes!!!!").unwrap();
        let err = verify_manifest_file(&manifest_path).unwrap_err();
        assert!(matches!(err, BifrostError::IntegrityMismatch { .. }));
    }
}
