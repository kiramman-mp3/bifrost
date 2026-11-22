//! Valhalla — Android application discovery, APK inspection, installation, and lifecycle management.

#![forbid(unsafe_code)]

pub mod apk;
pub mod manager;
pub mod package;
pub mod shortcuts;

pub use apk::{
    extract_apk_icon_png, extract_manifest_package, inspect_apk, wrap_png_as_ico, AbiCompatibility,
    ApkInspection,
};
pub use manager::{InstallOutcome, LaunchOutcome, ValhallaManager};
pub use package::{
    parse_dumpsys_package, parse_pm_list_packages, parse_resolved_activity, validate_package_name,
    InstalledPackage, PackageDetails,
};
pub use shortcuts::{derive_display_name, sanitize_shortcut_name, ShortcutManager, ShortcutRecord};
