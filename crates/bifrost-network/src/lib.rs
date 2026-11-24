//! Loopback-safe Windows <-> Android TCP port forwarding and network coordination.

#![forbid(unsafe_code)]

use bifrost_bridge::AdbClient;
use bifrost_core::{BifrostError, Result};
use serde::{Deserialize, Serialize};

/// An active host-to-guest ADB TCP forwarding rule.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PortForwardRule {
    pub serial: String,
    pub local_spec: String,
    pub remote_spec: String,
}

/// Coordinates strictly unprivileged loopback port forwarding between Windows and Midgard.
#[derive(Debug, Clone, Default)]
pub struct NetworkManager {
    adb: AdbClient,
}

impl NetworkManager {
    /// Creates a network manager using default ADB discovery.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            adb: AdbClient::new(),
        }
    }

    /// Creates a network manager backed by an explicit [`AdbClient`].
    #[must_use]
    pub const fn with_adb(adb: AdbClient) -> Self {
        Self { adb }
    }

    /// Lists active ADB TCP port-forwarding rules (`adb forward --list`).
    pub fn list_forwards(&self, serial: Option<&str>) -> Result<Vec<PortForwardRule>> {
        let out = self.adb.run_adb(&["forward", "--list"])?;
        let mut rules = parse_adb_forward_list(&out);
        if let Some(filter_serial) = serial {
            rules.retain(|r| r.serial == filter_serial);
        }
        Ok(rules)
    }

    /// Establishes an unprivileged host loopback TCP forward (`127.0.0.1:<local_port>` -> guest `<guest_port>`).
    pub fn add_forward(
        &self,
        serial: &str,
        local_port: u16,
        guest_port: u16,
    ) -> Result<PortForwardRule> {
        validate_unprivileged_port("local_port", local_port)?;
        validate_unprivileged_port("guest_port", guest_port)?;
        let local_spec = format!("tcp:{local_port}");
        let remote_spec = format!("tcp:{guest_port}");
        let _ = self
            .adb
            .run_adb(&["-s", serial, "forward", &local_spec, &remote_spec])?;
        Ok(PortForwardRule {
            serial: serial.to_string(),
            local_spec,
            remote_spec,
        })
    }

    /// Removes a previously configured loopback port forward.
    pub fn remove_forward(&self, serial: &str, local_port: u16) -> Result<()> {
        validate_unprivileged_port("local_port", local_port)?;
        let local_spec = format!("tcp:{local_port}");
        let _ = self
            .adb
            .run_adb(&["-s", serial, "forward", "--remove", &local_spec])?;
        Ok(())
    }

    /// Resolves a single online device serial when not specified explicitly.
    pub fn resolve_serial(&self, explicit: Option<&str>) -> Result<String> {
        if let Some(s) = explicit {
            return Ok(s.to_string());
        }
        let online = self.adb.online_devices()?;
        match online.len() {
            0 => Err(BifrostError::Bridge(
                "no online Android device found for networking command".to_string(),
            )),
            1 => Ok(online[0].serial.clone()),
            _ => Err(BifrostError::Bridge(
                "multiple Android devices online; specify --serial <id>".to_string(),
            )),
        }
    }
}

/// Rejects privileged ports (`< 1024`) to enforce least-privilege networking.
pub fn validate_unprivileged_port(field: &str, port: u16) -> Result<()> {
    if port < 1024 {
        return Err(BifrostError::ConfigValidation {
            field: field.to_string(),
            reason: "port must be unprivileged (>= 1024)".to_string(),
        });
    }
    Ok(())
}

/// Parses stdout from `adb forward --list`.
#[must_use]
pub fn parse_adb_forward_list(output: &str) -> Vec<PortForwardRule> {
    let mut rules = Vec::new();
    for line in output.lines() {
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() >= 3 {
            rules.push(PortForwardRule {
                serial: parts[0].to_string(),
                local_spec: parts[1].to_string(),
                remote_spec: parts[2].to_string(),
            });
        }
    }
    rules
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_forward_list_and_rejects_privileged_ports() {
        let sample = "127.0.0.1:5555 tcp:8080 tcp:8080\nemulator-5554 tcp:9000 tcp:9001\n";
        let rules = parse_adb_forward_list(sample);
        assert_eq!(rules.len(), 2);
        assert_eq!(rules[0].local_spec, "tcp:8080");
        assert_eq!(rules[1].remote_spec, "tcp:9001");

        assert!(validate_unprivileged_port("local_port", 80).is_err());
        assert!(validate_unprivileged_port("local_port", 8080).is_ok());
    }
}
