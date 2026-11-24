//! Automatic Windows Desktop & Start Menu shortcut synchronization for apps
//! installed or removed directly inside Android (e.g., via F-Droid).

use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

use bifrost_bridge::AdbClient;
use tracing::{debug, info, warn};
use valhalla::{parse_resolved_activity, validate_package_name, ShortcutManager, ValhallaManager};

const SYNC_EVERY_N_TICKS: u64 = 6;

/// Stateful background synchronizer embedded inside Heimdall's health loop.
#[derive(Debug, Default)]
pub struct AppShortcutSyncer {
    ticks: u64,
    skipped_packages: BTreeSet<String>,
}

impl AppShortcutSyncer {
    /// Periodically synchronizes third-party Android packages with Windows shortcuts.
    pub fn maybe_sync(&mut self, adb: &AdbClient, serial: &str, data_dir: &Path) {
        self.ticks = self.ticks.saturating_add(1);
        if self.ticks > 1 && !self.ticks.is_multiple_of(SYNC_EVERY_N_TICKS) {
            return;
        }

        let valhalla = ValhallaManager::with_adb(adb.clone());
        let installed = match valhalla.list_packages(Some(serial), true) {
            Ok(pkgs) => pkgs,
            Err(err) => {
                debug!(error = %err, "skipping app shortcut sync; failed to query pm list packages");
                return;
            }
        };

        let shortcut_mgr = ShortcutManager::new(data_dir);
        let existing = shortcut_mgr.list_shortcuts().unwrap_or_default();

        let installed_names: Vec<String> = installed.into_iter().map(|p| p.package_name).collect();
        let existing_names: Vec<String> = existing.into_iter().map(|s| s.package_name).collect();
        let (to_add, to_remove) = compute_shortcut_diff(&installed_names, &existing_names);

        for pkg in to_remove {
            self.skipped_packages.remove(&pkg);
            match shortcut_mgr.remove_for_app(&pkg) {
                Ok(Some(_)) => info!(
                    package = %pkg,
                    "removed stale Windows shortcuts for app uninstalled inside Android"
                ),
                Ok(None) => {}
                Err(err) => warn!(package = %pkg, error = %err, "failed removing stale shortcuts"),
            }
        }

        for pkg in to_add {
            if self.skipped_packages.contains(&pkg) || validate_package_name(&pkg).is_err() {
                continue;
            }

            // Only pin packages that expose a launcher activity.
            let has_launcher = adb
                .run_adb(&[
                    "-s",
                    serial,
                    "shell",
                    "cmd",
                    "package",
                    "resolve-activity",
                    "--brief",
                    &pkg,
                ])
                .ok()
                .and_then(|out| parse_resolved_activity(&out))
                .is_some();

            if !has_launcher {
                self.skipped_packages.insert(pkg);
                continue;
            }

            let temp_apk = pull_base_apk_for_icon(adb, serial, &pkg, data_dir);
            match shortcut_mgr.create_for_app(&pkg, None, temp_apk.as_deref()) {
                Ok(rec) => info!(
                    package = %pkg,
                    display_name = %rec.display_name,
                    "auto-created Windows Desktop & Start Menu shortcuts for in-Android app"
                ),
                Err(err) => {
                    warn!(package = %pkg, error = %err, "failed auto-creating Windows shortcuts");
                    self.skipped_packages.insert(pkg);
                }
            }

            if let Some(tmp) = temp_apk {
                let _ = fs::remove_file(tmp);
            }
        }
    }
}

/// Computes `(packages_to_add, packages_to_remove)` between installed Android third-party
/// packages and currently recorded Windows shortcuts.
#[must_use]
pub fn compute_shortcut_diff(
    installed: &[String],
    existing_shortcuts: &[String],
) -> (Vec<String>, Vec<String>) {
    let installed_set: BTreeSet<&str> = installed.iter().map(String::as_str).collect();
    let existing_set: BTreeSet<&str> = existing_shortcuts.iter().map(String::as_str).collect();

    let to_add = installed_set
        .difference(&existing_set)
        .map(|s| (*s).to_string())
        .collect();
    let to_remove = existing_set
        .difference(&installed_set)
        .map(|s| (*s).to_string())
        .collect();
    (to_add, to_remove)
}

/// Extracts and validates the primary `/data/app/.../base.apk` path from `pm path <pkg>`.
#[must_use]
pub fn parse_pm_base_apk_path(pm_path_output: &str) -> Option<String> {
    let mut fallback = None;
    for line in pm_path_output.lines() {
        let Some(raw) = line.trim().strip_prefix("package:") else {
            continue;
        };
        let path = raw.trim();
        if !path.starts_with("/data/app/")
            || !path.ends_with(".apk")
            || path.contains("..")
            || path.chars().any(char::is_whitespace)
        {
            continue;
        }
        if path.ends_with("/base.apk") {
            return Some(path.to_string());
        }
        if fallback.is_none() {
            fallback = Some(path.to_string());
        }
    }
    fallback
}

fn pull_base_apk_for_icon(
    adb: &AdbClient,
    serial: &str,
    package: &str,
    data_dir: &Path,
) -> Option<std::path::PathBuf> {
    let pm_out = adb
        .run_adb(&["-s", serial, "shell", "pm", "path", package])
        .ok()?;
    let remote_apk = parse_pm_base_apk_path(&pm_out)?;

    let cache_dir = data_dir.join("cache");
    fs::create_dir_all(&cache_dir).ok()?;
    let local_tmp = cache_dir.join(format!("sync-{package}.apk"));
    let local_str = local_tmp.to_string_lossy().to_string();

    if adb
        .run_adb(&["-s", serial, "pull", &remote_apk, &local_str])
        .is_ok()
        && local_tmp.is_file()
    {
        Some(local_tmp)
    } else {
        let _ = fs::remove_file(&local_tmp);
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn computes_added_and_removed_shortcuts_deterministically() {
        let installed = vec![
            "org.fdroid.fdroid".to_string(),
            "org.schabi.newpipe".to_string(),
        ];
        let existing = vec![
            "org.fdroid.fdroid".to_string(),
            "com.removed.app".to_string(),
        ];

        let (add, remove) = compute_shortcut_diff(&installed, &existing);
        assert_eq!(add, vec!["org.schabi.newpipe"]);
        assert_eq!(remove, vec!["com.removed.app"]);
    }

    #[test]
    fn parses_and_validates_pm_base_apk_path() {
        let sample = "package:/data/app/~~abc==/org.fdroid.fdroid-xyz==/split_config.en.apk\npackage:/data/app/~~abc==/org.fdroid.fdroid-xyz==/base.apk\n";
        assert_eq!(
            parse_pm_base_apk_path(sample),
            Some("/data/app/~~abc==/org.fdroid.fdroid-xyz==/base.apk".to_string())
        );

        assert_eq!(
            parse_pm_base_apk_path("package:/data/app/../etc/passwd.apk"),
            None
        );
        assert_eq!(parse_pm_base_apk_path("package:/system/app/Evil.apk"), None);
    }
}
