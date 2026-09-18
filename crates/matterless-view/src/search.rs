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

/// How many matches hang under the strip's field.
///
/// A handful. A droplist long enough to need scrolling is a panel, and the
/// panel is what Return is for.
const UNDER: usize = 6;

/// The gap between the field and the list under it, so the list reads as a
/// thing below the box rather than as part of it.
const DROP: f32 = 4.0;

/// How wide the droplist is, whatever the field it hangs from.
const LEAST_WIDE: f32 = 320.0;

/// Where the search is showing itself.
///
/// One state rather than two flags. A list under the strip and a pane down the
/// right are two answers to the same question, and both at once is a question
/// nobody asked.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Where {
    /// Shut.
    Away,
    /// Typing in the strip's own field, with the first few matches under it.
    ///
    /// Where a search starts. The reader clicked the box on the strip, and
    /// what they are after is usually in the first handful -- so finding it
    /// costs them no rearrangement of the window at all.
    Under,
    /// The pane down the right, which is where the whole list goes.
    Pane,
}

/// Where the search is on screen this frame.
///
/// The two rects travel together everywhere, and the box is not always in the
/// pane: it is on the strip while the list hangs under it, and in the pane's
/// header once the pane is open.
#[derive(Debug, Clone, Copy)]
pub struct Shown {
    /// The column down the right, whether or not it is showing.
    pub pane: Rect,
    /// The box the query is typed into.
    pub field: Rect,
}

/// What a frame of input did to the pane.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Did {
    /// Go to this message.
    Open(Box<Hit>),
    /// Show the rest of them, in the pane.
    Widen,
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
    showing: Where,
    pub query: Composer,
    found: Vec<Hit>,
    /// Which match the reader has walked to, and `None` until they walk.
    ///
    /// An `Option` rather than an index because under the strip the difference
    /// matters: Return with nothing picked out asks for the rest of the
    /// results, and Return on something picked out goes to it.
    chosen: Option<usize>,
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
            showing: Where::Away,
            query,
            found: Vec::new(),
            chosen: None,
            scroll: 0.0,
            bar: crate::scrollbar::Scrollbar::default(),
            asked: String::new(),
        }
    }

    /// Whether the pane is showing, which is what the column asks to know how
    /// much room the conversation has.
    pub fn open(&self) -> bool {
        self.showing == Where::Pane
    }

    /// Whether the strip's field is being typed into, with its list under it.
    pub fn asking(&self) -> bool {
        self.showing == Where::Under
    }

    /// Whether the search has the keyboard at all, either way.
    pub fn busy(&self) -> bool {
        self.showing != Where::Away
    }

    /// The box on the strip, clicked: the field takes the keyboard and what
    /// matches hangs under it.
    ///
    /// Nothing else moves. A reader looking for one message they can already
    /// half remember should not have the window rearranged around them to be
    /// shown it.
    pub fn peek(&mut self, fonts: &mut Fonts, input: &mut Input) {
        self.begin(Where::Under, fonts, input);
    }

    pub fn show(&mut self, fonts: &mut Fonts, input: &mut Input) {
        self.begin(Where::Pane, fonts, input);
    }

    fn begin(&mut self, showing: Where, fonts: &mut Fonts, input: &mut Input) {
        self.showing = showing;
        self.query.clear(fonts);
        self.found.clear();
        self.chosen = None;
        self.scroll = 0.0;
        self.asked.clear();
        input.focus_on(NAME);
    }

    /// Return, with nothing picked out of the list: the rest of them, in the
    /// pane.
    ///
    /// The query and what it found are kept. It is the same search shown with
    /// more room, so running it again would be work for an answer already in
    /// hand -- and would blink the results the reader is reading.
    pub fn widen(&mut self, input: &mut Input) {
        self.showing = Where::Pane;
        self.chosen = None;
        self.scroll = 0.0;
        input.focus_on(NAME);
    }

    pub fn hide(&mut self, input: &mut Input) {
        self.showing = Where::Away;
        if input.focus() == Some(NAME) {
            input.focus_on(crate::composer::NAME);
        }
    }

    /// How many matches this state puts within reach.
    fn shown(&self) -> usize {
        match self.showing {
            Where::Away => 0,
            Where::Under => self.found.len().min(UNDER),
            Where::Pane => self.found.len(),
        }
    }

    /// The list under the strip's field.
    ///
    /// `None` with nothing typed: a panel hanging under an empty box says the
    /// search is broken rather than that nothing has been asked yet. Once
    /// something is typed it is drawn even with nothing in it, because
    /// "nothing matched" is an answer and a vanishing panel is not.
    pub fn droplist(&self, field: Rect) -> Option<Rect> {
        if self.showing != Where::Under || self.query.text().trim().is_empty() {
            return None;
        }
        let rows = self.shown().max(1);
        Some(Rect::new(
            field.x,
            field.bottom() + DROP,
            field.width.max(LEAST_WIDE),
            rows as f32 * ROW + PADDING,
        ))
    }

    /// Where one row of that list sits.
    fn drop_row(&self, field: Rect, at: usize) -> Option<Rect> {
        let list = self.droplist(field)?;
        (at < self.shown()).then(|| {
            Rect::new(
                list.x,
                list.y + PADDING / 2.0 + at as f32 * ROW,
                list.width,
                ROW,
            )
        })
    }

    /// The box typed into when the strip has no room for one.
    ///
    /// The header's own field is the box normally: a reader clicks that one, so
    /// that is where the caret belongs. This is the fallback for a window too
    /// narrow to hold it, which `header::find` answers `None` for.
    pub fn in_pane(&self, pane: Rect) -> Rect {
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
    pub fn boxes(&self, shown: Shown) -> Vec<Placed> {
        let Shown { pane, field } = shown;
        match self.showing {
            Where::Away => return Vec::new(),
            Where::Under => {
                // The field itself, so a press in it can place the caret, and
                // a row each. Nothing else: the rest of the window is still
                // the reader's, which is the point of a droplist.
                let mut placed = vec![Placed {
                    name: NAME.to_string(),
                    rect: field,
                    depth: 9,
                }];
                if let Some(list) = self.droplist(field) {
                    placed.push(Placed {
                        name: named().of("list"),
                        rect: list,
                        depth: 9,
                    });
                    for at in 0..self.shown() {
                        if let Some(row) = self.drop_row(field, at) {
                            placed.push(named().at(&format!("under/{at}"), row, 10));
                        }
                    }
                }
                return placed;
            }
            Where::Pane => {}
        }
        let body = aside::body(pane);
        let mut placed = vec![
            Placed {
                name: NAME.to_string(),
                rect: pane,
                depth: 8,
            },
            // The field again, wherever it is. A press reaches the composer by
            // this name, and when the field sits up on the strip the pane
            // above does not cover it -- so without this the caret could not
            // be placed in the box the reader is typing into.
            Placed {
                name: NAME.to_string(),
                rect: field,
                depth: 9,
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
        // A new question, so whatever was picked out of the old answer is
        // not picked out of this one.
        self.chosen = None;
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
        if !self.open() {
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
        showing: Shown,
        clipboard: &mut String,
        store: &matterless_store::Store,
        me: &str,
    ) -> Option<Did> {
        if !self.busy() {
            return None;
        }
        let Shown { pane, field } = showing;
        let shown = self.shown();
        if input.struck(Key::Down) && shown > 0 {
            self.chosen = Some(match self.chosen {
                Some(at) => (at + 1).min(shown - 1),
                None => 0,
            });
        }
        // Up off the top lets go of the list rather than sticking on its first
        // row: under the strip, nothing picked out is what makes Return ask
        // for the rest of them.
        if input.struck(Key::Up) {
            self.chosen = match self.chosen {
                Some(0) | None => None,
                Some(at) => Some(at - 1),
            };
        }
        // Read before the box reacts, because a message box empties itself
        // when it is sent -- and a query is not a message. Return here asks
        // for more of the same answer rather than finishing with it, so the
        // question has to survive being asked.
        let asking = self.query.text();
        let entered = self
            .query
            .react(fonts, input, field, clipboard)
            .is_some_and(|text| !text.is_empty());
        if entered {
            self.query.fill(&asking, fonts);
        }
        self.query.lay_out(fonts, field.width);
        self.ask(store, me);
        // A query that answers with fewer results than the last one would
        // otherwise leave the reader scrolled past the end of the list.
        self.scroll = self.scroll.clamp(0.0, self.reach(pane));
        if let Some(slug) = input.clicked().and_then(|name| named().slug(name)) {
            if slug == "close" {
                return Some(Did::Close);
            }
            let row = slug
                .strip_prefix("under/")
                .unwrap_or(slug)
                .parse::<usize>()
                .ok();
            if let Some(at) = row {
                return self.found.get(at).cloned().map(Box::new).map(Did::Open);
            }
        }
        if entered {
            // Return under the strip asks for the rest of the results, which
            // is what the pane is. Unless the reader has walked into the list,
            // in which case they have already said which one they want.
            if self.showing == Where::Under && self.chosen.is_none() {
                return Some(Did::Widen);
            }
            return self
                .found
                .get(self.chosen.unwrap_or(0))
                .cloned()
                .map(Box::new)
                .map(Did::Open);
        }
        None
    }

    /// The list under the strip's field, drawn.
    ///
    /// Its own call rather than part of `draw`, because it belongs over the
    /// conversation rather than in the pane's column -- so the window draws it
    /// last, where nothing is clipped to a strip it hangs below.
    pub fn draw_droplist(&self, into: &mut Canvas<'_>, input: &Input, field: Rect) {
        let Some(list) = self.droplist(field) else {
            return;
        };
        let shown = self.shown();
        // Through the widget, which is what gives it the hairline a shadow
        // needs to fall away from.
        matterless_widgets::Panel::floating(list, 8.0, 12.0)
            .edge(into.palette.rule)
            .fill(into.palette.surface)
            .draw(into.scene);
        let Canvas {
            scene,
            painter,
            fonts,
            palette,
        } = into;

        if shown == 0 {
            let glyphs = painter.run(
                fonts,
                "nothing matched",
                list.x + PADDING,
                list.y + PADDING,
                Run::label(f32::MAX),
            );
            scene.glyphs(glyphs, palette.faint, palette.faint);
            return;
        }

        let room = list.width - PADDING * 2.0;
        for at in 0..shown {
            let Some(row) = self.drop_row(field, at) else {
                continue;
            };
            let hit = &self.found[at];
            if self.chosen == Some(at) || named().under(input, &format!("under/{at}")) {
                scene.fill(row.x, row.y, row.width, row.height, palette.ground);
            }
            let said = format!("{} \u{2014} {}", hit.channel, hit.author);
            let said = matterless_layout::elided(fonts, &said, room, crate::listing::label(true));
            let who = painter.run(
                fonts,
                &said,
                row.x + PADDING,
                row.y + 4.0,
                Run::label(f32::MAX).bold(),
            );
            scene.glyphs(who, palette.ink, palette.faint);
            let preview =
                matterless_layout::elided(fonts, &hit.preview, room, crate::listing::label(false));
            let what = painter.run(
                fonts,
                &preview,
                row.x + PADDING,
                row.y + 24.0,
                Run::label(f32::MAX),
            );
            scene.glyphs(what, palette.faint, palette.faint);
        }
    }

    pub fn draw(&self, into: &mut Canvas<'_>, input: &Input, pane: Rect) {
        if !self.open() {
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
            if self.chosen.unwrap_or(0) == at {
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

    /// The fallback field, for a window with no room on its strip, shares the
    /// pane's header with the way out.
    #[test]
    fn the_field_in_the_pane_shares_the_header_with_the_way_out() {
        let search = Search::new();
        let pane = pane();
        let field = search.in_pane(pane);
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
        search.showing = Where::Pane;
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

    /// The field can be somewhere else entirely -- the strip above the
    /// conversation, which is where a reader clicks -- so the press that puts
    /// the caret in it has to reach the composer by name from there. Without
    /// this box the pane placed only itself, and a click on the strip found
    /// nothing to focus.
    #[test]
    fn the_box_typed_into_is_placed_wherever_it_is() {
        let mut search = Search::new();
        search.showing = Where::Pane;
        let pane = pane();
        // Well clear of the pane, as the strip's own field is.
        let field = Rect::new(20.0, 4.0, 220.0, 26.0);
        let placed = search.boxes(Shown { pane, field });
        let over = placed
            .iter()
            .filter(|one| one.name == NAME)
            .find(|one| one.rect.x == field.x && one.rect.y == field.y)
            .expect("the field is reachable where it was drawn");
        assert_eq!(over.rect.width, field.width);
    }

    /// A search that is already holding an answer, so a test can press Return
    /// at it without a store full of messages behind it.
    fn holding(showing: Where, typed: &str, count: usize, fonts: &mut Fonts) -> Search {
        let mut search = Search::new();
        search.showing = showing;
        search.query.fill(typed, fonts);
        // What the results belong to, so running the query again is a no-op
        // and the hits below survive the frame.
        search.asked = typed.to_string();
        search.found = hits(count);
        search
    }

    fn struck(key: Key) -> Input {
        let mut input = Input::default();
        input.focus_on(NAME);
        input.apply(matterless_ui::input::Event::Key { key, down: true }, &[]);
        input
    }

    /// Clicking the box on the strip does not open the pane. The window stays
    /// as it was and what matched hangs under the box -- which is the whole
    /// point: somebody after one message should not have the conversation
    /// squeezed into two thirds of the width to be shown it.
    #[test]
    fn the_box_on_the_strip_hangs_a_list_rather_than_opening_the_pane() {
        let mut fonts = Fonts::new();
        let mut search = Search::new();
        let mut input = Input::default();
        search.peek(&mut fonts, &mut input);

        assert!(search.asking(), "the list is what opened");
        assert!(!search.open(), "and the pane is still shut");
        assert!(search.busy(), "but the keyboard is the search's");
        assert_eq!(input.focus(), Some(NAME));
    }

    /// Return, with nothing picked out of the list, asks for the rest of them.
    #[test]
    fn return_under_the_strip_asks_for_the_pane() {
        let mut fonts = Fonts::new();
        let store = matterless_store::Store::open_in_memory().expect("a store");
        let mut search = holding(Where::Under, "boop", 3, &mut fonts);
        let field = Rect::new(500.0, 6.0, 240.0, 28.0);

        let did = search.react(
            &mut fonts,
            &struck(Key::Enter),
            Shown {
                pane: pane(),
                field,
            },
            &mut String::new(),
            &store,
            "",
        );
        assert_eq!(did, Some(Did::Widen));

        // And widening keeps the answer rather than asking it again: it is the
        // same search with more room, and re-running it would blink the
        // results out from under the reader.
        let mut input = Input::default();
        search.widen(&mut input);
        assert!(search.open());
        assert!(!search.asking());
        assert_eq!(search.found.len(), 3, "the answer is kept");
        assert_eq!(search.query.text(), "boop", "and so is the question");
    }

    /// Walked into the list first, Return goes to what was walked to -- the
    /// reader has already said which one they want.
    #[test]
    fn return_on_something_picked_out_goes_to_it() {
        let mut fonts = Fonts::new();
        let store = matterless_store::Store::open_in_memory().expect("a store");
        let mut search = holding(Where::Under, "boop", 3, &mut fonts);
        let field = Rect::new(500.0, 6.0, 240.0, 28.0);
        let mut ask = |search: &mut Search, key| {
            search.react(
                &mut fonts,
                &struck(key),
                Shown {
                    pane: pane(),
                    field,
                },
                &mut String::new(),
                &store,
                "",
            )
        };

        assert_eq!(ask(&mut search, Key::Down), None, "down only moves");
        assert_eq!(ask(&mut search, Key::Down), None);
        let did = ask(&mut search, Key::Enter);
        let Some(Did::Open(hit)) = did else {
            panic!("the second one, not the pane: {did:?}")
        };
        assert_eq!(hit.post_id, "p1");
    }

    /// Up off the top of the list lets go of it rather than sticking on the
    /// first row, because nothing picked out is what makes Return ask for the
    /// pane -- so a reader who walked in must be able to walk back out.
    #[test]
    fn walking_back_off_the_top_of_the_list_lets_go_of_it() {
        let mut fonts = Fonts::new();
        let store = matterless_store::Store::open_in_memory().expect("a store");
        let mut search = holding(Where::Under, "boop", 3, &mut fonts);
        let field = Rect::new(500.0, 6.0, 240.0, 28.0);
        let mut ask = |search: &mut Search, key| {
            search.react(
                &mut fonts,
                &struck(key),
                Shown {
                    pane: pane(),
                    field,
                },
                &mut String::new(),
                &store,
                "",
            )
        };

        ask(&mut search, Key::Down);
        ask(&mut search, Key::Up);
        assert_eq!(ask(&mut search, Key::Enter), Some(Did::Widen));
    }

    /// The list is a handful, however many matched. More than that is a panel,
    /// and the panel is what Return is for.
    #[test]
    fn the_list_under_the_strip_is_a_handful() {
        let mut fonts = Fonts::new();
        let search = holding(Where::Under, "boop", HITS as usize, &mut fonts);
        let field = Rect::new(500.0, 6.0, 240.0, 28.0);

        let list = search.droplist(field).expect("a list");
        assert_eq!(search.shown(), UNDER);
        assert!(
            list.height <= UNDER as f32 * ROW + PADDING,
            "forty hits would be a window, not a droplist"
        );
        assert!(list.y >= field.bottom(), "under the box, not over it");
        assert!(search.drop_row(field, UNDER).is_none(), "and no more rows");
    }

    /// Nothing typed hangs nothing. A panel under an empty box says the search
    /// is broken rather than that nobody has asked anything yet.
    #[test]
    fn an_empty_question_hangs_no_list() {
        let mut fonts = Fonts::new();
        let mut search = Search::new();
        let mut input = Input::default();
        search.peek(&mut fonts, &mut input);
        let field = Rect::new(500.0, 6.0, 240.0, 28.0);
        assert!(search.droplist(field).is_none());

        // Typed, and nothing matched: that *is* an answer, so it is drawn.
        let asked = holding(Where::Under, "zzzz", 0, &mut fonts);
        assert!(asked.droplist(field).is_some());
    }

    /// A closed panel answers nothing and places nothing, whatever is typed.
    #[test]
    fn a_closed_search_answers_nothing() {
        let mut fonts = Fonts::new();
        let mut search = Search::new();
        let store = matterless_store::Store::open_in_memory().expect("a store");
        let showing = Shown {
            pane: pane(),
            field: search.in_pane(pane()),
        };
        let chosen = search.react(
            &mut fonts,
            &Input::default(),
            showing,
            &mut String::new(),
            &store,
            "",
        );
        assert!(chosen.is_none());
        assert!(search.boxes(showing).is_empty());
    }
}
