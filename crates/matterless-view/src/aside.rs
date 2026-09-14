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

/// `grid-template-columns: 260px minmax(0, 1fr) 320px`.
///
/// Narrower than the thread pane deliberately: this holds three-line excerpts
/// rather than a conversation.
pub const WIDTH: f32 = 320.0;
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

/// Where the column sits, given everything right of the sidebar.
///
/// Never more than half: on a narrow window a fixed pane would leave the
/// conversation it is read against too thin to read, which is the same rule
/// the thread pane follows.
pub fn rect(column: Rect) -> Rect {
    let width = WIDTH.min(column.width * 0.5);
    Rect::new(column.right() - width, column.y, width, column.height)
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
    let Canvas {
        scene, palette, ..
    } = into;
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
        "\u{00d7}",
        at.x + 7.0,
        at.y + 2.0,
        Run::label(f32::MAX).sized(15.0),
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
    fn the_column_is_on_the_right_and_fixed_width() {
        let column = Rect::new(300.0, 0.0, 1000.0, 700.0);
        let pane = rect(column);
        assert_eq!(pane.width, WIDTH);
        assert_eq!(pane.right(), column.right());
        assert_eq!(pane.height, column.height);
    }

    /// On a narrow window it gives way rather than squeezing the conversation
    /// out, the same way the thread pane does.
    #[test]
    fn a_narrow_window_gets_a_narrower_column() {
        let column = Rect::new(0.0, 0.0, 400.0, 700.0);
        assert_eq!(rect(column).width, 200.0);
    }

    /// The header and the body divide the pane between them with nothing over
    /// the rule, which would draw on top of the conversation's edge.
    #[test]
    fn the_header_and_the_body_fill_the_pane() {
        let pane = rect(Rect::new(300.0, 0.0, 1000.0, 700.0));
        assert_eq!(header(pane).bottom(), body(pane).y);
        assert_eq!(body(pane).bottom(), pane.bottom());
        assert!(header(pane).x >= pane.x + RULE);
        assert!(close(pane).right() <= pane.right());
    }
}
