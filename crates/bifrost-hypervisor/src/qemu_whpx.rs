//! Phase 1 WHPX backend orchestrating `qemu-system-x86_64` over Windows Hypervisor Platform.

use std::fs::{self, OpenOptions};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use bifrost_core::{BifrostError, GraphicsMode, Result};
use bifrost_storage::{DiskMedia, VerifiedBootMedia};
use tracing::{info, warn};

use crate::backend::{HypervisorBackend, VmLaunchInfo, VmSpec};
use crate::qmp::send_qmp_command;

/// QEMU-over-WHPX implementation of [`HypervisorBackend`].
#[derive(Debug, Clone, Default)]
pub struct QemuWhpxBackend {
    explicit_binary: Option<PathBuf>,
}

impl QemuWhpxBackend {
    /// Creates a backend using automatic discovery (`BIFROST_QEMU_BIN`, `PATH`, standard dirs).
    #[must_use]
    pub const fn new() -> Self {
        Self {
            explicit_binary: None,
        }
    }

    /// Creates a backend pinned to an explicit `qemu-system-x86_64` executable path.
    #[must_use]
    pub fn with_binary(path: impl Into<PathBuf>) -> Self {
        Self {
            explicit_binary: Some(path.into()),
        }
    }
}

impl HypervisorBackend for QemuWhpxBackend {
    fn name(&self) -> &'static str {
        "qemu-whpx"
    }

    fn locate_vmm(&self) -> Result<PathBuf> {
        if let Some(ref explicit) = self.explicit_binary {
            if explicit.is_file() {
                return Ok(explicit.clone());
            }
            return Err(BifrostError::Hypervisor(format!(
                "configured QEMU binary '{}' was not found",
                explicit.display()
            )));
        }

        if let Ok(env_bin) = std::env::var("BIFROST_QEMU_BIN") {
            let candidate = PathBuf::from(env_bin);
            if candidate.is_file() {
                return Ok(candidate);
            }
        }

        if let Some(on_path) =
            find_on_path("qemu-system-x86_64.exe").or_else(|| find_on_path("qemu-system-x86_64"))
        {
            return Ok(on_path);
        }

        let mut standard_candidates = vec![PathBuf::from(
            r"C:\Program Files\qemu\qemu-system-x86_64.exe",
        )];
        if let Ok(local_app_data) = std::env::var("LOCALAPPDATA") {
            standard_candidates.push(
                PathBuf::from(&local_app_data)
                    .join("Programs")
                    .join("qemu")
                    .join("qemu-system-x86_64.exe"),
            );
        }

        for candidate in standard_candidates {
            if candidate.is_file() {
                return Ok(candidate);
            }
        }

        Err(BifrostError::Hypervisor(
            "qemu-system-x86_64 was not found. Install QEMU (e.g., `winget install SoftwareFreedomConservancy.QEMU`) or set BIFROST_QEMU_BIN."
                .to_string(),
        ))
    }

    fn build_args(&self, spec: &VmSpec) -> Result<Vec<String>> {
        let mut args = vec![
            "-name".to_string(),
            format!("bifrost-midgard-{}", spec.instance_id),
            "-machine".to_string(),
            "q35,accel=whpx,kernel-irqchip=off".to_string(),
            "-cpu".to_string(),
            "qemu64,+ssse3,+sse4.1,+sse4.2,+popcnt,+x2apic".to_string(),
            "-m".to_string(),
            spec.memory_mb.to_string(),
            "-smp".to_string(),
            spec.cpu_cores.to_string(),
            "-device".to_string(),
            "virtio-rng-pci".to_string(),
            "-no-reboot".to_string(),
        ];

        // Boot media (canonical AOSP direct-kernel or bootable disk/ISO prototype).
        match &spec.bundle.boot {
            VerifiedBootMedia::DirectKernel {
                kernel,
                initrd,
                cmdline,
                disks,
            } => {
                args.push("-kernel".to_string());
                args.push(path_to_arg(kernel)?);
                if let Some(ref initrd_path) = initrd {
                    args.push("-initrd".to_string());
                    args.push(path_to_arg(initrd_path)?);
                }
                args.push("-append".to_string());
                args.push(cmdline.clone());

                for disk in disks {
                    let ro = if disk.read_only { "on" } else { "off" };
                    args.push("-drive".to_string());
                    args.push(format!(
                        "if=none,id={},file={},format={},readonly={ro}",
                        disk.id,
                        path_to_arg(&disk.path)?,
                        disk.format.as_qemu_format()
                    ));
                    args.push("-device".to_string());
                    args.push(format!("virtio-blk-pci,drive={}", disk.id));
                }
            }
            VerifiedBootMedia::BootableImage {
                image,
                format,
                media,
            } => {
                let media_str = match media {
                    DiskMedia::Cdrom => "cdrom",
                    DiskMedia::Disk => "disk",
                };
                args.push("-drive".to_string());
                args.push(format!(
                    "if=none,id=boot0,file={},format={},media={media_str},readonly=on",
                    path_to_arg(image)?,
                    format.as_qemu_format()
                ));
                args.push("-device".to_string());
                match media {
                    DiskMedia::Cdrom => args.push("ide-cd,drive=boot0".to_string()),
                    DiskMedia::Disk => args.push("virtio-blk-pci,drive=boot0".to_string()),
                }
                args.push("-boot".to_string());
                args.push(match media {
                    DiskMedia::Cdrom => "d".to_string(),
                    DiskMedia::Disk => "c".to_string(),
                });
            }
        }

        // Persistent Android userdata disk.
        args.push("-drive".to_string());
        args.push(format!(
            "if=none,id=userdata,file={},format={}",
            path_to_arg(&spec.userdata_disk)?,
            spec.userdata_format.as_qemu_format()
        ));
        args.push("-device".to_string());
        args.push("virtio-blk-pci,drive=userdata".to_string());

        // Networking with strictly scoped ADB port forwarding.
        args.push("-netdev".to_string());
        args.push(format!(
            "user,id=net0,hostfwd=tcp:{}:{}-:5555",
            spec.adb_bind_address, spec.adb_port
        ));
        args.push("-device".to_string());
        args.push("virtio-net-pci,netdev=net0".to_string());

        // Loopback QMP control channel for graceful ACPI shutdown.
        args.push("-qmp".to_string());
        args.push(format!(
            "tcp:127.0.0.1:{},server=on,wait=off",
            spec.qmp_port
        ));

        // Display / graphics configuration.
        if spec.headless {
            args.push("-display".to_string());
            args.push("none".to_string());
        } else {
            let gpu_device = match spec.graphics_mode {
                GraphicsMode::Auto | GraphicsMode::SwiftShader => "virtio-vga",
                GraphicsMode::Gfxstream => "virtio-vga",
            };
            args.push("-device".to_string());
            args.push(gpu_device.to_string());
        }

        // Serial console capture (file-backed log or interactive stdio).
        if spec.interactive_console {
            args.push("-serial".to_string());
            args.push("mon:stdio".to_string());
        } else {
            args.push("-chardev".to_string());
            args.push(format!(
                "file,id=serial0,path={}",
                path_to_arg(&spec.serial_log_path)?
            ));
            args.push("-serial".to_string());
            args.push("chardev:serial0".to_string());
        }

        Ok(args)
    }

    fn spawn(&self, spec: &VmSpec) -> Result<VmLaunchInfo> {
        let qemu_bin = self.locate_vmm()?;
        let args = self.build_args(spec)?;

        ensure_parent_dir(&spec.serial_log_path)?;
        ensure_parent_dir(&spec.vmm_stderr_path)?;

        let mut cmd = Command::new(&qemu_bin);
        cmd.args(&args);

        if spec.interactive_console {
            cmd.stdin(Stdio::inherit())
                .stdout(Stdio::inherit())
                .stderr(Stdio::inherit());
        } else {
            let stderr_file = OpenOptions::new()
                .create(true)
                .truncate(true)
                .write(true)
                .open(&spec.vmm_stderr_path)
                .map_err(|source| BifrostError::Io {
                    context: format!(
                        "creating VMM stderr log '{}'",
                        spec.vmm_stderr_path.display()
                    ),
                    source,
                })?;

            cmd.stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::from(stderr_file));
        }

        let child = cmd.spawn().map_err(|source| BifrostError::Io {
            context: format!("spawning QEMU VMM '{}'", qemu_bin.display()),
            source,
        })?;
        let pid = child.id();

        info!(
            pid,
            backend = self.name(),
            qmp_port = spec.qmp_port,
            adb_port = spec.adb_port,
            "spawned Midgard VM process"
        );

        let mut command_line = Vec::with_capacity(args.len() + 1);
        command_line.push(qemu_bin.display().to_string());
        command_line.extend(args);

        Ok(VmLaunchInfo {
            pid,
            backend_name: self.name().to_string(),
            command_line,
            serial_log_path: spec.serial_log_path.clone(),
            vmm_stderr_path: spec.vmm_stderr_path.clone(),
            qmp_endpoint: format!("127.0.0.1:{}", spec.qmp_port),
            adb_endpoint: format!("{}:{}", spec.adb_bind_address, spec.adb_port),
        })
    }

    fn shutdown_graceful(&self, qmp_endpoint: &str, pid: u32, timeout_secs: u64) -> Result<()> {
        if !self.is_running(pid) {
            return Ok(());
        }

        if let Err(err) = send_qmp_command(qmp_endpoint, "system_powerdown") {
            warn!(
                pid,
                endpoint = qmp_endpoint,
                error = %err,
                "ACPI system_powerdown via QMP failed; attempting QMP quit"
            );
            let _ = send_qmp_command(qmp_endpoint, "quit");
        }

        let deadline = Instant::now() + Duration::from_secs(timeout_secs);
        while Instant::now() < deadline {
            if !self.is_running(pid) {
                return Ok(());
            }
            thread::sleep(Duration::from_millis(250));
        }

        // Fall back to QMP `quit` before forceful OS termination.
        let _ = send_qmp_command(qmp_endpoint, "quit");
        thread::sleep(Duration::from_millis(500));
        if !self.is_running(pid) {
            return Ok(());
        }

        self.terminate_force(pid)
    }

    fn terminate_force(&self, pid: u32) -> Result<()> {
        if !self.is_running(pid) {
            return Ok(());
        }

        #[cfg(windows)]
        let status = Command::new("taskkill")
            .args(["/PID", &pid.to_string(), "/F", "/T"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map_err(|source| BifrostError::Io {
                context: format!("terminating VM PID {pid} via taskkill"),
                source,
            })?;

        #[cfg(not(windows))]
        let status = Command::new("kill")
            .args(["-9", &pid.to_string()])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map_err(|source| BifrostError::Io {
                context: format!("terminating VM PID {pid} via kill"),
                source,
            })?;

        if !status.success() && self.is_running(pid) {
            return Err(BifrostError::Hypervisor(format!(
                "failed to terminate VM process PID {pid}"
            )));
        }

        Ok(())
    }

    fn is_running(&self, pid: u32) -> bool {
        is_process_running(pid)
    }
}

/// Checks whether an OS process with the given `pid` is currently alive.
#[must_use]
pub fn is_process_running(pid: u32) -> bool {
    if pid == 0 {
        return false;
    }

    #[cfg(windows)]
    {
        let output = Command::new("tasklist")
            .args(["/FI", &format!("PID eq {pid}"), "/FO", "CSV", "/NH"])
            .output();
        match output {
            Ok(out) if out.status.success() => {
                let text = String::from_utf8_lossy(&out.stdout);
                let needle = format!("\"{pid}\"");
                text.lines().any(|line| line.contains(&needle))
            }
            _ => false,
        }
    }

    #[cfg(not(windows))]
    {
        Path::new(&format!("/proc/{pid}")).exists()
    }
}

fn find_on_path(binary_name: &str) -> Option<PathBuf> {
    let path_var = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path_var) {
        let candidate = dir.join(binary_name);
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

fn ensure_parent_dir(path: &Path) -> Result<()> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent).map_err(|source| BifrostError::Io {
                context: format!("creating directory '{}'", parent.display()),
                source,
            })?;
        }
    }
    Ok(())
}

fn path_to_arg(path: &Path) -> Result<String> {
    // Strip Windows extended-length prefix `\\?\` when passing paths to QEMU `-drive`.
    let raw = path.to_string_lossy();
    let cleaned = raw.strip_prefix(r"\\?\").unwrap_or(&raw);
    if cleaned.contains(',') {
        return Err(BifrostError::UnsafePath {
            path: path.to_path_buf(),
            reason: "commas in image paths are rejected to prevent QEMU option injection"
                .to_string(),
        });
    }
    Ok(cleaned.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use bifrost_storage::{DiskFormat, VerifiedBundle, VerifiedDisk};

    fn sample_direct_kernel_spec() -> VmSpec {
        VmSpec {
            instance_id: "default".to_string(),
            cpu_cores: 4,
            memory_mb: 4096,
            bundle: VerifiedBundle {
                id: "aosp-16-x86_64".to_string(),
                name: "AOSP 16".to_string(),
                android_version: "16".to_string(),
                arch: "x86_64".to_string(),
                manifest_path: PathBuf::from(r"C:\images\manifest.json"),
                boot: VerifiedBootMedia::DirectKernel {
                    kernel: PathBuf::from(r"C:\images\bzImage"),
                    initrd: Some(PathBuf::from(r"C:\images\ramdisk.img")),
                    cmdline: "console=ttyS0 androidboot.console=ttyS0".to_string(),
                    disks: vec![VerifiedDisk {
                        id: "super".to_string(),
                        path: PathBuf::from(r"C:\images\super.img"),
                        format: DiskFormat::Raw,
                        read_only: true,
                    }],
                },
            },
            userdata_disk: PathBuf::from(r"C:\data\userdata.img"),
            userdata_format: DiskFormat::Raw,
            adb_bind_address: "127.0.0.1".to_string(),
            adb_port: 5555,
            graphics_mode: GraphicsMode::Auto,
            headless: true,
            interactive_console: false,
            serial_log_path: PathBuf::from(r"C:\data\logs\serial.log"),
            vmm_stderr_path: PathBuf::from(r"C:\data\logs\vmm.stderr.log"),
            qmp_port: 4444,
        }
    }

    #[test]
    fn builds_whpx_virtio_and_loopback_adb_args() {
        let backend = QemuWhpxBackend::new();
        let args = backend.build_args(&sample_direct_kernel_spec()).unwrap();
        let joined = args.join(" ");

        assert!(joined.contains("q35,accel=whpx,kernel-irqchip=off"));
        assert!(joined.contains("-kernel C:\\images\\bzImage"));
        assert!(joined.contains("-initrd C:\\images\\ramdisk.img"));
        assert!(joined.contains("virtio-blk-pci,drive=super"));
        assert!(joined.contains("virtio-blk-pci,drive=userdata"));
        assert!(joined.contains("hostfwd=tcp:127.0.0.1:5555-:5555"));
        assert!(joined.contains("tcp:127.0.0.1:4444,server=on,wait=off"));
        assert!(joined.contains("-display none"));
    }

    #[test]
    fn rejects_comma_option_injection_in_paths() {
        let backend = QemuWhpxBackend::new();
        let mut spec = sample_direct_kernel_spec();
        spec.userdata_disk = PathBuf::from(r"C:\data\bad,readonly=on.img");
        let err = backend.build_args(&spec).unwrap_err();
        assert!(matches!(err, BifrostError::UnsafePath { .. }));
    }
}
