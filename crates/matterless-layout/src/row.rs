//! A whole row of the message list, laid out exactly.
//!
//! `extent_of` answers for one run of text. This answers for a row: a message
//! with its author line, its paragraphs and code blocks and lists, its
//! reactions and its attachments -- the number the virtualiser needs before the
//! row is drawn, and the number a renderer will draw from. The two cannot
//! disagree, because there is only one of them.

use crate::Fonts;
use cosmic_text::{Attrs, Buffer, Metrics, Shaping, Weight};
use matterless_render::Row;
use matterless_render::markdown::Node;

/// Every measurement the list is built from, in one place.
///
/// Held together rather than scattered because the layout and the renderer have
/// to agree exactly: a padding known to one of them and not the other is the
/// same class of bug as an estimated height.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Theme {
    /// The list's own width, before any gutter is taken out of it.
    pub width: f32,
    /// The avatar column and the gap after it. Text never gets this.
    pub gutter: f32,
    pub body_size: f32,
    pub line_height: f32,
    /// The author-and-time line above a message, which a continuation omits.
    pub header_height: f32,
    /// The author's name, which the stylesheet sets a little below the body.
    pub header_size: f32,
    /// Above and below a row's content. `.post { padding: 3px 0 }`.
    pub row_padding: f32,
    /// Between two rows. The stream is a flex column with `gap: 2px`, which is
    /// two pixels every row owes the one after it.
    pub row_gap: f32,
    /// What the stream keeps clear of its own edges: `padding: 12px 16px 4px`.
    /// Sideways this is why a message does not start against the panel, and a
    /// long line does not end against the scrollbar.
    pub pad_x: f32,
    pub pad_top: f32,
    pub pad_bottom: f32,
    /// Between the blocks inside a message.
    pub block_gap: f32,
    pub code_size: f32,
    pub code_line_height: f32,
    /// Padding a code block draws around its lines: `10px 12px`.
    pub code_padding: f32,
    pub code_padding_y: f32,
    /// A file card: an attachment that is not a picture, drawn as a fixed row
    /// with its name and size on it.
    pub card_height: f32,
    /// The words on a pill and on a separator, both set small.
    pub small_size: f32,
    /// A join, a leave, or a deleted root: `.system { font-size: 12.5px }`.
    pub system_size: f32,
    /// Inside a reaction pill, and between two of them.
    pub pill_padding: f32,
    pub pill_gap: f32,
    /// The square a custom emoji is drawn in, which has no character to shape.
    pub emoji_size: f32,
    pub reaction_height: f32,
    pub separator_height: f32,
    pub footer_height: f32,
    /// How far a list item or a quote is pushed in: `padding-left: 22px`.
    pub indent: f32,
    /// The bar beside a preview card, and the gap between it and the text.
    pub quote_bar: f32,
    /// `max-width: 520px` on a preview, and the `8px 10px` inside it.
    pub preview_width: f32,
    pub card_padding: f32,
    /// What a webhook's attachment is set at: `font-size: 13.5px`.
    pub attached_size: f32,
    /// How many lines of a description or a quoted message are kept. Past this
    /// a card stops being a summary and starts being the page.
    pub preview_lines: usize,
    /// The reader's own offset, so a timestamp says what their clock says.
    pub utc_offset_minutes: i32,
    /// Today, in the same days-since-epoch the separators count in.
    ///
    /// Only so a date in this year can leave the year off. Zero means nobody
    /// said, and every date then carries its year -- wordier, but never wrong.
    pub today: i64,
}

impl Default for Theme {
    /// The stylesheet's own numbers: 14px body at 1.5, a 28px avatar with an
    /// 8px gap.
    fn default() -> Self {
        Self {
            width: 900.0,
            gutter: 36.0,
            body_size: 14.0,
            line_height: 21.0,
            header_height: 20.0,
            header_size: 13.5,
            row_padding: 3.0,
            row_gap: 2.0,
            pad_x: 16.0,
            pad_top: 12.0,
            pad_bottom: 4.0,
            block_gap: 6.0,
            code_size: 12.5,
            code_line_height: 18.0,
            code_padding: 12.0,
            code_padding_y: 10.0,
            card_height: 56.0,
            pill_padding: 7.0,
            pill_gap: 5.0,
            small_size: 11.5,
            system_size: 12.5,
            emoji_size: 16.0,
            reaction_height: 25.0,
            separator_height: 34.0,
            footer_height: 26.0,
            indent: 22.0,
            quote_bar: 3.0,
            preview_width: 520.0,
            card_padding: 10.0,
            attached_size: 13.5,
            preview_lines: 3,
            utc_offset_minutes: 0,
            today: 0,
        }
    }
}

impl Theme {
    /// The width a line of body text actually gets.
    pub fn text_width(&self) -> f32 {
        (self.width - self.gutter).max(40.0)
    }
}

/// What a row occupies, and the pieces that make it up.
///
/// The parts are kept rather than summed away so a renderer can draw them and a
/// test can say *why* a height is what it is.
#[derive(Debug, Clone, PartialEq)]
pub struct RowLayout {
    pub height: f32,
    pub blocks: Vec<Block>,
}

/// One stacked piece of a row.
#[derive(Debug, Clone, PartialEq)]
pub struct Block {
    /// Distance from the row's top edge.
    pub y: f32,
    /// Distance from the text column's left edge, for indented content.
    pub x: f32,
    pub height: f32,
    pub lines: usize,
    pub kind: Kind,
    /// What to draw, for a block that is text. Empty for the rest.
    pub spans: Vec<TextSpan>,
    /// The size the spans were measured at, which a heading raises.
    pub size: f32,
    /// The width the spans were wrapped to. A renderer that wraps to anything
    /// else draws a different number of lines than the layout reserved.
    pub wrap: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// The author and time line.
    Header,
    Text,
    /// Pre-wrapped: it scrolls sideways rather than wrapping.
    Code,
    /// A line inside a blockquote: text with a bar down its left and set a
    /// step quieter.
    Quote,
    Reactions,
    /// The line that says where a reader stopped last time. Its own kind
    /// rather than a separator with a flag on it, because what it is drawn in
    /// is the whole of the difference.
    Unread,
    /// An image or a file card, whose size the server told us.
    Attachment,
    /// One line of a link or permalink card. A card is several of these
    /// stacked with no gap, so the bar drawn beside them reads as one.
    Preview,
    /// One line of a webhook's attachment -- a Jira notice, a build result.
    /// Stacked the same way, on its own ground with a bar down its left.
    Attached,
    Footer,
    Separator,
}

/// What pressing a run of words does.
///
/// Three things in a message point somewhere: a link, a person, and a
/// conversation. They are one idea rather than three, because what they share
/// is the part that is hard -- knowing where the words ended up -- and only
/// what happens afterwards differs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Press {
    /// A web address. Never any other scheme: a message is text somebody else
    /// wrote, and a markdown href can name anything the shell will run.
    Link(String),
    /// Somebody, by username.
    Person(String),
    /// A conversation, by its name rather than its id -- which is all a
    /// `~channel` in a message carries.
    Channel(String),
    /// One message, where it was said.
    ///
    /// What a quoted permalink card leads to. By ids rather than by its URL:
    /// the message is on this server and in the local store, so following it
    /// is a scroll rather than a browser.
    Post { channel_id: String, post_id: String },
}

/// A piece of text with the styling that changes its width.
///
/// Carried out of the layout rather than consumed by it: a renderer has to draw
/// exactly what was measured, and re-deriving the spans from the markdown is how
/// the two drift apart.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextSpan {
    pub text: String,
    pub bold: bool,
    pub italic: bool,
    pub mono: bool,
    /// What pressing these words does, when they are more than words. Carried
    /// out of the layout for the same reason the styling is: only the shaping
    /// knows where they landed, and re-deriving that from the markdown is how
    /// a hit box and the text under it come to disagree.
    pub press: Option<Press>,
    /// Drawn in the quieter ink: a timestamp, a reaction's count, anything the
    /// eye should pass over on its way to the message.
    pub faint: bool,
    /// A custom emoji standing in this span, by name.
    ///
    /// It has no character to shape, so the span holds non-breaking spaces and
    /// the renderer draws the picture over exactly the room they took.
    /// Reserving it in the text is what keeps the line count right, which is
    /// what keeps the row height right.
    pub emoji: Option<String>,
}

/// How many spaces a custom emoji reserves in a line.
///
/// Four measure close to the sixteen pixels the picture is drawn at, at the
/// sizes this app uses. Close rather than exact on purpose: the picture is
/// drawn to whatever they actually measured, so the reserved space and the
/// image agree however the font shapes them.
const EMOJI_ROOM: usize = 4;

/// One of them. Non-breaking, or a wrap could split the placeholder in half and
/// leave the picture straddling two lines.
const NBSP: &str = "\u{00A0}";

/// One paragraph-like run of inline content, and how far it is pushed in.
struct Line {
    /// Inside a blockquote, which is drawn with a bar rather than only an
    /// indent.
    quoted: bool,
    spans: Vec<TextSpan>,
    indent: f32,
    heading: bool,
    /// The bullet or number in front of it, for the first line of a list item.
    /// Empty for everything else.
    marker: String,
}

/// What goes in front of a list item, by how deep the list is.
///
/// The shapes the official client uses, and they have to differ by depth: a
/// sub-list drawn with the same dot as its parent is a sub-list nobody can see
/// is one.
fn marker_for(ordered: bool, at: usize, depth: f32) -> String {
    if ordered {
        return format!("{}.", at + 1);
    }
    match depth as usize {
        0 => "\u{2022}".to_string(),
        1 => "\u{25e6}".to_string(),
        _ => "\u{25aa}".to_string(),
    }
}

/// Ends the run of inline content being gathered, if there is one.
fn flush(pending: &mut Vec<TextSpan>, indent: f32, into: &mut Vec<Line>) {
    if pending.is_empty() {
        return;
    }
    into.push(Line {
        quoted: false,
        spans: std::mem::take(pending),
        indent,
        heading: false,
        marker: String::new(),
    });
}

/// Flattens the markdown into the blocks a reader sees stacked.
/// Flattens the markdown into the blocks a reader sees stacked.
///
/// Inline content is *gathered* rather than taken one node at a time. A
/// paragraph arrives wrapped and a list item does not: its children are the
/// text, the mention, the space between two mentions, the closing bracket. One
/// line per node put every `@name` on a line of its own with a blank line after
/// it, which is what a real message from this server looked like.
fn lines_of(nodes: &[Node], indent: f32, into: &mut Vec<Line>, code: &mut Vec<String>) {
    let mut pending: Vec<TextSpan> = Vec::new();
    for node in nodes {
        match node {
            Node::Paragraph { children } => {
                flush(&mut pending, indent, into);
                let mut spans = Vec::new();
                inline(children, false, false, false, None, &mut spans);
                into.push(Line {
                    quoted: false,
                    spans,
                    indent,
                    heading: false,
                    marker: String::new(),
                });
            }
            Node::Heading { children, .. } => {
                flush(&mut pending, indent, into);
                let mut spans = Vec::new();
                inline(children, true, false, false, None, &mut spans);
                into.push(Line {
                    quoted: false,
                    spans,
                    indent,
                    heading: true,
                    marker: String::new(),
                });
            }
            // A quote is pushed in like a list item, and marked so whoever
            // draws it can put a bar down its left and set it in the softer
            // ink: `blockquote { border-left: 3px solid var(--rule); color:
            // var(--ink-soft) }`.
            Node::Blockquote { children } => {
                flush(&mut pending, indent, into);
                let from = into.len();
                lines_of(children, indent + 1.0, into, code);
                for line in into.iter_mut().skip(from) {
                    line.quoted = true;
                }
            }
            Node::List { ordered, items } => {
                flush(&mut pending, indent, into);
                for (at, item) in items.iter().enumerate() {
                    let from = into.len();
                    lines_of(item, indent + 1.0, into, code);
                    // On the first line of the item only: the rest of a
                    // wrapped item hangs under the words, not under the dot.
                    if let Some(first) = into.get_mut(from) {
                        first.marker = marker_for(*ordered, at, indent);
                    }
                }
            }
            Node::CodeBlock { value, .. } => {
                flush(&mut pending, indent, into);
                code.push(value.clone())
            }
            Node::Table { head, rows } => {
                flush(&mut pending, indent, into);
                // Not laid out as a table yet: each cell is a line, which is
                // the same *number* of lines a stacked fallback draws.
                for cell in head {
                    lines_of(cell, indent, into, code);
                }
                for row in rows {
                    for cell in row {
                        lines_of(cell, indent, into, code);
                    }
                }
            }
            Node::Rule => {
                flush(&mut pending, indent, into);
                into.push(Line {
                    quoted: false,
                    spans: Vec::new(),
                    indent,
                    heading: false,
                    marker: String::new(),
                })
            }
            // Inline: gathered with whatever came before it, and ended by the
            // next thing that is not.
            other => inline(
                std::slice::from_ref(other),
                false,
                false,
                false,
                None,
                &mut pending,
            ),
        }
    }
    flush(&mut pending, indent, into);
}

/// The href, if it is one this window would follow.
///
/// A message is text somebody else wrote, and a markdown link can name any
/// scheme: `file:` reaching into the reader's disk, or anything the shell has
/// been taught to run. Only the two the web uses are carried; everything else
/// draws as words and does nothing when pressed.
/// The same question from outside this module: a preview card carries a URL
/// the server fetched, and it deserves the scheme check a link in a message
/// gets.
pub fn openable_link(href: &str) -> Option<Press> {
    openable(href)
}

fn openable(href: &str) -> Option<Press> {
    let scheme = href.split_once("://")?.0;
    matches!(scheme.to_ascii_lowercase().as_str(), "http" | "https")
        .then(|| Press::Link(href.to_string()))
}

/// Collects inline content into styled spans.
///
/// `press` is what the words are inside, threaded down like the styling is: a
/// link's text can be emphasised, code, or an emoji, and every piece of it
/// points at the same place.
fn inline(
    nodes: &[Node],
    bold: bool,
    italic: bool,
    mono: bool,
    press: Option<&Press>,
    into: &mut Vec<TextSpan>,
) {
    for node in nodes {
        match node {
            Node::Text { value } => into.push(TextSpan {
                text: value.clone(),
                bold,
                italic,
                mono,
                press: press.cloned(),
                faint: false,
                emoji: None,
            }),
            Node::Strong { children } => inline(children, true, italic, mono, press, into),
            Node::Emphasis { children } | Node::Strike { children } => {
                inline(children, bold, true, mono, press, into)
            }
            // Only what is worth opening. A message is untrusted text and a
            // href in it can name any scheme at all; anything but the web is
            // drawn as words and does nothing when pressed.
            Node::Link { children, href } => {
                inline(children, bold, italic, mono, openable(href).as_ref(), into)
            }
            Node::InlineCode { value } | Node::InlineMath { value } => into.push(TextSpan {
                text: value.clone(),
                bold,
                italic,
                mono: true,
                press: press.cloned(),
                faint: false,
                emoji: None,
            }),
            // A mention and a channel link point at somebody and somewhere
            // whatever they are nested inside, so they name their own press
            // rather than inheriting the surrounding one.
            Node::UserMention { username, .. } => into.push(TextSpan {
                text: format!("@{username}"),
                bold,
                italic,
                mono,
                press: Some(Press::Person(username.clone())),
                faint: false,
                emoji: None,
            }),
            Node::ChannelLink { name } => into.push(TextSpan {
                text: format!("~{name}"),
                bold,
                italic,
                mono,
                press: Some(Press::Channel(name.clone())),
                faint: false,
                emoji: None,
            }),
            // A standard emoji is a character and shapes like any other letter.
            // A custom one has no character at all, so the span holds spaces
            // wide enough for the picture and carries the name for whoever
            // draws it. Non-breaking, or a wrap could split the placeholder in
            // half and the picture would land across two lines.
            Node::Emoji { name, unicode } => into.push(match unicode {
                Some(character) => TextSpan {
                    text: character.clone(),
                    bold,
                    italic,
                    mono,
                    press: press.cloned(),
                    faint: false,
                    emoji: None,
                },
                None => TextSpan {
                    text: NBSP.repeat(EMOJI_ROOM),
                    bold,
                    italic,
                    mono,
                    press: press.cloned(),
                    faint: false,
                    emoji: Some(name.clone()),
                },
            }),
            Node::SoftBreak | Node::HardBreak => into.push(TextSpan {
                text: "\n".to_string(),
                bold,
                italic,
                mono,
                press: press.cloned(),
                faint: false,
                emoji: None,
            }),
            Node::Image { alt, .. } => into.push(TextSpan {
                text: alt.clone(),
                bold,
                italic,
                mono,
                press: press.cloned(),
                faint: false,
                emoji: None,
            }),
            _ => {}
        }
    }
}

fn attrs_for(bold: bool, italic: bool, mono: bool) -> Attrs<'static> {
    let mut attrs = Attrs::new();
    if mono {
        attrs = attrs.family(crate::mono_family());
    }
    if bold {
        attrs = attrs.weight(Weight::BOLD);
    }
    if italic {
        attrs = attrs.style(cosmic_text::Style::Italic);
    }
    attrs
}

/// Lays out one paragraph of mixed styles and answers how many lines it takes.
fn line_count(fonts: &mut Fonts, line: &Line, width: f32, theme: &Theme) -> usize {
    if line.spans.is_empty() {
        return 1;
    }
    let size = theme.body_size;
    let metrics = Metrics::new(size, theme.line_height);
    let mut buffer = Buffer::new(&mut fonts.system, metrics);
    let mut buffer = buffer.borrow_with(&mut fonts.system);
    buffer.set_size(Some(width), None);
    let spans: Vec<(&str, Attrs<'static>)> = line
        .spans
        .iter()
        .map(|span| {
            (
                span.text.as_str(),
                attrs_for(span.bold || line.heading, span.italic, span.mono),
            )
        })
        .collect();
    // Rich text rather than one flat string: bold is wider than regular, and a
    // paragraph measured entirely in one weight wraps in the wrong place.
    buffer.set_rich_text(spans, &Attrs::new(), Shaping::Advanced, None);
    buffer.shape_until_scroll(false);
    buffer.layout_runs().count().max(1)
}

/// Lays out a row and reports its exact height.
pub fn lay_out(fonts: &mut Fonts, row: &Row, theme: &Theme) -> RowLayout {
    let mut blocks = Vec::new();
    let mut y = 0.0_f32;

    /// A message's own time, on the reader's clock.
    ///
    /// Hours and minutes only. A date belongs to the separator above the run,
    /// which is why one exists.
    fn clock(at: i64, offset_minutes: i32) -> String {
        let local = at / 1000 + i64::from(offset_minutes) * 60;
        let day = local.rem_euclid(86_400);
        format!("{:02}:{:02}", day / 3600, (day % 3600) / 60)
    }

    fn plain(text: String) -> TextSpan {
        TextSpan {
            text,
            bold: false,
            italic: false,
            mono: false,
            press: None,
            faint: false,
            emoji: None,
        }
    }

    let (nodes, post, header) = match row {
        Row::DateSeparator { epoch_day } => {
            return RowLayout {
                height: theme.separator_height,
                blocks: vec![Block {
                    y: 0.0,
                    x: 0.0,
                    height: theme.separator_height,
                    lines: 1,
                    kind: Kind::Separator,
                    // The words go in the block, so both shells draw the same
                    // line rather than each deciding what a day is called.
                    spans: vec![plain(spaced(&day_name(*epoch_day, theme.today)))],
                    size: theme.small_size,
                    wrap: theme.text_width(),
                }],
            };
        }
        Row::UnreadDivider => {
            return RowLayout {
                height: theme.separator_height,
                blocks: vec![Block {
                    y: 0.0,
                    x: 0.0,
                    height: theme.separator_height,
                    lines: 1,
                    kind: Kind::Unread,
                    spans: vec![plain(spaced("New messages"))],
                    size: theme.small_size,
                    wrap: theme.text_width(),
                }],
            };
        }
        Row::ThreadFooter { .. } => {
            return RowLayout {
                height: theme.footer_height,
                blocks: vec![Block {
                    y: 0.0,
                    x: 0.0,
                    height: theme.footer_height,
                    lines: 1,
                    kind: Kind::Footer,
                    spans: Vec::new(),
                    size: theme.body_size,
                    wrap: theme.text_width(),
                }],
            };
        }
        // `.system { font-size: 12.5px; color: var(--ink-faint); font-style:
        // italic }`, and the sentence itself, which used to be missing.
        //
        // The row reserved its height and carried no words, so a channel whose
        // recent history is all joins and leaves drew as a column of date
        // separators with nothing under them -- which reads as messages having
        // gone missing rather than as nobody having said anything since 2024.
        Row::System { .. } | Row::DeletedRoot { .. } => {
            let text = match row {
                Row::System { text, .. } => text.clone(),
                // The app's own words for a root that is gone but still
                // anchors its replies.
                _ => "Message deleted — its replies remain.".to_string(),
            };
            return RowLayout {
                height: theme.line_height + theme.row_padding * 2.0,
                blocks: vec![Block {
                    y: theme.row_padding,
                    x: 0.0,
                    height: theme.line_height,
                    lines: 1,
                    kind: Kind::Text,
                    spans: vec![TextSpan {
                        italic: true,
                        faint: true,
                        ..plain(text)
                    }],
                    size: theme.system_size,
                    wrap: theme.text_width(),
                }],
            };
        }
        Row::Post { post } => (post.nodes.as_slice(), Some(post), true),
        Row::Continuation { post } => (post.nodes.as_slice(), Some(post), false),
    };

    let attachments = post.map(|post| post.files.len()).unwrap_or(0);

    y += theme.row_padding;
    if header {
        // Who and when, as text rather than a reserved rectangle: it is a line
        // like any other and it is what tells one message from the next.
        let mut spans = Vec::new();
        if let Some(post) = post {
            spans.push(TextSpan {
                text: post.author_name.clone(),
                bold: true,
                italic: false,
                mono: false,
                press: None,
                faint: false,
                emoji: None,
            });
            spans.push(plain("   ".to_string()));
            spans.push(TextSpan {
                text: clock(post.create_at, theme.utc_offset_minutes),
                bold: false,
                italic: false,
                mono: false,
                press: None,
                faint: true,
                emoji: None,
            });
            if post.edited {
                spans.push(TextSpan {
                    text: "  edited".to_string(),
                    bold: false,
                    italic: false,
                    mono: false,
                    press: None,
                    faint: true,
                    emoji: None,
                });
            }
        }
        blocks.push(Block {
            y,
            x: 0.0,
            height: theme.header_height,
            lines: 1,
            kind: Kind::Header,
            spans,
            size: theme.header_size,
            wrap: theme.text_width(),
        });
        y += theme.header_height;
    }

    let mut lines = Vec::new();
    let mut code = Vec::new();
    lines_of(nodes, 0.0, &mut lines, &mut code);

    for line in lines {
        let x = line.indent * theme.indent;
        let wrap = theme.text_width() - x;
        let count = line_count(fonts, &line, wrap, theme);
        let height = count as f32 * theme.line_height;
        // In the room the indent already made, at the same height as the first
        // line of the item -- so the words wrap under themselves rather than
        // under the dot, which is what a hanging indent is.
        if !line.marker.is_empty() {
            blocks.push(Block {
                y,
                x: (x - theme.indent).max(0.0),
                height: theme.line_height,
                lines: 1,
                kind: Kind::Text,
                spans: vec![plain(line.marker.clone())],
                size: theme.body_size,
                wrap: theme.indent,
            });
        }
        // A heading is not bigger, only heavier: `.heading { font-weight:
        // 600 }` and nothing about size. Scaling it by 1.15 made every `#` in
        // a message louder than the app draws it.
        let size = theme.body_size;
        blocks.push(Block {
            y,
            x,
            height,
            lines: count,
            kind: if line.quoted { Kind::Quote } else { Kind::Text },
            spans: line.spans,
            size,
            wrap,
        });
        y += height + theme.block_gap;
    }

    for block in &code {
        // Measured at the width it will actually be shaped at, rather than
        // counting the lines as typed. Those two disagreed: the painter wrapped
        // at the column while this counted newlines, so one long log line
        // reserved a single line's height and drew over the message below it.
        //
        // The app scrolls a code block sideways rather than wrapping it, and
        // this will too once the renderer can scroll sideways. That is a change
        // to `wrap` alone -- the count stays right, because it asks what will be
        // drawn rather than what was written.
        let wrap = (theme.text_width() - theme.code_padding * 2.0).max(40.0);
        let count = crate::extent_of(
            fonts,
            block,
            wrap,
            crate::Style {
                size: theme.code_size,
                line_height: theme.code_line_height,
                bold: false,
                italic: false,
                mono: true,
            },
        )
        .lines;
        let height = count as f32 * theme.code_line_height + theme.code_padding;
        blocks.push(Block {
            y,
            x: 0.0,
            height,
            lines: count,
            kind: Kind::Code,
            spans: vec![TextSpan {
                text: block.clone(),
                bold: false,
                italic: false,
                mono: true,
                press: None,
                faint: false,
                emoji: None,
            }],
            size: theme.code_size,
            wrap,
        });
        y += height + theme.block_gap;
    }

    if attachments > 0 {
        // The server gives every attachment its drawn size, so this is the one
        // height that was never in doubt: `box_width` and `box_height` are what
        // the planner already worked out, and reserving exactly them means a
        // picture arriving moves nothing.
        //
        // One block per file, stacked. Laying them side by side is a gallery,
        // which is a later decision about arrangement rather than about height.
        for file in post.map(|post| post.files.as_slice()).unwrap_or(&[]) {
            let height = if file.image || file.video {
                file.box_height.max(0) as f32
            } else {
                // A file card: a fixed row with a name and a size on it.
                theme.card_height
            };
            blocks.push(Block {
                y,
                x: 0.0,
                height,
                lines: 0,
                kind: Kind::Attachment,
                spans: Vec::new(),
                size: theme.body_size,
                wrap: (file.box_width.max(0) as f32).min(theme.text_width()),
            });
            y += height + theme.block_gap;
        }
    }

    // What a webhook sent, which for a great many posts here *is* the message:
    // a bot puts its whole payload in `attachments` and leaves the body empty,
    // so a post with none of this drawn is a post with nothing on it at all.
    for attached in post.map(|post| post.attachments.as_slice()).unwrap_or(&[]) {
        let x = theme.quote_bar + theme.indent / 2.0;
        let wrap = (theme.text_width() - x - theme.indent / 2.0).max(40.0);
        let mut push = |text: &str, bold: bool, faint: bool, fonts: &mut Fonts| {
            if text.trim().is_empty() {
                return;
            }
            let style = crate::Style {
                size: theme.attached_size,
                line_height: theme.line_height,
                bold,
                italic: false,
                mono: false,
            };
            let count = crate::extent_of(fonts, text, wrap, style).lines.max(1);
            blocks.push(Block {
                y,
                x,
                height: count as f32 * theme.line_height,
                lines: count,
                kind: Kind::Attached,
                spans: vec![TextSpan {
                    text: text.to_string(),
                    bold,
                    italic: false,
                    mono: false,
                    faint,
                    press: None,
                    emoji: None,
                }],
                size: theme.attached_size,
                wrap,
            });
            y += count as f32 * theme.line_height;
        };
        let plain_of = matterless_render::markdown::plain_text;
        push(&plain_of(&attached.pretext), false, true, fonts);
        push(
            attached.title.as_deref().unwrap_or_default(),
            true,
            false,
            fonts,
        );
        push(&plain_of(&attached.text), false, false, fonts);
        // A field is a name and a value, and the app sets them as a pair on
        // one line rather than as a table this renderer has no grid for.
        for field in &attached.fields {
            push(
                &format!("{}: {}", field.title, plain_of(&field.value)),
                false,
                false,
                fonts,
            );
        }
        y += theme.block_gap;
    }

    // A link or permalink card, as a stack of lines rather than one block: a
    // card mixes sizes -- a small site name over a bold title over a quieter
    // description -- and one block carries one size. Stacked with no gap
    // between them, so the bar drawn beside the run reads as a single card.
    for preview in post.map(|post| post.previews.as_slice()).unwrap_or(&[]) {
        let x = theme.quote_bar + theme.indent / 2.0;
        // `max-width: 520px` on the card, less the padding inside it: a
        // preview that ran the full width of the column read as part of the
        // message rather than as something attached to it.
        let wrap = (theme.text_width() - x)
            .min(theme.preview_width - theme.card_padding * 2.0)
            .max(40.0);
        let mut push = |text: &str, bold: bool, faint: bool, cap: usize, fonts: &mut Fonts| {
            if text.is_empty() {
                return;
            }
            let style = crate::Style {
                size: theme.body_size,
                line_height: theme.line_height,
                bold,
                italic: false,
                mono: false,
            };
            let count = crate::extent_of(fonts, text, wrap, style)
                .lines
                .max(1)
                .min(cap);
            blocks.push(Block {
                y,
                x,
                height: count as f32 * theme.line_height,
                lines: count,
                kind: Kind::Preview,
                spans: vec![TextSpan {
                    text: text.to_string(),
                    bold,
                    italic: false,
                    mono: false,
                    faint,
                    press: None,
                    emoji: None,
                }],
                size: theme.body_size,
                wrap,
            });
            y += count as f32 * theme.line_height;
        };
        match preview {
            matterless_render::Preview::Page {
                title,
                description,
                site_name,
                ..
            } => {
                push(site_name, false, true, 1, fonts);
                push(title, true, false, 2, fonts);
                push(description, false, true, theme.preview_lines, fonts);
            }
            matterless_render::Preview::Permalink {
                channel_label,
                author_name,
                nodes,
                ..
            } => {
                push(
                    &format!("{author_name} in {channel_label}"),
                    true,
                    false,
                    1,
                    fonts,
                );
                // The quoted body as plain text: a card is a summary, and a
                // quote that re-renders headings and lists inside a message is
                // a second message.
                let quoted = matterless_render::markdown::plain_text(nodes);
                push(&quoted, false, true, theme.preview_lines, fonts);
            }
        }
        y += theme.block_gap;
    }

    // One block per reaction rather than one line of them all, because each is
    // a thing to click: a pill nobody can press is a picture of a feature. The
    // blocks come out in the post's own reaction order, so the caller pairs the
    // nth with the nth reaction exactly as it does for attachments.
    if let Some(post) = post.filter(|post| !post.reactions.is_empty()) {
        let mut x = 0.0;
        for reaction in &post.reactions {
            // A standard emoji is a character and shapes like any other. A
            // custom one is an image behind the session token: it has no
            // character at all, so the pill reserves a square for it and the
            // renderer puts the picture there. Printing the name instead is
            // what made a reaction read ":bongo: 1".
            let custom = reaction.unicode.is_none();
            let label = match &reaction.unicode {
                Some(face) => format!("{face} {}", reaction.count),
                None => reaction.count.to_string(),
            };
            let width = crate::extent_of(
                fonts,
                &label,
                f32::MAX,
                crate::Style {
                    size: theme.body_size,
                    line_height: theme.reaction_height,
                    bold: false,
                    italic: false,
                    mono: false,
                },
            )
            .width
                + theme.pill_padding * 2.0
                + if custom { theme.emoji_size + 4.0 } else { 0.0 };
            // Wrapped by hand: a row of pills is a row of boxes, not a run of
            // text, so nothing else is going to wrap it.
            if x > 0.0 && x + width > theme.text_width() {
                x = 0.0;
                y += theme.reaction_height;
            }
            blocks.push(Block {
                y,
                x,
                height: theme.reaction_height,
                lines: 1,
                kind: Kind::Reactions,
                spans: vec![plain(label)],
                size: theme.body_size,
                // The pill's own width, which is what it is drawn and hit as.
                wrap: width,
            });
            x += width + theme.pill_gap;
        }
        y += theme.reaction_height;
    }

    // A message that has not been confirmed reads quieter than one that has,
    // which is the whole of what "pending" means to a reader: it is there, and
    // it is not certain yet. Done by marking the spans rather than by a state
    // the renderer would have to know about, so it draws through the same path
    // every other quiet thing does.
    if post.is_some_and(|post| post.pending) {
        for block in &mut blocks {
            for span in &mut block.spans {
                span.faint = true;
            }
        }
    }

    // A send that failed says so, in a line of its own that takes real height.
    // Leaving it to a colour would say nothing on a row already drawn faint,
    // and a message that silently looks delivered when it is not is the worst
    // of the available answers.
    if post.is_some_and(|post| post.failed) {
        blocks.push(Block {
            y,
            x: 0.0,
            height: theme.line_height,
            lines: 1,
            kind: Kind::Text,
            spans: vec![TextSpan {
                text: "Not sent. Click to try again.".to_string(),
                bold: false,
                italic: false,
                mono: false,
                press: None,
                faint: true,
                emoji: None,
            }],
            size: theme.body_size,
            wrap: theme.text_width(),
        });
        y += theme.line_height;
    }

    RowLayout {
        // The gap the stream's flex column puts between every pair. Owed by
        // each row rather than subtracted from the last, which would make the
        // last row a different height for no reason a reader could see.
        height: y + theme.row_padding + theme.row_gap,
        blocks,
    }
}

/// A separator's words, as the stylesheet sets them: upper case, with the
/// letters held apart.
///
/// A hair space between letters rather than a tracking value, because the
/// shaper is handed a string and nothing else -- and a run of small upper-
/// case letters set tight is a smudge at this size.
fn spaced(words: &str) -> String {
    words
        .to_uppercase()
        .chars()
        .flat_map(|letter| [letter, HAIR])
        .collect::<String>()
        .trim_end()
        .to_string()
}

/// The thinnest space the shaper has, which is what `letter-spacing` on a
/// small upper-case run comes to.
const HAIR: char = ' ';

/// What a date separator says: "Tuesday 4 September", and the year when it is
/// not the one the reader is in.
///
/// The year appears once the date stops being unambiguous. Scrolling back far
/// enough that "Tuesday 4 September" could be any of several years is exactly
/// when it matters, and repeating it on every separator would be noise the
/// rest of the time.
fn day_name(epoch_day: i64, today: i64) -> String {
    let (year, month, day) = civil_from_days(epoch_day);
    // 1970-01-01 was a Thursday, which is where the offset of 4 comes from.
    let weekday = WEEKDAYS[(epoch_day + 4).rem_euclid(7) as usize];
    let month = MONTHS[(month - 1) as usize];
    let same_year = today != 0 && civil_from_days(today).0 == year;
    if same_year {
        format!("{weekday} {day} {month}")
    } else {
        format!("{weekday} {day} {month} {year}")
    }
}

const WEEKDAYS: [&str; 7] = [
    "Sunday",
    "Monday",
    "Tuesday",
    "Wednesday",
    "Thursday",
    "Friday",
    "Saturday",
];

const MONTHS: [&str; 12] = [
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
];

/// Days since 1970-01-01 to a proleptic Gregorian date.
///
/// Hinnant's algorithm, which shifts the epoch to 0000-03-01 so leap days fall
/// at the end of a year and the month lengths become a single linear formula.
/// Written out rather than pulled in: this is the only date arithmetic in the
/// whole window, and a calendar crate would be a dependency for one function.
fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let shifted = days + 719_468;
    let era = if shifted >= 0 {
        shifted
    } else {
        shifted - 146_096
    } / 146_097;
    let day_of_era = shifted - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let shifted_month = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * shifted_month + 2) / 5 + 1;
    // March is month 0 in the shifted calendar, so the last two wrap round.
    let month = if shifted_month < 10 {
        shifted_month + 3
    } else {
        shifted_month - 9
    };
    (if month <= 2 { year + 1 } else { year }, month, day)
}

#[cfg(test)]
mod tests {
    use super::*;
    use matterless_render::PostRow;
    use matterless_render::markdown::Node;
    use std::sync::Arc;

    fn post(nodes: Vec<Node>) -> PostRow {
        PostRow {
            post_id: "p1".into(),
            root_id: String::new(),
            author_id: "u1".into(),
            author_name: "ada".into(),
            create_at: 0,
            update_at: 0,
            edited: false,
            nodes: Arc::new(nodes),
            reactions: Vec::new(),
            files: Vec::new(),
            attachments: Vec::new(),
            avatar_at: 0,
            bot: false,
            body_is_attachment_only: false,
            pending: false,
            failed: false,
            pinned: false,
            saved: false,
            following: false,
            previews: Vec::new(),
        }
    }

    fn text(value: &str) -> Node {
        Node::Paragraph {
            children: vec![Node::Text {
                value: value.into(),
            }],
        }
    }

    /// The dates a calendar has to agree with, including the awkward ones.
    #[test]
    fn a_day_number_names_the_right_date() {
        // 1970-01-01 was a Thursday.
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(day_name(0, 0), "Thursday 1 January 1970");
        // A leap day, and the day after it.
        assert_eq!(civil_from_days(19_782), (2024, 2, 29));
        assert_eq!(civil_from_days(19_783), (2024, 3, 1));
        // 2000 is a leap year and 2100 is not, which is the whole of the
        // century rule: February runs straight into March there.
        assert_eq!(civil_from_days(11_016), (2000, 2, 29));
        assert_eq!(civil_from_days(47_540), (2100, 2, 28));
        assert_eq!(civil_from_days(47_541), (2100, 3, 1));
        // Before the epoch, which a page of old history reaches.
        assert_eq!(civil_from_days(-1), (1969, 12, 31));
    }

    /// The year appears once the date stops being unambiguous, and not before.
    #[test]
    fn the_year_is_left_off_only_inside_this_one() {
        let in_2024 = 19_782;
        let also_2024 = 19_800;
        assert_eq!(day_name(in_2024, also_2024), "Thursday 29 February");
        // A different year, so it has to say which.
        assert!(day_name(in_2024, 0).ends_with("2024"));
        assert!(day_name(in_2024, 11_016).ends_with("2024"));
    }

    /// What a run of words points at, and what it refuses to.
    ///
    /// A message is text somebody else wrote. The scheme check lives here as
    /// well as at the point of opening, so neither one alone can be changed
    /// into a way to reach `file:` on the reader's disk.
    #[test]
    fn only_the_web_a_person_and_a_conversation_are_pressable() {
        let pressed = |node: Node| {
            let mut spans = Vec::new();
            inline(&[node], false, false, false, None, &mut spans);
            spans.into_iter().find_map(|span| span.press)
        };
        let words = || {
            vec![Node::Text {
                value: "somewhere".into(),
            }]
        };

        assert_eq!(
            pressed(Node::Link {
                href: "https://example.invalid/x".into(),
                children: words(),
            }),
            Some(Press::Link("https://example.invalid/x".into()))
        );
        assert_eq!(
            pressed(Node::UserMention {
                username: "ada".into(),
                everyone: false,
            }),
            Some(Press::Person("ada".into()))
        );
        assert_eq!(
            pressed(Node::ChannelLink { name: "dev".into() }),
            Some(Press::Channel("dev".into()))
        );
        for refused in [
            "file:///c:/windows",
            "javascript:alert(1)",
            "ms-settings://x",
        ] {
            assert_eq!(
                pressed(Node::Link {
                    href: refused.into(),
                    children: words(),
                }),
                None,
                "{refused} should not be pressable"
            );
        }
    }

    /// A mention inside a link points at the person, not at the page: it is
    /// its own destination whatever it is nested in.
    #[test]
    fn a_mention_keeps_its_own_destination_inside_a_link() {
        let mut spans = Vec::new();
        inline(
            &[Node::Link {
                href: "https://example.invalid/x".into(),
                children: vec![
                    Node::Text {
                        value: "see ".into(),
                    },
                    Node::UserMention {
                        username: "ada".into(),
                        everyone: false,
                    },
                ],
            }],
            false,
            false,
            false,
            None,
            &mut spans,
        );
        assert_eq!(
            spans[0].press,
            Some(Press::Link("https://example.invalid/x".into()))
        );
        assert_eq!(spans[1].press, Some(Press::Person("ada".into())));
    }

    /// A card takes room, or the message under it is drawn over.
    ///
    /// Reserved by the layout rather than discovered by the painter, which is
    /// the contract every other block keeps: the renderer draws exactly what
    /// was measured.
    #[test]
    fn a_link_card_reserves_the_room_it_draws_in() {
        let mut fonts = Fonts::new();
        let theme = Theme::default();
        let bare = lay_out(
            &mut fonts,
            &Row::Post {
                post: post(vec![text("look")]),
            },
            &theme,
        );

        let mut carded = post(vec![text("look")]);
        carded.previews = vec![matterless_render::Preview::Page {
            url: "https://example.invalid/thing".into(),
            title: "A page with a title".into(),
            description: "And a description long enough to say something about it.".into(),
            site_name: "example.invalid".into(),
            image: None,
        }];
        let with = lay_out(&mut fonts, &Row::Post { post: carded }, &theme);

        assert!(
            with.height > bare.height,
            "a card added no height: {} vs {}",
            with.height,
            bare.height
        );
        let card: Vec<&Block> = with
            .blocks
            .iter()
            .filter(|block| block.kind == Kind::Preview)
            .collect();
        // Site, title, description: three stacked lines, one card.
        assert_eq!(card.len(), 3);
        // Stacked with no gap, so the bar beside them reads as one.
        for pair in card.windows(2) {
            assert!(
                (pair[1].y - (pair[0].y + pair[0].height)).abs() < 0.5,
                "a gap opened between two lines of one card"
            );
        }
    }

    /// A description of a thousand words is not a summary. Past the cap the
    /// card would be the page.
    #[test]
    fn a_long_description_is_cut_rather_than_drawn_whole() {
        let mut fonts = Fonts::new();
        let theme = Theme::default();
        let mut carded = post(vec![text("look")]);
        carded.previews = vec![matterless_render::Preview::Page {
            url: "https://example.invalid/thing".into(),
            title: "Title".into(),
            description: "word ".repeat(500),
            site_name: "example.invalid".into(),
            image: None,
        }];
        let laid = lay_out(&mut fonts, &Row::Post { post: carded }, &theme);
        let longest = laid
            .blocks
            .iter()
            .filter(|block| block.kind == Kind::Preview)
            .map(|block| block.lines)
            .max()
            .expect("a card");
        assert!(
            longest <= theme.preview_lines,
            "{longest} lines is not a summary"
        );
    }

    /// The whole point: a row's height is known, and it is different at two
    /// widths, without anything having been drawn.
    #[test]
    fn a_row_is_taller_in_a_narrower_list() {
        let mut fonts = Fonts::new();
        let row = Row::Post {
            post: post(vec![text(
                "I would like to raise this limit on swarm please, to fifty or perhaps a hundred, and it appears to live in the configuration file under the data directory. Do you have access to that in the tools team, or is it more a question for the people who look after the servers? Either way suits me, but the current limit stops the review expanding at all.",
            )]),
        };
        let wide = lay_out(&mut fonts, &row, &Theme::default());
        let narrow = lay_out(
            &mut fonts,
            &row,
            &Theme {
                width: 640.0,
                ..Theme::default()
            },
        );
        assert!(
            narrow.height > wide.height,
            "narrower wraps more: {} vs {}",
            narrow.height,
            wide.height
        );
    }

    /// A continuation omits the author line, and that is the only difference.
    #[test]
    fn a_continuation_is_shorter_by_its_header() {
        let mut fonts = Fonts::new();
        let theme = Theme::default();
        let body = vec![text("Yo!")];
        let first = lay_out(
            &mut fonts,
            &Row::Post {
                post: post(body.clone()),
            },
            &theme,
        );
        let next = lay_out(&mut fonts, &Row::Continuation { post: post(body) }, &theme);
        assert_eq!(first.height - next.height, theme.header_height);
    }

    /// Short lines do not wrap, so the count is the lines as written.
    #[test]
    fn a_code_block_counts_its_own_lines() {
        let mut fonts = Fonts::new();
        let theme = Theme::default();
        let row = Row::Post {
            post: post(vec![Node::CodeBlock {
                language: None,
                value: "one\ntwo\nthree".into(),
            }]),
        };
        let laid = lay_out(&mut fonts, &row, &theme);
        let code = laid
            .blocks
            .iter()
            .find(|block| block.kind == Kind::Code)
            .expect("a code block");
        assert_eq!(code.lines, 3);
        assert_eq!(
            code.height,
            3.0 * theme.code_line_height + theme.code_padding
        );
    }

    /// A pasted build error is one line as typed and several as drawn. Counting
    /// what was written reserved a single line, and the message below it was
    /// drawn over the top.
    #[test]
    fn a_long_code_line_reserves_the_height_it_will_be_drawn_at() {
        let mut fonts = Fonts::new();
        let theme = Theme::default();
        let long = "UATHelper: Packaging (Windows): Module.Curiosity.27.cpp.obj : error \
                    LNK2019: unresolved external symbol \"protected: void __cdecl \
                    U3DFlowManager::ProcessPendingSpawningRequest(class FSCBudgetedWork &)\" \
                    referenced in function \"public: void __cdecl U3DFlowManager::Initialize\"";
        let row = Row::Post {
            post: post(vec![Node::CodeBlock {
                language: None,
                value: long.into(),
            }]),
        };
        let laid = lay_out(&mut fonts, &row, &theme);
        let code = laid
            .blocks
            .iter()
            .find(|block| block.kind == Kind::Code)
            .expect("a code block");

        assert_eq!(long.lines().count(), 1, "one line as typed");
        assert!(
            code.lines > 1,
            "a line far wider than the column has to wrap"
        );
        assert_eq!(
            code.height,
            code.lines as f32 * theme.code_line_height + theme.code_padding
        );
        // The block has to fit inside the row it is part of, or it draws over
        // whatever comes next.
        assert!(laid.height >= code.y + code.height);
    }

    /// The layout and the painter have to shape at the same width, or the count
    /// is right about a wrap that never happens.
    #[test]
    fn a_code_block_is_shaped_inside_its_own_padding() {
        let mut fonts = Fonts::new();
        let theme = Theme::default();
        let row = Row::Post {
            post: post(vec![Node::CodeBlock {
                language: None,
                value: "one".into(),
            }]),
        };
        let laid = lay_out(&mut fonts, &row, &theme);
        let code = laid
            .blocks
            .iter()
            .find(|block| block.kind == Kind::Code)
            .expect("a code block");
        // Both sides: the stylesheet pads a fence `10px 12px`, and shaping to
        // a width that only accounted for one of them would run the last
        // letter of a full line out past its own background.
        assert_eq!(code.wrap, theme.text_width() - theme.code_padding * 2.0);
    }

    /// A list item is a run of inline content, not one block per node.
    ///
    /// Unlike a paragraph, a list item's children arrive unwrapped: the text,
    /// the mention, the space between two mentions, the closing bracket. Each
    /// one becoming a line of its own is what put every `@name` on a line by
    /// itself with a blank line after it, against a real message from this
    /// server.
    #[test]
    fn a_list_item_is_one_line_however_many_pieces_it_is_made_of() {
        let mut fonts = Fonts::new();
        let theme = Theme::default();
        let item = vec![
            Node::Text {
                value: "decide with the LDs where it appears (poke ".into(),
            },
            Node::UserMention {
                username: "remi.gallet".into(),
                everyone: false,
            },
            Node::Text { value: " ".into() },
            Node::UserMention {
                username: "thibaut.machin".into(),
                everyone: false,
            },
            Node::Text { value: ")".into() },
        ];
        let laid = lay_out(
            &mut fonts,
            &Row::Post {
                post: post(vec![Node::List {
                    ordered: false,
                    items: vec![item],
                }]),
            },
            &theme,
        );
        // Two text blocks: the bullet, in the gutter, and the item itself.
        let text: Vec<&Block> = laid
            .blocks
            .iter()
            .filter(|block| block.kind == Kind::Text)
            .collect();
        assert_eq!(text.len(), 2, "the bullet and one line");
        let bullet = text[0];
        let text = &text[1..];
        assert_eq!(bullet.spans.len(), 1);
        assert_eq!(
            bullet.spans[0].text, "\u{2022}",
            "an unordered list gets a dot"
        );
        // Beside the words, not above them, and to their left.
        assert_eq!(bullet.y, text[0].y, "the dot sits on the first line");
        assert!(bullet.x < text[0].x, "the dot is in the gutter");
        // And all five pieces are on it, in order, the mentions still their own
        // spans so they stay pressable.
        let said: String = text[0]
            .spans
            .iter()
            .map(|span| span.text.as_str())
            .collect();
        assert!(said.contains("poke"), "{said:?}");
        assert!(said.contains("remi.gallet"), "{said:?}");
        assert!(said.contains("thibaut.machin"), "{said:?}");
        assert!(said.ends_with(')'), "{said:?}");
        assert!(
            text[0]
                .spans
                .iter()
                .filter(|span| span.press.is_some())
                .count()
                >= 2,
            "the mentions stopped being pressable"
        );
    }

    /// Bold is wider, so the same words can need another line -- which a
    /// paragraph measured in one weight would miss.
    #[test]
    fn mixed_weight_is_measured_per_span() {
        let mut fonts = Fonts::new();
        let theme = Theme {
            width: 260.0,
            ..Theme::default()
        };
        let plain = lay_out(
            &mut fonts,
            &Row::Post {
                post: post(vec![text("regular words that nearly fill the line here")]),
            },
            &theme,
        );
        let strong = lay_out(
            &mut fonts,
            &Row::Post {
                post: post(vec![Node::Paragraph {
                    children: vec![Node::Strong {
                        children: vec![Node::Text {
                            value: "regular words that nearly fill the line here".into(),
                        }],
                    }],
                }]),
            },
            &theme,
        );
        assert!(
            strong.height >= plain.height,
            "bold cannot be shorter: {} vs {}",
            strong.height,
            plain.height
        );
    }

    #[test]
    fn a_separator_is_a_fixed_height() {
        let mut fonts = Fonts::new();
        let theme = Theme::default();
        let laid = lay_out(&mut fonts, &Row::DateSeparator { epoch_day: 20137 }, &theme);
        assert_eq!(laid.height, theme.separator_height);
        assert_eq!(laid.blocks.len(), 1);
    }

    /// A system row says what happened, rather than reserving room for silence.
    ///
    /// It used to be laid out with no spans at all: the height was right and
    /// the words were missing, so a channel whose last two years are joins and
    /// leaves drew as a stack of date separators with gaps under them. The
    /// sentence is already built for the shell -- this only has to draw it.
    #[test]
    fn a_system_row_carries_the_sentence_it_was_given() {
        let mut fonts = Fonts::new();
        let theme = Theme::default();
        let laid = lay_out(
            &mut fonts,
            &Row::System {
                post_id: "p1".into(),
                post_type: "system_join_channel".into(),
                nodes: Vec::new(),
                text: "ada joined the channel".into(),
            },
            &theme,
        );
        let span = laid.blocks[0].spans.first().expect("words on the row");
        assert_eq!(span.text, "ada joined the channel");
        // `.system { color: var(--ink-faint); font-style: italic }`.
        assert!(span.faint && span.italic, "set apart from what people said");
        assert_eq!(laid.blocks[0].size, theme.system_size);

        // And a deleted root says why it is still there.
        let gone = lay_out(
            &mut fonts,
            &Row::DeletedRoot {
                post_id: "p2".into(),
            },
            &theme,
        );
        assert_eq!(
            gone.blocks[0].spans.first().map(|span| span.text.as_str()),
            Some("Message deleted \u{2014} its replies remain.")
        );
    }

    fn picture(id: &str, width: i32, height: i32) -> matterless_render::FileRef {
        matterless_render::FileRef {
            id: id.into(),
            name: "shot.png".into(),
            extension: "png".into(),
            size: 1024,
            mime_type: "image/png".into(),
            width,
            height,
            image: true,
            video: false,
            variant: matterless_render::ImageVariant::Preview,
            mini_preview: None,
            box_width: width,
            box_height: height,
            archived: false,
        }
    }

    /// The height the server already told us, reserved before the bytes
    /// arrive. Without this an image appearing would push the conversation
    /// down under it -- the one thing this whole layout exists to prevent.
    #[test]
    fn an_attachment_reserves_the_box_the_server_gave_it() {
        let mut fonts = Fonts::new();
        let theme = Theme::default();
        let mut with = post(vec![]);
        with.files = vec![picture("f1", 300, 200)];
        let laid = lay_out(&mut fonts, &Row::Post { post: with }, &theme);
        let block = laid
            .blocks
            .iter()
            .find(|block| block.kind == Kind::Attachment)
            .expect("an attachment block");
        assert_eq!(block.height, 200.0);
        assert_eq!(block.wrap, 300.0);
        assert!(laid.height >= block.y + block.height);
    }

    /// Two pictures are two blocks, stacked, and the row is tall enough for
    /// both.
    #[test]
    fn every_attachment_gets_its_own_room() {
        let mut fonts = Fonts::new();
        let theme = Theme::default();
        let mut with = post(vec![]);
        with.files = vec![picture("f1", 300, 200), picture("f2", 120, 90)];
        let laid = lay_out(&mut fonts, &Row::Post { post: with }, &theme);
        let blocks: Vec<&Block> = laid
            .blocks
            .iter()
            .filter(|block| block.kind == Kind::Attachment)
            .collect();
        assert_eq!(blocks.len(), 2);
        assert_eq!(blocks[0].height, 200.0);
        assert_eq!(blocks[1].height, 90.0);
        assert!(blocks[1].y >= blocks[0].y + blocks[0].height);
        assert!(laid.height >= blocks[1].y + blocks[1].height);
    }

    /// A picture wider than the column is drawn no wider than the column.
    #[test]
    fn an_attachment_never_runs_past_the_column() {
        let mut fonts = Fonts::new();
        let theme = Theme::default();
        let mut with = post(vec![]);
        with.files = vec![picture("f1", 4000, 100)];
        let laid = lay_out(&mut fonts, &Row::Post { post: with }, &theme);
        let block = laid
            .blocks
            .iter()
            .find(|block| block.kind == Kind::Attachment)
            .expect("an attachment block");
        assert_eq!(block.wrap, theme.text_width());
    }

    /// Something that is not a picture is a card, and a card has a fixed row of
    /// its own rather than a zero-height nothing.
    #[test]
    fn a_file_that_is_not_a_picture_gets_a_card() {
        let mut fonts = Fonts::new();
        let theme = Theme::default();
        let mut with = post(vec![]);
        let mut document = picture("f1", 0, 0);
        document.image = false;
        document.name = "notes.pdf".into();
        with.files = vec![document];
        let laid = lay_out(&mut fonts, &Row::Post { post: with }, &theme);
        let block = laid
            .blocks
            .iter()
            .find(|block| block.kind == Kind::Attachment)
            .expect("an attachment block");
        assert_eq!(block.height, theme.card_height);
    }

    /// A message that has not been confirmed reads quieter than one that has.
    /// Without this an optimistic send looks exactly like a delivered one, and
    /// the reader has no way to tell a message that went from one that might
    /// not have.
    #[test]
    fn a_pending_message_is_drawn_quietly() {
        let mut fonts = Fonts::new();
        let theme = Theme::default();
        let mut guess = post(vec![Node::Paragraph {
            children: vec![Node::Text {
                value: "on its way".into(),
            }],
        }]);
        guess.pending = true;
        let laid = lay_out(&mut fonts, &Row::Post { post: guess }, &theme);
        let spans: Vec<&TextSpan> = laid.blocks.iter().flat_map(|block| &block.spans).collect();
        assert!(!spans.is_empty());
        assert!(
            spans.iter().all(|span| span.faint),
            "every span of a pending message is quiet"
        );
    }

    /// A send that failed says so in a line of its own, which takes real
    /// height: a message that silently looks delivered when it is not is the
    /// worst of the available answers.
    #[test]
    fn a_failed_message_says_so_and_takes_the_room_to() {
        let mut fonts = Fonts::new();
        let theme = Theme::default();
        let body = vec![Node::Paragraph {
            children: vec![Node::Text {
                value: "did not go".into(),
            }],
        }];
        let sent = lay_out(
            &mut fonts,
            &Row::Post {
                post: post(body.clone()),
            },
            &theme,
        );

        let mut broken = post(body);
        broken.failed = true;
        let laid = lay_out(&mut fonts, &Row::Post { post: broken }, &theme);

        assert!(
            laid.blocks
                .iter()
                .flat_map(|block| &block.spans)
                .any(|span| span.text.contains("Not sent")),
            "a failed message says it did not go"
        );
        assert_eq!(
            laid.height,
            sent.height + theme.line_height,
            "and the row is a line taller for saying it"
        );
    }
}
