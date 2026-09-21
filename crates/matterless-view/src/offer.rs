//! The people and channels a half-typed name could mean.
//!
//! Opened by typing rather than by pressing anything, which is the whole of
//! what makes it different from the switcher: the query is the message being
//! written, so there is no field of its own and nothing to focus. What it
//! offers goes back into the box in place of what was typed.
//!
//! A list, not a panel. It hangs just above the box, because that is where
//! the eye already is -- and because a panel in the middle of the window
//! would cover the messages naming the person being named.

use crate::switcher::Face;
use matterless_paint::Run;
use matterless_ui::input::{Input, Key};
use matterless_ui::{Placed, Rect};
use matterless_widgets::{Canvas, Named, Panel};

pub const NAME: &str = "offer";

fn named() -> Named {
    Named::new(NAME)
}

/// What opens a list, and of what.
///
/// Teams are not here on purpose: Mattermost has no way of naming one inside
/// a message, so a list offering them would be a list whose answer cannot be
/// written down. Channels are `~`, which is what the store's own
/// `channels_matching` already says it is for.
pub const PEOPLE: char = '@';
pub const CHANNELS: char = '~';
pub const SIGILS: [char; 2] = [PEOPLE, CHANNELS];

/// One thing a half-typed name could mean.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Suggestion {
    /// What goes into the message, without its sigil.
    pub insert: String,
    /// What the row says, which is not always what is inserted: a person is
    /// read by their real name and mentioned by their username.
    pub label: String,
    /// Whose face to show, for a person.
    pub face: Option<Face>,
}

/// How many are shown. Past this the reader should type another letter rather
/// than read a longer list.
const ROWS: usize = 6;
const ROW: f32 = 30.0;
const PADDING: f32 = 6.0;
const WIDTH: f32 = 320.0;
const CORNER: f32 = 8.0;
const FACE: f32 = 20.0;
/// How far above the box it floats, so the two do not touch.
const LIFT: f32 = 6.0;

/// The list, open or shut.
#[derive(Debug, Default)]
pub struct Offer {
    /// What opened it, and what has been typed after it.
    pub sigil: char,
    pub said: String,
    pub found: Vec<Suggestion>,
    /// Which row Return would take. Always a real row while the list is up.
    pub chosen: usize,
    open: bool,
}

/// What a press or a keystroke asked for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Chose {
    /// Put this in the message.
    Name(Suggestion),
    /// Leave the typing alone and put the list away.
    Nothing,
}

impl Offer {
    pub fn open(&self) -> bool {
        self.open && !self.found.is_empty()
    }

    /// Offers these, for what is being typed now.
    ///
    /// The chosen row is kept while the query only grows, so typing another
    /// letter does not throw away the arrow keys a reader has already
    /// pressed -- and reset when it changes another way, because the list
    /// underneath is then a different list.
    pub fn show(&mut self, sigil: char, said: &str, found: Vec<Suggestion>) {
        let carried = self.open && self.sigil == sigil && said.starts_with(&self.said);
        self.sigil = sigil;
        self.said = said.to_string();
        self.found = found;
        self.open = true;
        self.chosen = match carried {
            true => self.chosen.min(self.found.len().saturating_sub(1)),
            false => 0,
        };
    }

    pub fn hide(&mut self) {
        self.open = false;
        self.found.clear();
        self.said.clear();
        self.chosen = 0;
    }

    /// How many rows are on show.
    fn shown(&self) -> usize {
        self.found.len().min(ROWS)
    }

    /// Where it hangs: above the box, against its left edge.
    ///
    /// Clamped to the top of the panel rather than allowed to run off it. A
    /// list whose first rows are off the screen is a list whose best answer
    /// cannot be read, and the best answer is the one at the top.
    pub fn rect(&self, box_of: Rect, within: Rect) -> Rect {
        let wanted = PADDING * 2.0 + self.shown() as f32 * ROW;
        // Whatever room there is, and no more -- not a row's worth insisted
        // on. A box pushed up under the header leaves less than a row above
        // it, and a list that took one anyway would be drawn over the very
        // box it is offering names to.
        let room = (box_of.y - LIFT - within.y).max(0.0);
        let height = wanted.min(room);
        Rect::new(
            box_of.x,
            (box_of.y - LIFT - height).max(within.y),
            WIDTH.min(box_of.width.max(120.0)),
            height,
        )
    }

    pub fn boxes(&self, box_of: Rect, within: Rect) -> Vec<Placed> {
        if !self.open() {
            return Vec::new();
        }
        let panel = self.rect(box_of, within);
        let mut placed = vec![Placed {
            name: NAME.to_string(),
            rect: panel,
            // Over the conversation and over the box: it is in front of both.
            depth: 7,
        }];
        for at in 0..self.shown() {
            placed.push(Placed {
                name: named().of(&format!("row/{at}")),
                rect: self.row(panel, at),
                depth: 8,
            });
        }
        placed
    }

    fn row(&self, panel: Rect, at: usize) -> Rect {
        Rect::new(
            panel.x + PADDING,
            panel.y + PADDING + at as f32 * ROW,
            (panel.width - PADDING * 2.0).max(0.0),
            ROW,
        )
    }

    /// Which row a hit box names.
    fn row_at(&self, name: &str) -> Option<usize> {
        named().slug(name)?.strip_prefix("row/")?.parse().ok()
    }

    /// Applies a frame's input, and says what the reader asked for.
    ///
    /// Answered before the box sees the keyboard, and the caller takes the
    /// keys it uses: Return here means "this name", and left to the box it
    /// would mean "send the message".
    pub fn react(&mut self, input: &Input) -> Option<Chose> {
        if !self.open() {
            return None;
        }
        if let Some(name) = input.clicked()
            && let Some(at) = self.row_at(name)
        {
            return self.found.get(at).cloned().map(Chose::Name);
        }
        if input.struck(Key::Escape) {
            return Some(Chose::Nothing);
        }
        // Wrapping, because a list this short is one where walking off an end
        // and expecting the other is the natural thing to try.
        let shown = self.shown().max(1);
        if input.struck(Key::Down) {
            self.chosen = (self.chosen + 1) % shown;
        }
        if input.struck(Key::Up) {
            self.chosen = (self.chosen + shown - 1) % shown;
        }
        if input.struck(Key::Enter) || input.struck(Key::Tab) {
            return self.found.get(self.chosen).cloned().map(Chose::Name);
        }
        None
    }

    /// Which keys it claims while it is up.
    ///
    /// Named in one place so the window takes exactly these off the box and
    /// no others: a list that swallowed every key would be a box that could
    /// not be typed in.
    pub const CLAIMS: [Key; 5] = [Key::Up, Key::Down, Key::Enter, Key::Tab, Key::Escape];

    pub fn draw(&self, into: &mut Canvas<'_>, box_of: Rect, within: Rect, input: &Input) {
        if !self.open() {
            return;
        }
        let panel = self.rect(box_of, within);
        let Canvas {
            scene,
            painter,
            fonts,
            palette,
        } = into;
        Panel::floating(panel, CORNER, 12.0)
            .edge(palette.rule)
            .fill(palette.surface)
            .draw(scene);

        for at in 0..self.shown() {
            let row = self.row(panel, at);
            let under = input.hovered() == Some(named().of(&format!("row/{at}")).as_str());
            // The chosen row is marked whether or not the pointer is near it:
            // the arrows are the way this list is meant to be used, and the
            // row Return takes has to say so.
            if at == self.chosen || under {
                let lit = match at == self.chosen {
                    true => palette.hover,
                    false => palette.raised,
                };
                scene.rounded(row.x, row.y, row.width, row.height, lit, 5.0);
            }
            let Some(one) = self.found.get(at) else {
                continue;
            };
            let mut left = row.x + 8.0;
            if let Some(face) = &one.face {
                scene.extend([matterless_paint::Piece::Image {
                    x: left,
                    y: row.y + (ROW - FACE) / 2.0,
                    width: FACE,
                    height: FACE,
                    key: crate::stream::avatar_key(&face.user_id, face.avatar_at),
                    radius: FACE / 2.0,
                }]);
                left += FACE + 8.0;
            }
            let said = matterless_layout::elided(
                fonts,
                &one.label,
                (row.right() - left - 8.0).max(20.0),
                crate::listing::label(false),
            );
            let glyphs = painter.run(fonts, &said, left, row.y + 6.0, Run::label(f32::MAX));
            scene.glyphs(glyphs, palette.ink, palette.faint);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn some(names: &[&str]) -> Vec<Suggestion> {
        names
            .iter()
            .map(|name| Suggestion {
                insert: (*name).to_string(),
                label: (*name).to_string(),
                face: None,
            })
            .collect()
    }

    fn one(name: &str) -> Option<Chose> {
        Some(Chose::Name(Suggestion {
            insert: name.to_string(),
            label: name.to_string(),
            face: None,
        }))
    }

    fn struck(key: Key) -> Input {
        let mut input = Input::default();
        input.apply(matterless_ui::input::Event::Key { key, down: true }, &[]);
        input
    }

    /// The arrows walk the list and wrap, because a list this short is one
    /// where walking off an end and expecting the other is what anybody tries.
    #[test]
    fn the_arrows_walk_the_list_and_wrap() {
        let mut offer = Offer::default();
        offer.show('@', "a", some(&["amy", "ada", "alex"]));
        assert_eq!(offer.chosen, 0);

        offer.react(&struck(Key::Down));
        assert_eq!(offer.chosen, 1);
        offer.react(&struck(Key::Up));
        offer.react(&struck(Key::Up));
        assert_eq!(offer.chosen, 2, "up from the first goes to the last");
        offer.react(&struck(Key::Down));
        assert_eq!(offer.chosen, 0, "and down from the last comes back");
    }

    /// Return takes the chosen row, and Escape takes none.
    #[test]
    fn return_takes_the_chosen_row_and_escape_takes_none() {
        let mut offer = Offer::default();
        offer.show('@', "a", some(&["amy", "ada"]));
        offer.react(&struck(Key::Down));

        assert_eq!(offer.react(&struck(Key::Enter)), one("ada"));
        assert_eq!(offer.react(&struck(Key::Tab)), one("ada"));
        assert_eq!(offer.react(&struck(Key::Escape)), Some(Chose::Nothing));
    }

    /// Typing another letter keeps the row already walked to; typing
    /// something else does not.
    ///
    /// A query that has only grown is the same question with more of it
    /// answered, and throwing the arrow keys away for that makes the list
    /// unusable by anybody who types while looking at it. A query that
    /// changed another way is a different list underneath.
    #[test]
    fn a_longer_query_keeps_the_chosen_row() {
        let mut offer = Offer::default();
        offer.show('@', "a", some(&["amy", "ada", "alex"]));
        offer.react(&struck(Key::Down));
        assert_eq!(offer.chosen, 1);

        offer.show('@', "ad", some(&["ada", "adam"]));
        assert_eq!(offer.chosen, 1, "the same question, more of it typed");

        offer.show('@', "b", some(&["ben", "bea"]));
        assert_eq!(offer.chosen, 0, "a different question starts at the top");
    }

    /// A list shorter than the row already chosen never points past its end.
    #[test]
    fn a_shrinking_list_never_points_past_itself() {
        let mut offer = Offer::default();
        offer.show('@', "a", some(&["amy", "ada", "alex"]));
        offer.react(&struck(Key::Down));
        offer.react(&struck(Key::Down));
        assert_eq!(offer.chosen, 2);

        offer.show('@', "amy", some(&["amy"]));
        assert_eq!(offer.chosen, 0);
        assert_eq!(offer.react(&struck(Key::Enter)), one("amy"));
    }

    /// Nothing found is nothing shown: an empty list over the box would be a
    /// panel saying nothing, in front of the conversation.
    #[test]
    fn nothing_found_is_not_open() {
        let mut offer = Offer::default();
        offer.show('@', "zzz", Vec::new());
        assert!(!offer.open());
        assert_eq!(offer.react(&struck(Key::Enter)), None);
    }

    /// It never hangs off the top of the panel it is drawn in.
    ///
    /// The best answer is the row at the top, so a list that ran off the top
    /// edge would be one whose best answer cannot be read.
    #[test]
    fn it_stays_inside_the_panel_above_a_box_near_the_top() {
        let mut offer = Offer::default();
        offer.show('@', "a", some(&["amy", "ada", "alex", "ann", "abe", "ali"]));
        let within = Rect::new(300.0, 40.0, 600.0, 700.0);

        let roomy = offer.rect(Rect::new(320.0, 600.0, 500.0, 90.0), within);
        assert!(roomy.y >= within.y);
        assert!(roomy.bottom() <= 600.0 - LIFT + 0.01);

        // A box pushed up under the header leaves almost nothing above it.
        let cramped = offer.rect(Rect::new(320.0, 70.0, 500.0, 90.0), within);
        assert!(cramped.y >= within.y, "it ran off the top of the panel");
        assert!(cramped.bottom() <= 70.0 - LIFT + 0.01, "it covers the box");
    }
}
