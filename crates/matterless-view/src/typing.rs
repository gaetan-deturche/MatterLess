//! Who is typing, and where.
//!
//! Its own small structure rather than part of the stream, for the reason the
//! app keeps it in its own scope: typing was 72-93% of all websocket traffic
//! here, and a signal that frequent must never be a reason to replan a
//! conversation.
//!
//! Keyed by conversation rather than by channel. A reply being typed in a
//! thread arrives on the thread's channel, so keying by channel alone put
//! "someone is typing" under a stream nobody was typing in.

use std::collections::HashMap;
use std::time::{Duration, Instant};

/// How long one signal stands for.
///
/// The server's clients repeat it every five seconds while somebody keeps
/// typing, so anything shorter would blink and anything much longer would
/// leave the line up after they stopped.
pub const FOR: Duration = Duration::from_secs(6);

/// Everyone currently typing, by conversation.
#[derive(Debug, Default)]
pub struct Typing {
    /// (channel, root) to each person and when they were last heard from. A
    /// map rather than a list, so somebody typing steadily is one entry that
    /// keeps moving rather than a hundred.
    seen: HashMap<(String, String), HashMap<String, Instant>>,
}

impl Typing {
    /// Records that somebody is typing, now.
    pub fn note(&mut self, channel_id: &str, root_id: &str, user_id: &str) {
        self.seen
            .entry((channel_id.to_string(), root_id.to_string()))
            .or_default()
            .insert(user_id.to_string(), Instant::now());
    }

    /// How many people are typing in one conversation.
    pub fn count(&self, channel_id: &str, root_id: &str) -> usize {
        self.seen
            .get(&(channel_id.to_string(), root_id.to_string()))
            .map(|who| who.values().filter(|at| at.elapsed() < FOR).count())
            .unwrap_or(0)
    }

    /// Drops what has gone stale.
    ///
    /// Answers whether anything was dropped, and when the next entry expires.
    /// Both halves matter to the caller: the deadline is when to wake, and
    /// "did anything change" is what keeps waking from becoming a redraw every
    /// frame for as long as somebody is typing.
    pub fn forget_stale(&mut self) -> (bool, Option<Instant>) {
        let before: usize = self.seen.values().map(|who| who.len()).sum();
        for who in self.seen.values_mut() {
            who.retain(|_, at| at.elapsed() < FOR);
        }
        self.seen.retain(|_, who| !who.is_empty());
        let after: usize = self.seen.values().map(|who| who.len()).sum();
        let next = self
            .seen
            .values()
            .flat_map(|who| who.values())
            .map(|at| *at + FOR)
            .min();
        (after < before, next)
    }

    /// What the line says, or nothing when it should not be there.
    ///
    /// Counted rather than named: resolving ids to names costs a store read on
    /// the most frequent signal there is, and "two people" answers the question
    /// the line exists for.
    pub fn line(&self, channel_id: &str, root_id: &str) -> Option<String> {
        match self.count(channel_id, root_id) {
            0 => None,
            1 => Some("someone is typing…".to_string()),
            many => Some(format!("{many} people are typing…")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A reply being typed in a thread must not put the line under the channel
    /// behind it: the signal arrives on the thread's channel either way.
    #[test]
    fn a_thread_and_its_channel_are_different_conversations() {
        let mut typing = Typing::default();
        typing.note("c1", "root", "u1");
        assert_eq!(typing.count("c1", "root"), 1);
        assert_eq!(typing.count("c1", ""), 0);
    }

    /// Somebody typing steadily is one person, not one per keystroke.
    #[test]
    fn the_same_person_is_counted_once() {
        let mut typing = Typing::default();
        for _ in 0..20 {
            typing.note("c1", "", "u1");
        }
        typing.note("c1", "", "u2");
        assert_eq!(typing.count("c1", ""), 2);
        assert_eq!(
            typing.line("c1", "").as_deref(),
            Some("2 people are typing…")
        );
    }

    /// Nothing to say when nobody is typing, so the line is not drawn at all.
    #[test]
    fn silence_says_nothing() {
        let mut typing = Typing::default();
        assert!(typing.line("c1", "").is_none());
        assert_eq!(typing.forget_stale(), (false, None));
    }

    /// A live entry names when it expires, which is when the window has to
    /// wake to take the line down.
    #[test]
    fn a_live_entry_says_when_it_goes_stale() {
        let mut typing = Typing::default();
        typing.note("c1", "", "u1");
        let (dropped, expires) = typing.forget_stale();
        assert!(!dropped, "nothing was stale yet");
        let expires = expires.expect("a deadline");
        assert!(expires > Instant::now());
        assert!(expires <= Instant::now() + FOR);
        assert_eq!(typing.count("c1", ""), 1);
    }
}
