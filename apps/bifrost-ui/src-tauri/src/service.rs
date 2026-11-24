//! Pure Rust backend service layer for `bifrost-ui`.
//!
//! All runtime supervision, configuration validation, diagnostics, ADB, and Valhalla
//! logic lives strictly in Rust here; the WebView frontend is purely presentational.

use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

use bifrost_bridge::{
    default_pipe_name, read_host_clipboard_text, AdbClient, AdbDevice, AdbEndpointState,
    DaemonHealthReport, HeimdallClient, IpcRequest, SharedDirectoryBridge, SharedFileEntry,
};
use bifrost_core::{
    validate_host_file_path, BifrostConfig, BifrostError, BifrostPaths, Result, RuntimeState,
    VersionInfo,
};
use bifrost_diagnostics::{run_doctor, DoctorReport};
use bifrost_display::{AppWindowHandle, AppWindowSpec, DisplayController};
use bifrost_network::{NetworkManager, PortForwardRule};
use bifrost_process::{AndroidProcess, ProcessManager};
use bifrost_runtime::{MidgardRuntime, RuntimeLogs, RuntimeRecord, StartOptions};
use bifrost_storage::registry::DEFAULT_USERDATA_COMPAT_VERSION;
use bifrost_storage::{
    backup_userdata, evaluate_manifest_trust, restore_userdata, verify_manifest_file,
    BackupMetadata, ImageRegistry, InstalledImageRecord,
};
use serde::Serialize;
use valhalla::manager::{InstallOutcome, LaunchOutcome};
use valhalla::{
    derive_display_name, inspect_apk, inspect_package_preview, query_heimdall_autostart,
    set_heimdall_autostart as apply_heimdall_autostart, ApkInspection, HeimdallAutostartStatus,
    InstalledPackage, PackageDetails, ShortcutManager, ShortcutRecord, ValhallaManager,
};

/// Preview metadata rendered by the Windows 11 App Installer confirmation dialog.
#[derive(Debug, Serialize)]
pub struct PackageInstallPreview {
    pub path: PathBuf,
    pub display_name: String,
    pub package_name: Option<String>,
    pub new_version: Option<String>,
    pub installed_version: Option<String>,
    pub is_update: bool,
    pub size_bytes: u64,
    pub sha256: String,
    pub is_split_bundle: bool,
    pub split_apk_count: usize,
    pub abi_supported: bool,
    pub detected_abis: Vec<String>,
    pub icon_data_url: Option<String>,
}

/// Real-time progress update emitted during `.apk` / `.apkm` installation.
#[derive(Debug, Clone, Serialize)]
pub struct InstallProgressEvent {
    pub percent: u8,
    pub stage: String,
    pub indeterminate: bool,
}

/// Aggregated Dashboard snapshot served to the UI.
#[derive(Debug, Serialize)]
pub struct DashboardSnapshot {
    pub version: VersionInfo,
    pub runtime_state: RuntimeState,
    pub heimdall_connected: bool,
    pub heimdall_health: Option<DaemonHealthReport>,
    pub heimdall_autostart: bool,
    pub runtime: RuntimeRecord,
    /// Why ADB cannot reach the active guest, as an actionable sentence, if it cannot.
    pub adb_access_hint: Option<String>,
    pub active_manifest_path: Option<PathBuf>,
    pub config: BifrostConfig,
    pub config_path: PathBuf,
    pub paths: BifrostPaths,
    pub doctor: DoctorReport,
    pub devices: Vec<AdbDevice>,
    pub shared_file_count: usize,
    pub clipboard_has_text: bool,
}

/// Resolves effective per-user BIFRÖST config and storage paths.
pub fn load_config_and_paths() -> Result<(PathBuf, BifrostConfig, BifrostPaths)> {
    let mut paths = BifrostPaths::discover()?;
    let config_path = paths.default_config_file();
    let config = if config_path.exists() {
        BifrostConfig::load_from_file(&config_path)?
    } else {
        BifrostConfig::default()
    };

    if let Some(ref custom_data_dir) = config.runtime.data_dir {
        paths.data_dir = custom_data_dir.clone();
        paths.images_dir = custom_data_dir.join("images");
        paths.logs_dir = custom_data_dir.join("logs");
    }

    Ok((config_path, config, paths))
}

static DOCTOR_CACHE: OnceLock<Mutex<Option<DoctorReport>>> = OnceLock::new();
static AUTOSTART_CACHE: OnceLock<Mutex<Option<bool>>> = OnceLock::new();

fn cached_doctor(config: &BifrostConfig, data_dir: &std::path::Path) -> DoctorReport {
    let lock = DOCTOR_CACHE.get_or_init(|| Mutex::new(None));
    if let Ok(guard) = lock.lock() {
        if let Some(ref cached) = *guard {
            return cached.clone();
        }
    }
    let fresh = run_doctor(config, data_dir);
    if let Ok(mut guard) = lock.lock() {
        *guard = Some(fresh.clone());
    }
    fresh
}

fn cached_autostart() -> bool {
    let lock = AUTOSTART_CACHE.get_or_init(|| Mutex::new(None));
    if let Ok(guard) = lock.lock() {
        if let Some(val) = *guard {
            return val;
        }
    }
    let val = query_heimdall_autostart()
        .map(|s| s.enabled)
        .unwrap_or(false);
    if let Ok(mut guard) = lock.lock() {
        *guard = Some(val);
    }
    val
}

fn heimdall_client(paths: &BifrostPaths) -> HeimdallClient {
    HeimdallClient::new(default_pipe_name(), &paths.data_dir)
}

/// Collects a full real-time Dashboard snapshot from core, diagnostics, Heimdall, and Bridge.
pub async fn get_dashboard_snapshot() -> Result<DashboardSnapshot> {
    let (config_path, config, paths) = load_config_and_paths()?;
    let client = heimdall_client(&paths);
    let heimdall_health = client.health().await.ok();
    let heimdall_connected = heimdall_health.is_some();

    let mut runtime = if heimdall_connected {
        client
            .send::<RuntimeRecord>(IpcRequest::RuntimeStatus)
            .await?
    } else {
        MidgardRuntime::with_default_backend(config.clone(), paths.clone()).status()?
    };

    let mut adb_access = heimdall_health.as_ref().and_then(|h| h.adb_access);
    if !heimdall_connected && runtime.state == RuntimeState::Booting {
        if let Some(ref endpoint) = runtime.adb_endpoint {
            let adb = AdbClient::new();
            adb_access = adb.diagnose_endpoint(endpoint).ok();
            if adb_access == Some(AdbEndpointState::Ready)
                && adb
                    .probe_guest_health(endpoint)
                    .is_ok_and(|g| g.boot_completed)
            {
                let _ = MidgardRuntime::with_default_backend(config.clone(), paths.clone())
                    .mark_running();
                runtime.state = RuntimeState::Running;
            }
        }
    }

    let active_manifest_path = ImageRegistry::load(&paths.images_dir)
        .ok()
        .and_then(|r| r.active_image().map(|img| img.manifest_path.clone()))
        .or_else(|| runtime.manifest_path.clone());

    let runtime_state = heimdall_health
        .as_ref()
        .map(|h| h.runtime_state)
        .unwrap_or(runtime.state);

    let devices = if heimdall_connected {
        client
            .send::<Vec<AdbDevice>>(IpcRequest::DevicesList)
            .await
            .unwrap_or_default()
    } else {
        AdbClient::new().list_devices().unwrap_or_default()
    };

    let doctor = cached_doctor(&config, &paths.data_dir);
    let share_root = paths.data_dir.join("shared");
    let shared_file_count = SharedDirectoryBridge::new(share_root, AdbClient::new())
        .list_host_files()
        .map(|v| v.len())
        .unwrap_or(0);
    let clipboard_has_text = read_host_clipboard_text().ok().flatten().is_some();
    let heimdall_autostart = cached_autostart();

    Ok(DashboardSnapshot {
        version: VersionInfo::current(),
        runtime_state,
        heimdall_connected,
        heimdall_health,
        heimdall_autostart,
        adb_access_hint: adb_access
            .and_then(AdbEndpointState::remediation)
            .map(str::to_string),
        runtime,
        active_manifest_path,
        config,
        config_path,
        paths,
        doctor,
        devices,
        shared_file_count,
        clipboard_has_text,
    })
}

/// Enables or disables unprivileged Windows login autostart for Heimdall.
pub fn set_heimdall_autostart(enabled: bool) -> Result<HeimdallAutostartStatus> {
    let status = apply_heimdall_autostart(enabled)?;
    let lock = AUTOSTART_CACHE.get_or_init(|| Mutex::new(None));
    if let Ok(mut guard) = lock.lock() {
        *guard = Some(status.enabled);
    }
    Ok(status)
}

/// Starts the Heimdall background supervisor daemon cleanly detached without a console window.
pub async fn start_heimdall() -> Result<DaemonHealthReport> {
    let (_, _, paths) = load_config_and_paths()?;
    let client = heimdall_client(&paths);
    if let Ok(health) = client.health().await {
        return Ok(health);
    }

    let heimdall_bin = locate_heimdall_bin()?;
    let mut cmd = std::process::Command::new(&heimdall_bin);
    cmd.arg("run")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NEW_PROCESS_GROUP | CREATE_NO_WINDOW);
    }
    let _ = cmd.spawn().map_err(|source| BifrostError::Io {
        context: format!("spawning Heimdall daemon '{}'", heimdall_bin.display()),
        source,
    })?;

    for _ in 0..20 {
        tokio::time::sleep(std::time::Duration::from_millis(250)).await;
        if let Ok(health) = client.health().await {
            return Ok(health);
        }
    }
    Err(BifrostError::Ipc(
        "Heimdall daemon did not become reachable over IPC within 5s".to_string(),
    ))
}

/// Requests graceful Heimdall daemon shutdown over authenticated IPC.
pub async fn stop_heimdall() -> Result<()> {
    let (_, _, paths) = load_config_and_paths()?;
    let client = heimdall_client(&paths);
    let _: serde_json::Value = client.send(IpcRequest::DaemonShutdown).await?;
    Ok(())
}

fn locate_heimdall_bin() -> Result<PathBuf> {
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            for name in ["heimdall.exe", "heimdall"] {
                let candidate = dir.join(name);
                if candidate.is_file() {
                    return Ok(candidate);
                }
            }
        }
    }
    if let Ok(local) = std::env::var("LOCALAPPDATA") {
        let installed = PathBuf::from(local)
            .join("Programs")
            .join("BIFRÖST")
            .join("heimdall.exe");
        if installed.is_file() {
            return Ok(installed);
        }
    }
    Err(BifrostError::Runtime(
        "heimdall.exe not found alongside bifrost-ui.exe".to_string(),
    ))
}

/// Runs fresh native Windows 11 compatibility checks (`bifrost doctor`) and refreshes cache.
pub fn get_doctor_report() -> Result<DoctorReport> {
    let (_, config, paths) = load_config_and_paths()?;
    let report = run_doctor(&config, &paths.data_dir);
    let lock = DOCTOR_CACHE.get_or_init(|| Mutex::new(None));
    if let Ok(mut guard) = lock.lock() {
        *guard = Some(report.clone());
    }
    Ok(report)
}

/// Computes and persists host-optimal CPU and RAM allocations into `bifrost.toml`.
pub fn auto_tune_config() -> Result<BifrostConfig> {
    let (config_path, mut config, paths) = load_config_and_paths()?;
    let doctor = cached_doctor(&config, &paths.data_dir);
    let (cpu_cores, memory_mb) = bifrost_core::config::RuntimeConfig::recommend_for_host(
        doctor.snapshot.cpu.logical_cores,
        doctor.snapshot.memory.total_bytes,
    );
    config.runtime.cpu_cores = cpu_cores;
    config.runtime.memory_mb = memory_mb;
    config.save_to_file(&config_path, true)?;
    Ok(config)
}

/// Validates and persists updated configuration to `bifrost.toml`.
pub fn save_config(new_config: BifrostConfig) -> Result<PathBuf> {
    new_config.validate()?;
    let paths = BifrostPaths::discover()?;
    let config_path = paths.default_config_file();
    new_config.save_to_file(&config_path, true)?;
    Ok(config_path)
}

/// Starts Midgard via Heimdall IPC (if online) or directly via `MidgardRuntime`.
pub async fn start_runtime(manifest_path: PathBuf, headless: bool) -> Result<RuntimeRecord> {
    let (_, config, paths) = load_config_and_paths()?;
    let client = heimdall_client(&paths);
    if client.health().await.is_ok() {
        return client
            .send(IpcRequest::RuntimeStart {
                manifest_path,
                headless,
            })
            .await;
    }
    MidgardRuntime::with_default_backend(config, paths).start(
        manifest_path,
        StartOptions {
            headless,
            interactive_console: false,
        },
    )
}

/// Stops Midgard cleanly (or forcefully) via Heimdall IPC or `MidgardRuntime`.
pub async fn stop_runtime(force: bool) -> Result<RuntimeRecord> {
    let (_, config, paths) = load_config_and_paths()?;
    let client = heimdall_client(&paths);
    if client.health().await.is_ok() {
        return client.send(IpcRequest::RuntimeStop { force }).await;
    }
    MidgardRuntime::with_default_backend(config, paths).stop(force)
}

/// Restarts Midgard via Heimdall IPC or `MidgardRuntime`.
pub async fn restart_runtime(
    manifest_path: Option<PathBuf>,
    headless: bool,
) -> Result<RuntimeRecord> {
    let (_, config, paths) = load_config_and_paths()?;
    let client = heimdall_client(&paths);
    if client.health().await.is_ok() {
        return client
            .send(IpcRequest::RuntimeRestart {
                manifest_path,
                headless,
            })
            .await;
    }
    MidgardRuntime::with_default_backend(config, paths).restart(
        manifest_path.as_deref(),
        StartOptions {
            headless,
            interactive_console: false,
        },
    )
}

/// Reads recent Midgard serial and VMM stderr logs.
pub async fn get_logs(tail: usize) -> Result<RuntimeLogs> {
    let (_, config, paths) = load_config_and_paths()?;
    let client = heimdall_client(&paths);
    if let Ok(logs) = client
        .send::<RuntimeLogs>(IpcRequest::RuntimeLogs { tail })
        .await
    {
        return Ok(logs);
    }
    MidgardRuntime::with_default_backend(config, paths).read_logs(tail)
}

/// Lists installed Android packages via Valhalla (or Heimdall IPC).
pub async fn list_apps(third_party_only: bool) -> Result<Vec<InstalledPackage>> {
    let (_, _, paths) = load_config_and_paths()?;
    let client = heimdall_client(&paths);
    if client.health().await.is_ok() {
        return client
            .send(IpcRequest::AppsList {
                serial: None,
                third_party_only,
            })
            .await;
    }
    ValhallaManager::new().list_packages(None, third_party_only)
}

/// Inspects detailed package metadata via Valhalla.
pub async fn get_app_info(package: String) -> Result<PackageDetails> {
    let (_, _, paths) = load_config_and_paths()?;
    let client = heimdall_client(&paths);
    if client.health().await.is_ok() {
        return client
            .send(IpcRequest::AppInfo {
                serial: None,
                package,
            })
            .await;
    }
    ValhallaManager::new().package_info(None, &package)
}

/// Performs offline APK ZIP, SHA-256, and ADR-0004 `x86_64` ABI inspection.
pub fn inspect_apk_file(path: PathBuf) -> Result<ApkInspection> {
    inspect_apk(path)
}

/// Inspects an `.apk` or `.apkm` package, extracts its PNG icon, and checks whether the
/// application is already installed on Android so the UI can show Install vs. Update.
pub async fn preview_package_install(apk_path: PathBuf) -> Result<PackageInstallPreview> {
    let safe_apk = valhalla::validate_package_file_path(&apk_path)?;
    let (inspection, icon_png) =
        tokio::task::spawn_blocking(move || inspect_package_preview(&safe_apk))
            .await
            .map_err(|err| {
                BifrostError::Runtime(format!("package preview task failed: {err}"))
            })??;

    let display_name = derive_display_name(
        inspection.package_name.as_deref().unwrap_or("app"),
        inspection.app_name.as_deref(),
        Some(&inspection.path),
    );
    let icon_data_url = icon_png
        .as_deref()
        .map(|bytes| format!("data:image/png;base64,{}", encode_base64(bytes)));

    let mut is_update = false;
    let mut installed_version = None;
    if let Some(ref pkg) = inspection.package_name {
        if let Ok(details) = get_app_info(pkg.clone()).await {
            is_update = true;
            installed_version = details.version_name;
        }
    }

    Ok(PackageInstallPreview {
        path: inspection.path,
        display_name,
        package_name: inspection.package_name,
        new_version: inspection.version_name,
        installed_version,
        is_update,
        size_bytes: inspection.size_bytes,
        sha256: inspection.sha256,
        is_split_bundle: inspection.is_split_bundle,
        split_apk_count: inspection.split_apk_count,
        abi_supported: inspection.abi_compatibility.is_supported(),
        detected_abis: inspection.detected_abis,
        icon_data_url,
    })
}

fn encode_base64(input: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(input.len().div_ceil(3) * 4);
    for chunk in input.chunks(3) {
        let b0 = u32::from(chunk[0]);
        let b1 = if chunk.len() > 1 {
            u32::from(chunk[1])
        } else {
            0
        };
        let b2 = if chunk.len() > 2 {
            u32::from(chunk[2])
        } else {
            0
        };
        let n = (b0 << 16) | (b1 << 8) | b2;
        out.push(TABLE[((n >> 18) & 0x3F) as usize] as char);
        out.push(TABLE[((n >> 12) & 0x3F) as usize] as char);
        out.push(if chunk.len() > 1 {
            TABLE[((n >> 6) & 0x3F) as usize] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            TABLE[(n & 0x3F) as usize] as char
        } else {
            '='
        });
    }
    out
}

/// Installs an inspected `.apk` or `.apkm` split bundle via Valhalla / Android `PackageManager`,
/// emitting live progress stages and automatically starting Midgard headlessly if stopped.
pub async fn install_apk_file_with_progress<F>(
    apk_path: PathBuf,
    mut on_progress: F,
) -> Result<InstallOutcome>
where
    F: FnMut(InstallProgressEvent),
{
    let safe_apk = valhalla::validate_package_file_path(&apk_path)?;
    on_progress(InstallProgressEvent {
        percent: 12,
        stage: "Validating package signature, ZIP integrity, and x86_64 ABI…".to_string(),
        indeterminate: false,
    });

    let (_, config, paths) = load_config_and_paths()?;
    let client = heimdall_client(&paths);
    let heimdall_up = client.health().await.is_ok();

    if ValhallaManager::new().resolve_target_serial(None).is_err() {
        let inspect_path = safe_apk.clone();
        let inspection = tokio::task::spawn_blocking(move || inspect_apk(&inspect_path))
            .await
            .map_err(|err| BifrostError::Runtime(format!("inspection task failed: {err}")))??;
        if !inspection.abi_compatibility.is_supported() {
            return Err(BifrostError::UnsupportedAbi {
                path: inspection.path,
                detected_abis: inspection.detected_abis.join(", "),
            });
        }
        let rt = MidgardRuntime::with_default_backend(config.clone(), paths.clone());
        if let Ok(manifest) = rt.resolve_default_manifest() {
            on_progress(InstallProgressEvent {
                percent: 30,
                stage: "Starting Midgard Android subsystem in headless mode…".to_string(),
                indeterminate: true,
            });
            if !rt.status()?.state.is_active() {
                if heimdall_up {
                    let _: RuntimeRecord = client
                        .send(IpcRequest::RuntimeStart {
                            manifest_path: manifest,
                            headless: true,
                        })
                        .await?;
                } else {
                    let _ = rt.ensure_started_headless()?;
                }
            }
            let endpoint = format!(
                "{}:{}",
                config.network.adb_bind_address, config.network.adb_port
            );
            let wait_endpoint = endpoint.clone();
            let _ = tokio::task::spawn_blocking(move || {
                DisplayController::new()
                    .wait_for_device_ready(&wait_endpoint, std::time::Duration::from_secs(90))
            })
            .await
            .map_err(|err| BifrostError::Runtime(format!("boot wait task failed: {err}")))??;
        }
    }

    let is_apkm = valhalla::is_apkm_bundle(&safe_apk);
    on_progress(InstallProgressEvent {
        percent: 62,
        stage: if is_apkm {
            "Extracting x86_64 splits and installing via Android PackageManager…".to_string()
        } else {
            "Installing package via Android PackageManager…".to_string()
        },
        indeterminate: true,
    });

    let outcome = if heimdall_up {
        client
            .send(IpcRequest::AppInstall {
                serial: None,
                apk_path: safe_apk,
            })
            .await?
    } else {
        tokio::task::spawn_blocking(move || ValhallaManager::new().install_apk(None, safe_apk))
            .await
            .map_err(|err| BifrostError::Runtime(format!("install task failed: {err}")))??
    };

    on_progress(InstallProgressEvent {
        percent: 100,
        stage: "Installed and integrated with Windows 11 Desktop & Start Menu!".to_string(),
        indeterminate: false,
    });

    Ok(outcome)
}

/// Uninstalls an Android package via Valhalla.
pub async fn uninstall_app(package: String, keep_data: bool) -> Result<String> {
    let (_, _, paths) = load_config_and_paths()?;
    let client = heimdall_client(&paths);
    if client.health().await.is_ok() {
        let resp: serde_json::Value = client
            .send(IpcRequest::AppUninstall {
                serial: None,
                package,
                keep_data,
            })
            .await?;
        return Ok(resp["output"].as_str().unwrap_or("Success").to_string());
    }
    ValhallaManager::new().uninstall_package(None, &package, keep_data)
}

/// Launches an installed Android application via Valhalla.
pub async fn launch_app(package: String) -> Result<LaunchOutcome> {
    let (_, _, paths) = load_config_and_paths()?;
    let client = heimdall_client(&paths);
    if client.health().await.is_ok() {
        return client
            .send(IpcRequest::AppLaunch {
                serial: None,
                package,
            })
            .await;
    }
    ValhallaManager::new().launch_app(None, &package)
}

/// Launches an Android app in a dedicated WSA-style resizable Windows 11 window,
/// automatically starting Midgard headlessly if it is currently stopped.
pub async fn launch_app_window(
    package: String,
    width: Option<u32>,
    height: Option<u32>,
    dpi: Option<u32>,
) -> Result<AppWindowHandle> {
    let (_, config, paths) = load_config_and_paths()?;
    let ctrl = DisplayController::new();
    let valhalla = ValhallaManager::new();
    let target = match valhalla.resolve_target_serial(None) {
        Ok(serial) => serial,
        Err(_) => {
            let adb_endpoint = format!(
                "{}:{}",
                config.network.adb_bind_address, config.network.adb_port
            );
            let rt = MidgardRuntime::with_default_backend(config.clone(), paths.clone());
            if !rt.status()?.state.is_active() {
                let manifest = rt.resolve_default_manifest()?;
                let client = heimdall_client(&paths);
                if client.health().await.is_ok() {
                    let _: RuntimeRecord = client
                        .send(IpcRequest::RuntimeStart {
                            manifest_path: manifest,
                            headless: true,
                        })
                        .await?;
                } else {
                    let _ = rt.ensure_started_headless()?;
                }
            }
            ctrl.wait_for_device_ready(&adb_endpoint, std::time::Duration::from_secs(90))?
        }
    };

    let spec = AppWindowSpec {
        width: width.unwrap_or(config.display.width),
        height: height.unwrap_or(config.display.height),
        dpi: dpi.unwrap_or(config.display.dpi),
    };
    ctrl.launch_in_independent_window(&target, &package, spec)
}

/// Lists all recorded Windows Desktop / Start Menu shortcuts for BIFRÖST Android apps.
pub fn list_app_shortcuts() -> Result<Vec<ShortcutRecord>> {
    let (_, _, paths) = load_config_and_paths()?;
    ShortcutManager::new(paths.data_dir).list_shortcuts()
}

/// Creates Windows Desktop and Start Menu shortcuts for an installed Android package.
pub fn create_app_shortcut(
    package: String,
    name: Option<String>,
    apk_path: Option<PathBuf>,
) -> Result<ShortcutRecord> {
    let (_, _, paths) = load_config_and_paths()?;
    ShortcutManager::new(paths.data_dir).create_for_app(
        &package,
        name.as_deref(),
        apk_path.as_deref(),
    )
}

/// Removes Windows Desktop and Start Menu shortcuts for an Android package.
pub fn remove_app_shortcut(package: String) -> Result<bool> {
    let (_, _, paths) = load_config_and_paths()?;
    Ok(ShortcutManager::new(paths.data_dir)
        .remove_for_app(&package)?
        .is_some())
}

/// Force-stops an Android application via Valhalla.
pub async fn stop_app(package: String) -> Result<()> {
    let (_, _, paths) = load_config_and_paths()?;
    let client = heimdall_client(&paths);
    if client.health().await.is_ok() {
        let _: serde_json::Value = client
            .send(IpcRequest::AppStop {
                serial: None,
                package,
            })
            .await?;
        return Ok(());
    }
    ValhallaManager::new().force_stop(None, &package)
}

/// Lists host files in the BIFRÖST shared folder.
pub fn list_shared_files() -> Result<Vec<SharedFileEntry>> {
    let (_, _, paths) = load_config_and_paths()?;
    let share_root = paths.data_dir.join("shared");
    SharedDirectoryBridge::new(share_root, AdbClient::new()).list_host_files()
}

/// Lists running Android processes (empty vector if no online Android device).
pub fn list_processes(filter: Option<String>) -> Result<Vec<AndroidProcess>> {
    let mgr = ProcessManager::new();
    let serial = mgr.resolve_serial(None)?;
    mgr.list_processes(&serial, filter.as_deref())
}

/// Lists active loopback ADB TCP port forwards.
pub fn list_network_forwards() -> Result<Vec<PortForwardRule>> {
    NetworkManager::new().list_forwards(None)
}

/// Lists all registered Midgard runtime image bundles and the active image ID.
pub fn list_images() -> Result<ImageRegistry> {
    let (_, _, paths) = load_config_and_paths()?;
    ImageRegistry::load(&paths.images_dir)
}

/// Verifies a Midgard `manifest.json` bundle (trust policy + SHA-256) and registers it.
///
/// A detached `manifest.json.sig` must verify against `[images] trusted_keys`; unsigned
/// bundles are only accepted when the user explicitly ticks "allow unsigned".
pub fn register_image(
    manifest_path: PathBuf,
    allow_unsigned: bool,
) -> Result<InstalledImageRecord> {
    let (_, config, paths) = load_config_and_paths()?;
    let safe_manifest = validate_host_file_path(&manifest_path, Some("json"))?;
    let trust = evaluate_manifest_trust(
        &safe_manifest,
        None,
        &config.images.trusted_keys,
        allow_unsigned,
    )?;
    let bundle = verify_manifest_file(&safe_manifest)?;
    let mut registry = ImageRegistry::load(&paths.images_dir)?;
    let record =
        registry.register_bundle(&bundle, trust.is_signed(), DEFAULT_USERDATA_COMPAT_VERSION)?;
    registry.save(&paths.images_dir)?;
    Ok(record)
}

/// Switches the active Midgard runtime version after verifying upgrade compatibility.
pub fn activate_image(image_id: String) -> Result<InstalledImageRecord> {
    let (_, _, paths) = load_config_and_paths()?;
    let mut registry = ImageRegistry::load(&paths.images_dir)?;
    let activated = registry.activate(&image_id)?;
    registry.save(&paths.images_dir)?;
    Ok(activated)
}

fn default_userdata_path(paths: &BifrostPaths) -> PathBuf {
    paths
        .data_dir
        .join("instances")
        .join("default")
        .join("userdata.img")
}

/// Creates a SHA-256-verified backup of persistent `userdata.img` inside `output_dir`.
pub fn backup_userdata_to(output_dir: PathBuf) -> Result<BackupMetadata> {
    let (_, _, paths) = load_config_and_paths()?;
    let userdata_path = default_userdata_path(&paths);
    backup_userdata(&userdata_path, &output_dir)
}

/// Verifies backup SHA-256 and restores `userdata.img` from `from_dir`.
///
/// Refuses to overwrite live `userdata.img` while Midgard VM is actively running.
pub fn restore_userdata_from(from_dir: PathBuf) -> Result<BackupMetadata> {
    let (_, config, paths) = load_config_and_paths()?;
    let rt = MidgardRuntime::with_default_backend(config, paths.clone());
    if rt.status()?.state.is_active() {
        return Err(BifrostError::Runtime(
            "stop the running Midgard VM before restoring a userdata backup".to_string(),
        ));
    }
    let userdata_path = default_userdata_path(&paths);
    restore_userdata(&from_dir, &userdata_path, true)
}

/// Opens a native Windows file picker dialog without blocking the async runtime.
pub async fn pick_file_dialog(title: String, filter: String) -> Result<Option<PathBuf>> {
    tokio::task::spawn_blocking(move || run_native_file_dialog(&title, &filter))
        .await
        .map_err(|err| BifrostError::Runtime(format!("file picker task failed: {err}")))?
}

/// Opens a native Windows folder picker dialog without blocking the async runtime.
pub async fn pick_folder_dialog(title: String) -> Result<Option<PathBuf>> {
    tokio::task::spawn_blocking(move || run_native_folder_dialog(&title))
        .await
        .map_err(|err| BifrostError::Runtime(format!("folder picker task failed: {err}")))?
}

/// Parses a .NET-style dialog filter (`"Label|*.a;*.b|All files|*.*"`) into
/// `(label, extensions)` pairs for the native dialog. `*.*` maps to `*`.
#[cfg_attr(not(windows), allow(dead_code))]
fn parse_dialog_filter(filter: &str) -> Vec<(String, Vec<String>)> {
    let parts: Vec<&str> = filter.split('|').collect();
    parts
        .chunks(2)
        .filter_map(|pair| {
            let [label, patterns] = pair else {
                return None;
            };
            let extensions: Vec<String> = patterns
                .split(';')
                .filter_map(|pat| {
                    let pat = pat.trim();
                    if pat == "*.*" || pat == "*" {
                        Some("*".to_string())
                    } else {
                        pat.strip_prefix("*.")
                            .filter(|ext| !ext.is_empty())
                            .map(str::to_string)
                    }
                })
                .collect();
            (!label.trim().is_empty() && !extensions.is_empty())
                .then(|| (label.trim().to_string(), extensions))
        })
        .collect()
}

/// Shows the native Windows `IFileOpenDialog` (through `rfd`, no scripts or child process).
#[cfg(windows)]
fn run_native_file_dialog(title: &str, filter: &str) -> Result<Option<PathBuf>> {
    let mut dialog = rfd::FileDialog::new().set_title(title);
    for (label, extensions) in parse_dialog_filter(filter) {
        dialog = dialog.add_filter(label, &extensions);
    }
    Ok(dialog.pick_file())
}

/// Shows the native Windows folder picker (through `rfd`).
#[cfg(windows)]
fn run_native_folder_dialog(title: &str) -> Result<Option<PathBuf>> {
    Ok(rfd::FileDialog::new().set_title(title).pick_folder())
}

#[cfg(not(windows))]
fn run_native_file_dialog(_title: &str, _filter: &str) -> Result<Option<PathBuf>> {
    Ok(None)
}

#[cfg(not(windows))]
fn run_native_folder_dialog(_title: &str) -> Result<Option<PathBuf>> {
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn dashboard_snapshot_returns_valid_host_diagnostics_and_config() {
        let snap = get_dashboard_snapshot().await.unwrap();
        assert_eq!(snap.version.identifier, "bifrost");
        assert_eq!(snap.doctor.checks.len(), 8);
        assert_eq!(snap.config.network.adb_bind_address, "127.0.0.1");
    }

    #[test]
    fn save_config_rejects_public_adb_binding() {
        let mut bad = BifrostConfig::default();
        bad.network.adb_bind_address = "0.0.0.0".to_string();
        assert!(save_config(bad).is_err());
    }

    #[test]
    fn list_images_loads_registry_cleanly() {
        let registry = list_images().unwrap();
        assert!(registry.images.len() < 10_000);
    }

    #[test]
    fn encodes_base64_for_inline_icon_data_urls() {
        assert_eq!(encode_base64(b""), "");
        assert_eq!(encode_base64(b"f"), "Zg==");
        assert_eq!(encode_base64(b"fo"), "Zm8=");
        assert_eq!(encode_base64(b"foo"), "Zm9v");
        assert_eq!(encode_base64(b"BIFROST"), "QklGUk9TVA==");
    }

    #[test]
    fn parses_dotnet_dialog_filters_for_native_picker() {
        let parsed =
            parse_dialog_filter("Android Package (*.apk;*.apkm)|*.apk;*.apkm|All files (*.*)|*.*");
        assert_eq!(
            parsed,
            vec![
                (
                    "Android Package (*.apk;*.apkm)".to_string(),
                    vec!["apk".to_string(), "apkm".to_string()]
                ),
                ("All files (*.*)".to_string(), vec!["*".to_string()]),
            ]
        );
        assert!(parse_dialog_filter("").is_empty());
        assert!(parse_dialog_filter("Dangling label").is_empty());
    }
}
