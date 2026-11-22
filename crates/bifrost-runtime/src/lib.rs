//! Midgard Android runtime lifecycle orchestration for BIFRÖST.

#![forbid(unsafe_code)]

pub mod manager;
pub mod state_store;

pub use manager::{MidgardRuntime, RuntimeLogs, StartOptions};
pub use state_store::RuntimeRecord;
