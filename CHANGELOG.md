# Changelog

All notable changes to **BIFRÖST** are documented in this file.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and Conventional Commits.

## [Unreleased]

### Documentation
- **ADR-0007 follow-up on the GPU mode's host errors**: the `SET_SCANOUT` / `INVALID_RESOURCE_ID` messages QEMU prints with `graphics_mode = "gfxstream"` occur only while Android boots (about 1,900 per boot with the shipped `blob=on`), not during use, idle, screen off or scrcpy windows. `blob=off` produces more of them and renders slower (median frame 45 ms vs 37 ms). The isolated 5 s GPU frame did not recur in eight benchmark runs.

### Added
- **Graphics mode advice (`valhalla`, `bifrost-client`, `mimir`, `bifrost-ui`)**: APK inspection now reads the OpenGL ES version an app requires (`<uses-feature android:glEsVersion>`, the highest across an `.apkm` bundle's splits). `bifrost apps graphics` and a Settings button read the same value for every installed third-party app and recommend GPU acceleration only when none needs more than OpenGL ES 2.0. Android does not report this value in `dumpsys`, so each base APK's `AndroidManifest.xml` is extracted with the guest's own `unzip` over `adb exec-out` (paths validated first) and parsed on the host. `bifrost install`, `--inspect-only` and the App Installer warn when the configured mode cannot provide an app's version.

## [0.5.1] - 2026-11-25

### Fixed
- **Shortcut names could end in a dot (`valhalla`)**: an app label such as `Viewer 0 ."."` became `Viewer 0 .`, because trailing dots and spaces were stripped only once. Windows drops the trailing dot, so the shortcut file would not match its expected name. Any mix of trailing dots and spaces is now removed. Found by the release workflow's property test, which stopped the v0.5.0 release before it was published.
- **Live VM e2e workflow was rejected by GitHub (CI)**: it read the `runner` context in job-level `env`, which is not allowed, so every push showed a failed run. The Cargo target directory is now set from a step.

## [0.5.0] - 2026-11-25

### Security
- **Dependency audit (`deny.toml`, CI)**: `cargo deny check` runs on every push and pull request (RustSec advisories, licenses compatible with Apache-2.0, crates.io as the only source). Its first run found RUSTSEC-2026-0367/0368/0369/0429/0545/0548 in `zip` 2.6.1 and RUSTSEC-2026-0594 in `cookie` 0.18.2. Valhalla never called the affected `extract`/`ZipStreamReader`/AES paths, but it parses untrusted APKs with `zip`, so it moved to `zip` 9.8 (pure-Rust deflate only) and `cookie` 0.18.3. APK entries whose names cannot be decoded now reject the archive.

### Changed
- **MSRV raised to Rust 1.89**, required by `zip` 9.
- **CI builds with `--locked`** (clippy, tests and release build), so a stale `Cargo.lock` fails CI as it already failed the release workflow.
- **Workspace crates are marked `publish = false`**; none is published to crates.io.

### Added
- **`bifrost-client` crate**: a `Session` facade shared by Mimir and the desktop UI. It loads the effective config, probes Heimdall once, and routes status, runtime, logs, app and install operations over IPC when the daemon answers or directly otherwise. It also owns the headless auto-boot before app launch/install and the detached Heimdall start, which were duplicated in both front ends (`mimir` lost ~300 lines, `bifrost-ui`'s `service.rs` ~200).

- **Trusted Publishers card (`bifrost-ui`, `bifrost-storage`, `bifrost-core`)**: Images & Recovery lists `[images] trusted_keys` with SHA-256 fingerprints, adds keys only when they are valid Ed25519 points (the config schema only checks 64 hex characters), and asks for confirmation before removing one, naming the registered images whose `manifest.json.sig` verifies only with that key. `bifrost.toml` is edited with `toml_edit`, so comments and other settings are kept.

- **System tray icon (`bifrost-ui`)**: the notification-area menu shows the Midgard state and offers Open, Start/Stop Midgard, pinned apps (those with BIFRÖST shortcuts) and Quit, rebuilt whenever the pointer enters the icon. Closing the window hides it to the tray (`[desktop] close_to_tray`, on by default, Settings checkbox), and launching the dashboard again focuses the existing window (`tauri-plugin-single-instance`). The App Installer and `--launch-app` modes keep their own windows.

- **First-run setup (`bifrost-client`, `bifrost-ui`, `mimir`)**: a checklist derived from `doctor` and the ADB endpoint state (WHPX, QEMU, `adb`, optional `scrcpy`, active image, first Android boot until ADB is `ready`). The desktop app opens a Setup view while required steps remain; `bifrost setup` prints it. Missing tools install only on request, from an allowlist of `winget` packages, in a visible console without `--silent` or `--accept-package-agreements`, so the license prompt, installer UI and UAC stay in front of the user. The first boot runs Midgard with a window (restarting a headless VM) so the user can finish Android's welcome screens and allow USB debugging. Enabling Windows features stays a manual step with instructions.
- **Tool lookup sees fresh installs (`bifrost-core`)**: `adb`, `scrcpy` and QEMU are also searched on the persisted user/machine `PATH` from the registry, so a tool `winget` just installed is found without restarting BIFRÖST. The three duplicated lookups now share `bifrost_core::find_on_path`.

- **Update notice (`bifrost-client`, `mimir`, `bifrost-ui`)**: `bifrost update` and a Settings card compare the running build with the latest GitHub release and explain how to upgrade with the SHA-256-verified release bundle. The check runs only on request, uses Windows' own `curl.exe` (HTTPS only, 15 s limit, no user or host data) and downloads or installs nothing.

- **Live VM e2e workflow (Experimental)**: `.github/workflows/e2e-live-vm.yml` runs the `e2e_live_vm` test on a self-hosted Windows 11 runner labelled `whpx`, only on manual dispatch (never for pull requests), and fails instead of passing vacuously when no image manifest is configured. No runner is registered yet; see `docs/research/self-hosted-e2e-runner.md`.

- **GPU acceleration actually reaches the host GPU (`bifrost-hypervisor`, Experimental)**: with `graphics_mode = "gfxstream"` (headless), Android fell back to Mesa `llvmpipe` because the guest SELinux policy labels the GPU's sysfs node only at PCI slots 01/02/03/1e and QEMU placed it at 07. The GL device now sits at `addr=0x1e`, and SurfaceFlinger renders through virgl on the host GPU (ANGLE Direct3D 11). On the reference Intel Arc 140T, median UI frame time drops from about 81 ms to 44 ms and janky frames from about 80 % to 27 %. The mode is limited to OpenGL ES 2.0 with no Vulkan, so the 2D default (OpenGL ES 3.1 in software) stays the default.

### Documentation
- **Hardware compatibility matrix** (`docs/research/hardware-matrix.md`): the verified reference host and the untested configurations, with how to verify each.
- **ADR-0007, guest GPU rendering**: documents the measured renderers, the root cause of the virgl software fallback and the benchmark behind the opt-in GPU mode.

### Fixed
- **Status badges overflowed (`bifrost-ui`)**: long labels such as `IMPLEMENTED` spilled out of the fixed 74 px badge column in Settings; check lists now share one column sized to the longest badge.
- **Blocking desktop commands ran on the window thread (`bifrost-ui`)**: synchronous Tauri commands execute on the main thread, so ADB calls (processes, port forwards), image SHA-256 hashing, `userdata.img` backup/restore and the doctor report froze the window while running. All 20 I/O commands now run on the blocking pool.
- **UI froze a Tokio worker while auto-booting (`bifrost-ui`)**: launching an app window with Midgard stopped waited up to 90 s for ADB on an async worker. Direct ADB, runtime and Valhalla calls from the UI now run on the blocking pool.
- **Install with no active image (`mimir`, `bifrost-ui`)**: installing while no device is online and no image is active now reports why Midgard could not auto-start, instead of failing later with "no device".

### Performance
- **Toast identity registered once per process (`bifrost-bridge`)**: every notification used to rewrite the icon file and the HKCU AUMID values. Registration now runs until it succeeds once; failures are retried on the next toast. The toast icon is embedded from `crates/bifrost-bridge/assets` instead of reaching into the desktop app's directory.

## [0.4.0] - 2026-11-24

### Security
- **Clipboard shell injection (`bifrost-bridge`)**: Windows clipboard text synced to Android (manually or automatically by Heimdall) was passed unquoted to `adb shell`, so the Android `sh` interpreter executed it. Added `quote_android_shell_arg` (POSIX single-quote escaping) with regression tests, and applied it to clipboard sync and the shared-folder `mkdir`.
- **Image trust store (`bifrost-core`, `bifrost-storage`, `mimir`, `bifrost-ui`)**: Ed25519 manifest signatures now count only when they verify against `[images] trusted_keys` in `bifrost.toml` (detached `manifest.json.sig` supported). Invalid signatures are always rejected, and unsigned bundles require an explicit `--allow-unsigned` (CLI) or the "Allow unsigned bundle" checkbox (UI). See ADR-0006.

- **QMP off TCP (`bifrost-hypervisor`, `bifrost-runtime`)**: QEMU's unauthenticated QMP monitor listened on `127.0.0.1:<port>`, so any local process or user could power off or control the VM. It now uses `-chardev socket,server=on,wait=off` on an AF_UNIX socket file with a random 128-bit name inside the per-user data directory (`data_dir\run`, falling back to `%TEMP%\bifrost-run`), protected by the file ACL. QMP I/O runs on a worker thread with timeouts. A Windows named pipe was tried first during development, but QEMU's pipe chardev blocked VM start-up until a client connected; the live end-to-end test caught this before release. VMs started by older versions (TCP endpoints) can still be shut down.
- **Clipboard via stdin (`bifrost-bridge`)**: clipboard text is now streamed through `adb` stdin into a fixed `cmd clipboard set-text "$(cat)"` script (`AdbClient::run_adb_with_stdin`). It never appears on a command line, which also removes the ~32K Windows command-line limit that broke large syncs.

- **Pinned CI actions (`.github/workflows`)**: every third-party action in the CI, fuzz and release workflows is pinned to a full commit SHA (version kept as a comment), so a moved tag cannot change what builds and signs a release.

### Changed
- **No PowerShell at runtime (`bifrost-bridge`, `valhalla`, `bifrost-ui`)**: replaced every runtime `powershell.exe` invocation with native APIs:
  - Toasts use WinRT directly under a registered `BIFROST.Bifrost` AppUserModelID (`HKCU\Software\Classes\AppUserModelId`), so they show "BIFRÖST" instead of "Windows PowerShell".
  - HKCU `.apk`/`.apkm` association and Heimdall login autostart use `winreg`; the cycle is tested against a scratch registry subtree.
  - Desktop and Start Menu `.lnk` files are written natively with `mslnk`, and the Desktop is resolved through the Known Folder API (OneDrive redirection aware).
  - The UI's Browse dialogs use the native `IFileOpenDialog` through `rfd`.
  - `uninstall-bifrost.ps1` also removes the AppUserModelID key.
- **Breaking**: removed `bifrost images register --pubkey`; trusted keys come only from `bifrost.toml`.

### Added
- **Release pipeline (`.github/workflows/release.yml`, `build-release-bundle.ps1`)**: `vX.Y.Z` tags trigger an MSVC release build with quality gates, optional Authenticode signing (secrets-driven; signing happens before hashing), a Sigstore build-provenance attestation, and a GitHub Release whose notes come from this changelog. The packager also bundles the installer scripts and `LICENSE`, emits a versioned `.zip` with its `.sha256`, builds with `--locked`, and refuses to wipe an output directory that is not a previous bundle.

### Fixed
- **Concurrent VM starts (`bifrost-runtime`)**: `start`, `stop`, `restart` and `ensure_started_headless` now hold a cross-process lifecycle lock (`instances\<id>\lifecycle.lock`, opened with share mode 0 and released by the OS when its holder exits). Two simultaneous starts from the CLI, the UI or Heimdall could previously both spawn a VMM on the same `userdata` disk; the second caller now waits and sees the running instance.
- **Heimdall token clobbering (`heimdall`)**: a second daemon wrote a new auth token before failing on `first_pipe_instance`, locking every client out of the running daemon. The pipe is now claimed before the token is written.
- **Heimdall responsiveness (`heimdall`)**: runtime start (image hashing), stop, `adb` round-trips and the periodic health probe ran on async workers and could stall the daemon for minutes; they now run on Tokio's blocking pool. Clients that connect without sending a request frame time out after 10 seconds.
- **`--json` output (`bifrost-core`)**: `tracing` logs were written to stdout, so `bifrost --json runtime start` emitted log lines before its JSON document and no consumer could parse it. Logs now go to stderr. Found by the live-VM end-to-end test.
- **Custom `adb_port` stuck in `booting` (`bifrost-bridge`, `mimir`, `heimdall`, `bifrost-ui`)**: the ADB server only auto-discovers ports 5555–5585 and nothing ran `adb connect` for Midgard's endpoint, so a VM on any other port never reached `running`. `AdbClient::ensure_connected` now attaches the endpoint while booting. Heimdall answers `DaemonHealth` from the monitor's last probe so a slow `adb connect` cannot time clients out (measured on a real VM: max 35 ms over 117 probes).
- **Silent "booting" when ADB is blocked (`bifrost-bridge`, `heimdall`, `mimir`, `bifrost-ui`)**: on a live VM, a fresh `userdata` boots Android 16+ into *trade-in mode* (adbd reports `device` but closes every service except `tradeinmode` until the setup wizard is completed), and a provisioned `userdata` rejects an unapproved host key as `unauthorized`. Both left Midgard stuck in `booting` with no explanation. `AdbClient::diagnose_endpoint` now classifies the endpoint, Heimdall reports it as `adb_access` and logs the fix, `bifrost status` shows the remediation as its note (and `adb_access` in `--json`), and the dashboard shows "ADB blocked: …". The live-VM end-to-end test leaves trade-in mode on its disposable disks and now passes against LineageOS 24.
- **"Multiple Android devices online" (`bifrost-bridge`, `valhalla`, `bifrost-process`, `bifrost-network`)**: the ADB server registers a VM answering on 5555–5585 a second time as `emulator-<N>`, so Midgard on the default port showed up twice and every command without `--serial` failed (app list/install/launch, Desktop shortcuts, processes). `AdbClient::online_devices` now collapses that alias onto the `127.0.0.1` endpoint.
- Non-Windows stubs in `valhalla` referenced a non-existent `BifrostError::Unsupported` field and would not compile off Windows.

### Documentation
- Rewrote `ARCHITECTURE.md` to reflect the implemented system, added ADR-0005 (scrcpy per-app windows) and ADR-0006 (image trust store), amended ADR-0001/0003, extended `ROADMAP.md` with v0.1.0–v0.3.3 and next steps, and aligned `SECURITY.md`, `README.md`, `CONTRIBUTING.md` and `CLAUDE.md` with the code (`116` passing workspace tests across 13 crates).

## [0.3.3] - 2026-11-22

### Added
- **Windows 11 Fluent App Installer Dialog (`bifrost-ui`, `valhalla`)**:
  - Double-clicking or right-clicking (**"Install with BIFRÖST"**) any `.apk` or `.apkm` file in Windows Explorer now launches a dedicated compact Fluent Design 2 App Installer window (`bifrost-ui.exe --install-apk "%1"`, zero console flash) — also reused for in-app installs and drag-and-drop.
  - Displays the app's real extracted PNG icon, clean app title (`info.json` `app_name` / package metadata), and `versionName` (parsed from binary AXML `AndroidManifest.xml` or `.apkm` `info.json`), automatically distinguishing **Install** vs. **Update** (`Installed: vX → New: vY`).
  - Live animated progress bar streaming real installation stages (`install-progress` events) plus an instant **Launch App** button upon completion.
- **Host CPU/RAM Auto-Tuning (`bifrost-core`, `mimir`, `bifrost-ui`)**:
  - Added `RuntimeConfig::recommend_for_host()`, `bifrost config autotune`, and an **Auto-Tune for this PC** button in `bifrost-ui` Runtime Config to scale vCPUs and RAM cleanly for high-core Windows 11 hosts while preserving host headroom.

### Changed & Optimized
- **Single-Pass `.apk` / `.apkm` Installation Pipeline (`valhalla`)**:
  - Eliminated redundant multi-pass decompression during `.apkm` inspection, split staging, and shortcut icon extraction, upgrading hashing and split-copy I/O to 512 KiB buffered streams.
- **Full-Stack Performance Tuning (`bifrost-display`, `bifrost-hypervisor`, `heimdall`, `bifrost-ui`)**:
  - Consolidated `bifrost-display` Android freeform setup from 3 sequential `adb.exe` calls into 1 call and tuned per-app `VirtualDisplay` streams to `--max-fps=60 --video-bit-rate=12M`.
  - Cached `DoctorReport` and Heimdall autostart status in `bifrost-ui` for sub-50ms Dashboard refreshes (freshly recalculated on **Run Doctor** or toggle).
  - Reduced Heimdall idle overhead to near-0% CPU by skipping `adb.exe` polls while Midgard is stopped, caching confirmed `boot_completed` guest health, and spacing `app_sync` to every 6 cycles.
  - Attached dedicated QEMU VirtIO block `iothread` (`-object iothread,id=io0`, `cache=writeback,aio=threads`), added `CREATE_NO_WINDOW` for headless QEMU, and upgraded experimental headless 3D GPU mode to `virtio-gpu-gl-pci,xres=1280,yres=720,blob=on,hostmem=256M` + `-display egl-headless` (`71` passing workspace tests across 13 crates).

## [0.3.2] - 2026-11-22

### Added
- **Full `.apkm` Split-APK Bundle Support (`valhalla`, `mimir`, `bifrost-ui`)**:
  - Extended Windows Explorer integration (`HKCU\Software\Classes`, zero UAC) to register both **`.apk` and `.apkm`** for the **"Install with BIFRÖST"** right-click context menu and default double-click handler (`Bifrost.ApkPackage`), with clean removal on uninstall.
  - Added pure-Rust `.apkm` split-bundle inspection in `crates/valhalla/src/apk.rs` with outer + inner Zip-Slip protection, `info.json` + `AndroidManifest.xml` package discovery, high-resolution `icon.png` / `base.apk` icon extraction, and ADR-0004 ABI verification across all split APKs (`69` passing workspace tests).
  - Automatic multi-ABI split filtering (excludes foreign-architecture-only splits such as `split_config.arm64_v8a.apk` when `x86_64` is present to avoid `INSTALL_FAILED_NO_MATCHING_ABIS`) and real installation via `adb install-multiple -r`.
  - Updated `bifrost-ui` native **Browse…** file picker and window-wide **Drag & Drop** overlay to accept `.apkm` alongside `.apk`.

## [0.3.1] - 2026-11-22

### Added
- **Silent GUI Shortcut & Login Launchers (`bifrost-ui`, `valhalla`)**:
  - Desktop and Start Menu app shortcuts now invoke `bifrost-ui.exe --launch-app <pkg>` (Windows GUI subsystem, zero console flash on double-click, with Windows Toast fallback on launch failure).
  - Unprivileged Windows login autostart for Heimdall under `HKCU\Software\Microsoft\Windows\CurrentVersion\Run\BifrostHeimdall` (`bifrost-ui.exe --start-heimdall`), manageable via `bifrost daemon autostart [--enable | --disable]` and `bifrost-ui` Settings, and cleaned automatically by `uninstall-bifrost.ps1`.
  - Added `bifrost daemon start` (detached background supervisor start with 5s IPC readiness poll) and **Start / Stop Heimdall** toggle in `bifrost-ui` Dashboard.
- **Host Runtime Tooling Diagnostics (`bifrost-diagnostics`)**:
  - Extended `bifrost doctor` to 8 checks with **Host Runtime Tooling**, verifying `qemu-system-x86_64`, EDK2 UEFI firmware (`edk2-x86_64-code.fd`), `adb`, `scrcpy`, and registered Midgard images with exact remediation commands.
- **Windows 11 Desktop UX Polish (`apps/bifrost-ui`)**:
  - Native Windows file and folder pickers (`Browse…`) for `manifest.json`, `.apk` packages, and backup/restore directories (zero `unsafe`, zero extra crate dependencies).
  - Native window-wide `.apk` and `manifest.json` drag-and-drop overlay (`tauri://drag-drop`) with automatic offline SHA-256 & ADR-0004 `x86_64` ABI inspection.
  - New **Images & Recovery** view in `bifrost-ui` for listing/registering/activating verified Midgard bundles and creating/restoring SHA-256-verified `userdata.img` backups.
  - Added headless Midgard auto-boot to GUI APK installation (`install_apk_file`) and automatic active-manifest autofill on Dashboard.

### Fixed
- **Runtime State Reconciliation (`bifrost-runtime`, `heimdall`, `mimir`, `bifrost-ui`)**:
  - Reconciled `Booting` → `Running` automatically once the Android guest reports `sys.boot_completed=1` over loopback ADB (both with and without Heimdall running).
- **Pre-IPC Input Validation & Test Determinism (`mimir`)**:
  - Validated host `.apk` paths and Android package identifiers locally before dispatching over Heimdall IPC, strengthening defense-in-depth and making `cargo test --workspace` (`67` passing tests across 13 crates) fully deterministic even while Heimdall is active.
- **PowerShell 5.1 Registry Query Exit Code (`valhalla`)**:
  - Added explicit `exit 0` guards to HKCU registry helper scripts so missing optional properties never leak a non-zero PowerShell exit code.

## [0.3.0] - 2026-11-22

### Added
- **Zero-Friction WSA-Style Auto-Boot & `bifrost-ui` Window Controls (Phase 1)**:
  - `bifrost apps launch <pkg> --window` (and `bifrost install <apk>`) automatically starts Midgard in invisible `--headless` mode from the active registered image in `ImageRegistry` and waits for loopback ADB + `sys.boot_completed=1` when Android is currently stopped (`--no-autostart` opt-out supported).
  - Added **Open Window** and **Pin / Unpin** Windows shortcut controls directly to `apps/bifrost-ui`'s Application Manager (`launch_app_window`, `list_app_shortcuts`, `create_app_shortcut`, `remove_app_shortcut`).
- **Automatic Windows Desktop & Start Menu App Icons + Heimdall In-Android Sync (Phase 2)**:
  - Pure-Rust AXML `package` extraction and PNG-to-`.ico` launcher icon extraction in `crates/valhalla/src/apk.rs`.
  - Created `crates/valhalla/src/shortcuts.rs` (`#![forbid(unsafe_code)]`) for automatic Desktop (`<Desktop>\<App>.lnk`) and Start Menu (`Start Menu\Programs\BIFRÖST Apps\<App>.lnk`) shortcut creation on `bifrost install`, clean removal on `bifrost uninstall`, and manual management via `bifrost apps shortcut`.
  - Created `crates/heimdall/src/app_sync.rs` so Heimdall automatically syncs shortcuts (and pulls base APK icons) for apps installed or uninstalled from inside Android (e.g., via F-Droid).
- **Unprivileged Windows 11 `.apk` File Association & Context Menu (Phase 3)**:
  - Created `crates/valhalla/src/file_assoc.rs` registering Explorer's **"Install with BIFRÖST"** context menu and default `.apk` double-click handler strictly under `HKCU\Software\Classes` (zero admin/UAC required), wired into `bifrost desktop register-apk-handler | unregister-apk-handler`, `install-bifrost.ps1`, and `uninstall-bifrost.ps1`.
- **Native Audio, Real-Time Windows 11 Toast Notifications & Auto Clipboard Sync (Phase 4)**:
  - Added DirectSound HDA audio (`-audiodev dsound,id=snd0 -device ich9-intel-hda -device hda-duplex`) to `QemuWhpxBackend` controlled by `runtime.audio_enabled = true`.
  - Added WinRT `dispatch_windows_toast` in `crates/bifrost-bridge/src/notifications.rs` and `crates/heimdall/src/desktop_bridge.rs` for deduplicated real-time Android → Windows 11 Toast forwarding and hash-guarded automatic clipboard sync (enabled by default in `DesktopConfig` and toggleable in `bifrost-ui`).
- **Empirical Windows 11 GPU 3D Evaluation & CI Enhancements (Phase 5)**:
  - Added `docs/research/gpu-3d-windows-evaluation.md` documenting empirical Intel Arc 140T + QEMU 11.1.0 ANGLE/`virglrenderer` tests and enabled opt-in headless `virtio-gpu-gl-pci` + `egl-headless` when `graphics_mode = "gfxstream"`.
  - Added `workflow_dispatch` to `.github/workflows/ci.yml` and CI status badge to `README.md` (`64` total passing tests across 13 workspace crates).

## [0.2.0] - 2026-11-22

### Added
- **WSA-Style Dedicated & Dynamically Resizable Per-App Windows (`bifrost-display`)**:
  - Dedicated single-app Android `VirtualDisplay` mode using `--no-vd-system-decorations`, `--display-ime-policy=local`, `--mouse-bind=b+++`, `--start-app=+<pkg>`, and automatic `VirtualDisplay` cleanup on window close (no Android Home launcher, status bar, or navigation bar inside app windows).
  - Real-time dynamic resizing (`--flex-display` + `--no-window-aspect-ratio-lock`) so dragging window borders, maximizing, or using Windows 11 Snap Layouts (`Win+Left` / `Win+Right`) continuously resizes the Android `VirtualDisplay` with native responsive app reflow and `D3D11VA` hardware decoding.
  - Added automatic fallback discovery of `%LOCALAPPDATA%\Programs\scrcpy\scrcpy.exe`.
- **Canonical Apache-2.0 Licensing & Workspace Metadata**:
  - Added canonical [LICENSE](LICENSE) (Apache License 2.0) and inherited `license.workspace = true` and `repository.workspace = true` across all 13 workspace crates.

### Fixed
- **Full Graphical Android 17 / LineageOS 24.0 Boot on Windows 11 Home WHPX (`qemu_whpx.rs`)**:
  - Disabled legacy Bochs VGA (`-vga none`), attached a real DRM/KMS VirtIO GPU (`-device virtio-gpu-pci,xres=1280,yres=720`), and added `+cx16` CPU feature so Android's `virtgpu_detect`, `drm_hwcomposer`, `surfaceflinger`, and `system_server` complete full graphical boot (`bootanim` finishes cleanly in ~28s).
  - Switched Windows graphical QEMU display from `-display sdl` to `-display gtk,zoom-to-fit=on` with `CREATE_NEW_PROCESS_GROUP` detachment to eliminate the Windows SDL "Not Responding" deadlock.

## [0.1.0] - 2026-11-22

### Added
- **Live Windows 11 Home WHPX Android Boot Verified (Milestone 3 Complete)**:
  - Added automatic `qcow2` (`QFI\xfb`) vs. `raw` UEFI NVRAM (`efi_vars`) format detection, default QEMU `edk2-i386-vars.fd` fallback, Intel WHPX `-svm` CPU flag cleanup, and post-spawn crash detection in `crates/bifrost-hypervisor/src/qemu_whpx.rs` (`56` total passing workspace tests across 13 crates).
  - Verified live end-to-end boot of real open-source LineageOS 24.0 (`virtio_x86_64`, Android 17, zero GApps) over QEMU 11.1.0 + WHPX via `bifrost runtime start`, inspected real Android `init`/SELinux/`apexd` serial console logs via `bifrost logs`, and verified clean QMP/ACPI shutdown via `bifrost runtime stop`.
- **UEFI VirtIO Boot, `crosvm-whpx` Backend & Per-App VirtualDisplay Windowing**:
  - Added `BootMode::UefiDisks` (EDK2 `pflash` + per-instance writable `efi_vars.fd`/`qcow2` staging) and Windows UTF-8 BOM tolerance to `crates/bifrost-storage` and `crates/bifrost-hypervisor`.
  - Implemented `CrosvmWhpxBackend` (`crosvm run --hypervisor whpx --gpu backend=gfxstream,...`) in `crates/bifrost-hypervisor/src/crosvm_whpx.rs` per ADR-0002 / ADR-0003 (`1` unit test).
  - Created `crates/bifrost-display` (`#![forbid(unsafe_code)]`) for Android freeform multi-display configuration and independent per-app Windows 11 `VirtualDisplay` windows, wired into `bifrost apps launch <pkg> --window [--width] [--height] [--dpi]` (`55` total passing workspace tests across 13 crates).
- **Milestone 8 — Distribution, Signed Updates & Recovery**:
  - Extended `crates/bifrost-storage` (`#![forbid(unsafe_code)]`) with Ed25519 cryptographic signature verification (`signature.rs`), local Midgard image version registry & upgrade/downgrade safety (`registry.rs`), and SHA-256-verified userdata backup/restore (`recovery.rs`) (`3` new unit tests).
  - Wired `bifrost images list|register|use` and `bifrost recovery backup|restore|wipe-userdata|export-diagnostics` into `crates/mimir` (`53` total passing workspace tests across 12 crates).
  - Added reproducible Windows release bundle script, unprivileged user-space installer/uninstaller preserving `userdata` by default (`installer/windows/`), `installer/README.md`, and `.github/workflows/ci.yml`.
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
