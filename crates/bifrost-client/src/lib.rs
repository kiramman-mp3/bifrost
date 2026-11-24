//! Client-side control plane shared by Mimir (`bifrost` CLI) and the desktop UI.
//!
//! Every front end needs the same rule: when the Heimdall daemon answers on its
//! authenticated Named Pipe, route the operation through IPC so the supervisor owns the
//! runtime; otherwise call the runtime and Valhalla directly. [`Session`] probes Heimdall
//! once, applies that rule for each operation, and runs every direct (blocking) call on
//! Tokio's blocking pool so async callers such as the Tauri UI never stall a worker.

use std::path::{Path, PathBuf};
use std::time::Duration;

use bifrost_bridge::{
    default_pipe_name, AdbClient, AdbEndpointState, DaemonHealthReport, HeimdallClient, IpcRequest,
};
use bifrost_core::{
    validate_host_file_path, BifrostConfig, BifrostError, BifrostPaths, Result, RuntimeState,
};
use bifrost_display::DisplayController;
use bifrost_runtime::{MidgardRuntime, RuntimeLogs, RuntimeRecord, StartOptions};
use serde::Serialize;
use valhalla::manager::{InstallOutcome, LaunchOutcome};
use valhalla::{InstalledPackage, PackageDetails, ValhallaManager};

/// How long an automatic headless boot may take before Android must accept ADB commands.
pub const DEVICE_READY_TIMEOUT: Duration = Duration::from_secs(90);

/// Effective configuration: the file it came from and the per-user paths it implies.
#[derive(Debug, Clone)]
pub struct LoadedConfig {
    pub config_path: PathBuf,
    pub config_exists: bool,
    pub config: BifrostConfig,
    pub paths: BifrostPaths,
}

impl LoadedConfig {
    /// Loads `explicit` (validated as an existing `.toml` host file) or the per-user
    /// default `bifrost.toml`, falling back to defaults when the default file is absent.
    /// A custom `[runtime] data_dir` relocates the data, images and logs directories.
    pub fn load(explicit: Option<&Path>) -> Result<Self> {
        let mut paths = BifrostPaths::discover()?;
        let config_path = match explicit {
            Some(path) => validate_host_file_path(path, Some("toml"))?,
            None => paths.default_config_file(),
        };
        let config_exists = config_path.exists();
        let config = if config_exists {
            BifrostConfig::load_from_file(&config_path)?
        } else {
            BifrostConfig::default()
        };
        if let Some(ref custom_data_dir) = config.runtime.data_dir {
            paths.data_dir = custom_data_dir.clone();
            paths.images_dir = custom_data_dir.join("images");
            paths.logs_dir = custom_data_dir.join("logs");
        }
        Ok(Self {
            config_path,
            config_exists,
            config,
            paths,
        })
    }

    /// Loopback ADB endpoint Midgard exposes for this configuration.
    #[must_use]
    pub fn adb_endpoint(&self) -> String {
        format!(
            "{}:{}",
            self.config.network.adb_bind_address, self.config.network.adb_port
        )
    }
}

/// Runtime status merged from Heimdall (when connected) and a direct ADB reconciliation.
#[derive(Debug, Clone, Serialize)]
pub struct StatusSnapshot {
    /// Persisted runtime record (from Heimdall, or read directly).
    pub record: RuntimeRecord,
    /// Effective state: Heimdall's live view when connected, else the reconciled record.
    pub runtime_state: RuntimeState,
    /// Heimdall's health report, or `None` when the daemon is not running.
    pub heimdall_health: Option<DaemonHealthReport>,
    /// Why ADB cannot (yet) drive Midgard, when known.
    pub adb_access: Option<AdbEndpointState>,
}

impl StatusSnapshot {
    /// User-facing remediation for a blocked ADB endpoint, if any.
    #[must_use]
    pub fn adb_remediation(&self) -> Option<&'static str> {
        self.adb_access.and_then(AdbEndpointState::remediation)
    }
}

/// One front-end operation context: configuration plus a single Heimdall probe.
pub struct Session {
    loaded: LoadedConfig,
    heimdall: HeimdallClient,
    health: Option<DaemonHealthReport>,
}

impl Session {
    /// Loads the configuration and probes Heimdall on the default per-user pipe.
    pub async fn open(explicit_config: Option<&Path>) -> Result<Self> {
        Ok(Self::connect(LoadedConfig::load(explicit_config)?, default_pipe_name()).await)
    }

    /// Builds a session from an already loaded configuration and a pipe name.
    pub async fn connect(loaded: LoadedConfig, pipe_name: String) -> Self {
        let heimdall = HeimdallClient::new(pipe_name, &loaded.paths.data_dir);
        let health = heimdall.health().await.ok();
        Self {
            loaded,
            heimdall,
            health,
        }
    }

    #[must_use]
    pub fn loaded(&self) -> &LoadedConfig {
        &self.loaded
    }

    #[must_use]
    pub fn config(&self) -> &BifrostConfig {
        &self.loaded.config
    }

    #[must_use]
    pub fn paths(&self) -> &BifrostPaths {
        &self.loaded.paths
    }

    /// Heimdall's health report captured when the session was opened.
    #[must_use]
    pub fn heimdall_health(&self) -> Option<&DaemonHealthReport> {
        self.health.as_ref()
    }

    #[must_use]
    pub fn heimdall_connected(&self) -> bool {
        self.health.is_some()
    }

    /// Authenticated Heimdall client, for daemon-only requests.
    #[must_use]
    pub fn heimdall(&self) -> &HeimdallClient {
        &self.heimdall
    }

    fn runtime(&self) -> MidgardRuntime {
        MidgardRuntime::with_default_backend(self.loaded.config.clone(), self.loaded.paths.clone())
    }

    /// Runs a blocking runtime call on Tokio's blocking pool.
    async fn with_runtime<T: Send + 'static>(
        &self,
        op: impl FnOnce(MidgardRuntime) -> Result<T> + Send + 'static,
    ) -> Result<T> {
        let runtime = self.runtime();
        blocking(move || op(runtime)).await
    }

    /// Sends `request` to Heimdall when it is up, else runs `direct` on the blocking pool.
    async fn route<T, F>(&self, request: IpcRequest, direct: F) -> Result<T>
    where
        T: serde::de::DeserializeOwned + Send + 'static,
        F: FnOnce() -> Result<T> + Send + 'static,
    {
        if self.heimdall_connected() {
            self.heimdall.send(request).await
        } else {
            blocking(direct).await
        }
    }

    /// Current runtime status. Without Heimdall, a `Booting` VM whose adbd answers and
    /// reports `sys.boot_completed=1` is promoted to `Running` (and persisted).
    pub async fn status(&self) -> Result<StatusSnapshot> {
        let mut record = if self.heimdall_connected() {
            self.heimdall.send(IpcRequest::RuntimeStatus).await?
        } else {
            self.with_runtime(|rt| rt.status()).await?
        };

        let mut adb_access = self.health.as_ref().and_then(|h| h.adb_access);
        if !self.heimdall_connected() && record.state == RuntimeState::Booting {
            if let Some(endpoint) = record.adb_endpoint.clone() {
                let runtime = self.runtime();
                let (access, promoted) = blocking(move || {
                    let adb = AdbClient::new();
                    let access = adb.diagnose_endpoint(&endpoint).ok();
                    let promoted = access == Some(AdbEndpointState::Ready)
                        && adb
                            .probe_guest_health(&endpoint)
                            .is_ok_and(|g| g.boot_completed);
                    if promoted {
                        let _ = runtime.mark_running();
                    }
                    Ok((access, promoted))
                })
                .await?;
                adb_access = access;
                if promoted {
                    record.state = RuntimeState::Running;
                }
            }
        }

        let runtime_state = self
            .health
            .as_ref()
            .map_or(record.state, |h| h.runtime_state);
        Ok(StatusSnapshot {
            record,
            runtime_state,
            heimdall_health: self.health.clone(),
            adb_access,
        })
    }

    /// Starts Midgard. `console` attaches the serial console to this process, which only
    /// a direct start can do, so it bypasses Heimdall.
    pub async fn start_runtime(
        &self,
        manifest_path: PathBuf,
        headless: bool,
        console: bool,
    ) -> Result<RuntimeRecord> {
        if self.heimdall_connected() && !console {
            return self
                .heimdall
                .send(IpcRequest::RuntimeStart {
                    manifest_path,
                    headless,
                })
                .await;
        }
        self.with_runtime(move |rt| {
            rt.start(
                manifest_path,
                StartOptions {
                    headless,
                    interactive_console: console,
                },
            )
        })
        .await
    }

    pub async fn stop_runtime(&self, force: bool) -> Result<RuntimeRecord> {
        let runtime = self.runtime();
        self.route(IpcRequest::RuntimeStop { force }, move || {
            runtime.stop(force)
        })
        .await
    }

    pub async fn restart_runtime(
        &self,
        manifest_path: Option<PathBuf>,
        headless: bool,
    ) -> Result<RuntimeRecord> {
        let runtime = self.runtime();
        let request = IpcRequest::RuntimeRestart {
            manifest_path: manifest_path.clone(),
            headless,
        };
        self.route(request, move || {
            runtime.restart(
                manifest_path.as_deref(),
                StartOptions {
                    headless,
                    interactive_console: false,
                },
            )
        })
        .await
    }

    /// Recent serial and VMM logs; read directly if Heimdall cannot serve them.
    pub async fn logs(&self, tail: usize) -> Result<RuntimeLogs> {
        if self.heimdall_connected() {
            if let Ok(logs) = self.heimdall.send(IpcRequest::RuntimeLogs { tail }).await {
                return Ok(logs);
            }
        }
        self.with_runtime(move |rt| rt.read_logs(tail)).await
    }

    pub async fn list_apps(
        &self,
        serial: Option<String>,
        third_party_only: bool,
    ) -> Result<Vec<InstalledPackage>> {
        let request = IpcRequest::AppsList {
            serial: serial.clone(),
            third_party_only,
        };
        self.route(request, move || {
            ValhallaManager::new().list_packages(serial.as_deref(), third_party_only)
        })
        .await
    }

    pub async fn app_info(
        &self,
        serial: Option<String>,
        package: String,
    ) -> Result<PackageDetails> {
        let request = IpcRequest::AppInfo {
            serial: serial.clone(),
            package: package.clone(),
        };
        self.route(request, move || {
            ValhallaManager::new().package_info(serial.as_deref(), &package)
        })
        .await
    }

    pub async fn launch_app(
        &self,
        serial: Option<String>,
        package: String,
    ) -> Result<LaunchOutcome> {
        let request = IpcRequest::AppLaunch {
            serial: serial.clone(),
            package: package.clone(),
        };
        self.route(request, move || {
            ValhallaManager::new().launch_app(serial.as_deref(), &package)
        })
        .await
    }

    pub async fn stop_app(&self, serial: Option<String>, package: String) -> Result<()> {
        let request = IpcRequest::AppStop {
            serial: serial.clone(),
            package: package.clone(),
        };
        self.route::<serde_json::Value, _>(request, move || {
            ValhallaManager::new()
                .force_stop(serial.as_deref(), &package)
                .map(|()| serde_json::Value::Null)
        })
        .await
        .map(|_| ())
    }

    pub async fn install_apk(
        &self,
        serial: Option<String>,
        apk_path: PathBuf,
    ) -> Result<InstallOutcome> {
        let request = IpcRequest::AppInstall {
            serial: serial.clone(),
            apk_path: apk_path.clone(),
        };
        self.route(request, move || {
            ValhallaManager::new().install_apk(serial.as_deref(), apk_path)
        })
        .await
    }

    /// Uninstalls `package` and returns PackageManager's output line.
    pub async fn uninstall_app(
        &self,
        serial: Option<String>,
        package: String,
        keep_data: bool,
    ) -> Result<String> {
        if self.heimdall_connected() {
            let resp: serde_json::Value = self
                .heimdall
                .send(IpcRequest::AppUninstall {
                    serial,
                    package,
                    keep_data,
                })
                .await?;
            return Ok(resp["output"].as_str().unwrap_or("Success").to_string());
        }
        blocking(move || {
            ValhallaManager::new().uninstall_package(serial.as_deref(), &package, keep_data)
        })
        .await
    }

    /// Returns the ADB serial to target, booting Midgard headlessly first when needed.
    ///
    /// An explicit `serial` is used as is. Otherwise the single online device is chosen;
    /// if there is none and `autostart` is set, Midgard is started headless from the
    /// active image (through Heimdall when it is up), `on_boot` is called, and the call
    /// waits up to [`DEVICE_READY_TIMEOUT`] for Android to accept ADB commands.
    pub async fn ensure_device_ready(
        &self,
        serial: Option<String>,
        autostart: bool,
        on_boot: impl FnOnce(),
    ) -> Result<String> {
        let target =
            blocking(move || ValhallaManager::new().resolve_target_serial(serial.as_deref())).await;
        let err = match target {
            Ok(serial) => return Ok(serial),
            Err(err) if !autostart => return Err(err),
            Err(err) => err,
        };

        // `None`: already active (still booting); `Some`: the image to boot headless.
        let boot_manifest = self
            .with_runtime(|rt| {
                if rt.status()?.state.is_active() {
                    return Ok(None);
                }
                rt.resolve_default_manifest().map(Some)
            })
            .await
            .map_err(|manifest_err| {
                BifrostError::Runtime(format!("{err}; cannot auto-start Midgard: {manifest_err}"))
            })?;
        on_boot();
        if let Some(manifest) = boot_manifest {
            if self.heimdall_connected() {
                let _: RuntimeRecord = self
                    .heimdall
                    .send(IpcRequest::RuntimeStart {
                        manifest_path: manifest,
                        headless: true,
                    })
                    .await?;
            } else {
                self.with_runtime(|rt| rt.ensure_started_headless()).await?;
            }
        }

        let endpoint = self.loaded.adb_endpoint();
        blocking(move || {
            DisplayController::new().wait_for_device_ready(&endpoint, DEVICE_READY_TIMEOUT)
        })
        .await
    }
}

/// Runs a blocking closure on Tokio's blocking pool, mapping a panicked task to an error.
pub async fn blocking<T, F>(op: F) -> Result<T>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T> + Send + 'static,
{
    tokio::task::spawn_blocking(op)
        .await
        .map_err(|err| BifrostError::Runtime(format!("background task failed: {err}")))?
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_config(dir: &Path, body: &str) -> PathBuf {
        let path = dir.join("bifrost.toml");
        std::fs::write(&path, body).unwrap();
        path
    }

    fn data_dir_config(dir: &Path) -> PathBuf {
        let data = dir.join("data").display().to_string().replace('\\', "/");
        temp_config(
            dir,
            &format!(
                "schema_version = 1\n\n[runtime]\ndata_dir = '{data}'\n\n[network]\nadb_bind_address = \"127.0.0.1\"\nadb_port = 5641\n"
            ),
        )
    }

    async fn offline_session(dir: &Path) -> Session {
        let loaded = LoadedConfig::load(Some(&data_dir_config(dir))).unwrap();
        // A pipe nobody serves: the session must fall back to direct calls.
        let pipe = format!(r"\\.\pipe\bifrost-client-test-{}", std::process::id());
        Session::connect(loaded, pipe).await
    }

    #[test]
    fn explicit_config_relocates_data_paths_and_endpoint() {
        let dir = tempfile::tempdir().unwrap();
        let loaded = LoadedConfig::load(Some(&data_dir_config(dir.path()))).unwrap();
        assert!(loaded.config_exists);
        let data = dir.path().join("data");
        assert_eq!(loaded.paths.data_dir, data);
        assert_eq!(loaded.paths.images_dir, data.join("images"));
        assert_eq!(loaded.paths.logs_dir, data.join("logs"));
        assert_eq!(loaded.adb_endpoint(), "127.0.0.1:5641");
    }

    #[test]
    fn explicit_config_is_validated_and_may_not_exist_yet() {
        let dir = tempfile::tempdir().unwrap();
        assert!(LoadedConfig::load(Some(&dir.path().join("bifrost.txt"))).is_err());
        assert!(LoadedConfig::load(Some(&dir.path().join("..").join("x.toml"))).is_err());
        // `bifrost --config new.toml config init` targets a file that does not exist yet.
        let fresh = LoadedConfig::load(Some(&dir.path().join("new.toml"))).unwrap();
        assert!(!fresh.config_exists);
        assert_eq!(fresh.config, BifrostConfig::default());
    }

    #[tokio::test]
    async fn offline_session_reads_status_directly() {
        let dir = tempfile::tempdir().unwrap();
        let session = offline_session(dir.path()).await;
        assert!(!session.heimdall_connected());
        let status = session.status().await.unwrap();
        assert_eq!(status.runtime_state, RuntimeState::Stopped);
        assert!(status.heimdall_health.is_none());
        assert!(status.adb_remediation().is_none());
    }

    #[tokio::test]
    async fn explicit_serial_is_used_without_booting() {
        let dir = tempfile::tempdir().unwrap();
        let session = offline_session(dir.path()).await;
        let mut booted = false;
        let serial = session
            .ensure_device_ready(Some("emulator-5554".to_string()), true, || booted = true)
            .await
            .unwrap();
        assert_eq!(serial, "emulator-5554");
        assert!(!booted);
    }

    #[tokio::test]
    async fn autostart_without_an_image_explains_why() {
        let dir = tempfile::tempdir().unwrap();
        let session = offline_session(dir.path()).await;
        let mut booted = false;
        let err = session
            .ensure_device_ready(None, true, || booted = true)
            .await;
        // Either a real device happens to be attached (CI/dev box), or no image exists.
        if let Err(err) = err {
            assert!(
                err.to_string().contains("cannot auto-start Midgard"),
                "unexpected error: {err}"
            );
            assert!(!booted);
        }
    }

    #[tokio::test]
    async fn blocking_maps_panics_to_errors() {
        let err = blocking::<(), _>(|| panic!("boom")).await.unwrap_err();
        assert!(err.to_string().contains("background task failed"));
    }
}
