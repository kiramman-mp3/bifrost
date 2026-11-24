# ADR-0006: Local Ed25519 Trust Store for Midgard Runtime Images

- **Status**: Accepted
- **Date**: 2026-11-23
- **Milestone**: Post-M8 (v0.4.0 development)

## Context

Up to v0.3.3, `bifrost images register` accepted an optional Ed25519 signature **together with a public key supplied on the same command line** (`--signature <sig> --pubkey <key>`). Anyone able to tamper with a manifest could also generate a matching key pair, so this check proved integrity only against the key the caller chose. It did not prove that a trusted publisher produced the manifest. Unsigned bundles were registered silently, and the desktop UI always registered images as unsigned. `SECURITY.md` described signature verification as mandatory, which did not match the code.

BIFRÖST does not yet publish its own signed Midgard images, and the verified development image (LineageOS 24.0 `virtio_x86_64`) is unsigned. A key compiled into the binary would therefore have nothing to sign today.

## Decision

1. Add a **local trust store** to `bifrost.toml`:
   ```toml
   [images]
   trusted_keys = ["<64-hex-char Ed25519 public key>"]
   ```
   Keys are validated as 32-byte hex public keys on config load.
2. `bifrost-storage::trust::evaluate_manifest_trust` decides trust:
   - The signature comes from `--signature` or a detached `manifest.json.sig` next to the manifest.
   - A signature counts **only** if it verifies against a key in `trusted_keys`. The `--pubkey` flag is removed.
   - A signature that is present but invalid or untrusted is **always rejected**. It is never downgraded to "unsigned".
   - A bundle with no signature is rejected unless the caller opts in explicitly: `--allow-unsigned` in the CLI, or the "Allow unsigned bundle" checkbox in the UI. It is then recorded as `sha256-only` and a warning is logged.
3. SHA-256 verification of every artifact stays mandatory for every bundle, signed or not.

## Alternatives Considered

- **Compile a project release key into the binary**: Deferred until BIFRÖST publishes official signed images. The trust store can ship that key as the default entry later.
- **Strict signatures with no opt-in**: Rejected for now because no signed images exist yet, so this would block the only verified image.
- **Documentation-only change**: Rejected because it would leave a security control that does not protect anything.

## Consequences

- **Positive**: The signed state now means something. An attacker can no longer pair a tampered manifest with a self-generated key, and unsigned use is a visible, deliberate choice.
- **Negative / Trade-off**: This is a breaking CLI change, since `--pubkey` is removed. Users must add publisher keys to `bifrost.toml` themselves. Images already in the registry keep their previous `signature_verified` value.
- **Follow-up**: Publish official signed Midgard bundles, then ship their key as a default trusted key and consider making signatures mandatory.
