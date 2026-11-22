//! Deterministic runtime lifecycle state machine for Midgard and Heimdall.

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::error::{BifrostError, Result};

/// Lifecycle states of a Midgard Android runtime instance.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeState {
    /// No virtual machine process is running.
    Stopped,
    /// Host resources and verified images are being prepared; VM is spawning.
    Starting,
    /// VM is active and waiting for Android `sys.boot_completed` / health readiness.
    Booting,
    /// Android runtime is booted, healthy, and ready for Bridge and Valhalla operations.
    Running,
    /// Runtime is up, but one or more non-fatal subsystems (e.g., graphics/bridge) are impaired.
    Degraded,
    /// Graceful shutdown has been requested and is in progress.
    Stopping,
    /// Runtime exited unexpectedly or failed to boot; recovery or explicit reset required.
    Failed,
}

impl RuntimeState {
    /// Returns whether transitioning from `self` to `next` is valid.
    #[must_use]
    pub const fn can_transition_to(self, next: Self) -> bool {
        matches!(
            (self, next),
            (Self::Stopped, Self::Starting)
                | (Self::Starting, Self::Booting)
                | (Self::Starting, Self::Stopping)
                | (Self::Starting, Self::Failed)
                | (Self::Booting, Self::Running)
                | (Self::Booting, Self::Degraded)
                | (Self::Booting, Self::Stopping)
                | (Self::Booting, Self::Failed)
                | (Self::Running, Self::Degraded)
                | (Self::Running, Self::Stopping)
                | (Self::Running, Self::Failed)
                | (Self::Degraded, Self::Running)
                | (Self::Degraded, Self::Stopping)
                | (Self::Degraded, Self::Failed)
                | (Self::Stopping, Self::Stopped)
                | (Self::Stopping, Self::Failed)
                | (Self::Failed, Self::Stopped)
                | (Self::Failed, Self::Starting)
        )
    }

    /// Returns true if the runtime currently holds active VM resources.
    #[must_use]
    pub const fn is_active(self) -> bool {
        matches!(
            self,
            Self::Starting | Self::Booting | Self::Running | Self::Degraded | Self::Stopping
        )
    }
}

impl fmt::Display for RuntimeState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let label = match self {
            Self::Stopped => "stopped",
            Self::Starting => "starting",
            Self::Booting => "booting",
            Self::Running => "running",
            Self::Degraded => "degraded",
            Self::Stopping => "stopping",
            Self::Failed => "failed",
        };
        f.write_str(label)
    }
}

/// Stateful controller enforcing valid runtime lifecycle transitions.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeStateMachine {
    current: RuntimeState,
    last_reason: Option<String>,
}

impl Default for RuntimeStateMachine {
    fn default() -> Self {
        Self::new()
    }
}

impl RuntimeStateMachine {
    /// Creates a new state machine initialized to [`RuntimeState::Stopped`].
    #[must_use]
    pub const fn new() -> Self {
        Self {
            current: RuntimeState::Stopped,
            last_reason: None,
        }
    }

    /// Returns the current lifecycle state.
    #[must_use]
    pub const fn state(&self) -> RuntimeState {
        self.current
    }

    /// Returns the diagnostic reason associated with the most recent transition, if any.
    #[must_use]
    pub fn last_reason(&self) -> Option<&str> {
        self.last_reason.as_deref()
    }

    /// Transitions to `next` if permitted by the lifecycle rules.
    pub fn transition_to(
        &mut self,
        next: RuntimeState,
        reason: Option<impl Into<String>>,
    ) -> Result<()> {
        if !self.current.can_transition_to(next) {
            return Err(BifrostError::InvalidStateTransition {
                from: self.current,
                to: next,
            });
        }
        self.current = next;
        self.last_reason = reason.map(Into::into);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normal_startup_and_shutdown_lifecycle_succeeds() {
        let mut sm = RuntimeStateMachine::new();
        assert_eq!(sm.state(), RuntimeState::Stopped);
        assert!(!sm.state().is_active());

        sm.transition_to(RuntimeState::Starting, Some("user requested start"))
            .unwrap();
        assert!(sm.state().is_active());

        sm.transition_to(RuntimeState::Booting, Some("VM process spawned"))
            .unwrap();
        sm.transition_to(RuntimeState::Running, Some("sys.boot_completed=1"))
            .unwrap();
        assert_eq!(sm.last_reason(), Some("sys.boot_completed=1"));

        sm.transition_to(RuntimeState::Degraded, Some("bridge heartbeat delayed"))
            .unwrap();
        sm.transition_to(RuntimeState::Running, Some("bridge recovered"))
            .unwrap();
        sm.transition_to(RuntimeState::Stopping, Some("graceful shutdown"))
            .unwrap();
        sm.transition_to(RuntimeState::Stopped, None::<String>)
            .unwrap();
        assert_eq!(sm.state(), RuntimeState::Stopped);
    }

    #[test]
    fn invalid_transitions_are_rejected() {
        let mut sm = RuntimeStateMachine::new();
        let err = sm
            .transition_to(RuntimeState::Running, None::<String>)
            .unwrap_err();
        assert!(matches!(
            err,
            BifrostError::InvalidStateTransition {
                from: RuntimeState::Stopped,
                to: RuntimeState::Running
            }
        ));
        assert_eq!(sm.state(), RuntimeState::Stopped);
    }

    #[test]
    fn recovery_from_failed_state_is_allowed() {
        let mut sm = RuntimeStateMachine::new();
        sm.transition_to(RuntimeState::Starting, None::<String>)
            .unwrap();
        sm.transition_to(RuntimeState::Failed, Some("WHPX unavailable"))
            .unwrap();
        assert!(!sm.state().is_active());

        sm.transition_to(RuntimeState::Starting, Some("retry"))
            .unwrap();
        assert_eq!(sm.state(), RuntimeState::Starting);
    }
}
