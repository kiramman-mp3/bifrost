//! First-run setup: an ordered checklist of what a fresh Windows 11 host still needs
//! before Android apps can run, and the visible `winget` installs that fix tooling gaps.
//!
//! The plan is derived from the `doctor` report and the ADB endpoint state; it never
//! changes the host by itself. Tool installs happen only on an explicit user request,
//! in a visible console where `winget` shows its license prompt and Windows shows UAC
//! when an installer needs it. Enabling Windows features is left to the user.

use std::path::PathBuf;
use std::process::{Command, ExitStatus};

use bifrost_bridge::AdbEndpointState;
use bifrost_core::{find_on_path, BifrostError, Result};
use bifrost_diagnostics::{CheckStatus, DiagnosticCheck, DoctorReport, HostToolingInfo};
use serde::{Deserialize, Serialize};

/// Progress of one setup step.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SetupStepStatus {
    /// Nothing left to do.
    Done,
    /// The user can act on it now.
    Pending,
    /// Waits for an earlier required step.
    Blocked,
}

/// What resolves a pending step.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SetupAction {
    /// Install a package with `winget` (see [`install_tool`]).
    Winget { package_id: String },
    /// The user has to do it in Windows; `instructions` says how.
    Manual { instructions: String },
    /// Register a Midgard image manifest (`bifrost images register`).
    RegisterImage,
    /// Boot Midgard with a visible window and finish Android's own first-run screens.
    FirstBoot,
}

/// One item of the first-run checklist.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SetupStep {
    pub id: String,
    pub title: String,
    /// Optional steps (scrcpy) do not block [`SetupPlan::complete`].
    pub required: bool,
    pub status: SetupStepStatus,
    pub detail: String,
    pub action: SetupAction,
}

/// The ordered first-run checklist.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SetupPlan {
    pub steps: Vec<SetupStep>,
    /// Every required step is done.
    pub complete: bool,
}

/// Inputs that the setup plan is derived from.
#[derive(Debug, Clone, Copy)]
pub struct SetupInputs<'a> {
    pub tooling: &'a HostToolingInfo,
    /// The doctor's `whpx` check, if it ran.
    pub whpx: Option<&'a DiagnosticCheck>,
    /// Whether Midgard is booting or running.
    pub runtime_active: bool,
    /// ADB endpoint state of the running guest, when known.
    pub adb_access: Option<AdbEndpointState>,
}

impl<'a> SetupInputs<'a> {
    /// Takes the tooling snapshot and the WHPX check from a doctor report.
    #[must_use]
    pub fn from_doctor(
        doctor: &'a DoctorReport,
        runtime_active: bool,
        adb_access: Option<AdbEndpointState>,
    ) -> Self {
        Self {
            tooling: &doctor.snapshot.tooling,
            whpx: doctor.checks.iter().find(|c| c.id == "whpx"),
            runtime_active,
            adb_access,
        }
    }
}

/// `winget` package for each tooling step. Installs are limited to this list.
const WINGET_PACKAGES: &[(&str, &str)] = &[
    ("qemu", "SoftwareFreedomConservancy.QEMU"),
    ("adb", "Google.PlatformTools"),
    ("scrcpy", "Genymobile.scrcpy"),
];

/// The `winget` package id for a tooling step, if the step is installable.
#[must_use]
pub fn winget_package(step_id: &str) -> Option<&'static str> {
    WINGET_PACKAGES
        .iter()
        .find(|(id, _)| *id == step_id)
        .map(|(_, package)| *package)
}

impl SetupPlan {
    /// Builds the checklist. A step is `Blocked` while an earlier required step is not
    /// done, so the user always sees one clear next action.
    #[must_use]
    pub fn evaluate(inputs: SetupInputs<'_>) -> Self {
        let tooling = inputs.tooling;
        let whpx = inputs.whpx;
        let whpx_ready = whpx.is_some_and(|c| c.status == CheckStatus::Pass);
        let whpx_fix = whpx.and_then(|c| c.remediation.clone()).unwrap_or_else(|| {
            "Enable \"Virtual Machine Platform\" in \"Turn Windows features on or off\" \
                 (optionalfeatures.exe), turn on virtualization in the firmware if it is off, \
                 and restart Windows."
                .to_string()
        });

        let tool = |id: &str, title: &str, required: bool, path: &Option<PathBuf>| {
            let package = winget_package(id).unwrap_or_default();
            raw_step(
                id,
                title,
                required,
                path.is_some(),
                path.as_ref().map_or_else(
                    || format!("Not found. BIFRÖST can install it with winget ({package})."),
                    |p| format!("Found at {}", p.display()),
                ),
                SetupAction::Winget {
                    package_id: package.to_string(),
                },
            )
        };

        let image_detail = match (&tooling.active_image_id, tooling.registered_images_count) {
            (Some(id), _) => format!("Active image: {id}"),
            (None, 0) => "No Android image registered yet. Register the manifest.json of a \
                          verified Midgard image bundle."
                .to_string(),
            (None, n) => format!("{n} image(s) registered but none is active."),
        };
        let image_done = tooling.active_image_id.is_some();

        let adb_ready = inputs.adb_access == Some(AdbEndpointState::Ready);
        let first_boot_detail = match (inputs.runtime_active, inputs.adb_access) {
            (_, Some(AdbEndpointState::Ready)) => {
                "Android is set up and reachable over ADB.".to_string()
            }
            (true, Some(state)) => state
                .remediation()
                .map_or_else(|| "Android is booting…".to_string(), ToString::to_string),
            (true, None) => "Android is booting…".to_string(),
            (false, _) => "Boot Midgard with a visible window once, finish Android's welcome \
                           screens and allow USB debugging for this PC when Android asks."
                .to_string(),
        };

        let mut steps = vec![
            raw_step(
                "virtualization",
                "Windows virtualization (WHPX)",
                true,
                whpx_ready,
                if whpx_ready {
                    "Windows Hypervisor Platform is ready.".to_string()
                } else {
                    whpx_fix.clone()
                },
                SetupAction::Manual {
                    instructions: whpx_fix,
                },
            ),
            tool(
                "qemu",
                "QEMU (virtual machine engine)",
                true,
                &tooling.qemu_path,
            ),
            tool(
                "adb",
                "Android Platform-Tools (adb)",
                true,
                &tooling.adb_path,
            ),
            tool(
                "scrcpy",
                "scrcpy (per-app windows)",
                false,
                &tooling.scrcpy_path,
            ),
            raw_step(
                "image",
                "Midgard Android image",
                true,
                image_done,
                image_detail,
                SetupAction::RegisterImage,
            ),
            raw_step(
                "first_boot",
                "First Android boot",
                true,
                adb_ready,
                first_boot_detail,
                SetupAction::FirstBoot,
            ),
        ];

        let mut earlier_required_missing = false;
        for step in &mut steps {
            if step.status == SetupStepStatus::Pending && earlier_required_missing {
                step.status = SetupStepStatus::Blocked;
            }
            if step.required && step.status != SetupStepStatus::Done {
                earlier_required_missing = true;
            }
        }
        // Optional tools are independent of the chain: they can be installed anytime.
        for step in &mut steps {
            if !step.required && step.status == SetupStepStatus::Blocked {
                step.status = SetupStepStatus::Pending;
            }
        }
        let complete = steps
            .iter()
            .all(|s| !s.required || s.status == SetupStepStatus::Done);
        Self { steps, complete }
    }
}

fn raw_step(
    id: &str,
    title: &str,
    required: bool,
    done: bool,
    detail: String,
    action: SetupAction,
) -> SetupStep {
    SetupStep {
        id: id.to_string(),
        title: title.to_string(),
        required,
        status: if done {
            SetupStepStatus::Done
        } else {
            SetupStepStatus::Pending
        },
        detail,
        action,
    }
}

/// `winget` arguments for a package. No `--silent` and no `--accept-package-agreements`:
/// the license prompt and any installer UI or UAC dialog stay visible to the user.
#[must_use]
pub fn winget_install_args(package_id: &str) -> Vec<String> {
    [
        "install",
        "--id",
        package_id,
        "--exact",
        "--source",
        "winget",
        "--accept-source-agreements",
    ]
    .into_iter()
    .map(str::to_string)
    .collect()
}

/// Installs the tool behind a setup step (`qemu`, `adb`, `scrcpy`) with `winget` in a
/// new, visible console window and waits for it to finish. Any other step id is
/// rejected, so callers cannot run arbitrary packages.
pub fn install_tool(step_id: &str) -> Result<ExitStatus> {
    let package = winget_package(step_id).ok_or_else(|| {
        BifrostError::Runtime(format!(
            "setup step '{step_id}' cannot be installed with winget"
        ))
    })?;
    let winget = find_on_path("winget.exe").ok_or_else(|| {
        BifrostError::Runtime(
            "winget (App Installer) is not available; install it from the Microsoft Store \
             or install the tool manually"
                .to_string(),
        )
    })?;
    let mut command = Command::new(winget);
    command.args(winget_install_args(package));
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NEW_CONSOLE: u32 = 0x0000_0010;
        command.creation_flags(CREATE_NEW_CONSOLE);
    }
    command
        .status()
        .map_err(|err| BifrostError::Runtime(format!("could not start winget: {err}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn whpx(status: CheckStatus) -> DiagnosticCheck {
        DiagnosticCheck {
            id: "whpx".to_string(),
            name: "WHPX".to_string(),
            status,
            summary: String::new(),
            detail: String::new(),
            remediation: Some("Enable Virtual Machine Platform.".to_string()),
        }
    }

    fn plan(
        whpx_status: CheckStatus,
        tooling: &HostToolingInfo,
        runtime_active: bool,
        adb_access: Option<AdbEndpointState>,
    ) -> SetupPlan {
        let check = whpx(whpx_status);
        SetupPlan::evaluate(SetupInputs {
            tooling,
            whpx: Some(&check),
            runtime_active,
            adb_access,
        })
    }

    fn all_tools() -> HostToolingInfo {
        HostToolingInfo {
            qemu_path: Some(PathBuf::from(r"C:\qemu\qemu-system-x86_64.exe")),
            edk2_firmware_path: None,
            adb_path: Some(PathBuf::from(r"C:\pt\adb.exe")),
            scrcpy_path: Some(PathBuf::from(r"C:\scrcpy\scrcpy.exe")),
            registered_images_count: 1,
            active_image_id: Some("lineage".to_string()),
        }
    }

    fn status_of(plan: &SetupPlan, id: &str) -> SetupStepStatus {
        plan.steps.iter().find(|s| s.id == id).unwrap().status
    }

    #[test]
    fn fresh_host_has_one_actionable_required_step_and_optional_scrcpy() {
        let tooling = HostToolingInfo::default();
        let plan = plan(CheckStatus::Pass, &tooling, false, None);
        assert!(!plan.complete);
        assert_eq!(status_of(&plan, "virtualization"), SetupStepStatus::Done);
        assert_eq!(status_of(&plan, "qemu"), SetupStepStatus::Pending);
        assert_eq!(status_of(&plan, "adb"), SetupStepStatus::Blocked);
        assert_eq!(status_of(&plan, "scrcpy"), SetupStepStatus::Pending);
        assert_eq!(status_of(&plan, "image"), SetupStepStatus::Blocked);
        assert_eq!(status_of(&plan, "first_boot"), SetupStepStatus::Blocked);
    }

    #[test]
    fn missing_whpx_blocks_everything_required_with_manual_instructions() {
        let tooling = all_tools();
        let plan = plan(CheckStatus::Fail, &tooling, false, None);
        let virt = &plan.steps[0];
        assert_eq!(virt.status, SetupStepStatus::Pending);
        assert!(matches!(virt.action, SetupAction::Manual { .. }));
        assert_eq!(status_of(&plan, "first_boot"), SetupStepStatus::Blocked);
    }

    #[test]
    fn first_boot_reports_trade_in_mode_and_completes_when_adb_is_ready() {
        let tooling = all_tools();
        let booting = plan(
            CheckStatus::Pass,
            &tooling,
            true,
            Some(AdbEndpointState::TradeInMode),
        );
        assert!(!booting.complete);
        let step = booting.steps.last().unwrap();
        assert_eq!(step.status, SetupStepStatus::Pending);
        assert_eq!(
            Some(step.detail.as_str()),
            AdbEndpointState::TradeInMode.remediation()
        );

        let ready = plan(
            CheckStatus::Pass,
            &tooling,
            true,
            Some(AdbEndpointState::Ready),
        );
        assert!(ready.complete);
    }

    #[test]
    fn missing_optional_scrcpy_does_not_block_completion() {
        let tooling = HostToolingInfo {
            scrcpy_path: None,
            ..all_tools()
        };
        let result = plan(
            CheckStatus::Pass,
            &tooling,
            true,
            Some(AdbEndpointState::Ready),
        );
        assert!(result.complete);
        assert_eq!(status_of(&result, "scrcpy"), SetupStepStatus::Pending);
    }

    #[test]
    fn installs_only_allowlisted_packages_and_never_silently() {
        assert_eq!(winget_package("adb"), Some("Google.PlatformTools"));
        assert_eq!(winget_package("image"), None);
        assert!(install_tool("Some.Arbitrary.Package").is_err());
        let args = winget_install_args("Google.PlatformTools");
        assert!(args
            .iter()
            .all(|a| a != "--silent" && a != "--accept-package-agreements"));
        assert!(args
            .windows(2)
            .any(|w| w == ["--id", "Google.PlatformTools"]));
    }
}
