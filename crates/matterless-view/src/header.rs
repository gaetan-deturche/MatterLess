//! The strip above the stream: which conversation this is.
//!
//! Small, but it is the thing that says where the reader is. The sidebar
//! scrolls, so the selected row is often out of view, and a stream with no name
//! above it is a wall of text belonging to nobody in particular.
//!
//! Buttons will live here -- pinned, saved, members -- and are left out until
//! there is something for them to open.

use crate::sidebar::Canvas;
use matterless_paint::Run;
use matterless_ui::{Placed, Rect};

/// What the strip says.
pub struct Header {
    pub title: String,
}

/// The strip's height. Fixed: it is one line of text and a rule.
pub const HEIGHT: f32 = 44.0;
/// Where the sigil starts, and how far past it the name does.
const LEFT: f32 = 14.0;
const SIGIL: f32 = 14.0;

impl Header {
    pub fn new(title: impl Into<String>) -> Self {
        Self {
            title: title.into(),
        }
    }

    /// Draws the strip and the rule beneath it.
    pub fn draw(&self, into: &mut Canvas<'_>, within: Rect) {
        let Canvas {
            scene,
            painter,
            fonts,
            palette,
        } = into;
        scene.fill(
            within.x,
            within.y,
            within.width,
            within.height,
            palette.surface,
        );
        // A hairline rather than a border: the strip and the stream are one
        // surface with a change of subject between them.
        scene.fill(
            within.x,
            within.bottom() - 1.0,
            within.width,
            1.0,
            palette.ground,
        );

        // The sigil is faint and the name is not, so the eye lands on the name.
        let sigil = painter.run(
            fonts,
            "#",
            within.x + LEFT,
            within.y + 13.0,
            Run {
                size: 15.0,
                line_height: 20.0,
                bold: false,
                wrap: f32::MAX,
            },
        );
        scene.glyphs(sigil, palette.faint, palette.faint);

        let name = painter.run(
            fonts,
            &self.title,
            within.x + LEFT + SIGIL,
            within.y + 13.0,
            // Never wrapped: the strip is one line tall, and the panel's clip
            // cuts a long name as it does in the sidebar.
            Run {
                size: 15.0,
                line_height: 20.0,
                bold: true,
                wrap: f32::MAX,
            },
        );
        scene.glyphs(name, palette.ink, palette.faint);
    }
}

/// The header's own rectangle at the top of a panel.
pub fn strip(within: Rect) -> Rect {
    Rect::new(within.x, within.y, within.width, HEIGHT.min(within.height))
}

/// What is left of the panel once the header has taken its strip.
pub fn below(within: Rect) -> Rect {
    Rect::new(
        within.x,
        within.y + HEIGHT,
        within.width,
        (within.height - HEIGHT).max(0.0),
    )
}

/// The strip as a box, so the wheel over it does not scroll the stream beneath.
pub fn boxes(within: Rect) -> Vec<Placed> {
    vec![Placed {
        name: "header".to_string(),
        rect: strip(within),
        depth: 1,
    }]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_stream_starts_below_the_strip() {
        let panel = Rect::new(260.0, 0.0, 740.0, 800.0);
        assert_eq!(strip(panel), Rect::new(260.0, 0.0, 740.0, HEIGHT));
        assert_eq!(
            below(panel),
            Rect::new(260.0, HEIGHT, 740.0, 800.0 - HEIGHT)
        );
    }

    /// A window shorter than the strip must not hand the stream a negative
    /// height, which would put every row above the top of it.
    #[test]
    fn a_tiny_window_leaves_no_room_rather_than_negative_room() {
        let panel = Rect::new(0.0, 0.0, 300.0, 20.0);
        assert_eq!(strip(panel).height, 20.0);
        assert_eq!(below(panel).height, 0.0);
    }

    /// The strip covers the top of the panel exactly, with no seam and no
    /// overlap: a gap would show the ground through it.
    #[test]
    fn the_strip_and_the_stream_tile_the_panel() {
        let panel = Rect::new(260.0, 12.0, 740.0, 800.0);
        assert_eq!(strip(panel).bottom(), below(panel).y);
        assert_eq!(below(panel).bottom(), panel.bottom());
    }
}
