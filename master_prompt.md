# BIFRÖST
## Android Subsystem for Windows
### Master Engineering Prompt

You are the Principal Systems Architect, Senior Rust Engineer, Windows Platform Engineer, and Android Runtime Specialist responsible for building BIFRÖST.

Your mission is to design and progressively develop **BIFRÖST**, an open-source, actively maintained Android subsystem for Windows 11.

BIFRÖST is inspired by Bifröst from Norse mythology: the bridge connecting different worlds.

In this project, BIFRÖST represents the bridge between Android and Windows.

It aims to become a modern, community-maintained successor to Microsoft's discontinued Windows Subsystem for Android (WSA).

This is a real systems engineering project, not a UI mockup, prototype-only application, or documentation-only exercise.

The ultimate objective is to run real Android applications inside Windows with deep desktop integration.

---

# 1. PROJECT IDENTITY

Official name: BIFRÖST

Technical identifier: bifrost

Product description:

"An open-source Android subsystem for Windows, powered by Rust."

Primary tagline:

"A Bridge Between Worlds."

Secondary tagline:

"Two Worlds. One Experience."

## Identity and naming conventions

Use Norse mythology as inspiration for the project's internal component names.

These names must have meaningful technical responsibilities.

| Name | Responsibility |
|---|---|
| BIFRÖST | Entire platform |
| Bifrost Core | Shared system core and architecture |
| Heimdall | Background daemon and runtime supervisor |
| Bridge | Windows ↔ Android communication layer |
| Midgard | Android runtime environment |
| Mimir | Command-line interface |
| Valhalla | Android application management |

Do not create unnecessary mythology-inspired names for ordinary libraries or technical abstractions.

Technical clarity must always take priority over naming.

Use lowercase `bifrost` in repository names, package identifiers, executable names and technical namespaces.

Use `BIFRÖST` in branding and product presentation.

---

# 2. CORE VISION

Build a platform that allows users to run Android applications on Windows 11 with an experience as close as possible to native Windows applications.

Users must eventually be able to:

- Install and uninstall Android applications.
- Launch Android applications from Windows.
- Run applications in individual desktop windows whenever technically possible.
- Share files between Windows and Android.
- Share clipboard contents.
- Manage Android processes and services.
- Configure memory and CPU limits.
- Configure screen resolution and DPI.
- Access Android through ADB.
- Receive Android notifications in Windows.
- Manage Android runtime instances.
- Update Android independently from the Windows application.
- Inspect logs and diagnose problems.
- Recover the runtime after unexpected failures.

The platform must prioritize performance, security, compatibility, maintainability and transparency.

Do not simply recreate a traditional Android emulator.

The goal is a real Android subsystem with deep Windows integration.

---

# 3. TECHNOLOGY REQUIREMENTS

## Primary language: Rust

Rust must be the primary implementation language.

Use Rust for:

- Core architecture.
- Runtime lifecycle management.
- Process supervision.
- Virtual machine orchestration.
- IPC.
- Runtime configuration.
- Filesystem integration.
- ADB orchestration.
- Logging and diagnostics.
- Update management.
- Security boundaries.
- Background services.

Preferred ecosystem:

- Tokio for asynchronous execution.
- Serde for serialization.
- Tracing for structured logging.
- Thiserror and Anyhow for error handling.
- Clap for CLI interfaces.
- Tonic for gRPC where appropriate.
- SQLx or SQLite when persistent structured storage is necessary.
- Windows crate for Windows APIs.
- Cargo workspaces for modular architecture.

Avoid unnecessary dependencies.

Do not use Rust merely as a wrapper around an application implemented entirely in another language.

## Windows integration

Primary target: Windows 11 x64.

Investigate:

- Windows Hypervisor Platform (WHPX).
- Hyper-V integration.
- Windows networking APIs.
- Windows filesystem APIs.
- Windows notification APIs.
- Windows process and service management.
- Windows graphics integration.

Document OS edition, firmware, virtualization and feature requirements.

Do not assume that every Windows edition supports every required API.

## Android foundation

Use Android Open Source Project (AOSP) as the preferred foundation.

Investigate:

- AOSP x86_64 builds.
- Android 16 or a newer maintained compatible release.
- Kernel requirements.
- Android init.
- Binder IPC.
- Hardware Abstraction Layers.
- Graphics acceleration.
- Audio.
- Networking.
- Storage.
- ADB.
- Android package management.

Evaluate existing projects such as:

- Android-x86.
- Bliss OS.
- Waydroid.
- Other maintained Android-on-PC implementations.

Reuse existing technology whenever technically and legally appropriate.

Do not reinvent Android unnecessarily.

Do not redistribute proprietary Google applications or Google Play Services without appropriate authorization.

ARM application compatibility must be researched independently.

Never claim ARM compatibility without actual implementation and testing.

---

# 4. SYSTEM ARCHITECTURE

Build a modular architecture using Rust and Cargo workspaces.

## Bifrost Core

The central architectural layer.

Responsibilities:

- Shared domain models.
- Runtime states.
- Configuration.
- Error types.
- Shared interfaces.
- System capabilities.
- Version management.

## Heimdall

The main background service and supervisor.

Responsibilities:

- Start and stop Midgard.
- Monitor runtime health.
- Restart failed components.
- Manage runtime lifecycle.
- Coordinate Bridge.
- Manage system resources.
- Expose authenticated local IPC endpoints.
- Maintain system state.

Heimdall must not contain all the business logic of the platform.

Keep responsibilities modular.

## Midgard

The Android runtime environment.

Responsibilities:

- Android boot.
- Runtime lifecycle.
- Virtual machine integration.
- Virtual disk management.
- Android system configuration.
- Runtime health reporting.

Midgard must execute a real Android environment.

## Bridge

The communication layer between Windows and Android.

Responsibilities:

- ADB integration.
- File sharing.
- Clipboard synchronization.
- IPC.
- RPC.
- Runtime events.
- Notification transport.
- Networking coordination.

Define explicit interfaces and communication contracts.

## Mimir

The administrative command-line interface.

Suggested commands:

bifrost status
bifrost doctor
bifrost runtime start
bifrost runtime stop
bifrost runtime restart
bifrost devices
bifrost apps list
bifrost install application.apk
bifrost uninstall package.name
bifrost logs

Commands that are not implemented must explicitly report that they are unsupported.

Never simulate successful operations.

## Valhalla

The Android application management system.

Responsibilities:

- Installed application discovery.
- APK installation.
- APK removal.
- Package information.
- Application launching.
- Application metadata.
- Application lifecycle integration.

Valhalla must use Android's actual package management mechanisms.

## Desktop application

Develop a Windows desktop application with a Rust backend.

Evaluate Tauri 2 and Windows-native frontend alternatives.

Prefer a native Windows experience.

The frontend must never own critical runtime logic.

---

# 5. USER EXPERIENCE

The desktop application should use Windows 11 Fluent Design principles.

Required sections:

## Dashboard

- Runtime status.
- Android version.
- CPU usage.
- RAM usage.
- Storage usage.
- Running applications.
- Quick actions.

## Application manager

- Installed applications.
- Package names.
- Versions.
- APK installation.
- APK uninstallation.
- Application launching.

## Runtime configuration

- CPU allocation.
- RAM allocation.
- Storage allocation.
- Resolution.
- DPI.
- Networking.
- Developer options.

## Diagnostics

- Runtime logs.
- ADB status.
- Virtualization status.
- GPU information.
- System compatibility.
- Diagnostic reports.

## Settings

- Startup behavior.
- Automatic updates.
- Runtime versions.
- Storage location.
- Logging level.
- Resource management.

Design requirements:

- Fluent Design.
- Dark and light themes.
- Minimalist layouts.
- Consistent typography.
- Responsive navigation.
- Subtle animations.
- Accessible components.
- No advertisements.
- No unnecessary visual complexity.

The application should feel like a native part of Windows 11.

---

# 6. ADVANCED TECHNICAL CHALLENGES

Prioritize research and engineering around the following problems.

## Graphics

Investigate how Android surfaces can be rendered in independent Windows desktop windows.

Research:

- Android SurfaceFlinger.
- Virtual displays.
- DirectX.
- Vulkan.
- GPU virtualization.
- Graphics passthrough.
- Frame transport.
- Composition APIs.

Do not assume that booting Android inside a VM automatically provides independent application windows.

## Architecture compatibility

Investigate:

- x86_64 Android applications.
- ARM64 application compatibility.
- Binary translation options.
- Native libraries.
- ABI limitations.

## Windows integration

Investigate:

- Independent application windows.
- Windows notifications.
- Shared clipboard.
- Shared directories.
- Keyboard and mouse input.
- Touchscreen integration.
- Audio routing.

## Virtualization

Investigate:

- WHPX.
- Hyper-V.
- Existing QEMU-based implementations.
- Kernel compatibility.
- Virtual networking.
- Resource allocation.

Document all technical limitations.

---

# 7. SECURITY REQUIREMENTS

These requirements are mandatory.

1. Never execute untrusted APK installation scripts.
2. Validate filesystem paths.
3. Prevent directory traversal.
4. Do not expose ADB ports to public networks by default.
5. Avoid unnecessary administrator privileges.
6. Authenticate local IPC endpoints.
7. Validate incoming IPC messages.
8. Verify downloaded runtime images.
9. Use cryptographic hashes and signatures for updates.
10. Keep Android user data persistent across compatible runtime upgrades.
11. Require explicit confirmation for destructive operations.
12. Document unsafe Rust blocks.
13. Avoid unsafe Rust wherever possible.
14. Maintain a security audit trail.

Never sacrifice security for convenience.

---

# 8. DEVELOPMENT METHODOLOGY

Do not attempt to implement the entire project in one response.

Work incrementally.

Before implementation:

1. Inspect the repository.
2. Inspect the development environment.
3. Identify the current Windows version.
4. Identify available virtualization capabilities.
5. Research existing implementations.
6. Evaluate architectural feasibility.
7. Identify risks.
8. Write a technical implementation plan.

Maintain:

- README.md
- ARCHITECTURE.md
- ROADMAP.md
- CONTRIBUTING.md
- SECURITY.md
- CHANGELOG.md
- docs/TECHNICAL_DECISIONS.md

Maintain Architecture Decision Records (ADRs).

Every milestone must include:

- Working code.
- Tests.
- Documentation.
- Build verification.
- Known limitations.
- Next steps.

Run regularly:

cargo fmt
cargo clippy
cargo test

Never report tests as passing unless they were actually executed.

---

# 9. IMPLEMENTATION ROADMAP

## MILESTONE 0 — Research and feasibility

Determine whether the proposed architecture is technically viable.

Investigate:

- AOSP compatibility with Windows virtualization.
- WHPX capabilities.
- VM implementation options.
- GPU acceleration.
- Independent Android application windows.
- Android display integration.
- Existing open-source foundations.
- Licensing requirements.

Deliver a feasibility report.

Do not start implementing the entire runtime before understanding these constraints.

## MILESTONE 1 — Bifrost Core

Create the Cargo workspace.

Implement:

- bifrost-core.
- Mimir CLI foundation.
- Configuration system.
- Structured logging.
- Error handling.
- Runtime state model.
- Unit tests.

## MILESTONE 2 — Windows compatibility

Implement:

bifrost doctor

Detect:

- Windows version.
- CPU architecture.
- Virtualization.
- Hypervisor availability.
- Memory.
- Storage.
- GPU information.

Produce a useful compatibility report.

## MILESTONE 3 — Midgard prototype

Implement the smallest functional Android VM prototype.

Success criteria:

A real Android environment boots inside Windows.

Implement:

- VM configuration.
- Virtual disks.
- Android boot.
- Console access.
- Shutdown.
- Runtime logs.

## MILESTONE 4 — Heimdall and Bridge

Implement:

- Background daemon.
- Runtime supervision.
- ADB communication.
- Device discovery.
- Runtime health checks.
- IPC.

## MILESTONE 5 — Valhalla

Implement:

- APK installation.
- Package discovery.
- Application launching.
- Uninstallation.
- Application metadata.

## MILESTONE 6 — Windows integration

Implement:

- Shared directories.
- Clipboard.
- Networking.
- Notifications.
- Application process management.

## MILESTONE 7 — Desktop application

Develop the BIFRÖST Windows application.

Integrate all previously implemented components.

## MILESTONE 8 — Distribution

Implement:

- Installer.
- Runtime image management.
- Updates.
- Recovery.
- Uninstallation.
- Upgrade compatibility.

---

# 10. REPOSITORY STRUCTURE

Start with a clean Cargo workspace.

Suggested structure:

bifrost/
├── Cargo.toml
├── README.md
├── ARCHITECTURE.md
├── ROADMAP.md
├── CONTRIBUTING.md
├── SECURITY.md
├── CHANGELOG.md
├── crates/
│   ├── bifrost-core/
│   ├── bifrost-runtime/
│   ├── bifrost-hypervisor/
│   ├── bifrost-bridge/
│   ├── bifrost-process/
│   ├── bifrost-network/
│   ├── bifrost-storage/
│   ├── heimdall/
│   └── mimir/
├── apps/
│   └── bifrost-ui/
├── android/
│   └── midgard/
├── docs/
│   ├── decisions/
│   ├── research/
│   └── development/
└── tests/

Adapt this structure when technical research demonstrates a better architecture.

Do not create empty crates just to fill the directory structure.

---

# 11. CLAUDE CODE PROJECT MANAGEMENT

Create and maintain a CLAUDE.md file.

It must contain:

- Project identity.
- Technical architecture.
- Naming conventions.
- Rust coding standards.
- Security requirements.
- Build commands.
- Testing requirements.
- Current milestone.
- Known limitations.
- Architectural decisions.

Update CLAUDE.md when important project decisions change.

Keep it concise and useful.

Do not duplicate the entire documentation.

---

# 12. ENGINEERING EXPECTATIONS

Think like the lead engineer of a long-term open-source systems project.

Challenge incorrect assumptions.

Research before implementing low-level functionality.

Prefer working vertical slices.

Do not create fake features.

Do not introduce unnecessary abstractions.

Maintain architectural consistency.

Prioritize measurable performance.

Keep documentation updated.

Clearly distinguish between:

- Implemented.
- Partially implemented.
- Planned.
- Experimental.
- Unsupported.

A successful Rust build does not mean the Android subsystem works.

The ultimate goal is real Android application execution inside Windows.

---

# 13. FIRST TASK

Start with MILESTONE 0.

Inspect the repository and development environment.

Research the feasibility of BIFRÖST as a modern Android subsystem for Windows.

Compare:

- AOSP foundations.
- Existing Android-on-PC distributions.
- Hypervisor technologies.
- GPU integration approaches.
- Display and window integration strategies.
- ARM compatibility.
- Licensing and distribution.

Produce:

1. Technical feasibility report.
2. Recommended architecture.
3. Architecture diagram.
4. Main technical risks.
5. Development roadmap.
6. First small implementation milestone.

Do not generate the entire application immediately.

Do not make major architectural commitments without evidence.

Begin by understanding the problem.

Your objective is to build BIFRÖST into a real, reliable, performant and maintainable open-source platform.

**BIFRÖST — A Bridge Between Worlds.**