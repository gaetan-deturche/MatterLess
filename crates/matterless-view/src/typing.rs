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

/// How often this reader's own "typing" goes out.
///
/// Every keystroke would be a message per character on the busiest signal
/// there is, and the other clients hold what they hear for `FOR` -- so once
/// every three seconds says the same thing for a fraction of the traffic, and
/// leaves room for one to go missing without the line dropping.
pub const EVERY: Duration = Duration::from_secs(3);

/// When this reader last said they were typing, per conversation.
///
/// Per conversation rather than once for the window, for the same reason
/// `Typing` is keyed that way: a reply being typed in a thread and a message
/// being typed in the channel behind it are different claims, and the server
/// carries `parent_id` to tell them apart. Held as one timestamp, opening a
/// thread and starting to type within three seconds of having typed in the
/// channel said nothing at all -- and so did the first keystroke in every
/// channel switched to.
///
/// `now` is passed in so the rule can be read back in a test without a clock.
#[derive(Debug, Default)]
pub struct Sending {
    sent: HashMap<(String, String), Instant>,
}

impl Sending {
    /// Whether to send one now, and records it if so.
    pub fn due(&mut self, channel_id: &str, root_id: &str, now: Instant) -> bool {
        let conversation = (channel_id.to_string(), root_id.to_string());
        match self.sent.get(&conversation) {
            Some(last) if now.duration_since(*last) < EVERY => false,
            _ => {
                self.sent.insert(conversation, now);
                true
            }
        }
    }
}

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

    /// Everyone still typing in one conversation, by id.
    ///
    /// Sorted, because the answer is drawn: a `HashMap` walks in a different
    /// order every time it is asked, so two people typing would swap places
    /// on the line from one frame to the next.
    pub fn who(&self, channel_id: &str, root_id: &str) -> Vec<&str> {
        let mut who: Vec<&str> = self
            .seen
            .get(&(channel_id.to_string(), root_id.to_string()))
            .map(|who| {
                who.iter()
                    .filter(|(_, at)| at.elapsed() < FOR)
                    .map(|(id, _)| id.as_str())
                    .collect()
            })
            .unwrap_or_default();
        who.sort_unstable();
        who
    }

    /// What the line says, or nothing when it should not be there.
    ///
    /// Named from what the caller already knows rather than from a store read
    /// here: typing is the most frequent signal there is, and this is asked on
    /// every frame the line is up. The window looks an id up once, when it
    /// first hears that person typing, and hands the answers back.
    ///
    /// All of them or none. A set that is only half resolved would read as
    /// "Amy and someone are typing", which says less than the count does and
    /// reads like a bug -- so an unknown face among them drops the whole line
    /// back to counting.
    pub fn line(
        &self,
        channel_id: &str,
        root_id: &str,
        named: &HashMap<String, String>,
    ) -> Option<String> {
        let who = self.who(channel_id, root_id);
        let names: Vec<&str> = who
            .iter()
            .filter_map(|id| named.get(*id).map(String::as_str))
            .collect();
        match names.len() == who.len() {
            true => phrase(&names),
            false => match who.len() {
                0 => None,
                1 => Some("someone is typing…".to_string()),
                many => Some(format!("{many} people are typing…")),
            },
        }
    }
}

/// How the line reads, given who is on it.
///
/// Three by name at the outside. Past that the line is longer than the
/// message being written and nobody reads a list of four to learn that the
/// conversation is busy -- which is the same place the official client stops.
///
/// Free of `Typing` so the wording can be read back without a clock or a
/// conversation, and public because the wording is the feature.
pub fn phrase(names: &[&str]) -> Option<String> {
    match names {
        [] => None,
        [one] => Some(format!("{one} is typing…")),
        [one, two] => Some(format!("{one} and {two} are typing…")),
        [one, two, three] => Some(format!("{one}, {two} and {three} are typing…")),
        _ => Some("several people are typing…".to_string()),
    }
}

#[cfg(test)]
mod tests {
    /// Carried over from the shell this replaced, which had it right: a reply
    /// typed in a thread and a message typed in the channel behind it are
    /// different claims. Held as one timestamp for the whole window, opening a
    /// thread and typing within three seconds of the channel said nothing --
    /// and so did the first keystroke in every channel switched to.
    #[test]
    fn typing_goes_out_once_per_interval_per_conversation() {
        let mut sending = super::Sending::default();
        let began = Instant::now();

        assert!(sending.due("c1", "", began), "the first keystroke sends");
        assert!(
            !sending.due("c1", "", began + super::EVERY - Duration::from_millis(1)),
            "and nothing else does until the interval is up"
        );
        assert!(sending.due("c1", "", began + super::EVERY));

        // Each conversation keeps its own clock.
        assert!(sending.due("c1", "root9", began), "a thread of its own");
        assert!(!sending.due("c1", "root9", began));
        assert!(
            sending.due("c2", "", began),
            "nor silenced by another channel"
        );
    }

    use super::*;

    /// The wording, all the way up to where it gives up naming.
    ///
    /// Three is the last one worth reading. A list of four is longer than
    /// most of the messages being written, and by then the only thing the
    /// line is telling anybody is that the conversation is busy.
    #[test]
    fn the_line_names_up_to_three_and_then_counts() {
        assert_eq!(phrase(&[]), None);
        assert_eq!(phrase(&["amy"]).unwrap(), "amy is typing\u{2026}");
        assert_eq!(
            phrase(&["amy", "ben"]).unwrap(),
            "amy and ben are typing\u{2026}"
        );
        assert_eq!(
            phrase(&["amy", "ben", "cara"]).unwrap(),
            "amy, ben and cara are typing\u{2026}"
        );
        assert_eq!(
            phrase(&["amy", "ben", "cara", "dev"]).unwrap(),
            "several people are typing\u{2026}"
        );
    }

    /// Two people keep their order between one frame and the next.
    ///
    /// They are held in a `HashMap`, which walks in a different order every
    /// time it is asked -- so unsorted, two people typing would swap places
    /// on the line while nobody did anything.
    #[test]
    fn who_is_typing_comes_back_in_the_same_order() {
        let mut typing = Typing::default();
        for id in ["u3", "u1", "u2"] {
            typing.note("c1", "", id);
        }
        assert_eq!(typing.who("c1", ""), vec!["u1", "u2", "u3"]);
    }

    /// A face nobody can put a name to drops the whole line back to counting.
    ///
    /// Half a set named reads as "amy and someone are typing", which says
    /// less than the count does and reads like a bug.
    #[test]
    fn a_line_is_named_only_when_everybody_on_it_is() {
        let mut typing = Typing::default();
        typing.note("c1", "", "u1");
        typing.note("c1", "", "u2");

        let mut named = HashMap::new();
        named.insert("u1".to_string(), "amy".to_string());
        assert_eq!(
            typing.line("c1", "", &named).unwrap(),
            "2 people are typing\u{2026}",
            "one name short and it should still be counting"
        );

        named.insert("u2".to_string(), "ben".to_string());
        assert_eq!(
            typing.line("c1", "", &named).unwrap(),
            "amy and ben are typing\u{2026}"
        );
    }

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
            typing.line("c1", "", &HashMap::new()).as_deref(),
            Some("2 people are typing…")
        );
    }

    /// Nothing to say when nobody is typing, so the line is not drawn at all.
    #[test]
    fn silence_says_nothing() {
        let mut typing = Typing::default();
        assert!(typing.line("c1", "", &HashMap::new()).is_none());
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
