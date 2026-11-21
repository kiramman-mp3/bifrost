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

## Milestone 1 — Bifrost Core & Mimir CLI Foundation (`Implemented`)

### Completed Deliverables
- [x] Initialized Cargo workspace (`Cargo.toml`, `Cargo.lock`) with `unsafe_code = "forbid"` and two focused crates (no empty placeholders):
  - `crates/bifrost-core`
  - `crates/mimir` (binary name: `bifrost`)
- [x] Implemented in `bifrost-core`:
  - Typed runtime state machine (`Stopped`, `Starting`, `Booting`, `Running`, `Degraded`, `Stopping`, `Failed`) with validated transitions.
  - Versioned TOML configuration loader/validator (CPU cores, RAM MB, storage GB, display resolution/DPI, log level, loopback-only ADB guard, safe path validation).
  - Structured error types (`thiserror`) including explicit `Unsupported` variant.
  - Structured logging bootstrap (`tracing` + `tracing-subscriber`).
  - Domain models for capability maturity and version metadata.
- [x] Implemented in `mimir` (`bifrost` CLI via `clap`):
  - `bifrost version` (`--json` supported) — prints version, platform target, and honest component maturity.
  - `bifrost config init | show | validate` — initializes, displays, and validates `bifrost.toml` safely.
  - `bifrost status` (`--json` supported) — reports honest stopped/disconnected state.
  - Stubbed future commands (`doctor`, `runtime start/stop/restart`, `devices`, `apps list`, `install`, `uninstall`, `logs`) returning exit code `2` and explicit `Unsupported` messages.
- [x] Verified with 17 unit tests (`12` in `bifrost-core`, `5` in `mimir`), `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo build --workspace`, and live `bifrost.exe` CLI smoke checks.

---

## Milestone 2 — Windows Compatibility (`bifrost doctor`) (`Implemented`)
- [x] Created `crates/bifrost-diagnostics` using native Windows APIs (`windows` crate, `RtlGetVersion`, Registry, CPUID, `WinHvPlatform.dll` `WHvGetCapability` + `WHvCreatePartition`/`WHvDeletePartition`, `GlobalMemoryStatusEx`, `GetDiskFreeSpaceExW`, and DXGI `IDXGIFactory1` + driver version decoding).
- [x] Handled Windows virtualization quirks accurately (preventing false negatives when `HypervisorPresent: True` hides raw firmware flags, and verifying real WHPX partition creation rather than trusting `HypervisorPresent` alone).
- [x] Wired `bifrost doctor [--strict] [--json]` into `mimir` with actionable remediation steps and 22 passing workspace unit tests (`4` new evaluator tests + `1` new CLI integration test).

---

## Milestone 3 — Midgard Prototype (`Planned` — **Next Implementation Milestone**)
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
