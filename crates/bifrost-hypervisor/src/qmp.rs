//! Minimal pure-Rust QMP (QEMU Machine Protocol) client.
//!
//! QEMU's QMP monitor has no authentication, so whoever can reach the endpoint can
//! control the VM. BIFRÖST therefore exposes QMP on a Windows **named pipe** with an
//! unpredictable 128-bit name (`pipe:bifrost-qmp-<hex>`) instead of a loopback TCP port
//! that any local process or user could connect to. QEMU creates a single pipe
//! instance with the default DACL: the owner, SYSTEM and Administrators get full
//! access, while other users only get read access and therefore cannot send commands.
//!
//! QEMU's Windows pipe chardev accepts **one** client connection for the lifetime of
//! the VM, so callers must run every command they need inside a single
//! [`QmpSession`]. Loopback TCP endpoints (`127.0.0.1:<port>`) are still accepted to
//! shut down VMs started by older BIFRÖST versions.
//!
//! All I/O happens on a dedicated worker thread that owns the connection; callers wait
//! with a timeout. This avoids concurrent reads and writes on one synchronous Windows
//! pipe handle, which would serialize and deadlock.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use std::time::Duration;

use bifrost_core::{BifrostError, Result};
use serde_json::Value;
use tracing::debug;

/// Default timeout for the QMP greeting/handshake and for each command reply.
pub const QMP_IO_TIMEOUT: Duration = Duration::from_secs(5);

/// Endpoint prefix for Windows named-pipe QMP channels.
pub const QMP_PIPE_ENDPOINT_PREFIX: &str = "pipe:";

/// Name prefix of every BIFRÖST QMP pipe.
pub const QMP_PIPE_NAME_PREFIX: &str = "bifrost-qmp-";

/// Generates an unpredictable QMP pipe name (`bifrost-qmp-<32 hex chars>`).
pub fn generate_qmp_pipe_name() -> Result<String> {
    let mut bytes = [0u8; 16];
    getrandom::getrandom(&mut bytes).map_err(|err| {
        BifrostError::Hypervisor(format!("failed to generate random QMP pipe name: {err}"))
    })?;
    Ok(format!("{QMP_PIPE_NAME_PREFIX}{}", hex::encode(bytes)))
}

/// Validates a QMP pipe name produced by [`generate_qmp_pipe_name`].
pub fn validate_qmp_pipe_name(name: &str) -> Result<()> {
    let Some(suffix) = name.strip_prefix(QMP_PIPE_NAME_PREFIX) else {
        return Err(BifrostError::Hypervisor(format!(
            "QMP pipe name '{name}' must start with '{QMP_PIPE_NAME_PREFIX}'"
        )));
    };
    if suffix.len() != 32 || !suffix.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(BifrostError::Hypervisor(format!(
            "QMP pipe name '{name}' must end with 32 hexadecimal characters"
        )));
    }
    Ok(())
}

/// Builds the endpoint string stored in runtime state for a QMP pipe name.
#[must_use]
pub fn pipe_endpoint(pipe_name: &str) -> String {
    format!("{QMP_PIPE_ENDPOINT_PREFIX}{pipe_name}")
}

/// Sends a single QMP command in a fresh session.
///
/// Prefer [`QmpSession`] when more than one command may be needed: a named-pipe QMP
/// endpoint cannot be reconnected after this session closes.
pub fn send_qmp_command(endpoint: &str, command: &str) -> Result<Value> {
    let session = QmpSession::connect(endpoint)?;
    session.execute(command)
}

type Job = (String, Sender<Result<Value>>);

/// A QMP connection owned by a worker thread, with per-command timeouts.
pub struct QmpSession {
    jobs: Sender<Job>,
    timeout: Duration,
}

impl QmpSession {
    /// Connects to `endpoint` (`pipe:<name>` or loopback `127.0.0.1:<port>`) and
    /// completes the QMP greeting and `qmp_capabilities` handshake.
    pub fn connect(endpoint: &str) -> Result<Self> {
        Self::connect_with_timeout(endpoint, QMP_IO_TIMEOUT)
    }

    /// Same as [`QmpSession::connect`] with an explicit timeout.
    pub fn connect_with_timeout(endpoint: &str, timeout: Duration) -> Result<Self> {
        let transport = Transport::parse(endpoint)?;
        let (jobs_tx, jobs_rx) = mpsc::channel::<Job>();
        let (ready_tx, ready_rx) = mpsc::channel::<Result<()>>();
        let endpoint_label = endpoint.to_string();

        thread::Builder::new()
            .name("bifrost-qmp".to_string())
            .spawn(move || worker(transport, &endpoint_label, &ready_tx, &jobs_rx))
            .map_err(|source| BifrostError::Io {
                context: "spawning QMP worker thread".to_string(),
                source,
            })?;

        match ready_rx.recv_timeout(timeout) {
            Ok(Ok(())) => Ok(Self {
                jobs: jobs_tx,
                timeout,
            }),
            Ok(Err(err)) => Err(err),
            Err(_) => Err(BifrostError::Hypervisor(format!(
                "timed out connecting to QMP endpoint '{endpoint}'"
            ))),
        }
    }

    /// Executes a QMP command without arguments (e.g. `system_powerdown`, `quit`).
    pub fn execute(&self, command: &str) -> Result<Value> {
        if command.is_empty()
            || !command
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-'))
        {
            return Err(BifrostError::Hypervisor(format!(
                "invalid QMP command name '{command}'"
            )));
        }
        let (reply_tx, reply_rx) = mpsc::channel();
        self.jobs
            .send((command.to_string(), reply_tx))
            .map_err(|_| BifrostError::Hypervisor("QMP session is closed".to_string()))?;
        match reply_rx.recv_timeout(self.timeout) {
            Ok(result) => result,
            Err(_) => Err(BifrostError::Hypervisor(format!(
                "timed out waiting for QMP reply to '{command}'"
            ))),
        }
    }
}

enum Transport {
    Tcp(String),
    Pipe(String),
}

impl Transport {
    fn parse(endpoint: &str) -> Result<Self> {
        if let Some(name) = endpoint.strip_prefix(QMP_PIPE_ENDPOINT_PREFIX) {
            validate_qmp_pipe_name(name)?;
            return Ok(Self::Pipe(name.to_string()));
        }
        if endpoint.starts_with("127.0.0.1:") || endpoint.starts_with("[::1]:") {
            return Ok(Self::Tcp(endpoint.to_string()));
        }
        Err(BifrostError::Hypervisor(format!(
            "refusing non-loopback QMP endpoint '{endpoint}'"
        )))
    }

    fn open(self) -> Result<(Box<dyn Read + Send>, Box<dyn Write + Send>)> {
        match self {
            Self::Tcp(addr) => {
                let stream = TcpStream::connect(&addr).map_err(|source| BifrostError::Io {
                    context: format!("connecting to QMP endpoint '{addr}'"),
                    source,
                })?;
                let writer = stream.try_clone().map_err(|source| BifrostError::Io {
                    context: "cloning QMP TCP stream".to_string(),
                    source,
                })?;
                Ok((Box::new(stream), Box::new(writer)))
            }
            Self::Pipe(name) => open_pipe(&name),
        }
    }
}

#[cfg(windows)]
fn open_pipe(name: &str) -> Result<(Box<dyn Read + Send>, Box<dyn Write + Send>)> {
    let path = format!(r"\\.\pipe\{name}");
    let file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(&path)
        .map_err(|source| BifrostError::Io {
            context: format!("opening QMP named pipe '{path}'"),
            source,
        })?;
    // Reads and writes happen sequentially on the single worker thread, so sharing the
    // same synchronous file object through a duplicated handle cannot deadlock.
    let writer = file.try_clone().map_err(|source| BifrostError::Io {
        context: "duplicating QMP named pipe handle".to_string(),
        source,
    })?;
    Ok((Box::new(file), Box::new(writer)))
}

#[cfg(not(windows))]
fn open_pipe(_name: &str) -> Result<(Box<dyn Read + Send>, Box<dyn Write + Send>)> {
    Err(BifrostError::Unsupported {
        feature: "QMP over Windows named pipes".to_string(),
        planned_milestone: "Windows 11 host only".to_string(),
    })
}

fn worker(transport: Transport, endpoint: &str, ready: &Sender<Result<()>>, jobs: &Receiver<Job>) {
    let opened = transport.open().and_then(|(reader, mut writer)| {
        let mut reader = BufReader::new(reader);
        handshake(&mut reader, &mut writer)?;
        Ok((reader, writer))
    });
    let (mut reader, mut writer) = match opened {
        Ok(io) => {
            let _ = ready.send(Ok(()));
            io
        }
        Err(err) => {
            let _ = ready.send(Err(err));
            return;
        }
    };
    debug!(endpoint, "QMP session established");

    while let Ok((command, reply)) = jobs.recv() {
        let result = run_command(&mut reader, &mut writer, &command);
        let broken = result.is_err();
        let _ = reply.send(result);
        if broken {
            break;
        }
    }
}

fn handshake(reader: &mut impl BufRead, writer: &mut impl Write) -> Result<()> {
    let mut greeting = String::new();
    let n = reader
        .read_line(&mut greeting)
        .map_err(|source| BifrostError::Io {
            context: "reading QMP greeting".to_string(),
            source,
        })?;
    if n == 0 || !greeting.contains("\"QMP\"") {
        return Err(BifrostError::Hypervisor(
            "QMP endpoint did not send a QMP greeting".to_string(),
        ));
    }
    debug!(greeting = greeting.trim(), "received QMP greeting");
    run_command(reader, writer, "qmp_capabilities").map(|_| ())
}

fn run_command(reader: &mut impl BufRead, writer: &mut impl Write, command: &str) -> Result<Value> {
    let payload = format!("{{\"execute\":\"{command}\"}}\r\n");
    writer
        .write_all(payload.as_bytes())
        .and_then(|()| writer.flush())
        .map_err(|source| BifrostError::Io {
            context: format!("sending QMP command '{command}'"),
            source,
        })?;
    read_non_event_reply(reader)
}

fn read_non_event_reply(reader: &mut impl BufRead) -> Result<Value> {
    for _ in 0..32 {
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

    #[test]
    fn rejects_non_loopback_and_malformed_endpoints() {
        let err = send_qmp_command("192.168.1.10:4444", "system_powerdown").unwrap_err();
        assert!(err.to_string().contains("non-loopback"));
        assert!(QmpSession::connect("pipe:not-ours").is_err());
        assert!(QmpSession::connect(r"pipe:bifrost-qmp-..\..\x").is_err());
    }

    #[test]
    fn generates_valid_unpredictable_pipe_names() {
        let a = generate_qmp_pipe_name().unwrap();
        let b = generate_qmp_pipe_name().unwrap();
        assert_ne!(a, b);
        assert!(validate_qmp_pipe_name(&a).is_ok());
        assert_eq!(pipe_endpoint(&a), format!("pipe:{a}"));
    }

    #[test]
    fn runs_multiple_commands_in_one_session() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();

        let server = thread::spawn(move || {
            let (mut sock, _) = listener.accept().unwrap();
            sock.write_all(b"{\"QMP\":{\"version\":{}}}\r\n").unwrap();
            let mut reader = BufReader::new(sock.try_clone().unwrap());
            for expected in ["qmp_capabilities", "system_powerdown", "quit"] {
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                assert!(line.contains(expected), "expected {expected}, got {line}");
                if expected == "system_powerdown" {
                    sock.write_all(b"{\"event\":\"POWERDOWN\"}\r\n").unwrap();
                }
                sock.write_all(b"{\"return\":{}}\r\n").unwrap();
            }
        });

        let session = QmpSession::connect(&addr.to_string()).unwrap();
        assert!(session
            .execute("system_powerdown")
            .unwrap()
            .get("return")
            .is_some());
        assert!(session.execute("quit").is_ok());
        assert!(session.execute("bad command;").is_err());
        server.join().unwrap();
    }

    /// Live check against a real QEMU process (`-machine none`, no guest needed).
    /// Run with `cargo test -p bifrost-hypervisor -- --ignored` on a host with QEMU.
    #[cfg(windows)]
    #[test]
    #[ignore = "requires qemu-system-x86_64 on the host"]
    fn live_qemu_named_pipe_session() {
        let qemu = std::env::var("BIFROST_QEMU_BIN")
            .unwrap_or_else(|_| r"C:\Program Files\qemu\qemu-system-x86_64.exe".to_string());
        let name = generate_qmp_pipe_name().unwrap();
        let mut child = std::process::Command::new(qemu)
            .args(["-machine", "none", "-display", "none", "-chardev"])
            .arg(format!("pipe,id=qmp0,path={name}"))
            .args(["-mon", "chardev=qmp0,mode=control"])
            .spawn()
            .unwrap();

        let endpoint = pipe_endpoint(&name);
        let mut session = None;
        for _ in 0..40 {
            if let Ok(s) = QmpSession::connect(&endpoint) {
                session = Some(s);
                break;
            }
            thread::sleep(Duration::from_millis(100));
        }
        let session = session.expect("QMP pipe never became available");
        let status = session.execute("query-status").unwrap();
        assert_eq!(status["return"]["running"], Value::Bool(true));

        // The pipe accepts a single client: a second connection must fail.
        assert!(QmpSession::connect_with_timeout(&endpoint, Duration::from_millis(500)).is_err());

        let _ = session.execute("quit");
        let exited = (0..40).any(|_| {
            thread::sleep(Duration::from_millis(100));
            matches!(child.try_wait(), Ok(Some(_)))
        });
        if !exited {
            let _ = child.kill();
        }
        assert!(exited, "QEMU did not exit after QMP quit");
    }

    #[test]
    fn times_out_when_endpoint_never_greets() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let _server = thread::spawn(move || {
            let (_sock, _) = listener.accept().unwrap();
            thread::sleep(Duration::from_secs(2));
        });
        let err = QmpSession::connect_with_timeout(&addr.to_string(), Duration::from_millis(300))
            .err()
            .unwrap();
        assert!(err.to_string().contains("timed out"));
    }
}
