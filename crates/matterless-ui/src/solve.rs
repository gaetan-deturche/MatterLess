//! Turning a tree of boxes into rectangles.
//!
//! One pass down. Fixed children take what they asked for; the rest share what
//! is left in proportion to their weight. Nothing here measures content, which
//! is why there is no second pass: a box that must fit its text asks the text
//! how tall it is and becomes `Fixed`, before it ever reaches this.

use crate::{Axis, Node, Placed, Rect, Size};

/// Solves the tree inside `within`, innermost boxes last.
///
/// The order is what hit testing depends on: the last rectangle holding a point
/// is the deepest one, which is the one the pointer is really over.
pub fn solve(root: &Node, within: Rect) -> Vec<Placed> {
    let mut placed = Vec::new();
    place(root, within, 0, &mut placed);
    placed
}

fn place(node: &Node, rect: Rect, depth: usize, into: &mut Vec<Placed>) {
    into.push(Placed {
        name: node.name.clone(),
        rect,
        depth,
    });
    if node.children.is_empty() {
        return;
    }

    let inner = rect.inset(node.padding);
    let gaps = node.gap * (node.children.len().saturating_sub(1)) as f32;
    let along = match node.axis {
        Axis::Row => inner.width,
        Axis::Column => inner.height,
    };
    let spare = (along - gaps).max(0.0);

    let taken: f32 = node
        .children
        .iter()
        .map(|child| match child.size {
            Size::Fixed(size) => size,
            Size::Grow(_) => 0.0,
        })
        .sum();
    let weight: f32 = node
        .children
        .iter()
        .map(|child| match child.size {
            Size::Grow(share) => share.max(0.0),
            Size::Fixed(_) => 0.0,
        })
        .sum();
    // What the growing children divide. Never negative: when the fixed ones ask
    // for more than there is, they overflow rather than the growing ones being
    // given a negative height, which would invert their rectangles.
    let left = (spare - taken).max(0.0);

    let mut pen = match node.axis {
        Axis::Row => inner.x,
        Axis::Column => inner.y,
    };
    for child in &node.children {
        let size = match child.size {
            Size::Fixed(size) => size,
            // With no weight anywhere, a growing child takes nothing rather than
            // dividing by zero.
            Size::Grow(share) if weight > 0.0 => left * (share.max(0.0) / weight),
            Size::Grow(_) => 0.0,
        };
        let child_rect = match node.axis {
            Axis::Row => Rect::new(pen, inner.y, size, inner.height),
            Axis::Column => Rect::new(inner.x, pen, inner.width, size),
        };
        place(child, child_rect, depth + 1, into);
        pen += size + node.gap;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn find<'a>(placed: &'a [Placed], name: &str) -> &'a Placed {
        placed
            .iter()
            .find(|item| item.name == name)
            .unwrap_or_else(|| panic!("{name} was not placed"))
    }

    /// The shape of the app: a fixed sidebar, then everything else.
    #[test]
    fn a_fixed_child_takes_its_size_and_the_rest_share() {
        let tree = Node::new("shell", Size::Grow(1.0))
            .axis(Axis::Row)
            .with(Node::new("sidebar", Size::Fixed(260.0)))
            .with(Node::new("stream", Size::Grow(1.0)))
            .with(Node::new("thread", Size::Fixed(316.0)));
        let placed = solve(&tree, Rect::new(0.0, 0.0, 1280.0, 800.0));

        assert_eq!(
            find(&placed, "sidebar").rect,
            Rect::new(0.0, 0.0, 260.0, 800.0)
        );
        assert_eq!(
            find(&placed, "stream").rect,
            Rect::new(260.0, 0.0, 1280.0 - 260.0 - 316.0, 800.0)
        );
        assert_eq!(
            find(&placed, "thread").rect,
            Rect::new(1280.0 - 316.0, 0.0, 316.0, 800.0)
        );
    }

    /// Two growing children divide what is left by their weights.
    #[test]
    fn weights_divide_what_is_left() {
        let tree = Node::new("row", Size::Grow(1.0))
            .axis(Axis::Row)
            .with(Node::new("one", Size::Grow(1.0)))
            .with(Node::new("two", Size::Grow(3.0)));
        let placed = solve(&tree, Rect::new(0.0, 0.0, 400.0, 100.0));
        assert_eq!(find(&placed, "one").rect.width, 100.0);
        assert_eq!(find(&placed, "two").rect.width, 300.0);
    }

    #[test]
    fn padding_and_gaps_come_out_of_the_children() {
        let tree = Node::new("column", Size::Grow(1.0))
            .padding(10.0)
            .gap(6.0)
            .with(Node::new("top", Size::Fixed(20.0)))
            .with(Node::new("bottom", Size::Grow(1.0)));
        let placed = solve(&tree, Rect::new(0.0, 0.0, 200.0, 100.0));
        let top = find(&placed, "top").rect;
        let bottom = find(&placed, "bottom").rect;
        assert_eq!(top, Rect::new(10.0, 10.0, 180.0, 20.0));
        // 100 tall, 20 of padding, 6 of gap, 20 for the fixed child.
        assert_eq!(bottom, Rect::new(10.0, 36.0, 180.0, 54.0));
    }

    /// A window too small for its fixed children must not invert anything.
    #[test]
    fn nothing_is_given_a_negative_size() {
        let tree = Node::new("row", Size::Grow(1.0))
            .axis(Axis::Row)
            .with(Node::new("wide", Size::Fixed(500.0)))
            .with(Node::new("rest", Size::Grow(1.0)));
        let placed = solve(&tree, Rect::new(0.0, 0.0, 300.0, 100.0));
        assert_eq!(find(&placed, "rest").rect.width, 0.0);
        for item in &placed {
            assert!(
                item.rect.width >= 0.0 && item.rect.height >= 0.0,
                "{item:?}"
            );
        }
    }

    /// Depth grows downwards, and children come after their parent -- which is
    /// what lets a hit test take the last match.
    #[test]
    fn children_are_placed_after_their_parent() {
        let tree = Node::new("outer", Size::Grow(1.0))
            .with(Node::new("inner", Size::Grow(1.0)).with(Node::new("leaf", Size::Grow(1.0))));
        let placed = solve(&tree, Rect::new(0.0, 0.0, 100.0, 100.0));
        let names: Vec<&str> = placed.iter().map(|item| item.name.as_str()).collect();
        assert_eq!(names, ["outer", "inner", "leaf"]);
        assert_eq!(placed[2].depth, 2);
    }
}
