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
    /// The same, for a message that carries on from the one above it.
    ///
    /// Less, because the two are one person still talking. A run of short
    /// messages -- "AH", "ouf", "I hadn't even read the one above" -- is a
    /// paragraph said in pieces, and at a full row's padding each piece stood
    /// eight pixels off the next while the lines *inside* one message stood
    /// at none. The eye reads that as five things rather than one turn.
    pub merged_padding: f32,
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
    /// A written rule's thickness: `hr { border-top: 1px }`. The air around it
    /// is `block_gap`, like any other pair of blocks.
    pub rule_height: f32,
    /// How many lines of a code block are shown before the rest is offered
    /// rather than drawn.
    ///
    /// A Sentry crash notice carries its stack trace in one, and drawn whole
    /// it is the window: the conversation around it cannot be read past it.
    /// Counted as the lines were written rather than as they wrap, so the
    /// height reserved is the height drawn whatever the column does.
    pub code_lines_shown: usize,
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
    /// The words on a pill: its count, and the face in front of one.
    ///
    /// Here rather than at each end because the width measured here is the
    /// width the pill is drawn and hit as. Measured at one size and drawn at
    /// another, a pill comes out wider than what is in it and the slack all
    /// falls on the right -- which reads as a pill whose contents are not
    /// centred, because they are not.
    pub pill_size: f32,
    /// The line a pill's words are set on, and so how tall they measure.
    pub pill_line: f32,
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
    /// The room inside a webhook's card, above its first line and below its
    /// last: `padding: 8px`.
    ///
    /// Reserved here rather than left to whoever draws the card. The painter
    /// used to reach eight pixels above the first line for it, which was room
    /// nothing had set aside -- and the thing directly above an attachment is
    /// the name of whoever sent it, so the card was drawn over the author and
    /// their avatar.
    pub attached_padding: f32,
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
            merged_padding: 0.0,
            row_gap: 2.0,
            pad_x: 16.0,
            pad_top: 12.0,
            pad_bottom: 4.0,
            block_gap: 6.0,
            rule_height: 1.0,
            code_lines_shown: 12,
            code_size: 12.5,
            code_line_height: 18.0,
            code_padding: 12.0,
            code_padding_y: 10.0,
            card_height: 56.0,
            pill_padding: 7.0,
            pill_gap: 5.0,
            pill_size: 13.0,
            pill_line: 18.0,
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
            attached_padding: 8.0,
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

    /// What a height measured against this theme may be reused for.
    ///
    /// Everything a row's height depends on except the width, which is a key
    /// of its own, and the clock, which changes a separator's words and not
    /// its height. A height kept under the wrong fingerprint is a wrong
    /// answer served confidently -- the emoji face changed on 2026-09-22 and
    /// moved the height of every row with one in it -- so the font stack is
    /// in here beside the numbers.
    ///
    /// Written out rather than derived from `Debug`: a field added to the
    /// theme should make somebody decide whether it changes a height, and a
    /// derived string decides silently.
    pub fn fingerprint(&self) -> String {
        let numbers = [
            self.body_size,
            self.line_height,
            self.header_height,
            self.header_size,
            self.row_padding,
            self.merged_padding,
            self.row_gap,
            self.pad_x,
            self.block_gap,
            self.rule_height,
            self.code_size,
            self.code_line_height,
            self.code_padding,
            self.code_padding_y,
            self.card_height,
            self.small_size,
            self.system_size,
            self.pill_size,
            self.pill_line,
            self.emoji_size,
            self.reaction_height,
            self.separator_height,
            self.footer_height,
            self.indent,
            self.preview_width,
            self.card_padding,
            self.attached_size,
            self.attached_padding,
            self.gutter,
        ];
        let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
        for number in numbers {
            for byte in number.to_bits().to_le_bytes() {
                hash ^= byte as u64;
                hash = hash.wrapping_mul(0x100_0000_01b3);
            }
        }
        for count in [self.code_lines_shown, self.preview_lines] {
            hash ^= count as u64;
            hash = hash.wrapping_mul(0x100_0000_01b3);
        }
        // The faces the shaper will actually use. A different emoji font is a
        // different set of advances and so a different set of heights -- the
        // bundled one changed on 2026-09-22 and moved every row carrying an
        // emoji.
        //
        // The body face is the machine's own and is not named here. A system
        // font update would change heights without changing this, which is
        // the one gap: it heals itself, because every row that is actually
        // shaped writes its height back, and it costs a scrollbar that is
        // slightly wrong until then rather than a wrong row.
        let mono = match crate::mono_family() {
            cosmic_text::Family::Name(name) => name,
            _ => "monospace",
        };
        for family in [crate::EMOJI_FAMILY, mono] {
            for byte in family.as_bytes() {
                hash ^= *byte as u64;
                hash = hash.wrapping_mul(0x100_0000_01b3);
            }
        }
        format!("{hash:016x}")
    }

    /// The line under a picture that says what it is called, and the gap
    /// above it. One number, because the layout reserves it and the tests
    /// have to be able to say why a shelf is as tall as it is.
    pub fn caption_height(&self) -> f32 {
        self.small_size + 4.0
    }

    /// Between a picture and its name.
    pub fn caption_gap(&self) -> f32 {
        2.0
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
    /// What a picture is called, under it.
    ///
    /// A preview alone is not enough to tell two attachments apart -- two
    /// screenshots of the same window look alike at any size a conversation
    /// can spare -- and the name alone was what the composer's tray used to
    /// show, which is no better. Its own kind so it is set quietly: a
    /// filename in full ink under every picture would read as something
    /// somebody said.
    Caption,
    /// One line of a link or permalink card. A card is several of these
    /// stacked with no gap, so the bar drawn beside them reads as one.
    Preview,
    /// The box a webhook's attachment is drawn in -- a Jira notice, a build
    /// result. It has no words of its own: what is in it is laid out as
    /// ordinary text inside it, and this is what the ground and the bar in the
    /// attachment's own colour are drawn to. The nth card is the nth
    /// attachment.
    Card,
    /// A horizontal rule written in the message: `---`, and `<hr>` in the
    /// app. A kind of its own because it is the one block with no words in
    /// it, and it used to be a blank line pretending -- which reserved the
    /// space a rule takes and then drew nothing in it.
    Rule,
    /// Where a cut code block trails off into its own ground, so the last line
    /// shown does not look like the last line there is. Drawn over the lines
    /// above it, which is why it is its own block and comes after them.
    Fade,
    /// The ground behind the offer to see the rest of a cut code block.
    ///
    /// Its own block for the same reason the fade is: what decides this is
    /// the order it is drawn in, between the listing it covers and the words
    /// it is under. Without it the offer is set over code, which is a line of
    /// text over another line of text and reads as neither.
    Pill,
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
    /// Show the whole of a message whose code block has been cut short, or
    /// cut it short again. By post id.
    ///
    /// Answered by the panel the message is in rather than by the app: what
    /// it changes is what that row measures, and the same message open in a
    /// channel and in a thread beside it can be shown whole in one of them.
    Whole(String),
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

/// A hair of room, put either side of a tag so its ground has somewhere to go.
///
/// The ground behind a mention reaches past its letters, and it was reaching
/// into the one space that separates it from whatever is beside it: two tags
/// in a row had their grounds touching, and a tag after a word had nothing
/// between them at all. The gap a reader sees is what is left *after* the
/// ground, so the room has to be made here rather than borrowed there.
///
/// A thin space rather than a full one, because this is a margin and not a
/// word break -- and spaces standing in for width is what the spans for a
/// custom emoji already do a few lines below.
const TAG_ROOM: &str = "\u{2009}";

/// Anything that carries a ground, with that room either side of it.
///
/// The room belongs to neither the press nor the ground. A reader aiming at a
/// name should not be able to miss it by a hair and still hit it, and a ground
/// is drawn to the glyphs it is for -- the pressed ones for a tag, the
/// monospaced ones for `code` -- rather than to these. Which is what makes
/// them room: they are the only thing either ground cannot reach into.
fn tagged(tag: TextSpan, into: &mut Vec<TextSpan>) {
    let room = TextSpan {
        text: TAG_ROOM.to_string(),
        bold: tag.bold,
        italic: tag.italic,
        // Never monospaced, whatever it is beside. `code` finds its ground by
        // asking which glyphs were set in the mono face, so room set in that
        // face would be swallowed by the very ground it is holding off.
        mono: false,
        press: None,
        faint: false,
        emoji: None,
    };
    // Not at the start of a line, where there is nothing to hold off: the
    // room pushed a paragraph's first line in by a hair, and a code span that
    // wrapped started its first line to the right of all the others.
    if into
        .last()
        .is_some_and(|before| !before.text.ends_with('\n'))
    {
        into.push(room.clone());
    }
    into.push(tag);
    into.push(room);
}

/// Words with no styling and nothing to press.
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

/// Markdown laid out as a message's body is, `inset` from the row's left and
/// `width` wide, from `y` down; answers where it ended.
///
/// The body goes through this and so does what a webhook's attachment
/// carries. The attachment used to be flattened to plain lines on the way in,
/// which is why a Jira notice lost its bullets and its italics and a link in
/// it could not be pressed.
#[allow(clippy::too_many_arguments)]
fn flow(
    fonts: &mut Fonts,
    nodes: &[Node],
    inset: f32,
    width: f32,
    mut y: f32,
    theme: &Theme,
    opened: bool,
    said: &str,
    blocks: &mut Vec<Block>,
) -> f32 {
    let mut pieces = Vec::new();
    lines_of(nodes, 0.0, &mut pieces);

    // Which pieces sit tight against the one after them, worked out before the
    // pass so each can be asked about its neighbour.
    let tight: Vec<bool> = pieces
        .iter()
        .map(|piece| matches!(piece, Piece::Line(line) if line.tight))
        .collect();

    // One pass, in the order the message was written. Two passes -- every line
    // and then every code block -- is what put a message's names above all of
    // its blocks instead of each name above its own.
    for (at, piece) in pieces.into_iter().enumerate() {
        // Nothing between two lines of one list. The ordinary gap everywhere
        // else, the end of a list included.
        let gap = match (tight.get(at), tight.get(at + 1)) {
            (Some(true), Some(true)) => 0.0,
            _ => theme.block_gap,
        };
        let line = match piece {
            Piece::Line(line) => line,
            Piece::Code(block) => {
                let from = blocks.len();
                y += code_block(fonts, &block, y, theme, opened, said, blocks);
                // Laid for the message's own column; inside a card it moves in
                // with everything else and is cut to the card's width.
                if inset > 0.0 {
                    for block in &mut blocks[from..] {
                        block.x += inset;
                        block.wrap = (block.wrap - inset).max(40.0);
                    }
                }
                continue;
            }
            // A hairline across the column, and the air either side of it is
            // the gap every block gets. It used to be a blank line: the room a
            // rule takes, with no rule drawn in it.
            Piece::Rule => {
                blocks.push(Block {
                    y,
                    x: inset,
                    height: theme.rule_height,
                    lines: 0,
                    kind: Kind::Rule,
                    spans: Vec::new(),
                    size: theme.body_size,
                    wrap: width,
                });
                y += theme.rule_height + gap;
                continue;
            }
        };
        let x = inset + line.indent * theme.indent;
        let wrap = inset + width - x;
        let count = line_count(fonts, &line, wrap, theme);
        let height = count as f32 * theme.line_height;
        // In the room the indent already made, at the same height as the first
        // line of the item -- so the words wrap under themselves rather than
        // under the dot, which is what a hanging indent is.
        if !line.marker.is_empty() {
            blocks.push(Block {
                y,
                x: (x - theme.indent).max(inset),
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
        y += height + gap;
    }
    y
}

/// One run of words as a block of its own, measured where it lands: a card's
/// title, a field's name. Answers where it ended.
#[allow(clippy::too_many_arguments)]
fn words(
    fonts: &mut Fonts,
    text: &str,
    bold: bool,
    press: Option<Press>,
    x: f32,
    width: f32,
    y: f32,
    theme: &Theme,
    blocks: &mut Vec<Block>,
) -> f32 {
    let line = Line {
        quoted: false,
        spans: vec![TextSpan {
            text: text.to_string(),
            bold,
            italic: false,
            mono: false,
            press,
            faint: false,
            emoji: None,
        }],
        indent: 0.0,
        heading: false,
        marker: String::new(),
        tight: false,
    };
    let count = line_count(fonts, &line, width, theme);
    let height = count as f32 * theme.line_height;
    blocks.push(Block {
        y,
        x,
        height,
        lines: count,
        kind: Kind::Text,
        spans: line.spans,
        size: theme.body_size,
        wrap: width,
    });
    y + height
}

/// An attachment's fields: the name over the value, two to a row when both
/// say they are short, the whole width when not -- which is how the official
/// client sets out a Jira notice's Assignee beside its Priority.
#[allow(clippy::too_many_arguments)]
fn fields(
    fonts: &mut Fonts,
    fields: &[matterless_render::AttachmentField],
    inset: f32,
    width: f32,
    mut y: f32,
    theme: &Theme,
    opened: bool,
    said: &str,
    blocks: &mut Vec<Block>,
) -> f32 {
    let mut at = 0;
    while at < fields.len() {
        let field = &fields[at];
        let beside = fields.get(at + 1).filter(|next| field.short && next.short);
        match beside {
            Some(next) => {
                let column = ((width - theme.indent) / 2.0).max(40.0);
                let left = field_cell(fonts, field, inset, column, y, theme, opened, said, blocks);
                let right = field_cell(
                    fonts,
                    next,
                    inset + column + theme.indent,
                    column,
                    y,
                    theme,
                    opened,
                    said,
                    blocks,
                );
                y = left.max(right);
                at += 2;
            }
            None => {
                y = field_cell(fonts, field, inset, width, y, theme, opened, said, blocks);
                at += 1;
            }
        }
    }
    y
}

/// One field: its name in bold, and under it the value as markdown.
#[allow(clippy::too_many_arguments)]
fn field_cell(
    fonts: &mut Fonts,
    field: &matterless_render::AttachmentField,
    x: f32,
    width: f32,
    mut y: f32,
    theme: &Theme,
    opened: bool,
    said: &str,
    blocks: &mut Vec<Block>,
) -> f32 {
    if !field.title.trim().is_empty() {
        y = words(fonts, &field.title, true, None, x, width, y, theme, blocks);
    }
    if field.value.is_empty() {
        return y + theme.block_gap;
    }
    flow(
        fonts,
        &field.value,
        x,
        width,
        y,
        theme,
        opened,
        said,
        blocks,
    )
}

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
    /// Part of a list, which is set with no gap between its items: `li {
    /// margin: 0 }` against the `p` above it.
    ///
    /// A gap after every line at all is what made a five-item list read as
    /// nearly double-spaced against the official client. The gap belongs to
    /// the list, not to each item of it, so it is taken only where two of
    /// these meet -- which leaves one after the last item, before whatever
    /// the message says next.
    tight: bool,
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

/// Reserves room for one code block, and says how much it took.
///
/// Measured at the width it will actually be shaped at, rather than counting
/// the lines as typed. Those two disagreed: the painter wrapped at the column
/// while this counted newlines, so one long log line reserved a single line's
/// height and drew over the message below it.
///
/// The app scrolls a code block sideways rather than wrapping it, and this
/// will too once the renderer can scroll sideways. That is a change to `wrap`
/// alone -- the count stays right, because it asks what will be drawn rather
/// than what was written.
fn code_block(
    fonts: &mut Fonts,
    block: &str,
    y: f32,
    theme: &Theme,
    opened: bool,
    key: &str,
    into: &mut Vec<Block>,
) -> f32 {
    let wrap = (theme.text_width() - theme.code_padding * 2.0).max(40.0);
    // Long enough to be worth cutting short, which a stack trace is and the
    // two lines of a command are not.
    let written = block.lines().count();
    let long = written > theme.code_lines_shown;
    let cut = long && !opened;
    // The lines as written, so what is reserved is what is drawn however the
    // column wraps them.
    let shown = match cut {
        true => block
            .lines()
            .take(theme.code_lines_shown)
            .collect::<Vec<_>>()
            .join("\n"),
        false => block.to_string(),
    };
    let count = crate::extent_of(
        fonts,
        &shown,
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
    // Where the listing itself stops, which is what the fade has to reach and
    // what the offer hangs below.
    let text_ends = theme.code_padding_y + count as f32 * theme.code_line_height;
    // Half a padding past that rather than centred on it: far enough down to
    // sit under the listing it is about, still far enough up to overlap its
    // last line.
    let offer = text_ends + theme.code_padding_y / 2.0 - theme.line_height / 2.0;
    // The ground ends halfway down the offer, so the offer straddles it: half
    // on the listing it is about and half on the message below. Which is what
    // makes it a control on the block rather than a last line inside it.
    let height = match long {
        true => offer + theme.line_height / 2.0,
        false => count as f32 * theme.code_line_height + theme.code_padding,
    };
    into.push(Block {
        y,
        x: 0.0,
        height,
        lines: count,
        kind: Kind::Code,
        spans: vec![TextSpan {
            text: shown,
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
    // The way to the rest of it, or back to being rid of it. Only for a block
    // long enough to have been cut: a line offering to show what is already
    // shown is one more thing to read.
    if long {
        let words = match cut {
            true => format!("Show the other {} lines", written - theme.code_lines_shown),
            false => "Show less".to_string(),
        };
        // The listing trailing off into its own ground, over the last lines
        // shown: what says the last line drawn is not the last line there is.
        // Only when there is something behind it to trail off from.
        if cut {
            // From three lines up -- the offer sits on the last of them, and a
            // fade still at half strength by then leaves the words behind the
            // offer as legible as the offer.
            let from =
                theme.code_padding_y + (count as f32 - 3.0).max(0.0) * theme.code_line_height;
            // To where the *text* ends, not where the block does. Run to the
            // floor of the block instead and the gradient is only seven parts
            // in ten of the way along by the time it passes the last line, so
            // that line stays a third readable however far the fade goes on
            // below it. Under the text it is the block's own colour either
            // way, so nothing shows the join.
            into.push(Block {
                y: y + from,
                x: 0.0,
                height: text_ends - from,
                lines: 0,
                kind: Kind::Fade,
                spans: Vec::new(),
                size: theme.code_size,
                wrap: theme.text_width(),
            });
        }
        // Centred across the block, which is where an end is looked for.
        let across = crate::extent_of(
            fonts,
            &words,
            theme.text_width(),
            crate::Style {
                size: theme.body_size,
                line_height: theme.line_height,
                bold: false,
                italic: false,
                mono: false,
            },
        )
        .width;
        // Over the last line shown, not under it: the listing carries on behind
        // the offer and fades out around it, which is what says there is more
        // of it rather than that it stopped here. Low on that line rather than
        // centred on it, so what shows around the offer is the faintest part of
        // the fade rather than the half of a line that is still perfectly
        // readable.
        let above = y + offer;
        let x = ((theme.text_width() - across) / 2.0).max(0.0);
        // Its own ground, because words set straight over code are two lines of
        // text in one place and read as neither.
        // A pixel of slack on the words, against the rounding between
        // measuring a run and shaping it again at exactly that width. The
        // ground carries the same slack, so it stays centred on them.
        let room = across + 1.0;
        into.push(Block {
            y: above,
            x: x - theme.pill_padding,
            height: theme.line_height,
            lines: 0,
            kind: Kind::Pill,
            spans: Vec::new(),
            size: theme.body_size,
            wrap: room + theme.pill_padding * 2.0,
        });
        into.push(Block {
            y: above,
            x,
            height: theme.line_height,
            lines: 1,
            kind: Kind::Text,
            // Room for the words and nothing else, so the line cannot wrap
            // somewhere other than where it was measured.
            wrap: room,
            spans: vec![TextSpan {
                text: words,
                bold: false,
                italic: false,
                mono: false,
                press: Some(Press::Whole(key.to_string())),
                faint: true,
                emoji: None,
            }],
            size: theme.body_size,
        });
        // The half of the offer that hangs below the ground: room the row owes
        // it even though the block itself has ended.
        return offer + theme.line_height + theme.block_gap;
    }
    height + theme.block_gap
}

/// One thing in a message body, in the order it was written.
///
/// A code block used to be gathered into a list of its own while every line of
/// text went into another, and the layout drew all of one and then all of the
/// other. A message that alternates the two -- a name, its block, the next
/// name -- came out as every name followed by every block, which is not the
/// message anybody sent. So there is one sequence, and its order is the order
/// it was written in.
enum Piece {
    Line(Line),
    Code(String),
    /// A written `---`, which divides one section of a long notice from the
    /// next. The Sentry crash notices are made of these.
    Rule,
}

/// Ends the run of inline content being gathered, if there is one.
fn flush(pending: &mut Vec<TextSpan>, indent: f32, into: &mut Vec<Piece>) {
    if pending.is_empty() {
        return;
    }
    into.push(Piece::Line(Line {
        quoted: false,
        spans: std::mem::take(pending),
        indent,
        heading: false,
        marker: String::new(),
        tight: false,
    }));
}

/// Flattens the markdown into the blocks a reader sees stacked.
///
/// Inline content is *gathered* rather than taken one node at a time. A
/// paragraph arrives wrapped and a list item does not: its children are the
/// text, the mention, the space between two mentions, the closing bracket. One
/// line per node put every `@name` on a line of its own with a blank line after
/// it, which is what a real message from this server looked like.
fn lines_of(nodes: &[Node], indent: f32, into: &mut Vec<Piece>) {
    let mut pending: Vec<TextSpan> = Vec::new();
    for node in nodes {
        match node {
            Node::Paragraph { children } => {
                flush(&mut pending, indent, into);
                let mut spans = Vec::new();
                inline(children, false, false, false, None, &mut spans);
                into.push(Piece::Line(Line {
                    quoted: false,
                    spans,
                    indent,
                    heading: false,
                    marker: String::new(),
                    tight: false,
                }));
            }
            Node::Heading { children, .. } => {
                flush(&mut pending, indent, into);
                let mut spans = Vec::new();
                inline(children, true, false, false, None, &mut spans);
                into.push(Piece::Line(Line {
                    quoted: false,
                    spans,
                    indent,
                    heading: true,
                    marker: String::new(),
                    tight: false,
                }));
            }
            // A quote is pushed in like a list item, and marked so whoever
            // draws it can put a bar down its left and set it in the softer
            // ink: `blockquote { border-left: 3px solid var(--rule); color:
            // var(--ink-soft) }`.
            Node::Blockquote { children } => {
                flush(&mut pending, indent, into);
                let from = into.len();
                lines_of(children, indent + 1.0, into);
                for piece in into.iter_mut().skip(from) {
                    if let Piece::Line(line) = piece {
                        line.quoted = true;
                    }
                }
            }
            Node::List { ordered, items } => {
                flush(&mut pending, indent, into);
                let list = into.len();
                for (at, item) in items.iter().enumerate() {
                    let from = into.len();
                    lines_of(item, indent + 1.0, into);
                    // On the first line of the item only: the rest of a
                    // wrapped item hangs under the words, not under the dot.
                    // The first *line*, not the first piece: an item that
                    // opens with a code block has no line to mark there.
                    if let Some(Piece::Line(first)) = into.get_mut(from) {
                        first.marker = marker_for(*ordered, at, indent);
                    }
                }
                // Every line the list produced, its nested lists included: the
                // gap belongs to the list rather than to each item, so it is
                // taken where the list ends and not between its items.
                for piece in into.iter_mut().skip(list) {
                    if let Piece::Line(line) = piece {
                        line.tight = true;
                    }
                }
            }
            Node::CodeBlock { value, .. } => {
                flush(&mut pending, indent, into);
                into.push(Piece::Code(value.clone()))
            }
            Node::Table { head, rows } => {
                flush(&mut pending, indent, into);
                // Not laid out as a table yet: each cell is a line, which is
                // the same *number* of lines a stacked fallback draws.
                for cell in head {
                    lines_of(cell, indent, into);
                }
                for row in rows {
                    for cell in row {
                        lines_of(cell, indent, into);
                    }
                }
            }
            Node::Rule => {
                flush(&mut pending, indent, into);
                into.push(Piece::Rule)
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
            // With room either side, for the same reason a tag has it: the
            // ground behind `code` reaches past its letters, and what it was
            // reaching into was the one space separating it from the word
            // beside it.
            Node::InlineCode { value } | Node::InlineMath { value } => tagged(
                TextSpan {
                    text: value.clone(),
                    bold,
                    italic,
                    mono: true,
                    press: press.cloned(),
                    faint: false,
                    emoji: None,
                },
                into,
            ),
            // A mention and a channel link point at somebody and somewhere
            // whatever they are nested inside, so they name their own press
            // rather than inheriting the surrounding one.
            Node::UserMention { username, .. } => tagged(
                TextSpan {
                    text: format!("@{username}"),
                    bold,
                    italic,
                    mono,
                    press: Some(Press::Person(username.clone())),
                    faint: false,
                    emoji: None,
                },
                into,
            ),
            Node::ChannelLink { name } => tagged(
                TextSpan {
                    text: format!("~{name}"),
                    bold,
                    italic,
                    mono,
                    press: Some(Press::Channel(name.clone())),
                    faint: false,
                    emoji: None,
                },
                into,
            ),
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
    lay_out_opened(fonts, row, theme, false)
}

/// The same, told whether the reader has asked to see the whole of this row.
///
/// Which for now means a code block too long to draw: cut short unless they
/// have said otherwise. The answer belongs to the panel rather than to the
/// message -- the same message read in a channel and in a thread beside it
/// can be opened in one and not the other.
pub fn lay_out_opened(fonts: &mut Fonts, row: &Row, theme: &Theme, opened: bool) -> RowLayout {
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
            // No height of its own, and a block lifted half its own depth so
            // the rule lands exactly on the join between the two messages.
            //
            // It is not a part of the conversation, it is a mark on it -- so
            // it must not move the conversation. As a row of `separator_height`
            // it did: every message below it sat 34 lower while it was there,
            // and it arrives and goes as messages are read, so the whole
            // conversation stepped up and down under the eye for a line that
            // nobody said.
            //
            // The date separator keeps its row. That one really is part of
            // the conversation -- it says when what follows was said, and it
            // belongs between the days rather than over them.
            return RowLayout {
                height: 0.0,
                blocks: vec![Block {
                    y: -theme.separator_height / 2.0,
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
            // A line each. A merged run of comings and goings says one thing
            // per kind of them, so the row is as tall as it has things to say
            // -- almost always one, which is what it was before.
            let said: Vec<&str> = text.split('\n').collect();
            return RowLayout {
                height: theme.line_height * said.len() as f32 + theme.row_padding * 2.0,
                blocks: said
                    .into_iter()
                    .enumerate()
                    .map(|(at, line)| Block {
                        y: theme.row_padding + theme.line_height * at as f32,
                        x: 0.0,
                        height: theme.line_height,
                        lines: 1,
                        kind: Kind::Text,
                        spans: vec![TextSpan {
                            italic: true,
                            faint: true,
                            ..plain(line.to_string())
                        }],
                        size: theme.system_size,
                        wrap: theme.text_width(),
                    })
                    .collect(),
            };
        }
        Row::Post { post } => (post.nodes.as_slice(), Some(post), true),
        Row::Continuation { post } => (post.nodes.as_slice(), Some(post), false),
    };

    let attachments = post.map(|post| post.files.len()).unwrap_or(0);

    // A message carrying on from the one above it is the same person still
    // talking, and is spaced as such.
    let padding = match header {
        true => theme.row_padding,
        false => theme.merged_padding,
    };
    y += padding;
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
                // The same press an `@name` in a sentence carries, so pressing
                // who said it opens the same card -- which is what the official
                // client does and the only thing the name was missing.
                //
                // Never for a webhook: what is shown there is a display name
                // the sender chose, not an account, and looking it up can only
                // fail.
                press: match post.bot {
                    true => None,
                    false => Some(Press::Person(post.author_name.clone())),
                },
                faint: false,
                emoji: None,
            });
            // As the official client marks one: the name is the integration's
            // own, and nothing else would say that no person wrote this.
            if post.bot {
                spans.push(TextSpan {
                    text: "  BOT".to_string(),
                    bold: true,
                    italic: false,
                    mono: false,
                    press: None,
                    faint: true,
                    emoji: None,
                });
            }
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
        // The same gap that separates every other pair of blocks. Without it
        // the first line of the message began exactly where the name's box
        // ended, which is close enough that anything with a ground of its own
        // -- `code` on that first line -- was drawn a pixel under the name and
        // into the reach of its descenders.
        y += theme.header_height + theme.block_gap;
    }

    // Which message a "show the rest" would be about.
    let said = post.map(|post| post.post_id.as_str()).unwrap_or_default();

    y = flow(
        fonts,
        nodes,
        0.0,
        theme.text_width(),
        y,
        theme,
        opened,
        said,
        &mut blocks,
    );

    if attachments > 0 {
        // The server gives every attachment its drawn size, so this is the one
        // height that was never in doubt: `box_width` and `box_height` are what
        // the planner already worked out, and reserving exactly them means a
        // picture arriving moves nothing.
        //
        // One block per file, in file order -- the nth block is the nth
        // file, which both the drawing and the hit test rely on.
        //
        // Pictures are packed along a shelf and wrap onto the next when one
        // will not take another: two screenshots stacked took two screenfuls
        // where side by side they take one, which is the whole reason. Each
        // keeps the size the server gave it rather than being squared off
        // into a cell -- nothing is cropped and nothing is scaled, so the
        // reserved height stays exactly right.
        //
        // A card is a line with a name on it rather than a thumbnail, so it
        // takes a shelf to itself: two beside each other are two truncated
        // names.
        let room = theme.text_width();
        // Where the next picture starts, and the tallest so far on this shelf.
        let mut along = 0.0_f32;
        let mut tallest = 0.0_f32;
        let close_the_shelf = |y: &mut f32, along: &mut f32, tallest: &mut f32| {
            if *tallest > 0.0 {
                *y += *tallest + theme.block_gap;
            }
            *along = 0.0;
            *tallest = 0.0;
        };
        for file in post.map(|post| post.files.as_slice()).unwrap_or(&[]) {
            if !(file.image || file.video) {
                close_the_shelf(&mut y, &mut along, &mut tallest);
                blocks.push(Block {
                    y,
                    x: 0.0,
                    height: theme.card_height,
                    lines: 0,
                    kind: Kind::Attachment,
                    spans: Vec::new(),
                    size: theme.body_size,
                    wrap: (file.box_width.max(0) as f32).min(room),
                });
                y += theme.card_height + theme.block_gap;
                continue;
            }

            // Both sides together: clamping the width alone to a narrower
            // column flattens the picture rather than shrinking it.
            // Both sides together: clamping the width alone to a narrower
            // column flattens the picture rather than shrinking it.
            let (width, height) = file.drawn_in(room);
            // Never on an empty shelf: a picture as wide as the column has to
            // go somewhere, and starting a shelf for it would never end.
            if along > 0.0 && along + width > room {
                close_the_shelf(&mut y, &mut along, &mut tallest);
            }
            blocks.push(Block {
                y,
                x: along,
                height,
                lines: 0,
                kind: Kind::Attachment,
                spans: Vec::new(),
                size: theme.body_size,
                wrap: width,
            });
            // A picture from the text has no file name, and the official client
            // draws none under it.
            if file.variant == matterless_render::ImageVariant::Linked {
                along += width + theme.block_gap;
                tallest = tallest.max(height);
                continue;
            }
            // What it is called, under it, elided to the picture's own width.
            //
            // A line of the layout rather than something the painter adds, so
            // the room it takes is reserved like everything else here: a name
            // drawn into space nobody set aside is drawn over the next
            // message. One line always -- a filename that wrapped would make
            // a picture's height an answer about text, which is the one thing
            // this layout refuses to guess at.
            let named = crate::elided(
                fonts,
                &file.name,
                width,
                crate::Style {
                    size: theme.small_size,
                    line_height: theme.caption_height(),
                    bold: false,
                    italic: false,
                    mono: false,
                },
            );
            let caption = theme.caption_height();
            blocks.push(Block {
                y: y + height + theme.caption_gap(),
                x: along,
                height: caption,
                lines: 1,
                kind: Kind::Caption,
                spans: vec![plain(named)],
                size: theme.small_size,
                wrap: width,
            });
            along += width + theme.block_gap;
            tallest = tallest.max(height + theme.caption_gap() + caption);
        }
        close_the_shelf(&mut y, &mut along, &mut tallest);
    }

    // What a webhook sent, which for a great many posts here *is* the message:
    // a bot puts its whole payload in `attachments` and leaves the body empty,
    // so a post with none of this drawn is a post with nothing on it at all.
    //
    // As the official client sets it out: the pretext above the card in the
    // message's own column, and inside the card a title that is a link when it
    // has one, the text as markdown, and the fields as a grid of name over
    // value. Each card is one `Card` block spanning the lot, which is what its
    // ground and its coloured bar are drawn to; the words inside are ordinary
    // text, so a link in them can be pressed and an emoji in them drawn.
    for attached in post.map(|post| post.attachments.as_slice()).unwrap_or(&[]) {
        if !attached.pretext.is_empty() {
            y = flow(
                fonts,
                &attached.pretext,
                0.0,
                theme.text_width(),
                y,
                theme,
                opened,
                said,
                &mut blocks,
            );
        }
        let top = y;
        let card = blocks.len();
        blocks.push(Block {
            y: top,
            x: 0.0,
            height: 0.0,
            lines: 0,
            kind: Kind::Card,
            spans: Vec::new(),
            size: theme.body_size,
            wrap: theme.text_width(),
        });
        // The card's own room, set aside before its first line so that the
        // ground drawn behind it lands on space this reserved rather than on
        // the author's name above.
        y += theme.attached_padding;
        let inset = theme.quote_bar + theme.indent / 2.0;
        let inner = (theme.text_width() - inset - theme.indent / 2.0).max(40.0);
        let first = y;
        if let Some(title) = attached
            .title
            .as_deref()
            .filter(|title| !title.trim().is_empty())
        {
            let press = attached.title_link.as_deref().and_then(openable);
            y = words(
                fonts,
                title,
                true,
                press,
                inset,
                inner,
                y,
                theme,
                &mut blocks,
            );
            y += theme.block_gap;
        }
        if !attached.text.is_empty() {
            y = flow(
                fonts,
                &attached.text,
                inset,
                inner,
                y,
                theme,
                opened,
                said,
                &mut blocks,
            );
        }
        y = fields(
            fonts,
            &attached.fields,
            inset,
            inner,
            y,
            theme,
            opened,
            said,
            &mut blocks,
        );
        // Every piece leaves a gap after itself for whatever follows, and what
        // follows the last one is the card's own padding instead.
        if y > first {
            y -= theme.block_gap;
        }
        y += theme.attached_padding;
        blocks[card].height = y - top;
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
                    size: theme.pill_size,
                    line_height: theme.pill_line,
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
                size: theme.pill_size,
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
        height: y + padding + theme.row_gap,
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
        // By what they lead to rather than by where they sit: a tag carries a
        // hair of room either side of it, so its span is not the one after the
        // words any more.
        let leads: Vec<&Press> = spans
            .iter()
            .filter_map(|span| span.press.as_ref())
            .collect();
        assert_eq!(
            leads,
            vec![
                &Press::Link("https://example.invalid/x".into()),
                &Press::Person("ada".into()),
            ]
        );
    }

    /// What one row costs to shape, which is what a resize pays per message.
    ///
    ///     cargo test -p matterless-layout --release shaping_one_row -- --ignored --nocapture
    #[test]
    #[ignore = "a measurement, not an assertion"]
    fn shaping_one_row_costs() {
        let mut fonts = Fonts::new();
        let theme = Theme::default();
        // A spread of shapes a real channel holds, rather than one sentence
        // repeated: a row is anything from a word to a screenful of code.
        let bodies: Vec<Vec<Node>> = (0..179)
            .map(|at| match at % 4 {
                0 => vec![text("ok")],
                1 => vec![text(&"a sentence with a fair few words in it ".repeat(3))],
                2 => vec![Node::List {
                    ordered: false,
                    items: vec![vec![text("one")], vec![text("two")], vec![text("three")]],
                }],
                _ => vec![Node::CodeBlock {
                    language: None,
                    value: "a line of a stack trace\n".repeat(8),
                }],
            })
            .collect();
        let rows: Vec<Row> = bodies
            .into_iter()
            .map(|nodes| Row::Post { post: post(nodes) })
            .collect();

        // Warmed, so the font matching and the atlas are not in the number.
        for row in &rows {
            let _ = lay_out(&mut fonts, row, &theme);
        }
        let began = std::time::Instant::now();
        for row in &rows {
            let _ = lay_out(&mut fonts, row, &theme);
        }
        let took = began.elapsed();
        println!(
            "COST {} rows in {:.1}ms, {:.3}ms each",
            rows.len(),
            took.as_secs_f64() * 1000.0,
            took.as_secs_f64() * 1000.0 / rows.len() as f64
        );
    }

    /// A tag makes its own room instead of borrowing the space beside it.
    ///
    /// The ground behind a mention reaches past its letters. With nothing but
    /// the one space between two of them, those grounds met in the middle and
    /// the pair read as one long tag -- and a tag straight after a word had
    /// nothing between them at all.
    #[test]
    fn a_tag_carries_a_hair_of_room_on_either_side() {
        let mut spans = Vec::new();
        inline(
            &[
                Node::Text {
                    value: "poke ".into(),
                },
                Node::UserMention {
                    username: "remi".into(),
                    everyone: false,
                },
                Node::Text { value: " ".into() },
                Node::UserMention {
                    username: "thibaut".into(),
                    everyone: false,
                },
            ],
            false,
            false,
            false,
            None,
            &mut spans,
        );
        let at = |who: &str| {
            spans
                .iter()
                .position(|span| span.press == Some(Press::Person(who.into())))
                .unwrap_or_else(|| panic!("no {who}: {spans:?}"))
        };
        for (who, side) in [("remi", 1usize), ("thibaut", 1)] {
            let tag = at(who);
            for beside in [tag - side, tag + side] {
                assert_eq!(spans[beside].text, super::TAG_ROOM, "{who} has no room");
                assert!(spans[beside].press.is_none(), "the room is pressable");
            }
        }
        // And the room is between them, not inside either: what is pressed is
        // the name and nothing else.
        assert_eq!(spans[at("remi")].text, "@remi");
        assert!(at("thibaut") > at("remi") + 1);
    }

    /// And so does `code`, whose ground reaches the same way a tag's does.
    ///
    /// The room must not be monospaced itself: the ground behind `code` is
    /// drawn to whichever glyphs were set in the mono face, so room in that
    /// face would be swallowed by the ground it is there to hold off.
    #[test]
    fn code_carries_a_hair_of_room_on_either_side() {
        let mut spans = Vec::new();
        inline(
            &[
                Node::Text {
                    value: "run ".into(),
                },
                Node::InlineCode {
                    value: "cargo test".into(),
                },
                Node::Text {
                    value: " first".into(),
                },
            ],
            false,
            false,
            false,
            None,
            &mut spans,
        );
        let at = spans
            .iter()
            .position(|span| span.text == "cargo test")
            .unwrap_or_else(|| panic!("no code: {spans:?}"));
        for beside in [at - 1, at + 1] {
            assert_eq!(spans[beside].text, super::TAG_ROOM, "code has no room");
            assert!(
                !spans[beside].mono,
                "the room is monospaced, so the ground will eat it"
            );
        }
        assert!(spans[at].mono, "the code is not monospaced");
    }

    /// Code that opens a line has nothing before it to hold off, so it has no
    /// room there -- or its first line starts right of the lines it wraps to.
    #[test]
    fn code_opening_a_line_has_no_room_before_it() {
        let mut spans = Vec::new();
        inline(
            &[
                Node::InlineCode {
                    value: "In BP_Upgrade : Error".into(),
                },
                Node::HardBreak,
                Node::InlineCode {
                    value: "Parent=BP_Harp".into(),
                },
            ],
            false,
            false,
            false,
            None,
            &mut spans,
        );
        assert!(spans[0].mono, "the paragraph starts with room: {spans:?}");
        let second = spans
            .iter()
            .position(|span| span.text == "Parent=BP_Harp")
            .expect("the second code");
        assert_eq!(
            spans[second - 1].text,
            "\n",
            "room after a break: {spans:?}"
        );
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
                "Could we raise the cache size the test runner is allowed, to a couple of gigabytes or so? It looks as though it is set in the file beside the runner rather than anywhere obvious, and I would rather not guess at it on a machine everybody shares. Either way suits me, but the current size stops a clean build finishing at all.",
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

    /// A continuation omits the author line, and is spaced as one person
    /// still talking.
    ///
    /// The line and the gap under it: the header is separated from the first
    /// block of the message by the same gap that separates every other pair of
    /// blocks, so a message without a header is shorter by both. And by the
    /// padding it does not carry either -- a run of short messages read as
    /// five separate things at a full row's padding each.
    #[test]
    fn a_continuation_is_shorter_by_its_header() {
        let mut fonts = Fonts::new();
        let theme = Theme::default();
        let body = vec![text("Morning!")];
        let first = lay_out(
            &mut fonts,
            &Row::Post {
                post: post(body.clone()),
            },
            &theme,
        );
        let next = lay_out(&mut fonts, &Row::Continuation { post: post(body) }, &theme);
        assert_eq!(
            first.height - next.height,
            theme.header_height
                + theme.block_gap
                + (theme.row_padding - theme.merged_padding) * 2.0
        );
        // Which leaves two things one person said in a row sitting like two
        // paragraphs of one message: the gap a block owes the next, plus the
        // gap the stream owes between rows, and nothing else. At a full row's
        // padding it was that plus six more, and a run of one-word messages
        // read as five separate things.
        let between = next.height - theme.line_height;
        assert_eq!(
            between,
            theme.block_gap + theme.row_gap,
            "{between} between two merged messages"
        );
    }

    /// Short lines do not wrap, so the count is the lines as written.
    #[test]
    fn a_message_keeps_its_blocks_in_the_order_they_were_written() {
        let mut fonts = Fonts::new();
        let theme = Theme::default();
        // The shape of a daily standup notice: a name, what they said, the
        // next name. Every name used to be drawn above every block, because
        // the lines and the code blocks were gathered into separate lists and
        // laid out one list after the other.
        let said = |who: &str| Node::Paragraph {
            children: vec![Node::Strong {
                children: vec![Node::Text { value: who.into() }],
            }],
        };
        let block = |what: &str| Node::CodeBlock {
            language: None,
            value: what.into(),
        };
        let row = Row::Post {
            post: post(vec![
                said("Lazare"),
                block("OoO"),
                said("Silana"),
                block("BWC avec Mos."),
            ]),
        };
        let laid = lay_out(&mut fonts, &row, &theme);
        // Each name above its own block, by the order they come out at.
        let order: Vec<Kind> = laid
            .blocks
            .iter()
            .filter(|block| matches!(block.kind, Kind::Text | Kind::Code))
            .filter(|block| !block.spans.is_empty())
            .map(|block| block.kind)
            .collect();
        assert_eq!(
            order,
            vec![Kind::Text, Kind::Code, Kind::Text, Kind::Code],
            "the message came out as {order:?}"
        );
        // And stacked downwards, rather than two piles at the same heights.
        let tops: Vec<f32> = laid
            .blocks
            .iter()
            .filter(|block| matches!(block.kind, Kind::Text | Kind::Code))
            .filter(|block| !block.spans.is_empty())
            .map(|block| block.y)
            .collect();
        assert!(
            tops.windows(2).all(|pair| pair[0] < pair[1]),
            "the blocks are not in descending order: {tops:?}"
        );
    }

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
        let long = "UATHelper: Packaging (Windows): Module.Voyager.27.cpp.obj : error \
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

    /// Pressing who said it opens them, the way pressing an `@name` does.
    #[test]
    fn the_author_of_a_message_can_be_pressed() {
        let mut fonts = Fonts::new();
        let theme = Theme::default();
        let mut post = post(vec![text("morning")]);
        post.author_name = "ada.lovelace".into();
        let laid = lay_out(&mut fonts, &Row::Post { post }, &theme);
        let header = laid
            .blocks
            .iter()
            .find(|block| block.kind == Kind::Header)
            .expect("no header");
        let pressed: Vec<&Press> = header
            .spans
            .iter()
            .filter_map(|span| span.press.as_ref())
            .collect();
        assert_eq!(
            pressed,
            vec![&Press::Person("ada.lovelace".into())],
            "the name, and only the name: a timestamp leads nowhere"
        );
    }

    /// What a webhook shows is a name its sender chose, not an account, so
    /// looking it up could only ever fail.
    #[test]
    fn a_webhooks_name_leads_nowhere() {
        let mut fonts = Fonts::new();
        let theme = Theme::default();
        let mut post = post(vec![text("build 412 failed")]);
        post.author_name = "Build Robot".into();
        post.bot = true;
        let laid = lay_out(&mut fonts, &Row::Post { post }, &theme);
        let header = laid
            .blocks
            .iter()
            .find(|block| block.kind == Kind::Header)
            .expect("no header");
        assert!(
            header.spans.iter().all(|span| span.press.is_none()),
            "a webhook's display name was made pressable"
        );
    }

    /// A pill is exactly its contents plus the same padding on either side.
    ///
    /// The width measured here is the width the pill is drawn and hit as, so
    /// if it is measured at one size and the words set at another the pill
    /// comes out wider than what is in it -- and every bit of the slack falls
    /// on the right, because the words start at the left padding. That reads
    /// as contents that are not centred, which is what they were.
    #[test]
    fn a_reaction_pill_is_no_wider_than_what_is_in_it() {
        let mut fonts = Fonts::new();
        let theme = Theme::default();
        let mut post = post(vec![text("nice")]);
        post.reactions = vec![matterless_render::ReactionSummary {
            emoji: "tada".into(),
            count: 12,
            mine: false,
            unicode: Some("\u{1F389}".into()),
            names: Vec::new(),
        }];
        let laid = lay_out(&mut fonts, &Row::Post { post }, &theme);
        let pill = laid
            .blocks
            .iter()
            .find(|block| block.kind == Kind::Reactions)
            .expect("no pill");
        let said: String = pill.spans.iter().map(|span| span.text.as_str()).collect();
        // Measured the way the pill will be drawn: at the size the block
        // itself carries, which is the only size anybody downstream knows.
        let words = crate::extent_of(
            &mut fonts,
            &said,
            f32::MAX,
            crate::Style {
                size: pill.size,
                line_height: theme.pill_line,
                bold: false,
                italic: false,
                mono: false,
            },
        )
        .width;
        assert!(
            (pill.wrap - (words + theme.pill_padding * 2.0)).abs() < 0.5,
            "pill {} wide for {words} of words and {} of padding",
            pill.wrap,
            theme.pill_padding * 2.0
        );
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

    /// The unread line takes no room, so the conversation does not move
    /// when it appears or goes.
    ///
    /// Reported: it was a row of `separator_height`, so every message below
    /// it sat 34 lower while it was there -- and it arrives and goes as
    /// messages are read, which stepped the whole conversation up and down
    /// under the eye for a line nobody said.
    ///
    /// Its block is lifted half its own depth so the rule lands on the join
    /// between the two messages rather than inside either of them.
    #[test]
    fn the_unread_line_costs_the_conversation_nothing() {
        let mut fonts = Fonts::new();
        let theme = Theme::default();
        let laid = lay_out(&mut fonts, &Row::UnreadDivider, &theme);

        assert_eq!(laid.height, 0.0, "it still pushes the conversation down");
        let block = laid.blocks.first().expect("something to draw");
        assert_eq!(block.kind, Kind::Unread);
        assert!(
            block.y < 0.0,
            "the rule sits below the join rather than on it"
        );
        assert_eq!(
            block.y + block.height / 2.0,
            0.0,
            "the rule is not centred on the join between the messages"
        );
    }

    /// The day separator keeps its row, because it is part of what is being
    /// read rather than a mark on it.
    #[test]
    fn a_separator_is_a_fixed_height() {
        let mut fonts = Fonts::new();
        let theme = Theme::default();
        let laid = lay_out(&mut fonts, &Row::DateSeparator { epoch_day: 20137 }, &theme);
        assert_eq!(laid.height, theme.separator_height);
        assert_eq!(laid.blocks.len(), 1);
    }

    /// A merged run of comings and goings says one thing per kind of them, so
    /// the row has to be as tall as it has things to say. Reserving one line
    /// for two would draw the second over whatever is under it.
    #[test]
    fn a_system_row_is_as_tall_as_it_has_things_to_say() {
        let mut fonts = Fonts::new();
        let theme = Theme::default();
        let laid = lay_out(
            &mut fonts,
            &Row::System {
                post_id: "p1".into(),
                post_type: "system_combined_user_activity".into(),
                nodes: Vec::new(),
                text: "ada and bob joined the channel\ncal left the channel".into(),
            },
            &theme,
        );

        assert_eq!(laid.blocks.len(), 2, "a block each");
        assert_eq!(
            laid.blocks[0].spans[0].text,
            "ada and bob joined the channel"
        );
        assert_eq!(laid.blocks[1].spans[0].text, "cal left the channel");
        assert_eq!(
            laid.blocks[1].y - laid.blocks[0].y,
            theme.line_height,
            "the second sits a line under the first"
        );
        assert_eq!(
            laid.height,
            theme.line_height * 2.0 + theme.row_padding * 2.0,
            "and the row reserved room for both"
        );
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

    /// Two pictures sit beside each other, because two stacked took two
    /// screenfuls where side by side they take one. Each keeps the size the
    /// server gave it, and the row is tall enough for the taller of them.
    #[test]
    fn pictures_share_a_shelf() {
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

        assert_eq!(blocks.len(), 2, "a block each, in file order");
        assert_eq!((blocks[0].wrap, blocks[0].height), (300.0, 200.0));
        assert_eq!((blocks[1].wrap, blocks[1].height), (120.0, 90.0));
        assert_eq!(blocks[0].y, blocks[1].y, "the same shelf");
        assert_eq!(blocks[0].x, 0.0);
        assert_eq!(
            blocks[1].x,
            300.0 + theme.block_gap,
            "the second starts where the first ends"
        );
        assert!(
            laid.height >= blocks[0].y + 200.0,
            "as tall as the taller of them"
        );
    }

    /// A shelf that will not take another wraps, and the next one clears the
    /// tallest above it rather than the last one along.
    #[test]
    fn a_shelf_that_is_full_wraps_under_the_tallest_on_it() {
        let mut fonts = Fonts::new();
        let theme = Theme::default();
        let room = theme.text_width();
        let wide = (room / 2.0) as i32 - 10;
        let mut with = post(vec![]);
        with.files = vec![
            picture("f1", wide, 300),
            picture("f2", wide, 80),
            picture("f3", wide, 50),
        ];
        let laid = lay_out(&mut fonts, &Row::Post { post: with }, &theme);
        let blocks: Vec<&Block> = laid
            .blocks
            .iter()
            .filter(|block| block.kind == Kind::Attachment)
            .collect();

        assert_eq!(blocks[0].y, blocks[1].y, "two fit");
        assert_eq!(
            blocks[2].y,
            blocks[0].y + 300.0 + theme.caption_gap() + theme.caption_height() + theme.block_gap,
            "the third clears the 300 above it and its name, not the 80"
        );
        assert_eq!(blocks[2].x, 0.0, "and starts a shelf of its own");
        assert!(laid.height >= blocks[2].y + 50.0);
    }

    /// A picture as wide as the column has to go somewhere: it takes a shelf
    /// alone rather than looking for one it fits on and never finding it.
    #[test]
    fn a_picture_too_wide_to_share_still_gets_a_shelf() {
        let mut fonts = Fonts::new();
        let theme = Theme::default();
        let mut with = post(vec![]);
        with.files = vec![picture("f1", 4000, 100), picture("f2", 4000, 120)];
        let laid = lay_out(&mut fonts, &Row::Post { post: with }, &theme);
        let blocks: Vec<&Block> = laid
            .blocks
            .iter()
            .filter(|block| block.kind == Kind::Attachment)
            .collect();

        assert_eq!(blocks.len(), 2);
        for block in &blocks {
            assert_eq!(block.x, 0.0);
            assert_eq!(block.wrap, theme.text_width());
        }
        // Past the first rather than past the height it was given: a
        // picture as wide as the column is scaled down to fit it, so the
        // hundred it arrived with is not the room it takes.
        assert!(
            blocks[1].y >= blocks[0].y + blocks[0].height,
            "one under the other"
        );
    }

    /// A card is a line with a name on it rather than a thumbnail, so it takes
    /// a shelf to itself: two beside each other are two truncated names.
    #[test]
    fn a_card_takes_a_shelf_to_itself() {
        let mut fonts = Fonts::new();
        let theme = Theme::default();
        let mut document = picture("f2", 0, 0);
        document.image = false;
        document.name = "notes.pdf".into();
        let mut with = post(vec![]);
        with.files = vec![picture("f1", 200, 150), document, picture("f3", 200, 90)];
        let laid = lay_out(&mut fonts, &Row::Post { post: with }, &theme);
        let blocks: Vec<&Block> = laid
            .blocks
            .iter()
            .filter(|block| block.kind == Kind::Attachment)
            .collect();

        assert_eq!(blocks.len(), 3);
        assert!(blocks[1].y >= blocks[0].y + 150.0, "under the picture");
        assert_eq!(blocks[1].x, 0.0);
        assert_eq!(blocks[1].height, theme.card_height);
        assert!(
            blocks[2].y >= blocks[1].y + theme.card_height,
            "and the picture after it starts again below the card"
        );
        assert_eq!(blocks[2].x, 0.0);
    }

    /// A picture wider than the column is drawn no wider than the column --
    /// and no taller than that leaves it.
    ///
    /// The width was clamped on its own, so a column narrower than the
    /// picture did not shrink it, it flattened it: reported by opening the
    /// thread pane, where the same screenshot went from 444x95 to 279x95.
    /// The reserved height is what the drawing stretches the picture into,
    /// so the ratio has to be settled here.
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
        let wanted = 100.0 * theme.text_width() / 4000.0;
        assert!(
            (block.height - wanted).abs() < 0.01,
            "{} tall where the ratio wants {wanted}",
            block.height
        );
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

    /// A build notice from a webhook: two lines, in an attachment, with the
    /// body left empty.
    fn webhook(theme: &Theme, fonts: &mut Fonts) -> RowLayout {
        let mut said = post(Vec::new());
        said.bot = true;
        said.author_name = "northwindbot".into();
        said.body_is_attachment_only = true;
        said.attachments = vec![matterless_render::Attachment {
            color: None,
            pretext: Vec::new(),
            title: None,
            text: vec![Node::Paragraph {
                children: vec![
                    Node::Text {
                        value: "Client Build Is Fixed".into(),
                    },
                    Node::SoftBreak,
                    Node::Text {
                        value: "For stream //VoyagerP4/Dev/Main".into(),
                    },
                ],
            }],
            title_link: None,
            fields: Vec::new(),
        }];
        lay_out(fonts, &Row::Post { post: said }, theme)
    }

    /// The card's padding is room this reserved, not room it borrows from
    /// whatever is above.
    ///
    /// What is above an attachment is the name of whoever sent it, so a card
    /// that reached upwards for its padding was drawn over the author and
    /// their avatar.
    #[test]
    fn a_webhook_card_leaves_itself_room_to_be_drawn_in() {
        let mut fonts = Fonts::new();
        let theme = Theme::default();
        let laid = webhook(&theme, &mut fonts);
        let header = laid
            .blocks
            .iter()
            .find(|block| block.kind == Kind::Header)
            .expect("a header");
        let card = laid
            .blocks
            .iter()
            .find(|block| block.kind == Kind::Card)
            .expect("a card");
        assert!(
            card.y >= header.y + header.height,
            "the card starts {}px above the header's foot, so it is drawn over the name",
            header.y + header.height - card.y
        );
        let first = words_in(&laid, card).next().expect("something in the card");
        assert!(
            first.y - card.y >= theme.attached_padding,
            "the card's first words are {}px inside its top, not {}",
            first.y - card.y,
            theme.attached_padding
        );
    }

    /// The words inside a card, in order.
    fn words_in<'a>(laid: &'a RowLayout, card: &'a Block) -> impl Iterator<Item = &'a Block> {
        laid.blocks.iter().filter(move |block| {
            block.kind == Kind::Text
                && block.y >= card.y
                && block.y + block.height <= card.y + card.height + 0.5
        })
    }

    /// A webhook post with one attachment built from these parts.
    fn attached(
        theme: &Theme,
        fonts: &mut Fonts,
        title: Option<(&str, Option<&str>)>,
        text: &str,
        fields: Vec<matterless_render::AttachmentField>,
    ) -> RowLayout {
        let mut said = post(Vec::new());
        said.bot = true;
        said.author_name = "jira".into();
        said.body_is_attachment_only = true;
        said.attachments = vec![matterless_render::Attachment {
            color: Some("#e01e5a".into()),
            pretext: Vec::new(),
            title: title.map(|(title, _)| title.to_string()),
            title_link: title.and_then(|(_, link)| link.map(str::to_string)),
            text: matterless_render::markdown::parse(text),
            fields,
        }];
        lay_out(fonts, &Row::Post { post: said }, theme)
    }

    fn field(title: &str, value: &str, short: bool) -> matterless_render::AttachmentField {
        matterless_render::AttachmentField {
            title: title.into(),
            value: matterless_render::markdown::parse(value),
            short,
        }
    }

    /// What an attachment says is markdown, and it is drawn as markdown: the
    /// steps of a Jira bug keep their bullets and read as three lines, not as
    /// one run-on paragraph, and an emphasis stays emphasised.
    #[test]
    fn an_attachment_keeps_its_markdown() {
        let mut fonts = Fonts::new();
        let theme = Theme::default();
        let laid = attached(
            &theme,
            &mut fonts,
            None,
            "Steps to reproduce\n\n- open the map\n- zoom out\n- crash\n\n*seen twice*",
            Vec::new(),
        );
        let card = laid
            .blocks
            .iter()
            .find(|block| block.kind == Kind::Card)
            .expect("a card");
        let inside: Vec<&Block> = words_in(&laid, card).collect();
        let bullets = inside
            .iter()
            .filter(|block| {
                block
                    .spans
                    .iter()
                    .any(|span| span.text.contains('\u{2022}'))
            })
            .count();
        assert_eq!(bullets, 3, "one bullet for each step");
        assert!(
            inside
                .iter()
                .flat_map(|block| &block.spans)
                .any(|span| span.italic && span.text.contains("seen twice")),
            "the emphasis was flattened"
        );
    }

    /// A title with somewhere to go is a link, as the official client draws the
    /// bug's id and summary at the top of a Jira notice.
    #[test]
    fn an_attachment_title_is_a_link_when_it_has_one() {
        let mut fonts = Fonts::new();
        let theme = Theme::default();
        let laid = attached(
            &theme,
            &mut fonts,
            Some((
                "CUR-1234 The map crashes",
                Some("https://example.invalid/CUR-1234"),
            )),
            "",
            Vec::new(),
        );
        let title = laid
            .blocks
            .iter()
            .flat_map(|block| &block.spans)
            .find(|span| span.text == "CUR-1234 The map crashes")
            .expect("the title");
        assert!(title.bold);
        assert_eq!(
            title.press,
            Some(Press::Link("https://example.invalid/CUR-1234".into()))
        );
        // And never a scheme this window would hand to the shell.
        let laid = attached(
            &theme,
            &mut fonts,
            Some(("Run me", Some("file:///C:/Windows/System32/calc.exe"))),
            "",
            Vec::new(),
        );
        let title = laid
            .blocks
            .iter()
            .flat_map(|block| &block.spans)
            .find(|span| span.text == "Run me")
            .expect("the title");
        assert_eq!(title.press, None);
    }

    /// Two short fields share a row, name over value; a long one takes the
    /// whole width below them.
    #[test]
    fn short_fields_sit_side_by_side() {
        let mut fonts = Fonts::new();
        let theme = Theme::default();
        let laid = attached(
            &theme,
            &mut fonts,
            None,
            "",
            vec![
                field("Assignee", "ada", true),
                field("Priority", "High", true),
                field(
                    "Description",
                    "It crashes when the map is zoomed out.",
                    false,
                ),
            ],
        );
        let named = |name: &str| {
            laid.blocks
                .iter()
                .find(|block| block.spans.iter().any(|span| span.text == name))
                .unwrap_or_else(|| panic!("no {name}"))
        };
        let (assignee, priority, description) =
            (named("Assignee"), named("Priority"), named("Description"));
        assert_eq!(assignee.y, priority.y, "the two short fields are one row");
        assert!(
            priority.x > assignee.x + assignee.wrap - 1.0,
            "and side by side"
        );
        assert!(description.y > assignee.y, "the long one is below them");
        assert_eq!(description.x, assignee.x, "starting at the card's left");
        assert!(assignee.spans[0].bold, "a field's name is bold");
        let value = named("ada");
        assert!(value.y > assignee.y, "the value is under its name");
        assert_eq!(value.x, assignee.x);
    }

    /// The card spans everything in it, so its ground and its bar reach the
    /// last line and no further than the next thing on the row.
    #[test]
    fn a_card_spans_what_is_in_it() {
        let mut fonts = Fonts::new();
        let theme = Theme::default();
        let laid = attached(
            &theme,
            &mut fonts,
            Some(("A title", None)),
            "one line\n\nand another",
            vec![field("Status", "Open", true)],
        );
        let card = laid
            .blocks
            .iter()
            .find(|block| block.kind == Kind::Card)
            .expect("a card");
        let last = laid
            .blocks
            .iter()
            .filter(|block| block.kind == Kind::Text && block.y >= card.y)
            .map(|block| block.y + block.height)
            .fold(0.0_f32, f32::max);
        assert!(
            (card.y + card.height - last - theme.attached_padding).abs() < 0.5,
            "the card ends {}px below its last line, not its padding of {}",
            card.y + card.height - last,
            theme.attached_padding
        );
        assert!(
            laid.height >= card.y + card.height,
            "and the row makes room for it"
        );
    }

    /// And it is written in lines, which is how it has to be read.
    #[test]
    fn a_webhook_keeps_the_lines_it_was_written_in() {
        let mut fonts = Fonts::new();
        let theme = Theme::default();
        let laid = webhook(&theme, &mut fonts);
        let card = laid
            .blocks
            .iter()
            .find(|block| block.kind == Kind::Card)
            .expect("a card");
        let attached = words_in(&laid, card).next().expect("some words");
        // Across its spans: through the body's path a soft break is a span of
        // its own rather than a character inside one.
        let said: String = attached
            .spans
            .iter()
            .map(|span| span.text.as_str())
            .collect();
        assert!(
            said.contains('\n'),
            "the two lines were run together: {said:?}"
        );
        // And the height reserved is for both of them, not for the one line
        // they used to be joined into.
        assert_eq!(attached.lines, 2);
        assert_eq!(attached.height, 2.0 * theme.line_height);
    }

    /// A stack trace is cut short, and the rest is offered rather than drawn.
    ///
    /// The Sentry crash notices carry one, and drawn whole it is the window:
    /// the conversation around it cannot be read past it.
    #[test]
    fn a_long_code_block_is_cut_short_and_the_rest_offered() {
        let mut fonts = Fonts::new();
        let theme = Theme::default();
        let trace = (0..40)
            .map(|at| format!("  at frame {at}"))
            .collect::<Vec<_>>()
            .join("\n");
        let said = post(vec![Node::CodeBlock {
            language: None,
            value: trace.clone(),
        }]);
        let row = Row::Post { post: said };

        let cut = lay_out(&mut fonts, &row, &theme);
        let block = cut
            .blocks
            .iter()
            .find(|block| block.kind == Kind::Code)
            .expect("a code block");
        assert_eq!(
            block.lines, theme.code_lines_shown,
            "the block was not cut short"
        );
        // And the way to the rest of it, which is what makes cutting it
        // something other than losing it.
        let offer = cut
            .blocks
            .iter()
            .flat_map(|block| block.spans.iter())
            .find(|span| matches!(span.press, Some(Press::Whole(_))))
            .expect("nothing offers the rest");
        assert!(
            offer
                .text
                .contains(&format!("{}", 40 - theme.code_lines_shown)),
            "the offer does not say how much is left: {:?}",
            offer.text
        );

        // Inside the block, over the last lines of the listing rather than
        // under them: the listing carries on behind the offer and fades out
        // around it, which is what says there is more of it.
        let offer_box = cut
            .blocks
            .iter()
            .find(|one| {
                one.spans
                    .iter()
                    .any(|span| matches!(span.press, Some(Press::Whole(_))))
            })
            .expect("nothing offers the rest");
        let floor = block.y + block.height;
        assert_eq!(
            floor,
            offer_box.y + offer_box.height / 2.0,
            "the ground does not end halfway down the offer, so it does not straddle it"
        );
        // Over the *last* of it, not the middle: the lines it covers are the
        // ones nobody can finish reading anyway.
        let last = block.y
            + theme.code_padding_y
            + (theme.code_lines_shown as f32 - 1.0) * theme.code_line_height;
        assert!(
            offer_box.y + offer_box.height > last,
            "the offer is above the last line shown"
        );
        // And across the middle of it, which is where an end is looked for.
        let middle = offer_box.x + offer_box.wrap / 2.0;
        assert!(
            (middle - theme.text_width() / 2.0).abs() < theme.indent,
            "the offer sits at {middle}, not across the middle of {}",
            theme.text_width()
        );
        // On a ground of its own, because words set straight over code are two
        // lines of text in one place and read as neither.
        let pill = cut
            .blocks
            .iter()
            .find(|one| one.kind == Kind::Pill)
            .expect("the offer is set straight over the listing");
        assert!(
            pill.x <= offer_box.x && pill.x + pill.wrap >= offer_box.x + offer_box.wrap,
            "the ground is narrower than the words on it"
        );
        assert_eq!(pill.y, offer_box.y, "the ground is not behind the words");

        // And the row is tall enough for the half of the offer that hangs
        // below the ground, which the block's own height does not cover.
        assert!(
            cut.height >= offer_box.y + offer_box.height,
            "the row ends at {} and the offer at {}, so it is drawn outside its row",
            cut.height,
            offer_box.y + offer_box.height
        );

        // With the listing fading out behind it rather than beside it, and
        // reaching full strength exactly where the text stops.
        //
        // Not where the *block* stops: the gradient would then still be seven
        // parts in ten along as it passed the last line, leaving that line a
        // third readable under the offer however far the fade ran on below it.
        let fade = cut
            .blocks
            .iter()
            .find(|one| one.kind == Kind::Fade)
            .expect("the listing stops dead rather than fading");
        let text_ends =
            block.y + theme.code_padding_y + theme.code_lines_shown as f32 * theme.code_line_height;
        assert!(
            fade.y < offer_box.y,
            "the fade starts below the offer, so the words behind it are untouched"
        );
        assert_eq!(
            fade.y + fade.height,
            text_ends,
            "the fade does not reach its full strength where the last line ends"
        );

        // Opened, it is all there, and the offer turns into its opposite.
        let whole = lay_out_opened(&mut fonts, &row, &theme, true);
        let block = whole
            .blocks
            .iter()
            .find(|block| block.kind == Kind::Code)
            .expect("a code block");
        assert_eq!(block.lines, 40);
        assert!(whole.height > cut.height, "the row did not grow");
        assert!(
            whole
                .blocks
                .iter()
                .flat_map(|block| block.spans.iter())
                .any(|span| matches!(span.press, Some(Press::Whole(_)))),
            "there is no way back to the short one"
        );
    }

    /// A short one is left alone, and offers nothing.
    #[test]
    fn a_short_code_block_is_left_whole() {
        let mut fonts = Fonts::new();
        let theme = Theme::default();
        let said = post(vec![Node::CodeBlock {
            language: None,
            value: "cargo test\ncargo clippy".into(),
        }]);
        let laid = lay_out(&mut fonts, &Row::Post { post: said }, &theme);
        assert!(
            !laid
                .blocks
                .iter()
                .flat_map(|block| block.spans.iter())
                .any(|span| matches!(span.press, Some(Press::Whole(_)))),
            "a two-line block offers to show the rest of itself"
        );
    }

    /// A written `---` is a rule, not a blank line.
    ///
    /// It used to reserve the room a rule takes and then draw nothing in it,
    /// so the sections of a long notice ran together with an unexplained gap
    /// where each divider belonged.
    #[test]
    fn a_written_rule_is_drawn_as_one() {
        let mut fonts = Fonts::new();
        let theme = Theme::default();
        let said = post(vec![text("before"), Node::Rule, text("after")]);
        let laid = lay_out(&mut fonts, &Row::Post { post: said }, &theme);
        let rule = laid
            .blocks
            .iter()
            .find(|block| block.kind == Kind::Rule)
            .expect("no rule: a blank line again");
        assert_eq!(rule.height, theme.rule_height);
        assert!(rule.spans.is_empty(), "a rule has no words in it");
        // Between the two lines it divides, and not on top of either.
        let lines: Vec<&Block> = laid
            .blocks
            .iter()
            .filter(|block| block.kind == Kind::Text && !block.spans.is_empty())
            .collect();
        assert!(
            lines[0].y < rule.y && rule.y < lines[1].y,
            "the rule is not between the sections it separates"
        );
    }

    /// A list is set with no gap between its items, and one after the last.
    ///
    /// The gap belongs to the list rather than to each item of it. Taken
    /// between every pair of lines, a five-item list read as nearly
    /// double-spaced against the official client.
    #[test]
    fn a_list_is_not_double_spaced() {
        let mut fonts = Fonts::new();
        let theme = Theme::default();
        let item = |what: &str| {
            vec![Node::Paragraph {
                children: vec![Node::Text { value: what.into() }],
            }]
        };
        let said = post(vec![
            Node::List {
                ordered: false,
                items: vec![item("one"), item("two"), item("three")],
            },
            text("after the list"),
        ]);
        let laid = lay_out(&mut fonts, &Row::Post { post: said }, &theme);
        // The item lines, which are the ones carrying words at the list's
        // indent -- not the markers, which sit outside it.
        let items: Vec<&Block> = laid
            .blocks
            .iter()
            .filter(|block| block.kind == Kind::Text && block.wrap > theme.indent)
            .collect();
        assert_eq!(items.len(), 4, "three items and the line after them");
        for pair in items[..3].windows(2) {
            assert_eq!(
                pair[1].y - (pair[0].y + pair[0].height),
                0.0,
                "two items of one list are a gap apart"
            );
        }
        // And the line after the list is a gap below it, so the list still
        // reads as a thing that ended.
        assert_eq!(
            items[3].y - (items[2].y + items[2].height),
            theme.block_gap,
            "the list does not end with a gap"
        );
    }

    /// The message starts a gap below the name, not against it.
    ///
    /// Every other pair of blocks in a message is separated by `block_gap`,
    /// and the header was the one join that got nothing: the first line began
    /// exactly where the name's box ended. Which is invisible for plain words
    /// -- a line box is taller than its letters -- and not invisible at all
    /// for anything carrying a ground of its own, since `code` on that first
    /// line was drawn a pixel under the name and into its descenders.
    #[test]
    fn the_first_line_of_a_message_clears_the_name_above_it() {
        let mut fonts = Fonts::new();
        let theme = Theme::default();
        let said = post(vec![Node::Paragraph {
            children: vec![
                Node::Text {
                    value: "run ".into(),
                },
                Node::InlineCode {
                    value: "cargo test".into(),
                },
                Node::Text {
                    value: " first".into(),
                },
            ],
        }]);
        let laid = lay_out(&mut fonts, &Row::Post { post: said }, &theme);
        let header = laid
            .blocks
            .iter()
            .find(|block| block.kind == Kind::Header)
            .expect("a header");
        let first = laid
            .blocks
            .iter()
            .find(|block| block.kind == Kind::Text)
            .expect("a line");
        assert_eq!(
            first.y - (header.y + header.height),
            theme.block_gap,
            "the message does not start a block's gap below the name"
        );
        // And the ground behind `code`, which is the line inset by a pixel,
        // clears the name's box rather than starting under it.
        assert!(
            first.y + 1.0 > header.y + header.height,
            "the code ground starts inside the header"
        );
    }
}
