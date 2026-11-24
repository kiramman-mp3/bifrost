//! BIFRÖST Desktop Application (`bifrost-ui`) — Tauri 2 Rust backend entry point.
//!
//! All business logic, configuration validation, diagnostics, and IPC live in
//! the `service` module and workspace crates; this file exposes thin `#[tauri::command]`
//! bindings to the Windows 11 Fluent Design WebView frontend.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
#![forbid(unsafe_code)]

mod service;
mod tray;

use std::path::PathBuf;
use std::sync::Mutex;

use bifrost_bridge::{
    dispatch_windows_toast, AndroidNotification, DaemonHealthReport, SharedFileEntry,
};
use bifrost_client::setup::SetupPlan;
use bifrost_core::config::LoggingConfig;
use bifrost_core::logging::init_logging;
use bifrost_core::BifrostConfig;
use bifrost_diagnostics::DoctorReport;
use bifrost_display::AppWindowHandle;
use bifrost_network::PortForwardRule;
use bifrost_process::AndroidProcess;
use bifrost_runtime::{RuntimeLogs, RuntimeRecord};
use bifrost_storage::{BackupMetadata, ImageRegistry, InstalledImageRecord, TrustedKeyInfo};
use tauri::{Emitter, Manager};
use valhalla::manager::{InstallOutcome, LaunchOutcome};
use valhalla::{
    ApkInspection, HeimdallAutostartStatus, InstalledPackage, PackageDetails, ShortcutRecord,
};

use crate::service::{DashboardSnapshot, PackageInstallPreview};

struct PendingInstallState(Mutex<Option<PathBuf>>);

fn map_err<T>(res: bifrost_core::Result<T>) -> Result<T, String> {
    res.map_err(|err| err.to_string())
}

/// Runs a blocking service call on Tokio's blocking pool. Synchronous Tauri commands
/// execute on the window's main thread, so ADB calls, image hashing or userdata copies
/// there would freeze the UI (and delay events such as hiding to the tray).
async fn off_main_thread<T: Send + 'static>(
    op: impl FnOnce() -> bifrost_core::Result<T> + Send + 'static,
) -> Result<T, String> {
    map_err(bifrost_client::blocking(op).await)
}

#[tauri::command]
async fn get_dashboard() -> Result<DashboardSnapshot, String> {
    map_err(service::get_dashboard_snapshot().await)
}

#[tauri::command]
async fn start_heimdall() -> Result<DaemonHealthReport, String> {
    map_err(service::start_heimdall().await)
}

#[tauri::command]
async fn stop_heimdall() -> Result<(), String> {
    map_err(service::stop_heimdall().await)
}

#[tauri::command]
async fn set_heimdall_autostart(enabled: bool) -> Result<HeimdallAutostartStatus, String> {
    off_main_thread(move || service::set_heimdall_autostart(enabled)).await
}

#[tauri::command]
async fn get_doctor_report() -> Result<DoctorReport, String> {
    off_main_thread(service::get_doctor_report).await
}

#[tauri::command]
async fn save_config(config: BifrostConfig) -> Result<PathBuf, String> {
    off_main_thread(move || service::save_config(config)).await
}

#[tauri::command]
async fn auto_tune_config() -> Result<BifrostConfig, String> {
    off_main_thread(service::auto_tune_config).await
}

#[tauri::command]
async fn start_runtime(manifest_path: PathBuf, headless: bool) -> Result<RuntimeRecord, String> {
    map_err(service::start_runtime(manifest_path, headless).await)
}

#[tauri::command]
async fn stop_runtime(force: bool) -> Result<RuntimeRecord, String> {
    map_err(service::stop_runtime(force).await)
}

#[tauri::command]
async fn restart_runtime(
    manifest_path: Option<PathBuf>,
    headless: bool,
) -> Result<RuntimeRecord, String> {
    map_err(service::restart_runtime(manifest_path, headless).await)
}

#[tauri::command]
async fn get_setup_plan() -> Result<SetupPlan, String> {
    map_err(service::get_setup_plan().await)
}

#[tauri::command]
async fn install_setup_tool(step_id: String) -> Result<SetupPlan, String> {
    map_err(service::install_setup_tool(step_id).await)
}

#[tauri::command]
async fn start_first_boot() -> Result<RuntimeRecord, String> {
    map_err(service::start_first_boot().await)
}

#[tauri::command]
async fn get_logs(tail: usize) -> Result<RuntimeLogs, String> {
    map_err(service::get_logs(tail).await)
}

#[tauri::command]
async fn list_apps(third_party_only: bool) -> Result<Vec<InstalledPackage>, String> {
    map_err(service::list_apps(third_party_only).await)
}

#[tauri::command]
async fn get_app_info(package: String) -> Result<PackageDetails, String> {
    map_err(service::get_app_info(package).await)
}

#[tauri::command]
fn get_pending_install(state: tauri::State<'_, PendingInstallState>) -> Option<PathBuf> {
    state.0.lock().ok().and_then(|g| g.clone())
}

#[tauri::command]
fn close_installer_window(app: tauri::AppHandle) {
    if let Some(win) = app.get_webview_window("main") {
        let _ = win.close();
    }
}

#[tauri::command]
async fn preview_package_install(apk_path: PathBuf) -> Result<PackageInstallPreview, String> {
    map_err(service::preview_package_install(apk_path).await)
}

#[tauri::command]
async fn inspect_apk_file(apk_path: PathBuf) -> Result<ApkInspection, String> {
    off_main_thread(move || service::inspect_apk_file(apk_path)).await
}

#[tauri::command]
async fn install_apk_file(
    app: tauri::AppHandle,
    apk_path: PathBuf,
) -> Result<InstallOutcome, String> {
    map_err(
        service::install_apk_file_with_progress(apk_path, |evt| {
            let _ = app.emit("install-progress", evt);
        })
        .await,
    )
}

#[tauri::command]
async fn uninstall_app(package: String, keep_data: bool) -> Result<String, String> {
    map_err(service::uninstall_app(package, keep_data).await)
}

#[tauri::command]
async fn launch_app(package: String) -> Result<LaunchOutcome, String> {
    map_err(service::launch_app(package).await)
}

#[tauri::command]
async fn launch_app_window(
    package: String,
    width: Option<u32>,
    height: Option<u32>,
    dpi: Option<u32>,
) -> Result<AppWindowHandle, String> {
    map_err(service::launch_app_window(package, width, height, dpi).await)
}

#[tauri::command]
async fn list_app_shortcuts() -> Result<Vec<ShortcutRecord>, String> {
    off_main_thread(service::list_app_shortcuts).await
}

#[tauri::command]
async fn create_app_shortcut(
    package: String,
    name: Option<String>,
    apk_path: Option<PathBuf>,
) -> Result<ShortcutRecord, String> {
    off_main_thread(move || service::create_app_shortcut(package, name, apk_path)).await
}

#[tauri::command]
async fn remove_app_shortcut(package: String) -> Result<bool, String> {
    off_main_thread(move || service::remove_app_shortcut(package)).await
}

#[tauri::command]
async fn stop_app(package: String) -> Result<(), String> {
    map_err(service::stop_app(package).await)
}

#[tauri::command]
async fn list_shared_files() -> Result<Vec<SharedFileEntry>, String> {
    off_main_thread(service::list_shared_files).await
}

#[tauri::command]
async fn list_processes(filter: Option<String>) -> Result<Vec<AndroidProcess>, String> {
    off_main_thread(move || service::list_processes(filter)).await
}

#[tauri::command]
async fn list_network_forwards() -> Result<Vec<PortForwardRule>, String> {
    off_main_thread(service::list_network_forwards).await
}

#[tauri::command]
async fn list_images() -> Result<ImageRegistry, String> {
    off_main_thread(service::list_images).await
}

#[tauri::command]
async fn register_image(
    manifest_path: PathBuf,
    allow_unsigned: bool,
) -> Result<InstalledImageRecord, String> {
    off_main_thread(move || service::register_image(manifest_path, allow_unsigned)).await
}

#[tauri::command]
async fn list_trusted_keys() -> Result<Vec<TrustedKeyInfo>, String> {
    off_main_thread(service::list_trusted_keys).await
}

#[tauri::command]
async fn add_trusted_key(public_key: String) -> Result<Vec<TrustedKeyInfo>, String> {
    off_main_thread(move || service::add_trusted_key(public_key)).await
}

#[tauri::command]
async fn trusted_key_removal_impact(public_key: String) -> Result<Vec<String>, String> {
    off_main_thread(move || service::trusted_key_removal_impact(public_key)).await
}

#[tauri::command]
async fn remove_trusted_key(public_key: String) -> Result<Vec<TrustedKeyInfo>, String> {
    off_main_thread(move || service::remove_trusted_key(public_key)).await
}

#[tauri::command]
async fn activate_image(image_id: String) -> Result<InstalledImageRecord, String> {
    off_main_thread(move || service::activate_image(image_id)).await
}

#[tauri::command]
async fn backup_userdata(output_dir: PathBuf) -> Result<BackupMetadata, String> {
    off_main_thread(move || service::backup_userdata_to(output_dir)).await
}

#[tauri::command]
async fn restore_userdata(from_dir: PathBuf) -> Result<BackupMetadata, String> {
    off_main_thread(move || service::restore_userdata_from(from_dir)).await
}

#[tauri::command]
async fn pick_file_dialog(title: String, filter: String) -> Result<Option<PathBuf>, String> {
    map_err(service::pick_file_dialog(title, filter).await)
}

#[tauri::command]
async fn pick_folder_dialog(title: String) -> Result<Option<PathBuf>, String> {
    map_err(service::pick_folder_dialog(title).await)
}

fn main() {
    init_logging(&LoggingConfig::default());

    // Silent GUI login autostart mode (`bifrost-ui.exe --start-heimdall`):
    // spawns Heimdall cleanly detached with zero console window flash at Windows login.
    let args: Vec<String> = std::env::args().collect();
    if args.len() >= 2 && args[1] == "--start-heimdall" {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("failed creating tokio runtime for Heimdall autostart");
        if rt.block_on(service::start_heimdall()).is_err() {
            std::process::exit(1);
        }
        return;
    }

    // Silent GUI shortcut launcher mode (`bifrost-ui.exe --launch-app <pkg>`):
    // launches an Android app directly in its WSA-style resizable Windows 11 window
    // without flashing any console window or opening the management dashboard.
    if args.len() >= 3 && args[1] == "--launch-app" {
        let package = args[2].clone();
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("failed creating tokio runtime for app launcher");
        if let Err(err) = rt.block_on(service::launch_app_window(
            package.clone(),
            None,
            None,
            None,
        )) {
            let _ = dispatch_windows_toast(&AndroidNotification {
                key: format!("bifrost-launch-err-{package}"),
                package_name: package,
                title: Some("BIFRÖST could not launch app".to_string()),
                text: Some(err.to_string()),
            });
            std::process::exit(1);
        }
        return;
    }

    let pending_install = if args.len() >= 3 && args[1] == "--install-apk" {
        Some(PathBuf::from(&args[2]))
    } else {
        None
    };
    let is_installer_mode = pending_install.is_some();

    let mut builder = tauri::Builder::default();
    if !is_installer_mode {
        // A second dashboard launch focuses the existing (possibly tray-hidden) window.
        // The App Installer (`--install-apk`) and `--launch-app` stay independent.
        builder = builder.plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            tray::show_main_window(app);
        }));
    }
    builder
        .manage(PendingInstallState(Mutex::new(pending_install)))
        .on_window_event(move |window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                if !is_installer_mode
                    && window.label() == "main"
                    && service::close_to_tray_enabled()
                {
                    api.prevent_close();
                    let _ = window.hide();
                }
            }
        })
        .setup(move |app| {
            if !is_installer_mode {
                tray::install(app.handle())?;
            }
            if is_installer_mode {
                if let Some(win) = app.get_webview_window("main") {
                    let _ = win.set_title("BIFRÖST — App Installer");
                    let _ = win.set_min_size(Some(tauri::LogicalSize::new(500.0, 440.0)));
                    let _ = win.set_size(tauri::LogicalSize::new(560.0, 500.0));
                    let _ = win.center();
                }
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_pending_install,
            close_installer_window,
            preview_package_install,
            get_dashboard,
            start_heimdall,
            stop_heimdall,
            set_heimdall_autostart,
            get_doctor_report,
            save_config,
            auto_tune_config,
            start_runtime,
            stop_runtime,
            restart_runtime,
            get_logs,
            get_setup_plan,
            install_setup_tool,
            start_first_boot,
            list_apps,
            get_app_info,
            inspect_apk_file,
            install_apk_file,
            uninstall_app,
            launch_app,
            launch_app_window,
            list_app_shortcuts,
            create_app_shortcut,
            remove_app_shortcut,
            stop_app,
            list_shared_files,
            list_processes,
            list_network_forwards,
            list_images,
            register_image,
            activate_image,
            list_trusted_keys,
            add_trusted_key,
            trusted_key_removal_impact,
            remove_trusted_key,
            backup_userdata,
            restore_userdata,
            pick_file_dialog,
            pick_folder_dialog,
        ])
        .run(tauri::generate_context!())
        .expect("failed to launch BIFRÖST desktop application");
}
