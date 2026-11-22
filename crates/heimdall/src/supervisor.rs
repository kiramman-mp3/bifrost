//! Heimdall runtime supervision and periodic health monitoring loop.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use bifrost_bridge::{AdbClient, DaemonHealthReport};
use bifrost_core::config::DesktopConfig;
use bifrost_core::RuntimeState;
use bifrost_runtime::MidgardRuntime;
use tokio::sync::{watch, Mutex, RwLock};
use tracing::{debug, warn};

use crate::app_sync::AppShortcutSyncer;
use crate::desktop_bridge::DesktopBridgeWorker;

/// Shared live supervision state maintained by Heimdall.
pub struct SupervisorState {
    pub started_at: Instant,
    pub pipe_name: String,
    pub runtime: MidgardRuntime,
    pub adb: AdbClient,
    pub data_dir: PathBuf,
    pub desktop: DesktopConfig,
    pub health: RwLock<DaemonHealthReport>,
    pub app_sync: Mutex<AppShortcutSyncer>,
    pub desktop_bridge: Mutex<DesktopBridgeWorker>,
}

impl SupervisorState {
    /// Initializes supervision state with default desktop settings (used by IPC tests).
    #[cfg(test)]
    pub fn new(runtime: MidgardRuntime, adb: AdbClient, pipe_name: String) -> Self {
        let data_dir = std::env::temp_dir().join("bifrost-heimdall-default");
        Self::with_desktop_config(
            runtime,
            adb,
            pipe_name,
            data_dir,
            DesktopConfig {
                auto_sync_shortcuts: false,
                forward_notifications: false,
                auto_sync_clipboard: false,
            },
        )
    }

    /// Initializes supervision state with explicit data directory and desktop automation config.
    pub fn with_desktop_config(
        runtime: MidgardRuntime,
        adb: AdbClient,
        pipe_name: String,
        data_dir: PathBuf,
        desktop: DesktopConfig,
    ) -> Self {
        let initial_state = runtime
            .status()
            .map(|r| r.state)
            .unwrap_or(RuntimeState::Stopped);
        let initial_pid = runtime.status().ok().and_then(|r| r.pid);
        let adb_available = adb.locate_adb().is_ok();

        let health = DaemonHealthReport {
            daemon_pid: std::process::id(),
            uptime_secs: 0,
            runtime_state: initial_state,
            vm_pid: initial_pid,
            adb_available,
            adb_device_count: 0,
            guest_health: None,
            health_checks_completed: 0,
            pipe_name: pipe_name.clone(),
        };

        Self {
            started_at: Instant::now(),
            pipe_name,
            runtime,
            adb,
            data_dir,
            desktop,
            health: RwLock::new(health),
            app_sync: Mutex::new(AppShortcutSyncer::default()),
            desktop_bridge: Mutex::new(DesktopBridgeWorker::default()),
        }
    }

    /// Executes a single health probe updating runtime state, PID liveness, and ADB devices.
    pub async fn refresh_health(&self) -> DaemonHealthReport {
        let uptime_secs = self.started_at.elapsed().as_secs();
        let (runtime_state, vm_pid, adb_endpoint) = match self.runtime.status() {
            Ok(rec) => (rec.state, rec.pid, rec.adb_endpoint),
            Err(err) => {
                warn!(error = %err, "failed to read runtime status during health check");
                (RuntimeState::Failed, None, None)
            }
        };

        let adb_available = self.adb.locate_adb().is_ok();
        let devices = if adb_available {
            self.adb.list_devices().unwrap_or_default()
        } else {
            Vec::new()
        };

        // If Midgard is active and its loopback ADB endpoint appears online, probe Android guest health.
        let guest_health = if runtime_state.is_active() {
            if let Some(ref endpoint) = adb_endpoint {
                if devices
                    .iter()
                    .any(|d| d.serial == *endpoint && d.state == "device")
                {
                    self.adb.probe_guest_health(endpoint).ok()
                } else {
                    None
                }
            } else {
                None
            }
        } else {
            None
        };

        let effective_state = if runtime_state == RuntimeState::Booting
            && guest_health.as_ref().is_some_and(|g| g.boot_completed)
        {
            RuntimeState::Running
        } else {
            runtime_state
        };

        let mut guard = self.health.write().await;
        guard.uptime_secs = uptime_secs;
        guard.runtime_state = effective_state;
        guard.vm_pid = vm_pid;
        guard.adb_available = adb_available;
        guard.adb_device_count = devices.len();
        guard.guest_health = guest_health;
        guard.health_checks_completed = guard.health_checks_completed.saturating_add(1);
        let snapshot = guard.clone();
        drop(guard);

        // Background desktop automation when Android is fully booted and reachable over ADB.
        if effective_state == RuntimeState::Running {
            if let Some(ref endpoint) = adb_endpoint {
                if self.desktop.auto_sync_shortcuts {
                    let mut syncer = self.app_sync.lock().await;
                    syncer.maybe_sync(&self.adb, endpoint, &self.data_dir);
                }
                if self.desktop.forward_notifications || self.desktop.auto_sync_clipboard {
                    let mut bridge = self.desktop_bridge.lock().await;
                    if self.desktop.forward_notifications {
                        bridge.sync_notifications(&self.adb, endpoint);
                    }
                    if self.desktop.auto_sync_clipboard {
                        bridge.sync_clipboard(&self.adb, endpoint);
                    }
                }
            }
        }

        debug!(
            state = %snapshot.runtime_state,
            checks = snapshot.health_checks_completed,
            adb_devices = snapshot.adb_device_count,
            "completed Heimdall health cycle"
        );
        snapshot
    }
}

/// Runs the periodic background health monitor until `shutdown_rx` signals termination.
pub async fn run_health_monitor(
    state: Arc<SupervisorState>,
    interval: Duration,
    mut shutdown_rx: watch::Receiver<bool>,
) {
    let mut ticker = tokio::time::interval(interval);
    loop {
        tokio::select! {
            _ = ticker.tick() => {
                let _ = state.refresh_health().await;
            }
            changed = shutdown_rx.changed() => {
                if changed.is_err() || *shutdown_rx.borrow() {
                    break;
                }
            }
        }
    }
}
