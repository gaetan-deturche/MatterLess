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

/// As wide as the official card, which is what gives its face room to be a
/// face rather than an icon.
const WIDTH: f32 = 300.0;
const PADDING: f32 = 16.0;
/// Between the card and the name it was opened from, on whichever side it
/// ends up.
const GAP: f32 = 6.0;
/// The presence dot on the corner of the face, and the ring of card around it
/// that keeps it off the picture.
const DOT: f32 = 16.0;
const DOT_RING: f32 = 3.0;
/// Between the face and the words under it.
const UNDER_FACE: f32 = 12.0;
/// The rule between who somebody is and how to reach them, and its room.
const RULE_ROOM: f32 = 12.0;
/// A line with a mark in front of it: the address.
const REACH: f32 = 22.0;
/// The button that opens a conversation with them.
const BUTTON: f32 = 32.0;
/// The cross in the corner.
const CROSS: f32 = 24.0;
/// The mail mark and the room after it, before the address.
const MARK_ROOM: f32 = 22.0;

/// How wide a line is, set the way it is drawn -- which is what centring it
/// needs, bold included.
fn wide(fonts: &mut matterless_layout::Fonts, text: &str, run: Run) -> f32 {
    matterless_layout::extent_of(
        fonts,
        text,
        f32::MAX,
        matterless_layout::Style {
            size: run.size,
            line_height: run.line_height,
            bold: run.bold,
            italic: false,
            mono: run.mono,
        },
    )
    .width
}

/// Which of the things a card says, and so how it is set.
///
/// Every line used to be the same words in the same face, bold for the handle
/// and faint for the rest, which made the card one column of grey text rather
/// than four answers to four questions. What somebody is called, what they
/// call themselves and whether they are at their desk are not the same kind of
/// fact and should not look like one.
#[derive(Clone, Copy, PartialEq, Debug)]
enum Shown {
    /// The handle, in the form you would type to reach them.
    Handle,
    /// Their name, which is the thing most readers came to find.
    Name,
    /// Whatever they chose to call themselves. Theirs rather than the
    /// server's, so it is set quieter than the name.
    Nickname,
    /// What they do there.
    Position,
    /// When they were last about, for somebody who is not about now.
    LastOnline,
}

impl Shown {
    /// How tall a line of it is.
    fn tall(self) -> f32 {
        match self {
            Shown::Name => 24.0,
            Shown::Handle | Shown::Nickname | Shown::Position | Shown::LastOnline => 19.0,
        }
    }

    fn run(self) -> Run {
        let (size, bold) = match self {
            Shown::Name => (16.0, true),
            Shown::Handle | Shown::Nickname => (13.0, false),
            Shown::Position => (12.5, false),
            Shown::LastOnline => (12.0, false),
        };
        Run {
            size,
            line_height: self.tall(),
            bold,
            mono: false,
            wrap: f32::MAX,
            icon: false,
            smooth: false,
        }
    }

    /// What it is written in. The name is worth reading and is inked as such;
    /// the two below it are context and recede.
    fn ink(self, palette: &matterless_paint::Palette) -> [u8; 3] {
        match self {
            Shown::Name => palette.ink,
            Shown::Handle | Shown::Position => palette.soft,
            Shown::Nickname | Shown::LastOnline => palette.faint,
        }
    }
}
/// The face, at the size the official card gives it.
const FACE: f32 = 120.0;
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
    /// Their address, when the server lets this reader see it.
    pub email: String,
    /// What they do there.
    pub position: String,
    /// When they were last active, in the server's milliseconds; zero when
    /// nobody has said.
    pub last_active: i64,
}

/// "Last online 7 min. ago", as the official card says it.
///
/// Only for somebody who is not about: for somebody who is, the dot on their
/// face says so and a time would say something else.
pub fn last_online(status: &str, since: i64, now: i64) -> Option<String> {
    if status == "online" || since <= 0 {
        return None;
    }
    let minutes = (now - since).max(0) / 60_000;
    Some(match minutes {
        0 => "Last online just now".to_string(),
        1..=59 => format!("Last online {minutes} min. ago"),
        60..=1439 => format!("Last online {} hr. ago", minutes / 60),
        1440..=2879 => "Last online yesterday".to_string(),
        _ => format!("Last online {} days ago", minutes / 1440),
    })
}

/// Where the cross sits on a card.
fn cross(panel: Rect) -> Rect {
    Rect::new(
        panel.right() - PADDING - CROSS + 6.0,
        panel.y + PADDING - 6.0,
        CROSS,
        CROSS,
    )
}

/// Where the button sits on a card.
fn button(panel: Rect) -> Rect {
    Rect::new(
        panel.x + PADDING,
        panel.bottom() - PADDING - BUTTON,
        panel.width - PADDING * 2.0,
        BUTTON,
    )
}

/// What pressing the cross is called.
pub const CLOSE: &str = "profile/close";
/// What pressing the button is called.
pub const MESSAGE: &str = "profile/message";

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
    fn lines(&self) -> Vec<(String, Shown)> {
        let Some(card) = self.of.as_ref() else {
            return Vec::new();
        };
        let mut lines = Vec::new();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|since| since.as_millis() as i64)
            .unwrap_or(0);
        if let Some(said) = last_online(&card.status, card.last_active, now) {
            lines.push((said, Shown::LastOnline));
        }
        // The name first, as the thing most readers came for; the handle
        // under it. Somebody with no name on the server is their handle.
        match card.full_name.is_empty() {
            true => lines.push((format!("@{}", card.username), Shown::Name)),
            false => {
                lines.push((card.full_name.clone(), Shown::Name));
                lines.push((format!("@{}", card.username), Shown::Handle));
            }
        }
        // Only what is there. A blank line where a nickname would go says the
        // card is broken rather than that the field is empty.
        for (said, shown) in [
            (&card.nickname, Shown::Nickname),
            (&card.position, Shown::Position),
        ] {
            if !said.is_empty() {
                lines.push((said.clone(), shown));
            }
        }
        lines
    }

    /// How tall the words are altogether, which is what the card is built
    /// around now that its lines are not all the same height.
    fn words_tall(&self) -> f32 {
        self.lines().iter().map(|(_, shown)| shown.tall()).sum()
    }

    /// How to reach them: the rule and the address, when there is one.
    fn reach_tall(&self) -> f32 {
        match self.of.as_ref().is_some_and(|card| !card.email.is_empty()) {
            true => RULE_ROOM * 2.0 + 1.0 + REACH,
            false => 0.0,
        }
    }

    /// Under the name that was pressed, and never off the panel.
    pub fn rect(&self, within: Rect) -> Rect {
        let height = PADDING * 2.0
            + FACE
            + UNDER_FACE
            + self.words_tall()
            + self.reach_tall()
            + UNDER_FACE
            + BUTTON;
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
        let panel = self.rect(within);
        // The card first: a hit takes the last box it lands in, so the two
        // things on it that can be pressed come after the card they sit on.
        vec![
            Placed {
                name: NAME.to_string(),
                rect: panel,
                depth: 8,
            },
            Placed {
                name: MESSAGE.to_string(),
                rect: button(panel),
                depth: 9,
            },
            Placed {
                name: CLOSE.to_string(),
                rect: cross(panel),
                depth: 9,
            },
        ]
    }

    /// The face it wants, so the caller can fetch it through the usual path.
    pub fn wants(&self) -> Vec<(String, u32, u32)> {
        self.of
            .as_ref()
            .map(|card| {
                vec![(
                    crate::stream::avatar_key_sized(&card.user_id, card.avatar_at, FACE as u32),
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
        // Everything on the card is centred on it, as the official card sets
        // it out: the face, each line, the address.
        let middle = panel.x + panel.width / 2.0;
        let left = middle - FACE / 2.0;
        let face_y = panel.y + PADDING;
        scene.rounded(left, face_y, FACE, FACE, palette.raised, FACE / 2.0);
        scene.extend([matterless_paint::Piece::Image {
            x: left,
            y: face_y,
            width: FACE,
            height: FACE,
            key: crate::stream::avatar_key_sized(&card.user_id, card.avatar_at, FACE as u32),
            radius: FACE / 2.0,
        }]);
        // Presence on the face's corner, as everywhere else a face is drawn,
        // in a ring of the card so it stays off the picture.
        if let Some(lit) = crate::sidebar::dot(&card.status, palette) {
            let (x, y) = (left + FACE - DOT - 2.0, face_y + FACE - DOT - 2.0);
            scene.rounded(
                x - DOT_RING,
                y - DOT_RING,
                DOT + DOT_RING * 2.0,
                DOT + DOT_RING * 2.0,
                palette.surface,
                DOT / 2.0 + DOT_RING,
            );
            scene.rounded(x, y, DOT, DOT, lit, DOT / 2.0);
        }
        // The cross, which is the one way to shut it that a reader can see.
        let shut = cross(panel);
        let glyphs = painter.run(
            fonts,
            matterless_layout::marks::CLOSE,
            shut.x + 6.0,
            shut.y + 4.0,
            Run::mark(13.0),
        );
        scene.glyphs(glyphs, palette.soft, palette.faint);

        let mut y = face_y + FACE + UNDER_FACE;
        for (said, shown) in lines {
            let x = middle - wide(fonts, &said, shown.run()) / 2.0;
            let glyphs = painter.run(fonts, &said, x, y, shown.run());
            scene.glyphs(glyphs, shown.ink(palette), palette.faint);
            y += shown.tall();
        }
        if !card.email.is_empty() {
            y += RULE_ROOM;
            scene.fill(
                panel.x + PADDING,
                y,
                panel.width - PADDING * 2.0,
                1.0,
                palette.rule_soft,
            );
            y += 1.0 + RULE_ROOM;
            // The mark and the address centred as one thing.
            let run = Shown::Handle.run();
            let x = middle - (MARK_ROOM + wide(fonts, &card.email, run)) / 2.0;
            let mark = painter.run(
                fonts,
                matterless_layout::marks::MAIL,
                x,
                y + 3.0,
                Run::mark(13.0),
            );
            scene.glyphs(mark, palette.faint, palette.faint);
            let said = painter.run(fonts, &card.email, x + MARK_ROOM, y, run);
            scene.glyphs(said, palette.soft, palette.faint);
        }
        // The one thing to do from here, as the official card offers it.
        let press = button(panel);
        scene.rounded(
            press.x,
            press.y,
            press.width,
            press.height,
            [palette.signal[0], palette.signal[1], palette.signal[2], 255],
            5.0,
        );
        let label = "Message";
        let run = Run::label(f32::MAX);
        let wide = matterless_widgets::width_of(fonts, label, run.size);
        let glyphs = painter.run(
            fonts,
            label,
            press.x + (press.width - wide) / 2.0,
            press.y + (press.height - run.line_height) / 2.0,
            run,
        );
        scene.glyphs(
            glyphs,
            [palette.ground[0], palette.ground[1], palette.ground[2]],
            palette.faint,
        );
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
            email: "ada@example.invalid".into(),
            position: "Analyst".into(),
            last_active: 0,
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

    /// No two kinds of line on the card are set the same way.
    ///
    /// They all were: bold for the handle and faint for the other three, one
    /// size throughout, so the card read as a column of grey text rather than
    /// as four answers to four questions.
    #[test]
    fn each_thing_the_card_says_is_set_differently() {
        let palette = matterless_paint::Palette::default();
        let every = [
            Shown::Name,
            Shown::Handle,
            Shown::Nickname,
            Shown::Position,
            Shown::LastOnline,
        ];
        // Taken together: two lines may share a size as long as something else
        // about them differs, which is what "told apart" means.
        let mut seen = Vec::new();
        for shown in every {
            let run = shown.run();
            let set = (
                run.size.to_bits(),
                run.bold,
                shown.ink(&palette),
                shown.tall().to_bits(),
            );
            assert!(
                !seen.contains(&set),
                "{shown:?} is set exactly like something above it"
            );
            seen.push(set);
        }
        // And the order of them reads down the card, as the official card
        // has it: the name is the loudest thing on it, the handle under it, and
        // when somebody was last about the quietest.
        assert!(Shown::Name.run().size > Shown::Handle.run().size);
        assert!(Shown::Name.run().bold && !Shown::Handle.run().bold);
        assert!(Shown::Handle.run().size > Shown::LastOnline.run().size);
    }

    /// Presence is the dot on the face, as everywhere a face is drawn -- never
    /// a line of words -- and when somebody is not about, the card says when
    /// they last were.
    #[test]
    fn presence_is_on_the_face_and_absence_says_since_when() {
        let mut profile = Profile::default();
        profile.show(someone(), Rect::new(0.0, 0.0, 10.0, 10.0));
        assert!(
            profile.lines().iter().all(|(said, _)| said != "online"),
            "the status was said in words"
        );
        assert!(
            profile
                .lines()
                .iter()
                .all(|(_, shown)| *shown != Shown::LastOnline),
            "somebody who is online has no last-online line"
        );
        let away = Card {
            status: "away".into(),
            last_active: 1_000,
            ..someone()
        };
        profile.show(away, Rect::new(0.0, 0.0, 10.0, 10.0));
        let (said, shown) = profile.lines().first().cloned().expect("lines");
        assert_eq!(shown, Shown::LastOnline);
        assert!(said.starts_with("Last online"), "{said}");
    }

    /// Worded as the official card words it.
    #[test]
    fn last_online_is_worded_as_the_app_words_it() {
        let minute = 60_000;
        let now = 100_000 * minute;
        assert_eq!(last_online("online", now - 7 * minute, now), None);
        assert_eq!(last_online("away", 0, now), None, "nobody said");
        assert_eq!(
            last_online("away", now - 20_000, now).as_deref(),
            Some("Last online just now")
        );
        assert_eq!(
            last_online("offline", now - 7 * minute, now).as_deref(),
            Some("Last online 7 min. ago")
        );
        assert_eq!(
            last_online("offline", now - 150 * minute, now).as_deref(),
            Some("Last online 2 hr. ago")
        );
        assert_eq!(
            last_online("offline", now - 30 * 60 * minute, now).as_deref(),
            Some("Last online yesterday")
        );
        assert_eq!(
            last_online("offline", now - 5 * 24 * 60 * minute, now).as_deref(),
            Some("Last online 5 days ago")
        );
    }

    /// The card has two things to press, on it, and a press lands on them
    /// rather than on the card under them.
    #[test]
    fn the_button_and_the_cross_are_pressable_on_the_card() {
        let mut profile = Profile::default();
        profile.show(someone(), Rect::new(100.0, 100.0, 60.0, 16.0));
        let within = Rect::new(0.0, 0.0, 1000.0, 800.0);
        let boxes = profile.boxes(within);
        let names: Vec<&str> = boxes.iter().map(|placed| placed.name.as_str()).collect();
        assert_eq!(
            names,
            vec![NAME, MESSAGE, CLOSE],
            "the card first, its controls over it"
        );
        let card = boxes[0].rect;
        for placed in &boxes[1..] {
            let r = placed.rect;
            assert!(
                r.x >= card.x
                    && r.right() <= card.right()
                    && r.y >= card.y
                    && r.bottom() <= card.bottom(),
                "{} is not on the card",
                placed.name
            );
        }
        let hit = matterless_ui::hit::at(&boxes, boxes[1].rect.x + 5.0, boxes[1].rect.y + 5.0);
        assert_eq!(hit.map(|placed| placed.name.as_str()), Some(MESSAGE));
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
                position: String::new(),
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
