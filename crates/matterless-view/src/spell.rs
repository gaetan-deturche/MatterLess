//! Spelling, in French and English at once.
//!
//! Hunspell dictionaries through Spellbook, a pure Rust reading of them -- the
//! same dictionaries the official desktop client checks with on Windows. A
//! word is right if either language accepts it, because the conversations
//! here switch between the two inside one sentence: "CL pour pouvoir
//! configurer le state initial d'un segment" is correct, and a checker that
//! picked one language per message would underline half of it.
//!
//! Measured before it went in: the English dictionary loads in 12ms and the
//! French in 26ms, and a word is checked against both in about 2us -- so a
//! whole message is checked every time it changes. Suggestions cost tens of
//! milliseconds a word and are only asked for when somebody asks.
//!
//! Loaded once, on a thread of its own, so the first frame never waits for
//! it: until it is there, nothing is underlined.

use std::collections::HashSet;
use std::ops::Range;
use std::sync::{LazyLock, OnceLock, RwLock};

/// The dictionaries, in the order suggestions are preferred when a word could
/// belong to either.
const BUNDLED: [(&str, &str, &str); 2] = [
    (
        "fr",
        include_str!("../resources/dictionaries/fr.aff"),
        include_str!("../resources/dictionaries/fr.dic"),
    ),
    (
        "en_US",
        include_str!("../resources/dictionaries/en_US.aff"),
        include_str!("../resources/dictionaries/en_US.dic"),
    ),
];

/// How many suggestions are offered for one word.
const SUGGESTIONS: usize = 5;

pub struct Speller {
    dictionaries: Vec<(String, spellbook::Dictionary)>,
}

static SPELLER: OnceLock<Speller> = OnceLock::new();

/// Words this reader has said are right: names, jargon, a team's own
/// vocabulary. Checked before the dictionaries, and case-insensitively.
///
/// Apart from the speller so they can be taken before it has loaded: they are
/// read back from the store when the reader is known, which can be before the
/// dictionaries are ready.
static OWN: LazyLock<RwLock<HashSet<String>>> = LazyLock::new(Default::default);

/// Takes words the reader has said are right.
pub fn learn_all(words: impl IntoIterator<Item = String>) {
    let mut own = OWN.write().unwrap_or_else(|held| held.into_inner());
    own.extend(words.into_iter().map(|word| word.to_lowercase()));
}

/// The name the reader's own words are kept under in the store's settings.
pub const OWN_WORDS: &str = "spelling.words";

/// The speller, once it has loaded. `None` until then, which draws as no
/// underlines rather than as a wait.
pub fn speller() -> Option<&'static Speller> {
    SPELLER.get()
}

/// Loads the bundled dictionaries now, on this thread. For a test, which has
/// to know they are there before it asks.
pub fn load_now() -> &'static Speller {
    SPELLER.get_or_init(Speller::bundled)
}

/// Starts loading the bundled dictionaries, off the thread that draws.
pub fn load_in_background() {
    std::thread::spawn(|| {
        let began = std::time::Instant::now();
        let speller = SPELLER.get_or_init(Speller::bundled);
        println!(
            "spelling: {} dictionaries in {}ms",
            speller.dictionaries.len(),
            began.elapsed().as_millis()
        );
    });
}

impl Speller {
    /// The French and English dictionaries this build carries.
    pub fn bundled() -> Self {
        let dictionaries = BUNDLED
            .iter()
            .filter_map(
                |(name, aff, dic)| match spellbook::Dictionary::new(aff, dic) {
                    Ok(dictionary) => Some((name.to_string(), dictionary)),
                    Err(error) => {
                        eprintln!("the {name} dictionary could not be read: {error}");
                        None
                    }
                },
            )
            .collect();
        Self { dictionaries }
    }

    /// Whether this one word is spelled right in either language.
    pub fn knows(&self, word: &str) -> bool {
        // Typed with a typographic apostrophe more often than not, and the
        // dictionaries are written with the straight one.
        let word = word.replace('\u{2019}', "'");
        if OWN
            .read()
            .unwrap_or_else(|held| held.into_inner())
            .contains(&word.to_lowercase())
        {
            return true;
        }
        if self
            .dictionaries
            .iter()
            .any(|(_, dictionary)| dictionary.check(&word))
        {
            return true;
        }
        // "peut-être" is one word to a dictionary; "Reach-Octesta" is two, each
        // of which is fine. A hyphenated run is right when all of its parts are.
        word.contains('-')
            && word.split('-').filter(|part| !part.is_empty()).all(|part| {
                self.dictionaries
                    .iter()
                    .any(|(_, dictionary)| dictionary.check(part))
            })
    }

    /// Where the misspelled words are in `text`, as byte ranges.
    pub fn misspelled(&self, text: &str) -> Vec<Range<usize>> {
        candidates(text)
            .into_iter()
            .filter(|range| !self.knows(&text[range.clone()]))
            .collect()
    }

    /// What the word might have been meant to be, best first.
    ///
    /// From the language the rest of the sentence is written in when that can
    /// be told, and from both otherwise: "netoyage" in a French sentence
    /// wants "nettoyage", not the English dictionary's "magneto".
    pub fn suggest(&self, word: &str, sentence: &str) -> Vec<String> {
        let word = word.replace('\u{2019}', "'");
        let leaning = self.leaning(sentence);
        let mut order: Vec<&(String, spellbook::Dictionary)> = self.dictionaries.iter().collect();
        if let Some(name) = leaning.as_ref() {
            order.sort_by_key(|(dictionary, _)| dictionary != name);
        }
        let mut out: Vec<String> = Vec::new();
        for (_, dictionary) in order {
            let mut found = Vec::new();
            dictionary.suggest(&word, &mut found);
            for one in found {
                if !out.iter().any(|kept| kept.eq_ignore_ascii_case(&one)) {
                    out.push(one);
                }
            }
            // Enough from the language the sentence is in: the other one only
            // tops the list up.
            if leaning.is_some() && out.len() >= SUGGESTIONS {
                break;
            }
        }
        out.truncate(SUGGESTIONS);
        out
    }

    /// Which dictionary the words of a sentence belong to, when one of them
    /// knows clearly more of them than the other.
    fn leaning(&self, sentence: &str) -> Option<String> {
        let mut counts: Vec<(String, usize)> = self
            .dictionaries
            .iter()
            .map(|(name, _)| (name.clone(), 0))
            .collect();
        for range in candidates(sentence) {
            let word = sentence[range].replace('\u{2019}', "'");
            let knowing: Vec<usize> = self
                .dictionaries
                .iter()
                .enumerate()
                .filter(|(_, (_, dictionary))| dictionary.check(&word))
                .map(|(at, _)| at)
                .collect();
            // A word both languages share says nothing about which this is.
            if let [only] = knowing.as_slice() {
                counts[*only].1 += 1;
            }
        }
        counts.sort_by_key(|(_, count)| std::cmp::Reverse(*count));
        match counts.as_slice() {
            [(name, best), (_, next), ..] if *best > *next => Some(name.clone()),
            [(name, best)] if *best > 0 => Some(name.clone()),
            _ => None,
        }
    }
}

/// The words in `text` that are worth checking, as byte ranges.
///
/// Prose only. Code, between backticks or fenced, is somebody's program;
/// a link, a mention, a channel or an emoji's name is a name; and anything
/// with a digit, an underscore, a dot or a slash in it is a version, a path or
/// an identifier -- `MI_Gunfuse_WhiteCrab`, `UE5.6`, `r.slcp.X`. A word in
/// capitals is an acronym (CL, PS5, BWC), and one with capitals inside it is
/// an identifier (`greyBlock`, `UCYExperiment`). None of those are spelling.
pub fn candidates(text: &str) -> Vec<Range<usize>> {
    let mut out = Vec::new();
    let mut fenced = false;
    let mut line_start = 0;
    for line in text.split_inclusive('\n') {
        let base = line_start;
        line_start += line.len();
        if line.trim_start().starts_with("```") {
            fenced = !fenced;
            continue;
        }
        if fenced {
            continue;
        }
        // Inline code: whatever sits between two backticks on this line.
        let mut in_code = false;
        let mut token_start: Option<usize> = None;
        let mut spans: Vec<Range<usize>> = Vec::new();
        for (at, c) in line.char_indices() {
            if c == '`' {
                if let Some(start) = token_start.take() {
                    spans.push(start..at);
                }
                in_code = !in_code;
                continue;
            }
            if in_code {
                continue;
            }
            match (c.is_whitespace(), token_start) {
                (true, Some(start)) => {
                    spans.push(start..at);
                    token_start = None;
                }
                (false, None) => token_start = Some(at),
                _ => {}
            }
        }
        if let (Some(start), false) = (token_start, in_code) {
            spans.push(start..line.len());
        }
        for span in spans {
            if let Some(word) = word_in(&line[span.clone()]) {
                out.push(base + span.start + word.start..base + span.start + word.end);
            }
        }
    }
    out
}

/// The word inside one whitespace-separated token, if the token is prose.
fn word_in(token: &str) -> Option<Range<usize>> {
    // A mention, a channel, an emoji's name, a link.
    if token.starts_with(['@', '~', ':', '#']) || token.contains("://") || token.starts_with("www.")
    {
        return None;
    }
    let is_word_char = |c: char| c.is_alphabetic() || c == '\'' || c == '\u{2019}' || c == '-';
    // Punctuation off both ends: "(crash)," is the word "crash".
    let start = token.char_indices().find(|(_, c)| c.is_alphabetic())?.0;
    let end = token
        .char_indices()
        .rev()
        .find(|(_, c)| c.is_alphabetic())
        .map(|(at, c)| at + c.len_utf8())?;
    let word = &token[start..end];
    // Anything left inside that is not a letter, an apostrophe or a hyphen
    // makes it something other than a word: a path, a version, an identifier.
    if !word.chars().all(is_word_char) {
        return None;
    }
    if token.chars().any(|c| c.is_ascii_digit() || c == '_') {
        return None;
    }
    let letters: Vec<char> = word.chars().filter(|c| c.is_alphabetic()).collect();
    if letters.len() < 2 {
        return None;
    }
    // An acronym.
    if letters.iter().all(|c| c.is_uppercase()) {
        return None;
    }
    // Capitals after the first letter: an identifier, or a name written as one.
    if word
        .split(['-', '\'', '\u{2019}'])
        .any(|part| part.chars().skip(1).any(char::is_uppercase))
    {
        return None;
    }
    Some(start..end)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn checked(text: &str) -> Vec<&str> {
        candidates(text)
            .into_iter()
            .map(|range| &text[range])
            .collect()
    }

    fn speller() -> &'static Speller {
        static ONE: OnceLock<Speller> = OnceLock::new();
        ONE.get_or_init(Speller::bundled)
    }

    fn wrong(text: &str) -> Vec<&str> {
        speller()
            .misspelled(text)
            .into_iter()
            .map(|range| &text[range])
            .collect()
    }

    /// What the conversations here are written in: French and English in the
    /// same sentence, and neither is a mistake.
    #[test]
    fn french_and_english_in_one_sentence_are_both_right() {
        assert!(wrong("CL pour pouvoir configurer le state initial d'un segment").is_empty());
        assert!(wrong("Merci pour le nettoyage du piège à rat").is_empty());
        assert!(wrong("Hello, I have a random crash when moving in editor view").is_empty());
        assert!(wrong("Est-ce que vous savez pourquoi ?").is_empty());
        assert!(
            wrong("Merci messieurs pour le coup de main hier").is_empty(),
            "accents"
        );
    }

    #[test]
    fn a_misspelling_is_found_in_either_language() {
        assert_eq!(wrong("Mercu pour le netoyage"), vec!["Mercu", "netoyage"]);
        assert_eq!(wrong("I hav a crsh when moving"), vec!["hav", "crsh"]);
    }

    /// A typographic apostrophe is the same apostrophe.
    #[test]
    fn a_curly_apostrophe_is_an_apostrophe() {
        assert!(wrong("l\u{2019}état d\u{2019}un segment").is_empty());
    }

    /// Code, links, names and identifiers are not prose, and nothing in them
    /// is a spelling mistake.
    #[test]
    fn what_is_not_prose_is_not_checked() {
        assert_eq!(checked("`grayblok xyzzy` and"), ["and"]);
        assert_eq!(checked("```\nfn mian() {}\n```\nok then"), ["ok", "then"]);
        assert_eq!(
            checked("see https://swarm.sloclap.net/reviews/208031"),
            ["see"]
        );
        assert_eq!(
            checked("@arthur.trouslard ~dev :kirbyjam: merci"),
            ["merci"]
        );
        assert_eq!(
            checked("MI_Gunfuse_WhiteCrab UE5.6 r.slcp.X PS5 BWC"),
            Vec::<&str>::new()
        );
        assert_eq!(checked("greyBlock UCYExperiment"), Vec::<&str>::new());
        assert_eq!(checked("(crash), \"moving\"!"), ["crash", "moving"]);
    }

    /// What is underlined is exactly the word, so the line lands under it.
    #[test]
    fn a_range_is_the_word_and_not_its_punctuation() {
        let text = "Oui, le netoyage.";
        let found = speller().misspelled(text);
        assert_eq!(found.len(), 1);
        assert_eq!(&text[found[0].clone()], "netoyage");
    }

    /// A word the reader has said is right stays right.
    #[test]
    fn a_word_learnt_is_known() {
        let speller = speller();
        assert!(!speller.knows("zorblaxian"));
        learn_all(["Zorblaxian".to_string()]);
        assert!(speller.knows("zorblaxian"));
        assert!(speller.knows("Zorblaxian"));
    }

    /// Suggestions come from the language the sentence is in.
    #[test]
    fn suggestions_follow_the_sentence() {
        let french = speller().suggest("netoyage", "merci pour le netoyage du piège");
        assert_eq!(
            french.first().map(String::as_str),
            Some("nettoyage"),
            "{french:?}"
        );
        let english = speller().suggest("crsh", "I have a crsh when moving");
        assert_eq!(
            english.first().map(String::as_str),
            Some("crash"),
            "{english:?}"
        );
        assert!(english.len() <= SUGGESTIONS);
    }
}
