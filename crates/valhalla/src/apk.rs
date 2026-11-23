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
    let mut abis = BTreeSet::new();
    let mut best_icon_score: u64 = 0;
    let mut best_icon_png: Option<Vec<u8>> = None;

    for idx in 0..entry_count {
        let mut entry = archive.by_index(idx).map_err(|err| {
            BifrostError::Package(format!("failed reading APK entry #{idx}: {err}"))
        })?;
        let name = entry.name().to_string();

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
        abis,
        best_icon_png,
    })
}

fn score_icon_candidate(name: &str, bytes: &[u8]) -> Option<u64> {
    let (w, h) = png_dimensions(bytes)?;
    if w != h || !(48..=512).contains(&w) {
        return None;
    }
    let lower = name.to_ascii_lowercase();
    let mut score: u64 = u64::from(w) * 100 + bytes.len() as u64;
    if lower.contains("ic_launcher") && !lower.contains("background") {
        score += 10_000_000;
    } else if lower.contains("launcher") || lower.contains("app_icon") {
        score += 5_000_000;
    }
    if matches!(w, 192 | 256 | 512 | 144 | 96) {
        score += 1_000_000;
    }
    Some(score)
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

fn inspect_and_stage_apkm_bundle(
    safe_path: PathBuf,
    size_bytes: u64,
    sha256: String,
    capture_icon: bool,
) -> Result<StagedPackage> {
    let stage_dir = tempfile::TempDir::new().map_err(|source| BifrostError::Io {
        context: "creating staging dir for .apkm bundle".to_string(),
        source,
    })?;
    let extracted = extract_apkm_entries(&safe_path, stage_dir.path(), capture_icon)?;

    let mut has_android_manifest = false;
    let mut has_dex = false;
    let mut package_name = extracted.info_package_name;
    let mut version_name = extracted.info_version_name;
    let mut abi_set = BTreeSet::new();
    let mut compatible_splits = Vec::with_capacity(extracted.all_split_apks.len());
    let mut icon_png = extracted.top_level_icon_png;

    for split_path in &extracted.all_split_apks {
        let need_split_icon = capture_icon && icon_png.is_none() && is_base_apk_path(split_path);
        let summary = inspect_single_apk_archive(split_path, need_split_icon)?;
        has_android_manifest |= summary.has_android_manifest;
        has_dex |= summary.has_dex;
        if package_name.is_none() || is_base_apk_path(split_path) {
            if let Some(pkg) = summary.package_name {
                package_name = Some(pkg);
            }
        }
        if version_name.is_none() || is_base_apk_path(split_path) {
            if let Some(ver) = summary.version_name {
                version_name = Some(ver);
            }
        }
        if icon_png.is_none() {
            icon_png = summary.best_icon_png;
        }
        let is_foreign_abi_split = !summary.abis.is_empty() && !summary.abis.contains("x86_64");
        if !is_foreign_abi_split {
            compatible_splits.push(split_path.clone());
        }
        abi_set.extend(summary.abis);
    }

    if !has_android_manifest {
        return Err(BifrostError::Package(format!(
            "split bundle '{}' contains no APK with AndroidManifest.xml",
            safe_path.display()
        )));
    }

    let detected_abis: Vec<String> = abi_set.into_iter().collect();
    let abi_compatibility = classify_abis(&detected_abis);

    Ok(StagedPackage {
        inspection: ApkInspection {
            path: safe_path,
            size_bytes,
            sha256,
            entry_count: extracted.outer_entry_count,
            has_android_manifest,
            has_dex,
            package_name,
            app_name: extracted.info_app_name,
            version_name,
            is_split_bundle: true,
            split_apk_count: extracted.all_split_apks.len(),
            detected_abis,
            abi_compatibility,
        },
        compatible_splits,
        icon_png,
        _stage_dir: Some(stage_dir),
    })
}

struct ExtractedApkm {
    outer_entry_count: usize,
    all_split_apks: Vec<PathBuf>,
    info_package_name: Option<String>,
    info_app_name: Option<String>,
    info_version_name: Option<String>,
    top_level_icon_png: Option<Vec<u8>>,
}

fn is_base_apk_path(path: &Path) -> bool {
    path.file_name()
        .and_then(|n| n.to_str())
        .is_some_and(|n| n.eq_ignore_ascii_case("base.apk"))
}

fn extract_apkm_entries(
    apkm_path: &Path,
    dest_dir: &Path,
    capture_icon: bool,
) -> Result<ExtractedApkm> {
    let file = File::open(apkm_path).map_err(|source| BifrostError::Io {
        context: format!("opening .apkm bundle '{}'", apkm_path.display()),
        source,
    })?;
    let mut archive = ZipArchive::new(file).map_err(|err| {
        BifrostError::Package(format!(
            "file '{}' is not a valid ZIP/.apkm bundle: {err}",
            apkm_path.display()
        ))
    })?;

    let outer_entry_count = archive.len();
    let mut all_split_apks = Vec::new();
    let mut info_package_name = None;
    let mut info_app_name = None;
    let mut info_version_name = None;
    let mut top_level_icon_png = None;
    let mut copy_buf = vec![0u8; IO_BUF_BYTES];

    for idx in 0..outer_entry_count {
        let mut entry = archive.by_index(idx).map_err(|err| {
            BifrostError::Package(format!("failed reading .apkm entry #{idx}: {err}"))
        })?;
        let name = entry.name().to_string();

        if name.contains("..") || name.starts_with('/') || name.starts_with('\\') {
            return Err(BifrostError::UnsafePath {
                path: PathBuf::from(name),
                reason: ".apkm contains unsafe archive entry path".to_string(),
            });
        }
        if entry.is_dir() {
            continue;
        }

        if name.eq_ignore_ascii_case("info.json") && entry.size() <= 1024 * 1024 {
            let mut raw = String::new();
            if entry.read_to_string(&mut raw).is_ok() {
                if let Ok(json) = serde_json::from_str::<serde_json::Value>(&raw) {
                    if let Some(pname) = json["pname"].as_str() {
                        if validate_package_name(pname).is_ok() {
                            info_package_name = Some(pname.to_string());
                        }
                    }
                    if let Some(aname) = json["app_name"].as_str() {
                        let trimmed = aname.trim();
                        if !trimmed.is_empty() {
                            info_app_name = Some(trimmed.to_string());
                        }
                    }
                    if let Some(ver) = json["release_version"].as_str() {
                        let trimmed = ver.trim();
                        if !trimmed.is_empty() {
                            info_version_name = Some(trimmed.to_string());
                        }
                    }
                }
            }
            continue;
        }

        if capture_icon
            && name.eq_ignore_ascii_case("icon.png")
            && entry.size() > 0
            && entry.size() <= MAX_ICON_PNG_BYTES as u64
        {
            let mut bytes = Vec::new();
            if entry.read_to_end(&mut bytes).is_ok() {
                if let Some((w, h)) = png_dimensions(&bytes) {
                    if w == h && (48..=512).contains(&w) {
                        top_level_icon_png = Some(bytes);
                    }
                }
            }
            continue;
        }

        if name.to_ascii_lowercase().ends_with(".apk") {
            let file_name = Path::new(&name)
                .file_name()
                .and_then(|n| n.to_str())
                .ok_or_else(|| BifrostError::UnsafePath {
                    path: PathBuf::from(&name),
                    reason: "invalid nested APK filename inside .apkm".to_string(),
                })?;
            let out_path = dest_dir.join(file_name);
            let out_file = File::create(&out_path).map_err(|source| BifrostError::Io {
                context: format!("extracting split APK '{}'", out_path.display()),
                source,
            })?;
            let mut writer = BufWriter::with_capacity(IO_BUF_BYTES, out_file);
            loop {
                let n = entry
                    .read(&mut copy_buf)
                    .map_err(|source| BifrostError::Io {
                        context: format!("reading split APK '{name}' from .apkm"),
                        source,
                    })?;
                if n == 0 {
                    break;
                }
                writer
                    .write_all(&copy_buf[..n])
                    .map_err(|source| BifrostError::Io {
                        context: format!("writing split APK '{}'", out_path.display()),
                        source,
                    })?;
            }
            writer.flush().map_err(|source| BifrostError::Io {
                context: format!("flushing split APK '{}'", out_path.display()),
                source,
            })?;
            all_split_apks.push(out_path);
        }
    }

    if all_split_apks.is_empty() {
        return Err(BifrostError::Package(format!(
            "split bundle '{}' does not contain any nested .apk files",
            apkm_path.display()
        )));
    }

    // Place `base.apk` first so `adb install-multiple` always receives the base split first.
    all_split_apks.sort_by(|a, b| {
        let a_base = is_base_apk_path(a);
        let b_base = is_base_apk_path(b);
        b_base.cmp(&a_base).then_with(|| a.cmp(b))
    });

    Ok(ExtractedApkm {
        outer_entry_count,
        all_split_apks,
        info_package_name,
        info_app_name,
        info_version_name,
        top_level_icon_png,
    })
}

/// Extracts the highest-quality PNG launcher icon embedded in an `.apk` or `.apkm` archive.
#[must_use]
pub fn extract_apk_icon_png(apk_path: &Path) -> Option<Vec<u8>> {
    if is_apkm_bundle(apk_path) {
        return extract_apkm_icon_png(apk_path);
    }
    extract_single_apk_icon_png(apk_path)
}

fn extract_apkm_icon_png(apkm_path: &Path) -> Option<Vec<u8>> {
    let file = File::open(apkm_path).ok()?;
    let mut archive = ZipArchive::new(file).ok()?;

    // APKMirror `.apkm` bundles often include a crisp top-level `icon.png`.
    if let Ok(mut icon_entry) = archive.by_name("icon.png") {
        if icon_entry.size() > 0 && icon_entry.size() <= MAX_ICON_PNG_BYTES as u64 {
            let mut bytes = Vec::new();
            if icon_entry.read_to_end(&mut bytes).is_ok() {
                if let Some((w, h)) = png_dimensions(&bytes) {
                    if w == h && (48..=512).contains(&w) {
                        return Some(bytes);
                    }
                }
            }
        }
    }

    // Fall back to scanning `base.apk` inside the `.apkm` bundle.
    let tmp = tempfile::TempDir::new().ok()?;
    let extracted = extract_apkm_entries(apkm_path, tmp.path(), false).ok()?;
    for split in extracted.all_split_apks {
        if let Some(png) = extract_single_apk_icon_png(&split) {
            return Some(png);
        }
    }
    None
}

fn extract_single_apk_icon_png(apk_path: &Path) -> Option<Vec<u8>> {
    let file = File::open(apk_path).ok()?;
    let mut archive = ZipArchive::new(file).ok()?;

    let mut best_score: u64 = 0;
    let mut best_png: Option<Vec<u8>> = None;

    for idx in 0..archive.len() {
        let mut entry = match archive.by_index(idx) {
            Ok(e) => e,
            Err(_) => continue,
        };
        let name = entry.name().to_string();
        if !name.starts_with("res/")
            || !name.ends_with(".png")
            || name.ends_with(".9.png")
            || entry.size() == 0
            || entry.size() > MAX_ICON_PNG_BYTES as u64
        {
            continue;
        }

        let mut bytes = Vec::new();
        if entry.read_to_end(&mut bytes).is_err() {
            continue;
        }
        if let Some(score) = score_icon_candidate(&name, &bytes) {
            if score > best_score {
                best_score = score;
                best_png = Some(bytes);
            }
        }
    }

    best_png
}

/// Wraps raw PNG bytes into a single-entry Windows Vista/10/11 `.ico` container.
#[must_use]
pub fn wrap_png_as_ico(png_bytes: &[u8]) -> Option<Vec<u8>> {
    let (w, h) = png_dimensions(png_bytes)?;
    let len_u32 = u32::try_from(png_bytes.len()).ok()?;
    let w_byte = if w >= 256 { 0u8 } else { w as u8 };
    let h_byte = if h >= 256 { 0u8 } else { h as u8 };

    let mut ico = Vec::with_capacity(22 + png_bytes.len());
    // ICONDIR (6 bytes): reserved=0, type=1 (ICO), count=1.
    ico.extend_from_slice(&[0, 0, 1, 0, 1, 0]);
    // ICONDIRENTRY (16 bytes)
    ico.push(w_byte);
    ico.push(h_byte);
    ico.push(0); // color count
    ico.push(0); // reserved
    ico.extend_from_slice(&1u16.to_le_bytes()); // color planes
    ico.extend_from_slice(&32u16.to_le_bytes()); // bits per pixel
    ico.extend_from_slice(&len_u32.to_le_bytes()); // PNG byte length
    ico.extend_from_slice(&22u32.to_le_bytes()); // image data offset (6 + 16)
    ico.extend_from_slice(png_bytes);
    Some(ico)
}

fn png_dimensions(bytes: &[u8]) -> Option<(u32, u32)> {
    const PNG_SIG: &[u8; 8] = b"\x89PNG\r\n\x1a\n";
    if bytes.len() < 24 || &bytes[0..8] != PNG_SIG || &bytes[12..16] != b"IHDR" {
        return None;
    }
    let w = u32::from_be_bytes([bytes[16], bytes[17], bytes[18], bytes[19]]);
    let h = u32::from_be_bytes([bytes[20], bytes[21], bytes[22], bytes[23]]);
    if w == 0 || h == 0 {
        return None;
    }
    Some((w, h))
}

/// Extracts the Android `package` attribute from a binary AXML `AndroidManifest.xml`.
#[must_use]
pub fn extract_manifest_package(axml: &[u8]) -> Option<String> {
    extract_manifest_metadata(axml).0
}

/// Extracts `(package, versionName)` from a binary AXML `AndroidManifest.xml`.
#[must_use]
pub fn extract_manifest_metadata(axml: &[u8]) -> (Option<String>, Option<String>) {
    let Some(magic) = read_u16_le(axml, 0) else {
        return (None, None);
    };
    if axml.len() < 8 || magic != 0x0003 {
        return (None, None);
    }
    let Some(start) = read_u16_le(axml, 2) else {
        return (None, None);
    };
    let mut pos = usize::from(start);
    let mut strings: Vec<String> = Vec::new();
    let mut package_name: Option<String> = None;
    let mut version_name: Option<String> = None;

    while pos + 8 <= axml.len() {
        let Some(chunk_type) = read_u16_le(axml, pos) else {
            break;
        };
        let Some(header_raw) = read_u16_le(axml, pos + 2) else {
            break;
        };
        let Some(chunk_raw) = read_u32_le(axml, pos + 4) else {
            break;
        };
        let header_size = usize::from(header_raw);
        let chunk_size = chunk_raw as usize;
        if chunk_size < 8 {
            break;
        }
        let Some(chunk_end) = pos.checked_add(chunk_size) else {
            break;
        };
        if chunk_end > axml.len() {
            break;
        }

        if chunk_type == 0x0001 && strings.is_empty() {
            strings = parse_axml_string_pool(&axml[pos..pos + chunk_size], header_size)
                .unwrap_or_default();
        } else if chunk_type == 0x0102 && !strings.is_empty() {
            if let Some(ext) = pos.checked_add(header_size) {
                if ext + 20 <= pos + chunk_size {
                    let name_idx = read_u32_le(axml, ext + 4).unwrap_or(u32::MAX) as usize;
                    let attr_start = usize::from(read_u16_le(axml, ext + 8).unwrap_or(0));
                    let attr_size = usize::from(read_u16_le(axml, ext + 10).unwrap_or(0));
                    let attr_count = usize::from(read_u16_le(axml, ext + 12).unwrap_or(0));

                    if strings.get(name_idx).map(String::as_str) == Some("manifest")
                        && attr_size >= 20
                    {
                        for i in 0..attr_count {
                            let Some(a_off) = ext
                                .checked_add(attr_start)
                                .and_then(|v| v.checked_add(i * attr_size))
                            else {
                                break;
                            };
                            if a_off + 20 > pos + chunk_size {
                                break;
                            }
                            let attr_name_idx =
                                read_u32_le(axml, a_off + 4).unwrap_or(u32::MAX) as usize;
                            let attr_name = strings.get(attr_name_idx).map(String::as_str);
                            if matches!(attr_name, Some("package" | "versionName")) {
                                let raw_val_idx =
                                    read_u32_le(axml, a_off + 8).unwrap_or(u32::MAX) as usize;
                                let typed_type = axml[a_off + 15];
                                let typed_data =
                                    read_u32_le(axml, a_off + 16).unwrap_or(u32::MAX) as usize;

                                let candidate = strings.get(raw_val_idx).or_else(|| {
                                    if typed_type == 0x03 {
                                        strings.get(typed_data)
                                    } else {
                                        None
                                    }
                                });
                                if let Some(val) = candidate {
                                    if attr_name == Some("package")
                                        && validate_package_name(val).is_ok()
                                    {
                                        package_name = Some(val.clone());
                                    } else if attr_name == Some("versionName") {
                                        let trimmed = val.trim();
                                        if !trimmed.is_empty() {
                                            version_name = Some(trimmed.to_string());
                                        }
                                    }
                                }
                            }
                        }
                        break;
                    }
                }
            }
        }

        pos += chunk_size;
    }

    (package_name, version_name)
}

fn parse_axml_string_pool(chunk: &[u8], header_size: usize) -> Option<Vec<String>> {
    if chunk.len() < 28 || header_size < 28 {
        return None;
    }
    let string_count = read_u32_le(chunk, 8)? as usize;
    let flags = read_u32_le(chunk, 16)?;
    let strings_start = read_u32_le(chunk, 20)? as usize;
    let is_utf8 = (flags & (1 << 8)) != 0;

    if string_count > 100_000 || strings_start >= chunk.len() {
        return None;
    }

    let mut out = Vec::with_capacity(string_count);
    for i in 0..string_count {
        let off_pos = header_size.checked_add(i * 4)?;
        let rel = read_u32_le(chunk, off_pos)? as usize;
        let mut s_pos = strings_start.checked_add(rel)?;
        if s_pos >= chunk.len() {
            out.push(String::new());
            continue;
        }

        if is_utf8 {
            // Skip UTF-16 character length (1 or 2 bytes)
            let b0 = *chunk.get(s_pos)?;
            s_pos += if (b0 & 0x80) != 0 { 2 } else { 1 };
            // Read UTF-8 byte length (1 or 2 bytes)
            let l0 = *chunk.get(s_pos)?;
            let byte_len = if (l0 & 0x80) != 0 {
                let l1 = *chunk.get(s_pos + 1)?;
                s_pos += 2;
                (usize::from(l0 & 0x7F) << 8) | usize::from(l1)
            } else {
                s_pos += 1;
                usize::from(l0)
            };
            let end = s_pos.checked_add(byte_len)?.min(chunk.len());
            out.push(String::from_utf8_lossy(&chunk[s_pos..end]).into_owned());
        } else {
            let l0 = read_u16_le(chunk, s_pos)?;
            let char_len = if (l0 & 0x8000) != 0 {
                let l1 = read_u16_le(chunk, s_pos + 2)?;
                s_pos += 4;
                ((usize::from(l0 & 0x7FFF)) << 16) | usize::from(l1)
            } else {
                s_pos += 2;
                usize::from(l0)
            };
            let byte_len = char_len.checked_mul(2)?;
            let end = s_pos.checked_add(byte_len)?.min(chunk.len());
            let mut units = Vec::with_capacity(char_len);
            let mut cur = s_pos;
            while cur + 2 <= end {
                units.push(u16::from_le_bytes([chunk[cur], chunk[cur + 1]]));
                cur += 2;
            }
            out.push(String::from_utf16_lossy(&units));
        }
    }

    Some(out)
}

fn read_u16_le(buf: &[u8], off: usize) -> Option<u16> {
    let slice = buf.get(off..off + 2)?;
    Some(u16::from_le_bytes([slice[0], slice[1]]))
}

fn read_u32_le(buf: &[u8], off: usize) -> Option<u32> {
    let slice = buf.get(off..off + 4)?;
    Some(u32::from_le_bytes([slice[0], slice[1], slice[2], slice[3]]))
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
