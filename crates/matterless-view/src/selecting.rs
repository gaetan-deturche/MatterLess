//! Choosing some of a conversation's words, to copy them.
//!
//! The words are shaped glyphs, not a text box, so there is no caret to move:
//! a place in them is found from where the glyphs were drawn last frame --
//! `Piece::Letters`, one per block, which the painter reports as it shapes --
//! and said as a message, a block of it and a byte into that block's text. A
//! message rather than a row number, so a selection means the same words when
//! history arrives above it, and nothing at all once the channel has changed.

use matterless_layout::row::{Block, Kind};
use matterless_paint::Letter;
use matterless_ui::Rect;

/// A place between two characters of one message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Spot {
    pub post: String,
    /// Which of the message's blocks, and how far into its text.
    pub block: usize,
    pub at: usize,
}

/// One block's letters as the last frame drew them.
#[derive(Debug, Clone)]
pub struct Seen {
    pub post: String,
    pub block: usize,
    pub letters: Vec<Letter>,
}

/// Whether a block holds words somebody can select: what was written, rather
/// than the name over it, a day's heading or a card's frame.
pub fn selectable(block: &Block) -> bool {
    matches!(
        block.kind,
        Kind::Text | Kind::Quote | Kind::Code | Kind::Caption | Kind::Preview
    )
}

/// The place nearest `(x, y)`: on the line under it, or on the nearest line
/// when it is between two -- so a drag across the gap between messages, or
/// over a name, carries on selecting rather than stopping.
pub fn spot_at(seen: &[Seen], x: f32, y: f32) -> Option<Spot> {
    let mut nearest: Option<(f32, &Seen, f32)> = None;
    for one in seen {
        for letter in &one.letters {
            let away = if y < letter.y {
                letter.y - y
            } else if y > letter.y + letter.height {
                y - letter.y - letter.height
            } else {
                0.0
            };
            if nearest.is_none_or(|(best, _, _)| away < best) {
                nearest = Some((away, one, letter.y));
            }
        }
    }
    let (_, one, line) = nearest?;
    let on_line: Vec<&Letter> = one
        .letters
        .iter()
        .filter(|letter| (letter.y - line).abs() < 0.5)
        .collect();
    // The letter the pointer is over, and which half of it; past the end of
    // the line, the end of it.
    let at = match on_line.iter().find(|letter| x < letter.x + letter.width) {
        Some(letter) if x < letter.x + letter.width / 2.0 => letter.start,
        Some(letter) => letter.end,
        None => on_line.iter().map(|letter| letter.end).max().unwrap_or(0),
    };
    Some(Spot {
        post: one.post.clone(),
        block: one.block,
        at,
    })
}

/// A block's text, as the letters count into it.
pub fn text_of(block: &Block) -> String {
    block.spans.iter().map(|span| span.text.as_str()).collect()
}

/// The word around `at` in a block's text, or the one character there when it
/// is not part of a word: what a double click takes.
pub fn word_at(text: &str, at: usize) -> (usize, usize) {
    let wordy = |c: char| c.is_alphanumeric() || c == '_';
    let at = at.min(text.len());
    let before = text[..at].char_indices().rev();
    let start = before
        .take_while(|(_, c)| wordy(*c))
        .last()
        .map_or(at, |(i, _)| i);
    let end = text[at..]
        .char_indices()
        .find(|(_, c)| !wordy(*c))
        .map_or(text.len(), |(i, _)| at + i);
    if start == end {
        // Not in a word: the character there, so the press still selects
        // something.
        let next = text[at..].chars().next().map_or(0, char::len_utf8);
        return (at, at + next);
    }
    (start, end)
}

/// Some of a block's text as it is copied: a custom emoji, drawn over spaces,
/// is copied as the `:name:` it was written as.
pub fn copied(block: &Block, from: usize, to: usize) -> String {
    let mut said = String::new();
    let mut offset = 0usize;
    for span in &block.spans {
        let (start, end) = (offset, offset + span.text.len());
        offset = end;
        let (lo, hi) = (from.max(start), to.min(end));
        if lo >= hi {
            continue;
        }
        match &span.emoji {
            Some(name) => said.push_str(&format!(":{name}:")),
            None => said.push_str(span.text.get(lo - start..hi - start).unwrap_or_default()),
        }
    }
    said
}

/// The boxes to light under the chosen part of one block, one per line.
pub fn lit(letters: &[Letter], from: usize, to: usize) -> Vec<Rect> {
    let mut boxes: Vec<Rect> = Vec::new();
    for letter in letters
        .iter()
        .filter(|letter| letter.start >= from && letter.end <= to)
    {
        match boxes.last_mut() {
            Some(last) if (last.y - letter.y).abs() < 0.5 => {
                let right = (letter.x + letter.width).max(last.right());
                last.x = last.x.min(letter.x);
                last.width = right - last.x;
            }
            _ => boxes.push(Rect::new(letter.x, letter.y, letter.width, letter.height)),
        }
    }
    boxes
}

#[cfg(test)]
mod tests {
    use super::*;
    use matterless_layout::row::TextSpan;

    fn letter(x: f32, y: f32, start: usize) -> Letter {
        Letter {
            x,
            y,
            width: 10.0,
            height: 18.0,
            start,
            end: start + 1,
        }
    }

    /// "abc" on one line and "de" under it, ten pixels a letter.
    fn two_lines() -> Vec<Seen> {
        vec![Seen {
            post: "p".into(),
            block: 0,
            letters: vec![
                letter(0.0, 0.0, 0),
                letter(10.0, 0.0, 1),
                letter(20.0, 0.0, 2),
                letter(0.0, 18.0, 3),
                letter(10.0, 18.0, 4),
            ],
        }]
    }

    /// The side of a letter the pointer is on decides whether the place is
    /// before it or after; past a line's end is its end; between lines is
    /// the nearest.
    #[test]
    fn a_place_is_found_from_where_the_letters_were_drawn() {
        let seen = two_lines();
        let at = |x, y| spot_at(&seen, x, y).map(|spot| spot.at);
        assert_eq!(at(12.0, 5.0), Some(1), "the left half of b is before it");
        assert_eq!(at(18.0, 5.0), Some(2), "the right half is after it");
        assert_eq!(at(90.0, 5.0), Some(3), "past the end of the line");
        assert_eq!(at(1.0, 25.0), Some(3), "the second line");
        assert_eq!(at(1.0, 400.0), Some(3), "below everything: the last line");
        assert_eq!(spot_at(&[], 1.0, 1.0), None);
    }

    /// A double click takes the word, or the one character when there is none.
    #[test]
    fn a_word_is_the_letters_around_the_place() {
        let text = "copy the_word, now";
        assert_eq!(word_at(text, 7), (5, 13), "inside the_word");
        assert_eq!(word_at(text, 0), (0, 4));
        assert_eq!(
            word_at(text, 13),
            (5, 13),
            "at the end of the word, the word"
        );
        assert_eq!(
            word_at("copy ,, now", 5),
            (5, 6),
            "no word either side: the comma"
        );
    }

    /// A custom emoji is copied as its name, and the rest as written.
    #[test]
    fn a_custom_emoji_is_copied_as_its_name() {
        let span = |text: &str, emoji: Option<&str>| TextSpan {
            text: text.to_string(),
            bold: false,
            italic: false,
            mono: false,
            press: None,
            faint: false,
            emoji: emoji.map(str::to_string),
        };
        let block = Block {
            y: 0.0,
            x: 0.0,
            height: 18.0,
            lines: 1,
            kind: Kind::Text,
            spans: vec![
                span("hi ", None),
                span("\u{a0}\u{a0}", Some("party")),
                span(" all", None),
            ],
            size: 14.0,
            wrap: 400.0,
        };
        let whole = text_of(&block);
        assert_eq!(copied(&block, 0, whole.len()), "hi :party: all");
        assert_eq!(copied(&block, 1, 3), "i ");
    }

    /// The light under a selection is one box a line.
    #[test]
    fn the_light_is_one_box_a_line() {
        let seen = two_lines();
        let boxes = lit(&seen[0].letters, 1, 5);
        assert_eq!(boxes.len(), 2);
        assert_eq!((boxes[0].x, boxes[0].width), (10.0, 20.0));
        assert_eq!((boxes[1].x, boxes[1].width), (0.0, 20.0));
    }
}
