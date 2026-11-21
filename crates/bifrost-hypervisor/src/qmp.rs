//! Minimal pure-Rust QMP (QEMU Machine Protocol) client over loopback TCP.
//!
//! Used exclusively on `127.0.0.1` to request graceful ACPI `system_powerdown`
//! or clean VMM exit without shelling out to external tools.

use std::io::{BufRead, BufReader, Write};
use std::net::TcpStream;
use std::time::Duration;

use bifrost_core::{BifrostError, Result};
use serde_json::Value;
use tracing::debug;

const QMP_IO_TIMEOUT: Duration = Duration::from_secs(3);

/// Sends a single QMP command (`system_powerdown`, `quit`, `query-status`) to a loopback endpoint.
pub fn send_qmp_command(endpoint: &str, command: &str) -> Result<Value> {
    if !endpoint.starts_with("127.0.0.1:") && !endpoint.starts_with("[::1]:") {
        return Err(BifrostError::Hypervisor(format!(
            "refusing non-loopback QMP endpoint '{endpoint}'"
        )));
    }

    let stream = TcpStream::connect(endpoint).map_err(|source| BifrostError::Io {
        context: format!("connecting to QMP endpoint '{endpoint}'"),
        source,
    })?;
    let _ = stream.set_read_timeout(Some(QMP_IO_TIMEOUT));
    let _ = stream.set_write_timeout(Some(QMP_IO_TIMEOUT));

    let mut writer = stream.try_clone().map_err(|source| BifrostError::Io {
        context: "cloning QMP TCP stream".to_string(),
        source,
    })?;
    let mut reader = BufReader::new(stream);

    // 1. Read QMP greeting banner.
    let mut greeting = String::new();
    reader
        .read_line(&mut greeting)
        .map_err(|source| BifrostError::Io {
            context: "reading QMP greeting".to_string(),
            source,
        })?;
    debug!(greeting = greeting.trim(), "received QMP greeting");

    // 2. Negotiate capabilities.
    writer
        .write_all(b"{\"execute\":\"qmp_capabilities\"}\r\n")
        .map_err(|source| BifrostError::Io {
            context: "sending qmp_capabilities".to_string(),
            source,
        })?;
    writer.flush().map_err(|source| BifrostError::Io {
        context: "flushing qmp_capabilities".to_string(),
        source,
    })?;
    let _caps_reply = read_non_event_reply(&mut reader)?;

    // 3. Execute target command.
    let payload = format!("{{\"execute\":\"{command}\"}}\r\n");
    writer
        .write_all(payload.as_bytes())
        .map_err(|source| BifrostError::Io {
            context: format!("sending QMP command '{command}'"),
            source,
        })?;
    writer.flush().map_err(|source| BifrostError::Io {
        context: format!("flushing QMP command '{command}'"),
        source,
    })?;

    read_non_event_reply(&mut reader)
}

fn read_non_event_reply(reader: &mut BufReader<TcpStream>) -> Result<Value> {
    for _ in 0..8 {
        let mut line = String::new();
        let n = reader
            .read_line(&mut line)
            .map_err(|source| BifrostError::Io {
                context: "reading QMP response".to_string(),
                source,
            })?;
        if n == 0 {
            return Err(BifrostError::Hypervisor(
                "QMP connection closed unexpectedly".to_string(),
            ));
        }
        let parsed: Value = serde_json::from_str(line.trim())
            .map_err(|err| BifrostError::Hypervisor(format!("invalid QMP JSON response: {err}")))?;
        if parsed.get("event").is_some() {
            continue;
        }
        if let Some(err_obj) = parsed.get("error") {
            return Err(BifrostError::Hypervisor(format!(
                "QMP command returned error: {err_obj}"
            )));
        }
        return Ok(parsed);
    }

    Err(BifrostError::Hypervisor(
        "too many QMP async events before command response".to_string(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;
    use std::thread;

    #[test]
    fn rejects_non_loopback_qmp_endpoint() {
        let err = send_qmp_command("192.168.1.10:4444", "system_powerdown").unwrap_err();
        assert!(err.to_string().contains("non-loopback"));
    }

    #[test]
    fn performs_qmp_capabilities_handshake_and_powerdown() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();

        let server = thread::spawn(move || {
            let (mut sock, _) = listener.accept().unwrap();
            sock.write_all(b"{\"QMP\":{\"version\":{}}}\r\n").unwrap();
            let mut reader = BufReader::new(sock.try_clone().unwrap());
            let mut line = String::new();
            reader.read_line(&mut line).unwrap();
            assert!(line.contains("qmp_capabilities"));
            sock.write_all(b"{\"return\":{}}\r\n").unwrap();

            line.clear();
            reader.read_line(&mut line).unwrap();
            assert!(line.contains("system_powerdown"));
            sock.write_all(b"{\"event\":\"POWERDOWN\"}\r\n").unwrap();
            sock.write_all(b"{\"return\":{}}\r\n").unwrap();
        });

        let reply = send_qmp_command(&addr.to_string(), "system_powerdown").unwrap();
        assert!(reply.get("return").is_some());
        server.join().unwrap();
    }
}
