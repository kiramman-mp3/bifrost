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

/// Extracts the highest OpenGL ES version an AXML `AndroidManifest.xml` requires through
/// `<uses-feature android:glEsVersion="0x00030000">`, encoded as `major << 16 | minor`.
/// Returns `None` when the manifest declares no requirement (Android then assumes 1.0).
#[must_use]
pub fn extract_required_gles_version(axml: &[u8]) -> Option<u32> {
    if read_u16_le(axml, 0)? != 0x0003 || axml.len() < 8 {
        return None;
    }
    let mut pos = usize::from(read_u16_le(axml, 2)?);
    let mut strings: Vec<String> = Vec::new();
    let mut required: Option<u32> = None;

    while pos + 8 <= axml.len() {
        let chunk_type = read_u16_le(axml, pos)?;
        let header_size = usize::from(read_u16_le(axml, pos + 2)?);
        let chunk_size = read_u32_le(axml, pos + 4)? as usize;
        if chunk_size < 8 {
            break;
        }
        let chunk_end = pos.checked_add(chunk_size)?;
        if chunk_end > axml.len() {
            break;
        }

        if chunk_type == 0x0001 && strings.is_empty() {
            strings =
                parse_axml_string_pool(&axml[pos..chunk_end], header_size).unwrap_or_default();
        } else if chunk_type == 0x0102 && !strings.is_empty() {
            if let Some(version) = uses_feature_gles(axml, &strings, pos, header_size, chunk_end) {
                required = Some(required.map_or(version, |r| r.max(version)));
            }
        }
        pos = chunk_end;
    }

    required
}

/// Reads `android:glEsVersion` from one start-element chunk when it is a `<uses-feature>`.
fn uses_feature_gles(
    axml: &[u8],
    strings: &[String],
    pos: usize,
    header_size: usize,
    chunk_end: usize,
) -> Option<u32> {
    let ext = pos.checked_add(header_size)?;
    if ext + 20 > chunk_end {
        return None;
    }
    let name_idx = read_u32_le(axml, ext + 4)? as usize;
    if strings.get(name_idx).map(String::as_str) != Some("uses-feature") {
        return None;
    }
    let attr_start = usize::from(read_u16_le(axml, ext + 8)?);
    let attr_size = usize::from(read_u16_le(axml, ext + 10)?);
    let attr_count = usize::from(read_u16_le(axml, ext + 12)?);
    if attr_size < 20 {
        return None;
    }
    for i in 0..attr_count {
        let a_off = ext
            .checked_add(attr_start)?
            .checked_add(i.checked_mul(attr_size)?)?;
        if a_off + 20 > chunk_end {
            break;
        }
        let attr_name_idx = read_u32_le(axml, a_off + 4)? as usize;
        if strings.get(attr_name_idx).map(String::as_str) != Some("glEsVersion") {
            continue;
        }
        // Typed value: 0x10 = decimal integer, 0x11 = hexadecimal integer.
        let typed_type = *axml.get(a_off + 15)?;
        if matches!(typed_type, 0x10 | 0x11) {
            let value = read_u32_le(axml, a_off + 16)?;
            return (value > 0).then_some(value);
        }
    }
    None
}

/// Formats an encoded OpenGL ES version (`0x00030001`) as `3.1`.
#[must_use]
pub fn format_gles_version(encoded: u32) -> String {
    format!("{}.{}", encoded >> 16, encoded & 0xFFFF)
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

#[cfg(test)]
mod tests {
    use super::*;

    /// One `(element, [(attribute, typed type, data)])` per start tag.
    type Element<'a> = (&'a str, &'a [(&'a str, u8, u32)]);
    /// An interned attribute: (name index, typed type, data).
    type Attr = (u32, u8, u32);

    /// Builds a minimal binary AXML document: a UTF-16 string pool plus start-element chunks.
    fn build_axml(elements: &[Element<'_>]) -> Vec<u8> {
        let mut strings: Vec<&str> = Vec::new();
        let mut intern = |s| match strings.iter().position(|x| *x == s) {
            Some(i) => i as u32,
            None => {
                strings.push(s);
                (strings.len() - 1) as u32
            }
        };
        let tags: Vec<(u32, Vec<Attr>)> = elements
            .iter()
            .map(|(name, attrs)| {
                let name_idx = intern(name);
                let attrs = attrs.iter().map(|(a, t, d)| (intern(a), *t, *d)).collect();
                (name_idx, attrs)
            })
            .collect();

        let mut data = Vec::new();
        let mut offsets = Vec::new();
        for s in &strings {
            offsets.push(data.len() as u32);
            let units: Vec<u16> = s.encode_utf16().collect();
            data.extend((units.len() as u16).to_le_bytes());
            for u in units {
                data.extend(u.to_le_bytes());
            }
            data.extend(0u16.to_le_bytes());
        }
        let header = 28usize;
        let strings_start = header + offsets.len() * 4;
        let mut pool = Vec::new();
        pool.extend(0x0001u16.to_le_bytes());
        pool.extend((header as u16).to_le_bytes());
        pool.extend(((strings_start + data.len()) as u32).to_le_bytes());
        pool.extend((offsets.len() as u32).to_le_bytes());
        pool.extend(0u32.to_le_bytes()); // style count
        pool.extend(0u32.to_le_bytes()); // flags: UTF-16
        pool.extend((strings_start as u32).to_le_bytes());
        pool.extend(0u32.to_le_bytes()); // styles start
        for o in offsets {
            pool.extend(o.to_le_bytes());
        }
        pool.extend(data);

        let mut body = Vec::new();
        for (name_idx, attrs) in tags {
            let size = 16 + 20 + attrs.len() * 20;
            body.extend(0x0102u16.to_le_bytes());
            body.extend(16u16.to_le_bytes());
            body.extend((size as u32).to_le_bytes());
            body.extend(1u32.to_le_bytes()); // line number
            body.extend(u32::MAX.to_le_bytes()); // comment
            body.extend(u32::MAX.to_le_bytes()); // namespace
            body.extend(name_idx.to_le_bytes());
            body.extend(20u16.to_le_bytes()); // attribute start
            body.extend(20u16.to_le_bytes()); // attribute size
            body.extend((attrs.len() as u16).to_le_bytes());
            body.extend([0u8; 6]); // id, class, style indexes
            for (attr_idx, typed_type, typed_data) in attrs {
                body.extend(u32::MAX.to_le_bytes()); // namespace
                body.extend(attr_idx.to_le_bytes());
                body.extend(u32::MAX.to_le_bytes()); // raw value
                body.extend(8u16.to_le_bytes());
                body.push(0);
                body.push(typed_type);
                body.extend(typed_data.to_le_bytes());
            }
        }

        let mut out = Vec::new();
        out.extend(0x0003u16.to_le_bytes());
        out.extend(8u16.to_le_bytes());
        out.extend(((8 + pool.len() + body.len()) as u32).to_le_bytes());
        out.extend(pool);
        out.extend(body);
        out
    }

    #[test]
    fn reads_the_highest_required_gles_version() {
        let axml = build_axml(&[
            ("manifest", &[]),
            ("uses-feature", &[("name", 0x03, 0)]),
            ("uses-feature", &[("glEsVersion", 0x11, 0x0002_0000)]),
            ("uses-feature", &[("glEsVersion", 0x11, 0x0003_0001)]),
            ("application", &[]),
        ]);
        assert_eq!(extract_required_gles_version(&axml), Some(0x0003_0001));
        assert_eq!(format_gles_version(0x0003_0001), "3.1");
    }

    #[test]
    fn manifests_without_a_gles_feature_require_nothing() {
        let axml = build_axml(&[
            ("manifest", &[]),
            // Same attribute name outside <uses-feature> is ignored.
            ("application", &[("glEsVersion", 0x10, 0x0003_0000)]),
        ]);
        assert_eq!(extract_required_gles_version(&axml), None);
        assert_eq!(extract_required_gles_version(b"not axml"), None);
    }
}
