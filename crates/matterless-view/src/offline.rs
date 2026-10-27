//! Saying the window is offline, across the top of it.
//!
//! What is on screen while the server cannot be reached is the store's copy of
//! the conversations, and it looks exactly like the live ones: without this a
//! window started before the network was up read as one where nobody had said
//! anything for a while. A strip rather than a screen, so what is already here
//! can still be read; and it says when the next try is, with a button that
//! does not wait for it.
//!
//! The same room the update strip takes, above it: it pushes the window down
//! by its own height rather than covering anything.

use std::time::{Duration, Instant};

use matterless_paint::Run;
use matterless_ui::input::Input;
use matterless_ui::{Placed, Rect};
use matterless_widgets::{Button, Canvas, Laid, Row};

pub const NAME: &str = "offline";

/// One row, as the update strip is.
pub const HEIGHT: f32 = 38.0;
const PAD_X: f32 = 14.0;
const SIZE: f32 = 13.0;

/// Where the connection stands, when it is not up.
#[derive(Debug, Clone, PartialEq)]
enum State {
    /// Up, or not known to be down: no strip.
    Up,
    /// The server could not be reached, and is tried again then.
    Waiting { until: Instant, why: String },
    /// Being tried again right now.
    Trying,
    /// Reached once and lost since: the socket finds its own way back.
    Lost,
}

/// The strip, and the countdown it last drew.
#[derive(Debug)]
pub struct Strip {
    state: State,
    /// The whole seconds the countdown said when last drawn, so the window is
    /// redrawn when it changes and not on every turn of the loop.
    said: Option<u64>,
    placed: Option<Laid>,
}

impl Default for Strip {
    fn default() -> Self {
        Self {
            state: State::Up,
            said: None,
            placed: None,
        }
    }
}

/// What the reader asked of it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Chose {
    /// Try now rather than when the countdown runs out.
    Retry,
}

impl Strip {
    pub fn open(&self) -> bool {
        self.state != State::Up
    }

    /// How much of the window this takes: nothing when it is not up.
    pub fn height(&self) -> f32 {
        match self.open() {
            true => HEIGHT,
            false => 0.0,
        }
    }

    /// The server could not be reached, and will be tried again in `again`.
    pub fn unreachable(&mut self, again: Duration, why: &str) {
        self.state = State::Waiting {
            until: Instant::now() + again,
            why: why.to_string(),
        };
        self.placed = None;
    }

    /// A try is under way.
    pub fn trying(&mut self) {
        self.state = State::Trying;
        self.placed = None;
    }

    /// The live connection went, having been up.
    pub fn lost(&mut self) {
        if self.state == State::Up {
            self.state = State::Lost;
            self.placed = None;
        }
    }

    /// Reached: the strip goes.
    pub fn reached(&mut self) {
        self.state = State::Up;
        self.said = None;
        self.placed = None;
    }

    /// What it says, now.
    fn sentence(&self, now: Instant) -> String {
        match &self.state {
            State::Up => String::new(),
            State::Waiting { until, why } => {
                let left = until.saturating_duration_since(now).as_secs_f32().ceil() as u64;
                let shown =
                    format!("Offline: the server cannot be reached. Trying again in {left} s.");
                match why.is_empty() {
                    true => shown,
                    false => format!("{shown}  ({why})"),
                }
            }
            State::Trying => "Offline: trying to reach the server\u{2026}".to_string(),
            State::Lost => "Connection lost. Reconnecting\u{2026}".to_string(),
        }
    }

    /// Whether the countdown now says something it did not last frame, which
    /// is the one reason this strip needs a frame with nothing else happening.
    pub fn ticked(&mut self, now: Instant) -> bool {
        let State::Waiting { until, .. } = &self.state else {
            return false;
        };
        let left = until.saturating_duration_since(now).as_secs_f32().ceil() as u64;
        let changed = self.said != Some(left);
        self.said = Some(left);
        changed
    }

    /// When the countdown next changes, so the window can wake for it.
    pub fn wakes(&self, now: Instant) -> Option<Instant> {
        let State::Waiting { until, .. } = &self.state else {
            return None;
        };
        let left = until.saturating_duration_since(now);
        // The next whole second, counting down to the try itself.
        let fraction = Duration::from_secs_f32(left.as_secs_f32().fract());
        Some(now + fraction.max(Duration::from_millis(50)))
    }

    fn row(&self, within: Rect) -> Row {
        let retry = !matches!(self.state, State::Lost);
        Row::new(NAME, Rect::new(within.x, within.y, within.width, HEIGHT))
            .pad(PAD_X)
            .depth(21)
            // Nothing to press while a try is already out.
            .enabled(self.state != State::Trying)
            .maybe(retry.then(|| Button::plain("retry", "Retry now")))
    }

    pub fn measure(&mut self, fonts: &mut matterless_layout::Fonts, within: Rect) {
        self.placed = self.open().then(|| self.row(within).measure(fonts));
    }

    pub fn boxes(&self, within: Rect) -> Vec<Placed> {
        let Some(laid) = self.placed.as_ref() else {
            return Vec::new();
        };
        let mut placed = vec![Placed {
            name: NAME.to_string(),
            rect: Rect::new(within.x, within.y, within.width, HEIGHT),
            depth: 20,
        }];
        placed.extend(laid.boxes());
        placed
    }

    pub fn react(&mut self, input: &Input) -> Option<Chose> {
        if !self.open() {
            return None;
        }
        match self.placed.as_ref()?.clicked(input)? {
            "retry" => {
                self.trying();
                Some(Chose::Retry)
            }
            _ => None,
        }
    }

    pub fn draw(&self, into: &mut Canvas<'_>, input: &Input, within: Rect) {
        let Some(laid) = self.placed.clone() else {
            return;
        };
        let strip = Rect::new(within.x, within.y, within.width, HEIGHT);
        let sentence = self.sentence(Instant::now());
        {
            let Canvas {
                scene,
                painter,
                fonts,
                palette,
            } = into;
            scene.fill(strip.x, strip.y, strip.width, strip.height, palette.raised);
            scene.fill(
                strip.x,
                strip.bottom() - 1.0,
                strip.width,
                1.0,
                palette.rule,
            );
            let sentence = matterless_layout::elided(
                fonts,
                &sentence,
                laid.spare(),
                matterless_widgets::style(SIZE),
            );
            let glyphs = painter.run(
                fonts,
                &sentence,
                strip.x + PAD_X,
                strip.y + (HEIGHT - SIZE * 1.4) / 2.0,
                Run::label(f32::MAX).sized(SIZE),
            );
            scene.glyphs(glyphs, palette.danger, palette.faint);
        }
        laid.draw(into, input);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Down says so and when it tries again; reached, it goes.
    #[test]
    fn it_says_when_the_next_try_is() {
        let mut strip = Strip::default();
        assert!(!strip.open());
        assert_eq!(strip.height(), 0.0);
        strip.unreachable(Duration::from_secs(8), "");
        assert!(strip.open());
        assert_eq!(strip.height(), HEIGHT);
        let now = Instant::now();
        assert!(
            strip.sentence(now).contains("Trying again in 8 s"),
            "{}",
            strip.sentence(now)
        );
        assert!(strip.ticked(now), "the first frame draws it");
        assert!(!strip.ticked(now), "and the same second does not again");
        assert!(strip.wakes(now).is_some());
        strip.reached();
        assert!(!strip.open());
        assert_eq!(strip.wakes(now), None);
    }

    /// A connection lost after it was up says so, but only when nothing worse
    /// is already being said.
    #[test]
    fn a_lost_connection_does_not_hide_a_countdown() {
        let mut strip = Strip::default();
        strip.unreachable(Duration::from_secs(8), "");
        strip.lost();
        assert!(matches!(strip.state, State::Waiting { .. }));
        strip.reached();
        strip.lost();
        assert_eq!(strip.state, State::Lost);
    }
}
