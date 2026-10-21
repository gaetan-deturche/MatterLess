//! A YouTube video, played in the viewer by YouTube's own embedded player.
//!
//! A WebView2 child of the window, over the viewer's picture: the window draws
//! the dim and the bar around it, and the web view the video. The player page
//! is served from a local origin rather than loaded bare, because YouTube's
//! embed refuses to play for a page with no origin to refer from.

use crate::youtube::Video;
use matterless_ui::Rect;

/// The origin the player page is served from.
const PAGE: &str = "https://matterless.player/watch";

/// Where an HTML attachment is served from.
const SHOWN: &str = "https://matterless.page/view";

/// The player, while a video is being watched.
pub struct Watching {
    view: wry::WebView,
    /// Where it is, so it is moved only when the viewer moves.
    at: Rect,
    /// Kept for as long as the view: WebView2 keeps its profile here.
    _context: wry::WebContext,
    pub id: String,
    /// Whether its page has loaded, and whether it has been shown since.
    ///
    /// Hidden until then: this window draws through Direct3D with no surface
    /// behind it, so a child window that has painted nothing yet is a hole
    /// showing whatever is behind the app.
    loaded: std::rc::Rc<std::cell::Cell<bool>>,
    shown: bool,
}

impl Watching {
    /// Opens the player on `video`, over `at` in the window's pixels.
    pub fn open(
        window: &impl wry::raw_window_handle::HasWindowHandle,
        video: &Video,
        at: Rect,
        profile: Option<std::path::PathBuf>,
    ) -> Result<Self, String> {
        let mut context = wry::WebContext::new(profile);
        let page = page_of(video);
        let loaded = std::rc::Rc::new(std::cell::Cell::new(false));
        let heard = loaded.clone();
        let url = format!("{PAGE}?v={}", video.id);
        let view = {
            use wry::WebViewBuilderExtWindows;
            wry::WebViewBuilder::new_with_web_context(&mut context)
                .with_https_scheme(true)
                .with_custom_protocol("matterless".to_string(), move |_, _| {
                    wry::http::Response::builder()
                        .header(wry::http::header::CONTENT_TYPE, "text/html")
                        .body(std::borrow::Cow::Owned(page.clone().into_bytes()))
                        .unwrap_or_default()
                })
                .with_url(url)
                .with_bounds(bounds(at))
                .with_visible(false)
                .with_on_page_load_handler(move |event, _| {
                    if matches!(event, wry::PageLoadEvent::Finished) {
                        heard.set(true);
                    }
                })
                .with_autoplay(true)
                .with_background_color((0, 0, 0, 255))
                .with_devtools(false)
                // Leaving the player -- the YouTube logo, "Watch on YouTube",
                // a related video -- goes to the browser rather than turning
                // this into one.
                .with_navigation_handler(|url| {
                    let ours = url.starts_with(PAGE);
                    if !ours {
                        crate::open::link(&url);
                    }
                    ours
                })
                .with_new_window_req_handler(|url, _| {
                    crate::open::link(&url);
                    wry::NewWindowResponse::Deny
                })
                .build_as_child(window)
                .map_err(|why| why.to_string())?
        };
        Ok(Self {
            view,
            at,
            _context: context,
            id: video.id.clone(),
            loaded,
            shown: false,
        })
    }

    /// Follows the viewer when it moves, which is a window being resized, and
    /// shows the player once its page has painted.
    pub fn place(&mut self, at: Rect) {
        if at != self.at {
            self.at = at;
            let _ = self.view.set_bounds(bounds(at));
        }
        self.reveal();
    }

    /// Shows the player once its page has loaded, and not before.
    pub fn reveal(&mut self) {
        if !self.shown && self.loaded.get() {
            self.shown = self.view.set_visible(true).is_ok();
        }
    }

    /// When the window should draw again to show it: soon, until it is shown,
    /// since its page loading is nothing the window would otherwise wake for.
    pub fn wakes(&self) -> Option<std::time::Instant> {
        (!self.shown).then(|| std::time::Instant::now() + std::time::Duration::from_millis(50))
    }
}

/// An HTML file somebody attached, shown as its page in the viewer.
///
/// In the system's web engine, scripts and all, as a browser would show it --
/// in a profile of its own, so nothing it stores reaches the video player's.
/// Its links go to the browser rather than turning the viewer into one.
pub struct Page {
    view: wry::WebView,
    at: Rect,
    _context: wry::WebContext,
    pub id: String,
    loaded: std::rc::Rc<std::cell::Cell<bool>>,
    shown: bool,
}

impl Page {
    /// Shows `html`, the file `id`, over `at` in the window's pixels.
    pub fn open(
        window: &impl wry::raw_window_handle::HasWindowHandle,
        id: &str,
        html: &str,
        at: Rect,
        profile: Option<std::path::PathBuf>,
    ) -> Result<Self, String> {
        let mut context = wry::WebContext::new(profile);
        let loaded = std::rc::Rc::new(std::cell::Cell::new(false));
        let heard = loaded.clone();
        // Served from a local origin rather than handed over as a string:
        // WebView2 takes a string of two megabytes at most, and a page with an
        // origin is one its own scripts can keep things for.
        let body = html.to_string();
        let view = {
            use wry::WebViewBuilderExtWindows;
            wry::WebViewBuilder::new_with_web_context(&mut context)
                .with_https_scheme(true)
                .with_custom_protocol("matterless".to_string(), move |_, _| {
                    wry::http::Response::builder()
                        .header(wry::http::header::CONTENT_TYPE, "text/html; charset=utf-8")
                        .body(std::borrow::Cow::Owned(body.clone().into_bytes()))
                        .unwrap_or_default()
                })
                .with_url(SHOWN)
                .with_bounds(bounds(at))
                .with_visible(false)
                .with_on_page_load_handler(move |event, _| {
                    if matches!(event, wry::PageLoadEvent::Finished) {
                        heard.set(true);
                    }
                })
                .with_background_color((255, 255, 255, 255))
                .with_devtools(false)
                .with_navigation_handler(|url| {
                    let ours = url.starts_with(SHOWN);
                    if !ours {
                        crate::open::link(&url);
                    }
                    ours
                })
                .with_new_window_req_handler(|url, _| {
                    crate::open::link(&url);
                    wry::NewWindowResponse::Deny
                })
                .build_as_child(window)
                .map_err(|why| why.to_string())?
        };
        Ok(Self {
            view,
            at,
            _context: context,
            id: id.to_string(),
            loaded,
            shown: false,
        })
    }

    /// Follows the viewer when it moves, and shows the page once painted.
    pub fn place(&mut self, at: Rect) {
        if at != self.at {
            self.at = at;
            let _ = self.view.set_bounds(bounds(at));
        }
        self.reveal();
    }

    /// Shows the page once it has loaded, and not before: until then it is a
    /// hole in a window drawn through Direct3D.
    pub fn reveal(&mut self) {
        if !self.shown && self.loaded.get() {
            self.shown = self.view.set_visible(true).is_ok();
        }
    }

    /// When the window should draw again to show it.
    pub fn wakes(&self) -> Option<std::time::Instant> {
        (!self.shown).then(|| std::time::Instant::now() + std::time::Duration::from_millis(50))
    }
}

/// In logical pixels, as the layout is: wry scales them by the window's DPI.
fn bounds(at: Rect) -> wry::Rect {
    wry::Rect {
        position: wry::dpi::LogicalPosition::new(at.x, at.y).into(),
        size: wry::dpi::LogicalSize::new(at.width.max(1.0), at.height.max(1.0)).into(),
    }
}

/// The page the player is in: YouTube's embed filling it, on black.
fn page_of(video: &Video) -> String {
    // A quiet run is somebody testing this window, and it must not play sound
    // at whoever is at the computer.
    let muted = u8::from(std::env::var_os("MATTERLESS_QUIET").is_some());
    format!(
        r#"<!doctype html><html><head><meta charset="utf-8">
<meta name="referrer" content="strict-origin-when-cross-origin">
<style>html,body{{margin:0;height:100%;background:#000;overflow:hidden}}
iframe{{border:0;width:100%;height:100%;display:block}}</style></head><body>
<iframe src="https://www.youtube-nocookie.com/embed/{id}?autoplay=1&mute={muted}&start={start}&rel=0"
 allow="autoplay; encrypted-media; picture-in-picture; fullscreen"
 referrerpolicy="strict-origin-when-cross-origin" allowfullscreen></iframe>
</body></html>"#,
        id = video.id,
        start = video.start,
    )
}
