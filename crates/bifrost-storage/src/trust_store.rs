//! Management of the `[images] trusted_keys` trust store: strict key parsing,
//! human-comparable fingerprints, and the impact of removing a publisher key.

use bifrost_core::{BifrostError, Result};
use ed25519_dalek::VerifyingKey;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::registry::ImageRegistry;
use crate::trust::evaluate_manifest_trust;

/// A trusted publisher key with a short fingerprint for visual comparison.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrustedKeyInfo {
    /// Normalized key: 64 lowercase hex characters.
    pub public_key: String,
    /// First 8 bytes of SHA-256(key bytes), colon-separated (`ab12:cd34:…`).
    pub fingerprint: String,
}

/// Parses a user-supplied Ed25519 public key and returns it normalized (lowercase hex).
///
/// Unlike the config schema check (64 hex characters), this also rejects byte strings
/// that are not a valid point on the Ed25519 curve, so a typo cannot be stored as a key
/// that would never verify anything.
pub fn parse_trusted_key(input: &str) -> Result<String> {
    let normalized = input.trim().to_ascii_lowercase();
    let invalid = |reason: &str| BifrostError::ConfigValidation {
        field: "images.trusted_keys".to_string(),
        reason: format!("'{}' {reason}", input.trim()),
    };
    if normalized.len() != 64 {
        return Err(invalid(
            "must be a 32-byte Ed25519 public key in hex (64 characters)",
        ));
    }
    let bytes: [u8; 32] = hex::decode(&normalized)
        .map_err(|_| invalid("contains non-hex characters"))?
        .try_into()
        .map_err(|_| invalid("must decode to exactly 32 bytes"))?;
    VerifyingKey::from_bytes(&bytes).map_err(|_| invalid("is not a valid Ed25519 public key"))?;
    Ok(normalized)
}

/// Returns `public_key` with its fingerprint; the key must already be valid hex.
pub fn describe_trusted_key(public_key: &str) -> Result<TrustedKeyInfo> {
    let normalized = parse_trusted_key(public_key)?;
    let bytes = hex::decode(&normalized).map_err(|err| BifrostError::Runtime(err.to_string()))?;
    let digest = Sha256::digest(bytes);
    let fingerprint = digest[..8]
        .chunks(2)
        .map(hex::encode)
        .collect::<Vec<_>>()
        .join(":");
    Ok(TrustedKeyInfo {
        public_key: normalized,
        fingerprint,
    })
}

/// Returns a new trust store with `key` added (normalized, without duplicates).
pub fn with_trusted_key(current: &[String], key: &str) -> Result<Vec<String>> {
    let key = parse_trusted_key(key)?;
    let mut keys: Vec<String> = current
        .iter()
        .map(|k| k.trim().to_ascii_lowercase())
        .collect();
    if keys.contains(&key) {
        return Err(BifrostError::ConfigValidation {
            field: "images.trusted_keys".to_string(),
            reason: "this publisher key is already trusted".to_string(),
        });
    }
    keys.push(key);
    Ok(keys)
}

/// Returns a new trust store without `key`; errors when it was not trusted.
pub fn without_trusted_key(current: &[String], key: &str) -> Result<Vec<String>> {
    let key = key.trim().to_ascii_lowercase();
    let keys: Vec<String> = current
        .iter()
        .map(|k| k.trim().to_ascii_lowercase())
        .filter(|k| *k != key)
        .collect();
    if keys.len() == current.len() {
        return Err(BifrostError::ConfigValidation {
            field: "images.trusted_keys".to_string(),
            reason: format!("publisher key '{key}' is not trusted"),
        });
    }
    Ok(keys)
}

/// Registered images whose detached `manifest.json.sig` verifies today but would no
/// longer verify with `remaining_keys`: they could not be re-registered as signed.
///
/// Existing registry entries keep their recorded `signature_verified` flag; trust is
/// evaluated when an image is registered.
#[must_use]
pub fn images_losing_trust(
    registry: &ImageRegistry,
    current_keys: &[String],
    remaining_keys: &[String],
) -> Vec<String> {
    registry
        .images
        .iter()
        .filter(|img| img.signature_verified)
        .filter(|img| {
            let verifies = |keys: &[String]| {
                evaluate_manifest_trust(&img.manifest_path, None, keys, false)
                    .is_ok_and(|trust| trust.is_signed())
            };
            verifies(current_keys) && !verifies(remaining_keys)
        })
        .map(|img| img.id.clone())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::SigningKey;

    fn valid_key(seed: u8) -> String {
        hex::encode(
            SigningKey::from_bytes(&[seed; 32])
                .verifying_key()
                .to_bytes(),
        )
    }

    #[test]
    fn parses_and_normalizes_real_ed25519_keys() {
        let key = valid_key(7);
        assert_eq!(
            parse_trusted_key(&format!("  {}  ", key.to_uppercase())).unwrap(),
            key
        );
    }

    #[test]
    fn rejects_wrong_length_non_hex_and_off_curve_keys() {
        assert!(parse_trusted_key("ab").is_err());
        assert!(parse_trusted_key(&"zz".repeat(32)).is_err());
        // 64 hex characters but not a valid compressed Edwards point.
        let off_curve = format!("02{}", "00".repeat(31));
        assert!(
            VerifyingKey::from_bytes(&hex::decode(&off_curve).unwrap().try_into().unwrap())
                .is_err()
        );
        assert!(parse_trusted_key(&off_curve).is_err());
    }

    #[test]
    fn fingerprint_is_stable_and_short() {
        let info = describe_trusted_key(&valid_key(1)).unwrap();
        assert_eq!(info.fingerprint.len(), 19);
        assert_eq!(info.fingerprint.matches(':').count(), 3);
        assert_eq!(
            info,
            describe_trusted_key(&valid_key(1).to_uppercase()).unwrap()
        );
    }

    #[test]
    fn reports_images_that_only_verify_with_the_removed_key() {
        use crate::registry::InstalledImageRecord;
        use crate::signature::sign_bytes_ed25519;
        use crate::trust::detached_signature_path;

        let dir = tempfile::tempdir().unwrap();
        let record = |id: &str, seed: u8| {
            let manifest = dir.path().join(format!("{id}.json"));
            std::fs::write(&manifest, format!("{{\"id\":\"{id}\"}}")).unwrap();
            let payload = std::fs::read(&manifest).unwrap();
            let (sig, _) = sign_bytes_ed25519(&payload, &hex::encode([seed; 32])).unwrap();
            std::fs::write(detached_signature_path(&manifest), sig).unwrap();
            InstalledImageRecord {
                id: id.to_string(),
                name: id.to_string(),
                android_version: "17".to_string(),
                arch: "x86_64".to_string(),
                manifest_path: manifest,
                manifest_sha256: String::new(),
                signature_verified: true,
                userdata_compat_version: 1,
                registered_at_unix: 0,
            }
        };
        let registry = ImageRegistry {
            active_image_id: None,
            images: vec![record("by-a", 1), record("by-b", 2)],
        };
        let (a, b) = (valid_key(1), valid_key(2));
        let current = vec![a.clone(), b.clone()];
        let remaining = without_trusted_key(&current, &a).unwrap();
        assert_eq!(
            images_losing_trust(&registry, &current, &remaining),
            vec!["by-a"]
        );
    }

    #[test]
    fn adds_without_duplicates_and_removes_existing_keys() {
        let a = valid_key(1);
        let b = valid_key(2);
        let keys = with_trusted_key(std::slice::from_ref(&a), &b).unwrap();
        assert_eq!(keys, vec![a.clone(), b.clone()]);
        assert!(with_trusted_key(&keys, &a.to_uppercase()).is_err());
        assert_eq!(without_trusted_key(&keys, &b).unwrap(), vec![a.clone()]);
        assert!(without_trusted_key(&[a], &b).is_err());
    }
}
