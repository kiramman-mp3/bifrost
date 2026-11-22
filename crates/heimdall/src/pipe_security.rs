//! Windows Named Pipe owner-only DACL builder using SDDL (`D:(A;;GA;;;OW)`).
//!
//! Restricts pipe access exclusively to the creating user account while keeping
//! all Win32 security descriptor FFI isolated and RAII-freed on drop.

#[cfg(windows)]
use std::ffi::c_void;
#[cfg(windows)]
use std::mem::size_of;

use bifrost_core::{BifrostError, Result};

/// RAII wrapper around a Windows `SECURITY_ATTRIBUTES` restricting access to the pipe owner.
#[cfg(windows)]
pub struct OwnerOnlySecurityAttributes {
    sd: windows::Win32::Security::PSECURITY_DESCRIPTOR,
    attrs: windows::Win32::Security::SECURITY_ATTRIBUTES,
}

#[cfg(windows)]
impl OwnerOnlySecurityAttributes {
    /// Builds a `SECURITY_ATTRIBUTES` with SDDL `D:(A;;GA;;;OW)` (Owner-only Generic All).
    #[allow(unsafe_code)]
    pub fn new() -> Result<Self> {
        use windows::core::w;
        use windows::Win32::Foundation::FALSE;
        use windows::Win32::Security::Authorization::{
            ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1,
        };
        use windows::Win32::Security::{PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES};

        let mut sd = PSECURITY_DESCRIPTOR::default();
        // SAFETY: SDDL literal is NUL-terminated UTF-16; `sd` receives an OS-allocated
        // security descriptor that is freed in `Drop` via `LocalFree`.
        unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                w!("D:(A;;GA;;;OW)"),
                SDDL_REVISION_1,
                &mut sd,
                None,
            )
        }
        .map_err(|err| {
            BifrostError::Ipc(format!(
                "failed to construct owner-only Named Pipe DACL: {err}"
            ))
        })?;

        let attrs = SECURITY_ATTRIBUTES {
            nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: sd.0,
            bInheritHandle: FALSE,
        };

        Ok(Self { sd, attrs })
    }

    /// Returns a raw pointer suitable for `ServerOptions::create_with_security_attributes_raw`.
    pub fn as_raw_mut(&mut self) -> *mut c_void {
        (&mut self.attrs as *mut windows::Win32::Security::SECURITY_ATTRIBUTES).cast::<c_void>()
    }
}

#[cfg(windows)]
impl Drop for OwnerOnlySecurityAttributes {
    #[allow(unsafe_code)]
    fn drop(&mut self) {
        use windows::Win32::Foundation::{LocalFree, HLOCAL};
        if !self.sd.0.is_null() {
            // SAFETY: `self.sd.0` was allocated by `ConvertStringSecurityDescriptorToSecurityDescriptorW`
            // and is freed exactly once on drop.
            unsafe {
                let _ = LocalFree(HLOCAL(self.sd.0));
            }
            self.sd.0 = std::ptr::null_mut();
        }
    }
}
