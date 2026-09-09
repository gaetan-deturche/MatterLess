//! Turns a laid-out message list into pixels.
//!
//! The layout crate answers where every line of every row sits. This draws it.
//! The two are separate on purpose: the layout is what the scroller needs and
//! can be checked without a GPU, and this is what the window needs.
//!
//! What it deliberately does *not* do is decide anything about size. Every
//! width, height and wrap comes from the layout, so the thing drawn and the
//! thing scrolled cannot disagree -- which is the whole reason for the port.
//!
//! The surface here is a plain CPU buffer. That is not the destination: it is
//! how the glyph positions get verified -- a snapshot can be looked at, and
//! asserted about in a test -- before the same draw list is handed to the GPU.

use cosmic_text::{Attrs, Buffer, Color, Family, Metrics, Shaping, SwashCache, Weight};
use matterless_layout::Fonts;
use matterless_layout::row::{Block, Kind, RowLayout, TextSpan, Theme};

/// A surface to draw on, in straight RGBA8.
pub struct Canvas {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
}

impl Canvas {
    pub fn new(width: u32, height: u32, background: [u8; 4]) -> Self {
        let mut pixels = Vec::with_capacity((width * height * 4) as usize);
        for _ in 0..(width * height) {
            pixels.extend_from_slice(&background);
        }
        Self {
            width,
            height,
            pixels,
        }
    }

    /// Blends one pixel by coverage, which is what a rasterised glyph gives:
    /// how much of this pixel the glyph covers, not what colour it is.
    fn blend(&mut self, x: i32, y: i32, colour: [u8; 3], coverage: u8) {
        if x < 0 || y < 0 || x >= self.width as i32 || y >= self.height as i32 || coverage == 0 {
            return;
        }
        let at = ((y as u32 * self.width + x as u32) * 4) as usize;
        let alpha = coverage as u32;
        for (channel, over) in colour.iter().enumerate() {
            let under = self.pixels[at + channel] as u32;
            let mixed = (*over as u32 * alpha + under * (255 - alpha)) / 255;
            self.pixels[at + channel] = mixed as u8;
        }
        self.pixels[at + 3] = 255;
    }

    fn fill(&mut self, x: i32, y: i32, width: i32, height: i32, colour: [u8; 4]) {
        for row in y..(y + height) {
            for column in x..(x + width) {
                if row < 0 || column < 0 || row >= self.height as i32 || column >= self.width as i32
                {
                    continue;
                }
                let at = ((row as u32 * self.width + column as u32) * 4) as usize;
                self.pixels[at..at + 4].copy_from_slice(&colour);
            }
        }
    }
}

/// The colours a row is drawn in. Separate from the layout's `Theme`, which
/// holds only what changes a size.
#[derive(Debug, Clone, Copy)]
pub struct Palette {
    pub ground: [u8; 4],
    pub ink: [u8; 3],
    pub faint: [u8; 3],
    /// Behind a code block.
    pub surface: [u8; 4],
}

impl Default for Palette {
    /// The app's dark theme.
    fn default() -> Self {
        Self {
            ground: [15, 20, 27, 255],
            ink: [223, 231, 240],
            faint: [138, 152, 168],
            surface: [23, 30, 39, 255],
        }
    }
}

/// Holds the rasterised glyphs between frames.
///
/// Rasterising is the expensive half -- shaping is cheap by comparison -- so the
/// cache is the thing that must outlive a frame.
pub struct Painter {
    glyphs: SwashCache,
}

impl Default for Painter {
    fn default() -> Self {
        Self::new()
    }
}

impl Painter {
    pub fn new() -> Self {
        Self {
            glyphs: SwashCache::new(),
        }
    }

    /// Draws one laid-out row at `top`.
    ///
    /// Every measurement is the layout's. Nothing here decides how wide a line
    /// is or how many of them there are: this walks what was already decided.
    pub fn paint_row(
        &mut self,
        canvas: &mut Canvas,
        fonts: &mut Fonts,
        row: &RowLayout,
        top: f32,
        theme: &Theme,
        palette: &Palette,
    ) {
        for block in &row.blocks {
            let x = theme.gutter + block.x;
            let y = top + block.y;
            match block.kind {
                Kind::Separator => {
                    // A rule across the row, with the label's space left for it.
                    let middle = y + block.height / 2.0;
                    canvas.fill(
                        0,
                        middle as i32,
                        canvas.width as i32,
                        1,
                        [palette.faint[0], palette.faint[1], palette.faint[2], 255],
                    );
                }
                Kind::Header => {
                    // The avatar's square, so the gutter is visibly accounted
                    // for until faces are drawn.
                    canvas.fill(0, y as i32, 28, 28, palette.surface);
                }
                Kind::Code => {
                    canvas.fill(
                        x as i32,
                        y as i32,
                        (block.wrap) as i32,
                        block.height as i32,
                        palette.surface,
                    );
                    self.draw_spans(
                        canvas,
                        fonts,
                        Placed {
                            block,
                            x: x + 8.0,
                            y: y + 8.0,
                            theme,
                            ink: palette.ink,
                        },
                    );
                }
                Kind::Text => {
                    self.draw_spans(
                        canvas,
                        fonts,
                        Placed {
                            block,
                            x,
                            y,
                            theme,
                            ink: palette.ink,
                        },
                    );
                }
                Kind::Reactions | Kind::Attachment | Kind::Footer => {
                    if block.height >= 1.0 {
                        canvas.fill(
                            x as i32,
                            y as i32,
                            120,
                            (block.height - 4.0).max(1.0) as i32,
                            palette.surface,
                        );
                    }
                }
            }
        }
    }

    fn draw_spans(&mut self, canvas: &mut Canvas, fonts: &mut Fonts, at: Placed<'_>) {
        let Placed {
            block,
            x,
            y,
            theme,
            ink,
        } = at;
        if block.spans.is_empty() {
            return;
        }
        let line_height = if block.kind == Kind::Code {
            theme.code_line_height
        } else {
            theme.line_height
        };
        let metrics = Metrics::new(block.size, line_height);
        let mut buffer = Buffer::new(fonts.system_mut(), metrics);
        let mut shaped = buffer.borrow_with(fonts.system_mut());
        // The layout's own wrap width, not this canvas's: drawing to a different
        // width would break the line somewhere else and the row would no longer
        // be the height that was reserved for it.
        shaped.set_size(Some(block.wrap), None);
        let spans: Vec<(&str, Attrs<'static>)> = block
            .spans
            .iter()
            .map(|span| (span.text.as_str(), attrs_of(span)))
            .collect();
        shaped.set_rich_text(spans, &Attrs::new(), Shaping::Advanced, None);
        shaped.shape_until_scroll(false);

        let colour = Color::rgb(ink[0], ink[1], ink[2]);
        let runs: Vec<_> = shaped
            .layout_runs()
            .map(|run| (run.line_y, run.glyphs.to_vec()))
            .collect();
        for (line_y, glyphs) in runs {
            for glyph in &glyphs {
                let physical = glyph.physical((x, y + line_y), 1.0);
                self.glyphs.with_pixels(
                    fonts.system_mut(),
                    physical.cache_key,
                    colour,
                    |dx, dy, pixel| {
                        canvas.blend(physical.x + dx, physical.y + dy, ink, pixel.a());
                    },
                );
            }
        }
    }
}

/// Where a block's spans go and how they look. Together rather than as six
/// arguments, because they are one thing: a placed run of text.
struct Placed<'a> {
    block: &'a Block,
    x: f32,
    y: f32,
    theme: &'a Theme,
    ink: [u8; 3],
}

fn attrs_of(span: &TextSpan) -> Attrs<'static> {
    let mut attrs = Attrs::new();
    if span.mono {
        attrs = attrs.family(Family::Monospace);
    }
    if span.bold {
        attrs = attrs.weight(Weight::BOLD);
    }
    if span.italic {
        attrs = attrs.style(cosmic_text::Style::Italic);
    }
    attrs
}
