//! Where the reader has been, for the two buttons under the thumb.
//!
//! A history of *places*, which in this window means conversations: a thread
//! is a pane beside the one you are in rather than somewhere else to be, and
//! a permalink lands in a channel, so it is recorded as an arrival in that
//! channel like any other.
//!
//! Walked rather than popped, exactly as a browser does. Going back and then
//! on again has to return where you were, so nothing is thrown away until a
//! new place is opened from part-way along -- and then what was ahead goes,
//! because it was reached through a past that no longer leads here.

/// The places visited, and where along them the reader is standing.
#[derive(Debug, Default)]
pub struct Places {
    seen: Vec<String>,
    at: usize,
}

impl Places {
    /// Notes an arrival somewhere.
    ///
    /// Arriving where you already are is not a new place. A conversation is
    /// reopened on every reread, and recording that would fill the history
    /// with one channel repeated -- and make the back button a way of going
    /// nowhere several times.
    pub fn arrived(&mut self, place: &str) {
        if self.here() == Some(place) {
            return;
        }
        self.seen.truncate(self.at + 1);
        self.seen.push(place.to_string());
        self.at = self.seen.len() - 1;
    }

    /// Where the reader is standing, if anywhere.
    pub fn here(&self) -> Option<&str> {
        self.seen.get(self.at).map(String::as_str)
    }

    /// Steps back one place and says where that is.
    ///
    /// `None` at the oldest place, which is the honest answer: there is
    /// nowhere behind it and the press should do nothing rather than reopen
    /// what is already on screen.
    pub fn back(&mut self) -> Option<&str> {
        self.at = self.at.checked_sub(1)?;
        self.here()
    }

    /// Steps on to a place already visited.
    ///
    /// `on` rather than `forward`: `Action::Forward` already means forwarding
    /// a message to somebody, and two things called forward in one window is
    /// one too many.
    pub fn on(&mut self) -> Option<&str> {
        if self.at + 1 >= self.seen.len() {
            return None;
        }
        self.at += 1;
        self.here()
    }
}

#[cfg(test)]
mod tests {
    use super::Places;

    /// Back and on walk the same list, and end up where they started.
    #[test]
    fn going_back_and_on_again_returns_where_you_were() {
        let mut places = Places::default();
        for one in ["dev", "art", "crash"] {
            places.arrived(one);
        }
        assert_eq!(places.here(), Some("crash"));

        assert_eq!(places.back(), Some("art"));
        assert_eq!(places.back(), Some("dev"));
        assert_eq!(places.back(), None, "there is nothing before the first");
        assert_eq!(places.here(), Some("dev"), "and it stayed put");

        assert_eq!(places.on(), Some("art"));
        assert_eq!(places.on(), Some("crash"));
        assert_eq!(places.on(), None, "there is nothing past the last");
        assert_eq!(places.here(), Some("crash"));
    }

    /// A new place opened from part-way along drops what was ahead.
    ///
    /// Which is what every browser does, and the only answer that leaves
    /// "on" meaning anything: the places ahead were reached through a past
    /// that no longer leads here.
    #[test]
    fn arriving_from_part_way_along_drops_what_was_ahead() {
        let mut places = Places::default();
        for one in ["dev", "art", "crash"] {
            places.arrived(one);
        }
        places.back();
        assert_eq!(places.here(), Some("art"));

        places.arrived("perf");
        assert_eq!(places.here(), Some("perf"));
        assert_eq!(places.on(), None, "crash is still ahead of perf");
        assert_eq!(places.back(), Some("art"), "and art is still behind it");
    }

    /// Arriving where you already are changes nothing.
    ///
    /// A conversation is reopened on every reread. Recorded, the history
    /// would be one channel repeated and the back button a way of going
    /// nowhere several times.
    #[test]
    fn arriving_where_you_already_are_is_not_a_place() {
        let mut places = Places::default();
        places.arrived("dev");
        for _ in 0..5 {
            places.arrived("dev");
        }
        assert_eq!(places.back(), None, "the same channel five times over");

        // And it does not swallow a real return to somewhere.
        places.arrived("art");
        places.arrived("dev");
        assert_eq!(places.back(), Some("art"));
    }

    /// An empty history answers nothing rather than pretending.
    #[test]
    fn nowhere_visited_is_nowhere_to_go() {
        let mut places = Places::default();
        assert_eq!(places.here(), None);
        assert_eq!(places.back(), None);
        assert_eq!(places.on(), None);
    }
}
