//! Host-side views (shared folder, guest processes, port forwards) and native file pickers.

use super::*;

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
pub(super) fn parse_dialog_filter(filter: &str) -> Vec<(String, Vec<String>)> {
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
pub(super) fn run_native_file_dialog(title: &str, filter: &str) -> Result<Option<PathBuf>> {
    let mut dialog = rfd::FileDialog::new().set_title(title);
    for (label, extensions) in parse_dialog_filter(filter) {
        dialog = dialog.add_filter(label, &extensions);
    }
    Ok(dialog.pick_file())
}

/// Shows the native Windows folder picker (through `rfd`).
#[cfg(windows)]
pub(super) fn run_native_folder_dialog(title: &str) -> Result<Option<PathBuf>> {
    Ok(rfd::FileDialog::new().set_title(title).pick_folder())
}

#[cfg(not(windows))]
pub(super) fn run_native_file_dialog(_title: &str, _filter: &str) -> Result<Option<PathBuf>> {
    Ok(None)
}

#[cfg(not(windows))]
pub(super) fn run_native_folder_dialog(_title: &str) -> Result<Option<PathBuf>> {
    Ok(None)
}
