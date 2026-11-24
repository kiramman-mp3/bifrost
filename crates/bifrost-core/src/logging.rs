//! Structured logging initialization using `tracing`.

use tracing_subscriber::EnvFilter;

use crate::config::LoggingConfig;

/// Initializes global structured logging respecting `RUST_LOG` or `LoggingConfig`.
///
/// Logs always go to **stderr**: stdout carries command output, which `--json` callers
/// parse as a single JSON document.
///
/// Safe to call multiple times in tests; subsequent calls are ignored if a subscriber
/// is already installed.
pub fn init_logging(config: &LoggingConfig) {
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new(config.level.as_directive()));

    if config.json {
        let _ = tracing_subscriber::fmt()
            .with_env_filter(filter)
            .with_target(true)
            .with_writer(std::io::stderr)
            .json()
            .try_init();
    } else {
        let _ = tracing_subscriber::fmt()
            .with_env_filter(filter)
            .with_target(true)
            .with_writer(std::io::stderr)
            .compact()
            .try_init();
    }
}
