//! Boxes, and where they land.
//!
//! Everything outside the message list -- the sidebar, the header, the
//! composer, the panels -- is boxes inside boxes. This solves where each one
//! sits, and answers which one a pointer is over. It draws nothing and knows
//! nothing about a GPU: a layout that can only be checked by looking at it is a
//! layout that cannot be checked.
//!
//! The model is deliberately small. A box has an axis, padding, a gap, and
//! children that are each fixed, growing, or sized to their content. That is
//! enough for every panel in this app, and stopping there is the point: a
//! general layout engine is a year of work and none of it would be spent on
//! chat.

pub mod hit;
pub mod input;
pub mod solve;

/// Straight RGBA, the same order the painter takes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Colour(pub [u8; 4]);

impl Colour {
    pub const fn rgb(red: u8, green: u8, blue: u8) -> Self {
        Self([red, green, blue, 255])
    }
}

/// A rectangle in window space, in physical pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

impl Rect {
    pub fn new(x: f32, y: f32, width: f32, height: f32) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    pub fn right(&self) -> f32 {
        self.x + self.width
    }

    pub fn bottom(&self) -> f32 {
        self.y + self.height
    }

    pub fn holds(&self, x: f32, y: f32) -> bool {
        x >= self.x && y >= self.y && x < self.right() && y < self.bottom()
    }

    /// The rectangle inside `padding` on every side, never inverted.
    pub fn inset(&self, padding: f32) -> Self {
        Self {
            x: self.x + padding,
            y: self.y + padding,
            width: (self.width - padding * 2.0).max(0.0),
            height: (self.height - padding * 2.0).max(0.0),
        }
    }
}

/// Which way a box lays its children out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Axis {
    Row,
    Column,
}

/// How much of the axis a child asks for.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Size {
    /// Exactly this many pixels.
    Fixed(f32),
    /// A share of whatever is left once the fixed ones have taken theirs.
    Grow(f32),
}

/// One box: how it divides its space, and what it is called.
///
/// The name is what hit testing answers with. A string rather than an integer
/// so a hit reads as `"sidebar/channel/abc"` in a log rather than `7`, which is
/// the difference between a diagnosis and a guess.
#[derive(Debug, Clone)]
pub struct Node {
    pub name: String,
    pub axis: Axis,
    pub size: Size,
    pub padding: f32,
    pub gap: f32,
    pub children: Vec<Node>,
}

impl Node {
    pub fn new(name: impl Into<String>, size: Size) -> Self {
        Self {
            name: name.into(),
            axis: Axis::Column,
            size,
            padding: 0.0,
            gap: 0.0,
            children: Vec::new(),
        }
    }

    pub fn axis(mut self, axis: Axis) -> Self {
        self.axis = axis;
        self
    }

    pub fn padding(mut self, padding: f32) -> Self {
        self.padding = padding;
        self
    }

    pub fn gap(mut self, gap: f32) -> Self {
        self.gap = gap;
        self
    }

    pub fn with(mut self, child: Node) -> Self {
        self.children.push(child);
        self
    }

    pub fn holding(mut self, children: impl IntoIterator<Item = Node>) -> Self {
        self.children.extend(children);
        self
    }
}

/// A solved box: its name and where it ended up.
#[derive(Debug, Clone, PartialEq)]
pub struct Placed {
    pub name: String,
    pub rect: Rect,
    /// How deep in the tree, so hit testing can prefer the innermost.
    pub depth: usize,
}
