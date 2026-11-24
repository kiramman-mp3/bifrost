//! Pure Rust backend service layer for `bifrost-ui`.
//!
//! All runtime supervision, configuration validation, diagnostics, ADB, and Valhalla
//! logic lives strictly in Rust here; the WebView frontend is purely presentational.

use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

use bifrost_bridge::{
    read_host_clipboard_text, AdbClient, AdbDevice, AdbEndpointState, DaemonHealthReport,
    IpcRequest, SharedDirectoryBridge, SharedFileEntry,
};
use bifrost_client::{blocking, LoadedConfig, Session, StatusSnapshot};
use bifrost_core::{
    validate_host_file_path, BifrostConfig, BifrostError, BifrostPaths, Result, RuntimeState,
    VersionInfo,
};
use bifrost_diagnostics::{run_doctor, DoctorReport};
use bifrost_display::{AppWindowHandle, AppWindowSpec, DisplayController};
use bifrost_network::{NetworkManager, PortForwardRule};
use bifrost_process::{AndroidProcess, ProcessManager};
use bifrost_runtime::{MidgardRuntime, RuntimeLogs, RuntimeRecord};
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
    let loaded = LoadedConfig::load(None)?;
    Ok((loaded.config_path, loaded.config, loaded.paths))
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

/// Opens a facade session on the per-user config (one Heimdall probe per command).
async fn session() -> Result<Session> {
    Session::open(None).await
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

/// Starts Midgard via Heimdall IPC (if online) or directly via `MidgardRuntime`.
pub async fn start_runtime(manifest_path: PathBuf, headless: bool) -> Result<RuntimeRecord> {
    session()
        .await?
        .start_runtime(manifest_path, headless, false)
        .await
}

/// Stops Midgard cleanly (or forcefully) via Heimdall IPC or `MidgardRuntime`.
pub async fn stop_runtime(force: bool) -> Result<RuntimeRecord> {
    session().await?.stop_runtime(force).await
}

/// Restarts Midgard via Heimdall IPC or `MidgardRuntime`.
pub async fn restart_runtime(
    manifest_path: Option<PathBuf>,
    headless: bool,
) -> Result<RuntimeRecord> {
    session()
        .await?
        .restart_runtime(manifest_path, headless)
        .await
}

/// Reads recent Midgard serial and VMM stderr logs.
pub async fn get_logs(tail: usize) -> Result<RuntimeLogs> {
    session().await?.logs(tail).await
}

/// Lists installed Android packages via Valhalla (or Heimdall IPC).
pub async fn list_apps(third_party_only: bool) -> Result<Vec<InstalledPackage>> {
    session().await?.list_apps(None, third_party_only).await
}

/// Inspects detailed package metadata via Valhalla.
pub async fn get_app_info(package: String) -> Result<PackageDetails> {
    session().await?.app_info(None, package).await
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

    let session = session().await?;
    if blocking(|| ValhallaManager::new().resolve_target_serial(None))
        .await
        .is_err()
    {
        let inspect_path = safe_apk.clone();
        let inspection = blocking(move || inspect_apk(&inspect_path)).await?;
        if !inspection.abi_compatibility.is_supported() {
            return Err(BifrostError::UnsupportedAbi {
                path: inspection.path,
                detected_abis: inspection.detected_abis.join(", "),
            });
        }
        session
            .ensure_device_ready(None, true, || {
                on_progress(InstallProgressEvent {
                    percent: 30,
                    stage: "Starting Midgard Android subsystem in headless mode…".to_string(),
                    indeterminate: true,
                });
            })
            .await?;
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

    let outcome = session.install_apk(None, safe_apk).await?;

    on_progress(InstallProgressEvent {
        percent: 100,
        stage: "Installed and integrated with Windows 11 Desktop & Start Menu!".to_string(),
        indeterminate: false,
    });

    Ok(outcome)
}

/// Uninstalls an Android package via Valhalla.
pub async fn uninstall_app(package: String, keep_data: bool) -> Result<String> {
    session()
        .await?
        .uninstall_app(None, package, keep_data)
        .await
}

/// Launches an installed Android application via Valhalla.
pub async fn launch_app(package: String) -> Result<LaunchOutcome> {
    session().await?.launch_app(None, package).await
}

/// Launches an Android app in a dedicated WSA-style resizable Windows 11 window,
/// automatically starting Midgard headlessly if it is currently stopped.
pub async fn launch_app_window(
    package: String,
    width: Option<u32>,
    height: Option<u32>,
    dpi: Option<u32>,
) -> Result<AppWindowHandle> {
    let session = session().await?;
    let target = session.ensure_device_ready(None, true, || {}).await?;
    let display = &session.config().display;
    let spec = AppWindowSpec {
        width: width.unwrap_or(display.width),
        height: height.unwrap_or(display.height),
        dpi: dpi.unwrap_or(display.dpi),
    };
    blocking(move || DisplayController::new().launch_in_independent_window(&target, &package, spec))
        .await
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
    session().await?.stop_app(None, package).await
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
