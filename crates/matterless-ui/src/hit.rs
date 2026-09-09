//! Which box the pointer is over.
//!
//! The DOM answered this for free and it is the first thing a hand-built UI
//! has to replace: hover, click, drag and the cursor shape all start here.

use crate::Placed;

/// The innermost box holding the point, or nothing.
///
/// Innermost, not first: a click lands on the button, not on the panel the
/// button sits in. `solve` emits children after their parents, so the last
/// match is the deepest -- and taking the last is cheaper and less brittle than
/// comparing depths.
pub fn at(placed: &[Placed], x: f32, y: f32) -> Option<&Placed> {
    placed.iter().rev().find(|item| item.rect.holds(x, y))
}

/// The innermost box whose name the caller accepts.
///
/// A pointer is usually over several things at once -- a row, inside a list,
/// inside a panel -- and which of them matters depends on the question being
/// asked. Scrolling wants the list; a click wants the row.
pub fn at_matching(
    placed: &[Placed],
    x: f32,
    y: f32,
    wanted: impl Fn(&str) -> bool,
) -> Option<&Placed> {
    placed
        .iter()
        .rev()
        .find(|item| item.rect.holds(x, y) && wanted(&item.name))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::solve::solve;
    use crate::{Axis, Node, Rect, Size};

    fn shell() -> Vec<Placed> {
        let tree = Node::new("shell", Size::Grow(1.0))
            .axis(Axis::Row)
            .with(
                Node::new("sidebar", Size::Fixed(260.0))
                    .padding(8.0)
                    .gap(4.0)
                    .with(Node::new("sidebar/threads", Size::Fixed(30.0)))
                    .with(Node::new("sidebar/channels", Size::Grow(1.0))),
            )
            .with(
                Node::new("main", Size::Grow(1.0))
                    .with(Node::new("main/header", Size::Fixed(44.0)))
                    .with(Node::new("main/stream", Size::Grow(1.0)))
                    .with(Node::new("main/composer", Size::Fixed(60.0))),
            );
        solve(&tree, Rect::new(0.0, 0.0, 1280.0, 800.0))
    }

    #[test]
    fn the_innermost_box_wins() {
        let placed = shell();
        assert_eq!(at(&placed, 100.0, 20.0).unwrap().name, "sidebar/threads");
        assert_eq!(at(&placed, 100.0, 400.0).unwrap().name, "sidebar/channels");
        assert_eq!(at(&placed, 700.0, 20.0).unwrap().name, "main/header");
        assert_eq!(at(&placed, 700.0, 770.0).unwrap().name, "main/composer");
    }

    #[test]
    fn a_caller_can_ask_for_the_container_instead() {
        let placed = shell();
        // Over a channel row, but the question is which panel scrolls.
        let found = at_matching(&placed, 100.0, 400.0, |name| !name.contains('/')).unwrap();
        assert_eq!(found.name, "sidebar");
    }

    #[test]
    fn outside_everything_is_nothing() {
        let placed = shell();
        assert!(at(&placed, -1.0, 10.0).is_none());
        assert!(at(&placed, 10.0, 900.0).is_none());
    }

    /// The right and bottom edges belong to the next box, not this one, or a
    /// pointer between two rows would land on both.
    #[test]
    fn the_far_edge_belongs_to_the_neighbour() {
        let placed = shell();
        assert_eq!(at(&placed, 259.0, 400.0).unwrap().name, "sidebar");
        assert_eq!(at(&placed, 260.0, 400.0).unwrap().name, "main/stream");
    }

    /// A parent's padding is the parent's, not its children's.
    ///
    /// Worth its own test because it is the difference between a click landing
    /// on a channel and landing beside one, and the first version of the test
    /// above assumed the opposite.
    #[test]
    fn padding_belongs_to_the_box_that_declared_it() {
        let placed = shell();
        // The sidebar pads by 8, so its children start at 8 and end at 252.
        assert_eq!(at(&placed, 4.0, 400.0).unwrap().name, "sidebar");
        assert_eq!(at(&placed, 8.0, 400.0).unwrap().name, "sidebar/channels");
        assert_eq!(at(&placed, 251.0, 400.0).unwrap().name, "sidebar/channels");
        assert_eq!(at(&placed, 252.0, 400.0).unwrap().name, "sidebar");
    }
}
