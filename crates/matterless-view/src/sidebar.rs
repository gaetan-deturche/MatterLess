//! The channel list, built from boxes rather than HTML.
//!
//! The first piece of chrome to be ported, and chosen deliberately: it is a
//! list of rows with hover and selection, which exercises layout, hit testing,
//! input and drawing together -- and needs no text entry, which is the part
//! that still has to be built.
//!
//! It decides nothing about where it sits. The caller hands it a rectangle and
//! it fills it, so the same widget works beside a thread pane or without one.

use matterless_paint::Run;
use matterless_ui::input::Input;
use matterless_ui::{Axis, Node, Placed, Rect, Size};

/// What a widget draws with.
///
/// Lives in `matterless-widgets` now, beside the controls that draw into it.
/// Re-exported from here because this is where every other widget imports it
/// from, and moving thirty import lines is a change about nothing.
pub use matterless_widgets::Canvas;

/// One line in the list: a channel, or the heading of a group.
#[derive(Debug, Clone)]
pub enum Entry {
    /// Who the reader is, and whether this window is hearing anything.
    ///
    /// The whole top of the sidebar in the app, and the only place either of
    /// those is said: a client that has quietly stopped receiving looks
    /// exactly like a quiet afternoon.
    Me {
        name: String,
        status: String,
        live: bool,
    },
    /// A team's name, above the groups that belong to it.
    Team { id: String, label: String },
    /// A group's name -- "Favourites", "Channels", "Direct messages".
    ///
    /// `directs` marks the one the rail's envelope points at: it is the only
    /// group that belongs to no team and so the only one the rail cannot
    /// reach by a team's id.
    Heading { label: String, directs: bool },
    /// The threads this reader follows, as a row of the list.
    ///
    /// First, and shaped like a channel, because that is what it is: a place
    /// to go and read, chosen the same way and opening in the same column. It
    /// is also the only row that can say a thread is waiting -- under collapsed
    /// threads a reply never touches its channel's counters, so without this
    /// the sidebar stays silent while the badge counts it.
    Threads { unread: i64, mentions: i64 },
    Channel {
        id: String,
        label: String,
        unread: i64,
        mentions: i64,
        muted: bool,
        /// A direct or group message, which is sigilled by a person rather than
        /// by a hash.
        direct: bool,
        /// A channel not everybody can see, which the app marks with a lock.
        private: bool,
        /// The other person, for a one-to-one conversation. `None` for a
        /// channel and for a group, which has no single other person to be
        /// around or not.
        counterpart: Option<String>,
        /// When that person last changed their picture.
        ///
        /// Part of what the picture is called, so a new photograph is a new
        /// name and nothing has to be evicted by hand -- and the same name the
        /// conversation uses, so one fetch serves both.
        counterpart_avatar_at: i64,
    },
}

/// The list, and where the reader is in it.
pub struct Sidebar {
    pub entries: Vec<Entry>,
    pub selected: Option<String>,
    /// How far down the list has been scrolled, in pixels.
    pub scroll: f32,
    /// Its own bar. A hundred and fourteen channels is a scroll, and a list
    /// that scrolls with nothing to say how far is a list you get lost in.
    pub bar: crate::scrollbar::Scrollbar,
}

/// What the threads row is selected as.
///
/// Not a channel id and never mistakable for one: the server's ids are twenty-
/// six characters of base-32, so no channel can ever be called this. Holding it
/// in the same `selected` as a channel is deliberate -- it is chosen the same
/// way, drawn the same way, and opens in the same column, so it should not need
/// a second piece of state saying which kind of thing is open.
pub const THREADS: &str = "threads";

/// A row's height, and a heading's. Fixed, because a channel name is one line
/// and a list of five hundred of them must not need measuring to be scrolled.
const ROW: f32 = 26.0;
const HEADING: f32 = 26.0;
/// The strip at the very top: a name on one line and a status on the next.
const ME: f32 = 52.0;
/// A team's name, above the groups belonging to it.
const TEAM: f32 = 30.0;
const PADDING: f32 = 8.0;

impl Sidebar {
    pub fn new(entries: Vec<Entry>) -> Self {
        Self {
            entries,
            selected: None,
            scroll: 0.0,
            bar: crate::scrollbar::Scrollbar::default(),
        }
    }

    /// The name a row answers to when it is hit.
    fn name_of(entry: &Entry, index: usize) -> String {
        match entry {
            Entry::Me { .. } => "sidebar/me".to_string(),
            Entry::Team { id, .. } => format!("sidebar/team/{id}"),
            Entry::Heading { .. } => format!("sidebar/heading/{index}"),
            Entry::Threads { .. } => format!("sidebar/channel/{THREADS}"),
            Entry::Channel { id, .. } => format!("sidebar/channel/{id}"),
        }
    }

    /// The whole list as one column of boxes, offset by the scroll.
    ///
    /// Every row is placed, including the ones above and below the rectangle.
    /// A sidebar is hundreds of rows, not the hundreds of thousands the message
    /// list holds, so windowing it would be machinery for no gain -- and rows
    /// off the top are what a hit test must *not* find, which the caller gets
    /// for free by clipping.
    pub fn boxes(&self, within: Rect) -> Vec<Placed> {
        let mut column = Node::new("sidebar", Size::Grow(1.0))
            .axis(Axis::Column)
            .padding(PADDING);
        for (index, entry) in self.entries.iter().enumerate() {
            let height = height_of(entry);
            column = column.with(Node::new(Self::name_of(entry, index), Size::Fixed(height)));
        }
        // Scrolling moves the box the rows are solved inside, so every row moves
        // with it and the arithmetic stays in one place.
        let scrolled = Rect::new(
            within.x,
            within.y - self.scroll,
            within.width,
            within.height + self.scroll,
        );
        let mut placed = matterless_ui::solve::solve(&column, scrolled);
        // The one button in the list, on the heading the conversations with
        // people sit under.
        for (index, entry) in self.entries.iter().enumerate() {
            if !matches!(entry, Entry::Heading { directs: true, .. }) {
                continue;
            }
            let name = Self::name_of(entry, index);
            if let Some(row) = placed.iter().find(|item| item.name == name) {
                let at = plus_rect(row.rect, within);
                placed.push(Placed {
                    name: NEW.to_string(),
                    rect: at,
                    depth: 3,
                });
            }
        }
        placed.extend(self.bar.boxes("sidebar", within, self.reach(within)));
        // The panel itself keeps its real rectangle: it is what the wheel is
        // tested against, and a scrolled one would stop matching the pointer.
        if let Some(panel) = placed.first_mut() {
            panel.rect = within;
        }
        placed
    }

    /// The pictures this list wants, so the caller can fetch them the usual
    /// way.
    ///
    /// The team icons -- the same ones the rail shows, under the same name, so
    /// they are asked for at one size and drawn at two. An atlas holds one
    /// picture per name, so asking for a second size here would mean whichever
    /// panel got there first decided the size for both.
    pub fn wants(&self) -> Vec<(String, u32, u32)> {
        self.entries
            .iter()
            .filter_map(|entry| match entry {
                Entry::Team { id, .. } => Some((
                    crate::rail::icon_key(id),
                    crate::rail::ICON_FETCHED,
                    crate::rail::ICON_FETCHED,
                )),
                // The other person's face, under the name the conversation
                // uses: asked for at the size the messages want, since one
                // picture serves both and the larger is the one worth holding.
                Entry::Channel {
                    counterpart: Some(who),
                    counterpart_avatar_at,
                    ..
                } => Some((
                    crate::stream::avatar_key(who, *counterpart_avatar_at),
                    crate::stream::AVATAR as u32,
                    crate::stream::AVATAR as u32,
                )),
                _ => None,
            })
            .collect()
    }

    /// Scrolls a team's heading to the top of the panel.
    ///
    /// What the rail does: a team is not somewhere to be, it is somewhere in
    /// the list, and taking the reader there is different from opening a
    /// conversation they did not ask for.
    /// The next conversation with something waiting in it, from where the
    /// reader is standing.
    ///
    /// Three decisions, and each of them has a reason rather than a
    /// preference behind it:
    ///
    /// **Muted rows are passed over.** Muted is the reader saying this
    /// conversation should not interrupt them, and a key that walks them to
    /// it is an interruption they asked not to have. The rail's dots already
    /// leave them out, so counting them here would make the badge and the key
    /// disagree about what is waiting.
    ///
    /// **It wraps.** Without it the last unread row is a dead end where the
    /// key stops working, and nothing on screen says why.
    ///
    /// **Direct messages are in the same walk**, because they are rows of the
    /// same list in the reader's own order. Two walks would need two gestures
    /// for one question.
    ///
    /// Counted from where the reader *is* rather than from the last row
    /// answered: they are usually standing in a conversation with nothing
    /// waiting in it, having just read it.
    pub fn next_unread(&self, on: bool) -> Option<&str> {
        let many = self.entries.len();
        if many == 0 {
            return None;
        }
        let here = self
            .entries
            .iter()
            .position(|entry| match entry {
                Entry::Channel { id, .. } => Some(id.as_str()) == self.selected.as_deref(),
                Entry::Threads { .. } => self.selected.as_deref() == Some(THREADS),
                _ => false,
            })
            .unwrap_or(0);
        // Every row once, starting with the one after this and ending with
        // this one -- so a walk from the only waiting row comes back to it
        // rather than answering nothing.
        (1..=many).find_map(|step| {
            let at = match on {
                true => (here + step) % many,
                // Twice the length keeps this positive without a signed cast,
                // since `step` never exceeds it.
                false => (here + many * 2 - step) % many,
            };
            self.waiting_at(at)
        })
    }

    /// The row at `at`, if it is somewhere to go with something in it.
    ///
    /// The threads row counts. It is shaped like a channel because that is
    /// what it is -- somewhere to go and read -- and it is the only row that
    /// can say a reply is waiting under a collapsed thread.
    fn waiting_at(&self, at: usize) -> Option<&str> {
        match self.entries.get(at)? {
            Entry::Channel {
                id,
                unread,
                mentions,
                muted,
                ..
            } if !muted && (*unread > 0 || *mentions > 0) => Some(id.as_str()),
            Entry::Threads { unread, mentions } if *unread > 0 || *mentions > 0 => Some(THREADS),
            _ => None,
        }
    }

    pub fn scroll_to(&mut self, wanted: &str, within: Rect) {
        let mut above = PADDING;
        for entry in &self.entries {
            let hit = match entry {
                Entry::Team { id, .. } => id == wanted,
                Entry::Heading { directs, .. } => *directs && wanted == crate::rail::DIRECTS,
                _ => false,
            };
            if hit {
                self.scroll = above.clamp(0.0, self.reach(within));
                return;
            }
            above += height_of(entry);
        }
    }

    /// How far the list can be scrolled before it runs out.
    pub fn reach(&self, within: Rect) -> f32 {
        let content: f32 = self.entries.iter().map(height_of).sum();
        (content + PADDING * 2.0 - within.height).max(0.0)
    }

    /// Applies a frame's input: what was clicked, and how far the wheel turned.
    ///
    /// Answers the channel the reader chose, if they chose one.
    pub fn react(&mut self, input: &Input, placed: &[Placed], within: Rect) -> Option<String> {
        // The bar first: while it is held nothing else may write the scroll.
        if let Some(scroll) =
            self.bar
                .react("sidebar", input, within, self.scroll, self.reach(within))
        {
            self.scroll = scroll.clamp(0.0, self.reach(within));
            return None;
        }
        if let Some((_, y)) = input.wheel_over(placed, |name| name == "sidebar") {
            self.scroll = (self.scroll - y).clamp(0.0, self.reach(within));
        }
        let clicked = input.clicked()?;
        let id = clicked.strip_prefix("sidebar/channel/")?;
        // Only inside the panel: a row scrolled above the top is still placed,
        // and clicking where it would have been must not select it.
        let row = placed.iter().find(|item| item.name == clicked)?;
        if row.rect.y < within.y || row.rect.bottom() > within.bottom() {
            return None;
        }
        self.selected = Some(id.to_string());
        Some(id.to_string())
    }

    /// Everything drawing a panel needs that is not the panel itself.
    ///
    /// Together rather than as six arguments: a scene to draw into, the tools
    /// to shape text with, and the colours. Every widget will want the same
    /// four, which is what makes them one thing.
    pub fn draw(
        &self,
        into: &mut Canvas<'_>,
        placed: &[Placed],
        within: Rect,
        input: &Input,
        presence: &std::collections::HashMap<String, String>,
    ) {
        let Canvas {
            scene,
            painter,
            fonts,
            palette,
        } = into;
        scene.fill(
            within.x,
            within.y,
            within.width,
            within.height,
            palette.surface,
        );
        // The rule between the sidebar and the conversation, which is what the
        // app draws and what says they are two surfaces rather than one.
        scene.fill(
            within.right() - 1.0,
            within.y,
            1.0,
            within.height,
            palette.rule,
        );
        for (index, entry) in self.entries.iter().enumerate() {
            let name = Self::name_of(entry, index);
            let Some(row) = placed.iter().find(|item| item.name == name) else {
                continue;
            };
            // Clipped by hand: a row half off the top would otherwise be drawn
            // over the panel above this one.
            if row.rect.bottom() <= within.y || row.rect.y >= within.bottom() {
                continue;
            }
            match entry {
                // Who the reader is, said once at the top rather than in a
                // channel's title: whether this window is hearing anything has
                // nothing to do with which conversation is open.
                Entry::Me { name, status, live } => {
                    let who = painter.run(
                        fonts,
                        name,
                        row.rect.x + 4.0,
                        row.rect.y + 10.0,
                        Run::label(f32::MAX).bold(),
                    );
                    scene.glyphs(who, palette.ink, palette.faint);
                    if let Some(lit) = dot(status, palette) {
                        scene.rounded(
                            row.rect.x + 4.0,
                            row.rect.y + 32.0,
                            DOT,
                            DOT,
                            lit,
                            DOT / 2.0,
                        );
                    }
                    let said = painter.run(
                        fonts,
                        &format!(
                            "{}{}",
                            spoken(status),
                            if *live { "" } else { " -- offline" }
                        ),
                        row.rect.x + 4.0 + DOT + 6.0,
                        row.rect.y + 28.0,
                        Run::label(f32::MAX),
                    );
                    scene.glyphs(
                        said,
                        if *live { palette.soft } else { palette.danger },
                        palette.faint,
                    );
                }
                // A team's name leads the groups that belong to it, rather than
                // every group's name carrying it: two teams each bring a
                // "Favorites" and a "Channels", and unqualified they read as
                // duplicates -- but qualifying each one says the team four
                // times over.
                Entry::Team { id, label } => {
                    // The team's own picture, which tells two teams apart
                    // faster than their names do. A tile behind it, so one
                    // with transparency in it -- or one that has not arrived
                    // yet -- reads as a tile rather than as a hole.
                    scene.rounded(
                        row.rect.x + 4.0,
                        row.rect.y + 6.0,
                        TEAM_ICON,
                        TEAM_ICON,
                        palette.raised,
                        TEAM_ICON_CORNER,
                    );
                    scene.extend([matterless_paint::Piece::Image {
                        x: row.rect.x + 4.0,
                        y: row.rect.y + 6.0,
                        width: TEAM_ICON,
                        height: TEAM_ICON,
                        key: crate::rail::icon_key(id),
                        radius: TEAM_ICON_CORNER,
                    }]);
                    let glyphs = painter.run(
                        fonts,
                        label,
                        row.rect.x + 4.0 + TEAM_ICON + 7.0,
                        row.rect.y + 8.0,
                        Run {
                            size: 14.5,
                            line_height: 20.0,
                            bold: true,
                            mono: false,
                            wrap: f32::MAX,
                            icon: false,
                            smooth: false,
                        },
                    );
                    scene.glyphs(glyphs, palette.ink, palette.faint);
                }
                Entry::Heading { label, directs } => {
                    let glyphs = painter.run(
                        fonts,
                        &small_caps(label),
                        row.rect.x + 4.0,
                        row.rect.y + 8.0,
                        Run {
                            size: 10.5,
                            line_height: 14.0,
                            bold: true,
                            mono: false,
                            wrap: f32::MAX,
                            icon: false,
                            smooth: false,
                        },
                    );
                    scene.glyphs(glyphs, palette.faint, palette.faint);
                    // Starting a conversation belongs beside the
                    // conversations, which is where the app puts it: on the
                    // one group that is people rather than channels.
                    if *directs {
                        let at = plus_rect(row.rect, within);
                        let lit = input.hovered() == Some(NEW);
                        let glyphs = painter.run(
                            fonts,
                            matterless_layout::marks::NEW,
                            at.x + 3.0,
                            at.y + 1.0,
                            // Drawn down from twice its size: a mark is a
                            // picture, and at the size of a word the font
                            // hints its detail into hard stems.
                            Run::mark(12.0),
                        );
                        scene.glyphs(
                            glyphs,
                            if lit { palette.ink } else { palette.soft },
                            palette.faint,
                        );
                    }
                }
                Entry::Channel {
                    id,
                    label,
                    unread,
                    mentions,
                    muted,
                    direct,
                    private,
                    counterpart,
                    counterpart_avatar_at,
                } => {
                    let chosen = self.selected.as_deref() == Some(id.as_str());
                    if chosen || input.hovered() == Some(name.as_str()) {
                        scene.fill(
                            row.rect.x,
                            row.rect.y,
                            row.rect.width,
                            row.rect.height,
                            palette.ground,
                        );
                    }
                    let loud = asking(*unread, *mentions, *muted);
                    let ink = ink_of(palette, chosen, loud, *muted);
                    let box_of = Rect::new(
                        row.rect.x + ICON_LEFT,
                        row.rect.y + (row.rect.height - ICON) / 2.0,
                        ICON,
                        ICON,
                    );
                    // What kind of conversation this is, in its own box
                    // beside the name rather than spliced onto the front of
                    // it: a channel's type is something about the channel, and
                    // a character sitting inside the name reads as part of it.
                    let behind = if chosen || input.hovered() == Some(name.as_str()) {
                        palette.ground
                    } else {
                        palette.surface
                    };
                    match counterpart.as_deref() {
                        // A one-to-one conversation is labelled by the person
                        // it is with. A face is quicker to recognise than a
                        // name is to read, which is the whole reason the app
                        // puts one here rather than a mark.
                        Some(who) => {
                            // `background: var(--surface-2)` under it, which is
                            // what a picture with transparency sits on and what
                            // fills the circle before one has arrived.
                            scene.rounded(
                                box_of.x,
                                box_of.y,
                                FACE,
                                FACE,
                                palette.raised,
                                FACE / 2.0,
                            );
                            scene.extend([matterless_paint::Piece::Image {
                                x: box_of.x,
                                y: box_of.y,
                                width: FACE,
                                height: FACE,
                                key: crate::stream::avatar_key(who, *counterpart_avatar_at),
                                // `border-radius: 50%`, which is half its side.
                                radius: FACE / 2.0,
                            }]);
                        }
                        None => kind_icon(
                            scene,
                            painter,
                            fonts,
                            palette,
                            box_of,
                            Kind::of(*direct, *private, false),
                            behind,
                        ),
                    }
                    // Whether somebody is around, which is the one thing that
                    // decides whether you write to them now.
                    //
                    // This said for a long time that it belonged only here,
                    // and that a dot against every message would turn the
                    // margin into a light display. The official client puts
                    // one on the face on each message and it does not, so the
                    // stream draws them too -- see `Stream::draw`.
                    //
                    // After the face and not before it. The scene is painted in
                    // the order it is built, so a dot drawn first is a dot the
                    // picture lands on top of -- which is what happened.
                    //
                    // On the corner of the face rather than out in the margin,
                    // so it reads as belonging to that person, with a ring in
                    // the panel's own colour holding it to the face instead of
                    // letting it float over the next row.
                    if let Some(lit) = counterpart
                        .as_deref()
                        .and_then(|who| presence.get(who))
                        .and_then(|status| dot(status, palette))
                    {
                        let at = (box_of.x + FACE - DOT + 1.0, box_of.y + FACE - DOT + 1.0);
                        scene.rounded(
                            at.0 - RING,
                            at.1 - RING,
                            DOT + RING * 2.0,
                            DOT + RING * 2.0,
                            palette.surface,
                            (DOT + RING * 2.0) / 2.0,
                        );
                        scene.rounded(at.0, at.1, DOT, DOT, lit, DOT / 2.0);
                    }
                    // The count first, because it decides how much room the
                    // name has. Muted channels keep theirs: muting is "do not
                    // interrupt me", not "hide this from me".
                    let pill = pill_for(fonts, *unread, *mentions, row.rect, within);

                    // Cut to what is left, with an ellipsis where it was cut.
                    // A clipped name ends mid-letter and says nothing about
                    // there being more of it -- and the clip never stopped one
                    // running under the scrollbar anyway, because the panel is
                    // wider than the rows inside it.
                    let left = row.rect.x + GUTTER;
                    let room = pill
                        .as_ref()
                        .map(|(_, rect)| rect.x - GAP)
                        .unwrap_or(within.right() - TRACK)
                        - left;
                    let named = matterless_layout::elided(fonts, label, room, name_style(loud));
                    let glyphs = painter.run(
                        fonts,
                        &named,
                        left,
                        row.rect.y + 4.0,
                        if loud {
                            Run::label(f32::MAX).bold()
                        } else {
                            Run::label(f32::MAX)
                        },
                    );
                    scene.glyphs(glyphs, ink, palette.faint);

                    if let Some((said, rect)) = pill {
                        draw_pill(
                            scene, painter, fonts, palette, &said, rect, *mentions, *muted, loud,
                            ink,
                        );
                    }
                }
                // Shaped like a channel because it is one kind of place to go,
                // and drawn through the same two helpers so it cannot drift
                // from the rows under it.
                Entry::Threads { unread, mentions } => {
                    let chosen = self.selected.as_deref() == Some(THREADS);
                    if chosen || input.hovered() == Some(name.as_str()) {
                        scene.fill(
                            row.rect.x,
                            row.rect.y,
                            row.rect.width,
                            row.rect.height,
                            palette.ground,
                        );
                    }
                    let loud = asking(*unread, *mentions, false);
                    let ink = ink_of(palette, chosen, loud, false);
                    let glyph = painter.run(
                        fonts,
                        // The app's own mark for it: three lines, which reads
                        // as a list rather than as a conversation.
                        matterless_layout::marks::THREADS,
                        row.rect.x + ICON_LEFT,
                        row.rect.y + 5.0,
                        Run::mark(13.0),
                    );
                    scene.glyphs(glyph, ink, palette.faint);
                    let pill = pill_for(fonts, *unread, *mentions, row.rect, within);
                    let left = row.rect.x + GUTTER;
                    let room = pill
                        .as_ref()
                        .map(|(_, rect)| rect.x - GAP)
                        .unwrap_or(within.right() - TRACK)
                        - left;
                    let named = matterless_layout::elided(fonts, "Threads", room, name_style(loud));
                    let glyphs = painter.run(
                        fonts,
                        &named,
                        left,
                        row.rect.y + 4.0,
                        if loud {
                            Run::label(f32::MAX).bold()
                        } else {
                            Run::label(f32::MAX)
                        },
                    );
                    scene.glyphs(glyphs, ink, palette.faint);
                    if let Some((said, rect)) = pill {
                        draw_pill(
                            scene, painter, fonts, palette, &said, rect, *mentions, false, loud,
                            ink,
                        );
                    }
                }
            }
        }
        let mut canvas = Canvas {
            scene,
            painter,
            fonts,
            palette,
        };
        self.bar.draw(
            &mut canvas,
            "sidebar",
            input,
            within,
            self.scroll,
            self.reach(within),
        );
    }
}

/// Where a row's count sits, and what it says. `None` when there is nothing to
/// count.
///
/// The rectangle is worked out before the name is, because it decides how much
/// room the name has left.
#[allow(clippy::too_many_arguments)]
fn pill_for(
    fonts: &mut matterless_layout::Fonts,
    unread: i64,
    mentions: i64,
    row: Rect,
    within: Rect,
) -> Option<(String, Rect)> {
    let count = if mentions > 0 { mentions } else { unread };
    if count <= 0 {
        return None;
    }
    let said = count.to_string();
    let wide = matterless_layout::extent_of(fonts, &said, f32::MAX, count_style()).width;
    let width = wide + COUNT_PADDING * 2.0;
    Some((
        said,
        Rect::new(
            // Clear of the scrollbar, which floats over the rows rather than
            // taking room from them.
            (row.right() - width).min(within.right() - TRACK - width - 2.0),
            row.y + (row.height - COUNT_HEIGHT) / 2.0,
            width,
            COUNT_HEIGHT,
        ),
    ))
}

/// The count itself.
///
/// A mention is the one count worth colouring: it is the difference between
/// "there is more here" and "you are being asked". Not in a muted channel,
/// where it was still asked for quietly.
#[allow(clippy::too_many_arguments)]
fn draw_pill(
    scene: &mut matterless_paint::Scene,
    painter: &mut matterless_paint::Painter,
    fonts: &mut matterless_layout::Fonts,
    palette: &matterless_paint::Palette,
    said: &str,
    rect: Rect,
    mentions: i64,
    muted: bool,
    loud: bool,
    ink: [u8; 3],
) {
    let (behind, over) = if mentions > 0 && !muted {
        (
            [palette.signal[0], palette.signal[1], palette.signal[2], 255],
            [palette.surface[0], palette.surface[1], palette.surface[2]],
        )
    } else if muted {
        let dim = palette.dimmed(
            [palette.raised[0], palette.raised[1], palette.raised[2]],
            MUTED,
        );
        ([dim[0], dim[1], dim[2], 255], ink)
    } else {
        (palette.raised, ink)
    };
    scene.rounded(
        rect.x,
        rect.y,
        rect.width,
        rect.height,
        behind,
        COUNT_HEIGHT / 2.0,
    );
    let glyphs = painter.run(
        fonts,
        said,
        rect.x + COUNT_PADDING,
        rect.y + (rect.height - COUNT_LINE) / 2.0,
        Run {
            size: COUNT_SIZE,
            line_height: COUNT_LINE,
            bold: loud,
            mono: true,
            wrap: f32::MAX,
            icon: false,
            smooth: false,
        },
    );
    scene.glyphs(glyphs, over, palette.faint);
}

/// What the button for a new conversation answers to.
pub const NEW: &str = "sidebar/new";

/// Where it sits: the right end of the direct messages heading, clear of the
/// scrollbar that floats over the rows.
fn plus_rect(row: Rect, within: Rect) -> Rect {
    let side = 18.0;
    Rect::new(
        (row.right() - side).min(within.right() - TRACK - side),
        row.y + (row.height - side) / 2.0,
        side,
        side,
    )
}

/// The presence dot, and the room kept for it at the start of every row.
///
/// Kept on channel rows too, which never have one: a sidebar whose names do
/// not line up reads as broken, and the alternative is indenting only the
/// conversations that happen to have a dot right now.
/// The presence dot, and the ring that holds it to the face: `max(7px, 32%)`
/// of an 18px face, inside `box-shadow: 0 0 0 1.5px var(--surface)`.
const DOT: f32 = 7.0;
const RING: f32 = 1.5;
/// A face in a row, which the app draws two pixels larger than the marks the
/// other kinds of conversation get.
const FACE: f32 = 18.0;
/// Where the name starts: past the type icon and the gap after it.
const GUTTER: f32 = ICON_LEFT + ICON + 7.0;
/// The channel-type icon, and where its box begins.
const ICON: f32 = 16.0;
const ICON_LEFT: f32 = 4.0;
/// How solid it is: `color: var(--ink-faint); opacity: 0.85`. Quieter than the
/// name it labels, because it says what kind of thing this is rather than
/// which one.
const ICON_INK: f32 = 0.85;
/// A team's picture beside its name: 18px at `border-radius: 4px`.
const TEAM_ICON: f32 = 18.0;
const TEAM_ICON_CORNER: f32 = 4.0;
/// The gap the name keeps from the count beside it.
const GAP: f32 = 6.0;
/// What the scrollbar floats over, which the rows have to keep clear of: it
/// takes no room from them, so nothing stopped a name running under it.
const TRACK: f32 = 12.0;
/// How much of its colour a muted row keeps: `opacity: 0.45`.
const MUTED: f32 = 0.45;
/// The unread count: `font-size: 10.5px` in a `border-radius: 9px` capsule
/// padded `0 6px`, set in the monospaced face so a column of one- and
/// two-digit numbers is the same width.
const COUNT_SIZE: f32 = 10.5;
const COUNT_LINE: f32 = 14.0;
const COUNT_PADDING: f32 = 6.0;
const COUNT_HEIGHT: f32 = 18.0;

/// What kind of conversation a row is.
///
/// Four, not two: the port had one flag for "direct" covering both a message
/// to one person and a message to several, which are different things with
/// different icons.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Public,
    Private,
    Group,
    Direct,
}

impl Kind {
    fn of(direct: bool, private: bool, counterpart: bool) -> Self {
        match (direct, private, counterpart) {
            // One other person, so there is somebody to name.
            (true, _, true) => Kind::Direct,
            // Several, so there is not.
            (true, _, false) => Kind::Group,
            (false, true, _) => Kind::Private,
            (false, false, _) => Kind::Public,
        }
    }
}

/// Draws what kind of conversation a row is, inside `box_of`.
///
/// Shapes rather than characters for the two that have no character. The
/// padlock in a text font is an emoji: it arrives in colour and at whatever
/// size the emoji face feels like, so beside a plain `#` it read as a picture
/// somebody had put in the channel's name. These are drawn from the same
/// rounded boxes as everything else here, so they are one ink, one size, and
/// sit in the same box the hash does.
fn kind_icon(
    scene: &mut matterless_paint::Scene,
    painter: &mut matterless_paint::Painter,
    fonts: &mut matterless_layout::Fonts,
    palette: &matterless_paint::Palette,
    box_of: Rect,
    kind: Kind,
    behind: [u8; 4],
) {
    let ink = palette.dimmed(palette.faint, ICON_INK);
    let solid = [ink[0], ink[1], ink[2], 255];
    // The box is sixteen wide, the same `viewBox` these shapes were drawn in,
    // so the numbers below are the stylesheet's own.
    let at = |x: f32, y: f32| (box_of.x + x, box_of.y + y);
    match kind {
        Kind::Private => {
            // A padlock, because posting in a private channel is a different
            // act. Its shackle is a ring made by punching the row's own colour
            // out of a capsule: a scene of filled boxes has no stroke.
            let (x, y) = at(4.5, 1.5);
            scene.rounded(x, y, 7.0, 8.0, solid, 3.5);
            let (x, y) = at(6.0, 3.0);
            scene.rounded(x, y, 4.0, 6.5, behind, 2.0);
            let (x, y) = at(3.0, 7.5);
            scene.rounded(x, y, 10.0, 6.5, solid, 1.5);
        }
        Kind::Group => {
            // Several people: two heads and two shoulders, the nearer one
            // whole and the further one behind it.
            let (x, y) = at(9.0, 3.5);
            scene.rounded(x, y, 3.5, 3.5, solid, 1.75);
            let (x, y) = at(9.6, 8.8);
            scene.rounded(x, y, 3.9, 4.2, solid, 1.7);
            let (x, y) = at(3.25, 2.5);
            scene.rounded(x, y, 4.5, 4.5, solid, 2.25);
            let (x, y) = at(1.5, 8.2);
            scene.rounded(x, y, 8.0, 4.8, solid, 2.4);
        }
        // The hash every chat client uses, so it needs no explaining, and the
        // at-sign that stands in for a face until the sidebar carries a
        // picture version to fetch one by.
        Kind::Public | Kind::Direct => {
            let glyphs = painter.run(
                fonts,
                if kind == Kind::Public { "#" } else { "@" },
                box_of.x + if kind == Kind::Public { 3.5 } else { 2.5 },
                box_of.y + 0.5,
                Run::label(f32::MAX).bold(),
            );
            scene.glyphs(glyphs, ink, ink);
        }
    }
}

/// What colour a channel's name and count are drawn in.
///
/// Unread is full contrast; a read channel recedes, but only one step -- both
/// on the same colour leaves font weight as the only difference between them,
/// which at 13px is nearly nothing. Where the reader actually is outranks both.
///
/// And a muted channel is that colour *dimmed*, count included, rather than a
/// different one. Replacing it is how muted and read came to look identical:
/// both of them were simply the faint ink. `opacity: 0.45`, which a scene of
/// opaque quads has to mix by hand.
fn ink_of(palette: &matterless_paint::Palette, chosen: bool, loud: bool, muted: bool) -> [u8; 3] {
    let ink = if chosen {
        palette.signal
    } else if loud {
        palette.ink
    } else {
        palette.soft
    };
    if muted {
        palette.dimmed(ink, MUTED)
    } else {
        ink
    }
}

/// Whether a row reads as unread: full contrast, and bold.
///
/// Nothing in a muted channel does, whatever is in it. That is what muting
/// says, and it is why the app works this out rather than reading the count --
/// a muted channel that went bold on every arriving message would be
/// interrupting in the one way it was told not to.
///
/// It still shows how much is there. Muting is "do not interrupt me", not
/// "hide this from me".
fn asking(unread: i64, mentions: i64, muted: bool) -> bool {
    (unread > 0 || mentions > 0) && !muted
}

/// How the count is measured, which has to match how it is drawn -- the pill
/// is sized from this and the number is set by it.
fn count_style() -> matterless_layout::Style {
    matterless_layout::Style {
        size: COUNT_SIZE,
        line_height: COUNT_LINE,
        bold: false,
        italic: false,
        mono: true,
    }
}

/// And the same for a name, which is bold once its channel is unread: a bold
/// name measured as a plain one is cut a letter or two short.
fn name_style(bold: bool) -> matterless_layout::Style {
    matterless_layout::Style {
        size: 13.0,
        line_height: 18.0,
        bold,
        italic: false,
        mono: false,
    }
}

/// How tall each kind of row is.
fn height_of(entry: &Entry) -> f32 {
    match entry {
        // Two lines: a name and what they are doing.
        Entry::Me { .. } => ME,
        Entry::Team { .. } => TEAM,
        Entry::Heading { .. } => HEADING,
        Entry::Channel { .. } | Entry::Threads { .. } => ROW,
    }
}

/// A heading, set the way the stylesheet sets one: upper case with the letters
/// held apart, which is what small type at this weight needs to stay legible.
fn small_caps(label: &str) -> String {
    label
        .to_uppercase()
        .chars()
        .flat_map(|letter| [letter, HAIR])
        .collect::<String>()
        .trim_end()
        .to_string()
}

const HAIR: char = ' ';

/// A status as a word somebody would say, rather than the token the server
/// keeps it under.
fn spoken(status: &str) -> &str {
    match status {
        "online" => "online",
        "away" => "away",
        "dnd" => "do not disturb",
        "ooo" => "out of office",
        "" => "signing in",
        _ => "offline",
    }
}

/// What colour says about somebody, or nothing at all.
///
/// Offline draws no dot rather than a grey one. Absence is the common case,
/// and a sidebar of grey dots is a sidebar of noise -- the question the dot
/// answers is "are they there", and the answer is the dot's presence.
pub fn dot(status: &str, palette: &matterless_paint::Palette) -> Option<[u8; 4]> {
    let solid = |ink: [u8; 3]| [ink[0], ink[1], ink[2], 255];
    match status {
        "online" => Some(solid(palette.ok)),
        "away" => Some(solid(palette.flag)),
        "dnd" | "ooo" => Some(solid(palette.danger)),
        // Includes "offline" and anything a newer server invents: a status
        // this build does not know is not one it should guess a colour for.
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::asking;
    use super::{Entry, Sidebar, THREADS};

    /// A channel row, said once so the walk's tests read as what they test.
    fn channel(id: &str, unread: i64, muted: bool) -> Entry {
        Entry::Channel {
            id: id.into(),
            label: id.into(),
            unread,
            mentions: 0,
            muted,
            direct: false,
            private: false,
            counterpart: None,
            counterpart_avatar_at: 0,
        }
    }

    fn list(selected: &str) -> Sidebar {
        let mut sidebar = Sidebar::new(vec![
            Entry::Heading {
                label: "Channels".into(),
                directs: false,
            },
            channel("quiet", 0, false),
            channel("dev", 3, false),
            channel("noisy", 9, true),
            channel("here", 0, false),
            channel("art", 1, false),
        ]);
        sidebar.selected = Some(selected.into());
        sidebar
    }

    /// The walk goes on from where the reader is, not from the top.
    ///
    /// They are nearly always standing in a conversation with nothing in it,
    /// having just read it -- so counting from the first waiting row would
    /// answer the same one every time.
    #[test]
    fn the_walk_goes_on_from_where_the_reader_is() {
        assert_eq!(list("here").next_unread(true), Some("art"));
        assert_eq!(list("quiet").next_unread(true), Some("dev"));
        // And backwards is the same question the other way.
        assert_eq!(list("here").next_unread(false), Some("dev"));
    }

    /// A muted conversation is passed over.
    ///
    /// Muted is the reader saying it should not interrupt them, and a key
    /// that walks them to it is an interruption they asked not to have. The
    /// rail's dots already leave them out, so counting them here would make
    /// the badge and the key disagree about what is waiting.
    #[test]
    fn a_muted_conversation_is_not_walked_to() {
        // `noisy` has nine unread and sits between `dev` and `here`.
        assert_eq!(list("dev").next_unread(true), Some("art"));
        assert_eq!(list("here").next_unread(false), Some("dev"));
    }

    /// It wraps, or the last one is a dead end where the key stops working
    /// and nothing on screen says why.
    #[test]
    fn the_walk_wraps_round_the_end_of_the_list() {
        assert_eq!(list("art").next_unread(true), Some("dev"));
        assert_eq!(list("dev").next_unread(false), Some("art"));
    }

    /// Standing on the only waiting row answers that row rather than nothing.
    ///
    /// A walk of every row that ends where it began is the honest answer:
    /// there *is* somewhere with something in it, and it is here.
    #[test]
    fn the_only_waiting_row_answers_itself() {
        let mut sidebar = Sidebar::new(vec![channel("quiet", 0, false), channel("dev", 2, false)]);
        sidebar.selected = Some("dev".into());
        assert_eq!(sidebar.next_unread(true), Some("dev"));
    }

    /// Nothing waiting is nothing to walk to, rather than the first row.
    #[test]
    fn a_list_with_nothing_in_it_answers_nothing() {
        let mut sidebar = Sidebar::new(vec![channel("quiet", 0, false), channel("calm", 0, true)]);
        sidebar.selected = Some("quiet".into());
        assert_eq!(sidebar.next_unread(true), None);
        assert_eq!(sidebar.next_unread(false), None);
    }

    /// The threads row is somewhere to go, so the walk stops at it.
    ///
    /// It is the only row that can say a reply is waiting under a collapsed
    /// thread, so leaving it out would make the walk silent about exactly the
    /// case nothing else reports.
    #[test]
    fn the_threads_row_is_walked_to_like_any_other() {
        let mut sidebar = Sidebar::new(vec![
            Entry::Threads {
                unread: 1,
                mentions: 0,
            },
            channel("quiet", 0, false),
        ]);
        sidebar.selected = Some("quiet".into());
        assert_eq!(sidebar.next_unread(true), Some(THREADS));
    }

    /// A presence dot goes on top of the face, not under it.
    ///
    /// This scene is painted in the order it is built, so anything drawn
    /// before a picture is drawn behind it -- and the dot went in first, which
    /// meant it was there and invisible. The same mistake has been made five
    /// times in this file in one form or another, always by adding the new
    /// thing where it reads best rather than where it paints.
    #[test]
    fn a_presence_dot_is_painted_over_the_face_it_belongs_to() {
        use matterless_paint::{Painter, Palette, Piece, Scene};

        let palette = Palette::default();
        let mut fonts = matterless_layout::Fonts::new();
        let mut painter = Painter::new();
        let mut scene = Scene::default();
        let mut sidebar = Sidebar::new(vec![Entry::Channel {
            id: "d1".into(),
            label: "somebody".into(),
            unread: 0,
            mentions: 0,
            muted: false,
            direct: true,
            private: false,
            counterpart: Some("u9".into()),
            counterpart_avatar_at: 7,
        }]);
        sidebar.selected = None;
        let within = Rect::new(0.0, 0.0, 240.0, 400.0);
        let mut presence = std::collections::HashMap::new();
        presence.insert("u9".to_string(), "online".to_string());
        let placed = sidebar.boxes(within);
        sidebar.draw(
            &mut Canvas {
                scene: &mut scene,
                painter: &mut painter,
                fonts: &mut fonts,
                palette: &palette,
            },
            &placed,
            within,
            &Input::default(),
            &presence,
        );

        let pieces: Vec<&Piece> = scene
            .layers
            .iter()
            .flat_map(|layer| layer.pieces.iter())
            .collect();
        let face = pieces
            .iter()
            .position(
                |piece| matches!(piece, Piece::Image { key, .. } if key.starts_with("avatar/u9")),
            )
            .expect("the face is drawn");
        // The dot is the only thing in the row painted in the live colour.
        let lit = dot("online", &palette).expect("online has a colour");
        let mark = pieces
            .iter()
            .position(|piece| matches!(piece, Piece::Fill { colour, .. } if *colour == lit))
            .expect("the dot is drawn");
        assert!(
            mark > face,
            "the dot is painted at {mark}, before the face at {face}, so the face covers it"
        );
    }

    /// The threads row is chosen the way a channel is.
    ///
    /// It is a place to go, so it answers through the same `react` and lands in
    /// the same `selected` -- which is what lets it draw as the chosen row
    /// without a second piece of state saying which kind of thing is open. The
    /// id it uses can never collide with a real one: the server's are twenty-
    /// six characters.
    #[test]
    fn the_threads_row_is_picked_like_a_channel() {
        let mut sidebar = Sidebar::new(vec![
            Entry::Threads {
                unread: 3,
                mentions: 1,
            },
            Entry::Channel {
                id: "c1".into(),
                label: "Dev".into(),
                unread: 0,
                mentions: 0,
                muted: false,
                direct: false,
                private: false,
                counterpart: None,
                counterpart_avatar_at: 0,
            },
        ]);
        let within = Rect::new(0.0, 0.0, 240.0, 400.0);
        let placed = sidebar.boxes(within);
        let row = placed
            .iter()
            .find(|item| item.name == format!("sidebar/channel/{THREADS}"))
            .expect("the threads row is placed");
        let mut input = Input::default();
        input.apply(
            matterless_ui::input::Event::PointerMoved {
                x: row.rect.x + 4.0,
                y: row.rect.y + 4.0,
            },
            &placed,
        );
        input.apply(matterless_ui::input::Event::PointerPressed, &placed);
        input.apply(matterless_ui::input::Event::PointerReleased, &placed);
        assert_eq!(
            sidebar.react(&input, &placed, within).as_deref(),
            Some(THREADS)
        );
        assert_eq!(sidebar.selected.as_deref(), Some(THREADS));
    }

    /// Starting a conversation is offered beside the conversations, on the one
    /// group that is people rather than channels.
    ///
    /// Where the app puts it, and the only place it can go: the other headings
    /// are teams' channel groups, and a team is not somebody to talk to.
    #[test]
    fn the_direct_messages_heading_offers_a_new_conversation() {
        let sidebar = Sidebar::new(vec![
            Entry::Heading {
                label: "Channels".into(),
                directs: false,
            },
            Entry::Heading {
                label: "Direct messages".into(),
                directs: true,
            },
        ]);
        let within = Rect::new(0.0, 0.0, 240.0, 400.0);
        let placed = sidebar.boxes(within);
        let plus: Vec<&Placed> = placed.iter().filter(|one| one.name == NEW).collect();
        assert_eq!(plus.len(), 1, "one button, on the one heading that gets it");
        let at = plus[0].rect;
        assert!(at.x >= within.x && at.right() <= within.right());
        // Clear of the bar, which floats over the rows rather than taking room
        // from them.
        assert!(
            at.right() <= within.right() - TRACK,
            "the button sits under the scrollbar"
        );
        // On the heading it belongs to, not the one above it.
        let heading = placed
            .iter()
            .find(|one| one.name == "sidebar/heading/1")
            .expect("the heading is placed");
        assert!(at.y >= heading.rect.y && at.bottom() <= heading.rect.bottom());
    }

    /// A muted channel never reads as unread, however much arrives in it.
    ///
    /// Twice now this has drifted: first to looking identical to a read
    /// channel, then to going bold on every message. Both are the same
    /// mistake -- deciding how loud a row is from its count alone, when the
    /// reader has already said they do not want to be interrupted by it.
    #[test]
    fn a_muted_channel_is_never_loud() {
        for (unread, mentions) in [(1, 0), (0, 1), (40, 12)] {
            assert!(
                !asking(unread, mentions, true),
                "{unread} unread and {mentions} mentions went loud while muted"
            );
            assert!(
                asking(unread, mentions, false),
                "{unread} unread and {mentions} mentions stayed quiet unmuted"
            );
        }
        // And an empty channel is quiet either way.
        assert!(!asking(0, 0, false));
        assert!(!asking(0, 0, true));
    }

    use super::*;
    use matterless_ui::input::Event;

    fn channels() -> Sidebar {
        Sidebar::new(vec![
            Entry::Heading {
                label: "Channels".into(),
                directs: false,
            },
            Entry::Channel {
                id: "one".into(),
                label: "general".into(),
                unread: 0,
                mentions: 0,
                muted: false,
                direct: false,
                private: false,
                counterpart: None,
                counterpart_avatar_at: 0,
            },
            Entry::Channel {
                id: "two".into(),
                label: "random".into(),
                unread: 3,
                mentions: 1,
                muted: false,
                direct: false,
                private: false,
                counterpart: None,
                counterpart_avatar_at: 0,
            },
        ])
    }

    fn panel() -> Rect {
        Rect::new(0.0, 0.0, 260.0, 400.0)
    }

    #[test]
    fn a_click_chooses_the_channel_it_landed_on() {
        let mut sidebar = channels();
        let placed = sidebar.boxes(panel());
        let mut input = Input::default();
        // The first channel sits below the heading and the padding.
        input.apply(
            Event::PointerMoved {
                x: 100.0,
                y: PADDING + HEADING + 5.0,
            },
            &placed,
        );
        input.apply(Event::PointerPressed, &placed);
        input.apply(Event::PointerReleased, &placed);
        assert_eq!(sidebar.react(&input, &placed, panel()), Some("one".into()));
        assert_eq!(sidebar.selected.as_deref(), Some("one"));
    }

    #[test]
    fn a_click_on_a_heading_chooses_nothing() {
        let mut sidebar = channels();
        let placed = sidebar.boxes(panel());
        let mut input = Input::default();
        input.apply(
            Event::PointerMoved {
                x: 100.0,
                y: PADDING + 5.0,
            },
            &placed,
        );
        input.apply(Event::PointerPressed, &placed);
        input.apply(Event::PointerReleased, &placed);
        assert_eq!(sidebar.react(&input, &placed, panel()), None);
        assert!(sidebar.selected.is_none());
    }

    /// A short list has nowhere to go, and the wheel must not move it.
    #[test]
    fn a_list_that_fits_does_not_scroll() {
        let mut sidebar = channels();
        let within = panel();
        let placed = sidebar.boxes(within);
        let mut input = Input::default();
        input.apply(Event::PointerMoved { x: 100.0, y: 200.0 }, &placed);
        input.apply(Event::Wheel { x: 0.0, y: -400.0 }, &placed);
        sidebar.react(&input, &placed, within);
        assert_eq!(sidebar.scroll, 0.0);
        assert_eq!(sidebar.reach(within), 0.0);
    }

    /// The rail takes the reader to a team, which means scrolling the list to
    /// where that team starts -- not opening a conversation they did not ask
    /// for.
    #[test]
    fn the_rail_scrolls_the_list_to_a_team() {
        let mut sidebar = Sidebar::new(vec![
            Entry::Team {
                id: "t1".into(),
                label: "Voyager".into(),
            },
            Entry::Heading {
                label: "Channels".into(),
                directs: false,
            },
            Entry::Channel {
                id: "c1".into(),
                label: "dev".into(),
                unread: 0,
                mentions: 0,
                muted: false,
                direct: false,
                private: false,
                counterpart: None,
                counterpart_avatar_at: 0,
            },
            Entry::Team {
                id: "t2".into(),
                label: "Northwind".into(),
            },
            Entry::Heading {
                label: "Direct messages".into(),
                directs: true,
            },
        ]);
        // Short enough that there is somewhere to scroll to.
        let panel = Rect::new(0.0, 0.0, 260.0, 60.0);
        // Where a named row ended up, which is the thing being claimed: the
        // scroll offset on its own is an implementation detail and the panel
        // has padding of its own above the first row.
        let top_of = |sidebar: &Sidebar, name: &str| {
            sidebar
                .boxes(panel)
                .into_iter()
                .find(|placed| placed.name == name)
                .map(|placed| placed.rect.y)
        };

        sidebar.scroll_to("t1", panel);
        assert_eq!(top_of(&sidebar, "sidebar/team/t1"), Some(panel.y));

        sidebar.scroll_to("t2", panel);
        let second = sidebar.scroll;
        assert!(second > 0.0, "the second team is below the first");

        // The envelope reaches the one group that belongs to no team.
        sidebar.scroll_to(crate::rail::DIRECTS, panel);
        assert!(sidebar.scroll >= second);

        // A team the list does not hold leaves the reader where they were.
        let before = sidebar.scroll;
        sidebar.scroll_to("nowhere", panel);
        assert_eq!(sidebar.scroll, before);
    }

    /// Three states need three inks. Muted was drawn in the same colour as
    /// every read channel, which is no state at all.
    #[test]
    fn the_three_states_of_a_row_are_three_colours() {
        let palette = matterless_paint::Palette::default();
        // Read, unread, muted -- as the row actually picks them. This used to
        // assert only that the palette held three distinct colours, which it
        // does whatever the rows do with them: it passed for as long as muted
        // and read were both drawn in the faint ink.
        let read = super::ink_of(&palette, false, false, false);
        let unread = super::ink_of(&palette, false, true, false);
        let muted = super::ink_of(&palette, false, false, true);
        let muted_with_messages = super::ink_of(&palette, false, false, true);
        assert_ne!(read, unread, "an unread channel looks like a read one");
        assert_ne!(read, muted, "a muted channel looks like a read one");
        assert_ne!(unread, muted);
        // A muted channel does not brighten when something arrives in it: it
        // is never `loud`, so this is the same colour as plain muted.
        assert_eq!(muted, muted_with_messages);
        // Where the reader is outranks all of it.
        assert_ne!(super::ink_of(&palette, true, false, false), read);
    }

    /// Offline draws nothing, and so does a status this build has never heard
    /// of: a colour guessed for an unknown word says something untrue.
    #[test]
    fn only_a_status_worth_showing_gets_a_dot() {
        let palette = matterless_paint::Palette::default();
        assert!(dot("online", &palette).is_some());
        assert!(dot("away", &palette).is_some());
        assert!(dot("dnd", &palette).is_some());
        assert!(dot("offline", &palette).is_none());
        assert!(dot("", &palette).is_none());
        assert!(dot("whatever-comes-next", &palette).is_none());
    }

    /// Every one of them reads differently, or the dot says only "somebody has
    /// a status" -- which nobody needed to know.
    #[test]
    fn the_states_do_not_look_alike() {
        let palette = matterless_paint::Palette::default();
        let shown = ["online", "away", "dnd"].map(|status| dot(status, &palette));
        assert_ne!(shown[0], shown[1]);
        assert_ne!(shown[1], shown[2]);
        assert_ne!(shown[0], shown[2]);
    }

    #[test]
    fn a_long_list_scrolls_and_stops_at_its_end() {
        let mut sidebar = Sidebar::new(
            (0..60)
                .map(|index| Entry::Channel {
                    id: format!("c{index}"),
                    label: format!("channel {index}"),
                    unread: 0,
                    mentions: 0,
                    muted: false,
                    direct: false,
                    private: false,
                    counterpart: None,
                    counterpart_avatar_at: 0,
                })
                .collect(),
        );
        let within = panel();
        let placed = sidebar.boxes(within);
        let mut input = Input::default();
        input.apply(Event::PointerMoved { x: 100.0, y: 200.0 }, &placed);
        input.apply(Event::Wheel { x: 0.0, y: -100.0 }, &placed);
        sidebar.react(&input, &placed, within);
        assert_eq!(sidebar.scroll, 100.0);

        let reach = sidebar.reach(within);
        input.settle();
        input.apply(
            Event::Wheel {
                x: 0.0,
                y: -100_000.0,
            },
            &placed,
        );
        sidebar.react(&input, &placed, within);
        assert_eq!(sidebar.scroll, reach);
    }

    /// Scrolling moves the rows and leaves the panel where it is, or the wheel
    /// would stop finding the panel it is turning over.
    #[test]
    fn scrolling_moves_the_rows_not_the_panel() {
        let mut sidebar = channels();
        sidebar.scroll = 40.0;
        let within = panel();
        let placed = sidebar.boxes(within);
        assert_eq!(placed[0].name, "sidebar");
        assert_eq!(placed[0].rect, within);
        let first = placed
            .iter()
            .find(|item| item.name == "sidebar/channel/one")
            .unwrap();
        assert_eq!(first.rect.y, PADDING + HEADING - 40.0);
    }
}
