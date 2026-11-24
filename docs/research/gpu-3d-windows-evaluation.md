# Windows 11 Hardware 3D GPU Acceleration Evaluation (ADR-0002 / ADR-0003 Phase 2)

**Date**: 2026-11-22  
**Test Host**: Windows 11 Home `25H2` (`10.0.26200.6584`, x64), Intel Core Ultra 9 285H (16 logical cores), 31.37 GB RAM, Intel Arc 140T GPU (16 GB, driver `32.0.101.8860`)  
**VMM Baseline**: QEMU `11.1.0` (`v11.1.0-12130-ge470268ff4`) over Windows Hypervisor Platform (`WHPX`)

---

## 1. Purpose

Evaluate the actual empirical viability of hardware-accelerated 3D graphics for Android `x86_64` guests on Windows 11 without relying on speculative claims, comparing:
1. **`virtio-gpu-pci` (2D DRM/KMS + `minigbm` / SwiftShader)** — Current default (`GraphicsMode::Auto`).
2. **`virtio-gpu-gl-pci` + ANGLE / `virglrenderer` on QEMU 11.1.0 Windows** — Experimental headless GL path (`GraphicsMode::Gfxstream` + `--headless`).
3. **`crosvm-whpx` + `gfxstream` (Rust VMM)** — Target ADR-0002 / ADR-0003 architecture.

---

## 2. Empirical Findings on Windows 11 + Intel Arc 140T

### 2.1 Bundled QEMU 11.1.0 Windows Graphics Libraries
Inspection of `C:\Program Files\qemu\` confirms official QEMU 11.1.0 for Windows bundles:
- `libvirglrenderer-1.dll` (`881 KB`)
- `libepoxy-0.dll` (`1.77 MB`)
- ANGLE OpenGL ES / Vulkan translation stack: `libEGL.dll` (`261 KB`), `libGLESv2.dll` (`11.6 MB`), `libGLESv2_vulkan_secondaries.dll` (`11.6 MB`)

### 2.2 Graphical GTK + GL Probe (`-device virtio-gpu-gl-pci -display gtk,gl=es`)
- **Result**: QEMU starts but repeatedly logs `qemu: GtkGLArea console lacks DMABUF support.` because Windows lacks Linux `DMABUF` file-descriptor sharing required by GTK's GL scanout path.
- **Decision**: Do **not** enable `gtk,gl=es` for graphical QEMU windows. Keep `-display gtk,zoom-to-fit=on` + `-device virtio-gpu-pci,xres=1280,yres=720` as the reliable default (verified full Android 17 / LineageOS 24.0 boot in ~28s, zero freeze).

### 2.3 Headless ANGLE / virgl Probe (`-device virtio-gpu-gl-pci,blob=on,hostmem=256M -display egl-headless`)
- **Result**: Initialized cleanly on Intel Arc 140T over WHPX alongside dedicated VirtIO block `iothread` (`-object iothread,id=io0`, `cache=writeback,aio=threads`) with zero graphics or block errors (`PROBE_ALIVE=True`).
- **Decision**: Exposed as an opt-in experimental mode in `QemuWhpxBackend` when `runtime.graphics_mode = "gfxstream"` AND Midgard runs in `--headless` mode (the primary WSA-style zero-friction path where per-app rendering uses hardware `D3D11VA` decoding in `bifrost-display` at 60 FPS / 12 Mbps).

### 2.4 `crosvm-whpx` + `gfxstream` (ADR-0002 Target Backend)
- **Host Integration Status**: `CrosvmWhpxBackend` is implemented and unit-tested in `crates/bifrost-hypervisor/src/crosvm_whpx.rs` (`--hypervisor whpx --gpu backend=gfxstream,egl=true,gles=true,vulkan=true`).
- **Remaining Upstream Gate**: Upstream `crosvm` does not yet publish signed prebuilt Windows `x86_64` release binaries with `gfxstream` bundled; enabling it requires a dedicated Windows MSVC/Clang build pipeline for `crosvm` + `rutabaga_gfx` + `gfxstream` (planned for a future `v0.4.x` release).

---

## 3. Summary Recommendations

| Mode | Status | Recommended Use |
|---|---|---|
| `qemu-whpx` + `virtio-gpu-pci` + `gtk` / `none` | **Implemented & Verified** | Default for all Windows 11 Home/Pro hosts (full Android 17 boot + WSA-style `D3D11VA` per-app windows) |
| `qemu-whpx` + `virtio-gpu-gl-pci` + `egl-headless` | **Experimental — no hardware acceleration** | Measured 2026-11-24: guest Mesa falls back to `llvmpipe`, host logs virgl `SET_SCANOUT` errors and boot is much slower (ADR-0007). Research only. |
| `crosvm-whpx` + `gfxstream` | **Implemented (host wrapper); Experimental (VMM binary)** | Future zero-copy 3D Vulkan/GLES backend once Windows `crosvm` CI artifacts are built |

## 4. Guest renderer measurement (2026-11-24)

The probes above only showed that QEMU starts. Asking Android which renderer SurfaceFlinger uses (`adb shell dumpsys SurfaceFlinger | grep GLES`) shows that neither QEMU mode reaches the host GPU:

- `auto` (2D `virtio-gpu-pci`): `ANGLE (Google, Vulkan 1.3.0 (SwiftShader Device (LLVM 16.0.0)))`, software.
- `gfxstream` + `egl-headless` (`virtio-gpu-gl-pci`): `Mesa, llvmpipe (LLVM 16.0.0, 128 bits), OpenGL ES 3.2`, software, with repeated host `virtio_gpu_virgl_process_cmd: ctrl 0x103, error 0x1203` and a boot over 6.5 minutes instead of about 2.5.

The decision that follows is recorded in [ADR-0007](../decisions/ADR-0007-guest-gpu-rendering-on-windows.md).

