//! Choosing an emoji to react with.
//!
//! The other half of reactions: the pills could be toggled but a new one could
//! never be added, which made them a feature you could only use on a message
//! somebody else had already reacted to.
//!
//! Standard and custom together in one list, scored by the same fuzzy matcher
//! the switcher uses. A reader typing "bong" does not know or care which kind
//! `:bongo:` is.

use crate::composer::Composer;
use crate::sidebar::Canvas;
use matterless_layout::Fonts;
use matterless_paint::Run;
use matterless_ui::input::{Input, Key};
use matterless_ui::{Placed, Rect};

pub const NAME: &str = "picker";

/// How many are offered. A grid rather than a column, because an emoji is a
/// picture and pictures read faster side by side.
const COLUMNS: usize = 8;
const ROWS: usize = 4;
const CELL: f32 = 34.0;
const PADDING: f32 = 8.0;
/// What the stylesheet cuts a floating panel's corners by.
const PANEL: f32 = 8.0;

/// One offered emoji.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Choice {
    pub name: String,
    /// The character, for a standard one. `None` means it is a picture.
    pub unicode: Option<String>,
    /// The id its picture is behind, for a custom one.
    pub id: Option<String>,
}

/// The picker, and the message it was opened for.
pub struct Picker {
    /// The post a chosen emoji would go on. `None` when it is shut.
    pub for_post: Option<String>,
    pub query: Composer,
    found: Vec<Choice>,
    chosen: usize,
    /// The query the offered set belongs to. `None` until one has been asked,
    /// which is not the same as having asked an empty one -- and a plain String
    /// could not tell those apart, so a fresh picker offered nothing.
    asked: Option<String>,
}

impl Default for Picker {
    fn default() -> Self {
        Self::new()
    }
}

impl Picker {
    pub fn new() -> Self {
        let mut query = Composer::new(NAME).plain();
        query.placeholder = "React with…".to_string();
        Self {
            for_post: None,
            query,
            found: Vec::new(),
            chosen: 0,
            asked: None,
        }
    }

    pub fn open(&self) -> bool {
        self.for_post.is_some()
    }

    pub fn show(&mut self, post_id: &str, fonts: &mut Fonts, input: &mut Input) {
        self.for_post = Some(post_id.to_string());
        self.query.clear(fonts);
        self.found.clear();
        self.chosen = 0;
        // Forgotten, so an empty query is asked afresh rather than looking
        // unchanged from the last time it was open.
        self.asked = None;
        input.focus_on(NAME);
    }

    pub fn hide(&mut self, input: &mut Input) {
        self.for_post = None;
        if input.focus() == Some(NAME) {
            input.focus_on(crate::composer::NAME);
        }
    }

    /// Anchored under the message it belongs to, and kept on screen.
    pub fn rect(&self, near: Rect, within: Rect) -> Rect {
        let width = COLUMNS as f32 * CELL + PADDING * 2.0;
        let height = ROWS as f32 * CELL + PADDING * 2.0 + self.query.height();
        Rect::new(
            near.x
                .min(within.right() - width - PADDING)
                .max(within.x + PADDING),
            near.y
                .min(within.bottom() - height - PADDING)
                .max(within.y + PADDING),
            width,
            height,
        )
    }

    pub fn field(&self, near: Rect, within: Rect) -> Rect {
        let panel = self.rect(near, within);
        Rect::new(panel.x, panel.y, panel.width, self.query.height())
    }

    /// Where each cell sits, so a click can land on one.
    fn cells(&self, near: Rect, within: Rect) -> Vec<(usize, Rect)> {
        let panel = self.rect(near, within);
        let top = panel.y + self.query.height() + PADDING;
        self.found
            .iter()
            .enumerate()
            .map(|(at, _)| {
                let (column, row) = (at % COLUMNS, at / COLUMNS);
                (
                    at,
                    Rect::new(
                        panel.x + PADDING + column as f32 * CELL,
                        top + row as f32 * CELL,
                        CELL,
                        CELL,
                    ),
                )
            })
            .collect()
    }

    pub fn boxes(&self, near: Rect, within: Rect) -> Vec<Placed> {
        if !self.open() {
            return Vec::new();
        }
        let mut placed = vec![Placed {
            name: NAME.to_string(),
            rect: self.rect(near, within),
            depth: 8,
        }];
        for (at, rect) in self.cells(near, within) {
            placed.push(Placed {
                name: format!("{NAME}/{at}"),
                rect,
                depth: 9,
            });
        }
        placed
    }

    /// Narrows the offered set, if the query has changed.
    fn ask(&mut self, store: Option<&matterless_store::Store>) {
        let typed = self.query.text().trim().to_lowercase();
        if self.asked.as_deref() == Some(typed.as_str()) {
            return;
        }
        self.asked = Some(typed.clone());
        self.chosen = 0;
        let room = COLUMNS * ROWS;

        let mut found: Vec<Choice> = Vec::new();
        // Custom first: they are this team's own, far fewer, and the ones a
        // reader is most often reaching for by name.
        if let Some(store) = store {
            for (name, id) in store
                .custom_emoji_matching(&typed, room as u32)
                .unwrap_or_default()
            {
                found.push(Choice {
                    name,
                    unicode: None,
                    id: Some(id),
                });
            }
        }
        let wanted = if typed.is_empty() {
            // Something to press before a word is typed. The common few rather
            // than the first alphabetically, which would be a wall of nothing
            // anybody reacts with.
            vec![
                "+1",
                "heart",
                "tada",
                "eyes",
                "rocket",
                "white_check_mark",
                "smile",
                "cry",
            ]
        } else {
            matterless_render::emoji::names_matching(&typed, room)
        };
        for name in wanted {
            if found.len() >= room {
                break;
            }
            // Only ones the table can actually draw. A standard name with no
            // character behind it would be an empty cell that does nothing
            // when pressed, which is worse than one fewer thing to press.
            let Some(unicode) = matterless_render::emoji::character_for(name) else {
                continue;
            };
            found.push(Choice {
                name: name.to_string(),
                unicode: Some(unicode),
                id: None,
            });
        }
        found.truncate(room);
        self.found = found;
    }

    /// Applies a frame's input. Answers the emoji chosen, by name.
    pub fn react(
        &mut self,
        fonts: &mut Fonts,
        input: &Input,
        near: Rect,
        within: Rect,
        clipboard: &mut String,
        store: Option<&matterless_store::Store>,
    ) -> Option<String> {
        if !self.open() {
            return None;
        }
        if input.struck(Key::Right) && !self.found.is_empty() {
            self.chosen = (self.chosen + 1).min(self.found.len() - 1);
        }
        if input.struck(Key::Left) {
            self.chosen = self.chosen.saturating_sub(1);
        }
        if input.struck(Key::Down) && !self.found.is_empty() {
            self.chosen = (self.chosen + COLUMNS).min(self.found.len() - 1);
        }
        if input.struck(Key::Up) {
            self.chosen = self.chosen.saturating_sub(COLUMNS);
        }
        // A click on a cell, which is the ordinary way to pick a picture.
        if let Some(clicked) = input.clicked()
            && let Some(at) = clicked
                .strip_prefix(&format!("{NAME}/"))
                .and_then(|at| at.parse::<usize>().ok())
            && let Some(choice) = self.found.get(at)
        {
            return Some(choice.name.clone());
        }
        let field = self.field(near, within);
        let entered = self
            .query
            .react(fonts, input, field, clipboard)
            .is_some_and(|text| !text.is_empty());
        self.query.lay_out(fonts, self.rect(near, within).width);
        self.ask(store);
        if entered {
            return self.found.get(self.chosen).map(|one| one.name.clone());
        }
        None
    }

    pub fn draw(&self, into: &mut Canvas<'_>, near: Rect, within: Rect) {
        if !self.open() {
            return;
        }
        let panel = self.rect(near, within);
        let cells = self.cells(near, within);
        let Canvas {
            scene,
            painter,
            fonts,
            palette,
        } = into;
        scene.floating(
            panel.x,
            panel.y,
            panel.width,
            panel.height,
            palette.surface,
            PANEL,
            10.0,
        );
        for (at, rect) in cells {
            let Some(choice) = self.found.get(at) else {
                continue;
            };
            if at == self.chosen {
                scene.fill(rect.x, rect.y, rect.width, rect.height, palette.ground);
            }
            match (&choice.unicode, &choice.id) {
                // A character, drawn as one: the colour atlas already handles
                // it, exactly as it does in a message.
                (Some(face), _) => {
                    let glyphs = painter.run(
                        fonts,
                        face,
                        rect.x + 8.0,
                        rect.y + 6.0,
                        Run {
                            size: 18.0,
                            line_height: 22.0,
                            bold: false,
                            mono: false,
                            wrap: f32::MAX,
                            icon: false,
                            smooth: false,
                        },
                    );
                    scene.glyphs(glyphs, palette.ink, palette.faint);
                }
                // A picture, through the same path every other picture takes.
                (None, Some(id)) => scene.extend([matterless_paint::Piece::Image {
                    x: rect.x + 8.0,
                    y: rect.y + 8.0,
                    width: 18.0,
                    height: 18.0,
                    key: crate::stream::emoji_key(id),
                    radius: 0.0,
                }]),
                (None, None) => {}
            }
        }
    }

    /// The pictures the offered set needs, so the caller can fetch them.
    pub fn wants(&self) -> Vec<(String, u32, u32)> {
        self.found
            .iter()
            .filter_map(|choice| choice.id.as_ref())
            .map(|id| (crate::stream::emoji_key(id), 32, 32))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Something to press before a word is typed, or the picker opens blank
    /// and looks broken.
    #[test]
    fn it_offers_something_before_anything_is_typed() {
        let mut picker = Picker::new();
        picker.for_post = Some("p1".into());
        picker.ask(None);
        assert!(!picker.found.is_empty());
        assert!(picker.found.iter().all(|one| one.unicode.is_some()));
    }

    /// Never more than the grid can hold, whatever the query matches.
    #[test]
    fn it_never_offers_more_than_the_grid_holds() {
        let mut picker = Picker::new();
        picker.for_post = Some("p1".into());
        picker.asked = Some("x".into());
        picker.ask(None);
        assert!(picker.found.len() <= COLUMNS * ROWS);
    }

    /// The panel follows the message but never leaves the window.
    #[test]
    fn the_panel_stays_on_screen() {
        let picker = Picker::new();
        let window = Rect::new(0.0, 0.0, 900.0, 600.0);
        // Anchored off the bottom right corner, which is where a message near
        // the composer would put it.
        let panel = picker.rect(Rect::new(880.0, 590.0, 10.0, 10.0), window);
        assert!(panel.right() <= window.right());
        assert!(panel.bottom() <= window.bottom());
        assert!(panel.x >= window.x);
        assert!(panel.y >= window.y);
    }
}
