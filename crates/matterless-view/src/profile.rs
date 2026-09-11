//! Who somebody is, when their name is pressed.
//!
//! A small card rather than a pane: the question a mention raises is "who is
//! that", and the answer is a few lines. Anything larger would be a second
//! conversation covering the one being read.
//!
//! Answered from the local store. Every name in a message belongs to somebody
//! this client has already seen -- the plan resolved their display name to
//! draw the mention at all -- so a round trip would be asking again for what
//! is already held.

use crate::sidebar::Canvas;
use matterless_paint::Run;
use matterless_ui::{Placed, Rect};

pub const NAME: &str = "profile";

const WIDTH: f32 = 280.0;
const PADDING: f32 = 12.0;
const LINE: f32 = 20.0;
const FACE: f32 = 48.0;

/// A person, reduced to what a card says about them.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Card {
    pub user_id: String,
    pub username: String,
    /// Their real name, when the server holds one. Empty is ordinary.
    pub full_name: String,
    /// What they call themselves, when they have set one.
    pub nickname: String,
    /// Their presence, as a word. Empty when nobody has asked.
    pub status: String,
    /// The avatar's version, so a new picture is a new name in the atlas.
    pub avatar_at: i64,
}

/// The open card, if one is.
#[derive(Debug)]
pub struct Profile {
    pub of: Option<Card>,
    /// Where the name that was pressed sits, so the card points at it.
    pub near: Rect,
}

impl Default for Profile {
    fn default() -> Self {
        Self {
            of: None,
            near: Rect::new(0.0, 0.0, 0.0, 0.0),
        }
    }
}

impl Profile {
    pub fn open(&self) -> bool {
        self.of.is_some()
    }

    pub fn show(&mut self, card: Card, near: Rect) {
        self.of = Some(card);
        self.near = near;
    }

    pub fn hide(&mut self) {
        self.of = None;
    }

    /// What the card actually says, so its height is what it needs.
    fn lines(&self) -> Vec<(String, bool)> {
        let Some(card) = self.of.as_ref() else {
            return Vec::new();
        };
        let mut lines = vec![(format!("@{}", card.username), true)];
        // Only what is there. A blank line where a name would go says the
        // card is broken rather than that the field is empty.
        for said in [&card.full_name, &card.nickname, &card.status] {
            if !said.is_empty() {
                lines.push((said.clone(), false));
            }
        }
        lines
    }

    /// Under the name that was pressed, and never off the panel.
    pub fn rect(&self, within: Rect) -> Rect {
        let height = PADDING * 2.0 + (self.lines().len() as f32 * LINE).max(FACE);
        Rect::new(
            self.near
                .x
                .min(within.right() - WIDTH - PADDING)
                .max(within.x + PADDING),
            self.near
                .bottom()
                .min(within.bottom() - height - PADDING)
                .max(within.y + PADDING),
            WIDTH,
            height,
        )
    }

    pub fn boxes(&self, within: Rect) -> Vec<Placed> {
        if !self.open() {
            return Vec::new();
        }
        vec![Placed {
            name: NAME.to_string(),
            rect: self.rect(within),
            depth: 8,
        }]
    }

    /// The face it wants, so the caller can fetch it through the usual path.
    pub fn wants(&self) -> Vec<(String, u32, u32)> {
        self.of
            .as_ref()
            .map(|card| {
                vec![(
                    crate::stream::avatar_key(&card.user_id, card.avatar_at),
                    FACE as u32,
                    FACE as u32,
                )]
            })
            .unwrap_or_default()
    }

    pub fn draw(&self, into: &mut Canvas<'_>, within: Rect) {
        let Some(card) = self.of.as_ref() else {
            return;
        };
        let panel = self.rect(within);
        let lines = self.lines();
        let Canvas {
            scene,
            painter,
            fonts,
            palette,
        } = into;
        scene.fill(panel.x, panel.y, panel.width, panel.height, palette.surface);
        scene.extend([matterless_paint::Piece::Image {
            x: panel.x + PADDING,
            y: panel.y + PADDING,
            width: FACE,
            height: FACE,
            key: crate::stream::avatar_key(&card.user_id, card.avatar_at),
        }]);
        for (at, (said, bold)) in lines.into_iter().enumerate() {
            let run = if bold {
                Run::label(f32::MAX).bold()
            } else {
                Run::label(f32::MAX)
            };
            let glyphs = painter.run(
                fonts,
                &said,
                panel.x + PADDING + FACE + 10.0,
                panel.y + PADDING + at as f32 * LINE,
                run,
            );
            let ink = if bold { palette.ink } else { palette.faint };
            scene.glyphs(glyphs, ink, palette.faint);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn someone() -> Card {
        Card {
            user_id: "u1".into(),
            username: "ada".into(),
            full_name: "Ada Lovelace".into(),
            nickname: "Analyst".into(),
            status: "online".into(),
            avatar_at: 7,
        }
    }

    /// The card follows the name it was opened from, but a name at the edge
    /// must not push it off: a card you cannot read answers nothing.
    #[test]
    fn the_card_stays_inside_the_panel() {
        let mut profile = Profile::default();
        profile.show(someone(), Rect::new(980.0, 690.0, 60.0, 16.0));
        let panel = Rect::new(0.0, 0.0, 1000.0, 700.0);
        let card = profile.rect(panel);
        assert!(card.right() <= panel.right());
        assert!(card.bottom() <= panel.bottom());
        assert!(card.x >= panel.x);
        assert!(card.y >= panel.y);
    }

    /// Only what the server actually holds. A blank line where a name would
    /// go reads as a broken card rather than an empty field.
    #[test]
    fn an_empty_field_is_left_out_rather_than_drawn_blank() {
        let mut profile = Profile::default();
        profile.show(
            Card {
                full_name: String::new(),
                nickname: String::new(),
                status: String::new(),
                ..someone()
            },
            Rect::new(0.0, 0.0, 0.0, 0.0),
        );
        assert_eq!(profile.lines().len(), 1);
        profile.show(someone(), Rect::new(0.0, 0.0, 0.0, 0.0));
        assert_eq!(profile.lines().len(), 4);
    }

    /// A shut card places nothing and asks for no picture.
    #[test]
    fn a_shut_card_is_not_there_at_all() {
        let profile = Profile::default();
        assert!(profile.boxes(Rect::new(0.0, 0.0, 800.0, 600.0)).is_empty());
        assert!(profile.wants().is_empty());
    }
}
