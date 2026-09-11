//! What a button is for, said after the pointer has waited on it.
//!
//! Nearly every control in the app carries a `title`, and the browser turns
//! that into a tooltip for nothing. A native window has no such gift: the port
//! inherited a strip of bare emoji -- a pin, a bookmark, a bell, a door -- with
//! no way to find out what any of them did short of pressing it.
//!
//! Timed rather than immediate, the way a `title` is. A label that appears the
//! instant the pointer crosses a button follows the hand around the window and
//! is read as an error; one that waits is only there for somebody who stopped
//! to ask.

use crate::sidebar::Canvas;
use matterless_layout::{Style, extent_of};
use matterless_paint::Run;
use matterless_ui::Rect;
use std::time::{Duration, Instant};

/// How long the pointer has to rest before anything is said.
///
/// A little over the platform's own, because the window is being read next to
/// the one it is a copy of and a tooltip that fires while the eye is still
/// travelling is noise.
pub const DWELL: Duration = Duration::from_millis(450);

/// How small it is, and how far it clears the pointer.
///
/// Below and to the right, where a cursor is not: a tooltip drawn under the
/// hotspot is a tooltip covered by the arrow pointing at it.
const SIZE: f32 = 11.5;
const LINE: f32 = 15.0;
const PAD_X: f32 = 7.0;
const PAD_Y: f32 = 4.0;
const CORNER: f32 = 4.0;
const WIDEST: f32 = 320.0;
const FROM_POINTER: (f32, f32) = (12.0, 20.0);

/// What the pointer is resting on, and what it would say.
#[derive(Debug, Default)]
pub struct Tooltip {
    /// The box the pointer is on, and when it arrived. Cleared the moment it
    /// moves to another, so crossing a row of buttons says nothing at all.
    over: Option<(String, Instant)>,
    /// Where the pointer was when it arrived.
    ///
    /// Anchored there rather than followed: a label that slides along under
    /// the hand while it settles on a button is harder to read than one that
    /// simply appears, and a wide box -- a link across half a line -- would
    /// drag it a long way.
    at: (f32, f32),
    says: String,
    /// Whether the window has already been told this one is due.
    ///
    /// The wait ends with nothing happening -- no click, no key, no message --
    /// so somebody has to ask for the frame that draws it, once.
    told: bool,
}

impl Tooltip {
    /// Follows the pointer for one frame.
    ///
    /// `explains` is asked only when the box has changed, because it is the
    /// expensive half -- a name has to be resolved to a person, a reaction to
    /// the people who left it -- and the answer does not change while the
    /// pointer sits still.
    pub fn follows(
        &mut self,
        hovered: Option<&str>,
        at: Option<(f32, f32)>,
        explains: impl FnOnce(&str) -> Option<String>,
    ) {
        match (hovered, at) {
            (Some(name), Some(_)) if self.over.as_ref().is_some_and(|(on, _)| on == name) => {}
            (Some(name), Some(at)) => {
                self.says = explains(name).unwrap_or_default();
                self.over = Some((name.to_string(), Instant::now()));
                self.at = at;
                self.told = false;
            }
            _ => {
                self.over = None;
                self.says.clear();
                self.told = false;
            }
        }
    }

    /// What is being said right now, once the wait is over.
    pub fn shown(&self) -> Option<&str> {
        if self.says.is_empty() {
            return None;
        }
        let (_, since) = self.over.as_ref()?;
        (since.elapsed() >= DWELL).then_some(self.says.as_str())
    }

    /// True the once, when the wait ends.
    ///
    /// The window is idle by then: the pointer stopped moving, which is the
    /// whole condition. Without this the tooltip would sit ready and unseen
    /// until something unrelated happened to redraw the window.
    pub fn ripened(&mut self) -> bool {
        let ripe = !self.told && self.shown().is_some();
        if ripe {
            self.told = true;
        }
        ripe
    }

    /// When the window has to draw again for this to appear.
    ///
    /// `None` once it is showing or when there is nothing to show: a window
    /// that keeps asking for the next frame while a tooltip is merely open
    /// redraws forever.
    pub fn wakes(&self) -> Option<Instant> {
        if self.says.is_empty() {
            return None;
        }
        let (_, since) = self.over.as_ref()?;
        let due = *since + DWELL;
        (due > Instant::now()).then_some(due)
    }

    /// Where it sits, and how tall the words make it.
    pub fn rect(&self, fonts: &mut matterless_layout::Fonts, within: Rect) -> Rect {
        let at = self.at;
        let extent = extent_of(
            fonts,
            &self.says,
            WIDEST,
            Style {
                size: SIZE,
                line_height: LINE,
                bold: false,
                italic: false,
                mono: false,
            },
        );
        let width = extent.width + PAD_X * 2.0;
        let height = extent.lines as f32 * LINE + PAD_Y * 2.0;
        // Pushed back onto the window rather than flipped, and flipped above
        // the pointer only when there is no room below it at all -- which is
        // the one case where pushing would put it under the cursor.
        let below = at.1 + FROM_POINTER.1;
        Rect::new(
            (at.0 + FROM_POINTER.0)
                .min(within.right() - width - 4.0)
                .max(within.x + 4.0),
            if below + height <= within.bottom() {
                below
            } else {
                (at.1 - 8.0 - height).max(within.y + 4.0)
            },
            width,
            height,
        )
    }

    pub fn draw(&self, into: &mut Canvas<'_>, within: Rect) {
        if self.shown().is_none() {
            return;
        }
        let rect = self.rect(into.fonts, within);
        // A hairline round it, drawn as the ground inset by one: without an
        // edge a tooltip over a panel of the same colour is a floating
        // sentence.
        into.scene.floating(
            rect.x,
            rect.y,
            rect.width,
            rect.height,
            into.palette.rule,
            CORNER,
            4.0,
        );
        into.scene.rounded(
            rect.x + 1.0,
            rect.y + 1.0,
            rect.width - 2.0,
            rect.height - 2.0,
            into.palette.raised,
            CORNER - 1.0,
        );
        let glyphs = into.painter.run(
            into.fonts,
            &self.says,
            rect.x + PAD_X,
            rect.y + PAD_Y,
            Run::label(WIDEST).sized(SIZE),
        );
        into.scene.glyphs(glyphs, into.palette.ink, into.palette.faint);
    }
}

/// Who left a reaction, as the app words it.
///
/// The server hands back only as many names as it was asked for, so the rest
/// are counted: "six others" is the honest answer where a list would be a lie
/// about what is known.
pub fn reacted_by(emoji: &str, count: usize, names: &[String]) -> String {
    /// How many names go on one line before it wraps.
    const PER_LINE: usize = 6;

    let mut names: Vec<String> = names.to_vec();
    let unnamed = count.saturating_sub(names.len());
    if unnamed > 0 {
        names.push(format!(
            "{unnamed} {}",
            if unnamed == 1 { "other" } else { "others" }
        ));
    }
    let Some(last) = names.pop() else {
        return format!(":{emoji}:");
    };
    let people = if names.is_empty() {
        last
    } else {
        let lines: Vec<String> = names
            .chunks(PER_LINE)
            .map(|chunk| chunk.join(", "))
            .collect();
        format!("{} and {last}", lines.join(",\n"))
    };
    format!("{people} reacted with :{emoji}:")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(of: &[&str]) -> Vec<String> {
        of.iter().map(|name| name.to_string()).collect()
    }

    /// The wait ends with nothing else happening, so the tooltip has to ask
    /// for the frame that draws it -- once, or the window redraws forever.
    #[test]
    fn it_asks_for_the_frame_that_draws_it() {
        let mut tooltip = Tooltip::default();
        tooltip.follows(Some("header/saved"), Some((10.0, 10.0)), |_| {
            Some("Saved messages".into())
        });
        assert!(!tooltip.ripened(), "not until the wait is over");

        tooltip.over = Some(("header/saved".into(), Instant::now() - DWELL));
        assert!(tooltip.ripened(), "and then exactly once");
        assert!(!tooltip.ripened());
        assert!(!tooltip.ripened());

        // The next button starts again, and asks again.
        tooltip.follows(Some("header/pinned"), Some((30.0, 10.0)), |_| {
            Some("Pinned messages".into())
        });
        tooltip.over = Some(("header/pinned".into(), Instant::now() - DWELL));
        assert!(tooltip.ripened());
    }

    /// Nothing is said until the pointer has stopped: a label that follows the
    /// hand across a row of buttons is noise rather than an explanation.
    #[test]
    fn it_says_nothing_until_the_pointer_rests() {
        let mut tooltip = Tooltip::default();
        tooltip.follows(Some("header/saved"), Some((10.0, 10.0)), |_| {
            Some("Saved messages".into())
        });
        assert_eq!(tooltip.shown(), None, "not yet");
        assert!(tooltip.wakes().is_some(), "but a frame is owed");

        // Wound back, which is what waiting would have done.
        tooltip.over = Some(("header/saved".into(), Instant::now() - DWELL));
        assert_eq!(tooltip.shown(), Some("Saved messages"));
        // And once it is up, nothing further is owed: asking for a frame while
        // a tooltip is merely open would redraw the window forever.
        assert_eq!(tooltip.wakes(), None);
    }

    /// Moving to the next button starts the wait again, rather than carrying
    /// the last one's answer across.
    #[test]
    fn each_button_is_asked_about_on_its_own() {
        let mut tooltip = Tooltip::default();
        tooltip.follows(Some("header/saved"), Some((10.0, 10.0)), |_| {
            Some("Saved messages".into())
        });
        tooltip.over = Some(("header/saved".into(), Instant::now() - DWELL));
        assert_eq!(tooltip.shown(), Some("Saved messages"));

        tooltip.follows(Some("header/pinned"), Some((20.0, 10.0)), |_| {
            Some("Pinned messages".into())
        });
        assert_eq!(tooltip.shown(), None, "the wait starts again");

        tooltip.follows(None, None, |_| None);
        assert_eq!(tooltip.shown(), None);
    }

    /// A box with nothing to say costs no wake-up: most of the window is not a
    /// button, and a frame owed for every one of them is a frame a second.
    #[test]
    fn a_box_with_nothing_to_say_asks_for_no_frame() {
        let mut tooltip = Tooltip::default();
        tooltip.follows(Some("stream"), Some((10.0, 10.0)), |_| None);
        assert_eq!(tooltip.wakes(), None);
        assert_eq!(tooltip.shown(), None);
    }

    /// It stays on the window wherever the pointer is.
    #[test]
    fn it_stays_on_the_window() {
        let mut fonts = matterless_layout::Fonts::new();
        let window = Rect::new(0.0, 0.0, 900.0, 600.0);
        let mut tooltip = Tooltip::default();
        for at in [(880.0, 590.0), (2.0, 2.0), (450.0, 598.0)] {
            tooltip.follows(Some("header/leave"), Some(at), |_| {
                Some("Leave Channel".into())
            });
            // Each corner is a fresh arrival, or the second would keep the
            // first one's anchor.
            tooltip.over = Some(("header/leave".into(), Instant::now()));
            let rect = tooltip.rect(&mut fonts, window);
            assert!(rect.x >= window.x, "{at:?} ran off the left");
            assert!(rect.right() <= window.right(), "{at:?} ran off the right");
            assert!(rect.y >= window.y, "{at:?} ran off the top");
            assert!(
                rect.bottom() <= window.bottom(),
                "{at:?} ran off the bottom"
            );
            // Never under the cursor, which is the one place it cannot be.
            assert!(
                rect.y > at.1 || rect.bottom() < at.1 || rect.x > at.0,
                "{at:?} is under the pointer"
            );
        }
    }

    /// Who reacted, named while the names are known and counted once they run
    /// out -- because the server only ever returns the first few.
    #[test]
    fn a_reaction_says_who_left_it() {
        assert_eq!(
            reacted_by("tada", 1, &names(&["ana"])),
            "ana reacted with :tada:"
        );
        assert_eq!(
            reacted_by("tada", 2, &names(&["ana", "bo"])),
            "ana and bo reacted with :tada:"
        );
        // More people than names: the rest are counted, and counted in the
        // plural only when there is more than one of them.
        assert_eq!(
            reacted_by("tada", 3, &names(&["ana", "bo"])),
            "ana, bo and 1 other reacted with :tada:"
        );
        assert_eq!(
            reacted_by("tada", 5, &names(&["ana", "bo"])),
            "ana, bo and 3 others reacted with :tada:"
        );
        // No names at all is what an emoji this window has never resolved
        // looks like, and the emoji alone is a better answer than a blank.
        assert_eq!(reacted_by("tada", 0, &[]), ":tada:");
    }

    /// Seven names wrap, so a popular reaction is not one long line.
    #[test]
    fn a_long_list_of_names_wraps() {
        let said = reacted_by(
            "tada",
            8,
            &names(&["a", "b", "c", "d", "e", "f", "g", "h"]),
        );
        assert!(said.contains('\n'), "{said}");
        assert!(said.ends_with("and h reacted with :tada:"), "{said}");
    }
}
