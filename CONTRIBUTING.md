# Contributing to BIFRÖST

Thank you for your interest in contributing to **BIFRÖST** — an open-source, Rust-powered Android subsystem for Windows 11.

## Engineering Philosophy

- **Real Systems Engineering**: No fake features, no simulated command outputs, and no empty placeholder crates.
- **Incremental Vertical Slices**: Keep changes small, verifiable, and aligned with the active milestone in [ROADMAP.md](ROADMAP.md).
- **Security First**: Review [SECURITY.md](SECURITY.md) before modifying IPC, filesystem, ADB, or virtualization code.
- **English Documentation**: All documentation, ADRs, commit messages, and code comments are written in English.

## Git Workflow

BIFRÖST uses a structured Git Flow model:

```text
main        # Stable releases only — never commit directly to main
 └── develop
      ├── feature/<kebab-case-name>
      ├── fix/<kebab-case-name>
      ├── refactor/<kebab-case-name>
      ├── perf/<kebab-case-name>
      ├── docs/<kebab-case-name>
      ├── chore/<kebab-case-name>
      └── release/vX.Y.Z          # Version preparation, merged --no-ff into main and develop
```

1. Branch from `develop`:
   ```bash
   git checkout develop
   git checkout -b feature/your-feature-name
   ```
2. Make small, coherent commits following **Conventional Commits** (`feat:`, `fix:`, `refactor:`, `docs:`, `test:`, `chore:`, `perf:`, `build:`, `ci:`).
3. Verify formatting, lints, tests, and build before merging (with the GNU toolchain in a non-ASCII checkout path such as `BIFRÖST`, set an ASCII `CARGO_TARGET_DIR` first):
   ```bash
   cargo fmt --all -- --check
   cargo clippy --workspace --all-targets --locked -- -D warnings
   cargo test --workspace --locked
   cargo build --workspace --locked
   cargo deny check   # cargo install cargo-deny --locked
   ```
4. Integrate completed work into `develop` with `--no-ff`.
5. Releases: prepare `release/vX.Y.Z` (version bump + `CHANGELOG.md` section), merge it into `main` and `develop`, then push the `vX.Y.Z` tag. Release binaries are built only by `.github/workflows/release.yml` (see `installer/README.md`).

## Rust Coding Standards

- Follow idiomatic Rust (`rustfmt` defaults and zero `clippy` warnings).
- Use `thiserror` for structured library errors and `anyhow` only at top-level CLI/daemon boundaries where appropriate.
- Avoid `unsafe` Rust unless interfacing directly with Windows/FFI APIs; document every `unsafe` block with an explicit `// SAFETY:` invariant comment.
- Never put arbitrary host data on an `adb shell` command line: stream it through stdin (`AdbClient::run_adb_with_stdin`) or quote it with `bifrost_bridge::quote_android_shell_arg`, or validate it against a strict character set (see `SECURITY.md`).
- Prefer native APIs over spawning `powershell.exe` (WinRT via the `windows` crate, `winreg`, `mslnk`, `rfd`). If a script is unavoidable (installers, tests), never interpolate dynamic values into it; pass them through environment variables to a fixed script.
- Keep component naming faithful to [ARCHITECTURE.md](ARCHITECTURE.md) without inventing unnecessary mythology names for ordinary abstractions.

## Architecture Decision Records (ADRs)

Any significant technical choice (hypervisor backend, graphics transport, IPC protocol, AOSP target, security policy) must be documented in `docs/decisions/ADR-XXXX-<slug>.md` and indexed in [docs/TECHNICAL_DECISIONS.md](docs/TECHNICAL_DECISIONS.md).
