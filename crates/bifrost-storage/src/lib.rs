//! Verified Midgard runtime image storage and persistent virtual disk management.

#![forbid(unsafe_code)]

pub mod disk;
pub mod manifest;
pub mod verify;

pub use disk::{ensure_userdata_disk, wipe_userdata_disk, UserdataDiskInfo};
pub use manifest::{
    ArtifactSpec, BootMode, DiskArtifactSpec, DiskFormat, DiskMedia, MidgardImageManifest,
    CURRENT_MANIFEST_SCHEMA,
};
pub use verify::{
    sha256_file, verify_manifest_file, VerifiedBootMedia, VerifiedBundle, VerifiedDisk,
};
