//! Heimdall runtime supervision and periodic health monitoring loop.

use std::sync::Arc;
use std::time::{Duration, Instant};

use bifrost_bridge::{AdbClient, DaemonHealthReport};
use bifrost_core::RuntimeState;
use bifrost_runtime::MidgardRuntime;
use tokio::sync::{watch, RwLock};
use tracing::{debug, warn};

/// Shared live supervision state maintained by Heimdall.
pub struct SupervisorState {
    pub started_at: Instant,
    pub pipe_name: String,
    pub runtime: MidgardRuntime,
    pub adb: AdbClient,
    pub health: RwLock<DaemonHealthReport>,
}

impl SupervisorState {
    /// Initializes supervision state and runs an immediate baseline probe.
    pub fn new(runtime: MidgardRuntime, adb: AdbClient, pipe_name: String) -> Self {
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
            health: RwLock::new(health),
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
