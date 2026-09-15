//! The notification area: what keeps the client running once its window shuts.
//!
//! Closing the window hides it rather than quitting, which is only defensible
//! because there is a way back and a way out. Both live here: notifications
//! exist only while this process runs -- there is no push proxy for a desktop
//! client -- so quitting on the close button would quietly turn them off, which
//! is the one thing that lets somebody keep the webapp closed.
//!
//! Its menu carries the two things that cannot live in the window, because the
//! window may not be there: a way back to it, and a way to actually quit. Start
//! with Windows is here too rather than among the app's own settings, for the
//! same reason -- it is a question about the client existing, not about how it
//! draws.
//!
//! Tauri supplied all of this as a builder. A window that owns its own message
//! loop has to reach `Shell_NotifyIcon` itself, which needs a window to deliver
//! the callback to -- so this makes a message-only one of its own rather than
//! trying to get inside winit's.

/// What the reader asked the tray for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Act {
    /// Come back: show the window, wherever it had got to.
    Open,
    /// Turn starting with Windows on or off.
    Startup,
    /// The one way out, now that closing the window only hides it.
    Quit,
}

impl Act {
    /// The menu's ids. `TrackPopupMenu` answers with one of these directly,
    /// which is why nothing here has to handle `WM_COMMAND`.
    fn from_id(id: u32) -> Option<Self> {
        Some(match id {
            1 => Act::Open,
            2 => Act::Startup,
            3 => Act::Quit,
            _ => return None,
        })
    }
}

/// The icon in the notification area, for as long as this is alive.
#[derive(Debug, Default)]
pub struct Tray {
    live: bool,
}

impl Tray {
    /// Puts the icon there, and answers whether it went.
    ///
    /// `asked` is called on the thread that owns the window -- the callback
    /// arrives as a window message like any other, so it is delivered by the
    /// same message pump winit is already running.
    pub fn show(&mut self, asked: impl Fn(Act) + 'static) -> bool {
        if self.live {
            return true;
        }
        self.live = platform::add(Box::new(asked));
        self.live
    }

    /// Takes it away again. Windows does not always tidy up an icon whose
    /// process has gone, and a dead icon somebody can click is worse than
    /// none.
    pub fn hide(&mut self) {
        if self.live {
            platform::remove();
            self.live = false;
        }
    }
}

impl Drop for Tray {
    fn drop(&mut self) {
        self.hide();
    }
}

/// Whether this client starts with Windows, and the switch for it.
///
/// A registry value under `Run` rather than a shortcut in the Startup folder:
/// it is what the autostart plugin wrote, so a reader who had it on in the app
/// still has it on here.
pub mod startup {
    /// Whether it is on right now.
    pub fn enabled() -> bool {
        super::platform::startup_enabled()
    }

    /// Turns it on or off. Answers what it ended up as, which is not
    /// necessarily what was asked for -- a locked-down machine may refuse.
    pub fn set(on: bool) -> bool {
        super::platform::startup_set(on);
        enabled()
    }
}

#[cfg(windows)]
mod platform {
    use super::Act;
    use std::cell::RefCell;
    use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM};
    use windows::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows::Win32::System::Registry::{
        HKEY, HKEY_CURRENT_USER, KEY_READ, KEY_WRITE, REG_SZ, RegCloseKey, RegDeleteValueW,
        RegOpenKeyExW, RegQueryValueExW, RegSetValueExW,
    };
    use windows::Win32::UI::Shell::{
        NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NOTIFYICONDATAW, Shell_NotifyIconW,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        AppendMenuW, CreatePopupMenu, CreateWindowExW, DefWindowProcW, DestroyMenu, DestroyWindow,
        GetCursorPos, HWND_MESSAGE, MF_CHECKED, MF_SEPARATOR, MF_STRING, MF_UNCHECKED,
        PostMessageW, RegisterClassW, SetForegroundWindow, TPM_RETURNCMD, TPM_RIGHTBUTTON,
        TrackPopupMenu, WINDOW_EX_STYLE, WINDOW_STYLE, WM_APP, WM_DESTROY, WM_LBUTTONUP, WM_NULL,
        WM_RBUTTONUP, WNDCLASSW,
    };
    use windows::core::{HSTRING, PCWSTR, w};

    /// Our own message for the icon's callbacks. Anything from `WM_APP` up is
    /// the application's to define.
    const CALLBACK: u32 = WM_APP + 1;
    /// The icon's id within this window. One icon, so one id.
    const ICON_ID: u32 = 1;

    // The message-only window and what to call when the icon is used. Thread
    // local because all of it belongs to the thread that pumps the messages,
    // and there is exactly one such thread: the window's.
    thread_local! {
        static STATE: RefCell<Option<State>> = const { RefCell::new(None) };
    }

    struct State {
        window: HWND,
        asked: Box<dyn Fn(Act)>,
    }

    /// Where the value that starts this client with Windows lives.
    const RUN_KEY: PCWSTR = w!("Software\\Microsoft\\Windows\\CurrentVersion\\Run");
    /// What it is called, matching what the app's autostart plugin wrote.
    const RUN_VALUE: PCWSTR = w!("MatterLess");

    pub fn add(asked: Box<dyn Fn(Act)>) -> bool {
        unsafe {
            let Ok(instance) = GetModuleHandleW(None) else {
                return false;
            };
            let class = WNDCLASSW {
                lpfnWndProc: Some(wndproc),
                hInstance: instance.into(),
                lpszClassName: w!("MatterLessTray"),
                ..Default::default()
            };
            // Registering twice is an error worth ignoring: the class survives
            // for the life of the process, and this may be the second tray.
            let _ = RegisterClassW(&raw const class);
            // Message-only: it has no place on screen and never paints, it
            // exists to be somewhere the shell can deliver a click.
            let window = CreateWindowExW(
                WINDOW_EX_STYLE(0),
                w!("MatterLessTray"),
                w!("MatterLess"),
                WINDOW_STYLE(0),
                0,
                0,
                0,
                0,
                Some(HWND_MESSAGE),
                None,
                Some(instance.into()),
                None,
            );
            let Ok(window) = window else {
                return false;
            };
            STATE.with(|state| *state.borrow_mut() = Some(State { window, asked }));

            let mut data = NOTIFYICONDATAW {
                cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
                hWnd: window,
                uID: ICON_ID,
                uFlags: NIF_ICON | NIF_MESSAGE | NIF_TIP,
                uCallbackMessage: CALLBACK,
                // A blank one would still be added, and would still answer a
                // click -- an invisible thing in the notification area that
                // does something when pressed is worse than no tray at all.
                hIcon: match icon() {
                    Some(icon) => icon,
                    None => {
                        eprintln!("the tray icon would not decode");
                        let _ = DestroyWindow(window);
                        STATE.with(|state| *state.borrow_mut() = None);
                        return false;
                    }
                },
                ..Default::default()
            };
            // `szTip` is a fixed array of UTF-16, not a pointer.
            for (at, unit) in "MatterLess".encode_utf16().enumerate() {
                data.szTip[at] = unit;
            }
            let added = Shell_NotifyIconW(NIM_ADD, &raw const data).as_bool();
            if !added {
                let _ = DestroyWindow(window);
                STATE.with(|state| *state.borrow_mut() = None);
            }
            added
        }
    }

    pub fn remove() {
        STATE.with(|state| {
            let Some(state) = state.borrow_mut().take() else {
                return;
            };
            unsafe {
                let data = NOTIFYICONDATAW {
                    cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
                    hWnd: state.window,
                    uID: ICON_ID,
                    ..Default::default()
                };
                let _ = Shell_NotifyIconW(NIM_DELETE, &raw const data);
                let _ = DestroyWindow(state.window);
            }
        });
    }

    /// The tray's own picture, at the size the tray asks for.
    ///
    /// Its own image, not the window's: the window icon is 512px and Windows
    /// would hand the tray a filtered downscale of it -- the same mistake the
    /// badge made before it was drawn at the size asked for. `tray.png` is 32px
    /// and sharpened, so the halving to 16 is clean.
    fn icon() -> Option<windows::Win32::UI::WindowsAndMessaging::HICON> {
        const TRAY: &[u8] = include_bytes!("../resources/icons/tray.png");
        let decoded = image::load_from_memory(TRAY).ok()?.into_rgba8();
        let (width, height) = decoded.dimensions();
        if width != height {
            return None;
        }
        crate::taskbar::platform::icon_from(width, &decoded.into_raw())
    }

    /// Everything the icon sends arrives here.
    unsafe extern "system" fn wndproc(
        window: HWND,
        message: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        if message == CALLBACK {
            // The button is in the low word of `lParam`; the high word is the
            // icon's id, which there is only one of.
            match (lparam.0 as u32) & 0xFFFF {
                // A left click just brings the window back, which is what
                // every other tray icon does.
                WM_LBUTTONUP => tell(Act::Open),
                WM_RBUTTONUP => menu(window),
                _ => {}
            }
            return LRESULT(0);
        }
        if message == WM_DESTROY {
            return LRESULT(0);
        }
        unsafe { DefWindowProcW(window, message, wparam, lparam) }
    }

    /// Opens the menu at the pointer and acts on what was chosen.
    ///
    /// `TPM_RETURNCMD` answers with the item's id rather than posting
    /// `WM_COMMAND`, so the whole menu is this one function.
    fn menu(window: HWND) {
        unsafe {
            let Ok(popup) = CreatePopupMenu() else {
                return;
            };
            let _ = AppendMenuW(popup, MF_STRING, 1, w!("Open MatterLess"));
            let _ = AppendMenuW(
                popup,
                MF_STRING
                    | if startup_enabled() {
                        MF_CHECKED
                    } else {
                        MF_UNCHECKED
                    },
                2,
                w!("Start with Windows"),
            );
            let _ = AppendMenuW(popup, MF_SEPARATOR, 0, PCWSTR::null());
            let _ = AppendMenuW(popup, MF_STRING, 3, w!("Quit"));

            let mut at = POINT::default();
            let _ = GetCursorPos(&raw mut at);
            // Required, and required in this order: without it the menu does
            // not close when the reader clicks away from it, which is a menu
            // stuck on the screen.
            let _ = SetForegroundWindow(window);
            let chosen = TrackPopupMenu(
                popup,
                TPM_RETURNCMD | TPM_RIGHTBUTTON,
                at.x,
                at.y,
                Some(0),
                window,
                None,
            );
            let _ = PostMessageW(Some(window), WM_NULL, WPARAM(0), LPARAM(0));
            let _ = DestroyMenu(popup);
            if let Some(act) = Act::from_id(chosen.0 as u32) {
                tell(act);
            }
        }
    }

    /// Hands one act to whoever asked for the tray.
    ///
    /// The callback is taken out of the cell for the call and put back after:
    /// it is free to do anything, including something that reaches back in
    /// here, and holding a borrow across that would panic.
    fn tell(act: Act) {
        let Some(state) = STATE.with(|state| state.borrow_mut().take()) else {
            return;
        };
        (state.asked)(act);
        STATE.with(|cell| {
            // Unless the callback tore the tray down, in which case it stays
            // down.
            if cell.borrow().is_none() {
                *cell.borrow_mut() = Some(state);
            }
        });
    }

    pub fn startup_enabled() -> bool {
        unsafe {
            let mut key = HKEY::default();
            if RegOpenKeyExW(HKEY_CURRENT_USER, RUN_KEY, Some(0), KEY_READ, &raw mut key).is_err() {
                return false;
            }
            let mut size = 0u32;
            let found = RegQueryValueExW(key, RUN_VALUE, None, None, None, Some(&raw mut size));
            let _ = RegCloseKey(key);
            found.is_ok()
        }
    }

    pub fn startup_set(on: bool) {
        let Ok(exe) = std::env::current_exe() else {
            return;
        };
        unsafe {
            let mut key = HKEY::default();
            if RegOpenKeyExW(HKEY_CURRENT_USER, RUN_KEY, Some(0), KEY_WRITE, &raw mut key).is_err()
            {
                return;
            }
            if on {
                // Quoted, because a path with a space in it is otherwise read
                // as a command and its arguments.
                let quoted = HSTRING::from(format!("\"{}\"", exe.display()));
                let bytes = std::slice::from_raw_parts(
                    quoted.as_ptr() as *const u8,
                    (quoted.len() + 1) * 2,
                );
                let _ = RegSetValueExW(key, RUN_VALUE, None, REG_SZ, Some(bytes));
            } else {
                let _ = RegDeleteValueW(key, RUN_VALUE);
            }
            let _ = RegCloseKey(key);
        }
    }
}

#[cfg(not(windows))]
mod platform {
    use super::Act;

    /// No notification area here, so the window is the whole client.
    pub fn add(_asked: Box<dyn Fn(Act)>) -> bool {
        false
    }
    pub fn remove() {}
    pub fn startup_enabled() -> bool {
        false
    }
    pub fn startup_set(_on: bool) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The picture is the whole point of the icon, and it travels in the
    /// binary: a file that failed to decode would put a blank in the
    /// notification area that still answers a click.
    #[cfg(windows)]
    #[test]
    fn the_tray_picture_is_a_square_that_becomes_an_icon() {
        const TRAY: &[u8] = include_bytes!("../resources/icons/tray.png");
        let decoded = image::load_from_memory(TRAY)
            .expect("the bundled tray picture must decode")
            .into_rgba8();
        let (width, height) = decoded.dimensions();
        assert_eq!(width, height, "an icon is square");
        // 32, so the halving to the 16 the tray asks for is clean rather than
        // a filtered downscale from the window's 512.
        assert_eq!(width, 32);
        assert!(
            decoded.pixels().any(|pixel| pixel.0[3] > 0),
            "every pixel is transparent, so there is nothing to see"
        );
        assert!(
            crate::taskbar::platform::icon_from(width, &decoded.into_raw()).is_some(),
            "Windows refused to make an icon of it"
        );
    }

    /// Every menu id means exactly one thing, and nothing else means anything.
    ///
    /// The ids are raw numbers crossing a Win32 boundary, so an off-by-one here
    /// would quit when the reader asked to open.
    #[test]
    fn a_menu_id_names_exactly_one_act() {
        assert_eq!(Act::from_id(1), Some(Act::Open));
        assert_eq!(Act::from_id(2), Some(Act::Startup));
        assert_eq!(Act::from_id(3), Some(Act::Quit));
        // Nought is what `TrackPopupMenu` answers when the menu was dismissed
        // without choosing anything, which must do nothing at all.
        assert_eq!(Act::from_id(0), None);
        assert_eq!(Act::from_id(99), None);
    }

    /// A tray that never went up is not taken down, and one taken down twice
    /// is only removed once: `Shell_NotifyIcon(NIM_DELETE)` on an icon that is
    /// not there fails, and `Drop` runs after an explicit `hide`.
    #[test]
    fn it_is_only_removed_when_it_was_added() {
        let mut tray = Tray::default();
        assert!(!tray.live);
        tray.hide();
        assert!(!tray.live);
        // Pretending it went up, which is all this half can test without a
        // shell to put it in.
        tray.live = true;
        tray.hide();
        assert!(!tray.live);
    }
}
