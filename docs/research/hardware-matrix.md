# Hardware compatibility matrix

This page records which Windows 11 host configurations BIFRÖST has actually been run on, and which ones are still unverified. A row is only marked **Verified** after the live checks below pass on that hardware; everything else is **Untested**, whatever we expect from the design.

## How a configuration is verified

On the host, with a staged and SHA-256-verified Midgard image bundle:

1. `bifrost --json doctor` — all eight checks reported (attach the JSON, with the user name removed from paths).
2. `bifrost setup` — every required step `done`.
3. The live end-to-end test, which boots Android headless on its own loopback ADB port, round-trips the clipboard and shuts down through QMP:
   ```powershell
   $env:BIFROST_E2E_MANIFEST = "<path>\manifest.json"
   cargo test -p mimir --test e2e_live_vm -- --ignored --nocapture
   ```
4. One app installed with `bifrost install`, opened in its own window (`bifrost apps launch <package> --window`), then the VM restarted to confirm user data persists.

Open an issue with the outputs to add or update a row.

## Verified

| Host | CPU / virtualization | GPU | RAM | OS | Result |
|---|---|---|---|---|---|
| Reference laptop | Intel Core Ultra 9 285H, 16 logical cores, VT-x under Windows Hypervisor (WHPX `WHvCreatePartition` ok) | Intel Arc 140T (driver `32.0.101.8860`) | 31.37 GB | Windows 11 Home 25H2 (26200) | Steps 1–3 pass with QEMU 11.1.0 + LineageOS 24.0 `virtio_x86_64`. Step 4 is exercised in daily use (F-Droid installed, opened in scrcpy windows, kept across VM restarts). Default display: `virtio-gpu` 2D with SwiftShader. Opt-in GPU mode (`graphics_mode = "gfxstream"`): virgl on the Arc, OpenGL ES 2.0, scrcpy windows verified (ADR-0007). |

## Untested

| Configuration | What could differ | What to check first |
|---|---|---|
| AMD Ryzen (AMD-V / SVM) | WHPX runs on AMD-V, but QEMU's WHPX accelerator has only been exercised here on Intel VT-x. CPUID and APIC handling differ between vendors. | `doctor` WHPX check, then whether the guest kernel boots past early start-up (`bifrost logs`). |
| NVIDIA GPU (GeForce / RTX) | Only affects the experimental GL path (`graphics_mode = "gfxstream"`: `virtio-gpu-gl-pci` + `egl-headless` through ANGLE). The default 2D path does not use the host GPU. | Default mode first; then the GL path, recording QEMU's stderr. |
| AMD Radeon GPU | Same as NVIDIA. | Same as NVIDIA. |
| 16 GB RAM | `config autotune` picks 6144 MB for Midgard (15 GB or more). `doctor` warns when the VM allocation plus 4096 MB of headroom exceeds total RAM. | `doctor` memory check, then the e2e test while normal desktop apps are open. |
| 8 GB RAM | The minimum `doctor` accepts; `config autotune` picks 2048 MB, below the 4096 MB the e2e test uses. Android 17 may be slow or be killed by its low-memory killer at that size. | Boot time and whether `sys.boot_completed` is reached; run the e2e test with `memory_mb = 2048`. |
| Windows 11 Pro / Enterprise | Same Windows Hypervisor Platform as Home; Hyper-V being enabled as well should not matter, but has not been tried. | `doctor`, e2e test. |
| Host already running Hyper-V VMs, WSL 2 or Windows Sandbox | They share the Windows hypervisor with WHPX; contention for memory and cores. | e2e test with WSL 2 running. |
| Windows 11 on ARM64 | **Unsupported**: Midgard is an x86_64 image and BIFRÖST builds for x86_64 Windows only (ADR-0004). | — |

## Notes

- GitHub-hosted runners cannot verify any row: they are virtual machines without nested virtualization, so `WHvCreatePartition` fails. See [self-hosted-e2e-runner.md](self-hosted-e2e-runner.md) for running the e2e test on real hardware from GitHub Actions.
- GPU 3D acceleration status is tracked separately in [gpu-3d-windows-evaluation.md](gpu-3d-windows-evaluation.md).
