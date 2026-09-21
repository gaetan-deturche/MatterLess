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
        // Save and Cancel rather than a paperclip and Send. There is nothing
        // to attach to a message already said, and the way out was on the
        // keyboard only: Escape cancelled and nothing on screen said so.
        box_of.tools_for = crate::composer::Tools::Editing;
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

    /// How much room the row has to make for it.
    ///
    /// Told to the stream, which gives the row this height in place of its
    /// own words -- so the conversation below is pushed down rather than
    /// covered, and the name and time stay above the box.
    pub fn height(&self) -> f32 {
        self.box_of.height() + HINT
    }

    /// In the row it is changing, and never off the panel.
    ///
    /// Pushed up when the row is near the bottom rather than allowed to hang
    /// past it: a box half off the screen cannot be finished. The row is as
    /// tall as this now, so the clamp only bites while the row is scrolled
    /// half out of view.
    pub fn rect(&self, row: Rect, within: Rect) -> Rect {
        let height = self.height();
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
        let field = self.field(row, within);
        let mut placed = vec![Placed {
            name: named().of("panel"),
            rect: panel,
            depth: 8,
        }];
        // The composer's own box, deeper, so a click in the text lands on the
        // text rather than on the panel behind it.
        placed.extend(self.box_of.boxes(field).into_iter().map(|mut one| {
            one.depth = 9;
            one
        }));
        // And its buttons, deeper again: Save and Cancel are inside the box,
        // so a press on one would otherwise land in the words behind it.
        placed.extend(self.box_of.boxes_in(field).into_iter().map(|mut one| {
            one.depth = 10;
            one
        }));
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

    pub fn draw(&self, into: &mut Canvas<'_>, input: &Input, row: Rect, within: Rect) {
        if !self.open() {
            return;
        }
        let panel = self.rect(row, within);
        let field = self.field(row, within);
        self.box_of.draw(into, field, true);
        self.box_of.draw_over(into, input, field);
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

impl Edit {
    /// Which of its own buttons was pressed, if either was.
    pub fn pressed(&self, input: &Input) -> Option<crate::composer::Button> {
        self.open().then(|| self.box_of.pressed(input)).flatten()
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

    /// The box offers a way out, and it is not the one that saves.
    ///
    /// Reported against the official client, which has a Cancel beside its
    /// Save. This box had a paperclip and a Send: nothing to attach to a
    /// message already said, and the way out was on the keyboard only --
    /// Escape cancelled and nothing on screen said so.
    #[test]
    fn the_editor_offers_a_way_out_beside_the_way_on() {
        let mut edit = Edit::new();
        edit.for_post = Some("p1".into());
        let panel = Rect::new(260.0, 40.0, 740.0, 600.0);
        let row = Rect::new(260.0, 100.0, 740.0, 80.0);
        let field = edit.field(row, panel);

        let cancel = edit.box_of.cancel(field).expect("a way out");
        let save = edit.box_of.send(field);
        assert!(
            cancel.right() <= save.x,
            "the two buttons are on top of each other"
        );
        assert_eq!(cancel.y, save.y, "and not on different rows");
        assert!(
            save.right() <= field.right(),
            "the button that saves is off the end of the box"
        );

        // And a box a message is *written* in has neither: nothing has been
        // said yet, so there is nothing to abandon.
        let writing = crate::composer::Composer::new("composer");
        assert!(writing.cancel(field).is_none());
    }

    /// The row makes room for the box rather than the box covering the row.
    ///
    /// Reported against the official client: there the row grows, the name
    /// and the time stay above the box and the conversation below is pushed
    /// down. Here the panel floated at the row's own height, so it covered
    /// the name above it and the messages under it.
    #[test]
    fn the_box_asks_for_the_room_it_needs() {
        let mut fonts = Fonts::new();
        let mut edit = Edit::new();
        let mut input = Input::default();
        edit.show("p1", "one line", &mut fonts, &mut input);
        let one = edit.height();

        edit.box_of.fill("one\ntwo\nthree\nfour", &mut fonts);
        edit.box_of.lay_out(&mut fonts, 700.0);
        let four = edit.height();

        assert!(
            four > one,
            "{four} is no more room than {one} for four times the message"
        );
        assert!(
            one > HINT,
            "the height it asks for leaves nothing for the box itself"
        );
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
