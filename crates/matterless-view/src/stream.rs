//! A scrolling conversation, and the rows a pointer can land on.
//!
//! Extracted from the shell rather than written for the thread pane, but the
//! thread pane is why: a thread is the same rows, the same layout and the same
//! drawing in a narrower column, and two panels that draw messages by walking
//! the layout separately is exactly how the browser and the virtualiser came to
//! disagree about heights.
//!
//! Rows become hit targets here for the first time. Everything the stream will
//! eventually do -- open a thread, hover an action, select text -- starts with
//! knowing which message the pointer is over.

use crate::sidebar::Canvas;
use matterless_layout::Fonts;
use matterless_layout::row::{RowLayout, Theme, lay_out};
use matterless_render::Row;
use matterless_ui::input::Input;
use matterless_ui::{Placed, Rect};

/// One conversation: its rows, their heights, and where the reader is in it.
pub struct Stream {
    /// What this panel answers to, so a channel and a thread can coexist.
    pub name: String,
    pub rows: Vec<Row>,
    pub laid: Vec<RowLayout>,
    pub theme: Theme,
    pub scroll: f32,
}

impl Stream {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            rows: Vec::new(),
            laid: Vec::new(),
            theme: Theme::default(),
            scroll: 0.0,
        }
    }

    /// Lays every row out for this width.
    ///
    /// Once per width change, never per frame: a row's height does not depend
    /// on the scroll position, which is the property that makes this list
    /// honest and the DOM one not.
    pub fn lay_out(&mut self, fonts: &mut Fonts, width: f32) {
        self.theme = Theme {
            width,
            ..Theme::default()
        };
        self.laid = self
            .rows
            .iter()
            .map(|row| lay_out(fonts, row, &self.theme))
            .collect();
    }

    pub fn total(&self) -> f32 {
        self.laid.iter().map(|row| row.height).sum()
    }

    /// How far it can be scrolled before it runs out.
    pub fn reach(&self, within: Rect) -> f32 {
        (self.total() - within.height).max(0.0)
    }

    /// Shows the newest message, which is where a conversation opens.
    pub fn to_bottom(&mut self, within: Rect) {
        self.scroll = self.reach(within);
    }

    pub fn clamp(&mut self, within: Rect) {
        self.scroll = self.scroll.clamp(0.0, self.reach(within));
    }

    /// The thread a row belongs to: its own id when it is a root, and the root
    /// it hangs from when it is a reply.
    ///
    /// `None` for the rows that are not messages -- a date separator has no
    /// thread to open, and clicking one must do nothing rather than open the
    /// last thread that happened to be under the pointer.
    pub fn root_of(&self, index: usize) -> Option<String> {
        match self.rows.get(index)? {
            Row::Post { post } | Row::Continuation { post } => Some(if post.root_id.is_empty() {
                post.post_id.clone()
            } else {
                post.root_id.clone()
            }),
            Row::ThreadFooter { root_id, .. } => Some(root_id.clone()),
            _ => None,
        }
    }

    fn row_name(&self, index: usize) -> String {
        format!("{}/row/{index}", self.name)
    }

    /// The panel, and every row currently on screen.
    ///
    /// Only the visible ones, unlike the sidebar: a channel is hundreds of
    /// thousands of rows where a sidebar is hundreds, and placing them all
    /// would cost more than the drawing does.
    pub fn boxes(&self, within: Rect) -> Vec<Placed> {
        let mut placed = vec![Placed {
            name: self.name.clone(),
            rect: within,
            depth: 1,
        }];
        let mut top = within.y - self.scroll;
        for (index, row) in self.laid.iter().enumerate() {
            let bottom = top + row.height;
            if bottom >= within.y && top <= within.bottom() {
                placed.push(Placed {
                    name: self.row_name(index),
                    // Clipped to the panel, so a row half off the top is only
                    // hit where it can actually be seen.
                    rect: Rect::new(
                        within.x,
                        top.max(within.y),
                        within.width,
                        (bottom.min(within.bottom()) - top.max(within.y)).max(0.0),
                    ),
                    depth: 2,
                });
            }
            top = bottom;
        }
        placed
    }

    /// Applies a frame's input. Answers the thread the reader opened.
    pub fn react(&mut self, input: &Input, placed: &[Placed], within: Rect) -> Option<String> {
        if let Some((_, y)) = input.wheel_over(placed, |name| name == self.name) {
            self.scroll = (self.scroll - y).clamp(0.0, self.reach(within));
        }
        let clicked = input.clicked()?;
        let index = self.index_of(clicked)?;
        self.root_of(index)
    }

    /// The row a name refers to, if it is one of this panel's.
    fn index_of(&self, name: &str) -> Option<usize> {
        name.strip_prefix(&format!("{}/row/", self.name))?
            .parse()
            .ok()
    }

    /// The row under the pointer, for drawing it hovered.
    fn hovered(&self, input: &Input) -> Option<usize> {
        self.index_of(input.hovered()?)
    }

    pub fn draw(&self, into: &mut Canvas<'_>, within: Rect, input: &Input) {
        let Canvas {
            scene,
            painter,
            fonts,
            palette,
        } = into;
        let hovered = self.hovered(input);
        let mut top = within.y - self.scroll;
        for (index, row) in self.laid.iter().enumerate() {
            let bottom = top + row.height;
            if bottom >= within.y && top <= within.bottom() {
                if hovered == Some(index) {
                    scene.fill(within.x, top, within.width, row.height, palette.surface);
                }
                let pieces = painter.pieces_of(fonts, row, top, &self.theme, palette);
                // Shifted into this panel's column: a row plan is laid out from
                // zero and knows nothing of where it lands.
                scene.extend(pieces.into_iter().map(|piece| shift(piece, within.x)));
            }
            top = bottom;
        }
    }
}

/// Moves a piece sideways into its panel.
fn shift(piece: matterless_paint::Piece, by: f32) -> matterless_paint::Piece {
    use matterless_paint::Piece;
    match piece {
        Piece::Fill {
            x,
            y,
            width,
            height,
            colour,
        } => Piece::Fill {
            x: x + by,
            y,
            width,
            height,
            colour,
        },
        Piece::Text { glyphs, ink, faint } => Piece::Text {
            glyphs: glyphs
                .into_iter()
                .map(|glyph| matterless_paint::PlacedGlyph {
                    x: glyph.x + by as i32,
                    ..glyph
                })
                .collect(),
            ink,
            faint,
        },
    }
}
