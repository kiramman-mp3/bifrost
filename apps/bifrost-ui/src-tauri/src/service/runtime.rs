//! Midgard runtime lifecycle and logs.

use super::*;

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

/// Starts the active registered image (tray "Start Midgard").
pub async fn start_active_image(headless: bool) -> Result<RuntimeRecord> {
    let session = session().await?;
    let runtime =
        MidgardRuntime::with_default_backend(session.config().clone(), session.paths().clone());
    let manifest = blocking(move || runtime.resolve_default_manifest()).await?;
    session.start_runtime(manifest, headless, false).await
}

/// Persisted runtime state read locally, without probing Heimdall or ADB.
pub fn local_runtime_state() -> Result<RuntimeState> {
    let (_, config, paths) = load_config_and_paths()?;
    Ok(MidgardRuntime::with_default_backend(config, paths)
        .status()?
        .state)
}

/// Whether closing the main window should hide it to the tray.
#[must_use]
pub fn close_to_tray_enabled() -> bool {
    load_config_and_paths().map_or(true, |(_, config, _)| config.desktop.close_to_tray)
}
