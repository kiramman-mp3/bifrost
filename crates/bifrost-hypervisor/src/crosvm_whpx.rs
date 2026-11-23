//! Target-evolution Rust VMM backend orchestrating `crosvm.exe` over WHPX + `gfxstream`
//! (ADR-0002 / ADR-0003).

use std::fs::{self, OpenOptions};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use bifrost_core::{BifrostError, GraphicsMode, Result};
use bifrost_storage::VerifiedBootMedia;
use tracing::info;

use crate::backend::{HypervisorBackend, VmLaunchInfo, VmSpec};
use crate::qemu_whpx::is_process_running;

/// `crosvm`-over-WHPX implementation of [`HypervisorBackend`].
#[derive(Debug, Clone, Default)]
pub struct CrosvmWhpxBackend {
    explicit_binary: Option<PathBuf>,
}

impl CrosvmWhpxBackend {
    /// Creates a `crosvm` backend using automatic discovery (`BIFROST_CROSVM_BIN`, `PATH`).
    #[must_use]
    pub const fn new() -> Self {
        Self {
            explicit_binary: None,
        }
    }

    /// Pins this backend to an explicit `crosvm.exe` binary path.
    #[must_use]
    pub fn with_binary(path: impl Into<PathBuf>) -> Self {
        Self {
            explicit_binary: Some(path.into()),
        }
    }
}

impl HypervisorBackend for CrosvmWhpxBackend {
    fn name(&self) -> &'static str {
        "crosvm-whpx"
    }

    fn locate_vmm(&self) -> Result<PathBuf> {
        if let Some(ref explicit) = self.explicit_binary {
            if explicit.is_file() {
                return Ok(explicit.clone());
            }
            return Err(BifrostError::Hypervisor(format!(
                "configured crosvm binary '{}' was not found",
                explicit.display()
            )));
        }

        if let Ok(env_bin) = std::env::var("BIFROST_CROSVM_BIN") {
            let candidate = PathBuf::from(env_bin);
            if candidate.is_file() {
                return Ok(candidate);
            }
        }

        if let Some(on_path) = find_on_path("crosvm.exe").or_else(|| find_on_path("crosvm")) {
            return Ok(on_path);
        }

        Err(BifrostError::Hypervisor(
            "crosvm binary not found. Provide a Windows WHPX crosvm build and set BIFROST_CROSVM_BIN or add crosvm.exe to PATH."
                .to_string(),
        ))
    }

    fn build_args(&self, spec: &VmSpec) -> Result<Vec<String>> {
        let VerifiedBootMedia::DirectKernel {
            ref kernel,
            ref initrd,
            ref cmdline,
            ref disks,
        } = spec.bundle.boot
        else {
            return Err(BifrostError::Hypervisor(
                "crosvm-whpx backend currently requires a direct_kernel AOSP bundle".to_string(),
            ));
        };

        let mut args = vec![
            "run".to_string(),
            "--hypervisor".to_string(),
            "whpx".to_string(),
            "--cpus".to_string(),
            spec.cpu_cores.to_string(),
            "--mem".to_string(),
            spec.memory_mb.to_string(),
            "--params".to_string(),
            cmdline.clone(),
        ];

        if let Some(ref initrd_path) = initrd {
            args.push("--initrd".to_string());
            args.push(clean_path(initrd_path));
        }

        for disk in disks {
            args.push("--block".to_string());
            let ro = if disk.read_only { ",ro=true" } else { "" };
            args.push(format!("{}{ro}", clean_path(&disk.path)));
        }

        args.push("--block".to_string());
        args.push(clean_path(&spec.userdata_disk));

        if !spec.headless {
            let gpu_arg = match spec.graphics_mode {
                GraphicsMode::Auto | GraphicsMode::Gfxstream => {
                    "backend=gfxstream,egl=true,gles=true,vulkan=true"
                }
                GraphicsMode::SwiftShader => "backend=2d",
            };
            args.push("--gpu".to_string());
            args.push(gpu_arg.to_string());
        }

        if !spec.interactive_console {
            args.push("--serial".to_string());
            args.push(format!(
                "type=file,hardware=serial,path={}",
                clean_path(&spec.serial_log_path)
            ));
        }

        args.push(clean_path(kernel));
        Ok(args)
    }

    fn spawn(&self, spec: &VmSpec) -> Result<VmLaunchInfo> {
        let bin = self.locate_vmm()?;
        let args = self.build_args(spec)?;

        if let Some(parent) = spec.vmm_stderr_path.parent() {
            fs::create_dir_all(parent).map_err(|source| BifrostError::Io {
                context: format!("creating log directory '{}'", parent.display()),
                source,
            })?;
        }

        let stderr_file = OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .open(&spec.vmm_stderr_path)
            .map_err(|source| BifrostError::Io {
                context: format!(
                    "creating crosvm stderr log '{}'",
                    spec.vmm_stderr_path.display()
                ),
                source,
            })?;

        let child = Command::new(&bin)
            .args(&args)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::from(stderr_file))
            .spawn()
            .map_err(|source| BifrostError::Io {
                context: format!("spawning crosvm '{}'", bin.display()),
                source,
            })?;

        let pid = child.id();
        info!(pid, backend = self.name(), "spawned Midgard crosvm VM");

        let mut command_line = Vec::with_capacity(args.len() + 1);
        command_line.push(bin.display().to_string());
        command_line.extend(args);

        Ok(VmLaunchInfo {
            pid,
            backend_name: self.name().to_string(),
            command_line,
            serial_log_path: spec.serial_log_path.clone(),
            vmm_stderr_path: spec.vmm_stderr_path.clone(),
            // crosvm has no QMP monitor; shutdown always terminates the process.
            qmp_endpoint: String::new(),
            adb_endpoint: format!("{}:{}", spec.adb_bind_address, spec.adb_port),
        })
    }

    fn shutdown_graceful(&self, _qmp_endpoint: &str, pid: u32, _timeout_secs: u64) -> Result<()> {
        self.terminate_force(pid)
    }

    fn terminate_force(&self, pid: u32) -> Result<()> {
        if !self.is_running(pid) {
            return Ok(());
        }
        #[cfg(windows)]
        let _ = Command::new("taskkill")
            .args(["/PID", &pid.to_string(), "/F", "/T"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
        Ok(())
    }

    fn is_running(&self, pid: u32) -> bool {
        is_process_running(pid)
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

fn clean_path(path: &Path) -> String {
    let raw = path.to_string_lossy();
    raw.strip_prefix(r"\\?\").unwrap_or(&raw).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use bifrost_storage::{DiskFormat, VerifiedBundle, VerifiedDisk};

    #[test]
    fn builds_crosvm_whpx_gfxstream_command_args() {
        let spec = VmSpec {
            instance_id: "default".to_string(),
            cpu_cores: 4,
            memory_mb: 4096,
            bundle: VerifiedBundle {
                id: "aosp-16".to_string(),
                name: "AOSP 16".to_string(),
                android_version: "16".to_string(),
                arch: "x86_64".to_string(),
                manifest_path: PathBuf::from(r"C:\img\manifest.json"),
                boot: VerifiedBootMedia::DirectKernel {
                    kernel: PathBuf::from(r"C:\img\bzImage"),
                    initrd: Some(PathBuf::from(r"C:\img\ramdisk.img")),
                    cmdline: "console=ttyS0".to_string(),
                    disks: vec![VerifiedDisk {
                        id: "super".to_string(),
                        path: PathBuf::from(r"C:\img\super.img"),
                        format: DiskFormat::Raw,
                        read_only: true,
                    }],
                },
            },
            userdata_disk: PathBuf::from(r"C:\data\userdata.img"),
            userdata_format: DiskFormat::Raw,
            adb_bind_address: "127.0.0.1".to_string(),
            adb_port: 5555,
            graphics_mode: GraphicsMode::Gfxstream,
            audio_enabled: true,
            headless: false,
            interactive_console: false,
            serial_log_path: PathBuf::from(r"C:\logs\serial.log"),
            vmm_stderr_path: PathBuf::from(r"C:\logs\stderr.log"),
            qmp_pipe_name: "bifrost-qmp-00112233445566778899aabbccddeeff".to_string(),
        };

        let args = CrosvmWhpxBackend::new().build_args(&spec).unwrap();
        let joined = args.join(" ");
        assert!(joined.contains("--hypervisor whpx"));
        assert!(joined.contains("backend=gfxstream,egl=true,gles=true,vulkan=true"));
        assert!(joined.contains(r"C:\img\bzImage"));
    }
}
