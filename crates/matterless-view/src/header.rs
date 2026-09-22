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
/// The narrowest it is worth typing into. Below this it becomes its mark.
///
/// A field has to show enough of what was typed to be worth typing in. Past
/// that it is a box that swallows words, and the mark at least says what
/// pressing it is for.
const FIND_MIN: f32 = 120.0;
/// What is kept for the conversation's name before anything else is placed.
///
/// The strip exists to say where the reader is, so the name is the last
/// thing to give ground rather than the first.
const NAME: f32 = 160.0;
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
    /// Search this conversation.
    ///
    /// Offered like the rest, and drawn unlike them: `fit` gives it a field
    /// to type in while there is room for one and this mark when there is
    /// not. Offering it is how a strip says it has a search at all -- the
    /// thread pane's does not, and so never grows one.
    Search,
    /// Everything that did not fit, behind one mark.
    ///
    /// Never offered: `fit` puts it on the row when it has had to fold
    /// something away, so no caller has to know the strip's arithmetic.
    More,
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
            (Act::Search, _) => marks::SEARCH,
            (Act::More, _) => marks::MORE,
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
            (Act::Search, _) => "Search messages",
            (Act::More, _) => "More",
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
            Act::Search => "search",
            Act::More => "more",
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
            "search" => Act::Search,
            "more" => Act::More,
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
    // Search first, which is both where it is drawn and where it stands in
    // the order things are given up: it is the last to fold away, being the
    // one a reader reaches for most.
    //
    // Muting is offered everywhere, including a direct message: a conversation
    // that need not interrupt you is not only ever a channel.
    let mut offered = vec![
        Act::Search,
        Act::Threads,
        Act::Saved,
        Act::Pinned,
        Act::Mute,
    ];
    // Adding and leaving both belong to a channel. A direct message's
    // membership is the two people in it, and the server decides that.
    if !direct {
        offered.push(Act::Add);
        offered.push(Act::Leave);
    }
    offered
}

/// What the strip can actually show, once everything has been asked to fit.
///
/// A strip narrows for two reasons -- the window shrinks, or the thread pane
/// takes half the column -- and it gives ground in an order. The field goes
/// first, because it is the widest thing on the strip and the only one that
/// can be narrower and still itself: it shrinks to `FIND_MIN`, then becomes
/// the mark it stands for, and only when that is not enough do the buttons
/// fold away behind an ellipsis.
///
/// The name never gives ground: `NAME` is kept for it before anything else
/// is placed. The strip exists to say where the reader is, and everything
/// else on it is a convenience by comparison.
#[derive(Debug, Clone, PartialEq)]
pub struct Strip {
    /// The field, when there is room to type in one.
    pub field: Option<Rect>,
    /// The buttons on show, left to right. `Act::Search` is among them once
    /// the field has become its mark, and `Act::More` is last whenever
    /// anything is folded behind it.
    pub shown: Vec<(Act, Rect)>,
    /// What `Act::More` opens, in the order they were offered.
    pub folded: Vec<Act>,
}

/// How wide one button is. Only one of them is a word.
fn width_of(act: Act) -> f32 {
    match act {
        // "following" is a state, and a mark cannot hold one.
        Act::Follow => FOLLOW,
        _ => BUTTON,
    }
}

/// Divides the strip up between the name, the field and the buttons.
///
/// `offered` is a priority order as well as a left-to-right one: folding
/// takes from the end, so whatever is listed last is the first to go behind
/// the ellipsis. Leaving a channel is last for that reason -- it is the
/// rarest thing on the strip and the one worth a deliberate press.
pub fn fit(within: Rect, offered: &[Act]) -> Strip {
    let strip = strip(within);
    let room = (strip.width - LEFT - SIGIL - NAME - GAP).max(0.0);
    let cost = |act: Act| width_of(act) + GAP;

    // The field is the one thing on the strip that is not a button, so it is
    // taken out of the row and given whatever the buttons leave.
    let searchable = offered.contains(&Act::Search);
    let buttons: Vec<Act> = offered
        .iter()
        .copied()
        .filter(|act| *act != Act::Search)
        .collect();
    let wanted: f32 = buttons.iter().copied().map(cost).sum();

    // Every button, and a field with whatever is left over.
    let spare = room - wanted;
    if searchable && spare >= FIND_MIN {
        let field = spare.min(FIND);
        let placed = lay(strip, &buttons);
        let right = placed
            .first()
            .map(|(_, rect)| rect.x)
            .unwrap_or(strip.right());
        return Strip {
            // Twelve, not sixteen: a one-line field paints a 32-tall box, and
            // a rect any shorter has the box painted into a squeeze rather
            // than drawn at the size it wants.
            field: Some(Rect::new(
                right - GAP * 2.0 - field,
                strip.y + 6.0,
                field,
                strip.height - 12.0,
            )),
            shown: placed,
            folded: Vec::new(),
        };
    }

    // The field becomes its mark, which joins the row where the field was.
    let mut shown: Vec<Act> = offered.to_vec();
    let mut folded: Vec<Act> = Vec::new();
    while !shown.is_empty() {
        let ellipsis = if folded.is_empty() {
            0.0
        } else {
            cost(Act::More)
        };
        let taken: f32 = shown.iter().copied().map(cost).sum();
        if taken + ellipsis <= room {
            break;
        }
        // From the end, so the menu comes back out in the order it went in.
        folded.insert(0, shown.pop().expect("a button to fold"));
    }
    if !folded.is_empty() {
        shown.push(Act::More);
    }
    Strip {
        field: None,
        shown: lay(strip, &shown),
        folded,
    }
}

/// Where each button sits, laid out from the right edge inwards.
fn lay(strip: Rect, acts: &[Act]) -> Vec<(Act, Rect)> {
    let mut placed = Vec::new();
    let mut right = strip.right() - GAP;
    for act in acts.iter().rev() {
        let width = width_of(*act);
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

/// The search field, when the strip is wide enough to hold one.
pub fn find(within: Rect, offered: &[Act]) -> Option<Rect> {
    fit(within, offered).field
}

/// Where each button that is shown sits, left to right.
pub fn place(within: Rect, offered: &[Act]) -> Vec<(Act, Rect)> {
    fit(within, offered).shown
}

/// What the ellipsis on this strip would open, if it is on it at all.
pub fn folded(within: Rect, offered: &[Act]) -> Vec<Act> {
    fit(within, offered).folded
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
    /// How the name is set, which is what measures it.
    fn name_style() -> matterless_layout::Style {
        matterless_layout::Style {
            size: 15.0,
            line_height: 20.0,
            bold: true,
            italic: false,
            mono: false,
        }
    }

    /// Where the name starts, and how much room it has before the first thing
    /// on the right of the strip.
    ///
    /// The name used to be drawn with no width at all and left to the scene's
    /// clip, "as it does in the sidebar". The sidebar clips each row to its
    /// own width and the strip is clipped to the whole strip, so a name longer
    /// than the room did not stop at the buttons: it ran under every one of
    /// them and out to the edge of the column. A group conversation is named
    /// after the people in it, so five names and a thread pane open is all it
    /// took, and then nothing on the strip could be read or found.
    pub fn name_box(&self, fonts: &mut matterless_layout::Fonts, within: Rect) -> (f32, f32) {
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
        let from = within.x + LEFT + SIGIL.max(past);
        let laid = fit(within, &self.offered);
        // Whichever is leftmost: the field when there is one, otherwise the
        // first button, otherwise the edge.
        let until = laid
            .field
            .map(|field| field.x)
            .or_else(|| laid.shown.first().map(|(_, rect)| rect.x))
            .unwrap_or_else(|| strip(within).right());
        (from, (until - from - GAP).max(0.0))
    }

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

        let (from, room) = self.name_box(fonts, within);
        // Cut to that room rather than wrapped: the strip is one line tall,
        // and an ellipsis says a name goes on where a name simply stopping
        // would read as the whole of it.
        let shown = matterless_layout::elided(fonts, &self.title, room, Self::name_style());
        let name = painter.run(
            fonts,
            &shown,
            from,
            within.y + 13.0,
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
            // Cut to the field, which is no longer always 240 wide: it
            // narrows before it gives up and becomes a mark, and a hint
            // running out of its own box reads as a broken field.
            let said = matterless_layout::elided(
                fonts,
                "Search messages",
                rect.width - 12.0,
                crate::listing::label(false),
            );
            let glyphs = painter.run(
                fonts,
                &said,
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

    /// The buttons are laid in from the right edge of what they are given,
    /// so which rect the caller hands over decides where they end up.
    ///
    /// Worth a test of its own because getting it wrong is invisible in this
    /// module and fatal outside it: the window asked with the whole column,
    /// thread pane included, and every button slid under the pane -- where
    /// the pane's own header was then drawn over them. The buttons did not
    /// move a pixel from where they were told to go.
    #[test]
    fn the_buttons_follow_the_right_edge_they_are_given() {
        // Two widths that both show every button, so the only difference
        // between them is the edge they were laid in from.
        let whole = Rect::new(260.0, 0.0, 700.0, 600.0);
        let short = Rect::new(260.0, 0.0, 600.0, 600.0);
        let wide = place(whole, &offered(false));
        let narrow = place(short, &offered(false));

        assert_eq!(wide.len(), narrow.len());
        for (one, two) in wide.iter().zip(narrow.iter()) {
            assert!(
                two.1.x < one.1.x,
                "a narrower strip left {:?} where it was",
                two.0
            );
        }
        let last = narrow.last().expect("a button");
        assert!(
            last.1.right() <= short.right(),
            "the last button is past the right edge of the strip it was given"
        );
    }

    /// The strip gives ground in one direction, a step at a time.
    ///
    /// Walked rather than spot-checked, and asserted as a ladder rather than
    /// against the constants: the field narrows, then becomes its mark, then
    /// the buttons fold away behind the ellipsis -- and none of it ever goes
    /// back the other way as the strip keeps shrinking. Written against the
    /// order rather than the widths, so moving `FIND_MIN` or adding a button
    /// needs this rerun rather than rewritten.
    #[test]
    fn the_strip_gives_ground_in_order() {
        let offers = offered(false);
        let mut field = f32::MAX;
        let mut lost_the_field = false;
        let mut folded = 0usize;

        let mut width = 900.0_f32;
        while width >= 120.0 {
            let strip = fit(Rect::new(260.0, 0.0, width, 600.0), &offers);

            if let Some(rect) = strip.field {
                assert!(
                    !lost_the_field,
                    "at {width} the field came back after becoming a mark"
                );
                assert!(
                    rect.width <= field + 0.01,
                    "at {width} the field grew as the strip shrank"
                );
                assert!(rect.width >= FIND_MIN, "at {width} the field is unusable");
                assert!(
                    !strip.shown.iter().any(|(act, _)| *act == Act::Search),
                    "at {width} there is a field and a search mark"
                );
                field = rect.width;
            } else {
                lost_the_field = true;
                assert!(
                    strip.shown.iter().any(|(act, _)| *act == Act::Search)
                        || strip.folded.contains(&Act::Search),
                    "at {width} the search went missing rather than becoming a mark"
                );
            }

            assert!(
                strip.folded.len() >= folded,
                "at {width} something came back out of the ellipsis"
            );
            folded = strip.folded.len();
            assert_eq!(
                strip.folded.is_empty(),
                !strip.shown.iter().any(|(act, _)| *act == Act::More),
                "at {width} the ellipsis and what is behind it disagree"
            );
            width -= 5.0;
        }

        assert!(lost_the_field, "the field never gave way to its mark");
        assert!(folded > 0, "nothing ever folded away");
    }

    /// Nothing drawn on the strip runs past it or into its neighbour.
    ///
    /// The field sits left of the buttons and neither may reach the other.
    /// What the strip did before was drop the field outright the moment it
    /// was under 240, and let the buttons run off the edge.
    /// A name too long for the strip stops before the controls.
    ///
    /// It used to be drawn at its full width and left to the scene's clip,
    /// which is the whole strip -- so on a narrowed column a group
    /// conversation, named after the people in it, ran straight under the
    /// search field and every button and out to the far edge. Nothing on the
    /// strip could be read, and nothing on it could be found to press.
    #[test]
    fn a_long_name_stops_before_the_controls() {
        let mut fonts = matterless_layout::Fonts::new();
        // A column with a thread pane beside it, which is when this bites.
        let within = Rect::new(310.0, 0.0, 320.0, 44.0);
        let mut header =
            Header::new("alexandre.boulet, florine.fouquart, leo-paul.couturier, and several more");
        header.offered = offered(true);

        let (from, room) = header.name_box(&mut fonts, within);
        let shown =
            matterless_layout::elided(&mut fonts, &header.title, room, Header::name_style());
        let wide =
            matterless_layout::extent_of(&mut fonts, &shown, f32::MAX, Header::name_style()).width;

        assert!(room > 0.0, "the name was given no room at all");
        assert!(
            shown != header.title,
            "a name far too long for the strip was not cut"
        );
        let laid = fit(within, &header.offered);
        let until = laid
            .field
            .map(|field| field.x)
            .or_else(|| laid.shown.first().map(|(_, rect)| rect.x))
            .expect("something on the right of the strip");
        assert!(
            from + wide <= until + 0.5,
            "the name reaches {} and the first control starts at {until}",
            from + wide
        );
    }

    #[test]
    fn nothing_on_the_strip_overlaps_or_overruns() {
        let offers = offered(false);
        let mut width = 900.0_f32;
        while width >= 120.0 {
            let within = Rect::new(260.0, 0.0, width, 600.0);
            let laid = fit(within, &offers);
            let edge = strip(within);
            for pair in laid.shown.windows(2) {
                assert!(
                    pair[0].1.right() <= pair[1].1.x,
                    "two buttons overlap at {width}"
                );
            }
            if let Some((_, last)) = laid.shown.last() {
                assert!(last.right() <= edge.right(), "a button runs past {width}");
            }
            if let (Some(field), Some((_, first))) = (laid.field, laid.shown.first()) {
                assert!(
                    field.right() <= first.x,
                    "the field reaches the buttons at {width}"
                );
            }
            width -= 5.0;
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
