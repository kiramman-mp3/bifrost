# BIFRÖST Distribution, Upgrades & Recovery

## Reproducible Windows 11 Release Bundle

Build all release binaries (`bifrost.exe`, `heimdall.exe`, `bifrost-ui.exe`) and generate a SHA-256 verification manifest:

```powershell
.\installer\windows\build-release-bundle.ps1 -OutputDir .\dist\bifrost-win64
```

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
# Register a SHA-256 + Ed25519-verified Midgard runtime bundle
bifrost images register --manifest path\to\manifest.json --signature <sig_hex> --pubkey <pub_hex>
bifrost images list
bifrost images use <image_id>

# Verified userdata backup, restore, and sanitized diagnostic export
bifrost recovery backup --output C:\Backups\bifrost-userdata
bifrost recovery restore --from C:\Backups\bifrost-userdata --confirm
bifrost recovery export-diagnostics --output C:\Backups\bifrost-diagnostics.json
```
