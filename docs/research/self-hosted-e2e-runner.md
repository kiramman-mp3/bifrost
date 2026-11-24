# Self-hosted runner for the live VM e2e test

**Status: Experimental.** The workflow exists (`.github/workflows/e2e-live-vm.yml`) but no runner is registered for the repository yet, so it has not run in GitHub Actions. The test itself (`crates/mimir/tests/e2e_live_vm.rs`) is run by hand on the reference host before each release.

## Why a self-hosted runner

GitHub-hosted Windows runners are themselves virtual machines without nested virtualization, so `WHvCreatePartition` fails there and Midgard cannot boot. The live test needs a physical Windows 11 host with:

- Windows Hypervisor Platform enabled (`bifrost doctor` → WHPX `pass`)
- QEMU and Android Platform-Tools (`bifrost setup` → tools `done`)
- a staged, SHA-256-verified Midgard image bundle
- a Rust toolchain (`rustup`, plus MSVC Build Tools or WinLibs for the GNU target)

## Security model

- The workflow only runs on `workflow_dispatch`, which needs write access to the repository. It never runs on `pull_request`, so code from forks never executes on the runner.
- `permissions: contents: read`, and the checkout does not persist the token.
- Register the runner **for this repository only**, under a dedicated, non-administrator Windows account. That account then owns the throwaway BIFRÖST data directory the test creates; the test never touches another user's VM, config or ADB keys.
- The test uses its own loopback ADB port and QMP socket, so it can run next to a VM the account's owner is using.

## Setup

1. In GitHub: **Settings → Actions → Runners → New self-hosted runner → Windows x64**, and follow the download and `config.cmd` steps as the dedicated account. When asked for labels, add `whpx` (the workflow requires `self-hosted, Windows, X64, whpx`).
2. Run the runner as that account (`run.cmd`, or the service option of `config.cmd` with that account's credentials).
3. As that account, stage the image bundle and confirm the host is ready:
   ```powershell
   bifrost setup
   bifrost runtime verify --image <path>\manifest.json
   ```
4. In GitHub: **Settings → Secrets and variables → Actions → Variables**, add `BIFROST_E2E_MANIFEST` with the manifest path on the runner (or pass it as the `manifest` input when starting the workflow).
5. Start it from **Actions → BIFRÖST live VM e2e → Run workflow**.

The job fails, rather than passing vacuously, when the manifest path is missing, and `bifrost doctor --strict` fails it early on a host that is not ready.
