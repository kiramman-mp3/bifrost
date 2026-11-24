//! Cross-process exclusive lock serializing Midgard VM lifecycle transitions.
//!
//! `bifrost` (Mimir), `bifrost-ui` and Heimdall each drive [`crate::MidgardRuntime`]
//! from their own process. Without a shared lock, two concurrent `start` calls can both
//! observe a stopped instance while hashing multi-GB images and then spawn two VMMs on
//! the same persistent `userdata` disk. The lock is held for the whole transition.
//!
//! On Windows the lock is a file opened with a zero share mode: any other open of the
//! same file fails with `ERROR_SHARING_VIOLATION` until the owning handle is closed. The
//! OS closes the handle when the owner exits or crashes, so a stale lock can never
//! outlive its process.

use std::fs::{self, File, OpenOptions};
use std::io;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use bifrost_core::{BifrostError, Result};

/// How often a waiting caller retries the lock.
const POLL_INTERVAL: Duration = Duration::from_millis(200);

/// Windows `ERROR_SHARING_VIOLATION`.
#[cfg(windows)]
const ERROR_SHARING_VIOLATION: i32 = 32;

/// Held while a lifecycle transition runs; released on drop.
#[derive(Debug)]
pub struct LifecycleLock {
    _file: File,
    path: PathBuf,
}

impl LifecycleLock {
    /// Acquires the lock at `path`, waiting up to `timeout` for another holder to finish.
    pub fn acquire(path: &Path, timeout: Duration) -> Result<Self> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|source| BifrostError::Io {
                context: format!("creating lifecycle lock directory '{}'", parent.display()),
                source,
            })?;
        }

        let deadline = Instant::now() + timeout;
        loop {
            match open_exclusive(path) {
                Ok(file) => {
                    return Ok(Self {
                        _file: file,
                        path: path.to_path_buf(),
                    })
                }
                Err(err) if is_held_elsewhere(&err) => {
                    if Instant::now() >= deadline {
                        return Err(BifrostError::Runtime(format!(
                            "another BIFRÖST process is still starting or stopping Midgard \
                             (lifecycle lock '{}' held for more than {}s)",
                            path.display(),
                            timeout.as_secs()
                        )));
                    }
                    std::thread::sleep(POLL_INTERVAL);
                }
                Err(source) => {
                    return Err(BifrostError::Io {
                        context: format!("opening lifecycle lock '{}'", path.display()),
                        source,
                    })
                }
            }
        }
    }

    /// Path of the lock file.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }
}

#[cfg(windows)]
fn open_exclusive(path: &Path) -> io::Result<File> {
    use std::os::windows::fs::OpenOptionsExt;

    OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .share_mode(0)
        .open(path)
}

#[cfg(windows)]
fn is_held_elsewhere(err: &io::Error) -> bool {
    err.raw_os_error() == Some(ERROR_SHARING_VIOLATION)
}

// Midgard runs on Windows hosts only (WHPX). Other targets compile for development and
// CI tooling; they open the file without cross-process exclusion.
#[cfg(not(windows))]
fn open_exclusive(path: &Path) -> io::Result<File> {
    OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(path)
}

#[cfg(not(windows))]
fn is_held_elsewhere(_err: &io::Error) -> bool {
    false
}

#[cfg(all(test, windows))]
mod tests {
    use tempfile::TempDir;

    use super::*;

    #[test]
    fn second_holder_waits_and_times_out_then_succeeds_after_release() {
        let tmp = TempDir::new().unwrap();
        let path = tmp
            .path()
            .join("instances")
            .join("default")
            .join("lifecycle.lock");

        let first = LifecycleLock::acquire(&path, Duration::from_secs(1)).unwrap();
        assert_eq!(first.path(), path);

        let err = LifecycleLock::acquire(&path, Duration::from_millis(300)).unwrap_err();
        assert!(err.to_string().contains("lifecycle lock"));

        drop(first);
        LifecycleLock::acquire(&path, Duration::from_millis(300)).unwrap();
    }
}
