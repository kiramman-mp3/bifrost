# Changelog

All notable changes to **BIFRÖST** are documented in this file.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and Conventional Commits.

## [Unreleased]

### Added
- **Milestone 7 — Desktop Application (`apps/bifrost-ui`)**:
  - Built `apps/bifrost-ui/src-tauri` (`#![forbid(unsafe_code)]`) using Tauri 2 with a pure-Rust `service.rs` backend integrating Heimdall IPC, Midgard runtime, Valhalla APK/package management, native `bifrost-diagnostics`, shared directories, Win32 clipboard, processes, and networking (`2` service tests).
  - Built `apps/bifrost-ui/ui/` with Windows 11 Fluent Design 2 styling, Dark/Light themes, accessible navigation, strict CSP, and all 5 required sections (Dashboard, Application Manager, Runtime Configuration, Diagnostics, Settings).
  - Added ASCII `OUT_DIR` icon staging in `build.rs` for GNU `windres` compatibility and verified `bifrost-ui.exe` build with `50` passing tests across 12 workspace crates.
- **Milestone 6 — Deep Windows Integration (Partially Implemented)**:
  - Extended `crates/bifrost-bridge` with scoped host ↔ Android shared directories (`files.rs`, anti-traversal + guest path validation), native Win32 Unicode clipboard integration (`clipboard.rs`, audited Win32 `CF_UNICODETEXT` bounded to 1 MiB), and Android notification parser + XML-escaped Windows 11 Toast XML builder (`notifications.rs`) (`5` new unit tests).
  - Created `crates/bifrost-process` (`#![forbid(unsafe_code)]`, Android `ps -A` parser sorted by RSS) and `crates/bifrost-network` (`#![forbid(unsafe_code)]`, unprivileged loopback-only TCP port-forwarding rules) (`2` new unit tests).
  - Wired `bifrost share`, `bifrost clipboard`, `bifrost notifications`, `bifrost processes`, and `bifrost network` commands into `crates/mimir` (`48` total passing workspace tests across 11 crates).
  - Independent per-app Windows desktop windows remain explicitly classified as `Planned / Experimental` pending live Android `VirtualDisplay` streaming.
- **Milestone 5 — Valhalla Application Management**:
  - Created `crates/valhalla` (`#![forbid(unsafe_code)]`) with pure-Rust APK ZIP/`AndroidManifest.xml`/Zip-Slip inspection, SHA-256 hashing, ADR-0004 `x86_64` ABI enforcement (rejecting ARM64-only APKs with `UnsupportedAbi`), strict package ID validation, and real `pm`/`dumpsys`/`am` orchestration (`6` unit tests).
  - Extended Heimdall's authenticated Named Pipe IPC with `AppsList`, `AppInfo`, `AppInstall`, `AppUninstall`, `AppLaunch`, and `AppStop`.
  - Wired `bifrost apps list|info|launch|stop`, `bifrost install [--inspect-only]`, and `bifrost uninstall [--keep-data]` into `crates/mimir` (`41` total passing workspace tests across 9 crates).
- **Milestone 4 — Heimdall & Bridge**:
  - Created `crates/bifrost-bridge` (`#![forbid(unsafe_code)]`) with loopback-only ADB orchestration (`AdbClient`, `adb devices -l` parser, Android guest health probing), `HeimdallClient`, and authenticated IPC protocol (256-bit random session token, constant-time auth check, 64 KiB max frame guard) (`3` unit tests).
  - Created `crates/heimdall` (`heimdall.exe`) async Tokio supervisor daemon with periodic health monitoring and Windows Named Pipe IPC restricted via owner-only SDDL DACL `D:(A;;GA;;;OW)` + session token validation (`1` live Named Pipe integration test verifying valid auth, forged-token rejection, and clean shutdown).
  - Wired `bifrost devices`, `bifrost daemon status|stop`, and Heimdall IPC routing into `crates/mimir`, verified live with `heimdall.exe`, `bifrost.exe`, and host `adb.exe` (`35` total passing workspace tests).
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
