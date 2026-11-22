//! Valhalla — Android application discovery, APK inspection, installation, and lifecycle management.

#![forbid(unsafe_code)]

pub mod apk;
pub mod manager;
pub mod package;

pub use apk::{inspect_apk, AbiCompatibility, ApkInspection};
pub use manager::ValhallaManager;
pub use package::{
    parse_dumpsys_package, parse_pm_list_packages, parse_resolved_activity, validate_package_name,
    InstalledPackage, PackageDetails,
};
