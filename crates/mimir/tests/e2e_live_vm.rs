//! End-to-end test against a real Midgard VM (QEMU-WHPX + LineageOS x86_64).
//!
//! Ignored by default: it needs WHPX, QEMU, `adb.exe` and a staged image bundle, and it
//! boots Android for a few minutes. Run it explicitly with:
//!
//! ```powershell
//! $env:BIFROST_E2E_MANIFEST = "$env:LOCALAPPDATA\bifrost\images\lineage-24-virtio-x86_64\manifest.json"
//! cargo test -p mimir --test e2e_live_vm -- --ignored --nocapture
//! ```
//!
//! The test drives the real `bifrost` CLI with a throwaway config and data directory, so
//! it never touches the user's runtime state and can run next to the user's own VM:
//! writable bundle disks are staged per instance, and it uses its own loopback ADB port
//! and QMP socket.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use bifrost_bridge::{AdbClient, ANDROID_SET_CLIPBOARD_FROM_STDIN};
use serde_json::Value;

const BOOT_TIMEOUT: Duration = Duration::from_secs(420);
const STOP_TIMEOUT: Duration = Duration::from_secs(120);
/// Loopback ADB port distinct from the default so a stray user VM cannot be hit.
const E2E_ADB_PORT: u16 = 5597;

fn bifrost(config: &Path, args: &[&str]) -> (bool, String) {
    // Capture through a file, not a pipe: Windows children inherit inheritable handles,
    // so a detached QEMU spawned by `runtime start` would keep a stdout pipe open and
    // `Command::output()` would wait for EOF forever.
    let out_log = tempfile::NamedTempFile::new().expect("CLI stdout file");
    let err_log = tempfile::NamedTempFile::new().expect("CLI stderr file");
    let out_file = out_log.reopen().expect("reopen stdout file");
    let err_file = err_log.reopen().expect("reopen stderr file");
    let status = Command::new(env!("CARGO_BIN_EXE_bifrost"))
        .arg("--config")
        .arg(config)
        .arg("--json")
        .args(args)
        .stdin(Stdio::null())
        .stdout(out_file)
        .stderr(err_file)
        .status()
        .expect("spawn bifrost CLI");
    let mut text = std::fs::read_to_string(out_log.path()).unwrap_or_default();
    if !status.success() {
        text.push_str(&std::fs::read_to_string(err_log.path()).unwrap_or_default());
    }
    (status.success(), text)
}

fn runtime_state(config: &Path) -> Option<String> {
    let (ok, text) = bifrost(config, &["status"]);
    if !ok {
        return None;
    }
    let json: Value = serde_json::from_str(text.trim()).ok()?;
    json["runtime_state"].as_str().map(str::to_string)
}

/// Stops the VM even when an assertion fails, so a failed run never leaks QEMU.
struct VmGuard {
    config: PathBuf,
}

impl Drop for VmGuard {
    fn drop(&mut self) {
        let (ok, _) = bifrost(&self.config, &["runtime", "stop"]);
        if !ok {
            let _ = bifrost(&self.config, &["runtime", "stop", "--force"]);
        }
    }
}

#[test]
#[ignore = "boots a real Android VM; set BIFROST_E2E_MANIFEST and run with --ignored"]
fn live_vm_boot_clipboard_and_shutdown() {
    let Some(manifest) = std::env::var_os("BIFROST_E2E_MANIFEST").map(PathBuf::from) else {
        eprintln!("BIFROST_E2E_MANIFEST is not set; skipping live VM test");
        return;
    };
    assert!(
        manifest.is_file(),
        "manifest not found: {}",
        manifest.display()
    );

    let work = tempfile::tempdir().expect("temp dir");
    let data_dir = work.path().join("data");
    let config = work.path().join("bifrost.toml");
    std::fs::write(
        &config,
        format!(
            "schema_version = 1\n\n[runtime]\ncpu_cores = 4\nmemory_mb = 4096\ndata_dir = '{}'\n\n[network]\nadb_bind_address = \"127.0.0.1\"\nadb_port = {E2E_ADB_PORT}\n\n[desktop]\nauto_sync_shortcuts = false\nforward_notifications = false\nauto_sync_clipboard = false\n",
            data_dir.display()
        ),
    )
    .expect("write e2e config");

    // 1. Boot headless and wait for Android `sys.boot_completed=1`.
    let manifest_arg = manifest.to_string_lossy().into_owned();
    let (ok, out) = bifrost(
        &config,
        &["runtime", "start", "--image", &manifest_arg, "--headless"],
    );
    assert!(ok, "runtime start failed:\n{out}");
    let _guard = VmGuard {
        config: config.clone(),
    };
    let record: Value = serde_json::from_str(out.trim()).expect("start JSON");
    let endpoint = record["adb_endpoint"]
        .as_str()
        .expect("adb endpoint")
        .to_string();
    assert!(
        endpoint.starts_with("127.0.0.1:"),
        "ADB must stay on loopback, got {endpoint}"
    );
    assert!(
        record["qmp_endpoint"]
            .as_str()
            .is_some_and(|q| q.starts_with("unix:") && q.ends_with(".sock")),
        "QMP must use a per-user AF_UNIX socket: {record}"
    );

    let started = Instant::now();
    loop {
        match runtime_state(&config).as_deref() {
            Some("running") => break,
            Some("failed") => panic!("Midgard entered 'failed' during boot"),
            _ if started.elapsed() > BOOT_TIMEOUT => {
                panic!("Android did not finish booting within {BOOT_TIMEOUT:?}")
            }
            _ => std::thread::sleep(Duration::from_secs(5)),
        }
    }
    eprintln!("booted in {:?}", started.elapsed());

    // 2. Guest sanity: x86_64 ABI and no ARM translator bundled.
    let adb = AdbClient::new();
    let _ = adb.connect_loopback(&endpoint);
    let abi = adb
        .run_adb(&["-s", &endpoint, "shell", "getprop", "ro.product.cpu.abi"])
        .expect("getprop abi");
    assert_eq!(abi.trim(), "x86_64");

    // 3. Clipboard stdin path: hostile text must arrive verbatim and never execute.
    let marker = "/data/local/tmp/bifrost-e2e-pwned";
    let hostile = format!(
        "$(touch {marker}) `touch {marker}` ; touch {marker} 'single' \"double\" \\ ñ ✓\nsecond line"
    );
    adb.run_adb_with_stdin(
        &["-s", &endpoint, "shell", ANDROID_SET_CLIPBOARD_FROM_STDIN],
        hostile.as_bytes(),
    )
    .expect("clipboard set via stdin");
    let probe = adb
        .run_adb(&[
            "-s",
            &endpoint,
            "shell",
            &format!("test -e {marker} && echo PWNED || echo clean"),
        ])
        .expect("probe marker");
    assert_eq!(probe.trim(), "clean", "clipboard text was executed by sh");

    // 4. Graceful shutdown over the QMP named pipe.
    let (ok, out) = bifrost(&config, &["runtime", "stop"]);
    assert!(ok, "runtime stop failed:\n{out}");
    let stopping = Instant::now();
    while runtime_state(&config).as_deref() != Some("stopped") {
        assert!(
            stopping.elapsed() < STOP_TIMEOUT,
            "VM did not stop within {STOP_TIMEOUT:?}"
        );
        std::thread::sleep(Duration::from_secs(2));
    }
}
