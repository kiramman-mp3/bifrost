//! VMM-agnostic hypervisor abstraction (`HypervisorBackend`) and WHPX backends.

#![forbid(unsafe_code)]

pub mod backend;
pub mod qemu_whpx;
pub mod qmp;

pub use backend::{HypervisorBackend, VmLaunchInfo, VmSpec};
pub use qemu_whpx::{is_process_running, QemuWhpxBackend};
