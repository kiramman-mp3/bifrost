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
    backup_userdata, describe_trusted_key, evaluate_manifest_trust, images_losing_trust,
    restore_userdata, verify_manifest_file, with_trusted_key, without_trusted_key, BackupMetadata,
    ImageRegistry, InstalledImageRecord, TrustedKeyInfo,
};
use serde::Serialize;
use valhalla::manager::{InstallOutcome, LaunchOutcome};
use valhalla::{
    derive_display_name, inspect_apk, inspect_package_preview, query_heimdall_autostart,
    set_heimdall_autostart as apply_heimdall_autostart, ApkInspection, HeimdallAutostartStatus,
    InstalledPackage, PackageDetails, ShortcutManager, ShortcutRecord, ValhallaManager,
};

mod apps;
mod dashboard;
mod host;
mod images;
mod runtime;

pub use apps::*;
pub use dashboard::*;
pub use host::*;
pub use images::*;
pub use runtime::*;

/// Resolves effective per-user BIFRÖST config and storage paths.
pub fn load_config_and_paths() -> Result<(PathBuf, BifrostConfig, BifrostPaths)> {
    let loaded = LoadedConfig::load(None)?;
    Ok((loaded.config_path, loaded.config, loaded.paths))
}

/// Opens a facade session on the per-user config (one Heimdall probe per command).
async fn session() -> Result<Session> {
    Session::open(None).await
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
