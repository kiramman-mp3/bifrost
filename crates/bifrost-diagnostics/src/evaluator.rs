//! Pure compatibility evaluation engine for `bifrost doctor`.

use bifrost_core::BifrostConfig;

use crate::model::{
    CheckStatus, CpuInfo, DiagnosticCheck, DoctorReport, GpuAdapterInfo, HostToolingInfo,
    MemoryInfo, OsInfo, OverallReadiness, StorageInfo, SystemSnapshot, WhpxInfo,
};

const BYTES_PER_MIB: u64 = 1024 * 1024;
const BYTES_PER_GIB: u64 = 1024 * 1024 * 1024;
const MIN_WINDOWS_11_BUILD: u32 = 22_000;
const MIN_RAM_GIB: u64 = 8;
const RECOMMENDED_RAM_GIB: u64 = 16;
const HOST_RAM_HEADROOM_MIB: u64 = 4096;
const MIN_STORAGE_GIB: u64 = 20;
const STORAGE_HEADROOM_GIB: u64 = 10;

/// Evaluates a raw host [`SystemSnapshot`] against [`BifrostConfig`] requirements.
#[must_use]
pub fn evaluate_snapshot(snapshot: SystemSnapshot, config: &BifrostConfig) -> DoctorReport {
    let checks = vec![
        check_os(&snapshot.os),
        check_cpu(&snapshot.cpu),
        check_virtualization(&snapshot.cpu),
        check_whpx(&snapshot.whpx),
        check_memory(&snapshot.memory, config),
        check_storage(&snapshot.storage, config),
        check_gpu(&snapshot.gpus),
        check_tooling(&snapshot.tooling),
    ];

    let pass_count = checks
        .iter()
        .filter(|c| c.status == CheckStatus::Pass)
        .count();
    let warn_count = checks
        .iter()
        .filter(|c| c.status == CheckStatus::Warn)
        .count();
    let fail_count = checks
        .iter()
        .filter(|c| c.status == CheckStatus::Fail)
        .count();

    let overall = if fail_count > 0 {
        OverallReadiness::NotReady
    } else if warn_count > 0 {
        OverallReadiness::ReadyWithWarnings
    } else {
        OverallReadiness::Ready
    };

    DoctorReport {
        target_platform: "Windows 11 x64 (Home / Pro / Enterprise)".to_string(),
        snapshot,
        checks,
        overall,
        pass_count,
        warn_count,
        fail_count,
    }
}

fn check_os(os: &OsInfo) -> DiagnosticCheck {
    let edition_label = human_edition(&os.edition_id);
    let version_str = if os.display_version.is_empty() {
        format!("{}.{}.{}", os.major, os.minor, os.build)
    } else {
        format!(
            "{} ({}.{}.{}.{})",
            os.display_version, os.major, os.minor, os.build, os.ubr
        )
    };

    if os.is_windows_11 && os.build >= MIN_WINDOWS_11_BUILD {
        let note = if os.edition_id.eq_ignore_ascii_case("Core") {
            "Windows 11 Home detected; fully supported via Windows Hypervisor Platform (WHPX)."
        } else {
            "Supported Windows 11 edition detected."
        };
        DiagnosticCheck {
            id: "os".to_string(),
            name: "Operating System".to_string(),
            status: CheckStatus::Pass,
            summary: format!("Windows 11 {edition_label} — {version_str}"),
            detail: note.to_string(),
            remediation: None,
        }
    } else {
        DiagnosticCheck {
            id: "os".to_string(),
            name: "Operating System".to_string(),
            status: CheckStatus::Fail,
            summary: format!("{} (build {})", os.product_name, os.build),
            detail: format!(
                "BIFRÖST targets Windows 11 x64 (build >= {MIN_WINDOWS_11_BUILD}); older or non-Windows operating systems are unsupported."
            ),
            remediation: Some("Upgrade to Windows 11 64-bit (Home, Pro, or Enterprise).".to_string()),
        }
    }
}

fn check_cpu(cpu: &CpuInfo) -> DiagnosticCheck {
    if cpu.architecture != "x86_64" {
        return DiagnosticCheck {
            id: "cpu".to_string(),
            name: "CPU Architecture".to_string(),
            status: CheckStatus::Fail,
            summary: format!("{} ({} logical cores)", cpu.architecture, cpu.logical_cores),
            detail: format!(
                "Detected CPU '{}'. BIFRÖST currently targets x86_64 hosts only.",
                cpu.brand
            ),
            remediation: Some("Run BIFRÖST on an x86_64 Windows 11 PC.".to_string()),
        };
    }

    if cpu.logical_cores >= 4 {
        DiagnosticCheck {
            id: "cpu".to_string(),
            name: "CPU Architecture".to_string(),
            status: CheckStatus::Pass,
            summary: format!(
                "{} — x86_64 ({} logical cores)",
                cpu.brand, cpu.logical_cores
            ),
            detail: "x86_64 host processor meets core count recommendations.".to_string(),
            remediation: None,
        }
    } else if cpu.logical_cores >= 2 {
        DiagnosticCheck {
            id: "cpu".to_string(),
            name: "CPU Architecture".to_string(),
            status: CheckStatus::Warn,
            summary: format!(
                "{} — x86_64 ({} logical cores)",
                cpu.brand, cpu.logical_cores
            ),
            detail:
                "At least 4 logical CPU cores are recommended for smooth Android VM performance."
                    .to_string(),
            remediation: Some("Reduce `runtime.cpu_cores` in bifrost.toml to 1 or 2.".to_string()),
        }
    } else {
        DiagnosticCheck {
            id: "cpu".to_string(),
            name: "CPU Architecture".to_string(),
            status: CheckStatus::Fail,
            summary: format!("x86_64 ({} logical cores)", cpu.logical_cores),
            detail: "Insufficient CPU cores to run an Android virtual machine.".to_string(),
            remediation: Some(
                "Use a host with at least 2 logical CPU cores (4+ recommended).".to_string(),
            ),
        }
    }
}

fn check_virtualization(cpu: &CpuInfo) -> DiagnosticCheck {
    // When Windows Hyper-V / Virtual Machine Platform / VBS is already active,
    // Windows hides raw VMX/SVM and firmware flags from the root partition.
    // Detecting CPUID hypervisor leaf "Microsoft Hv" prevents false negatives.
    if cpu.hypervisor_present && cpu.hypervisor_vendor.as_deref() == Some("Microsoft Hv") {
        let extra = if !cpu.firmware_virt_reported {
            " (Windows hides raw firmware virtualization flags while the Windows hypervisor is active; this is normal.)"
        } else {
            ""
        };
        return DiagnosticCheck {
            id: "virtualization".to_string(),
            name: "Hardware Virtualization".to_string(),
            status: CheckStatus::Pass,
            summary: "Active under Windows Hypervisor (Microsoft Hv)".to_string(),
            detail: format!("Hardware-assisted virtualization is enabled and active.{extra}"),
            remediation: None,
        };
    }

    if cpu.firmware_virt_reported || cpu.vmx_or_svm_exposed {
        DiagnosticCheck {
            id: "virtualization".to_string(),
            name: "Hardware Virtualization".to_string(),
            status: CheckStatus::Pass,
            summary: "VT-x / AMD-V available in firmware".to_string(),
            detail: "CPU reports hardware-assisted virtualization capability.".to_string(),
            remediation: None,
        }
    } else if cpu.hypervisor_present {
        let vendor = cpu
            .hypervisor_vendor
            .as_deref()
            .unwrap_or("unknown hypervisor");
        DiagnosticCheck {
            id: "virtualization".to_string(),
            name: "Hardware Virtualization".to_string(),
            status: CheckStatus::Warn,
            summary: format!("Running inside third-party hypervisor ({vendor})"),
            detail: "Nested virtualization under non-Microsoft hypervisors may not expose WHPX reliably."
                .to_string(),
            remediation: Some(
                "Run BIFRÖST directly on bare-metal Windows 11 or enable nested virtualization if supported."
                    .to_string(),
            ),
        }
    } else {
        DiagnosticCheck {
            id: "virtualization".to_string(),
            name: "Hardware Virtualization".to_string(),
            status: CheckStatus::Fail,
            summary: "Hardware virtualization not detected".to_string(),
            detail: "Neither Intel VT-x/AMD-V nor an active Windows hypervisor was detected."
                .to_string(),
            remediation: Some(
                "Enable Intel VT-x / Intel Virtualization Technology or AMD-V / SVM in UEFI/BIOS settings."
                    .to_string(),
            ),
        }
    }
}

fn check_whpx(whpx: &WhpxInfo) -> DiagnosticCheck {
    if whpx.api_available
        && whpx.hypervisor_present
        && whpx.partition_create_supported
        && !whpx.feature_likely_disabled
    {
        DiagnosticCheck {
            id: "whpx".to_string(),
            name: "Windows Hypervisor Platform (WHPX)".to_string(),
            status: CheckStatus::Pass,
            summary: "WHPX enabled and partition creation verified".to_string(),
            detail:
                "WinHvPlatform.dll loaded, WHvGetCapability confirmed hypervisor presence, and WHvCreatePartition succeeded."
                    .to_string(),
            remediation: None,
        }
    } else if whpx.dll_present {
        let hr_note = whpx
            .partition_hresult
            .or(whpx.hresult)
            .map(|hr| format!(" (HRESULT 0x{:08X})", hr as u32))
            .unwrap_or_default();
        let summary = if whpx.hypervisor_present && !whpx.partition_create_supported {
            "Hypervisor present, but WHPX partition creation failed (feature likely disabled)"
        } else {
            "WHPX feature is disabled or unavailable"
        };
        DiagnosticCheck {
            id: "whpx".to_string(),
            name: "Windows Hypervisor Platform (WHPX)".to_string(),
            status: CheckStatus::Fail,
            summary: summary.to_string(),
            detail: format!(
                "WinHvPlatform.dll is present, but creating a WHPX VM partition failed{hr_note}."
            ),
            remediation: Some(
                "Run in an elevated PowerShell: DISM /Online /Enable-Feature /FeatureName:HypervisorPlatform /All — then reboot Windows."
                    .to_string(),
            ),
        }
    } else {
        DiagnosticCheck {
            id: "whpx".to_string(),
            name: "Windows Hypervisor Platform (WHPX)".to_string(),
            status: CheckStatus::Fail,
            summary: "WinHvPlatform.dll not found".to_string(),
            detail: "The Windows Hypervisor Platform API library is missing on this host."
                .to_string(),
            remediation: Some(
                "Ensure you are running Windows 11 x64 and enable 'Windows Hypervisor Platform' in Windows Features."
                    .to_string(),
            ),
        }
    }
}

fn check_memory(memory: &MemoryInfo, config: &BifrostConfig) -> DiagnosticCheck {
    let total_gib = memory.total_bytes as f64 / BYTES_PER_GIB as f64;
    let avail_gib = memory.available_bytes as f64 / BYTES_PER_GIB as f64;
    let total_mib = memory.total_bytes / BYTES_PER_MIB;

    if memory.total_bytes < MIN_RAM_GIB * BYTES_PER_GIB {
        return DiagnosticCheck {
            id: "memory".to_string(),
            name: "System Memory (RAM)".to_string(),
            status: CheckStatus::Fail,
            summary: format!("{total_gib:.2} GB total ({avail_gib:.2} GB available)"),
            detail: format!(
                "Minimum {MIN_RAM_GIB} GB host RAM is required ({RECOMMENDED_RAM_GIB} GB recommended)."
            ),
            remediation: Some("Upgrade system memory to at least 8 GB (16 GB recommended).".to_string()),
        };
    }

    if config.runtime.memory_mb + HOST_RAM_HEADROOM_MIB > total_mib {
        return DiagnosticCheck {
            id: "memory".to_string(),
            name: "System Memory (RAM)".to_string(),
            status: CheckStatus::Warn,
            summary: format!("{total_gib:.2} GB total ({avail_gib:.2} GB available)"),
            detail: format!(
                "Configured `runtime.memory_mb` ({} MB) leaves less than {} MB headroom for Windows.",
                config.runtime.memory_mb, HOST_RAM_HEADROOM_MIB
            ),
            remediation: Some(
                "Lower `runtime.memory_mb` in bifrost.toml to leave at least 4 GB for Windows."
                    .to_string(),
            ),
        };
    }

    if memory.total_bytes < RECOMMENDED_RAM_GIB * BYTES_PER_GIB {
        return DiagnosticCheck {
            id: "memory".to_string(),
            name: "System Memory (RAM)".to_string(),
            status: CheckStatus::Warn,
            summary: format!("{total_gib:.2} GB total ({avail_gib:.2} GB available)"),
            detail: format!(
                "Meets minimum {MIN_RAM_GIB} GB requirement, but {RECOMMENDED_RAM_GIB} GB is recommended for Android 15/16."
            ),
            remediation: Some("Keep `runtime.memory_mb` at 4096 MB or lower.".to_string()),
        };
    }

    DiagnosticCheck {
        id: "memory".to_string(),
        name: "System Memory (RAM)".to_string(),
        status: CheckStatus::Pass,
        summary: format!("{total_gib:.2} GB total ({avail_gib:.2} GB available)"),
        detail: format!(
            "Sufficient host memory for configured Midgard allocation ({} MB).",
            config.runtime.memory_mb
        ),
        remediation: None,
    }
}

fn check_storage(storage: &StorageInfo, config: &BifrostConfig) -> DiagnosticCheck {
    let free_gib = storage.available_bytes as f64 / BYTES_PER_GIB as f64;
    let total_gib = storage.total_bytes as f64 / BYTES_PER_GIB as f64;
    let required_gib = config.runtime.storage_gb.max(MIN_STORAGE_GIB);

    if storage.total_bytes == 0 {
        return DiagnosticCheck {
            id: "storage".to_string(),
            name: "Storage Capacity".to_string(),
            status: CheckStatus::Warn,
            summary: format!(
                "Unable to query free space for '{}'",
                storage.target_path.display()
            ),
            detail:
                "Storage query returned 0 bytes; verify permissions on the configured data path."
                    .to_string(),
            remediation: Some(
                "Verify `runtime.data_dir` points to an accessible local NTFS drive.".to_string(),
            ),
        };
    }

    if storage.available_bytes < required_gib * BYTES_PER_GIB {
        DiagnosticCheck {
            id: "storage".to_string(),
            name: "Storage Capacity".to_string(),
            status: CheckStatus::Fail,
            summary: format!(
                "{free_gib:.2} GB free / {total_gib:.2} GB total at '{}'",
                storage.target_path.display()
            ),
            detail: format!(
                "At least {required_gib} GB free space is needed for Midgard images and configured userdata disk ({} GB).",
                config.runtime.storage_gb
            ),
            remediation: Some(
                "Free disk space or set `runtime.data_dir` in bifrost.toml to another local drive."
                    .to_string(),
            ),
        }
    } else if storage.available_bytes < (required_gib + STORAGE_HEADROOM_GIB) * BYTES_PER_GIB {
        DiagnosticCheck {
            id: "storage".to_string(),
            name: "Storage Capacity".to_string(),
            status: CheckStatus::Warn,
            summary: format!(
                "{free_gib:.2} GB free / {total_gib:.2} GB total at '{}'",
                storage.target_path.display()
            ),
            detail: format!(
                "Meets minimum {required_gib} GB requirement, but less than {STORAGE_HEADROOM_GIB} GB extra headroom remains for image upgrades."
            ),
            remediation: Some("Consider freeing additional disk space before Midgard image updates.".to_string()),
        }
    } else {
        DiagnosticCheck {
            id: "storage".to_string(),
            name: "Storage Capacity".to_string(),
            status: CheckStatus::Pass,
            summary: format!(
                "{free_gib:.2} GB free / {total_gib:.2} GB total at '{}'",
                storage.target_path.display()
            ),
            detail: format!(
                "Ample free disk space for Midgard images and configured {} GB virtual storage.",
                config.runtime.storage_gb
            ),
            remediation: None,
        }
    }
}

fn check_gpu(gpus: &[GpuAdapterInfo]) -> DiagnosticCheck {
    let hw_gpus: Vec<&GpuAdapterInfo> = gpus.iter().filter(|g| !g.is_software).collect();

    if let Some(primary) = hw_gpus.first() {
        let driver = primary
            .driver_version
            .as_deref()
            .unwrap_or("driver version unknown");
        let vram_gib = primary.dedicated_vram_bytes as f64 / BYTES_PER_GIB as f64;
        let shared_gib = primary.shared_system_memory_bytes as f64 / BYTES_PER_GIB as f64;
        let extra_count = if hw_gpus.len() > 1 {
            format!(" (+{} additional hardware GPU)", hw_gpus.len() - 1)
        } else {
            String::new()
        };

        DiagnosticCheck {
            id: "gpu".to_string(),
            name: "Graphics Adapter (GPU)".to_string(),
            status: CheckStatus::Pass,
            summary: format!("{} ({}, driver {}){extra_count}", primary.name, primary.vendor_name, driver),
            detail: format!(
                "Dedicated VRAM: {vram_gib:.2} GB, Shared system memory: {shared_gib:.2} GB. Suitable candidate for virtio-gpu + gfxstream acceleration."
            ),
            remediation: None,
        }
    } else if !gpus.is_empty() {
        DiagnosticCheck {
            id: "gpu".to_string(),
            name: "Graphics Adapter (GPU)".to_string(),
            status: CheckStatus::Warn,
            summary: format!("Software adapter only ({})", gpus[0].name),
            detail: "No hardware DirectX/Vulkan GPU detected; Midgard will fall back to SwiftShader CPU rendering."
                .to_string(),
            remediation: Some("Install official Intel, AMD, or NVIDIA WDDM graphics drivers.".to_string()),
        }
    } else {
        DiagnosticCheck {
            id: "gpu".to_string(),
            name: "Graphics Adapter (GPU)".to_string(),
            status: CheckStatus::Warn,
            summary: "No DXGI graphics adapters reported".to_string(),
            detail: "Unable to enumerate host GPUs via DXGI; hardware graphics acceleration cannot be confirmed."
                .to_string(),
            remediation: Some("Verify host graphics drivers are installed and up to date.".to_string()),
        }
    }
}

fn check_tooling(tooling: &HostToolingInfo) -> DiagnosticCheck {
    let mut missing: Vec<&str> = Vec::new();
    let mut actions: Vec<&str> = Vec::new();

    if tooling.qemu_path.is_none() {
        missing.push("QEMU (qemu-system-x86_64)");
        actions.push("install QEMU: `winget install SoftwareFreedomConservancy.QEMU`");
    }
    if tooling.adb_path.is_none() {
        missing.push("Android Platform-Tools (adb)");
        actions.push("install ADB: `winget install Google.PlatformTools`");
    }
    if tooling.scrcpy_path.is_none() {
        missing.push("scrcpy (per-app Windows 11 windows)");
        actions.push("install scrcpy: `winget install Genymobile.scrcpy`");
    }
    if tooling.registered_images_count == 0 {
        missing.push("registered Midgard Android image");
        actions.push(
            "register an image: `bifrost images register --manifest <path-to-manifest.json>`",
        );
    }

    let edk2_note = if tooling.edk2_firmware_path.is_some() {
        "EDK2 UEFI firmware found"
    } else {
        "EDK2 UEFI firmware not detected"
    };
    let active_img = tooling.active_image_id.as_deref().unwrap_or("none");

    if missing.is_empty() {
        DiagnosticCheck {
            id: "tooling".to_string(),
            name: "Host Runtime Tooling".to_string(),
            status: CheckStatus::Pass,
            summary: format!(
                "QEMU, ADB, scrcpy & {} Midgard image(s) ready (active: {active_img})",
                tooling.registered_images_count
            ),
            detail: format!(
                "All runtime prerequisites detected ({edk2_note}); zero-friction app window launch is ready."
            ),
            remediation: None,
        }
    } else {
        DiagnosticCheck {
            id: "tooling".to_string(),
            name: "Host Runtime Tooling".to_string(),
            status: CheckStatus::Warn,
            summary: format!("Missing: {}", missing.join(", ")),
            detail: format!(
                "QEMU: {}, ADB: {}, scrcpy: {}, Images: {} ({edk2_note}).",
                if tooling.qemu_path.is_some() {
                    "found"
                } else {
                    "missing"
                },
                if tooling.adb_path.is_some() {
                    "found"
                } else {
                    "missing"
                },
                if tooling.scrcpy_path.is_some() {
                    "found"
                } else {
                    "missing"
                },
                tooling.registered_images_count
            ),
            remediation: Some(actions.join("; ")),
        }
    }
}

fn human_edition(edition_id: &str) -> &str {
    match edition_id {
        "Core" | "CoreSingleLanguage" | "CoreCountrySpecific" => "Home",
        "Professional" | "ProfessionalWorkstation" => "Pro",
        "Enterprise" | "EnterpriseS" => "Enterprise",
        "Education" => "Education",
        other if !other.is_empty() => other,
        _ => "Unknown Edition",
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    fn baseline_win11_home_snapshot() -> SystemSnapshot {
        SystemSnapshot {
            os: OsInfo {
                product_name: "Windows 11 Home".to_string(),
                edition_id: "Core".to_string(),
                display_version: "25H2".to_string(),
                major: 10,
                minor: 0,
                build: 26200,
                ubr: 1000,
                is_windows_11: true,
            },
            cpu: CpuInfo {
                brand: "Intel(R) Core(TM) Ultra 9 285H".to_string(),
                architecture: "x86_64".to_string(),
                logical_cores: 16,
                vmx_or_svm_exposed: false,
                firmware_virt_reported: false,
                hypervisor_present: true,
                hypervisor_vendor: Some("Microsoft Hv".to_string()),
            },
            whpx: WhpxInfo {
                dll_present: true,
                api_available: true,
                hypervisor_present: true,
                partition_create_supported: true,
                hresult: Some(0),
                partition_hresult: Some(0),
                feature_likely_disabled: false,
            },
            memory: MemoryInfo {
                total_bytes: 32 * BYTES_PER_GIB,
                available_bytes: 20 * BYTES_PER_GIB,
            },
            storage: StorageInfo {
                target_path: PathBuf::from(r"C:\Users\dev\AppData\Local\bifrost"),
                total_bytes: 950 * BYTES_PER_GIB,
                available_bytes: 800 * BYTES_PER_GIB,
            },
            gpus: vec![GpuAdapterInfo {
                name: "Intel(R) Arc(TM) 140T GPU (16GB)".to_string(),
                vendor_id: 0x8086,
                vendor_name: "Intel".to_string(),
                dedicated_vram_bytes: 2 * BYTES_PER_GIB,
                shared_system_memory_bytes: 16 * BYTES_PER_GIB,
                driver_version: Some("32.0.101.8860".to_string()),
                is_software: false,
            }],
            tooling: HostToolingInfo {
                qemu_path: Some(PathBuf::from(
                    r"C:\Program Files\qemu\qemu-system-x86_64.exe",
                )),
                edk2_firmware_path: Some(PathBuf::from(
                    r"C:\Program Files\qemu\share\edk2-x86_64-code.fd",
                )),
                adb_path: Some(PathBuf::from(r"C:\Android\Sdk\platform-tools\adb.exe")),
                scrcpy_path: Some(PathBuf::from(r"C:\Programs\scrcpy\scrcpy.exe")),
                registered_images_count: 1,
                active_image_id: Some("lineage-24-virtio-x86_64".to_string()),
            },
        }
    }

    #[test]
    fn win11_home_with_active_hv_and_whpx_is_ready() {
        let report = evaluate_snapshot(baseline_win11_home_snapshot(), &BifrostConfig::default());
        assert_eq!(report.overall, OverallReadiness::Ready);
        assert_eq!(report.fail_count, 0);
        assert_eq!(report.warn_count, 0);
        assert_eq!(report.pass_count, 8);
    }

    #[test]
    fn missing_host_tooling_reports_warning_with_remediation() {
        let mut snap = baseline_win11_home_snapshot();
        snap.tooling.scrcpy_path = None;
        snap.tooling.registered_images_count = 0;

        let report = evaluate_snapshot(snap, &BifrostConfig::default());
        assert_eq!(report.overall, OverallReadiness::ReadyWithWarnings);
        let tooling_check = report.checks.iter().find(|c| c.id == "tooling").unwrap();
        assert_eq!(tooling_check.status, CheckStatus::Warn);
        let rem = tooling_check.remediation.as_deref().unwrap();
        assert!(rem.contains("Genymobile.scrcpy"));
        assert!(rem.contains("bifrost images register"));
    }

    #[test]
    fn disabled_whpx_reports_not_ready_with_dism_remediation() {
        let mut snap = baseline_win11_home_snapshot();
        snap.whpx.partition_create_supported = false;
        snap.whpx.partition_hresult = Some(-2143878910); // 0x80370102
        snap.whpx.feature_likely_disabled = true;

        let report = evaluate_snapshot(snap, &BifrostConfig::default());
        assert_eq!(report.overall, OverallReadiness::NotReady);
        let whpx_check = report.checks.iter().find(|c| c.id == "whpx").unwrap();
        assert_eq!(whpx_check.status, CheckStatus::Fail);
        assert!(whpx_check
            .remediation
            .as_deref()
            .unwrap()
            .contains("HypervisorPlatform"));
    }

    #[test]
    fn windows_10_fails_os_check() {
        let mut snap = baseline_win11_home_snapshot();
        snap.os.build = 19045;
        snap.os.is_windows_11 = false;
        snap.os.product_name = "Windows 10 Pro".to_string();

        let report = evaluate_snapshot(snap, &BifrostConfig::default());
        assert_eq!(report.overall, OverallReadiness::NotReady);
        let os_check = report.checks.iter().find(|c| c.id == "os").unwrap();
        assert_eq!(os_check.status, CheckStatus::Fail);
    }

    #[test]
    fn software_only_gpu_produces_warning_not_hard_failure() {
        let mut snap = baseline_win11_home_snapshot();
        snap.gpus = vec![GpuAdapterInfo {
            name: "Microsoft Basic Render Driver".to_string(),
            vendor_id: 0x1414,
            vendor_name: "Microsoft".to_string(),
            dedicated_vram_bytes: 0,
            shared_system_memory_bytes: 8 * BYTES_PER_GIB,
            driver_version: None,
            is_software: true,
        }];

        let report = evaluate_snapshot(snap, &BifrostConfig::default());
        assert_eq!(report.overall, OverallReadiness::ReadyWithWarnings);
        let gpu_check = report.checks.iter().find(|c| c.id == "gpu").unwrap();
        assert_eq!(gpu_check.status, CheckStatus::Warn);
    }
}
