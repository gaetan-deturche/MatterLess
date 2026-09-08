//! Standard emoji names to characters.
//!
//! Two kinds of `:name:` arrive in a message and they need different answers. A
//! custom emoji is an image behind the session token, resolved by name against
//! the server and drawn through the `mmedia` scheme. A standard one is a
//! character, and the server has no endpoint for those at all -- that set is
//! compiled into its webapp.
//!
//! It *is* in the server's source, though: `model.SystemEmojis` in Mattermost's
//! public model package is the very map the server uses to validate reaction
//! names, and `emoji_table.rs` is generated from it by
//! `tools/generate_emoji_table.py`. **4463 names** -- which is what a measured
//! table was never going to reach: a first hand-written pass covered the top 60
//! by use, a second added 100 more, and both were still missing names the
//! moment somebody typed a new one.
//!
//! Because it agrees with the server by construction, the spelling rules that
//! earlier pass had to guess at are simply entries now: 2130 skin-tone
//! variants, 903 hyphenated aliases (`star-struck` beside `star_struck`) and
//! 261 flags, including subdivision flags like `flag-scotland` that no
//! two-letter rule produces.

use crate::emoji_table::{GENERATED_ON, SYSTEM_EMOJI};

/// How many standard names are known, and when the table was generated.
///
/// Reported at startup next to the server's own version: the standard set
/// changes only when the server is upgraded, so a table older than the server
/// is the one condition under which a genuine emoji could still render as
/// `:name:`.
pub fn provenance() -> (usize, &'static str) {
    (SYSTEM_EMOJI.len(), GENERATED_ON)
}

/// The standard emoji grouped the way a picker shows them, in the order every
/// other client uses.
///
/// A category holds only names `character_for` can answer, so a grid built from
/// this can never offer a reaction that would draw as `:name:`. It is also the
/// *base* set: the skin-tone variants and hyphenated aliases that make the full
/// table 4463 names long belong in `:name:` completion, not in a grid of tiles.
pub fn categories() -> &'static [(&'static str, &'static [&'static str])] {
    &crate::emoji_categories::EMOJI_CATEGORIES
}

/// How many categorised names there are, and when that table was generated.
pub fn categories_provenance() -> (usize, &'static str) {
    (
        crate::emoji_categories::EMOJI_CATEGORIES
            .iter()
            .map(|(_, names)| names.len())
            .sum(),
        crate::emoji_categories::CATEGORIES_GENERATED_ON,
    )
}

/// The character for a standard emoji name.
///
/// `None` means "not a standard emoji": for a name in a message that is either
/// custom -- resolved separately, against the server -- or not an emoji at all.
/// Both stay as `:name:`, which is readable.
pub fn character_for(name: &str) -> Option<String> {
    decode(codepoints_for(name)?)
}

/// The codepoint sequence for a name, in the server's own spelling.
///
/// Public because it is also the path component of
/// `/static/emoji/<codepoints>.png`, a real route on the server, should a font
/// ever turn out to lack a glyph.
pub fn codepoints_for(name: &str) -> Option<&'static str> {
    SYSTEM_EMOJI
        .binary_search_by(|(candidate, _)| (*candidate).cmp(name))
        .ok()
        .map(|index| SYSTEM_EMOJI[index].1)
}

/// `1f937-200d-2642-fe0f` to the characters it names.
fn decode(codepoints: &str) -> Option<String> {
    let mut text = String::with_capacity(codepoints.len() / 2);
    for part in codepoints.split('-') {
        text.push(char::from_u32(u32::from_str_radix(part, 16).ok()?)?);
    }
    Some(text)
}

/// Every `:name:` in a message that is not a standard emoji.
///
/// Scanned from the raw text rather than walked out of the parsed nodes,
/// because the nodes are cached: a name resolved after the parse would never be
/// seen again. Standard names are dropped here -- they already have a
/// character, and asking the server about them is the request this exists to
/// avoid.
/// Standard emoji names matching `query` as a subsequence, best first.
///
/// A linear scan of the generated table: 4463 entries scored against a
/// keystroke is nothing, and the alternative -- a second index -- would be one
/// more thing to keep in step with a table regenerated from the server's own
/// map.
pub fn names_matching(query: &str, limit: usize) -> Vec<&'static str> {
    let mut found: Vec<(i32, &'static str)> = SYSTEM_EMOJI
        .iter()
        .filter_map(|(name, _)| {
            matterless_core::fuzzy::score(name, query).map(|points| (points, *name))
        })
        .collect();
    // Shortest name breaks a tie: typing `smi` should offer `smile` before
    // `smiling_face_with_three_hearts`.
    found.sort_by(|left, right| right.0.cmp(&left.0).then(left.1.len().cmp(&right.1.len())));
    found
        .into_iter()
        .take(limit)
        .map(|(_, name)| name)
        .collect()
}

#[cfg(test)]
mod completion_tests {
    #[test]
    fn a_short_exact_name_leads_its_own_family() {
        let found = super::names_matching("smi", 5);
        assert_eq!(found.first().copied(), Some("smile"), "got {found:?}");
    }

    #[test]
    fn scattered_letters_still_find_a_long_name() {
        // A prefix search could never answer this one.
        let found = super::names_matching("thnkng", 5);
        assert!(found.contains(&"thinking_face"), "got {found:?}");
    }
}

pub fn custom_candidates(message: &str) -> Vec<String> {
    let mut found = Vec::new();
    let mut index = 0;
    while let Some(offset) = message[index..].find(':') {
        let start = index + offset;
        let body = start + 1;
        let Some(close) = message[body..].find(':') else {
            break;
        };
        let name = &message[body..body + close];
        let plausible = !name.is_empty()
            && name.len() <= 64
            && name.chars().all(|character| {
                character.is_ascii_alphanumeric()
                    || character == '_'
                    || character == '+'
                    || character == '-'
            });
        if plausible && codepoints_for(name).is_none() {
            found.push(name.to_string());
        }
        // Past the closing colon, so `:a:b:` cannot yield `b` twice.
        index = if plausible {
            body + close + 1
        } else {
            start + 1
        };
        if index >= message.len() {
            break;
        }
    }
    found.sort();
    found.dedup();
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The table has to stay sorted: the lookup is a binary search, and an
    /// unsorted table fails *silently* for the names either side of the break.
    #[test]
    fn the_generated_table_is_sorted_and_large() {
        assert!(
            SYSTEM_EMOJI.len() > 4000,
            "only {} names -- the generator may have parsed the wrong thing",
            SYSTEM_EMOJI.len()
        );
        assert!(
            SYSTEM_EMOJI.windows(2).all(|pair| pair[0].0 < pair[1].0),
            "the table is not sorted by name"
        );
    }

    /// Everything the two hand-written passes missed, which is why this is
    /// generated from the server's own map instead.
    #[test]
    fn the_names_that_were_missing_resolve() {
        for name in [
            // From the screenshots and the measured gaps.
            "magic_wand",
            "pray",
            "open_mouth",
            "bagel",
            "zzz",
            "pinched_fingers",
            "desert_island",
            "sports_medal",
            "negative_squared_cross_mark",
            // Spellings a rule had to guess at before.
            "star-struck",
            "man-shrugging",
            "female-technologist",
            "muscle_medium_light_skin_tone",
            "+1_dark_skin_tone",
            // Flags, including subdivisions no letter rule produces.
            "flag-pt",
            "flag-be",
            "flag-scotland",
        ] {
            assert!(character_for(name).is_some(), "{name} should resolve");
        }
    }

    #[test]
    fn a_multi_codepoint_name_decodes_the_whole_sequence() {
        // A gendered gesture is one glyph made of four codepoints.
        assert_eq!(
            codepoints_for("man-shrugging"),
            Some("1f937-200d-2642-fe0f")
        );
        assert_eq!(
            character_for("man-shrugging")
                .expect("resolves")
                .chars()
                .count(),
            4
        );
        // And a flag is a pair of regional indicators.
        assert_eq!(character_for("flag-pt").unwrap().chars().count(), 2);
    }

    /// Custom emoji must NOT resolve here: they are images, and claiming
    /// otherwise would replace someone's `:bongo:` with nothing.
    #[test]
    fn custom_names_are_left_alone() {
        for name in ["bongo", "prayge", "blobwave", "catjammers", "salutmonpote"] {
            assert!(character_for(name).is_none(), "{name} is custom");
        }
    }

    #[test]
    fn an_unknown_name_is_not_guessed_at() {
        assert!(character_for("").is_none());
        assert!(character_for("definitely_not_an_emoji_name").is_none());
        // Scanner artefacts from text like `12:53:` are not emoji either.
        assert!(character_for("53").is_none());
        assert!(character_for("https").is_none());
    }

    #[test]
    fn candidates_are_the_names_worth_asking_about() {
        let names = custom_candidates("ship it :tada: :bongo: and :prayge: :tada:");
        assert_eq!(
            names,
            vec!["bongo".to_string(), "prayge".to_string()],
            "standard names already have a character, and each name is asked once"
        );
    }

    #[test]
    fn a_bare_colon_is_not_a_candidate() {
        assert!(custom_candidates("no emoji here").is_empty());
        assert!(custom_candidates(":").is_empty());
        assert!(custom_candidates("::").is_empty());
    }
}
