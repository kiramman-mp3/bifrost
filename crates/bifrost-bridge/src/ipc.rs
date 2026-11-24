//! Authenticated local IPC protocol, framing, and security primitives for Heimdall.

use std::path::{Path, PathBuf};

use bifrost_core::{BifrostError, Result, RuntimeState};
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

use crate::adb::{AdbEndpointState, GuestHealth};

/// Current Heimdall IPC wire protocol version.
pub const IPC_PROTOCOL_VERSION: u32 = 1;

/// Maximum permitted single IPC frame size (64 KiB) to prevent memory exhaustion.
pub const MAX_IPC_FRAME_BYTES: usize = 64 * 1024;

/// Authenticated IPC request envelope sent from Mimir / `bifrost-ui` to Heimdall.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IpcRequestEnvelope {
    pub protocol_version: u32,
    pub request_id: u64,
    pub auth_token: String,
    pub request: IpcRequest,
}

/// Supported authenticated control-plane operations exposed by Heimdall.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum IpcRequest {
    Ping,
    DaemonHealth,
    RuntimeStatus,
    RuntimeStart {
        manifest_path: PathBuf,
        headless: bool,
    },
    RuntimeStop {
        force: bool,
    },
    RuntimeRestart {
        manifest_path: Option<PathBuf>,
        headless: bool,
    },
    RuntimeLogs {
        tail: usize,
    },
    DevicesList,
    AppsList {
        serial: Option<String>,
        third_party_only: bool,
    },
    AppInfo {
        serial: Option<String>,
        package: String,
    },
    AppInstall {
        serial: Option<String>,
        apk_path: PathBuf,
    },
    AppUninstall {
        serial: Option<String>,
        package: String,
        keep_data: bool,
    },
    AppLaunch {
        serial: Option<String>,
        package: String,
    },
    AppStop {
        serial: Option<String>,
        package: String,
    },
    DaemonShutdown,
}

/// Structured IPC response envelope returned by Heimdall.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IpcResponseEnvelope {
    pub protocol_version: u32,
    pub request_id: u64,
    pub ok: bool,
    pub payload: Option<serde_json::Value>,
    pub error: Option<String>,
}

impl IpcResponseEnvelope {
    /// Builds a successful IPC response with serialized JSON payload.
    pub fn success<T: Serialize>(request_id: u64, value: &T) -> Result<Self> {
        let payload = serde_json::to_value(value)
            .map_err(|err| BifrostError::Ipc(format!("failed to encode IPC payload: {err}")))?;
        Ok(Self {
            protocol_version: IPC_PROTOCOL_VERSION,
            request_id,
            ok: true,
            payload: Some(payload),
            error: None,
        })
    }

    /// Builds a failed IPC response carrying a safe error string.
    #[must_use]
    pub fn failure(request_id: u64, error: impl Into<String>) -> Self {
        Self {
            protocol_version: IPC_PROTOCOL_VERSION,
            request_id,
            ok: false,
            payload: None,
            error: Some(error.into()),
        }
    }
}

/// Live health report maintained by the Heimdall supervisor loop.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DaemonHealthReport {
    pub daemon_pid: u32,
    pub uptime_secs: u64,
    pub runtime_state: RuntimeState,
    pub vm_pid: Option<u32>,
    pub adb_available: bool,
    pub adb_device_count: usize,
    pub guest_health: Option<GuestHealth>,
    pub health_checks_completed: u64,
    pub pipe_name: String,
    /// ADB access to Midgard while it is active; `None` when stopped or not yet probed.
    #[serde(default)]
    pub adb_access: Option<AdbEndpointState>,
}

/// Returns the default per-user Windows Named Pipe path for Heimdall.
#[must_use]
pub fn default_pipe_name() -> String {
    let raw_user = std::env::var("USERNAME")
        .or_else(|_| std::env::var("USER"))
        .unwrap_or_else(|_| "local".to_string());
    let sanitized: String = raw_user
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect();
    format!(r"\\.\pipe\bifrost-heimdall-{sanitized}")
}

/// Path to the per-session Heimdall authentication token file inside `data_dir`.
#[must_use]
pub fn token_file_path(data_dir: &Path) -> PathBuf {
    data_dir.join("heimdall.token")
}

/// Generates a 256-bit cryptographically secure random hex authentication token.
pub fn generate_auth_token() -> Result<String> {
    let mut bytes = [0u8; 32];
    getrandom::getrandom(&mut bytes)
        .map_err(|err| BifrostError::Ipc(format!("failed to generate IPC auth token: {err}")))?;
    Ok(hex::encode(bytes))
}

/// Constant-time equality comparison for IPC authentication tokens.
#[must_use]
pub fn constant_time_eq(a: &str, b: &str) -> bool {
    let a_bytes = a.as_bytes();
    let b_bytes = b.as_bytes();
    if a_bytes.len() != b_bytes.len() {
        return false;
    }
    let mut diff: u8 = 0;
    for (&x, &y) in a_bytes.iter().zip(b_bytes.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

/// Writes a length-prefixed JSON frame (`u32` BE length + body) bounded by [`MAX_IPC_FRAME_BYTES`].
pub async fn write_frame<W, T>(writer: &mut W, message: &T) -> Result<()>
where
    W: AsyncWrite + Unpin,
    T: Serialize,
{
    let payload = serde_json::to_vec(message)
        .map_err(|err| BifrostError::Ipc(format!("failed to serialize IPC frame: {err}")))?;
    if payload.len() > MAX_IPC_FRAME_BYTES {
        return Err(BifrostError::Ipc(format!(
            "IPC frame size {} exceeds maximum {} bytes",
            payload.len(),
            MAX_IPC_FRAME_BYTES
        )));
    }
    let len = payload.len() as u32;
    writer
        .write_all(&len.to_be_bytes())
        .await
        .map_err(|source| BifrostError::Io {
            context: "writing IPC frame length".to_string(),
            source,
        })?;
    writer
        .write_all(&payload)
        .await
        .map_err(|source| BifrostError::Io {
            context: "writing IPC frame body".to_string(),
            source,
        })?;
    writer.flush().await.map_err(|source| BifrostError::Io {
        context: "flushing IPC stream".to_string(),
        source,
    })?;
    Ok(())
}

/// Reads and deserializes a bounded length-prefixed JSON frame.
pub async fn read_frame<R, T>(reader: &mut R) -> Result<T>
where
    R: AsyncRead + Unpin,
    T: for<'de> Deserialize<'de>,
{
    let mut len_buf = [0u8; 4];
    reader
        .read_exact(&mut len_buf)
        .await
        .map_err(|source| BifrostError::Io {
            context: "reading IPC frame length".to_string(),
            source,
        })?;
    let len = u32::from_be_bytes(len_buf) as usize;
    if len == 0 || len > MAX_IPC_FRAME_BYTES {
        return Err(BifrostError::Ipc(format!(
            "rejected IPC frame with invalid length {len} (max {MAX_IPC_FRAME_BYTES})"
        )));
    }
    let mut body = vec![0u8; len];
    reader
        .read_exact(&mut body)
        .await
        .map_err(|source| BifrostError::Io {
            context: "reading IPC frame body".to_string(),
            source,
        })?;
    serde_json::from_slice(&body)
        .map_err(|err| BifrostError::Ipc(format!("malformed IPC JSON frame: {err}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn frames_roundtrip_and_reject_oversized_messages() {
        let (mut client, mut server) = tokio::io::duplex(4096);
        let token = generate_auth_token().unwrap();
        assert_eq!(token.len(), 64);
        assert!(constant_time_eq(&token, &token));
        assert!(!constant_time_eq(&token, "deadbeef"));

        let req = IpcRequestEnvelope {
            protocol_version: IPC_PROTOCOL_VERSION,
            request_id: 7,
            auth_token: token.clone(),
            request: IpcRequest::Ping,
        };
        write_frame(&mut client, &req).await.unwrap();
        let decoded: IpcRequestEnvelope = read_frame(&mut server).await.unwrap();
        assert_eq!(decoded.request_id, 7);
        assert_eq!(decoded.request, IpcRequest::Ping);
        assert!(constant_time_eq(&decoded.auth_token, &token));

        // Oversized length prefix must be rejected immediately.
        let (mut bad_client, mut bad_server) = tokio::io::duplex(64);
        let huge = (MAX_IPC_FRAME_BYTES as u32 + 1).to_be_bytes();
        bad_client.write_all(&huge).await.unwrap();
        let err = read_frame::<_, IpcRequestEnvelope>(&mut bad_server)
            .await
            .unwrap_err();
        assert!(err.to_string().contains("invalid length"));
    }
}
