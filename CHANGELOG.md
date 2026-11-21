# Changelog

All notable changes to **BIFRÖST** are documented in this file.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and Conventional Commits.

## [Unreleased]

### Added
- **Milestone 3 — Midgard Prototype (Partially Implemented — Host Stack Complete, Live Boot Pending)**:
  - Created `crates/bifrost-storage` (`#![forbid(unsafe_code)]`) with versioned `MidgardImageManifest`, streaming SHA-256 artifact verification, and persistent `userdata.img` provisioning with confirmation-guarded wipes (`4` unit tests).
  - Created `crates/bifrost-hypervisor` (`#![forbid(unsafe_code)]`) with VMM-agnostic `HypervisorBackend` trait, pure-Rust loopback QMP client (`system_powerdown`/`quit`), and `QemuWhpxBackend` (`-machine q35,accel=whpx,kernel-irqchip=off`, VirtIO storage/net/RNG, loopback-only ADB forwarding, option-injection guards) (`4` unit tests).
  - Created `crates/bifrost-runtime` (`#![forbid(unsafe_code)]`) with `MidgardRuntime` lifecycle orchestration, atomic state persistence with live PID reconciliation, and serial/VMM log inspection (`1` lifecycle unit test).
  - Wired `bifrost runtime start|stop|restart|verify`, live `bifrost status`, and `bifrost logs` into `crates/mimir`, and added `android/midgard/README.md` + `manifest.example.json` (`31` total passing workspace tests).
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
