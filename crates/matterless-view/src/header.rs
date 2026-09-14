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
    /// Whether this conversation is muted, which is the one button whose word
    /// changes with the state it is in.
    pub muted: bool,
    /// The mark before the name, saying what kind of place this is.
    ///
    /// A hash for a channel, which is nearly always right -- and not for the
    /// followed threads, which are every conversation at once and read as a
    /// channel called "Threads" with one on them.
    pub sigil: &'static str,
}

/// The strip's height. Fixed: it is one line of text and a rule.
pub const HEIGHT: f32 = 44.0;
/// Where the sigil starts, and how far past it the name does.
const LEFT: f32 = 14.0;
const SIGIL: f32 = 14.0;
/// A button in the strip, and the gap between two of them.
const BUTTON: f32 = 30.0;
const GAP: f32 = 4.0;
/// The search field in the middle of the strip, which is where the app puts
/// it and where a reader raised on any chat client will look.
const FIND: f32 = 240.0;
/// The follow button, which is a word and not a mark.
const FOLLOW: f32 = 72.0;

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
    /// Put somebody else in this channel.
    Add,
    /// Stop this conversation counting unread, or start again.
    Mute,
    /// Stop being in this channel.
    Leave,
    /// Start or stop following this thread, which is what decides whether its
    /// replies interrupt the reader.
    Follow,
    /// Shut the thread pane.
    Close,
}

impl Act {
    /// What the button shows.
    ///
    /// A mark rather than a word, which is what the app has. Six words is a
    /// sentence across the top of the strip, and it left no room for the
    /// search field that belongs in the middle of it.
    ///
    /// `on` only means anything to the one that is a toggle: a state a button
    /// can be in has to read differently, or pressing it twice looks like
    /// nothing happened.
    ///
    /// Every one of these is a character the fonts here draw as line art, and
    /// none of them is the obvious emoji for the job. A control is not
    /// content: a row of little coloured pictures across the top of the strip
    /// competes with the conversation for the eye, and reads as something
    /// somebody sent rather than as something to press.
    ///
    /// Chosen by measuring rather than by guessing. U+FE0E, the text
    /// presentation selector, is the proper way to ask for the monochrome form
    /// and this font stack ignores it -- so the mark has to be a character
    /// with no coloured form at all, and which those are is what
    /// `--example what_marks` answers: it rasterises each candidate and says
    /// whether what came back was a mask or a bitmap.
    ///
    /// Threads gets the three lines its sidebar row uses, because they are the
    /// same place. Muting has no crossed bell in line art anywhere, so the
    /// pair is a bell and a struck-out circle -- two shapes rather than one
    /// shape twice, which is what a toggle needs.
    pub fn label(self, on: bool) -> &'static str {
        match (self, on) {
            (Act::Pinned, _) => "🖈",
            (Act::Saved, _) => "⚑",
            (Act::Threads, _) => "☰",
            (Act::Add, _) => "⊕",
            (Act::Mute, false) => "🕭",
            (Act::Mute, true) => "⊘",
            (Act::Leave, _) => "⎋",
            (Act::Follow, false) => "follow",
            (Act::Follow, true) => "following",
            (Act::Close, _) => "×",
        }
    }

    /// What the mark means, for somebody who has stopped to ask.
    ///
    /// A pin, a bookmark, a bell and a door say nothing on their own. In the
    /// app every one of these carries a `title` and the browser explains it
    /// for nothing; a native window has to be told to. The words are the
    /// app's own, so the two clients answer the same question the same way.
    pub fn explains(self, on: bool) -> &'static str {
        match (self, on) {
            (Act::Pinned, _) => "Pinned messages",
            (Act::Saved, _) => "Saved messages",
            (Act::Threads, _) => "Threads",
            (Act::Add, _) => "Add Members",
            (Act::Mute, false) => "Mute Channel",
            (Act::Mute, true) => "Unmute Channel",
            (Act::Leave, _) => "Leave Channel",
            (Act::Follow, false) => "Follow thread",
            (Act::Follow, true) => "Unfollow thread",
            (Act::Close, _) => "Close thread",
        }
    }

    /// What a hit test carries, which never changes with the state: a click on
    /// "unmute" has to land on the same button "mute" did.
    pub fn slug(self) -> &'static str {
        match self {
            Act::Pinned => "pinned",
            Act::Saved => "saved",
            Act::Threads => "threads",
            Act::Add => "add",
            Act::Mute => "mute",
            Act::Leave => "leave",
            Act::Follow => "follow",
            Act::Close => "close",
        }
    }

    pub fn from_slug(slug: &str) -> Option<Self> {
        Some(match slug {
            "pinned" => Act::Pinned,
            "saved" => Act::Saved,
            "threads" => Act::Threads,
            "add" => Act::Add,
            "mute" => Act::Mute,
            "leave" => Act::Leave,
            "follow" => Act::Follow,
            "close" => Act::Close,
            _ => return None,
        })
    }
}

/// What a thread pane's strip offers: whether to keep hearing about it, and a
/// way out. Nothing on the channel's strip applies to one thread.
pub fn for_thread() -> Vec<Act> {
    vec![Act::Follow, Act::Close]
}

/// What the strip offers for this conversation.
///
/// Leaving is only offered where it means something: a direct message cannot
/// be left, and the server would refuse. The rest are the reader's own lists
/// and are the same everywhere.
pub fn offered(direct: bool) -> Vec<Act> {
    // Muting is offered everywhere, including a direct message: a conversation
    // that need not interrupt you is not only ever a channel.
    let mut offered = vec![Act::Threads, Act::Saved, Act::Pinned, Act::Mute];
    // Adding and leaving both belong to a channel. A direct message's
    // membership is the two people in it, and the server decides that.
    if !direct {
        offered.push(Act::Add);
        offered.push(Act::Leave);
    }
    offered
}

/// The search field, in the middle of the strip.
///
/// `None` when the strip is too narrow to hold one without crowding the name
/// on its left or the buttons on its right: a field squeezed between two
/// things it collides with is worse than a keystroke.
pub fn find(within: Rect, offered: &[Act]) -> Option<Rect> {
    let strip = strip(within);
    let buttons = place(within, offered);
    let right = buttons
        .first()
        .map(|(_, rect)| rect.x)
        .unwrap_or(strip.right());
    let room = right - (strip.x + LEFT + SIGIL + 160.0);
    if room < FIND {
        return None;
    }
    Some(Rect::new(
        right - GAP * 3.0 - FIND,
        strip.y + 8.0,
        FIND,
        strip.height - 16.0,
    ))
}

/// Where each button sits, laid out from the right edge inwards.
pub fn place(within: Rect, offered: &[Act]) -> Vec<(Act, Rect)> {
    let strip = strip(within);
    let mut placed = Vec::new();
    let mut right = strip.right() - GAP;
    for act in offered.iter().rev() {
        // One of them is a word rather than a mark, because "following" is a
        // state and a mark cannot hold one.
        let width = match act {
            Act::Follow => FOLLOW,
            _ => BUTTON,
        };
        right -= width;
        placed.push((
            *act,
            Rect::new(right, strip.y + 10.0, width, strip.height - 20.0),
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
            muted: false,
            sigil: "#",
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
            self.sigil,
            within.x + LEFT,
            within.y + 13.0,
            Run {
                size: 15.0,
                line_height: 20.0,
                bold: false,
                mono: false,
                wrap: f32::MAX,
            },
        );
        scene.glyphs(sigil, palette.faint, palette.faint);

        // Past the mark, whichever it is. A hash is narrower than the column
        // kept for it; three lines are wider, and at a fixed offset the name
        // lands on top of them.
        let past = matterless_layout::extent_of(
            fonts,
            self.sigil,
            f32::MAX,
            matterless_layout::Style {
                size: 15.0,
                line_height: 20.0,
                bold: false,
                italic: false,
                mono: false,
            },
        )
        .width
            + 4.0;
        let name = painter.run(
            fonts,
            &self.title,
            within.x + LEFT + SIGIL.max(past),
            within.y + 13.0,
            // Never wrapped: the strip is one line tall, and the panel's clip
            // cuts a long name as it does in the sidebar.
            Run {
                size: 15.0,
                line_height: 20.0,
                bold: true,
                mono: false,
                wrap: f32::MAX,
            },
        );
        scene.glyphs(name, palette.ink, palette.faint);

        // A field rather than a button: it is the only thing on the strip that
        // takes words, and drawing it as anything else would hide that.
        if let Some(rect) = find(within, &self.offered) {
            scene.rounded(rect.x, rect.y, rect.width, rect.height, palette.raised, 5.0);
            let glyphs = painter.run(
                fonts,
                "Search messages...",
                rect.x + 10.0,
                rect.y + 4.0,
                Run::label(f32::MAX),
            );
            scene.glyphs(glyphs, palette.faint, palette.faint);
        }

        // Right-aligned, because the left is where the name is and the eye
        // reads from there.
        for (act, rect) in place(within, &self.offered) {
            let lit = hovered == Some(act);
            if lit {
                scene.fill(rect.x, rect.y, rect.width, rect.height, palette.ground);
            }
            let glyphs = painter.run(
                fonts,
                act.label(self.muted),
                rect.x + 7.0,
                rect.y + 1.0,
                Run {
                    size: 15.0,
                    line_height: 20.0,
                    bold: false,
                    mono: false,
                    wrap: f32::MAX,
                },
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
    if let Some(rect) = find(within, offered) {
        placed.push(Placed {
            name: "header/find".to_string(),
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
        for only_a_channel in [Act::Leave, Act::Add] {
            assert!(offered(false).contains(&only_a_channel));
            assert!(!offered(true).contains(&only_a_channel));
        }
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
            // And the slug does not move when the word does, or a click on
            // "unmute" lands on nothing.
            assert_eq!(Act::from_slug(act.slug()), Some(act));
        }
        assert_ne!(Act::Mute.label(false), Act::Mute.label(true));
        assert_eq!(Act::Mute.slug(), "mute");
        assert_eq!(Act::from_slug("nonsense"), None);
    }

    /// In order, inside the strip, without overlapping -- and never over the
    /// name, which is what the strip is for.
    #[test]
    fn the_buttons_sit_inside_the_strip() {
        let panel = Rect::new(260.0, 0.0, 700.0, 600.0);
        let placed = place(panel, &offered(false));
        assert_eq!(placed.len(), 6);
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
