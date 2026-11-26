//! `.apkm` split bundles: ABI-filtered extraction of split APKs into a staging directory.

use super::icon::png_dimensions;
use super::*;

pub(super) fn inspect_and_stage_apkm_bundle(
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

pub(super) struct ExtractedApkm {
    pub(super) outer_entry_count: usize,
    pub(super) all_split_apks: Vec<PathBuf>,
    pub(super) info_package_name: Option<String>,
    pub(super) info_app_name: Option<String>,
    pub(super) info_version_name: Option<String>,
    pub(super) top_level_icon_png: Option<Vec<u8>>,
}

pub(super) fn is_base_apk_path(path: &Path) -> bool {
    path.file_name()
        .and_then(|n| n.to_str())
        .is_some_and(|n| n.eq_ignore_ascii_case("base.apk"))
}

pub(super) fn extract_apkm_entries(
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
        let name = entry
            .name()
            .map_err(|err| {
                BifrostError::Package(format!(".apkm entry #{idx} has an invalid name: {err}"))
            })?
            .into_owned();

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
