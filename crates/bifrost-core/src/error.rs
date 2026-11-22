//! Structured error taxonomy for BIFRÖST core and host components.

use std::path::PathBuf;
use thiserror::Error;

use crate::state::RuntimeState;

/// Convenience result alias for BIFRÖST operations.
pub type Result<T> = std::result::Result<T, BifrostError>;

/// Core error type shared across BIFRÖST crates.
#[derive(Debug, Error)]
pub enum BifrostError {
    /// Failed to read or parse a configuration document.
    #[error("configuration parse error: {0}")]
    ConfigParse(String),

    /// Failed to serialize a configuration document.
    #[error("configuration serialization error: {0}")]
    ConfigSerialize(String),

    /// Configuration contains an out-of-range or insecure value.
    #[error("invalid configuration for '{field}': {reason}")]
    ConfigValidation { field: String, reason: String },

    /// Attempted an illegal runtime lifecycle state transition.
    #[error("invalid runtime state transition from '{from}' to '{to}'")]
    InvalidStateTransition {
        from: RuntimeState,
        to: RuntimeState,
    },

    /// Rejected a filesystem path that violates security boundaries (e.g., traversal).
    #[error("unsafe path '{path}': {reason}")]
    UnsafePath { path: PathBuf, reason: String },

    /// Standard I/O failure with contextual path or operation description.
    #[error("I/O error during {context}: {source}")]
    Io {
        context: String,
        #[source]
        source: std::io::Error,
    },

    /// Requested operation or command is not yet implemented.
    #[error("unsupported operation '{feature}' (status: {planned_milestone})")]
    Unsupported {
        feature: String,
        planned_milestone: String,
    },

    /// A target file already exists and overwrite was not explicitly confirmed.
    #[error("refusing to overwrite existing file '{path}' without explicit confirmation")]
    AlreadyExists { path: PathBuf },

    /// Cryptographic integrity check failed on a runtime image or update artifact.
    #[error(
        "integrity verification failed for '{path}': expected sha256 '{expected}', got '{actual}'"
    )]
    IntegrityMismatch {
        path: PathBuf,
        expected: String,
        actual: String,
    },

    /// A required external dependency (such as the VMM binary) is missing or unusable.
    #[error("hypervisor error: {0}")]
    Hypervisor(String),

    /// Runtime lifecycle operation failed (e.g., already running, not running, or missing image).
    #[error("runtime error: {0}")]
    Runtime(String),

    /// Destructive action requires explicit user confirmation.
    #[error("destructive operation '{operation}' requires explicit confirmation")]
    ConfirmationRequired { operation: String },

    /// Bridge or ADB communication error.
    #[error("bridge error: {0}")]
    Bridge(String),

    /// Authenticated local IPC transport or protocol error.
    #[error("IPC error: {0}")]
    Ipc(String),

    /// APK validation, ABI compatibility, or Android package management error.
    #[error("package error: {0}")]
    Package(String),

    /// APK contains only unsupported native ABIs (e.g., ARM64 without x86_64; ADR-0004).
    #[error("unsupported APK ABI for '{path}': detected [{detected_abis}] without x86_64 native libraries (ARM64 translation is unsupported)")]
    UnsupportedAbi {
        path: PathBuf,
        detected_abis: String,
    },
}

impl BifrostError {
    /// Returns true if this error represents an unimplemented/unsupported feature.
    #[must_use]
    pub fn is_unsupported(&self) -> bool {
        matches!(self, Self::Unsupported { .. })
    }
}
