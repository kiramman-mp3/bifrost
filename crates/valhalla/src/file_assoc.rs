//! Unprivileged Windows 11 `.apk` file association & Explorer context menu registration.
//!
//! Writes strictly under `HKCU\Software\Classes` (never `HKLM`) so Administrator
//! privileges are never required, and cleans up every key on uninstall.

use std::path::Path;

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

/// Builds the quoted Windows Shell command line invoked for `.apk` / `.apkm` files.
///
/// Prefers the Fluent Design 2 graphical installer dialog (`bifrost-ui.exe --install-apk "%1"`)
/// when `bifrost-ui.exe` is installed alongside `bifrost.exe`, falling back to `bifrost.exe install "%1"`.
#[must_use]
pub fn build_apk_open_command(bifrost_exe: &Path) -> String {
    if let Some(dir) = bifrost_exe.parent() {
        let ui_exe = dir.join("bifrost-ui.exe");
        if ui_exe.is_file() {
            return format!("\"{}\" --install-apk \"%1\"", ui_exe.display());
        }
    }
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

/// `HKCU`-relative key holding per-user file classes.
const USER_CLASSES_KEY: &str = r"Software\Classes";

/// `HKCU`-relative key holding per-user login autostart commands.
const USER_RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";

/// File extensions handled by BIFRÖST.
const APK_EXTENSIONS: [&str; 2] = [".apk", ".apkm"];

#[cfg(windows)]
fn apply_windows_apk_association(command: &str, icon: &str, set_default: bool) -> Result<()> {
    let hkcu = winreg::RegKey::predef(winreg::enums::HKEY_CURRENT_USER);
    apply_apk_association_at(&hkcu, USER_CLASSES_KEY, command, icon, set_default)
}

#[cfg(windows)]
fn apply_apk_association_at(
    root: &winreg::RegKey,
    classes: &str,
    command: &str,
    icon: &str,
    set_default: bool,
) -> Result<()> {
    let prog_root = format!(r"{classes}\{BIFROST_APK_PROG_ID}");
    set_string(
        root,
        &prog_root,
        "",
        "Android Application Package (BIFRÖST)",
    )?;
    set_string(root, &format!(r"{prog_root}\DefaultIcon"), "", icon)?;
    set_string(
        root,
        &format!(r"{prog_root}\shell\open"),
        "",
        "Install with BIFRÖST",
    )?;
    set_string(root, &format!(r"{prog_root}\shell\open"), "Icon", icon)?;
    set_string(
        root,
        &format!(r"{prog_root}\shell\open\command"),
        "",
        command,
    )?;

    for ext in APK_EXTENSIONS {
        let ctx_root = format!(r"{classes}\SystemFileAssociations\{ext}\shell\BifrostInstall");
        set_string(root, &ctx_root, "", "Install with BIFRÖST")?;
        set_string(root, &ctx_root, "Icon", icon)?;
        set_string(root, &format!(r"{ctx_root}\command"), "", command)?;

        let ext_root = format!(r"{classes}\{ext}");
        set_string(
            root,
            &format!(r"{ext_root}\OpenWithProgids"),
            BIFROST_APK_PROG_ID,
            "",
        )?;
        if set_default {
            set_string(root, &ext_root, "", BIFROST_APK_PROG_ID)?;
        }
    }
    Ok(())
}

#[cfg(not(windows))]
fn apply_windows_apk_association(_command: &str, _icon: &str, _set_default: bool) -> Result<()> {
    Err(unsupported("Windows .apk file association"))
}

#[cfg(windows)]
fn remove_windows_apk_association() -> Result<()> {
    let hkcu = winreg::RegKey::predef(winreg::enums::HKEY_CURRENT_USER);
    remove_apk_association_at(&hkcu, USER_CLASSES_KEY)
}

#[cfg(windows)]
fn remove_apk_association_at(root: &winreg::RegKey, classes: &str) -> Result<()> {
    delete_tree(root, &format!(r"{classes}\{BIFROST_APK_PROG_ID}"))?;
    for ext in APK_EXTENSIONS {
        delete_tree(
            root,
            &format!(r"{classes}\SystemFileAssociations\{ext}\shell\BifrostInstall"),
        )?;
        let ext_root = format!(r"{classes}\{ext}");
        delete_value(
            root,
            &format!(r"{ext_root}\OpenWithProgids"),
            BIFROST_APK_PROG_ID,
        )?;
        // Only clear the default handler when it still points at BIFRÖST.
        if read_string(root, &ext_root, "")?.as_deref() == Some(BIFROST_APK_PROG_ID) {
            delete_value(root, &ext_root, "")?;
        }
    }
    Ok(())
}

#[cfg(not(windows))]
fn remove_windows_apk_association() -> Result<()> {
    Err(unsupported("Windows .apk file association"))
}

#[cfg(windows)]
fn read_windows_run_value() -> Result<Option<String>> {
    let hkcu = winreg::RegKey::predef(winreg::enums::HKEY_CURRENT_USER);
    read_string(&hkcu, USER_RUN_KEY, BIFROST_HEIMDALL_RUN_VALUE)
}

#[cfg(not(windows))]
fn read_windows_run_value() -> Result<Option<String>> {
    Ok(None)
}

#[cfg(windows)]
fn write_windows_run_value(command: Option<&str>) -> Result<()> {
    let hkcu = winreg::RegKey::predef(winreg::enums::HKEY_CURRENT_USER);
    match command {
        Some(cmd) => set_string(&hkcu, USER_RUN_KEY, BIFROST_HEIMDALL_RUN_VALUE, cmd),
        None => delete_value(&hkcu, USER_RUN_KEY, BIFROST_HEIMDALL_RUN_VALUE),
    }
}

#[cfg(not(windows))]
fn write_windows_run_value(_command: Option<&str>) -> Result<()> {
    Err(unsupported("Windows login autostart"))
}

#[cfg(not(windows))]
fn unsupported(feature: &str) -> BifrostError {
    BifrostError::Unsupported {
        feature: feature.to_string(),
        planned_milestone: "Windows 11 host only".to_string(),
    }
}

#[cfg(windows)]
fn registry_error(action: &str, path: &str, source: std::io::Error) -> BifrostError {
    BifrostError::Io {
        context: format!(r"{action} HKCU\{path}"),
        source,
    }
}

#[cfg(windows)]
fn set_string(root: &winreg::RegKey, path: &str, name: &str, value: &str) -> Result<()> {
    let (key, _) = root
        .create_subkey(path)
        .map_err(|e| registry_error("creating", path, e))?;
    key.set_value(name, &value)
        .map_err(|e| registry_error("writing", path, e))
}

#[cfg(windows)]
fn read_string(root: &winreg::RegKey, path: &str, name: &str) -> Result<Option<String>> {
    let key = match root.open_subkey(path) {
        Ok(key) => key,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(err) => return Err(registry_error("opening", path, err)),
    };
    match key.get_value::<String, _>(name) {
        Ok(value) if value.is_empty() => Ok(None),
        Ok(value) => Ok(Some(value)),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(err) => Err(registry_error("reading", path, err)),
    }
}

#[cfg(windows)]
fn delete_value(root: &winreg::RegKey, path: &str, name: &str) -> Result<()> {
    let key = match root.open_subkey_with_flags(path, winreg::enums::KEY_SET_VALUE) {
        Ok(key) => key,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(err) => return Err(registry_error("opening", path, err)),
    };
    match key.delete_value(name) {
        Err(err) if err.kind() != std::io::ErrorKind::NotFound => {
            Err(registry_error("deleting value in", path, err))
        }
        _ => Ok(()),
    }
}

#[cfg(windows)]
fn delete_tree(root: &winreg::RegKey, path: &str) -> Result<()> {
    match root.delete_subkey_all(path) {
        Err(err) if err.kind() != std::io::ErrorKind::NotFound => {
            Err(registry_error("deleting", path, err))
        }
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    #[test]
    fn builds_properly_quoted_apk_shell_command() {
        let exe = PathBuf::from(r"C:\NonExistentDir\BIFRÖST\bifrost.exe");
        assert_eq!(
            build_apk_open_command(&exe),
            "\"C:\\NonExistentDir\\BIFRÖST\\bifrost.exe\" install \"%1\""
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

    /// Exercises the full register/unregister cycle against a scratch HKCU subtree
    /// so the user's real file associations are never touched.
    #[cfg(windows)]
    #[test]
    fn registers_and_removes_apk_association_in_scratch_registry() {
        use winreg::enums::HKEY_CURRENT_USER;
        use winreg::RegKey;

        let scratch = r"Software\BifrostUnitTest\FileAssoc";
        let classes = format!(r"{scratch}\Classes");
        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        let _ = hkcu.delete_subkey_all(scratch);

        // Another app owns .apkm; BIFRÖST must not clear it on removal.
        set_string(&hkcu, &format!(r"{classes}\.apkm"), "", "Other.Handler").unwrap();

        let cmd = r#""C:\BIFRÖST\bifrost-ui.exe" --install-apk "%1""#;
        apply_apk_association_at(&hkcu, &classes, cmd, r"C:\BIFRÖST\bifrost-ui.exe,0", true)
            .unwrap();
        let open_cmd = format!(r"{classes}\{BIFROST_APK_PROG_ID}\shell\open\command");
        assert_eq!(
            read_string(&hkcu, &open_cmd, "").unwrap().as_deref(),
            Some(cmd)
        );
        assert_eq!(
            read_string(&hkcu, &format!(r"{classes}\.apk"), "")
                .unwrap()
                .as_deref(),
            Some(BIFROST_APK_PROG_ID)
        );

        // Simulate the user switching .apkm to another app after registration.
        set_string(&hkcu, &format!(r"{classes}\.apkm"), "", "Other.Handler").unwrap();

        remove_apk_association_at(&hkcu, &classes).unwrap();
        assert!(read_string(&hkcu, &open_cmd, "").unwrap().is_none());
        assert!(read_string(&hkcu, &format!(r"{classes}\.apk"), "")
            .unwrap()
            .is_none());
        assert_eq!(
            read_string(&hkcu, &format!(r"{classes}\.apkm"), "")
                .unwrap()
                .as_deref(),
            Some("Other.Handler")
        );
        // Removing twice is a no-op.
        remove_apk_association_at(&hkcu, &classes).unwrap();

        hkcu.delete_subkey_all(r"Software\BifrostUnitTest").unwrap();
    }
}
