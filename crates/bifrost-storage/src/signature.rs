//! Cryptographic Ed25519 signature verification for Midgard manifests and updates.

use std::fs;
use std::path::Path;

use bifrost_core::{validate_host_file_path, BifrostError, Result};
use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};

/// Verifies an Ed25519 signature (64-byte hex) over `payload` against a 32-byte hex public key.
pub fn verify_bytes_ed25519(
    payload: &[u8],
    signature_hex: &str,
    public_key_hex: &str,
) -> Result<()> {
    let pub_bytes = decode_fixed_hex::<32>(public_key_hex.trim(), "Ed25519 public key")?;
    let sig_bytes = decode_fixed_hex::<64>(signature_hex.trim(), "Ed25519 signature")?;

    let verifying_key = VerifyingKey::from_bytes(&pub_bytes)
        .map_err(|err| BifrostError::Runtime(format!("invalid Ed25519 public key: {err}")))?;
    let signature = Signature::from_bytes(&sig_bytes);

    verifying_key.verify(payload, &signature).map_err(|_| {
        BifrostError::Runtime(
            "Ed25519 signature verification failed: manifest may be tampered or untrusted"
                .to_string(),
        )
    })
}

/// Signs `payload` using a 32-byte hex Ed25519 secret seed (used by release tooling and tests).
pub fn sign_bytes_ed25519(payload: &[u8], secret_key_hex: &str) -> Result<(String, String)> {
    let seed = decode_fixed_hex::<32>(secret_key_hex.trim(), "Ed25519 secret seed")?;
    let signing_key = SigningKey::from_bytes(&seed);
    let signature = signing_key.sign(payload);
    let pub_hex = hex::encode(signing_key.verifying_key().to_bytes());
    let sig_hex = hex::encode(signature.to_bytes());
    Ok((sig_hex, pub_hex))
}

/// Verifies a manifest file on disk against an Ed25519 hex signature and public key.
pub fn verify_manifest_signature_file(
    manifest_path: impl AsRef<Path>,
    signature_hex: &str,
    public_key_hex: &str,
) -> Result<()> {
    let safe_path = validate_host_file_path(manifest_path, Some("json"))?;
    let bytes = fs::read(&safe_path).map_err(|source| BifrostError::Io {
        context: format!(
            "reading manifest '{}' for signature check",
            safe_path.display()
        ),
        source,
    })?;
    verify_bytes_ed25519(&bytes, signature_hex, public_key_hex)
}

fn decode_fixed_hex<const N: usize>(input: &str, label: &str) -> Result<[u8; N]> {
    let decoded = hex::decode(input).map_err(|err| BifrostError::ConfigValidation {
        field: label.to_string(),
        reason: format!("invalid hexadecimal encoding: {err}"),
    })?;
    decoded
        .try_into()
        .map_err(|v: Vec<u8>| BifrostError::ConfigValidation {
            field: label.to_string(),
            reason: format!(
                "expected {N} bytes ({} hex chars), got {} bytes",
                N * 2,
                v.len()
            ),
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEST_SEED_HEX: &str = "1111111111111111111111111111111111111111111111111111111111111111";

    #[test]
    fn verifies_valid_ed25519_signature_and_rejects_tampering() {
        let payload = br#"{"schema_version":1,"id":"aosp-16-x86_64"}"#;
        let (sig, pubkey) = sign_bytes_ed25519(payload, TEST_SEED_HEX).unwrap();

        assert!(verify_bytes_ed25519(payload, &sig, &pubkey).is_ok());

        // Tampered payload must fail.
        let tampered = br#"{"schema_version":1,"id":"aosp-99-x86_64"}"#;
        assert!(verify_bytes_ed25519(tampered, &sig, &pubkey).is_err());

        // Wrong public key must fail.
        let (_, other_pub) = sign_bytes_ed25519(
            payload,
            "2222222222222222222222222222222222222222222222222222222222222222",
        )
        .unwrap();
        assert!(verify_bytes_ed25519(payload, &sig, &other_pub).is_err());
    }
}
