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
- [x] Initialized Cargo workspace (`Cargo.toml`, `Cargo.lock`) with workspace-wide `unsafe_code = "deny"` (`#![forbid(unsafe_code)]` per crate where no FFI is needed) and two focused crates (no empty placeholders):
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

## Milestone 3 — Midgard Prototype (`Implemented` — **Verified Live on Windows 11 Home WHPX**)
- [x] Created `crates/bifrost-storage` (`MidgardImageManifest` schema v1 supporting `direct_kernel`, `bootable_image`, and `uefi_disks` + Windows UTF-8 BOM tolerance, streaming SHA-256 verification, path traversal guards, and persistent sparse `userdata.img` management).
- [x] Created `crates/bifrost-hypervisor` (VMM-agnostic `HypervisorBackend` trait, pure-Rust loopback QMP client, Phase 1 `QemuWhpxBackend` with UEFI `pflash` EDK2 + auto-detected `qcow2`/`raw` NVRAM + VirtIO disk staging + immediate crash detection, and target-evolution `CrosvmWhpxBackend` with WHPX + `gfxstream`).
- [x] Created `crates/bifrost-runtime` (`MidgardRuntime` lifecycle coordinator, atomic `runtime-state.json` persistence with live PID reconciliation, and serial/VMM log reading).
- [x] Wired `bifrost runtime start|stop|restart|verify`, live `bifrost status`, and `bifrost logs` into `mimir`.
- [x] **Live Windows 11 WHPX Verification**: Verified 2.31 GB open-source LineageOS 24.0 (`virtio_x86_64`) SHA-256 manifest, booted real Android kernel + `init` + SELinux + `apexd` (50 preinstalled APEXes) on QEMU 11.1.0 + WHPX (`bifrost runtime start`), inspected live serial console (`bifrost logs`), and verified clean ACPI/QMP shutdown (`bifrost runtime stop`).

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

## Milestone 6 — Deep Windows Integration (`Implemented`)
- [x] Implemented scoped host ↔ Android shared directory bridge (`bifrost-bridge::files`) with root containment, guest path validation, and `bifrost share list|push|pull`.
- [x] Implemented native Win32 Unicode clipboard integration (`bifrost-bridge::clipboard` via `OpenClipboard`/`CF_UNICODETEXT`, bounded to 1 MiB) and Android clipboard sync (`bifrost clipboard get|sync-to-android`).
- [x] Implemented Android active notification parsing (`dumpsys notification --noredact`) and XML-escaped Windows 11 Toast payload rendering (`bifrost notifications list`).
- [x] Created `crates/bifrost-process` (`ps -A` discovery sorted by RSS) and `crates/bifrost-network` (unprivileged loopback-only TCP port forwarding) wired into `bifrost processes list` and `bifrost network forwards|forward|unforward`.
- [x] Created `crates/bifrost-display` (`DisplayController`: Android freeform/resizable multi-display configuration + isolated `VirtualDisplay` per-app Windows 11 desktop window launcher via `bifrost apps launch <pkg> --window`) and `CrosvmWhpxBackend` (`virtio-gpu` + `gfxstream`) in `crates/bifrost-hypervisor`.

---

## Milestone 7 — Desktop Application (`apps/bifrost-ui`) (`Implemented`)
- [x] Built `apps/bifrost-ui` with **Tauri 2** and a `#![forbid(unsafe_code)]` Rust backend integrating `bifrost-core`, `bifrost-diagnostics`, `bifrost-runtime`, `bifrost-bridge`, `bifrost-process`, `bifrost-network`, and `valhalla` (`17` Tauri commands over a testable `service.rs` layer).
- [x] Designed a zero-bloat, offline-first **Windows 11 Fluent Design 2** frontend (`apps/bifrost-ui/ui/`) with Dark & Light themes, accessible navigation, strict CSP, and all 5 required sections (**Dashboard**, **Application Manager**, **Runtime Configuration**, **Diagnostics**, and **Settings**).
- [x] Verified `bifrost-ui.exe` compiles cleanly with `50` total passing workspace tests across 12 crates.

---

## Milestone 8 — Distribution & Lifecycle (`Implemented`)
- [x] Added pure-Rust **Ed25519 signature verification** (`bifrost-storage::signature` via `ed25519-dalek`) for Midgard manifests and updates alongside streaming SHA-256 verification.
- [x] Added local Midgard **Image Registry & Upgrade Compatibility** (`bifrost-storage::registry`, `bifrost images list|register|use`) blocking silent Android downgrades and incompatible userdata switches while keeping `userdata.img` persistent across compatible upgrades.
- [x] Added verified **Backup, Restore, Guarded Wipe & Diagnostic Export** (`bifrost-storage::recovery`, `bifrost recovery backup|restore|wipe-userdata|export-diagnostics`) with mandatory `--confirm` protection for destructive operations and zero leaked auth tokens.
- [x] Added reproducible Windows 11 release packaging, unprivileged user-space installer/uninstaller scripts (`installer/windows/`), and automated Windows CI workflow (`.github/workflows/ci.yml`).

---

## Post-M8 Releases — WSA-Grade Desktop Integration (`Implemented`)
Details per release are in [CHANGELOG.md](CHANGELOG.md).
- [x] **v0.1.0**: Live LineageOS 24.0 `virtio_x86_64` boot on Windows 11 Home WHPX, `uefi_disks` boot mode, `crosvm-whpx` backend wrapper, `bifrost-display`.
- [x] **v0.2.0**: Dedicated, dynamically resizable per-app windows (`scrcpy --new-display --flex-display`, see [ADR-0005](docs/decisions/ADR-0005-scrcpy-virtualdisplay-windowing.md)), full graphical boot with `virtio-gpu-pci`, Apache-2.0 licensing.
- [x] **v0.3.0**: Headless auto-boot on app launch/install, Desktop & Start Menu shortcuts (including apps installed inside Android), HKCU `.apk` handler, DirectSound audio, Toast notifications, automatic Windows → Android clipboard sync, GPU 3D evaluation.
- [x] **v0.3.1**: GUI launchers without console windows, Heimdall login autostart, host tooling checks in `bifrost doctor`, native pickers, drag-and-drop, Images & Recovery view.
- [x] **v0.3.2**: `.apkm` split bundle support with ABI split filtering.
- [x] **v0.3.3**: Fluent App Installer dialog, host CPU/RAM auto-tune, single-pass install pipeline, performance tuning.
- [x] **v0.4.0**: Security & reliability hardening: quoted/stdin `adb shell` input, image trust store (ADR-0006), QMP off TCP, no PowerShell at runtime, cross-process VM lifecycle lock, Heimdall single-instance fix, ADB trade-in/authorization diagnostics, pinned CI actions, signed release pipeline, live-VM end-to-end test.
- [x] **v0.5.0**: First-run setup (`bifrost setup` and Setup view), system tray, update notice (`bifrost update`), shared `bifrost-client` session, trusted publishers card, dependency audit in CI, hardware matrix, and opt-in GPU acceleration through virgl on the host GPU (OpenGL ES 2.0, ADR-0007).

---

## Next — GPU Mode Maturity (`In progress`)
- [ ] Explain or fix the `virtio_gpu_virgl_process_cmd: ctrl 0x103, error 0x1203` (`SET_SCANOUT`) host errors and the isolated multi-second GPU frame seen in the virgl mode (ADR-0007).
- [ ] Recommend a graphics mode from what the installed apps declare: the virgl GPU mode is only offered when no installed app requires OpenGL ES 3.0 or higher (`<uses-feature android:glEsVersion>`).

---

## Next — Hardening & Platform Depth (`Planned`)
- [x] Quote host-controlled `adb shell` arguments (clipboard injection fix) — *done in v0.4.0*.
- [x] Local Ed25519 trust store for Midgard images ([ADR-0006](docs/decisions/ADR-0006-image-trust-store.md)) — *done in v0.4.0*.
- [ ] Publish official signed Midgard bundles from a reproducible Linux CI pipeline (ADR-0001) and ship the release key as a default trusted key.
- [ ] Midgard guest agent over `virtio-vsock` (events instead of ADB polling, Android → Windows clipboard).
- [ ] Windows `crosvm` + `gfxstream` build pipeline and live validation of `CrosvmWhpxBackend`, with a Midgard image that carries the gfxstream guest drivers, to expose OpenGL ES 3.x and Vulkan on the host GPU (ADR-0003, ADR-0007).
- [ ] Native Win32 + D3D12 per-app window host replacing `scrcpy` (ADR-0003 Stage 2 target).
- [x] End-to-end test against a live Midgard VM (`crates/mimir/tests/e2e_live_vm.rs`, opt-in) — *done in v0.4.0*.
- [ ] Broader unit coverage for `bifrost-display`, `bifrost-process`, `bifrost-network` and `bifrost-runtime`.
- [ ] Verify more hosts in the [hardware matrix](docs/research/hardware-matrix.md): an NVIDIA and an AMD GPU (default and GPU modes), an AMD Ryzen CPU, and 8 GB / 16 GB RAM machines.
- [ ] Signed MSI or MSIX installer that installs or downloads QEMU, `adb` and `scrcpy` (each verified by hash), so a new user needs one download.
- [ ] Live-verified Midgard image upgrade (LineageOS 24 to a later build) that keeps `userdata`, as an opt-in e2e test.
- [ ] Contributor on-ramp: GitHub issue templates (bug report, hardware report following the matrix procedure), project page and a short demo video.

---

## Research (`Experimental`, no commitment)
- [ ] ARM64 app compatibility through an open-source, redistributable binary translator. ARM64 support stays `Unsupported` (ADR-0004) unless such a translator proves viable; proprietary translators (`libhoudini`, `libndk_translation`) remain excluded.

---

## Explicitly Unsupported Capabilities
- **ARM64 binary translation (`libhoudini` / `libndk_translation`)**: `Unsupported` (proprietary binaries prohibited; clean-room research only).
- **Proprietary Google Play Services / GApps redistribution**: `Unsupported`.
- **Windows 10 / 32-bit Windows / Nested Cloud VMs without WHPX**: `Unsupported`.
