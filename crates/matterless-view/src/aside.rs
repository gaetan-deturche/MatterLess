//! The column on the right that search, saved, pinned and threads share.
//!
//! A column and not a floating panel, because that is what the app is: `.pane
//! { border-left: 1px solid var(--rule) }` in a grid whose third track is
//! `320px` while one of them is open. A dialog over the conversation hides the
//! thing the reader is looking for a message *in*, and every one of these
//! lists is read against the conversation beside it -- a search result is a
//! line out of context until you can see where it came from.
//!
//! Shared rather than written three times for the same reason the app shares
//! `Hits`: they are one shape -- a title, a list of messages, and a way out --
//! and three copies would be three places for the rule down the left edge to
//! drift.

use crate::sidebar::Canvas;
use matterless_paint::Run;
use matterless_ui::{Placed, Rect};

/// What a pane starts at, before the reader has moved it.
///
/// One number for this pane and for the thread's. They were 320 and 420, and
/// the note here said this one was narrower "deliberately: this holds
/// three-line excerpts rather than a conversation" -- which is a reason for a
/// default and not for two edges that never line up. The reader is the one
/// who knows how much of the window they want spent on a side pane, so it is
/// one edge now and they can drag it.
pub const WIDTH: f32 = 420.0;
/// The narrowest a pane can be dragged.
///
/// A result is a name, a time and two lines of what was said. Below this the
/// lines are cut to nothing and the pane is a column of names.
pub const NARROWEST: f32 = 280.0;
/// The grip on a pane's left edge, and how far either side of it answers.
///
/// Wider than the rule it sits on: a one-pixel target is one nobody can hit,
/// and the edge is the only part of a pane that does something when dragged.
pub const GRIP: f32 = 6.0;
/// What the conversation keeps beside an open pane.
///
/// A number of pixels, like every other bound here. Roughly what this pane
/// itself needs plus the gutter the avatars sit in -- a conversation is the
/// same shape as a list of results with pictures down the left.
pub const BESIDE: f32 = 320.0;
/// `border-left: 1px solid var(--rule)`.
pub const RULE: f32 = 1.0;
/// `header { padding: 8px 10px }` around a field that is itself 30px.
pub const HEADER: f32 = 46.0;
/// `padding: 8px 12px` on what the header sits above.
pub const PADDING: f32 = 10.0;
/// One result: two lines, the name above the words.
pub const ROW: f32 = 52.0;
/// The way out, in the top right corner of the header.
pub const CLOSE: f32 = 24.0;

/// Where the column sits, given everything right of the sidebar and how wide
/// the reader has made it.
///
/// Bounded in pixels at both ends, and at neither end by a share of the
/// window. A pane is a column of text and what it needs is a number of
/// characters, which does not change because the screen did -- capped at half
/// the column, the same drag stopped somewhere different on every window, and
/// on a small one the pane could not reach even its own default.
///
/// So the conversation's claim is a number too: `BESIDE`, and the pane may
/// have everything else. Where the column cannot hold both, the pane keeps
/// `NARROWEST` and the conversation gives way -- the reader opened the pane
/// in order to read it, and a sliver is no use to anybody.
///
/// Narrowing the window squeezes the pane and widening it gives back the one
/// that was dragged, because nothing here is written down: this is an answer
/// about one frame, and the width the reader asked for is kept by the window.
pub fn rect(column: Rect, width: f32) -> Rect {
    let room = column.width.max(0.0);
    let cap = (room - BESIDE).max(NARROWEST.min(room));
    let width = width.clamp(NARROWEST.min(cap), cap);
    Rect::new(column.right() - width, column.y, width, column.height)
}

/// The strip down a pane's left edge that resizes it.
pub fn grip(pane: Rect) -> Rect {
    Rect::new(pane.x - GRIP / 2.0, pane.y, GRIP, pane.height)
}

/// The strip along the top, which holds the title or the field.
pub fn header(pane: Rect) -> Rect {
    Rect::new(pane.x + RULE, pane.y, pane.width - RULE, HEADER)
}

/// Everything under the header: the list itself.
pub fn body(pane: Rect) -> Rect {
    Rect::new(
        pane.x + RULE,
        pane.y + HEADER,
        pane.width - RULE,
        (pane.height - HEADER).max(0.0),
    )
}

/// Where the close button sits.
pub fn close(pane: Rect) -> Rect {
    let head = header(pane);
    Rect::new(
        head.right() - PADDING - CLOSE,
        head.y + (head.height - CLOSE) / 2.0,
        CLOSE,
        CLOSE,
    )
}

/// The ground, and the rule down its left edge.
pub fn ground(into: &mut Canvas<'_>, pane: Rect) {
    let Canvas { scene, palette, .. } = into;
    scene.fill(pane.x, pane.y, pane.width, pane.height, palette.surface);
    scene.fill(pane.x, pane.y, RULE, pane.height, palette.rule);
}

/// The way out, drawn.
pub fn draw_close(into: &mut Canvas<'_>, pane: Rect) {
    let at = close(pane);
    let Canvas {
        scene,
        painter,
        fonts,
        palette,
    } = into;
    let glyphs = painter.run(
        fonts,
        matterless_layout::marks::CLOSE,
        at.x + 7.0,
        at.y + 2.0,
        Run::mark(13.0),
    );
    scene.glyphs(glyphs, palette.soft, palette.faint);
}

/// The close button's own hit box.
pub fn close_box(pane: Rect, name: &str) -> Placed {
    Placed {
        name: format!("{name}/close"),
        rect: close(pane),
        depth: 9,
    }
}

/// How many whole rows fit in the body, which is what a list may show.
pub fn rows_in(pane: Rect) -> usize {
    ((body(pane).height - PADDING) / ROW).max(0.0) as usize
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The column takes the right edge, and leaves the rest alone.
    #[test]
    fn the_column_is_on_the_right_and_as_wide_as_asked() {
        let column = Rect::new(300.0, 0.0, 1000.0, 700.0);
        let pane = rect(column, WIDTH);
        assert_eq!(pane.width, WIDTH);
        assert_eq!(pane.right(), column.right());
        assert_eq!(pane.height, column.height);

        // And a width the reader dragged to, between the two bounds.
        assert_eq!(rect(column, 360.0).width, 360.0);
        assert_eq!(rect(column, 360.0).right(), column.right());
    }

    /// The conversation's claim is the cap, and it is a number of pixels.
    ///
    /// Not a share of the window. Reported by dragging: capped at half the
    /// column the pane stopped at 347 on a 1002-wide window and could never
    /// reach even its own 420 default -- and the same drag ended somewhere
    /// different on every window.
    #[test]
    fn the_cap_is_what_the_conversation_keeps_not_a_share_of_the_window() {
        // The window it was reported on: 1002 across, less the rail and the
        // sidebar.
        let reported = Rect::new(308.0, 0.0, 694.0, 700.0);
        assert_eq!(rect(reported, WIDTH).width, 694.0 - BESIDE);
        assert_eq!(
            rect(reported, 10_000.0).width,
            694.0 - BESIDE,
            "the conversation gave up more than its own minimum"
        );

        // Twice the room is twice the pane, not the same fraction of it.
        let roomy = Rect::new(0.0, 0.0, 1388.0, 700.0);
        assert_eq!(rect(roomy, 10_000.0).width, 1388.0 - BESIDE);
    }

    /// On a window too small for both, the pane keeps its floor.
    ///
    /// The reader opened it in order to read it, and a sliver is no use to
    /// anybody -- so here it is the conversation that gives way.
    #[test]
    fn a_window_too_small_for_both_still_leaves_a_usable_pane() {
        let column = Rect::new(0.0, 0.0, 400.0, 700.0);
        assert_eq!(rect(column, WIDTH).width, NARROWEST);
        assert_eq!(rect(column, 10_000.0).width, NARROWEST);
        assert_eq!(rect(column, 0.0).width, NARROWEST);
    }

    /// And it cannot be dragged down to a strip of nothing.
    ///
    /// A result is a name, a time and two lines of what was said. Below
    /// `NARROWEST` the lines are cut to nothing and the pane is a column of
    /// names -- so the drag stops there rather than letting the reader make
    /// something useless and then wonder what the pane is for.
    #[test]
    fn a_pane_cannot_be_dragged_away_to_nothing() {
        let column = Rect::new(0.0, 0.0, 1000.0, 700.0);
        assert_eq!(rect(column, 0.0).width, NARROWEST);
        assert_eq!(rect(column, -500.0).width, NARROWEST);

        // And it never exceeds the column, however little of one there is.
        let sliver = Rect::new(0.0, 0.0, 120.0, 700.0);
        assert!(rect(sliver, WIDTH).width <= 120.0);
    }

    /// The width the reader asked for is kept, not the width that fitted.
    ///
    /// Narrowing the window squeezes the pane; widening it again gives back
    /// the pane they dragged. Kept by the window rather than clamped into,
    /// so this is the property that says `rect` stays a pure answer about
    /// one frame.
    #[test]
    fn a_squeezed_pane_comes_back_when_there_is_room() {
        let wide = Rect::new(0.0, 0.0, 1400.0, 700.0);
        let narrow = Rect::new(0.0, 0.0, 600.0, 700.0);
        let asked = 520.0;

        assert_eq!(rect(wide, asked).width, asked);
        assert_eq!(
            rect(narrow, asked).width,
            NARROWEST,
            "a column with 600 in it cannot give 520 away and keep 320"
        );
        assert_eq!(
            rect(wide, asked).width,
            asked,
            "and back again, because nothing was written down"
        );
    }

    /// The grip straddles the edge it moves, rather than sitting beside it.
    #[test]
    fn the_grip_sits_on_the_edge() {
        let pane = rect(Rect::new(0.0, 0.0, 1000.0, 700.0), WIDTH);
        let grip = grip(pane);
        assert!(grip.x < pane.x, "the grip is all inside the pane");
        assert!(grip.right() > pane.x, "the grip is all outside the pane");
        assert_eq!(grip.height, pane.height);
        assert!(grip.width >= 4.0, "a grip this thin is one nobody can hit");
    }

    /// The header and the body divide the pane between them with nothing over
    /// the rule, which would draw on top of the conversation's edge.
    #[test]
    fn the_header_and_the_body_fill_the_pane() {
        let pane = rect(Rect::new(300.0, 0.0, 1000.0, 700.0), WIDTH);
        assert_eq!(header(pane).bottom(), body(pane).y);
        assert_eq!(body(pane).bottom(), pane.bottom());
        assert!(header(pane).x >= pane.x + RULE);
        assert!(close(pane).right() <= pane.right());
    }
}
