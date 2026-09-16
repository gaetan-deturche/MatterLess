//! Jumping to a conversation by name.
//!
//! With a hundred and fourteen channels, the sidebar is a scroll and a hunt.
//! This is how a reader who knows where they are going gets there: type a few
//! letters, press return.
//!
//! It filters what the sidebar already holds rather than asking the store. The
//! window has every channel in memory, so the answer is instant and the reader
//! never sees a list catch up with their typing.

use crate::composer::Composer;
use crate::sidebar::{Canvas, Entry};
use matterless_layout::Fonts;
use matterless_paint::Run;
use matterless_ui::Rect;
use matterless_ui::input::{Input, Key};
use matterless_widgets::Panel;

/// What the box answers to when the pointer is tested against it.
pub const NAME: &str = "switcher";

/// How many matches are shown. Past this the reader should type another letter
/// rather than read a longer list.
const ROWS: usize = 8;
const ROW: f32 = 30.0;
const PADDING: f32 = 10.0;
const WIDTH: f32 = 560.0;
/// What the stylesheet cuts a dialog's corners by.
const PANEL: f32 = 8.0;

/// How many letters before the server is asked. One or two match half a team,
/// and the reader is still typing.
const LEAST: usize = 2;

/// What choosing a match does.
///
/// The same list offers three things a reader might mean by a name: a
/// conversation they are in, one they are not, and a person they have never
/// written to. Only the last step differs, so they are one list rather than
/// three panels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reach {
    /// A conversation in the sidebar. Opening it is a local read.
    Open,
    /// A public channel this reader is not in. Joining comes first.
    Join,
    /// Somebody, by user id. The conversation may not exist yet.
    Direct,
}

/// What the list is for this time.
///
/// One list rather than three panels, because the hard part -- finding the
/// thing by name -- is the same every time and only what happens afterwards
/// differs. A field per mode would be several booleans pretending to be a
/// state, and two of them true is a question nobody asked.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum Asking {
    /// Where am I going. The ordinary case.
    #[default]
    Jump,
    /// Where is this message going.
    Forward(String),
    /// Who should be in this channel.
    Add(String),
    /// Who this conversation should be with.
    ///
    /// The only question answered by more than one name: one person is a
    /// direct message and several are a group, and a reader choosing is not
    /// picking between two features -- they are picking who to talk to, and
    /// the number decides which kind of conversation that is.
    Start,
}

/// Mattermost holds a group conversation to eight people including the reader,
/// so seven others is the ceiling.
///
/// Held here rather than left to the server, whose refusal would arrive after
/// the reader had chosen.
pub const OTHERS: usize = 7;

impl Asking {
    /// What the box says, which is the only thing telling a reader which of
    /// the three they are answering.
    fn placeholder(&self) -> &'static str {
        match self {
            Asking::Jump => "Jump to…",
            Asking::Forward(_) => "Forward to…",
            Asking::Add(_) => "Add who…",
            Asking::Start => "Talk to who…",
        }
    }

    /// Whether a conversation the reader is already in is an answer.
    ///
    /// It is not, when the question is who to add or who to talk to: a channel
    /// is not somebody.
    fn wants_channels(&self) -> bool {
        !matches!(self, Asking::Add(_) | Asking::Start)
    }

    /// Whether the answer is more than one name.
    fn wants_several(&self) -> bool {
        matches!(self, Asking::Start)
    }
}

/// What the reader settled on.
///
/// Two shapes because one question takes several names and the rest take one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Chose {
    One(Match),
    /// Everybody picked, for a conversation that has not been started yet.
    These(Vec<Match>),
}

/// One match: what it names, how to say it, and what to do with it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Match {
    pub id: String,
    pub label: String,
    pub direct: bool,
    pub reach: Reach,
}

/// The quick switcher, open or shut.
pub struct Switcher {
    /// Which question this list is asking. Set through `instead`, which also
    /// changes what the box says: one list answering three questions has to
    /// say which one it is on.
    pub asking: Asking,
    pub open: bool,
    /// The query, in a real text field: it has a caret, a selection and
    /// clipboard, and writing a second lesser one would be a second thing to
    /// get wrong.
    pub query: Composer,
    found: Vec<Match>,
    /// Who has been picked, for the one question that takes more than one
    /// name. Empty for every other.
    picked: Vec<Match>,
    /// What the server suggested for the query last asked, which the reader is
    /// not already in. Kept apart from `found` so a slow answer never reorders
    /// the rows under a reader who is mid-keystroke.
    offered: Vec<Match>,
    /// The query `offered` belongs to, so a late answer for an older query is
    /// dropped rather than shown against a newer one.
    asked: String,
    /// Which row return would take.
    chosen: usize,
}

impl Default for Switcher {
    fn default() -> Self {
        Self::new()
    }
}

impl Switcher {
    pub fn new() -> Self {
        let mut query = Composer::new(NAME).plain();
        query.placeholder = "Jump to…".to_string();
        Self {
            open: false,
            asking: Asking::Jump,
            query,
            found: Vec::new(),
            picked: Vec::new(),
            offered: Vec::new(),
            asked: String::new(),
            chosen: 0,
        }
    }

    /// Opens it empty, which is what a reader expects the second time as much
    /// as the first: the last search is not this one.
    pub fn show(&mut self, fonts: &mut Fonts, input: &mut Input) {
        self.open = true;
        // Forgotten here rather than on close, so a switcher opened the
        // ordinary way after a forward is the ordinary switcher again.
        self.asking = Asking::Jump;
        self.query.placeholder = Asking::Jump.placeholder().to_string();
        self.offered.clear();
        self.asked.clear();
        self.query.clear(fonts);
        self.chosen = 0;
        self.found.clear();
        self.picked.clear();
        input.focus_on(NAME);
    }

    /// Who has been picked so far, in the order they were.
    pub fn picked(&self) -> &[Match] {
        &self.picked
    }

    /// Turns an open switcher onto a different question.
    pub fn instead(&mut self, asking: Asking) {
        self.query.placeholder = asking.placeholder().to_string();
        self.asking = asking;
    }

    pub fn hide(&mut self, input: &mut Input) {
        self.open = false;
        self.asking = Asking::Jump;
        self.picked.clear();
        if input.focus() == Some(NAME) {
            input.focus_on(crate::composer::NAME);
        }
    }

    /// The line naming who has been picked, when there is one.
    fn picked_height(&self) -> f32 {
        if self.picked.is_empty() { 0.0 } else { ROW }
    }

    /// The panel, centred across the top where the eye already is.
    pub fn rect(&self, within: Rect) -> Rect {
        let height = PADDING * 2.0
            + self.query.height()
            + self.picked_height()
            + self.found.len() as f32 * ROW;
        let width = WIDTH.min(within.width - 40.0);
        Rect::new(
            within.x + (within.width - width) / 2.0,
            within.y + 80.0,
            width,
            height,
        )
    }

    /// Narrows the list to what the query matches.
    ///
    /// Scored rather than filtered by prefix: a reader typing "dev" means the
    /// channel called Dev, and one typing "curdev" means the same one -- which
    /// is exactly what a fuzzy score answers and a prefix does not.
    pub fn narrow(&mut self, entries: &[Entry]) {
        let query = self.query.text();
        let query = query.trim();
        let mut scored: Vec<(i32, Match)> = entries
            .iter()
            .filter(|_| self.asking.wants_channels())
            .filter_map(|entry| match entry {
                Entry::Channel {
                    id, label, direct, ..
                } => {
                    // An empty query lists the channels in the order the
                    // sidebar already put them, which is the reader's own.
                    let score = if query.is_empty() {
                        0
                    } else {
                        matterless_core::fuzzy::best_score([label.as_str()], query)?
                    };
                    Some((
                        score,
                        Match {
                            id: id.clone(),
                            label: label.clone(),
                            direct: *direct,
                            reach: Reach::Open,
                        },
                    ))
                }
                // Only a conversation is somewhere to go. A heading, a team
                // and the reader's own name are not.
                Entry::Heading { .. }
                | Entry::Team { .. }
                | Entry::Me { .. }
                // Not a conversation to jump to: it is every conversation at once.
                | Entry::Threads { .. } => None,
            })
            .collect();
        // Stable, so equal scores keep the sidebar's order rather than
        // shuffling as the reader types.
        scored.sort_by_key(|(score, _)| std::cmp::Reverse(*score));
        self.found = scored.into_iter().take(ROWS).map(|(_, one)| one).collect();
        // What the reader is already in comes first, always. Somewhere they
        // visit every day must never be pushed down the list by a channel they
        // have never opened.
        let held: std::collections::HashSet<&str> =
            self.found.iter().map(|one| one.id.as_str()).collect();
        let room = ROWS.saturating_sub(self.found.len());
        let extra: Vec<Match> = self
            .offered
            .iter()
            .filter(|one| !held.contains(one.id.as_str()))
            .take(room)
            .cloned()
            .collect();
        self.found.extend(extra);
        // Nothing typed, nothing to pick. The other questions list what the
        // reader is already in, which needs no query; this one lists people,
        // which only the server can find -- so an empty field here would show
        // whoever the *last* query turned up. It also makes return mean one
        // thing: with a name under the cursor it adds, and with none it opens.
        if self.asking.wants_several() && query.is_empty() {
            self.found.clear();
        }
        // Somebody already picked is not somebody to pick. Offering them again
        // is a row that does nothing, and it is the row under the cursor:
        // return would land on it rather than on the next name.
        if !self.picked.is_empty() {
            self.found
                .retain(|one| !self.picked.iter().any(|held| held.id == one.id));
        }
        self.chosen = self.chosen.min(self.found.len().saturating_sub(1));
    }

    /// Applies a frame's input. Answers the channel the reader chose.
    pub fn react(
        &mut self,
        fonts: &mut Fonts,
        input: &Input,
        within: Rect,
        clipboard: &mut String,
        entries: &[Entry],
    ) -> Option<Chose> {
        if !self.open {
            return None;
        }
        // A name already picked comes off with the same key that would have
        // deleted a letter, which is what a reader expects of a field full of
        // names and is what the app's chips do.
        if self.asking.wants_several()
            && input.struck(Key::Backspace)
            && self.query.text().is_empty()
        {
            self.picked.pop();
        }
        // Moving the highlight before the field sees the keys: on one line a
        // caret has nowhere to go up or down to, so nothing is taken away.
        if input.struck(Key::Down) && !self.found.is_empty() {
            self.chosen = (self.chosen + 1).min(self.found.len() - 1);
        }
        if input.struck(Key::Up) {
            self.chosen = self.chosen.saturating_sub(1);
        }
        let panel = self.rect(within);
        let field = Rect::new(panel.x, panel.y, panel.width, self.query.height());
        // What the cursor is on, read before the field is given the frame:
        // return is how the field reports, and reporting clears it -- so by
        // the time it has, the list has already narrowed to nothing typed.
        let aiming = self.found.get(self.chosen).cloned();
        // Return arrives as the field reporting a message, which is the same
        // key meaning the same thing: this is the one I want.
        let entered = self
            .query
            .react(fonts, input, field, clipboard)
            .is_some_and(|text| !text.is_empty());
        self.query.lay_out(fonts, panel.width);
        self.narrow(entries);
        if !self.asking.wants_several() {
            return entered.then_some(aiming).flatten().map(Chose::One);
        }
        // The key itself rather than the field's report of it. A field with
        // nothing typed in it reports nothing -- there is no message to send --
        // and on this question an empty field is exactly where return means
        // something: "these are all of them".
        if !input.struck(Key::Enter) {
            return None;
        }
        // Return on an empty field is "these are all of them". On a name it is
        // "and this one too", so the same key both adds and finishes -- which
        // it can, because the field says which it will do: there is nothing
        // typed to add.
        match aiming {
            Some(one) if self.picked.len() < OTHERS => {
                if !self.picked.iter().any(|held| held.id == one.id) {
                    self.picked.push(one);
                }
                self.query.clear(fonts);
                self.asked.clear();
                self.offered.clear();
                self.chosen = 0;
                None
            }
            _ if !self.picked.is_empty() => Some(Chose::These(self.picked.clone())),
            _ => None,
        }
    }

    /// What the server has not been asked about yet.
    ///
    /// `None` while the query is unchanged or too short to be worth a request:
    /// one or two letters match half a team, and the reader is still typing.
    pub fn to_ask(&mut self) -> Option<String> {
        let query = self.query.text().trim().to_string();
        if query.len() < LEAST || query == self.asked {
            return None;
        }
        self.asked = query.clone();
        Some(query)
    }

    /// Takes what the server suggested, if it is still the question being
    /// asked: a slow answer to an older query would reorder the list under a
    /// reader who has since typed more.
    pub fn offer(&mut self, query: &str, offered: Vec<Match>) {
        if query == self.asked {
            self.offered = offered;
        }
    }

    pub fn draw(&self, into: &mut Canvas<'_>, within: Rect) {
        if !self.open {
            return;
        }
        let panel = self.rect(within);
        let Canvas {
            scene,
            painter,
            fonts,
            palette,
        } = into;
        // A panel over the conversation rather than beside it: this is a thing
        // the reader is doing instead of reading, not as well as.
        // The window behind it, dimmed. `rgb(0 0 0 / 0.4)` over everything:
        // the switcher is a thing the reader is doing instead of reading, and
        // a panel floating over a fully lit window does not say that.
        scene.fill(
            within.x,
            within.y,
            within.width,
            within.height,
            [0, 0, 0, 102],
        );
        // Through the widget rather than by hand, which is what gives it the
        // hairline. Drawn as a shadow and a fill alone, a panel has no edge at
        // all: the shadow fades into the surface with nothing to fade away
        // *from*, and what says a popup is over the window rather than part of
        // it is that hard boundary, not the darkness under it.
        Panel::floating(panel, PANEL, 12.0)
            .edge(palette.rule)
            .fill(palette.surface)
            .draw(scene);
        // Who is already in it, above the list they were picked from, so the
        // reader can see what Return will open.
        if !self.picked.is_empty() {
            let names: Vec<&str> = self.picked.iter().map(|one| one.label.as_str()).collect();
            let said = format!(
                "{} \u{2014} return to open, backspace to take one off",
                names.join(", ")
            );
            let glyphs = painter.run(
                fonts,
                &said,
                panel.x + PADDING + 4.0,
                panel.y + self.query.height() + 4.0,
                Run::label(panel.width - PADDING * 2.0),
            );
            scene.glyphs(glyphs, palette.signal, palette.faint);
        }
        for (at, one) in self.found.iter().enumerate() {
            let y =
                panel.y + PADDING + self.query.height() + self.picked_height() + at as f32 * ROW;
            if at == self.chosen {
                scene.fill(panel.x, y, panel.width, ROW, palette.ground);
            }
            let glyphs = painter.run(
                fonts,
                &format!("{} {}", if one.direct { "@" } else { "#" }, one.label),
                panel.x + PADDING + 4.0,
                y + 6.0,
                Run::label(f32::MAX),
            );
            scene.glyphs(glyphs, palette.ink, palette.faint);
        }
    }

    /// The field the query is typed into, for the caller to draw.
    pub fn field(&self, within: Rect) -> Rect {
        let panel = self.rect(within);
        Rect::new(panel.x, panel.y, panel.width, self.query.height())
    }
}

#[cfg(test)]
mod tests {
    /// What the reader is already in comes first, always. Somewhere they visit
    /// every day must never be pushed down the list by a channel they have
    /// never opened.
    #[test]
    fn the_conversations_already_held_lead() {
        use super::*;
        let mut switcher = Switcher::new();
        switcher.offered = vec![Match {
            id: "far".into(),
            label: "dev far away -- join".into(),
            direct: false,
            reach: Reach::Join,
        }];
        switcher.narrow(&[Entry::Channel {
            id: "near".into(),
            label: "dev".into(),
            unread: 0,
            mentions: 0,
            muted: false,
            direct: false,
            private: false,
            counterpart: None,
            counterpart_avatar_at: 0,
        }]);
        let order: Vec<&str> = switcher.found.iter().map(|one| one.id.as_str()).collect();
        assert_eq!(order, vec!["near", "far"]);
    }

    /// A slow answer to an older query must not reorder the list under a
    /// reader who has since typed more.
    #[test]
    fn a_late_answer_to_an_older_question_is_dropped() {
        use super::*;
        let mut switcher = Switcher::new();
        switcher.asked = "curio".into();
        let stale = vec![Match {
            id: "x".into(),
            label: "something".into(),
            direct: false,
            reach: Reach::Join,
        }];
        switcher.offer("cur", stale.clone());
        assert!(switcher.offered.is_empty());
        switcher.offer("curio", stale);
        assert_eq!(switcher.offered.len(), 1);
    }

    /// One or two letters match half a team, and the reader is still typing.
    #[test]
    fn the_server_is_not_asked_about_a_letter_or_two() {
        use super::*;
        let mut fonts = Fonts::new();
        let mut switcher = Switcher::new();
        switcher.query.fill("c", &mut fonts);
        assert_eq!(switcher.to_ask(), None);
        switcher.query.fill("cur", &mut fonts);
        assert_eq!(switcher.to_ask().as_deref(), Some("cur"));
        // And not twice for the same question.
        assert_eq!(switcher.to_ask(), None);
    }

    /// One list answers three questions, and it has to say which one it is on
    /// -- otherwise a reader adds somebody to a channel believing they are
    /// going to one.
    #[test]
    fn it_says_which_question_it_is_asking() {
        use super::*;
        let mut fonts = Fonts::new();
        let mut input = Input::default();
        let mut switcher = Switcher::new();

        switcher.show(&mut fonts, &mut input);
        assert_eq!(switcher.asking, Asking::Jump);
        let jumping = switcher.query.placeholder.clone();

        for question in [Asking::Forward("p1".into()), Asking::Add("c1".into())] {
            switcher.instead(question.clone());
            assert_eq!(switcher.asking, question);
            assert_ne!(switcher.query.placeholder, jumping);
        }

        // Opened the ordinary way afterwards, it is the ordinary switcher
        // again rather than one still pointed at a message.
        switcher.show(&mut fonts, &mut input);
        assert_eq!(switcher.asking, Asking::Jump);
        assert_eq!(switcher.query.placeholder, jumping);
    }

    /// A channel is not somebody, so none is offered while the question is who
    /// to add -- otherwise the obvious thing to press is the wrong answer.
    #[test]
    fn a_conversation_is_no_answer_to_who_should_join() {
        use super::*;
        let mut switcher = Switcher::new();
        let dev = [Entry::Channel {
            id: "near".into(),
            label: "dev".into(),
            unread: 0,
            mentions: 0,
            muted: false,
            direct: false,
            private: false,
            counterpart: None,
            counterpart_avatar_at: 0,
        }];
        switcher.narrow(&dev);
        assert_eq!(switcher.found.len(), 1);

        switcher.instead(Asking::Add("c1".into()));
        switcher.narrow(&dev);
        assert!(switcher.found.is_empty());
    }

    use super::*;

    fn channels() -> Vec<Entry> {
        ["Curiosity | Dev", "Curiosity | Art", "Horde | Cooking"]
            .into_iter()
            .enumerate()
            .map(|(at, label)| Entry::Channel {
                id: format!("c{at}"),
                label: label.to_string(),
                unread: 0,
                mentions: 0,
                muted: false,
                direct: false,
                private: false,
                counterpart: None,
                counterpart_avatar_at: 0,
            })
            .collect()
    }

    fn typed(text: &str) -> Switcher {
        let mut fonts = Fonts::new();
        let mut switcher = Switcher::new();
        switcher.open = true;
        let mut input = Input::default();
        input.focus_on(NAME);
        input.apply(matterless_ui::input::Event::Typed(text.to_string()), &[]);
        switcher.query.lay_out(&mut fonts, WIDTH);
        switcher.query.react(
            &mut fonts,
            &input,
            Rect::new(0.0, 0.0, WIDTH, 60.0),
            &mut String::new(),
        );
        switcher.narrow(&channels());
        switcher
    }

    /// Letters scattered through the name, which is what a reader types when
    /// they know where they are going and not how it is spelled.
    #[test]
    fn a_query_matches_letters_that_are_not_adjacent() {
        let switcher = typed("curdev");
        assert_eq!(
            switcher.found.first().map(|one| one.label.as_str()),
            Some("Curiosity | Dev")
        );
    }

    #[test]
    fn an_empty_query_offers_the_sidebar_order() {
        let mut switcher = Switcher::new();
        switcher.narrow(&channels());
        assert_eq!(switcher.found.len(), 3);
        assert_eq!(switcher.found[0].label, "Curiosity | Dev");
    }

    /// Nothing matching is an empty list, not every channel: offering the whole
    /// list to somebody who typed a typo is worse than offering none.
    #[test]
    fn a_query_matching_nothing_offers_nothing() {
        let switcher = typed("zzzz");
        assert!(switcher.found.is_empty());
    }

    /// The highlight must never point past the end of a list that just shrank
    /// under it, or return would choose nothing.
    #[test]
    fn the_highlight_stays_inside_a_list_that_narrowed() {
        let mut switcher = Switcher::new();
        switcher.narrow(&channels());
        switcher.chosen = 2;
        switcher.query = Composer::new(NAME);
        switcher.narrow(&[]);
        assert_eq!(switcher.chosen, 0);
    }
}

#[cfg(test)]
mod starting {
    use super::*;
    use matterless_ui::input::Event;

    fn people(count: usize) -> Vec<Match> {
        (0..count)
            .map(|at| Match {
                id: format!("u{at}"),
                label: format!("person{at}"),
                direct: true,
                reach: Reach::Direct,
            })
            .collect()
    }

    fn panel() -> Rect {
        Rect::new(0.0, 0.0, 900.0, 600.0)
    }

    /// One frame: type nothing, press return, and see what the switcher makes
    /// of it. Driven through `react` rather than around it, because what is
    /// being tested is exactly what `react` decides.
    fn returned(switcher: &mut Switcher, fonts: &mut Fonts, typed: &str) -> Option<Chose> {
        // The people to pick from are offered by the server, not the sidebar,
        // so the entries this narrows against are empty.
        switcher.offered = people(3);
        // Typing and returning are two frames, as they are at a keyboard: the
        // field reads what was typed and what was struck from the same
        // snapshot, so a return in the frame that typed the name arrives
        // before the name does.
        if !typed.is_empty() {
            let mut typing = Input::default();
            typing.focus_on(NAME);
            typing.apply(Event::Typed(typed.to_string()), &[]);
            switcher.react(fonts, &typing, panel(), &mut String::new(), &[]);
        }
        let mut input = Input::default();
        input.focus_on(NAME);
        input.apply(
            Event::Key {
                key: Key::Enter,
                down: true,
            },
            &[],
        );
        switcher.react(fonts, &input, panel(), &mut String::new(), &[])
    }

    /// One person is a direct message and several are a group, and the reader
    /// is choosing who to talk to rather than between two features.
    ///
    /// Return both adds and finishes, which it can because the field says
    /// which it will do: with something typed there is a name to add, and with
    /// nothing typed there is not.
    #[test]
    fn several_people_can_be_picked_before_anything_opens() {
        let mut fonts = Fonts::new();
        let mut switcher = Switcher::new();
        switcher.show(&mut fonts, &mut Input::default());
        switcher.instead(Asking::Start);

        assert!(returned(&mut switcher, &mut fonts, "person").is_none());
        assert_eq!(switcher.picked().len(), 1, "the first name was taken");
        assert!(
            switcher.query.text().is_empty(),
            "the field is cleared for the next name"
        );

        assert!(returned(&mut switcher, &mut fonts, "person").is_none());
        assert_eq!(switcher.picked().len(), 2);

        // A field with nothing in it reports nothing, which is why the key is
        // read here rather than the field's account of it.
        let done = returned(&mut switcher, &mut fonts, "");
        match done {
            Some(Chose::These(these)) => assert_eq!(these.len(), 2),
            other => panic!("return on an empty field answered {other:?}"),
        }
    }

    /// Mattermost holds a group to eight including the reader, and the ceiling
    /// is held here rather than left to the server -- whose refusal would
    /// arrive after the reader had chosen.
    #[test]
    fn the_eighth_person_is_refused_here_rather_than_by_the_server() {
        let mut fonts = Fonts::new();
        let mut switcher = Switcher::new();
        switcher.show(&mut fonts, &mut Input::default());
        switcher.instead(Asking::Start);
        switcher.picked = people(OTHERS);

        let done = returned(&mut switcher, &mut fonts, "person");
        assert_eq!(switcher.picked().len(), OTHERS, "an eighth got in");
        assert!(
            matches!(done, Some(Chose::These(these)) if these.len() == OTHERS),
            "a full list should open rather than sit there refusing"
        );
    }

    /// Shutting it forgets who was picked: the next conversation started is a
    /// different one.
    #[test]
    fn closing_forgets_who_was_picked() {
        let mut fonts = Fonts::new();
        let mut input = Input::default();
        let mut switcher = Switcher::new();
        switcher.show(&mut fonts, &mut input);
        switcher.instead(Asking::Start);
        returned(&mut switcher, &mut fonts, "person");
        assert_eq!(switcher.picked().len(), 1);
        switcher.hide(&mut input);
        assert!(switcher.picked().is_empty());
        assert_eq!(switcher.asking, Asking::Jump);
    }

    /// A question that takes one name still answers with exactly one, and
    /// never collects.
    #[test]
    fn the_other_questions_answer_with_one_name() {
        let mut fonts = Fonts::new();
        let mut switcher = Switcher::new();
        switcher.show(&mut fonts, &mut Input::default());
        switcher.instead(Asking::Add("c1".into()));
        let done = returned(&mut switcher, &mut fonts, "person");
        assert!(
            matches!(done, Some(Chose::One(_))),
            "adding somebody answered {done:?}"
        );
        assert!(switcher.picked().is_empty());
    }

    /// Somewhere the reader already is, is not somebody to talk to.
    #[test]
    fn a_conversation_is_not_an_answer_to_who_to_talk_to() {
        assert!(!Asking::Start.wants_channels());
        assert!(Asking::Start.wants_several());
        assert!(!Asking::Jump.wants_several());
        assert!(!Asking::Add(String::new()).wants_several());
    }
}
