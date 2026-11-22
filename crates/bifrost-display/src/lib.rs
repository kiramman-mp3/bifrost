//! Independent per-app Android window launcher on Windows 11 (ADR-0003 Stage 2).
//!
//! Configures Android freeform/resizable multi-display flags and launches an
//! individual Android app in its own resizable Windows 11 desktop window using
//! an isolated Android `VirtualDisplay` (backed by `scrcpy >= 3.0 --new-display --start-app`
//! when available on the host).

#![forbid(unsafe_code)]

use std::path::PathBuf;
use std::process::{Command, Stdio};

use bifrost_bridge::AdbClient;
use bifrost_core::{BifrostError, Result};
use serde::{Deserialize, Serialize};
use tracing::info;
use valhalla::validate_package_name;

/// Dimensions and pixel density for an independent per-app Android `VirtualDisplay`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppWindowSpec {
    pub width: u32,
    pub height: u32,
    pub dpi: u32,
}

impl Default for AppWindowSpec {
    fn default() -> Self {
        Self {
            width: 1280,
            height: 720,
            dpi: 240,
        }
    }
}

impl AppWindowSpec {
    /// Validates window resolution and DPI bounds.
    pub fn validate(self) -> Result<()> {
        if !(320..=7680).contains(&self.width) || !(320..=4320).contains(&self.height) {
            return Err(BifrostError::ConfigValidation {
                field: "window.dimensions".to_string(),
                reason: "width/height must be between 320x320 and 7680x4320".to_string(),
            });
        }
        if !(120..=640).contains(&self.dpi) {
            return Err(BifrostError::ConfigValidation {
                field: "window.dpi".to_string(),
                reason: "DPI must be between 120 and 640".to_string(),
            });
        }
        Ok(())
    }
}

/// Metadata returned after spawning an independent Windows desktop window for an Android app.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppWindowHandle {
    pub serial: String,
    pub package_name: String,
    pub window_title: String,
    pub transport: String,
    pub pid: u32,
    pub spec: AppWindowSpec,
}

/// Controller for Android multi-display preparation and independent app windows.
#[derive(Debug, Clone, Default)]
pub struct DisplayController {
    adb: AdbClient,
    explicit_scrcpy: Option<PathBuf>,
}

impl DisplayController {
    /// Creates a display controller with default `adb` and `scrcpy` discovery.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            adb: AdbClient::new(),
            explicit_scrcpy: None,
        }
    }

    /// Locates a host `scrcpy` (>= 3.0) binary for 60 FPS Android `VirtualDisplay` projection.
    #[must_use]
    pub fn locate_scrcpy(&self) -> Option<PathBuf> {
        if let Some(ref p) = self.explicit_scrcpy {
            if p.is_file() {
                return Some(p.clone());
            }
        }
        if let Ok(env_bin) = std::env::var("BIFROST_SCRCPY_BIN") {
            let p = PathBuf::from(env_bin);
            if p.is_file() {
                return Some(p);
            }
        }
        if let Some(on_path) = find_on_path("scrcpy.exe").or_else(|| find_on_path("scrcpy")) {
            return Some(on_path);
        }
        if let Ok(local) = std::env::var("LOCALAPPDATA") {
            let user_install = PathBuf::from(local)
                .join("Programs")
                .join("scrcpy")
                .join("scrcpy.exe");
            if user_install.is_file() {
                return Some(user_install);
            }
        }
        None
    }

    /// Enables Android freeform and resizable-activity settings required for clean independent windows.
    pub fn prepare_android_freeform(&self, serial: &str) -> Result<()> {
        for (key, val) in [
            ("enable_freeform_support", "1"),
            ("force_resizable_activities", "1"),
            ("enable_non_resizable_multi_window", "1"),
        ] {
            let _ = self
                .adb
                .run_adb(&["-s", serial, "shell", "settings", "put", "global", key, val])?;
        }
        Ok(())
    }

    /// Builds deterministic arguments for launching `package` inside an isolated Android `VirtualDisplay`.
    pub fn build_virtual_display_args(
        &self,
        serial: &str,
        package: &str,
        spec: AppWindowSpec,
    ) -> Result<Vec<String>> {
        validate_package_name(package)?;
        spec.validate()?;

        let display_spec = format!("{}x{}/{}", spec.width, spec.height, spec.dpi);
        let title = format!("BIFRÖST — {package}");
        // Dedicated single-app VirtualDisplay flags (WSA-grade windowing):
        // - `--flex-display`: continuously resizes the Android VirtualDisplay to match
        //   the Windows 11 window in real time so Android apps reflow natively (no bars).
        // - `--no-window-aspect-ratio-lock`: allows free horizontal/vertical resizing
        //   and Windows 11 Snap Layouts (`Win+Left`/`Win+Right`, maximize, phone/tablet).
        // - `--no-vd-system-decorations`: removes Android Home launcher, status bar,
        //   and navigation bar so the window belongs strictly to this app.
        // - `--display-ime-policy=local`: keeps keyboard/IME input inside this window.
        // - `--mouse-bind=b+++`: maps right-click to Back and disables Home/Recents clicks.
        // - `--start-app=+{package}`: force-restarts the app cleanly inside this display.
        // - Omit `--no-vd-destroy-content`: closing the Windows 11 window destroys the
        //   VirtualDisplay content cleanly instead of moving it to Display 0.
        Ok(vec![
            "-s".to_string(),
            serial.to_string(),
            format!("--new-display={display_spec}"),
            "--flex-display".to_string(),
            "--no-window-aspect-ratio-lock".to_string(),
            "--no-vd-system-decorations".to_string(),
            "--display-ime-policy=local".to_string(),
            "--mouse-bind=b+++".to_string(),
            format!("--start-app=+{package}"),
            format!("--window-title={title}"),
        ])
    }

    /// Launches an Android application in its own independent Windows 11 desktop window.
    pub fn launch_in_independent_window(
        &self,
        serial: &str,
        package: &str,
        spec: AppWindowSpec,
    ) -> Result<AppWindowHandle> {
        let args = self.build_virtual_display_args(serial, package, spec)?;
        let scrcpy_bin = self.locate_scrcpy().ok_or_else(|| {
            BifrostError::Bridge(
                "independent per-app VirtualDisplay window requires `scrcpy` v3.0+ (install via `winget install Genymobile.scrcpy` or set BIFROST_SCRCPY_BIN)"
                    .to_string(),
            )
        })?;

        self.prepare_android_freeform(serial)?;

        let mut cmd = Command::new(&scrcpy_bin);
        cmd.args(&args)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());

        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            cmd.creation_flags(CREATE_NEW_PROCESS_GROUP | CREATE_NO_WINDOW);
        }

        let child = cmd.spawn().map_err(|source| BifrostError::Io {
            context: format!("launching per-app window via '{}'", scrcpy_bin.display()),
            source,
        })?;

        let pid = child.id();
        let window_title = format!("BIFRÖST — {package}");
        info!(
            serial,
            package,
            pid,
            width = spec.width,
            height = spec.height,
            "launched Android app in independent Windows 11 VirtualDisplay window"
        );

        Ok(AppWindowHandle {
            serial: serial.to_string(),
            package_name: package.to_string(),
            window_title,
            transport: "virtual-display-scrcpy".to_string(),
            pid,
            spec,
        })
    }
}

fn find_on_path(binary_name: &str) -> Option<PathBuf> {
    let path_var = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path_var) {
        let candidate = dir.join(binary_name);
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_virtual_display_args_and_validates_bounds() {
        let ctrl = DisplayController::new();
        let args = ctrl
            .build_virtual_display_args(
                "127.0.0.1:5555",
                "org.fdroid.fdroid",
                AppWindowSpec {
                    width: 1280,
                    height: 800,
                    dpi: 240,
                },
            )
            .unwrap();
        assert!(args.contains(&"--new-display=1280x800/240".to_string()));
        assert!(args.contains(&"--flex-display".to_string()));
        assert!(args.contains(&"--no-window-aspect-ratio-lock".to_string()));
        assert!(args.contains(&"--no-vd-system-decorations".to_string()));
        assert!(args.contains(&"--display-ime-policy=local".to_string()));
        assert!(args.contains(&"--mouse-bind=b+++".to_string()));
        assert!(args.contains(&"--start-app=+org.fdroid.fdroid".to_string()));
        assert!(!args.contains(&"--no-vd-destroy-content".to_string()));

        // Invalid package or out-of-range DPI rejected.
        assert!(ctrl
            .build_virtual_display_args("127.0.0.1:5555", "bad;cmd", AppWindowSpec::default())
            .is_err());
        assert!(ctrl
            .build_virtual_display_args(
                "127.0.0.1:5555",
                "org.fdroid.fdroid",
                AppWindowSpec {
                    width: 1280,
                    height: 720,
                    dpi: 50,
                },
            )
            .is_err());
    }
}
