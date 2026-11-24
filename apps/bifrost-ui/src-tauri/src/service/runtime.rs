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
