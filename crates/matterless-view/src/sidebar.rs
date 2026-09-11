//! The channel list, built from boxes rather than HTML.
//!
//! The first piece of chrome to be ported, and chosen deliberately: it is a
//! list of rows with hover and selection, which exercises layout, hit testing,
//! input and drawing together -- and needs no text entry, which is the part
//! that still has to be built.
//!
//! It decides nothing about where it sits. The caller hands it a rectangle and
//! it fills it, so the same widget works beside a thread pane or without one.

use matterless_layout::Fonts;
use matterless_paint::{Painter, Palette, Run, Scene};
use matterless_ui::input::Input;
use matterless_ui::{Axis, Node, Placed, Rect, Size};

/// What a widget draws with.
pub struct Canvas<'a> {
    pub scene: &'a mut Scene,
    pub painter: &'a mut Painter,
    pub fonts: &'a mut Fonts,
    pub palette: &'a Palette,
}

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
        placed.extend(self.bar.boxes("sidebar", within, self.reach(within)));
        // The panel itself keeps its real rectangle: it is what the wheel is
        // tested against, and a scrolled one would stop matching the pointer.
        if let Some(panel) = placed.first_mut() {
            panel.rect = within;
        }
        placed
    }

    /// Scrolls a team's heading to the top of the panel.
    ///
    /// What the rail does: a team is not somewhere to be, it is somewhere in
    /// the list, and taking the reader there is different from opening a
    /// conversation they did not ask for.
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
                Entry::Team { label, .. } => {
                    let glyphs = painter.run(
                        fonts,
                        label,
                        row.rect.x + 4.0,
                        row.rect.y + 8.0,
                        Run {
                            size: 14.5,
                            line_height: 20.0,
                            bold: true,
                            mono: false,
                            wrap: f32::MAX,
                        },
                    );
                    scene.glyphs(glyphs, palette.ink, palette.faint);
                }
                Entry::Heading { label, .. } => {
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
                        },
                    );
                    scene.glyphs(glyphs, palette.faint, palette.faint);
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
                    // Unread is full contrast and bold, read recedes, and
                    // where the reader actually is outranks both.
                    let loud = *unread > 0 || *mentions > 0;
                    let mut ink = if chosen {
                        palette.signal
                    } else if loud {
                        palette.ink
                    } else {
                        palette.faint
                    };
                    // A muted channel is its own colour dimmed, count
                    // included, not a different colour: replacing it is how
                    // muted and read came to look identical -- both of them
                    // were simply the faint ink. `opacity: 0.45`.
                    if *muted {
                        ink = palette.dimmed(ink, MUTED);
                    }
                    // Whether somebody is around, which is the one thing that
                    // decides whether you write to them now. Only here: a dot
                    // on every message row would say nothing about the
                    // conversation and turn the margin into a light display.
                    if let Some(lit) = counterpart
                        .as_deref()
                        .and_then(|who| presence.get(who))
                        .and_then(|status| dot(status, palette))
                    {
                        scene.fill(
                            row.rect.x + 2.0,
                            row.rect.y + row.rect.height / 2.0 - DOT / 2.0,
                            DOT,
                            DOT,
                            lit,
                        );
                    }
                    // What kind of conversation this is, in one character.
                    let sigil = match (*direct, *private) {
                        (true, _) => "@",
                        (false, true) => "🔒",
                        (false, false) => "#",
                    };
                    // The count first, because it decides how much room the
                    // name has. Muted channels keep theirs: muting is "do not
                    // interrupt me", not "hide this from me".
                    let count = if *mentions > 0 { *mentions } else { *unread };
                    let pill = (count > 0).then(|| {
                        let said = count.to_string();
                        let wide =
                            matterless_layout::extent_of(fonts, &said, f32::MAX, count_style())
                                .width;
                        let width = wide + COUNT_PADDING * 2.0;
                        (
                            said,
                            Rect::new(
                                // Clear of the scrollbar, which floats over
                                // the rows rather than taking room from them.
                                (row.rect.right() - width)
                                    .min(within.right() - TRACK - width - 2.0),
                                row.rect.y + (row.rect.height - COUNT_HEIGHT) / 2.0,
                                width,
                                COUNT_HEIGHT,
                            ),
                        )
                    });

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
                    let named = matterless_layout::elided(
                        fonts,
                        &format!("{sigil} {label}"),
                        room,
                        name_style(loud),
                    );
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
                        // A mention is the one count worth colouring: it is
                        // the difference between "there is more here" and "you
                        // are being asked". Not in a muted channel, where it
                        // was still asked for quietly.
                        let asked = *mentions > 0 && !*muted;
                        let (behind, over) = if asked {
                            (
                                [palette.signal[0], palette.signal[1], palette.signal[2], 255],
                                [palette.surface[0], palette.surface[1], palette.surface[2]],
                            )
                        } else if *muted {
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
                            &said,
                            rect.x + COUNT_PADDING,
                            rect.y + (rect.height - COUNT_LINE) / 2.0,
                            Run {
                                size: COUNT_SIZE,
                                line_height: COUNT_LINE,
                                bold: loud,
                                mono: true,
                                wrap: f32::MAX,
                            },
                        );
                        scene.glyphs(glyphs, over, palette.faint);
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
        self.bar
            .draw(
                &mut canvas,
                "sidebar",
                input,
                within,
                self.scroll,
                self.reach(within),
            );
    }
}

/// The presence dot, and the room kept for it at the start of every row.
///
/// Kept on channel rows too, which never have one: a sidebar whose names do
/// not line up reads as broken, and the alternative is indenting only the
/// conversations that happen to have a dot right now.
const DOT: f32 = 6.0;
const GUTTER: f32 = 12.0;
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
        Entry::Channel { .. } => ROW,
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
fn dot(status: &str, palette: &matterless_paint::Palette) -> Option<[u8; 4]> {
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
                label: "Curiosity".into(),
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
            },
            Entry::Team {
                id: "t2".into(),
                label: "Sloclap".into(),
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
        assert_ne!(palette.ink, palette.soft);
        assert_ne!(palette.soft, palette.faint);
        assert_ne!(palette.ink, palette.faint);
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
