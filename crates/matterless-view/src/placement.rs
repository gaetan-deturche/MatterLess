//! Where the window was and how big, kept across runs.
//!
//! Through the window placement Windows itself keeps, rather than a size and a
//! position read from winit: it holds the restored rectangle while the window
//! is maximised or minimised, and applying it pulls a window whose monitor has
//! gone back onto one that is still there.

use crate::taskbar::RawWindow;

/// The setting it is kept under.
pub const SETTING: &str = "window.placement";

/// One run's placement: the restored rectangle, and whether it was maximised.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Placement {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
    pub maximized: bool,
}

impl Placement {
    pub fn write(&self) -> String {
        format!(
            "{},{},{},{},{}",
            self.left,
            self.top,
            self.right,
            self.bottom,
            u8::from(self.maximized)
        )
    }

    /// `None` for anything but five numbers making a rectangle with room in it.
    pub fn read(text: &str) -> Option<Self> {
        let numbers: Vec<i32> = text
            .split(',')
            .map(|part| part.trim().parse().ok())
            .collect::<Option<_>>()?;
        let [left, top, right, bottom, maximized] = numbers[..] else {
            return None;
        };
        if right - left < 200 || bottom - top < 150 {
            return None;
        }
        Some(Self {
            left,
            top,
            right,
            bottom,
            maximized: maximized != 0,
        })
    }
}

/// The window's placement now.
#[cfg(windows)]
pub fn of(window: RawWindow) -> Option<Placement> {
    use windows::Win32::Foundation::HWND;
    use windows::Win32::UI::WindowsAndMessaging::{
        GetWindowPlacement, SW_SHOWMAXIMIZED, SW_SHOWMINIMIZED, WINDOWPLACEMENT,
        WPF_RESTORETOMAXIMIZED,
    };
    if window == 0 {
        return None;
    }
    let mut placement = WINDOWPLACEMENT {
        length: std::mem::size_of::<WINDOWPLACEMENT>() as u32,
        ..Default::default()
    };
    unsafe { GetWindowPlacement(HWND(window as *mut _), &mut placement) }.ok()?;
    let shown = placement.showCmd as i32;
    // Minimised, it is whatever it will come back as.
    let maximized = shown == SW_SHOWMAXIMIZED.0
        || (shown == SW_SHOWMINIMIZED.0 && placement.flags.contains(WPF_RESTORETOMAXIMIZED));
    let normal = placement.rcNormalPosition;
    Some(Placement {
        left: normal.left,
        top: normal.top,
        right: normal.right,
        bottom: normal.bottom,
        maximized,
    })
}

#[cfg(not(windows))]
pub fn of(_window: RawWindow) -> Option<Placement> {
    None
}

/// Puts a hidden window back where it was, and leaves it hidden: maximising is
/// the caller's, once it is shown.
#[cfg(windows)]
pub fn restore(window: RawWindow, placement: Placement) {
    use windows::Win32::Foundation::{HWND, RECT};
    use windows::Win32::UI::WindowsAndMessaging::{SW_HIDE, SetWindowPlacement, WINDOWPLACEMENT};
    if window == 0 {
        return;
    }
    let wanted = WINDOWPLACEMENT {
        length: std::mem::size_of::<WINDOWPLACEMENT>() as u32,
        showCmd: SW_HIDE.0 as u32,
        rcNormalPosition: RECT {
            left: placement.left,
            top: placement.top,
            right: placement.right,
            bottom: placement.bottom,
        },
        ..Default::default()
    };
    if let Err(error) = unsafe { SetWindowPlacement(HWND(window as *mut _), &wanted) } {
        eprintln!("restoring the window's place: {error}");
    }
}

#[cfg(not(windows))]
pub fn restore(_window: RawWindow, _placement: Placement) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_placement_reads_back_as_written() {
        let placement = Placement {
            left: -1600,
            top: 40,
            right: -200,
            bottom: 1000,
            maximized: true,
        };
        assert_eq!(Placement::read(&placement.write()), Some(placement));
    }

    #[test]
    fn nonsense_or_a_sliver_is_no_placement() {
        assert_eq!(Placement::read(""), None);
        assert_eq!(Placement::read("1,2,3"), None);
        assert_eq!(Placement::read("0,0,x,10,0"), None);
        assert_eq!(Placement::read("0,0,50,40,0"), None);
    }
}
