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
}

impl BifrostError {
    /// Returns true if this error represents an unimplemented/unsupported feature.
    #[must_use]
    pub fn is_unsupported(&self) -> bool {
        matches!(self, Self::Unsupported { .. })
    }
}
