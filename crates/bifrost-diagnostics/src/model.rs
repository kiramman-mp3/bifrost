//! Serializable diagnostic data models for `bifrost doctor`.

use std::fmt;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// Outcome of an individual compatibility check.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CheckStatus {
    Pass,
    Warn,
    Fail,
}

impl fmt::Display for CheckStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Pass => f.write_str("PASS"),
            Self::Warn => f.write_str("WARN"),
            Self::Fail => f.write_str("FAIL"),
        }
    }
}

/// Aggregated host readiness verdict for running Midgard on this machine.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OverallReadiness {
    Ready,
    ReadyWithWarnings,
    NotReady,
}

impl fmt::Display for OverallReadiness {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Ready => f.write_str("READY"),
            Self::ReadyWithWarnings => f.write_str("READY WITH WARNINGS"),
            Self::NotReady => f.write_str("NOT READY"),
        }
    }
}

/// Result of a single compatibility rule evaluation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiagnosticCheck {
    pub id: String,
    pub name: String,
    pub status: CheckStatus,
    pub summary: String,
    pub detail: String,
    pub remediation: Option<String>,
}

/// Operating system version and edition details.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OsInfo {
    pub product_name: String,
    pub edition_id: String,
    pub display_version: String,
    pub major: u32,
    pub minor: u32,
    pub build: u32,
    pub ubr: u32,
    pub is_windows_11: bool,
}

/// Processor architecture, core topology, and virtualization visibility.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CpuInfo {
    pub brand: String,
    pub architecture: String,
    pub logical_cores: u32,
    pub vmx_or_svm_exposed: bool,
    pub firmware_virt_reported: bool,
    pub hypervisor_present: bool,
    pub hypervisor_vendor: Option<String>,
}

/// Windows Hypervisor Platform (WHPX) runtime availability.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WhpxInfo {
    pub dll_present: bool,
    pub api_available: bool,
    pub hypervisor_present: bool,
    pub partition_create_supported: bool,
    pub hresult: Option<i32>,
    pub partition_hresult: Option<i32>,
    pub feature_likely_disabled: bool,
}

/// Host physical RAM measurements.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemoryInfo {
    pub total_bytes: u64,
    pub available_bytes: u64,
}

/// Host storage measurements for the target BIFRÖST data volume.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StorageInfo {
    pub target_path: PathBuf,
    pub total_bytes: u64,
    pub available_bytes: u64,
}

/// Graphics adapter metadata discovered via DXGI.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GpuAdapterInfo {
    pub name: String,
    pub vendor_id: u32,
    pub vendor_name: String,
    pub dedicated_vram_bytes: u64,
    pub shared_system_memory_bytes: u64,
    pub driver_version: Option<String>,
    pub is_software: bool,
}

/// Host runtime tooling and registered Midgard image availability.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct HostToolingInfo {
    pub qemu_path: Option<PathBuf>,
    pub edk2_firmware_path: Option<PathBuf>,
    pub adb_path: Option<PathBuf>,
    pub scrcpy_path: Option<PathBuf>,
    pub registered_images_count: usize,
    pub active_image_id: Option<String>,
}

/// Raw snapshot collected from the host prior to compatibility evaluation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SystemSnapshot {
    pub os: OsInfo,
    pub cpu: CpuInfo,
    pub whpx: WhpxInfo,
    pub memory: MemoryInfo,
    pub storage: StorageInfo,
    pub gpus: Vec<GpuAdapterInfo>,
    pub tooling: HostToolingInfo,
}

/// Complete report emitted by `bifrost doctor`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DoctorReport {
    pub target_platform: String,
    pub snapshot: SystemSnapshot,
    pub checks: Vec<DiagnosticCheck>,
    pub overall: OverallReadiness,
    pub pass_count: usize,
    pub warn_count: usize,
    pub fail_count: usize,
}
