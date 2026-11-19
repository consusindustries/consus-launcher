// The clipboard, for one thing: the key VS Code needs pasted, when the
// person clicks "Copy key". Written from Rust, so the key never passes
// through the webview and the webview has no clipboard permission.
//
// The entry is marked as a secret where the system has a way: on macOS with
// org.nspasteboard.ConcealedType, which clipboard managers honor; on Windows
// with the formats that keep it out of clipboard history, the cloud
// clipboard, and clipboard monitors. Two minutes later it is cleared, but
// only if nothing else was copied since: the launcher compares the
// system's change counter and never reads what the clipboard holds.

use std::sync::Mutex;
use std::thread;
use std::time::Duration;

/// How long the key stays on the clipboard.
const CLEAR_AFTER: Duration = Duration::from_secs(120);

/// The change counter right after the launcher's last copy, while that copy
/// may still be on the clipboard.
static PENDING: Mutex<Option<u64>> = Mutex::new(None);

/// Puts `secret` on the clipboard and clears it after two minutes if it is
/// still the launcher's copy.
pub fn copy_secret(secret: &str) -> Result<(), String> {
    let token = os::copy(secret)?;
    *PENDING.lock().unwrap() = Some(token);
    thread::spawn(move || {
        thread::sleep(CLEAR_AFTER);
        clear_if(token);
    });
    Ok(())
}

/// At launcher exit: a key copied moments ago does not outlive the launcher.
pub fn clear_pending() {
    let pending = *PENDING.lock().unwrap();
    if let Some(token) = pending {
        clear_if(token);
    }
}

fn clear_if(token: u64) {
    let mut pending = PENDING.lock().unwrap();
    if *pending == Some(token) {
        os::clear_if_unchanged(token);
        *pending = None;
    }
}

#[cfg(target_os = "macos")]
mod os {
    use objc2_app_kit::{NSPasteboard, NSPasteboardTypeString};
    use objc2_foundation::NSString;

    /// Clipboard managers that follow nspasteboard.org skip an entry
    /// carrying this type.
    const CONCEALED: &str = "org.nspasteboard.ConcealedType";

    pub fn copy(secret: &str) -> Result<u64, String> {
        let pb = NSPasteboard::generalPasteboard();
        pb.clearContents();
        // SAFETY: an AppKit constant, valid for the life of the process.
        let text_type = unsafe { NSPasteboardTypeString };
        if !pb.setString_forType(&NSString::from_str(secret), text_type) {
            return Err("The key could not be put on the clipboard.".into());
        }
        let _ = pb.setString_forType(&NSString::from_str(""), &NSString::from_str(CONCEALED));
        Ok(pb.changeCount() as u64)
    }

    pub fn clear_if_unchanged(token: u64) {
        let pb = NSPasteboard::generalPasteboard();
        if pb.changeCount() as u64 == token {
            pb.clearContents();
        }
    }
}

#[cfg(windows)]
mod os {
    use std::thread;
    use std::time::Duration;
    use windows_sys::Win32::Foundation::GlobalFree;
    use windows_sys::Win32::System::DataExchange::{
        CloseClipboard, EmptyClipboard, GetClipboardSequenceNumber, OpenClipboard, RegisterClipboardFormatW, SetClipboardData,
    };
    use windows_sys::Win32::System::Memory::{GlobalAlloc, GlobalLock, GlobalUnlock, GMEM_MOVEABLE};
    use windows_sys::Win32::System::Ole::CF_UNICODETEXT;

    /// Another program can hold the clipboard open for a moment.
    fn open() -> Result<(), String> {
        for _ in 0..10 {
            // SAFETY: no owner window; closed by the caller.
            if unsafe { OpenClipboard(std::ptr::null_mut()) } != 0 {
                return Ok(());
            }
            thread::sleep(Duration::from_millis(50));
        }
        Err("The clipboard is in use by another program. Try again.".into())
    }

    /// One format's data, in memory the clipboard then owns.
    fn set(format: u32, bytes: &[u8]) -> bool {
        // SAFETY: the block is sized for `bytes`, filled while locked, and
        // freed here only if the clipboard did not take it.
        unsafe {
            let h = GlobalAlloc(GMEM_MOVEABLE, bytes.len().max(1));
            if h.is_null() {
                return false;
            }
            let p = GlobalLock(h);
            if p.is_null() {
                GlobalFree(h);
                return false;
            }
            std::ptr::copy_nonoverlapping(bytes.as_ptr(), p.cast::<u8>(), bytes.len());
            GlobalUnlock(h);
            if SetClipboardData(format, h).is_null() {
                GlobalFree(h);
                return false;
            }
            true
        }
    }

    fn registered(name: &str) -> u32 {
        let wide: Vec<u16> = name.encode_utf16().chain(Some(0)).collect();
        // SAFETY: a null-terminated UTF-16 string.
        unsafe { RegisterClipboardFormatW(wide.as_ptr()) }
    }

    pub fn copy(secret: &str) -> Result<u64, String> {
        open()?;
        let text: Vec<u8> = secret.encode_utf16().chain(Some(0)).flat_map(u16::to_le_bytes).collect();
        // SAFETY: the clipboard is open.
        let ok = unsafe { EmptyClipboard() } != 0 && set(CF_UNICODETEXT as u32, &text);
        if ok {
            // Kept out of Win+V history and the cloud clipboard, and
            // skipped by clipboard monitors that honor the flag.
            let zero = 0u32.to_le_bytes();
            for name in ["CanIncludeInClipboardHistory", "CanUploadToCloudClipboard", "ExcludeClipboardContentFromMonitorProcessing"] {
                let format = registered(name);
                if format != 0 {
                    let _ = set(format, &zero);
                }
            }
        }
        // SAFETY: opened above.
        unsafe { CloseClipboard() };
        if !ok {
            return Err("The key could not be put on the clipboard.".into());
        }
        // SAFETY: no arguments, no preconditions.
        Ok(unsafe { GetClipboardSequenceNumber() } as u64)
    }

    pub fn clear_if_unchanged(token: u64) {
        // SAFETY: no arguments, no preconditions.
        if unsafe { GetClipboardSequenceNumber() } as u64 != token || open().is_err() {
            return;
        }
        // SAFETY: the clipboard is open.
        unsafe {
            EmptyClipboard();
            CloseClipboard();
        }
    }
}

#[cfg(not(any(target_os = "macos", windows)))]
mod os {
    pub fn copy(_: &str) -> Result<u64, String> {
        Err("Copying the key is not supported on this platform.".into())
    }

    pub fn clear_if_unchanged(_: u64) {}
}
