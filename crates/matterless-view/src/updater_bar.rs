//! The offer of a newer build, across the top of the window.
//!
//! An offer, not a notice: nothing is fetched or replaced until it is
//! accepted, and it can be dismissed for the rest of the session. That is the
//! whole reason this exists rather than the client simply updating itself --
//! replacing the program somebody is running, and restarting it under them, is
//! theirs to agree to.
//!
//! And an offer somebody can act on: "What's new" opens the release's own
//! notes. Being asked to restart on the strength of a version number alone is
//! asking for a decision with nothing to make it on.
//!
//! A strip rather than a floating card, and one that takes its own room rather
//! than covering anything. A panel that hovers over a conversation hides the
//! thing the reader came for, and an offer nobody asked for has no business
//! doing that -- so the window is a row shorter while this is up, and exactly
//! as tall as it was once it goes.

use crate::sidebar::Canvas;
use matterless_paint::Run;
use matterless_ui::input::Input;
use matterless_ui::{Placed, Rect};

pub const NAME: &str = "update";

/// One row, and what sits in it.
///
/// Fixed: the strip pushes the whole window down by its own height, so a
/// height that depended on what it had to say would move the conversation
/// every time it changed its mind.
pub const HEIGHT: f32 = 38.0;
const PAD_X: f32 = 14.0;
const GAP: f32 = 8.0;
/// The sentence, and the buttons: `font-size: 13px` and `12.5px` padded
/// `5px 10px` at `border-radius: 5px`.
const SIZE: f32 = 13.0;
const BUTTON_SIZE: f32 = 12.5;
const BUTTON_PAD_X: f32 = 10.0;
const BUTTON_HEIGHT: f32 = 25.0;
const BUTTON_CORNER: f32 = 5.0;

/// What the reader did with the offer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Chose {
    /// Install it and restart into it.
    Install,
    /// Not now: put it away for the rest of the session.
    Later,
    /// Show what the release said about itself.
    News,
}

/// Where the strip's buttons were last measured to be.
#[derive(Debug, Clone, Copy)]
struct Laid {
    news: Rect,
    accept: Rect,
    dismiss: Rect,
    /// What room the sentence has left once the buttons have taken theirs.
    words: f32,
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
    /// What the release said about itself. Empty when it said nothing, and
    /// then there is no button to show it.
    notes: String,
    /// Where the buttons sit, measured when any of that last changed.
    ///
    /// Held rather than worked out per frame because measuring needs the
    /// fonts, and the hit test does not have them.
    placed: Option<Laid>,
}

impl Bar {
    pub fn open(&self) -> bool {
        !self.version.is_empty()
    }

    /// How much of the window this is taking. Nothing, when it is not up.
    ///
    /// The shell asks before it lays anything out, which is why it cannot
    /// depend on having been measured.
    pub fn height(&self) -> f32 {
        match self.open() {
            true => HEIGHT,
            false => 0.0,
        }
    }

    /// What the release said about itself, for whatever shows it.
    pub fn notes(&self) -> &str {
        &self.notes
    }

    pub fn offer(&mut self, version: &str, notes: &str) {
        self.version = version.to_string();
        self.notes = notes.trim().to_string();
        self.working = false;
        self.failed.clear();
        self.placed = None;
    }

    pub fn hide(&mut self) {
        self.version.clear();
        self.notes.clear();
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

    /// Whether the release said anything about itself.
    ///
    /// No notes, no button: offering to show nothing and then showing nothing
    /// is worse than not offering.
    pub fn has_notes(&self) -> bool {
        !self.notes.is_empty()
    }

    /// The sentence, which says what happened rather than only what is on
    /// offer: a failed install is the one thing a reader needs told.
    fn sentence(&self) -> String {
        match self.failed.is_empty() {
            true => format!("MatterLess {} is available.", self.version),
            false => format!(
                "MatterLess {} could not be installed: {}",
                self.version, self.failed
            ),
        }
    }

    fn accept(&self) -> &'static str {
        match self.working {
            true => "Installing\u{2026}",
            false => "Install and restart",
        }
    }

    /// How wide a run of text is, so a button is sized to its own label.
    fn wide(fonts: &mut matterless_layout::Fonts, text: &str, size: f32) -> f32 {
        Self::style(size, |style| {
            matterless_layout::extent_of(fonts, text, f32::MAX, style).width
        })
    }

    fn style<T>(size: f32, with: impl FnOnce(matterless_layout::Style) -> T) -> T {
        with(matterless_layout::Style {
            size,
            line_height: size * 1.4,
            bold: false,
            italic: false,
            mono: false,
        })
    }

    /// Works out where the buttons go, which is the one thing here that needs
    /// fonts.
    pub fn measure(&mut self, fonts: &mut matterless_layout::Fonts, within: Rect) {
        self.placed = self.open().then(|| self.place(fonts, within));
    }

    fn laid(&self) -> Option<Laid> {
        self.placed
    }

    /// The buttons, right to left from the far edge, and what is left over.
    fn place(&self, fonts: &mut matterless_layout::Fonts, within: Rect) -> Laid {
        let dismiss = Self::wide(fonts, "Not now", BUTTON_SIZE) + BUTTON_PAD_X * 2.0;
        let accept = Self::wide(fonts, self.accept(), BUTTON_SIZE) + BUTTON_PAD_X * 2.0;
        let news = match self.has_notes() {
            true => Self::wide(fonts, "What's new", BUTTON_SIZE) + BUTTON_PAD_X * 2.0,
            false => 0.0,
        };
        let row = within.y + (HEIGHT - BUTTON_HEIGHT) / 2.0;
        let dismiss_at = Rect::new(
            within.right() - PAD_X - dismiss,
            row,
            dismiss,
            BUTTON_HEIGHT,
        );
        let accept_at = Rect::new(dismiss_at.x - GAP - accept, row, accept, BUTTON_HEIGHT);
        let news_at = Rect::new(accept_at.x - GAP - news, row, news, BUTTON_HEIGHT);
        let taken = match self.has_notes() {
            true => news_at.x,
            false => accept_at.x,
        };
        Laid {
            news: news_at,
            accept: accept_at,
            dismiss: dismiss_at,
            // The sentence gets what the buttons left, and is cut to it rather
            // than drawn over them.
            words: (taken - GAP - (within.x + PAD_X)).max(0.0),
        }
    }

    pub fn boxes(&self, within: Rect) -> Vec<Placed> {
        let Some(laid) = self.laid() else {
            return Vec::new();
        };
        // The strip itself first, so a press on it does not fall through to
        // whatever the window put underneath.
        let mut placed = vec![Placed {
            name: NAME.to_string(),
            rect: Rect::new(within.x, within.y, within.width, HEIGHT),
            depth: 20,
        }];
        // While it is working there is nothing to press: the buttons are drawn
        // dimmed and are not there to be hit, which is what `disabled` does.
        if !self.working {
            placed.push(Placed {
                name: format!("{NAME}/install"),
                rect: laid.accept,
                depth: 21,
            });
            placed.push(Placed {
                name: format!("{NAME}/later"),
                rect: laid.dismiss,
                depth: 21,
            });
            if self.has_notes() {
                placed.push(Placed {
                    name: format!("{NAME}/news"),
                    rect: laid.news,
                    depth: 21,
                });
            }
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
                self.placed = None;
                Some(Chose::Install)
            }
            name if name == format!("{NAME}/later") => {
                self.hide();
                Some(Chose::Later)
            }
            name if name == format!("{NAME}/news") => Some(Chose::News),
            _ => None,
        }
    }

    pub fn draw(&self, into: &mut Canvas<'_>, input: &Input, within: Rect) {
        let Some(laid) = self.laid() else {
            return;
        };
        let strip = Rect::new(within.x, within.y, within.width, HEIGHT);
        let Canvas {
            scene,
            painter,
            fonts,
            palette,
        } = into;
        // `signal_soft` is the ground for something signalled, which is what
        // this is -- and a line under it, because the strip has taken the room
        // the window above it used to have and the edge is what says so.
        scene.fill(
            strip.x,
            strip.y,
            strip.width,
            strip.height,
            palette.signal_soft,
        );
        scene.fill(
            strip.x,
            strip.bottom() - 1.0,
            strip.width,
            1.0,
            palette.rule,
        );

        // Cut to the room the buttons left rather than drawn under them. A
        // reason can be a paragraph and the strip is one row.
        let sentence = Self::style(SIZE, |style| {
            matterless_layout::elided(fonts, &self.sentence(), laid.words, style)
        });
        let ink = match self.failed.is_empty() {
            true => palette.ink,
            false => palette.flag,
        };
        let glyphs = painter.run(
            fonts,
            &sentence,
            strip.x + PAD_X,
            strip.y + (HEIGHT - SIZE * 1.4) / 2.0,
            Run::label(f32::MAX).sized(SIZE),
        );
        scene.glyphs(glyphs, ink, palette.faint);

        let mut buttons = vec![
            (laid.accept, self.accept(), "install", true),
            (laid.dismiss, "Not now", "later", false),
        ];
        if self.has_notes() {
            buttons.push((laid.news, "What's new", "news", false));
        }
        // The one that does something takes the signal colour; the ones that
        // do not are plain, which is the difference between the answers.
        for (rect, label, slug, primary) in buttons {
            let under = !self.working && input.hovered() == Some(format!("{NAME}/{slug}").as_str());
            let ground = if primary {
                [palette.signal[0], palette.signal[1], palette.signal[2], 255]
            } else if under {
                palette.raised
            } else {
                palette.surface
            };
            scene.rounded(
                rect.x,
                rect.y,
                rect.width,
                rect.height,
                if primary {
                    [palette.signal[0], palette.signal[1], palette.signal[2], 255]
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
            let ink = match primary {
                true => [255, 255, 255],
                false => palette.ink,
            };
            // Dimmed while it is working, which is what `:disabled` does and
            // the only thing saying the press was heard.
            let ink = match self.working {
                true => palette.dimmed(ink, 0.55),
                false => ink,
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

    fn told(notes: &str) -> (Bar, matterless_layout::Fonts) {
        let mut bar = Bar::default();
        let mut fonts = matterless_layout::Fonts::new();
        bar.offer("0.1.6", notes);
        bar.measure(&mut fonts, window());
        (bar, fonts)
    }

    fn offered() -> (Bar, matterless_layout::Fonts) {
        told("a change list, of sorts")
    }

    /// Nothing is offered until there is something to offer, and it takes no
    /// room until then -- the window is exactly as tall as it was.
    #[test]
    fn it_is_not_there_until_there_is_a_build() {
        let mut fonts = matterless_layout::Fonts::new();
        let mut bar = Bar::default();
        bar.measure(&mut fonts, window());
        assert!(!bar.open());
        assert!(bar.boxes(window()).is_empty());
        assert_eq!(bar.height(), 0.0);
    }

    /// One row across the top, and the room it takes is the room the shell
    /// gives it: a strip measuring one thing and reserving another would draw
    /// over the window or leave a gap under itself.
    #[test]
    fn it_is_a_row_across_the_top() {
        let (bar, _fonts) = offered();
        assert_eq!(bar.height(), HEIGHT);
        let boxes = bar.boxes(window());
        let strip = &boxes[0];
        assert_eq!(strip.rect.y, window().y);
        assert_eq!(strip.rect.width, window().width);
        assert_eq!(strip.rect.height, bar.height());

        let laid = bar.laid().expect("measured");
        // The buttons are inside it, in order, and none of them hangs off.
        assert!(laid.news.right() <= laid.accept.x);
        assert!(laid.accept.right() <= laid.dismiss.x);
        assert!(laid.dismiss.right() <= window().right());
        assert!(laid.news.y >= strip.rect.y);
        assert!(laid.news.bottom() <= strip.rect.bottom());
    }

    /// The sentence gets what the buttons left. Drawn at its full width it
    /// would run under them, and a reason can be a paragraph.
    #[test]
    fn the_sentence_is_cut_to_the_room_the_buttons_left() {
        let (mut bar, mut fonts) = offered();
        bar.failed(&"a very long reason indeed, ".repeat(20));
        bar.measure(&mut fonts, window());
        let laid = bar.laid().expect("measured");
        assert!(laid.words > 0.0);
        let cut = Bar::style(SIZE, |style| {
            matterless_layout::elided(&mut fonts, &bar.sentence(), laid.words, style)
        });
        let drawn = Bar::wide(&mut fonts, &cut, SIZE);
        assert!(drawn <= laid.words + 1.0, "{drawn} into {}", laid.words);
        assert!(cut.ends_with(matterless_layout::ELLIPSIS), "{cut:?}");
    }

    /// A release that said nothing about itself offers no button. Showing one
    /// that opens an empty panel is worse than not showing one.
    #[test]
    fn a_release_that_says_nothing_offers_no_button() {
        let (bar, _fonts) = told("   ");
        assert!(bar.open());
        assert!(!bar.has_notes());
        assert!(
            !bar.boxes(window())
                .iter()
                .any(|placed| placed.name == "update/news"),
            "a button for nothing"
        );
    }

    /// Asking for the notes leaves the strip alone: it is one row before and
    /// one row after, and something else shows them.
    #[test]
    fn asking_for_the_notes_does_not_move_the_window() {
        let (mut bar, _fonts) = offered();
        let boxes = bar.boxes(window());
        let button = boxes
            .iter()
            .find(|placed| placed.name == "update/news")
            .expect("the button");
        let mut input = Input::default();
        press(&mut input, &boxes, button.rect);
        assert_eq!(bar.react(&input), Some(Chose::News));
        assert_eq!(bar.height(), HEIGHT);
        assert!(bar.open(), "asking to read is not dismissing");
    }

    /// Accepting says so once. A second press while the download is in flight
    /// would start a second download.
    #[test]
    fn it_can_only_be_accepted_once() {
        let (mut bar, mut fonts) = offered();
        let boxes = bar.boxes(window());
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
        let working = bar.boxes(window());
        assert!(
            !working.iter().any(|placed| placed.name.contains('/')),
            "a button survived the press"
        );
        assert_eq!(bar.react(&input), None);
    }

    /// "Not now" puts it away for the session, and gives the room back.
    #[test]
    fn it_can_be_put_away() {
        let (mut bar, _fonts) = offered();
        let boxes = bar.boxes(window());
        let button = boxes
            .iter()
            .find(|placed| placed.name == "update/later")
            .expect("the button");
        let mut input = Input::default();
        press(&mut input, &boxes, button.rect);
        assert_eq!(bar.react(&input), Some(Chose::Later));
        assert!(!bar.open());
        assert_eq!(bar.height(), 0.0);
    }

    /// A failure is shown and can be answered again: the reader may have been
    /// offline, and refusing to try twice would mean restarting the client to
    /// take an update it already knows about.
    #[test]
    fn a_failure_can_be_tried_again() {
        let (mut bar, mut fonts) = offered();
        let boxes = bar.boxes(window());
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
        // The buttons are back, and it is still one row: the reason is cut to
        // fit rather than given a line of its own.
        let again = bar.boxes(window());
        assert!(again.iter().any(|placed| placed.name == "update/install"));
        assert_eq!(bar.height(), HEIGHT);
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
