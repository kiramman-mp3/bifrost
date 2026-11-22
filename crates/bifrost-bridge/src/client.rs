//! Async client for communicating with the Heimdall background daemon over local IPC.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use bifrost_core::{BifrostError, Result};
use serde::de::DeserializeOwned;
use tokio::time::timeout;

use crate::ipc::{
    read_frame, token_file_path, write_frame, DaemonHealthReport, IpcRequest, IpcRequestEnvelope,
    IpcResponseEnvelope, IPC_PROTOCOL_VERSION,
};

const IPC_FAST_TIMEOUT: Duration = Duration::from_secs(10);
const IPC_LONG_OP_TIMEOUT: Duration = Duration::from_secs(120);

fn timeout_for_request(req: &IpcRequest) -> Duration {
    match req {
        IpcRequest::AppInstall { .. }
        | IpcRequest::AppUninstall { .. }
        | IpcRequest::RuntimeStart { .. }
        | IpcRequest::RuntimeStop { .. }
        | IpcRequest::RuntimeRestart { .. } => IPC_LONG_OP_TIMEOUT,
        _ => IPC_FAST_TIMEOUT,
    }
}

/// Client speaking the authenticated Heimdall IPC protocol.
#[derive(Debug, Clone)]
pub struct HeimdallClient {
    pipe_name: String,
    token_path: PathBuf,
}

impl HeimdallClient {
    /// Creates a client targeting `pipe_name` and reading the session auth token from `data_dir`.
    #[must_use]
    pub fn new(pipe_name: impl Into<String>, data_dir: &Path) -> Self {
        Self {
            pipe_name: pipe_name.into(),
            token_path: token_file_path(data_dir),
        }
    }

    /// Returns the Windows Named Pipe path targeted by this client.
    #[must_use]
    pub fn pipe_name(&self) -> &str {
        &self.pipe_name
    }

    /// Sends an authenticated IPC request to Heimdall and decodes the response payload.
    pub async fn send<T: DeserializeOwned>(&self, request: IpcRequest) -> Result<T> {
        let auth_token = fs::read_to_string(&self.token_path)
            .map_err(|_| {
                BifrostError::Ipc(format!(
                    "Heimdall auth token not found at '{}' (daemon not running)",
                    self.token_path.display()
                ))
            })?
            .trim()
            .to_string();

        let envelope = IpcRequestEnvelope {
            protocol_version: IPC_PROTOCOL_VERSION,
            request_id: 1,
            auth_token,
            request,
        };

        let response = self.exchange(&envelope).await?;
        if response.protocol_version != IPC_PROTOCOL_VERSION {
            return Err(BifrostError::Ipc(format!(
                "unexpected IPC response protocol version {}",
                response.protocol_version
            )));
        }
        if !response.ok {
            return Err(BifrostError::Ipc(response.error.unwrap_or_else(|| {
                "Heimdall returned an unspecified error".to_string()
            })));
        }
        let payload = response.payload.ok_or_else(|| {
            BifrostError::Ipc("Heimdall returned empty success payload".to_string())
        })?;
        serde_json::from_value(payload)
            .map_err(|err| BifrostError::Ipc(format!("failed to decode IPC response: {err}")))
    }

    /// Probes whether Heimdall is reachable and returns its live health snapshot.
    pub async fn health(&self) -> Result<DaemonHealthReport> {
        self.send(IpcRequest::DaemonHealth).await
    }

    #[cfg(windows)]
    async fn exchange(&self, req: &IpcRequestEnvelope) -> Result<IpcResponseEnvelope> {
        use tokio::net::windows::named_pipe::ClientOptions;

        let mut pipe = ClientOptions::new()
            .open(&self.pipe_name)
            .map_err(|source| {
                BifrostError::Ipc(format!(
                    "unable to connect to Heimdall pipe '{}': {source}",
                    self.pipe_name
                ))
            })?;

        let op_timeout = timeout_for_request(&req.request);
        timeout(op_timeout, async {
            write_frame(&mut pipe, req).await?;
            read_frame::<_, IpcResponseEnvelope>(&mut pipe).await
        })
        .await
        .map_err(|_| BifrostError::Ipc("timed out waiting for Heimdall IPC response".to_string()))?
    }

    #[cfg(not(windows))]
    async fn exchange(&self, _req: &IpcRequestEnvelope) -> Result<IpcResponseEnvelope> {
        Err(BifrostError::Unsupported {
            feature: "Heimdall Windows Named Pipe IPC".to_string(),
            planned_milestone: "Windows 11 x64 host only".to_string(),
        })
    }
}
