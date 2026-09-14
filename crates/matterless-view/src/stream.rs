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

use crate::sidebar::Canvas;
use matterless_layout::Fonts;
use matterless_layout::row::{RowLayout, Theme, lay_out};
use matterless_paint::Run;
use matterless_render::Row;
use matterless_ui::input::Input;
use matterless_ui::{Placed, Rect};

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
    Press(matterless_layout::row::Press),
    /// Keep a file somebody attached.
    Save { file_id: String, name: String },
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
        }
    }

    /// Lays every row out for this width.
    ///
    /// Once per width change, never per frame: a row's height does not depend
    /// on the scroll position, which is the property that makes this list
    /// honest and the DOM one not.
    pub fn lay_out(&mut self, fonts: &mut Fonts, width: f32) {
        self.theme = Theme {
            // The content's width, not the panel's: the stream keeps a margin
            // clear of its own edges, so a message never starts against the
            // sidebar nor ends against the scrollbar.
            width: width - Theme::default().pad_x * 2.0,
            // So a date in this year can leave the year off.
            today: crate::clock::today(),
            // The clock on every row, in the reader's own time rather than
            // UTC. Read here rather than held, so a machine that crosses a
            // daylight-saving boundary while running is right afterwards.
            utc_offset_minutes: crate::clock::utc_offset_minutes(),
            ..Theme::default()
        };
        self.laid = self
            .rows
            .iter()
            .map(|row| lay_out(fonts, row, &self.theme))
            .collect();
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
    pub fn to_post(&mut self, post_id: &str, within: Rect) -> bool {
        let mut top = 0.0;
        for (index, laid) in self.laid.iter().enumerate() {
            let found = matches!(
                self.rows.get(index),
                Some(Row::Post { post } | Row::Continuation { post }) if post.post_id == post_id
            );
            if found {
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
                for (ordinal, rect) in
                    self.preview_runs(index, top, inner.x).into_iter().enumerate()
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
        self.rows.iter().any(|row| match row {
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
    fn preview_press(
        &self,
        index: usize,
        ordinal: usize,
    ) -> Option<matterless_layout::row::Press> {
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
    fn quote_bars(
        &self,
        into: &mut Canvas<'_>,
        index: usize,
        top: f32,
        left: f32,
        input: &Input,
    ) {
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
                Some(at) if at.parse::<usize>().is_ok_and(|at| at < crate::actions::QUICK.len()) => {
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
                crate::actions::Tool::Reply => {
                    self.root_of(index).map(Chose::Thread)
                }
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
        if clicked == format!("{}/newest", self.name) {
            self.scroll = self.reach(within);
            return None;
        }
        // Words first: they are the innermost thing in a row, and a click on
        // one is a click on them rather than on the message holding them.
        if let Some(at) = clicked
            .strip_prefix(&format!("{}/press/", self.name))
            .and_then(|at| at.parse::<usize>().ok())
            && let Some((press, _)) = self.presses.get(at)
        {
            return Some(Chose::Press(press.clone()));
        }
        // A preview card, which sits inside a row and is a link in its own
        // right: the whole card, not just the words in it.
        if let Some(rest) = clicked.strip_prefix(&format!("{}/row/", self.name))
            && let Some((index, ordinal)) = rest.split_once("/preview/")
            && let (Ok(index), Ok(ordinal)) = (index.parse::<usize>(), ordinal.parse::<usize>())
            && let Some(press) = self.preview_press(index, ordinal)
        {
            return Some(Chose::Press(press));
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
            scene.rounded(
                x,
                y,
                block.wrap,
                block.height - 3.0,
                if reaction.mine {
                    palette.signal_soft
                } else {
                    palette.raised
                },
                PILL,
            );
            let mut text_at = x + self.theme.pill_padding;
            // A custom emoji has no character to shape, so the square the
            // layout reserved gets the picture instead. Without this the pill
            // printed the name and read ":bongo: 1".
            if reaction.unicode.is_none() {
                if let Some(id) = self.custom.get(&reaction.emoji) {
                    scene.extend([matterless_paint::Piece::Image {
                        x: text_at,
                        y: y + 3.0,
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
                y + 3.0,
                Run::label(f32::MAX),
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
        scene.floating(
            strip.x,
            strip.y,
            strip.width,
            strip.height,
            palette.rule,
            crate::actions::CORNER,
            1.0,
        );
        scene.rounded(
            strip.x + 1.0,
            strip.y + 1.0,
            strip.width - 2.0,
            strip.height - 2.0,
            palette.surface,
            crate::actions::CORNER - 1.0,
        );
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
                rect.x + 6.0,
                rect.y + 4.0,
                Run::label(f32::MAX).sized(14.0),
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
        scene.floating(
            panel.x,
            panel.y,
            panel.width,
            panel.height,
            palette.rule,
            crate::actions::CORNER,
            4.0,
        );
        scene.rounded(
            panel.x + 1.0,
            panel.y + 1.0,
            panel.width - 2.0,
            panel.height - 2.0,
            palette.surface,
            crate::actions::CORNER - 1.0,
        );
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
        let mut x = left + self.theme.gutter;
        // The first three, which is what the app shows: past that the faces
        // stop identifying anybody and become a texture.
        for face in participants.iter().take(FACES) {
            scene.rounded(x, y, FACE, FACE, palette.raised, FACE / 2.0);
            scene.extend([matterless_paint::Piece::Image {
                x,
                y,
                width: FACE,
                height: FACE,
                key: avatar_key(&face.user_id, face.avatar_at),
                radius: FACE / 2.0,
            }]);
            x += FACE + 3.0;
        }
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
        let glyphs = painter.run(
            fonts,
            &said,
            x + 5.0,
            top + (self.theme.footer_height - self.theme.line_height) / 2.0,
            Run::label(f32::MAX),
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

    pub fn draw(&mut self, into: &mut Canvas<'_>, within: Rect, input: &Input) {
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
                    } = piece
                    {
                        let box_of = Rect::new(*x, *y, *width, *height);
                        // A mention and a channel link sit on a signalled
                        // ground, as they do in the stylesheet. Drawn before
                        // the words rather than after, or the background would
                        // cover what it is meant to be behind.
                        if !matches!(press, matterless_layout::row::Press::Link(_)) {
                            scene.rounded(
                                box_of.x - 2.0,
                                box_of.y + 1.0,
                                box_of.width + 4.0,
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
        self.bar
            .draw(
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

/// The corners the stylesheet cuts: a capsule on a reaction, a softer one on
/// a card or a code block.
const PILL: f32 = 10.0;
const CARD: f32 = 6.0;
const MENTION: f32 = 3.0;
/// How wide the "jump to newest" button is.
const JUMP: f32 = 150.0;
/// How wide the button that keeps a file is.
const SAVE: f32 = 46.0;

/// A face on a thread footer, and how many of them are shown.
const FACE: f32 = 18.0;
const FACES: usize = 3;

/// What a user's picture is called, versioned so a new picture is a new name.
///
/// `last_picture_update` is the only thing that says the bytes changed under an
/// unchanged id, so it belongs in the key -- nothing then has to be evicted by
/// hand when somebody changes their photograph.
pub fn avatar_key(user_id: &str, version: i64) -> String {
    format!("avatar/{user_id}?v={version}")
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
        } => Piece::Press {
            x: x + by,
            y,
            width,
            height,
            press,
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
