//! `bifrost setup` (first-run checklist, visible `winget` installs) and `bifrost update`.

use bifrost_client::setup::{install_tool, SetupAction, SetupPlan, SetupStepStatus};

use super::*;

pub(super) async fn handle_setup(
    explicit_config: Option<&PathBuf>,
    as_json: bool,
    install: Option<&str>,
) -> Result<String> {
    if let Some(step_id) = install {
        let step = step_id.to_string();
        let status = blocking(move || install_tool(&step)).await?;
        if !status.success() {
            return Err(BifrostError::Runtime(format!(
                "winget did not finish installing '{step_id}' ({status}); \
                 see the winget window for details"
            )));
        }
    }

    let session = Session::open(explicit_config.map(PathBuf::as_path)).await?;
    let (config, data_dir) = (session.config().clone(), session.paths().data_dir.clone());
    let doctor = blocking(move || Ok(run_doctor(&config, &data_dir))).await?;
    let plan = session.setup_plan(&doctor).await?;

    if as_json {
        return serde_json::to_string_pretty(&plan)
            .map_err(|err| BifrostError::ConfigSerialize(err.to_string()));
    }
    Ok(render_setup_plan(&plan, install))
}

fn render_setup_plan(plan: &SetupPlan, installed: Option<&str>) -> String {
    let mut out = String::new();
    if let Some(step) = installed {
        out.push_str(&format!("winget finished installing '{step}'.\n\n"));
    }
    out.push_str("BIFRÖST first-run setup\n\n");
    for step in &plan.steps {
        let mark = match step.status {
            SetupStepStatus::Done => "[x]",
            SetupStepStatus::Pending => "[ ]",
            SetupStepStatus::Blocked => "[-]",
        };
        let optional = if step.required { "" } else { " (optional)" };
        out.push_str(&format!(
            "{mark} {}{optional}\n    {}\n",
            step.title, step.detail
        ));
    }
    out.push('\n');
    let next = plan
        .steps
        .iter()
        .find(|s| s.required && s.status == SetupStepStatus::Pending);
    match next {
        None => out.push_str("Setup is complete. Android apps are ready to run.\n"),
        Some(step) => {
            let hint = match &step.action {
                SetupAction::Winget { .. } => format!(
                    "run `bifrost setup --install {}` (opens a visible winget window)",
                    step.id
                ),
                SetupAction::Manual { .. } => "follow the instructions above".to_string(),
                SetupAction::RegisterImage => {
                    "run `bifrost images register --manifest <path-to-manifest.json>`".to_string()
                }
                SetupAction::FirstBoot => {
                    "run `bifrost runtime start` (visible window) and finish Android's welcome \
                     screens"
                        .to_string()
                }
            };
            out.push_str(&format!("Next: {hint}.\n"));
        }
    }
    out
}

pub(super) async fn handle_update(as_json: bool) -> Result<String> {
    let current = VersionInfo::current().version;
    let info = blocking(move || bifrost_client::update::check_for_update(&current)).await?;
    if as_json {
        return serde_json::to_string_pretty(&info)
            .map_err(|err| BifrostError::ConfigSerialize(err.to_string()));
    }
    if !info.update_available {
        return Ok(format!(
            "BIFRÖST {} is up to date (latest release: {}).",
            info.current_version, info.latest_version
        ));
    }
    let bundle = info
        .bundle_asset
        .as_deref()
        .map_or_else(String::new, |a| format!(" ({a})"));
    Ok(format!(
        "BIFRÖST {} is available (installed: {}).\n\
         Download the release bundle{bundle} from {}\n\
         and run `install-bifrost.ps1 -BundleDir <extracted folder>`; it verifies every \
         file's SHA-256 and keeps your Android data.",
        info.latest_version, info.current_version, info.release_url
    ))
}
