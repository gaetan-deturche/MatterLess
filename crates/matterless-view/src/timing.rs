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

use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};
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
        add(self.what, took);
        if took >= SLOW {
            println!("{}", says(self.what, self.over, self.each, took));
        }
    }
}

/// What every watch in this run has cost, added up.
///
/// The printed line answers "what just stalled". This answers "where does a
/// frame go", which needs every watch rather than only the ones over the bar:
/// six pieces of work at 3ms each say nothing one at a time and are the whole
/// frame together.
static TALLY: LazyLock<Mutex<HashMap<&'static str, Tallied>>> = LazyLock::new(Default::default);

#[derive(Default, Clone, Copy)]
struct Tallied {
    runs: u32,
    total: Duration,
    worst: Duration,
}

fn add(what: &'static str, took: Duration) {
    let Ok(mut tally) = TALLY.lock() else { return };
    let seen = tally.entry(what).or_default();
    seen.runs += 1;
    seen.total += took;
    seen.worst = seen.worst.max(took);
}

/// Throws the tally away, so what comes after is measured without it.
///
/// For the warm-up before a measured run: the first frames of a window carry
/// the fonts being loaded and the atlas being filled, which are real costs and
/// are not what a frame costs.
pub fn forget() {
    if let Ok(mut tally) = TALLY.lock() {
        tally.clear();
    }
}

/// Everything measured so far, dearest first.
pub fn table() -> String {
    let Ok(tally) = TALLY.lock() else {
        return String::new();
    };
    let mut rows: Vec<(&str, Tallied)> = tally.iter().map(|(what, seen)| (*what, *seen)).collect();
    rows.sort_by_key(|(_, seen)| std::cmp::Reverse(seen.total));
    let mut out = format!(
        "\n{:<28}{:>7}{:>12}{:>12}{:>12}\n",
        "what", "runs", "total", "mean", "worst"
    );
    for (what, seen) in rows {
        out.push_str(&format!(
            "{:<28}{:>7}{:>11.1}{}{:>11.2}{}{:>11.2}{}\n",
            what,
            seen.runs,
            seen.total.as_secs_f64() * 1000.0,
            "ms",
            seen.total.as_secs_f64() * 1000.0 / seen.runs.max(1) as f64,
            "ms",
            seen.worst.as_secs_f64() * 1000.0,
            "ms",
        ));
    }
    out
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
            says(
                "shaping the channel",
                263,
                "rows",
                Duration::from_millis(2604)
            ),
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

    /// A tally is what says where a frame goes; one line at a time cannot.
    #[test]
    fn the_table_adds_up_every_run_and_keeps_the_worst() {
        super::forget();
        super::add("the sidebar", Duration::from_millis(2));
        super::add("the sidebar", Duration::from_millis(8));
        super::add("the rail", Duration::from_millis(1));
        let table = super::table();
        let sidebar = table
            .lines()
            .find(|line| line.starts_with("the sidebar"))
            .expect("{table}");
        assert!(sidebar.contains('2'), "two runs: {sidebar:?}");
        assert!(sidebar.contains("10.0ms"), "added up: {sidebar:?}");
        assert!(sidebar.contains("8.00ms"), "the worst kept: {sidebar:?}");
        // And the dearest is first, which is the only order worth reading.
        let order: Vec<&str> = table
            .lines()
            .filter(|line| line.starts_with("the "))
            .collect();
        assert!(order[0].starts_with("the sidebar"), "{order:?}");
        super::forget();
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
