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
use matterless_layout::marks;
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
    /// Whether the sigil is one of the interface's own marks rather than a
    /// character. A mark comes from the icon family and has to be asked for.
    pub sigil_is_mark: bool,
    /// The mark before the name, saying what kind of place this is.
    ///
    /// A hash for a channel, which is nearly always right -- and not for the
    /// followed threads, which are every conversation at once and read as a
    /// channel called "Threads" with one on them.
    pub sigil: &'static str,
}

/// The strip's height. Fixed: it is one line of text and a rule.
pub const HEIGHT: f32 = 44.0;
/// What a button's mark is set at.
///
/// An icon family fills the size it is asked for, near enough: a mark at 18
/// measures sixteen pixels across. The symbol font it replaced drew at about
/// two thirds of that, which is why every one of these numbers came down when
/// the family changed and the marks all arrived a size too big.
const MARK: f32 = 18.0;

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
    /// From the bundled icon family, not from whatever the system's symbol
    /// font has at a given codepoint. A control is not content: a row of
    /// little coloured pictures across the top of the strip competes with the
    /// conversation for the eye and reads as something somebody sent.
    ///
    /// One family is also what makes them read as a set. The marks before
    /// these were scavenged from Segoe UI Symbol by measuring which characters
    /// came back as line art at all, and a pushpin drawn as a picture next to
    /// a solid flag next to three lines is not an icon set. Muting now has a
    /// crossed bell, which no symbol font had -- so the toggle is one shape
    /// with and without a stroke through it rather than two unrelated ones.
    pub fn label(self, on: bool) -> &'static str {
        match (self, on) {
            (Act::Pinned, _) => marks::PINNED,
            (Act::Saved, _) => marks::SAVED,
            (Act::Threads, _) => marks::THREADS,
            (Act::Add, _) => marks::ADD_PEOPLE,
            (Act::Mute, false) => marks::BELL,
            (Act::Mute, true) => marks::BELL_OFF,
            (Act::Leave, _) => marks::LEAVE,
            (Act::Follow, false) => "follow",
            (Act::Follow, true) => "following",
            (Act::Close, _) => marks::CLOSE,
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
    // Twelve, not sixteen: a one-line field paints a 32-tall box, and a rect
    // any shorter than that has the box painted into a squeeze rather than
    // drawn at the size it wants.
    Some(Rect::new(
        right - GAP * 3.0 - FIND,
        strip.y + 6.0,
        FIND,
        strip.height - 12.0,
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
            sigil_is_mark: false,
        }
    }

    /// Draws the strip and the rule beneath it.
    /// `typed_in` is true while the search is open and its box is drawn here
    /// by whoever owns it. The placeholder below is what a *shut* search looks
    /// like, and drawing it under a real field would print two hints in one
    /// box.
    pub fn draw(&self, into: &mut Canvas<'_>, within: Rect, hovered: Option<Act>, typed_in: bool) {
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
        let set_in = if self.sigil_is_mark {
            Run::mark(15.0)
        } else {
            Run {
                size: 15.0,
                line_height: 20.0,
                bold: false,
                mono: false,
                wrap: f32::MAX,
                icon: false,
                smooth: false,
            }
        };
        let sigil = painter.run(
            fonts,
            self.sigil,
            within.x + LEFT,
            within.y + if self.sigil_is_mark { 14.0 } else { 13.0 },
            set_in,
        );
        scene.glyphs(sigil, palette.faint, palette.faint);

        // Past the mark, whichever it is. A hash is narrower than the column
        // kept for it; three lines are wider, and at a fixed offset the name
        // lands on top of them.
        let past = matterless_layout::extent_of(
            fonts,
            if self.sigil_is_mark { "#" } else { self.sigil },
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
                icon: false,
                smooth: false,
            },
        );
        scene.glyphs(name, palette.ink, palette.faint);

        // A field rather than a button: it is the only thing on the strip that
        // takes words, and drawing it as anything else would hide that. It is
        // a real one now -- it used to look like this and put the caret in a
        // different box on the far side of the window.
        if let Some(rect) = find(within, &self.offered).filter(|_| !typed_in) {
            scene.rounded(rect.x, rect.y, rect.width, rect.height, palette.raised, 5.0);
            // Where the field's own hint lands, and the same words, so clicking
            // the box does not shift the writing in it. `Run::label` is an
            // eighteen-tall line where the field's is twenty, so it is centred
            // on that rather than sharing the field's six-pixel inset.
            let glyphs = painter.run(
                fonts,
                "Search messages",
                rect.x + 6.0,
                rect.y + (rect.height - 18.0) / 2.0,
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
            // Larger than the words around it, and in the softer ink rather
            // than the faintest. A mark is a picture, not a letter: a bell
            // rasterised at fifteen pixels is twelve across with three lines
            // inside it, and in the faintest ink those lines are a smudge. The
            // sampler is not what makes it one -- a glyph is rasterised at the
            // size it is drawn and sampled one texel to one pixel, so there is
            // nothing to filter. There are simply not enough pixels.
            let glyphs = painter.run(
                fonts,
                act.label(self.muted),
                rect.x + 6.0,
                rect.y - 1.0,
                Run::mark(MARK),
            );
            scene.glyphs(
                glyphs,
                if lit { palette.ink } else { palette.soft },
                palette.soft,
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
