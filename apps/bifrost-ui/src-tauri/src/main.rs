//! BIFRÖST Desktop Application (`bifrost-ui`) — Tauri 2 Rust backend entry point.
//!
//! All business logic, configuration validation, diagnostics, and IPC live in
//! `service.rs` and workspace crates; this file exposes thin `#[tauri::command]`
//! bindings to the Windows 11 Fluent Design WebView frontend.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
#![forbid(unsafe_code)]

mod service;

use std::path::PathBuf;

use bifrost_bridge::{
    dispatch_windows_toast, AndroidNotification, DaemonHealthReport, SharedFileEntry,
};
use bifrost_core::config::LoggingConfig;
use bifrost_core::logging::init_logging;
use bifrost_core::BifrostConfig;
use bifrost_diagnostics::DoctorReport;
use bifrost_display::AppWindowHandle;
use bifrost_network::PortForwardRule;
use bifrost_process::AndroidProcess;
use bifrost_runtime::{RuntimeLogs, RuntimeRecord};
use valhalla::manager::{InstallOutcome, LaunchOutcome};
use valhalla::{
    ApkInspection, HeimdallAutostartStatus, InstalledPackage, PackageDetails, ShortcutRecord,
};

use crate::service::DashboardSnapshot;

fn map_err<T>(res: bifrost_core::Result<T>) -> Result<T, String> {
    res.map_err(|err| err.to_string())
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
fn set_heimdall_autostart(enabled: bool) -> Result<HeimdallAutostartStatus, String> {
    map_err(service::set_heimdall_autostart(enabled))
}

#[tauri::command]
fn get_doctor_report() -> Result<DoctorReport, String> {
    map_err(service::get_doctor_report())
}

#[tauri::command]
fn save_config(config: BifrostConfig) -> Result<PathBuf, String> {
    map_err(service::save_config(config))
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
fn inspect_apk_file(apk_path: PathBuf) -> Result<ApkInspection, String> {
    map_err(service::inspect_apk_file(apk_path))
}

#[tauri::command]
async fn install_apk_file(apk_path: PathBuf) -> Result<InstallOutcome, String> {
    map_err(service::install_apk_file(apk_path).await)
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
fn list_app_shortcuts() -> Result<Vec<ShortcutRecord>, String> {
    map_err(service::list_app_shortcuts())
}

#[tauri::command]
fn create_app_shortcut(
    package: String,
    name: Option<String>,
    apk_path: Option<PathBuf>,
) -> Result<ShortcutRecord, String> {
    map_err(service::create_app_shortcut(package, name, apk_path))
}

#[tauri::command]
fn remove_app_shortcut(package: String) -> Result<bool, String> {
    map_err(service::remove_app_shortcut(package))
}

#[tauri::command]
async fn stop_app(package: String) -> Result<(), String> {
    map_err(service::stop_app(package).await)
}

#[tauri::command]
fn list_shared_files() -> Result<Vec<SharedFileEntry>, String> {
    map_err(service::list_shared_files())
}

#[tauri::command]
fn list_processes(filter: Option<String>) -> Result<Vec<AndroidProcess>, String> {
    map_err(service::list_processes(filter))
}

#[tauri::command]
fn list_network_forwards() -> Result<Vec<PortForwardRule>, String> {
    map_err(service::list_network_forwards())
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

    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![
            get_dashboard,
            start_heimdall,
            stop_heimdall,
            set_heimdall_autostart,
            get_doctor_report,
            save_config,
            start_runtime,
            stop_runtime,
            restart_runtime,
            get_logs,
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
        ])
        .run(tauri::generate_context!())
        .expect("failed to launch BIFRÖST desktop application");
}
