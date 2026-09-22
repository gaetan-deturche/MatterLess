//! A turn of the wheel, given over a few frames instead of all at once.
//!
//! A notch used to arrive as one jump: the conversation was sixty pixels
//! further on between one frame and the next, and the eye has nothing to
//! follow across the gap. Every other client eases the same distance in over
//! about a sixth of a second, which is short enough to feel immediate and long
//! enough that the text appears to travel rather than teleport.
//!
//! What is eased is the *delivery*, not the panels. Which panel a turn belongs
//! to is decided by the pointer, and that decision already exists -- so this
//! hands the same routing a slice of the distance each frame and nothing
//! downstream knows the difference.

/// How long one turn takes to arrive in full.
///
/// An eighth of a second. Most of the distance is gone in the first third of
/// that, so what this number really sets is how long the tail takes to settle
/// rather than how long the scroll feels.
pub const OVER: std::time::Duration = std::time::Duration::from_millis(130);

/// A distance still to be travelled, and how far along it is.
#[derive(Debug, Default)]
pub struct Glide {
    /// What is left to deliver, measured from the moment it was last added to.
    total: f32,
    /// How much of `total` has gone out already.
    delivered: f32,
    began: Option<std::time::Instant>,
}

/// Quickest at the start, settling towards the end.
///
/// Out only, not in and out. Easing both ends reads well on paper and feels
/// late in the hand: a cubic spends six per cent of the distance in the first
/// quarter of its time, so for about forty milliseconds after the wheel moves
/// almost nothing does. That delay is the whole of what "laggy" means here,
/// and it is the half of the curve that causes it.
///
/// Out only answers at once -- better than half the distance is gone in the
/// first quarter -- and still lands softly, which is the part worth having.
/// It also survives a fast series of notches: each one restarts the curve at
/// its quickest rather than at its slowest, so turning the wheel hard reads as
/// one continuous movement instead of a stutter per notch.
fn eased(part: f32) -> f32 {
    let part = part.clamp(0.0, 1.0);
    let left = 1.0 - part;
    1.0 - left * left * left
}

impl Glide {
    /// Adds a turn of the wheel to whatever is still travelling.
    ///
    /// Whatever has not gone out yet is carried into the new one rather than
    /// dropped, so a fast series of notches covers the sum of them. The clock
    /// restarts, which is what makes the last notch of a series the one that
    /// decides when everything stops.
    pub fn push(&mut self, by: f32, now: std::time::Instant) {
        self.total = (self.total - self.delivered) + by;
        self.delivered = 0.0;
        self.began = Some(now);
    }

    /// Whether there is still distance to hand over.
    pub fn travelling(&self) -> bool {
        self.began.is_some()
    }

    /// How much to scroll by on this frame, and nothing once it has arrived.
    ///
    /// Answers `0.0` rather than `None` for a frame too soon to have moved
    /// anything: the glide is still going, and a caller that stopped asking
    /// would leave the rest of the distance undelivered.
    pub fn taken(&mut self, now: std::time::Instant) -> f32 {
        let Some(began) = self.began else {
            return 0.0;
        };
        let part = now.duration_since(began).as_secs_f32() / OVER.as_secs_f32();
        let want = self.total * eased(part);
        let step = want - self.delivered;
        self.delivered = want;
        if part >= 1.0 {
            // Exactly the distance asked for, not the distance a float
            // happened to sum to: a pixel left behind on every notch is a
            // conversation that drifts.
            let last = self.total - self.delivered;
            self.total = 0.0;
            self.delivered = 0.0;
            self.began = None;
            return step + last;
        }
        step
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    /// Every pixel asked for arrives, and none twice.
    #[test]
    fn the_whole_distance_is_delivered_and_no_more() {
        let start = Instant::now();
        let mut glide = Glide::default();
        glide.push(120.0, start);
        let mut total = 0.0;
        // Sixteen frames across the duration, which is more than sixty a
        // second manages in a sixth of a second.
        for frame in 0..=16 {
            total += glide.taken(start + Duration::from_millis(frame * 10));
        }
        assert!(
            (total - 120.0).abs() < 0.001,
            "delivered {total} of a 120 pixel turn"
        );
        assert!(!glide.travelling(), "still going after it arrived");
    }

    /// And it only ever moves one way.
    #[test]
    fn it_never_goes_backwards() {
        let start = Instant::now();
        let mut glide = Glide::default();
        glide.push(90.0, start);
        for frame in 0..=16 {
            let step = glide.taken(start + Duration::from_millis(frame * 10));
            assert!(step >= -0.001, "a step of {step} on frame {frame}");
        }
    }

    /// A second notch part way through carries the first one's remainder.
    ///
    /// Dropping it is how a fast series of turns ends up short of where the
    /// wheel was actually turned to.
    #[test]
    fn a_turn_during_a_turn_keeps_what_was_left() {
        let start = Instant::now();
        let mut glide = Glide::default();
        glide.push(100.0, start);
        let mut total = glide.taken(start + Duration::from_millis(40));
        glide.push(100.0, start + Duration::from_millis(40));
        for frame in 0..=20 {
            total += glide.taken(start + Duration::from_millis(40 + frame * 10));
        }
        assert!(
            (total - 200.0).abs() < 0.001,
            "delivered {total} of two 100 pixel turns"
        );
    }

    /// It answers the wheel at once, and settles rather than stops.
    ///
    /// The first of those is the one that was wrong. An ease-in-out curve had
    /// moved six per cent of the distance a quarter of the way through, which
    /// is a wheel that does nothing for forty milliseconds and then catches
    /// up -- and it read as lag, because it is.
    #[test]
    fn it_starts_at_speed_and_settles() {
        let quarter = eased(0.25);
        assert!(
            quarter > 0.5,
            "only {quarter} of the distance a quarter of the way through"
        );
        let first = eased(0.1) - eased(0.0);
        let last = eased(1.0) - eased(0.9);
        assert!(
            first > last * 4.0,
            "the start ({first}) is not decisively quicker than the end ({last})"
        );
    }

    /// Nothing to give when nothing was asked for.
    #[test]
    fn a_glide_nobody_started_hands_over_nothing() {
        let mut glide = Glide::default();
        assert_eq!(glide.taken(Instant::now()), 0.0);
        assert!(!glide.travelling());
    }
}
