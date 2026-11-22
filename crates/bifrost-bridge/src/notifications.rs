//! Android active notification parsing and XML-safe Windows toast payload rendering.

use std::process::Command;

use bifrost_core::{BifrostError, Result};
use serde::{Deserialize, Serialize};

use crate::adb::AdbClient;

/// Structured Android notification parsed from `dumpsys notification --noredact`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AndroidNotification {
    pub key: String,
    pub package_name: String,
    pub title: Option<String>,
    pub text: Option<String>,
}

/// Queries active Android notifications over Bridge/ADB.
pub fn list_active_notifications(
    adb: &AdbClient,
    serial: &str,
) -> Result<Vec<AndroidNotification>> {
    let raw = adb.run_adb(&[
        "-s",
        serial,
        "shell",
        "dumpsys",
        "notification",
        "--noredact",
    ])?;
    Ok(parse_dumpsys_notifications(&raw))
}

/// Parses `NotificationRecord` blocks from `dumpsys notification --noredact` output.
#[must_use]
pub fn parse_dumpsys_notifications(output: &str) -> Vec<AndroidNotification> {
    let mut results = Vec::new();
    let mut current: Option<AndroidNotification> = None;

    for line in output.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("NotificationRecord(") {
            if let Some(record) = current.take() {
                results.push(record);
            }
            let pkg = extract_token_value(trimmed, "pkg=").unwrap_or_else(|| "unknown".to_string());
            let key = extract_token_value(trimmed, "key=").unwrap_or_else(|| pkg.clone());
            current = Some(AndroidNotification {
                key,
                package_name: pkg,
                title: None,
                text: None,
            });
            continue;
        }

        if let Some(ref mut notif) = current {
            if notif.title.is_none() && trimmed.starts_with("android.title=") {
                notif.title = extract_string_extra(trimmed);
            } else if notif.text.is_none() && trimmed.starts_with("android.text=") {
                notif.text = extract_string_extra(trimmed);
            }
        }
    }

    if let Some(record) = current {
        results.push(record);
    }
    results
}

/// Renders an XML-escaped Windows 11 Toast notification payload for an Android notification.
#[must_use]
pub fn build_windows_toast_xml(notif: &AndroidNotification) -> String {
    let app = escape_xml(&notif.package_name);
    let title = escape_xml(notif.title.as_deref().unwrap_or(&notif.package_name));
    let body = escape_xml(notif.text.as_deref().unwrap_or(""));

    format!(
        "<toast><visual><binding template=\"ToastGeneric\"><text>{title}</text><text>{body}</text><text placement=\"attribution\">BIFRÖST · {app}</text></binding></visual></toast>"
    )
}

/// Dispatches an Android notification as a native Windows 11 Toast notification via WinRT.
///
/// Passes the XML-escaped toast payload strictly via an environment variable into a fixed
/// PowerShell script to eliminate any script injection risk.
#[cfg(windows)]
pub fn dispatch_windows_toast(notif: &AndroidNotification) -> Result<()> {
    const SCRIPT: &str = r#"
$ErrorActionPreference = 'Stop'
[Windows.UI.Notifications.ToastNotificationManager, Windows.UI.Notifications, ContentType = WindowsRuntime] | Out-Null
[Windows.Data.Xml.Dom.XmlDocument, Windows.Data.Xml.Dom.XmlDocument, ContentType = WindowsRuntime] | Out-Null
$xml = New-Object Windows.Data.Xml.Dom.XmlDocument
$xml.LoadXml($env:BIFROST_TOAST_XML)
$toast = [Windows.UI.Notifications.ToastNotification]::new($xml)
$appId = '{1AC14E77-02E7-4E5D-B744-2EB1AE5198B7}\WindowsPowerShell\v1.0\powershell.exe'
$notifier = [Windows.UI.Notifications.ToastNotificationManager]::CreateToastNotifier($appId)
$notifier.Show($toast)
"#;

    let xml = build_windows_toast_xml(notif);
    let output = Command::new("powershell.exe")
        .args(["-NoProfile", "-NonInteractive", "-Command", SCRIPT])
        .env("BIFROST_TOAST_XML", xml)
        .output()
        .map_err(|source| BifrostError::Io {
            context: "dispatching Windows 11 Toast notification".to_string(),
            source,
        })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(BifrostError::Bridge(format!(
            "failed showing Windows 11 Toast notification: {stderr}"
        )));
    }
    Ok(())
}

/// Non-Windows stub returning `Unsupported`.
#[cfg(not(windows))]
pub fn dispatch_windows_toast(_notif: &AndroidNotification) -> Result<()> {
    Err(BifrostError::Unsupported {
        feature: "Windows 11 Toast notifications".to_string(),
        milestone: "Windows 11 host only".to_string(),
    })
}

fn extract_token_value(line: &str, prefix: &str) -> Option<String> {
    let start = line.find(prefix)? + prefix.len();
    let rest = &line[start..];
    let end = rest
        .find(|c: char| c.is_whitespace() || c == ')' || c == '}')
        .unwrap_or(rest.len());
    let val = rest[..end].trim();
    if val.is_empty() {
        None
    } else {
        Some(val.to_string())
    }
}

fn extract_string_extra(line: &str) -> Option<String> {
    if let (Some(open), Some(close)) = (line.find('('), line.rfind(')')) {
        if close > open {
            let inner = line[open + 1..close].trim();
            if !inner.is_empty() && inner != "null" {
                return Some(inner.to_string());
            }
        }
    }
    None
}

fn escape_xml(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for ch in input.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            other => out.push(other),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_dumpsys_notifications_and_escapes_toast_xml() {
        let sample = "\
  NotificationRecord(0x1234: pkg=org.signal.app user=UserHandle{0} id=1 key=0|org.signal.app|1|null|10123):
    extras={
      android.title=String (Alice <Admin>)
      android.text=String (Check \"BIFRÖST\" & logs)
    }
";
        let list = parse_dumpsys_notifications(sample);
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].package_name, "org.signal.app");
        assert_eq!(list[0].title.as_deref(), Some("Alice <Admin>"));
        assert_eq!(list[0].text.as_deref(), Some("Check \"BIFRÖST\" & logs"));

        let xml = build_windows_toast_xml(&list[0]);
        assert!(xml.contains("Alice &lt;Admin&gt;"));
        assert!(xml.contains("Check &quot;BIFRÖST&quot; &amp; logs"));
        assert!(!xml.contains("<Admin>"));
    }
}
