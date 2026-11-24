//! Executable lookup that also sees tools installed after this process started.
//!
//! A running process keeps the `PATH` it was launched with, but installers such as
//! `winget` update the persisted user/machine `PATH` in the registry. Searching the
//! persisted entries too lets BIFRÖST find QEMU, `adb` or `scrcpy` right after the
//! first-run setup installs them, without restarting the app.

use std::ffi::OsString;
use std::path::PathBuf;

/// Returns the first `dir/binary_name` that is a file, searching the process `PATH`
/// first and then the persisted user and machine `PATH` entries (Windows).
#[must_use]
pub fn find_on_path(binary_name: &str) -> Option<PathBuf> {
    search_dirs()
        .into_iter()
        .map(|dir| dir.join(binary_name))
        .find(|candidate| candidate.is_file())
}

/// Directories searched by [`find_on_path`], in order and without duplicates.
#[must_use]
pub fn search_dirs() -> Vec<PathBuf> {
    let process = std::env::var_os("PATH").unwrap_or_default();
    merge_dirs(&process, &persisted_path_values())
}

fn merge_dirs(process: &OsString, persisted: &[String]) -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = Vec::new();
    let persisted = persisted
        .iter()
        .map(|raw| OsString::from(expand_env_refs(raw)));
    for value in std::iter::once(process.clone()).chain(persisted) {
        for dir in std::env::split_paths(&value) {
            if dir.as_os_str().is_empty() {
                continue;
            }
            let seen = dirs.iter().any(|known| {
                known
                    .to_string_lossy()
                    .eq_ignore_ascii_case(&dir.to_string_lossy())
            });
            if !seen {
                dirs.push(dir);
            }
        }
    }
    dirs
}

/// Expands `%NAME%` references (as stored in `REG_EXPAND_SZ` values) from the current
/// environment; unknown names are left untouched.
fn expand_env_refs(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut rest = raw;
    while let Some(start) = rest.find('%') {
        out.push_str(&rest[..start]);
        let after = &rest[start + 1..];
        match after.find('%') {
            Some(end) if end > 0 => {
                let name = &after[..end];
                match std::env::var(name) {
                    Ok(value) => out.push_str(&value),
                    Err(_) => {
                        out.push('%');
                        out.push_str(name);
                        out.push('%');
                    }
                }
                rest = &after[end + 1..];
            }
            _ => {
                out.push('%');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

#[cfg(windows)]
fn persisted_path_values() -> Vec<String> {
    use winreg::enums::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE};
    use winreg::RegKey;

    let read = |root, key: &str| -> Option<String> {
        RegKey::predef(root)
            .open_subkey(key)
            .ok()?
            .get_value::<String, _>("Path")
            .ok()
    };
    [
        read(HKEY_CURRENT_USER, "Environment"),
        read(
            HKEY_LOCAL_MACHINE,
            r"SYSTEM\CurrentControlSet\Control\Session Manager\Environment",
        ),
    ]
    .into_iter()
    .flatten()
    .collect()
}

#[cfg(not(windows))]
fn persisted_path_values() -> Vec<String> {
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expands_known_variables_and_keeps_unknown_ones() {
        let var = if cfg!(windows) { "TEMP" } else { "TMPDIR" };
        if std::env::var(var).is_ok() {
            assert_eq!(
                expand_env_refs(&format!("%{var}%\\bin")),
                format!("{}\\bin", std::env::var(var).unwrap())
            );
        }
        assert_eq!(
            expand_env_refs("%BIFROST_SURELY_UNSET_VAR%\\x"),
            "%BIFROST_SURELY_UNSET_VAR%\\x"
        );
        assert_eq!(expand_env_refs("50% done"), "50% done");
    }

    #[cfg(windows)]
    #[test]
    fn reads_the_persisted_path_from_the_registry() {
        // Every Windows install has a machine-wide PATH (System32 at least).
        let values = persisted_path_values();
        assert!(values
            .iter()
            .any(|v| v.to_ascii_lowercase().contains("system32")));
        assert!(search_dirs().len() >= values.len());
    }

    #[test]
    fn process_path_comes_first_and_duplicates_are_dropped() {
        let a = std::env::temp_dir().join("bifrost-a");
        let b = std::env::temp_dir().join("bifrost-b");
        let process = std::env::join_paths([&a]).unwrap();
        let persisted = vec![std::env::join_paths([&a, &b])
            .unwrap()
            .to_string_lossy()
            .into_owned()];
        assert_eq!(merge_dirs(&process, &persisted), vec![a, b]);
    }

    #[test]
    fn finds_a_binary_on_a_persisted_only_directory() {
        let dir = tempfile_dir();
        let exe = dir.join("bifrost-probe-tool.exe");
        std::fs::write(&exe, b"").unwrap();
        let dirs = merge_dirs(&OsString::new(), &[dir.display().to_string()]);
        let found = dirs
            .into_iter()
            .map(|d| d.join("bifrost-probe-tool.exe"))
            .find(|p| p.is_file());
        assert_eq!(found, Some(exe));
        std::fs::remove_dir_all(dir).unwrap();
    }

    fn tempfile_dir() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("bifrost-search-path-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }
}
