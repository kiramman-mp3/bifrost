//! Safe APK structural inspection, SHA-256 hashing, and ADR-0004 ABI verification.
//!
//! Never executes scripts or external installers; inspects the ZIP structure
//! directly in pure Rust before delegating installation to Android's `PackageManager`.

use std::collections::BTreeSet;
use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};

use bifrost_core::{validate_host_file_path, BifrostError, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use zip::ZipArchive;

const HASH_BUF_BYTES: usize = 64 * 1024;

/// Classification of an APK's native library compatibility with BIFRÖST's x86_64 AOSP baseline.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AbiCompatibility {
    /// No native `.so` libraries found (`lib/` absent); pure DEX/ART app runs natively on x86_64.
    PureArt,
    /// Native `x86_64` libraries are bundled alongside any other ABIs.
    NativeX86_64 { abis: Vec<String> },
    /// Only non-x86_64 native libraries (e.g., `arm64-v8a`) were found; unsupported per ADR-0004.
    UnsupportedNativeAbi { abis: Vec<String> },
}

impl AbiCompatibility {
    /// Returns true if the APK can execute natively on BIFRÖST's clean-room x86_64 runtime.
    #[must_use]
    pub const fn is_supported(&self) -> bool {
        matches!(self, Self::PureArt | Self::NativeX86_64 { .. })
    }
}

/// Result of offline structural, cryptographic, and ABI inspection of an APK file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApkInspection {
    pub path: PathBuf,
    pub size_bytes: u64,
    pub sha256: String,
    pub entry_count: usize,
    pub has_android_manifest: bool,
    pub has_dex: bool,
    pub detected_abis: Vec<String>,
    pub abi_compatibility: AbiCompatibility,
}

/// Validates host path safety, ZIP integrity, `AndroidManifest.xml` presence,
/// SHA-256 digest, and `x86_64` ABI compatibility for a local `.apk` file.
pub fn inspect_apk(path: impl AsRef<Path>) -> Result<ApkInspection> {
    let safe_path = validate_host_file_path(path, Some("apk"))?;
    let metadata = std::fs::metadata(&safe_path).map_err(|source| BifrostError::Io {
        context: format!("inspecting APK '{}'", safe_path.display()),
        source,
    })?;
    if !metadata.is_file() || metadata.len() == 0 {
        return Err(BifrostError::Package(format!(
            "APK '{}' does not exist as a non-empty file",
            safe_path.display()
        )));
    }

    let sha256 = sha256_file(&safe_path)?;
    let file = File::open(&safe_path).map_err(|source| BifrostError::Io {
        context: format!("opening APK '{}'", safe_path.display()),
        source,
    })?;

    let mut archive = ZipArchive::new(file).map_err(|err| {
        BifrostError::Package(format!(
            "file '{}' is not a valid ZIP/APK archive: {err}",
            safe_path.display()
        ))
    })?;

    let entry_count = archive.len();
    let mut has_android_manifest = false;
    let mut has_dex = false;
    let mut abi_set = BTreeSet::new();

    for idx in 0..entry_count {
        let entry = archive.by_index(idx).map_err(|err| {
            BifrostError::Package(format!("failed reading APK entry #{idx}: {err}"))
        })?;
        let name = entry.name();

        // Reject Zip Slip traversal entries inside untrusted APKs.
        if name.contains("..") || name.starts_with('/') || name.starts_with('\\') {
            return Err(BifrostError::UnsafePath {
                path: PathBuf::from(name),
                reason: "APK contains unsafe archive entry path".to_string(),
            });
        }

        if name == "AndroidManifest.xml" {
            has_android_manifest = true;
        } else if name.ends_with(".dex") && !name.contains('/') {
            has_dex = true;
        } else if let Some(rest) = name.strip_prefix("lib/") {
            if let Some((abi, file_part)) = rest.split_once('/') {
                if !abi.is_empty() && file_part.ends_with(".so") {
                    abi_set.insert(abi.to_string());
                }
            }
        }
    }

    if !has_android_manifest {
        return Err(BifrostError::Package(format!(
            "archive '{}' is missing required AndroidManifest.xml",
            safe_path.display()
        )));
    }

    let detected_abis: Vec<String> = abi_set.into_iter().collect();
    let abi_compatibility = if detected_abis.is_empty() {
        AbiCompatibility::PureArt
    } else if detected_abis.iter().any(|abi| abi == "x86_64") {
        AbiCompatibility::NativeX86_64 {
            abis: detected_abis.clone(),
        }
    } else {
        AbiCompatibility::UnsupportedNativeAbi {
            abis: detected_abis.clone(),
        }
    };

    Ok(ApkInspection {
        path: safe_path,
        size_bytes: metadata.len(),
        sha256,
        entry_count,
        has_android_manifest,
        has_dex,
        detected_abis,
        abi_compatibility,
    })
}

fn sha256_file(path: &Path) -> Result<String> {
    let mut file = File::open(path).map_err(|source| BifrostError::Io {
        context: format!("hashing APK '{}'", path.display()),
        source,
    })?;
    let mut hasher = Sha256::new();
    let mut buf = [0u8; HASH_BUF_BYTES];
    loop {
        let n = file.read(&mut buf).map_err(|source| BifrostError::Io {
            context: format!("reading APK '{}'", path.display()),
            source,
        })?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hex::encode(hasher.finalize()))
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use tempfile::TempDir;
    use zip::write::SimpleFileOptions;
    use zip::ZipWriter;

    use super::*;

    fn build_test_apk(path: &Path, entries: &[(&str, &[u8])]) {
        let file = File::create(path).unwrap();
        let mut writer = ZipWriter::new(file);
        let opts = SimpleFileOptions::default();
        for (name, data) in entries {
            writer.start_file(*name, opts).unwrap();
            writer.write_all(data).unwrap();
        }
        writer.finish().unwrap();
    }

    #[test]
    fn accepts_pure_art_and_x86_64_apks() {
        let tmp = TempDir::new().unwrap();
        let pure_apk = tmp.path().join("pure.apk");
        build_test_apk(
            &pure_apk,
            &[
                ("AndroidManifest.xml", b"binary-xml"),
                ("classes.dex", b"dex"),
            ],
        );
        let res = inspect_apk(&pure_apk).unwrap();
        assert_eq!(res.abi_compatibility, AbiCompatibility::PureArt);
        assert!(res.abi_compatibility.is_supported());
        assert!(res.has_dex);

        let native_apk = tmp.path().join("native.apk");
        build_test_apk(
            &native_apk,
            &[
                ("AndroidManifest.xml", b"binary-xml"),
                ("lib/arm64-v8a/libapp.so", b"elf"),
                ("lib/x86_64/libapp.so", b"elf"),
            ],
        );
        let res_native = inspect_apk(&native_apk).unwrap();
        assert!(res_native.abi_compatibility.is_supported());
        assert_eq!(res_native.detected_abis, vec!["arm64-v8a", "x86_64"]);
    }

    #[test]
    fn flags_arm64_only_apk_as_unsupported_abi() {
        let tmp = TempDir::new().unwrap();
        let arm_apk = tmp.path().join("armonly.apk");
        build_test_apk(
            &arm_apk,
            &[
                ("AndroidManifest.xml", b"binary-xml"),
                ("lib/arm64-v8a/libgame.so", b"elf"),
            ],
        );
        let res = inspect_apk(&arm_apk).unwrap();
        assert!(!res.abi_compatibility.is_supported());
        assert_eq!(
            res.abi_compatibility,
            AbiCompatibility::UnsupportedNativeAbi {
                abis: vec!["arm64-v8a".to_string()]
            }
        );
    }

    #[test]
    fn rejects_missing_manifest_and_zip_slip() {
        let tmp = TempDir::new().unwrap();
        let no_manifest = tmp.path().join("bad.apk");
        build_test_apk(&no_manifest, &[("classes.dex", b"dex")]);
        assert!(inspect_apk(&no_manifest).is_err());

        let zip_slip = tmp.path().join("slip.apk");
        build_test_apk(
            &zip_slip,
            &[("AndroidManifest.xml", b"xml"), ("../evil.so", b"payload")],
        );
        let err = inspect_apk(&zip_slip).unwrap_err();
        assert!(matches!(err, BifrostError::UnsafePath { .. }));
    }
}
