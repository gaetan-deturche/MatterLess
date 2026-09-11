//! The offer of a newer build, in the corner.
//!
//! An offer, not a notice: nothing is fetched or replaced until it is
//! accepted, and it can be dismissed for the rest of the session. That is the
//! whole reason this exists rather than the client simply updating itself --
//! replacing the program somebody is running, and restarting it under them, is
//! theirs to agree to.

use crate::sidebar::Canvas;
use matterless_paint::Run;
use matterless_ui::input::Input;
use matterless_ui::{Placed, Rect};

pub const NAME: &str = "update";

/// `right: 16px; bottom: 16px`, padded `10px 12px` with `gap: 8px`, a hairline
/// round it at `border-radius: 8px` under a `0 8px 24px` shadow.
const MARGIN: f32 = 16.0;
const PAD_X: f32 = 12.0;
const PAD_Y: f32 = 10.0;
const GAP: f32 = 8.0;
const CORNER: f32 = 8.0;
const DROP: f32 = 8.0;
/// The sentence, and the buttons: `font-size: 13px` and `12.5px` padded
/// `5px 10px` at `border-radius: 5px`.
const SIZE: f32 = 13.0;
const BUTTON_SIZE: f32 = 12.5;
const BUTTON_PAD_X: f32 = 10.0;
const BUTTON_HEIGHT: f32 = 25.0;
const BUTTON_CORNER: f32 = 5.0;
/// What a failure is written at: `font-size: 11.5px` in the flag colour.
const FAILED_SIZE: f32 = 11.5;

/// What the reader did with the offer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Chose {
    /// Install it and restart into it.
    Install,
    /// Not now: put it away for the rest of the session.
    Later,
}

/// The offer currently on screen, if there is one.
#[derive(Debug, Default)]
pub struct Bar {
    /// What is being offered. Empty when nothing is.
    version: String,
    /// True once it has been accepted, so it cannot be accepted twice while
    /// the download is in flight.
    working: bool,
    /// Why it did not work, if it did not.
    failed: String,
    /// Where it and its two buttons sit, measured when any of that last
    /// changed.
    ///
    /// Held rather than worked out per frame because measuring needs the
    /// fonts, and the hit test does not have them: everything that can change
    /// the size of this -- a new offer, a failure under it, a window resized
    /// around it -- goes through `measure`.
    placed: Option<(Rect, Rect, Rect)>,
}

impl Bar {
    pub fn open(&self) -> bool {
        !self.version.is_empty()
    }

    pub fn offer(&mut self, version: &str) {
        self.version = version.to_string();
        self.working = false;
        self.failed.clear();
        self.placed = None;
    }

    pub fn hide(&mut self) {
        self.version.clear();
        self.working = false;
        self.failed.clear();
        self.placed = None;
    }

    /// Says why the install did not work, and lets it be tried again.
    pub fn failed(&mut self, why: &str) {
        self.failed = why.to_string();
        self.working = false;
        self.placed = None;
    }

    /// The three lines of text, which decide how wide it is.
    fn said(&self) -> (String, &'static str, &'static str) {
        (
            format!("MatterLess {} is available.", self.version),
            if self.working {
                "Installing\u{2026}"
            } else {
                "Install and restart"
            },
            "Not now",
        )
    }

    /// How wide a run of text is, so the panel can be sized to its contents
    /// rather than to a guess.
    fn wide(fonts: &mut matterless_layout::Fonts, text: &str, size: f32) -> f32 {
        matterless_layout::extent_of(
            fonts,
            text,
            f32::MAX,
            matterless_layout::Style {
                size,
                line_height: size * 1.4,
                bold: false,
                italic: false,
                mono: false,
            },
        )
        .width
    }

    /// Works out where it goes, which is the one thing here that needs fonts.
    ///
    /// Called whenever what it says or the room it has changes, and never per
    /// frame: the hit test and the drawing both read what this leaves behind.
    pub fn measure(&mut self, fonts: &mut matterless_layout::Fonts, within: Rect) {
        self.placed = self.open().then(|| self.place(fonts, within));
    }

    /// Where it and its buttons were last measured to be.
    fn laid(&self) -> Option<(Rect, Rect, Rect)> {
        self.placed
    }

    /// The panel, and the two buttons inside it.
    fn place(&self, fonts: &mut matterless_layout::Fonts, within: Rect) -> (Rect, Rect, Rect) {
        let (sentence, install, later) = self.said();
        let words = Self::wide(fonts, &sentence, SIZE);
        let accept = Self::wide(fonts, install, BUTTON_SIZE) + BUTTON_PAD_X * 2.0;
        let dismiss = Self::wide(fonts, later, BUTTON_SIZE) + BUTTON_PAD_X * 2.0;
        let mut width = PAD_X * 2.0 + words + GAP + accept + GAP + dismiss;
        let mut height = PAD_Y * 2.0 + BUTTON_HEIGHT;
        if !self.failed.is_empty() {
            // The reason goes under the sentence rather than beside it: it can
            // be a paragraph, and a panel as wide as one would cover the
            // conversation it is floating over.
            width = width.max(PAD_X * 2.0 + Self::wide(fonts, &self.failed, FAILED_SIZE));
            height += FAILED_SIZE * 1.4;
        }
        let width = width.min(within.width - MARGIN * 2.0);
        let panel = Rect::new(
            within.right() - MARGIN - width,
            within.bottom() - MARGIN - height,
            width,
            height,
        );
        let row = panel.y + PAD_Y;
        let accept_at = Rect::new(
            panel.right() - PAD_X - dismiss - GAP - accept,
            row,
            accept,
            BUTTON_HEIGHT,
        );
        let dismiss_at = Rect::new(panel.right() - PAD_X - dismiss, row, dismiss, BUTTON_HEIGHT);
        (panel, accept_at, dismiss_at)
    }

    pub fn boxes(&self) -> Vec<Placed> {
        let Some((panel, accept, dismiss)) = self.laid() else {
            return Vec::new();
        };
        let mut placed = vec![Placed {
            name: NAME.to_string(),
            rect: panel,
            depth: 30,
        }];
        // While it is working there is nothing to press: the buttons are drawn
        // dimmed and are not there to be hit, which is what `disabled` does.
        if !self.working {
            placed.push(Placed {
                name: format!("{NAME}/install"),
                rect: accept,
                depth: 31,
            });
            placed.push(Placed {
                name: format!("{NAME}/later"),
                rect: dismiss,
                depth: 31,
            });
        }
        placed
    }

    pub fn react(&mut self, input: &Input) -> Option<Chose> {
        if !self.open() || self.working {
            return None;
        }
        match input.clicked()? {
            name if name == format!("{NAME}/install") => {
                self.working = true;
                self.failed.clear();
                Some(Chose::Install)
            }
            name if name == format!("{NAME}/later") => {
                self.hide();
                Some(Chose::Later)
            }
            _ => None,
        }
    }

    pub fn draw(&self, into: &mut Canvas<'_>, input: &Input) {
        let Some((panel, accept, dismiss)) = self.laid() else {
            return;
        };
        let (sentence, install, later) = self.said();
        let Canvas {
            scene,
            painter,
            fonts,
            palette,
        } = into;
        // A hairline round it, drawn as the ground inset by one.
        scene.floating(
            panel.x,
            panel.y,
            panel.width,
            panel.height,
            palette.rule,
            CORNER,
            DROP,
        );
        scene.rounded(
            panel.x + 1.0,
            panel.y + 1.0,
            panel.width - 2.0,
            panel.height - 2.0,
            palette.surface,
            CORNER - 1.0,
        );
        let glyphs = painter.run(
            fonts,
            &sentence,
            panel.x + PAD_X,
            panel.y + PAD_Y + (BUTTON_HEIGHT - SIZE * 1.4) / 2.0,
            Run::label(f32::MAX).sized(SIZE),
        );
        scene.glyphs(glyphs, palette.ink, palette.faint);

        if !self.failed.is_empty() {
            let glyphs = painter.run(
                fonts,
                &self.failed,
                panel.x + PAD_X,
                panel.y + PAD_Y + BUTTON_HEIGHT,
                Run::label(panel.width - PAD_X * 2.0).sized(FAILED_SIZE),
            );
            scene.glyphs(glyphs, palette.flag, palette.faint);
        }

        // The one that does something takes the signal colour; the one that
        // puts it away is a plain button, which is the difference between the
        // two answers.
        for (rect, label, primary) in [(accept, install, true), (dismiss, later, false)] {
            let under = !self.working
                && input.hovered()
                    == Some(
                        format!("{NAME}/{}", if primary { "install" } else { "later" }).as_str(),
                    );
            let edge = if primary { palette.signal } else { [0, 0, 0] };
            let ground = if primary {
                [palette.signal[0], palette.signal[1], palette.signal[2], 255]
            } else if under {
                palette.raised
            } else {
                palette.ground
            };
            scene.rounded(
                rect.x,
                rect.y,
                rect.width,
                rect.height,
                if primary {
                    [edge[0], edge[1], edge[2], 255]
                } else {
                    palette.rule
                },
                BUTTON_CORNER,
            );
            scene.rounded(
                rect.x + 1.0,
                rect.y + 1.0,
                rect.width - 2.0,
                rect.height - 2.0,
                ground,
                BUTTON_CORNER - 1.0,
            );
            let ink = if primary {
                [255, 255, 255]
            } else {
                palette.ink
            };
            // Dimmed while it is working, which is what `:disabled` does and
            // the only thing saying the press was heard.
            let ink = if self.working {
                palette.dimmed(ink, 0.55)
            } else {
                ink
            };
            let glyphs = painter.run(
                fonts,
                label,
                rect.x + BUTTON_PAD_X,
                rect.y + (rect.height - BUTTON_SIZE * 1.4) / 2.0,
                Run::label(f32::MAX).sized(BUTTON_SIZE),
            );
            scene.glyphs(glyphs, ink, palette.faint);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn window() -> Rect {
        Rect::new(0.0, 0.0, 900.0, 600.0)
    }

    fn offered() -> (Bar, matterless_layout::Fonts) {
        let mut bar = Bar::default();
        let mut fonts = matterless_layout::Fonts::new();
        bar.offer("0.1.6");
        bar.measure(&mut fonts, window());
        (bar, fonts)
    }

    /// Nothing is offered until there is something to offer.
    #[test]
    fn it_is_not_there_until_there_is_a_build() {
        let mut fonts = matterless_layout::Fonts::new();
        let mut bar = Bar::default();
        bar.measure(&mut fonts, window());
        assert!(!bar.open());
        assert!(bar.boxes().is_empty());
    }

    /// In the corner, and on the window.
    #[test]
    fn it_sits_in_the_corner() {
        let (bar, _fonts) = offered();
        let (panel, accept, dismiss) = bar.laid().expect("measured");
        assert!(panel.right() <= window().right() && panel.bottom() <= window().bottom());
        assert!(panel.x >= window().x && panel.y >= window().y);
        // The two answers do not overlap, and both are inside the panel.
        assert!(accept.right() <= dismiss.x);
        assert!(dismiss.right() <= panel.right());
        assert!(accept.x >= panel.x);
    }

    /// Accepting says so once. A second press while the download is in flight
    /// would start a second download.
    #[test]
    fn it_can_only_be_accepted_once() {
        let (mut bar, mut fonts) = offered();
        let boxes = bar.boxes();
        let button = boxes
            .iter()
            .find(|placed| placed.name == "update/install")
            .expect("the button");
        let mut input = Input::default();
        press(&mut input, &boxes, button.rect);
        assert_eq!(bar.react(&input), Some(Chose::Install));

        // It stays on screen -- the install is happening -- but there is
        // nothing left to press.
        assert!(bar.open());
        bar.measure(&mut fonts, window());
        let working = bar.boxes();
        assert!(
            !working.iter().any(|placed| placed.name.contains('/')),
            "a button survived the press"
        );
        assert_eq!(bar.react(&input), None);
    }

    /// "Not now" puts it away for the session.
    #[test]
    fn it_can_be_put_away() {
        let (mut bar, _fonts) = offered();
        let boxes = bar.boxes();
        let button = boxes
            .iter()
            .find(|placed| placed.name == "update/later")
            .expect("the button");
        let mut input = Input::default();
        press(&mut input, &boxes, button.rect);
        assert_eq!(bar.react(&input), Some(Chose::Later));
        assert!(!bar.open());
    }

    /// A failure is shown and can be answered again: the reader may have been
    /// offline, and refusing to try twice would mean restarting the client to
    /// take an update it already knows about.
    #[test]
    fn a_failure_can_be_tried_again() {
        let (mut bar, mut fonts) = offered();
        let boxes = bar.boxes();
        let button = boxes
            .iter()
            .find(|placed| placed.name == "update/install")
            .expect("the button");
        let mut input = Input::default();
        press(&mut input, &boxes, button.rect);
        bar.react(&input);
        bar.failed("the download is not what was signed");
        assert!(bar.open());
        bar.measure(&mut fonts, window());
        // The buttons are back.
        let again = bar.boxes();
        assert!(again.iter().any(|placed| placed.name == "update/install"));
        // And it is taller, because the reason is on it.
        let (bigger, _, _) = bar.laid().expect("measured");
        let mut quiet = Bar::default();
        quiet.offer("0.1.6");
        quiet.measure(&mut fonts, window());
        let (smaller, _, _) = quiet.laid().expect("measured");
        assert!(bigger.height > smaller.height);
    }

    fn press(input: &mut Input, boxes: &[Placed], rect: Rect) {
        input.apply(
            matterless_ui::input::Event::PointerMoved {
                x: rect.x + rect.width / 2.0,
                y: rect.y + rect.height / 2.0,
            },
            boxes,
        );
        input.apply(matterless_ui::input::Event::PointerPressed, boxes);
        input.apply(matterless_ui::input::Event::PointerReleased, boxes);
    }
}
