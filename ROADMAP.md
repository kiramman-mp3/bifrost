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

## Milestone 3 — Midgard Prototype (`Partially implemented` — **Live AOSP Boot Verification Pending**)
- [x] Created `crates/bifrost-storage` (`MidgardImageManifest` schema v1, streaming SHA-256 verification, path traversal guards, and persistent sparse `userdata.img` management with confirmation-guarded wipe).
- [x] Created `crates/bifrost-hypervisor` (VMM-agnostic `HypervisorBackend` trait, pure-Rust loopback QMP client for graceful ACPI `system_powerdown`, and Phase 1 `QemuWhpxBackend` with VirtIO devices and loopback-only ADB forwarding).
- [x] Created `crates/bifrost-runtime` (`MidgardRuntime` lifecycle coordinator, atomic `runtime-state.json` persistence with live PID reconciliation, and serial/VMM log reading).
- [x] Wired `bifrost runtime start|stop|restart|verify`, live `bifrost status`, and `bifrost logs` into `mimir`, plus documented bundle layout in [android/midgard/README.md](android/midgard/README.md) (`31` passing workspace tests).
- [ ] **Exit gate (pending)**: Install `qemu-system-x86_64` (requires UAC approval via `winget install SoftwareFreedomConservancy.QEMU`) and execute a real end-to-end WHPX boot + serial console + clean ACPI shutdown with a verified open-source Android `x86_64` image bundle.

---

## Milestone 4 — Heimdall & Bridge (`Implemented`)
- [x] Created `crates/bifrost-bridge` (`#![forbid(unsafe_code)]`) with safe loopback-only ADB orchestration, `adb devices -l` parser, Android guest health probing (`sys.boot_completed`, release/ABI/model), 256-bit random session token generation, constant-time token comparison, and bounded 64 KiB length-prefixed JSON IPC framing.
- [x] Created `crates/heimdall` (`heimdall.exe`) background Tokio supervisor daemon with periodic runtime/ADB health monitoring and Windows Named Pipe server secured by an owner-only SDDL DACL (`D:(A;;GA;;;OW)`) plus mandatory session auth token verification.
- [x] Wired `bifrost devices`, `bifrost daemon status|stop`, and automatic authenticated Heimdall IPC routing (`status`, `runtime start|stop|restart`, `logs`) into `mimir`.
- [x] Verified live end-to-end on Windows 11 (`heimdall.exe` ↔ `bifrost.exe` Named Pipe IPC + forged-token rejection test + real `adb.exe` discovery).

---

## Milestone 5 — Valhalla Application Management (`Implemented`)
- [x] Created `crates/valhalla` (`#![forbid(unsafe_code)]`) with offline ZIP/APK structural inspection, Zip Slip rejection, streaming SHA-256 hashing, and ADR-0004 `x86_64` ABI verification (explicitly rejecting `arm64-v8a`-only APKs with `UnsupportedAbi`).
- [x] Implemented strict Android package identifier validation and real Android package management via `bifrost-bridge` (`pm list packages -f`, `dumpsys package`, `cmd package resolve-activity`, `adb install -r`, `adb uninstall`, `am start`, `am force-stop`).
- [x] Exposed all Valhalla operations over Heimdall's authenticated Named Pipe IPC and wired `bifrost apps list|info|launch|stop`, `bifrost install [--inspect-only]`, and `bifrost uninstall [--keep-data]` into `mimir`.

---

## Milestone 6 — Deep Windows Integration (`Partially implemented` — **Host/Bridge Modules Complete; Per-App Windows Pending Live Display Stream**)
- [x] Implemented scoped host ↔ Android shared directory bridge (`bifrost-bridge::files`) with root containment, guest path validation, and `bifrost share list|push|pull`.
- [x] Implemented native Win32 Unicode clipboard integration (`bifrost-bridge::clipboard` via `OpenClipboard`/`CF_UNICODETEXT`, bounded to 1 MiB) and Android clipboard sync (`bifrost clipboard get|sync-to-android`).
- [x] Implemented Android active notification parsing (`dumpsys notification --noredact`) and XML-escaped Windows 11 Toast payload rendering (`bifrost notifications list`).
- [x] Created `crates/bifrost-process` (`ps -A` discovery sorted by RSS) and `crates/bifrost-network` (unprivileged loopback-only TCP port forwarding) wired into `bifrost processes list` and `bifrost network forwards|forward|unforward`.
- [ ] **Remaining (Experimental / Pending Live VM)**: Independent per-app Windows desktop windows (`VirtualDisplay` + `vsock` frame/input stream → Win32 D3D12/Vulkan host windows) and `crosvm` + `gfxstream` backend evaluation.

---

## Milestone 7 — Desktop Application (`apps/bifrost-ui`) (`Planned` — **Next Implementation Milestone**)
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
