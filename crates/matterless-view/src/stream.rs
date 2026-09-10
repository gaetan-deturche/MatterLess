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
#[derive(Debug, Clone, PartialEq, Eq)]
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
    /// Add or remove this reaction.
    React {
        post_id: String,
        emoji: String,
        /// What it becomes, decided here rather than waited for: a pill has to
        /// change the instant it is pressed.
        on: bool,
    },
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
        }
    }

    /// Lays every row out for this width.
    ///
    /// Once per width change, never per frame: a row's height does not depend
    /// on the scroll position, which is the property that makes this list
    /// honest and the DOM one not.
    pub fn lay_out(&mut self, fonts: &mut Fonts, width: f32) {
        self.theme = Theme {
            width,
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
        (self.total() - within.height).max(0.0)
    }

    /// Shows the newest message, which is where a conversation opens.
    pub fn to_bottom(&mut self, within: Rect) {
        self.scroll = self.reach(within);
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
        let mut placed = vec![Placed {
            name: self.name.clone(),
            rect: within,
            depth: 1,
        }];
        let mut top = within.y - self.scroll;
        for (index, row) in self.laid.iter().enumerate() {
            let bottom = top + row.height;
            if bottom >= within.y && top <= within.bottom() {
                placed.push(Placed {
                    name: self.row_name(index),
                    // Clipped to the panel, so a row half off the top is only
                    // hit where it can actually be seen.
                    rect: Rect::new(
                        within.x,
                        top.max(within.y),
                        within.width,
                        (bottom.min(within.bottom()) - top.max(within.y)).max(0.0),
                    ),
                    depth: 2,
                });
                // The first thing inside a message a pointer can land on.
                // Deeper than the row, so a click on a pill is a click on the
                // pill rather than on the message behind it.
                // Only the hovered row has a toolbar, so only it has buttons.
                if hovered == Some(index) {
                    for (action, rect) in self.toolbar(index, top, within) {
                        placed.push(Placed {
                            name: self.action_name(index, action),
                            rect,
                            depth: 3,
                        });
                    }
                }
                for (ordinal, (block, _)) in self.pills(index).into_iter().enumerate() {
                    placed.push(Placed {
                        name: format!("{}/row/{index}/reaction/{ordinal}", self.name),
                        rect: Rect::new(
                            within.x + self.theme.gutter + block.x,
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
        placed
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

    /// The bar beside each preview card.
    ///
    /// Drawn over the run of consecutive `Preview` blocks rather than per
    /// line, so a card of three lines has one bar down its side rather than
    /// three stubs with gaps between them.
    fn quote_bars(&self, into: &mut Canvas<'_>, index: usize, top: f32, left: f32) {
        let Some(laid) = self.laid.get(index) else {
            return;
        };
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
                        into.scene.fill(
                            left + self.theme.gutter,
                            top + start,
                            self.theme.quote_bar,
                            end - start,
                            into.palette.faint_fill(),
                        );
                    }
                    run = block.map(|block| (block.y, block.y + block.height));
                }
            }
        }
    }

    /// Where one message sits on screen, for a panel that has to point at it.
    ///
    /// `None` when it is scrolled out of view, which is the honest answer: a
    /// panel anchored to a row nobody can see would float over nothing.
    pub fn row_rect(&self, post_id: &str, within: Rect) -> Option<Rect> {
        let mut top = within.y - self.scroll;
        for (index, laid) in self.laid.iter().enumerate() {
            let bottom = top + laid.height;
            let matches = matches!(
                self.rows.get(index),
                Some(Row::Post { post } | Row::Continuation { post }) if post.post_id == post_id
            );
            if matches && bottom >= within.y && top <= within.bottom() {
                return Some(Rect::new(within.x, top, within.width, laid.height));
            }
            top = bottom;
        }
        None
    }

    /// Applies a frame's input. Answers what the click asked for.
    pub fn react(&mut self, input: &Input, placed: &[Placed], within: Rect) -> Option<Chose> {
        if let Some((_, y)) = input.wheel_over(placed, |name| name == self.name) {
            self.scroll = (self.scroll - y).clamp(0.0, self.reach(within));
        }
        let clicked = input.clicked()?;
        // A pill first, because it sits inside a row and its name says so.
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
        // Then a toolbar button, which also sits inside a row.
        if let Some((index, action)) = self.action_at(clicked)
            && let Some(Row::Post { post } | Row::Continuation { post }) = self.rows.get(index)
        {
            return Some(Chose::Act {
                action,
                post_id: post.post_id.clone(),
                on: match action {
                    crate::actions::Action::Save => !post.saved,
                    crate::actions::Action::Pin => !post.pinned,
                    _ => true,
                },
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
        self.root_of(index).map(Chose::Thread)
    }

    /// The actions offered on a row, and where each button sits.
    ///
    /// Only on the row under the pointer: a toolbar on every message at once
    /// would be a wall of buttons over a conversation.
    fn toolbar(&self, index: usize, top: f32, within: Rect) -> Vec<(crate::actions::Action, Rect)> {
        let Some(Row::Post { post } | Row::Continuation { post }) = self.rows.get(index) else {
            return Vec::new();
        };
        // Nothing to act on until the server has agreed it exists.
        if post.pending || post.failed {
            return Vec::new();
        }
        let strip = Rect::new(within.x, top + 2.0, within.width, crate::actions::HEIGHT);
        crate::actions::place(
            strip,
            &crate::actions::offered(post.author_id == self.me),
            // Measured against the same label that is drawn, so a button is
            // never narrower than the word inside it.
            |action| action.label(false).chars().count() as f32 * 7.0,
        )
    }

    /// What the toolbar of a row is called, per button.
    fn action_name(&self, index: usize, action: crate::actions::Action) -> String {
        format!("{}/row/{index}/action/{}", self.name, action.slug())
    }

    /// The row and action a button name refers to.
    fn action_at(&self, name: &str) -> Option<(usize, crate::actions::Action)> {
        let rest = name.strip_prefix(&format!("{}/row/", self.name))?;
        let (index, slug) = rest.split_once("/action/")?;
        Some((
            index.parse().ok()?,
            crate::actions::Action::from_slug(slug)?,
        ))
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
            .or_else(|| self.action_at(name).map(|(index, _)| index))
            .or_else(|| self.reaction_at(name).map(|(index, _)| index))
    }

    /// The faces the rows on screen need, so the caller can fetch the missing.
    ///
    /// Only a `Post` has one: a continuation is the same person still talking,
    /// which is exactly what leaving the gutter empty says.
    pub fn faces(&self, within: Rect) -> Vec<(String, u32, u32)> {
        let mut wanted = Vec::new();
        let mut top = within.y - self.scroll;
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
    fn pictures(&self, index: usize, top: f32, left: f32) -> Vec<matterless_paint::Piece> {
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
            .map(|(block, file)| matterless_paint::Piece::Image {
                x: left + self.theme.gutter,
                y: top + block.y,
                width: block.wrap,
                height: block.height,
                key: picture_key(file),
            })
            .collect()
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
            scene.fill(
                x,
                y,
                block.wrap,
                block.height - 3.0,
                if reaction.mine {
                    [palette.ink[0], palette.ink[1], palette.ink[2], 40]
                } else {
                    palette.surface
                },
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
            scene.glyphs(label, palette.ink, palette.faint);
        }
    }

    /// Draws the row's toolbar, one button at a time.
    fn buttons(&self, into: &mut Canvas<'_>, index: usize, top: f32, within: Rect, input: &Input) {
        let Some(Row::Post { post } | Row::Continuation { post }) = self.rows.get(index) else {
            return;
        };
        let Canvas {
            scene,
            painter,
            fonts,
            palette,
        } = into;
        for (action, rect) in self.toolbar(index, top, within) {
            let on = match action {
                crate::actions::Action::Save => post.saved,
                crate::actions::Action::Pin => post.pinned,
                _ => false,
            };
            let under = input.hovered() == Some(self.action_name(index, action).as_str());
            scene.fill(
                rect.x,
                rect.y,
                rect.width,
                rect.height,
                if under {
                    [palette.ink[0], palette.ink[1], palette.ink[2], 45]
                } else {
                    palette.surface
                },
            );
            let glyphs = painter.run(
                fonts,
                action.label(on),
                rect.x + 8.0,
                rect.y + 3.0,
                Run::label(f32::MAX),
            );
            // A button already done reads loud, so pressing it twice is
            // visibly two different things.
            scene.glyphs(
                glyphs,
                if on { palette.ink } else { palette.faint },
                palette.faint,
            );
        }
    }

    /// Draws the file cards: an attachment that is not a picture.
    ///
    /// A name and a size on a panel, which is all a message list can honestly
    /// say about a document -- and considerably more than the blank space the
    /// reserved height would otherwise be.
    fn cards(&self, into: &mut Canvas<'_>, index: usize, top: f32, left: f32) {
        let Some(Row::Post { post } | Row::Continuation { post }) = self.rows.get(index) else {
            return;
        };
        let Some(laid) = self.laid.get(index) else {
            return;
        };
        let Canvas {
            scene,
            painter,
            fonts,
            palette,
        } = into;
        for (block, file) in laid
            .blocks
            .iter()
            .filter(|block| block.kind == matterless_layout::row::Kind::Attachment)
            .zip(post.files.iter())
            .filter(|(_, file)| !file.image && !file.video)
        {
            let x = left + self.theme.gutter;
            let y = top + block.y;
            let width = (self.theme.text_width() * 0.6).min(320.0);
            scene.fill(x, y, width, block.height, palette.surface);
            let name = painter.run(
                fonts,
                &file.name,
                x + 12.0,
                y + 10.0,
                Run {
                    size: 13.5,
                    line_height: 18.0,
                    bold: true,
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

    pub fn draw(&self, into: &mut Canvas<'_>, within: Rect, input: &Input) {
        let Canvas {
            scene,
            painter,
            fonts,
            palette,
        } = into;
        let hovered = self.hovered(input);
        let mut top = within.y - self.scroll;
        for (index, row) in self.laid.iter().enumerate() {
            let bottom = top + row.height;
            if bottom >= within.y && top <= within.bottom() {
                if hovered == Some(index) {
                    scene.fill(within.x, top, within.width, row.height, palette.surface);
                }
                {
                    let mut canvas = Canvas {
                        scene,
                        painter,
                        fonts,
                        palette,
                    };
                    // Before the text, or a pill would cover the count on it.
                    self.reactions(&mut canvas, index, top, within.x);
                }
                let pieces = painter.pieces_of(fonts, row, top, &self.theme, palette, &self.custom);
                // Shifted into this panel's column: a row plan is laid out from
                // zero and knows nothing of where it lands.
                scene.extend(pieces.into_iter().map(|piece| shift(piece, within.x)));
                // The face goes in the gutter the layout already leaves empty,
                // so it costs no height and a continuation simply has none.
                if let Some(Row::Post { post }) = self.rows.get(index) {
                    scene.extend([matterless_paint::Piece::Image {
                        x: within.x + 2.0,
                        y: top + 4.0,
                        width: AVATAR,
                        height: AVATAR,
                        key: avatar_key(&post.author_id, post.avatar_at),
                    }]);
                }
                scene.extend(self.pictures(index, top, within.x));
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
                self.cards(&mut canvas, index, top, within.x);
                self.quote_bars(&mut canvas, index, top, within.x);
            }
            top = bottom;
        }
    }
}

/// The size a face is drawn at, and the room the gutter already leaves for it.
pub const AVATAR: f32 = 28.0;

/// What a user's picture is called, versioned so a new picture is a new name.
///
/// `last_picture_update` is the only thing that says the bytes changed under an
/// unchanged id, so it belongs in the key -- nothing then has to be evicted by
/// hand when somebody changes their photograph.
pub fn avatar_key(user_id: &str, version: i64) -> String {
    format!("avatar/{user_id}?v={version}")
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
        } => Piece::Fill {
            x: x + by,
            y,
            width,
            height,
            colour,
        },
        Piece::Text { glyphs, ink, faint } => Piece::Text {
            glyphs: glyphs
                .into_iter()
                .map(|glyph| matterless_paint::PlacedGlyph {
                    x: glyph.x + by as i32,
                    ..glyph
                })
                .collect(),
            ink,
            faint,
        },
        Piece::Image {
            x,
            y,
            width,
            height,
            key,
        } => Piece::Image {
            x: x + by,
            y,
            width,
            height,
            key,
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
