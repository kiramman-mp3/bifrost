//! Pure Rust backend service layer for `bifrost-ui`.
//!
//! All runtime supervision, configuration validation, diagnostics, ADB, and Valhalla
//! logic lives strictly in Rust here; the WebView frontend is purely presentational.

use std::path::PathBuf;

use bifrost_bridge::{
    default_pipe_name, read_host_clipboard_text, AdbClient, AdbDevice, DaemonHealthReport,
    HeimdallClient, IpcRequest, SharedDirectoryBridge, SharedFileEntry,
};
use bifrost_core::{BifrostConfig, BifrostError, BifrostPaths, Result, RuntimeState, VersionInfo};
use bifrost_diagnostics::{run_doctor, DoctorReport};
use bifrost_display::{AppWindowHandle, AppWindowSpec, DisplayController};
use bifrost_network::{NetworkManager, PortForwardRule};
use bifrost_process::{AndroidProcess, ProcessManager};
use bifrost_runtime::{MidgardRuntime, RuntimeLogs, RuntimeRecord, StartOptions};
use bifrost_storage::ImageRegistry;
use serde::Serialize;
use valhalla::manager::{InstallOutcome, LaunchOutcome};
use valhalla::{
    inspect_apk, ApkInspection, InstalledPackage, PackageDetails, ShortcutManager, ShortcutRecord,
    ValhallaManager,
};

/// Aggregated Dashboard snapshot served to the UI.
#[derive(Debug, Serialize)]
pub struct DashboardSnapshot {
    pub version: VersionInfo,
    pub runtime_state: RuntimeState,
    pub heimdall_connected: bool,
    pub heimdall_health: Option<DaemonHealthReport>,
    pub runtime: RuntimeRecord,
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

    if !heimdall_connected && runtime.state == RuntimeState::Booting {
        if let Some(ref endpoint) = runtime.adb_endpoint {
            if AdbClient::new()
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

    let doctor = run_doctor(&config, &paths.data_dir);
    let share_root = paths.data_dir.join("shared");
    let shared_file_count = SharedDirectoryBridge::new(share_root, AdbClient::new())
        .list_host_files()
        .map(|v| v.len())
        .unwrap_or(0);
    let clipboard_has_text = read_host_clipboard_text().ok().flatten().is_some();

    Ok(DashboardSnapshot {
        version: VersionInfo::current(),
        runtime_state,
        heimdall_connected,
        heimdall_health,
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

/// Runs native Windows 11 compatibility checks (`bifrost doctor`).
pub fn get_doctor_report() -> Result<DoctorReport> {
    let (_, config, paths) = load_config_and_paths()?;
    Ok(run_doctor(&config, &paths.data_dir))
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

/// Installs an inspected APK via Valhalla / Android `PackageManager`.
pub async fn install_apk_file(apk_path: PathBuf) -> Result<InstallOutcome> {
    let (_, _, paths) = load_config_and_paths()?;
    let client = heimdall_client(&paths);
    if client.health().await.is_ok() {
        return client
            .send(IpcRequest::AppInstall {
                serial: None,
                apk_path,
            })
            .await;
    }
    ValhallaManager::new().install_apk(None, apk_path)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn dashboard_snapshot_returns_valid_host_diagnostics_and_config() {
        let snap = get_dashboard_snapshot().await.unwrap();
        assert_eq!(snap.version.identifier, "bifrost");
        assert_eq!(snap.doctor.checks.len(), 7);
        assert_eq!(snap.config.network.adb_bind_address, "127.0.0.1");
    }

    #[test]
    fn save_config_rejects_public_adb_binding() {
        let mut bad = BifrostConfig::default();
        bad.network.adb_bind_address = "0.0.0.0".to_string();
        assert!(save_config(bad).is_err());
    }
}
