use std::env;
use std::fs;
use std::path::PathBuf;

fn main() {
    let out_dir = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR missing"));
    let src_ico = PathBuf::from("icons/icon.ico");
    let dst_ico = out_dir.join("bifrost-icon.ico");

    println!("cargo:rerun-if-changed=icons/icon.ico");
    println!("cargo:rerun-if-changed=icons/icon.png");

    // Copy icon into OUT_DIR so GNU windres receives an ASCII path even when
    // the repository directory contains non-ASCII characters (e.g., `BIFRÖST`).
    let windows_attrs = if src_ico.is_file() && fs::copy(&src_ico, &dst_ico).is_ok() {
        tauri_build::WindowsAttributes::new().window_icon_path(dst_ico)
    } else {
        tauri_build::WindowsAttributes::new()
    };

    tauri_build::try_build(tauri_build::Attributes::new().windows_attributes(windows_attrs))
        .expect("failed to run tauri-build");
}
