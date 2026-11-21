# ADR-0002: WHPX-First Virtualization Behind a Rust VMM Abstraction

- **Status**: Accepted
- **Date**: 2026-11-21
- **Milestone**: Milestone 0

## Context

Windows 11 **Home** (verified on the primary development host: Windows 11 Home Build `26200`) does **not** include the full `Microsoft-Hyper-V` management role. It **does** include `VirtualMachinePlatform` (VMP) and `HypervisorPlatform` (WHPX, `WinHvPlatform.dll`). Requiring Hyper-V Pro would exclude a large portion of Windows 11 users. Meanwhile, writing a complete Virtual Machine Monitor (VMM) with PCI/VirtIO/ACPI/GPU devices from scratch in Rust carries severe schedule and reliability risk before proving end-to-end Android execution.

## Decision

1. Target **Windows Hypervisor Platform (WHPX)** as the primary hardware virtualization layer so BIFRÖST works across Windows 11 Home, Pro, and Enterprise.
2. Define a strict VMM-agnostic Rust interface (`HypervisorBackend` / `VmInstance`) inside `bifrost-hypervisor` so `heimdall`, `bifrost-runtime`, `bridge`, and `valhalla` never depend directly on a specific VMM CLI or process model.
3. Execute a **two-phase VMM backend plan**:
   - **Phase 1 (Milestone 3 prototype)**: Implement a Rust-orchestrated **QEMU (`-accel whpx`)** backend to prove a real, deterministic Android x86_64 boot quickly and verify storage, console, and ADB bring-up.
   - **Phase 2 (Production performance target)**: Evaluate and integrate **`crosvm`** (open-source Rust VMM with Windows WHPX + `virtio-gpu` / `gfxstream` support, proven by Google Play Games on PC) as the primary native Rust VMM backend.

## Alternatives Considered

- **Full Hyper-V (`Microsoft-Hyper-V`)**: Rejected because Windows 11 Home does not support it.
- **HCS / Virtual Machine Platform (WSL 2 / WSA approach)**: Viable for CPU/RAM VM lifecycle on Windows 11 Home, but Microsoft's vGPU/DirectX paravirtualization and RAIL window plumbing used by WSLg/WSA are proprietary and not publicly exposed for third-party Android guests.
- **Custom Rust VMM directly over `WinHvPlatform.dll`**: Deferred; reinventing VirtIO PCI/MMIO and GPU streaming before proving Midgard boot creates unnecessary risk.

## Consequences

- **Positive**: Broad Windows 11 Home/Pro compatibility; fastest verified path to Milestone 3 real Android boot; clear migration path to a permissive-licensed (`BSD-3-Clause`) Rust VMM (`crosvm`).
- **Negative / Trade-off**: Users must enable the Windows optional feature `HypervisorPlatform` (WHPX); QEMU (GPLv2) must remain a strictly external child process with proper GPLv2 compliance while used in Phase 1.
