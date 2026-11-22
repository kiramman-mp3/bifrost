//! Valhalla real Android package manager orchestrator backed by `bifrost-bridge`.

use std::path::Path;

use bifrost_bridge::AdbClient;
use bifrost_core::{BifrostError, Result};
use serde::{Deserialize, Serialize};
use tracing::info;

use crate::apk::{inspect_apk, ApkInspection};
use crate::package::{
    parse_dumpsys_package, parse_pm_list_packages, parse_resolved_activity, validate_package_name,
    InstalledPackage, PackageDetails,
};

/// Result returned after a real APK installation via Android `PackageManager`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InstallOutcome {
    pub serial: String,
    pub inspection: ApkInspection,
    pub pm_output: String,
}

/// Result returned after launching an Android application.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LaunchOutcome {
    pub serial: String,
    pub package_name: String,
    pub component: Option<String>,
    pub am_output: String,
}

/// Manages real Android applications through Android's `pm`, `am`, and `dumpsys` over Bridge/ADB.
#[derive(Debug, Clone, Default)]
pub struct ValhallaManager {
    adb: AdbClient,
}

impl ValhallaManager {
    /// Creates a Valhalla manager using the default host ADB discovery.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            adb: AdbClient::new(),
        }
    }

    /// Creates a Valhalla manager backed by an explicit [`AdbClient`].
    #[must_use]
    pub const fn with_adb(adb: AdbClient) -> Self {
        Self { adb }
    }

    /// Resolves the target online Android device serial (explicit or single connected `device`).
    pub fn resolve_target_serial(&self, explicit: Option<&str>) -> Result<String> {
        if let Some(serial) = explicit {
            return Ok(serial.to_string());
        }
        let online: Vec<_> = self
            .adb
            .list_devices()?
            .into_iter()
            .filter(|d| d.state == "device")
            .collect();

        match online.len() {
            0 => Err(BifrostError::Package(
                "no online Android device found (start Midgard or connect an ADB device first)"
                    .to_string(),
            )),
            1 => Ok(online[0].serial.clone()),
            _ => {
                let list: Vec<&str> = online.iter().map(|d| d.serial.as_str()).collect();
                Err(BifrostError::Package(format!(
                    "multiple Android devices online ({}); specify --serial <id>",
                    list.join(", ")
                )))
            }
        }
    }

    /// Lists installed Android packages via `pm list packages -f`.
    pub fn list_packages(
        &self,
        serial: Option<&str>,
        third_party_only: bool,
    ) -> Result<Vec<InstalledPackage>> {
        let target = self.resolve_target_serial(serial)?;
        let out = if third_party_only {
            self.adb
                .run_adb(&["-s", &target, "shell", "pm", "list", "packages", "-f", "-3"])?
        } else {
            self.adb
                .run_adb(&["-s", &target, "shell", "pm", "list", "packages", "-f"])?
        };
        Ok(parse_pm_list_packages(&out))
    }

    /// Queries detailed metadata for an installed package via `dumpsys package` and `cmd package`.
    pub fn package_info(&self, serial: Option<&str>, package: &str) -> Result<PackageDetails> {
        validate_package_name(package)?;
        let target = self.resolve_target_serial(serial)?;
        let dump = self
            .adb
            .run_adb(&["-s", &target, "shell", "dumpsys", "package", package])?;
        if dump.contains("Unable to find package:") {
            return Err(BifrostError::Package(format!(
                "package '{package}' is not installed on '{target}'"
            )));
        }

        let mut details = parse_dumpsys_package(package, &dump);
        if let Ok(resolved) = self.adb.run_adb(&[
            "-s",
            &target,
            "shell",
            "cmd",
            "package",
            "resolve-activity",
            "--brief",
            package,
        ]) {
            details.launcher_component = parse_resolved_activity(&resolved);
        }
        Ok(details)
    }

    /// Inspects and installs an APK file using Android's native `PackageManager`.
    ///
    /// Enforces path traversal protection, ZIP/`AndroidManifest.xml` validation,
    /// and ADR-0004 `x86_64` ABI verification before invoking `adb install -r`.
    pub fn install_apk(
        &self,
        serial: Option<&str>,
        apk_path: impl AsRef<Path>,
    ) -> Result<InstallOutcome> {
        let inspection = inspect_apk(apk_path)?;
        if !inspection.abi_compatibility.is_supported() {
            return Err(BifrostError::UnsupportedAbi {
                path: inspection.path,
                detected_abis: inspection.detected_abis.join(", "),
            });
        }

        let target = self.resolve_target_serial(serial)?;
        let apk_str = inspection.path.to_string_lossy().to_string();
        info!(
            serial = %target,
            apk = %inspection.path.display(),
            sha256 = %inspection.sha256,
            "installing verified APK via Android PackageManager"
        );

        let pm_output = self
            .adb
            .run_adb(&["-s", &target, "install", "-r", &apk_str])?;
        if !pm_output.contains("Success") {
            return Err(BifrostError::Package(format!(
                "Android PackageManager rejected APK installation: {}",
                pm_output.trim()
            )));
        }

        Ok(InstallOutcome {
            serial: target,
            inspection,
            pm_output: pm_output.trim().to_string(),
        })
    }

    /// Uninstalls an Android package via `adb uninstall`.
    pub fn uninstall_package(
        &self,
        serial: Option<&str>,
        package: &str,
        keep_data: bool,
    ) -> Result<String> {
        validate_package_name(package)?;
        let target = self.resolve_target_serial(serial)?;
        let out = if keep_data {
            self.adb.run_adb(&[
                "-s",
                &target,
                "shell",
                "cmd",
                "package",
                "uninstall",
                "-k",
                package,
            ])?
        } else {
            self.adb.run_adb(&["-s", &target, "uninstall", package])?
        };
        if !out.contains("Success") {
            return Err(BifrostError::Package(format!(
                "failed to uninstall '{package}': {}",
                out.trim()
            )));
        }
        Ok(out.trim().to_string())
    }

    /// Launches an installed Android application's primary launcher activity.
    pub fn launch_app(&self, serial: Option<&str>, package: &str) -> Result<LaunchOutcome> {
        validate_package_name(package)?;
        let target = self.resolve_target_serial(serial)?;
        let resolved_out = self
            .adb
            .run_adb(&[
                "-s",
                &target,
                "shell",
                "cmd",
                "package",
                "resolve-activity",
                "--brief",
                package,
            ])
            .ok();

        if let Some(component) = resolved_out.as_deref().and_then(parse_resolved_activity) {
            let am_output = self
                .adb
                .run_adb(&["-s", &target, "shell", "am", "start", "-n", &component])?;
            return Ok(LaunchOutcome {
                serial: target,
                package_name: package.to_string(),
                component: Some(component),
                am_output: am_output.trim().to_string(),
            });
        }

        let monkey_out = self.adb.run_adb(&[
            "-s",
            &target,
            "shell",
            "monkey",
            "-p",
            package,
            "-c",
            "android.intent.category.LAUNCHER",
            "1",
        ])?;
        Ok(LaunchOutcome {
            serial: target,
            package_name: package.to_string(),
            component: None,
            am_output: monkey_out.trim().to_string(),
        })
    }

    /// Force-stops a running Android application via `am force-stop`.
    pub fn force_stop(&self, serial: Option<&str>, package: &str) -> Result<()> {
        validate_package_name(package)?;
        let target = self.resolve_target_serial(serial)?;
        let _ = self
            .adb
            .run_adb(&["-s", &target, "shell", "am", "force-stop", package])?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::fs::File;
    use std::io::Write;

    use tempfile::TempDir;
    use zip::write::SimpleFileOptions;
    use zip::ZipWriter;

    use super::*;

    #[test]
    fn install_rejects_arm64_only_apk_before_calling_adb() {
        let tmp = TempDir::new().unwrap();
        let arm_apk = tmp.path().join("arm64-only.apk");
        let file = File::create(&arm_apk).unwrap();
        let mut writer = ZipWriter::new(file);
        let opts = SimpleFileOptions::default();
        writer.start_file("AndroidManifest.xml", opts).unwrap();
        writer.write_all(b"xml").unwrap();
        writer.start_file("lib/arm64-v8a/libfoo.so", opts).unwrap();
        writer.write_all(b"elf").unwrap();
        writer.finish().unwrap();

        let mgr = ValhallaManager::new();
        let err = mgr
            .install_apk(Some("127.0.0.1:5555"), &arm_apk)
            .unwrap_err();
        assert!(matches!(err, BifrostError::UnsupportedAbi { .. }));
    }
}
