//! Midgard image registry and userdata backup/restore.

use super::*;

/// Lists all registered Midgard runtime image bundles and the active image ID.
pub fn list_images() -> Result<ImageRegistry> {
    let (_, _, paths) = load_config_and_paths()?;
    ImageRegistry::load(&paths.images_dir)
}

/// Verifies a Midgard `manifest.json` bundle (trust policy + SHA-256) and registers it.
///
/// A detached `manifest.json.sig` must verify against `[images] trusted_keys`; unsigned
/// bundles are only accepted when the user explicitly ticks "allow unsigned".
pub fn register_image(
    manifest_path: PathBuf,
    allow_unsigned: bool,
) -> Result<InstalledImageRecord> {
    let (_, config, paths) = load_config_and_paths()?;
    let safe_manifest = validate_host_file_path(&manifest_path, Some("json"))?;
    let trust = evaluate_manifest_trust(
        &safe_manifest,
        None,
        &config.images.trusted_keys,
        allow_unsigned,
    )?;
    let bundle = verify_manifest_file(&safe_manifest)?;
    let mut registry = ImageRegistry::load(&paths.images_dir)?;
    let record =
        registry.register_bundle(&bundle, trust.is_signed(), DEFAULT_USERDATA_COMPAT_VERSION)?;
    registry.save(&paths.images_dir)?;
    Ok(record)
}

/// Switches the active Midgard runtime version after verifying upgrade compatibility.
pub fn activate_image(image_id: String) -> Result<InstalledImageRecord> {
    let (_, _, paths) = load_config_and_paths()?;
    let mut registry = ImageRegistry::load(&paths.images_dir)?;
    let activated = registry.activate(&image_id)?;
    registry.save(&paths.images_dir)?;
    Ok(activated)
}

pub(super) fn default_userdata_path(paths: &BifrostPaths) -> PathBuf {
    paths
        .data_dir
        .join("instances")
        .join("default")
        .join("userdata.img")
}

/// Creates a SHA-256-verified backup of persistent `userdata.img` inside `output_dir`.
pub fn backup_userdata_to(output_dir: PathBuf) -> Result<BackupMetadata> {
    let (_, _, paths) = load_config_and_paths()?;
    let userdata_path = default_userdata_path(&paths);
    backup_userdata(&userdata_path, &output_dir)
}

/// Verifies backup SHA-256 and restores `userdata.img` from `from_dir`.
///
/// Refuses to overwrite live `userdata.img` while Midgard VM is actively running.
pub fn restore_userdata_from(from_dir: PathBuf) -> Result<BackupMetadata> {
    let (_, config, paths) = load_config_and_paths()?;
    let rt = MidgardRuntime::with_default_backend(config, paths.clone());
    if rt.status()?.state.is_active() {
        return Err(BifrostError::Runtime(
            "stop the running Midgard VM before restoring a userdata backup".to_string(),
        ));
    }
    let userdata_path = default_userdata_path(&paths);
    restore_userdata(&from_dir, &userdata_path, true)
}
