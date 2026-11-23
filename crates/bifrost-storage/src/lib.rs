//! Verified Midgard runtime image storage, Ed25519 signature verification,
//! image version registry, upgrade compatibility, persistent virtual disks,
//! and backup/recovery management.

#![forbid(unsafe_code)]

pub mod disk;
pub mod manifest;
pub mod recovery;
pub mod registry;
pub mod signature;
pub mod trust;
pub mod verify;

pub use disk::{ensure_userdata_disk, wipe_userdata_disk, UserdataDiskInfo};
pub use manifest::{
    ArtifactSpec, BootMode, DiskArtifactSpec, DiskFormat, DiskMedia, MidgardImageManifest,
    CURRENT_MANIFEST_SCHEMA,
};
pub use recovery::{backup_userdata, restore_userdata, BackupMetadata};
pub use registry::{
    check_upgrade_compatibility, ImageRegistry, InstalledImageRecord, UpgradeCompatibility,
};
pub use signature::{sign_bytes_ed25519, verify_bytes_ed25519, verify_manifest_signature_file};
pub use trust::{
    detached_signature_path, evaluate_manifest_trust, evaluate_payload_trust, ImageTrust,
    DETACHED_SIGNATURE_SUFFIX,
};
pub use verify::{
    sha256_file, verify_manifest_file, VerifiedBootMedia, VerifiedBundle, VerifiedDisk,
};
