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

/// Lists `[images] trusted_keys` with fingerprints. A key that passes the config schema
/// but is not a valid Ed25519 point is still listed, flagged instead of hidden.
pub fn list_trusted_keys() -> Result<Vec<TrustedKeyInfo>> {
    let (_, config, _) = load_config_and_paths()?;
    Ok(config
        .images
        .trusted_keys
        .iter()
        .map(|key| {
            describe_trusted_key(key).unwrap_or_else(|_| TrustedKeyInfo {
                public_key: key.trim().to_ascii_lowercase(),
                fingerprint: "invalid Ed25519 key".to_string(),
            })
        })
        .collect())
}

/// Adds a publisher key to `[images] trusted_keys` after strict Ed25519 validation.
pub fn add_trusted_key(public_key: String) -> Result<Vec<TrustedKeyInfo>> {
    let (config_path, config, _) = load_config_and_paths()?;
    let keys = with_trusted_key(&config.images.trusted_keys, &public_key)?;
    BifrostConfig::write_trusted_keys(&config_path, &keys)?;
    list_trusted_keys()
}

/// IDs of registered images whose detached signature only verifies with `public_key`.
pub fn trusted_key_removal_impact(public_key: String) -> Result<Vec<String>> {
    let (_, config, paths) = load_config_and_paths()?;
    let current = &config.images.trusted_keys;
    let remaining = without_trusted_key(current, &public_key)?;
    let registry = ImageRegistry::load(&paths.images_dir)?;
    Ok(images_losing_trust(&registry, current, &remaining))
}

/// Removes a publisher key from `[images] trusted_keys`.
pub fn remove_trusted_key(public_key: String) -> Result<Vec<TrustedKeyInfo>> {
    let (config_path, config, _) = load_config_and_paths()?;
    let keys = without_trusted_key(&config.images.trusted_keys, &public_key)?;
    BifrostConfig::write_trusted_keys(&config_path, &keys)?;
    list_trusted_keys()
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
