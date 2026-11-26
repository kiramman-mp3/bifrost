//! Android package identifier validation and `pm` / `dumpsys` output parsers.

use std::path::Path;

use bifrost_core::{BifrostError, Result};
use serde::{Deserialize, Serialize};

/// Summary of an installed Android package returned by `pm list packages -f`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InstalledPackage {
    pub package_name: String,
    pub apk_path: Option<String>,
    pub is_system: bool,
}

/// Detailed metadata parsed from Android `dumpsys package <pkg>`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PackageDetails {
    pub package_name: String,
    pub version_name: Option<String>,
    pub version_code: Option<String>,
    pub primary_cpu_abi: Option<String>,
    pub code_path: Option<String>,
    pub first_install_time: Option<String>,
    pub last_update_time: Option<String>,
    pub launcher_component: Option<String>,
}

/// Enforces strict Android application ID syntax (`com.example.app`) to prevent command injection.
pub fn validate_package_name(package: &str) -> Result<()> {
    let trimmed = package.trim();
    if trimmed.is_empty() || trimmed.len() > 255 {
        return Err(BifrostError::Package(format!(
            "invalid Android package name '{package}'"
        )));
    }
    let segments: Vec<&str> = trimmed.split('.').collect();
    if segments.len() < 2 {
        return Err(BifrostError::Package(format!(
            "Android package name '{package}' must contain at least two dot-separated segments"
        )));
    }
    for seg in segments {
        let mut chars = seg.chars();
        let Some(first) = chars.next() else {
            return Err(BifrostError::Package(format!(
                "invalid empty segment in package name '{package}'"
            )));
        };
        if !first.is_ascii_alphabetic() {
            return Err(BifrostError::Package(format!(
                "package segment '{seg}' in '{package}' must start with an ASCII letter"
            )));
        }
        if !chars.all(|c| c.is_ascii_alphanumeric() || c == '_') {
            return Err(BifrostError::Package(format!(
                "package segment '{seg}' in '{package}' contains illegal characters"
            )));
        }
    }
    Ok(())
}

/// Accepts an APK path reported by `pm list packages -f` only when it is an absolute
/// guest path to an `.apk` made of conservative characters, with no `..` component.
pub fn validate_guest_apk_path(path: &str) -> Result<()> {
    let ok = path.starts_with('/')
        && path.len() <= 512
        && Path::new(path).extension().and_then(|e| e.to_str()) == Some("apk")
        && !path.split('/').any(|part| part == "..")
        && path.chars().all(|c| {
            c.is_ascii_alphanumeric() || matches!(c, '/' | '.' | '_' | '-' | '~' | '=' | '+')
        });
    if ok {
        Ok(())
    } else {
        Err(BifrostError::Package(format!(
            "unexpected guest APK path '{path}'"
        )))
    }
}

/// Parses output from `pm list packages -f` into sorted [`InstalledPackage`] records.
#[must_use]
pub fn parse_pm_list_packages(output: &str) -> Vec<InstalledPackage> {
    let mut items = Vec::new();
    for line in output.lines() {
        let trimmed = line.trim();
        let Some(rest) = trimmed.strip_prefix("package:") else {
            continue;
        };
        if let Some((path, pkg)) = rest.rsplit_once('=') {
            let is_system = path.starts_with("/system")
                || path.starts_with("/product")
                || path.starts_with("/vendor")
                || path.starts_with("/apex")
                || path.starts_with("/system_ext");
            items.push(InstalledPackage {
                package_name: pkg.trim().to_string(),
                apk_path: Some(path.trim().to_string()),
                is_system,
            });
        } else if !rest.is_empty() {
            items.push(InstalledPackage {
                package_name: rest.to_string(),
                apk_path: None,
                is_system: false,
            });
        }
    }
    items.sort_by(|a, b| a.package_name.cmp(&b.package_name));
    items
}

/// Parses core package metadata fields from `dumpsys package <package_name>`.
#[must_use]
pub fn parse_dumpsys_package(package_name: &str, output: &str) -> PackageDetails {
    let mut details = PackageDetails {
        package_name: package_name.to_string(),
        version_name: None,
        version_code: None,
        primary_cpu_abi: None,
        code_path: None,
        first_install_time: None,
        last_update_time: None,
        launcher_component: None,
    };

    for line in output.lines() {
        let trimmed = line.trim();
        if details.version_name.is_none() {
            if let Some(val) = trimmed.strip_prefix("versionName=") {
                details.version_name = Some(val.trim().to_string());
            }
        }
        if details.version_code.is_none() {
            if let Some(val) = trimmed.strip_prefix("versionCode=") {
                let code = val.split_whitespace().next().unwrap_or("").trim();
                if !code.is_empty() {
                    details.version_code = Some(code.to_string());
                }
            }
        }
        if details.primary_cpu_abi.is_none() {
            if let Some(val) = trimmed.strip_prefix("primaryCpuAbi=") {
                let v = val.trim();
                if !v.is_empty() && v != "null" {
                    details.primary_cpu_abi = Some(v.to_string());
                }
            }
        }
        if details.code_path.is_none() {
            if let Some(val) = trimmed.strip_prefix("codePath=") {
                details.code_path = Some(val.trim().to_string());
            }
        }
        if details.first_install_time.is_none() {
            if let Some(val) = trimmed.strip_prefix("firstInstallTime=") {
                details.first_install_time = Some(val.trim().to_string());
            }
        }
        if details.last_update_time.is_none() {
            if let Some(val) = trimmed.strip_prefix("lastUpdateTime=") {
                details.last_update_time = Some(val.trim().to_string());
            }
        }
    }

    details
}

/// Extracts the `pkg/activity` component name from `cmd package resolve-activity --brief <pkg>`.
#[must_use]
pub fn parse_resolved_activity(output: &str) -> Option<String> {
    for line in output.lines().rev() {
        let trimmed = line.trim();
        if trimmed.contains('/') && !trimmed.contains(' ') && !trimmed.starts_with("priority=") {
            return Some(trimmed.to_string());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_only_plain_absolute_guest_apk_paths() {
        assert!(validate_guest_apk_path(
            "/data/app/~~Zk3_q-Ab==/org.fdroid.fdroid-Xy9+z==/base.apk"
        )
        .is_ok());
        assert!(validate_guest_apk_path("/system/app/BasicDreams/BasicDreams.apk").is_ok());
        for bad in [
            "data/app/base.apk",
            "/data/app/../../sdcard/x.apk",
            "/data/app/x.apk; reboot",
            "/data/app/$(id).apk",
            "/data/app/x apk.apk",
            "/data/app/base.odex",
        ] {
            assert!(validate_guest_apk_path(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn validates_android_package_identifiers() {
        assert!(validate_package_name("org.mozilla.firefox").is_ok());
        assert!(validate_package_name("com.example.my_app2").is_ok());
        assert!(validate_package_name("invalid").is_err());
        assert!(validate_package_name("com.example; rm -rf /").is_err());
        assert!(validate_package_name("com..bad").is_err());
    }

    #[test]
    fn parses_pm_list_and_dumpsys_output() {
        let pm = "\
package:/system/priv-app/Settings/Settings.apk=com.android.settings
package:/data/app/~~abc/org.fdroid.fdroid-1/base.apk=org.fdroid.fdroid
";
        let list = parse_pm_list_packages(pm);
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].package_name, "com.android.settings");
        assert!(list[0].is_system);
        assert_eq!(list[1].package_name, "org.fdroid.fdroid");
        assert!(!list[1].is_system);

        let dumpsys = "\
  Package [org.fdroid.fdroid] (1234):
    codePath=/data/app/~~abc/org.fdroid.fdroid-1
    versionCode=1020050 minSdk=23 targetSdk=34
    versionName=1.20.0
    primaryCpuAbi=x86_64
    firstInstallTime=2026-11-21 10:00:00
    lastUpdateTime=2026-11-21 10:00:00
";
        let details = parse_dumpsys_package("org.fdroid.fdroid", dumpsys);
        assert_eq!(details.version_name.as_deref(), Some("1.20.0"));
        assert_eq!(details.version_code.as_deref(), Some("1020050"));
        assert_eq!(details.primary_cpu_abi.as_deref(), Some("x86_64"));

        let resolved = "priority=0 preferredOrder=0 match=0x108000 specificIndex=-1 isDefault=true\norg.fdroid.fdroid/.views.main.MainActivity\n";
        assert_eq!(
            parse_resolved_activity(resolved).as_deref(),
            Some("org.fdroid.fdroid/.views.main.MainActivity")
        );
    }
}
