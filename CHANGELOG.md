# Changelog

All notable changes to **BIFRÖST** are documented in this file.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and Conventional Commits.

## [Unreleased]

### Added
- **Milestone 0 — Research & Feasibility**:
  - Added [docs/research/feasibility-report.md](docs/research/feasibility-report.md) evaluating AOSP foundations, existing Android-on-PC projects, WHPX vs. HCS/Hyper-V virtualization, `virtio-gpu` + `gfxstream`, independent windowing strategies, ARM64 legal/technical limits, and licensing.
  - Added [ARCHITECTURE.md](ARCHITECTURE.md) with modular component boundaries, Mermaid system/sequence diagrams, security boundaries, and planned workspace layout.
  - Added Architecture Decision Records ([ADR-0001](docs/decisions/ADR-0001-aosp-x86-64-foundation.md), [ADR-0002](docs/decisions/ADR-0002-whpx-vmm-abstraction.md), [ADR-0003](docs/decisions/ADR-0003-graphics-and-windowing-strategy.md), [ADR-0004](docs/decisions/ADR-0004-arm64-compatibility-policy.md)) and [docs/TECHNICAL_DECISIONS.md](docs/TECHNICAL_DECISIONS.md).
  - Added [ROADMAP.md](ROADMAP.md), [SECURITY.md](SECURITY.md), [CONTRIBUTING.md](CONTRIBUTING.md), [README.md](README.md), and `.gitignore`.
  - Updated [CLAUDE.md](CLAUDE.md) with BIFRÖST engineering context while preserving Git workflow and documentation guidelines.
