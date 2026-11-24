//! Minimal pure-Rust QMP (QEMU Machine Protocol) client.
//!
//! QEMU's QMP monitor has no authentication, so whoever can reach the endpoint can
//! control the VM. BIFRÖST therefore exposes QMP on an **AF_UNIX socket file**
//! (`unix:<dir>\qmp-<32 hex>.sock`) inside the user's own profile (`data_dir\run`, or
//! `%TEMP%\bifrost-run` as a fallback) instead of a loopback TCP port that any local
//! process or user could connect to. Windows checks the socket file's ACL on
//! `connect`, and both directories inherit the per-user ACL of `%LOCALAPPDATA%`, so
//! other users cannot reach it. QEMU removes the socket file when it exits.
//!
//! A Windows named pipe was used briefly during development, but QEMU's pipe chardev
//! blocks VM start-up until a client connects and accepts only one client for the
//! VM's lifetime; the socket chardev with `server=on,wait=off` has neither problem.
//! Loopback TCP endpoints (`127.0.0.1:<port>`) are still accepted to shut down VMs
//! started by older BIFRÖST releases.
//!
//! All I/O happens on a dedicated worker thread that owns the connection; callers wait
//! with a timeout.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use std::time::Duration;

use bifrost_core::{BifrostError, Result};
use serde_json::Value;
use tracing::debug;

/// Default timeout for the QMP greeting/handshake and for each command reply.
pub const QMP_IO_TIMEOUT: Duration = Duration::from_secs(5);

/// Endpoint prefix for AF_UNIX socket QMP channels.
pub const QMP_SOCKET_ENDPOINT_PREFIX: &str = "unix:";

/// File-name prefix of every BIFRÖST QMP socket.
pub const QMP_SOCKET_FILE_PREFIX: &str = "qmp-";

/// `sockaddr_un::sun_path` holds 108 bytes including the terminating NUL; keep margin.
pub const QMP_SOCKET_MAX_PATH_BYTES: usize = 100;

/// Chooses a fresh, unpredictable QMP socket path (`<dir>\qmp-<32 hex>.sock`) and
/// creates its parent directory.
///
/// `preferred_dir` (normally `data_dir\run`) is used when the resulting path is valid
/// for QEMU and AF_UNIX (ASCII, no commas, at most [`QMP_SOCKET_MAX_PATH_BYTES`]);
/// otherwise `%TEMP%\bifrost-run` is tried.
pub fn allocate_qmp_socket_path(preferred_dir: &Path) -> Result<PathBuf> {
    let mut bytes = [0u8; 16];
    getrandom::getrandom(&mut bytes).map_err(|err| {
        BifrostError::Hypervisor(format!("failed to generate random QMP socket name: {err}"))
    })?;
    let file_name = format!("{QMP_SOCKET_FILE_PREFIX}{}.sock", hex::encode(bytes));

    let fallback = std::env::temp_dir().join("bifrost-run");
    let mut last_err = None;
    for dir in [preferred_dir, fallback.as_path()] {
        let candidate = dir.join(&file_name);
        match validate_qmp_socket_path(&candidate) {
            Ok(()) => {
                std::fs::create_dir_all(dir).map_err(|source| BifrostError::Io {
                    context: format!("creating QMP socket directory '{}'", dir.display()),
                    source,
                })?;
                return Ok(candidate);
            }
            Err(err) => last_err = Some(err),
        }
    }
    Err(last_err
        .unwrap_or_else(|| BifrostError::Hypervisor("no usable QMP socket directory".to_string())))
}

/// Validates a QMP socket path produced by [`allocate_qmp_socket_path`].
pub fn validate_qmp_socket_path(path: &Path) -> Result<()> {
    let reject = |reason: &str| {
        Err(BifrostError::UnsafePath {
            path: path.to_path_buf(),
            reason: format!("invalid QMP socket path: {reason}"),
        })
    };
    let Some(text) = path.to_str() else {
        return reject("not valid Unicode");
    };
    if !path.is_absolute() {
        return reject("must be absolute");
    }
    // QEMU parses `-chardev socket,path=...` with ',' as the option separator and reads
    // its argv in the ANSI code page, so only plain ASCII without commas is safe.
    if !text.is_ascii() || text.contains(',') || text.chars().any(char::is_control) {
        return reject("must be ASCII without commas or control characters");
    }
    if text.len() > QMP_SOCKET_MAX_PATH_BYTES {
        return reject("longer than the AF_UNIX path limit");
    }
    let file_name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
    let hex_part = file_name
        .strip_prefix(QMP_SOCKET_FILE_PREFIX)
        .and_then(|rest| rest.strip_suffix(".sock"));
    if !hex_part.is_some_and(|h| h.len() == 32 && h.chars().all(|c| c.is_ascii_hexdigit())) {
        return reject("file name must be 'qmp-<32 hex>.sock'");
    }
    if path
        .components()
        .any(|c| matches!(c, std::path::Component::ParentDir))
    {
        return reject("must not contain '..'");
    }
    Ok(())
}

/// Builds the endpoint string stored in runtime state for a QMP socket path.
#[must_use]
pub fn socket_endpoint(path: &Path) -> String {
    format!("{QMP_SOCKET_ENDPOINT_PREFIX}{}", path.display())
}

/// Removes a leftover QMP socket file (e.g. after a forced kill). Only paths that
/// validate as BIFRÖST QMP sockets are touched; other endpoints are ignored.
pub fn remove_stale_qmp_socket(endpoint: &str) {
    if let Some(raw) = endpoint.strip_prefix(QMP_SOCKET_ENDPOINT_PREFIX) {
        let path = Path::new(raw);
        if validate_qmp_socket_path(path).is_ok() && path.exists() {
            let _ = std::fs::remove_file(path);
        }
    }
}

/// Sends a single QMP command in a fresh session.
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
    /// Connects to `endpoint` (`unix:<path>` or loopback `127.0.0.1:<port>`) and
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
    Unix(PathBuf),
}

impl Transport {
    fn parse(endpoint: &str) -> Result<Self> {
        if let Some(raw) = endpoint.strip_prefix(QMP_SOCKET_ENDPOINT_PREFIX) {
            let path = PathBuf::from(raw);
            validate_qmp_socket_path(&path)?;
            return Ok(Self::Unix(path));
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
            Self::Unix(path) => open_unix(&path),
        }
    }
}

#[cfg(windows)]
fn open_unix(path: &Path) -> Result<(Box<dyn Read + Send>, Box<dyn Write + Send>)> {
    let stream = uds_windows::UnixStream::connect(path).map_err(|source| BifrostError::Io {
        context: format!("connecting to QMP socket '{}'", path.display()),
        source,
    })?;
    let writer = stream.try_clone().map_err(|source| BifrostError::Io {
        context: "cloning QMP socket".to_string(),
        source,
    })?;
    Ok((Box::new(stream), Box::new(writer)))
}

#[cfg(unix)]
fn open_unix(path: &Path) -> Result<(Box<dyn Read + Send>, Box<dyn Write + Send>)> {
    let stream =
        std::os::unix::net::UnixStream::connect(path).map_err(|source| BifrostError::Io {
            context: format!("connecting to QMP socket '{}'", path.display()),
            source,
        })?;
    let writer = stream.try_clone().map_err(|source| BifrostError::Io {
        context: "cloning QMP socket".to_string(),
        source,
    })?;
    Ok((Box::new(stream), Box::new(writer)))
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
        assert!(QmpSession::connect("pipe:bifrost-qmp-00").is_err());
        assert!(QmpSession::connect(r"unix:C:\Windows\evil.sock").is_err());
        assert!(QmpSession::connect(r"unix:relative\qmp-00.sock").is_err());
    }

    #[test]
    fn allocates_valid_unpredictable_socket_paths() {
        let dir = tempfile::tempdir().unwrap();
        let a = allocate_qmp_socket_path(dir.path()).unwrap();
        let b = allocate_qmp_socket_path(dir.path()).unwrap();
        assert_ne!(a, b);
        assert!(validate_qmp_socket_path(&a).is_ok());
        assert!(socket_endpoint(&a).starts_with("unix:"));
    }

    #[test]
    fn falls_back_when_preferred_dir_is_unusable() {
        // Non-ASCII and comma-containing directories are rejected for QEMU's argv.
        let preferred = std::env::temp_dir().join("BIFRÖST, data");
        let path = allocate_qmp_socket_path(&preferred).unwrap();
        assert!(path.starts_with(std::env::temp_dir().join("bifrost-run")));
    }

    #[test]
    fn validates_socket_path_shape() {
        let base = std::env::temp_dir();
        let good = base.join(format!("qmp-{}.sock", "a".repeat(32)));
        if good.to_str().is_some_and(str::is_ascii) {
            assert!(validate_qmp_socket_path(&good).is_ok());
        }
        assert!(validate_qmp_socket_path(&base.join("qmp-zz.sock")).is_err());
        assert!(
            validate_qmp_socket_path(&base.join("a,b").join(good.file_name().unwrap())).is_err()
        );
        let long = base.join("x".repeat(120)).join(good.file_name().unwrap());
        assert!(validate_qmp_socket_path(&long).is_err());
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

    #[cfg(windows)]
    #[test]
    fn talks_qmp_over_af_unix_socket() {
        let dir = tempfile::tempdir().unwrap();
        let path = allocate_qmp_socket_path(dir.path()).unwrap();
        let listener = uds_windows::UnixListener::bind(&path).unwrap();
        let server = thread::spawn(move || {
            // Two sequential clients: reconnecting must work (unlike the pipe chardev).
            for _ in 0..2 {
                let (mut sock, _) = listener.accept().unwrap();
                sock.write_all(b"{\"QMP\":{}}\r\n").unwrap();
                let mut reader = BufReader::new(sock.try_clone().unwrap());
                for _ in 0..2 {
                    let mut line = String::new();
                    reader.read_line(&mut line).unwrap();
                    sock.write_all(b"{\"return\":{}}\r\n").unwrap();
                }
            }
        });
        let endpoint = socket_endpoint(&path);
        for _ in 0..2 {
            let session = QmpSession::connect(&endpoint).unwrap();
            assert!(session.execute("query-status").is_ok());
        }
        server.join().unwrap();
        remove_stale_qmp_socket(&endpoint);
        assert!(!path.exists());
    }

    /// Live check against a real QEMU process (`-machine none`, no guest needed).
    /// Run with `cargo test -p bifrost-hypervisor -- --ignored` on a host with QEMU.
    #[cfg(windows)]
    #[test]
    #[ignore = "requires qemu-system-x86_64 on the host"]
    fn live_qemu_unix_socket_sessions() {
        let qemu = std::env::var("BIFROST_QEMU_BIN")
            .unwrap_or_else(|_| r"C:\Program Files\qemu\qemu-system-x86_64.exe".to_string());
        let path = allocate_qmp_socket_path(&std::env::temp_dir().join("bifrost-run")).unwrap();
        let mut child = std::process::Command::new(qemu)
            .args(["-machine", "none", "-display", "none", "-chardev"])
            .arg(format!(
                "socket,id=qmp0,path={},server=on,wait=off",
                path.display()
            ))
            .args(["-mon", "chardev=qmp0,mode=control"])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .unwrap();

        let endpoint = socket_endpoint(&path);
        let connect = || {
            (0..40).find_map(|_| {
                QmpSession::connect(&endpoint).ok().or_else(|| {
                    thread::sleep(Duration::from_millis(100));
                    None
                })
            })
        };
        // QEMU must not block start-up waiting for a client, and must accept reconnects.
        for _ in 0..2 {
            let session = connect().expect("QMP socket never became available");
            let status = session.execute("query-status").unwrap();
            assert_eq!(status["return"]["running"], Value::Bool(true));
        }

        let _ = connect().expect("third connection").execute("quit");
        let exited = (0..40).any(|_| {
            thread::sleep(Duration::from_millis(100));
            matches!(child.try_wait(), Ok(Some(_)))
        });
        if !exited {
            let _ = child.kill();
        }
        assert!(exited, "QEMU did not exit after QMP quit");
        remove_stale_qmp_socket(&endpoint);
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
