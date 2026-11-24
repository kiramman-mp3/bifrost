//! Native Windows 11 Toast notifications through WinRT with a BIFRÖST app identity.
//!
//! An unpackaged desktop process may only show toasts under an AppUserModelID (AUMID)
//! that Windows knows about. BIFRÖST registers its own AUMID under
//! `HKCU\Software\Classes\AppUserModelId\<AUMID>` (no elevation needed) with a display
//! name and icon, so toasts are attributed to "BIFRÖST" instead of borrowing Windows
//! PowerShell's identity. No child process or script is involved.

#[cfg(windows)]
use std::path::PathBuf;

#[cfg(windows)]
use bifrost_core::BifrostPaths;
use bifrost_core::{BifrostError, Result};

/// AppUserModelID used for every BIFRÖST toast notification.
pub const BIFROST_AUMID: &str = "BIFROST.Bifrost";

/// Display name shown by Windows as the toast source.
pub const BIFROST_TOAST_DISPLAY_NAME: &str = "BIFRÖST";

/// Toast icon owned by this crate, so it builds without the desktop app's directory.
#[cfg(windows)]
const TOAST_ICON_PNG: &[u8] = include_bytes!("../assets/bifrost-toast.png");

/// Registry key (relative to `HKCU`) that declares the BIFRÖST AUMID.
#[must_use]
pub fn aumid_registry_key(aumid: &str) -> String {
    format!(r"Software\Classes\AppUserModelId\{aumid}")
}

/// Registers (idempotently) the BIFRÖST AUMID with its display name and icon.
#[cfg(windows)]
pub fn ensure_toast_identity() -> Result<()> {
    let icon = ensure_toast_icon()?;
    register_aumid(
        &aumid_registry_key(BIFROST_AUMID),
        BIFROST_TOAST_DISPLAY_NAME,
        &icon,
    )
}

/// Non-Windows stub returning `Unsupported`.
#[cfg(not(windows))]
pub fn ensure_toast_identity() -> Result<()> {
    Err(unsupported())
}

/// Shows a Toast whose XML payload has already been built and escaped by the caller.
#[cfg(windows)]
pub fn show_toast_xml(xml: &str) -> Result<()> {
    use windows::core::HSTRING;
    use windows::Data::Xml::Dom::XmlDocument;
    use windows::UI::Notifications::{ToastNotification, ToastNotificationManager};

    ensure_toast_identity()?;

    let winrt = |context: &str, err: windows::core::Error| {
        BifrostError::Bridge(format!("Windows Toast {context} failed: {err}"))
    };
    let doc = XmlDocument::new().map_err(|e| winrt("XmlDocument creation", e))?;
    doc.LoadXml(&HSTRING::from(xml))
        .map_err(|e| winrt("XML parsing", e))?;
    let toast = ToastNotification::CreateToastNotification(&doc)
        .map_err(|e| winrt("notification creation", e))?;
    let notifier =
        ToastNotificationManager::CreateToastNotifierWithId(&HSTRING::from(BIFROST_AUMID))
            .map_err(|e| winrt("notifier creation", e))?;
    notifier.Show(&toast).map_err(|e| winrt("display", e))
}

/// Non-Windows stub returning `Unsupported`.
#[cfg(not(windows))]
pub fn show_toast_xml(_xml: &str) -> Result<()> {
    Err(unsupported())
}

#[cfg(windows)]
fn ensure_toast_icon() -> Result<PathBuf> {
    let paths = BifrostPaths::discover()?;
    let dir = paths.data_dir.join("assets");
    let icon = dir.join("bifrost-toast.png");
    let up_to_date = std::fs::read(&icon).is_ok_and(|bytes| bytes == TOAST_ICON_PNG);
    if !up_to_date {
        std::fs::create_dir_all(&dir).map_err(|source| BifrostError::Io {
            context: format!("creating '{}'", dir.display()),
            source,
        })?;
        std::fs::write(&icon, TOAST_ICON_PNG).map_err(|source| BifrostError::Io {
            context: format!("writing toast icon '{}'", icon.display()),
            source,
        })?;
    }
    Ok(icon)
}

#[cfg(windows)]
fn register_aumid(key_path: &str, display_name: &str, icon: &std::path::Path) -> Result<()> {
    use winreg::enums::HKEY_CURRENT_USER;
    use winreg::RegKey;

    let io = |source| BifrostError::Io {
        context: format!(r"registering toast identity under HKCU\{key_path}"),
        source,
    };
    let (key, _) = RegKey::predef(HKEY_CURRENT_USER)
        .create_subkey(key_path)
        .map_err(io)?;
    key.set_value("DisplayName", &display_name).map_err(io)?;
    key.set_value("IconUri", &icon.as_os_str()).map_err(io)?;
    Ok(())
}

#[cfg(not(windows))]
fn unsupported() -> BifrostError {
    BifrostError::Unsupported {
        feature: "Windows 11 Toast notifications".to_string(),
        planned_milestone: "Windows 11 host only".to_string(),
    }
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;
    use winreg::enums::HKEY_CURRENT_USER;
    use winreg::RegKey;

    #[test]
    fn registers_aumid_display_name_and_icon_under_hkcu() {
        let key_path = aumid_registry_key("BIFROST.UnitTest.Toast");
        let icon = std::env::temp_dir().join("bifrost-unit-test-icon.png");
        register_aumid(&key_path, "BIFRÖST Test", &icon).unwrap();

        let key = RegKey::predef(HKEY_CURRENT_USER)
            .open_subkey(&key_path)
            .unwrap();
        let name: String = key.get_value("DisplayName").unwrap();
        let uri: String = key.get_value("IconUri").unwrap();
        assert_eq!(name, "BIFRÖST Test");
        assert_eq!(uri, icon.display().to_string());

        RegKey::predef(HKEY_CURRENT_USER)
            .delete_subkey_all(&key_path)
            .unwrap();
    }

    #[test]
    fn embedded_toast_icon_is_a_png() {
        assert!(TOAST_ICON_PNG.starts_with(&[0x89, b'P', b'N', b'G']));
    }

    /// Shows a real toast on the desktop. Run manually:
    /// `cargo test -p bifrost-bridge toast -- --ignored`.
    #[test]
    #[ignore = "displays a real Windows notification"]
    fn live_shows_native_toast() {
        show_toast_xml(
            "<toast><visual><binding template=\"ToastGeneric\"><text>BIFRÖST</text>\
             <text>Native WinRT toast test</text></binding></visual></toast>",
        )
        .unwrap();
    }
}
