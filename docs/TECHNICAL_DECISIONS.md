# Technical Decisions & ADR Index

This document indexes all Architecture Decision Records (ADRs) for **BIFRÖST**. Each ADR records the technical context, alternatives evaluated, final decision, and consequences.

| ID | Title | Status | Milestone | File |
|---|---|---|---|---|
| **ADR-0001** | AOSP x86_64 (VirtIO / Cuttlefish-Aligned) as Midgard Foundation | Accepted | M0 | [ADR-0001-aosp-x86-64-foundation.md](decisions/ADR-0001-aosp-x86-64-foundation.md) |
| **ADR-0002** | WHPX-First Virtualization Behind a Rust VMM Abstraction | Accepted | M0 | [ADR-0002-whpx-vmm-abstraction.md](decisions/ADR-0002-whpx-vmm-abstraction.md) |
| **ADR-0003** | Graphics Acceleration (`virtio-gpu` + `gfxstream`) and Phased Windowing | Accepted | M0 | [ADR-0003-graphics-and-windowing-strategy.md](decisions/ADR-0003-graphics-and-windowing-strategy.md) |
| **ADR-0004** | Clean-Room x86_64 Policy and Exclusion of Proprietary ARM Translators & GApps | Accepted | M0 | [ADR-0004-arm64-compatibility-policy.md](decisions/ADR-0004-arm64-compatibility-policy.md) |
| **ADR-0005** | Per-App Windows via `scrcpy` VirtualDisplay Streaming (amends ADR-0003 Stage 2) | Accepted | M6 / v0.2.0 | [ADR-0005-scrcpy-virtualdisplay-windowing.md](decisions/ADR-0005-scrcpy-virtualdisplay-windowing.md) |
| **ADR-0006** | Local Ed25519 Trust Store for Midgard Runtime Images | Accepted | Post-M8 | [ADR-0006-image-trust-store.md](decisions/ADR-0006-image-trust-store.md) |
| **ADR-0007** | Opt-in Host GPU Rendering Through virgl on QEMU-WHPX | Accepted | v0.5.0 | [ADR-0007-guest-gpu-rendering-on-windows.md](decisions/ADR-0007-guest-gpu-rendering-on-windows.md) |

## ADR Status Definitions
- **Proposed**: Under active technical investigation.
- **Accepted**: Approved baseline governing current implementation.
- **Superseded**: Replaced by a newer ADR (retained for historical auditability).
- **Deprecated**: No longer applicable.
