# Changelog

All notable changes to **BIFRÖST** are documented in this file.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and Conventional Commits.

## [Unreleased]

### Added
- **Milestone 2 — Windows Compatibility (`bifrost doctor`)**:
  - Created `crates/bifrost-diagnostics` with pure compatibility evaluation (`evaluator`, `model`, `format`) and tightly audited native Windows probes (`RtlGetVersion`, Registry, safe x86_64 CPUID, `WinHvPlatform.dll` `WHvGetCapability` + live `WHvCreatePartition`/`WHvDeletePartition`, `GlobalMemoryStatusEx`, `GetDiskFreeSpaceExW`, and DXGI adapter/driver inspection).
  - Wired `bifrost doctor [--strict] [--json]` into `crates/mimir` and verified live on Windows 11 Home `25H2` (`10.0.26200.6584`), confirming unprivileged WHPX partition creation succeeds with `VirtualMachinePlatform` active (`22` passing workspace tests).
- **Milestone 1 — Bifrost Core & Mimir CLI Foundation**:
  - Initialized Cargo workspace (`Cargo.toml`, `Cargo.lock`) with workspace-wide `unsafe_code = "forbid"`.
  - Implemented `crates/bifrost-core`: deterministic `RuntimeStateMachine`, versioned TOML `BifrostConfig` with loopback-only ADB security guard and bounds checks, directory-traversal-resistant path validation (`paths`), structured `BifrostError` taxonomy, `VersionInfo` capability models, and `tracing` logging setup (`12` unit tests).
  - Implemented `crates/mimir` (`bifrost` binary): `version`, `status`, `config init|show|validate`, and explicit `Unsupported` (exit code `2`) stubs for planned commands (`doctor`, `runtime`, `devices`, `apps`, `install`, `uninstall`, `logs`) (`5` unit tests).
- **Milestone 0 — Research & Feasibility**:
  - Added [docs/research/feasibility-report.md](docs/research/feasibility-report.md) evaluating AOSP foundations, existing Android-on-PC projects, WHPX vs. HCS/Hyper-V virtualization, `virtio-gpu` + `gfxstream`, independent windowing strategies, ARM64 legal/technical limits, and licensing.
  - Added [ARCHITECTURE.md](ARCHITECTURE.md) with modular component boundaries, Mermaid system/sequence diagrams, security boundaries, and planned workspace layout.
  - Added Architecture Decision Records ([ADR-0001](docs/decisions/ADR-0001-aosp-x86-64-foundation.md), [ADR-0002](docs/decisions/ADR-0002-whpx-vmm-abstraction.md), [ADR-0003](docs/decisions/ADR-0003-graphics-and-windowing-strategy.md), [ADR-0004](docs/decisions/ADR-0004-arm64-compatibility-policy.md)) and [docs/TECHNICAL_DECISIONS.md](docs/TECHNICAL_DECISIONS.md).
  - Added [ROADMAP.md](ROADMAP.md), [SECURITY.md](SECURITY.md), [CONTRIBUTING.md](CONTRIBUTING.md), [README.md](README.md), and `.gitignore`.
  - Updated [CLAUDE.md](CLAUDE.md) with BIFRÖST engineering context while preserving Git workflow and documentation guidelines.
