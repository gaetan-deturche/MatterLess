//! Subsequence matching and ranking, shared by every completion list.
//!
//! Fuzzy rather than prefix, because a prefix match asks the reader to know how
//! a name *starts*: `al` should find `ada.lovelace` and `mtrv` should find
//! `Design | Material Review`.
//!
//! Two halves, deliberately split by where they are cheap:
//!
//! * [`like_pattern`] turns a query into a SQL `LIKE` pattern with a wildcard
//!   between every character, so SQLite does the *filtering* -- a subsequence
//!   test is exactly what `%g%d%t%` is, and it uses the same scan the old prefix
//!   match did.
//! * [`score`] ranks what came back. Ranking is what makes a fuzzy list usable
//!   and it needs to look at where the matches landed, which SQL cannot say.

/// Characters after which a match counts as starting a word.
///
/// `ada.lovelace`, `Design | Material Review`, `flag-scotland`: the
/// separators people actually use in usernames, channel names and emoji names.
fn starts_a_word(before: char) -> bool {
    matches!(
        before,
        '.' | '_' | '-' | ' ' | '|' | '/' | ':' | ',' | '(' | '['
    )
}

/// How well `candidate` matches `query`, or `None` if it does not.
///
/// A match means every character of the query appears in the candidate, in
/// order. The number is only meaningful against other scores for the same
/// query.
pub fn score(candidate: &str, query: &str) -> Option<i32> {
    if query.is_empty() {
        return Some(0);
    }
    let haystack: Vec<char> = candidate.to_lowercase().chars().collect();
    let needle: Vec<char> = query.to_lowercase().chars().collect();

    let mut total: i32 = 0;
    let mut from = 0usize;
    let mut previous: Option<usize> = None;

    for wanted in needle {
        let found = haystack[from..].iter().position(|have| *have == wanted)? + from;

        // Where the match landed matters more than that it landed: the start of
        // a name, or the start of a word inside it, is what the reader meant.
        if found == 0 {
            total += 40;
        } else if starts_a_word(haystack[found - 1]) {
            total += 25;
        }
        if let Some(previous) = previous {
            if found == previous + 1 {
                // A contiguous run is a stronger signal than two scattered
                // letters, which is the whole difference between `mtrv` finding
                // "Material Review" and finding "Moto".
                total += 20;
            } else {
                total -= ((found - previous - 1) as i32).min(12);
            }
        }
        previous = Some(found);
        from = found + 1;
    }

    // A tiebreak, not a rule: among equally good matches the shorter name is
    // the one more likely to be meant.
    total -= haystack.len() as i32 / 8;
    Some(total)
}

/// The best score across several fields, for a candidate that can be matched by
/// more than one name (a username *or* a real name).
pub fn best_score<'a>(fields: impl IntoIterator<Item = &'a str>, query: &str) -> Option<i32> {
    fields
        .into_iter()
        .filter_map(|field| score(field, query))
        .max()
}

/// A SQL `LIKE` pattern that matches the query as a subsequence.
///
/// `%` and `_` in the query are escaped and the caller must use
/// `ESCAPE '\'`, or a reader typing `_` would match everything.
pub fn like_pattern(query: &str) -> String {
    let mut pattern = String::with_capacity(query.len() * 2 + 1);
    pattern.push('%');
    for character in query.chars().flat_map(char::to_lowercase) {
        if matches!(character, '%' | '_' | '\\') {
            pattern.push('\\');
        }
        pattern.push(character);
        pattern.push('%');
    }
    pattern
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rank<'a>(candidates: &[&'a str], query: &str) -> Vec<&'a str> {
        let mut scored: Vec<(&str, i32)> = candidates
            .iter()
            .filter_map(|candidate| score(candidate, query).map(|points| (*candidate, points)))
            .collect();
        scored.sort_by(|left, right| right.1.cmp(&left.1).then(left.0.len().cmp(&right.0.len())));
        scored.into_iter().map(|(candidate, _)| candidate).collect()
    }

    #[test]
    fn initials_find_a_dotted_username() {
        // The case that motivated this: a prefix match cannot answer it at all.
        assert!(score("ada.lovelace", "al").is_some());
        assert!(score("ada.lovelace", "adlv").is_some());
        assert!(score("ada.lovelace", "xyz").is_none());
    }

    #[test]
    fn word_starts_outrank_scattered_letters() {
        let ranked = rank(
            &["Design | Material Review", "Moto", "Games Sharing"],
            "mtrv",
        );
        assert_eq!(
            ranked.first().copied(),
            Some("Design | Material Review"),
            "ranked: {ranked:?}"
        );
    }

    #[test]
    fn a_prefix_still_wins_when_one_exists() {
        // Fuzzy must not *lose* the obvious answer: typing a real prefix should
        // put the thing it prefixes first.
        let ranked = rank(&["off-topic", "the-office", "programming"], "off");
        assert_eq!(
            ranked.first().copied(),
            Some("off-topic"),
            "ranked: {ranked:?}"
        );
    }

    #[test]
    fn contiguity_beats_a_scattered_match() {
        let together = score("material", "mat").expect("matches");
        let scattered = score("my-abstract-thing", "mat").expect("matches");
        assert!(
            together > scattered,
            "together {together} should beat scattered {scattered}"
        );
    }

    #[test]
    fn an_empty_query_matches_everything() {
        // `@` on its own offers people rather than nothing.
        assert_eq!(score("anyone", ""), Some(0));
    }

    #[test]
    fn the_like_pattern_is_a_subsequence_test() {
        assert_eq!(like_pattern("gdt"), "%g%d%t%");
        assert_eq!(like_pattern(""), "%");
        // A wildcard typed by the reader is a literal, not a wildcard.
        assert_eq!(like_pattern("a_b"), "%a%\\_%b%");
        assert_eq!(like_pattern("50%"), "%5%0%\\%%");
    }
}
