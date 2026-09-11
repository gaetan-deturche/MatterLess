//! Where the reader is in a conversation, and a handle to move it.
//!
//! Its own, rather than anything derived from the scroll position. A thumb
//! computed from where the list *is* moves out from under the hand whenever
//! something else writes that position -- a page of older history landing, a
//! row re-measuring after a picture arrives -- and both of those happen while
//! somebody is dragging.
//!
//! So this runs the other way round: while the thumb is held it is drawn where
//! the pointer put it and the list is scrolled to match. The hand is the
//! input; the scroll position is the output.

use crate::sidebar::Canvas;
use matterless_ui::input::Input;
use matterless_ui::{Placed, Rect};

/// The strip a press is caught in, wider than anything drawn in it so the bar
/// is easy to hit: `.track { width: 12px }`.
const TRACK: f32 = 12.0;
/// The groove behind the thumb, and the thumb itself, with the gap each keeps
/// from the right edge. Straight from the stylesheet: a 4px rail at `right:
/// 4px` and an 8px thumb at `right: 2px`.
const GROOVE: f32 = 4.0;
const GROOVE_INSET: f32 = 4.0;
const THUMB: f32 = 8.0;
const THUMB_INSET: f32 = 2.0;
/// What the ends are kept clear of, so the bar does not run into a corner.
const EDGE: f32 = 4.0;

/// How solid each part is. The bar is furniture: present enough to find,
/// quiet enough to ignore, and louder under the hand that is using it.
const GROOVE_ALPHA: f32 = 0.14;
const THUMB_ALPHA: f32 = 0.42;
const HELD_ALPHA: f32 = 0.75;
/// And louder again with the pointer anywhere on the track, before anything is
/// held: `.track:hover .rail`, which is how a reader finds the bar at all.
const GROOVE_HOVER_ALPHA: f32 = 0.24;

/// A thumb shorter than this is not worth aiming at.
///
/// Unbounded history makes the proportional height tend to nothing, and having
/// a floor is exactly why the mapping is written out rather than left to a
/// proportion: once the thumb stops being proportional, so does everything
/// derived from it.
const LEAST: f32 = 28.0;

/// The bar for one panel.
#[derive(Debug, Default)]
pub struct Scrollbar {
    /// Where inside the thumb it was grabbed, while it is held. `None` when
    /// nobody is dragging.
    held: Option<f32>,
}

impl Scrollbar {
    /// The strip a press is caught in, which is wider than the thumb drawn in
    /// it: a four-pixel target is a target nobody hits.
    pub fn track(&self, within: Rect) -> Rect {
        Rect::new(
            within.right() - TRACK,
            within.y + EDGE,
            TRACK,
            (within.height - EDGE * 2.0).max(0.0),
        )
    }

    /// How far the list can travel, and how far the thumb can.
    fn travel(&self, within: Rect, reach: f32) -> (f32, f32) {
        let track = self.track(within);
        let thumb = self.height(within, reach);
        ((track.height - thumb).max(0.0), reach.max(0.0))
    }

    fn height(&self, within: Rect, reach: f32) -> f32 {
        let track = self.track(within);
        if reach <= 0.0 {
            return track.height;
        }
        // What fraction of the whole is on screen, floored so it stays worth
        // aiming at however much history is behind it.
        let whole = within.height + reach;
        let shown = (within.height / whole) * track.height;
        shown.clamp(LEAST.min(track.height), track.height)
    }

    /// Where the thumb sits for a given scroll.
    ///
    /// Narrower than the track it travels in, and to the right of it: the
    /// track is what catches a press, the thumb is what says where you are.
    pub fn thumb(&self, within: Rect, scroll: f32, reach: f32) -> Rect {
        let track = self.track(within);
        let height = self.height(within, reach);
        let (room, span) = self.travel(within, reach);
        let along = if span > 0.0 {
            (scroll / span).clamp(0.0, 1.0) * room
        } else {
            0.0
        };
        Rect::new(
            within.right() - THUMB - THUMB_INSET,
            track.y + along,
            THUMB,
            height,
        )
    }

    /// Nothing to show when everything fits: a full-length thumb says only
    /// that there is nothing to scroll, which the absence of a bar says too.
    pub fn needed(&self, reach: f32) -> bool {
        reach > 0.5
    }

    pub fn boxes(&self, name: &str, within: Rect, reach: f32) -> Vec<Placed> {
        if !self.needed(reach) {
            return Vec::new();
        }
        vec![Placed {
            // Under the stream's own name, so the wheel over the bar scrolls
            // the list it belongs to rather than doing nothing.
            name: format!("{name}/scrollbar"),
            rect: self.track(within),
            depth: 4,
        }]
    }

    /// Applies a frame's input. Answers a new scroll while the thumb is held.
    ///
    /// The grab offset is taken once, when the button goes down, so the thumb
    /// keeps the same point under the cursor for the whole drag rather than
    /// jumping to centre itself on the first move.
    pub fn react(
        &mut self,
        name: &str,
        input: &Input,
        within: Rect,
        scroll: f32,
        reach: f32,
    ) -> Option<f32> {
        let mine = format!("{name}/scrollbar");
        if input.pressed() != Some(mine.as_str()) {
            self.held = None;
            return None;
        }
        let (x, y) = input.pointer_at()?;
        let _ = x;
        let track = self.track(within);
        let thumb = self.thumb(within, scroll, reach);
        if input.pressed_now() == Some(mine.as_str()) {
            // Pressing the track rather than the thumb takes the reader there,
            // with the thumb centred under the cursor.
            self.held = Some(if y >= thumb.y && y <= thumb.bottom() {
                y - thumb.y
            } else {
                thumb.height / 2.0
            });
        }
        let grabbed = self.held?;
        let (room, span) = self.travel(within, reach);
        if room <= 0.0 {
            return None;
        }
        let top = (y - grabbed - track.y).clamp(0.0, room);
        Some(top / room * span)
    }

    pub fn draw(
        &self,
        into: &mut Canvas<'_>,
        name: &str,
        input: &Input,
        within: Rect,
        scroll: f32,
        reach: f32,
    ) {
        if !self.needed(reach) {
            return;
        }
        // The whole track, not the thumb: the bar has to brighten before the
        // pointer has found the four pixels it is aiming at.
        let under = input.hovered() == Some(format!("{name}/scrollbar").as_str());
        let track = self.track(within);
        let thumb = self.thumb(within, scroll, reach);
        let Canvas { scene, palette, .. } = into;
        let quiet = |alpha: f32| {
            [
                palette.faint[0],
                palette.faint[1],
                palette.faint[2],
                (alpha * 255.0) as u8,
            ]
        };
        // The groove first. It is what says where the ends are, which is the
        // reference a thumb needs to mean "near the top" rather than just
        // "somewhere".
        scene.rounded(
            within.right() - GROOVE - GROOVE_INSET,
            track.y,
            GROOVE,
            track.height,
            quiet(if under || self.held.is_some() {
                GROOVE_HOVER_ALPHA
            } else {
                GROOVE_ALPHA
            }),
            GROOVE / 2.0,
        );
        scene.rounded(
            thumb.x,
            thumb.y,
            thumb.width,
            thumb.height,
            quiet(if under || self.held.is_some() {
                HELD_ALPHA
            } else {
                THUMB_ALPHA
            }),
            thumb.width / 2.0,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn panel() -> Rect {
        Rect::new(260.0, 44.0, 700.0, 400.0)
    }

    /// The two ends map to the two ends. Anything else and dragging to the
    /// bottom stops short of the newest message.
    #[test]
    fn the_thumb_reaches_both_ends() {
        let bar = Scrollbar::default();
        let track = bar.track(panel());
        let top = bar.thumb(panel(), 0.0, 4000.0);
        let bottom = bar.thumb(panel(), 4000.0, 4000.0);
        assert_eq!(top.y, track.y);
        assert!((bottom.bottom() - track.bottom()).abs() < 0.5);
    }

    /// Unbounded history makes a proportional thumb tend to nothing, so it has
    /// a floor -- and having one is why the mapping is written out.
    #[test]
    fn a_long_history_still_leaves_something_to_aim_at() {
        let bar = Scrollbar::default();
        // Four hundred thousand pixels of conversation behind a 400px panel.
        let thumb = bar.thumb(panel(), 0.0, 400_000.0);
        assert!(
            thumb.height >= LEAST,
            "{} is too small to grab",
            thumb.height
        );
        // And it still reaches the end despite not being proportional.
        let bottom = bar.thumb(panel(), 400_000.0, 400_000.0);
        assert!((bottom.bottom() - bar.track(panel()).bottom()).abs() < 0.5);
    }

    /// Nothing to scroll, nothing to draw: a full-length thumb says only what
    /// the absence of a bar already says.
    #[test]
    fn a_conversation_that_fits_has_no_bar() {
        let bar = Scrollbar::default();
        assert!(!bar.needed(0.0));
        assert!(bar.boxes("stream", panel(), 0.0).is_empty());
        assert!(bar.needed(1.0));
    }

    /// The bar belongs to its panel: two of them must not answer to the same
    /// name, or a drag in the thread pane scrolls the channel.
    #[test]
    fn each_panel_has_its_own_bar() {
        let bar = Scrollbar::default();
        let channel = bar.boxes("stream", panel(), 900.0);
        let thread = bar.boxes("thread/root", panel(), 900.0);
        assert_ne!(channel[0].name, thread[0].name);
        assert!(channel[0].name.starts_with("stream"));
    }
}
