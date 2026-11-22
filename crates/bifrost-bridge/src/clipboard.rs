//! Native Windows Unicode clipboard integration and bounded Android clipboard sync.

use bifrost_core::{BifrostError, Result};

use crate::adb::AdbClient;

/// Maximum text clipboard payload size accepted by Bridge (1 MiB).
pub const MAX_CLIPBOARD_BYTES: usize = 1024 * 1024;

/// Validates clipboard text bounds and rejects embedded NUL bytes.
pub fn validate_clipboard_text(text: &str) -> Result<()> {
    if text.len() > MAX_CLIPBOARD_BYTES {
        return Err(BifrostError::Bridge(format!(
            "clipboard payload ({} bytes) exceeds maximum {} bytes",
            text.len(),
            MAX_CLIPBOARD_BYTES
        )));
    }
    if text.contains('\0') {
        return Err(BifrostError::Bridge(
            "clipboard text must not contain embedded NUL characters".to_string(),
        ));
    }
    Ok(())
}

/// Reads plain Unicode text (`CF_UNICODETEXT`) directly from the Windows host clipboard.
///
/// Returns `Ok(None)` when the clipboard is empty or holds non-text data (e.g., an image).
#[cfg(windows)]
#[allow(unsafe_code)]
pub fn read_host_clipboard_text() -> Result<Option<String>> {
    use windows::Win32::Foundation::{HANDLE, HGLOBAL};
    use windows::Win32::System::DataExchange::{
        CloseClipboard, GetClipboardData, IsClipboardFormatAvailable, OpenClipboard,
    };
    use windows::Win32::System::Memory::{GlobalLock, GlobalUnlock};

    const CF_UNICODETEXT: u32 = 13;

    // SAFETY: `IsClipboardFormatAvailable` is a pure read-only Win32 query.
    if unsafe { IsClipboardFormatAvailable(CF_UNICODETEXT) }.is_err() {
        return Ok(None);
    }

    // SAFETY: `OpenClipboard(None)` opens the clipboard for the current task; paired with `CloseClipboard` before returning.
    unsafe { OpenClipboard(None) }
        .map_err(|err| BifrostError::Bridge(format!("failed to open Windows clipboard: {err}")))?;

    let result = (|| -> Result<Option<String>> {
        // SAFETY: Clipboard is currently open; `GetClipboardData` returns a clipboard-owned `HANDLE`.
        let handle: HANDLE = match unsafe { GetClipboardData(CF_UNICODETEXT) } {
            Ok(h) if !h.is_invalid() => h,
            _ => return Ok(None),
        };
        let hglobal = HGLOBAL(handle.0);

        // SAFETY: `GlobalLock` locks the UTF-16 NUL-terminated buffer until `GlobalUnlock`.
        let ptr = unsafe { GlobalLock(hglobal) } as *const u16;
        if ptr.is_null() {
            return Ok(None);
        }

        let mut len: usize = 0;
        let max_u16 = MAX_CLIPBOARD_BYTES / 2;
        // SAFETY: Clipboard `CF_UNICODETEXT` is NUL-terminated; bounded by `max_u16`.
        while len < max_u16 && unsafe { *ptr.add(len) } != 0 {
            len += 1;
        }
        // SAFETY: Slice covers `len` valid UTF-16 code units prior to the NUL terminator.
        let slice = unsafe { std::slice::from_raw_parts(ptr, len) };
        let text = String::from_utf16_lossy(slice);

        // SAFETY: Paired with `GlobalLock(hglobal)` above.
        unsafe {
            let _ = GlobalUnlock(hglobal);
        }

        Ok(Some(text))
    })();

    // SAFETY: Paired with `OpenClipboard(None)` above.
    unsafe {
        let _ = CloseClipboard();
    }

    result
}

/// Writes plain Unicode text (`CF_UNICODETEXT`) to the Windows host clipboard.
#[cfg(windows)]
#[allow(unsafe_code)]
pub fn write_host_clipboard_text(text: &str) -> Result<()> {
    use windows::Win32::Foundation::{HANDLE, HGLOBAL};
    use windows::Win32::System::DataExchange::{
        CloseClipboard, EmptyClipboard, OpenClipboard, SetClipboardData,
    };
    use windows::Win32::System::Memory::{GlobalAlloc, GlobalLock, GlobalUnlock, GMEM_MOVEABLE};

    const CF_UNICODETEXT: u32 = 13;

    validate_clipboard_text(text)?;
    let utf16: Vec<u16> = text.encode_utf16().chain(std::iter::once(0)).collect();
    let byte_len = utf16.len() * std::mem::size_of::<u16>();

    // SAFETY: Opens Windows clipboard; always closed before returning.
    unsafe { OpenClipboard(None) }
        .map_err(|err| BifrostError::Bridge(format!("failed to open Windows clipboard: {err}")))?;

    let outcome = (|| -> Result<()> {
        // SAFETY: Clipboard is open and owned by this task.
        unsafe { EmptyClipboard() }.map_err(|err| {
            BifrostError::Bridge(format!("failed to empty Windows clipboard: {err}"))
        })?;

        // SAFETY: Allocates moveable global memory transferred to clipboard ownership on success.
        let hglobal: HGLOBAL = unsafe { GlobalAlloc(GMEM_MOVEABLE, byte_len) }.map_err(|err| {
            BifrostError::Bridge(format!("failed to allocate clipboard memory: {err}"))
        })?;

        // SAFETY: Locks freshly allocated buffer of exact size `byte_len`.
        let dest = unsafe { GlobalLock(hglobal) } as *mut u16;
        if dest.is_null() {
            return Err(BifrostError::Bridge(
                "GlobalLock returned null for clipboard buffer".to_string(),
            ));
        }

        // SAFETY: `dest` has capacity for `utf16.len()` `u16` elements; non-overlapping copy.
        unsafe {
            std::ptr::copy_nonoverlapping(utf16.as_ptr(), dest, utf16.len());
            let _ = GlobalUnlock(hglobal);
            SetClipboardData(CF_UNICODETEXT, HANDLE(hglobal.0))
        }
        .map_err(|err| BifrostError::Bridge(format!("failed to set Windows clipboard: {err}")))?;

        Ok(())
    })();

    // SAFETY: Paired with `OpenClipboard(None)` above.
    unsafe {
        let _ = CloseClipboard();
    }

    outcome
}

#[cfg(not(windows))]
pub fn read_host_clipboard_text() -> Result<Option<String>> {
    Err(BifrostError::Unsupported {
        feature: "Windows host clipboard".to_string(),
        planned_milestone: "Windows 11 x64 host only".to_string(),
    })
}

#[cfg(not(windows))]
pub fn write_host_clipboard_text(_text: &str) -> Result<()> {
    Err(BifrostError::Unsupported {
        feature: "Windows host clipboard".to_string(),
        planned_milestone: "Windows 11 x64 host only".to_string(),
    })
}

/// Syncs text from the Windows clipboard into Android via `cmd clipboard set-text`.
pub fn sync_clipboard_to_android(adb: &AdbClient, serial: &str) -> Result<usize> {
    let Some(text) = read_host_clipboard_text()? else {
        return Err(BifrostError::Bridge(
            "Windows host clipboard does not currently contain Unicode text".to_string(),
        ));
    };
    validate_clipboard_text(&text)?;
    let _ = adb.run_adb(&["-s", serial, "shell", "cmd", "clipboard", "set-text", &text])?;
    Ok(text.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_clipboard_size_and_nul_bytes() {
        assert!(validate_clipboard_text("Hello BIFRÖST").is_ok());
        assert!(validate_clipboard_text("bad\0nul").is_err());
        let oversized = "a".repeat(MAX_CLIPBOARD_BYTES + 1);
        assert!(validate_clipboard_text(&oversized).is_err());
    }

    #[cfg(windows)]
    #[test]
    fn reads_host_clipboard_non_destructively() {
        // Read-only probe must succeed cleanly without modifying user clipboard contents.
        assert!(read_host_clipboard_text().is_ok());
    }
}
