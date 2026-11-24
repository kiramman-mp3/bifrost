//! Android process inspection and management over Bridge/ADB.

#![forbid(unsafe_code)]

use bifrost_bridge::AdbClient;
use bifrost_core::{BifrostError, Result};
use serde::{Deserialize, Serialize};

/// A running Android guest process reported by `ps -A -o PID,USER,RSS,NAME`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AndroidProcess {
    pub pid: u32,
    pub user: String,
    pub rss_kb: u64,
    pub name: String,
}

/// Queries and controls Android guest processes via Bridge.
#[derive(Debug, Clone, Default)]
pub struct ProcessManager {
    adb: AdbClient,
}

impl ProcessManager {
    /// Creates a process manager using default ADB discovery.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            adb: AdbClient::new(),
        }
    }

    /// Creates a process manager backed by an explicit [`AdbClient`].
    #[must_use]
    pub const fn with_adb(adb: AdbClient) -> Self {
        Self { adb }
    }

    /// Lists running Android processes sorted by resident memory (RSS) descending.
    pub fn list_processes(
        &self,
        serial: &str,
        filter: Option<&str>,
    ) -> Result<Vec<AndroidProcess>> {
        let out =
            self.adb
                .run_adb(&["-s", serial, "shell", "ps", "-A", "-o", "PID,USER,RSS,NAME"])?;
        let mut procs = parse_android_ps_output(&out);
        if let Some(needle) = filter {
            let lower = needle.to_ascii_lowercase();
            procs.retain(|p| p.name.to_ascii_lowercase().contains(&lower));
        }
        Ok(procs)
    }

    /// Resolves a single online Android device serial if `explicit` is omitted.
    pub fn resolve_serial(&self, explicit: Option<&str>) -> Result<String> {
        if let Some(s) = explicit {
            return Ok(s.to_string());
        }
        let online = self.adb.online_devices()?;
        match online.len() {
            0 => Err(BifrostError::Bridge(
                "no online Android device found for process inspection".to_string(),
            )),
            1 => Ok(online[0].serial.clone()),
            _ => Err(BifrostError::Bridge(
                "multiple Android devices online; specify --serial <id>".to_string(),
            )),
        }
    }
}

/// Parses Android toybox `ps -A -o PID,USER,RSS,NAME` output sorted by RSS descending.
#[must_use]
pub fn parse_android_ps_output(output: &str) -> Vec<AndroidProcess> {
    let mut processes = Vec::new();
    for line in output.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with("PID") {
            continue;
        }
        let parts: Vec<&str> = trimmed.split_whitespace().collect();
        if parts.len() < 4 {
            continue;
        }
        let Ok(pid) = parts[0].parse::<u32>() else {
            continue;
        };
        let user = parts[1].to_string();
        let rss_kb = parts[2].parse::<u64>().unwrap_or(0);
        let name = parts[3..].join(" ");
        processes.push(AndroidProcess {
            pid,
            user,
            rss_kb,
            name,
        });
    }
    processes.sort_by(|a, b| b.rss_kb.cmp(&a.rss_kb).then_with(|| a.pid.cmp(&b.pid)));
    processes
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_and_sorts_android_ps_output_by_rss() {
        let sample = "\
  PID USER            RSS NAME
    1 root           4096 init
  512 system       184320 system_server
 1204 u0_a102       96256 org.fdroid.fdroid
";
        let procs = parse_android_ps_output(sample);
        assert_eq!(procs.len(), 3);
        assert_eq!(procs[0].name, "system_server");
        assert_eq!(procs[0].rss_kb, 184_320);
        assert_eq!(procs[1].name, "org.fdroid.fdroid");
        assert_eq!(procs[2].pid, 1);
    }
}
