//! One client per executable: launching it again brings back the one running.
//!
//! Closing the window only hides it, so the Start menu, a pinned taskbar
//! button, or Windows reopening the app for a notification clicked in the
//! Action Center all started a second client beside the hidden first -- a
//! second window, a second socket, and every notification twice.
//!
//! Per executable rather than per user: the dev build and the installed one are
//! different files, and each has its own.

/// Whether this process is the client, or only asked the running one to show.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Launch {
    First,
    Handed,
}

/// What the tray's window is told when a second launch hands over.
#[cfg(windows)]
pub const HANDED: u32 = windows::Win32::UI::WindowsAndMessaging::WM_APP + 2;

/// This executable's mark, the same for every launch of the same file.
fn mark() -> String {
    let path = std::env::current_exe()
        .ok()
        .and_then(|path| path.canonicalize().ok().or(Some(path)))
        .map(|path| path.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    format!("{:016x}", fnv(path.as_bytes()))
}

/// FNV-1a: std's hasher is free to change between releases, and the two
/// launches may not have been built by the same one.
fn fnv(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |hash, byte| {
        (hash ^ u64::from(*byte)).wrapping_mul(0x0100_0000_01b3)
    })
}

/// The title of the tray's window, which is how a second launch finds it.
pub fn tray_title() -> String {
    format!("MatterLess {}", mark())
}

/// Claims the executable for this process, or hands over to the one that has.
#[cfg(windows)]
pub fn claim() -> Launch {
    use windows::Win32::Foundation::{ERROR_ALREADY_EXISTS, GetLastError};
    use windows::Win32::System::Threading::CreateMutexW;
    use windows::core::HSTRING;

    let name = HSTRING::from(format!("Local\\MatterLess-{}", mark()));
    // Never closed: held for the life of the process, let go of at its exit.
    if unsafe { CreateMutexW(None, false, &name) }.is_err() {
        // Nothing to coordinate through, so nothing to stop this one.
        return Launch::First;
    }
    if unsafe { GetLastError() } != ERROR_ALREADY_EXISTS {
        return Launch::First;
    }
    hand_over();
    Launch::Handed
}

#[cfg(not(windows))]
pub fn claim() -> Launch {
    Launch::First
}

/// Asks the running client to show its window.
///
/// Waits a little for its tray, which it puts up with the window: a second
/// launch right behind the first can arrive before there is one.
#[cfg(windows)]
fn hand_over() {
    use windows::Win32::Foundation::{LPARAM, WPARAM};
    use windows::Win32::UI::WindowsAndMessaging::{
        AllowSetForegroundWindow, FindWindowExW, GetWindowThreadProcessId, HWND_MESSAGE,
        PostMessageW,
    };
    use windows::core::{HSTRING, w};

    let title = HSTRING::from(tray_title());
    let started = std::time::Instant::now();
    while started.elapsed() < std::time::Duration::from_secs(5) {
        let found =
            unsafe { FindWindowExW(Some(HWND_MESSAGE), None, w!("MatterLessTray"), &title) };
        if let Ok(tray) = found {
            let mut owner = 0u32;
            unsafe {
                GetWindowThreadProcessId(tray, Some(&raw mut owner));
                // This launch is what the reader just did, so it may come to
                // the front; this passes that on to the window that will.
                let _ = AllowSetForegroundWindow(owner);
                let _ = PostMessageW(Some(tray), HANDED, WPARAM(0), LPARAM(0));
            }
            println!("already running, so its window was asked back");
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    println!("already running, but its tray never answered");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_mark_is_the_same_for_every_launch() {
        assert_eq!(mark(), mark());
        assert_eq!(mark().len(), 16);
    }

    #[test]
    fn different_files_have_different_marks() {
        assert_ne!(
            fnv(b"c:\\users\\a\\appdata\\local\\matterless\\matterless-view.exe"),
            fnv(b"h:\\dev\\matterless\\target\\release\\matterless-view.exe")
        );
    }
}
