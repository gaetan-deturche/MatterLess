//! The strip above the stream: which conversation this is.
//!
//! Small, but it is the thing that says where the reader is. The sidebar
//! scrolls, so the selected row is often out of view, and a stream with no name
//! above it is a wall of text belonging to nobody in particular.
//!
//! The buttons live here too, now that there is something for them to open.
//! They are the only visible way to reach the lists: a keystroke nobody has
//! been told about is not a feature anybody has.

use crate::sidebar::Canvas;
use matterless_paint::Run;
use matterless_ui::{Placed, Rect};

/// What the strip says.
pub struct Header {
    pub title: String,
    /// What this conversation offers, decided by the caller: only it knows
    /// whether the conversation can be left.
    pub offered: Vec<Act>,
}

/// The strip's height. Fixed: it is one line of text and a rule.
pub const HEIGHT: f32 = 44.0;
/// Where the sigil starts, and how far past it the name does.
const LEFT: f32 = 14.0;
const SIGIL: f32 = 14.0;
/// A button in the strip, and the gap between two of them.
const BUTTON: f32 = 62.0;
const GAP: f32 = 4.0;

/// What a button in the strip does.
///
/// Only what this window can already do. A button that opens nothing is worse
/// than one that is not there, which is why this strip had none until now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Act {
    /// The messages pinned in this conversation.
    Pinned,
    /// The messages this reader saved, anywhere.
    Saved,
    /// The threads they follow.
    Threads,
    /// Stop being in this channel.
    Leave,
}

impl Act {
    pub fn label(self) -> &'static str {
        match self {
            Act::Pinned => "pinned",
            Act::Saved => "saved",
            Act::Threads => "threads",
            Act::Leave => "leave",
        }
    }

    pub fn slug(self) -> &'static str {
        self.label()
    }

    pub fn from_slug(slug: &str) -> Option<Self> {
        Some(match slug {
            "pinned" => Act::Pinned,
            "saved" => Act::Saved,
            "threads" => Act::Threads,
            "leave" => Act::Leave,
            _ => return None,
        })
    }
}

/// What the strip offers for this conversation.
///
/// Leaving is only offered where it means something: a direct message cannot
/// be left, and the server would refuse. The rest are the reader's own lists
/// and are the same everywhere.
pub fn offered(direct: bool) -> Vec<Act> {
    let mut offered = vec![Act::Threads, Act::Saved, Act::Pinned];
    if !direct {
        offered.push(Act::Leave);
    }
    offered
}

/// Where each button sits, laid out from the right edge inwards.
pub fn place(within: Rect, offered: &[Act]) -> Vec<(Act, Rect)> {
    let strip = strip(within);
    let mut placed = Vec::new();
    let mut right = strip.right() - GAP;
    for act in offered.iter().rev() {
        right -= BUTTON;
        placed.push((
            *act,
            Rect::new(right, strip.y + 10.0, BUTTON, strip.height - 20.0),
        ));
        right -= GAP;
    }
    placed.reverse();
    placed
}

impl Header {
    pub fn new(title: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            offered: offered(false),
        }
    }

    /// Draws the strip and the rule beneath it.
    pub fn draw(&self, into: &mut Canvas<'_>, within: Rect, hovered: Option<Act>) {
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

        // Right-aligned, because the left is where the name is and the eye
        // reads from there.
        for (act, rect) in place(within, &self.offered) {
            let lit = hovered == Some(act);
            if lit {
                scene.fill(rect.x, rect.y, rect.width, rect.height, palette.ground);
            }
            let glyphs = painter.run(
                fonts,
                act.label(),
                rect.x + 8.0,
                rect.y + 2.0,
                Run::label(f32::MAX),
            );
            scene.glyphs(
                glyphs,
                if lit { palette.ink } else { palette.faint },
                palette.faint,
            );
        }
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

/// The strip as a box, so the wheel over it does not scroll the stream
/// beneath, plus one for each button on it.
pub fn boxes(within: Rect, offered: &[Act]) -> Vec<Placed> {
    let mut placed = vec![Placed {
        name: "header".to_string(),
        rect: strip(within),
        depth: 1,
    }];
    for (act, rect) in place(within, offered) {
        placed.push(Placed {
            name: format!("header/{}", act.slug()),
            rect,
            depth: 2,
        });
    }
    placed
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A direct message cannot be left -- the server would refuse -- so the
    /// button is not there rather than there and refused.
    #[test]
    fn leaving_is_only_offered_where_it_means_something() {
        assert!(offered(false).contains(&Act::Leave));
        assert!(!offered(true).contains(&Act::Leave));
        // The lists are the reader's own and are the same everywhere.
        for direct in [true, false] {
            assert!(offered(direct).contains(&Act::Saved));
            assert!(offered(direct).contains(&Act::Threads));
        }
    }

    /// Every slug survives the round trip, or a click lands on nothing.
    #[test]
    fn a_slug_names_exactly_one_button() {
        for act in offered(false) {
            assert_eq!(Act::from_slug(act.slug()), Some(act));
        }
        assert_eq!(Act::from_slug("nonsense"), None);
    }

    /// In order, inside the strip, without overlapping -- and never over the
    /// name, which is what the strip is for.
    #[test]
    fn the_buttons_sit_inside_the_strip() {
        let panel = Rect::new(260.0, 0.0, 700.0, 600.0);
        let placed = place(panel, &offered(false));
        assert_eq!(placed.len(), 4);
        for pair in placed.windows(2) {
            assert!(pair[0].1.right() <= pair[1].1.x);
        }
        let strip = strip(panel);
        assert!(placed[0].1.x > strip.x + LEFT + SIGIL);
        let last = placed.last().expect("a button");
        assert!(last.1.right() <= strip.right());
        for (_, rect) in &placed {
            assert!(rect.y >= strip.y && rect.bottom() <= strip.bottom());
        }
    }

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
