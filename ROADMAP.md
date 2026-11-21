# BIFRÖST Development Roadmap

This roadmap tracks BIFRÖST's incremental delivery from feasibility research to a distributable Windows 11 Android subsystem.

**Status legend**:
- `Implemented` — Working code, tests executed and passing, documented.
- `Partially implemented` — Core slice working; remaining items explicitly listed.
- `Planned` — Scoped and designed, not yet started.
- `Experimental` — Prototype under evaluation; interfaces may change.
- `Unsupported` — Explicitly out of scope for now.

---

## Milestone 0 — Research & Feasibility (`Implemented`)
- [x] Inspect repository and host Windows 11 development environment.
- [x] Produce [Technical Feasibility Report](docs/research/feasibility-report.md).
- [x] Define baseline architecture, diagrams, and technical risks in [ARCHITECTURE.md](ARCHITECTURE.md).
- [x] Record initial ADRs ([ADR-0001](docs/decisions/ADR-0001-aosp-x86-64-foundation.md) through [ADR-0004](docs/decisions/ADR-0004-arm64-compatibility-policy.md)).
- [x] Establish Git workflow, security policy, contributing guide, changelog, and roadmap.

---

## Milestone 1 — Bifrost Core & Mimir CLI Foundation (`Planned` — **Next Small Implementation Milestone**)

### Prerequisites on Development Host
1. Install Rust stable toolchain (`x86_64-pc-windows-msvc`) via `rustup`, including `rustfmt` and `clippy`.
2. Install Visual Studio C++ Build Tools (MSVC linker + Windows 11 SDK).

### Concrete Deliverables (First Small Vertical Slice)
- [ ] Initialize Cargo workspace (`Cargo.toml`) with two focused crates only (no empty placeholders):
  - `crates/bifrost-core`
  - `crates/mimir` (binary name: `bifrost`)
- [ ] Implement in `bifrost-core`:
  - Typed runtime state machine (`Stopped`, `Starting`, `Booting`, `Running`, `Degraded`, `Stopping`, `Failed`) with validated transitions.
  - Versioned TOML configuration loader/validator (CPU cores, RAM MB, storage GB, display resolution/DPI, log level, safe canonical paths with traversal protection).
  - Structured error types (`thiserror`) including explicit `Unsupported` variant.
  - Structured logging bootstrap (`tracing` + `tracing-subscriber`).
  - Domain models for system capabilities and version metadata.
- [ ] Implement in `mimir` (`bifrost` CLI via `clap`):
  - `bifrost version` — prints version and platform metadata.
  - `bifrost config show` / `bifrost config validate` — loads and validates config safely.
  - `bifrost status` — reports honest unstarted/no-daemon state (never fakes a running runtime).
  - Stubbed future commands (`doctor`, `runtime start/stop/restart`, `devices`, `apps list`, `install`, `uninstall`, `logs`) returning explicit exit code & `Unsupported (Planned in Milestone X)` messages.
- [ ] Unit tests for state transitions, config bounds validation, path traversal rejection, and CLI exit behavior.
- [ ] Verify cleanly with `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets -- -D warnings`, and `cargo test --workspace`.

---

## Milestone 2 — Windows Compatibility (`bifrost doctor`) (`Planned`)
- [ ] Implement native Windows 11 diagnostics using the `windows` crate:
  - Windows version, build number, and edition (Home/Pro/Enterprise).
  - CPU architecture, core count, and virtualization capability (handling the `HypervisorPresent: True` vs. `VirtualizationFirmwareEnabled: False` WMI quirk accurately).
  - `HypervisorPlatform` (WHPX) API availability via `WHvGetCapability` (`WinHvPlatform.dll`).
  - Total/available RAM, target drive free disk space, and GPU/driver detection.
- [ ] Output human-readable and `--json` compatibility reports with actionable remediation steps.

---

## Milestone 3 — Midgard Prototype (`Planned`)
- [ ] Create `crates/bifrost-storage`, `crates/bifrost-hypervisor`, and `crates/bifrost-runtime`.
- [ ] Implement image manifest SHA-256 verification and persistent `userdata` virtual disk management.
- [ ] Implement `HypervisorBackend` trait + Phase 1 WHPX-accelerated QEMU backend.
- [ ] Boot a **real AOSP x86_64** image inside Windows 11 with serial console access, runtime log capture, and clean shutdown.
- [ ] Document hardware acceleration bring-up (`virtio-gpu` + `gfxstream` vs. SwiftShader fallback).

---

## Milestone 4 — Heimdall & Bridge (`Planned`)
- [ ] Create `crates/heimdall` and `crates/bifrost-bridge`.
- [ ] Implement Heimdall background daemon with runtime supervision, health heartbeats, and crash recovery.
- [ ] Implement Windows Named Pipe local IPC authenticated via current-user SID DACL + strict message validation.
- [ ] Implement loopback-only (`127.0.0.1`) ADB orchestration and device discovery.
- [ ] Wire `mimir` (`bifrost status`, `runtime start|stop|restart`, `devices`, `logs`) to Heimdall IPC.

---

## Milestone 5 — Valhalla Application Management (`Planned`)
- [ ] Create `crates/valhalla`.
- [ ] Query real installed packages from Android `PackageManager`.
- [ ] Implement safe APK validation (canonical path check, zip/manifest inspection, ABI check flagging ARM64-only APKs).
- [ ] Implement real APK installation, uninstallation, metadata retrieval, and activity launching via Bridge.

---

## Milestone 6 — Deep Windows Integration (`Planned` / `Experimental`)
- [ ] Evaluate and migrate/add `crosvm` WHPX + `gfxstream` backend in `bifrost-hypervisor`.
- [ ] Implement scoped host ↔ guest directory sharing with strict path traversal guards.
- [ ] Implement bidirectional clipboard sync and Android → Windows native toast notifications.
- [ ] Implement **independent per-app Windows desktop windows** (Android `VirtualDisplay` + freeform + `vsock` frame/input bridge → lightweight Rust Win32 host windows).
- [ ] Audio routing and touch/mouse/DPI scaling integration.

---

## Milestone 7 — Desktop Application (`apps/bifrost-ui`) (`Planned`)
- [ ] Build Windows 11 Fluent Design 2 desktop application (Tauri 2 with Rust backend talking strictly to Heimdall IPC).
- [ ] Sections: Dashboard, Application Manager, Runtime Configuration, Diagnostics, and Settings (light/dark themes, accessible layout, zero ads).

---

## Milestone 8 — Distribution & Lifecycle (`Planned`)
- [ ] Signed Windows installer (MSIX / WiX) and clean uninstaller.
- [ ] Runtime image downloader with Ed25519 signature + SHA-256 verification.
- [ ] Independent Android runtime upgrades preserving persistent `userdata`.
- [ ] Runtime backup, recovery, and diagnostic report export.

---

## Explicitly Unsupported Capabilities
- **ARM64 binary translation (`libhoudini` / `libndk_translation`)**: `Unsupported` (proprietary binaries prohibited; clean-room research only).
- **Proprietary Google Play Services / GApps redistribution**: `Unsupported`.
- **Windows 10 / 32-bit Windows / Nested Cloud VMs without WHPX**: `Unsupported`.
