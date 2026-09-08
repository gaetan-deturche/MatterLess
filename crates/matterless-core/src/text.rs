//! Small text predicates shared by rendering and notification.
//!
//! Both need the same answer to "what characters can a username contain" -- the
//! renderer to find a mention's span, the notifier to test membership -- and two
//! copies would drift.

/// Characters Mattermost allows in a username, which is what bounds a `@mention`.
pub fn is_username_char(character: char) -> bool {
    character.is_ascii_alphanumeric() || matches!(character, '.' | '-' | '_')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_username_stops_at_punctuation_that_is_not_part_of_a_name() {
        assert!(is_username_char('a'));
        assert!(is_username_char('9'));
        assert!(is_username_char('.'));
        assert!(is_username_char('-'));
        assert!(is_username_char('_'));
        assert!(!is_username_char(' '));
        assert!(!is_username_char(','));
        assert!(!is_username_char('@'));
    }
}
