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
use std::collections::HashMap;

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

impl Palette {
    /// The quieter ink as something to fill with.
    ///
    /// A bar beside a quote is the same colour as the text it belongs to, and
    /// `ink`/`faint` are three channels because they colour glyphs while a
    /// fill takes four.
    pub fn faint_fill(&self) -> [u8; 4] {
        [self.faint[0], self.faint[1], self.faint[2], 255]
    }
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

/// One glyph, rasterised or not, at the place it belongs.
///
/// The unit both consumers share: the CPU snapshot rasterises it straight onto
/// a buffer, and the GPU renderer looks it up in an atlas and emits a quad.
/// Neither of them decides *where* -- that is settled here, once.
#[derive(Debug, Clone, Copy)]
pub struct PlacedGlyph {
    pub key: cosmic_text::CacheKey,
    pub x: i32,
    pub y: i32,
    /// Drawn in the quieter ink. Carried per glyph because one line mixes them:
    /// an author's name and the time beside it are one shaped run.
    pub faint: bool,
}

/// Where the glyphs of an already-shaped buffer land.
///
/// `Painter::run` shapes its own text and is the right thing for a label. This
/// is for a buffer somebody else owns and keeps between frames -- the composer's
/// editor, which holds the text being typed and must not be reshaped from a
/// string every frame, because the caret and the selection are positions inside
/// the shaping and would be lost with it.
pub fn placed_glyphs(buffer: &Buffer, x: f32, y: f32) -> Vec<PlacedGlyph> {
    let mut placed = Vec::new();
    for line in buffer.layout_runs() {
        for glyph in line.glyphs {
            let physical = glyph.physical((x, y + line.line_y), 1.0);
            placed.push(PlacedGlyph {
                key: physical.cache_key,
                x: physical.x,
                y: physical.y,
                faint: false,
            });
        }
    }
    placed
}

/// A row reduced to what a renderer has to put on screen.
///
/// Kept as a list rather than drawn directly so the snapshot and the window
/// draw the same thing. Two renderers walking the layout separately is exactly
/// how the browser and the virtualiser came to disagree.
#[derive(Debug, Clone)]
pub enum Piece {
    Fill {
        x: f32,
        y: f32,
        width: f32,
        height: f32,
        colour: [u8; 4],
    },
    Text {
        glyphs: Vec<PlacedGlyph>,
        ink: [u8; 3],
        faint: [u8; 3],
    },
    /// A picture, named by the route it came from.
    ///
    /// Nothing is drawn until the bytes have arrived, and the space it occupies
    /// was reserved by the layout rather than by the image -- which is the same
    /// rule every other row follows, and the reason a face appearing does not
    /// move the conversation under it.
    Image {
        x: f32,
        y: f32,
        width: f32,
        height: f32,
        key: String,
    },
}

/// A frame's worth of drawing, built up piece by piece.
///
/// The message list produces these from a row plan; the sidebar, the header and
/// the composer produce them from boxes. Both end in the same list, so the same
/// renderer draws them and there is one place where drawing happens.
#[derive(Debug, Default, Clone)]
pub struct Scene {
    pub layers: Vec<Layer>,
}

/// A group of pieces and the rectangle they are confined to.
///
/// Panels sit side by side and their contents overrun them: a message wider
/// than the stream, a channel name longer than the sidebar. Clipping is what
/// keeps each panel's spill inside itself, and it is per group because the GPU
/// sets it once per draw rather than per shape.
#[derive(Debug, Clone)]
pub struct Layer {
    pub clip: (f32, f32, f32, f32),
    pub pieces: Vec<Piece>,
}

impl Scene {
    /// Starts a group clipped to `rect`. Everything drawn after this belongs to
    /// it until the next one.
    pub fn clip_to(&mut self, x: f32, y: f32, width: f32, height: f32) {
        self.layers.push(Layer {
            clip: (x, y, width.max(0.0), height.max(0.0)),
            pieces: Vec::new(),
        });
    }

    fn current(&mut self) -> &mut Layer {
        if self.layers.is_empty() {
            // Unclipped until told otherwise, so a caller that only wants to
            // draw does not have to think about panels.
            self.layers.push(Layer {
                clip: (0.0, 0.0, f32::MAX, f32::MAX),
                pieces: Vec::new(),
            });
        }
        self.layers.last_mut().expect("a layer")
    }

    pub fn fill(&mut self, x: f32, y: f32, width: f32, height: f32, colour: [u8; 4]) {
        if width <= 0.0 || height <= 0.0 {
            return;
        }
        self.current().pieces.push(Piece::Fill {
            x,
            y,
            width,
            height,
            colour,
        });
    }

    pub fn glyphs(&mut self, glyphs: Vec<PlacedGlyph>, ink: [u8; 3], faint: [u8; 3]) {
        if glyphs.is_empty() {
            return;
        }
        self.current()
            .pieces
            .push(Piece::Text { glyphs, ink, faint });
    }

    pub fn extend(&mut self, pieces: impl IntoIterator<Item = Piece>) {
        self.current().pieces.extend(pieces);
    }
}

/// How a run of interface text is drawn. Not `row::Style`, which describes a
/// message body: this is for a channel name, a button, a heading.
#[derive(Debug, Clone, Copy)]
pub struct Run {
    pub size: f32,
    pub line_height: f32,
    pub bold: bool,
    /// Where it wraps. Interface text is usually given more room than it needs
    /// and clipped by its box instead.
    pub wrap: f32,
}

impl Run {
    pub fn label(wrap: f32) -> Self {
        Self {
            size: 13.0,
            line_height: 18.0,
            bold: false,
            wrap,
        }
    }

    pub fn bold(mut self) -> Self {
        self.bold = true;
        self
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
    /// Reduces a laid-out row to the pieces a renderer puts on screen.
    ///
    /// The one place that turns layout into draw calls. Both renderers consume
    /// this, so neither can drift from the other or from the measured heights.
    pub fn pieces_of(
        &mut self,
        fonts: &mut Fonts,
        row: &RowLayout,
        top: f32,
        theme: &Theme,
        palette: &Palette,
        // Custom emoji by name to the id their picture is behind. Passed in
        // because only the caller has read the store: the layout knows the name
        // and nothing more.
        custom: &HashMap<String, String>,
    ) -> Vec<Piece> {
        let mut pieces = Vec::new();
        for block in &row.blocks {
            let x = theme.gutter + block.x;
            let y = top + block.y;
            match block.kind {
                Kind::Separator => pieces.push(Piece::Fill {
                    x: 0.0,
                    y: y + block.height / 2.0,
                    width: theme.width,
                    height: 1.0,
                    colour: [palette.faint[0], palette.faint[1], palette.faint[2], 255],
                }),
                Kind::Header => {
                    // The avatar's square, until faces are drawn.
                    pieces.push(Piece::Fill {
                        x: 0.0,
                        y,
                        width: 28.0,
                        height: 28.0,
                        colour: palette.surface,
                    });
                    pieces.push(Piece::Text {
                        glyphs: self.glyphs_of(fonts, block, x, y, theme).0,
                        ink: palette.ink,
                        faint: palette.faint,
                    });
                }
                Kind::Code => {
                    // The full column, not the wrap: `wrap` is the width the
                    // text is shaped at, which is inside the padding the text
                    // is drawn at. Filling to it would leave the last letter of
                    // a full line sitting outside its own background.
                    pieces.push(Piece::Fill {
                        x,
                        y,
                        width: theme.text_width(),
                        height: block.height,
                        colour: palette.surface,
                    });
                    pieces.push(Piece::Text {
                        glyphs: self.glyphs_of(fonts, block, x + 8.0, y + 8.0, theme).0,
                        ink: palette.ink,
                        faint: palette.faint,
                    });
                }
                Kind::Text => {
                    let (glyphs, rooms) = self.glyphs_of(fonts, block, x, y, theme);
                    pieces.push(Piece::Text {
                        glyphs,
                        ink: palette.ink,
                        faint: palette.faint,
                    });
                    // A custom emoji's placeholder, turned into the picture it
                    // was standing in for. Drawn to the room the spaces
                    // actually measured, so the line and the image agree
                    // however the font shaped them.
                    for (at, (left, top, width)) in rooms {
                        let Some(name) = block.spans.get(at).and_then(|span| span.emoji.as_ref())
                        else {
                            continue;
                        };
                        let Some(id) = custom.get(name) else {
                            continue;
                        };
                        pieces.push(Piece::Image {
                            x: left,
                            y: top,
                            width,
                            height: width,
                            key: format!("emoji/{id}"),
                        });
                    }
                }
                // One line of a card: the bar beside it is drawn by whoever
                // owns the row, because it spans the whole run of them and a
                // single line does not know it is the first or the last.
                Kind::Preview => {
                    let (glyphs, _) = self.glyphs_of(fonts, block, x, y, theme);
                    pieces.push(Piece::Text {
                        glyphs,
                        ink: palette.ink,
                        faint: palette.faint,
                    });
                }
                // Drawn by whoever owns the row rather than here: a pill is a
                // box with a picture and a count in it, not a run of text, and
                // only the caller knows which emoji is which.
                Kind::Reactions => {}
                Kind::Attachment | Kind::Footer => {
                    if block.height >= 1.0 {
                        pieces.push(Piece::Fill {
                            x,
                            y,
                            width: 120.0,
                            height: (block.height - 4.0).max(1.0),
                            colour: palette.surface,
                        });
                    }
                }
            }
        }
        pieces
    }

    /// Draws a row onto a CPU buffer, for a snapshot.
    pub fn paint_row(
        &mut self,
        canvas: &mut Canvas,
        fonts: &mut Fonts,
        row: &RowLayout,
        top: f32,
        theme: &Theme,
        palette: &Palette,
    ) {
        // The snapshot path draws a picture as the space it occupies, so it
        // has no emoji to resolve and hands over an empty map.
        let pieces = self.pieces_of(fonts, row, top, theme, palette, &HashMap::new());
        self.paint_pieces(canvas, fonts, &pieces);
    }

    /// Rasterises a draw list onto a CPU buffer.
    pub fn paint_pieces(&mut self, canvas: &mut Canvas, fonts: &mut Fonts, pieces: &[Piece]) {
        for piece in pieces {
            match piece {
                Piece::Fill {
                    x,
                    y,
                    width,
                    height,
                    colour,
                } => canvas.fill(*x as i32, *y as i32, *width as i32, *height as i32, *colour),
                // The snapshot path has no network, so a picture is drawn as
                // the space it occupies. That is the honest answer: this path
                // exists to check heights and glyph positions, and a filled box
                // shows the room was reserved without pretending to bytes it
                // never fetched.
                Piece::Image {
                    x,
                    y,
                    width,
                    height,
                    ..
                } => canvas.fill(
                    *x as i32,
                    *y as i32,
                    *width as i32,
                    *height as i32,
                    [40, 48, 58, 255],
                ),
                Piece::Text { glyphs, ink, faint } => {
                    for glyph in glyphs {
                        let shade = if glyph.faint { *faint } else { *ink };
                        let colour = Color::rgb(shade[0], shade[1], shade[2]);
                        self.glyphs.with_pixels(
                            fonts.system_mut(),
                            glyph.key,
                            colour,
                            |dx, dy, pixel| {
                                // The pixel's own colour, not the requested one:
                                // for a letter they are the same, and for an
                                // emoji only the pixel knows.
                                canvas.blend(
                                    glyph.x + dx,
                                    glyph.y + dy,
                                    [pixel.r(), pixel.g(), pixel.b()],
                                    pixel.a(),
                                );
                            },
                        );
                    }
                }
            }
        }
    }

    /// Shapes one run of interface text and reports where each glyph lands.
    ///
    /// `y` is the run's *top*, not its baseline: every other measurement in
    /// this app is a box, and a caller that has just been handed a rectangle
    /// should not have to know about baselines to put text in it.
    pub fn run(
        &mut self,
        fonts: &mut Fonts,
        text: &str,
        x: f32,
        y: f32,
        run: Run,
    ) -> Vec<PlacedGlyph> {
        if text.is_empty() {
            return Vec::new();
        }
        let mut buffer = Buffer::new(fonts.system_mut(), Metrics::new(run.size, run.line_height));
        let mut shaped = buffer.borrow_with(fonts.system_mut());
        shaped.set_size(Some(run.wrap), None);
        let mut attrs = Attrs::new();
        if run.bold {
            attrs = attrs.weight(Weight::BOLD);
        }
        shaped.set_text(text, &attrs, Shaping::Advanced, None);
        shaped.shape_until_scroll(false);

        let mut placed = Vec::new();
        for line in shaped.layout_runs() {
            for glyph in line.glyphs {
                let physical = glyph.physical((x, y + line.line_y), 1.0);
                placed.push(PlacedGlyph {
                    key: physical.cache_key,
                    x: physical.x,
                    y: physical.y,
                    faint: false,
                });
            }
        }
        placed
    }

    /// Shapes a block's spans and reports where each glyph lands.
    ///
    /// Shaped against the layout's own wrap width, never the surface's: wrapping
    /// anywhere else would break the lines somewhere other than where they were
    /// counted, and the row would no longer be the height reserved for it.
    fn glyphs_of(
        &mut self,
        fonts: &mut Fonts,
        block: &Block,
        x: f32,
        y: f32,
        theme: &Theme,
    ) -> (Vec<PlacedGlyph>, Rooms) {
        if block.spans.is_empty() {
            return (Vec::new(), HashMap::new());
        }
        let line_height = if block.kind == Kind::Code {
            theme.code_line_height
        } else {
            theme.line_height
        };
        let mut buffer = Buffer::new(fonts.system_mut(), Metrics::new(block.size, line_height));
        let mut shaped = buffer.borrow_with(fonts.system_mut());
        shaped.set_size(Some(block.wrap), None);
        let spans: Vec<(&str, Attrs<'static>)> = block
            .spans
            .iter()
            .enumerate()
            .map(|(at, span)| (span.text.as_str(), attrs_of(span, at)))
            .collect();
        shaped.set_rich_text(spans, &Attrs::new(), Shaping::Advanced, None);
        shaped.shape_until_scroll(false);

        let mut placed = Vec::new();
        // Where each custom emoji's placeholder ended up, gathered as the runs
        // are walked: its glyphs are spaces and drawing them would draw
        // nothing, so the span they belong to is turned into a picture instead.
        let mut rooms: Rooms = HashMap::new();
        // Never more lines than the layout reserved. The buffer wraps to the
        // width it was given and will happily produce a fourth line for a
        // three-line block -- which draws over the message underneath. The
        // layout decides how tall a block is; this draws that and no more.
        for run in shaped.layout_runs().take(block.lines.max(1)) {
            for glyph in run.glyphs {
                if let Some(at) = emoji_span(glyph.metadata)
                    && block.spans.get(at).is_some_and(|span| span.emoji.is_some())
                {
                    // Widened to cover every space of the placeholder, so the
                    // picture fills exactly the room the line reserved for it
                    // however the font measured them.
                    let room = rooms
                        .entry(at)
                        .or_insert((glyph.x + x, y + run.line_top, 0.0));
                    room.2 = (glyph.x + x + glyph.w - room.0).max(room.2);
                    continue;
                }
                let physical = glyph.physical((x, y + run.line_y), 1.0);
                placed.push(PlacedGlyph {
                    key: physical.cache_key,
                    x: physical.x,
                    y: physical.y,
                    faint: glyph.metadata == FAINT,
                });
            }
        }
        (placed, rooms)
    }
}

/// Where a custom emoji's placeholder ended up: its left edge, its top, and
/// how wide the spaces standing in for it actually measured.
type Room = (f32, f32, f32);

/// The placeholders in one block, by the span each belongs to.
type Rooms = HashMap<usize, Room>;

/// Marks a faint span so its glyphs can be told apart after shaping.
const FAINT: usize = 1;

/// Marks a span as a custom emoji's placeholder, with the span index added on
/// so a line holding several can tell them apart.
const EMOJI: usize = 16;

fn emoji_span(metadata: usize) -> Option<usize> {
    metadata.checked_sub(EMOJI)
}

fn attrs_of(span: &TextSpan, at: usize) -> Attrs<'static> {
    let mut attrs = Attrs::new();
    // A custom emoji marks its span so the placeholder can be found again once
    // the glyphs come back, which is the only way to know where the picture
    // goes. Its index rides along, because a line may hold several.
    if span.emoji.is_some() {
        attrs = attrs.metadata(EMOJI + at);
    } else if span.faint {
        // Metadata rides through shaping onto every glyph the span produces,
        // which is how one run can be drawn in two colours.
        attrs = attrs.metadata(FAINT);
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use matterless_layout::row::{Theme, lay_out};
    use matterless_render::{PostRow, Preview, Row};
    use std::sync::Arc;

    fn post(previews: Vec<Preview>) -> PostRow {
        PostRow {
            post_id: "p1".into(),
            root_id: String::new(),
            author_id: "u1".into(),
            author_name: "ada".into(),
            create_at: 0,
            update_at: 0,
            edited: false,
            nodes: Arc::new(vec![matterless_render::markdown::Node::Paragraph {
                children: vec![matterless_render::markdown::Node::Text {
                    value: "look".into(),
                }],
            }]),
            reactions: Vec::new(),
            files: Vec::new(),
            attachments: Vec::new(),
            avatar_at: 0,
            bot: false,
            body_is_attachment_only: false,
            pending: false,
            failed: false,
            pinned: false,
            saved: false,
            following: false,
            previews,
        }
    }

    /// Nothing is drawn below the height the layout reserved.
    ///
    /// The buffer wraps to the width it is given and will produce a fourth
    /// line for a three-line block, which draws over the message underneath.
    /// A capped description is the case that made this reachable, and every
    /// block has the same contract: the layout decides the height.
    #[test]
    fn no_glyph_is_drawn_below_the_row_it_belongs_to() {
        let mut fonts = Fonts::new();
        let mut painter = Painter::new();
        let theme = Theme::default();
        let row = Row::Post {
            post: post(vec![Preview::Page {
                url: "https://example.invalid/thing".into(),
                title: "A title".into(),
                // Far more than any cap, which is the whole point.
                description: "word ".repeat(800),
                site_name: "example.invalid".into(),
                image: None,
            }]),
        };
        let laid = lay_out(&mut fonts, &row, &theme);
        let pieces = painter.pieces_of(
            &mut fonts,
            &laid,
            0.0,
            &theme,
            &Palette::default(),
            &HashMap::new(),
        );
        let lowest = pieces
            .iter()
            .filter_map(|piece| match piece {
                Piece::Text { glyphs, .. } => glyphs.iter().map(|glyph| glyph.y).max(),
                _ => None,
            })
            .max()
            .expect("some text");
        assert!(
            (lowest as f32) <= laid.height,
            "drew down to {lowest} in a row {} tall",
            laid.height
        );
    }
}
