# Security Policy

Security is a foundational requirement of **BIFRÖST**, not an optional feature. Because BIFRÖST orchestrates virtualization, local IPC, Android runtime images, and APK installation on Windows 11, every component must adhere to strict security boundaries.

## Mandatory Security Invariants

1. **No Untrusted APK Scripts**: BIFRÖST never executes external shell or installation scripts bundled with APKs; package installation is delegated exclusively to Android's native `PackageManager`.
2. **Path Validation & Traversal Prevention**: All host and shared filesystem paths are canonicalized and verified against allowed roots to prevent directory traversal (`..`) and symlink/reparse-point escapes.
3. **Loopback-Only ADB by Default**: ADB ports bind strictly to `127.0.0.1` (or `virtio-vsock` tunnels) and are never exposed to `0.0.0.0` or public network interfaces by default.
4. **Least Privilege**: `mimir`, `heimdall`, `bifrost-ui`, and VM processes run as standard unprivileged user processes. Administrator privileges are never requested during normal runtime execution.
5. **Authenticated Local IPC**: All local IPC endpoints (`heimdall` Windows Named Pipes) enforce current-user SID DACLs and validate every incoming request schema.
6. **Cryptographic Verification of Runtime Images**: Every Midgard artifact must pass SHA-256 digest verification before boot. Ed25519 manifest signatures are trusted **only** when they verify against a publisher key listed in `bifrost.toml` under `[images] trusted_keys`; a key supplied alongside the signature is never trusted on its own. Invalid or untrusted signatures are always rejected. Unsigned bundles are refused unless the user opts in explicitly per registration (`--allow-unsigned` or the UI checkbox), and they are then recorded as `sha256-only` (see [ADR-0006](docs/decisions/ADR-0006-image-trust-store.md)).
7. **Data Persistence & Destructive Safeguards**: Android `userdata` lives on an isolated persistent disk across compatible upgrades; destructive actions (wipe, reset, disk deletion) require explicit user confirmation.
8. **Memory Safety & `unsafe` Auditing**: Avoid `unsafe` Rust wherever possible. Every `unsafe` block must include a `// SAFETY:` comment explaining the upheld invariants and must be reviewed explicitly.
9. **No Data on Guest Shell Command Lines**: `adb shell` evaluates its arguments with the Android `sh` interpreter. Arbitrary host data (clipboard text) is streamed through stdin into a fixed script; any other host-controlled value must be quoted with `quote_android_shell_arg` or validated against a strict character set.
10. **Unauthenticated VMM Monitors Stay Off the Network**: QEMU's QMP monitor has no authentication, so it is exposed only on a randomly named AF_UNIX socket file inside the user's profile (protected by its ACL), never on a TCP port.
11. **Clean-Room Distribution**: BIFRÖST never bundles or redistributes proprietary Google Apps / Play Services or unauthorized proprietary ARM translation binaries (`libhoudini`, `libndk_translation`).

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
