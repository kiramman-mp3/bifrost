# BIFRÖST Distribution, Upgrades & Recovery

## Official Releases (CI)

Official releases are built only by [`.github/workflows/release.yml`](../.github/workflows/release.yml), never on a developer machine:

1. Update `[workspace.package] version` in `Cargo.toml` and add a `## [X.Y.Z]` section to `CHANGELOG.md`.
2. Merge `release/vX.Y.Z` into `main`, then tag it: `git tag vX.Y.Z && git push origin vX.Y.Z`.
3. The workflow checks that the tag matches the workspace version and runs fmt, clippy and the tests. It then builds the bundle with the MSVC toolchain, Authenticode-signs it when signing secrets exist, records a Sigstore build-provenance attestation, and publishes the GitHub Release with notes taken from the CHANGELOG.

Users can verify a download with:

```powershell
gh attestation verify bifrost-vX.Y.Z-win64.zip --repo kiramman-mp3/bifrost
Get-AuthenticodeSignature .\bifrost.exe   # only meaningful for signed releases
```

### Authenticode signing

Signing is optional and controlled by two repository secrets:

| Secret | Content |
|---|---|
| `WINDOWS_SIGNING_PFX_BASE64` | Base64 of a code-signing certificate (`.pfx`) |
| `WINDOWS_SIGNING_PFX_PASSWORD` | Password of that `.pfx` |

Without them the release is published **unsigned** and its notes say so (Windows SmartScreen will warn on first run). Only BIFRÖST's own `.exe` and `.ps1` files are signed; `WebView2Loader.dll` keeps Microsoft's signature. Signing happens before hashing, so `release-checksums.sha256` matches the signed files.

## Local Release Bundle

Build all release binaries (`bifrost.exe`, `heimdall.exe`, `bifrost-ui.exe`), copy the installer scripts and `LICENSE`, and generate a SHA-256 verification manifest:

```powershell
.\installer\windows\build-release-bundle.ps1 -OutputDir .\dist\bifrost-win64 -Zip
# Optional local signing (password only through the environment):
$env:BIFROST_SIGN_PFX_PASSWORD = "<password>"
.\installer\windows\build-release-bundle.ps1 -OutputDir .\dist\bifrost-win64 -SignCertificatePath C:\keys\codesign.pfx -Zip
```

`-OutputDir` is only wiped when it is empty or already contains a previous bundle (`release-checksums.sha256`).

## Unprivileged User-Space Installation

Install verified binaries into `%LOCALAPPDATA%\Programs\BIFRÖST`, update user `PATH`, and register the Start Menu shortcut without requiring Administrator privileges:

```powershell
.\installer\windows\install-bifrost.ps1 -BundleDir .\dist\bifrost-win64
```

## Clean Uninstallation

Remove binaries, `PATH` entry, and shortcut while preserving `%LOCALAPPDATA%\bifrost` (Android `userdata` and `bifrost.toml`):

```powershell
.\installer\windows\uninstall-bifrost.ps1
```

To also erase Android `userdata` and configuration, both explicit confirmation switches are required:

```powershell
.\installer\windows\uninstall-bifrost.ps1 -PurgeUserData -ConfirmPurge
```

## Verified Runtime Updates & Recovery CLI

```powershell
# Register a Midgard bundle signed by a publisher key listed in bifrost.toml:
#   [images]
#   trusted_keys = ["<64-hex-char Ed25519 public key>"]
# The signature is read from --signature or from a detached manifest.json.sig.
bifrost images register --manifest path\to\manifest.json --signature <sig_hex>
# Unsigned bundles (SHA-256 only) require an explicit opt-in:
bifrost images register --manifest path\to\manifest.json --allow-unsigned
bifrost images list
bifrost images use <image_id>

# Verified userdata backup, restore, and sanitized diagnostic export
bifrost recovery backup --output C:\Backups\bifrost-userdata
bifrost recovery restore --from C:\Backups\bifrost-userdata --confirm
bifrost recovery export-diagnostics --output C:\Backups\bifrost-diagnostics.json
```
