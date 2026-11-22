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

use crate::package::validate_package_name;

const HASH_BUF_BYTES: usize = 64 * 1024;
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

/// Result of offline structural, cryptographic, and ABI inspection of an APK file.
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
    let mut package_name = None;
    let mut abi_set = BTreeSet::new();

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
                    package_name = extract_manifest_package(&raw);
                }
            }
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
        package_name,
        detected_abis,
        abi_compatibility,
    })
}

/// Extracts the highest-quality PNG launcher icon embedded in an APK archive, if present.
#[must_use]
pub fn extract_apk_icon_png(apk_path: &Path) -> Option<Vec<u8>> {
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
        let Some((w, h)) = png_dimensions(&bytes) else {
            continue;
        };
        if w != h || !(48..=512).contains(&w) {
            continue;
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

        if score > best_score {
            best_score = score;
            best_png = Some(bytes);
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
    if axml.len() < 8 || read_u16_le(axml, 0)? != 0x0003 {
        return None;
    }
    let mut pos = usize::from(read_u16_le(axml, 2)?);
    let mut strings: Vec<String> = Vec::new();

    while pos + 8 <= axml.len() {
        let chunk_type = read_u16_le(axml, pos)?;
        let header_size = usize::from(read_u16_le(axml, pos + 2)?);
        let chunk_size = read_u32_le(axml, pos + 4)? as usize;
        if chunk_size < 8 || pos.checked_add(chunk_size)? > axml.len() {
            break;
        }

        if chunk_type == 0x0001 && strings.is_empty() {
            strings = parse_axml_string_pool(&axml[pos..pos + chunk_size], header_size)
                .unwrap_or_default();
        } else if chunk_type == 0x0102 && !strings.is_empty() {
            let ext = pos.checked_add(header_size)?;
            if ext + 20 <= pos + chunk_size {
                let name_idx = read_u32_le(axml, ext + 4)? as usize;
                let attr_start = usize::from(read_u16_le(axml, ext + 8)?);
                let attr_size = usize::from(read_u16_le(axml, ext + 10)?);
                let attr_count = usize::from(read_u16_le(axml, ext + 12)?);

                if strings.get(name_idx).map(String::as_str) == Some("manifest") && attr_size >= 20
                {
                    for i in 0..attr_count {
                        let a_off = ext.checked_add(attr_start)?.checked_add(i * attr_size)?;
                        if a_off + 20 > pos + chunk_size {
                            break;
                        }
                        let attr_name_idx = read_u32_le(axml, a_off + 4)? as usize;
                        if strings.get(attr_name_idx).map(String::as_str) == Some("package") {
                            let raw_val_idx = read_u32_le(axml, a_off + 8)? as usize;
                            let typed_type = axml[a_off + 15];
                            let typed_data = read_u32_le(axml, a_off + 16)? as usize;

                            let candidate = strings.get(raw_val_idx).or_else(|| {
                                if typed_type == 0x03 {
                                    strings.get(typed_data)
                                } else {
                                    None
                                }
                            });
                            if let Some(pkg) = candidate {
                                if validate_package_name(pkg).is_ok() {
                                    return Some(pkg.clone());
                                }
                            }
                        }
                    }
                }
            }
        }

        pos += chunk_size;
    }

    None
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
}
