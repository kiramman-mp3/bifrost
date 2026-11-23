# ADR-0001: AOSP x86_64 (VirtIO / Cuttlefish-Aligned) as Midgard Foundation

- **Status**: Accepted
- **Date**: 2026-11-21
- **Milestone**: Milestone 0

## Context

Midgard must boot a real, maintainable Android 15/16 environment on Windows 11 x64 with VirtIO hardware interfaces, clean open-source licensing, and independent OS upgradability. We evaluated AOSP Cuttlefish, AOSP Goldfish, Android-x86, Bliss OS, and Waydroid.

## Decision

Adopt **AOSP x86_64 (Android 15/16)** aligned with **VirtIO / Cuttlefish** virtual hardware conventions (GKI Linux kernel, `virtio-blk`, `virtio-net`, `virtio-gpu`, `virtio-vsock`, `virtio-input`, `virtio-snd`) as the canonical Midgard Android foundation.

- Build Midgard images reproducibly in Linux x86_64 CI environments (due to AOSP's 64 GB RAM / 300–400 GB SSD build requirements).
- Distribute versioned, cryptographically signed image bundles consumed by the Windows host runtime.
- Keep Android `userdata` on a separate persistent virtual disk to survive compatible Midgard OS upgrades.

## Alternatives Considered

- **Android-x86**: Largely stalled on older Android releases; unsuitable for Android 15/16.
- **Bliss OS**: Active codebase, but public ISO pipeline is paused during rewrite and primarily targets bare-metal x86 hardware rather than a clean VirtIO VM subsystem.
- **Waydroid**: Linux container runtime requiring native host Binder/Ashmem; cannot run directly on Windows without an intermediate Linux VM.
- **AOSP Goldfish**: Tied to Android Studio emulator quirks rather than clean VirtIO subsystem interfaces (still usable only as a temporary smoke-test image if needed during early bring-up).

## Consequences

- **Positive**: Clean Apache-2.0 userspace, modern Android 15/16 and 16 KB page-size readiness, standard VirtIO hardware compatibility with both QEMU and `crosvm`.
- **Negative / Trade-off**: Full AOSP compilation cannot happen natively on a 32 GB Windows laptop; requires Linux build infrastructure or verified prebuilt AOSP CI artifacts.
- **Legal obligation**: Distributing compiled Linux GKI kernels requires publishing exact matching GPLv2 kernel source trees and configs.

## Implementation Note (2026-11-23)

The Linux CI image pipeline described above has **not** been built yet. Midgard currently boots a prebuilt, unsigned, open-source **LineageOS 24.0 `virtio_x86_64`** bundle (Android 17, no GApps), staged locally and verified by SHA-256 with `bifrost runtime verify`. This keeps the VirtIO / AOSP-aligned and clean-room goals of this ADR, but the project does not yet produce its own reproducible, signed images. Image trust is handled by [ADR-0006](ADR-0006-image-trust-store.md).
