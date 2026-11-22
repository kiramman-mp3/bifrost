//! Native Windows 11 compatibility diagnostics powering `bifrost doctor`.
//!
//! Host system probing is isolated in `probe_windows` with strictly audited
//! `// SAFETY:` comments on every FFI call, while compatibility evaluation and
//! report formatting remain 100% pure, safe, and unit-testable.

pub mod evaluator;
pub mod format;
pub mod model;

#[cfg(windows)]
mod probe_windows;

use std::path::Path;

use bifrost_core::BifrostConfig;

pub use evaluator::evaluate_snapshot;
pub use format::render_human_report;
pub use model::{
    CheckStatus, CpuInfo, DiagnosticCheck, DoctorReport, GpuAdapterInfo, MemoryInfo, OsInfo,
    OverallReadiness, StorageInfo, SystemSnapshot, WhpxInfo,
};

/// Probes the local host and evaluates its compatibility with BIFRÖST.
#[must_use]
pub fn run_doctor(config: &BifrostConfig, target_storage_path: &Path) -> DoctorReport {
    let snapshot = probe_host(target_storage_path);
    evaluate_snapshot(snapshot, config)
}

#[cfg(windows)]
fn probe_host(target_storage_path: &Path) -> SystemSnapshot {
    probe_windows::probe_system(target_storage_path)
}

#[cfg(not(windows))]
fn probe_host(target_storage_path: &Path) -> SystemSnapshot {
    SystemSnapshot {
        os: OsInfo {
            product_name: std::env::consts::OS.to_string(),
            edition_id: "Non-Windows".to_string(),
            display_version: String::new(),
            major: 0,
            minor: 0,
            build: 0,
            ubr: 0,
            is_windows_11: false,
        },
        cpu: CpuInfo {
            brand: "Unknown".to_string(),
            architecture: std::env::consts::ARCH.to_string(),
            logical_cores: std::thread::available_parallelism()
                .map(|n| n.get() as u32)
                .unwrap_or(0),
            vmx_or_svm_exposed: false,
            firmware_virt_reported: false,
            hypervisor_present: false,
            hypervisor_vendor: None,
        },
        whpx: WhpxInfo {
            dll_present: false,
            api_available: false,
            hypervisor_present: false,
            partition_create_supported: false,
            hresult: None,
            partition_hresult: None,
            feature_likely_disabled: false,
        },
        memory: MemoryInfo {
            total_bytes: 0,
            available_bytes: 0,
        },
        storage: StorageInfo {
            target_path: target_storage_path.to_path_buf(),
            total_bytes: 0,
            available_bytes: 0,
        },
        gpus: Vec::new(),
    }
}
