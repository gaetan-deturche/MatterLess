//! Pictures with more than one frame in them.
//!
//! A custom emoji is a picture the server keeps, and some of them move: the
//! ones a team makes out of a looping GIF, which every other client plays and
//! this one drew as whatever the first frame happened to be -- a hand halfway
//! through a wave, a pair of hands not yet clapping.
//!
//! What moves is the *picture*, not the layout. A frame is the same size as
//! the one before it and goes into the same slot in the atlas, so nothing
//! above this knows the difference: the painter still emits one image piece
//! under one key, and the row it sits in is the height it always was.
//!
//! The clock is the reader's, not the server's. Frames are played from the
//! moment the picture arrived, looping, and are only advanced while the window
//! is being looked at -- a conversation left open behind an editor should cost
//! nothing at all.

use std::time::{Duration, Instant};

/// A delay shorter than this is not a delay at all.
///
/// A GIF's is stored in hundredths of a second and a great many of them say
/// zero, meaning "as fast as the machine can", which in 1995 meant something
/// reasonable and today means a strobe. Every browser reads a delay under two
/// hundredths as an unset one and plays it at a tenth of a second instead;
/// this follows them, so a loop runs here at the speed it runs everywhere
/// else.
const FLOOR: Duration = Duration::from_millis(20);
const WHEN_UNSAID: Duration = Duration::from_millis(100);

/// How much room all the moving pictures may take between them.
///
/// An emoji is small -- sixty-four pixels square is sixteen kilobytes a frame
/// -- but a long loop is a hundred frames, and a channel full of them would
/// otherwise be held in memory for as long as the window is open.
const ROOM: usize = 32 * 1024 * 1024;

/// One picture that moves, and where in its loop it is.
#[derive(Debug)]
pub struct Reel {
    /// Every frame, straight RGBA, at the size the atlas holds.
    frames: Vec<Vec<u8>>,
    /// How long each of them lasts.
    delays: Vec<Duration>,
    pub width: u32,
    pub height: u32,
    /// When the loop started, which is when the picture arrived.
    began: Instant,
    /// Which frame is in the atlas now, so a frame is uploaded once rather
    /// than on every pass.
    shown: usize,
    /// When it was last drawn, for deciding what to drop.
    seen: Instant,
}

impl Reel {
    pub fn new(width: u32, height: u32, frames: Vec<(Vec<u8>, Duration)>, now: Instant) -> Self {
        let (frames, delays) = frames
            .into_iter()
            .map(|(pixels, delay)| {
                let delay = match delay < FLOOR {
                    true => WHEN_UNSAID,
                    false => delay,
                };
                (pixels, delay)
            })
            .unzip();
        Self {
            frames,
            delays,
            width,
            height,
            began: now,
            // Nothing is in the atlas yet; the first pass puts the frame the
            // clock asks for, which for a picture that has just arrived is
            // the first one.
            shown: usize::MAX,
            seen: now,
        }
    }

    /// How long the whole loop takes.
    fn round(&self) -> Duration {
        self.delays.iter().sum()
    }

    /// Which frame belongs to `now`.
    fn at(&self, now: Instant) -> usize {
        let round = self.round();
        if round.is_zero() || self.frames.is_empty() {
            return 0;
        }
        // Modulo the loop, so a picture that has been on screen for an hour
        // is on the same frame as one that arrived a moment ago.
        let mut into = now.saturating_duration_since(self.began).as_nanos() % round.as_nanos();
        for (at, delay) in self.delays.iter().enumerate() {
            let delay = delay.as_nanos();
            if into < delay {
                return at;
            }
            into -= delay;
        }
        self.frames.len() - 1
    }

    /// When the frame after the one showing is due.
    fn next(&self, now: Instant) -> Option<Instant> {
        let round = self.round();
        if round.is_zero() || self.frames.len() < 2 {
            return None;
        }
        let round = round.as_nanos();
        let into = now.saturating_duration_since(self.began).as_nanos() % round;
        let mut edge = 0u128;
        for delay in &self.delays {
            edge += delay.as_nanos();
            if edge > into {
                let left = edge - into;
                return Some(now + Duration::from_nanos(left.min(u128::from(u64::MAX)) as u64));
            }
        }
        None
    }

    fn bytes(&self) -> usize {
        self.frames.iter().map(Vec::len).sum()
    }
}

/// Every picture that moves, by the key it is drawn under.
#[derive(Debug, Default)]
pub struct Moving {
    reels: std::collections::HashMap<String, Reel>,
}

impl Moving {
    /// Takes a picture that moves, replacing whatever was under that key.
    ///
    /// Drops the least recently drawn ones when there is no room left. A
    /// picture dropped is a picture that stops moving, not one that
    /// disappears: the frame it was last left on is still in the atlas.
    pub fn keep(&mut self, key: &str, reel: Reel) {
        self.reels.insert(key.to_string(), reel);
        while self.reels.values().map(Reel::bytes).sum::<usize>() > ROOM && self.reels.len() > 1 {
            let Some(oldest) = self
                .reels
                .iter()
                .min_by_key(|(_, reel)| reel.seen)
                .map(|(key, _)| key.clone())
            else {
                break;
            };
            self.reels.remove(&oldest);
        }
    }

    pub fn holds(&self, key: &str) -> bool {
        self.reels.contains_key(key)
    }

    pub fn len(&self) -> usize {
        self.reels.len()
    }

    pub fn is_empty(&self) -> bool {
        self.reels.is_empty()
    }

    /// The frames that have come due, for whichever keys are still being
    /// drawn.
    ///
    /// `drawn` answers whether a key is on screen, which is the atlas's to
    /// say: a picture nobody is looking at must not cost an upload a frame.
    /// Marks what it hands over as seen, so what gets dropped under pressure
    /// is what nobody is looking at.
    pub fn due(
        &mut self,
        now: Instant,
        drawn: impl Fn(&str) -> bool,
    ) -> Vec<(String, u32, u32, Vec<u8>)> {
        let mut ready = Vec::new();
        for (key, reel) in &mut self.reels {
            if !drawn(key) {
                continue;
            }
            reel.seen = now;
            let want = reel.at(now);
            if want == reel.shown {
                continue;
            }
            let Some(frame) = reel.frames.get(want) else {
                continue;
            };
            reel.shown = want;
            ready.push((key.clone(), reel.width, reel.height, frame.clone()));
        }
        ready
    }

    /// Whether anything on screen is showing a frame that is no longer the
    /// right one.
    ///
    /// The question `wakes` cannot answer. `wakes` says when the frame showing
    /// runs out, so at the moment it fires the answer is the end of the *next*
    /// frame -- another tenth of a second away. A window that asked "is one due
    /// yet?" by comparing that against the clock was told no every single time:
    /// it woke on the timer, found nothing to do, and went back to sleep
    /// without drawing. The loop then only advanced when something else caused
    /// a frame, which is a pointer moving across the window.
    pub fn overdue(&self, now: Instant, drawn: impl Fn(&str) -> bool) -> bool {
        self.reels
            .iter()
            .any(|(key, reel)| drawn(key) && reel.at(now) != reel.shown)
    }

    /// When the window next has to wake to keep something moving.
    ///
    /// `None` when nothing is moving, which is the ordinary case: the window
    /// then waits for something to happen rather than for a clock.
    pub fn wakes(&self, now: Instant, drawn: impl Fn(&str) -> bool) -> Option<Instant> {
        self.reels
            .iter()
            .filter(|(key, _)| drawn(key))
            .filter_map(|(_, reel)| reel.next(now))
            .min()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reel(now: Instant, delays: [u64; 3]) -> Reel {
        Reel::new(
            4,
            4,
            delays
                .into_iter()
                .map(|ms| (vec![0u8; 64], Duration::from_millis(ms)))
                .collect(),
            now,
        )
    }

    /// The frame is the one the clock asks for, and the loop comes round.
    #[test]
    fn the_clock_decides_which_frame() {
        let now = Instant::now();
        let reel = reel(now, [100, 100, 100]);
        assert_eq!(reel.at(now), 0);
        assert_eq!(reel.at(now + Duration::from_millis(50)), 0);
        assert_eq!(reel.at(now + Duration::from_millis(150)), 1);
        assert_eq!(reel.at(now + Duration::from_millis(250)), 2);
        // Round again, rather than stopping on the last frame.
        assert_eq!(reel.at(now + Duration::from_millis(350)), 0);
        // And an hour in is the same answer as a moment in.
        assert_eq!(reel.at(now + Duration::from_secs(3600)), 0);
    }

    /// A file that asks for no delay at all gets one anyway.
    ///
    /// A great many GIFs store zero, which meant "as fast as you can" on the
    /// hardware they were made for and means a strobe on this one.
    #[test]
    fn a_frame_that_asks_for_nothing_still_lasts() {
        let now = Instant::now();
        let unsaid = reel(now, [0, 0, 0]);
        assert_eq!(unsaid.round(), WHEN_UNSAID * 3);
        assert_eq!(unsaid.at(now + Duration::from_millis(50)), 0);
        assert_eq!(unsaid.at(now + Duration::from_millis(150)), 1);
        // And a delay that is merely very short is read the same way, which
        // is what a browser does with anything under two hundredths.
        assert_eq!(reel(now, [10, 10, 10]).round(), WHEN_UNSAID * 3);
        // A real delay is left alone.
        assert_eq!(reel(now, [40, 40, 40]).round(), Duration::from_millis(120));
    }

    /// The next wake is when the frame showing runs out, not a frame later.
    #[test]
    fn it_wakes_when_the_frame_runs_out() {
        let now = Instant::now();
        let reel = reel(now, [100, 100, 100]);
        let next = reel.next(now).expect("a frame is due");
        let waits = next.saturating_duration_since(now);
        assert!(
            waits > Duration::from_millis(90) && waits <= Duration::from_millis(100),
            "waited {waits:?}"
        );
        // Part way through a frame, only what is left of it.
        let part = now + Duration::from_millis(60);
        let waits = reel.next(part).expect("a frame is due") - part;
        assert!(waits <= Duration::from_millis(41), "waited {waits:?}");
    }

    /// What asks for a frame is a frame coming due, not a clock running out.
    ///
    /// The window wakes on a timer and then has to decide whether to draw.
    /// Asked "has the wake time passed", it is told no every time -- the wake
    /// is the end of the frame showing, so the moment it fires the answer is
    /// the end of the next one. The loop then only moved when something else
    /// caused a frame, which in a window drawn on demand means a pointer
    /// crossing it.
    #[test]
    fn a_frame_coming_due_is_what_asks_for_a_draw() {
        let now = Instant::now();
        let mut moving = Moving::default();
        moving.keep("emoji/wave", reel(now, [100, 100, 100]));
        assert!(
            moving.overdue(now, |_| true),
            "nothing is showing at all yet"
        );
        moving.due(now, |_| true);
        assert!(
            !moving.overdue(now, |_| true),
            "the first frame is up and current"
        );
        assert!(!moving.overdue(now + Duration::from_millis(50), |_| true));
        assert!(
            moving.overdue(now + Duration::from_millis(150), |_| true),
            "the second frame is due and the first is still showing"
        );
        // And nothing off screen ever is, however long it has been.
        assert!(!moving.overdue(now + Duration::from_secs(60), |_| false));
    }

    /// A picture of one frame is a picture, not an animation: nothing to wake
    /// for, ever.
    #[test]
    fn a_single_frame_never_asks_for_a_wake() {
        let now = Instant::now();
        let still = Reel::new(4, 4, vec![(vec![0u8; 64], Duration::from_millis(100))], now);
        assert!(still.next(now).is_none());
        let mut moving = Moving::default();
        moving.keep("emoji/one", still);
        assert!(moving.wakes(now, |_| true).is_none());
    }

    /// Only what is on screen moves, and only what moves is uploaded.
    #[test]
    fn nothing_off_screen_costs_anything() {
        let now = Instant::now();
        let mut moving = Moving::default();
        moving.keep("emoji/seen", reel(now, [100, 100, 100]));
        moving.keep("emoji/hidden", reel(now, [100, 100, 100]));

        let due = moving.due(now, |key| key == "emoji/seen");
        assert_eq!(due.len(), 1, "only the one being drawn");
        assert_eq!(due[0].0, "emoji/seen");
        // And the same frame is not sent twice.
        assert!(moving.due(now, |key| key == "emoji/seen").is_empty());
        // The one nobody is drawing asks for no wake either.
        let wake = moving.wakes(now, |key| key == "emoji/hidden");
        assert!(wake.is_some(), "it would wake if it were drawn");
        assert!(moving.wakes(now, |_| false).is_none());
    }

    /// The next frame goes up when its time comes.
    #[test]
    fn the_next_frame_goes_up_when_it_is_due() {
        let now = Instant::now();
        let mut moving = Moving::default();
        moving.keep("emoji/wave", reel(now, [100, 100, 100]));
        assert_eq!(moving.due(now, |_| true).len(), 1);
        assert!(
            moving
                .due(now + Duration::from_millis(50), |_| true)
                .is_empty()
        );
        assert_eq!(
            moving.due(now + Duration::from_millis(150), |_| true).len(),
            1
        );
    }
}
