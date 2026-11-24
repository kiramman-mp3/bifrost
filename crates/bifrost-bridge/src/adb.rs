//! Loopback-safe ADB orchestration, device discovery, and Android guest health checks.

use bifrost_core::find_on_path;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::process::Command;

use bifrost_core::{BifrostError, Result};
use serde::{Deserialize, Serialize};
use tracing::debug;

/// Quotes `arg` as a single POSIX `sh` word for `adb shell`.
///
/// `adb shell` joins its arguments with spaces and hands the resulting string to the
/// Android `/system/bin/sh` interpreter, so any host-controlled value (clipboard text,
/// paths, labels) must be quoted to prevent shell metacharacter injection. The value is
/// wrapped in single quotes and every embedded `'` becomes `'\''`.
#[must_use]
pub fn quote_android_shell_arg(arg: &str) -> String {
    let mut quoted = String::with_capacity(arg.len() + 2);
    quoted.push('\'');
    for ch in arg.chars() {
        if ch == '\'' {
            quoted.push_str(r"'\''");
        } else {
            quoted.push(ch);
        }
    }
    quoted.push('\'');
    quoted
}

/// An Android device or emulator instance reported by `adb devices -l`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AdbDevice {
    pub serial: String,
    pub state: String,
    pub product: Option<String>,
    pub model: Option<String>,
    pub device: Option<String>,
    pub transport_id: Option<String>,
}

/// Online (`device`) entries with emulator aliases of loopback endpoints removed.
///
/// The ADB server auto-registers anything answering on 5555–5585 as `emulator-<N>`
/// (console port `N`, ADB port `N + 1`). Midgard on the default port 5555 therefore
/// shows up twice, as `127.0.0.1:5555` and `emulator-5554`, which made every command
/// without `--serial` fail with "multiple Android devices online". The explicit
/// loopback entry is kept because it is the endpoint BIFRÖST manages.
#[must_use]
pub fn online_devices_deduplicated(devices: Vec<AdbDevice>) -> Vec<AdbDevice> {
    let online: Vec<AdbDevice> = devices
        .into_iter()
        .filter(|d| d.state == "device")
        .collect();
    let loopback_ports: Vec<u32> = online
        .iter()
        .filter_map(|d| d.serial.strip_prefix("127.0.0.1:"))
        .filter_map(|port| port.parse().ok())
        .collect();
    online
        .into_iter()
        .filter(|d| {
            let alias_of_loopback = d
                .serial
                .strip_prefix("emulator-")
                .and_then(|console| console.parse::<u32>().ok())
                .is_some_and(|console| loopback_ports.contains(&(console + 1)));
            !alias_of_loopback
        })
        .collect()
}

/// Runtime health metadata probed from a booted Android guest over ADB.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GuestHealth {
    pub serial: String,
    pub boot_completed: bool,
    pub android_release: Option<String>,
    pub sdk_int: Option<String>,
    pub cpu_abi: Option<String>,
    pub model: Option<String>,
}

/// How far ADB access to Midgard's loopback endpoint gets, and what blocks it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AdbEndpointState {
    /// No usable transport yet: Android is still booting or adbd is not running.
    Offline,
    /// adbd requires this PC's key to be approved in Android's "Allow USB debugging?"
    /// dialog (provisioned `userdata`, `ro.adb.secure=1`).
    Unauthorized,
    /// Android 16+ trade-in mode: the setup wizard has not been completed, so adbd runs
    /// in the `adbd_tradeinmode` SELinux domain and closes every service except
    /// `tradeinmode`. Typical for a freshly provisioned `userdata` disk.
    TradeInMode,
    /// Full ADB access (`shell`, `sync`, `install`).
    Ready,
}

impl AdbEndpointState {
    /// Classifies an endpoint from its `adb devices` state and two shell probes.
    #[must_use]
    pub fn classify(device_state: Option<&str>, shell_ok: bool, tradeinmode_ok: bool) -> Self {
        match device_state {
            Some("unauthorized") => Self::Unauthorized,
            Some("device") if shell_ok => Self::Ready,
            Some("device") if tradeinmode_ok => Self::TradeInMode,
            _ => Self::Offline,
        }
    }

    /// What the user has to do when this state blocks ADB, if anything.
    #[must_use]
    pub const fn remediation(self) -> Option<&'static str> {
        match self {
            Self::Unauthorized => Some(
                "Android has not authorized this PC for ADB. Restart Midgard with a visible                  display (`bifrost runtime restart`, without `--headless`) and accept                  \"Allow USB debugging\" with \"Always allow from this computer\".",
            ),
            Self::TradeInMode => Some(
                "Android's initial setup has not been completed, so adbd runs in trade-in                  mode and refuses shell access. Restart Midgard with a visible display                  (`bifrost runtime restart`, without `--headless`) and finish the setup wizard.",
            ),
            Self::Offline | Self::Ready => None,
        }
    }
}

/// Safe wrapper around the host `adb` binary (never shells through `cmd`/`sh`).
#[derive(Debug, Clone, Default)]
pub struct AdbClient {
    explicit_binary: Option<PathBuf>,
}

impl AdbClient {
    /// Creates an ADB client using automatic discovery (`BIFROST_ADB_BIN`, `PATH`, Android SDK).
    #[must_use]
    pub const fn new() -> Self {
        Self {
            explicit_binary: None,
        }
    }

    /// Creates an ADB client pinned to an explicit `adb` executable path.
    #[must_use]
    pub fn with_binary(path: impl Into<PathBuf>) -> Self {
        Self {
            explicit_binary: Some(path.into()),
        }
    }

    /// Locates the `adb` binary on the host.
    pub fn locate_adb(&self) -> Result<PathBuf> {
        if let Some(ref explicit) = self.explicit_binary {
            if explicit.is_file() {
                return Ok(explicit.clone());
            }
            return Err(BifrostError::Bridge(format!(
                "configured ADB binary '{}' was not found",
                explicit.display()
            )));
        }

        if let Ok(env_bin) = std::env::var("BIFROST_ADB_BIN") {
            let candidate = PathBuf::from(env_bin);
            if candidate.is_file() {
                return Ok(candidate);
            }
        }

        if let Some(on_path) = find_on_path("adb.exe").or_else(|| find_on_path("adb")) {
            return Ok(on_path);
        }

        if let Ok(local_app_data) = std::env::var("LOCALAPPDATA") {
            let sdk_adb = PathBuf::from(&local_app_data)
                .join("Android")
                .join("Sdk")
                .join("platform-tools")
                .join("adb.exe");
            if sdk_adb.is_file() {
                return Ok(sdk_adb);
            }
        }

        if let Ok(android_home) = std::env::var("ANDROID_HOME") {
            let home_adb = PathBuf::from(android_home)
                .join("platform-tools")
                .join("adb.exe");
            if home_adb.is_file() {
                return Ok(home_adb);
            }
        }

        Err(BifrostError::Bridge(
            "adb binary not found. Install Android Platform Tools or set BIFROST_ADB_BIN."
                .to_string(),
        ))
    }

    /// Queries the installed `adb version` banner.
    pub fn version(&self) -> Result<String> {
        let stdout = self.run_adb(&["version"])?;
        Ok(stdout.lines().next().unwrap_or("").trim().to_string())
    }

    /// Lists all devices currently visible to the local ADB server (`adb devices -l`).
    pub fn list_devices(&self) -> Result<Vec<AdbDevice>> {
        let stdout = self.run_adb(&["devices", "-l"])?;
        Ok(parse_adb_devices_output(&stdout))
    }

    /// Connects ADB to a strictly validated loopback endpoint (`127.0.0.1:<port>` or `[::1]:<port>`).
    pub fn connect_loopback(&self, endpoint: &str) -> Result<String> {
        validate_loopback_endpoint(endpoint)?;
        self.run_adb(&["connect", endpoint])
    }

    /// Makes sure the ADB server is attached to Midgard's loopback `endpoint`.
    ///
    /// The ADB server only auto-discovers emulators on ports 5555–5585, so a VM on any
    /// other configured `adb_port` is invisible until it is explicitly connected.
    /// Returns whether the endpoint is online (`device` state) afterwards.
    pub fn ensure_connected(&self, endpoint: &str) -> Result<bool> {
        validate_loopback_endpoint(endpoint)?;
        let online = |devices: &[AdbDevice]| {
            devices
                .iter()
                .any(|d| d.serial == endpoint && d.state == "device")
        };
        let devices = self.list_devices()?;
        if online(&devices) {
            return Ok(true);
        }
        // A transport opened before adbd existed can linger as `offline` while
        // `adb connect` keeps answering "already connected"; drop it and reconnect.
        if devices
            .iter()
            .any(|d| d.serial == endpoint && d.state == "offline")
        {
            let _ = self.disconnect(endpoint);
        }
        // `adb connect` reports refusals on stdout with exit code 0; the re-list decides.
        let _ = self.connect_loopback(endpoint);
        Ok(online(&self.list_devices()?))
    }

    /// Attaches `endpoint` and reports how far ADB access gets (see [`AdbEndpointState`]).
    pub fn diagnose_endpoint(&self, endpoint: &str) -> Result<AdbEndpointState> {
        let _ = self.ensure_connected(endpoint)?;
        let devices = self.list_devices()?;
        let device_state = devices
            .iter()
            .find(|d| d.serial == endpoint)
            .map(|d| d.state.as_str());
        let probe = |args: &[&str]| {
            let mut full = vec!["-s", endpoint, "shell"];
            full.extend_from_slice(args);
            self.run_adb(&full)
        };
        let shell_ok = device_state == Some("device")
            && probe(&["echo", "bifrost-ok"]).is_ok_and(|out| out.trim() == "bifrost-ok");
        let tradeinmode_ok = device_state == Some("device")
            && !shell_ok
            && probe(&["tradeinmode", "getstatus"]).is_ok();
        Ok(AdbEndpointState::classify(
            device_state,
            shell_ok,
            tradeinmode_ok,
        ))
    }

    /// Lists online devices, collapsing emulator aliases of loopback endpoints
    /// (see [`online_devices_deduplicated`]).
    pub fn online_devices(&self) -> Result<Vec<AdbDevice>> {
        Ok(online_devices_deduplicated(self.list_devices()?))
    }

    /// Disconnects ADB from an endpoint.
    pub fn disconnect(&self, endpoint: &str) -> Result<String> {
        validate_loopback_endpoint(endpoint)?;
        self.run_adb(&["disconnect", endpoint])
    }

    /// Executes an Android `getprop` query on `serial` after strict property-name validation.
    pub fn getprop(&self, serial: &str, property: &str) -> Result<String> {
        validate_serial(serial)?;
        validate_android_prop(property)?;
        let out = self.run_adb(&["-s", serial, "shell", "getprop", property])?;
        Ok(out.trim().to_string())
    }

    /// Probes real Android boot completion and OS metadata over ADB.
    pub fn probe_guest_health(&self, serial: &str) -> Result<GuestHealth> {
        validate_serial(serial)?;
        let boot_completed = self.getprop(serial, "sys.boot_completed")? == "1";
        let android_release = non_empty(self.getprop(serial, "ro.build.version.release").ok());
        let sdk_int = non_empty(self.getprop(serial, "ro.build.version.sdk").ok());
        let cpu_abi = non_empty(self.getprop(serial, "ro.product.cpu.abi").ok());
        let model = non_empty(self.getprop(serial, "ro.product.model").ok());

        Ok(GuestHealth {
            serial: serial.to_string(),
            boot_completed,
            android_release,
            sdk_int,
            cpu_abi,
            model,
        })
    }

    /// Runs an ADB command directly with an explicit argument vector (never via a shell).
    pub fn run_adb(&self, args: &[&str]) -> Result<String> {
        let adb_bin = self.locate_adb()?;
        debug!(adb = %adb_bin.display(), args = ?args, "invoking adb");

        let output = Command::new(&adb_bin)
            .args(args)
            .output()
            .map_err(|source| BifrostError::Io {
                context: format!("executing '{}'", adb_bin.display()),
                source,
            })?;

        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        if !output.status.success() {
            return Err(BifrostError::Bridge(format!(
                "adb {:?} failed (exit {:?}): {}",
                args,
                output.status.code(),
                stderr.trim()
            )));
        }

        Ok(stdout)
    }

    /// Runs an ADB command and streams `input` to its standard input.
    ///
    /// Used to hand arbitrary data (e.g. clipboard text) to a fixed `adb shell` script,
    /// so the data never becomes part of a command line: it cannot be interpreted by the
    /// Android shell and is not limited by the Windows command-line length (~32K chars).
    pub fn run_adb_with_stdin(&self, args: &[&str], input: &[u8]) -> Result<String> {
        use std::io::Write;
        use std::process::Stdio;

        let adb_bin = self.locate_adb()?;
        debug!(adb = %adb_bin.display(), args = ?args, stdin_bytes = input.len(), "invoking adb with stdin");

        let mut child = Command::new(&adb_bin)
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|source| BifrostError::Io {
                context: format!("executing '{}'", adb_bin.display()),
                source,
            })?;

        // Feed stdin from a separate thread so a full stdout/stderr pipe cannot deadlock.
        let mut stdin = child.stdin.take().ok_or_else(|| {
            BifrostError::Bridge("adb child process has no stdin pipe".to_string())
        })?;
        let payload = input.to_vec();
        let feeder = std::thread::spawn(move || {
            let result = stdin.write_all(&payload);
            drop(stdin); // EOF tells the guest-side `cat` that the input is complete.
            result
        });

        let output = child
            .wait_with_output()
            .map_err(|source| BifrostError::Io {
                context: format!("waiting for '{}'", adb_bin.display()),
                source,
            })?;
        let fed = feeder
            .join()
            .map_err(|_| BifrostError::Bridge("adb stdin feeder thread panicked".to_string()))?;
        fed.map_err(|source| BifrostError::Io {
            context: "writing adb stdin".to_string(),
            source,
        })?;

        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr).to_string();
            return Err(BifrostError::Bridge(format!(
                "adb {:?} failed (exit {:?}): {}",
                args,
                output.status.code(),
                stderr.trim()
            )));
        }
        Ok(stdout)
    }
}

/// Parses stdout from `adb devices -l` into structured [`AdbDevice`] records.
#[must_use]
pub fn parse_adb_devices_output(output: &str) -> Vec<AdbDevice> {
    let mut devices = Vec::new();
    for line in output.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty()
            || trimmed.starts_with("List of devices attached")
            || trimmed.starts_with('*')
        {
            continue;
        }
        let mut parts = trimmed.split_whitespace();
        let Some(serial) = parts.next() else {
            continue;
        };
        let Some(state) = parts.next() else {
            continue;
        };

        let mut product = None;
        let mut model = None;
        let mut device = None;
        let mut transport_id = None;

        for token in parts {
            if let Some((k, v)) = token.split_once(':') {
                match k {
                    "product" => product = Some(v.to_string()),
                    "model" => model = Some(v.to_string()),
                    "device" => device = Some(v.to_string()),
                    "transport_id" => transport_id = Some(v.to_string()),
                    _ => {}
                }
            }
        }

        devices.push(AdbDevice {
            serial: serial.to_string(),
            state: state.to_string(),
            product,
            model,
            device,
            transport_id,
        });
    }
    devices
}

/// Ensures an ADB network endpoint strictly targets loopback (`127.0.0.1` or `::1`).
pub fn validate_loopback_endpoint(endpoint: &str) -> Result<()> {
    let addr: SocketAddr = endpoint.parse().map_err(|_| {
        BifrostError::Bridge(format!(
            "invalid ADB endpoint '{endpoint}' (expected host:port)"
        ))
    })?;
    if !addr.ip().is_loopback() {
        return Err(BifrostError::Bridge(format!(
            "refusing non-loopback ADB endpoint '{endpoint}' for security"
        )));
    }
    Ok(())
}

fn validate_serial(serial: &str) -> Result<()> {
    if serial.is_empty()
        || !serial
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | ':' | '-' | '_' | '[' | ']'))
    {
        return Err(BifrostError::Bridge(format!(
            "invalid ADB device serial '{serial}'"
        )));
    }
    Ok(())
}

fn validate_android_prop(prop: &str) -> Result<()> {
    if prop.is_empty()
        || !prop
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
    {
        return Err(BifrostError::Bridge(format!(
            "invalid Android property name '{prop}'"
        )));
    }
    Ok(())
}

fn non_empty(value: Option<String>) -> Option<String> {
    value.and_then(|v| {
        let t = v.trim();
        if t.is_empty() {
            None
        } else {
            Some(t.to_string())
        }
    })
}

#[cfg(test)]
mod dedupe_tests {
    use super::{online_devices_deduplicated, parse_adb_devices_output};

    #[test]
    fn collapses_emulator_alias_of_midgard_loopback_endpoint() {
        // `adb devices -l` captured from a live Midgard VM on the default port 5555.
        let raw = "List of devices attached
127.0.0.1:5555         device product:lineage_virtio_x86_64 model:Standard_PC__Q35___ICH9__2009_ device:virtio_x86_64 transport_id:2
emulator-5554          device product:lineage_virtio_x86_64 model:Standard_PC__Q35___ICH9__2009_ device:virtio_x86_64 transport_id:107
";
        let online = online_devices_deduplicated(parse_adb_devices_output(raw));
        let serials: Vec<_> = online.iter().map(|d| d.serial.as_str()).collect();
        assert_eq!(serials, ["127.0.0.1:5555"]);
    }

    #[test]
    fn keeps_real_emulators_and_drops_offline_entries() {
        let raw = "List of devices attached
127.0.0.1:5597         device transport_id:3
emulator-5554          device transport_id:1
127.0.0.1:5641         offline transport_id:4
";
        let online = online_devices_deduplicated(parse_adb_devices_output(raw));
        let serials: Vec<_> = online.iter().map(|d| d.serial.as_str()).collect();
        assert_eq!(serials, ["127.0.0.1:5597", "emulator-5554"]);
    }
}

#[cfg(test)]
mod endpoint_state_tests {
    use super::AdbEndpointState as S;

    #[test]
    fn classifies_real_adb_observations() {
        // Values observed against a live Midgard VM (LineageOS 24, Android 17).
        assert_eq!(S::classify(None, false, false), S::Offline);
        assert_eq!(S::classify(Some("offline"), false, false), S::Offline);
        assert_eq!(
            S::classify(Some("unauthorized"), false, false),
            S::Unauthorized
        );
        // Fresh userdata: transport is `device` but every service except tradeinmode closes.
        assert_eq!(S::classify(Some("device"), false, true), S::TradeInMode);
        assert_eq!(S::classify(Some("device"), true, false), S::Ready);
        assert_eq!(S::classify(Some("device"), false, false), S::Offline);
    }

    #[test]
    fn only_blocking_states_carry_remediation() {
        assert!(S::Unauthorized
            .remediation()
            .unwrap()
            .contains("Allow USB debugging"));
        assert!(S::TradeInMode
            .remediation()
            .unwrap()
            .contains("setup wizard"));
        assert!(S::Offline.remediation().is_none());
        assert!(S::Ready.remediation().is_none());
    }

    #[test]
    fn serializes_as_snake_case() {
        assert_eq!(
            serde_json::to_string(&S::TradeInMode).unwrap(),
            "\"trade_in_mode\""
        );
    }
}

#[cfg(test)]
mod stdin_tests {
    use super::AdbClient;

    /// Uses `findstr` as a stand-in for `adb`: it echoes stdin, proving that large,
    /// metacharacter-heavy payloads travel through stdin instead of the command line.
    #[cfg(windows)]
    #[test]
    fn streams_large_payload_through_stdin() {
        let client = AdbClient::with_binary(r"C:\Windows\System32\findstr.exe");
        let line = "x; rm -rf /sdcard/* $(id) `id` | cat 'quoted' \"dq\"\n";
        let payload = line.repeat(2000); // ~100 KB, far above the ~32K command-line cap
        let out = client
            .run_adb_with_stdin(&["/R", "^"], payload.as_bytes())
            .unwrap();
        assert_eq!(out.replace("\r\n", "\n"), payload);
    }
}

#[cfg(test)]
mod shell_quote_tests {
    use super::quote_android_shell_arg;

    #[test]
    fn quotes_shell_metacharacters_as_single_word() {
        assert_eq!(quote_android_shell_arg("hello"), "'hello'");
        assert_eq!(quote_android_shell_arg(""), "''");
        assert_eq!(
            quote_android_shell_arg("x; rm -rf /sdcard/* $(id) `id` | cat"),
            "'x; rm -rf /sdcard/* $(id) `id` | cat'"
        );
        assert_eq!(quote_android_shell_arg("it's"), r"'it'\''s'");
        assert_eq!(quote_android_shell_arg("a\nb"), "'a\nb'");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_adb_devices_long_output() {
        let sample = "\
* daemon not running; starting now at tcp:5037
* daemon started successfully
List of devices attached
127.0.0.1:5555         device product:midgard_x86_64 model:Midgard_x86_64 device:virtio_x86_64 transport_id:1
emulator-5554          offline transport_id:2
";
        let devices = parse_adb_devices_output(sample);
        assert_eq!(devices.len(), 2);
        assert_eq!(devices[0].serial, "127.0.0.1:5555");
        assert_eq!(devices[0].state, "device");
        assert_eq!(devices[0].model.as_deref(), Some("Midgard_x86_64"));
        assert_eq!(devices[0].transport_id.as_deref(), Some("1"));
        assert_eq!(devices[1].serial, "emulator-5554");
        assert_eq!(devices[1].state, "offline");
    }

    #[test]
    fn rejects_non_loopback_adb_endpoints_and_injected_props() {
        assert!(validate_loopback_endpoint("127.0.0.1:5555").is_ok());
        assert!(validate_loopback_endpoint("[::1]:5555").is_ok());
        assert!(validate_loopback_endpoint("0.0.0.0:5555").is_err());
        assert!(validate_loopback_endpoint("192.168.1.50:5555").is_err());

        assert!(validate_android_prop("sys.boot_completed").is_ok());
        assert!(validate_android_prop("sys.boot_completed; rm -rf /").is_err());
        assert!(validate_serial("127.0.0.1:5555").is_ok());
        assert!(validate_serial("127.0.0.1:5555 && whoami").is_err());
    }
}
