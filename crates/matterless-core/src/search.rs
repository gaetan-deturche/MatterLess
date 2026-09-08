//! Parsing what a reader typed into a search into its parts.
//!
//! Mattermost's search modifiers are not server-only knowledge: `from:` names a
//! user, `in:` names a channel, and the date modifiers are a comparison against
//! `create_at`. A local store that holds users, channels and posts can answer
//! all of them -- so it does, and the local half of a search filters rather
//! than pretending the filter was not there.
//!
//! Only the free-text terms reach FTS5. The modifiers become SQL, because that
//! is what they are.

use crate::model::Timestamp;

/// A search, split into the part FTS5 can answer and the parts SQL can.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SearchQuery {
    /// Free text, with the modifiers removed.
    pub terms: Vec<String>,
    /// Usernames from `from:`. Several are an "or", as on the server.
    pub from: Vec<String>,
    /// Channel names from `in:`.
    pub in_channels: Vec<String>,
    /// `create_at >= this`, from `after:` or `on:`.
    pub since: Option<Timestamp>,
    /// `create_at < this`, from `before:` or `on:`.
    pub until: Option<Timestamp>,
}

impl SearchQuery {
    /// Splits a typed query. `utc_offset_minutes` is the reader's offset, so a
    /// date means their day rather than a day in UTC -- `on:2026-09-04` should
    /// cover the messages they remember sending that afternoon.
    pub fn parse(typed: &str, utc_offset_minutes: i32) -> Self {
        let mut query = SearchQuery::default();
        for word in typed.split_whitespace() {
            match word.split_once(':') {
                Some(("from", who)) if !who.is_empty() => {
                    // `@amy` and `amy` are the same person.
                    query.from.push(who.trim_start_matches('@').to_lowercase());
                }
                Some(("in", where_)) | Some(("channel", where_)) if !where_.is_empty() => {
                    query
                        .in_channels
                        .push(where_.trim_start_matches('~').to_lowercase());
                }
                // A date that does not parse stays *text*. Swallowing it
                // would filter nothing and lose the word as well, so
                // `before:soon` would quietly search for nothing at all.
                Some(("after", date)) => match parse_date(date) {
                    // Exclusive, as on the server: the day after the one named.
                    Some(day) => query.since = Some(day_start_ms(day + 1, utc_offset_minutes)),
                    None => query.terms.push(word.to_string()),
                },
                Some(("before", date)) => match parse_date(date) {
                    Some(day) => query.until = Some(day_start_ms(day, utc_offset_minutes)),
                    None => query.terms.push(word.to_string()),
                },
                Some(("on", date)) => match parse_date(date) {
                    Some(day) => {
                        query.since = Some(day_start_ms(day, utc_offset_minutes));
                        query.until = Some(day_start_ms(day + 1, utc_offset_minutes));
                    }
                    None => query.terms.push(word.to_string()),
                },
                // Not a modifier: `10:30` and `http://x` are text, and so is
                // anything with an unknown prefix.
                _ => query.terms.push(word.to_string()),
            }
        }
        query
    }

    /// True when nothing was typed that could match anything.
    pub fn is_empty(&self) -> bool {
        self.terms.is_empty()
            && self.from.is_empty()
            && self.in_channels.is_empty()
            && self.since.is_none()
            && self.until.is_none()
    }

    /// The FTS5 expression for the free-text half, or `None` when there is no
    /// free text -- `from:amy` alone is a perfectly good search, answered by
    /// filtering rather than by matching.
    ///
    /// Every term is quoted because FTS5 reads `-`, `*`, `:`, `^` and `NEAR` as
    /// syntax: a reader searching for `-Wall` would otherwise get a parse error
    /// rather than results.
    pub fn fts_expression(&self) -> Option<String> {
        let quoted: Vec<String> = self
            .terms
            .iter()
            .map(|term| term.replace('"', " ").trim().to_string())
            .filter(|term| !term.is_empty())
            .map(|term| format!("\"{term}\""))
            .collect();
        if quoted.is_empty() {
            None
        } else {
            Some(quoted.join(" "))
        }
    }
}

/// `YYYY-MM-DD` to days since the epoch, or `None` if it is not a date.
fn parse_date(text: &str) -> Option<i64> {
    let mut parts = text.split('-');
    let year: i64 = parts.next()?.parse().ok()?;
    let month: i64 = parts.next()?.parse().ok()?;
    let day: i64 = parts.next()?.parse().ok()?;
    if parts.next().is_some() || !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    Some(days_from_civil(year, month, day))
}

/// Days from the epoch for a civil date.
///
/// Hinnant's algorithm, and written out rather than pulled in: the workspace
/// does no other date arithmetic, and `epoch_day` in the render crate is this
/// same conversion in the other direction.
fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = if year >= 0 { year } else { year - 399 } / 400;
    let year_of_era = year - era * 400;
    let day_of_year = (153 * (if month > 2 { month - 3 } else { month + 9 }) + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

/// Midnight of a day, in the reader's own timezone, as a server timestamp.
fn day_start_ms(epoch_day: i64, utc_offset_minutes: i32) -> Timestamp {
    epoch_day * 86_400_000 - utc_offset_minutes as i64 * 60_000
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn modifiers_are_separated_from_the_text() {
        let query = SearchQuery::parse("from:@amy in:~town-square budget report", 0);
        assert_eq!(query.terms, vec!["budget", "report"]);
        assert_eq!(query.from, vec!["amy"], "the @ is not part of the name");
        assert_eq!(query.in_channels, vec!["town-square"]);
        assert_eq!(
            query.fts_expression().as_deref(),
            Some("\"budget\" \"report\"")
        );
    }

    #[test]
    fn a_modifier_alone_is_still_a_search() {
        // Answered by filtering; there is nothing for FTS5 to match.
        let query = SearchQuery::parse("from:amy", 0);
        assert!(!query.is_empty());
        assert_eq!(query.fts_expression(), None);
    }

    #[test]
    fn a_colon_that_is_not_a_modifier_stays_text() {
        // The case that made the first version of this refuse to search at all.
        let query = SearchQuery::parse("meeting at 10:30 see http://wiki/x", 0);
        assert!(query.from.is_empty() && query.in_channels.is_empty());
        assert_eq!(query.terms.len(), 5, "got {:?}", query.terms);
    }

    #[test]
    fn terms_are_quoted_against_fts_syntax() {
        let query = SearchQuery::parse("-Wall NEAR *", 0);
        assert_eq!(
            query.fts_expression().as_deref(),
            Some("\"-Wall\" \"NEAR\" \"*\"")
        );
    }

    #[test]
    fn dates_become_a_window_in_the_readers_own_day() {
        // 2026-09-04 is 20700 days after the epoch (checked against a real
        // calendar, not by hand); the rest is that the two bounds are one day
        // apart and that the offset moves them.
        let utc = SearchQuery::parse("on:2026-09-04", 0);
        let since = utc.since.expect("a start");
        let until = utc.until.expect("an end");
        assert_eq!(until - since, 86_400_000);
        assert_eq!(since, 20_700 * 86_400_000);

        // Two hours east: their midnight is two hours earlier in server time.
        let local = SearchQuery::parse("on:2026-09-04", 120);
        assert_eq!(local.since.expect("a start"), since - 2 * 3_600_000);

        // `before` and `after` are open-ended and exclusive of the named day.
        let before = SearchQuery::parse("before:2026-09-04", 0);
        assert_eq!(before.until, Some(since));
        assert_eq!(before.since, None);
        let after = SearchQuery::parse("after:2026-09-04", 0);
        assert_eq!(after.since, Some(until));
        assert_eq!(after.until, None);

        // Nonsense is text, not a filter.
        let nonsense = SearchQuery::parse("before:soon on:2026-13-01", 0);
        assert!(nonsense.since.is_none() && nonsense.until.is_none());
        assert_eq!(nonsense.terms.len(), 2);
    }

    #[test]
    fn the_civil_date_conversion_matches_known_days() {
        assert_eq!(days_from_civil(1970, 1, 1), 0);
        assert_eq!(days_from_civil(1969, 12, 31), -1);
        assert_eq!(days_from_civil(2000, 3, 1), 11_017);
        assert_eq!(days_from_civil(2026, 9, 4), 20_700);
    }
}
