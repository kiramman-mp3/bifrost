//! Windows Desktop & Start Menu shortcut management for installed Android apps.
//!
//! Creates `.lnk` shortcuts on the user's Windows Desktop and Start Menu
//! (`BIFRÖST Apps`) pointing to `bifrost.exe apps launch <package> --window`,
//! extracts native PNG launcher icons into `.ico` files when available, and
//! tracks created shortcuts in `shortcuts.json` for clean removal on uninstall.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use bifrost_core::{BifrostError, BifrostPaths, Result};
use serde::{Deserialize, Serialize};
use tracing::{info, warn};

use crate::apk::{extract_apk_icon_png, wrap_png_as_ico};
use crate::package::validate_package_name;

const RESERVED_WINDOWS_NAMES: &[&str] = &[
    "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
    "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
];

/// Metadata recorded for Windows shortcuts created for an Android application.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShortcutRecord {
    pub package_name: String,
    pub display_name: String,
    pub desktop_shortcut: PathBuf,
    pub start_menu_shortcut: PathBuf,
    pub icon_path: Option<PathBuf>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct ShortcutRegistry {
    #[serde(default)]
    entries: BTreeMap<String, ShortcutRecord>,
}

/// Manages Windows Desktop and Start Menu shortcuts for BIFRÖST Android apps.
#[derive(Debug, Clone)]
pub struct ShortcutManager {
    data_dir: PathBuf,
}

impl ShortcutManager {
    /// Creates a shortcut manager using standard per-user BIFRÖST data directories.
    pub fn discover() -> Result<Self> {
        let paths = BifrostPaths::discover()?;
        Ok(Self::new(paths.data_dir))
    }

    /// Creates a shortcut manager rooted at an explicit data directory.
    #[must_use]
    pub fn new(data_dir: impl Into<PathBuf>) -> Self {
        Self {
            data_dir: data_dir.into(),
        }
    }

    fn registry_path(&self) -> PathBuf {
        self.data_dir.join("shortcuts.json")
    }

    fn icons_dir(&self) -> PathBuf {
        self.data_dir.join("icons")
    }

    fn load_registry(&self) -> Result<ShortcutRegistry> {
        let path = self.registry_path();
        if !path.is_file() {
            return Ok(ShortcutRegistry::default());
        }
        let raw = fs::read_to_string(&path).map_err(|source| BifrostError::Io {
            context: format!("reading shortcut registry '{}'", path.display()),
            source,
        })?;
        let trimmed = raw.strip_prefix('\u{feff}').unwrap_or(&raw);
        serde_json::from_str(trimmed)
            .map_err(|err| BifrostError::Runtime(format!("invalid shortcuts.json: {err}")))
    }

    fn save_registry(&self, registry: &ShortcutRegistry) -> Result<()> {
        let path = self.registry_path();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|source| BifrostError::Io {
                context: format!(
                    "creating shortcut registry directory '{}'",
                    parent.display()
                ),
                source,
            })?;
        }
        let json = serde_json::to_string_pretty(registry)
            .map_err(|err| BifrostError::ConfigSerialize(err.to_string()))?;
        fs::write(&path, json).map_err(|source| BifrostError::Io {
            context: format!("writing shortcut registry '{}'", path.display()),
            source,
        })
    }

    /// Lists all recorded Windows shortcuts for BIFRÖST Android apps.
    pub fn list_shortcuts(&self) -> Result<Vec<ShortcutRecord>> {
        Ok(self.load_registry()?.entries.into_values().collect())
    }

    /// Creates (or updates) Windows Desktop and Start Menu shortcuts for `package_name`.
    ///
    /// When `apk_path` is provided and contains a PNG launcher icon, it is extracted
    /// and wrapped into a native Windows `.ico` file under BIFRÖST's data directory.
    pub fn create_for_app(
        &self,
        package_name: &str,
        explicit_name: Option<&str>,
        apk_path: Option<&Path>,
    ) -> Result<ShortcutRecord> {
        validate_package_name(package_name)?;

        let display_name = derive_display_name(package_name, explicit_name, apk_path);
        let bifrost_exe = locate_bifrost_cli()?;
        let work_dir = bifrost_exe
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| self.data_dir.clone());

        let (icon_file, icon_location) = self.prepare_icon(package_name, apk_path, &bifrost_exe);
        let args = format!("apps launch {package_name} --window");
        let description = format!("Launch {display_name} ({package_name}) via BIFRÖST");

        let (desktop_shortcut, start_menu_shortcut) = create_windows_shortcuts_via_com(
            &display_name,
            &bifrost_exe,
            &args,
            &work_dir,
            &icon_location,
            &description,
        )?;

        let record = ShortcutRecord {
            package_name: package_name.to_string(),
            display_name,
            desktop_shortcut,
            start_menu_shortcut,
            icon_path: icon_file,
        };

        let mut registry = self.load_registry().unwrap_or_default();
        // Clean up older shortcuts if the display name changed on reinstall.
        if let Some(prev) = registry.entries.get(package_name) {
            if prev.desktop_shortcut != record.desktop_shortcut {
                let _ = fs::remove_file(&prev.desktop_shortcut);
            }
            if prev.start_menu_shortcut != record.start_menu_shortcut {
                let _ = fs::remove_file(&prev.start_menu_shortcut);
            }
        }
        registry
            .entries
            .insert(package_name.to_string(), record.clone());
        self.save_registry(&registry)?;

        info!(
            package = package_name,
            desktop = %record.desktop_shortcut.display(),
            start_menu = %record.start_menu_shortcut.display(),
            "created Windows Desktop and Start Menu shortcuts for Android app"
        );
        Ok(record)
    }

    /// Removes Desktop shortcut, Start Menu shortcut, and cached `.ico` for `package_name`.
    pub fn remove_for_app(&self, package_name: &str) -> Result<Option<ShortcutRecord>> {
        validate_package_name(package_name)?;
        let mut registry = self.load_registry().unwrap_or_default();
        let removed = registry.entries.remove(package_name);
        if let Some(ref rec) = removed {
            for p in [&rec.desktop_shortcut, &rec.start_menu_shortcut] {
                if p.is_file() {
                    let _ = fs::remove_file(p);
                }
            }
            if let Some(ref ico) = rec.icon_path {
                if ico.is_file() {
                    let _ = fs::remove_file(ico);
                }
            }
            self.save_registry(&registry)?;
            info!(
                package = package_name,
                "removed Windows Desktop and Start Menu shortcuts for Android app"
            );
        }
        Ok(removed)
    }

    fn prepare_icon(
        &self,
        package_name: &str,
        apk_path: Option<&Path>,
        bifrost_exe: &Path,
    ) -> (Option<PathBuf>, String) {
        if let Some(apk) = apk_path {
            if let Some(png_bytes) = extract_apk_icon_png(apk) {
                if let Some(ico_bytes) = wrap_png_as_ico(&png_bytes) {
                    let icons_dir = self.icons_dir();
                    if fs::create_dir_all(&icons_dir).is_ok() {
                        let ico_path = icons_dir.join(format!("{package_name}.ico"));
                        if fs::write(&ico_path, ico_bytes).is_ok() {
                            let loc = format!("{},0", ico_path.display());
                            return (Some(ico_path), loc);
                        }
                    }
                }
            }
        }

        // Fall back to bifrost-ui.exe icon if installed alongside bifrost.exe.
        if let Some(dir) = bifrost_exe.parent() {
            let ui_exe = dir.join("bifrost-ui.exe");
            if ui_exe.is_file() {
                return (None, format!("{},0", ui_exe.display()));
            }
        }
        (None, format!("{},0", bifrost_exe.display()))
    }
}

/// Derives a clean, safe Windows shortcut display name from explicit input, APK filename,
/// or Android package identifier.
#[must_use]
pub fn derive_display_name(
    package_name: &str,
    explicit_name: Option<&str>,
    apk_path: Option<&Path>,
) -> String {
    if let Some(name) = explicit_name {
        if let Some(clean) = sanitize_shortcut_name(name) {
            return clean;
        }
    }

    if let Some(apk) = apk_path {
        if let Some(stem) = apk.file_stem().and_then(|s| s.to_str()) {
            let cleaned = strip_apk_filename_noise(stem);
            if let Some(clean) = sanitize_shortcut_name(&cleaned) {
                return clean;
            }
        }
    }

    let last_segment = package_name.rsplit('.').next().unwrap_or(package_name);
    let mut chars = last_segment.chars();
    let titled = match chars.next() {
        Some(first) => format!("{}{}", first.to_uppercase(), chars.as_str()),
        None => "Android App".to_string(),
    };
    sanitize_shortcut_name(&titled).unwrap_or_else(|| "Android App".to_string())
}

fn strip_apk_filename_noise(stem: &str) -> String {
    // Trim trailing version/ABI suffixes like `-1.2.3-x86_64` before turning `_` into spaces.
    let mut parts: Vec<&str> = stem.split('-').collect();
    while parts.len() > 1 {
        let last = parts.last().copied().unwrap_or("").trim();
        let lower = last.to_ascii_lowercase();
        let is_noise = lower.is_empty()
            || matches!(
                lower.as_str(),
                "x86_64" | "x86" | "arm64" | "v8a" | "universal" | "nodpi" | "release" | "debug"
            )
            || last.chars().all(|c| c.is_ascii_digit() || c == '.');
        if is_noise {
            parts.pop();
        } else {
            break;
        }
    }
    parts.join("-").replace('_', " ").trim().to_string()
}

/// Sanitizes an arbitrary string into a safe Windows `.lnk` filename stem.
#[must_use]
pub fn sanitize_shortcut_name(raw: &str) -> Option<String> {
    let mut out = String::with_capacity(raw.len().min(64));
    let mut last_was_space = false;

    for ch in raw.chars() {
        if ch.is_control() || matches!(ch, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*') {
            if !last_was_space && !out.is_empty() {
                out.push(' ');
                last_was_space = true;
            }
            continue;
        }
        if ch.is_whitespace() {
            if !last_was_space && !out.is_empty() {
                out.push(' ');
                last_was_space = true;
            }
        } else {
            out.push(ch);
            last_was_space = false;
        }
        if out.len() >= 64 {
            break;
        }
    }

    let trimmed = out.trim().trim_end_matches('.').trim().to_string();
    if trimmed.is_empty() || trimmed == "." || trimmed == ".." {
        return None;
    }

    let upper = trimmed.to_ascii_uppercase();
    if RESERVED_WINDOWS_NAMES.contains(&upper.as_str()) {
        return Some(format!("{trimmed} App"));
    }

    Some(trimmed)
}

pub(crate) fn locate_bifrost_cli() -> Result<PathBuf> {
    if let Ok(local) = std::env::var("LOCALAPPDATA") {
        let installed = PathBuf::from(local)
            .join("Programs")
            .join("BIFRÖST")
            .join("bifrost.exe");
        if installed.is_file() {
            return Ok(installed);
        }
    }

    if let Ok(exe) = std::env::current_exe() {
        if exe
            .file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| n.eq_ignore_ascii_case("bifrost.exe") || n == "bifrost")
        {
            return Ok(exe);
        }
        if let Some(dir) = exe.parent() {
            let sibling = dir.join("bifrost.exe");
            if sibling.is_file() {
                return Ok(sibling);
            }
        }
    }

    Err(BifrostError::Runtime(
        "unable to locate bifrost.exe to create Windows app shortcuts".to_string(),
    ))
}

#[cfg(windows)]
fn create_windows_shortcuts_via_com(
    display_name: &str,
    target_exe: &Path,
    arguments: &str,
    work_dir: &Path,
    icon_location: &str,
    description: &str,
) -> Result<(PathBuf, PathBuf)> {
    // Fixed static script reading all inputs exclusively from environment variables
    // to prevent any command or PowerShell string interpolation injection.
    const SCRIPT: &str = r#"
$ErrorActionPreference = 'Stop'
$OutputEncoding = [Console]::OutputEncoding = [System.Text.Encoding]::UTF8
$desktopDir = [Environment]::GetFolderPath('Desktop')
$startMenuDir = Join-Path ([Environment]::GetFolderPath('Programs')) 'BIFRÖST Apps'
New-Item -ItemType Directory -Force -Path $startMenuDir | Out-Null

$fileName = "$env:BIFROST_SC_NAME.lnk"
$desktopLnk = Join-Path $desktopDir $fileName
$startMenuLnk = Join-Path $startMenuDir $fileName

$shell = New-Object -ComObject WScript.Shell
foreach ($lnkPath in @($desktopLnk, $startMenuLnk)) {
    $sc = $shell.CreateShortcut($lnkPath)
    $sc.TargetPath = $env:BIFROST_SC_TARGET
    $sc.Arguments = $env:BIFROST_SC_ARGS
    $sc.WorkingDirectory = $env:BIFROST_SC_WORKDIR
    $sc.IconLocation = $env:BIFROST_SC_ICON
    $sc.Description = $env:BIFROST_SC_DESC
    $sc.WindowStyle = 7
    $sc.Save()
}
Write-Output $desktopLnk
Write-Output $startMenuLnk
"#;

    let output = Command::new("powershell.exe")
        .args(["-NoProfile", "-NonInteractive", "-Command", SCRIPT])
        .env("BIFROST_SC_NAME", display_name)
        .env("BIFROST_SC_TARGET", target_exe)
        .env("BIFROST_SC_ARGS", arguments)
        .env("BIFROST_SC_WORKDIR", work_dir)
        .env("BIFROST_SC_ICON", icon_location)
        .env("BIFROST_SC_DESC", description)
        .output()
        .map_err(|source| BifrostError::Io {
            context: "invoking PowerShell WScript.Shell to create Windows shortcuts".to_string(),
            source,
        })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        warn!(error = %stderr, "failed creating Windows shortcuts");
        return Err(BifrostError::Runtime(format!(
            "failed creating Windows Desktop/Start Menu shortcuts: {stderr}"
        )));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut lines = stdout.lines().map(str::trim).filter(|l| !l.is_empty());
    let desktop = lines
        .next()
        .map(PathBuf::from)
        .ok_or_else(|| BifrostError::Runtime("missing desktop shortcut path output".to_string()))?;
    let start_menu = lines.next().map(PathBuf::from).ok_or_else(|| {
        BifrostError::Runtime("missing start menu shortcut path output".to_string())
    })?;

    Ok((desktop, start_menu))
}

#[cfg(not(windows))]
fn create_windows_shortcuts_via_com(
    _display_name: &str,
    _target_exe: &Path,
    _arguments: &str,
    _work_dir: &Path,
    _icon_location: &str,
    _description: &str,
) -> Result<(PathBuf, PathBuf)> {
    Err(BifrostError::Unsupported {
        feature: "Windows Desktop/Start Menu shortcuts".to_string(),
        milestone: "Windows 11 host only".to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitizes_windows_shortcut_names_safely() {
        assert_eq!(
            sanitize_shortcut_name("F-Droid"),
            Some("F-Droid".to_string())
        );
        assert_eq!(
            sanitize_shortcut_name("My:App/Name*?  .."),
            Some("My App Name".to_string())
        );
        assert_eq!(sanitize_shortcut_name("CON"), Some("CON App".to_string()));
        assert_eq!(sanitize_shortcut_name("..."), None);
    }

    #[test]
    fn derives_clean_display_names_from_apk_and_package() {
        assert_eq!(
            derive_display_name(
                "org.fdroid.fdroid",
                None,
                Some(Path::new(r"C:\dl\F-Droid.apk"))
            ),
            "F-Droid"
        );
        assert_eq!(
            derive_display_name(
                "org.mozilla.firefox",
                None,
                Some(Path::new(r"C:\dl\Firefox_130-130.0.1-x86_64.apk"))
            ),
            "Firefox 130"
        );
        assert_eq!(
            derive_display_name("com.android.settings", None, None),
            "Settings"
        );
        assert_eq!(
            derive_display_name("org.fdroid.fdroid", Some("Custom F-Droid"), None),
            "Custom F-Droid"
        );
    }
}
