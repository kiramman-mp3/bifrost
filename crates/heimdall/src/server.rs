//! Authenticated Windows Named Pipe IPC server for Heimdall.

use std::fs;
use std::path::PathBuf;
use std::sync::Arc;

use bifrost_bridge::{
    constant_time_eq, generate_auth_token, read_frame, token_file_path, write_frame, IpcRequest,
    IpcRequestEnvelope, IpcResponseEnvelope, IPC_PROTOCOL_VERSION,
};
use bifrost_core::{BifrostError, Result};
use bifrost_runtime::StartOptions;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::sync::watch;
use tracing::{info, warn};

use crate::supervisor::SupervisorState;

/// Runs the authenticated Heimdall IPC server until shutdown is requested.
#[cfg(windows)]
pub async fn serve_named_pipe(
    state: Arc<SupervisorState>,
    data_dir: PathBuf,
    shutdown_tx: watch::Sender<bool>,
    mut shutdown_rx: watch::Receiver<bool>,
) -> Result<()> {
    fs::create_dir_all(&data_dir).map_err(|source| BifrostError::Io {
        context: format!("creating Heimdall data dir '{}'", data_dir.display()),
        source,
    })?;

    let pipe_name = state.pipe_name.clone();

    // Claim the pipe name *before* touching the token file. A second Heimdall started
    // while one is already running fails here (`first_pipe_instance`) and must not
    // overwrite the running daemon's token, which would lock every client out of it.
    let mut server = create_pipe_instance(&pipe_name, true)?;

    let token = generate_auth_token()?;
    let token_path = token_file_path(&data_dir);
    fs::write(&token_path, &token).map_err(|source| BifrostError::Io {
        context: format!("writing Heimdall auth token '{}'", token_path.display()),
        source,
    })?;
    let auth_token = Arc::new(token);

    info!(pipe = %pipe_name, "Heimdall authenticated Named Pipe server listening");

    loop {
        tokio::select! {
            connect_res = server.connect() => {
                // Always have the next instance listening before handing this one off.
                let next = match create_pipe_instance(&pipe_name, false) {
                    Ok(next) => next,
                    Err(err) => {
                        let _ = fs::remove_file(&token_path);
                        return Err(err);
                    }
                };
                let connected = std::mem::replace(&mut server, next);
                if let Err(err) = connect_res {
                    warn!(error = %err, "Named Pipe client connect failed");
                    continue;
                }
                let state_clone = Arc::clone(&state);
                let token_clone = Arc::clone(&auth_token);
                let shutdown_clone = shutdown_tx.clone();
                tokio::spawn(async move {
                    handle_connection(connected, state_clone, token_clone, shutdown_clone).await;
                });
            }
            changed = shutdown_rx.changed() => {
                if changed.is_err() || *shutdown_rx.borrow() {
                    info!("Heimdall IPC server shutting down");
                    break;
                }
            }
        }
    }

    let _ = fs::remove_file(&token_path);
    Ok(())
}

/// Creates one owner-only Named Pipe server instance.
#[cfg(windows)]
fn create_pipe_instance(
    pipe_name: &str,
    first_instance: bool,
) -> Result<tokio::net::windows::named_pipe::NamedPipeServer> {
    use tokio::net::windows::named_pipe::ServerOptions;

    use crate::pipe_security::OwnerOnlySecurityAttributes;

    let mut sec_attrs = OwnerOnlySecurityAttributes::new()?;
    let mut opts = ServerOptions::new();
    opts.first_pipe_instance(first_instance);

    // SAFETY: `sec_attrs` owns a valid owner-only `SECURITY_ATTRIBUTES` that outlives this
    // synchronous Win32 pipe creation call; the descriptor is copied into the kernel object.
    #[allow(unsafe_code)]
    unsafe { opts.create_with_security_attributes_raw(pipe_name, sec_attrs.as_raw_mut()) }.map_err(
        |source| BifrostError::Io {
            context: if first_instance {
                format!("creating Named Pipe '{pipe_name}' (is another Heimdall already running?)")
            } else {
                format!("creating Named Pipe '{pipe_name}'")
            },
            source,
        },
    )
}

#[cfg(not(windows))]
pub async fn serve_named_pipe(
    _state: Arc<SupervisorState>,
    _data_dir: PathBuf,
    _shutdown_tx: watch::Sender<bool>,
    _shutdown_rx: watch::Receiver<bool>,
) -> Result<()> {
    Err(BifrostError::Unsupported {
        feature: "Heimdall Windows Named Pipe server".to_string(),
        planned_milestone: "Windows 11 x64 host only".to_string(),
    })
}

async fn handle_connection<S>(
    mut stream: S,
    state: Arc<SupervisorState>,
    expected_token: Arc<String>,
    shutdown_tx: watch::Sender<bool>,
) where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let envelope: IpcRequestEnvelope = match read_frame(&mut stream).await {
        Ok(env) => env,
        Err(err) => {
            warn!(error = %err, "rejected invalid IPC request frame");
            let resp = IpcResponseEnvelope::failure(0, err.to_string());
            let _ = write_frame(&mut stream, &resp).await;
            return;
        }
    };

    let response = dispatch_request(&envelope, &state, &expected_token, &shutdown_tx).await;
    if let Err(err) = write_frame(&mut stream, &response).await {
        warn!(error = %err, "failed to write IPC response frame");
    }
}

/// Runs a blocking supervisor operation on Tokio's blocking pool and serializes its result.
async fn run_blocking<T, F>(state: &Arc<SupervisorState>, op: F) -> Result<serde_json::Value>
where
    T: serde::Serialize,
    F: FnOnce(&SupervisorState) -> Result<T> + Send + 'static,
{
    let state = Arc::clone(state);
    tokio::task::spawn_blocking(move || {
        op(&state)
            .and_then(|v| serde_json::to_value(v).map_err(|e| BifrostError::Ipc(e.to_string())))
    })
    .await
    .map_err(|err| BifrostError::Ipc(format!("IPC worker task failed: {err}")))?
}

async fn dispatch_request(
    envelope: &IpcRequestEnvelope,
    state: &Arc<SupervisorState>,
    expected_token: &str,
    shutdown_tx: &watch::Sender<bool>,
) -> IpcResponseEnvelope {
    if envelope.protocol_version != IPC_PROTOCOL_VERSION {
        return IpcResponseEnvelope::failure(
            envelope.request_id,
            format!(
                "unsupported IPC protocol version {} (expected {})",
                envelope.protocol_version, IPC_PROTOCOL_VERSION
            ),
        );
    }

    if !constant_time_eq(&envelope.auth_token, expected_token) {
        warn!(
            request_id = envelope.request_id,
            "rejected unauthenticated IPC request"
        );
        return IpcResponseEnvelope::failure(envelope.request_id, "IPC authentication failed");
    }

    // Every arm below performs blocking work (image hashing, VMM spawn, `adb.exe`
    // round-trips), so it runs on Tokio's blocking pool instead of an async worker.
    let request = envelope.request.clone();
    let result: Result<serde_json::Value> = match request {
        IpcRequest::Ping => Ok(serde_json::json!({
            "pong": true,
            "daemon_pid": std::process::id(),
        })),
        IpcRequest::DaemonShutdown => {
            let _ = shutdown_tx.send(true);
            Ok(serde_json::json!({ "shutting_down": true }))
        }
        IpcRequest::DaemonHealth => run_blocking(state, |s| Ok(s.refresh_health())).await,
        IpcRequest::RuntimeStatus => run_blocking(state, |s| s.runtime.status()).await,
        IpcRequest::RuntimeStart {
            manifest_path,
            headless,
        } => {
            run_blocking(state, move |s| {
                s.runtime.start(
                    manifest_path,
                    StartOptions {
                        headless,
                        interactive_console: false,
                    },
                )
            })
            .await
        }
        IpcRequest::RuntimeStop { force } => {
            run_blocking(state, move |s| s.runtime.stop(force)).await
        }
        IpcRequest::RuntimeRestart {
            manifest_path,
            headless,
        } => {
            run_blocking(state, move |s| {
                s.runtime.restart(
                    manifest_path.as_deref(),
                    StartOptions {
                        headless,
                        interactive_console: false,
                    },
                )
            })
            .await
        }
        IpcRequest::RuntimeLogs { tail } => {
            run_blocking(state, move |s| s.runtime.read_logs(tail)).await
        }
        IpcRequest::DevicesList => run_blocking(state, |s| s.adb.list_devices()).await,
        IpcRequest::AppsList {
            serial,
            third_party_only,
        } => {
            run_blocking(state, move |s| {
                valhalla::ValhallaManager::with_adb(s.adb.clone())
                    .list_packages(serial.as_deref(), third_party_only)
            })
            .await
        }
        IpcRequest::AppInfo { serial, package } => {
            run_blocking(state, move |s| {
                valhalla::ValhallaManager::with_adb(s.adb.clone())
                    .package_info(serial.as_deref(), &package)
            })
            .await
        }
        IpcRequest::AppInstall { serial, apk_path } => {
            run_blocking(state, move |s| {
                valhalla::ValhallaManager::with_adb(s.adb.clone())
                    .install_apk(serial.as_deref(), apk_path)
            })
            .await
        }
        IpcRequest::AppUninstall {
            serial,
            package,
            keep_data,
        } => {
            run_blocking(state, move |s| {
                valhalla::ValhallaManager::with_adb(s.adb.clone())
                    .uninstall_package(serial.as_deref(), &package, keep_data)
                    .map(|msg| serde_json::json!({ "package": package, "output": msg }))
            })
            .await
        }
        IpcRequest::AppLaunch { serial, package } => {
            run_blocking(state, move |s| {
                valhalla::ValhallaManager::with_adb(s.adb.clone())
                    .launch_app(serial.as_deref(), &package)
            })
            .await
        }
        IpcRequest::AppStop { serial, package } => {
            run_blocking(state, move |s| {
                valhalla::ValhallaManager::with_adb(s.adb.clone())
                    .force_stop(serial.as_deref(), &package)
                    .map(|()| serde_json::json!({ "package": package, "stopped": true }))
            })
            .await
        }
    };

    match result {
        Ok(value) => IpcResponseEnvelope {
            protocol_version: IPC_PROTOCOL_VERSION,
            request_id: envelope.request_id,
            ok: true,
            payload: Some(value),
            error: None,
        },
        Err(err) => IpcResponseEnvelope::failure(envelope.request_id, err.to_string()),
    }
}

#[cfg(all(test, windows))]
mod tests {
    use std::time::Duration;

    use bifrost_bridge::{token_file_path, AdbClient, DaemonHealthReport, HeimdallClient};
    use bifrost_core::{BifrostConfig, BifrostPaths};
    use bifrost_runtime::MidgardRuntime;
    use tempfile::TempDir;

    use super::*;

    #[tokio::test]
    async fn authenticated_named_pipe_enforces_token_and_handles_shutdown() {
        let tmp = TempDir::new().unwrap();
        let paths = BifrostPaths::from_root(tmp.path().join("bifrost"));
        let data_dir = paths.data_dir.clone();
        let pipe_name = format!(r"\\.\pipe\bifrost-heimdall-test-{}", std::process::id());

        let runtime = MidgardRuntime::with_default_backend(BifrostConfig::default(), paths);
        let state = Arc::new(SupervisorState::new(
            runtime,
            AdbClient::new(),
            pipe_name.clone(),
        ));
        let (shutdown_tx, shutdown_rx) = watch::channel(false);

        let server_data = data_dir.clone();
        let server_task = tokio::spawn(async move {
            serve_named_pipe(state, server_data, shutdown_tx, shutdown_rx)
                .await
                .unwrap();
        });

        // Wait briefly for server to create token file and listen on Named Pipe.
        let tok_file = token_file_path(&data_dir);
        for _ in 0..40 {
            if tok_file.exists() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        assert!(tok_file.exists(), "Heimdall should create auth token file");

        // 1. Valid client succeeds on DaemonHealth.
        let client = HeimdallClient::new(&pipe_name, &data_dir);
        let health: DaemonHealthReport = client.health().await.unwrap();
        assert_eq!(health.pipe_name, pipe_name);
        assert_eq!(health.daemon_pid, std::process::id());

        // 2. Client presenting a forged token is rejected.
        let bad_dir = TempDir::new().unwrap();
        fs::write(token_file_path(bad_dir.path()), "forged-invalid-token").unwrap();
        let bad_client = HeimdallClient::new(&pipe_name, bad_dir.path());
        let err = bad_client.health().await.unwrap_err();
        assert!(err.to_string().contains("authentication failed"));

        // 3. A second Heimdall on the same pipe must fail without replacing the token.
        let token_before = fs::read_to_string(&tok_file).unwrap();
        let dup_runtime = MidgardRuntime::with_default_backend(
            BifrostConfig::default(),
            BifrostPaths::from_root(tmp.path().join("bifrost")),
        );
        let dup_state = Arc::new(SupervisorState::new(
            dup_runtime,
            AdbClient::new(),
            pipe_name.clone(),
        ));
        let (dup_tx, dup_rx) = watch::channel(false);
        let dup_err = serve_named_pipe(dup_state, data_dir.clone(), dup_tx, dup_rx)
            .await
            .unwrap_err();
        assert!(dup_err.to_string().contains("another Heimdall"));
        assert_eq!(fs::read_to_string(&tok_file).unwrap(), token_before);
        client.health().await.unwrap();

        // 4. Valid client triggers graceful shutdown and cleans up token file.
        let shutdown_resp: serde_json::Value =
            client.send(IpcRequest::DaemonShutdown).await.unwrap();
        assert_eq!(shutdown_resp["shutting_down"], true);

        server_task.await.unwrap();
        assert!(
            !tok_file.exists(),
            "Heimdall should remove auth token on exit"
        );
    }
}
