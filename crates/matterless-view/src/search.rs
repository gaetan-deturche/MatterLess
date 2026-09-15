//! Finding a message that has already been said.
//!
//! Answered from the local store rather than the server: this deployment has no
//! Elasticsearch, so the database beats the network for everything recent -- and
//! everything recent is what a reader is usually looking for.
//!
//! The modifiers come free. `from:` and `in:` and the date ones are parsed by
//! the same `SearchQuery` the app uses, so `from:ada limit` means here exactly
//! what it means there.
//!
//! Drawn down the right-hand column beside the conversation rather than over
//! it -- see `aside`. A result is a line out of context, and the context is
//! the channel it is read against.

use crate::composer::Composer;
use matterless_layout::Fonts;
use matterless_paint::Run;
use matterless_ui::input::{Input, Key};
use matterless_ui::{Placed, Rect};
use matterless_widgets::{Canvas, Named};

/// What the box answers to.
pub const NAME: &str = "search";
/// What this widget's hit boxes are called. The join and its inverse in one
/// place, so the box it registers and the press it answers cannot disagree.
fn named() -> Named {
    Named::new(NAME)
}

/// How many hits are shown. A reader who wants more should say more.
const HITS: u32 = 40;
const ROW: f32 = 46.0;
use crate::aside;

const PADDING: f32 = aside::PADDING;

/// What a frame of input did to the pane.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Did {
    /// Go to this message.
    Open(Box<Hit>),
    /// Shut the pane.
    Close,
}

/// One message that matched.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hit {
    pub post_id: String,
    pub channel_id: String,
    /// Who said it, resolved to a name where the store knows one.
    pub author: String,
    /// Where it was said, named the way the sidebar names it.
    ///
    /// The app's result rows lead with the channel in bold, and they have to:
    /// a search runs across every conversation, so a line with only an author
    /// on it does not say where the reader would be going.
    pub channel: String,
    /// One line of it, which is all a result row has room for.
    pub preview: String,
}

/// The search panel, open or shut.
pub struct Search {
    pub open: bool,
    pub query: Composer,
    found: Vec<Hit>,
    chosen: usize,
    scroll: f32,
    bar: crate::scrollbar::Scrollbar,
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
        let mut query = Composer::new(NAME).plain();
        query.placeholder = "Search messages".to_string();
        Self {
            open: false,
            query,
            found: Vec::new(),
            chosen: 0,
            scroll: 0.0,
            bar: crate::scrollbar::Scrollbar::default(),
            asked: String::new(),
        }
    }

    pub fn show(&mut self, fonts: &mut Fonts, input: &mut Input) {
        self.open = true;
        self.query.clear(fonts);
        self.found.clear();
        self.chosen = 0;
        self.scroll = 0.0;
        self.asked.clear();
        input.focus_on(NAME);
    }

    pub fn hide(&mut self, input: &mut Input) {
        self.open = false;
        if input.focus() == Some(NAME) {
            input.focus_on(crate::composer::NAME);
        }
    }

    /// The box typed into, which is what the pane's header holds.
    pub fn field(&self, pane: Rect) -> Rect {
        let head = aside::header(pane);
        let height = self.query.height();
        Rect::new(
            head.x + PADDING,
            head.y + (head.height - height) / 2.0,
            // Room for the way out beside it, which the title panes give to
            // their titles.
            (head.width - PADDING * 2.0 - aside::CLOSE - 6.0).max(0.0),
            height,
        )
    }

    /// How far the results can travel: what they occupy, less the room there is.
    fn reach(&self, pane: Rect) -> f32 {
        let wanted = self.found.len() as f32 * ROW + PADDING * 2.0;
        (wanted - aside::body(pane).height).max(0.0)
    }

    /// Where one result sits, wherever the list has been scrolled to.
    fn row_rect(&self, pane: Rect, at: usize) -> Rect {
        let body = aside::body(pane);
        Rect::new(
            body.x,
            body.y + PADDING + at as f32 * ROW - self.scroll,
            body.width,
            ROW,
        )
    }

    /// Where each result sits, so a click can land on one.
    pub fn boxes(&self, pane: Rect) -> Vec<Placed> {
        if !self.open {
            return Vec::new();
        }
        let body = aside::body(pane);
        let mut placed = vec![
            Placed {
                name: NAME.to_string(),
                rect: pane,
                depth: 8,
            },
            aside::close_box(pane, NAME),
        ];
        placed.extend(self.bar.boxes(NAME, body, self.reach(pane)));
        for at in 0..self.found.len() {
            let row = self.row_rect(pane, at);
            if row.bottom() <= body.y || row.y >= body.bottom() {
                continue;
            }
            placed.push(named().at(&at.to_string(), row, 9));
        }
        placed
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
        // Names and conversation labels in one place, because a direct
        // message has no display name of its own: labelling one means looking
        // up the other person, and `found_for` is where that already happens.
        self.found = crate::listing::found_for(store, posts, me)
            .into_iter()
            .map(|found| Hit {
                author: found.author,
                channel: found.channel,
                preview: found.preview,
                post_id: found.post_id,
                channel_id: found.channel_id,
            })
            .collect();
    }

    /// What a turn of the wheel or a drag of the bar does to the results.
    ///
    /// Apart from `react` because the pointer needs the frame's boxes to say
    /// which panel it is over, and nothing else about answering a query does.
    pub fn scrolled(&mut self, input: &Input, placed: &[Placed], pane: Rect) {
        if !self.open {
            return;
        }
        let body = aside::body(pane);
        let reach = self.reach(pane);
        // The bar first: while it is held nothing else may write the scroll.
        if let Some(scroll) = self.bar.react(NAME, input, body, self.scroll, reach) {
            self.scroll = scroll.clamp(0.0, reach);
            return;
        }
        if let Some((_, y)) = input.wheel_over(placed, |name| name == NAME) {
            self.scroll = (self.scroll - y).clamp(0.0, reach);
        }
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
    ) -> Option<Did> {
        if !self.open {
            return None;
        }
        let pane = within;
        if input.struck(Key::Down) && !self.found.is_empty() {
            self.chosen = (self.chosen + 1).min(self.found.len() - 1);
        }
        if input.struck(Key::Up) {
            self.chosen = self.chosen.saturating_sub(1);
        }
        let field = self.field(pane);
        let entered = self
            .query
            .react(fonts, input, field, clipboard)
            .is_some_and(|text| !text.is_empty());
        self.query.lay_out(fonts, field.width);
        self.ask(store, me);
        // A query that answers with fewer results than the last one would
        // otherwise leave the reader scrolled past the end of the list.
        self.scroll = self.scroll.clamp(0.0, self.reach(pane));
        if let Some(slug) = input.clicked().and_then(|name| named().slug(name)) {
            if slug == "close" {
                return Some(Did::Close);
            }
            if let Ok(at) = slug.parse::<usize>() {
                return self.found.get(at).cloned().map(Box::new).map(Did::Open);
            }
        }
        if entered {
            return self
                .found
                .get(self.chosen)
                .cloned()
                .map(Box::new)
                .map(Did::Open);
        }
        None
    }

    pub fn draw(&self, into: &mut Canvas<'_>, input: &Input, pane: Rect) {
        if !self.open {
            return;
        }
        let body = aside::body(pane);
        let reach = self.reach(pane);
        aside::ground(into, pane);
        aside::draw_close(into, pane);
        let Canvas {
            scene,
            painter,
            fonts,
            palette,
        } = into;

        // Nothing typed yet is not the same as nothing found, and a blank pane
        // says neither.
        if self.found.is_empty() {
            let said = if self.query.text().trim().is_empty() {
                "type to search this machine's copy"
            } else {
                "nothing matched"
            };
            let glyphs = painter.run(
                fonts,
                said,
                body.x + PADDING,
                body.y + PADDING,
                Run::label(f32::MAX),
            );
            scene.glyphs(glyphs, palette.faint, palette.faint);
            return;
        }

        // Clipped to the body, so a result scrolled halfway off the top is cut
        // at the field rather than drawn across it.
        scene.clip_to(body.x, body.y, body.width, body.height);
        let room = body.width - PADDING * 2.0 - crate::scrollbar::TRACK;
        for (at, hit) in self.found.iter().enumerate() {
            let row = self.row_rect(pane, at);
            if row.bottom() <= body.y || row.y >= body.bottom() {
                continue;
            }
            if at == self.chosen {
                scene.fill(row.x, row.y, row.width, row.height, palette.ground);
            }
            // Where it was said leads, as it does in the app: a search runs
            // across every conversation, so the channel is what tells the
            // reader where following this result would take them.
            let said = format!("{} \u{2014} {}", hit.channel, hit.author);
            let said = matterless_layout::elided(fonts, &said, room, crate::listing::label(true));
            let who = painter.run(
                fonts,
                &said,
                row.x + PADDING,
                row.y + 6.0,
                Run::label(f32::MAX).bold(),
            );
            scene.glyphs(who, palette.ink, palette.faint);
            let preview =
                matterless_layout::elided(fonts, &hit.preview, room, crate::listing::label(false));
            let what = painter.run(
                fonts,
                &preview,
                row.x + PADDING,
                row.y + 26.0,
                Run::label(f32::MAX),
            );
            scene.glyphs(what, palette.faint, palette.faint);
        }
        let mut canvas = Canvas {
            scene,
            painter,
            fonts,
            palette,
        };
        self.bar
            .draw(&mut canvas, NAME, input, body, self.scroll, reach);
        // Back to the whole window, so what is drawn after this is not clipped
        // to a pane it has nothing to do with.
        canvas.scene.clip_to(0.0, 0.0, f32::MAX, f32::MAX);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pane() -> Rect {
        aside::rect(Rect::new(300.0, 0.0, 1000.0, 700.0))
    }

    fn hits(count: usize) -> Vec<Hit> {
        (0..count)
            .map(|at| Hit {
                post_id: format!("p{at}"),
                channel_id: "c1".into(),
                author: "ada".into(),
                channel: "Dev".into(),
                preview: "something".into(),
            })
            .collect()
    }

    /// The field is in the header, with room left beside it for the way out.
    #[test]
    fn the_field_shares_the_header_with_the_way_out() {
        let search = Search::new();
        let pane = pane();
        let field = search.field(pane);
        assert!(field.y >= aside::header(pane).y);
        assert!(field.bottom() <= aside::header(pane).bottom());
        assert!(
            field.right() <= aside::close(pane).x,
            "the field runs under the close button"
        );
    }

    /// Every result is reachable: the list scrolls rather than stopping at
    /// whatever happened to fit on screen.
    #[test]
    fn every_result_can_be_scrolled_to() {
        let mut search = Search::new();
        search.open = true;
        search.found = hits(HITS as usize);
        let pane = pane();
        assert!(
            search.reach(pane) > 0.0,
            "forty hits do not fit in a column"
        );
        search.scroll = search.reach(pane);
        let last = search.row_rect(pane, HITS as usize - 1);
        assert!(
            last.bottom() <= pane.bottom() + 0.5 && last.y >= aside::body(pane).y,
            "the last hit sits at {last:?}"
        );
    }

    /// A closed panel answers nothing and places nothing, whatever is typed.
    #[test]
    fn a_closed_search_answers_nothing() {
        let mut fonts = Fonts::new();
        let mut search = Search::new();
        let store = matterless_store::Store::open_in_memory().expect("a store");
        let chosen = search.react(
            &mut fonts,
            &Input::default(),
            pane(),
            &mut String::new(),
            &store,
            "",
        );
        assert!(chosen.is_none());
        assert!(search.boxes(pane()).is_empty());
    }
}
