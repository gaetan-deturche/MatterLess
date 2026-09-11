//! Putting the badge on the taskbar button, and asking for attention.
//!
//! In the app this was Tauri's: `set_overlay_icon` and
//! `request_user_attention`, each a line. A window that owns its own `HWND` has
//! to reach `ITaskbarList3` and `FlashWindowEx` itself, so this is the part of
//! the badge that had nowhere to be ported *from* -- everything it draws comes
//! from [`crate::badge`].
//!
//! All of it is best-effort. A taskbar overlay is decoration on top of a window
//! that already works: a machine that refuses one -- an unusual shell, a remote
//! session, COM declining to start -- should carry on without it rather than
//! fail to start.

use crate::badge::Overlay;

/// The last thing handed to the shell, so an unchanged badge is not redrawn.
///
/// Every arriving message recomputes what the badge should be, and almost every
/// one of them computes the same answer. Re-sending it means creating and
/// destroying an icon per message, and asking the shell to repaint the button.
#[derive(Debug, Default)]
pub struct Taskbar {
    showing: Option<Overlay>,
    /// Whether `showing` was ever actually handed over, as opposed to merely
    /// computed. Without it, "no badge" computed before the window exists is
    /// indistinguishable from "no badge, and the shell has been told".
    delivered: bool,
    /// Whether the button is flashing, so it is only stopped once.
    flashing: bool,
}

impl Taskbar {
    /// Shows `overlay`, or takes the badge off when there is nothing to show.
    ///
    /// Answers whether anything actually changed, which is worth knowing only
    /// because the alternative is a log line per message.
    ///
    /// Nothing is remembered unless it was delivered. Counts are computed from
    /// the store long before there is a window to put them on -- the first one
    /// lands while the sidebar is being built -- and recording that as shown
    /// would make every identical answer afterwards look like a no-op, so the
    /// badge would never appear at all.
    pub fn show(&mut self, window: RawWindow, overlay: Option<Overlay>, description: &str) -> bool {
        if window == 0 {
            return false;
        }
        if self.delivered && self.showing == overlay {
            return false;
        }
        platform::set_overlay(window, overlay.as_ref(), description);
        self.showing = overlay;
        self.delivered = true;
        true
    }

    /// Asks for attention: the taskbar button flashes.
    ///
    /// `urgent` keeps it flashing until the window is looked at, which is right
    /// for something that named the reader; otherwise it is a single nudge.
    /// Only meaningful while the window is unfocused, which the caller decides:
    /// asking for attention you already have is how an app becomes irritating.
    pub fn ask_for_attention(&mut self, window: RawWindow, urgent: bool) {
        platform::flash(window, urgent);
        self.flashing = true;
    }

    /// Stops the flashing, once the window has been looked at.
    pub fn calm(&mut self, window: RawWindow) {
        if !self.flashing {
            return;
        }
        platform::calm(window);
        self.flashing = false;
    }
}

/// A window handle, in the one form this needs it.
///
/// Carried as a number rather than a `HWND` so the callers, the tests and the
/// platforms that have no such thing all compile without a `cfg` each.
pub type RawWindow = isize;

#[cfg(windows)]
mod platform {
    use super::RawWindow;
    use crate::badge::Overlay;
    use windows::Win32::Foundation::HWND;
    use windows::Win32::Graphics::Gdi::{
        BI_RGB, BITMAPINFO, BITMAPINFOHEADER, CreateBitmap, CreateDIBSection, DIB_RGB_COLORS,
        DeleteObject,
    };
    use windows::Win32::System::Com::{
        CLSCTX_ALL, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx,
    };
    use windows::Win32::UI::Shell::{ITaskbarList3, TaskbarList};
    use windows::Win32::UI::WindowsAndMessaging::{
        CreateIconIndirect, DestroyIcon, FLASHW_ALL, FLASHW_STOP, FLASHW_TIMERNOFG, FLASHWINFO,
        FlashWindowEx, HICON, ICONINFO,
    };
    use windows::core::HSTRING;

    /// The shell's taskbar object, created once and kept.
    ///
    /// `HrInit` has to be called before anything else on it, and the whole
    /// thing is apartment-threaded: this is only ever touched from the thread
    /// that owns the window, which is the same rule every other Win32 call in
    /// this window follows.
    fn taskbar() -> Option<&'static ITaskbarList3> {
        thread_local! {
            static LIST: Option<ITaskbarList3> = unsafe {
                // Already initialised is a success, not a failure: winit may
                // well have got there first.
                let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
                let list: ITaskbarList3 = CoCreateInstance(&TaskbarList, None, CLSCTX_ALL).ok()?;
                list.HrInit().ok()?;
                Some(list)
            };
        }
        // The object lives for the thread, and the thread outlives every caller
        // here -- it is the one running the event loop.
        LIST.with(|list| {
            list.as_ref()
                .map(|list| unsafe { &*(list as *const ITaskbarList3) })
        })
    }

    pub fn set_overlay(window: RawWindow, overlay: Option<&Overlay>, description: &str) {
        let Some(list) = taskbar() else {
            return;
        };
        let hwnd = HWND(window as *mut _);
        let icon = overlay.and_then(icon_from);
        let told = HSTRING::from(description);
        unsafe {
            // A null icon is how the overlay is taken off again, which is why
            // this is not conditional on there being one.
            let _ = list.SetOverlayIcon(hwnd, icon.unwrap_or_default(), &told);
            // The shell copies it, so the handle is ours to release.
            if let Some(icon) = icon {
                let _ = DestroyIcon(icon);
            }
        }
    }

    /// Builds an icon from straight RGBA.
    ///
    /// A DIB section rather than `CreateBitmap`: a device-dependent bitmap has
    /// nowhere to keep an alpha channel, and an icon without one has square
    /// corners over whatever the taskbar is painted with. The colour has to be
    /// premultiplied and in BGRA order, which is what the loop does.
    fn icon_from(overlay: &Overlay) -> Option<HICON> {
        let side = overlay.size as i32;
        let header = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: side,
                // Negative: top-down rows, as the overlay is stored.
                biHeight: -side,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        unsafe {
            let mut bits: *mut std::ffi::c_void = std::ptr::null_mut();
            let colour = CreateDIBSection(
                None,
                &raw const header,
                DIB_RGB_COLORS,
                &raw mut bits,
                None,
                0,
            )
            .ok()?;
            if bits.is_null() {
                let _ = DeleteObject(colour.into());
                return None;
            }
            let into = std::slice::from_raw_parts_mut(bits as *mut u8, overlay.pixels.len());
            for (out, pixel) in into.chunks_exact_mut(4).zip(overlay.pixels.chunks_exact(4)) {
                let alpha = u32::from(pixel[3]);
                let premultiplied = |channel: u8| ((u32::from(channel) * alpha) / 255) as u8;
                out[0] = premultiplied(pixel[2]);
                out[1] = premultiplied(pixel[1]);
                out[2] = premultiplied(pixel[0]);
                out[3] = pixel[3];
            }
            // A mask is required even though the alpha decides everything: all
            // zeroes means "every pixel is part of the icon".
            let mask = CreateBitmap(side, side, 1, 1, None);
            let info = ICONINFO {
                fIcon: true.into(),
                xHotspot: 0,
                yHotspot: 0,
                hbmMask: mask,
                hbmColor: colour,
            };
            let icon = CreateIconIndirect(&raw const info).ok();
            // `CreateIconIndirect` copies both bitmaps.
            let _ = DeleteObject(mask.into());
            let _ = DeleteObject(colour.into());
            icon
        }
    }

    pub fn flash(window: RawWindow, urgent: bool) {
        let info = FLASHWINFO {
            cbSize: std::mem::size_of::<FLASHWINFO>() as u32,
            hwnd: HWND(window as *mut _),
            // Until it is looked at for something that named the reader; once
            // for anything else.
            dwFlags: if urgent {
                FLASHW_ALL | FLASHW_TIMERNOFG
            } else {
                FLASHW_ALL
            },
            uCount: if urgent { 0 } else { 1 },
            dwTimeout: 0,
        };
        unsafe {
            // The return says whether the window *was* flashing before, which
            // is not a question anybody here is asking.
            let _ = FlashWindowEx(&raw const info);
        }
    }

    pub fn calm(window: RawWindow) {
        let info = FLASHWINFO {
            cbSize: std::mem::size_of::<FLASHWINFO>() as u32,
            hwnd: HWND(window as *mut _),
            dwFlags: FLASHW_STOP,
            uCount: 0,
            dwTimeout: 0,
        };
        unsafe {
            // The return says whether the window *was* flashing before, which
            // is not a question anybody here is asking.
            let _ = FlashWindowEx(&raw const info);
        }
    }
}

#[cfg(not(windows))]
mod platform {
    use super::RawWindow;
    use crate::badge::Overlay;

    /// No taskbar overlay exists off Windows.
    pub fn set_overlay(_window: RawWindow, _overlay: Option<&Overlay>, _description: &str) {}
    pub fn flash(_window: RawWindow, _urgent: bool) {}
    pub fn calm(_window: RawWindow) {}
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::badge;

    /// A badge computed before there is a window is not remembered.
    ///
    /// The first count is worked out while the sidebar is being built, which is
    /// before `resumed` has made a window. Recording it as shown would make
    /// every identical answer afterwards a no-op, and the badge would never
    /// appear -- which is exactly what happened.
    #[test]
    fn a_badge_with_no_window_to_go_on_is_not_remembered() {
        let mut taskbar = Taskbar::default();
        let dot = badge::wanted(0, true, 1.0);
        assert!(
            !taskbar.show(0, dot.clone(), "Unread messages"),
            "there is nowhere to put it"
        );
        // And once there is a window, the same badge is new again.
        assert!(taskbar.show(1, dot.clone(), "Unread messages"));
        assert!(!taskbar.show(1, dot, "Unread messages"), "but only once");
    }

    /// Quiet is a state like any other, and has to reach the shell once: a
    /// badge that is never taken off says something is always waiting.
    #[test]
    fn going_quiet_is_delivered_too() {
        let mut taskbar = Taskbar::default();
        assert!(taskbar.show(1, badge::wanted(2, true, 1.0), "2 messages want you"));
        assert!(taskbar.show(1, None, ""), "the badge comes off");
        assert!(!taskbar.show(1, None, ""));
    }

    /// The same badge is not sent twice.
    ///
    /// Every arriving message recomputes it and almost every one computes the
    /// same answer; without this each would create an icon, hand it to the
    /// shell and destroy it again, for no visible change.
    #[test]
    fn an_unchanged_badge_is_not_sent_again() {
        let mut taskbar = Taskbar::default();
        let three = badge::wanted(3, true, 1.0);
        assert!(taskbar.show(1, three.clone(), "3 messages want you"));
        assert!(!taskbar.show(1, three.clone(), "3 messages want you"));

        // A different count is a different badge.
        assert!(taskbar.show(1, badge::wanted(4, true, 1.0), "4 messages want you"));
        // And going quiet takes it off, which is a change like any other.
        assert!(taskbar.show(1, None, ""));
        assert!(!taskbar.show(1, None, ""));
    }

    /// The flashing is stopped once, not on every frame the window is focused.
    #[test]
    fn the_flashing_is_only_stopped_when_it_was_started() {
        let mut taskbar = Taskbar::default();
        assert!(!taskbar.flashing);
        taskbar.calm(1);
        taskbar.ask_for_attention(1, true);
        assert!(taskbar.flashing);
        taskbar.calm(1);
        assert!(!taskbar.flashing, "and it knows it has stopped");
    }
}
