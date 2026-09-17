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
use matterless_layout::Fonts;
use matterless_paint::Run;
use matterless_ui::input::{Input, Key};
use matterless_ui::{Placed, Rect};
use matterless_widgets::{Canvas, Named, Panel};

pub const NAME: &str = "picker";
/// What this widget's hit boxes are called. The join and its inverse in one
/// place, so the box it registers and the press it answers cannot disagree.
fn named() -> Named {
    Named::new(NAME)
}

/// What the group tabs are called, which is a name of their own so a press on
/// one is not read as a press on a tile.
fn tabbed() -> Named {
    Named::new(TAB)
}
const TAB: &str = "picker/tab";

/// Whether a hit box belongs to the picker.
///
/// The panel, the field in it -- which carries the same name -- every tile,
/// every group tab, and the bar. Asked so that a press anywhere else can put
/// the picker away, which is what a popover opened from a message has instead
/// of a close button.
pub fn owns(name: &str) -> bool {
    name == NAME
        || named().slug(name).is_some()
        || tabbed().slug(name).is_some()
        || name == format!("{NAME}/scrollbar")
}

/// How many are offered. A grid rather than a column, because an emoji is a
/// picture and pictures read faster side by side.
const COLUMNS: usize = 8;
const ROWS: usize = 4;
/// How many rows deep the offered set is gathered, against the four on show.
///
/// A fuzzy search for one letter matches hundreds, and thirty-two of them was
/// the whole answer while the grid could not be scrolled. Forty rows is more
/// than anybody will walk and cheap to hold: a name and two empty options.
const DEEP: usize = 40;
const CELL: f32 = 34.0;
const PADDING: f32 = 8.0;
/// Between the button that opened it and the panel itself, so the two read as
/// one thing hanging off the other rather than as a join.
const GAP: f32 = 4.0;
/// The row of tabs between the field and the grid, which is how the official
/// client offers the groups: a mark each, and pressing one goes there.
const TABS: f32 = 30.0;
/// How an emoji is set here, on a tab and on a tile alike.
fn mark_run() -> Run {
    Run {
        size: 15.0,
        line_height: 18.0,
        bold: false,
        mono: false,
        wrap: f32::MAX,
        icon: false,
        smooth: false,
    }
}
/// What each group's tab shows. A representative rather than the first name in
/// it, which for `People` is a hand and says nothing about the group.
///
/// Falls back to the group's first emoji when a name here is not one the table
/// can draw, so a regenerated table can never leave a tab blank.
const MARKS: [(&str, &str); 9] = [
    ("Smileys", "smile"),
    ("People", "wave"),
    ("Nature", "deciduous_tree"),
    ("Food", "apple"),
    ("Activities", "basketball"),
    ("Travel", "rocket"),
    ("Objects", "bulb"),
    ("Symbols", "heart"),
    ("Flags", "checkered_flag"),
];
/// What the stylesheet cuts a floating panel's corners by.
const PANEL: f32 = 8.0;

/// One standard emoji, if the renderer can draw it.
///
/// A name with no character behind it would be an empty cell that does nothing
/// when pressed, which is worse than one fewer thing to press.
fn standard(name: &str) -> Option<Choice> {
    Some(Choice {
        name: name.to_string(),
        unicode: Some(matterless_render::emoji::character_for(name)?),
        id: None,
    })
}

/// What a group's tab shows: its representative, or its first if that name is
/// not one the table can draw.
fn mark_of(group: &str, names: &[&str]) -> Choice {
    MARKS
        .iter()
        .find(|(of, _)| *of == group)
        .and_then(|(_, mark)| standard(mark))
        .or_else(|| names.first().and_then(|name| standard(name)))
        .unwrap_or(Choice {
            name: group.to_string(),
            unicode: None,
            id: None,
        })
}

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
    /// Whether the press being answered is the one that opened it.
    fresh: bool,
    /// How far down the grid is, in pixels.
    ///
    /// Pixels rather than whole rows: a grid that jumped a row at a time was
    /// a grid that moved in thirty-four pixel steps, which against a
    /// conversation that scrolls smoothly reads as something stuttering.
    scroll: f32,
    /// The bar down the grid's right, which also says how much there is.
    bar: crate::scrollbar::Scrollbar,
    /// Where each group of the offered set begins, and what its tab shows.
    ///
    /// Empty while something is typed: a search has one order, by how well it
    /// matched, and groups would be saying there is another.
    tabs: Vec<(usize, Choice)>,
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
            fresh: false,
            scroll: 0.0,
            bar: crate::scrollbar::Scrollbar::default(),
            tabs: Vec::new(),
        }
    }

    /// Whether the press being answered is the one that opened it.
    ///
    /// Answers true once. The press that opens the picker arrives in the same
    /// frame the picker first reacts to, so without this the click that opened
    /// it would be read as a click outside it and shut it again -- which is
    /// exactly how the profile card came to be impossible to open.
    pub fn opening(&mut self) -> bool {
        std::mem::take(&mut self.fresh)
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
        self.fresh = true;
        self.scroll = 0.0;
        input.focus_on(NAME);
    }

    pub fn hide(&mut self, input: &mut Input) {
        self.for_post = None;
        self.fresh = false;
        if input.focus() == Some(NAME) {
            input.focus_on(crate::composer::NAME);
        }
    }

    /// Anchored below what opened it, and kept on screen.
    ///
    /// Below, or above when there is not the room -- never *over* it. Anchored
    /// to the button's top edge before, which put the grid across the toolbar
    /// it was opened from and across the row of quick faces beside it: the
    /// reader could no longer see the thing they had pressed.
    pub fn rect(&self, near: Rect, within: Rect) -> Rect {
        // Room for the bar beside the tiles rather than over them. The bar is
        // drawn down the grid's right-hand edge, and a grid exactly eight
        // cells wide put it across the eighth of them.
        let width = COLUMNS as f32 * CELL + PADDING * 2.0 + crate::scrollbar::TRACK;
        let height = ROWS as f32 * CELL + PADDING * 2.0 + self.query.height() + TABS;
        let below = near.bottom() + GAP;
        let above = near.y - height - GAP;
        let y = match below + height <= within.bottom() - PADDING {
            true => below,
            // Clamped rather than left off the top: a window shorter than the
            // grid has nowhere to put it that is not over something, and over
            // the conversation is better than off the screen.
            false => above.max(within.y + PADDING),
        };
        Rect::new(
            near.x
                .min(within.right() - width - PADDING)
                .max(within.x + PADDING),
            y,
            width,
            height,
        )
    }

    pub fn field(&self, near: Rect, within: Rect) -> Rect {
        let panel = self.rect(near, within);
        Rect::new(panel.x, panel.y, panel.width, self.query.height())
    }

    /// The row of tabs, between the field and the grid.
    fn tabs_rect(&self, near: Rect, within: Rect) -> Rect {
        let panel = self.rect(near, within);
        Rect::new(panel.x, panel.y + self.query.height(), panel.width, TABS)
    }

    /// Where the tiles live: the panel, less the field and the tabs.
    ///
    /// What the grid scrolls inside, so it is also what the tiles are clipped
    /// to and what the bar is measured against.
    fn grid(&self, near: Rect, within: Rect) -> Rect {
        let panel = self.rect(near, within);
        let top = panel.y + self.query.height() + TABS + PADDING;
        Rect::new(
            panel.x + PADDING,
            top,
            panel.width - PADDING * 2.0,
            panel.bottom() - PADDING - top,
        )
    }

    /// How far the grid can be scrolled before it runs out.
    fn reach(&self, near: Rect, within: Rect) -> f32 {
        let rows = self.found.len().div_ceil(COLUMNS) as f32;
        (rows * CELL - self.grid(near, within).height).max(0.0)
    }

    /// Where each cell on show sits, so a click can land on one.
    ///
    /// Only the rows that reach into the grid, which under a smooth scroll
    /// includes the two it is halfway through. Everything else has no cell at
    /// all: the boxes a press is answered against and the tiles a reader sees
    /// are this one list, so they cannot disagree about where anything is.
    fn cells(&self, near: Rect, within: Rect) -> Vec<(usize, Rect)> {
        let grid = self.grid(near, within);
        let top = grid.y - self.scroll;
        // The first row the grid reaches, and one more than it can hold: a
        // scroll partway through a row shows the tail of one and the head of
        // another.
        let first = (self.scroll / CELL).floor().max(0.0) as usize;
        self.found
            .iter()
            .enumerate()
            .skip(first * COLUMNS)
            .take(COLUMNS * (ROWS + 2))
            .map(|(at, _)| {
                let (column, row) = (at % COLUMNS, at / COLUMNS);
                (
                    at,
                    Rect::new(
                        grid.x + column as f32 * CELL,
                        top + row as f32 * CELL,
                        CELL,
                        CELL,
                    ),
                )
            })
            .filter(|(_, rect)| rect.bottom() > grid.y && rect.y < grid.bottom())
            .collect()
    }

    /// Where each tab sits, and the group it jumps to.
    fn tab_cells(&self, near: Rect, within: Rect) -> Vec<(usize, Rect)> {
        let row = self.tabs_rect(near, within);
        // Across the tiles rather than the whole panel, so a tab sits over the
        // column it leads to rather than drifting right of it by the bar.
        let across = row.width - PADDING * 2.0 - crate::scrollbar::TRACK;
        let wide = match self.tabs.is_empty() {
            true => 0.0,
            false => across / self.tabs.len() as f32,
        };
        self.tabs
            .iter()
            .enumerate()
            .map(|(which, _)| {
                (
                    which,
                    Rect::new(
                        row.x + PADDING + which as f32 * wide,
                        row.y,
                        wide,
                        row.height,
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
            placed.push(named().at(&at.to_string(), rect, 9));
        }
        for (which, rect) in self.tab_cells(near, within) {
            placed.push(tabbed().at(&which.to_string(), rect, 9));
        }
        placed.extend(
            self.bar
                .boxes(NAME, self.grid(near, within), self.reach(near, within)),
        );
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
        // A new question, answered from the top.
        self.scroll = 0.0;
        // Deeper than the grid shows, because the grid scrolls: a set cut to
        // the four rows on show is a set whose tail can never be reached.
        let room = COLUMNS * DEEP;

        let mut found: Vec<Choice> = Vec::new();
        // Custom first: they are this team's own and the ones a reader is most
        // often reaching for by name.
        //
        // A row of them with nothing typed, though, and the whole set once
        // something is. This team has hundreds, and unbounded they filled
        // every row the grid shows -- so the standard emoji began somewhere
        // below the fourth screenful and the categories could not be seen at
        // all. A row keeps them to hand without burying what follows.
        let theirs = match typed.is_empty() {
            true => COLUMNS,
            false => room,
        };
        if let Some(store) = store {
            for (name, id) in store
                .custom_emoji_matching(&typed, theirs as u32)
                .unwrap_or_default()
            {
                found.push(Choice {
                    name,
                    unicode: None,
                    id: Some(id),
                });
            }
        }
        // With nothing typed, all of them, grouped the way every other client
        // groups them: faces, then people, then nature, and so on. It used to
        // be eight hand-picked names, because eight was all a grid of four
        // rows could hold and the rest could not be reached -- now that it
        // scrolls, "the common few" is the wrong answer to "which ones".
        // Where each group starts, so its tab can go there. Taken as the set
        // is built rather than worked out from it afterwards: a name the
        // renderer cannot draw takes no cell, so a group's first tile is not
        // its place in the table.
        let mut tabs: Vec<(usize, Choice)> = Vec::new();
        match typed.is_empty() {
            true => {
                for (group, names) in matterless_render::emoji::categories() {
                    let from = found.len();
                    found.extend(names.iter().filter_map(|name| standard(name)));
                    // Only a group that put something in gets a tab: one that
                    // drew nothing would be a mark leading to the group after
                    // it.
                    if found.len() > from {
                        tabs.push((from, mark_of(group, names)));
                    }
                }
            }
            // A search has one order, by how well each matched, and is cut to
            // what can be walked. Groups over the top of that would be
            // claiming an order it does not have.
            false => found.extend(
                matterless_render::emoji::names_matching(&typed, room)
                    .into_iter()
                    .filter_map(standard)
                    .take(room.saturating_sub(found.len())),
            ),
        }
        self.found = found;
        self.tabs = tabs;
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
        let stepped = [Key::Left, Key::Right, Key::Up, Key::Down]
            .into_iter()
            .any(|key| input.struck(key));
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
        let reach = self.reach(near, within);
        let grid = self.grid(near, within);
        let boxes = self.boxes(near, within);
        // The bar first: while it is held, the hand decides where the grid is
        // and nothing else may write that position.
        if let Some(scroll) = self.bar.react(NAME, input, grid, self.scroll, reach) {
            self.scroll = scroll.clamp(0.0, reach);
            return None;
        }
        // The wheel, in pixels, the way the conversation behind it moves. By
        // whole rows it stepped thirty-four at a time, which against a
        // conversation that slides reads as something stuttering.
        if let Some((_, y)) = input.wheel_over(&boxes, owns) {
            self.scroll = (self.scroll - y).clamp(0.0, reach);
        }
        // A tab: straight to where its group starts.
        if let Some(slug) = input.clicked().and_then(|name| tabbed().slug(name))
            && let Ok(which) = slug.parse::<usize>()
            && let Some((at, _)) = self.tabs.get(which)
        {
            self.scroll = ((at / COLUMNS) as f32 * CELL).clamp(0.0, reach);
            return None;
        }
        // Whatever the arrows picked has to be on show, or the reader is
        // walking a selection they cannot see.
        //
        // Only when they have just moved it. Run every frame, this drags the
        // grid back onto the selection the moment the wheel has moved it away
        // -- which is to say the wheel did nothing at all, and every turn of
        // it left the grid on its first thirty-two.
        if stepped {
            let row = (self.chosen / COLUMNS) as f32 * CELL;
            self.scroll = self
                .scroll
                .clamp((row + CELL - grid.height).max(0.0), row)
                .clamp(0.0, reach);
        }
        // A click on a cell, which is the ordinary way to pick a picture.
        if let Some(slug) = input.clicked().and_then(|name| named().slug(name))
            && let Ok(at) = slug.parse::<usize>()
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

    pub fn draw(&self, into: &mut Canvas<'_>, input: &Input, near: Rect, within: Rect) {
        if !self.open() {
            return;
        }
        let panel = self.rect(near, within);
        let grid = self.grid(near, within);
        let reach = self.reach(near, within);
        let cells = self.cells(near, within);
        let tabs = self.tab_cells(near, within);
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
        Panel::floating(panel, PANEL, 10.0)
            .edge(palette.rule)
            .fill(palette.surface)
            .draw(scene);

        // The groups, each a mark that goes there. Under the field and over
        // the grid, which is where the official client puts them and where a
        // reader raised on it will look.
        let here = self
            .tabs
            .iter()
            .rposition(|(at, _)| (*at / COLUMNS) as f32 * CELL <= self.scroll + 1.0);
        for (which, rect) in &tabs {
            let Some((_, mark)) = self.tabs.get(*which) else {
                continue;
            };
            // The one the grid is in, said with a ground rather than only by
            // the tiles below it -- a row of marks with none of them lit says
            // nothing about where the reader is.
            if here == Some(*which) {
                scene.fill(rect.x, rect.y, rect.width, rect.height, palette.ground);
            }
            let Some(face) = mark.unicode.as_ref() else {
                continue;
            };
            matterless_widgets::centred(scene, painter, fonts, palette, face, *rect, mark_run());
        }
        // A hairline under them, so the row reads as a header on the grid
        // rather than as a first row of it.
        if !tabs.is_empty() {
            scene.fill(
                panel.x + PADDING,
                self.tabs_rect(near, within).bottom() - 1.0,
                panel.width - PADDING * 2.0 - crate::scrollbar::TRACK,
                1.0,
                palette.rule,
            );
        }

        // The tiles, inside the grid and nowhere else: a smooth scroll shows
        // the half of a row it is partway through, and the half that is past
        // the edge must not be drawn over the tabs above it.
        scene.clip_to(grid.x, grid.y, grid.width, grid.height);
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
                (Some(face), _) => matterless_widgets::centred(
                    scene,
                    painter,
                    fonts,
                    palette,
                    face,
                    rect,
                    Run {
                        size: 18.0,
                        line_height: 22.0,
                        ..mark_run()
                    },
                ),
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
        // Inside the same clip, because it belongs to the grid and sits down
        // its right-hand edge.
        self.bar.draw(
            &mut Canvas {
                scene,
                painter,
                fonts,
                palette,
            },
            NAME,
            input,
            grid,
            self.scroll,
            reach,
        );
        // And back to the panel, because the field is drawn after this and
        // would otherwise be clipped away by the grid it sits above.
        scene.clip_to(panel.x, panel.y, panel.width, panel.height);
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
    use matterless_ui::input::Event;

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

    /// More offered than the grid holds, and never more shown than it holds.
    ///
    /// This used to assert the offered set itself was capped to the four rows
    /// on show, which is the same thing as saying the rest could not be
    /// reached. The invariant it was protecting -- that the grid cannot
    /// overflow -- now belongs to `cells`, which is the one place that decides
    /// both where a tile is drawn and where a press is answered.
    #[test]
    fn it_offers_more_than_it_shows_and_shows_no_more_than_it_holds() {
        let mut picker = Picker::new();
        picker.for_post = Some("p1".into());
        picker.ask(None);
        assert!(
            picker.found.len() > COLUMNS * ROWS,
            "only {} offered, so there is nothing to scroll to",
            picker.found.len()
        );
        let window = Rect::new(0.0, 0.0, 900.0, 600.0);
        let near = Rect::new(100.0, 100.0, 10.0, 10.0);
        // A row either side of what the grid holds, because a smooth scroll
        // shows the tail of one row and the head of another.
        assert!(picker.cells(near, window).len() <= COLUMNS * (ROWS + 2));
        // And scrolled to the end, the end is what is there: it holds the last
        // of them rather than stopping short or running past into nothing.
        picker.scroll = picker.reach(near, window);
        let cells = picker.cells(near, window);
        assert_eq!(
            cells.last().map(|(at, _)| *at),
            Some(picker.found.len() - 1),
            "the last of them cannot be scrolled to"
        );
    }

    /// With nothing typed they come in the order every other client shows
    /// them: the categories in turn, and each in the dataset's own order.
    #[test]
    fn it_offers_them_grouped_the_way_every_client_groups_them() {
        let mut picker = Picker::new();
        picker.for_post = Some("p1".into());
        picker.ask(None);
        let first = matterless_render::emoji::categories()
            .first()
            .and_then(|(_, names)| names.first())
            .expect("no categories to group by");
        assert_eq!(
            picker.found.first().map(|one| one.name.as_str()),
            Some(*first),
            "the grid does not start where the first category does"
        );
    }

    /// The wheel walks the grid, and the arrows drag it back to the selection.
    ///
    /// Keeping the selection on show ran every frame rather than only when the
    /// arrows had moved it, so it dragged the grid back the instant the wheel
    /// moved it away: the wheel arrived, the arithmetic was right, and the
    /// view stayed on its first thirty-two anyway. Which also looked like the
    /// categories were missing, since the first thirty-two are where they
    /// start.
    #[test]
    fn the_wheel_walks_past_what_the_grid_holds() {
        let mut fonts = Fonts::new();
        let mut picker = Picker::new();
        let window = Rect::new(0.0, 0.0, 900.0, 900.0);
        let button = Rect::new(400.0, 100.0, 24.0, 24.0);
        let mut input = Input::default();
        picker.show("p1", &mut fonts, &mut input);
        picker.ask(None);

        let panel = picker.rect(button, window);
        input.apply(
            Event::PointerMoved {
                x: panel.x + panel.width / 2.0,
                y: panel.y + panel.height / 2.0,
            },
            &picker.boxes(button, window),
        );
        input.apply(
            Event::Wheel { x: 0.0, y: -63.0 },
            &picker.boxes(button, window),
        );
        let mut nothing = String::new();
        picker.react(&mut fonts, &input, button, window, &mut nothing, None);
        assert!(
            picker.scroll > 0.0,
            "the wheel turned and the grid stayed on its first rows"
        );
        // By what the wheel said rather than by a whole row, which is what a
        // conversation does and what this now does beside one.
        assert_eq!(picker.scroll, 63.0);
        // And what is on show is further in than the grid holds, which is the
        // whole point of being able to turn it.
        let first = picker
            .cells(button, window)
            .first()
            .map(|(at, _)| *at)
            .expect("no cells on show");
        assert!(first >= COLUMNS, "the first row on show is still the first");
    }

    /// A tab per group, and pressing one goes to where that group starts.
    ///
    /// By where its first *tile* landed rather than by its place in the table:
    /// a name the renderer cannot draw takes no cell, so the two differ.
    #[test]
    fn a_tab_goes_to_where_its_group_starts() {
        let mut fonts = Fonts::new();
        let mut picker = Picker::new();
        let window = Rect::new(0.0, 0.0, 900.0, 900.0);
        let button = Rect::new(400.0, 100.0, 24.0, 24.0);
        let mut input = Input::default();
        picker.show("p1", &mut fonts, &mut input);
        picker.ask(None);

        assert_eq!(
            picker.tabs.len(),
            matterless_render::emoji::categories().len(),
            "a group without a tab cannot be reached except by scrolling to it"
        );
        // Each tab names a tile that is actually in the set, and they come in
        // the order the groups do.
        for pair in picker.tabs.windows(2) {
            assert!(pair[0].0 < pair[1].0, "the tabs are out of order");
        }
        let (at, _) = *picker.tabs.last().expect("no tabs");
        assert!(at < picker.found.len(), "a tab points past the end");

        // Pressing the last one takes the grid to it.
        let last = picker.tabs.len() - 1;
        let tab = picker
            .tab_cells(button, window)
            .into_iter()
            .find(|(which, _)| *which == last)
            .expect("no tab to press")
            .1;
        let placed = picker.boxes(button, window);
        input.apply(
            Event::PointerMoved {
                x: tab.x + tab.width / 2.0,
                y: tab.y + tab.height / 2.0,
            },
            &placed,
        );
        input.apply(Event::PointerPressed, &placed);
        input.apply(Event::PointerReleased, &placed);
        let mut nothing = String::new();
        picker.react(&mut fonts, &input, button, window, &mut nothing, None);
        assert!(
            picker.scroll > 0.0,
            "the last group's tab left the grid at the top"
        );
        // And what is on show includes where that group begins.
        assert!(
            picker
                .cells(button, window)
                .iter()
                .any(|(one, _)| *one == at),
            "the group's first tile is not on show"
        );
    }

    /// The bar sits beside the tiles, not over the last column of them.
    #[test]
    fn the_bar_keeps_clear_of_the_tiles() {
        let mut fonts = Fonts::new();
        let mut picker = Picker::new();
        let window = Rect::new(0.0, 0.0, 900.0, 900.0);
        let button = Rect::new(400.0, 100.0, 24.0, 24.0);
        let mut input = Input::default();
        picker.show("p1", &mut fonts, &mut input);
        picker.ask(None);

        let grid = picker.grid(button, window);
        let track = picker.bar.track(grid);
        let widest = picker
            .cells(button, window)
            .into_iter()
            .map(|(_, rect)| rect.right())
            .fold(0.0_f32, f32::max);
        assert!(
            widest <= track.x,
            "the last column reaches {widest} and the bar starts at {}",
            track.x
        );
        // And the full eight columns are still there: the room came from the
        // panel rather than from the tiles.
        assert_eq!(
            picker
                .cells(button, window)
                .into_iter()
                .filter(|(_, rect)| rect.y == grid.y)
                .count(),
            COLUMNS
        );
    }

    /// The grid hangs off the button, below it or above it but never over it.
    ///
    /// Anchored to the button's top edge before, which drew the grid across
    /// the toolbar it was opened from and across the quick faces beside it --
    /// so the reader could not see what they had just pressed.
    #[test]
    fn the_grid_never_covers_what_opened_it() {
        let picker = Picker::new();
        let window = Rect::new(0.0, 0.0, 900.0, 900.0);
        // Room below, which is the ordinary case.
        let button = Rect::new(400.0, 100.0, 24.0, 24.0);
        let panel = picker.rect(button, window);
        assert!(
            panel.y >= button.bottom(),
            "the grid starts at {} and the button ends at {}",
            panel.y,
            button.bottom()
        );

        // No room below, so above it instead -- still not over it.
        let low = Rect::new(400.0, 860.0, 24.0, 24.0);
        let panel = picker.rect(low, window);
        assert!(
            panel.bottom() <= low.y,
            "the grid ends at {} and the button starts at {}",
            panel.bottom(),
            low.y
        );
        assert!(panel.y >= window.y, "the grid is off the top of the window");
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
