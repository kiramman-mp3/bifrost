# Midgard Android x86_64 Runtime Bundles

This directory documents the image bundle format consumed by `bifrost-storage`, `bifrost-hypervisor`, and `bifrost-runtime` to boot a real Android `x86_64` environment over Windows Hypervisor Platform (WHPX).

## Bundle Manifest Format (`manifest.json`)

Every Midgard bundle must include a `manifest.json` (`schema_version = 1`) specifying `arch: "x86_64"` and exact lowercase SHA-256 digests for every boot file. `bifrost-storage` verifies all digests in streaming mode and enforces strict path containment before spawning the VM.

See [manifest.example.json](manifest.example.json) for a canonical AOSP `x86_64` direct-kernel VirtIO template.

### Boot Modes

1. **`direct_kernel` (Canonical AOSP `x86_64` / Cuttlefish-aligned)**:
   - `kernel`: Linux GKI / AOSP `bzImage`.
   - `initrd`: Android `ramdisk.img` / `initrd.img`.
   - `cmdline`: Kernel command line (including `console=ttyS0 androidboot.console=ttyS0`).
   - `disks`: Ordered VirtIO block images (e.g., `super.img` read-only).
   - BIFRÖST automatically attaches a persistent, user-scoped `userdata.img` VirtIO disk.
2. **`bootable_image` (Prototype ISO / Disk)**:
   - `image`: Bootable open-source Android `x86_64` ISO (`media: "cdrom"`) or raw/qcow2 disk (`media: "disk"`).
   - Useful for rapid WHPX bring-up tests before dedicated Linux AOSP CI bundles are published.

## Security Rules

- Never commit `.img`, `.iso`, `.qcow2`, or proprietary Google Play Services / ARM translator binaries into Git.
- Always compute real SHA-256 digests (`Get-FileHash -Algorithm SHA256 <file>` on PowerShell) and verify with:
  ```powershell
  bifrost runtime verify --image path\to\manifest.json
  ```
- ADB port forwarding binds strictly to `127.0.0.1` by default.
