//! VMM-agnostic hypervisor traits and launch specifications (ADR-0002).

use std::path::PathBuf;

use bifrost_core::{GraphicsMode, Result};
use bifrost_storage::{DiskFormat, VerifiedBundle};
use serde::{Deserialize, Serialize};

/// Complete validated specification for launching a Midgard Android VM.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VmSpec {
    pub instance_id: String,
    pub cpu_cores: u32,
    pub memory_mb: u64,
    pub bundle: VerifiedBundle,
    pub userdata_disk: PathBuf,
    pub userdata_format: DiskFormat,
    pub adb_bind_address: String,
    pub adb_port: u16,
    pub graphics_mode: GraphicsMode,
    pub headless: bool,
    pub interactive_console: bool,
    pub serial_log_path: PathBuf,
    pub vmm_stderr_path: PathBuf,
    pub qmp_port: u16,
}

/// Metadata returned after a hypervisor backend successfully spawns a VM process.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VmLaunchInfo {
    pub pid: u32,
    pub backend_name: String,
    pub command_line: Vec<String>,
    pub serial_log_path: PathBuf,
    pub vmm_stderr_path: PathBuf,
    pub qmp_endpoint: String,
    pub adb_endpoint: String,
}

/// Abstraction over concrete VMM implementations (Phase 1: QEMU-WHPX; Target: crosvm-WHPX).
pub trait HypervisorBackend: Send + Sync {
    /// Short identifier for this backend (e.g., `"qemu-whpx"`).
    fn name(&self) -> &'static str;

    /// Verifies that the underlying VMM binary/runtime is present and executable.
    fn locate_vmm(&self) -> Result<PathBuf>;

    /// Builds the deterministic argument vector for spawning a VM without executing it.
    fn build_args(&self, spec: &VmSpec) -> Result<Vec<String>>;

    /// Spawns a new Midgard VM instance (detached unless `interactive_console` is requested).
    fn spawn(&self, spec: &VmSpec) -> Result<VmLaunchInfo>;

    /// Requests a graceful ACPI shutdown and waits up to `timeout_secs` for the process to exit.
    fn shutdown_graceful(&self, qmp_endpoint: &str, pid: u32, timeout_secs: u64) -> Result<()>;

    /// Forcefully terminates the VM process by PID.
    fn terminate_force(&self, pid: u32) -> Result<()>;

    /// Checks whether a previously spawned VM PID is still alive.
    fn is_running(&self, pid: u32) -> bool;
}
