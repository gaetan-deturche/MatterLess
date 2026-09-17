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
use matterless_layout::row::{Block, Kind, Press, RowLayout, TextSpan, Theme};
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
    /// Behind everything.
    pub ground: [u8; 4],
    /// A panel raised off the ground: the sidebar, a card, a dialog.
    pub surface: [u8; 4],
    /// Under the message the pointer is over, and nothing else.
    ///
    /// A step off the ground rather than a surface of its own. Drawn in
    /// `surface` it was the same colour as a card or a panel, so a row lit
    /// under the pointer read as having become one -- which is a great deal
    /// louder than saying where the pointer is.
    pub hover: [u8; 4],
    /// Raised again: a code block, a pill, a field inside a panel.
    pub raised: [u8; 4],
    /// What the eye should land on.
    pub ink: [u8; 3],
    /// A step down: a name beside a message, a label above a group.
    pub soft: [u8; 3],
    /// Quieter still: a timestamp, a read channel, a count.
    pub faint: [u8; 3],
    /// A line between two panes.
    pub rule: [u8; 4],
    /// A line inside one, which should carry less weight than the panes do.
    pub rule_soft: [u8; 4],
    /// Something to follow: a link, a mention, the selected row.
    pub signal: [u8; 3],
    /// Behind something signalled -- a mention's own line.
    pub signal_soft: [u8; 4],
    /// Something waiting: an unread divider, a pinned mark.
    pub flag: [u8; 3],
    /// Somebody is here.
    pub ok: [u8; 3],
    /// Something is wrong, or somebody does not want to be disturbed.
    pub danger: [u8; 3],
}

impl Palette {
    /// `colour` at `opacity` over this palette's own panel.
    ///
    /// What CSS gets for nothing and a scene of opaque quads does not: a muted
    /// channel is its own colour dimmed, not a different colour. Replacing it
    /// instead is how "muted" and "read" came to look identical -- both of
    /// them were simply the faint ink.
    pub fn dimmed(&self, colour: [u8; 3], opacity: f32) -> [u8; 3] {
        let opacity = opacity.clamp(0.0, 1.0);
        let mut over = [0u8; 3];
        for channel in 0..3 {
            let under = f32::from(self.surface[channel]);
            let ink = f32::from(colour[channel]);
            over[channel] = (ink * opacity + under * (1.0 - opacity)).round() as u8;
        }
        over
    }
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
    /// The app's dark theme, to the byte.
    ///
    /// Taken from its stylesheet rather than matched by eye: two clients of
    /// one server looking almost the same is worse than looking different,
    /// because the difference reads as a rendering fault rather than a choice.
    fn default() -> Self {
        Self {
            ground: [12, 18, 24, 255],
            surface: [20, 29, 38, 255],
            // Halfway from the ground to a surface: enough to follow the
            // pointer down a conversation, not enough to read as a thing.
            hover: [16, 23, 31, 255],
            raised: [27, 39, 52, 255],
            ink: [227, 234, 241],
            soft: [148, 164, 179],
            faint: [109, 125, 140],
            rule: [37, 51, 66, 255],
            rule_soft: [28, 39, 51, 255],
            signal: [75, 203, 236],
            signal_soft: [16, 50, 61, 255],
            flag: [224, 161, 63],
            ok: [78, 196, 155],
            danger: [226, 104, 90],
        }
    }
}

/// One glyph, rasterised or not, at the place it belongs.
///
/// The unit both consumers share: the CPU snapshot rasterises it straight onto
/// a buffer, and the GPU renderer looks it up in an atlas and emits a quad.
/// Neither of them decides *where* -- that is settled here, once.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Shade {
    /// What the eye should land on.
    #[default]
    Ink,
    /// A timestamp, a count, anything to pass over.
    Faint,
    /// Something to follow. `a { color: var(--signal) }`, which a run of text
    /// could not express while a glyph had only two inks to choose between.
    Signal,
}

/// One glyph, placed, with the ink it takes.
#[derive(Debug, Clone, Copy)]
pub struct PlacedGlyph {
    pub key: cosmic_text::CacheKey,
    pub x: i32,
    pub y: i32,
    /// Which of the three inks this glyph takes. Carried per glyph because one
    /// line mixes them: an author's name, the time beside it and a link in the
    /// sentence after are all one shaped run.
    pub shade: Shade,
    /// How much of its rasterised size it is drawn at. One for a letter.
    ///
    /// Below one means it was rasterised larger than it is drawn and is to be
    /// filtered down -- which is supersampling, and is what makes a pictogram
    /// legible at the size an interface wants one. A letter is hinted for the
    /// size it is drawn at and wants none of it.
    pub scale: f32,
}

/// A run shaped at the origin, put where the caller asked for it.
///
/// Added to rather than shaped against: the pen and the offset are rounded
/// apart, which is what the shaping did when it was the caller's own, so a
/// label does not shift by a pixel the first time it comes from the store.
fn moved(at_origin: &[PlacedGlyph], x: f32, y: f32) -> Vec<PlacedGlyph> {
    let (x, y) = (x as i32, y as i32);
    at_origin
        .iter()
        .map(|glyph| PlacedGlyph {
            x: glyph.x + x,
            y: glyph.y + y,
            ..*glyph
        })
        .collect()
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
                shade: Shade::Ink,
                // Text, rasterised at the size it is drawn.
                scale: 1.0,
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
    /// A rectangle that fades from nothing at its top into `colour` at its
    /// bottom.
    ///
    /// What a cut code block trails off into, so the reader can see that the
    /// last line they are shown is not the last line there is. One quad: the
    /// fragment already interpolates whatever the corners carry.
    Fade {
        x: f32,
        y: f32,
        width: f32,
        height: f32,
        colour: [u8; 4],
    },
    Fill {
        x: f32,
        y: f32,
        width: f32,
        height: f32,
        colour: [u8; 4],
        /// How far the corners are cut. Zero is a square one.
        radius: f32,
        /// How far the edge fades. One is a crisp shape; twenty is a shadow,
        /// which `box-shadow` is and which the rounded-box distance already
        /// describes -- it needs a wider falloff and nothing else.
        softness: f32,
    },
    Text {
        glyphs: Vec<PlacedGlyph>,
        ink: [u8; 3],
        faint: [u8; 3],
        /// What a glyph a reader can follow is drawn in.
        signal: [u8; 3],
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
        /// How far the corners are cut. Half the side makes a circle, which is
        /// what a face is drawn as.
        radius: f32,
    },
    /// The picture a reader has opened, drawn from a texture of its own.
    ///
    /// Not the atlas, deliberately. The atlas is a fixed sheet shared by every
    /// glyph and every thumbnail on screen, with no eviction: putting a
    /// two-thousand-pixel photograph in it would push out the faces around it
    /// and never give the room back. This is one picture at a time, in its own
    /// texture, replaced when another is opened and dropped when none is.
    ///
    /// No key, for the same reason: there is only ever one.
    Shown {
        x: f32,
        y: f32,
        width: f32,
        height: f32,
    },
    /// Where a pressable run of words ended up. Nothing is drawn for it.
    ///
    /// A piece all the same, because only the shaping knows where the words
    /// landed and it already runs once per block: asking a second time would
    /// be a second shaping pass, and a hit box derived any other way drifts
    /// from the text under it. One per line the run spans, so a link that
    /// wraps is pressable on both halves.
    Press {
        x: f32,
        y: f32,
        width: f32,
        height: f32,
        press: Press,
        /// Pressable, but not dressed as a mention.
        ///
        /// A name in a sentence is called out from the words around it -- a
        /// signalled ground, the signal ink. The author's name over a message
        /// is not in a sentence: it is the heading, it is already bold, and
        /// dressing it the same way made every message look as though it began
        /// by mentioning its own writer.
        quiet: bool,
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

    /// A rectangle with square corners.
    pub fn fill(&mut self, x: f32, y: f32, width: f32, height: f32, colour: [u8; 4]) {
        self.rounded(x, y, width, height, colour, 0.0);
    }

    /// A rectangle with its corners cut, which is most of them.
    pub fn rounded(
        &mut self,
        x: f32,
        y: f32,
        width: f32,
        height: f32,
        colour: [u8; 4],
        radius: f32,
    ) {
        self.soft(x, y, width, height, colour, radius, 1.0);
    }

    /// A panel and the shadow it casts, which is what every floating thing in
    /// the app has: `box-shadow: 0 10px 30px rgb(0 0 0 / 0.3)`.
    ///
    /// Drawn under it rather than around it, offset down, because that is what
    /// a shadow is -- and one shape with a wide edge is the whole of it.
    #[allow(clippy::too_many_arguments)]
    pub fn floating(
        &mut self,
        x: f32,
        y: f32,
        width: f32,
        height: f32,
        colour: [u8; 4],
        radius: f32,
        drop: f32,
    ) {
        // Spread as well as offset, and darker than it looks like it needs
        // to be. This palette's ground is (13, 18, 24): there are two dozen
        // levels between it and black, so a shadow drawn the way a light
        // theme's is comes out as a three pixel smudge that nobody sees. The
        // first attempt at this measured as a real change to the pixels and
        // was invisible on screen, which is the same as not being there.
        // `box-shadow: 0 <offset> <blur> rgba(0, 0, 0, .7)`, with a little
        // spread so that something at the bottom of the window still casts
        // upwards -- offset alone puts nothing above a panel.
        //
        // The blur is the part that has to stay small. Widened to four times
        // the drop it stopped being the panel's shape at all: a ten pixel drop
        // gave a forty pixel fade, which is a twenty pixel smear either side of
        // every edge, and the shadow under a card read as a grey cloud beside
        // it rather than as the card being off the page.
        //
        // What actually separates a popup from the window is the hairline
        // around it, not the darkness underneath: the shadow only has to fall
        // away from that edge. Panels drawn without one had nothing for it to
        // fall away from, and no amount of darkening fixed that.
        let spread = drop * 0.15;
        let blur = drop;
        // A radius wider than the box is not a rounder box, it is a shape the
        // distance function has no answer for.
        let corner = (radius + spread).min((width + height) / 4.0);
        self.soft(
            x - spread,
            y - spread + drop * 0.35,
            width + spread * 2.0,
            height + spread * 2.0,
            [0, 0, 0, 200],
            corner,
            blur,
        );
        self.rounded(x, y, width, height, colour, radius);
    }

    /// The same, drawn soft. A shadow is a shape with a wide edge.
    #[allow(clippy::too_many_arguments)]
    pub fn soft(
        &mut self,
        x: f32,
        y: f32,
        width: f32,
        height: f32,
        colour: [u8; 4],
        radius: f32,
        softness: f32,
    ) {
        if width <= 0.0 || height <= 0.0 {
            return;
        }
        self.current().pieces.push(Piece::Fill {
            x,
            y,
            width,
            height,
            colour,
            radius,
            softness,
        });
    }

    pub fn glyphs(&mut self, glyphs: Vec<PlacedGlyph>, ink: [u8; 3], faint: [u8; 3]) {
        self.inked(glyphs, ink, faint, ink);
    }

    /// The same, with a third ink for whatever a reader can follow.
    pub fn inked(
        &mut self,
        glyphs: Vec<PlacedGlyph>,
        ink: [u8; 3],
        faint: [u8; 3],
        signal: [u8; 3],
    ) {
        if glyphs.is_empty() {
            return;
        }
        self.current().pieces.push(Piece::Text {
            glyphs,
            ink,
            faint,
            signal,
        });
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
    /// The monospaced face, for the counts the stylesheet sets in one.
    pub mono: bool,
    /// Where it wraps. Interface text is usually given more room than it needs
    /// and clipped by its box instead.
    pub wrap: f32,
    /// The bundled icon family rather than the reader's text font.
    ///
    /// Its own flag rather than a family name threaded through every call: the
    /// only non-text face this draws with is the marks', and naming it in
    /// twenty places is twenty chances to misspell it.
    pub icon: bool,
    /// Rasterise at twice the size and draw it down.
    ///
    /// For a mark rather than for words. A pictogram at the size an interface
    /// wants one is a dozen pixels across with detail inside it, and a font
    /// hints that detail into hard stems: the bell loses its clapper and the
    /// pushpin becomes a smudge. Rasterised at twice and filtered down, the
    /// detail survives as shades.
    ///
    /// Never for text. A letter is hinted for the size it is drawn at, and
    /// halving a bitmap of one is how text gets blurry.
    pub smooth: bool,
}

impl Run {
    pub fn label(wrap: f32) -> Self {
        Self {
            size: 13.0,
            line_height: 18.0,
            bold: false,
            mono: false,
            wrap,
            icon: false,
            smooth: false,
        }
    }

    /// One of the interface's own marks, from the bundled icon family.
    ///
    /// Drawn down from twice its size, because that is what a mark wants and
    /// asking for both separately would only ever be done wrong once.
    pub fn mark(size: f32) -> Self {
        Self {
            size,
            line_height: size * 1.4,
            bold: false,
            mono: false,
            wrap: f32::MAX,
            icon: true,
            smooth: true,
        }
    }

    /// A mark rather than words: rasterised at twice and filtered down.
    pub fn smooth(mut self) -> Self {
        self.smooth = true;
        self
    }

    pub fn bold(mut self) -> Self {
        self.bold = true;
        self
    }

    /// The monospaced face. Asked for by the unread counts, where it is what
    /// keeps a column of one- and two-digit numbers the same width.
    pub fn mono(mut self) -> Self {
        self.mono = true;
        self
    }

    /// The same run at another size, with the line box following it.
    ///
    /// Line height moves with the size rather than staying where `label` put
    /// it: a 12.5px menu row spaced for 13px text sits a pixel low, and a
    /// column of them sits a pixel lower each time.
    pub fn sized(mut self, size: f32) -> Self {
        self.size = size;
        self.line_height = size * 1.4;
        self
    }
}

/// One run of interface text, as the thing that decides its shaping.
///
/// The floats are held as their bits because that is what a key needs and
/// because they are never arithmetic here: two runs either were asked for at
/// the same size or they were not.
#[derive(PartialEq, Eq, Hash, Clone)]
struct Asked {
    text: String,
    size: u32,
    line_height: u32,
    wrap: u32,
    bold: bool,
    mono: bool,
    icon: bool,
    smooth: bool,
}

impl Asked {
    fn of(text: &str, run: Run) -> Self {
        Self {
            text: text.to_string(),
            size: run.size.to_bits(),
            line_height: run.line_height.to_bits(),
            wrap: run.wrap.to_bits(),
            bold: run.bold,
            mono: run.mono,
            icon: run.icon,
            smooth: run.smooth,
        }
    }
}

/// How many shaped runs to keep before the oldest of them are let go.
///
/// A window holds a few hundred labels at once -- every channel in the
/// sidebar, every heading over them, every word on the toolbar. Well above
/// that, so a full sidebar never sweeps, and well below the size at which the
/// map itself would be worth thinking about.
const KEPT: usize = 512;

/// Holds the rasterised glyphs between frames, and the shaped runs with them.
///
/// This said for a long time that rasterising was the expensive half and
/// shaping cheap by comparison. Measured, that is the wrong way round for
/// interface text: a frame spent 11ms of its 21 in the sidebar, which is sixty
/// labels that had not changed since the frame before, each one built into a
/// `cosmic_text::Buffer` and shaped again from its string. The glyphs behind
/// them were all cache hits.
///
/// So a label's shaping outlives the frame too, held at the origin and moved
/// into place on the way out. The store is swept in two halves rather than by
/// counting uses on every hit: when the live half fills, it becomes the spare
/// and a new one starts, and anything asked for again is carried across. Text
/// that changes every frame costs a sweep every `KEPT` runs and nothing else.
pub struct Painter {
    glyphs: SwashCache,
    /// Runs shaped at the origin, ready to be moved into place.
    shaped: HashMap<Asked, Vec<PlacedGlyph>>,
    /// The half before this one. Read on a miss, dropped on the next sweep.
    spare: HashMap<Asked, Vec<PlacedGlyph>>,
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
            shaped: HashMap::new(),
            spare: HashMap::new(),
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
                Kind::Unread | Kind::Separator => {
                    // The words first, so their width decides where the two
                    // rules stop: a rule drawn under the label would strike
                    // through it.
                    let (glyphs, _, _, _) = self.glyphs_of(fonts, block, 0.0, y, theme);
                    let middle = y + block.height / 2.0;
                    let said = glyphs
                        .iter()
                        .map(|glyph| glyph.x)
                        .max()
                        .map(|right| right as f32 + 8.0)
                        .unwrap_or(0.0);
                    // `.unread` takes `--flag` for its words and for the
                    // rules either side: it is the one separator that is not
                    // just saying where a day ended.
                    let flagged = block.kind == Kind::Unread;
                    let ink = if flagged { palette.flag } else { palette.faint };
                    let rule = [ink[0], ink[1], ink[2], 255];
                    let gap = 14.0;
                    if said > 0.0 {
                        // Centred, with a rule either side of it.
                        let left = ((theme.width - said) / 2.0).max(0.0);
                        pieces.push(Piece::Fill {
                            x: 0.0,
                            y: middle,
                            width: (left - gap).max(0.0),
                            height: 1.0,
                            colour: rule,
                            radius: 0.0,
                            softness: 1.0,
                        });
                        pieces.push(Piece::Fill {
                            x: left + said + gap,
                            y: middle,
                            width: (theme.width - left - said - gap).max(0.0),
                            height: 1.0,
                            colour: rule,
                            radius: 0.0,
                            softness: 1.0,
                        });
                        let (glyphs, _, _, _) = self.glyphs_of(
                            fonts,
                            block,
                            left,
                            y + (block.height - theme.line_height) / 2.0,
                            theme,
                        );
                        pieces.push(Piece::Text {
                            glyphs,
                            ink,
                            faint: ink,
                            signal: palette.signal,
                        });
                    } else {
                        pieces.push(Piece::Fill {
                            x: 0.0,
                            y: middle,
                            width: theme.width,
                            height: 1.0,
                            colour: rule,
                            radius: 0.0,
                            softness: 1.0,
                        });
                    }
                }
                Kind::Header => {
                    // No square behind the face any more. It was a stand-in
                    // from before faces were drawn at all, and once they were
                    // it sat under every round avatar as a hard-cornered grey
                    // block -- visible at each corner of the circle. Whoever
                    // draws the face draws its ground with it.
                    //
                    // The presses come out with the glyphs rather than being
                    // dropped on the floor. This took `.0` and threw the rest
                    // away, so the author's name could carry a press all it
                    // liked and nothing downstream ever heard about it.
                    let (glyphs, _, presses, _) = self.glyphs_of(fonts, block, x, y, theme);
                    // A span that can be pressed is shaded as a link, which is
                    // right in a sentence and wrong here: the author's name is
                    // the heading of the message rather than a reference to
                    // somebody inside it, so it keeps the ink it would have
                    // had.
                    let glyphs: Vec<PlacedGlyph> = glyphs
                        .into_iter()
                        .map(|glyph| match glyph.shade {
                            Shade::Signal => PlacedGlyph {
                                shade: Shade::Ink,
                                ..glyph
                            },
                            _ => glyph,
                        })
                        .collect();
                    pieces.push(Piece::Text {
                        glyphs,
                        ink: palette.ink,
                        faint: palette.faint,
                        signal: palette.signal,
                    });
                    pieces.extend(presses.into_iter().map(|one| Piece::Press {
                        x: one.x,
                        y: one.y,
                        width: one.width,
                        height: one.height,
                        press: one.press,
                        quiet: true,
                    }));
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
                        // Raised, not surface: a code block sits on top of the
                        // panel it is in, as the stylesheet has it.
                        colour: palette.raised,
                        radius: CODE,
                        softness: 1.0,
                    });
                    pieces.push(Piece::Text {
                        glyphs: self
                            .glyphs_of(
                                fonts,
                                block,
                                x + theme.code_padding,
                                y + theme.code_padding_y,
                                theme,
                            )
                            .0,
                        ink: palette.ink,
                        faint: palette.faint,
                        signal: palette.signal,
                    });
                }
                // The ground under the offer to see the rest of a cut listing.
                // A hairline beneath the surface, which is how everything
                // raised on this palette gets an edge to be seen against.
                Kind::Pill => {
                    let round = block.height / 2.0;
                    pieces.push(Piece::Fill {
                        x: x - 1.0,
                        y: y - 1.0,
                        width: block.wrap + 2.0,
                        height: block.height + 2.0,
                        colour: palette.rule,
                        radius: round + 1.0,
                        softness: 1.0,
                    });
                    pieces.push(Piece::Fill {
                        x,
                        y,
                        width: block.wrap,
                        height: block.height,
                        colour: palette.surface,
                        radius: round,
                        softness: 1.0,
                    });
                }
                // The bottom of a cut code block, fading into the ground the
                // block is drawn on. Over the lines rather than beside them,
                // which is what makes the listing trail off.
                Kind::Fade => {
                    pieces.push(Piece::Fade {
                        x,
                        y,
                        width: theme.text_width(),
                        height: block.height,
                        colour: palette.raised,
                    });
                }
                // A rule written in the message, dividing one section of a
                // long notice from the next. Across the text column rather
                // than the whole row, so it lines up with the words it
                // separates.
                Kind::Rule => {
                    pieces.push(Piece::Fill {
                        x,
                        y,
                        width: theme.text_width(),
                        height: block.height,
                        colour: palette.rule,
                        radius: 0.0,
                        softness: 1.0,
                    });
                }
                // The same as text, a step quieter, with a bar down its left.
                Kind::Quote => {
                    let (glyphs, _, _, _) = self.glyphs_of(fonts, block, x, y, theme);
                    pieces.push(Piece::Fill {
                        x: x - theme.indent + theme.quote_bar,
                        y,
                        width: theme.quote_bar,
                        height: block.height,
                        colour: palette.rule,
                        radius: 0.0,
                        softness: 1.0,
                    });
                    pieces.push(Piece::Text {
                        glyphs,
                        ink: palette.soft,
                        faint: palette.faint,
                        signal: palette.signal,
                    });
                }
                Kind::Text => {
                    let (glyphs, rooms, presses, code) = self.glyphs_of(fonts, block, x, y, theme);
                    // The ground behind inline code, before the words: a
                    // background pushed after them covers what it is for.
                    pieces.extend(code.into_iter().map(|(cx, cy, width, height)| Piece::Fill {
                        x: cx - 2.0,
                        y: cy + 1.0,
                        width: width + 4.0,
                        height: height - 2.0,
                        colour: palette.raised,
                        radius: 3.0,
                        softness: 1.0,
                    }));
                    pieces.push(Piece::Text {
                        glyphs,
                        ink: palette.ink,
                        faint: palette.faint,
                        signal: palette.signal,
                    });
                    pieces.extend(presses.into_iter().map(|one| Piece::Press {
                        x: one.x,
                        y: one.y,
                        width: one.width,
                        height: one.height,
                        press: one.press,
                        quiet: false,
                    }));
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
                            radius: 0.0,
                        });
                    }
                }
                // One line of a card: the bar beside it is drawn by whoever
                // owns the row, because it spans the whole run of them and a
                // single line does not know it is the first or the last.
                // Its own ground and its own bar, as `.attachment` has them:
                // a border down the left, the panel behind it, and the corners
                // cut on the right only -- which is what `0 4px 4px 0` says.
                Kind::Attached => {
                    let (glyphs, _, _, _) = self.glyphs_of(fonts, block, x, y, theme);
                    pieces.push(Piece::Text {
                        glyphs,
                        ink: palette.ink,
                        faint: palette.soft,
                        signal: palette.signal,
                    });
                }
                Kind::Preview => {
                    let (glyphs, _, _, _) = self.glyphs_of(fonts, block, x, y, theme);
                    pieces.push(Piece::Text {
                        glyphs,
                        ink: palette.ink,
                        faint: palette.faint,
                        signal: palette.signal,
                    });
                }
                // Drawn by whoever owns the row rather than here. A pill is a
                // box with a picture and a count in it and only the caller
                // knows which emoji is which; a thread footer is faces and a
                // count and only the caller has the faces. Neither is a run of
                // text, which is all this can draw.
                Kind::Reactions | Kind::Footer => {}
                Kind::Attachment => {
                    if block.height >= 1.0 {
                        pieces.push(Piece::Fill {
                            x,
                            y,
                            width: 120.0,
                            height: (block.height - 4.0).max(1.0),
                            colour: palette.surface,
                            radius: 0.0,
                            softness: 1.0,
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
                // Square corners here whatever the radius says: this path
                // exists to check heights and glyph positions on a machine
                // with no display, and a rounded corner is neither.
                Piece::Fill {
                    x,
                    y,
                    width,
                    height,
                    colour,
                    ..
                } => canvas.fill(*x as i32, *y as i32, *width as i32, *height as i32, *colour),
                // Flat here, at half its strength: this path exists to check
                // heights and glyph positions on a machine with no display,
                // and a gradient is neither -- but drawing nothing would let a
                // snapshot claim a cut listing runs on.
                Piece::Fade {
                    x,
                    y,
                    width,
                    height,
                    colour,
                } => canvas.fill(
                    *x as i32,
                    *y as i32,
                    *width as i32,
                    *height as i32,
                    [colour[0], colour[1], colour[2], colour[3] / 2],
                ),
                // Nothing is drawn for a press: it is a box a pointer can
                // land on, and the words in it are already drawn as text.
                Piece::Press { .. } => {}
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
                }
                | Piece::Shown {
                    x,
                    y,
                    width,
                    height,
                } => canvas.fill(
                    *x as i32,
                    *y as i32,
                    *width as i32,
                    *height as i32,
                    [40, 48, 58, 255],
                ),
                Piece::Text {
                    glyphs, ink, faint, ..
                } => {
                    for glyph in glyphs {
                        // This path checks heights and glyph positions on a
                        // machine with no display, so a link takes the loud
                        // ink rather than a third colour it cannot show.
                        let shade = match glyph.shade {
                            Shade::Faint => *faint,
                            _ => *ink,
                        };
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
        let asked = Asked::of(text, run);
        if !self.shaped.contains_key(&asked)
            && let Some(before) = self.spare.remove(&asked)
        {
            self.shaped.insert(asked.clone(), before);
        }
        if let Some(at_origin) = self.shaped.get(&asked) {
            return moved(at_origin, x, y);
        }
        // Twice the size when it is a mark, and drawn back down below. The
        // shaping is what carries the size into the glyph's cache key, so this
        // is the only place it can be asked for.
        let over = if run.smooth { 2.0 } else { 1.0 };
        let mut buffer = Buffer::new(
            fonts.system_mut(),
            Metrics::new(run.size * over, run.line_height * over),
        );
        let mut shaped = buffer.borrow_with(fonts.system_mut());
        shaped.set_size(Some(run.wrap), None);
        let mut attrs = Attrs::new();
        if run.icon {
            attrs = attrs.family(Family::Name(matterless_layout::marks::FAMILY));
        } else if run.mono {
            attrs = attrs.family(matterless_layout::mono_family());
        }
        if run.bold {
            attrs = attrs.weight(Weight::BOLD);
        }
        shaped.set_text(text, &attrs, Shaping::Advanced, None);
        shaped.shape_until_scroll(false);

        let mut placed = Vec::new();
        for line in shaped.layout_runs() {
            for glyph in line.glyphs {
                // Laid out at the size it was shaped and then brought back to
                // where it belongs: the pen is the caller's, and only the
                // distance from it is doubled.
                let physical = glyph.physical((0.0, line.line_y), 1.0);
                placed.push(PlacedGlyph {
                    key: physical.cache_key,
                    x: (physical.x as f32 / over) as i32,
                    y: (physical.y as f32 / over) as i32,
                    shade: Shade::Ink,
                    scale: 1.0 / over,
                });
            }
        }
        if self.shaped.len() >= KEPT {
            self.spare = std::mem::take(&mut self.shaped);
        }
        let at_origin = self.shaped.entry(asked).or_insert(placed);
        moved(at_origin, x, y)
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
    ) -> (Vec<PlacedGlyph>, Rooms, Presses, Vec<CodeBox>) {
        if block.spans.is_empty() {
            return (Vec::new(), HashMap::new(), Vec::new(), Vec::new());
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
        // Where each pressable run ended up, per line, widened glyph by glyph
        // the same way an emoji's room is.
        let mut presses: Presses = Vec::new();
        let mut code: Vec<CodeBox> = Vec::new();
        // Never more lines than the layout reserved. The buffer wraps to the
        // width it was given and will happily produce a fourth line for a
        // three-line block -- which draws over the message underneath. The
        // layout decides how tall a block is; this draws that and no more.
        for run in shaped.layout_runs().take(block.lines.max(1)) {
            for glyph in run.glyphs {
                let at = span_of(glyph.metadata);
                if marks(glyph.metadata, PRESS)
                    && let Some(press) = block.spans.get(at).and_then(|span| span.press.as_ref())
                {
                    // One box per line the run spans, widened glyph by glyph:
                    // a link that wraps is pressable on both halves rather
                    // than on one box straddling the gap between them.
                    match presses.last_mut() {
                        Some(last)
                            if last.press == *press
                                && (last.y - (y + run.line_top)).abs() < 0.5 =>
                        {
                            last.width = (glyph.x + x + glyph.w - last.x).max(last.width);
                        }
                        _ => presses.push(PressBox {
                            x: glyph.x + x,
                            y: y + run.line_top,
                            width: glyph.w,
                            height: line_height,
                            press: press.clone(),
                        }),
                    }
                }
                if marks(glyph.metadata, EMOJI) {
                    // Widened to cover every space of the placeholder, so the
                    // picture fills exactly the room the line reserved for it
                    // however the font measured them.
                    let room = rooms
                        .entry(at)
                        .or_insert((glyph.x + x, y + run.line_top, 0.0));
                    room.2 = (glyph.x + x + glyph.w - room.0).max(room.2);
                    continue;
                }
                // Where a run of `code` landed, widened glyph by glyph the
                // same way a link's box is: `code` carries a ground of its own
                // and only the shaping knows how wide to make it.
                if marks(glyph.metadata, MONO) && block.kind != Kind::Code {
                    let top = y + run.line_top;
                    match code.last_mut() {
                        Some(last) if (last.1 - top).abs() < 0.5 => {
                            last.2 = (glyph.x + x + glyph.w - last.0).max(last.2);
                        }
                        _ => code.push((glyph.x + x, top, glyph.w, line_height)),
                    }
                }
                let physical = glyph.physical((x, y + run.line_y), 1.0);
                placed.push(PlacedGlyph {
                    key: physical.cache_key,
                    x: physical.x,
                    y: physical.y,
                    // A link wins over faint: a quiet link is still a link,
                    // and there is nowhere in a message where both are meant.
                    shade: if marks(glyph.metadata, PRESS) {
                        Shade::Signal
                    } else if marks(glyph.metadata, FAINT) {
                        Shade::Faint
                    } else {
                        Shade::Ink
                    },
                    // A message's own words, rasterised at the size they are
                    // drawn.
                    scale: 1.0,
                });
            }
        }
        (placed, rooms, presses, code)
    }
}

/// Where a custom emoji's placeholder ended up: its left edge, its top, and
/// how wide the spaces standing in for it actually measured.
type Room = (f32, f32, f32);

/// The placeholders in one block, by the span each belongs to.
type Rooms = HashMap<usize, Room>;

/// Marks a faint span so its glyphs can be told apart after shaping.
/// Where a run of inline code ended up on one line, so a ground can be drawn
/// behind it. The same trick the press boxes use, and for the same reason:
/// only the shaping knows where the words landed.
pub type CodeBox = (f32, f32, f32, f32);

/// Where a pressable run of words ended up on one line.
#[derive(Debug, Clone, PartialEq)]
pub struct PressBox {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    pub press: Press,
}

/// The press boxes one block produced, in the order they were shaped.
type Presses = Vec<PressBox>;

/// What the stylesheet cuts a code block's corners by.
const CODE: f32 = 6.0;

/// What a glyph remembers about the span it came from.
///
/// Metadata rides through shaping onto every glyph a span produces, and it is
/// the only channel back from the shaper. It carries the span's index *and*
/// what is special about it, because a line holds several spans and a glyph
/// has to say which one it belongs to -- a link that wraps, an emoji
/// placeholder, and a faint timestamp are all "which span was that".
const FAINT: usize = 1;
const EMOJI: usize = 2;
const PRESS: usize = 4;
/// Set in a monospace face, which in a sentence means `code` -- and `code`
/// has a ground of its own.
const MONO: usize = 8;
/// How many flags share the low bits. The span index rides above them.
const FLAGS: usize = 16;

fn marked(span: &TextSpan, at: usize) -> usize {
    let mut flags = 0;
    if span.faint {
        flags |= FAINT;
    }
    if span.emoji.is_some() {
        flags |= EMOJI;
    }
    if span.press.is_some() {
        flags |= PRESS;
    }
    if span.mono {
        flags |= MONO;
    }
    at * FLAGS + flags
}

fn span_of(metadata: usize) -> usize {
    metadata / FLAGS
}

fn marks(metadata: usize, flag: usize) -> bool {
    (metadata % FLAGS) & flag != 0
}

fn attrs_of(span: &TextSpan, at: usize) -> Attrs<'static> {
    let mut attrs = Attrs::new();
    // The span marks itself so its glyphs can be found again once they come
    // back: it is the only way to know where a picture goes, where a link is,
    // or which half of a run is quiet.
    attrs = attrs.metadata(marked(span, at));
    if span.mono {
        attrs = attrs.family(matterless_layout::mono_family());
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

    fn same(left: &[PlacedGlyph], right: &[PlacedGlyph]) -> bool {
        left.len() == right.len()
            && left.iter().zip(right).all(|(one, two)| {
                one.key == two.key && one.x == two.x && one.y == two.y && one.scale == two.scale
            })
    }

    /// A label from the store lands where shaping it again would have put it.
    ///
    /// The whole risk of keeping a shaping is that the second time it is used
    /// it comes back a pixel off, which is invisible in one label and is a
    /// sidebar that shivers when anything else redraws it.
    #[test]
    fn a_kept_shaping_draws_exactly_where_a_fresh_one_would() {
        let mut fonts = Fonts::new();
        let run = Run::label(f32::MAX);
        let fresh = Painter::new().run(&mut fonts, "Voyager | Progression", 31.0, 17.0, run);
        let mut painter = Painter::new();
        // The first call shapes it, the second can only have come from the
        // store -- and both are asked for in the same place.
        let first = painter.run(&mut fonts, "Voyager | Progression", 31.0, 17.0, run);
        let again = painter.run(&mut fonts, "Voyager | Progression", 31.0, 17.0, run);
        assert!(!fresh.is_empty(), "nothing was shaped at all");
        assert!(same(&fresh, &first), "the first call moved");
        assert!(same(&fresh, &again), "the kept one moved");
    }

    /// The same words at another size are another shaping, not the same one
    /// scaled -- so everything the shaping depends on has to be in the key.
    #[test]
    fn a_run_asked_for_differently_is_not_the_one_already_kept() {
        let mut fonts = Fonts::new();
        let mut painter = Painter::new();
        let plain = painter.run(&mut fonts, "Threads", 0.0, 0.0, Run::label(f32::MAX));
        let bold = painter.run(&mut fonts, "Threads", 0.0, 0.0, Run::label(f32::MAX).bold());
        let large = painter.run(
            &mut fonts,
            "Threads",
            0.0,
            0.0,
            Run::label(f32::MAX).sized(30.0),
        );
        assert!(!same(&plain, &bold), "bold came back as the plain shaping");
        assert!(
            !same(&plain, &large),
            "a larger size came back as the small one"
        );
    }

    /// The store is swept rather than grown without end, and a sweep keeps
    /// answering: what is asked for again is carried across from the half
    /// being let go.
    #[test]
    fn a_sweep_does_not_lose_a_label_that_is_still_in_use() {
        let mut fonts = Fonts::new();
        let mut painter = Painter::new();
        let run = Run::label(f32::MAX);
        let kept = painter.run(&mut fonts, "Voyager | Dev", 4.0, 9.0, run);
        // Enough one-off text to fill the live half twice over, which is what
        // a clock that reads a changing count every frame does.
        for at in 0..KEPT * 2 {
            let _ = painter.run(&mut fonts, &format!("{at} replies"), 0.0, 0.0, run);
        }
        let after = painter.run(&mut fonts, "Voyager | Dev", 4.0, 9.0, run);
        assert!(
            same(&kept, &after),
            "the label came back different after a sweep"
        );
    }

    /// A press on the author's name survives into the draw list.
    ///
    /// The header arm took only the glyphs out of the shaping and dropped
    /// everything else, so the name could carry a press all it liked and
    /// nothing downstream ever heard about it -- the layout looked right and
    /// the click did nothing.
    #[test]
    fn a_header_hands_on_the_press_it_was_given() {
        use matterless_layout::row::Press;

        let mut fonts = Fonts::new();
        let mut painter = Painter::new();
        let theme = Theme::default();
        let mut row = post(Vec::new());
        row.author_name = "ada.lovelace".into();
        let laid = lay_out(&mut fonts, &Row::Post { post: row }, &theme);
        let pieces = painter.pieces_of(
            &mut fonts,
            &laid,
            0.0,
            &theme,
            &Palette::default(),
            &HashMap::new(),
        );
        let pressed: Vec<&Press> = pieces
            .iter()
            .filter_map(|piece| match piece {
                Piece::Press { press, .. } => Some(press),
                _ => None,
            })
            .collect();
        assert!(
            pressed.contains(&&Press::Person("ada.lovelace".into())),
            "the author is not pressable in the draw list: {pressed:?}"
        );
        // And quietly: whoever draws the pill behind a mention has to be able
        // to tell that this is not one. Dressed the same way, every message
        // read as though it opened by naming its own writer.
        assert!(
            pieces
                .iter()
                .all(|piece| !matches!(piece, Piece::Press { quiet: false, .. })),
            "the author's name is dressed as a mention"
        );
        // Nor shaded as a link, for the same reason.
        assert!(
            pieces.iter().all(|piece| match piece {
                Piece::Text { glyphs, .. } =>
                    glyphs.iter().all(|glyph| glyph.shade != Shade::Signal),
                _ => true,
            }),
            "the author's name is inked as a link"
        );
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
