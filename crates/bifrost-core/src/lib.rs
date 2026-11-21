//! Shared architectural core for BIFRÖST.
//!
//! Provides deterministic runtime state transitions, validated configuration,
//! security-hardened path utilities, error taxonomy, capability metadata,
//! and structured logging initialization.

pub mod capabilities;
pub mod config;
pub mod error;
pub mod logging;
pub mod paths;
pub mod state;

pub use capabilities::{ComponentCapability, FeatureStatus, VersionInfo};
pub use config::{
    BifrostConfig, DisplayConfig, GraphicsMode, LogLevel, NetworkConfig, RuntimeConfig,
};
pub use error::{BifrostError, Result};
pub use paths::{
    resolve_within_root, validate_host_file_path, validate_relative_safe_path, BifrostPaths,
};
pub use state::{RuntimeState, RuntimeStateMachine};

/// Official product display name.
pub const PRODUCT_NAME: &str = "BIFRÖST";

/// Lowercase technical identifier used across paths, namespaces, and executables.
pub const TECHNICAL_ID: &str = "bifrost";

/// Primary product tagline.
pub const PRIMARY_TAGLINE: &str = "A Bridge Between Worlds.";

/// Crate / workspace semantic version.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
