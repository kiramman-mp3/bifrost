//! Capability status and version metadata shared across BIFRÖST components.

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::{PRIMARY_TAGLINE, PRODUCT_NAME, TECHNICAL_ID, VERSION};

/// Explicit maturity classification mandated by BIFRÖST engineering standards.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FeatureStatus {
    Implemented,
    PartiallyImplemented,
    Planned,
    Experimental,
    Unsupported,
}

impl fmt::Display for FeatureStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let label = match self {
            Self::Implemented => "implemented",
            Self::PartiallyImplemented => "partially_implemented",
            Self::Planned => "planned",
            Self::Experimental => "experimental",
            Self::Unsupported => "unsupported",
        };
        f.write_str(label)
    }
}

/// Status descriptor for a named BIFRÖST subsystem.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ComponentCapability {
    pub name: String,
    pub status: FeatureStatus,
    pub summary: String,
}

/// Build and platform metadata emitted by `bifrost version`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VersionInfo {
    pub product: String,
    pub identifier: String,
    pub tagline: String,
    pub version: String,
    pub target_os: String,
    pub target_arch: String,
    pub current_milestone: String,
    pub capabilities: Vec<ComponentCapability>,
}

impl VersionInfo {
    /// Constructs current compile-time version and honest capability metadata.
    #[must_use]
    pub fn current() -> Self {
        Self {
            product: PRODUCT_NAME.to_string(),
            identifier: TECHNICAL_ID.to_string(),
            tagline: PRIMARY_TAGLINE.to_string(),
            version: VERSION.to_string(),
            target_os: std::env::consts::OS.to_string(),
            target_arch: std::env::consts::ARCH.to_string(),
            current_milestone: "Milestone 3 — Midgard Prototype (WHPX VM, Storage & Logs)"
                .to_string(),
            capabilities: vec![
                ComponentCapability {
                    name: "bifrost-core".to_string(),
                    status: FeatureStatus::Implemented,
                    summary: "Domain models, state machine, config, security path checks, logging"
                        .to_string(),
                },
                ComponentCapability {
                    name: "mimir".to_string(),
                    status: FeatureStatus::PartiallyImplemented,
                    summary:
                        "CLI active (version, status, config, doctor, runtime, logs); Bridge/Valhalla pending"
                            .to_string(),
                },
                ComponentCapability {
                    name: "windows-doctor".to_string(),
                    status: FeatureStatus::Implemented,
                    summary:
                        "Native Windows 11, virtualization, WHPX, RAM, storage, and GPU checks"
                            .to_string(),
                },
                ComponentCapability {
                    name: "midgard".to_string(),
                    status: FeatureStatus::Experimental,
                    summary:
                        "Verified image manifests, userdata disks, QEMU-WHPX launch, QMP shutdown & logs"
                            .to_string(),
                },
                ComponentCapability {
                    name: "heimdall-bridge".to_string(),
                    status: FeatureStatus::Planned,
                    summary: "Milestone 4 — Supervisor daemon, authenticated IPC, loopback ADB"
                        .to_string(),
                },
                ComponentCapability {
                    name: "valhalla".to_string(),
                    status: FeatureStatus::Planned,
                    summary: "Milestone 5 — Real Android APK & package management".to_string(),
                },
                ComponentCapability {
                    name: "arm64-translation".to_string(),
                    status: FeatureStatus::Unsupported,
                    summary: "Proprietary translators excluded; clean-room x86_64 baseline"
                        .to_string(),
                },
            ],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_info_reports_honest_arm64_unsupported_status() {
        let info = VersionInfo::current();
        assert_eq!(info.product, "BIFRÖST");
        assert_eq!(info.identifier, "bifrost");
        assert!(info
            .capabilities
            .iter()
            .any(|c| c.name == "arm64-translation" && c.status == FeatureStatus::Unsupported));
    }
}
