//! Valhalla — Android application discovery, APK inspection, installation, and lifecycle management.

#![forbid(unsafe_code)]

pub mod apk;
pub mod file_assoc;
pub mod manager;
pub mod package;
pub mod shortcuts;

pub use apk::{
    extract_apk_icon_png, extract_manifest_metadata, extract_manifest_package,
    extract_required_gles_version, format_gles_version, inspect_apk, inspect_package_preview,
    is_apkm_bundle, validate_package_file_path, wrap_png_as_ico, AbiCompatibility, ApkInspection,
};
pub use file_assoc::{
    build_apk_open_command, build_heimdall_autostart_command, query_heimdall_autostart,
    register_apk_handler, set_heimdall_autostart, unregister_apk_handler, ApkAssociationStatus,
    HeimdallAutostartStatus, BIFROST_APK_PROG_ID, BIFROST_HEIMDALL_RUN_VALUE,
};
pub use manager::{InstallOutcome, LaunchOutcome, ValhallaManager};
pub use package::{
    parse_dumpsys_package, parse_pm_list_packages, parse_resolved_activity, validate_package_name,
    InstalledPackage, PackageDetails,
};
pub use shortcuts::{derive_display_name, sanitize_shortcut_name, ShortcutManager, ShortcutRecord};
