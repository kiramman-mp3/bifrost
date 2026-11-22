//! VMM-agnostic hypervisor abstraction (`HypervisorBackend`), Phase 1 QEMU-WHPX,
//! and Target Evolution `crosvm`-WHPX + `gfxstream` backend (ADR-0002 / ADR-0003).

#![forbid(unsafe_code)]

pub mod backend;
pub mod crosvm_whpx;
pub mod qemu_whpx;
pub mod qmp;

pub use backend::{HypervisorBackend, VmLaunchInfo, VmSpec};
pub use crosvm_whpx::CrosvmWhpxBackend;
pub use qemu_whpx::{is_process_running, QemuWhpxBackend};
