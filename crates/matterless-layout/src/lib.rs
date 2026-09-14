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

pub mod marks;
pub mod row;

use cosmic_text::{Attrs, Buffer, Family, FontSystem, Metrics, Shaping, Weight};

/// The fonts, loaded once. Building this scans the system's font directories,
/// which is slow enough that it must not happen per row.
pub struct Fonts {
    /// Visible to `row`, which lays out whole rows against the same fonts.
    pub(crate) system: FontSystem,
}

/// The monospace face, found once and named.
///
/// `Family::Monospace` is a generic, and cosmic-text resolves a generic
/// against the whole font database for every buffer that asks for one. Naming
/// the face it resolves to costs a quarter as much: measured at 110ms against
/// 26ms for the same hundred thousand characters, which is slower than plain
/// text rather than faster. A channel of crash reports is mostly code blocks,
/// and it was paying that lookup for every line of every one of them.
///
/// Global rather than per-`Fonts`, because the answer is a property of the
/// machine: two `Fonts` built from the same system resolve the same face.
static MONO: std::sync::OnceLock<String> = std::sync::OnceLock::new();

/// The monospace face by name, for anything shaping code.
pub fn mono_family() -> Family<'static> {
    match MONO.get() {
        Some(name) => Family::Name(name),
        // Nothing has built a `Fonts` yet, so nothing has looked. The generic
        // is still right, only slower.
        None => Family::Monospace,
    }
}

impl Default for Fonts {
    fn default() -> Self {
        Self::new()
    }
}

impl Fonts {
    /// The loaded fonts, for a renderer that has to shape the very same text.
    ///
    /// Exposed rather than duplicated: a renderer with its own `FontSystem`
    /// could resolve a different face for the same request and draw a different
    /// number of lines than the layout reserved.
    pub fn system_mut(&mut self) -> &mut FontSystem {
        &mut self.system
    }

    pub fn new() -> Self {
        let mut system = FontSystem::new();
        MONO.get_or_init(|| {
            let query = cosmic_text::fontdb::Query {
                families: &[cosmic_text::fontdb::Family::Monospace],
                ..Default::default()
            };
            system
                .db()
                .query(&query)
                .and_then(|id| system.db().face(id))
                .map(|face| face.families[0].0.clone())
                // No monospace face at all. `Family::Monospace` would find
                // nothing either, so the name is only a label.
                .unwrap_or_else(|| "monospace".to_string())
        });
        // The interface's own marks, bundled rather than hoped for: a system
        // symbol font has *a* glyph for most of these and they do not belong
        // to one another. Loaded into the same `FontSystem` everything else
        // shapes against, so a mark resolves the same way a letter does.
        system
            .db_mut()
            .load_font_data(include_bytes!("../resources/fonts/lucide.ttf").to_vec());
        Self { system }
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
/// The character that stands in for what was cut.
///
/// One glyph rather than three dots: `text-overflow: ellipsis` is a single
/// character, and three periods are wider and read as a pause in the name.
pub const ELLIPSIS: char = '\u{2026}';

/// `text` cut to fit `width`, with an ellipsis where it was cut.
///
/// What `text-overflow: ellipsis` does, and it has to be done here rather than
/// left to a clip: a clipped name ends mid-letter, so there is no way to tell a
/// channel whose name is long from one whose name happens to end there. And a
/// clip only hides what is drawn inside the panel -- it says nothing about a
/// name running under the scrollbar.
///
/// Measured rather than counted, because the answer depends on the letters: a
/// name of fifteen `i`s and one of fifteen `W`s do not end in the same place.
pub fn elided(fonts: &mut Fonts, text: &str, width: f32, style: Style) -> String {
    if width <= 0.0 {
        return String::new();
    }
    if extent_of(fonts, text, f32::MAX, style).width <= width {
        return text.to_string();
    }
    // The longest prefix that still fits once the ellipsis is on it. Found by
    // halving rather than by walking: a sidebar is hundreds of rows and a
    // channel name is dozens of characters, and this runs on every frame.
    let ends: Vec<usize> = text
        .char_indices()
        .map(|(at, _)| at)
        .chain(std::iter::once(text.len()))
        .collect();
    let mut low = 0usize;
    let mut high = ends.len() - 1;
    while low < high {
        let middle = (low + high).div_ceil(2);
        let trial = format!("{}{ELLIPSIS}", &text[..ends[middle]]);
        if extent_of(fonts, &trial, f32::MAX, style).width <= width {
            low = middle;
        } else {
            high = middle - 1;
        }
    }
    // Nothing fits: the ellipsis alone still says the name goes on, where an
    // empty row says the channel has no name.
    format!("{}{ELLIPSIS}", &text[..ends[low]])
}

pub fn extent_of(fonts: &mut Fonts, text: &str, width: f32, style: Style) -> Extent {
    let metrics = Metrics::new(style.size, style.line_height);
    let mut buffer = Buffer::new(&mut fonts.system, metrics);
    let mut buffer = buffer.borrow_with(&mut fonts.system);
    // No height bound: the question is how tall this becomes, not how much of
    // it fits in a box.
    buffer.set_size(Some(width), None);

    let mut attrs = Attrs::new();
    if style.mono {
        attrs = attrs.family(mono_family());
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
mod elide_tests {
    use super::*;

    fn style() -> Style {
        Style {
            size: 13.0,
            line_height: 18.0,
            bold: false,
            italic: false,
            mono: false,
        }
    }

    /// A name that fits is left alone: an ellipsis on a name with room to
    /// spare is a lie about the name.
    #[test]
    fn a_name_that_fits_is_untouched() {
        let mut fonts = Fonts::new();
        assert_eq!(elided(&mut fonts, "Hot Line", 400.0, style()), "Hot Line");
    }

    /// One that does not is cut, and says so.
    #[test]
    fn a_name_that_does_not_fit_is_cut_and_says_so() {
        let mut fonts = Fonts::new();
        let long = "Voyager | Demande d'archivage Branch";
        let cut = elided(&mut fonts, long, 120.0, style());
        assert!(cut.ends_with(ELLIPSIS), "{cut}");
        assert!(cut.chars().count() < long.chars().count());
        assert!(long.starts_with(cut.trim_end_matches(ELLIPSIS)));
        // And it actually fits, which is the whole point.
        assert!(extent_of(&mut fonts, &cut, f32::MAX, style()).width <= 120.0);
    }

    /// The width is measured, not counted: the same number of wide letters
    /// takes more room than narrow ones, so they cannot cut in the same place.
    #[test]
    fn where_it_cuts_depends_on_the_letters() {
        let mut fonts = Fonts::new();
        let narrow = elided(&mut fonts, &"i".repeat(60), 100.0, style());
        let wide = elided(&mut fonts, &"W".repeat(60), 100.0, style());
        assert!(
            narrow.chars().count() > wide.chars().count(),
            "{} narrow against {} wide",
            narrow.chars().count(),
            wide.chars().count()
        );
    }

    /// No room at all still says the name goes on, where an empty row would
    /// say the channel has no name.
    #[test]
    fn nothing_fitting_still_says_something() {
        let mut fonts = Fonts::new();
        assert_eq!(elided(&mut fonts, "anything", 1.0, style()), "\u{2026}");
        assert_eq!(elided(&mut fonts, "anything", 0.0, style()), "");
    }

    /// Cutting has to land on a character, not inside one: slicing a string by
    /// bytes panics halfway through a letter, and a sidebar is full of them.
    #[test]
    fn it_cuts_between_letters_not_inside_them() {
        let mut fonts = Fonts::new();
        for width in [3.0, 9.0, 17.0, 44.0, 90.0] {
            let cut = elided(&mut fonts, "Curiosite\u{301} | Cymatique\u{301}s", width, style());
            assert!(cut.is_char_boundary(cut.len()));
        }
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
