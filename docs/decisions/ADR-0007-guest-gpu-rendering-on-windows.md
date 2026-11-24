# ADR-0007: Guest GPU Rendering on Windows Stays in Software Until a Host 3D Path Is Proven

- **Status**: Accepted
- **Date**: 2026-11-24
- **Milestone**: v0.5.0 development

## Context

ADR-0003 chose `virtio-gpu` with host-side 3D (`gfxstream`, or virgl as an interim) so Android apps could render on the host GPU. Two paths exist in the code today:

- `graphics_mode = "auto"` (default): QEMU `virtio-gpu-pci`, a 2D device. The host GPU is not used.
- `graphics_mode = "gfxstream"` with `--headless`: QEMU `virtio-gpu-gl-pci,blob=on,hostmem=256M` with `-display egl-headless`, backed by the `libvirglrenderer-1.dll` that ships with QEMU 11.1.0 for Windows (ANGLE on the host).

The earlier evaluation (`docs/research/gpu-3d-windows-evaluation.md`) only showed that the GL device starts without errors. It did not check what renderer Android actually used.

## Measurements (2026-11-24)

Reference host from the hardware matrix (Intel Core Ultra 9 285H, Intel Arc 140T, Windows 11 Home 25H2, QEMU 11.1.0, LineageOS 24.0 `virtio_x86_64`). The GL run used a throwaway instance (own config, data directory and loopback ADB port 5598, 4 vCPUs, 4096 MB), next to the user's 2D VM.

| | `auto` (2D) | `gfxstream` + `egl-headless` |
|---|---|---|
| QEMU GPU device | `virtio-gpu-pci` | `virtio-gpu-gl-pci,blob=on,hostmem=256M` |
| Guest `/dev/dri` | `card0`, `renderD128` | `card0`, `renderD128` |
| `ro.hardware.egl` | `angle` | `mesa` |
| SurfaceFlinger `GLES` | ANGLE on Vulkan **SwiftShader** (LLVM 16) | Mesa **llvmpipe** (LLVM 16), OpenGL ES 3.2 |
| Host stderr | — | repeated `virtio_gpu_virgl_process_cmd: ctrl 0x103, error 0x1203` (`SET_SCANOUT` → invalid resource id) |
| Guest kernel log | — | SELinux denials for `udmabuf` from `RenderThread` |
| Boot (fresh `userdata`) | about 2.5 minutes (e2e test) | over 6.5 minutes; `llvmpipe` threads busy in `surfaceflinger` and the setup wizard |

So with the GL device, Mesa in the guest does not use its virgl driver. It falls back to `llvmpipe`, rendering on the CPU like the default path does, while the virgl device adds host errors and makes boot much slower. **Neither mode renders on the host GPU today.**

## Decision

1. `auto` (2D `virtio-gpu-pci` + guest ANGLE/SwiftShader) stays the default and the only recommended mode.
2. `graphics_mode = "gfxstream"` on QEMU stays **Experimental** and is documented as giving **no hardware acceleration** on Windows. It is kept only for further research.
3. BIFRÖST does not claim GPU acceleration anywhere (README, UI, docs) until a measurement like the one above shows a hardware renderer string in SurfaceFlinger (for example a virgl, gfxstream or ANGLE-on-D3D11 renderer, not SwiftShader or llvmpipe).
4. The hardware 3D work continues on ADR-0003's target, `crosvm` + `gfxstream` on WHPX. It is gated on a reproducible Windows build of `crosvm` with `rutabaga_gfx`/`gfxstream`, and on the same SurfaceFlinger check passing on at least one Intel and one other GPU vendor (see `docs/research/hardware-matrix.md`).

## Alternatives Considered

- **Make the QEMU virgl path the default**: rejected. It is slower than the default and still renders in software.
- **Debug the virgl capset / scanout errors in QEMU's Windows `virglrenderer`**: not pursued now. It depends on how the QEMU Windows build is compiled (ANGLE, EGL surfaceless), which BIFRÖST does not control or redistribute. Contributions with measurements are welcome.
- **Ship our own QEMU build with a patched `virglrenderer`**: rejected for now. It would make BIFRÖST responsible for maintaining and signing a VMM build before there is evidence it reaches the GPU.

## Consequences

- App windows remain usable for typical apps, but 3D-heavy apps and games run on the CPU.
- The README GPU requirement no longer suggests the host GPU is used.
- Measuring the renderer (`dumpsys SurfaceFlinger | grep GLES`) becomes part of verifying any future graphics change.
