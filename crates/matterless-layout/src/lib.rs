//! Exact text layout for the message list.
//!
//! The virtualised list needs a row's height *before* the row exists. A browser
//! will not answer that: it reports a height only once it has laid the row out,
//! which is a frame after the question was asked. Everything built around that
//! gap -- estimating heights, measuring afterwards, reconciling the two, holding
//! the reader's place across the correction -- was the source of every scroll
//! artefact in this app.
//!
//! Here the height is computed instead. `cosmic-text` shapes the text with the
//! real font, applies Unicode line breaking, and reports how many lines it
//! occupies at a given width. Nothing is predicted, so nothing has to be
//! corrected.
//!
//! This crate is deliberately independent of any renderer: it answers "how tall
//! and where does each glyph sit", which a GPU renderer draws and which tests
//! can check without a GPU at all.

pub mod row;

use cosmic_text::{Attrs, Buffer, Family, FontSystem, Metrics, Shaping, Weight};

/// The fonts, loaded once. Building this scans the system's font directories,
/// which is slow enough that it must not happen per row.
pub struct Fonts {
    /// Visible to `row`, which lays out whole rows against the same fonts.
    pub(crate) system: FontSystem,
}

impl Default for Fonts {
    fn default() -> Self {
        Self::new()
    }
}

impl Fonts {
    pub fn new() -> Self {
        Self {
            system: FontSystem::new(),
        }
    }
}

/// How a run of text is drawn: the parts that change its width.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Style {
    pub size: f32,
    pub line_height: f32,
    pub bold: bool,
    pub italic: bool,
    /// Monospace, for code.
    pub mono: bool,
}

impl Style {
    /// The body of a message, matching the stylesheet's 14px/1.5.
    pub fn body() -> Self {
        Self {
            size: 14.0,
            line_height: 21.0,
            bold: false,
            italic: false,
            mono: false,
        }
    }
}

/// What a piece of text occupies once laid out.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Extent {
    pub lines: usize,
    pub height: f32,
    /// The widest line, which is what a horizontally scrolling block needs.
    pub width: f32,
}

/// Lays out one run of text at `width` and reports what it occupies.
///
/// The width is the width the text actually gets -- inside whatever gutter the
/// caller draws around it. Getting that wrong by even a few percent moves the
/// line count by one on a great many messages, which is a whole line of height.
pub fn extent_of(fonts: &mut Fonts, text: &str, width: f32, style: Style) -> Extent {
    let metrics = Metrics::new(style.size, style.line_height);
    let mut buffer = Buffer::new(&mut fonts.system, metrics);
    let mut buffer = buffer.borrow_with(&mut fonts.system);
    // No height bound: the question is how tall this becomes, not how much of
    // it fits in a box.
    buffer.set_size(Some(width), None);

    let mut attrs = Attrs::new();
    if style.mono {
        attrs = attrs.family(Family::Monospace);
    }
    if style.bold {
        attrs = attrs.weight(Weight::BOLD);
    }
    if style.italic {
        attrs = attrs.style(cosmic_text::Style::Italic);
    }
    // `Shaping::Advanced` rather than `Basic`: basic shaping cannot handle
    // ligatures or scripts that need reordering, and a wrong advance is a wrong
    // line count.
    // No alignment: the list is left-aligned, and alignment cannot change how
    // many lines the text needs.
    buffer.set_text(text, &attrs, Shaping::Advanced, None);
    buffer.shape_until_scroll(false);

    let lines = buffer.layout_runs().count().max(1);
    let widest = buffer
        .layout_runs()
        .map(|run| run.line_w)
        .fold(0.0_f32, f32::max);
    Extent {
        lines,
        height: lines as f32 * style.line_height,
        width: widest,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The property the whole crate exists for: the same text at two widths has
    /// two known heights, and both are known before anything is drawn.
    #[test]
    fn a_narrower_column_is_taller_and_both_are_known() {
        let mut fonts = Fonts::new();
        let text = "The quick brown fox jumps over the lazy dog, and then keeps \
                    going for long enough to need more than one line at any \
                    sensible width.";
        let wide = extent_of(&mut fonts, text, 900.0, Style::body());
        let narrow = extent_of(&mut fonts, text, 300.0, Style::body());
        assert!(
            narrow.lines > wide.lines,
            "narrower has to wrap more: {narrow:?} vs {wide:?}"
        );
        assert_eq!(wide.height, wide.lines as f32 * 21.0);
        assert_eq!(narrow.height, narrow.lines as f32 * 21.0);
    }

    #[test]
    fn one_short_line_is_one_line() {
        let mut fonts = Fonts::new();
        let extent = extent_of(&mut fonts, "Yo!", 600.0, Style::body());
        assert_eq!(extent.lines, 1);
        assert_eq!(extent.height, 21.0);
        assert!(extent.width > 0.0 && extent.width < 600.0);
    }

    /// Bold is wider, so the same text can need another line.
    #[test]
    fn weight_changes_the_width() {
        let mut fonts = Fonts::new();
        let text = "measured against the real font rather than an average glyph";
        let plain = extent_of(&mut fonts, text, 400.0, Style::body());
        let bold = extent_of(
            &mut fonts,
            text,
            400.0,
            Style {
                bold: true,
                ..Style::body()
            },
        );
        assert!(
            bold.width >= plain.width,
            "bold cannot be narrower: {bold:?} vs {plain:?}"
        );
    }

    #[test]
    fn empty_text_still_occupies_a_line() {
        let mut fonts = Fonts::new();
        let extent = extent_of(&mut fonts, "", 400.0, Style::body());
        assert_eq!(extent.lines, 1);
    }
}
