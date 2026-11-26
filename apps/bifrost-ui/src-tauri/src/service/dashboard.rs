//! Dashboard snapshot, Heimdall control, diagnostics and configuration commands.

use super::*;

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

pub(super) static DOCTOR_CACHE: OnceLock<Mutex<Option<DoctorReport>>> = OnceLock::new();

pub(super) static AUTOSTART_CACHE: OnceLock<Mutex<Option<bool>>> = OnceLock::new();

pub(super) fn cached_doctor(config: &BifrostConfig, data_dir: &std::path::Path) -> DoctorReport {
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

pub(super) fn cached_autostart() -> bool {
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

/// Collects a full real-time Dashboard snapshot from core, diagnostics, Heimdall, and Bridge.
pub async fn get_dashboard_snapshot() -> Result<DashboardSnapshot> {
    let session = session().await?;
    let LoadedConfig {
        config_path,
        config,
        paths,
        ..
    } = session.loaded().clone();
    let heimdall_connected = session.heimdall_connected();
    let StatusSnapshot {
        record: runtime,
        runtime_state,
        heimdall_health,
        adb_access,
    } = session.status().await?;

    let active_manifest_path = ImageRegistry::load(&paths.images_dir)
        .ok()
        .and_then(|r| r.active_image().map(|img| img.manifest_path.clone()))
        .or_else(|| runtime.manifest_path.clone());

    let devices = if heimdall_connected {
        session
            .heimdall()
            .send::<Vec<AdbDevice>>(IpcRequest::DevicesList)
            .await
            .unwrap_or_default()
    } else {
        blocking(|| AdbClient::new().list_devices())
            .await
            .unwrap_or_default()
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
    let started = session().await?.start_heimdall(None).await?;
    Ok(started.health().clone())
}

/// Requests graceful Heimdall daemon shutdown over authenticated IPC.
pub async fn stop_heimdall() -> Result<()> {
    let _: serde_json::Value = session()
        .await?
        .heimdall()
        .send(IpcRequest::DaemonShutdown)
        .await?;
    Ok(())
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
