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
    /// A group's name -- "Favourites", a team, "Direct messages".
    Heading { label: String },
    Channel {
        id: String,
        label: String,
        unread: i64,
        mentions: i64,
        muted: bool,
    },
}

/// The list, and where the reader is in it.
pub struct Sidebar {
    pub entries: Vec<Entry>,
    pub selected: Option<String>,
    /// How far down the list has been scrolled, in pixels.
    pub scroll: f32,
}

/// A row's height, and a heading's. Fixed, because a channel name is one line
/// and a list of five hundred of them must not need measuring to be scrolled.
const ROW: f32 = 26.0;
const HEADING: f32 = 30.0;
const PADDING: f32 = 8.0;

impl Sidebar {
    pub fn new(entries: Vec<Entry>) -> Self {
        Self {
            entries,
            selected: None,
            scroll: 0.0,
        }
    }

    /// The name a row answers to when it is hit.
    fn name_of(entry: &Entry, index: usize) -> String {
        match entry {
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
            let height = match entry {
                Entry::Heading { .. } => HEADING,
                Entry::Channel { .. } => ROW,
            };
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
        // The panel itself keeps its real rectangle: it is what the wheel is
        // tested against, and a scrolled one would stop matching the pointer.
        if let Some(panel) = placed.first_mut() {
            panel.rect = within;
        }
        placed
    }

    /// How far the list can be scrolled before it runs out.
    pub fn reach(&self, within: Rect) -> f32 {
        let content: f32 = self
            .entries
            .iter()
            .map(|entry| match entry {
                Entry::Heading { .. } => HEADING,
                Entry::Channel { .. } => ROW,
            })
            .sum();
        (content + PADDING * 2.0 - within.height).max(0.0)
    }

    /// Applies a frame's input: what was clicked, and how far the wheel turned.
    ///
    /// Answers the channel the reader chose, if they chose one.
    pub fn react(&mut self, input: &Input, placed: &[Placed], within: Rect) -> Option<String> {
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
    pub fn draw(&self, into: &mut Canvas<'_>, placed: &[Placed], within: Rect, input: &Input) {
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
                Entry::Heading { label } => {
                    let glyphs = painter.run(
                        fonts,
                        label,
                        row.rect.x,
                        row.rect.y + 10.0,
                        Run::label(row.rect.width).bold(),
                    );
                    scene.glyphs(glyphs, palette.faint, palette.faint);
                }
                Entry::Channel {
                    id,
                    label,
                    unread,
                    mentions,
                    muted,
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
                    // Three states, and the quiet one is the default: a read
                    // channel recedes, an unread one does not, and a muted one
                    // recedes further still.
                    let ink = if *muted {
                        palette.faint
                    } else if *unread > 0 || chosen {
                        palette.ink
                    } else {
                        palette.faint
                    };
                    let glyphs = painter.run(
                        fonts,
                        &format!("# {label}"),
                        row.rect.x + 6.0,
                        row.rect.y + 4.0,
                        Run::label(row.rect.width - 40.0),
                    );
                    scene.glyphs(glyphs, ink, palette.faint);
                    let count = if *mentions > 0 { *mentions } else { *unread };
                    if count > 0 && !*muted {
                        let glyphs = painter.run(
                            fonts,
                            &count.to_string(),
                            row.rect.right() - 22.0,
                            row.rect.y + 4.0,
                            Run::label(30.0),
                        );
                        scene.glyphs(glyphs, palette.ink, palette.faint);
                    }
                }
            }
        }
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
            },
            Entry::Channel {
                id: "one".into(),
                label: "general".into(),
                unread: 0,
                mentions: 0,
                muted: false,
            },
            Entry::Channel {
                id: "two".into(),
                label: "random".into(),
                unread: 3,
                mentions: 1,
                muted: false,
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
