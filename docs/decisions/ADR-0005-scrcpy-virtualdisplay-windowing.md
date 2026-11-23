# ADR-0005: Per-App Windows via `scrcpy` VirtualDisplay Streaming (Interim Stage 2)

- **Status**: Accepted
- **Date**: 2026-11-23
- **Milestone**: Milestone 6 / v0.2.0 (recorded retroactively)
- **Amends**: [ADR-0003](ADR-0003-graphics-and-windowing-strategy.md), Stage 2

## Context

ADR-0003 planned Stage 2 independent app windows as Android `VirtualDisplay`s streamed over `virtio-vsock` / shared memory to a native Rust Win32 + D3D12/Vulkan host window. That design requires a Midgard guest agent, a vsock transport, and a custom host renderer, and none of them exist yet.

`scrcpy >= 3.0` already provides the needed pieces over loopback ADB: `--new-display` creates a dedicated Android `VirtualDisplay`, `--start-app` launches a single package in it, `--flex-display` resizes the display with the host window, and the host decodes H.264 with `D3D11VA` hardware acceleration. It ships as an open-source (Apache-2.0) external executable.

## Decision

1. Implement per-app windows in `bifrost-display` by launching **`scrcpy` as an external child process** per app, with `--new-display`, `--start-app=+<pkg>`, `--no-vd-system-decorations`, `--display-ime-policy=local`, `--flex-display`, `--max-fps=60` and `--video-bit-rate=12M`.
2. Treat `scrcpy` as an **optional host dependency** that is discovered on `PATH`, at `%LOCALAPPDATA%\Programs\scrcpy`, or through `BIFROST_SCRCPY_BIN`, and is reported by `bifrost doctor`. BIFRÖST does not bundle it.
3. Keep the ADR-0003 native host window (vsock + Win32/D3D12) as the long-term target. This ADR covers the interim implementation and does not replace that goal.

## Alternatives Considered

- **Native Win32 + D3D12 host with a guest agent (ADR-0003 Stage 2 as written)**: Deferred because it needs a guest agent, a vsock transport, and an encoder/compositor path inside Android before any window can be shown.
- **Single QEMU display window**: Rejected because it does not meet the per-app WSA-style window requirement.

## Consequences

- **Positive**: Real, resizable, Snap-compatible per-app windows shipped in v0.2.0 without forking Android or writing a renderer.
- **Negative / Trade-off**: Frames are video-encoded in the guest and decoded on the host, which adds latency, uses CPU in the guest, and loses some image quality compared with shared textures. Window chrome, focus and input belong to `scrcpy`, not BIFRÖST. Running apps add one external process each.
- **Follow-up**: Revisit when a Midgard guest agent and vsock channel exist. A replacement must be recorded as a new ADR that supersedes this one.
