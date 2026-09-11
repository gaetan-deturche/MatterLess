//! Finding a message that has already been said.
//!
//! Answered from the local store rather than the server: this deployment has no
//! Elasticsearch, so the database beats the network for everything recent -- and
//! everything recent is what a reader is usually looking for.
//!
//! The modifiers come free. `from:` and `in:` and the date ones are parsed by
//! the same `SearchQuery` the app uses, so `from:ada limit` means here exactly
//! what it means there.

use crate::composer::Composer;
use crate::sidebar::Canvas;
use matterless_layout::Fonts;
use matterless_paint::Run;
use matterless_ui::Rect;
use matterless_ui::input::{Input, Key};

/// What the box answers to.
pub const NAME: &str = "search";

/// How many hits are shown. A reader who wants more should say more.
const HITS: u32 = 40;
const ROW: f32 = 46.0;
const PADDING: f32 = 10.0;
const WIDTH: f32 = 620.0;
/// What the stylesheet cuts a floating panel's corners by.
const PANEL: f32 = 8.0;

/// One message that matched.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hit {
    pub post_id: String,
    pub channel_id: String,
    /// Who said it, resolved to a name where the store knows one.
    pub author: String,
    /// One line of it, which is all a result row has room for.
    pub preview: String,
}

/// The search panel, open or shut.
pub struct Search {
    pub open: bool,
    pub query: Composer,
    found: Vec<Hit>,
    chosen: usize,
    /// The query the results belong to, so they are not rebuilt on every frame
    /// for a query that has not changed. Search reads the database; a hover
    /// should not.
    asked: String,
}

impl Default for Search {
    fn default() -> Self {
        Self::new()
    }
}

impl Search {
    pub fn new() -> Self {
        let mut query = Composer::new(NAME);
        query.placeholder = "Search messages".to_string();
        Self {
            open: false,
            query,
            found: Vec::new(),
            chosen: 0,
            asked: String::new(),
        }
    }

    pub fn show(&mut self, fonts: &mut Fonts, input: &mut Input) {
        self.open = true;
        self.query.clear(fonts);
        self.found.clear();
        self.chosen = 0;
        self.asked.clear();
        input.focus_on(NAME);
    }

    pub fn hide(&mut self, input: &mut Input) {
        self.open = false;
        if input.focus() == Some(NAME) {
            input.focus_on(crate::composer::NAME);
        }
    }

    pub fn rect(&self, within: Rect) -> Rect {
        let height = PADDING * 2.0 + self.query.height() + self.found.len() as f32 * ROW;
        let width = WIDTH.min(within.width - 40.0);
        Rect::new(
            within.x + (within.width - width) / 2.0,
            within.y + 60.0,
            width,
            height.min(within.height - 120.0),
        )
    }

    pub fn field(&self, within: Rect) -> Rect {
        let panel = self.rect(within);
        Rect::new(panel.x, panel.y, panel.width, self.query.height())
    }

    /// Runs the query, if it has changed since the last one.
    fn ask(&mut self, store: &matterless_store::Store, me: &str) {
        let typed = self.query.text().trim().to_string();
        if typed == self.asked {
            return;
        }
        self.asked = typed.clone();
        self.chosen = 0;
        if typed.is_empty() {
            self.found.clear();
            return;
        }
        // The reader's own offset is not known here, so dates are read in UTC.
        // Everything else -- the words, from:, in: -- is exact.
        let parsed = matterless_core::search::SearchQuery::parse(&typed, 0);
        let posts = store.search_query(&parsed, HITS).unwrap_or_default();
        let mut authors: Vec<String> = posts.iter().map(|post| post.user_id.clone()).collect();
        authors.sort();
        authors.dedup();
        let known = store.users_by_ids(&authors).unwrap_or_default();
        self.found = posts
            .into_iter()
            .map(|post| Hit {
                author: known
                    .get(&post.user_id)
                    .map(|user| user.username.clone())
                    // A name the store has never met is still an answer, and an
                    // id says more than a blank.
                    .unwrap_or_else(|| post.user_id.clone()),
                preview: matterless_sync::notify::preview_of(&post.message),
                post_id: post.id,
                channel_id: post.channel_id,
            })
            .collect();
        let _ = me;
    }

    /// Applies a frame's input. Answers the channel a hit was chosen in.
    pub fn react(
        &mut self,
        fonts: &mut Fonts,
        input: &Input,
        within: Rect,
        clipboard: &mut String,
        store: &matterless_store::Store,
        me: &str,
    ) -> Option<Hit> {
        if !self.open {
            return None;
        }
        if input.struck(Key::Down) && !self.found.is_empty() {
            self.chosen = (self.chosen + 1).min(self.found.len() - 1);
        }
        if input.struck(Key::Up) {
            self.chosen = self.chosen.saturating_sub(1);
        }
        let field = self.field(within);
        let entered = self
            .query
            .react(fonts, input, field, clipboard)
            .is_some_and(|text| !text.is_empty());
        self.query.lay_out(fonts, self.rect(within).width);
        self.ask(store, me);
        if entered {
            return self.found.get(self.chosen).cloned();
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
        scene.rounded(
            panel.x,
            panel.y,
            panel.width,
            panel.height,
            palette.surface,
            PANEL,
        );
        for (at, hit) in self.found.iter().enumerate() {
            let y = panel.y + PADDING + self.query.height() + at as f32 * ROW;
            // Only what fits: the panel is capped so a thousand hits do not
            // make a box taller than the window.
            if y + ROW > panel.bottom() {
                break;
            }
            if at == self.chosen {
                scene.fill(panel.x, y, panel.width, ROW, palette.ground);
            }
            let who = painter.run(
                fonts,
                &hit.author,
                panel.x + PADDING + 4.0,
                y + 4.0,
                Run::label(f32::MAX).bold(),
            );
            scene.glyphs(who, palette.ink, palette.faint);
            let what = painter.run(
                fonts,
                &hit.preview,
                panel.x + PADDING + 4.0,
                y + 22.0,
                // Cut by the panel rather than wrapped: a result is one line,
                // and a wrapped one would run into the row beneath it.
                Run::label(f32::MAX),
            );
            scene.glyphs(what, palette.faint, palette.faint);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The panel never grows past the window, however many hits there are.
    #[test]
    fn the_panel_stays_inside_the_window() {
        let mut search = Search::new();
        search.open = true;
        search.found = (0..500)
            .map(|at| Hit {
                post_id: format!("p{at}"),
                channel_id: "c1".into(),
                author: "ada".into(),
                preview: "something".into(),
            })
            .collect();
        let window = Rect::new(0.0, 0.0, 1000.0, 700.0);
        let panel = search.rect(window);
        assert!(panel.height <= window.height - 120.0);
        assert!(panel.bottom() <= window.bottom());
    }

    /// A closed panel answers nothing, whatever is typed at it.
    #[test]
    fn a_closed_search_answers_nothing() {
        let mut fonts = Fonts::new();
        let mut search = Search::new();
        let store = matterless_store::Store::open_in_memory().expect("a store");
        let chosen = search.react(
            &mut fonts,
            &Input::default(),
            Rect::new(0.0, 0.0, 800.0, 600.0),
            &mut String::new(),
            &store,
            "",
        );
        assert!(chosen.is_none());
    }
}
