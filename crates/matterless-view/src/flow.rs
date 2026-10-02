//! A few words of markdown as words and links, laid out in lines.
//!
//! For text that is not a message -- a channel's header and purpose -- but
//! whose links still have to be followable: each word is placed on its own,
//! so a link's words know where they landed and can be pressed.

use matterless_layout::{Fonts, Style};
use matterless_render::markdown::{self, Node};

/// A run of text, and where it leads when it is a link.
pub type Segment = (String, Option<String>);

/// One word where it landed, relative to the block's top left.
#[derive(Debug, Clone, PartialEq)]
pub struct Word {
    pub text: String,
    pub x: f32,
    pub y: f32,
    pub width: f32,
    /// Which link it belongs to, by the order links come in.
    pub link: Option<usize>,
}

/// The markdown as runs of text and links, and the links' addresses in order.
pub fn segments(written: &str) -> (Vec<Segment>, Vec<String>) {
    let mut runs = Vec::new();
    walk(&markdown::parse(written), &mut runs);
    let mut links: Vec<String> = Vec::new();
    for (_, href) in &runs {
        if let Some(href) = href
            && !links.contains(href)
        {
            links.push(href.clone());
        }
    }
    (runs, links)
}

fn walk(nodes: &[Node], runs: &mut Vec<Segment>) {
    for node in nodes {
        match node {
            Node::Link { href, children } => {
                runs.push((markdown::plain_text(children), Some(href.clone())));
            }
            Node::Paragraph { children }
            | Node::Heading { children, .. }
            | Node::Blockquote { children }
            | Node::Strong { children }
            | Node::Emphasis { children }
            | Node::Strike { children } => {
                walk(children, runs);
                runs.push((" ".into(), None));
            }
            Node::SoftBreak | Node::HardBreak => runs.push((" ".into(), None)),
            other => runs.push((markdown::plain_text(std::slice::from_ref(other)), None)),
        }
    }
}

/// Lays the runs out in `width`: wrapped, or on one line cut with an ellipsis.
/// Answers the words, the height they take, and whether any were cut away.
pub fn lay(
    fonts: &mut Fonts,
    runs: &[Segment],
    links: &[String],
    width: f32,
    style: Style,
    one_line: bool,
) -> (Vec<Word>, f32, bool) {
    let measure = |fonts: &mut Fonts, text: &str| {
        matterless_layout::extent_of(fonts, text, f32::MAX, style).width
    };
    let space = measure(fonts, "a b") - measure(fonts, "ab");
    let mut words = Vec::new();
    let (mut x, mut y) = (0.0_f32, 0.0_f32);
    for (text, href) in runs {
        let link = href
            .as_ref()
            .and_then(|href| links.iter().position(|one| one == href));
        for word in text.split_whitespace() {
            // A gap before every word but the first on a line.
            let gap = if x > 0.0 { space } else { 0.0 };
            let wide = measure(fonts, word);
            if x + gap + wide > width && x > 0.0 {
                if one_line {
                    let room = width - x - gap;
                    let cut = matterless_layout::elided(fonts, word, room.max(0.0), style);
                    if room > 12.0 && !cut.is_empty() {
                        let cut_wide = measure(fonts, &cut);
                        words.push(Word {
                            text: cut,
                            x: x + gap,
                            y,
                            width: cut_wide,
                            link,
                        });
                    }
                    return (words, style.line_height, true);
                }
                x = 0.0;
                y += style.line_height;
            }
            let gap = if x > 0.0 { gap } else { 0.0 };
            // A word wider than the whole line is cut to it.
            let (shown, shown_wide) = match wide > width {
                true => {
                    let cut = matterless_layout::elided(fonts, word, width, style);
                    let cut_wide = measure(fonts, &cut);
                    (cut, cut_wide)
                }
                false => (word.to_string(), wide),
            };
            words.push(Word {
                text: shown,
                x: x + gap,
                y,
                width: shown_wide,
                link,
            });
            x += gap + shown_wide;
        }
    }
    let height = if words.is_empty() {
        0.0
    } else {
        y + style.line_height
    };
    (words, height, false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_header_is_its_words_and_its_links() {
        let (runs, links) = segments("[Trombi](https://miro.com/x) | [Map](https://maps/y) ok");
        let linked: Vec<(&str, Option<&str>)> = runs
            .iter()
            .filter(|(text, _)| !text.trim().is_empty())
            .map(|(text, href)| (text.trim(), href.as_deref()))
            .collect();
        assert_eq!(
            linked,
            [
                ("Trombi", Some("https://miro.com/x")),
                ("|", None),
                ("Map", Some("https://maps/y")),
                ("ok", None),
            ]
        );
        assert_eq!(links, ["https://miro.com/x", "https://maps/y"]);
    }

    #[test]
    fn a_bare_address_is_a_link_too() {
        let (_, links) = segments("see https://example.com/page");
        assert_eq!(links, ["https://example.com/page"]);
    }
}
