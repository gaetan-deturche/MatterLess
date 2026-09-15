//! One picture, looked at properly.
//!
//! Clicking a picture in a message should make it bigger. That is the whole
//! feature, and without it a screenshot posted at 320 pixels is a screenshot
//! nobody can read -- and, because Save only ever lived on a file *card*, a
//! picture was also the one attachment that could not be kept at all.
//!
//! The picture is drawn from a texture of its own rather than from the atlas,
//! which is why `Piece::Shown` exists. The atlas is a fixed sheet shared by
//! every glyph and thumbnail on screen, allocated in shelves with no eviction:
//! a two-thousand-pixel photograph in it would push out the faces around it
//! and never give the room back. One picture at a time, in its own texture,
//! dropped when this shuts.
//!
//! Stepping wraps, as the app's does: with three attachments the one after the
//! last is the first, which is what a reader flicking through expects. Only
//! pictures and videos are steppable -- there is nothing to magnify about a
//! file card, so stepping never lands on one.

use crate::sidebar::Canvas;
use matterless_paint::Run;
use matterless_ui::input::{Input, Key};
use matterless_ui::{Placed, Rect};

pub const NAME: &str = "viewer";

/// What the picture keeps clear of the window's edges, so it never reads as
/// the window itself.
const MARGIN: f32 = 48.0;
/// The bar along the bottom: the name on the left, the buttons on the right.
const BAR: f32 = 44.0;
const BUTTON: f32 = 30.0;
const GAP: f32 = 8.0;
const PADDING: f32 = 12.0;

/// One attachment the viewer can show.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Looking {
    pub file_id: String,
    pub name: String,
    /// Whether the original is the right rendition: the server re-encodes a
    /// photograph and does not re-encode a GIF or an SVG.
    pub original: bool,
}

/// What a press on the viewer meant.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Did {
    /// Shut it.
    Close,
    /// Show a different one; the caller fetches it.
    Show(Looking),
    /// Keep this one, next to the reader's other downloads.
    Save { file_id: String, name: String },
}

/// The viewer, open or shut.
#[derive(Debug, Default)]
pub struct Viewer {
    /// Every picture on the message it was opened from, in the order they were
    /// attached, and which of them is showing.
    shown: Vec<Looking>,
    at: usize,
    /// The size of what is being drawn, once it has arrived. `None` while it
    /// is still on its way.
    size: Option<(u32, u32)>,
    /// Why there is nothing to look at, when there is nothing.
    failed: String,
}

impl Viewer {
    pub fn open(&self) -> bool {
        !self.shown.is_empty()
    }

    /// Opens it on one of a message's pictures.
    pub fn show(&mut self, all: Vec<Looking>, file_id: &str) -> Option<Looking> {
        let at = all.iter().position(|one| one.file_id == file_id)?;
        self.shown = all;
        self.at = at;
        self.size = None;
        self.failed.clear();
        self.shown.get(at).cloned()
    }

    pub fn hide(&mut self) {
        self.shown.clear();
        self.at = 0;
        self.size = None;
        self.failed.clear();
    }

    /// What is on screen, if anything is.
    pub fn current(&self) -> Option<&Looking> {
        self.shown.get(self.at)
    }

    /// The bytes arrived, and this is how big they turned out to be.
    pub fn arrived(&mut self, file_id: &str, size: (u32, u32)) {
        // Only for the one being looked at: a reader who stepped on while the
        // last one was in flight must not have it land on top of this.
        if self.current().is_some_and(|one| one.file_id == file_id) {
            self.size = Some(size);
            self.failed.clear();
        }
    }

    pub fn gave_up(&mut self, file_id: &str, why: &str) {
        if self.current().is_some_and(|one| one.file_id == file_id) {
            self.size = None;
            self.failed = why.to_string();
        }
    }

    /// Steps by `by`, wrapping. Answers the one to fetch.
    fn step(&mut self, by: isize) -> Option<Looking> {
        if self.shown.len() < 2 {
            return None;
        }
        let count = self.shown.len() as isize;
        self.at = ((self.at as isize + by).rem_euclid(count)) as usize;
        self.size = None;
        self.failed.clear();
        self.current().cloned()
    }

    /// Where the picture is drawn: the window less its margins, with the
    /// picture's own shape kept.
    ///
    /// Never enlarged. A small picture opened full size and blown up is worse
    /// than the same picture at its own size, and the reader opened it to see
    /// it rather than to see it bigger than it is.
    pub fn picture_rect(&self, window: Rect) -> Option<Rect> {
        let (width, height) = self.size?;
        let room = Rect::new(
            window.x + MARGIN,
            window.y + MARGIN,
            (window.width - MARGIN * 2.0).max(1.0),
            (window.height - MARGIN * 2.0 - BAR).max(1.0),
        );
        let scale = (room.width / width as f32)
            .min(room.height / height as f32)
            .min(1.0);
        let (drawn, tall) = (width as f32 * scale, height as f32 * scale);
        Some(Rect::new(
            room.x + (room.width - drawn) / 2.0,
            room.y + (room.height - tall) / 2.0,
            drawn,
            tall,
        ))
    }

    /// The bar under the picture, and the three things on it.
    fn bar_rect(&self, window: Rect) -> Rect {
        Rect::new(
            window.x + MARGIN,
            window.bottom() - MARGIN - BAR,
            (window.width - MARGIN * 2.0).max(1.0),
            BAR,
        )
    }

    fn button_rects(&self, window: Rect) -> Vec<(&'static str, Rect)> {
        let bar = self.bar_rect(window);
        let mut at = bar.right() - PADDING;
        let mut found = Vec::new();
        for name in ["save", "next", "back"] {
            // Only one picture, so there is nowhere to step: a button that
            // does nothing is a button that has to be pressed to find out.
            if (name == "next" || name == "back") && self.shown.len() < 2 {
                continue;
            }
            let width = if name == "save" { 62.0 } else { BUTTON };
            at -= width;
            found.push((
                name,
                Rect::new(at, bar.y + (bar.height - BUTTON) / 2.0, width, BUTTON),
            ));
            at -= GAP;
        }
        found
    }

    /// Everything a press can land on, deepest in the window.
    pub fn boxes(&self, window: Rect) -> Vec<Placed> {
        if !self.open() {
            return Vec::new();
        }
        // The whole window first, so a press beside the picture shuts the
        // viewer rather than reaching the conversation it is covering.
        let mut placed = vec![Placed {
            name: NAME.to_string(),
            rect: window,
            depth: 30,
        }];
        for (name, rect) in self.button_rects(window) {
            placed.push(Placed {
                name: format!("{NAME}/{name}"),
                rect,
                depth: 31,
            });
        }
        // The picture itself catches its own press, so clicking what you are
        // looking at does not shut it.
        if let Some(picture) = self.picture_rect(window) {
            placed.push(Placed {
                name: format!("{NAME}/picture"),
                rect: picture,
                depth: 31,
            });
        }
        placed
    }

    /// Applies a frame's input.
    pub fn react(&mut self, input: &Input) -> Option<Did> {
        if !self.open() {
            return None;
        }
        if input.struck(Key::Escape) {
            return Some(Did::Close);
        }
        if input.struck(Key::Left) {
            return self.step(-1).map(Did::Show);
        }
        if input.struck(Key::Right) {
            return self.step(1).map(Did::Show);
        }
        let clicked = input.clicked()?;
        match clicked.strip_prefix(&format!("{NAME}/")) {
            Some("back") => self.step(-1).map(Did::Show),
            Some("next") => self.step(1).map(Did::Show),
            Some("save") => self.current().map(|one| Did::Save {
                file_id: one.file_id.clone(),
                name: one.name.clone(),
            }),
            // The picture itself: pressed and nothing happens, which is what
            // keeps a click on what you are reading from closing it.
            Some("picture") => None,
            _ if clicked == NAME => Some(Did::Close),
            _ => None,
        }
    }

    pub fn draw(&self, into: &mut Canvas<'_>, input: &Input, window: Rect) {
        if !self.open() {
            return;
        }
        let picture = self.picture_rect(window);
        let bar = self.bar_rect(window);
        let buttons = self.button_rects(window);
        let Canvas {
            scene,
            painter,
            fonts,
            palette,
        } = into;
        // The window behind it, dimmed rather than hidden: what a reader is
        // looking at came from a conversation, and covering it entirely loses
        // where they are.
        scene.fill(
            window.x,
            window.y,
            window.width,
            window.height,
            [0, 0, 0, 216],
        );
        match picture {
            Some(at) => scene.extend([matterless_paint::Piece::Shown {
                x: at.x,
                y: at.y,
                width: at.width,
                height: at.height,
            }]),
            None => {
                // Something to look at while it arrives, and something to read
                // if it never does.
                let said = if self.failed.is_empty() {
                    "opening\u{2026}".to_string()
                } else {
                    format!("this picture could not be opened: {}", self.failed)
                };
                let glyphs = painter.run(
                    fonts,
                    &said,
                    window.x + window.width / 2.0 - 80.0,
                    window.y + window.height / 2.0,
                    Run::label(f32::MAX),
                );
                scene.glyphs(glyphs, palette.soft, palette.faint);
            }
        }

        // What it is called, and which of them this is.
        if let Some(one) = self.current() {
            let said = if self.shown.len() > 1 {
                format!("{} ({} of {})", one.name, self.at + 1, self.shown.len())
            } else {
                one.name.clone()
            };
            let room = buttons
                .iter()
                .map(|(_, rect)| rect.x)
                .fold(bar.right(), f32::min)
                - bar.x
                - PADDING * 2.0;
            let said = matterless_layout::elided(
                fonts,
                &said,
                room,
                matterless_layout::Style {
                    size: 13.0,
                    line_height: 18.0,
                    bold: false,
                    italic: false,
                    mono: false,
                },
            );
            let glyphs = painter.run(
                fonts,
                &said,
                bar.x + PADDING,
                bar.y + (bar.height - 18.0) / 2.0,
                Run::label(f32::MAX),
            );
            scene.glyphs(glyphs, palette.ink, palette.faint);
        }

        for (name, rect) in buttons {
            let under = input.hovered() == Some(format!("{NAME}/{name}").as_str());
            scene.rounded(
                rect.x,
                rect.y,
                rect.width,
                rect.height,
                if under {
                    palette.raised
                } else {
                    [palette.raised[0], palette.raised[1], palette.raised[2], 160]
                },
                6.0,
            );
            let said = match name {
                "save" => "Save",
                "back" => matterless_layout::marks::BACK,
                _ => matterless_layout::marks::NEXT,
            };
            // The arrows are marks and "Save" is a word, so they are neither
            // the same size nor from the same family, and only the word can be
            // measured with the text metrics.
            let mark = name != "save";
            let wide = if mark {
                15.0
            } else {
                matterless_layout::extent_of(
                    fonts,
                    said,
                    f32::MAX,
                    matterless_layout::Style {
                        size: 13.0,
                        line_height: 18.0,
                        bold: false,
                        italic: false,
                        mono: false,
                    },
                )
                .width
            };
            let glyphs = painter.run(
                fonts,
                said,
                rect.x + (rect.width - wide) / 2.0,
                rect.y + (rect.height - 18.0) / 2.0,
                if mark {
                    Run::mark(16.0)
                } else {
                    Run::label(f32::MAX)
                },
            );
            scene.glyphs(glyphs, palette.ink, palette.faint);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn three() -> Vec<Looking> {
        (0..3)
            .map(|at| Looking {
                file_id: format!("f{at}"),
                name: format!("shot{at}.png"),
                original: false,
            })
            .collect()
    }

    fn window() -> Rect {
        Rect::new(0.0, 0.0, 1000.0, 700.0)
    }

    /// Opening lands on the one that was pressed, not on the first.
    #[test]
    fn it_opens_on_the_picture_that_was_pressed() {
        let mut viewer = Viewer::default();
        let opened = viewer.show(three(), "f2").expect("f2 is on the message");
        assert_eq!(opened.file_id, "f2");
        assert_eq!(viewer.current().map(|one| one.file_id.as_str()), Some("f2"));
        // And a file that is not on it opens nothing at all.
        let mut other = Viewer::default();
        assert!(other.show(three(), "nope").is_none());
        assert!(!other.open());
    }

    /// Stepping wraps both ways: with three pictures the one after the last is
    /// the first, which is what a reader flicking through expects.
    #[test]
    fn stepping_wraps_in_both_directions() {
        let mut viewer = Viewer::default();
        viewer.show(three(), "f2");
        assert_eq!(
            viewer.step(1).map(|one| one.file_id),
            Some("f0".to_string())
        );
        assert_eq!(
            viewer.step(-1).map(|one| one.file_id),
            Some("f2".to_string())
        );
        // One picture has nowhere to step, and says so rather than fetching
        // the same one again.
        let mut alone = Viewer::default();
        alone.show(vec![three().remove(0)], "f0");
        assert!(alone.step(1).is_none());
    }

    /// A picture that arrives after the reader has stepped on is dropped.
    ///
    /// Two fetches can be in flight at once -- a reader flicking through
    /// outruns the network -- and the one that lands is not necessarily the
    /// one being looked at.
    #[test]
    fn a_late_picture_does_not_land_on_the_one_being_looked_at() {
        let mut viewer = Viewer::default();
        viewer.show(three(), "f0");
        viewer.step(1);
        viewer.arrived("f0", (800, 600));
        assert!(
            viewer.picture_rect(window()).is_none(),
            "f0 arrived while f1 was open"
        );
        viewer.arrived("f1", (800, 600));
        assert!(viewer.picture_rect(window()).is_some());
    }

    /// A picture larger than the window is fitted into it, and a small one is
    /// left at its own size rather than blown up.
    #[test]
    fn the_picture_is_fitted_and_never_enlarged() {
        let mut viewer = Viewer::default();
        viewer.show(three(), "f0");
        viewer.arrived("f0", (4000, 1000));
        let at = viewer.picture_rect(window()).expect("it is drawn");
        assert!(at.width <= window().width - MARGIN * 2.0 + 0.5);
        assert!(
            (at.width / at.height - 4.0).abs() < 0.01,
            "the shape is kept"
        );

        viewer.arrived("f0", (40, 30));
        let small = viewer.picture_rect(window()).expect("it is drawn");
        assert_eq!((small.width, small.height), (40.0, 30.0));
    }

    /// A press beside the picture shuts it; a press on the picture does not.
    #[test]
    fn pressing_beside_the_picture_shuts_it_and_pressing_it_does_not() {
        let mut viewer = Viewer::default();
        viewer.show(three(), "f0");
        viewer.arrived("f0", (200, 200));
        let placed = viewer.boxes(window());
        let picture = viewer.picture_rect(window()).expect("it is drawn");

        let mut input = Input::default();
        input.apply(
            matterless_ui::input::Event::PointerMoved {
                x: picture.x + 4.0,
                y: picture.y + 4.0,
            },
            &placed,
        );
        input.apply(matterless_ui::input::Event::PointerPressed, &placed);
        input.apply(matterless_ui::input::Event::PointerReleased, &placed);
        assert_eq!(viewer.react(&input), None);

        let mut beside = Input::default();
        beside.apply(
            matterless_ui::input::Event::PointerMoved { x: 4.0, y: 4.0 },
            &placed,
        );
        beside.apply(matterless_ui::input::Event::PointerPressed, &placed);
        beside.apply(matterless_ui::input::Event::PointerReleased, &placed);
        assert_eq!(viewer.react(&beside), Some(Did::Close));
    }

    /// A shut viewer answers nothing and places nothing.
    #[test]
    fn a_shut_viewer_answers_nothing() {
        let mut viewer = Viewer::default();
        assert!(viewer.boxes(window()).is_empty());
        assert_eq!(viewer.react(&Input::default()), None);
    }
}
