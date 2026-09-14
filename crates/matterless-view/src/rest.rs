//! How long the "New messages" divider stays once the reader can see it.
//!
//! The divider says where somebody left off. Left on screen it stops being
//! true: come back to the same conversation an hour later, having read every
//! word of it, and a line across the middle still claims the rest is new. The
//! web client leaves it until the channel is left, which is why it is so often
//! the oldest thing on the screen.
//!
//! So it is a moment, not a state. The clock runs only while the window has
//! focus -- a channel opened and then abandoned behind an editor has not been
//! read, and a divider that expired while nobody was looking would take the
//! only mark of where to start reading with it.

use std::time::{Duration, Instant};

/// How long the reader has to be looking before it goes.
///
/// Long enough to find it and start reading from it, short enough that it is
/// gone by the time they have finished the first message. Under a couple of
/// seconds it reads as a flicker -- something that appeared by mistake and was
/// taken back -- which is worse than not drawing it at all.
pub const REST: Duration = Duration::from_secs(4);

/// The clock on the divider in the open channel.
#[derive(Debug, Default, PartialEq)]
pub enum Rest {
    /// No divider, or one the reader asked for by marking a message unread,
    /// which is not ours to take away.
    #[default]
    Idle,
    /// Counting down. The window is focused and this is when it runs out.
    Running(Instant),
    /// Waiting for the window to come back, with this much still owed.
    Paused(Duration),
}

impl Rest {
    /// Starts the clock for a divider just put on screen.
    pub fn begin(&mut self, now: Instant, focused: bool) {
        *self = if focused {
            Rest::Running(now + REST)
        } else {
            Rest::Paused(REST)
        };
    }

    /// Stops it, for a divider that has gone or that must stay.
    pub fn stop(&mut self) {
        *self = Rest::Idle;
    }

    /// Follows the window in and out of focus.
    ///
    /// What is owed is kept rather than restarted: a reader glancing at
    /// another window twice would otherwise never spend four unbroken seconds
    /// here and the divider would never go.
    pub fn focus(&mut self, now: Instant, focused: bool) {
        *self = match (std::mem::take(self), focused) {
            (Rest::Running(until), false) => Rest::Paused(until.saturating_duration_since(now)),
            (Rest::Paused(left), true) => Rest::Running(now + left),
            (was, _) => was,
        };
    }

    /// Whether the wait is over, told once.
    pub fn ripened(&mut self, now: Instant) -> bool {
        let ripe = matches!(*self, Rest::Running(until) if now >= until);
        if ripe {
            *self = Rest::Idle;
        }
        ripe
    }

    /// When the window has to wake for it, so a frame arrives with nothing
    /// else happening. `None` whenever the clock is not running, or a quiet
    /// window would spin.
    pub fn wakes(&self) -> Option<Instant> {
        match *self {
            Rest::Running(until) => Some(until),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{REST, Rest};
    use std::time::{Duration, Instant};

    #[test]
    fn a_divider_goes_once_it_has_been_looked_at() {
        let now = Instant::now();
        let mut rest = Rest::default();
        rest.begin(now, true);
        assert!(!rest.ripened(now + REST - Duration::from_millis(1)));
        assert!(rest.ripened(now + REST));
        // And only once: the window asks every time it waits.
        assert!(!rest.ripened(now + REST * 2));
    }

    /// A channel opened behind another window has not been read.
    #[test]
    fn the_clock_does_not_run_while_the_window_is_away() {
        let now = Instant::now();
        let mut rest = Rest::default();
        rest.begin(now, false);
        assert_eq!(rest.wakes(), None, "nothing to wake for");
        assert!(!rest.ripened(now + REST * 10));
        rest.focus(now + REST * 10, true);
        assert!(!rest.ripened(now + REST * 10));
        assert!(rest.ripened(now + REST * 11));
    }

    /// Three seconds here, away, three seconds here: that is six seconds of
    /// looking, and the divider has long gone.
    #[test]
    fn what_is_owed_survives_looking_away() {
        let now = Instant::now();
        let mut rest = Rest::default();
        rest.begin(now, true);
        let left = REST - Duration::from_secs(3);
        rest.focus(now + Duration::from_secs(3), false);
        assert_eq!(rest, Rest::Paused(left));
        // An hour elsewhere costs it nothing.
        rest.focus(now + Duration::from_secs(3600), true);
        assert!(!rest.ripened(now + Duration::from_secs(3600)));
        assert!(rest.ripened(now + Duration::from_secs(3600) + left));
    }

    /// Marking a message unread is asking for the divider. Nothing here takes
    /// it away again.
    #[test]
    fn a_stopped_clock_never_ripens() {
        let now = Instant::now();
        let mut rest = Rest::default();
        rest.begin(now, true);
        rest.stop();
        assert_eq!(rest.wakes(), None);
        assert!(!rest.ripened(now + REST * 10));
        // Nor does coming back to the window start it again.
        rest.focus(now + REST, true);
        assert!(!rest.ripened(now + REST * 10));
    }
}
