//! Android apps: inspection, the App Installer flow, launch/stop and Windows shortcuts.

use super::*;

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

pub(super) fn encode_base64(input: &[u8]) -> String {
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
