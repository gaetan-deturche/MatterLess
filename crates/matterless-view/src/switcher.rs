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

/// What the box answers to when the pointer is tested against it.
pub const NAME: &str = "switcher";

/// How many matches are shown. Past this the reader should type another letter
/// rather than read a longer list.
const ROWS: usize = 8;
const ROW: f32 = 30.0;
const PADDING: f32 = 10.0;
const WIDTH: f32 = 520.0;

/// One match: the channel it names and how to say it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Match {
    pub id: String,
    pub label: String,
    pub direct: bool,
}

/// The quick switcher, open or shut.
pub struct Switcher {
    /// The message being forwarded, when it was opened to pick a destination
    /// rather than to go somewhere. `None` is the ordinary case.
    ///
    /// Set through `forward_instead`, which also changes what the box says: the
    /// same list answering two questions has to say which one it is asking.
    pub forwarding: Option<String>,
    pub open: bool,
    /// The query, in a real text field: it has a caret, a selection and
    /// clipboard, and writing a second lesser one would be a second thing to
    /// get wrong.
    pub query: Composer,
    found: Vec<Match>,
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
        let mut query = Composer::new(NAME);
        query.placeholder = "Jump to…".to_string();
        Self {
            open: false,
            forwarding: None,
            query,
            found: Vec::new(),
            chosen: 0,
        }
    }

    /// Opens it empty, which is what a reader expects the second time as much
    /// as the first: the last search is not this one.
    pub fn show(&mut self, fonts: &mut Fonts, input: &mut Input) {
        self.open = true;
        // Forgotten here rather than on close, so a switcher opened the
        // ordinary way after a forward is the ordinary switcher again.
        self.forwarding = None;
        self.query.placeholder = "Jump to…".to_string();
        self.query.clear(fonts);
        self.chosen = 0;
        self.found.clear();
        input.focus_on(NAME);
    }

    /// Turns an open switcher into "where should this go".
    pub fn forward_instead(&mut self, post_id: &str) {
        self.forwarding = Some(post_id.to_string());
        self.query.placeholder = "Forward to…".to_string();
    }

    pub fn hide(&mut self, input: &mut Input) {
        self.open = false;
        self.forwarding = None;
        if input.focus() == Some(NAME) {
            input.focus_on(crate::composer::NAME);
        }
    }

    /// The panel, centred across the top where the eye already is.
    pub fn rect(&self, within: Rect) -> Rect {
        let height = PADDING * 2.0 + self.query.height() + self.found.len() as f32 * ROW;
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
                        },
                    ))
                }
                Entry::Heading { .. } => None,
            })
            .collect();
        // Stable, so equal scores keep the sidebar's order rather than
        // shuffling as the reader types.
        scored.sort_by_key(|(score, _)| std::cmp::Reverse(*score));
        self.found = scored.into_iter().take(ROWS).map(|(_, one)| one).collect();
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
    ) -> Option<String> {
        if !self.open {
            return None;
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
        // Return arrives as the field reporting a message, which is the same
        // key meaning the same thing: this is the one I want.
        let entered = self
            .query
            .react(fonts, input, field, clipboard)
            .is_some_and(|text| !text.is_empty());
        self.query.lay_out(fonts, panel.width);
        self.narrow(entries);
        if entered {
            return self.found.get(self.chosen).map(|one| one.id.clone());
        }
        None
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
        scene.fill(panel.x, panel.y, panel.width, panel.height, palette.surface);
        for (at, one) in self.found.iter().enumerate() {
            let y = panel.y + PADDING + self.query.height() + at as f32 * ROW;
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
    /// The same list answers two questions, and it has to say which one it is
    /// asking -- otherwise a reader forwards a message believing they are
    /// changing channel.
    #[test]
    fn it_says_which_question_it_is_asking() {
        use super::*;
        let mut fonts = Fonts::new();
        let mut input = Input::default();
        let mut switcher = Switcher::new();

        switcher.show(&mut fonts, &mut input);
        assert_eq!(switcher.forwarding, None);
        let jumping = switcher.query.placeholder.clone();

        switcher.forward_instead("p1");
        assert_eq!(switcher.forwarding.as_deref(), Some("p1"));
        assert_ne!(switcher.query.placeholder, jumping);

        // Opened the ordinary way afterwards, it is the ordinary switcher
        // again rather than one still pointed at a message.
        switcher.show(&mut fonts, &mut input);
        assert_eq!(switcher.forwarding, None);
        assert_eq!(switcher.query.placeholder, jumping);
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
                counterpart: None,
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
