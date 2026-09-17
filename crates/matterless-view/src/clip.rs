//! The system clipboard, so copy and paste cross to other windows.
//!
//! The window kept a `String` of its own and called it the clipboard, which
//! meant copy and paste worked inside it and nowhere else -- a reader who had
//! copied a stack trace from their editor pressed `Ctrl+V` here and nothing
//! happened at all.
//!
//! Text only. A picture on the clipboard is a device-independent bitmap and a
//! rather different job: it wants uploading rather than inserting, which is a
//! decision about what a message *is* and not about the clipboard.

/// What the system is holding, if it is holding text.
#[cfg(windows)]
pub fn read() -> Option<String> {
    use windows::Win32::Foundation::HGLOBAL;
    use windows::Win32::System::DataExchange::{GetClipboardData, OpenClipboard};
    use windows::Win32::System::Memory::{GlobalLock, GlobalUnlock};
    use windows::Win32::System::Ole::CF_UNICODETEXT;

    // Every path from here has to close it again: the clipboard is one
    // system-wide lock, and a window that opens it and returns leaves every
    // other application unable to copy anything.
    unsafe { OpenClipboard(None) }.ok()?;
    let said = unsafe {
        let Ok(handle) = GetClipboardData(CF_UNICODETEXT.0 as u32) else {
            // Nothing on it, or nothing on it as text -- a copied picture or a
            // copied file both look like this.
            return close(None);
        };
        let held = HGLOBAL(handle.0);
        let at = GlobalLock(held) as *const u16;
        if at.is_null() {
            return close(None);
        }
        // Counted to its terminator, because the handle's size is rounded up
        // to an allocation and says nothing about the text in it.
        let mut many = 0;
        while *at.add(many) != 0 {
            many += 1;
        }
        let said = String::from_utf16_lossy(std::slice::from_raw_parts(at, many));
        let _ = GlobalUnlock(held);
        said
    };
    close(Some(said))
}

/// Closes the clipboard and hands back whatever was read from it.
#[cfg(windows)]
fn close(said: Option<String>) -> Option<String> {
    let _ = unsafe { windows::Win32::System::DataExchange::CloseClipboard() };
    said
}

/// Puts text on the system clipboard.
#[cfg(windows)]
pub fn write(text: &str) {
    use windows::Win32::Foundation::HANDLE;
    use windows::Win32::System::DataExchange::{
        CloseClipboard, EmptyClipboard, OpenClipboard, SetClipboardData,
    };
    use windows::Win32::System::Memory::{
        GLOBAL_ALLOC_FLAGS, GlobalAlloc, GlobalLock, GlobalUnlock,
    };
    use windows::Win32::System::Ole::CF_UNICODETEXT;

    /// `GMEM_MOVEABLE`, which is the only kind of handle the clipboard takes.
    const MOVEABLE: GLOBAL_ALLOC_FLAGS = GLOBAL_ALLOC_FLAGS(0x0002);

    let wide: Vec<u16> = text.encode_utf16().chain(std::iter::once(0)).collect();
    if unsafe { OpenClipboard(None) }.is_err() {
        return;
    }
    unsafe {
        let _ = EmptyClipboard();
        let Ok(held) = GlobalAlloc(MOVEABLE, wide.len() * size_of::<u16>()) else {
            let _ = CloseClipboard();
            return;
        };
        let at = GlobalLock(held) as *mut u16;
        if at.is_null() {
            let _ = windows::Win32::Foundation::GlobalFree(Some(held));
            let _ = CloseClipboard();
            return;
        }
        std::ptr::copy_nonoverlapping(wide.as_ptr(), at, wide.len());
        let _ = GlobalUnlock(held);
        // The system takes the handle on success and frees it in its own time,
        // so it must not be freed here -- and must be freed here if it refused
        // it, or the window leaks a copy of every copy.
        match SetClipboardData(CF_UNICODETEXT.0 as u32, Some(HANDLE(held.0))) {
            Ok(_) => {}
            Err(_) => {
                let _ = windows::Win32::Foundation::GlobalFree(Some(held));
            }
        }
        let _ = CloseClipboard();
    }
}

/// Nothing to read anywhere else, which leaves the window's own copy of the
/// text as the whole of its clipboard -- as it was everywhere before this.
#[cfg(not(windows))]
pub fn read() -> Option<String> {
    None
}

#[cfg(not(windows))]
pub fn write(_text: &str) {}

#[cfg(all(test, windows))]
mod tests {
    /// Text put on the clipboard comes back off it.
    ///
    /// Ignored by default, and not because it is slow: it takes over the
    /// machine's clipboard, and a test suite that throws away whatever the
    /// person running it had just copied is a test suite nobody runs twice.
    ///
    ///     cargo test -p matterless-view clipboard -- --ignored
    #[test]
    #[ignore = "takes over the machine's clipboard"]
    fn the_clipboard_carries_text_both_ways() {
        let said = "a stack trace, or a permalink";
        super::write(said);
        assert_eq!(super::read().as_deref(), Some(said));
        // Including the empty one, which is what a reader who cleared the box
        // and copied it puts there.
        super::write("");
        assert_eq!(super::read().as_deref(), Some(""));
    }
}
