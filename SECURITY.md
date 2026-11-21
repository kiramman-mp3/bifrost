# Security Policy

Security is a foundational requirement of **BIFRÖST**, not an optional feature. Because BIFRÖST orchestrates virtualization, local IPC, Android runtime images, and APK installation on Windows 11, every component must adhere to strict security boundaries.

## Mandatory Security Invariants

1. **No Untrusted APK Scripts**: BIFRÖST never executes external shell or installation scripts bundled with APKs; package installation is delegated exclusively to Android's native `PackageManager`.
2. **Path Validation & Traversal Prevention**: All host and shared filesystem paths are canonicalized and verified against allowed roots to prevent directory traversal (`..`) and symlink/reparse-point escapes.
3. **Loopback-Only ADB by Default**: ADB ports bind strictly to `127.0.0.1` (or `virtio-vsock` tunnels) and are never exposed to `0.0.0.0` or public network interfaces by default.
4. **Least Privilege**: `mimir`, `heimdall`, `bifrost-ui`, and VM processes run as standard unprivileged user processes. Administrator privileges are never requested during normal runtime execution.
5. **Authenticated Local IPC**: All local IPC endpoints (`heimdall` Windows Named Pipes) enforce current-user SID DACLs and validate every incoming request schema.
6. **Cryptographic Verification of Runtime Images**: Downloaded Midgard Android images and updates must pass mandatory SHA-256 digest and cryptographic signature verification before mounting or booting.
7. **Data Persistence & Destructive Safeguards**: Android `userdata` lives on an isolated persistent disk across compatible upgrades; destructive actions (wipe, reset, disk deletion) require explicit user confirmation.
8. **Memory Safety & `unsafe` Auditing**: Avoid `unsafe` Rust wherever possible. Every `unsafe` block must include a `// SAFETY:` comment explaining the upheld invariants and must be reviewed explicitly.
9. **Clean-Room Distribution**: BIFRÖST never bundles or redistributes proprietary Google Apps / Play Services or unauthorized proprietary ARM translation binaries (`libhoudini`, `libndk_translation`).

## Reporting a Vulnerability

If you discover a security vulnerability in BIFRÖST:

1. **Do not** open a public GitHub issue with exploit details.
2. Report the issue privately to the project maintainer with:
   - Affected component/crate and commit hash.
   - Clear reproduction steps or proof of concept.
   - Potential security impact (e.g., privilege escalation, IPC bypass, path traversal, ADB exposure).
3. Expect an acknowledgment and coordinated disclosure timeline before public release of the fix.

## Security Audit Trail

All security-relevant architectural decisions are documented in [docs/TECHNICAL_DECISIONS.md](docs/TECHNICAL_DECISIONS.md) and tracked in [CHANGELOG.md](CHANGELOG.md).
