//! First-run setup wizard: checklist, visible winget installs and the first visible boot.

use bifrost_client::setup::{install_tool, SetupPlan};

use super::*;

/// The first-run checklist, with a fresh doctor run (tools may have just been installed).
pub async fn get_setup_plan() -> Result<SetupPlan> {
    let session = session().await?;
    let doctor = blocking(get_doctor_report).await?;
    session.setup_plan(&doctor).await
}

/// Installs `qemu`, `adb` or `scrcpy` with winget in a visible console, then re-evaluates.
pub async fn install_setup_tool(step_id: String) -> Result<SetupPlan> {
    let step = step_id.clone();
    let status = blocking(move || install_tool(&step)).await?;
    if !status.success() {
        return Err(BifrostError::Runtime(format!(
            "winget did not finish installing '{step_id}' ({status}); see the winget window"
        )));
    }
    get_setup_plan().await
}

/// Boots the active image with a visible window so the user can finish Android's welcome
/// screens and approve USB debugging. A running (headless) VM is restarted visibly.
pub async fn start_first_boot() -> Result<RuntimeRecord> {
    let session = session().await?;
    if session.status().await?.runtime_state.is_active() {
        session.restart_runtime(None, false).await
    } else {
        drop(session);
        start_active_image(false).await
    }
}
