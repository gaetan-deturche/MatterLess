//! What the window spends its time on, in the dev build.
//!
//! Everything between a click and the frame that answers it happens on the one
//! thread, so from outside all that can be seen is that the window stopped.
//! The measurement that found the first of these -- opening a channel of crash
//! reports shaping a quarter of a million characters -- was a `println!` added
//! for an afternoon and taken out again, which means the next person to make
//! something slow has to write it a second time.
//!
//! So the watches stay. They cost nothing in a release build: `watch` answers
//! `None`, no clock is read, and `Drop` has nothing to drop.
//!
//! Quiet by default. A line for every piece of work would bury the one that
//! matters under a thousand that took no time at all, so only work slow enough
//! to be felt says anything.

use std::time::{Duration, Instant};

/// Slow enough to be worth a line.
///
/// A frame at 60Hz is 16ms, and a stall has to be a good fraction of one
/// before a hand notices it. Under this the window still feels attached to the
/// pointer.
pub const SLOW: Duration = Duration::from_millis(6);

/// A piece of work being timed, which says what it cost when it goes out of
/// scope.
pub struct Watch {
    what: &'static str,
    over: usize,
    each: &'static str,
    began: Instant,
}

/// Starts a watch, in the dev build only.
///
/// `over` and `each` are how much work there was -- 263 "rows", 40 "channels"
/// -- because milliseconds alone never say whether something is slow or merely
/// large. Pass `0` for work that has no natural count.
///
/// Keep the returned value alive for as long as the work takes:
/// `let _open = timing::watch(...);`. Binding it to `_` drops it immediately
/// and times nothing.
#[must_use = "a watch dropped straight away measures nothing"]
pub fn watch(what: &'static str, over: usize, each: &'static str) -> Option<Watch> {
    cfg!(debug_assertions).then(|| Watch {
        what,
        over,
        each,
        began: Instant::now(),
    })
}

impl Drop for Watch {
    fn drop(&mut self) {
        let took = self.began.elapsed();
        if took >= SLOW {
            println!("{}", says(self.what, self.over, self.each, took));
        }
    }
}

/// The line a slow watch prints.
///
/// Separated from the printing so the wording can be read back in a test: the
/// per-unit figure is the whole point of the line, because it is what says
/// whether a channel was slow because it was big or slow because of the way
/// one row in it is shaped.
fn says(what: &str, over: usize, each: &str, took: Duration) -> String {
    let ms = took.as_millis();
    match over {
        0 => format!("slow: {what} took {ms}ms"),
        over => format!(
            "slow: {what} took {ms}ms for {over} {each}, {:.2}ms each",
            took.as_secs_f64() * 1000.0 / over as f64
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::{says, watch};
    use std::time::Duration;

    #[test]
    fn a_line_carries_what_it_was_and_what_each_of_it_cost() {
        assert_eq!(
            says("shaping the channel", 263, "rows", Duration::from_millis(2604)),
            "slow: shaping the channel took 2604ms for 263 rows, 9.90ms each"
        );
    }

    /// Work with nothing to count is still worth timing.
    #[test]
    fn a_line_without_a_count_leaves_it_out() {
        assert_eq!(
            says("the frame", 0, "rows", Duration::from_millis(40)),
            "slow: the frame took 40ms"
        );
    }

    /// Nothing divided by nothing.
    #[test]
    fn a_count_of_zero_never_divides_by_it() {
        assert!(!says("nothing at all", 0, "rows", Duration::ZERO).contains("each"));
    }

    /// The release build reads no clock at all.
    #[test]
    fn a_watch_exists_only_in_the_dev_build() {
        assert_eq!(
            watch("anything", 1, "rows").is_some(),
            cfg!(debug_assertions)
        );
    }
}
