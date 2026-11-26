# ADR-0007: Opt-in Host GPU Rendering Through virgl on QEMU-WHPX

- **Status**: Accepted (revised 2026-11-24 after root-causing the software fallback)
- **Date**: 2026-11-24
- **Milestone**: v0.5.0 development

## Context

ADR-0003 chose `virtio-gpu` with host-side 3D so Android apps could render on the host GPU. QEMU-WHPX offers two graphics paths:

- `graphics_mode = "auto"` (default): `virtio-gpu-pci`, a 2D device. Android renders with ANGLE on SwiftShader Vulkan, on the CPU.
- `graphics_mode = "gfxstream"` with `--headless`: `virtio-gpu-gl-pci,blob=on,hostmem=256M` with `-display egl-headless`, backed by the `libvirglrenderer-1.dll` that ships with QEMU 11.1.0 for Windows. The host side runs on ANGLE (Direct3D 11).

The first version of this ADR measured the GL path and found that Android still rendered with Mesa `llvmpipe`. That finding was correct, but the cause was ours, not a limit of the platform.

## Root cause of the software fallback

- The guest kernel saw a working 3D device: `[drm] features: +virgl +edid +resource_blob +host_visible`, with the virgl and virgl2 capsets.
- The image's Mesa includes the `virgl` Gallium driver, and gralloc (minigbm) opened it (`Loaded driver virtgpu_virgl on node /dev/dri/renderD128`).
- Mesa's EGL then logged `Failed to open any DRM device` and fell back to `llvmpipe`. To enumerate GPUs it reads the PCI device's `uevent` in sysfs, and SELinux denied that read.
- The image's SELinux policy (derived from Cuttlefish) labels the GPU's sysfs node `sysfs_gpu` only at PCI slots `00:01.0`, `00:02.0`, `00:03.0` and `00:1e.0`. QEMU had placed the GPU at `00:07.0`.

**Fix:** BIFRÖST now places the GL device at `addr=0x1e`. Mesa then selects virgl:

```
GLES: Mesa, virgl (ANGLE (Intel, Intel(R) Arc(TM) 140T GPU (16GB) ...)), OpenGL ES 2.0 Mesa 26.2.3
```

## Measurements (2026-11-24)

Reference host from the hardware matrix (Intel Core Ultra 9 285H, Intel Arc 140T, Windows 11 Home 25H2, QEMU 11.1.0, LineageOS 24.0 `virtio_x86_64`). Throwaway instances with 4 vCPUs and 4096 MB on their own loopback port, next to the user's VM. The benchmark fling-scrolls the Settings list 30 times and reads `dumpsys gfxinfo`; each mode was run twice.

| | `auto` (2D, SwiftShader) | `gfxstream` (virgl on host GPU) |
|---|---|---|
| SurfaceFlinger renderer | ANGLE / SwiftShader Vulkan (CPU) | Mesa virgl → ANGLE D3D11 → Intel Arc |
| OpenGL ES exposed to apps | **3.1** | **2.0** |
| Vulkan in the guest | SwiftShader (CPU) | none |
| Median frame time | 81 ms / 81 ms | 42 ms / 46 ms |
| 90th percentile | 113 ms / 117 ms | 65 ms / 73 ms |
| Janky frames | 79 % / 85 % | 24 % / 30 % |
| Wall time of the 30 swipes | 12.9 s / 12.6 s | 6.6 s / 6.6 s |
| Per-app windows (scrcpy `--new-display`) | work | work (D3D11VA decode, verified with Settings) |
| Host stderr | clean | ~2,000 `virtio_gpu_virgl_process_cmd: ctrl 0x103, error 0x1203` (`SET_SCANOUT` on the unused `egl-headless` scanout), no visible effect |
| Boot from a fresh `userdata` | 1–5 min on both, dominated by first-boot work and trade-in mode | same |

One virgl run had a single GPU frame outlier of about 5 s (90th GPU percentile 4950 ms). It did not recur in the follow-up below.

### Follow-up: `SET_SCANOUT` errors and the GPU stall (2026-11-25)

The error is `VIRTIO_GPU_RESP_ERR_INVALID_RESOURCE_ID` (0x1203) answering `VIRTIO_GPU_CMD_SET_SCANOUT` (0x103): the guest asks QEMU to show a resource on the `egl-headless` scanout that QEMU's virgl path does not accept. Two fresh instances (same 4 vCPU / 4096 MB setup) counted the errors after each phase and ran the Settings benchmark four times each:

| Phase | `blob=on,hostmem=256M` (shipped) | `blob=off` |
|---|---|---|
| Boot until `sys.boot_completed` | 1,895 | 3,515 |
| 60 s idle, 30 s screen off, wake | +1 | +1 |
| 4 benchmark runs | +0 | +0 |
| Opening and closing a scrcpy app window | +0 | not measured |
| Median frame time | 36–38 ms | 44–46 ms |
| Janky frames | 18–21 % | 37–40 % |
| 90th GPU percentile | 21–24 ms (no stall) | 19–20 ms (no stall) |

The errors happen while Android boots, when `drm_hwcomposer` brings up the display (it also logs `could not create drm fb -22` then); they stop once the system is up, so frames shown to the user (through SurfaceFlinger, `screencap` and scrcpy, none of which read the QEMU scanout) are not affected. `blob=off` does not remove them and is slower, so `blob=on` stays. The 5 s GPU frame did not recur in eight runs and is treated as a one-off on a busy host. Both are host-side log noise and no longer block the mode; they would only matter for a future path that presents the QEMU scanout itself.

Variants that do not work with the stock QEMU 11.1.0 Windows build:

- **Vulkan (Venus)**: `venus=on` fails with `Render server support was not enabled in virglrenderer`.
- **ANGLE Vulkan backend** (`ANGLE_DEFAULT_PLATFORM=vulkan`): QEMU crashes (0xC0000005) at start-up.
- **ANGLE on the native OpenGL driver** (`ANGLE_DEFAULT_PLATFORM=gl`): `Intel OpenGL ES drivers are not supported`.
- **SDL display with GL** (`sdl,gl=core` or `gl=es`, with and without blob): QEMU crashes (0xC0000005).

So with this QEMU build the ceiling is **OpenGL ES 2.0 on the host GPU**. ES 3.x and Vulkan need a different host stack (see Decision 4).

## Decision

1. `auto` stays the default. It is slower but exposes OpenGL ES 3.1, so apps that require ES 3.0 (many games and some engines) keep working.
2. `graphics_mode = "gfxstream"` becomes the opt-in **GPU acceleration (Experimental)** mode: virgl on the host GPU, GPU at PCI slot `0x1e`, headless only. It roughly halves frame times for ordinary apps but limits apps to OpenGL ES 2.0 and offers no Vulkan. The Settings label says so.
3. A graphics claim is only made after reading the renderer string from `dumpsys SurfaceFlinger | grep GLES`: a hardware renderer (virgl, gfxstream, ANGLE on D3D) rather than SwiftShader or llvmpipe.
4. OpenGL ES 3.x and Vulkan continue on ADR-0003's target, `crosvm` + `gfxstream` on WHPX (or the Android Emulator's QEMU, used as an external tool like scrcpy), which needs a guest image with gfxstream drivers. A QEMU build with a Venus-enabled virglrenderer is not an option on Windows: Venus depends on virglrenderer's render server.

## Consequences

- Choosing between the modes is supported by data: `bifrost apps graphics` (and the Settings check) reads `<uses-feature android:glEsVersion>` from every installed app and recommends the GPU mode only when no app needs more than OpenGL ES 2.0. Installing an app that needs more than the configured mode provides shows a warning. Apps that use OpenGL ES 3 only when it is available do not declare it and fall back to 2.0.

- Users can trade ES 3.x compatibility for much smoother rendering, per machine.
- The `gfxstream` config value now names the virgl path on QEMU. It keeps its name for compatibility with existing `bifrost.toml` files; on the `crosvm-whpx` backend it selects real gfxstream.
- NVIDIA and AMD GPUs are untested (see `docs/research/hardware-matrix.md`). ANGLE's D3D11 backend is vendor-neutral, but driver quirks are possible.
- The `SET_SCANOUT` errors are explained (boot-time only, no visible effect) and the 5 s GPU stall did not recur. What still keeps the GPU mode from being the default is OpenGL ES 2.0 and the lack of measurements on NVIDIA and AMD GPUs.
