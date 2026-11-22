//! Unprivileged Windows 11 `.apk` file association & Explorer context menu registration.
//!
//! Writes strictly under `HKCU\Software\Classes` (never `HKLM`) so Administrator
//! privileges are never required, and cleans up every key on uninstall.

use std::path::Path;
use std::process::Command;

use bifrost_core::{BifrostError, Result};
use serde::{Deserialize, Serialize};
use tracing::info;

use crate::shortcuts::locate_bifrost_cli;

/// ProgID registered in `HKCU\Software\Classes` for BIFRÖST `.apk` handling.
pub const BIFROST_APK_PROG_ID: &str = "Bifrost.ApkPackage";

/// Registry value name under `HKCU\Software\Microsoft\Windows\CurrentVersion\Run`
/// for silent Heimdall login autostart (zero UAC required).
pub const BIFROST_HEIMDALL_RUN_VALUE: &str = "BifrostHeimdall";

/// Summary of the Windows `.apk` file association registration state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApkAssociationStatus {
    pub prog_id: String,
    pub command: String,
    pub icon: String,
    pub context_menu_registered: bool,
    pub default_handler_registered: bool,
}

/// Summary of the user-space Windows login autostart registration for Heimdall.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HeimdallAutostartStatus {
    pub value_name: String,
    pub enabled: bool,
    pub command: Option<String>,
}

/// Builds the quoted Windows Shell command line invoked for `.apk` files.
#[must_use]
pub fn build_apk_open_command(bifrost_exe: &Path) -> String {
    format!("\"{}\" install \"%1\"", bifrost_exe.display())
}

/// Builds the quoted Windows login Run command line for silent Heimdall startup.
///
/// Prefers `bifrost-ui.exe --start-heimdall` (GUI subsystem, zero console flash)
/// and falls back to `bifrost.exe daemon start`.
#[must_use]
pub fn build_heimdall_autostart_command(bifrost_exe: &Path) -> String {
    if let Some(dir) = bifrost_exe.parent() {
        let ui_exe = dir.join("bifrost-ui.exe");
        if ui_exe.is_file() {
            return format!("\"{}\" --start-heimdall", ui_exe.display());
        }
    }
    format!("\"{}\" daemon start", bifrost_exe.display())
}

/// Queries whether Heimdall is registered to start automatically at Windows user login.
pub fn query_heimdall_autostart() -> Result<HeimdallAutostartStatus> {
    let command = read_windows_run_value()?;
    Ok(HeimdallAutostartStatus {
        value_name: BIFROST_HEIMDALL_RUN_VALUE.to_string(),
        enabled: command.is_some(),
        command,
    })
}

/// Enables or disables unprivileged Windows login autostart for Heimdall under
/// `HKCU\Software\Microsoft\Windows\CurrentVersion\Run`.
pub fn set_heimdall_autostart(enabled: bool) -> Result<HeimdallAutostartStatus> {
    if enabled {
        let bifrost_exe = locate_bifrost_cli()?;
        let command = build_heimdall_autostart_command(&bifrost_exe);
        write_windows_run_value(Some(&command))?;
        info!(
            value_name = BIFROST_HEIMDALL_RUN_VALUE,
            "enabled HKCU Windows login autostart for Heimdall"
        );
        Ok(HeimdallAutostartStatus {
            value_name: BIFROST_HEIMDALL_RUN_VALUE.to_string(),
            enabled: true,
            command: Some(command),
        })
    } else {
        write_windows_run_value(None)?;
        info!(
            value_name = BIFROST_HEIMDALL_RUN_VALUE,
            "disabled HKCU Windows login autostart for Heimdall"
        );
        Ok(HeimdallAutostartStatus {
            value_name: BIFROST_HEIMDALL_RUN_VALUE.to_string(),
            enabled: false,
            command: None,
        })
    }
}

/// Registers the Explorer context menu ("Install with BIFRÖST") and optionally sets
/// BIFRÖST as the current user's default double-click handler for `.apk` files.
pub fn register_apk_handler(set_default: bool) -> Result<ApkAssociationStatus> {
    let bifrost_exe = locate_bifrost_cli()?;
    let command = build_apk_open_command(&bifrost_exe);
    let icon = resolve_bifrost_icon(&bifrost_exe);

    apply_windows_apk_association(&command, &icon, set_default)?;

    info!(
        prog_id = BIFROST_APK_PROG_ID,
        set_default, "registered user-space Windows .apk file association"
    );

    Ok(ApkAssociationStatus {
        prog_id: BIFROST_APK_PROG_ID.to_string(),
        command,
        icon,
        context_menu_registered: true,
        default_handler_registered: set_default,
    })
}

/// Removes BIFRÖST's user-space `.apk` Explorer context menu and ProgID registration.
pub fn unregister_apk_handler() -> Result<()> {
    remove_windows_apk_association()?;
    info!(
        prog_id = BIFROST_APK_PROG_ID,
        "unregistered user-space Windows .apk file association"
    );
    Ok(())
}

fn resolve_bifrost_icon(bifrost_exe: &Path) -> String {
    if let Some(dir) = bifrost_exe.parent() {
        let ui_exe = dir.join("bifrost-ui.exe");
        if ui_exe.is_file() {
            return format!("{},0", ui_exe.display());
        }
    }
    format!("{},0", bifrost_exe.display())
}

#[cfg(windows)]
fn apply_windows_apk_association(command: &str, icon: &str, set_default: bool) -> Result<()> {
    const SCRIPT: &str = r#"
$ErrorActionPreference = 'Stop'
$progId = $env:BIFROST_APK_PROGID
$cmd = $env:BIFROST_APK_CMD
$icon = $env:BIFROST_APK_ICON
$setDefault = $env:BIFROST_APK_DEFAULT -eq '1'

$progRoot = "HKCU:\Software\Classes\$progId"
New-Item -Path "$progRoot\DefaultIcon" -Force | Out-Null
New-Item -Path "$progRoot\shell\open\command" -Force | Out-Null
Set-ItemProperty -Path $progRoot -Name '(Default)' -Value 'Android Application Package (BIFRÖST)'
Set-ItemProperty -Path "$progRoot\DefaultIcon" -Name '(Default)' -Value $icon
Set-ItemProperty -Path "$progRoot\shell\open" -Name '(Default)' -Value 'Install with BIFRÖST'
Set-ItemProperty -Path "$progRoot\shell\open" -Name 'Icon' -Value $icon
Set-ItemProperty -Path "$progRoot\shell\open\command" -Name '(Default)' -Value $cmd

$ctxRoot = 'HKCU:\Software\Classes\SystemFileAssociations\.apk\shell\BifrostInstall'
New-Item -Path "$ctxRoot\command" -Force | Out-Null
Set-ItemProperty -Path $ctxRoot -Name '(Default)' -Value 'Install with BIFRÖST'
Set-ItemProperty -Path $ctxRoot -Name 'Icon' -Value $icon
Set-ItemProperty -Path "$ctxRoot\command" -Name '(Default)' -Value $cmd

$extRoot = 'HKCU:\Software\Classes\.apk'
New-Item -Path "$extRoot\OpenWithProgids" -Force | Out-Null
New-ItemProperty -Path "$extRoot\OpenWithProgids" -Name $progId -PropertyType String -Value '' -Force | Out-Null
if ($setDefault) {
    Set-ItemProperty -Path $extRoot -Name '(Default)' -Value $progId
}
"#;

    let output = Command::new("powershell.exe")
        .args(["-NoProfile", "-NonInteractive", "-Command", SCRIPT])
        .env("BIFROST_APK_PROGID", BIFROST_APK_PROG_ID)
        .env("BIFROST_APK_CMD", command)
        .env("BIFROST_APK_ICON", icon)
        .env("BIFROST_APK_DEFAULT", if set_default { "1" } else { "0" })
        .output()
        .map_err(|source| BifrostError::Io {
            context: "registering HKCU .apk file association via PowerShell".to_string(),
            source,
        })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(BifrostError::Runtime(format!(
            "failed registering .apk file association: {stderr}"
        )));
    }
    Ok(())
}

#[cfg(not(windows))]
fn apply_windows_apk_association(_command: &str, _icon: &str, _set_default: bool) -> Result<()> {
    Err(BifrostError::Unsupported {
        feature: "Windows .apk file association".to_string(),
        milestone: "Windows 11 host only".to_string(),
    })
}

#[cfg(windows)]
fn remove_windows_apk_association() -> Result<()> {
    const SCRIPT: &str = r#"
$ErrorActionPreference = 'Stop'
$progId = $env:BIFROST_APK_PROGID

$progRoot = "HKCU:\Software\Classes\$progId"
if (Test-Path $progRoot) { Remove-Item -Path $progRoot -Recurse -Force }

$ctxRoot = 'HKCU:\Software\Classes\SystemFileAssociations\.apk\shell\BifrostInstall'
if (Test-Path $ctxRoot) { Remove-Item -Path $ctxRoot -Recurse -Force }

$extRoot = 'HKCU:\Software\Classes\.apk'
if (Test-Path "$extRoot\OpenWithProgids") {
    Remove-ItemProperty -Path "$extRoot\OpenWithProgids" -Name $progId -ErrorAction SilentlyContinue
}
if (Test-Path $extRoot) {
    $current = (Get-ItemProperty -Path $extRoot -Name '(Default)' -ErrorAction SilentlyContinue).'(Default)'
    if ($current -eq $progId) {
        Remove-ItemProperty -Path $extRoot -Name '(Default)' -ErrorAction SilentlyContinue
    }
}
exit 0
"#;

    let output = Command::new("powershell.exe")
        .args(["-NoProfile", "-NonInteractive", "-Command", SCRIPT])
        .env("BIFROST_APK_PROGID", BIFROST_APK_PROG_ID)
        .output()
        .map_err(|source| BifrostError::Io {
            context: "unregistering HKCU .apk file association via PowerShell".to_string(),
            source,
        })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(BifrostError::Runtime(format!(
            "failed unregistering .apk file association: {stderr}"
        )));
    }
    Ok(())
}

#[cfg(not(windows))]
fn remove_windows_apk_association() -> Result<()> {
    Err(BifrostError::Unsupported {
        feature: "Windows .apk file association".to_string(),
        milestone: "Windows 11 host only".to_string(),
    })
}

#[cfg(windows)]
fn read_windows_run_value() -> Result<Option<String>> {
    const SCRIPT: &str = r#"
$OutputEncoding = [Console]::OutputEncoding = [System.Text.Encoding]::UTF8
$path = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Run'
$name = $env:BIFROST_RUN_NAME
if (Test-Path $path) {
    $prop = Get-ItemProperty -Path $path -Name $name -ErrorAction SilentlyContinue
    if ($null -ne $prop) {
        Write-Output $prop.$name
    }
}
exit 0
"#;
    let output = Command::new("powershell.exe")
        .args(["-NoProfile", "-NonInteractive", "-Command", SCRIPT])
        .env("BIFROST_RUN_NAME", BIFROST_HEIMDALL_RUN_VALUE)
        .output()
        .map_err(|source| BifrostError::Io {
            context: "querying HKCU Run autostart via PowerShell".to_string(),
            source,
        })?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(BifrostError::Runtime(format!(
            "failed querying Heimdall autostart: {stderr}"
        )));
    }
    let text = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if text.is_empty() {
        Ok(None)
    } else {
        Ok(Some(text))
    }
}

#[cfg(not(windows))]
fn read_windows_run_value() -> Result<Option<String>> {
    Ok(None)
}

#[cfg(windows)]
fn write_windows_run_value(command: Option<&str>) -> Result<()> {
    const SCRIPT: &str = r#"
$ErrorActionPreference = 'Stop'
$path = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Run'
$name = $env:BIFROST_RUN_NAME
$cmd = $env:BIFROST_RUN_CMD
if ([string]::IsNullOrEmpty($cmd)) {
    if (Test-Path $path) {
        Remove-ItemProperty -Path $path -Name $name -ErrorAction SilentlyContinue
    }
} else {
    if (-not (Test-Path $path)) { New-Item -Path $path -Force | Out-Null }
    Set-ItemProperty -Path $path -Name $name -Value $cmd
}
exit 0
"#;
    let output = Command::new("powershell.exe")
        .args(["-NoProfile", "-NonInteractive", "-Command", SCRIPT])
        .env("BIFROST_RUN_NAME", BIFROST_HEIMDALL_RUN_VALUE)
        .env("BIFROST_RUN_CMD", command.unwrap_or(""))
        .output()
        .map_err(|source| BifrostError::Io {
            context: "updating HKCU Run autostart via PowerShell".to_string(),
            source,
        })?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(BifrostError::Runtime(format!(
            "failed updating Heimdall autostart: {stderr}"
        )));
    }
    Ok(())
}

#[cfg(not(windows))]
fn write_windows_run_value(_command: Option<&str>) -> Result<()> {
    Err(BifrostError::Unsupported {
        feature: "Windows login autostart".to_string(),
        milestone: "Windows 11 host only".to_string(),
    })
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    #[test]
    fn builds_properly_quoted_apk_shell_command() {
        let exe = PathBuf::from(r"C:\Users\johan\AppData\Local\Programs\BIFRÖST\bifrost.exe");
        assert_eq!(
            build_apk_open_command(&exe),
            "\"C:\\Users\\johan\\AppData\\Local\\Programs\\BIFRÖST\\bifrost.exe\" install \"%1\""
        );
    }

    #[test]
    fn builds_properly_quoted_heimdall_autostart_command() {
        let exe = PathBuf::from(r"C:\NonExistentDir\bifrost.exe");
        assert_eq!(
            build_heimdall_autostart_command(&exe),
            "\"C:\\NonExistentDir\\bifrost.exe\" daemon start"
        );
    }
}
