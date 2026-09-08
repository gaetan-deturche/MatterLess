//! Mattermost-flavoured markdown to a node tree, parsed once in Rust.
//!
//! The official webapp re-parses markdown in JavaScript on every render. Doing
//! it here, once, cached by `post.id + update_at`, is the main lever this client
//! has that the webapp does not.
//!
//! Two layers: `pulldown-cmark` handles CommonMark, then a scanner walks the
//! resulting **text runs only** and lifts out the Mattermost extensions --
//! `@mentions`, `~channel-links`, `:emoji:`, bare URLs and inline `$maths$`.
//! Running that scanner on text runs rather than the raw source is what keeps it
//! from mangling the inside of a code span.
//!
//! Scope is set by the Phase 0 corpus rather than by guesswork: 20% of posts are
//! multiline, 13% mention someone, 7% carry a URL, 6% an emoji shortcode, 2%
//! inline code, 1% a fence, 1% a list, and none had a table or blockquote.
//! Syntax highlighting is deliberately **not** here -- see `CodeBlock`.

use matterless_core::text::is_username_char;
use pulldown_cmark::{CodeBlockKind, Event, Options, Parser, Tag, TagEnd};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "t", rename_all = "snake_case")]
pub enum Node {
    Text {
        value: String,
    },
    Emphasis {
        children: Vec<Node>,
    },
    Strong {
        children: Vec<Node>,
    },
    Strike {
        children: Vec<Node>,
    },
    InlineCode {
        value: String,
    },
    /// Raw text plus the fence's language. Highlighting is left to the shell:
    /// fences are 1% of posts, and `syntect` would cost a large dependency and
    /// a slower build to serve that 1%. Revisit if the number changes.
    CodeBlock {
        language: Option<String>,
        value: String,
    },
    Link {
        href: String,
        children: Vec<Node>,
    },
    /// A markdown image: `![alt](url)`.
    ///
    /// Dropped for a long time as "out of scope for v1", which showed a GIF
    /// posted through Mattermost's own picker as nothing but its alt text --
    /// the picker posts exactly this shape. The alt is flattened to a string
    /// because it is a label, not a body: it becomes the `alt` attribute and
    /// what is drawn if the image cannot load.
    Image {
        url: String,
        alt: String,
    },
    /// `@someone`. `user_id` is filled in by the row planner, which knows the
    /// member list; the parser only sees the name.
    UserMention {
        username: String,
        /// `@all`, `@here` and `@channel` address a room, not a person.
        ///
        /// Marked here rather than left to the shell because it is a fact about
        /// the mention, not a display choice -- and getting it wrong is
        /// visible: clicking `@here` asked the server for an account called
        /// "here" and got back "Unable to find an existing account matching
        /// your username for this team".
        everyone: bool,
    },
    /// `~channel-name`.
    ChannelLink {
        name: String,
    },
    /// `:shrug:`.
    ///
    /// `unicode` is filled here when the name is one of the standard set, which
    /// is a static table and so safe to resolve at parse time and cache with
    /// the parse. A custom emoji is an image behind the session token: it has
    /// no character, and the row payload carries its id separately because
    /// resolving it needs the server.
    Emoji {
        name: String,
        unicode: Option<String>,
    },
    /// Inline `$x^2$`. `EnableInlineLatex` is on for this server (block LaTeX is
    /// off), so this has to exist; the shell may render it as monospace for v1.
    InlineMath {
        value: String,
    },
    Paragraph {
        children: Vec<Node>,
    },
    Heading {
        level: u8,
        children: Vec<Node>,
    },
    Blockquote {
        children: Vec<Node>,
    },
    List {
        ordered: bool,
        items: Vec<Vec<Node>>,
    },
    Table {
        head: Vec<Vec<Node>>,
        rows: Vec<Vec<Vec<Node>>>,
    },
    Rule,
    SoftBreak,
    HardBreak,
}

/// Where the extension scanner found something in a text run.
enum Found {
    Url {
        start: usize,
        end: usize,
    },
    Mention {
        start: usize,
        end: usize,
        name: String,
    },
    Channel {
        start: usize,
        end: usize,
        name: String,
    },
    Emoji {
        start: usize,
        end: usize,
        name: String,
    },
    Math {
        start: usize,
        end: usize,
        value: String,
    },
}

impl Found {
    fn start(&self) -> usize {
        match self {
            Found::Url { start, .. }
            | Found::Mention { start, .. }
            | Found::Channel { start, .. }
            | Found::Emoji { start, .. }
            | Found::Math { start, .. } => *start,
        }
    }
}

/// True when the byte before `index` cannot be part of a word, which is what
/// stops `email@host` reading as a mention of `host`.
fn boundary_before(text: &str, index: usize) -> bool {
    if index == 0 {
        return true;
    }
    text[..index]
        .chars()
        .next_back()
        .is_none_or(|character| !character.is_alphanumeric() && character != '@')
}

fn scan_url(text: &str, from: usize) -> Option<Found> {
    for scheme in ["https://", "http://"] {
        if let Some(offset) = text[from..].find(scheme) {
            let start = from + offset;
            let rest = &text[start..];
            let mut end = start + rest.len();
            // A URL ends at whitespace; trailing sentence punctuation is not
            // part of it, which matters because people write "see https://x."
            if let Some(space) = rest.find(char::is_whitespace) {
                end = start + space;
            }
            while end > start
                && matches!(
                    text.as_bytes()[end - 1],
                    b'.' | b',' | b')' | b';' | b':' | b'!' | b'?' | b'\''
                )
            {
                end -= 1;
            }
            if end > start + scheme.len() {
                return Some(Found::Url { start, end });
            }
        }
    }
    None
}

fn scan_token(text: &str, from: usize, sigil: char) -> Option<(usize, usize, String)> {
    let mut cursor = from;
    while let Some(offset) = text[cursor..].find(sigil) {
        let start = cursor + offset;
        if !boundary_before(text, start) {
            cursor = start + sigil.len_utf8();
            continue;
        }
        let body_start = start + sigil.len_utf8();
        let body: String = text[body_start..]
            .chars()
            .take_while(|character| is_username_char(*character))
            .collect();
        // Trailing punctuation belongs to the sentence, not the name.
        let trimmed = body.trim_end_matches(['.', '-', '_']);
        if trimmed.is_empty() {
            cursor = body_start;
            continue;
        }
        return Some((start, body_start + trimmed.len(), trimmed.to_string()));
    }
    None
}

fn scan_emoji(text: &str, from: usize) -> Option<Found> {
    let mut cursor = from;
    while let Some(offset) = text[cursor..].find(':') {
        let start = cursor + offset;
        let body_start = start + 1;
        let close = text[body_start..].find(':')?;
        let name = &text[body_start..body_start + close];
        let plausible = !name.is_empty()
            && name.len() <= 64
            && name.chars().all(|character| {
                character.is_ascii_alphanumeric()
                    || character == '_'
                    || character == '+'
                    || character == '-'
            });
        if plausible {
            return Some(Found::Emoji {
                start,
                end: body_start + close + 1,
                name: name.to_string(),
            });
        }
        cursor = body_start;
    }
    None
}

fn scan_math(text: &str, from: usize) -> Option<Found> {
    let start = from + text[from..].find('$')?;
    let body_start = start + 1;
    let close = text[body_start..].find('$')?;
    if close == 0 {
        return None;
    }
    Some(Found::Math {
        start,
        end: body_start + close + 1,
        value: text[body_start..body_start + close].to_string(),
    })
}

/// Splits one text run into text and extension nodes.
fn expand_text(text: &str) -> Vec<Node> {
    let mut nodes = Vec::new();
    let mut cursor = 0;

    while cursor < text.len() {
        let mut candidates: Vec<Found> = Vec::new();
        if let Some(found) = scan_url(text, cursor) {
            candidates.push(found);
        }
        if let Some((start, end, name)) = scan_token(text, cursor, '@') {
            candidates.push(Found::Mention { start, end, name });
        }
        if let Some((start, end, name)) = scan_token(text, cursor, '~') {
            candidates.push(Found::Channel { start, end, name });
        }
        if let Some(found) = scan_emoji(text, cursor) {
            candidates.push(found);
        }
        if let Some(found) = scan_math(text, cursor) {
            candidates.push(found);
        }

        let Some(earliest) = candidates
            .into_iter()
            .min_by_key(|candidate| candidate.start())
        else {
            break;
        };

        let (start, end, node) = match earliest {
            Found::Url { start, end } => (
                start,
                end,
                Node::Link {
                    href: text[start..end].to_string(),
                    children: vec![Node::Text {
                        value: text[start..end].to_string(),
                    }],
                },
            ),
            Found::Mention { start, end, name } => {
                let everyone = matches!(name.as_str(), "all" | "here" | "channel");
                (
                    start,
                    end,
                    Node::UserMention {
                        username: name,
                        everyone,
                    },
                )
            }
            Found::Channel { start, end, name } => (start, end, Node::ChannelLink { name }),
            Found::Emoji { start, end, name } => {
                let unicode = crate::emoji::character_for(&name);
                (start, end, Node::Emoji { name, unicode })
            }
            Found::Math { start, end, value } => (start, end, Node::InlineMath { value }),
        };

        if start > cursor {
            nodes.push(Node::Text {
                value: text[cursor..start].to_string(),
            });
        }
        nodes.push(node);
        cursor = end;
    }

    if cursor < text.len() {
        nodes.push(Node::Text {
            value: text[cursor..].to_string(),
        });
    }
    nodes
}

/// Flushes buffered text through the extension scanner into the current frame.
///
/// Buffering matters: `pulldown-cmark` splits a text run at any character that
/// *might* be a delimiter, so with strikethrough enabled `~channel` arrives as
/// `"~"` then `"channel"`. A scanner that cannot see across that boundary never
/// finds the link.
fn flush_text(stack: &mut [Frame], pending: &mut String) {
    if pending.is_empty() {
        return;
    }
    let expanded = expand_text(pending);
    pending.clear();
    if let Some(children) = stack.last_mut().and_then(Frame::children) {
        children.extend(expanded);
    }
}

/// Frames the parser builds up as it walks events.
/// An image's alt text, flattened from whatever inline markup it was written
/// with.
///
/// A label rather than a body: it becomes the `alt` attribute and the thing
/// drawn when the image will not load, so nested emphasis and links have
/// nothing to contribute but their words.
fn plain_text(nodes: &[Node]) -> String {
    let mut out = String::new();
    fn walk(nodes: &[Node], out: &mut String) {
        for node in nodes {
            match node {
                Node::Text { value } | Node::InlineCode { value } | Node::InlineMath { value } => {
                    out.push_str(value)
                }
                Node::Emoji { name, unicode } => match unicode {
                    Some(character) => out.push_str(character),
                    None => {
                        out.push(':');
                        out.push_str(name);
                        out.push(':');
                    }
                },
                Node::UserMention { username, .. } => {
                    out.push('@');
                    out.push_str(username);
                }
                Node::ChannelLink { name } => {
                    out.push('~');
                    out.push_str(name);
                }
                Node::Image { alt, .. } => out.push_str(alt),
                Node::Emphasis { children }
                | Node::Strong { children }
                | Node::Strike { children }
                | Node::Link { children, .. }
                | Node::Paragraph { children }
                | Node::Heading { children, .. }
                | Node::Blockquote { children } => walk(children, out),
                Node::SoftBreak | Node::HardBreak => out.push(' '),
                _ => {}
            }
        }
    }
    walk(nodes, &mut out);
    out.trim().to_string()
}

enum Frame {
    Inline(Vec<Node>),
    Paragraph(Vec<Node>),
    Emphasis(Vec<Node>),
    Strong(Vec<Node>),
    Strike(Vec<Node>),
    Link {
        href: String,
        children: Vec<Node>,
    },
    Image {
        url: String,
        alt: Vec<Node>,
    },
    Heading {
        level: u8,
        children: Vec<Node>,
    },
    Blockquote(Vec<Node>),
    List {
        ordered: bool,
        items: Vec<Vec<Node>>,
    },
    ListItem(Vec<Node>),
    CodeBlock {
        language: Option<String>,
        text: String,
    },
    TableHead(Vec<Vec<Node>>),
    TableRow(Vec<Vec<Node>>),
    TableCell(Vec<Node>),
    Table {
        head: Vec<Vec<Node>>,
        rows: Vec<Vec<Vec<Node>>>,
    },
}

impl Frame {
    fn children(&mut self) -> Option<&mut Vec<Node>> {
        match self {
            Frame::Inline(children)
            | Frame::Paragraph(children)
            | Frame::Emphasis(children)
            | Frame::Strong(children)
            | Frame::Strike(children)
            | Frame::Link { children, .. }
            | Frame::Image { alt: children, .. }
            | Frame::Heading { children, .. }
            | Frame::Blockquote(children)
            | Frame::ListItem(children)
            | Frame::TableCell(children) => Some(children),
            _ => None,
        }
    }
}

pub fn parse(source: &str) -> Vec<Node> {
    if source.trim().is_empty() {
        return Vec::new();
    }
    let mut options = Options::empty();
    options.insert(Options::ENABLE_STRIKETHROUGH);
    options.insert(Options::ENABLE_TABLES);

    let mut stack: Vec<Frame> = vec![Frame::Inline(Vec::new())];
    let mut pending = String::new();

    let push_node = |stack: &mut Vec<Frame>, node: Node| {
        if let Some(children) = stack.last_mut().and_then(Frame::children) {
            children.push(node);
        }
    };

    for event in Parser::new_ext(source, options) {
        // Anything that is not more text closes the run being accumulated.
        if !matches!(event, Event::Text(_)) {
            flush_text(&mut stack, &mut pending);
        }
        match event {
            Event::Start(tag) => match tag {
                Tag::Paragraph => stack.push(Frame::Paragraph(Vec::new())),
                Tag::Emphasis => stack.push(Frame::Emphasis(Vec::new())),
                Tag::Strong => stack.push(Frame::Strong(Vec::new())),
                Tag::Strikethrough => stack.push(Frame::Strike(Vec::new())),
                Tag::Link { dest_url, .. } => stack.push(Frame::Link {
                    href: dest_url.to_string(),
                    children: Vec::new(),
                }),
                // The alt text arrives as inline events between here and
                // `TagEnd::Image`, so it collects like any other frame's
                // children and is flattened to a string at the end.
                Tag::Image { dest_url, .. } => stack.push(Frame::Image {
                    url: dest_url.to_string(),
                    alt: Vec::new(),
                }),
                Tag::Heading { level, .. } => stack.push(Frame::Heading {
                    level: level as u8,
                    children: Vec::new(),
                }),
                Tag::BlockQuote(_) => stack.push(Frame::Blockquote(Vec::new())),
                Tag::List(first) => stack.push(Frame::List {
                    ordered: first.is_some(),
                    items: Vec::new(),
                }),
                Tag::Item => stack.push(Frame::ListItem(Vec::new())),
                Tag::CodeBlock(kind) => {
                    let language = match kind {
                        CodeBlockKind::Fenced(info) if !info.is_empty() => {
                            Some(info.split_whitespace().next().unwrap_or("").to_string())
                        }
                        _ => None,
                    };
                    stack.push(Frame::CodeBlock {
                        language,
                        text: String::new(),
                    });
                }
                Tag::Table(_) => stack.push(Frame::Table {
                    head: Vec::new(),
                    rows: Vec::new(),
                }),
                Tag::TableHead => stack.push(Frame::TableHead(Vec::new())),
                Tag::TableRow => stack.push(Frame::TableRow(Vec::new())),
                Tag::TableCell => stack.push(Frame::TableCell(Vec::new())),
                // Footnotes are out of scope; their inline content still
                // flows through as text.
                _ => {}
            },
            Event::End(end) => {
                let finished = match end {
                    TagEnd::Paragraph
                    | TagEnd::Emphasis
                    | TagEnd::Strong
                    | TagEnd::Strikethrough
                    | TagEnd::Link
                    | TagEnd::Image
                    | TagEnd::Heading(_)
                    | TagEnd::BlockQuote(_)
                    | TagEnd::List(_)
                    | TagEnd::Item
                    | TagEnd::CodeBlock
                    | TagEnd::Table
                    | TagEnd::TableHead
                    | TagEnd::TableRow
                    | TagEnd::TableCell => stack.pop(),
                    _ => None,
                };
                let Some(frame) = finished else { continue };
                match frame {
                    Frame::Paragraph(children) => {
                        push_node(&mut stack, Node::Paragraph { children })
                    }
                    Frame::Emphasis(children) => push_node(&mut stack, Node::Emphasis { children }),
                    Frame::Strong(children) => push_node(&mut stack, Node::Strong { children }),
                    Frame::Strike(children) => push_node(&mut stack, Node::Strike { children }),
                    Frame::Link { href, children } => {
                        push_node(&mut stack, Node::Link { href, children })
                    }
                    Frame::Image { url, alt } => push_node(
                        &mut stack,
                        Node::Image {
                            url,
                            alt: plain_text(&alt),
                        },
                    ),
                    Frame::Heading { level, children } => {
                        push_node(&mut stack, Node::Heading { level, children })
                    }
                    Frame::Blockquote(children) => {
                        push_node(&mut stack, Node::Blockquote { children })
                    }
                    Frame::List { ordered, items } => {
                        push_node(&mut stack, Node::List { ordered, items })
                    }
                    Frame::ListItem(children) => {
                        if let Some(Frame::List { items, .. }) = stack.last_mut() {
                            items.push(children);
                        }
                    }
                    Frame::CodeBlock { language, text } => push_node(
                        &mut stack,
                        Node::CodeBlock {
                            language,
                            value: text.trim_end_matches('\n').to_string(),
                        },
                    ),
                    Frame::TableCell(children) => match stack.last_mut() {
                        Some(Frame::TableHead(cells)) | Some(Frame::TableRow(cells)) => {
                            cells.push(children)
                        }
                        _ => {}
                    },
                    Frame::TableHead(cells) => {
                        if let Some(Frame::Table { head, .. }) = stack.last_mut() {
                            *head = cells;
                        }
                    }
                    Frame::TableRow(cells) => {
                        if let Some(Frame::Table { rows, .. }) = stack.last_mut() {
                            rows.push(cells);
                        }
                    }
                    Frame::Table { head, rows } => {
                        push_node(&mut stack, Node::Table { head, rows })
                    }
                    Frame::Inline(_) => {}
                }
            }
            Event::Text(text) => {
                if let Some(Frame::CodeBlock { text: body, .. }) = stack.last_mut() {
                    body.push_str(&text);
                } else {
                    pending.push_str(&text);
                }
            }
            Event::Code(code) => push_node(
                &mut stack,
                Node::InlineCode {
                    value: code.to_string(),
                },
            ),
            Event::SoftBreak => push_node(&mut stack, Node::SoftBreak),
            Event::HardBreak => push_node(&mut stack, Node::HardBreak),
            Event::Rule => push_node(&mut stack, Node::Rule),
            // Raw HTML is never trusted: it is shown as the text it is.
            Event::Html(raw) | Event::InlineHtml(raw) => push_node(
                &mut stack,
                Node::Text {
                    value: raw.to_string(),
                },
            ),
            _ => {}
        }
    }

    flush_text(&mut stack, &mut pending);
    match stack.into_iter().next() {
        Some(Frame::Inline(nodes)) => nodes,
        _ => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(value: &str) -> Node {
        Node::Text {
            value: value.to_string(),
        }
    }

    /// Unwraps the single paragraph most posts are.
    fn inline(source: &str) -> Vec<Node> {
        match parse(source).into_iter().next() {
            Some(Node::Paragraph { children }) => children,
            other => panic!("expected one paragraph, got {other:?}"),
        }
    }

    #[test]
    fn a_markdown_image_is_a_node_not_its_alt_text() {
        // The shape Mattermost's own GIF picker posts. It used to parse as the
        // alt text alone, so a GIF arrived as the words "GIF by Somebody".
        let nodes = parse("![GIF by Somebody](https://media.example.com/a.gif)");
        let Some(Node::Paragraph { children }) = nodes.first() else {
            panic!("expected a paragraph, got {nodes:?}");
        };
        assert_eq!(
            children.as_slice(),
            [Node::Image {
                url: "https://media.example.com/a.gif".into(),
                alt: "GIF by Somebody".into(),
            }]
        );
    }

    #[test]
    fn an_image_alt_keeps_only_its_words() {
        let nodes = parse("![a *very* nice :tada: pic](https://example.com/x.png)");
        let Some(Node::Paragraph { children }) = nodes.first() else {
            panic!("expected a paragraph");
        };
        let Some(Node::Image { alt, .. }) = children.first() else {
            panic!("expected an image, got {children:?}");
        };
        assert_eq!(alt, "a very nice 🎉 pic");
    }

    #[test]
    fn a_linked_image_keeps_both() {
        // A GIF that is also a link: the image must survive being inside one.
        let nodes = parse("[![alt](https://example.com/a.gif)](https://example.com/page)");
        let Some(Node::Paragraph { children }) = nodes.first() else {
            panic!("expected a paragraph");
        };
        let Some(Node::Link {
            children: inner, ..
        }) = children.first()
        else {
            panic!("expected a link, got {children:?}");
        };
        assert!(
            matches!(inner.first(), Some(Node::Image { .. })),
            "the image is inside the link, got {inner:?}"
        );
    }

    #[test]
    fn plain_text_is_one_paragraph() {
        assert_eq!(inline("just a message"), vec![text("just a message")]);
    }

    #[test]
    fn an_empty_message_yields_nothing() {
        assert!(parse("").is_empty());
        assert!(parse("   \n  ").is_empty());
    }

    #[test]
    fn room_mentions_are_not_people() {
        // Clicking `@here` asked the server for an account called "here" and
        // got a 404 back, in a card, at the reader.
        for name in ["all", "here", "channel"] {
            let parsed = inline(&format!("hey @{name} look"));
            assert!(
                parsed.contains(&Node::UserMention {
                    username: name.into(),
                    everyone: true,
                }),
                "@{name} should be marked as addressing the room: {parsed:?}"
            );
        }
        assert!(inline("hey @ada").contains(&Node::UserMention {
            username: "ada".into(),
            everyone: false,
        }));
    }

    #[test]
    fn mentions_channels_and_emoji_are_lifted_out_of_text() {
        assert_eq!(
            inline("hey @ada see ~build-alerts :shipit:"),
            vec![
                text("hey "),
                Node::UserMention {
                    username: "ada".into(),
                    everyone: false,
                },
                text(" see "),
                Node::ChannelLink {
                    name: "build-alerts".into()
                },
                text(" "),
                Node::Emoji {
                    name: "shipit".into(),
                    // Custom on this server, so no character stands in for it.
                    unicode: None
                },
            ]
        );
    }

    #[test]
    fn an_email_is_not_a_mention() {
        assert_eq!(
            inline("write to ada@example.com please"),
            vec![text("write to ada@example.com please")],
            "the @ inside an address must not start a mention"
        );
    }

    #[test]
    fn a_mention_survives_trailing_punctuation() {
        assert_eq!(
            inline("thanks @ada."),
            vec![
                text("thanks "),
                Node::UserMention {
                    username: "ada".into(),
                    everyone: false,
                },
                text("."),
            ]
        );
    }

    #[test]
    fn bare_urls_become_links_without_swallowing_the_full_stop() {
        assert_eq!(
            inline("see https://mattermost.example.com/docs. thanks"),
            vec![
                text("see "),
                Node::Link {
                    href: "https://mattermost.example.com/docs".into(),
                    children: vec![text("https://mattermost.example.com/docs")],
                },
                text(". thanks"),
            ]
        );
    }

    #[test]
    fn extensions_are_not_applied_inside_code() {
        // The whole reason the scanner runs on text runs, not on the source.
        assert_eq!(
            inline("try `@ada :shipit:` verbatim"),
            vec![
                text("try "),
                Node::InlineCode {
                    value: "@ada :shipit:".into()
                },
                text(" verbatim"),
            ]
        );

        let fenced = parse("```rust\nlet who = \"@ada\";\n```");
        assert_eq!(
            fenced,
            vec![Node::CodeBlock {
                language: Some("rust".into()),
                value: "let who = \"@ada\";".into(),
            }]
        );
    }

    #[test]
    fn emphasis_and_strike_nest() {
        assert_eq!(
            inline("**bold _and italic_** ~~gone~~"),
            vec![
                Node::Strong {
                    children: vec![
                        text("bold "),
                        Node::Emphasis {
                            children: vec![text("and italic")]
                        }
                    ]
                },
                text(" "),
                Node::Strike {
                    children: vec![text("gone")]
                },
            ]
        );
    }

    #[test]
    fn inline_maths_is_kept_because_the_server_enables_it() {
        assert_eq!(
            inline("cost is $O(n^2)$ here"),
            vec![
                text("cost is "),
                Node::InlineMath {
                    value: "O(n^2)".into()
                },
                text(" here"),
            ]
        );
    }

    #[test]
    fn raw_html_is_shown_as_text_never_interpreted() {
        let nodes = parse("<img src=x onerror=alert(1)>");
        let flattened = format!("{nodes:?}");
        assert!(
            flattened.contains("Text"),
            "html must arrive as a Text node: {flattened}"
        );
        assert!(
            !flattened.contains("Link"),
            "and must not be turned into markup"
        );
    }

    #[test]
    fn multiline_posts_keep_their_soft_breaks() {
        let nodes = inline("first line\nsecond line");
        assert!(nodes.contains(&Node::SoftBreak));
    }

    #[test]
    fn lists_and_tables_survive() {
        assert_eq!(
            parse("- one\n- two"),
            vec![Node::List {
                ordered: false,
                items: vec![vec![text("one")], vec![text("two")],],
            }]
        );

        let table = parse("| a | b |\n|---|---|\n| 1 | 2 |");
        match &table[0] {
            Node::Table { head, rows } => {
                assert_eq!(head.len(), 2);
                assert_eq!(rows.len(), 1);
                assert_eq!(rows[0].len(), 2);
            }
            other => panic!("expected a table, got {other:?}"),
        }
    }
    #[test]
    fn a_standard_emoji_carries_its_character_and_a_custom_one_does_not() {
        let nodes = inline("ship it :tada: with :bongo:");
        let emoji: Vec<(&str, Option<&str>)> = nodes
            .iter()
            .filter_map(|node| match node {
                Node::Emoji { name, unicode } => Some((name.as_str(), unicode.as_deref())),
                _ => None,
            })
            .collect();
        assert_eq!(
            emoji,
            vec![("tada", Some("\u{1F389}")), ("bongo", None)],
            "a standard name resolves at parse time; a custom one is the payload's job"
        );
    }
}
