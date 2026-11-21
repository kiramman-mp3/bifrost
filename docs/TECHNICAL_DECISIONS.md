# Technical Decisions & ADR Index

This document indexes all Architecture Decision Records (ADRs) for **BIFRÖST**. Each ADR records the technical context, alternatives evaluated, final decision, and consequences.

| ID | Title | Status | Milestone | File |
|---|---|---|---|---|
| **ADR-0001** | AOSP x86_64 (VirtIO / Cuttlefish-Aligned) as Midgard Foundation | Accepted | M0 | [ADR-0001-aosp-x86-64-foundation.md](decisions/ADR-0001-aosp-x86-64-foundation.md) |
| **ADR-0002** | WHPX-First Virtualization Behind a Rust VMM Abstraction | Accepted | M0 | [ADR-0002-whpx-vmm-abstraction.md](decisions/ADR-0002-whpx-vmm-abstraction.md) |
| **ADR-0003** | Graphics Acceleration (`virtio-gpu` + `gfxstream`) and Phased Windowing | Accepted | M0 | [ADR-0003-graphics-and-windowing-strategy.md](decisions/ADR-0003-graphics-and-windowing-strategy.md) |
| **ADR-0004** | Clean-Room x86_64 Policy and Exclusion of Proprietary ARM Translators & GApps | Accepted | M0 | [ADR-0004-arm64-compatibility-policy.md](decisions/ADR-0004-arm64-compatibility-policy.md) |

## ADR Status Definitions
- **Proposed**: Under active technical investigation.
- **Accepted**: Approved baseline governing current implementation.
- **Superseded**: Replaced by a newer ADR (retained for historical auditability).
- **Deprecated**: No longer applicable.
