//! Safe APK structural inspection, SHA-256 hashing, and ADR-0004 ABI verification.
//!
//! Never executes scripts or external installers; inspects the ZIP structure
//! directly in pure Rust before delegating installation to Android's `PackageManager`.

use std::collections::BTreeSet;
use std::fs::File;
use std::io::{BufWriter, Read, Write};
use std::path::{Path, PathBuf};

use bifrost_core::{validate_host_file_path, BifrostError, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use zip::ZipArchive;

use crate::package::validate_package_name;

mod apkm;
mod axml;
mod icon;

use apkm::inspect_and_stage_apkm_bundle;
pub use axml::{
    extract_manifest_metadata, extract_manifest_package, extract_required_gles_version,
    format_gles_version,
};
use icon::score_icon_candidate;
pub use icon::{extract_apk_icon_png, wrap_png_as_ico};

const IO_BUF_BYTES: usize = 512 * 1024;
const MAX_MANIFEST_BYTES: usize = 8 * 1024 * 1024;
const MAX_ICON_PNG_BYTES: usize = 2 * 1024 * 1024;

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

/// Result of offline structural, cryptographic, and ABI inspection of an `.apk` or `.apkm` file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApkInspection {
    pub path: PathBuf,
    pub size_bytes: u64,
    pub sha256: String,
    pub entry_count: usize,
    pub has_android_manifest: bool,
    pub has_dex: bool,
    #[serde(default)]
    pub package_name: Option<String>,
    #[serde(default)]
    pub app_name: Option<String>,
    #[serde(default)]
    pub version_name: Option<String>,
    /// Highest OpenGL ES version required by `<uses-feature android:glEsVersion>`
    /// (`major << 16 | minor`), or `None` when the manifest declares none.
    #[serde(default)]
    pub required_gles_version: Option<u32>,
    #[serde(default)]
    pub is_split_bundle: bool,
    #[serde(default)]
    pub split_apk_count: usize,
    pub detected_abis: Vec<String>,
    pub abi_compatibility: AbiCompatibility,
}

/// Validates that a host file path is safe (no traversal/control chars) and has an
/// `.apk` or `.apkm` extension.
pub fn validate_package_file_path(path: impl AsRef<Path>) -> Result<PathBuf> {
    let safe_path = validate_host_file_path(path, None)?;
    let ext = safe_path
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase);
    match ext.as_deref() {
        Some("apk" | "apkm") => Ok(safe_path),
        _ => Err(BifrostError::UnsafePath {
            path: safe_path,
            reason: "expected file extension '.apk' or '.apkm'".to_string(),
        }),
    }
}

/// Returns true if `path` has the `.apkm` split-APK bundle extension.
#[must_use]
pub fn is_apkm_bundle(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("apkm"))
}

/// Result of single-pass package inspection and split staging ready for installation.
pub(crate) struct StagedPackage {
    pub inspection: ApkInspection,
    pub compatible_splits: Vec<PathBuf>,
    pub icon_png: Option<Vec<u8>>,
    pub _stage_dir: Option<tempfile::TempDir>,
}

/// Validates host path safety, ZIP integrity, `AndroidManifest.xml` presence,
/// SHA-256 digest, and `x86_64` ABI compatibility for a local `.apk` or `.apkm` file.
pub fn inspect_apk(path: impl AsRef<Path>) -> Result<ApkInspection> {
    Ok(inspect_and_stage_package(path, false)?.inspection)
}

/// Inspects a local `.apk` or `.apkm` package and extracts its PNG launcher icon in a
/// single pass for graphical installer preview dialogs.
pub fn inspect_package_preview(path: impl AsRef<Path>) -> Result<(ApkInspection, Option<Vec<u8>>)> {
    let staged = inspect_and_stage_package(path, true)?;
    Ok((staged.inspection, staged.icon_png))
}

/// Performs single-pass inspection, ABI validation, optional launcher icon extraction,
/// and split-APK staging for `.apk` and `.apkm` packages without redundant decompression.
pub(crate) fn inspect_and_stage_package(
    path: impl AsRef<Path>,
    capture_icon: bool,
) -> Result<StagedPackage> {
    let safe_path = validate_package_file_path(path)?;
    let metadata = std::fs::metadata(&safe_path).map_err(|source| BifrostError::Io {
        context: format!("inspecting package '{}'", safe_path.display()),
        source,
    })?;
    if !metadata.is_file() || metadata.len() == 0 {
        return Err(BifrostError::Package(format!(
            "package '{}' does not exist as a non-empty file",
            safe_path.display()
        )));
    }

    let sha256 = sha256_file(&safe_path)?;
    if is_apkm_bundle(&safe_path) {
        return inspect_and_stage_apkm_bundle(safe_path, metadata.len(), sha256, capture_icon);
    }

    let single = inspect_single_apk_archive(&safe_path, capture_icon)?;
    if !single.has_android_manifest {
        return Err(BifrostError::Package(format!(
            "archive '{}' is missing required AndroidManifest.xml",
            safe_path.display()
        )));
    }

    let detected_abis: Vec<String> = single.abis.into_iter().collect();
    let abi_compatibility = classify_abis(&detected_abis);
    let compatible_splits = vec![safe_path.clone()];

    Ok(StagedPackage {
        inspection: ApkInspection {
            path: safe_path,
            size_bytes: metadata.len(),
            sha256,
            entry_count: single.entry_count,
            has_android_manifest: single.has_android_manifest,
            has_dex: single.has_dex,
            package_name: single.package_name,
            app_name: None,
            version_name: single.version_name,
            required_gles_version: single.required_gles_version,
            is_split_bundle: false,
            split_apk_count: 1,
            detected_abis,
            abi_compatibility,
        },
        compatible_splits,
        icon_png: single.best_icon_png,
        _stage_dir: None,
    })
}

struct SingleApkSummary {
    entry_count: usize,
    has_android_manifest: bool,
    has_dex: bool,
    package_name: Option<String>,
    version_name: Option<String>,
    required_gles_version: Option<u32>,
    abis: BTreeSet<String>,
    best_icon_png: Option<Vec<u8>>,
}

fn inspect_single_apk_archive(path: &Path, capture_icon: bool) -> Result<SingleApkSummary> {
    let file = File::open(path).map_err(|source| BifrostError::Io {
        context: format!("opening APK '{}'", path.display()),
        source,
    })?;

    let mut archive = ZipArchive::new(file).map_err(|err| {
        BifrostError::Package(format!(
            "file '{}' is not a valid ZIP/APK archive: {err}",
            path.display()
        ))
    })?;

    let entry_count = archive.len();
    let mut has_android_manifest = false;
    let mut has_dex = false;
    let mut package_name = None;
    let mut version_name = None;
    let mut required_gles_version = None;
    let mut abis = BTreeSet::new();
    let mut best_icon_score: u64 = 0;
    let mut best_icon_png: Option<Vec<u8>> = None;

    for idx in 0..entry_count {
        let mut entry = archive.by_index(idx).map_err(|err| {
            BifrostError::Package(format!("failed reading APK entry #{idx}: {err}"))
        })?;
        // zip >= 9 rejects names it cannot decode; an APK with such an entry is malformed.
        let name = entry
            .name()
            .map_err(|err| {
                BifrostError::Package(format!("APK entry #{idx} has an invalid name: {err}"))
            })?
            .into_owned();

        // Reject Zip Slip traversal entries inside untrusted APKs.
        if name.contains("..") || name.starts_with('/') || name.starts_with('\\') {
            return Err(BifrostError::UnsafePath {
                path: PathBuf::from(name),
                reason: "APK contains unsafe archive entry path".to_string(),
            });
        }

        if name == "AndroidManifest.xml" {
            has_android_manifest = true;
            if entry.size() <= MAX_MANIFEST_BYTES as u64 {
                let mut raw = Vec::new();
                if entry.read_to_end(&mut raw).is_ok() {
                    let (pkg, ver) = extract_manifest_metadata(&raw);
                    package_name = pkg;
                    version_name = ver;
                    required_gles_version = extract_required_gles_version(&raw);
                }
            }
        } else if name.ends_with(".dex") && !name.contains('/') {
            has_dex = true;
        } else if let Some(rest) = name.strip_prefix("lib/") {
            if let Some((abi, file_part)) = rest.split_once('/') {
                if !abi.is_empty() && file_part.ends_with(".so") {
                    abis.insert(abi.to_string());
                }
            }
        } else if capture_icon
            && name.starts_with("res/")
            && name.ends_with(".png")
            && !name.ends_with(".9.png")
            && entry.size() > 0
            && entry.size() <= MAX_ICON_PNG_BYTES as u64
        {
            let mut bytes = Vec::new();
            if entry.read_to_end(&mut bytes).is_ok() {
                if let Some(score) = score_icon_candidate(&name, &bytes) {
                    if score > best_icon_score {
                        best_icon_score = score;
                        best_icon_png = Some(bytes);
                    }
                }
            }
        }
    }

    Ok(SingleApkSummary {
        entry_count,
        has_android_manifest,
        has_dex,
        package_name,
        version_name,
        required_gles_version,
        abis,
        best_icon_png,
    })
}

fn classify_abis(detected_abis: &[String]) -> AbiCompatibility {
    if detected_abis.is_empty() {
        AbiCompatibility::PureArt
    } else if detected_abis.iter().any(|abi| abi == "x86_64") {
        AbiCompatibility::NativeX86_64 {
            abis: detected_abis.to_vec(),
        }
    } else {
        AbiCompatibility::UnsupportedNativeAbi {
            abis: detected_abis.to_vec(),
        }
    }
}

fn sha256_file(path: &Path) -> Result<String> {
    let mut file = File::open(path).map_err(|source| BifrostError::Io {
        context: format!("hashing APK '{}'", path.display()),
        source,
    })?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; IO_BUF_BYTES];
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

    #[test]
    fn wraps_png_into_valid_windows_ico_and_extracts_from_apk() {
        let mut fake_png = Vec::new();
        fake_png.extend_from_slice(b"\x89PNG\r\n\x1a\n");
        fake_png.extend_from_slice(&13u32.to_be_bytes());
        fake_png.extend_from_slice(b"IHDR");
        fake_png.extend_from_slice(&192u32.to_be_bytes());
        fake_png.extend_from_slice(&192u32.to_be_bytes());
        fake_png.extend_from_slice(&[8, 6, 0, 0, 0]);

        let ico = wrap_png_as_ico(&fake_png).unwrap();
        assert_eq!(&ico[0..6], &[0, 0, 1, 0, 1, 0]);
        assert_eq!(ico[6], 192);
        assert_eq!(ico[7], 192);
        assert_eq!(&ico[22..], fake_png.as_slice());

        let tmp = TempDir::new().unwrap();
        let apk = tmp.path().join("icon.apk");
        build_test_apk(
            &apk,
            &[
                ("AndroidManifest.xml", b"xml"),
                ("res/mipmap-xxxhdpi/ic_launcher.png", &fake_png),
            ],
        );
        let extracted = extract_apk_icon_png(&apk).unwrap();
        assert_eq!(extracted, fake_png);
    }

    #[test]
    fn inspects_multi_abi_apkm_bundle_and_filters_foreign_abi_splits() {
        let tmp = TempDir::new().unwrap();
        let base_apk = tmp.path().join("base.apk");
        build_test_apk(
            &base_apk,
            &[
                ("AndroidManifest.xml", b"binary-xml"),
                ("classes.dex", b"dex"),
            ],
        );
        let x86_split = tmp.path().join("split_config.x86_64.apk");
        build_test_apk(
            &x86_split,
            &[
                ("AndroidManifest.xml", b"binary-xml"),
                ("lib/x86_64/libapp.so", b"elf"),
            ],
        );
        let arm_split = tmp.path().join("split_config.arm64_v8a.apk");
        build_test_apk(
            &arm_split,
            &[
                ("AndroidManifest.xml", b"binary-xml"),
                ("lib/arm64-v8a/libapp.so", b"elf"),
            ],
        );

        let apkm = tmp.path().join("sample-app.apkm");
        build_test_apk(
            &apkm,
            &[
                (
                    "info.json",
                    br#"{"pname":"com.example.splitapp","app_name":"Split Demo"}"#,
                ),
                ("base.apk", &std::fs::read(&base_apk).unwrap()),
                (
                    "split_config.x86_64.apk",
                    &std::fs::read(&x86_split).unwrap(),
                ),
                (
                    "split_config.arm64_v8a.apk",
                    &std::fs::read(&arm_split).unwrap(),
                ),
            ],
        );

        let staged = inspect_and_stage_package(&apkm, true).unwrap();
        assert!(staged.inspection.is_split_bundle);
        assert_eq!(staged.inspection.split_apk_count, 3);
        assert_eq!(
            staged.inspection.package_name.as_deref(),
            Some("com.example.splitapp")
        );
        assert_eq!(staged.inspection.app_name.as_deref(), Some("Split Demo"));
        assert!(staged.inspection.abi_compatibility.is_supported());
        assert_eq!(staged.inspection.detected_abis, vec!["arm64-v8a", "x86_64"]);

        let names: Vec<String> = staged
            .compatible_splits
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, vec!["base.apk", "split_config.x86_64.apk"]);
    }

    #[test]
    fn flags_arm64_only_apkm_bundle_as_unsupported() {
        let tmp = TempDir::new().unwrap();
        let base_apk = tmp.path().join("base.apk");
        build_test_apk(
            &base_apk,
            &[
                ("AndroidManifest.xml", b"binary-xml"),
                ("classes.dex", b"dex"),
            ],
        );
        let arm_split = tmp.path().join("split_config.arm64_v8a.apk");
        build_test_apk(
            &arm_split,
            &[
                ("AndroidManifest.xml", b"binary-xml"),
                ("lib/arm64-v8a/libonlyarm.so", b"elf"),
            ],
        );

        let apkm = tmp.path().join("arm-only.apkm");
        build_test_apk(
            &apkm,
            &[
                ("base.apk", &std::fs::read(&base_apk).unwrap()),
                (
                    "split_config.arm64_v8a.apk",
                    &std::fs::read(&arm_split).unwrap(),
                ),
            ],
        );

        let inspected = inspect_apk(&apkm).unwrap();
        assert!(inspected.is_split_bundle);
        assert!(!inspected.abi_compatibility.is_supported());
    }
}
