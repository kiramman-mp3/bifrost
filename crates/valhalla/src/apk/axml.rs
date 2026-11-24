//! Minimal parser for Android binary XML (AXML) manifests: package name and version.

use super::*;

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

pub(super) fn parse_axml_string_pool(chunk: &[u8], header_size: usize) -> Option<Vec<String>> {
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

pub(super) fn read_u16_le(buf: &[u8], off: usize) -> Option<u16> {
    let slice = buf.get(off..off + 2)?;
    Some(u16::from_le_bytes([slice[0], slice[1]]))
}

pub(super) fn read_u32_le(buf: &[u8], off: usize) -> Option<u32> {
    let slice = buf.get(off..off + 4)?;
    Some(u32::from_le_bytes([slice[0], slice[1], slice[2], slice[3]]))
}
