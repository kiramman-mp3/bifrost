//! High-level Midgard Android runtime lifecycle coordinator.

use std::fs;
use std::net::TcpListener;
use std::path::{Path, PathBuf};

use bifrost_core::{BifrostConfig, BifrostError, BifrostPaths, Result, RuntimeState};
use bifrost_hypervisor::{HypervisorBackend, QemuWhpxBackend, VmSpec};
use bifrost_storage::{ensure_userdata_disk, verify_manifest_file, ImageRegistry, VerifiedBundle};
use serde::{Deserialize, Serialize};
use tracing::info;

use crate::state_store::{load_record, save_record, RuntimeRecord};

const DEFAULT_INSTANCE_ID: &str = "default";
const GRACEFUL_SHUTDOWN_TIMEOUT_SECS: u64 = 12;

/// Options controlling how a Midgard VM instance is started.
#[derive(Debug, Clone, Default)]
pub struct StartOptions {
    pub headless: bool,
    pub interactive_console: bool,
}

/// Snapshot of runtime serial and VMM stderr logs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeLogs {
    pub serial_log_path: PathBuf,
    pub serial_lines: Vec<String>,
    pub vmm_stderr_path: PathBuf,
    pub vmm_stderr_lines: Vec<String>,
}

/// Coordinates Midgard image verification, persistent disks, WHPX launch, shutdown, and logs.
pub struct MidgardRuntime<B: HypervisorBackend = QemuWhpxBackend> {
    config: BifrostConfig,
    paths: BifrostPaths,
    backend: B,
    instance_id: String,
}

impl MidgardRuntime<QemuWhpxBackend> {
    /// Constructs a Midgard runtime using the default Phase 1 QEMU-WHPX backend.
    #[must_use]
    pub fn with_default_backend(config: BifrostConfig, paths: BifrostPaths) -> Self {
        Self::new(config, paths, QemuWhpxBackend::new())
    }
}

impl<B: HypervisorBackend> MidgardRuntime<B> {
    /// Constructs a Midgard runtime with an explicit [`HypervisorBackend`] implementation.
    #[must_use]
    pub fn new(config: BifrostConfig, paths: BifrostPaths, backend: B) -> Self {
        Self {
            config,
            paths,
            backend,
            instance_id: DEFAULT_INSTANCE_ID.to_string(),
        }
    }

    fn state_file(&self) -> PathBuf {
        self.paths
            .data_dir
            .join("instances")
            .join(&self.instance_id)
            .join("runtime-state.json")
    }

    fn userdata_disk_path(&self) -> PathBuf {
        self.paths
            .data_dir
            .join("instances")
            .join(&self.instance_id)
            .join("userdata.img")
    }

    fn serial_log_path(&self) -> PathBuf {
        self.paths
            .logs_dir
            .join(format!("midgard-{}-serial.log", self.instance_id))
    }

    fn vmm_stderr_path(&self) -> PathBuf {
        self.paths
            .logs_dir
            .join(format!("midgard-{}-vmm.stderr.log", self.instance_id))
    }

    /// Cryptographically verifies a Midgard image bundle manifest without booting it.
    pub fn verify_image(&self, manifest_path: impl AsRef<Path>) -> Result<VerifiedBundle> {
        verify_manifest_file(manifest_path)
    }

    /// Reads the persisted runtime record and reconciles it with actual OS process liveness.
    pub fn status(&self) -> Result<RuntimeRecord> {
        let state_file = self.state_file();
        let mut record = load_record(&state_file, &self.instance_id)?;

        if record.state.is_active() {
            let alive = record.pid.is_some_and(|pid| self.backend.is_running(pid));
            if !alive {
                record.state = RuntimeState::Stopped;
                record.pid = None;
                record.note = Some(
                    "Previous Midgard VM process is no longer running; state reconciled to stopped."
                        .to_string(),
                );
                let _ = save_record(&state_file, &record);
            }
        }

        Ok(record)
    }

    /// Verifies the image bundle, ensures persistent `userdata`, and starts Midgard on WHPX.
    pub fn start(
        &self,
        manifest_path: impl AsRef<Path>,
        options: StartOptions,
    ) -> Result<RuntimeRecord> {
        self.config.validate()?;

        let current = self.status()?;
        if current.state.is_active() {
            return Err(BifrostError::Runtime(format!(
                "Midgard instance '{}' is already {} (PID {})",
                self.instance_id,
                current.state,
                current.pid.unwrap_or(0)
            )));
        }

        // Verify VMM availability before spending time hashing multi-GB images.
        let _vmm_path = self.backend.locate_vmm()?;

        // Cryptographically verify every image artifact referenced by the manifest.
        let bundle = verify_manifest_file(manifest_path.as_ref())?;

        // Provision or preserve persistent Android userdata virtual disk.
        let userdata =
            ensure_userdata_disk(self.userdata_disk_path(), self.config.runtime.storage_gb)?;

        let qmp_port = allocate_loopback_port()?;
        let spec = VmSpec {
            instance_id: self.instance_id.clone(),
            cpu_cores: self.config.runtime.cpu_cores,
            memory_mb: self.config.runtime.memory_mb,
            bundle: bundle.clone(),
            userdata_disk: userdata.path.clone(),
            userdata_format: userdata.format,
            adb_bind_address: self.config.network.adb_bind_address.clone(),
            adb_port: self.config.network.adb_port,
            graphics_mode: self.config.runtime.graphics_mode,
            audio_enabled: self.config.runtime.audio_enabled,
            headless: options.headless,
            interactive_console: options.interactive_console,
            serial_log_path: self.serial_log_path(),
            vmm_stderr_path: self.vmm_stderr_path(),
            qmp_port,
        };

        let launch = self.backend.spawn(&spec)?;
        let record = RuntimeRecord {
            instance_id: self.instance_id.clone(),
            state: RuntimeState::Booting,
            pid: Some(launch.pid),
            backend: Some(launch.backend_name),
            image_id: Some(bundle.id),
            android_version: Some(bundle.android_version),
            manifest_path: Some(bundle.manifest_path),
            userdata_disk: Some(userdata.path),
            adb_endpoint: Some(launch.adb_endpoint),
            qmp_endpoint: Some(launch.qmp_endpoint),
            serial_log_path: Some(launch.serial_log_path),
            vmm_stderr_path: Some(launch.vmm_stderr_path),
            note: Some(
                "VM spawned over WHPX; inspect serial output via `bifrost logs`.".to_string(),
            ),
        };

        save_record(&self.state_file(), &record)?;
        info!(
            instance = %self.instance_id,
            pid = launch.pid,
            "Midgard runtime started"
        );
        Ok(record)
    }

    /// Gracefully shuts down (or forcefully terminates) the active Midgard VM.
    pub fn stop(&self, force: bool) -> Result<RuntimeRecord> {
        let mut record = self.status()?;
        let Some(pid) = record.pid else {
            return Err(BifrostError::Runtime(format!(
                "Midgard instance '{}' is not running",
                self.instance_id
            )));
        };

        if force {
            self.backend.terminate_force(pid)?;
        } else if let Some(ref qmp) = record.qmp_endpoint {
            self.backend
                .shutdown_graceful(qmp, pid, GRACEFUL_SHUTDOWN_TIMEOUT_SECS)?;
        } else {
            self.backend.terminate_force(pid)?;
        }

        record.state = RuntimeState::Stopped;
        record.pid = None;
        record.note = Some(if force {
            "Midgard VM terminated forcefully.".to_string()
        } else {
            "Midgard VM shut down cleanly.".to_string()
        });
        save_record(&self.state_file(), &record)?;
        Ok(record)
    }

    /// Restarts Midgard using the given manifest (or the previously launched manifest if omitted).
    pub fn restart(
        &self,
        manifest_path: Option<&Path>,
        options: StartOptions,
    ) -> Result<RuntimeRecord> {
        let previous = self.status()?;
        let resolved_manifest = manifest_path
            .map(Path::to_path_buf)
            .or(previous.manifest_path)
            .ok_or_else(|| {
                BifrostError::Runtime(
                    "no manifest path provided and no previous Midgard image recorded".to_string(),
                )
            })?;

        if previous.state.is_active() {
            let _ = self.stop(false)?;
        }

        self.start(resolved_manifest, options)
    }

    /// Resolves the active Midgard image manifest from `ImageRegistry` or the previous runtime record.
    pub fn resolve_default_manifest(&self) -> Result<PathBuf> {
        if let Ok(registry) = ImageRegistry::load(&self.paths.images_dir) {
            if let Some(active) = registry.active_image() {
                if active.manifest_path.is_file() {
                    return Ok(active.manifest_path.clone());
                }
            }
        }

        if let Ok(previous) = self.status() {
            if let Some(manifest) = previous.manifest_path {
                if manifest.is_file() {
                    return Ok(manifest);
                }
            }
        }

        Err(BifrostError::Runtime(
            "no registered Midgard image found for automatic headless startup; register one with `bifrost images register --manifest <path>`"
                .to_string(),
        ))
    }

    /// Ensures Midgard is running, automatically launching the active registered image
    /// in invisible `--headless` mode if currently stopped (WSA-style zero-friction boot).
    pub fn ensure_started_headless(&self) -> Result<RuntimeRecord> {
        let current = self.status()?;
        if current.state.is_active() {
            return Ok(current);
        }
        let manifest = self.resolve_default_manifest()?;
        self.start(
            manifest,
            StartOptions {
                headless: true,
                interactive_console: false,
            },
        )
    }

    /// Reads the tail of the Midgard serial console log and VMM stderr log.
    pub fn read_logs(&self, tail: usize) -> Result<RuntimeLogs> {
        let record = self.status()?;
        let serial_path = record
            .serial_log_path
            .unwrap_or_else(|| self.serial_log_path());
        let stderr_path = record
            .vmm_stderr_path
            .unwrap_or_else(|| self.vmm_stderr_path());

        Ok(RuntimeLogs {
            serial_lines: read_tail_lines(&serial_path, tail)?,
            serial_log_path: serial_path,
            vmm_stderr_lines: read_tail_lines(&stderr_path, tail)?,
            vmm_stderr_path: stderr_path,
        })
    }
}

fn allocate_loopback_port() -> Result<u16> {
    let listener = TcpListener::bind("127.0.0.1:0").map_err(|source| BifrostError::Io {
        context: "allocating ephemeral loopback QMP port".to_string(),
        source,
    })?;
    let port = listener
        .local_addr()
        .map_err(|source| BifrostError::Io {
            context: "querying ephemeral loopback QMP port".to_string(),
            source,
        })?
        .port();
    Ok(port)
}

fn read_tail_lines(path: &Path, tail: usize) -> Result<Vec<String>> {
    if !path.exists() {
        return Ok(Vec::new());
    }
    let bytes = fs::read(path).map_err(|source| BifrostError::Io {
        context: format!("reading log file '{}'", path.display()),
        source,
    })?;
    let text = String::from_utf8_lossy(&bytes);
    let lines: Vec<String> = text.lines().map(ToOwned::to_owned).collect();
    if tail == 0 || lines.len() <= tail {
        Ok(lines)
    } else {
        Ok(lines[lines.len() - tail..].to_vec())
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;

    use bifrost_hypervisor::VmLaunchInfo;
    use bifrost_storage::sha256_file;
    use tempfile::TempDir;

    use super::*;

    struct MockBackend {
        running: Arc<AtomicBool>,
    }

    impl MockBackend {
        fn new() -> Self {
            Self {
                running: Arc::new(AtomicBool::new(false)),
            }
        }
    }

    impl HypervisorBackend for MockBackend {
        fn name(&self) -> &'static str {
            "mock-whpx"
        }

        fn locate_vmm(&self) -> Result<PathBuf> {
            Ok(PathBuf::from("mock-qemu.exe"))
        }

        fn build_args(&self, _spec: &VmSpec) -> Result<Vec<String>> {
            Ok(vec!["-accel".to_string(), "whpx".to_string()])
        }

        fn spawn(&self, spec: &VmSpec) -> Result<VmLaunchInfo> {
            self.running.store(true, Ordering::SeqCst);
            if let Some(parent) = spec.serial_log_path.parent() {
                fs::create_dir_all(parent).unwrap();
            }
            fs::write(
                &spec.serial_log_path,
                "[    0.000000] Linux version mock\n[    1.200000] init: android boot\n",
            )
            .unwrap();
            fs::write(&spec.vmm_stderr_path, "").unwrap();
            Ok(VmLaunchInfo {
                pid: 4242,
                backend_name: self.name().to_string(),
                command_line: vec!["mock-qemu.exe".to_string()],
                serial_log_path: spec.serial_log_path.clone(),
                vmm_stderr_path: spec.vmm_stderr_path.clone(),
                qmp_endpoint: format!("127.0.0.1:{}", spec.qmp_port),
                adb_endpoint: format!("{}:{}", spec.adb_bind_address, spec.adb_port),
            })
        }

        fn shutdown_graceful(&self, _qmp: &str, _pid: u32, _timeout: u64) -> Result<()> {
            self.running.store(false, Ordering::SeqCst);
            Ok(())
        }

        fn terminate_force(&self, _pid: u32) -> Result<()> {
            self.running.store(false, Ordering::SeqCst);
            Ok(())
        }

        fn is_running(&self, _pid: u32) -> bool {
            self.running.load(Ordering::SeqCst)
        }
    }

    #[test]
    fn start_status_logs_restart_and_stop_lifecycle_works() {
        let tmp = TempDir::new().unwrap();
        let img_dir = tmp.path().join("bundle");
        fs::create_dir_all(&img_dir).unwrap();
        let iso = img_dir.join("android.iso");
        fs::write(&iso, b"mock-android-iso").unwrap();
        let digest = sha256_file(&iso).unwrap();
        let manifest = img_dir.join("manifest.json");
        fs::write(
            &manifest,
            format!(
                r#"{{
                    "schema_version": 1,
                    "id": "proto-x86_64",
                    "name": "Midgard Test",
                    "android_version": "15",
                    "arch": "x86_64",
                    "boot": {{
                        "type": "bootable_image",
                        "image": {{ "relative_path": "android.iso", "sha256": "{digest}" }},
                        "format": "raw",
                        "media": "cdrom"
                    }}
                }}"#
            ),
        )
        .unwrap();

        let mut cfg = BifrostConfig::default();
        cfg.runtime.storage_gb = 8;
        let paths = BifrostPaths::from_root(tmp.path().join("bifrost"));
        let runtime = MidgardRuntime::new(cfg, paths, MockBackend::new());

        assert_eq!(runtime.status().unwrap().state, RuntimeState::Stopped);

        let started = runtime
            .start(
                &manifest,
                StartOptions {
                    headless: true,
                    interactive_console: false,
                },
            )
            .unwrap();
        assert_eq!(started.state, RuntimeState::Booting);
        assert_eq!(started.pid, Some(4242));

        // Duplicate start while active must fail.
        assert!(runtime.start(&manifest, StartOptions::default()).is_err());

        // Logs must capture serial output.
        let logs = runtime.read_logs(10).unwrap();
        assert_eq!(logs.serial_lines.len(), 2);
        assert!(logs.serial_lines[1].contains("android boot"));

        // Restart reuses recorded manifest cleanly.
        let restarted = runtime
            .restart(
                None,
                StartOptions {
                    headless: true,
                    interactive_console: false,
                },
            )
            .unwrap();
        assert_eq!(restarted.state, RuntimeState::Booting);

        // Clean stop transitions back to Stopped.
        let stopped = runtime.stop(false).unwrap();
        assert_eq!(stopped.state, RuntimeState::Stopped);
        assert_eq!(stopped.pid, None);
    }
}
