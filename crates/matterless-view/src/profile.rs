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
use matterless_widgets::Panel;

pub const NAME: &str = "profile";

const WIDTH: f32 = 268.0;
const PADDING: f32 = 12.0;
const LINE: f32 = 20.0;
/// Between the card and the name it was opened from, on whichever side it
/// ends up.
const GAP: f32 = 6.0;
const FACE: f32 = 48.0;
/// How far the card is lifted off the conversation it was opened from.
const DROP: f32 = 10.0;
/// What a panel's corners are cut by, from the stylesheet.
const PANEL: f32 = 8.0;

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
    /// Opened by the press being answered right now.
    ///
    /// The card is dismissed by a click anywhere that is not on it, and the
    /// click that opens it is a click on a name in a message -- which is not
    /// on it. Presses are answered before the dismissal is considered, so
    /// without this the card was shown and hidden again inside one frame and
    /// never appeared at all: a name could be pressed, the pointer said so,
    /// and nothing happened.
    fresh: bool,
}

impl Default for Profile {
    fn default() -> Self {
        Self {
            of: None,
            near: Rect::new(0.0, 0.0, 0.0, 0.0),
            fresh: false,
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
        self.fresh = true;
    }

    pub fn hide(&mut self) {
        self.of = None;
        self.fresh = false;
    }

    /// Whether the press being answered is the one that opened it.
    ///
    /// Answers true once. The click that opened the card must not also close
    /// it; the next one must.
    pub fn opening(&mut self) -> bool {
        std::mem::take(&mut self.fresh)
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
            self.above_or_below(within, height),
            WIDTH,
            height,
        )
    }

    /// Under the name, or over it when there is no room under.
    ///
    /// Sliding it up to fit was the same as putting it on top of the name it
    /// is about: a card opened from the last message in a channel covered that
    /// message, so what the reader had just pressed was the one thing they
    /// could no longer see. The side flips instead, which is what a popover
    /// does and what leaves the name it belongs to visible.
    fn above_or_below(&self, within: Rect, height: f32) -> f32 {
        let below = self.near.bottom() + GAP;
        let above = self.near.y - GAP - height;
        if below + height + PADDING <= within.bottom() {
            return below;
        }
        if above >= within.y + PADDING {
            return above;
        }
        // Taller than the room on either side of it, which no amount of
        // choosing fixes: keep it on screen and let it overlap.
        below
            .min(within.bottom() - height - PADDING)
            .max(within.y + PADDING)
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
        // Through the widget rather than by hand, which is what gives it the
        // hairline. Drawn as a shadow and a fill alone, a panel has no edge at
        // all: the shadow fades into the surface with nothing to fade away
        // *from*, and what says a popup is over the window rather than part of
        // it is that hard boundary, not the darkness under it.
        Panel::floating(panel, PANEL, DROP)
            .edge(palette.rule)
            .fill(palette.surface)
            .draw(scene);
        scene.rounded(
            panel.x + PADDING,
            panel.y + PADDING,
            FACE,
            FACE,
            palette.raised,
            FACE / 2.0,
        );
        scene.extend([matterless_paint::Piece::Image {
            x: panel.x + PADDING,
            y: panel.y + PADDING,
            width: FACE,
            height: FACE,
            key: crate::stream::avatar_key(&card.user_id, card.avatar_at),
            radius: FACE / 2.0,
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

    /// The click that opens the card does not also close it.
    ///
    /// The card is dismissed by a click that is not on it, and the click that
    /// opens it is on a name in a message -- which is not on it. Presses are
    /// answered before dismissal is considered, so the card was shown and
    /// hidden inside the same frame and never appeared: pressing a name did
    /// nothing, while the pointer and the underline said it would.
    #[test]
    fn the_press_that_opens_the_card_is_not_the_one_that_shuts_it() {
        let mut profile = Profile::default();
        profile.show(someone(), Rect::new(100.0, 100.0, 60.0, 16.0));
        assert!(profile.opening(), "the press that opened it");
        assert!(
            !profile.opening(),
            "and only that one -- the next click has to be able to shut it"
        );
    }

    /// Answered once whether or not anything acted on it, or the flag outlives
    /// the frame it belongs to and swallows a later click instead.
    #[test]
    fn opening_is_forgotten_even_when_nobody_acts_on_it() {
        let mut profile = Profile::default();
        profile.show(someone(), Rect::new(0.0, 0.0, 10.0, 10.0));
        profile.hide();
        assert!(
            !profile.opening(),
            "a card that was shut still claimed to be opening"
        );
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

    /// A name with nothing under it puts the card over it instead.
    ///
    /// Slid up to fit, the card covered the very name it was opened from --
    /// so pressing somebody in the last message of a channel hid that message
    /// behind the answer.
    #[test]
    fn a_card_with_no_room_below_opens_above_the_name() {
        let mut profile = Profile::default();
        let panel = Rect::new(0.0, 0.0, 1000.0, 700.0);
        let name = Rect::new(200.0, 660.0, 90.0, 16.0);
        profile.show(someone(), name);
        let card = profile.rect(panel);
        assert!(
            card.bottom() <= name.y,
            "the card sits over the name it is about: {card:?} against {name:?}"
        );
        assert!(card.y >= panel.y, "and off the top of the panel");
    }

    /// And stays under it whenever it fits, which is the ordinary case.
    #[test]
    fn a_card_with_room_opens_under_the_name() {
        let mut profile = Profile::default();
        let panel = Rect::new(0.0, 0.0, 1000.0, 700.0);
        let name = Rect::new(200.0, 120.0, 90.0, 16.0);
        profile.show(someone(), name);
        let card = profile.rect(panel);
        assert!(
            card.y >= name.bottom(),
            "the card jumped above a name with room under it: {card:?}"
        );
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
