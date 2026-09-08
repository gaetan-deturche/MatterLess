//! The taskbar button: flashing, and the overlay badge.
//!
//! Windows draws a small overlay in the corner of the taskbar icon
//! (`ITaskbarList3::SetOverlayIcon`, reached through Tauri's
//! `set_overlay_icon`) at the *small icon* size -- 16 physical pixels at 100%
//! scaling, 20 at 125%, 24 at 150%, 32 at 200%. The badge is built at exactly
//! that size: handing Windows a bigger bitmap looks like free quality and is the
//! opposite, since it then arrives through a filtered downscale that softens
//! every edge.
//!
//! **The number is set in Lato SemiBold, rasterised by GDI at the exact pixel
//! size it will be shown at.** Three hand-drawn bitmap faces came first and were
//! the wrong answer twice over: the digits were amateur (a "1" that read as an
//! "I"), and picking a different face per icon size meant the same number
//! changed shape with the DPI. A real font hinted onto the pixel grid is what
//! the official client gets from Skia -- and it is one typeface at every size.
//!
//! The font travels inside the binary (`include_bytes!`) and is registered with
//! GDI from memory, so it needs no resource path, cannot go missing between a
//! dev run and an installed build, and never joins the user's font list.
//!
//! Two states, matching what a person needs to know at a glance:
//!
//! * **a plain dot** -- something unread, nothing demanding attention
//! * **a number** -- messages that named you, plus everything in channels you
//!   asked to be told about (`desktop: all`)
//!
//! Muted channels contribute to neither: muting is a statement that this
//! channel should not interrupt.

use tauri::image::Image;

/// The overlay at 100% scaling. Everything else follows the window's scale
/// factor, so the badge is native at any DPI.
pub const BASE_SIZE: u32 = 16;

/// Fraction of the icon the badge disc spans.
const BADGE_RADIUS: f32 = 0.47;
/// The dot says less, so it takes less room.
const DOT_RADIUS: f32 = 0.31;
/// How much of the icon a number may span. Room either side is what makes the
/// disc read as a badge rather than a coloured square.
const MAX_TEXT_HEIGHT: f32 = 0.62;
/// Below this a digit is a smudge, whatever the geometry says: fitting inside
/// the disc and being readable are different tests, and text that fails this one
/// is dropped rather than drawn.
const MIN_TEXT_HEIGHT: i32 = 7;

struct Canvas {
    size: u32,
    pixels: Vec<u8>,
}

impl Canvas {
    fn new(size: u32) -> Self {
        Self {
            size,
            pixels: vec![0; (size * size * 4) as usize],
        }
    }

    fn centre(&self) -> f32 {
        self.size as f32 / 2.0
    }

    /// Paints `colour` over whatever is there, `coverage` deciding how much of
    /// it lands. Blending rather than replacing keeps a white digit's edge soft
    /// against the amber disc under it.
    fn blend(&mut self, x: i32, y: i32, colour: [u8; 3], coverage: f32) {
        if coverage <= 0.0 || x < 0 || y < 0 || x >= self.size as i32 || y >= self.size as i32 {
            return;
        }
        let coverage = coverage.min(1.0);
        let index = ((y as u32 * self.size + x as u32) * 4) as usize;
        let pixel = &mut self.pixels[index..index + 4];
        for channel in 0..3 {
            let under = f32::from(pixel[channel]);
            let over = f32::from(colour[channel]);
            pixel[channel] = (over * coverage + under * (1.0 - coverage)).round() as u8;
        }
        let alpha = f32::from(pixel[3]) / 255.0;
        pixel[3] = ((coverage + alpha * (1.0 - coverage)) * 255.0).round() as u8;
    }

    /// A filled circle. Coverage is sampled 4x4 per pixel: a circle is the one
    /// thing here that needs antialiasing, since no radius makes a round shape
    /// land on pixel boundaries.
    fn disc(&mut self, radius: f32, colour: [u8; 3]) {
        const SAMPLES: i32 = 4;
        let centre = self.centre();
        for y in 0..self.size as i32 {
            for x in 0..self.size as i32 {
                let mut inside = 0;
                for sub_y in 0..SAMPLES {
                    for sub_x in 0..SAMPLES {
                        let dx = x as f32 + (sub_x as f32 + 0.5) / SAMPLES as f32 - centre;
                        let dy = y as f32 + (sub_y as f32 + 0.5) / SAMPLES as f32 - centre;
                        if dx * dx + dy * dy <= radius * radius {
                            inside += 1;
                        }
                    }
                }
                self.blend(x, y, colour, inside as f32 / (SAMPLES * SAMPLES) as f32);
            }
        }
    }

    /// Stamps a rasterised glyph run, centred on its ink.
    ///
    /// Centring on the ink rather than the font's line box matters here: a line
    /// box carries room for accents and descenders that digits never use, and a
    /// number sitting high in its badge looks broken.
    fn stamp(&mut self, ink: &Ink, colour: [u8; 3]) {
        let left = ((self.size as i32 - ink.width) as f32 / 2.0).round() as i32;
        let top = ((self.size as i32 - ink.height) as f32 / 2.0).round() as i32;
        for y in 0..ink.height {
            for x in 0..ink.width {
                let coverage = f32::from(ink.coverage[(y * ink.width + x) as usize]) / 255.0;
                self.blend(left + x, top + y, colour, coverage);
            }
        }
    }

    fn into_image(self) -> Image<'static> {
        Image::new_owned(self.pixels, self.size, self.size)
    }
}

/// A rasterised run of text, cropped to its ink: one coverage byte per pixel.
pub struct Ink {
    width: i32,
    height: i32,
    coverage: Vec<u8>,
}

impl Ink {
    /// Does this fit inside a disc of `radius` centred on the box?
    ///
    /// Checked at the corners because the badge is round: a wide number reaches
    /// the rim at its corners long before its middle does.
    fn fits(&self, radius: f32) -> bool {
        let half_width = self.width as f32 / 2.0;
        let half_height = self.height as f32 / 2.0;
        half_width * half_width + half_height * half_height <= radius * radius
    }
}

/// Rasterises text with the system UI font, hinted onto the pixel grid.
#[cfg(windows)]
mod system_font {
    use super::Ink;
    use std::ffi::c_void;
    use windows::Win32::Foundation::{COLORREF, SIZE};
    use windows::Win32::Graphics::Gdi::{
        ANTIALIASED_QUALITY, AddFontMemResourceEx, BI_RGB, BITMAPINFO, BITMAPINFOHEADER,
        CLIP_DEFAULT_PRECIS, CreateCompatibleDC, CreateDIBSection, CreateFontW, DEFAULT_CHARSET,
        DEFAULT_PITCH, DIB_RGB_COLORS, DeleteDC, DeleteObject, FF_DONTCARE, FW_NORMAL,
        GetTextExtentPoint32W, NONANTIALIASED_QUALITY, OUT_TT_PRECIS, SelectObject, SetBkMode,
        SetTextColor, TRANSPARENT, TextOutW,
    };
    use windows::core::{PCWSTR, w};

    /// SIL Open Font Licence, from the google/fonts repository; the licence
    /// travels beside it in `resources/fonts/OFL.txt`.
    const LATO_SEMIBOLD: &[u8] = include_bytes!("../resources/fonts/Lato-SemiBold.ttf");

    /// The face's own name. Its weight is in the face, so GDI is asked for a
    /// normal weight -- asking for 600 as well makes it synthesise a second
    /// helping of boldness on top.
    const FACE: PCWSTR = w!("Lato SemiBold");

    /// Falls back to the Windows UI font, which is always present.
    const FALLBACK_FACE: PCWSTR = w!("Segoe UI");

    /// Registers the bundled font with GDI, once per process.
    ///
    /// `AddFontMemResourceEx` keeps it private to this process, so nothing here
    /// touches the user's installed fonts. The handle is deliberately never
    /// released: it has to outlive every badge the process draws.
    fn bundled_font_registered() -> bool {
        static REGISTERED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
        *REGISTERED.get_or_init(|| {
            let mut faces: u32 = 0;
            let handle = unsafe {
                AddFontMemResourceEx(
                    LATO_SEMIBOLD.as_ptr() as *const c_void,
                    LATO_SEMIBOLD.len() as u32,
                    None,
                    &raw mut faces,
                )
            };
            let ok = !handle.is_invalid() && faces > 0;
            if ok {
                tracing::debug!(faces, "registered the bundled badge font");
            } else {
                tracing::warn!("could not register Lato; badges fall back to Segoe UI");
            }
            ok
        })
    }

    /// The face to ask GDI for.
    fn face() -> PCWSTR {
        if bundled_font_registered() {
            FACE
        } else {
            FALLBACK_FACE
        }
    }

    /// How glyph edges are resolved.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum Edges {
        /// Greyscale antialiasing, as every other UI surface uses. Deliberately
        /// not ClearType: that spreads a stem across the red, green and blue
        /// subpixels, and collapsing those three to one coverage byte -- which a
        /// white-on-any-colour badge has to -- leaves the digits mottled.
        Smooth,
        /// Hard pixels: sharper, at the cost of ragged curves. Reached only by
        /// the preview dump, which exists to make this comparable by eye.
        #[allow(dead_code)]
        Hard,
    }

    /// Rasterises `text` at `em` pixels, cropped to its ink.
    ///
    /// `em` is the font's em size, not the height of the result -- digits come
    /// out around 70% of it -- which is why callers search for a size rather
    /// than computing one.
    pub fn rasterise(text: &str, em: i32, edges: Edges) -> Option<Ink> {
        if text.is_empty() || em <= 0 {
            return None;
        }
        // Generous, since the em size is not the drawn size.
        let box_size = (em * 3).max(8);
        let wide: Vec<u16> = text.encode_utf16().collect();

        unsafe {
            let dc = CreateCompatibleDC(None);
            if dc.is_invalid() {
                return None;
            }

            let quality = match edges {
                Edges::Smooth => ANTIALIASED_QUALITY,
                Edges::Hard => NONANTIALIASED_QUALITY,
            };
            let font = CreateFontW(
                -em, // negative: em size rather than cell height
                0,
                0,
                0,
                FW_NORMAL.0 as i32,
                0,
                0,
                0,
                DEFAULT_CHARSET,
                OUT_TT_PRECIS,
                CLIP_DEFAULT_PRECIS,
                quality,
                u32::from(DEFAULT_PITCH.0 | FF_DONTCARE.0),
                face(),
            );
            if font.is_invalid() {
                let _ = DeleteDC(dc);
                return None;
            }

            let header = BITMAPINFO {
                bmiHeader: BITMAPINFOHEADER {
                    biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                    biWidth: box_size,
                    // Negative: top-down rows, matching every other buffer here.
                    biHeight: -box_size,
                    biPlanes: 1,
                    biBitCount: 32,
                    biCompression: BI_RGB.0,
                    ..Default::default()
                },
                ..Default::default()
            };
            let mut bits: *mut c_void = std::ptr::null_mut();
            let bitmap = CreateDIBSection(
                Some(dc),
                &raw const header,
                DIB_RGB_COLORS,
                &raw mut bits,
                None,
                0,
            );
            let Ok(bitmap) = bitmap else {
                let _ = DeleteObject(font.into());
                let _ = DeleteDC(dc);
                return None;
            };

            let previous_bitmap = SelectObject(dc, bitmap.into());
            let previous_font = SelectObject(dc, font.into());
            // The DIB starts zeroed, so white on a transparent background gives
            // coverage directly in any channel.
            SetBkMode(dc, TRANSPARENT);
            SetTextColor(dc, COLORREF(0x00FF_FFFF));

            let mut extent = SIZE::default();
            let measured = GetTextExtentPoint32W(dc, &wide, &raw mut extent).as_bool();
            let drawn = measured && TextOutW(dc, 0, 0, &wide).as_bool();

            let ink = if drawn && !bits.is_null() {
                let pixels = std::slice::from_raw_parts(
                    bits as *const u8,
                    (box_size * box_size * 4) as usize,
                );
                crop(pixels, box_size)
            } else {
                None
            };

            SelectObject(dc, previous_font);
            SelectObject(dc, previous_bitmap);
            let _ = DeleteObject(font.into());
            let _ = DeleteObject(bitmap.into());
            let _ = DeleteDC(dc);
            ink
        }
    }

    /// Crops a rasterised box to the pixels that actually got ink.
    fn crop(pixels: &[u8], box_size: i32) -> Option<Ink> {
        let coverage_at = |x: i32, y: i32| {
            let index = ((y * box_size + x) * 4) as usize;
            // Any channel: the text was drawn white.
            pixels[index].max(pixels[index + 1]).max(pixels[index + 2])
        };

        let mut bounds = (box_size, box_size, -1, -1);
        for y in 0..box_size {
            for x in 0..box_size {
                if coverage_at(x, y) > 0 {
                    bounds.0 = bounds.0.min(x);
                    bounds.1 = bounds.1.min(y);
                    bounds.2 = bounds.2.max(x);
                    bounds.3 = bounds.3.max(y);
                }
            }
        }
        let (left, top, right, bottom) = bounds;
        if right < left || bottom < top {
            return None;
        }
        let width = right - left + 1;
        let height = bottom - top + 1;
        let mut coverage = Vec::with_capacity((width * height) as usize);
        for y in top..=bottom {
            for x in left..=right {
                coverage.push(coverage_at(x, y));
            }
        }
        Some(Ink {
            width,
            height,
            coverage,
        })
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        /// A silent fall back to Segoe UI would pass every other test in this
        /// file, so the registration itself has to be asserted.
        #[test]
        fn the_bundled_font_registers_and_is_the_one_used() {
            assert!(
                LATO_SEMIBOLD.starts_with(&[0x00, 0x01, 0x00, 0x00]),
                "the bundled file is not a TrueType font"
            );
            assert!(
                bundled_font_registered(),
                "fell back to the system font: Lato did not register"
            );
        }
    }
}

#[cfg(not(windows))]
mod system_font {
    use super::Ink;

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum Edges {
        Smooth,
        Hard,
    }

    /// No overlay badge exists off Windows, so there is nothing to rasterise.
    pub fn rasterise(_text: &str, _em: i32, _edges: Edges) -> Option<Ink> {
        None
    }
}

pub use system_font::Edges;

/// How badge numbers are drawn. Antialiased edges are what every other UI
/// surface uses; `Hard` exists because at 16 pixels the trade is arguable.
const EDGES: Edges = Edges::Smooth;

/// The tallest a number may be drawn in an icon of this size, in pixels.
///
/// Rounded once, here, so the fit search and the tests that check it cannot
/// disagree by a pixel over the same fraction.
fn tallest_text(size: u32) -> i32 {
    (size as f32 * MAX_TEXT_HEIGHT).round() as i32
}

/// The largest rendering of `text` that fits inside the badge.
///
/// Searching beats computing: the em size a font is asked for is not the height
/// it draws, and how a stem lands on the pixel grid depends on the hinting, so
/// the only honest measure of "does this fit" is a rasterised glyph.
fn best_fit(text: &str, size: u32, edges: Edges) -> Option<Ink> {
    let radius = size as f32 * BADGE_RADIUS;
    let tallest = tallest_text(size);
    // A digit is roughly 70% of its em size, so start above the target and walk
    // down: the first fit is the largest.
    let start = ((tallest as f32 / 0.7).round() as i32).max(4);
    (4..=start)
        .rev()
        .filter_map(|em| system_font::rasterise(text, em, edges))
        .find(|ink| ink.height <= tallest && ink.fits(radius))
        .filter(|ink| ink.height >= MIN_TEXT_HEIGHT.min(tallest))
}

/// Unread, but nothing that named you: a quiet dot.
pub fn dot(size: u32) -> Image<'static> {
    let mut canvas = Canvas::new(size);
    // The plan's signal teal, so the badge belongs to the same palette.
    canvas.disc(size as f32 * DOT_RADIUS, [75, 203, 236]);
    canvas.into_image()
}

/// A count of things that want an answer. Amber, because it is not the same
/// kind of information as the dot.
pub fn count(value: i64, size: u32) -> Image<'static> {
    badge(value, size, EDGES)
}

fn badge(value: i64, size: u32, edges: Edges) -> Image<'static> {
    let mut canvas = Canvas::new(size);
    canvas.disc(size as f32 * BADGE_RADIUS, [224, 108, 61]);
    if let Some(ink) = drawn_form(value, size, edges) {
        canvas.stamp(&ink, [255, 255, 255]);
    }
    canvas.into_image()
}

/// What `count` will draw: the number, clamped to two digits.
///
/// Clamping beats "99+": the plus needs a third character, which at 16 pixels
/// forces the whole number down to a smudge -- and once it is unreadable it says
/// less than a plain 99 does. Nothing is drawn if even that will not fit, since
/// the disc alone still says "something wants you".
fn drawn_form(value: i64, size: u32, edges: Edges) -> Option<Ink> {
    best_fit(&value.clamp(0, 99).to_string(), size, edges)
}

/// The overlay size for a window's DPI: Windows asks for a small icon, which is
/// 16 logical pixels.
pub fn size_for_scale(scale_factor: f64) -> u32 {
    let scaled = (BASE_SIZE as f64 * scale_factor).round();
    (scaled as u32).clamp(BASE_SIZE, 64)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn opaque_pixels(canvas: &Canvas) -> usize {
        canvas.pixels.chunks(4).filter(|pixel| pixel[3] > 0).count()
    }

    #[test]
    fn a_fresh_canvas_is_fully_transparent() {
        let canvas = Canvas::new(BASE_SIZE);
        assert_eq!(canvas.pixels.len(), (BASE_SIZE * BASE_SIZE * 4) as usize);
        assert_eq!(opaque_pixels(&canvas), 0);
    }

    #[test]
    fn the_overlay_size_follows_the_dpi() {
        assert_eq!(size_for_scale(1.0), 16);
        assert_eq!(size_for_scale(1.25), 20);
        assert_eq!(size_for_scale(1.5), 24);
        assert_eq!(size_for_scale(2.0), 32);
        // A wild scale factor must not produce a giant or empty icon.
        assert_eq!(size_for_scale(0.1), 16);
        assert_eq!(size_for_scale(9.0), 64);
    }

    #[test]
    fn the_dot_is_a_disc_that_fits_inside_the_icon() {
        let mut canvas = Canvas::new(BASE_SIZE);
        canvas.disc(BASE_SIZE as f32 * DOT_RADIUS, [1, 2, 3]);
        let painted = opaque_pixels(&canvas);
        assert!(painted > 50, "too small to read as a badge: {painted}");
        assert!(
            painted < (BASE_SIZE * BASE_SIZE) as usize,
            "must not fill the icon"
        );
        // Corners stay clear, so it looks round rather than square.
        assert_eq!(canvas.pixels[3], 0, "top-left corner must be transparent");
    }

    #[cfg(windows)]
    mod on_windows {
        use super::*;

        #[test]
        fn the_system_font_rasterises_a_digit() {
            let ink = system_font::rasterise("1", 12, Edges::Smooth).expect("GDI must draw a 1");
            assert!(ink.width > 0 && ink.height > 0);
            assert!(
                ink.coverage.iter().any(|value| *value > 128),
                "all faint edges, so nothing was really drawn"
            );
            assert_eq!(ink.coverage.len(), (ink.width * ink.height) as usize);
        }

        /// The em size a font is asked for is not the height it draws, which is
        /// the relationship the fit search leans on.
        #[test]
        fn a_digit_is_shorter_than_the_em_it_was_asked_for() {
            let ink = system_font::rasterise("8", 20, Edges::Smooth).unwrap();
            assert!(
                ink.height < 20,
                "an 8 drawn at em 20 came out {} tall",
                ink.height
            );
            assert!(ink.height > 10, "suspiciously small: {}", ink.height);
        }

        /// One typeface at every size: three hand-drawn faces made the same
        /// number change shape with the DPI.
        #[test]
        fn a_bigger_icon_draws_the_same_digit_bigger() {
            let mut previous = 0;
            for size in [16, 20, 24, 32] {
                let ink = best_fit("7", size, Edges::Smooth).expect("a digit must always fit");
                assert!(
                    ink.height >= previous,
                    "text shrank from {previous} to {} going up to {size}px",
                    ink.height
                );
                assert!(
                    ink.height <= tallest_text(size),
                    "text spans {} of {size}px, over the {} cap",
                    ink.height,
                    tallest_text(size)
                );
                previous = ink.height;
            }
        }

        #[test]
        fn every_count_stays_inside_the_disc_at_every_dpi() {
            for size in [16, 20, 24, 32] {
                let radius = size as f32 * BADGE_RADIUS;
                for value in [1, 7, 10, 42, 99, 100, 4321] {
                    let ink = drawn_form(value, size, Edges::Smooth)
                        .unwrap_or_else(|| panic!("{value} is undrawable at {size}px"));
                    assert!(
                        ink.fits(radius),
                        "{value} is {}x{} at {size}px, outside the disc",
                        ink.width,
                        ink.height
                    );
                }
            }
        }

        /// A count over 99 is clamped rather than turned into "99+": the plus
        /// costs a third character, and at 16 pixels that shrinks the number
        /// below reading size.
        #[test]
        fn a_big_count_is_clamped_to_two_digits() {
            let clamped = drawn_form(4321, BASE_SIZE, Edges::Smooth).unwrap();
            let ninety_nine = drawn_form(99, BASE_SIZE, Edges::Smooth).unwrap();
            assert_eq!(
                (clamped.width, clamped.height),
                (ninety_nine.width, ninety_nine.height),
                "4321 must be drawn exactly as 99 is"
            );
            assert!(
                clamped.height >= MIN_TEXT_HEIGHT,
                "the clamped number is unreadable at {} tall",
                clamped.height
            );
            let single = drawn_form(7, BASE_SIZE, Edges::Smooth).unwrap();
            assert!(
                clamped.width > single.width,
                "two digits are wider than one"
            );
        }

        #[test]
        fn the_number_is_centred_on_its_ink() {
            let mut canvas = Canvas::new(32);
            let ink = best_fit("1", 32, Edges::Smooth).unwrap();
            canvas.stamp(&ink, [255, 255, 255]);
            let mut bounds: (i32, i32, i32, i32) = (32, 32, -1, -1);
            for y in 0..32 {
                for x in 0..32 {
                    if canvas.pixels[((y * 32 + x) * 4 + 3) as usize] > 0 {
                        bounds.0 = bounds.0.min(x);
                        bounds.1 = bounds.1.min(y);
                        bounds.2 = bounds.2.max(x);
                        bounds.3 = bounds.3.max(y);
                    }
                }
            }
            let (left, top, right, bottom) = bounds;
            assert!(
                (left - (31 - right)).abs() <= 1,
                "not centred across: {left} vs {right}"
            );
            assert!(
                (top - (31 - bottom)).abs() <= 1,
                "not centred down: {top} vs {bottom}"
            );
        }

        #[test]
        fn a_digit_paints_inside_its_disc() {
            let mut canvas = Canvas::new(BASE_SIZE);
            canvas.disc(BASE_SIZE as f32 * BADGE_RADIUS, [10, 10, 10]);
            let background = opaque_pixels(&canvas);
            let ink = drawn_form(3, BASE_SIZE, Edges::Smooth).unwrap();
            canvas.stamp(&ink, [255, 255, 255]);
            // The glyph sits on the disc, so the opaque count cannot grow.
            assert_eq!(opaque_pixels(&canvas), background);
        }

        #[test]
        fn a_large_count_becomes_ninety_nine_plus() {
            let image = count(347, BASE_SIZE);
            assert_eq!(image.width(), BASE_SIZE);
            assert_eq!(image.height(), BASE_SIZE);
        }

        /// Writes the real badges out as raw RGBA so they can be looked at,
        /// since "does this digit read correctly" is not a thing an assertion
        /// can answer. A small script wraps these into a PNG to look at.
        #[test]
        #[ignore = "writes files for a human to look at"]
        fn dump_previews() {
            let out = std::env::var("BADGE_DUMP_DIR").expect("set BADGE_DUMP_DIR");
            let mut manifest = String::new();
            for (label, edges) in [("smooth", Edges::Smooth), ("hard", Edges::Hard)] {
                for size in [BASE_SIZE, 24, 32] {
                    let mut images = vec![(format!("{label}_{size}_dot"), dot(size))];
                    for value in [1, 7, 12, 99, 347] {
                        images.push((
                            format!("{label}_{size}_count_{value}"),
                            badge(value, size, edges),
                        ));
                    }
                    for (name, image) in images {
                        std::fs::write(format!("{out}/{name}.rgba"), image.rgba())
                            .expect("write preview");
                        manifest.push_str(&format!(
                            "{name} {} {}\n",
                            image.width(),
                            image.height()
                        ));
                    }
                }
            }
            std::fs::write(format!("{out}/manifest.txt"), manifest).expect("write manifest");
        }
    }
}
