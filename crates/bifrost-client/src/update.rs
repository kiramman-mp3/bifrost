//! Update notice: compares this build with the latest published GitHub release.
//!
//! The check only reads public release metadata. It sends no user or host data, and it
//! downloads and installs nothing: updating stays with the release bundle and
//! `install-bifrost.ps1`, which verifies every file's SHA-256 before installing.

use std::path::PathBuf;
use std::process::Command;

use bifrost_core::{BifrostError, Result};
use serde::{Deserialize, Serialize};

/// Public GitHub API endpoint for the latest (non-draft, non-prerelease) release.
pub const LATEST_RELEASE_API: &str =
    "https://api.github.com/repos/kiramman-mp3/bifrost/releases/latest";

/// Result of an update check.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UpdateInfo {
    pub current_version: String,
    pub latest_version: String,
    pub update_available: bool,
    pub release_name: Option<String>,
    pub release_url: String,
    pub published_at: Option<String>,
    /// The Windows release bundle, if the release has one.
    pub bundle_asset: Option<String>,
}

#[derive(Debug, Deserialize)]
struct GithubRelease {
    tag_name: String,
    name: Option<String>,
    html_url: String,
    published_at: Option<String>,
    #[serde(default)]
    assets: Vec<GithubAsset>,
}

#[derive(Debug, Deserialize)]
struct GithubAsset {
    name: String,
}

/// Parses `v1.2.3` / `1.2.3` (a pre-release or build suffix is ignored).
#[must_use]
pub fn parse_version(raw: &str) -> Option<(u64, u64, u64)> {
    let core = raw
        .trim()
        .trim_start_matches(['v', 'V'])
        .split(['-', '+'])
        .next()?;
    let mut parts = core.split('.').map(|p| p.parse::<u64>().ok());
    let version = (parts.next()??, parts.next()??, parts.next()??);
    parts.next().is_none().then_some(version)
}

/// Builds the update notice from the release JSON returned by the GitHub API.
pub fn evaluate_release(current_version: &str, release_json: &str) -> Result<UpdateInfo> {
    let release: GithubRelease = serde_json::from_str(release_json)
        .map_err(|err| BifrostError::Runtime(format!("unexpected release metadata: {err}")))?;
    let latest = parse_version(&release.tag_name).ok_or_else(|| {
        BifrostError::Runtime(format!(
            "latest release tag '{}' is not a version",
            release.tag_name
        ))
    })?;
    let current = parse_version(current_version)
        .ok_or_else(|| BifrostError::Runtime(format!("'{current_version}' is not a version")))?;
    Ok(UpdateInfo {
        current_version: current_version.to_string(),
        latest_version: release.tag_name.trim_start_matches(['v', 'V']).to_string(),
        update_available: latest > current,
        release_name: release.name,
        release_url: release.html_url,
        published_at: release.published_at,
        bundle_asset: release
            .assets
            .into_iter()
            .map(|a| a.name)
            .find(|n| n.ends_with("-win64.zip")),
    })
}

/// Windows' own `curl.exe`, by absolute path so a `curl` earlier on `PATH` is not used.
fn system_curl() -> Result<PathBuf> {
    let root = std::env::var_os("SystemRoot").unwrap_or_else(|| r"C:\Windows".into());
    let curl = PathBuf::from(root).join("System32").join("curl.exe");
    if curl.is_file() {
        Ok(curl)
    } else {
        Err(BifrostError::Runtime(
            "Windows curl.exe was not found; check for updates on the GitHub releases page"
                .to_string(),
        ))
    }
}

/// Fetches the latest release metadata (HTTPS only, 15 s limit) and compares versions.
pub fn check_for_update(current_version: &str) -> Result<UpdateInfo> {
    let output = Command::new(system_curl()?)
        .args([
            "--fail",
            "--silent",
            "--show-error",
            "--location",
            "--proto",
            "=https",
            "--max-time",
            "15",
            "--user-agent",
            "bifrost-update-check",
            "--header",
            "Accept: application/vnd.github+json",
            LATEST_RELEASE_API,
        ])
        .output()
        .map_err(|err| BifrostError::Runtime(format!("could not run curl.exe: {err}")))?;
    if !output.status.success() {
        return Err(BifrostError::Runtime(format!(
            "update check failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    evaluate_release(current_version, &String::from_utf8_lossy(&output.stdout))
}

#[cfg(test)]
mod tests {
    use super::*;

    const RELEASE: &str = r#"{
        "tag_name": "v0.5.0",
        "name": "BIFRÖST v0.5.0",
        "html_url": "https://github.com/kiramman-mp3/bifrost/releases/tag/v0.5.0",
        "published_at": "2026-12-01T00:00:00Z",
        "assets": [{"name": "SHA256SUMS.txt"}, {"name": "bifrost-v0.5.0-win64.zip"}]
    }"#;

    #[test]
    fn parses_versions_and_rejects_garbage() {
        assert_eq!(parse_version("v0.4.0"), Some((0, 4, 0)));
        assert_eq!(parse_version("1.10.2-rc.1"), Some((1, 10, 2)));
        assert_eq!(parse_version("1.2"), None);
        assert_eq!(parse_version("1.2.3.4"), None);
        assert_eq!(parse_version("latest"), None);
    }

    #[test]
    fn reports_a_newer_release_with_its_bundle() {
        let info = evaluate_release("0.4.0", RELEASE).unwrap();
        assert!(info.update_available);
        assert_eq!(info.latest_version, "0.5.0");
        assert_eq!(
            info.bundle_asset.as_deref(),
            Some("bifrost-v0.5.0-win64.zip")
        );
    }

    #[test]
    fn same_or_older_release_is_not_an_update() {
        assert!(!evaluate_release("0.5.0", RELEASE).unwrap().update_available);
        assert!(
            !evaluate_release("0.10.0", RELEASE)
                .unwrap()
                .update_available
        );
    }

    #[test]
    fn malformed_metadata_is_an_error() {
        assert!(evaluate_release("0.4.0", "{}").is_err());
        assert!(evaluate_release("0.4.0", r#"{"tag_name":"nightly","html_url":"x"}"#).is_err());
    }
}
