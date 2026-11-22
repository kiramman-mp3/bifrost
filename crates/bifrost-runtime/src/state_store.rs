//! Persisted runtime state tracking across CLI invocations (pre-Heimdall Milestone 3).

use std::fs;
use std::path::{Path, PathBuf};

use bifrost_core::{BifrostError, Result, RuntimeState};
use serde::{Deserialize, Serialize};

/// Persisted snapshot of the active or most recent Midgard VM instance.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeRecord {
    pub instance_id: String,
    pub state: RuntimeState,
    pub pid: Option<u32>,
    pub backend: Option<String>,
    pub image_id: Option<String>,
    pub android_version: Option<String>,
    pub manifest_path: Option<PathBuf>,
    pub userdata_disk: Option<PathBuf>,
    pub adb_endpoint: Option<String>,
    pub qmp_endpoint: Option<String>,
    pub serial_log_path: Option<PathBuf>,
    pub vmm_stderr_path: Option<PathBuf>,
    pub note: Option<String>,
}

impl RuntimeRecord {
    /// Default record when no Midgard instance has been started.
    #[must_use]
    pub fn stopped(instance_id: impl Into<String>) -> Self {
        Self {
            instance_id: instance_id.into(),
            state: RuntimeState::Stopped,
            pid: None,
            backend: None,
            image_id: None,
            android_version: None,
            manifest_path: None,
            userdata_disk: None,
            adb_endpoint: None,
            qmp_endpoint: None,
            serial_log_path: None,
            vmm_stderr_path: None,
            note: None,
        }
    }
}

/// Loads `runtime-state.json` if present, or returns a clean `Stopped` record.
pub fn load_record(path: &Path, instance_id: &str) -> Result<RuntimeRecord> {
    if !path.exists() {
        return Ok(RuntimeRecord::stopped(instance_id));
    }
    let raw = fs::read_to_string(path).map_err(|source| BifrostError::Io {
        context: format!("reading runtime state '{}'", path.display()),
        source,
    })?;
    serde_json::from_str(&raw).map_err(|err| BifrostError::ConfigParse(err.to_string()))
}

/// Atomically writes `runtime-state.json` to disk.
pub fn save_record(path: &Path, record: &RuntimeRecord) -> Result<()> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent).map_err(|source| BifrostError::Io {
                context: format!("creating runtime state directory '{}'", parent.display()),
                source,
            })?;
        }
    }
    let serialized = serde_json::to_string_pretty(record)
        .map_err(|err| BifrostError::ConfigSerialize(err.to_string()))?;
    let tmp_path = path.with_extension("json.tmp");
    fs::write(&tmp_path, serialized).map_err(|source| BifrostError::Io {
        context: format!("writing temporary runtime state '{}'", tmp_path.display()),
        source,
    })?;
    fs::rename(&tmp_path, path).map_err(|source| BifrostError::Io {
        context: format!("persisting runtime state '{}'", path.display()),
        source,
    })?;
    Ok(())
}
