//! Changing a message that has already been said.
//!
//! In the row rather than in a dialog, and in place of the message it is
//! changing: the reason to reopen a sentence is to see it in the conversation
//! it belongs to, and a box floating somewhere else answers a different
//! question.
//!
//! Enter saves and Escape cancels, matching the composer beneath it. The raw
//! markdown comes back, not the rendered body: what was typed is what should
//! reappear.

use crate::composer::Composer;
use matterless_layout::Fonts;
use matterless_paint::Run;
use matterless_ui::input::{Input, Key};
use matterless_ui::{Placed, Rect};
use matterless_widgets::{Canvas, Named};

pub const NAME: &str = "edit";
/// What this widget's hit boxes are called. The join and its inverse in one
/// place, so the box it registers and the press it answers cannot disagree.
fn named() -> Named {
    Named::new(NAME)
}

/// The line under the box saying how to leave it.
const HINT: f32 = 18.0;

/// An open editor, and the message it belongs to.
pub struct Edit {
    /// The message being changed. `None` when nothing is being edited.
    pub for_post: Option<String>,
    pub box_of: Composer,
}

impl Default for Edit {
    fn default() -> Self {
        Self::new()
    }
}

impl Edit {
    pub fn new() -> Self {
        let mut box_of = Composer::new(NAME);
        box_of.placeholder = "Empty messages are deleted".to_string();
        Self {
            for_post: None,
            box_of,
        }
    }

    pub fn open(&self) -> bool {
        self.for_post.is_some()
    }

    /// Opens on a message, with its own words already in the box.
    pub fn show(&mut self, post_id: &str, text: &str, fonts: &mut Fonts, input: &mut Input) {
        self.for_post = Some(post_id.to_string());
        self.box_of.fill(text, fonts);
        input.focus_on(NAME);
    }

    pub fn hide(&mut self, input: &mut Input) {
        self.for_post = None;
        if input.focus() == Some(NAME) {
            input.focus_on(crate::composer::NAME);
        }
    }

    /// Over the row it is changing, and never off the panel.
    ///
    /// Pushed up when the row is near the bottom rather than allowed to hang
    /// past it: a box half off the screen cannot be finished.
    pub fn rect(&self, row: Rect, within: Rect) -> Rect {
        let height = self.box_of.height() + HINT;
        let top = row.y.min(within.bottom() - height).max(within.y);
        Rect::new(row.x, top, row.width, height)
    }

    /// The rect the composer lays itself out in, which is the box without the
    /// hint line under it.
    pub fn field(&self, row: Rect, within: Rect) -> Rect {
        let panel = self.rect(row, within);
        Rect::new(panel.x, panel.y, panel.width, self.box_of.height())
    }

    pub fn boxes(&self, row: Rect, within: Rect) -> Vec<Placed> {
        if !self.open() {
            return Vec::new();
        }
        let panel = self.rect(row, within);
        let mut placed = vec![Placed {
            name: named().of("panel"),
            rect: panel,
            depth: 8,
        }];
        // The composer's own box, deeper, so a click in the text lands on the
        // text rather than on the panel behind it.
        placed.extend(
            self.box_of
                .boxes(self.field(row, within))
                .into_iter()
                .map(|mut one| {
                    one.depth = 9;
                    one
                }),
        );
        placed
    }

    /// Applies a frame's input. Answers the new text when the reader saves.
    ///
    /// An empty answer is a real one: Mattermost deletes a message edited to
    /// nothing, and refusing to send it here would leave the reader with a box
    /// that will not close.
    pub fn react(
        &mut self,
        fonts: &mut Fonts,
        input: &Input,
        row: Rect,
        within: Rect,
        clipboard: &mut String,
    ) -> Option<String> {
        if !self.open() {
            return None;
        }
        let field = self.field(row, within);
        let saved = self.box_of.react(fonts, input, field, clipboard);
        self.box_of.lay_out(fonts, field.width);
        saved
    }

    pub fn draw(&self, into: &mut Canvas<'_>, row: Rect, within: Rect) {
        if !self.open() {
            return;
        }
        let panel = self.rect(row, within);
        let field = self.field(row, within);
        self.box_of.draw(into, field, true);
        let Canvas {
            scene,
            painter,
            fonts,
            palette,
        } = into;
        let hint = painter.run(
            fonts,
            "Enter saves, Escape cancels",
            panel.x + 24.0,
            panel.bottom() - HINT + 2.0,
            Run::label(f32::MAX),
        );
        scene.glyphs(hint, palette.faint, palette.faint);
    }
}

/// Whether Escape should close the editor rather than reach anything else.
pub fn cancels(input: &Input) -> bool {
    input.struck(Key::Escape)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The box follows the row, but a row at the very bottom must not push it
    /// off the panel: an editor you cannot see the end of cannot be finished.
    #[test]
    fn the_box_stays_inside_the_panel() {
        let mut edit = Edit::new();
        edit.for_post = Some("p1".into());
        let panel = Rect::new(260.0, 40.0, 740.0, 600.0);
        let low = Rect::new(260.0, 600.0, 740.0, 60.0);
        let box_of = edit.rect(low, panel);
        assert!(box_of.bottom() <= panel.bottom());
        assert!(box_of.y >= panel.y);
    }

    /// A shut editor answers nothing, whatever is typed at it.
    #[test]
    fn a_shut_editor_answers_nothing() {
        let mut fonts = Fonts::new();
        let mut edit = Edit::new();
        let saved = edit.react(
            &mut fonts,
            &Input::default(),
            Rect::new(0.0, 0.0, 700.0, 60.0),
            Rect::new(0.0, 0.0, 700.0, 600.0),
            &mut String::new(),
        );
        assert!(saved.is_none());
    }

    /// Opening it puts the message's own words in the box, so an edit starts
    /// from what was said rather than from nothing.
    #[test]
    fn it_opens_with_the_message_already_in_it() {
        let mut fonts = Fonts::new();
        let mut edit = Edit::new();
        let mut input = Input::default();
        edit.show("p1", "the first thing said", &mut fonts, &mut input);
        assert!(edit.open());
        assert_eq!(edit.box_of.text(), "the first thing said");
        assert_eq!(input.focus(), Some(NAME));
    }
}
