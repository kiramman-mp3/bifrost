# ADR-0003: Graphics Acceleration (`virtio-gpu` + `gfxstream`) and Phased Windowing

- **Status**: Accepted
- **Date**: 2026-11-21
- **Milestone**: Milestone 0

## Context

Booting Android inside a VM provides only a single framebuffer surface and defaults to slow software rendering unless hardware GPU acceleration is configured. Achieving an experience close to native Windows 11 apps requires both accelerated 3D graphics (OpenGL ES 3.x and Vulkan for modern Android 15/16 HWUI) and independent per-app desktop windows.

## Decision

1. **Graphics Pipeline**: Standardize on **`virtio-gpu` + `gfxstream`** (via `rutabaga_gfx`, backed by host Vulkan / ANGLE-D3D12) as the target hardware-accelerated graphics stack, with **SwiftShader** retained exclusively as a headless/diagnostics fallback renderer.
2. **Phased Windowing Strategy**:
   - **Stage 1 (Milestones 3–5)**: Console / single-display bring-up to stabilize Midgard boot, Heimdall supervision, Bridge, and Valhalla APK workflows.
   - **Stage 2 (Milestone 6)**: Independent per-app Windows desktop windows using Android's native **`VirtualDisplay` + freeform resizable activities**, streaming frames and input/DPI/resize events over low-latency **`virtio-vsock` / shared memory** to lightweight native Rust **Win32 + D3D12/Vulkan host windows** (not WebView2).
   - **Stage 3 (Future Research, Post-M7)**: Investigate zero-copy custom AOSP Hardware Composer (HWC) + shared DXGI/Vulkan texture composition via Windows `DirectComposition`.
3. **Management UI (`bifrost-ui`)**: Use **Tauri 2** (Rust backend + WebView2 Fluent Design 2 frontend) strictly for the management console (Dashboard, Apps, Config, Diagnostics, Settings), keeping all critical state and IPC in Rust.

## Alternatives Considered

- **Custom AOSP HWC + RAIL-style compositor in Milestone 3**: Rejected due to extreme upfront complexity and AOSP fork maintenance burden before core runtime stability exists.
- **Direct GPU Passthrough (DDA)**: Rejected because consumer Windows 11 laptops/desktops rarely have a spare dedicated GPU and DDA breaks host desktop sharing.
- **Rendering Android app windows inside WebView2**: Rejected due to frame presentation overhead and inferior native Win32 input/composition control.

## Consequences

- **Positive**: Delivers real, usable independent Android windows in Milestone 6 without heavily forking Android's `SurfaceFlinger`, while keeping a clear upgrade path to zero-copy composition later.
- **Negative / Trade-off**: Frame-transport multi-window (Stage 2) has slightly higher latency and memory bandwidth than zero-copy shared textures; requires careful DPI scaling and focus/input synchronization testing.
