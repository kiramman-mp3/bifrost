//! Midgard image trust policy: Ed25519 signatures checked against a local trust store.
//!
//! A manifest is only treated as signed when its Ed25519 signature verifies against a
//! publisher key listed in `bifrost.toml` (`[images] trusted_keys`). A user-supplied key
//! alone proves nothing, so the trust store is the only source of trusted keys.
//! Bundles without a signature are rejected unless the caller explicitly opts in with
//! `allow_unsigned`, in which case they are recorded as unsigned (`sha256-only`).

use std::fs;
use std::path::{Path, PathBuf};

use bifrost_core::{validate_host_file_path, BifrostError, Result};
use serde::{Deserialize, Serialize};

use crate::signature::verify_bytes_ed25519;

/// File extension appended to `manifest.json` for a detached hex Ed25519 signature.
pub const DETACHED_SIGNATURE_SUFFIX: &str = ".sig";

/// Trust decision reached for a Midgard manifest.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ImageTrust {
    /// Signature verified against a key from the local trust store.
    TrustedSignature { public_key: String },
    /// No signature; accepted only because the caller explicitly allowed unsigned bundles.
    UnsignedAllowed,
}

impl ImageTrust {
    /// Returns `true` when the manifest is signed by a trusted publisher key.
    #[must_use]
    pub const fn is_signed(&self) -> bool {
        matches!(self, Self::TrustedSignature { .. })
    }
}

/// Returns the detached signature path (`manifest.json.sig`) for a manifest.
#[must_use]
pub fn detached_signature_path(manifest_path: &Path) -> PathBuf {
    let mut name = manifest_path.as_os_str().to_os_string();
    name.push(DETACHED_SIGNATURE_SUFFIX);
    PathBuf::from(name)
}

/// Evaluates the trust of a manifest file on disk.
///
/// The signature is taken from `signature_hex` when provided, otherwise from a detached
/// `manifest.json.sig` file next to the manifest when one exists.
pub fn evaluate_manifest_trust(
    manifest_path: impl AsRef<Path>,
    signature_hex: Option<&str>,
    trusted_keys: &[String],
    allow_unsigned: bool,
) -> Result<ImageTrust> {
    let safe_path = validate_host_file_path(manifest_path, Some("json"))?;
    let payload = fs::read(&safe_path).map_err(|source| BifrostError::Io {
        context: format!("reading manifest '{}' for trust check", safe_path.display()),
        source,
    })?;

    let signature = match signature_hex {
        Some(sig) => Some(sig.trim().to_string()),
        None => read_detached_signature(&safe_path)?,
    };

    evaluate_payload_trust(&payload, signature.as_deref(), trusted_keys, allow_unsigned)
}

/// Evaluates the trust of raw manifest bytes (pure policy, used by `evaluate_manifest_trust`).
pub fn evaluate_payload_trust(
    payload: &[u8],
    signature_hex: Option<&str>,
    trusted_keys: &[String],
    allow_unsigned: bool,
) -> Result<ImageTrust> {
    let Some(signature) = signature_hex.map(str::trim).filter(|s| !s.is_empty()) else {
        if allow_unsigned {
            return Ok(ImageTrust::UnsignedAllowed);
        }
        return Err(BifrostError::Runtime(
            "Midgard manifest is unsigned: provide --signature (or a detached manifest.json.sig) \
             signed by a key in bifrost.toml [images] trusted_keys, or pass --allow-unsigned to \
             register it explicitly as sha256-only"
                .to_string(),
        ));
    };

    if trusted_keys.is_empty() {
        return Err(BifrostError::Runtime(
            "Midgard manifest is signed but no trusted publisher keys are configured; add the \
             publisher's Ed25519 public key to bifrost.toml [images] trusted_keys"
                .to_string(),
        ));
    }

    for key in trusted_keys {
        let key = key.trim().to_ascii_lowercase();
        if verify_bytes_ed25519(payload, signature, &key).is_ok() {
            return Ok(ImageTrust::TrustedSignature { public_key: key });
        }
    }

    // A present-but-invalid signature is never downgraded to "unsigned", even when
    // `allow_unsigned` is set: it indicates tampering or an untrusted publisher.
    Err(BifrostError::Runtime(
        "Midgard manifest signature does not verify against any key in bifrost.toml \
         [images] trusted_keys: the manifest may be tampered or from an untrusted publisher"
            .to_string(),
    ))
}

fn read_detached_signature(manifest_path: &Path) -> Result<Option<String>> {
    let sig_path = detached_signature_path(manifest_path);
    if !sig_path.is_file() {
        return Ok(None);
    }
    let text = fs::read_to_string(&sig_path).map_err(|source| BifrostError::Io {
        context: format!("reading detached signature '{}'", sig_path.display()),
        source,
    })?;
    Ok(Some(text.trim().to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::signature::sign_bytes_ed25519;

    const TRUSTED_SEED: &str = "1111111111111111111111111111111111111111111111111111111111111111";
    const OTHER_SEED: &str = "2222222222222222222222222222222222222222222222222222222222222222";
    const PAYLOAD: &[u8] = br#"{"schema_version":1,"id":"aosp-16-x86_64"}"#;

    #[test]
    fn accepts_only_signatures_from_trusted_keys() {
        let (sig, trusted_pub) = sign_bytes_ed25519(PAYLOAD, TRUSTED_SEED).unwrap();
        let (other_sig, _) = sign_bytes_ed25519(PAYLOAD, OTHER_SEED).unwrap();
        let store = vec![trusted_pub.clone()];

        let trust = evaluate_payload_trust(PAYLOAD, Some(&sig), &store, false).unwrap();
        assert_eq!(
            trust,
            ImageTrust::TrustedSignature {
                public_key: trusted_pub
            }
        );

        // Valid signature from a key outside the trust store is rejected, even with opt-in.
        assert!(evaluate_payload_trust(PAYLOAD, Some(&other_sig), &store, true).is_err());
        // Signed manifest with an empty trust store is rejected.
        assert!(evaluate_payload_trust(PAYLOAD, Some(&sig), &[], true).is_err());
        // Tampered payload is rejected.
        assert!(evaluate_payload_trust(b"{}", Some(&sig), &store, true).is_err());
    }

    #[test]
    fn unsigned_manifests_require_explicit_opt_in() {
        assert!(evaluate_payload_trust(PAYLOAD, None, &[], false).is_err());
        assert_eq!(
            evaluate_payload_trust(PAYLOAD, None, &[], true).unwrap(),
            ImageTrust::UnsignedAllowed
        );
    }

    #[test]
    fn reads_detached_signature_next_to_manifest() {
        let dir = tempfile::tempdir().unwrap();
        let manifest = dir.path().join("manifest.json");
        fs::write(&manifest, PAYLOAD).unwrap();
        let (sig, pubkey) = sign_bytes_ed25519(PAYLOAD, TRUSTED_SEED).unwrap();
        fs::write(detached_signature_path(&manifest), format!("{sig}\n")).unwrap();

        let trust = evaluate_manifest_trust(&manifest, None, &[pubkey], false).unwrap();
        assert!(trust.is_signed());
    }
}
