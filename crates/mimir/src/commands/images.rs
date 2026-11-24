//! `bifrost images` (registry, verification and trust of Midgard bundles).

use super::*;

pub(super) fn handle_images(
    action: &ImagesAction,
    explicit_config: Option<&PathBuf>,
    as_json: bool,
) -> Result<String> {
    let (_, _, config, paths) = load_effective_config_and_paths(explicit_config)?;
    let mut registry = ImageRegistry::load(&paths.images_dir)?;

    match action {
        ImagesAction::List => {
            if as_json {
                return serde_json::to_string_pretty(&registry)
                    .map_err(|err| BifrostError::ConfigSerialize(err.to_string()));
            }
            if registry.images.is_empty() {
                return Ok("No Midgard runtime images registered yet.".to_string());
            }
            let mut out = format!(
                "Registered Midgard Runtime Images ({}):\n",
                registry.images.len()
            );
            for img in &registry.images {
                let marker = if registry.active_image_id.as_deref() == Some(img.id.as_str()) {
                    "* "
                } else {
                    "  "
                };
                let sig = if img.signature_verified {
                    "signed:ed25519"
                } else {
                    "sha256-only"
                };
                out.push_str(&format!(
                    "{marker}{:<24} Android {:<6} [{} | compat v{}] {}\n",
                    img.id,
                    img.android_version,
                    sig,
                    img.userdata_compat_version,
                    img.manifest_path.display()
                ));
            }
            Ok(out.trim_end().to_string())
        }
        ImagesAction::Register {
            manifest,
            signature,
            allow_unsigned,
            userdata_compat,
        } => {
            let trust = evaluate_manifest_trust(
                manifest,
                signature.as_deref(),
                &config.images.trusted_keys,
                *allow_unsigned,
            )?;
            let signature_verified = trust.is_signed();
            if let ImageTrust::UnsignedAllowed = trust {
                tracing::warn!(
                    manifest = %manifest.display(),
                    "registering UNSIGNED Midgard image (sha256-only) by explicit --allow-unsigned"
                );
            }

            let bundle = verify_manifest_file(manifest)?;
            let record = registry.register_bundle(&bundle, signature_verified, *userdata_compat)?;
            registry.save(&paths.images_dir)?;

            if as_json {
                return serde_json::to_string_pretty(&record)
                    .map_err(|err| BifrostError::ConfigSerialize(err.to_string()));
            }
            Ok(format!(
                "Registered verified Midgard image '{}' (Android {}, signature_verified={}).",
                record.id, record.android_version, record.signature_verified
            ))
        }
        ImagesAction::Use { image_id } => {
            let activated = registry.activate(image_id)?;
            registry.save(&paths.images_dir)?;

            if as_json {
                return serde_json::to_string_pretty(&activated)
                    .map_err(|err| BifrostError::ConfigSerialize(err.to_string()));
            }
            Ok(format!(
                "Active Midgard runtime set to '{}' (Android {}; persistent userdata preserved).",
                activated.id, activated.android_version
            ))
        }
    }
}
