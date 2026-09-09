//! The message list as a native surface inside the app's window.
//!
//! The webview fills the window and the list sits on top of it, in a child
//! window positioned over a rectangle the page leaves empty. Everything else --
//! sidebar, header, composer, every panel -- stays HTML, because none of it had
//! the problem this is here to solve.
//!
//! That problem was never speed. It was that a browser reports a row's height
//! only after laying it out, so the virtualiser had to predict heights and then
//! reconcile them a frame later; opening the thread pane changed every height at
//! once and the reconciliation was visible as a jump. Here the heights are
//! computed before anything is drawn, so there is nothing to reconcile.
//!
//! Windows only, and openly so: `cfg(windows)` guards the whole module, and the
//! app runs without it exactly as it did before.

#![cfg(windows)]

use matterless_layout::Fonts;
use matterless_layout::row::{RowLayout, Theme, lay_out};
use matterless_paint::{Painter, Palette};
use matterless_render::Row;
use raw_window_handle::{
    RawDisplayHandle, RawWindowHandle, Win32WindowHandle, WindowsDisplayHandle,
};
use std::num::NonZeroIsize;
use std::sync::Arc;
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::Graphics::Gdi::HBRUSH;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, HCURSOR, HICON, HMENU, RegisterClassW, SW_HIDE, SW_SHOWNA,
    SetWindowPos, ShowWindow, WNDCLASSW, WS_CHILD, WS_CLIPSIBLINGS, WS_EX_NOREDIRECTIONBITMAP,
    WS_EX_TRANSPARENT, WS_VISIBLE,
};
use windows::core::PCWSTR;

/// The class is registered once per process; registering it twice fails, and
/// the second window would then never appear.
static CLASS: std::sync::OnceLock<Vec<u16>> = std::sync::OnceLock::new();

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

/// Nothing to handle yet: the page still owns input, and the surface only draws.
unsafe extern "system" fn proc(window: HWND, message: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    unsafe { DefWindowProcW(window, message, w, l) }
}

fn class_name() -> PCWSTR {
    let name = CLASS.get_or_init(|| {
        let name = wide("MatterLessList");
        let class = WNDCLASSW {
            lpfnWndProc: Some(proc),
            hInstance: unsafe { GetModuleHandleW(None) }.unwrap_or_default().into(),
            lpszClassName: PCWSTR(name.as_ptr()),
            hIcon: HICON::default(),
            hCursor: HCURSOR::default(),
            hbrBackground: HBRUSH::default(),
            ..Default::default()
        };
        unsafe { RegisterClassW(&class) };
        name
    });
    PCWSTR(name.as_ptr())
}

/// Where the list sits, in physical pixels inside the window.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Bounds {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

/// The native list: its window, its GPU surface, and the rows it draws.
pub struct NativeList {
    child: HWND,
    surface: wgpu::Surface<'static>,
    format: wgpu::TextureFormat,
    view: matterless_view::View,
    fonts: Fonts,
    painter: Painter,
    theme: Theme,
    palette: Palette,
    laid: Vec<RowLayout>,
    bounds: Bounds,
    scroll: f32,
}

// The child window handle is only ever touched from the thread that owns it;
// the surface and device are `Send` in wgpu's own right.
unsafe impl Send for NativeList {}

impl NativeList {
    /// Creates the child window over `parent` and a Vulkan surface on it.
    pub fn new(parent: isize) -> Result<Self, String> {
        let parent = HWND(parent as *mut std::ffi::c_void);
        let child = unsafe {
            CreateWindowExW(
                // No redirection bitmap: the surface presents straight to the
                // compositor, which is what avoids a copy per frame.
                //
                // Transparent to the mouse as well, so every click and wheel
                // goes to the webview underneath as it did before. The page
                // still owns input and forwards scrolling; giving this window
                // its own input handling means rebuilding hit testing,
                // selection and hover, which is a later stage and not one to
                // start by accident.
                WS_EX_NOREDIRECTIONBITMAP | WS_EX_TRANSPARENT,
                class_name(),
                PCWSTR::null(),
                // Clipped against its siblings, so the webview underneath is
                // not drawn over the top of it.
                WS_CHILD | WS_VISIBLE | WS_CLIPSIBLINGS,
                0,
                0,
                1,
                1,
                Some(parent),
                None::<HMENU>,
                None,
                None,
            )
        }
        .map_err(|error| format!("create the list window: {error}"))?;

        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
            backends: wgpu::Backends::VULKAN,
            ..Default::default()
        });
        let mut handle = Win32WindowHandle::new(
            NonZeroIsize::new(child.0 as isize).ok_or("the list window has no handle")?,
        );
        handle.hinstance = NonZeroIsize::new(
            unsafe { GetModuleHandleW(None) }
                .map(|module| module.0 as isize)
                .unwrap_or(0),
        );
        let target = wgpu::SurfaceTargetUnsafe::RawHandle {
            raw_display_handle: RawDisplayHandle::Windows(WindowsDisplayHandle::new()),
            raw_window_handle: RawWindowHandle::Win32(handle),
        };
        // SAFETY: the handle belongs to the window created above, which this
        // struct owns and outlives the surface.
        let surface = unsafe { instance.create_surface_unsafe(target) }
            .map_err(|error| format!("surface: {error}"))?;
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::LowPower,
            compatible_surface: Some(&surface),
            force_fallback_adapter: false,
        }))
        .map_err(|error| format!("no Vulkan adapter: {error}"))?;
        let (device, queue) =
            pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default()))
                .map_err(|error| format!("device: {error}"))?;
        let format = surface.get_capabilities(&adapter).formats[0];
        tracing::info!(
            adapter = %adapter.get_info().name,
            backend = ?adapter.get_info().backend,
            "native list surface created"
        );

        Ok(Self {
            child,
            surface,
            format,
            view: matterless_view::View::new(device, queue, format),
            fonts: Fonts::new(),
            painter: Painter::new(),
            theme: Theme::default(),
            palette: Palette::default(),
            laid: Vec::new(),
            bounds: Bounds::default(),
            scroll: 0.0,
        })
    }

    /// Moves the surface to the rectangle the page reserved for it.
    ///
    /// Re-laying out here rather than per frame: the heights depend on the width
    /// and nothing else, so this is the only moment they can change.
    pub fn place(&mut self, bounds: Bounds, rows: Option<&[Row]>) {
        let resized = bounds.width != self.bounds.width;
        self.bounds = bounds;
        unsafe {
            let _ = SetWindowPos(
                self.child,
                None,
                bounds.x,
                bounds.y,
                bounds.width.max(1),
                bounds.height.max(1),
                Default::default(),
            );
        }
        self.surface.configure(
            &self.view.device,
            &wgpu::SurfaceConfiguration {
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                format: self.format,
                width: bounds.width.max(1) as u32,
                height: bounds.height.max(1) as u32,
                present_mode: wgpu::PresentMode::AutoVsync,
                alpha_mode: wgpu::CompositeAlphaMode::Auto,
                view_formats: Vec::new(),
                desired_maximum_frame_latency: 2,
            },
        );
        if let Some(rows) = rows {
            self.relayout(rows);
        } else if resized {
            // The rows are unchanged, but their heights are not.
            let held = std::mem::take(&mut self.laid);
            self.laid = held;
        }
    }

    /// Lays out the rows for the current width.
    pub fn relayout(&mut self, rows: &[Row]) {
        self.theme = Theme {
            width: self.bounds.width.max(1) as f32,
            ..Theme::default()
        };
        self.laid = rows
            .iter()
            .map(|row| lay_out(&mut self.fonts, row, &self.theme))
            .collect();
        self.clamp();
    }

    fn clamp(&mut self) {
        let total: f32 = self.laid.iter().map(|row| row.height).sum();
        let reach = (total - self.bounds.height as f32).max(0.0);
        self.scroll = self.scroll.clamp(0.0, reach);
    }

    pub fn scroll_by(&mut self, delta: f32) {
        self.scroll -= delta;
        self.clamp();
    }

    /// Puts the reader at the newest message, which is where a channel opens.
    pub fn jump_to_newest(&mut self) {
        let total: f32 = self.laid.iter().map(|row| row.height).sum();
        self.scroll = (total - self.bounds.height as f32).max(0.0);
    }

    pub fn show(&self, visible: bool) {
        unsafe {
            // `SW_SHOWNA` rather than `SW_SHOW`: showing a child must not take
            // the focus off whatever the reader was typing in.
            let _ = ShowWindow(self.child, if visible { SW_SHOWNA } else { SW_HIDE });
        }
    }

    /// Draws one frame.
    pub fn render(&mut self) {
        if self.bounds.width <= 0 || self.bounds.height <= 0 {
            return;
        }
        let Ok(frame) = self.surface.get_current_texture() else {
            return;
        };
        let target = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        self.view.draw(
            &target,
            &mut self.fonts,
            &mut self.painter,
            &self.laid,
            self.scroll,
            (self.bounds.width as u32, self.bounds.height as u32),
            &self.theme,
            &self.palette,
        );
        frame.present();
    }
}

/// The list, once created, shared with the commands that drive it.
pub type Shared = Arc<std::sync::Mutex<Option<NativeList>>>;
