//! Windows Desktop & Start Menu shortcut management for installed Android apps.
//!
//! Creates `.lnk` shortcuts on the user's Windows Desktop and Start Menu
//! (`BIFRÖST Apps`) pointing to `bifrost.exe apps launch <package> --window`,
//! extracts native PNG launcher icons into `.ico` files when available, and
//! tracks created shortcuts in `shortcuts.json` for clean removal on uninstall.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

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
        self.create_for_app_with_icon(package_name, explicit_name, apk_path, None)
    }

    /// Creates (or updates) Windows shortcuts using optional pre-extracted PNG icon bytes
    /// to avoid re-opening large `.apk`/`.apkm` archives during installation.
    pub fn create_for_app_with_icon(
        &self,
        package_name: &str,
        explicit_name: Option<&str>,
        apk_path: Option<&Path>,
        preextracted_icon_png: Option<&[u8]>,
    ) -> Result<ShortcutRecord> {
        validate_package_name(package_name)?;

        let display_name = derive_display_name(package_name, explicit_name, apk_path);
        let bifrost_exe = locate_bifrost_cli()?;
        let (target_exe, args) = resolve_shortcut_launcher(&bifrost_exe, package_name);
        let work_dir = target_exe
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| self.data_dir.clone());

        let (icon_file, icon_location) =
            self.prepare_icon(package_name, apk_path, preextracted_icon_png, &bifrost_exe);
        let description = format!("Launch {display_name} ({package_name}) via BIFRÖST");

        let (desktop_shortcut, start_menu_shortcut) = create_windows_shortcuts(
            &display_name,
            &target_exe,
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
        preextracted_icon_png: Option<&[u8]>,
        bifrost_exe: &Path,
    ) -> (Option<PathBuf>, String) {
        let extracted_owned;
        let png_opt = match preextracted_icon_png {
            Some(bytes) => Some(bytes),
            None => {
                extracted_owned = apk_path.and_then(extract_apk_icon_png);
                extracted_owned.as_deref()
            }
        };
        if let Some(png_bytes) = png_opt {
            if let Some(ico_bytes) = wrap_png_as_ico(png_bytes) {
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

    // Windows silently drops trailing dots and spaces, so strip any mix of them.
    let trimmed = out
        .trim_start()
        .trim_end_matches(|c: char| c == '.' || c.is_whitespace())
        .to_string();
    if trimmed.is_empty() || trimmed == "." || trimmed == ".." {
        return None;
    }

    // Windows treats the part before the first '.' (ignoring trailing spaces) as the
    // device name, so `CON.txt` or `NUL .x` are reserved too.
    let (stem, rest) = trimmed.split_at(trimmed.find('.').unwrap_or(trimmed.len()));
    if is_reserved_windows_stem(stem) {
        return Some(format!("{} App{rest}", stem.trim_end()));
    }

    Some(trimmed)
}

/// Returns true when `stem` is a Windows reserved device name (`CON`, `NUL`, `COM1`,
/// `LPT¹`, ...), compared case-insensitively and ignoring trailing spaces.
fn is_reserved_windows_stem(stem: &str) -> bool {
    let upper = stem.trim_end().to_ascii_uppercase();
    if RESERVED_WINDOWS_NAMES.contains(&upper.as_str()) {
        return true;
    }
    // Superscript digits are also reserved after COM/LPT (`COM¹`, `LPT³`).
    let mut chars = upper.chars();
    let prefix: String = chars.by_ref().take(3).collect();
    let tail: String = chars.collect();
    (prefix == "COM" || prefix == "LPT") && matches!(tail.as_str(), "¹" | "²" | "³")
}

/// Selects the cleanest launcher executable for Windows `.lnk` shortcuts.
///
/// Prefers `bifrost-ui.exe --launch-app <pkg>` (pure GUI subsystem, zero console flash)
/// when installed alongside `bifrost.exe`, falling back to `bifrost.exe apps launch <pkg> --window`.
fn resolve_shortcut_launcher(bifrost_exe: &Path, package_name: &str) -> (PathBuf, String) {
    if let Some(dir) = bifrost_exe.parent() {
        let ui_exe = dir.join("bifrost-ui.exe");
        if ui_exe.is_file() {
            return (ui_exe, format!("--launch-app {package_name}"));
        }
    }
    (
        bifrost_exe.to_path_buf(),
        format!("apps launch {package_name} --window"),
    )
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

/// Resolves the current user's Desktop and Start Menu `Programs` folders.
///
/// The Desktop comes from the Known Folder API (via `directories`), so OneDrive or
/// policy redirection is honoured; `Programs` lives under Roaming AppData.
fn shortcut_folders() -> Result<(PathBuf, PathBuf)> {
    let user_dirs = directories::UserDirs::new().ok_or_else(|| {
        BifrostError::Runtime("unable to resolve the user's Desktop folder".to_string())
    })?;
    let desktop = user_dirs
        .desktop_dir()
        .map(Path::to_path_buf)
        .ok_or_else(|| BifrostError::Runtime("user has no Desktop folder".to_string()))?;
    let base = directories::BaseDirs::new().ok_or_else(|| {
        BifrostError::Runtime("unable to resolve the user's AppData folder".to_string())
    })?;
    let programs = base
        .config_dir()
        .join("Microsoft")
        .join("Windows")
        .join("Start Menu")
        .join("Programs")
        .join("BIFRÖST Apps");
    Ok((desktop, programs))
}

/// Splits a Shell icon location (`C:\path\file.ico,0`) into path and index.
fn split_icon_location(icon_location: &str) -> (String, i32) {
    if let Some((path, index)) = icon_location.rsplit_once(',') {
        if let Ok(index) = index.trim().parse::<i32>() {
            return (path.to_string(), index);
        }
    }
    (icon_location.to_string(), 0)
}

/// Writes one `.lnk` file in the native Shell Link binary format (no COM, no scripts).
fn write_shell_link(
    lnk_path: &Path,
    target_exe: &Path,
    arguments: &str,
    work_dir: &Path,
    icon_location: &str,
    description: &str,
) -> Result<()> {
    // `mslnk` unwraps UTF-8 conversions internally; reject non-UTF-8 paths up front.
    let (Some(target_str), Some(work_dir_str)) = (target_exe.to_str(), work_dir.to_str()) else {
        return Err(BifrostError::Runtime(format!(
            "shortcut target '{}' is not a valid UTF-8 path",
            target_exe.display()
        )));
    };
    let mut link = mslnk::ShellLink::new(target_str).map_err(|err| {
        BifrostError::Runtime(format!(
            "failed preparing shortcut to '{target_str}': {err}"
        ))
    })?;
    let (icon_path, icon_index) = split_icon_location(icon_location);
    link.set_arguments((!arguments.is_empty()).then(|| arguments.to_string()));
    link.set_working_dir(Some(work_dir_str.to_string()));
    link.set_icon_location(Some(icon_path));
    link.set_name(Some(description.to_string()));
    link.header_mut().set_icon_index(icon_index);
    // Matches the former WScript `WindowStyle = 7`: the launcher is a GUI-subsystem
    // helper that must not steal focus before the Android window appears.
    link.header_mut()
        .set_show_command(mslnk::ShowCommand::ShowMinNoActive);
    link.create_lnk(lnk_path).map_err(|err| {
        BifrostError::Runtime(format!(
            "failed writing shortcut '{}': {err}",
            lnk_path.display()
        ))
    })
}

#[cfg(windows)]
fn create_windows_shortcuts(
    display_name: &str,
    target_exe: &Path,
    arguments: &str,
    work_dir: &Path,
    icon_location: &str,
    description: &str,
) -> Result<(PathBuf, PathBuf)> {
    let (desktop_dir, start_menu_dir) = shortcut_folders()?;
    fs::create_dir_all(&start_menu_dir).map_err(|source| BifrostError::Io {
        context: format!("creating '{}'", start_menu_dir.display()),
        source,
    })?;

    let file_name = format!("{display_name}.lnk");
    let desktop_lnk = desktop_dir.join(&file_name);
    let start_menu_lnk = start_menu_dir.join(&file_name);
    for lnk in [&desktop_lnk, &start_menu_lnk] {
        write_shell_link(
            lnk,
            target_exe,
            arguments,
            work_dir,
            icon_location,
            description,
        )
        .inspect_err(|err| warn!(error = %err, "failed creating Windows shortcut"))?;
    }
    Ok((desktop_lnk, start_menu_lnk))
}

#[cfg(not(windows))]
fn create_windows_shortcuts(
    _display_name: &str,
    _target_exe: &Path,
    _arguments: &str,
    _work_dir: &Path,
    _icon_location: &str,
    _description: &str,
) -> Result<(PathBuf, PathBuf)> {
    Err(BifrostError::Unsupported {
        feature: "Windows Desktop/Start Menu shortcuts".to_string(),
        planned_milestone: "Windows 11 host only".to_string(),
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
    fn strips_alternating_trailing_dots_and_spaces() {
        // Found by the release proptest: an invalid character between dots turned into a
        // space, and a single dot-then-space trim left a trailing '.'.
        assert_eq!(
            sanitize_shortcut_name("Viewer 0 .\"."),
            Some("Viewer 0".to_string())
        );
        assert_eq!(sanitize_shortcut_name("App . . ."), Some("App".to_string()));
        assert_eq!(sanitize_shortcut_name(". . ."), None);
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

    /// Asks the real Windows Shell (WScript.Shell, read-only) to parse a link written by
    /// `write_shell_link`. Run with `cargo test -p valhalla shortcuts -- --ignored`.
    #[cfg(windows)]
    #[test]
    #[ignore = "invokes the Windows Shell through PowerShell"]
    fn live_windows_shell_reads_written_link() {
        let dir = tempfile::tempdir().unwrap();
        // Copy a real executable into a non-ASCII folder, like `Programs\BIFRÖST`.
        let unicode_dir = dir.path().join("BIFRÖST Ünïcödé");
        fs::create_dir_all(&unicode_dir).unwrap();
        let target = unicode_dir.join("bifrost-ui.exe");
        fs::copy(std::env::current_exe().unwrap(), &target).unwrap();
        let lnk = dir.path().join("BIFRÖST Live Test.lnk");
        write_shell_link(
            &lnk,
            &target,
            "--launch-app com.example.app",
            dir.path(),
            &format!("{},0", target.display()),
            "Launch via BIFRÖST",
        )
        .unwrap();

        let script = "$OutputEncoding=[Console]::OutputEncoding=[Text.Encoding]::UTF8;\
            $s=(New-Object -ComObject WScript.Shell).CreateShortcut($env:LNK);\
            $s.TargetPath;$s.Arguments;$s.Description;$s.IconLocation;$s.WindowStyle";
        let out = std::process::Command::new("powershell.exe")
            .args(["-NoProfile", "-NonInteractive", "-Command", script])
            .env("LNK", &lnk)
            .output()
            .unwrap();
        let text = String::from_utf8_lossy(&out.stdout);
        let lines: Vec<&str> = text.lines().map(str::trim).collect();
        assert_eq!(
            lines[0].to_lowercase(),
            target.display().to_string().to_lowercase()
        );
        assert_eq!(lines[1], "--launch-app com.example.app");
        assert_eq!(lines[2], "Launch via BIFRÖST");
        assert_eq!(lines[3], format!("{},0", target.display()));
        assert_eq!(lines[4], "7");
    }

    #[test]
    fn splits_shell_icon_locations() {
        assert_eq!(
            split_icon_location(r"C:\BIFRÖST\icons\app.ico,0"),
            (r"C:\BIFRÖST\icons\app.ico".to_string(), 0)
        );
        assert_eq!(
            split_icon_location(r"C:\x\bifrost-ui.exe,3"),
            (r"C:\x\bifrost-ui.exe".to_string(), 3)
        );
        assert_eq!(
            split_icon_location(r"C:\a,b\icon.ico"),
            (r"C:\a,b\icon.ico".to_string(), 0)
        );
    }

    #[test]
    fn writes_unicode_shell_link_file() {
        let dir = tempfile::tempdir().unwrap();
        let target = std::env::current_exe().unwrap();
        let lnk = dir.path().join("Ünïcödé App.lnk");
        write_shell_link(
            &lnk,
            &target,
            "--launch-app com.example.app",
            dir.path(),
            r"C:\BIFRÖST\icons\app.ico,0",
            "Launch Ünïcödé App via BIFRÖST",
        )
        .unwrap();

        let bytes = fs::read(&lnk).unwrap();
        // Shell Link header: size 0x4C followed by the fixed LinkCLSID.
        assert_eq!(&bytes[..4], &[0x4C, 0, 0, 0]);
        let utf16 = |s: &str| -> Vec<u8> { s.encode_utf16().flat_map(u16::to_le_bytes).collect() };
        let contains = |needle: &[u8]| bytes.windows(needle.len()).any(|w| w == needle);
        assert!(contains(&utf16("--launch-app com.example.app")));
        assert!(contains(&utf16(r"C:\BIFRÖST\icons\app.ico")));
        assert!(contains(&utf16("Launch Ünïcödé App via BIFRÖST")));
    }
}
