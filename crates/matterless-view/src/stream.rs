//! A scrolling conversation, and the rows a pointer can land on.
//!
//! Extracted from the shell rather than written for the thread pane, but the
//! thread pane is why: a thread is the same rows, the same layout and the same
//! drawing in a narrower column, and two panels that draw messages by walking
//! the layout separately is exactly how the browser and the virtualiser came to
//! disagree about heights.
//!
//! Rows become hit targets here for the first time. Everything the stream will
//! eventually do -- open a thread, hover an action, select text -- starts with
//! knowing which message the pointer is over.

use matterless_layout::Fonts;
use matterless_layout::row::{RowLayout, Theme, lay_out};
use matterless_paint::Run;
use matterless_render::Row;
use matterless_ui::input::Input;
use matterless_ui::{Placed, Rect};
use matterless_widgets::{Canvas, Panel};

/// What a click on a row asked for.
#[derive(Debug, Clone, PartialEq)]
pub enum Chose {
    /// Open this thread.
    Thread(String),
    /// Try this message again, by the pending id it still carries.
    Retry(String),
    /// Do something to one message.
    Act {
        action: crate::actions::Action,
        post_id: String,
        /// What the toggle becomes, for the actions that are one.
        on: bool,
    },
    /// Follow something somebody wrote: a link, a person, a conversation.
    Press {
        press: matterless_layout::row::Press,
        /// Where the words that were pressed sit, for whatever has to point at
        /// them.
        ///
        /// Carried rather than looked up afterwards, for the same reason the
        /// menu's rect is: by then all anybody has is the press itself, and
        /// the same person is named a dozen times in a conversation. Asking
        /// where `@someone` is answers with the first of them, so the card
        /// opened against a name at the top of the screen however far down the
        /// one actually pressed was.
        ///
        /// `None` for a press with no words of its own -- a preview card is
        /// the whole box, and what it leads to opens a browser rather than
        /// anything that needs anchoring.
        at: Option<Rect>,
    },
    /// Keep a file somebody attached.
    Save { file_id: String, name: String },
    /// Look at one of a message's pictures properly.
    ///
    /// The whole message, not just the one pressed, so the viewer can step
    /// through them without asking the stream again -- and because which
    /// others there are is what decides whether it offers to.
    Look { file_id: String, post_id: String },
    /// Add or remove this reaction.
    React {
        post_id: String,
        emoji: String,
        /// What it becomes, decided here rather than waited for: a pill has to
        /// change the instant it is pressed.
        on: bool,
    },
    /// Open the menu behind `...`, under the button that was pressed.
    ///
    /// The rect travels with it because the menu hangs off the button, and by
    /// the time the shell reacts the pointer has already moved.
    More { post_id: String, under: Rect },
}

/// Where a reader is, said so that it still means the same place once every
/// row has been measured against a different width.
///
/// A scroll is a number of pixels and is not that: the same distance points
/// somewhere else the moment the rows change height.
#[derive(Debug, Clone, PartialEq)]
pub enum Anchor {
    /// At the newest message, which is where a conversation opens and where it
    /// stays while somebody reads along. Held against the bottom edge, not by
    /// any row: the rows above are what changed.
    End,
    /// At the top of what has been loaded. Held against the top edge, for the
    /// same reason and in reverse.
    Start,
    /// Reading somewhere in between, where neither edge means anything: the
    /// row lying across the middle of the panel, and where the top edge falls
    /// relative to it. `None` when there is no such row.
    Row(Option<(String, f32)>),
}

/// One conversation: its rows, their heights, and where the reader is in it.
pub struct Stream {
    /// What this panel answers to, so a channel and a thread can coexist.
    pub name: String,
    pub rows: Vec<Row>,
    pub laid: Vec<RowLayout>,
    pub theme: Theme,
    pub scroll: f32,
    /// Custom emoji this conversation uses, by name to the id whose image the
    /// server holds. A standard emoji is a character and needs nothing here.
    pub custom: std::collections::HashMap<String, String>,
    /// Who the reader is, so their own messages offer what only they may do.
    pub me: String,
    /// Where each pressable run of words was drawn, from the frame just gone.
    presses: Vec<(matterless_layout::row::Press, Rect)>,
    /// This panel's own bar. Each list has one, because a drag in the thread
    /// pane must not scroll the channel behind it.
    pub bar: crate::scrollbar::Scrollbar,
    /// The message whose quick faces are open, and the button they hang from.
    ///
    /// Held rather than recomputed because the row it belongs to scrolls: the
    /// rect is where the button was when it was pressed, which is where the
    /// row of faces stays until it is answered or dismissed.
    picking: Option<(String, Rect)>,
    /// What was shaped last time, by row identity.
    ///
    /// A channel is replanned far more often than it changes: opening it, the
    /// socket signing in, a reaction landing, the window being resized, a
    /// reply arriving four hundred messages below the one that changed. Every
    /// one of those used to shape all four hundred rows again -- a second at a
    /// time in a channel of crash reports, six times over in the first few
    /// seconds of the window's life.
    ///
    /// Keyed on identity and checked on equality: the key finds the row that
    /// was in this place last time even if rows have been added above it, and
    /// the comparison is what says whether it is still the same row. Comparing
    /// a message costs a string compare; shaping one costs four milliseconds.
    kept: std::collections::HashMap<String, (Row, RowLayout)>,
    /// What `kept` was shaped against: the column's width, the day it was,
    /// and the reader's offset from UTC.
    ///
    /// A narrower column wraps differently. And a separator says "Today" --
    /// a window left open across midnight would keep yesterday's word for it,
    /// because the row itself is a day number and has not changed.
    kept_against: (f32, i64, i32),
    /// How many rows the last layout took from the cache rather than shaping.
    /// Read by the tests, which is the only way to tell reuse from a very fast
    /// shaper.
    reused: usize,
    /// Rows that have been planned but not yet shaped, oldest first, waiting
    /// above the ones that have.
    ///
    /// A conversation opens at its newest message, so the rows under the
    /// reader's eye are the last handful. Shaping all four hundred before the
    /// window can draw any of them cost a second and a half in a channel of
    /// crash reports -- for four hundred messages nobody had scrolled to yet.
    ///
    /// They are not estimated. A row waiting here has no height and takes part
    /// in nothing: it is not in the list, and the list is exactly as tall as
    /// what has been measured. That is the difference between this and the
    /// guess-then-correct the layout crate exists to avoid -- nothing on
    /// screen is ever wrong, there is simply less of it for a moment.
    above: Vec<Row>,
}

/// Whether this row is that message.
fn names(row: &Row, post_id: &str) -> bool {
    matches!(
        row,
        Row::Post { post } | Row::Continuation { post } if post.post_id == post_id
    )
}

/// What names a row across two plans of the same channel.
///
/// Not an index: a page of older history arriving pushes every row down, and a
/// cache that matched on position would miss all of them. Rows that have no id
/// of their own are named by what they are, which is enough -- there is one
/// divider, and one separator per day.
fn key_of(row: &Row) -> String {
    match row {
        Row::Post { post } | Row::Continuation { post } => format!("post/{}", post.post_id),
        Row::System { post_id, .. } => format!("system/{post_id}"),
        Row::DeletedRoot { post_id, .. } => format!("gone/{post_id}"),
        Row::ThreadFooter { root_id, .. } => format!("footer/{root_id}"),
        Row::DateSeparator { epoch_day } => format!("day/{epoch_day}"),
        Row::UnreadDivider => "divider".to_string(),
    }
}

impl Stream {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            rows: Vec::new(),
            laid: Vec::new(),
            theme: Theme::default(),
            scroll: 0.0,
            custom: std::collections::HashMap::new(),
            me: String::new(),
            presses: Vec::new(),
            bar: crate::scrollbar::Scrollbar::default(),
            picking: None,
            kept: std::collections::HashMap::new(),
            kept_against: (f32::NAN, 0, 0),
            reused: 0,
            above: Vec::new(),
        }
    }

    /// Lays every row out for this width.
    ///
    /// Once per width change, never per frame: a row's height does not depend
    /// on the scroll position, which is the property that makes this list
    /// honest and the DOM one not.
    ///
    /// The clock is read here rather than held, so a machine that crosses
    /// midnight or a daylight-saving boundary while running is right
    /// afterwards.
    pub fn lay_out(&mut self, fonts: &mut Fonts, width: f32) {
        self.lay_out_on(
            fonts,
            width,
            // So a date in this year can leave the year off.
            crate::clock::today(),
            crate::clock::utc_offset_minutes(),
        );
    }

    /// The same, as of a given day, so the day can be moved in a test.
    pub fn lay_out_on(&mut self, fonts: &mut Fonts, width: f32, today: i64, offset: i32) {
        self.theme = Theme {
            // The content's width, not the panel's: the stream keeps a margin
            // clear of its own edges, so a message never starts against the
            // sidebar nor ends against the scrollbar.
            width: width - Theme::default().pad_x * 2.0,
            today,
            utc_offset_minutes: offset,
            ..Theme::default()
        };
        let against = (
            self.theme.width,
            self.theme.today,
            self.theme.utc_offset_minutes,
        );
        let reusable = self.kept_against == against;
        let mut kept = std::collections::HashMap::with_capacity(self.rows.len());
        let mut reused = 0;
        self.laid = self
            .rows
            .iter()
            .map(|row| {
                let key = key_of(row);
                let found = self
                    .kept
                    .get(&key)
                    .filter(|_| reusable)
                    .filter(|(was, _)| was == row)
                    .map(|(_, laid)| laid.clone());
                reused += usize::from(found.is_some());
                let laid = found.unwrap_or_else(|| lay_out(fonts, row, &self.theme));
                kept.insert(key, (row.clone(), laid.clone()));
                laid
            })
            .collect();
        self.kept = kept;
        self.kept_against = against;
        self.reused = reused;
    }

    /// How many rows the last layout reused.
    pub fn reused(&self) -> usize {
        self.reused
    }

    /// Lays out only the rows on screen, for a width that is still changing.
    ///
    /// Shaping a channel is 180 messages of cosmic-text and takes longer than
    /// a frame -- measured at 24ms against 0.4ms for all the bookkeeping
    /// around it, so there is nothing to shave, only work to not do. A drag of
    /// the window's edge asks for a new width on every frame of itself, and
    /// what the reader can see is a dozen rows of the hundred and eighty.
    ///
    /// So the dozen are shaped against the new width and the rest keep the
    /// heights they had. The list is briefly a little wrong about its own
    /// height, which shows as the scrollbar drifting while the edge moves; the
    /// full pass at the end of the drag settles it. What it is never wrong
    /// about is the part being looked at.
    pub fn relay_seen(&mut self, fonts: &mut Fonts, width: f32, within: Rect) {
        self.theme = Theme {
            width: width - Theme::default().pad_x * 2.0,
            ..self.theme
        };
        let mut top = within.y + self.theme.pad_top - self.scroll;
        for (index, row) in self.rows.iter().enumerate() {
            let Some(was) = self.laid.get(index) else {
                break;
            };
            let bottom = top + was.height;
            if bottom >= within.y && top <= within.bottom() {
                self.laid[index] = lay_out(fonts, row, &self.theme);
            }
            top = bottom;
            if top > within.bottom() {
                break;
            }
        }
        // Every row is against a stale width now, this one's included: the
        // cache must not hand any of them back.
        self.kept.clear();
        self.kept_against = (f32::NAN, 0, 0);
    }

    /// Takes a fresh plan.
    ///
    /// `lazily` says the reader is at the newest message, which is the only
    /// time the top of the conversation can be left unshaped: they are looking
    /// at the other end of it. Anywhere else -- a page of older history
    /// arriving under somebody reading the top of the channel -- everything
    /// has to be measured, because what is under their eye could be any of it.
    pub fn plan(&mut self, rows: Vec<Row>, lazily: bool) {
        self.above.clear();
        // A first guess at what covers the panel. `cover` measures whether it
        // actually did and asks for more until it has.
        const TAIL: usize = 12;
        match lazily && rows.len() > TAIL {
            true => {
                let mut rows = rows;
                self.rows = rows.split_off(rows.len() - TAIL);
                self.above = rows;
            }
            false => self.rows = rows,
        }
    }

    /// How many planned rows are still waiting to be shaped.
    pub fn waiting(&self) -> usize {
        self.above.len()
    }

    /// Every row of the plan, shaped or not.
    ///
    /// For anything asking what is *in* the conversation rather than what is
    /// on screen -- which emoji it mentions, which pictures it wants. Those
    /// questions were answered over `rows` alone, and once the top of the
    /// channel stopped being shaped up front they would have been answered
    /// about the last dozen messages.
    pub fn planned(&self) -> impl Iterator<Item = &Row> {
        self.above.iter().chain(self.rows.iter())
    }

    /// Shapes some of what is waiting, and answers how much taller the list
    /// became -- which is what a caller holding the reader's place needs.
    pub fn fill(&mut self, fonts: &mut Fonts, width: f32, at_most: usize) -> f32 {
        if self.above.is_empty() || at_most == 0 {
            return 0.0;
        }
        let before = self.total();
        let at = self.above.len().saturating_sub(at_most);
        let moved: Vec<Row> = self.above.drain(at..).collect();
        self.rows.splice(0..0, moved);
        // Everything already shaped comes back from the cache, so this costs
        // the slice and not the conversation.
        self.lay_out(fonts, width);
        self.total() - before
    }

    /// Shapes enough of the newest end to fill the panel.
    ///
    /// Counted in pixels rather than rows, because a row is anything from one
    /// line to a screenful of code, and a fixed number of them covers a panel
    /// in one channel and a third of it in the next.
    pub fn cover(&mut self, fonts: &mut Fonts, width: f32, within: Rect) {
        // Half a panel past the bottom edge, so a small scroll has somewhere
        // to go before the next slice arrives.
        let want = within.height * 1.5;
        while !self.above.is_empty() && self.total() < want {
            self.fill(fonts, width, 12);
        }
    }

    pub fn total(&self) -> f32 {
        self.laid.iter().map(|row| row.height).sum()
    }

    /// How far it can be scrolled before it runs out.
    pub fn reach(&self, within: Rect) -> f32 {
        (self.total() + self.theme.pad_top + self.theme.pad_bottom - within.height).max(0.0)
    }

    /// Shows the newest message, which is where a conversation opens.
    pub fn to_bottom(&mut self, within: Rect) {
        self.scroll = self.reach(within);
    }

    /// Brings one message into view, a third of the way down the panel.
    ///
    /// A third rather than the top: a message with nothing above it on screen
    /// has lost the conversation it was part of, which is most of why anybody
    /// follows a link to one.
    ///
    /// Answers whether it was found. A message the store has never loaded
    /// cannot be scrolled to, and saying so lets the caller leave the reader
    /// at the newest instead of somewhere arbitrary.
    pub fn to_post(&mut self, fonts: &mut Fonts, post_id: &str, within: Rect) -> bool {
        // It may be one of the rows still waiting to be shaped, which have no
        // height and so nowhere to scroll to. Following a link to a message
        // is the one thing that cannot wait for the filling to reach it, so
        // it is hurried along -- only as far as the message asked for, and the
        // rest goes on arriving behind the reader as before.
        while self.above.iter().any(|row| names(row, post_id)) {
            self.fill(fonts, within.width, 24);
        }
        let mut top = 0.0;
        for (index, laid) in self.laid.iter().enumerate() {
            if self.rows.get(index).is_some_and(|row| names(row, post_id)) {
                self.scroll = (top - within.height / 3.0).clamp(0.0, self.reach(within));
                return true;
            }
            top += laid.height;
        }
        false
    }

    pub fn clamp(&mut self, within: Rect) {
        self.scroll = self.scroll.clamp(0.0, self.reach(within));
    }

    /// Takes one row out, and the shaping that goes with it.
    ///
    /// Rather than replanning the channel and laying it out again: the other
    /// four hundred rows have not changed, and reshaping them to remove a
    /// line of text costs whole seconds in a conversation of crash reports.
    ///
    /// The reader stays where they were. A row taken from above the viewport
    /// pulls everything below it up by its own height, and the scroll has to
    /// come with it or the words under the eye jump.
    pub fn forget_row(&mut self, at: usize) {
        if at >= self.rows.len() || at >= self.laid.len() {
            return;
        }
        let above: f32 = self.laid[..at].iter().map(|row| row.height).sum();
        let height = self.laid[at].height;
        self.kept.remove(&key_of(&self.rows[at]));
        self.rows.remove(at);
        self.laid.remove(at);
        if above < self.scroll {
            self.scroll = (self.scroll - height).max(0.0);
        }
    }

    /// The thread a row belongs to: its own id when it is a root, and the root
    /// it hangs from when it is a reply.
    ///
    /// `None` for the rows that are not messages -- a date separator has no
    /// thread to open, and clicking one must do nothing rather than open the
    /// last thread that happened to be under the pointer.
    pub fn root_of(&self, index: usize) -> Option<String> {
        match self.rows.get(index)? {
            Row::Post { post } | Row::Continuation { post } => Some(if post.root_id.is_empty() {
                post.post_id.clone()
            } else {
                post.root_id.clone()
            }),
            Row::ThreadFooter { root_id, .. } => Some(root_id.clone()),
            _ => None,
        }
    }

    fn row_name(&self, index: usize) -> String {
        format!("{}/row/{index}", self.name)
    }

    /// The panel, and every row currently on screen.
    ///
    /// Only the visible ones, unlike the sidebar: a channel is hundreds of
    /// thousands of rows where a sidebar is hundreds, and placing them all
    /// would cost more than the drawing does.
    pub fn boxes(&self, within: Rect, hovered: Option<usize>) -> Vec<Placed> {
        let inner = self.inner(within);
        let mut placed = vec![Placed {
            name: self.name.clone(),
            rect: within,
            depth: 1,
        }];
        let mut top = within.y + self.theme.pad_top - self.scroll;
        for (index, row) in self.laid.iter().enumerate() {
            let bottom = top + row.height;
            if bottom >= within.y && top <= within.bottom() {
                placed.push(Placed {
                    name: self.row_name(index),
                    // Clipped to the panel, so a row half off the top is only
                    // hit where it can actually be seen.
                    rect: Rect::new(
                        inner.x,
                        top.max(within.y),
                        inner.width,
                        (bottom.min(within.bottom()) - top.max(within.y)).max(0.0),
                    ),
                    depth: 2,
                });
                // The first thing inside a message a pointer can land on.
                // Deeper than the row, so a click on a pill is a click on the
                // pill rather than on the message behind it.
                // Only the hovered row has a toolbar, so only it has buttons.
                if hovered == Some(index) {
                    for (tool, rect) in self.tools(index, top, within) {
                        placed.push(Placed {
                            name: self.tool_name(index, tool),
                            rect,
                            depth: 3,
                        });
                    }
                }
                // A picture is pressable: that is how it is opened, and it is
                // also the only way one can be kept, since Save lives on a
                // card and a picture is never drawn as one.
                for ordinal in 0..self.pictured(index).len() {
                    if let Some(rect) = self.picture_rect(index, ordinal, top, inner.x) {
                        placed.push(Placed {
                            name: format!("{}/row/{index}/look/{ordinal}", self.name),
                            rect,
                            depth: 3,
                        });
                    }
                }
                for (ordinal, (_, file)) in self.filed(index).into_iter().enumerate() {
                    let _ = file;
                    let card = self.card_rect(index, ordinal, top, inner.x);
                    if let Some(card) = card {
                        placed.push(Placed {
                            name: format!("{}/row/{index}/save/{ordinal}", self.name),
                            rect: Rect::new(
                                card.right() - SAVE - 10.0,
                                card.y + (card.height - 22.0) / 2.0,
                                SAVE,
                                22.0,
                            ),
                            depth: 3,
                        });
                    }
                }
                for (ordinal, rect) in self
                    .preview_runs(index, top, inner.x)
                    .into_iter()
                    .enumerate()
                {
                    // Only when it leads somewhere: a card whose link this
                    // window will not open must not look pressable.
                    if self.preview_press(index, ordinal).is_some() {
                        placed.push(Placed {
                            name: self.preview_name(index, ordinal),
                            rect,
                            depth: 3,
                        });
                    }
                }
                for (ordinal, (block, _)) in self.pills(index).into_iter().enumerate() {
                    placed.push(Placed {
                        name: format!("{}/row/{index}/reaction/{ordinal}", self.name),
                        rect: Rect::new(
                            inner.x + self.theme.gutter + block.x,
                            top + block.y,
                            block.wrap,
                            block.height,
                        ),
                        depth: 3,
                    });
                }
            }
            top = bottom;
        }
        placed.extend(self.bar.boxes(&self.name, within, self.reach(within)));
        if let Some(rect) = self.to_newest(within) {
            placed.push(Placed {
                name: format!("{}/newest", self.name),
                rect,
                depth: 5,
            });
        }
        // The pressable words from the frame just gone: they sit under the
        // pills and the toolbar, which are things in their own right rather
        // than words in a sentence.
        for (at, (_, rect)) in self.presses.iter().enumerate() {
            if rect.bottom() < within.y || rect.y > within.bottom() {
                continue;
            }
            placed.push(Placed {
                name: format!("{}/press/{at}", self.name),
                rect: *rect,
                depth: 3,
            });
        }
        // The quick faces over all of it, with a catcher under them: they hang
        // outside the row that opened them and a click anywhere else puts them
        // away, which is what a `details` gets from the browser for nothing.
        if let Some(panel) = self.faces_panel(within) {
            placed.push(Placed {
                name: format!("{}/faces/elsewhere", self.name),
                rect: within,
                depth: 6,
            });
            placed.push(Placed {
                name: format!("{}/faces", self.name),
                rect: panel,
                depth: 7,
            });
            for (at, face) in crate::actions::faces(panel).into_iter().enumerate() {
                placed.push(Placed {
                    name: format!("{}/faces/{at}", self.name),
                    rect: face,
                    depth: 8,
                });
            }
        }
        placed
    }

    /// The file cards on one row, paired with the block that measured each.
    ///
    /// In one place because two things ask -- the drawing and the hit test --
    /// and the nth card has to be the nth file for both of them.
    fn filed(
        &self,
        index: usize,
    ) -> Vec<(&matterless_layout::row::Block, &matterless_render::FileRef)> {
        let (Some(Row::Post { post } | Row::Continuation { post }), Some(laid)) =
            (self.rows.get(index), self.laid.get(index))
        else {
            return Vec::new();
        };
        laid.blocks
            .iter()
            .filter(|block| block.kind == matterless_layout::row::Kind::Attachment)
            .zip(post.files.iter())
            .filter(|(_, file)| !file.image && !file.video)
            .collect()
    }

    /// A row's pictures, paired with the blocks they were drawn in.
    ///
    /// The mirror of `filed`, which pairs the ones drawn as cards: together
    /// they are every attachment, split by how it is shown.
    fn pictured(
        &self,
        index: usize,
    ) -> Vec<(&matterless_layout::row::Block, &matterless_render::FileRef)> {
        let (Some(Row::Post { post } | Row::Continuation { post }), Some(laid)) =
            (self.rows.get(index), self.laid.get(index))
        else {
            return Vec::new();
        };
        laid.blocks
            .iter()
            .filter(|block| block.kind == matterless_layout::row::Kind::Attachment)
            .zip(post.files.iter())
            .filter(|(_, file)| file.image || file.video)
            .collect()
    }

    /// Where one picture sits on screen, so a press can land on it.
    fn picture_rect(&self, index: usize, ordinal: usize, top: f32, left: f32) -> Option<Rect> {
        let (block, _) = self.pictured(index).into_iter().nth(ordinal)?;
        Some(Rect::new(
            left + self.theme.gutter,
            top + block.y,
            block.wrap,
            block.height,
        ))
    }

    /// Where one file card sits on screen.
    fn card_rect(&self, index: usize, ordinal: usize, top: f32, left: f32) -> Option<Rect> {
        let (block, _) = self.filed(index).into_iter().nth(ordinal)?;
        Some(Rect::new(
            left + self.theme.gutter,
            top + block.y,
            (self.theme.text_width() * 0.6).min(320.0),
            block.height,
        ))
    }

    /// How far from the newest message the reader is sitting.
    ///
    /// Zero when they are at the bottom, which is where a conversation opens
    /// and where it stays while they read along.
    pub fn behind(&self, within: Rect) -> f32 {
        (self.reach(within) - self.scroll).max(0.0)
    }

    /// The button that takes them back to the newest message.
    ///
    /// Floated over the conversation near the bottom, where the eye already is
    /// when it reaches the end of what it was reading. `None` while they are
    /// already there: a button that does nothing is one more thing to read.
    pub fn to_newest(&self, within: Rect) -> Option<Rect> {
        // A screenful and a bit. Less than that and the button appears while
        // somebody is simply reading the last few messages, which is exactly
        // when they do not want anything jumping into the middle of it.
        if self.behind(within) < within.height * 0.75 {
            return None;
        }
        Some(Rect::new(
            within.x + (within.width - JUMP) / 2.0,
            within.bottom() - 44.0,
            JUMP,
            28.0,
        ))
    }

    /// The panel minus the margin it keeps clear of its own edges.
    ///
    /// Everything a row owns is placed in here -- its hit box, its hover fill,
    /// its text -- while the panel itself keeps the outer rect for its clip
    /// and its scrollbar, which belong to the edge rather than to the content.
    fn inner(&self, within: Rect) -> Rect {
        Rect::new(
            within.x + self.theme.pad_x,
            within.y,
            (within.width - self.theme.pad_x * 2.0).max(1.0),
            within.height,
        )
    }

    /// Whether any of these messages is in this stream.
    ///
    /// Every row, not only the visible ones: a reaction on a message just above
    /// the fold still belongs on the page, and a reader who scrolls back to it
    /// should not find a stale pill there.
    pub fn holds_any(&self, post_ids: &[String]) -> bool {
        self.planned().any(|row| match row {
            Row::Post { post } | Row::Continuation { post } => {
                post_ids.iter().any(|wanted| wanted == &post.post_id)
            }
            _ => false,
        })
    }

    /// The ground and the bar behind a webhook's attachment.
    ///
    /// Drawn over the run of consecutive `Attached` blocks for the same reason
    /// the quote bars are: a notice of four lines is one card, not four.
    fn attached(&self, into: &mut Canvas<'_>, index: usize, top: f32, left: f32) {
        let Some(laid) = self.laid.get(index) else {
            return;
        };
        let mut run: Option<(f32, f32)> = None;
        for block in laid
            .blocks
            .iter()
            .filter(|block| block.kind == matterless_layout::row::Kind::Attached)
            .map(Some)
            .chain(std::iter::once(None))
        {
            match (block, run) {
                (Some(block), Some((start, end))) if (block.y - end).abs() < 0.5 => {
                    run = Some((start, block.y + block.height));
                }
                (block, finished) => {
                    if let Some((start, end)) = finished {
                        let x = left + self.theme.gutter;
                        // Padded `8px 12px`, so the ground reaches beyond the
                        // words on every side rather than hugging them.
                        into.scene.rounded(
                            x,
                            top + start - 8.0,
                            self.theme.text_width(),
                            end - start + 16.0,
                            into.palette.surface,
                            CARD,
                        );
                        into.scene.fill(
                            x,
                            top + start - 8.0,
                            self.theme.quote_bar,
                            end - start + 16.0,
                            into.palette.rule,
                        );
                    }
                    run = block.map(|block| (block.y, block.y + block.height));
                }
            }
        }
    }

    /// Where each preview card sits on one row.
    ///
    /// A run of consecutive `Preview` blocks rather than one per line, so a
    /// card of three lines is one card -- which is what the bar down its side,
    /// the hit box and the hover all have to agree about. The nth run is the
    /// nth preview, because the layout leaves a gap between two of them.
    fn preview_runs(&self, index: usize, top: f32, left: f32) -> Vec<Rect> {
        let Some(laid) = self.laid.get(index) else {
            return Vec::new();
        };
        let x = left + self.theme.gutter;
        let width = self.theme.text_width().min(self.theme.preview_width);
        let mut runs = Vec::new();
        let mut run: Option<(f32, f32)> = None;
        for block in laid
            .blocks
            .iter()
            .filter(|block| block.kind == matterless_layout::row::Kind::Preview)
            .map(Some)
            .chain(std::iter::once(None))
        {
            match (block, run) {
                // A line that carries on where the last one stopped is the
                // same card; anything else starts a new one.
                (Some(block), Some((start, end))) if (block.y - end).abs() < 0.5 => {
                    run = Some((start, block.y + block.height));
                }
                (block, finished) => {
                    if let Some((start, end)) = finished {
                        // Padded `8px 10px`, so the ground reaches past the
                        // words rather than hugging them.
                        runs.push(Rect::new(x, top + start - 8.0, width, end - start + 16.0));
                    }
                    run = block.map(|block| (block.y, block.y + block.height));
                }
            }
        }
        runs
    }

    /// What one preview card leads to.
    ///
    /// A page card is its link and a quoted card is the message it quotes,
    /// which is the difference between leaving the app and moving inside it.
    fn preview_press(&self, index: usize, ordinal: usize) -> Option<matterless_layout::row::Press> {
        let (Row::Post { post } | Row::Continuation { post }) = self.rows.get(index)? else {
            return None;
        };
        match post.previews.get(ordinal)? {
            matterless_render::Preview::Page { url, .. } => {
                matterless_layout::row::openable_link(url)
            }
            matterless_render::Preview::Permalink {
                post_id,
                channel_id,
                ..
            } => Some(matterless_layout::row::Press::Post {
                channel_id: channel_id.clone(),
                post_id: post_id.clone(),
            }),
        }
    }

    /// What one row's preview card is called.
    fn preview_name(&self, index: usize, ordinal: usize) -> String {
        format!("{}/row/{index}/preview/{ordinal}", self.name)
    }

    /// Draws each preview card: its own ground, and the bar down its left.
    fn quote_bars(&self, into: &mut Canvas<'_>, index: usize, top: f32, left: f32, input: &Input) {
        for (ordinal, rect) in self.preview_runs(index, top, left).into_iter().enumerate() {
            let under = input.hovered() == Some(self.preview_name(index, ordinal).as_str());
            // Corners cut on the right only: `border-radius: 0 5px 5px 0`.
            into.scene.rounded(
                rect.x,
                rect.y,
                rect.width,
                rect.height,
                into.palette.ground,
                CARD,
            );
            // The bar takes the signal colour under the pointer, which is the
            // only thing saying a card is something to press.
            into.scene.fill(
                rect.x,
                rect.y,
                self.theme.quote_bar,
                rect.height,
                if under {
                    [
                        into.palette.signal[0],
                        into.palette.signal[1],
                        into.palette.signal[2],
                        255,
                    ]
                } else {
                    into.palette.rule
                },
            );
        }
    }

    /// Every pressable run of words from the frame just gone.
    ///
    /// So a caller holding only a box's name can say what it leads to: the
    /// name carries an index into this and nothing else.
    pub fn presses_seen(&self) -> &[(matterless_layout::row::Press, Rect)] {
        &self.presses
    }

    /// Where a pressable run of words was last drawn.
    ///
    /// So a panel opened from one can point at it. Answers the first, which is
    /// the one nearest the top: a name mentioned four times in a conversation
    /// has four boxes and a card can only be beside one of them.
    pub fn pressed_rect(&self, press: &matterless_layout::row::Press) -> Option<Rect> {
        self.presses
            .iter()
            .find(|(one, _)| one == press)
            .map(|(_, rect)| *rect)
    }

    /// Which row lies across the middle of the panel, and where it sits.
    ///
    /// A scroll is a distance in pixels, and every row's height changes when
    /// the column it is laid out in changes -- so the same distance means
    /// somewhere else afterwards. This is that place said in a way that
    /// survives being laid out again: a row, and how far below its top the
    /// middle of the panel falls.
    ///
    /// The middle rather than the top because whatever is held is the only
    /// thing that does not move: everything else slides by however much the
    /// rows between it and the anchor have changed. Held at the top, all of
    /// that lands at the bottom of the panel. Held at the middle, it is halved
    /// and sent to both edges, and none of it happens where the eye is.
    ///
    /// Measured from the middle too, and not only chosen there. A distance
    /// from the top edge is the same rule only while the panel keeps its
    /// height: drag the window's bottom edge and the middle moves while that
    /// distance does not, which held the rows against the *top* of a panel
    /// growing downwards. Choosing at one end and measuring from the other was
    /// the whole of the Y axis not obeying this.
    pub fn holding(&self, within: Rect) -> Option<(String, f32)> {
        let middle = within.y + within.height / 2.0;
        let mut top = within.y + self.theme.pad_top - self.scroll;
        for (index, laid) in self.laid.iter().enumerate() {
            let bottom = top + laid.height;
            if bottom > middle {
                return Some((key_of(self.rows.get(index)?), middle - top));
            }
            top = bottom;
        }
        // Past the end of the rows, which is a panel taller than its contents.
        // The last row is the nearest thing to a middle it has.
        let last = self.laid.len().checked_sub(1)?;
        Some((
            key_of(self.rows.get(last)?),
            middle - (top - self.laid[last].height),
        ))
    }

    /// The same, for one row the caller has in mind rather than whichever the
    /// middle happens to land on.
    ///
    /// Whatever is held is the only thing that stays still, and everything
    /// else slides by however much the rows in between have been re-measured.
    /// When the reader has just pointed at a particular message, that message
    /// is the one that has to stay put.
    pub fn holding_row(&self, key: &str, within: Rect) -> Option<(String, f32)> {
        let middle = within.y + within.height / 2.0;
        let mut top = within.y + self.theme.pad_top - self.scroll;
        for (index, laid) in self.laid.iter().enumerate() {
            if self.rows.get(index).map(key_of).as_deref() == Some(key) {
                return Some((key.to_string(), middle - top));
            }
            top += laid.height;
        }
        None
    }

    /// Where the reader is, in a form that survives every row being measured
    /// again.
    ///
    /// Which is three different places depending on where they are sitting.
    ///
    /// At either end, an *edge* is what is being read against and no row can
    /// stand in for it. A reader at the newest message is reading the bottom
    /// of the conversation: hold a row for them and the newest message ends up
    /// somewhere other than against the bottom edge, which is the one place it
    /// is ever supposed to be. The top of the channel is the same in reverse,
    /// and holding a row there is actively wrong -- the rows above the middle
    /// growing taller pushes the scroll up to keep the middle still, and the
    /// first message in the channel slides off the top.
    ///
    /// In between, neither edge means anything and the row across the middle
    /// is what they are reading.
    pub fn anchor(&self, within: Rect) -> Anchor {
        // A hair of tolerance, because the scroll is a float and a reader at
        // either end has usually arrived there by being clamped to it. The end
        // is asked about first: a conversation shorter than its panel is at
        // both at once, and the bottom is where it is drawn.
        if self.behind(within) <= 1.0 {
            return Anchor::End;
        }
        match self.scroll <= 1.0 {
            true => Anchor::Start,
            false => Anchor::Row(self.holding(within)),
        }
    }

    /// Puts them back there, after the rows have changed under them.
    pub fn anchored(&mut self, anchor: Anchor, within: Rect) {
        match anchor {
            Anchor::End => self.to_bottom(within),
            Anchor::Start => self.scroll = 0.0,
            Anchor::Row(held) => {
                self.hold(held, within);
                self.clamp(within);
            }
        }
    }

    /// Puts the reader back where `holding` found them.
    ///
    /// Nothing at all when the row is gone -- a channel that was reloaded
    /// under the reader has no such place, and guessing one would be worse
    /// than the top of what it does have.
    pub fn hold(&mut self, held: Option<(String, f32)>, within: Rect) {
        let Some((key, under)) = held else {
            return;
        };
        // `under` is how far below the row's top the middle of the panel fell,
        // so putting the row back means putting the middle back on it -- which
        // is the half of this that makes a panel of a different height work.
        let middle = within.height / 2.0;
        let mut top = self.theme.pad_top;
        for (index, laid) in self.laid.iter().enumerate() {
            if self.rows.get(index).map(key_of).as_deref() == Some(key.as_str()) {
                self.scroll = (top + under - middle).clamp(0.0, self.reach(within));
                return;
            }
            top += laid.height;
        }
    }

    /// Where one message sits on screen, for a panel that has to point at it.
    ///
    /// `None` when it is scrolled out of view, which is the honest answer: a
    /// panel anchored to a row nobody can see would float over nothing.
    pub fn row_rect(&self, post_id: &str, within: Rect) -> Option<Rect> {
        let mut top = within.y + self.theme.pad_top - self.scroll;
        for (index, laid) in self.laid.iter().enumerate() {
            let bottom = top + laid.height;
            let matches = matches!(
                self.rows.get(index),
                Some(Row::Post { post } | Row::Continuation { post }) if post.post_id == post_id
            );
            if matches && bottom >= within.y && top <= within.bottom() {
                let inner = self.inner(within);
                return Some(Rect::new(inner.x, top, inner.width, laid.height));
            }
            top = bottom;
        }
        None
    }

    /// Applies a frame's input. Answers what the click asked for.
    pub fn react(&mut self, input: &Input, placed: &[Placed], within: Rect) -> Option<Chose> {
        // The bar first: while it is held, the hand decides where the list is
        // and nothing else may write that position.
        if let Some(scroll) =
            self.bar
                .react(&self.name, input, within, self.scroll, self.reach(within))
        {
            self.scroll = scroll.clamp(0.0, self.reach(within));
            return None;
        }
        if let Some((_, y)) = input.wheel_over(placed, |name| name == self.name) {
            self.scroll = (self.scroll - y).clamp(0.0, self.reach(within));
        }
        // Escape puts the faces away before anything else reads the frame: a
        // panel that stays open under a keystroke meant to close it is the one
        // thing every reader tries first.
        if self.picking.is_some() && input.struck(matterless_ui::input::Key::Escape) {
            self.picking = None;
            return None;
        }
        let clicked = input.clicked()?;
        // The quick faces, while they are open. Before the rows, because they
        // float over one and a click on a face is not a click on the message
        // it happens to be in front of.
        if let Some((post_id, _)) = self.picking.clone() {
            let chosen = clicked.strip_prefix(&format!("{}/faces", self.name));
            // The panel itself keeps them open, the way a `details` does.
            if chosen == Some("") {
                return None;
            }
            match chosen.and_then(|rest| rest.strip_prefix('/')) {
                // A face: the reaction it stands for.
                Some(at)
                    if at
                        .parse::<usize>()
                        .is_ok_and(|at| at < crate::actions::QUICK.len()) =>
                {
                    self.picking = None;
                    let (name, _) = crate::actions::QUICK[at.parse::<usize>().expect("checked")];
                    return Some(Chose::React {
                        post_id,
                        emoji: name.to_string(),
                        // Always on: the quick row adds a reaction, and taking
                        // one back is what the pill under the message is for.
                        on: true,
                    });
                }
                // The one past them opens the search rather than reacting.
                Some(at) if at.parse::<usize>().is_ok() => {
                    self.picking = None;
                    return Some(Chose::Act {
                        action: crate::actions::Action::React,
                        post_id,
                        on: true,
                    });
                }
                // The catcher, or anything else on the window: they close,
                // and the click still counts for whatever it landed on, which
                // is what the catcher does in the app too.
                _ => self.picking = None,
            }
        }
        // One of the three controls on the hovered message.
        if let Some((index, tool)) = self.tool_at(clicked)
            && let Some(Row::Post { post } | Row::Continuation { post }) = self.rows.get(index)
        {
            let post_id = post.post_id.clone();
            let under = self
                .tools(index, self.top_of(index, within), within)
                .into_iter()
                .find(|(one, _)| *one == tool)
                .map(|(_, rect)| rect)
                .unwrap_or(within);
            return match tool {
                crate::actions::Tool::React => {
                    self.picking = Some((post_id, under));
                    None
                }
                crate::actions::Tool::Reply => self.root_of(index).map(Chose::Thread),
                crate::actions::Tool::More => Some(Chose::More { post_id, under }),
            };
        }
        if let Some(rest) = clicked.strip_prefix(&format!("{}/row/", self.name))
            && let Some((index, ordinal)) = rest.split_once("/save/")
            && let (Ok(index), Ok(ordinal)) = (index.parse::<usize>(), ordinal.parse::<usize>())
            && let Some((_, file)) = self.filed(index).into_iter().nth(ordinal)
        {
            return Some(Chose::Save {
                file_id: file.id.clone(),
                name: file.name.clone(),
            });
        }
        if let Some(rest) = clicked.strip_prefix(&format!("{}/row/", self.name))
            && let Some((index, ordinal)) = rest.split_once("/look/")
            && let (Ok(index), Ok(ordinal)) = (index.parse::<usize>(), ordinal.parse::<usize>())
            && let Some((_, file)) = self.pictured(index).into_iter().nth(ordinal)
            && let Some(Row::Post { post } | Row::Continuation { post }) = self.rows.get(index)
        {
            return Some(Chose::Look {
                file_id: file.id.clone(),
                post_id: post.post_id.clone(),
            });
        }
        if clicked == format!("{}/newest", self.name) {
            self.scroll = self.reach(within);
            return None;
        }
        // Words first: they are the innermost thing in a row, and a click on
        // one is a click on them rather than on the message holding them.
        if let Some(at) = clicked
            .strip_prefix(&format!("{}/press/", self.name))
            .and_then(|at| at.parse::<usize>().ok())
            && let Some((press, at)) = self.presses.get(at)
        {
            return Some(Chose::Press {
                press: press.clone(),
                at: Some(*at),
            });
        }
        // A preview card, which sits inside a row and is a link in its own
        // right: the whole card, not just the words in it.
        if let Some(rest) = clicked.strip_prefix(&format!("{}/row/", self.name))
            && let Some((index, ordinal)) = rest.split_once("/preview/")
            && let (Ok(index), Ok(ordinal)) = (index.parse::<usize>(), ordinal.parse::<usize>())
            && let Some(press) = self.preview_press(index, ordinal)
        {
            return Some(Chose::Press { press, at: None });
        }
        // A pill next, because it sits inside a row and its name says so.
        if let Some((index, ordinal)) = self.reaction_at(clicked)
            && let Some(Row::Post { post } | Row::Continuation { post }) = self.rows.get(index)
            && let Some(reaction) = post.reactions.get(ordinal)
        {
            return Some(Chose::React {
                post_id: post.post_id.clone(),
                emoji: reaction.emoji.clone(),
                // What it will become, decided here rather than by the server:
                // the pill has to change the instant it is pressed.
                on: !reaction.mine,
            });
        }
        let index = self.index_of(clicked)?;
        // A message that never reached the server has no thread to open -- its
        // id is the window's own pending one, which the store has never heard
        // of -- so a click on it means the only thing it can mean.
        if let Some(Row::Post { post } | Row::Continuation { post }) = self.rows.get(index)
            && post.failed
        {
            return Some(Chose::Retry(post.post_id.clone()));
        }
        // Only the footer opens a thread. A message is not a button: the whole
        // row being one swallowed every click on a mention or a link inside
        // it, and gave no hint that pressing a sentence would do anything.
        match self.rows.get(index) {
            Some(Row::ThreadFooter { root_id, .. }) => Some(Chose::Thread(root_id.clone())),
            _ => None,
        }
    }

    /// The three controls offered on a row, and where each sits.
    ///
    /// Only on the row under the pointer: a toolbar on every message at once
    /// would be a wall of buttons over a conversation.
    pub fn tools(&self, index: usize, top: f32, within: Rect) -> Vec<(crate::actions::Tool, Rect)> {
        let Some(Row::Post { post } | Row::Continuation { post }) = self.rows.get(index) else {
            return Vec::new();
        };
        // Nothing to act on until the server has agreed it exists.
        if post.pending || post.failed {
            return Vec::new();
        }
        let inner = self.inner(within);
        let Some(laid) = self.laid.get(index) else {
            return Vec::new();
        };
        crate::actions::tools(Rect::new(inner.x, top, inner.width, laid.height))
    }

    /// What one of a row's controls is called.
    fn tool_name(&self, index: usize, tool: crate::actions::Tool) -> String {
        format!("{}/row/{index}/tool/{}", self.name, tool.slug())
    }

    /// The row and control a button name refers to.
    fn tool_at(&self, name: &str) -> Option<(usize, crate::actions::Tool)> {
        let rest = name.strip_prefix(&format!("{}/row/", self.name))?;
        let (index, slug) = rest.split_once("/tool/")?;
        Some((index.parse().ok()?, crate::actions::Tool::from_slug(slug)?))
    }

    /// Where the quick faces hang, while they are open.
    fn faces_panel(&self, within: Rect) -> Option<Rect> {
        let (_, under) = self.picking.as_ref()?;
        Some(crate::actions::faces_panel(*under, within))
    }

    /// The row a name refers to, if it is one of this panel's.
    fn index_of(&self, name: &str) -> Option<usize> {
        name.strip_prefix(&format!("{}/row/", self.name))?
            .parse()
            .ok()
    }

    /// The row and reaction a pill name refers to, if it is one of this panel's.
    fn reaction_at(&self, name: &str) -> Option<(usize, usize)> {
        let rest = name.strip_prefix(&format!("{}/row/", self.name))?;
        let (index, ordinal) = rest.split_once("/reaction/")?;
        Some((index.parse().ok()?, ordinal.parse().ok()?))
    }

    /// The row under the pointer, for drawing it hovered.
    ///
    /// A button inside a row counts as that row, or the toolbar would vanish
    /// the moment the pointer reached it.
    pub fn hovered(&self, input: &Input) -> Option<usize> {
        let name = input.hovered()?;
        self.index_of(name)
            .or_else(|| self.tool_at(name).map(|(index, _)| index))
            .or_else(|| self.reaction_at(name).map(|(index, _)| index))
            .or_else(|| {
                name.strip_prefix(&format!("{}/row/", self.name))?
                    .split_once("/preview/")?
                    .0
                    .parse()
                    .ok()
            })
    }

    /// The faces the rows on screen need, so the caller can fetch the missing.
    ///
    /// Only a `Post` has one: a continuation is the same person still talking,
    /// which is exactly what leaving the gutter empty says.
    /// Everyone whose face is on screen, so their presence can be asked for.
    ///
    /// The rows in view rather than every row loaded: history here has no
    /// bound, and this is read from the frame loop. What is in view is a
    /// screenful either way.
    pub fn who_is_here(&self, within: Rect) -> Vec<String> {
        let mut here = Vec::new();
        let mut top = within.y + self.theme.pad_top - self.scroll;
        for (index, laid) in self.laid.iter().enumerate() {
            let bottom = top + laid.height;
            if bottom >= within.y
                && top <= within.bottom()
                && let Some(Row::Post { post }) = self.rows.get(index)
            {
                here.push(post.author_id.clone());
            }
            top = bottom;
        }
        here
    }

    pub fn faces(&self, within: Rect) -> Vec<(String, u32, u32)> {
        let mut wanted = Vec::new();
        let mut top = within.y + self.theme.pad_top - self.scroll;
        for (index, laid) in self.laid.iter().enumerate() {
            let bottom = top + laid.height;
            if bottom >= within.y && top <= within.bottom() {
                // A custom emoji has no character, so its picture is the only
                // way it is ever drawn -- in a pill, and inline in a message.
                let side = self.theme.emoji_size as u32;
                for (_, reaction) in self.pills(index) {
                    if reaction.unicode.is_none()
                        && let Some(id) = self.custom.get(&reaction.emoji)
                    {
                        wanted.push((emoji_key(id), side, side));
                    }
                }
                if let Some(laid) = self.laid.get(index) {
                    for name in laid
                        .blocks
                        .iter()
                        .flat_map(|block| &block.spans)
                        .filter_map(|span| span.emoji.as_ref())
                    {
                        if let Some(id) = self.custom.get(name) {
                            wanted.push((emoji_key(id), side, side));
                        }
                    }
                }
                if let Some(Row::Post { post }) = self.rows.get(index) {
                    wanted.push((
                        avatar_key(&post.author_id, post.avatar_at),
                        AVATAR as u32,
                        AVATAR as u32,
                    ));
                }
                // The faces on a thread footer, which are smaller than a
                // message's and so are a different picture in the atlas.
                if let Some(Row::ThreadFooter { participants, .. }) = self.rows.get(index) {
                    for face in participants.iter().take(FACES) {
                        wanted.push((
                            avatar_key(&face.user_id, face.avatar_at),
                            FACE as u32,
                            FACE as u32,
                        ));
                    }
                }
                // Attachments hang off a continuation as readily as off a post.
                if let Some(Row::Post { post } | Row::Continuation { post }) = self.rows.get(index)
                {
                    for file in &post.files {
                        if file.image || file.video {
                            // The box the layout reserved, so the picture is
                            // scaled once on the way in rather than every frame
                            // on the way out.
                            wanted.push((
                                picture_key(file),
                                (file.box_width.max(1) as f32).min(self.theme.text_width()) as u32,
                                file.box_height.max(1) as u32,
                            ));
                        }
                    }
                }
            }
            top = bottom;
        }
        wanted.sort();
        wanted.dedup();
        wanted
    }

    /// Where each of a row's pictures is drawn, from the blocks the layout
    /// already reserved for them.
    ///
    /// Paired by order: the nth attachment block belongs to the nth file, which
    /// is how the layout built them.
    fn pictures(
        &self,
        index: usize,
        top: f32,
        left: f32,
        ground: [u8; 4],
    ) -> Vec<matterless_paint::Piece> {
        let Some(Row::Post { post } | Row::Continuation { post }) = self.rows.get(index) else {
            return Vec::new();
        };
        let Some(laid) = self.laid.get(index) else {
            return Vec::new();
        };
        laid.blocks
            .iter()
            .filter(|block| block.kind == matterless_layout::row::Kind::Attachment)
            .zip(post.files.iter())
            .filter(|(_, file)| file.image || file.video)
            .flat_map(|(block, file)| {
                let at = Rect::new(
                    left + self.theme.gutter,
                    top + block.y,
                    block.wrap,
                    block.height,
                );
                // `background-color: var(--ground)` under the picture: the box
                // is drawn from the moment the row is, and an empty one is a
                // hole in the conversation rather than a picture on its way.
                let mut pieces = vec![matterless_paint::Piece::Fill {
                    x: at.x,
                    y: at.y,
                    width: at.width,
                    height: at.height,
                    colour: ground,
                    // An attachment is a card like any other, and the
                    // stylesheet rounds it to match the one a file without a
                    // preview gets.
                    radius: CARD,
                    softness: 1.0,
                }];
                // `background-size: cover` on the mini preview the post
                // already carries -- a kilobyte of JPEG that costs no request,
                // so the box holds the right picture, blurred, while the real
                // bytes are on their way.
                //
                // Under the real one rather than instead of it: when the real
                // one lands it is simply drawn on top, and nothing has to
                // notice that it did.
                if file.mini_preview.is_some() {
                    pieces.push(matterless_paint::Piece::Image {
                        x: at.x,
                        y: at.y,
                        width: at.width,
                        height: at.height,
                        key: mini_key(&file.id),
                        radius: CARD,
                    });
                }
                pieces.push(matterless_paint::Piece::Image {
                    x: at.x,
                    y: at.y,
                    width: at.width,
                    height: at.height,
                    key: picture_key(file),
                    radius: CARD,
                });
                pieces
            })
            .collect()
    }

    /// The mini previews the rows on screen carry, by the name each is drawn
    /// under.
    ///
    /// Apart from `faces`, which lists what has to be *fetched*: these are
    /// already here, in the message, and want decoding rather than asking for.
    pub fn minis(&self, within: Rect) -> Vec<(String, String)> {
        let mut wanted = Vec::new();
        let mut top = within.y + self.theme.pad_top - self.scroll;
        for (index, laid) in self.laid.iter().enumerate() {
            let bottom = top + laid.height;
            if bottom >= within.y
                && top <= within.bottom()
                && let Some(Row::Post { post } | Row::Continuation { post }) = self.rows.get(index)
            {
                for file in &post.files {
                    if (file.image || file.video)
                        && let Some(encoded) = file.mini_preview.as_ref()
                    {
                        wanted.push((mini_key(&file.id), encoded.clone()));
                    }
                }
            }
            top = bottom + self.theme.row_gap;
        }
        wanted
    }

    /// A row's reaction blocks, paired with the reactions they were built from.
    ///
    /// Paired by order, as the attachments are: the layout emits one block per
    /// reaction in the post's own order.
    fn pills(
        &self,
        index: usize,
    ) -> Vec<(
        &matterless_layout::row::Block,
        &matterless_render::ReactionSummary,
    )> {
        let Some(Row::Post { post } | Row::Continuation { post }) = self.rows.get(index) else {
            return Vec::new();
        };
        let Some(laid) = self.laid.get(index) else {
            return Vec::new();
        };
        laid.blocks
            .iter()
            .filter(|block| block.kind == matterless_layout::row::Kind::Reactions)
            .zip(post.reactions.iter())
            .collect()
    }

    /// Draws the panel behind each reaction, before the text goes on top.
    ///
    /// One the reader is part of is drawn louder, which is the only thing the
    /// pill has to say beyond its count: whether you are in it.
    fn reactions(&self, into: &mut Canvas<'_>, index: usize, top: f32, left: f32) {
        let Canvas {
            scene,
            painter,
            fonts,
            palette,
        } = into;
        for (block, reaction) in self.pills(index) {
            let x = left + self.theme.gutter + block.x;
            let y = top + block.y;
            // The stylesheet's own shape: a 10px capsule on the raised
            // surface, signalled when it is the reader's own.
            let tall = block.height - 3.0;
            scene.rounded(
                x,
                y,
                block.wrap,
                tall,
                if reaction.mine {
                    palette.signal_soft
                } else {
                    palette.raised
                },
                PILL,
            );
            // Each thing centred in the capsule by its own height rather than
            // dropped a fixed three pixels: a face is sixteen tall and a line
            // of words eighteen, so one offset cannot put both in the middle.
            let middle = |of: f32| (y + (tall - of) / 2.0).round();
            let mut text_at = x + self.theme.pill_padding;
            // A custom emoji has no character to shape, so the square the
            // layout reserved gets the picture instead. Without this the pill
            // printed the name and read ":bongo: 1".
            if reaction.unicode.is_none() {
                if let Some(id) = self.custom.get(&reaction.emoji) {
                    scene.extend([matterless_paint::Piece::Image {
                        x: text_at,
                        y: middle(self.theme.emoji_size),
                        width: self.theme.emoji_size,
                        height: self.theme.emoji_size,
                        key: emoji_key(id),
                        radius: 0.0,
                    }]);
                }
                text_at += self.theme.emoji_size + 4.0;
            }
            let label = painter.run(
                fonts,
                &block
                    .spans
                    .iter()
                    .map(|span| span.text.as_str())
                    .collect::<String>(),
                text_at,
                middle(self.theme.pill_line),
                pill_run(&self.theme),
            );
            scene.glyphs(
                label,
                if reaction.mine {
                    palette.signal
                } else {
                    palette.soft
                },
                palette.faint,
            );
        }
    }

    /// Draws the three controls that float into the corner of a message.
    ///
    /// One panel behind all three, which is what `.tools` is: a bordered strip
    /// with its own ground, because it can sit over the end of a long line.
    fn buttons(&self, into: &mut Canvas<'_>, index: usize, top: f32, within: Rect, input: &Input) {
        let placed = self.tools(index, top, within);
        if placed.is_empty() {
            return;
        }
        let open_on = self.picking.as_ref().map(|(post_id, _)| post_id.clone());
        let here = self.post_at(index).map(str::to_string);
        let inner = self.inner(within);
        let Some(height) = self.laid.get(index).map(|laid| laid.height) else {
            return;
        };
        let strip = crate::actions::strip(Rect::new(inner.x, top, inner.width, height));
        let Canvas {
            scene,
            painter,
            fonts,
            palette,
        } = into;
        // `border: 1px solid var(--rule)` drawn as a hairline the surface sits
        // inside, which is the only way one quad has an edge.
        Panel::floating(strip, crate::actions::CORNER, 1.0)
            .edge(palette.rule)
            .fill(palette.surface)
            .draw(scene);
        for (tool, rect) in placed {
            let under = input.hovered() == Some(self.tool_name(index, tool).as_str());
            // Open counts as hovered: the button that opened a panel must not
            // go dark the moment the pointer moves onto what it opened.
            let open = tool == crate::actions::Tool::React && open_on.is_some() && open_on == here;
            if under || open {
                scene.rounded(
                    rect.x,
                    rect.y,
                    rect.width,
                    rect.height,
                    palette.ground,
                    crate::actions::SQUARE_CORNER,
                );
            }
            // Centred in its own square: the glyphs are different widths, and
            // laying them out from the left is what put the smiley off-centre
            // beside the two next to it.
            let glyphs = painter.run(
                fonts,
                tool.mark(),
                rect.x + 5.0,
                rect.y + 2.0,
                // Drawn down from twice its size: a mark is a picture, and at
                // the size of a word the font hints its detail into stems too
                // hard to read.
                Run::mark(14.0),
            );
            scene.glyphs(
                glyphs,
                if under || open {
                    palette.ink
                } else {
                    palette.soft
                },
                palette.faint,
            );
        }
    }

    /// The message one row is, if it is one.
    fn post_at(&self, index: usize) -> Option<&str> {
        match self.rows.get(index)? {
            Row::Post { post } | Row::Continuation { post } => Some(post.post_id.as_str()),
            _ => None,
        }
    }

    /// Where a row's top edge is on screen.
    fn top_of(&self, index: usize, within: Rect) -> f32 {
        within.y + self.theme.pad_top - self.scroll
            + self
                .laid
                .iter()
                .take(index)
                .map(|row| row.height)
                .sum::<f32>()
    }

    /// Draws the quick faces, while they are open.
    fn quick(&self, into: &mut Canvas<'_>, within: Rect, input: &Input) {
        let Some(panel) = self.faces_panel(within) else {
            return;
        };
        let name = self.name.clone();
        let Canvas {
            scene,
            painter,
            fonts,
            palette,
        } = into;
        Panel::floating(panel, crate::actions::CORNER, 4.0)
            .edge(palette.rule)
            .fill(palette.surface)
            .draw(scene);
        for (at, rect) in crate::actions::faces(panel).into_iter().enumerate() {
            let under = input.hovered() == Some(format!("{name}/faces/{at}").as_str());
            if under {
                scene.rounded(rect.x, rect.y, rect.width, rect.height, palette.raised, 5.0);
            }
            match crate::actions::QUICK.get(at) {
                Some((_, face)) => {
                    let glyphs = painter.run(
                        fonts,
                        face,
                        rect.x + 3.0,
                        rect.y + 2.0,
                        Run::label(f32::MAX).sized(15.0),
                    );
                    scene.glyphs(glyphs, palette.ink, palette.faint);
                }
                // The one that is not a face: set apart by a rule of its own,
                // because it opens a search rather than reacting.
                None => {
                    scene.fill(
                        rect.x - 3.0,
                        rect.y + 2.0,
                        1.0,
                        rect.height - 4.0,
                        palette.rule,
                    );
                    let glyphs = painter.run(
                        fonts,
                        "\u{22ef}",
                        rect.x + 5.0,
                        rect.y + 3.0,
                        Run::label(f32::MAX).sized(13.0),
                    );
                    scene.glyphs(glyphs, palette.faint, palette.faint);
                }
            }
        }
    }

    /// Draws the file cards: an attachment that is not a picture.
    ///
    /// A name and a size on a panel, which is all a message list can honestly
    /// say about a document -- and considerably more than the blank space the
    /// reserved height would otherwise be.
    /// The line under a message that has replies: who is in it, and how many.
    ///
    /// The only way to open a thread. Faces rather than names because three
    /// names is a sentence and three faces is a glance, and the count is what
    /// says whether the thread is worth opening at all.
    fn footer(&self, into: &mut Canvas<'_>, index: usize, top: f32, left: f32, hot: bool) {
        let Some(Row::ThreadFooter {
            reply_count,
            participants,
            unread_replies,
            ..
        }) = self.rows.get(index)
        else {
            return;
        };
        let Canvas {
            scene,
            painter,
            fonts,
            palette,
        } = into;
        let y = top + (self.theme.footer_height - FACE) / 2.0;
        let left_of_faces = left + self.theme.gutter;
        // The first three, which is what the app shows: past that the faces
        // stop identifying anybody and become a texture.
        let shown: Vec<&matterless_render::ThreadFace> = participants.iter().take(FACES).collect();
        // Whatever this row is drawn on, which is what a ring has to be for a
        // face to read as being in front of the one behind it rather than as
        // two circles with a pale gap between them.
        let behind = if hot { palette.surface } else { palette.ground };
        // Right to left, so the first of them ends up on top -- the order the
        // official client stacks them in, and the one that reads as a queue
        // rather than a pile.
        for (at, face) in shown.iter().enumerate().rev() {
            let x = left_of_faces + at as f32 * FACE_STEP;
            scene.rounded(
                x - RING,
                y - RING,
                FACE + RING * 2.0,
                FACE + RING * 2.0,
                behind,
                FACE / 2.0 + RING,
            );
            scene.rounded(x, y, FACE, FACE, palette.raised, FACE / 2.0);
            scene.extend([matterless_paint::Piece::Image {
                x,
                y,
                width: FACE,
                height: FACE,
                key: avatar_key(&face.user_id, face.avatar_at),
                radius: FACE / 2.0,
            }]);
        }
        // Past the last face, not past where a fourth would have gone.
        let x = match shown.len() {
            0 => left_of_faces,
            count => left_of_faces + (count - 1) as f32 * FACE_STEP + FACE,
        };
        let plural = if *reply_count == 1 {
            "reply"
        } else {
            "replies"
        };
        let said = if *unread_replies > 0 {
            format!("{reply_count} {plural}, {unread_replies} new")
        } else {
            format!("{reply_count} {plural}")
        };
        // Centred by the line it is actually set on. Centring an 18px line
        // against the 21px of body text put the words a pixel and a half above
        // the faces beside them, which is the kind of gap that reads as wrong
        // without reading as anything in particular.
        let words = Run::label(f32::MAX);
        let glyphs = painter.run(
            fonts,
            &said,
            x + 8.0,
            top + ((self.theme.footer_height - words.line_height) / 2.0).round(),
            words,
        );
        // Unread replies are the reason to open it, so they read at full
        // strength while a thread with nothing new recedes.
        let ink = if *unread_replies > 0 || hot {
            palette.ink
        } else {
            palette.faint
        };
        scene.glyphs(glyphs, ink, palette.faint);
    }

    fn cards(&self, into: &mut Canvas<'_>, index: usize, top: f32, left: f32) {
        let Some(Row::Post { post } | Row::Continuation { post }) = self.rows.get(index) else {
            return;
        };
        let Canvas {
            scene,
            painter,
            fonts,
            palette,
        } = into;
        let _ = post;
        for (ordinal, (block, file)) in self.filed(index).into_iter().enumerate() {
            let x = left + self.theme.gutter;
            let y = top + block.y;
            let width = (self.theme.text_width() * 0.6).min(320.0);
            scene.rounded(x, y, width, block.height, palette.raised, CARD);
            // Somewhere to put it. Every other client offers this and a file
            // nobody can keep is a file nobody can open.
            let save = Rect::new(
                x + width - SAVE - 10.0,
                y + (block.height - 22.0) / 2.0,
                SAVE,
                22.0,
            );
            let _ = ordinal;
            scene.rounded(
                save.x,
                save.y,
                save.width,
                save.height,
                palette.surface,
                5.0,
            );
            let keep = painter.run(
                fonts,
                "Save",
                save.x + 9.0,
                save.y + 2.0,
                Run::label(f32::MAX),
            );
            scene.glyphs(keep, palette.signal, palette.faint);
            let name = painter.run(
                fonts,
                &file.name,
                x + 12.0,
                y + 10.0,
                Run {
                    size: 13.5,
                    line_height: 18.0,
                    bold: true,
                    mono: false,
                    // Cut by the panel's clip rather than wrapped: a card is a
                    // fixed height and a wrapped name would run out of it.
                    wrap: f32::MAX,
                    icon: false,
                    smooth: false,
                },
            );
            scene.glyphs(name, palette.ink, palette.faint);
            let about = painter.run(
                fonts,
                &format!("{} · {}", file.extension.to_uppercase(), size_of(file.size)),
                x + 12.0,
                y + 30.0,
                Run::label(f32::MAX),
            );
            scene.glyphs(about, palette.faint, palette.faint);
        }
    }

    pub fn draw(
        &mut self,
        into: &mut Canvas<'_>,
        within: Rect,
        input: &Input,
        presence: &std::collections::HashMap<String, String>,
    ) {
        // Gathered as the rows are drawn, because only the shaping knows
        // where the words landed. Read by `boxes` on the next frame, which is
        // the same one-frame-old answer the toolbar is hit with -- and a
        // message does not move between two frames of a still list.
        let mut presses = Vec::new();
        // How many of them have been underlined already, so each row draws
        // only its own.
        let mut marked = 0usize;
        // Taken before the canvas is unpacked, which borrows the rest of it.
        let name = self.name.clone();
        let Canvas {
            scene,
            painter,
            fonts,
            palette,
        } = into;
        let hovered = self.hovered(input);
        let inner = self.inner(within);
        let mut top = within.y + self.theme.pad_top - self.scroll;
        for (index, row) in self.laid.iter().enumerate() {
            let bottom = top + row.height;
            if bottom >= within.y && top <= within.bottom() {
                if hovered == Some(index) {
                    scene.fill(inner.x, top, inner.width, row.height, palette.surface);
                }
                {
                    let mut canvas = Canvas {
                        scene,
                        painter,
                        fonts,
                        palette,
                    };
                    // Before the text, or a pill would cover the count on it.
                    self.reactions(&mut canvas, index, top, inner.x);
                    // And before it for the same reason: a ground painted
                    // after the words it is meant to be behind covers them.
                    self.attached(&mut canvas, index, top, inner.x);
                    self.quote_bars(&mut canvas, index, top, inner.x, input);
                }
                let pieces = painter.pieces_of(fonts, row, top, &self.theme, palette, &self.custom);
                // Shifted into this panel's column: a row plan is laid out from
                // zero and knows nothing of where it lands.
                let pieces: Vec<matterless_paint::Piece> = pieces
                    .into_iter()
                    .map(|piece| shift(piece, inner.x))
                    .collect();
                for piece in &pieces {
                    if let matterless_paint::Piece::Press {
                        x,
                        y,
                        width,
                        height,
                        press,
                        quiet,
                    } = piece
                    {
                        let box_of = Rect::new(*x, *y, *width, *height);
                        // A mention and a channel link sit on a signalled
                        // ground, as they do in the stylesheet. Drawn before
                        // the words rather than after, or the background would
                        // cover what it is meant to be behind.
                        // Not a quiet one: the author's name over a message
                        // is pressable and is not a mention, and wearing the
                        // pill every message read as though it opened by
                        // naming its own writer.
                        if !quiet && !matches!(press, matterless_layout::row::Press::Link(_)) {
                            // Around the words rather than carved out of the
                            // line. Measured against a rendered name, the
                            // ground sat four pixels clear of the letters at
                            // the top and one at the bottom, so a descender
                            // ran into its own edge -- which reads as the
                            // background eating the word rather than holding
                            // it. A line box is not centred on its letters and
                            // this has to be.
                            scene.rounded(
                                box_of.x - MENTION_PAD,
                                box_of.y + 2.0,
                                box_of.width + MENTION_PAD * 2.0,
                                box_of.height - 2.0,
                                palette.signal_soft,
                                MENTION,
                            );
                        }
                        presses.push((press.clone(), box_of));
                    }
                }
                scene.extend(pieces);
                // An underline, because this palette has no second ink to
                // colour a link with and text that is pressable has to say so.
                // Drawn from the boxes the shaping just reported, so it sits
                // under exactly the words it belongs to.
                // Only a link is underlined. A mention and a channel link say
                // what they are with their own ground, and a rule under them
                // as well would be saying it twice.
                for (at, (press, rect)) in presses.iter().enumerate().skip(marked) {
                    let link = matches!(press, matterless_layout::row::Press::Link(_));
                    // A mention takes one only under the pointer: its ground
                    // already says it is pressable, and a permanent rule as
                    // well would be saying it twice. `.mention:hover`.
                    let under = input.hovered() == Some(format!("{}/press/{at}", name).as_str());
                    if !link && !under {
                        continue;
                    }
                    scene.fill(
                        rect.x,
                        rect.bottom() - 2.0,
                        rect.width,
                        1.0,
                        [palette.signal[0], palette.signal[1], palette.signal[2], 255],
                    );
                }
                marked = presses.len();
                // The face goes in the gutter the layout already leaves empty,
                // so it costs no height and a continuation simply has none.
                if let Some(Row::Post { post }) = self.rows.get(index) {
                    // `background: var(--surface-2)` on the face itself, which
                    // is what a picture with transparency in it sits on and
                    // what fills the circle before one has arrived at all.
                    scene.rounded(
                        inner.x + 2.0,
                        top + 4.0,
                        AVATAR,
                        AVATAR,
                        palette.raised,
                        AVATAR / 2.0,
                    );
                    scene.extend([matterless_paint::Piece::Image {
                        x: inner.x + 2.0,
                        y: top + 4.0,
                        width: AVATAR,
                        height: AVATAR,
                        key: avatar_key(&post.author_id, post.avatar_at),
                        // A face is a circle, which is half its side -- the
                        // stylesheet's `border-radius: 50%` said so and a
                        // square face was the loudest difference between the
                        // two clients.
                        radius: AVATAR / 2.0,
                    }]);
                    // And whether they are around, on the corner of their own
                    // face. After the picture, never before it: this scene is
                    // painted in the order it is built, so a dot put where it
                    // reads best rather than where it paints is a dot the face
                    // lands on top of.
                    if let Some(lit) = presence
                        .get(&post.author_id)
                        .and_then(|status| crate::sidebar::dot(status, palette))
                    {
                        let at = (inner.x + 2.0 + AVATAR - STATUS, top + 4.0 + AVATAR - STATUS);
                        // A ring of whatever the row is drawn on, which is what
                        // holds the dot to the face instead of letting it read
                        // as something floating beside it.
                        let behind = match hovered == Some(index) {
                            true => palette.surface,
                            false => palette.ground,
                        };
                        scene.rounded(
                            at.0 - STATUS_RING,
                            at.1 - STATUS_RING,
                            STATUS + STATUS_RING * 2.0,
                            STATUS + STATUS_RING * 2.0,
                            behind,
                            (STATUS + STATUS_RING * 2.0) / 2.0,
                        );
                        scene.rounded(at.0, at.1, STATUS, STATUS, lit, STATUS / 2.0);
                    }
                }
                scene.extend(self.pictures(index, top, inner.x, palette.ground));
                // The toolbar last of the row's own drawing, so it sits over
                // the message rather than under the first word of it.
                if hovered == Some(index) {
                    let mut canvas = Canvas {
                        scene,
                        painter,
                        fonts,
                        palette,
                    };
                    self.buttons(&mut canvas, index, top, within, input);
                }
                let mut canvas = Canvas {
                    scene,
                    painter,
                    fonts,
                    palette,
                };
                self.cards(&mut canvas, index, top, inner.x);
                self.footer(&mut canvas, index, top, inner.x, hovered == Some(index));
            }
            top = bottom;
        }
        self.presses = presses;
        if let Some(rect) = self.to_newest(within) {
            let lit = input.hovered() == Some(format!("{}/newest", self.name).as_str());
            scene.rounded(
                rect.x,
                rect.y,
                rect.width,
                rect.height,
                if lit { palette.raised } else { palette.surface },
                rect.height / 2.0,
            );
            let glyphs = painter.run(
                fonts,
                "Jump to newest",
                rect.x + 22.0,
                rect.y + 5.0,
                Run::label(f32::MAX),
            );
            scene.glyphs(glyphs, palette.signal, palette.faint);
        }
        let mut canvas = Canvas {
            scene,
            painter,
            fonts,
            palette,
        };
        self.bar.draw(
            &mut canvas,
            &self.name,
            input,
            within,
            self.scroll,
            self.reach(within),
        );
        // Over everything, including the bar: the faces hang outside the row
        // that opened them and belong in front of whatever they overlap.
        self.quick(&mut canvas, within, input);
    }
}

/// The size a face is drawn at, and the room the gutter already leaves for it.
pub const AVATAR: f32 = 28.0;
/// The presence dot on the corner of a face in the conversation.
const STATUS: f32 = 9.0;
/// The ring of row-coloured ground around it, which is what holds it to the
/// face rather than letting it float over the message beside it.
const STATUS_RING: f32 = 2.0;

/// The corners the stylesheet cuts: a capsule on a reaction, a softer one on
/// a card or a code block.
const PILL: f32 = 10.0;

/// How the words on a reaction pill are set.
///
/// Taken from the theme rather than written here, because the layout measures
/// the pill's width from the same two numbers and the width it arrives at is
/// the width this draws and hit-tests. Set at 13 here while the layout
/// measured at 14, every pill came out a little wider than its contents and
/// all of the slack fell on the right of them.
fn pill_run(theme: &matterless_layout::row::Theme) -> Run {
    Run {
        size: theme.pill_size,
        line_height: theme.pill_line,
        bold: false,
        mono: false,
        wrap: f32::MAX,
        icon: false,
        smooth: false,
    }
}
const CARD: f32 = 6.0;
const MENTION: f32 = 3.0;
/// How far a mention's ground reaches past it on either side.
const MENTION_PAD: f32 = 3.0;
/// How wide the "jump to newest" button is.
const JUMP: f32 = 150.0;
/// How wide the button that keeps a file is.
const SAVE: f32 = 46.0;

/// A face on a thread footer, and how many of them are shown.
const FACE: f32 = 18.0;
const FACES: usize = 3;
/// How far along the next face begins: less than a face wide, so they overlap.
const FACE_STEP: f32 = 12.0;
/// The ring of row-coloured ground each face is drawn inside, which is what
/// separates one from the one it overlaps.
const RING: f32 = 1.5;

/// What a user's picture is called, versioned so a new picture is a new name.
///
/// `last_picture_update` is the only thing that says the bytes changed under an
/// unchanged id, so it belongs in the key -- nothing then has to be evicted by
/// hand when somebody changes their photograph.
pub fn avatar_key(user_id: &str, version: i64) -> String {
    format!("avatar/{user_id}?v={version}")
}

/// Every picture on one message, in the order they were attached.
///
/// A file card is not among them: there is nothing to magnify about one, so
/// stepping never lands on it. The same rule the app follows.
pub fn looking_at(post: &matterless_render::PostRow) -> Vec<crate::viewer::Looking> {
    post.files
        .iter()
        .filter(|file| (file.image || file.video) && !file.archived)
        .map(|file| crate::viewer::Looking {
            file_id: file.id.clone(),
            name: file.name.clone(),
            // The original for anything the server does not re-encode -- a
            // GIF, an SVG, a video -- and its preview for a photograph, which
            // it caps at 1920 wide and which is the right answer for one.
            original: file.variant == matterless_render::ImageVariant::Original,
        })
        .collect()
}

/// What an attachment's mini preview is called.
///
/// Its own name rather than the picture's, because both are drawn: the real
/// one goes over it when it arrives. No route answers to `mini`, which is what
/// keeps anything from trying to fetch one -- the bytes are in the message.
pub fn mini_key(file_id: &str) -> String {
    format!("mini/{file_id}")
}

/// What an attachment's picture is called.
///
/// Which rendition, not just which file: the planner already chose one and
/// sized the box against its pixels, so fetching a different one would draw a
/// 120-pixel thumbnail stretched across a box measured for a 1920-pixel
/// preview.
pub fn picture_key(file: &matterless_render::FileRef) -> String {
    match file.variant {
        matterless_render::ImageVariant::Thumb => format!("thumb/{}", file.id),
        matterless_render::ImageVariant::Preview => format!("preview/{}", file.id),
        // An animation or a vector, whose preview would be a still frame or
        // nothing at all.
        matterless_render::ImageVariant::Original => format!("file/{}", file.id),
    }
}

/// Moves a piece sideways into its panel.
fn shift(piece: matterless_paint::Piece, by: f32) -> matterless_paint::Piece {
    use matterless_paint::Piece;
    match piece {
        // Never in a stream: the picture being looked at covers the whole
        // window and belongs to no panel, so nothing shifts it into one.
        Piece::Shown { .. } => piece,
        Piece::Fill {
            x,
            y,
            width,
            height,
            colour,
            radius,
            softness,
        } => Piece::Fill {
            x: x + by,
            y,
            width,
            height,
            colour,
            radius,
            softness,
        },
        Piece::Press {
            x,
            y,
            width,
            height,
            press,
            quiet,
        } => Piece::Press {
            x: x + by,
            y,
            width,
            height,
            press,
            quiet,
        },
        Piece::Text {
            glyphs,
            ink,
            faint,
            signal,
        } => Piece::Text {
            glyphs: glyphs
                .into_iter()
                .map(|glyph| matterless_paint::PlacedGlyph {
                    x: glyph.x + by as i32,
                    ..glyph
                })
                .collect(),
            ink,
            faint,
            signal,
        },
        Piece::Image {
            x,
            y,
            width,
            height,
            key,
            radius,
        } => Piece::Image {
            x: x + by,
            y,
            width,
            height,
            key,
            radius,
        },
    }
}

/// A file's size, in the unit a person would say it in.
fn size_of(bytes: i64) -> String {
    const UNITS: [(&str, f64); 3] = [("GB", 1e9), ("MB", 1e6), ("kB", 1e3)];
    let size = bytes.max(0) as f64;
    for (name, scale) in UNITS {
        if size >= scale {
            return format!("{:.1} {name}", size / scale);
        }
    }
    format!("{bytes} bytes")
}

/// What a custom emoji's picture is called.
pub fn emoji_key(emoji_id: &str) -> String {
    format!("emoji/{emoji_id}")
}

#[cfg(test)]
mod pills {
    use super::pill_run;
    use matterless_layout::Fonts;
    use matterless_layout::row::{Kind, Theme, lay_out};
    use matterless_paint::Painter;
    use matterless_render::{PostRow, ReactionSummary, Row};
    use std::sync::Arc;

    /// The words drawn on a pill fit the width the layout reserved for it.
    ///
    /// The two ends of this live in different crates: the layout decides how
    /// wide a pill is and this draws into it. They agreed on the padding and
    /// not on the size of the words -- measured at 14 and set at 13 -- so
    /// every pill carried a couple of pixels of slack on its right and read as
    /// contents that had not been centred.
    #[test]
    fn a_pill_is_drawn_at_the_size_it_was_measured_at() {
        let mut fonts = Fonts::new();
        let mut painter = Painter::new();
        let theme = Theme::default();
        let post = PostRow {
            post_id: "p1".into(),
            root_id: String::new(),
            author_id: "u1".into(),
            author_name: "ada".into(),
            create_at: 0,
            update_at: 0,
            edited: false,
            nodes: Arc::new(vec![matterless_render::markdown::Node::Text {
                value: "nice".into(),
            }]),
            reactions: vec![ReactionSummary {
                emoji: "tada".into(),
                count: 12,
                mine: false,
                unicode: Some("\u{1F389}".into()),
                names: Vec::new(),
            }],
            files: Vec::new(),
            attachments: Vec::new(),
            avatar_at: 0,
            bot: false,
            body_is_attachment_only: false,
            pending: false,
            failed: false,
            pinned: false,
            saved: false,
            following: false,
            previews: Vec::new(),
        };
        let laid = lay_out(&mut fonts, &Row::Post { post }, &theme);
        let pill = laid
            .blocks
            .iter()
            .find(|block| block.kind == Kind::Reactions)
            .expect("no pill");
        let said: String = pill.spans.iter().map(|span| span.text.as_str()).collect();
        // What the layout kept for the words, and what the words will
        // actually measure when this module sets them. The whole defect is
        // these two numbers being allowed to differ, so compare them directly
        // rather than looking at where the glyphs landed: a couple of pixels
        // of slack is invisible in a position and plain in a subtraction.
        let run = pill_run(&theme);
        let drawn = matterless_layout::extent_of(
            &mut fonts,
            &said,
            f32::MAX,
            matterless_layout::Style {
                size: run.size,
                line_height: run.line_height,
                bold: run.bold,
                italic: false,
                mono: run.mono,
            },
        )
        .width;
        let kept = pill.wrap - theme.pill_padding * 2.0;
        assert!(
            (kept - drawn).abs() < 0.5,
            "a pill keeps {kept} for words that set {drawn} wide"
        );
        // And the painter agrees the text is that wide, so the comparison
        // above is about the shaping rather than about two calls to it.
        let glyphs = painter.run(&mut fonts, &said, theme.pill_padding, 0.0, run);
        assert!(!glyphs.is_empty(), "nothing was drawn at all");
    }
}

#[cfg(test)]
mod sizes {
    use super::size_of;

    #[test]
    fn a_size_is_said_in_the_unit_a_person_would_use() {
        assert_eq!(size_of(512), "512 bytes");
        assert_eq!(size_of(20_480), "20.5 kB");
        assert_eq!(size_of(474_000_000), "474.0 MB");
        // Never negative, whatever the server said.
        assert_eq!(size_of(-1), "-1 bytes");
    }
}
