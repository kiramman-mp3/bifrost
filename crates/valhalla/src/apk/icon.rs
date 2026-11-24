//! Launcher icon discovery inside APK/APKM archives and PNG-to-ICO wrapping for shortcuts.

use super::apkm::extract_apkm_entries;
use super::*;

pub(super) fn score_icon_candidate(name: &str, bytes: &[u8]) -> Option<u64> {
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

/// Extracts the highest-quality PNG launcher icon embedded in an `.apk` or `.apkm` archive.
#[must_use]
pub fn extract_apk_icon_png(apk_path: &Path) -> Option<Vec<u8>> {
    if is_apkm_bundle(apk_path) {
        return extract_apkm_icon_png(apk_path);
    }
    extract_single_apk_icon_png(apk_path)
}

pub(super) fn extract_apkm_icon_png(apkm_path: &Path) -> Option<Vec<u8>> {
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

pub(super) fn extract_single_apk_icon_png(apk_path: &Path) -> Option<Vec<u8>> {
    let file = File::open(apk_path).ok()?;
    let mut archive = ZipArchive::new(file).ok()?;

    let mut best_score: u64 = 0;
    let mut best_png: Option<Vec<u8>> = None;

    for idx in 0..archive.len() {
        let mut entry = match archive.by_index(idx) {
            Ok(e) => e,
            Err(_) => continue,
        };
        // Best-effort icon scan: skip entries whose names cannot be decoded.
        let Ok(name) = entry.name().map(std::borrow::Cow::into_owned) else {
            continue;
        };
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

pub(super) fn png_dimensions(bytes: &[u8]) -> Option<(u32, u32)> {
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
