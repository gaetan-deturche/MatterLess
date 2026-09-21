//! Where a message is written.
//!
//! The first widget in this port that has to hold state between frames, and the
//! reason the input model grew modifiers. A caret is not a character offset: it
//! is a position inside a *shaped* run, which is what makes an accented letter
//! one step rather than two and what puts the caret in the right place in a
//! line that has been laid out with kerning.
//!
//! So the shaping is not redone from a string each frame -- `cosmic-text`'s own
//! editor owns the buffer and the cursor together, and moves them together.
//! Hand-rolling that would mean re-deriving grapheme clusters, word boundaries
//! and bidirectional runs, all of which it already has right.

use cosmic_text::{
    Action, Attrs, Buffer, Change, Cursor, Edit, Editor, Metrics, Motion, Selection, Shaping,
};
use matterless_layout::Fonts;
use matterless_paint::Run;
use matterless_ui::input::{Input, Key};
use matterless_ui::{Placed, Rect};
use matterless_widgets::{Canvas, Panel};

/// The text being written, and where the caret is in it.
pub struct Composer {
    editor: Editor<'static>,
    /// What this box answers to. A name rather than a constant because the
    /// channel and the thread each have one, and a click has to land in the
    /// right one.
    pub name: String,
    /// Drawn when there is nothing written, so the box says what it is for.
    pub placeholder: String,
    /// What the row along the bottom offers.
    pub tools_for: Tools,
    /// What is attached and waiting to go with the next message, by name.
    ///
    /// Set by the window, which is what holds the uploads and decides which
    /// conversation they belong to. The box only has to say they are there
    /// and offer to take one off -- a file waiting invisibly is worse than
    /// one sent by accident, because nobody can undo what they cannot see.
    pub waiting: Vec<String>,
    /// True once anything has been typed, so an empty buffer can be told from
    /// one the reader has emptied on purpose.
    pub touched: bool,
    /// How far the text is scrolled inside the box, in pixels.
    ///
    /// A box that stops growing at `MAX_LINES` and holds more than that is
    /// showing a window onto its text, and four things have to agree about
    /// which window: the glyphs, the selection behind them, the caret, and the
    /// press that puts the caret somewhere. Before this there was no window at
    /// all -- the text ran on past the bottom edge and was drawn over whatever
    /// happened to be under the box.
    scroll: f32,
    /// The bar down the side of the text, and what drags it.
    ///
    /// The same one the conversation and the lists use: a box holding more
    /// than it shows has to say so, and say how much more.
    bar: crate::scrollbar::Scrollbar,
    /// What has been done, and what has been undone, so both can be walked.
    ///
    /// Changes rather than copies of the text: `cosmic-text` hands back what
    /// an edit *was* and can reverse one, so a stack of those is exact and
    /// costs the words that moved rather than the whole message each time.
    ///
    /// Redo is emptied by any fresh edit, which is what every editor does: a
    /// reader who undid something and then typed has chosen a different
    /// future, and offering to redo the one they left would be offering to
    /// throw away what they just wrote.
    done: Vec<Change>,
    undone: Vec<Change>,
    /// A field, not a message box: no paperclip, no Send, and none of the
    /// height they take.
    ///
    /// The query boxes -- search, the switcher, the emoji picker -- are all
    /// this. They were message composers, which meant each of them offered to
    /// attach a file to a search and reserved 34 pixels to do it in.
    pub plain: bool,
}

/// What a box's row of buttons is for.
///
/// The row is the same shape either way -- a strip along the bottom of the
/// box with something on each end -- and what is on it depends on whether the
/// words in the box are on their way out or already said.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tools {
    /// A paperclip and Send: a message being written.
    Writing,
    /// Save and Cancel: a message being changed.
    Editing,
}

/// A button inside the box.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Button {
    /// Pick a file, for a reader who would rather not drag one.
    Attach,
    /// Keep the change, for a reader editing a message.
    Save,
    /// Drop it, which is the button this box was missing: Escape cancels and
    /// nothing on screen said so.
    Cancel,
    /// Send what is typed, for a reader who would rather not press return.
    Send,
    /// Take one of the waiting attachments off again.
    Unattach(usize),
}

/// What the channel's own composer is called.
pub const NAME: &str = "composer";

const SIZE: f32 = 14.0;
const LINE: f32 = 20.0;
const PADDING: f32 = 10.0;
/// The same two on a field, which the app sets tighter: `input { padding: 6px
/// 8px }` inside a `header { padding: 8px 10px }`, which is a 46px strip.
const FIELD_PADDING: f32 = 6.0;
const FIELD_MARGIN: f32 = 7.0;
/// The margin outside the box, matching the stream's own gutter.
const MARGIN: f32 = 12.0;
/// How tall it is allowed to grow before the text scrolls inside it. Eight
/// lines is a long message; past that the composer would be eating the
/// conversation it is a reply to.
const MAX_LINES: usize = 8;
/// What the stylesheet cuts the box's corners by.
const BOX: f32 = 8.0;
/// How far the box is lifted off the conversation behind it.
///
/// Small: this is furniture that stays put, not a card that has just appeared
/// over the window, and the shadow is there to separate it from the last
/// message rather than to announce it. It has to stay inside `MARGIN`, which
/// is the only room there is between the box and the edge of its own clip.
const DROP: f32 = 6.0;
/// The buttons inside the box: one to attach, one to send.
const BUTTON: f32 = 26.0;
const SEND: f32 = 52.0;
/// The way out of an edit, which is a word rather than a mark.
///
/// Wider than Send because "Cancel" is a longer word, and there is nothing
/// gained by making two buttons the same width when the words in them differ.
const CANCEL: f32 = 60.0;
/// The row they sit on, along the bottom of the box.
const TOOLS: f32 = 34.0;
/// The row of waiting attachments, and the widest one of them.
const WAITING: f32 = 26.0;
const CHIP: f32 = 180.0;
/// What is kept clear at the ends of that row, and between two of them.
const CHIP_MARGIN: f32 = 6.0;
const CHIP_GAP: f32 = 4.0;
/// What the attach button shows.
///
/// A sheet of paper rather than the paperclip every client uses, because there
/// is no paperclip in line art: the fonts here have one glyph for it and it is
/// a coloured bitmap. A control is not content, and a coloured one reads as
/// something somebody sent rather than as something to press.
const CLIP: &str = matterless_layout::marks::ATTACH;

/// Whether an edit carries straight on from the one before it.
///
/// A run of typing is one step to undo rather than one step per letter: a
/// reader who typed a word and pressed undo wants the word gone, not its last
/// character. Only insertions, and only where the new one begins exactly where
/// the last one ended -- a caret moved elsewhere starts a step of its own.
///
/// The run ends *after* whitespace rather than before it, so a step is a word
/// and the space that follows it. Ending before would leave the space as a
/// step of its own, which is an undo that visibly does nothing.
fn carries_on(last: &Change, next: &Change) -> bool {
    let (Some(ended), Some(starts)) = (last.items.last(), next.items.first()) else {
        return false;
    };
    ended.insert
        && ended.end == starts.start
        && !ended.text.chars().any(char::is_whitespace)
        && next.items.iter().all(|item| item.insert)
}

impl Default for Composer {
    fn default() -> Self {
        Self::new(NAME)
    }
}

impl Composer {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            editor: Editor::new(Buffer::new_empty(Metrics::new(SIZE, LINE))),
            name: name.into(),
            placeholder: "Write a message... (Enter to send, Shift+Enter for a new line)"
                .to_string(),
            tools_for: Tools::Writing,
            waiting: Vec::new(),
            touched: false,
            scroll: 0.0,
            bar: crate::scrollbar::Scrollbar::default(),
            done: Vec::new(),
            undone: Vec::new(),
            plain: false,
        }
    }

    /// The same box as a field: no paperclip, no Send, and none of their room.
    pub fn plain(mut self) -> Self {
        self.plain = true;
        self
    }

    /// The text as it stands.
    pub fn text(&self) -> String {
        self.editor.with_buffer(|buffer| {
            buffer
                .lines
                .iter()
                .map(|line| line.text())
                .collect::<Vec<&str>>()
                .join("\n")
        })
    }

    fn is_empty(&self) -> bool {
        self.editor
            .with_buffer(|buffer| buffer.lines.iter().all(|line| line.text().is_empty()))
    }

    /// Shapes the text for this width. Must run before the height is asked for
    /// or the box is drawn, because both are answers about the shaping.
    pub fn lay_out(&mut self, fonts: &mut Fonts, width: f32) {
        // A caret is a place in a laid-out line, and `new` builds the buffer
        // with no lines at all -- so a box nobody had typed in yet had nowhere
        // to put one, and showed none until the first keystroke made a line.
        // Which is the one moment a reader does not need telling the box is
        // theirs: they have just written in it.
        //
        // Only when there are none. Setting the text every time would empty
        // the box on every layout, which is every frame of a window resize.
        if self.editor.with_buffer(|buffer| buffer.lines.is_empty()) {
            self.editor.with_buffer_mut(|buffer| {
                buffer.set_text("", &Attrs::new(), Shaping::Advanced, None);
            });
        }
        let inner = (width - self.margin() * 2.0 - self.padding() * 2.0).max(1.0);
        self.editor.with_buffer_mut(|buffer| {
            buffer.set_size(Some(inner), None);
        });
        self.editor.shape_as_needed(fonts.system_mut(), false);
        // The width just changed, which re-wraps: a caret that was on the last
        // line of the window can be two lines below it now, with nobody having
        // touched the keyboard.
        self.follow_caret();
    }

    /// The placeholder, cut to the room there is.
    ///
    /// Left to the clip it ended mid-letter: opening a thread narrows this box
    /// and the hint read "Shift+Enter for a ne", which is not a shorter
    /// sentence but a broken one. An ellipsis says the words go on.
    fn hint(&self, fonts: &mut Fonts, width: f32) -> String {
        matterless_layout::elided(
            fonts,
            &self.placeholder,
            width,
            matterless_layout::Style {
                size: SIZE,
                line_height: LINE,
                bold: false,
                italic: false,
                mono: false,
            },
        )
    }

    /// Inside the box, where the words go: what the caret is placed against,
    /// what a click is measured from, and what the text is drawn at.
    fn inner(&self, within: Rect) -> Rect {
        self.box_of(within).inset(self.padding())
    }

    /// What to hand `draw` and `react` so the box they paint lands exactly on
    /// `field`.
    ///
    /// A field paints itself *inside* what it is given, with room left over
    /// round it. Everywhere else that room is part of the panel the field sits
    /// in, and the caller can hand over the whole strip. The app's header has
    /// no room to spare -- it is 44 tall and a field wants 46 -- so there the
    /// caller says where the box goes and this works back to the rect that
    /// puts it there. Handing the box's own rect over instead squeezed
    /// everything inside it: measured, a 240x28 field came out as a 226x14
    /// border with two pixels of room for a twenty-pixel line, so the words
    /// sat across the bottom edge.
    pub fn around(&self, field: Rect) -> Rect {
        let margin = self.margin();
        Rect::new(
            field.x - margin,
            field.y - margin,
            field.width + margin * 2.0,
            field.height + margin * 2.0,
        )
    }

    /// How tall a box this field paints, given the room to paint it in.
    ///
    /// The inverse of `around`, for a caller that has to reserve the room
    /// before it knows where the box goes.
    pub fn box_height(&self) -> f32 {
        self.height() - self.margin() * 2.0
    }

    /// Where the caret sits inside the box, if it has one to show.
    ///
    /// Answered here rather than worked out while drawing, so a test can ask.
    /// `None` when the editor has no laid-out line to put it on, which is what
    /// a field that has never been laid out looks like.
    pub fn caret(&self, within: Rect) -> Option<(f32, f32)> {
        let inner = self.inner(within);
        let (x, y) = self.editor.cursor_position()?;
        Some((inner.x + x as f32, inner.y + y as f32 - self.scroll))
    }

    /// How many lines the text occupies, capped at what the box will show.
    fn lines(&self) -> usize {
        self.written().clamp(1, MAX_LINES)
    }

    /// Every line the text takes, which past `MAX_LINES` is not the number
    /// shown.
    fn written(&self) -> usize {
        self.editor
            .with_buffer(|buffer| buffer.layout_runs().count())
            .max(1)
    }

    /// How far the text can be scrolled inside the box.
    fn reach(&self) -> f32 {
        ((self.written() - self.lines()) as f32 * LINE).max(0.0)
    }

    /// Every run, for a test that has to see how the text is broken up.
    #[cfg(test)]
    pub fn runs(&self) -> Vec<(usize, f32, usize, String)> {
        self.editor.with_buffer(|buffer| {
            buffer
                .layout_runs()
                .map(|run| {
                    (
                        run.line_i,
                        run.line_top,
                        run.glyphs.len(),
                        run.text.chars().take(24).collect::<String>(),
                    )
                })
                .collect()
        })
    }

    /// The selection's own bounds, for a test that has to see them.
    #[cfg(test)]
    pub fn bounds(&self) -> Option<((usize, usize), (usize, usize))> {
        let (start, end) = self.editor.selection_bounds()?;
        Some(((start.line, start.index), (end.line, end.index)))
    }

    /// Every rectangle the selection is drawn as: `(x, y, width)`, each one
    /// line tall.
    ///
    /// Worked out here rather than in `draw` so that what a test reads is
    /// what the window paints. The line filter below is the whole reason it
    /// is worth having in one place.
    pub fn selection_marks(&self, within: Rect) -> Vec<(f32, f32, f32)> {
        let inner = self.inner(within);
        let scroll = self.scroll;
        let mut out = Vec::new();
        if let Some((start, end)) = self.editor.selection_bounds() {
            self.editor.with_buffer(|buffer| {
                for run in buffer.layout_runs() {
                    // `highlight` only knows about the two lines a selection
                    // ends on: for any other line both of its tests
                    // short-circuit and every character comes back selected.
                    // Which lines are in range at all is the caller's to
                    // know, so a box of one line was right and a box of
                    // several lit up every line but the one being worked on.
                    if run.line_i < start.line || run.line_i > end.line {
                        continue;
                    }
                    for (x, width) in run.highlight(start, end) {
                        out.push((inner.x + x, inner.y + run.line_top - scroll, width));
                    }
                }
            });
        }
        out
    }

    /// What is selected, as text. `None` when nothing is.
    pub fn selection(&self) -> Option<String> {
        self.editor.copy_selection()
    }

    /// A drag of the bar down the side of the text.
    ///
    /// Apart from the wheel because the two arrive through different doors: a
    /// drag is a pointer move with something held, which the window answers
    /// by reacting to the frame; a wheel turn is an event of its own with a
    /// path of its own. One method answering both was applied twice whenever
    /// that path reacted as well, and scrolled at double speed.
    ///
    /// Only the message boxes call either: a field is one line and has
    /// nowhere to scroll to.
    pub fn dragged(&mut self, input: &Input, within: Rect) {
        let reach = self.reach();
        let over = self.over(within);
        if let Some(scroll) = self.bar.react(&self.name, input, over, self.scroll, reach) {
            self.scroll = scroll.clamp(0.0, reach);
        }
    }

    /// A turn of the wheel over the box.
    ///
    /// The frame's boxes, because a wheel is answered by whichever panel the
    /// pointer is over and only they can say which that is -- the same
    /// question the conversation and the lists ask.
    pub fn wheeled(&mut self, input: &Input, placed: &[Placed], within: Rect) {
        let _ = within;
        if let Some((_, y)) = input.wheel_over(placed, |name| name == self.name) {
            self.scroll = (self.scroll - y).clamp(0.0, self.reach());
        }
    }

    /// The parts of a box that answer to where the pointer is: the bar down
    /// the side of the text, and what is waiting to be sent with it.
    ///
    /// Their own call rather than part of `draw`, because `draw` has no input
    /// to hand and six of the eight boxes in this window are one-line fields
    /// with nothing to scroll and nothing attached. The two that can have
    /// either ask for it.
    pub fn draw_over(&self, into: &mut Canvas<'_>, input: &Input, within: Rect) {
        let reach = self.reach();
        if self.bar.needed(reach) {
            self.bar.draw(
                into,
                &self.name,
                input,
                self.over(within),
                self.scroll,
                reach,
            );
        }
        let Canvas {
            scene,
            painter,
            fonts,
            palette,
        } = into;
        // What is waiting to go with this message, above the tools. A press
        // takes one off: the window holds the uploads, so this only says
        // which one was pressed.
        for at in 0..self.waiting.len() {
            let Some(chip) = self.waiting_at(within, at) else {
                continue;
            };
            let under = input.hovered() == Some(format!("{}/unattach/{at}", self.name).as_str());
            scene.rounded(
                chip.x,
                chip.y,
                chip.width,
                chip.height,
                if under { palette.hover } else { palette.raised },
                5.0,
            );
            let room = (chip.width - 22.0).max(10.0);
            let said = matterless_layout::elided(
                fonts,
                &self.waiting[at],
                room,
                matterless_layout::Style {
                    size: 12.0,
                    line_height: 16.0,
                    bold: false,
                    italic: false,
                    mono: false,
                },
            );
            let glyphs = painter.run(
                fonts,
                &said,
                chip.x + 7.0,
                chip.y + 2.0,
                Run::label(f32::MAX),
            );
            scene.glyphs(glyphs, palette.soft, palette.faint);
            // The way to take it off, on its own end of the chip.
            let cross = painter.run(
                fonts,
                matterless_layout::marks::CLOSE,
                chip.right() - 15.0,
                chip.y + 3.0,
                Run::mark(11.0),
            );
            scene.glyphs(
                cross,
                if under { palette.ink } else { palette.faint },
                palette.faint,
            );
        }
    }

    /// The window the text is shown through: the lines on screen, and nothing
    /// of the paperclip and Send below them.
    fn over(&self, within: Rect) -> Rect {
        let inner = self.inner(within);
        Rect::new(inner.x, inner.y, inner.width, self.lines() as f32 * LINE)
    }

    /// Keeps the caret inside the window the box shows.
    ///
    /// Called wherever the text or the caret can have moved, rather than only
    /// on a keystroke: a caret nobody can see is a box being typed into
    /// blind, and re-wrapping at a new width moves one without anybody
    /// touching the keyboard.
    fn follow_caret(&mut self) {
        let shown = self.lines() as f32 * LINE;
        let Some((_, y)) = self.editor.cursor_position() else {
            self.scroll = self.scroll.clamp(0.0, self.reach());
            return;
        };
        let top = y as f32;
        if top < self.scroll {
            self.scroll = top;
        } else if top + LINE > self.scroll + shown {
            self.scroll = top + LINE - shown;
        }
        self.scroll = self.scroll.clamp(0.0, self.reach());
    }

    /// The height the strip needs, which grows with the message.
    pub fn height(&self) -> f32 {
        let tools = if self.plain { 0.0 } else { TOOLS };
        self.lines() as f32 * LINE
            + self.padding() * 2.0
            + self.margin() * 2.0
            + tools
            + self.held()
    }

    /// The room the waiting attachments take, which is a row or nothing.
    ///
    /// One row however many there are: they are names on a line and they run
    /// out of width long before they run out of box, which is what the
    /// eliding is for.
    fn held(&self) -> f32 {
        match self.plain || self.waiting.is_empty() {
            true => 0.0,
            false => WAITING,
        }
    }

    /// Where the waiting attachments sit: between the words and the tools.
    fn shelf(&self, within: Rect) -> Rect {
        let tools = self.tools(within);
        Rect::new(tools.x, tools.y - self.held(), tools.width, self.held())
    }

    /// Where one of them sits on that shelf.
    ///
    /// The gaps and the margins come out of the width before it is shared, so
    /// that however many are waiting the last one ends inside the box: a chip
    /// drawn past the edge can be neither read nor clicked off again.
    fn waiting_at(&self, within: Rect, at: usize) -> Option<Rect> {
        let shelf = self.shelf(within);
        let count = self.waiting.len();
        if shelf.height <= 0.0 || at >= count {
            return None;
        }
        let room = (shelf.width - CHIP_MARGIN * 2.0 - CHIP_GAP * (count - 1) as f32).max(0.0);
        let each = (room / count as f32).min(CHIP);
        Some(Rect::new(
            shelf.x + CHIP_MARGIN + at as f32 * (each + CHIP_GAP),
            shelf.y + 2.0,
            each,
            WAITING - 6.0,
        ))
    }

    /// Inside the box, and around it. Tighter on a field than on a message.
    fn padding(&self) -> f32 {
        if self.plain { FIELD_PADDING } else { PADDING }
    }

    /// The clear room between the box and the edges of its strip.
    ///
    /// Public because it is blank ground somebody else can use: the typing
    /// pill sits in the band above the box, which belongs to the strip and
    /// has nothing drawn in it.
    pub fn margin(&self) -> f32 {
        if self.plain { FIELD_MARGIN } else { MARGIN }
    }

    /// The row of buttons along the bottom of the box, and the room it takes.
    ///
    /// Its own row rather than beside the text: a button level with the first
    /// line would be somewhere different once the box had grown to eight, and
    /// a reader should not have to find it again after typing.
    fn tools(&self, within: Rect) -> Rect {
        let outer = self.box_of(within);
        Rect::new(outer.x, outer.bottom() - TOOLS, outer.width, TOOLS)
    }

    /// The strip at the bottom of a panel, and what is left above it.
    /// Where the attach button sits, inside the box on the left.
    pub fn attach(&self, within: Rect) -> Rect {
        let tools = self.tools(within);
        Rect::new(tools.x + 6.0, tools.y + 4.0, BUTTON, BUTTON)
    }

    /// Where the send button sits, inside the box on the right.
    ///
    /// The same place Save takes when the box is changing a message rather
    /// than writing one: the button that does the thing is on the right in
    /// both, so the reader does not have to look for it twice.
    pub fn send(&self, within: Rect) -> Rect {
        let tools = self.tools(within);
        let width = match self.tools_for {
            Tools::Writing => SEND,
            Tools::Editing => SEND,
        };
        Rect::new(tools.right() - width - 6.0, tools.y + 4.0, width, BUTTON)
    }

    /// Where the way out sits: left of Save, and only while editing.
    ///
    /// `None` for a box a message is being written in. Nothing is being
    /// abandoned there -- the words have not been said yet -- and a Cancel
    /// beside Send would read as a way to unsay them.
    pub fn cancel(&self, within: Rect) -> Option<Rect> {
        if self.tools_for != Tools::Editing {
            return None;
        }
        let send = self.send(within);
        Some(Rect::new(
            send.x - CHIP_GAP - CANCEL,
            send.y,
            CANCEL,
            send.height,
        ))
    }

    /// The two buttons, so a pointer can land on them.
    pub fn boxes_in(&self, within: Rect) -> Vec<Placed> {
        if self.plain {
            return Vec::new();
        }
        let mut placed = self.bar.boxes(&self.name, self.over(within), self.reach());
        for at in 0..self.waiting.len() {
            if let Some(chip) = self.waiting_at(within, at) {
                placed.push(Placed {
                    name: format!("{}/unattach/{at}", self.name),
                    rect: chip,
                    depth: 4,
                });
            }
        }
        // The paperclip belongs to a message being written: there is
        // nothing to attach to a message that has already been said.
        if self.tools_for == Tools::Writing {
            placed.push(Placed {
                name: format!("{}/attach", self.name),
                rect: self.attach(within),
                depth: 3,
            });
        }
        if let Some(cancel) = self.cancel(within) {
            placed.push(Placed {
                name: format!("{}/cancel", self.name),
                rect: cancel,
                depth: 3,
            });
        }
        placed.push(Placed {
            name: format!("{}/send", self.name),
            rect: self.send(within),
            depth: 3,
        });
        placed
    }

    /// What a press on one of them means, if it landed on one.
    pub fn pressed(&self, input: &Input) -> Option<Button> {
        let clicked = input.clicked()?;
        match clicked.strip_prefix(&format!("{}/", self.name))? {
            "attach" => Some(Button::Attach),
            "cancel" => Some(Button::Cancel),
            // One name for the button on the right, because it is one button
            // in one place -- what it means is the box's business, not the hit
            // test's.
            "send" => Some(match self.tools_for {
                Tools::Writing => Button::Send,
                Tools::Editing => Button::Save,
            }),
            other => other
                .strip_prefix("unattach/")
                .and_then(|at| at.parse().ok())
                .map(Button::Unattach),
        }
    }

    pub fn strip(&self, within: Rect) -> Rect {
        let height = self.height().min(within.height);
        Rect::new(within.x, within.bottom() - height, within.width, height)
    }

    pub fn above(&self, within: Rect) -> Rect {
        let height = self.height().min(within.height);
        Rect::new(
            within.x,
            within.y,
            within.width,
            (within.height - height).max(0.0),
        )
    }

    /// The box itself, inside the strip's margin.
    fn box_of(&self, within: Rect) -> Rect {
        let strip = self.strip(within);
        Rect::new(
            strip.x + self.margin(),
            strip.y + self.margin(),
            (strip.width - self.margin() * 2.0).max(0.0),
            (strip.height - self.margin() * 2.0).max(0.0),
        )
    }

    /// The box as a hit target, so a click lands on it and the wheel over it
    /// does not scroll the conversation behind.
    pub fn boxes(&self, within: Rect) -> Vec<Placed> {
        vec![Placed {
            name: self.name.clone(),
            rect: self.strip(within),
            depth: 1,
        }]
    }

    /// Applies a frame's input. Answers the message when the reader sent one.
    ///
    /// Enter sends and Shift+Enter breaks the line, which is the convention
    /// every chat client shares and the opposite of what a text area does by
    /// default -- so it is decided here rather than left to the editor.
    pub fn react(
        &mut self,
        fonts: &mut Fonts,
        input: &Input,
        within: Rect,
        clipboard: &mut String,
    ) -> Option<String> {
        let inner = self.inner(within);
        let focused = input.focus() == Some(self.name.as_str());

        // The pointer puts the caret where it was clicked, and dragging from
        // there selects. Coordinates are the buffer's own, so the box's origin
        // comes off first.
        if let Some((x, y)) = input.pointer_at()
            && input.pressed() == Some(self.name.as_str())
        {
            // Through the scroll: the point is where the pointer is on
            // screen and the buffer counts from the first line of the text,
            // not from the first line on show.
            let at = ((x - inner.x) as i32, (y - inner.y + self.scroll) as i32);
            // The frame the button went down places the caret; every frame
            // after that drags a selection from it.
            // Two presses take the word under the pointer and three take the
            // line, which is what every other text box does.
            let action = match input.pressed_now() == Some(self.name.as_str()) {
                true => match input.clicks() {
                    1 => Action::Click { x: at.0, y: at.1 },
                    2 => Action::DoubleClick { x: at.0, y: at.1 },
                    _ => Action::TripleClick { x: at.0, y: at.1 },
                },
                false => Action::Drag { x: at.0, y: at.1 },
            };
            let system = fonts.system_mut();
            self.editor.action(system, action);
        }

        if !focused {
            return None;
        }

        let mods = input.mods();
        let mut sent = None;
        for key in input.keys() {
            match key {
                Key::Enter if mods.shift => self.act(fonts, Action::Enter),
                Key::Enter => {
                    let text = self.text();
                    // An empty box with a file waiting in it is still worth
                    // sending: the file is the message, and refusing it
                    // strands the attachment with no way to get it out.
                    if !text.trim().is_empty() || !self.waiting.is_empty() {
                        sent = Some(text);
                    }
                }
                // A word at a time, which is what every other text box does
                // and what a mistyped `@name` wants.
                Key::Backspace if mods.command => self.rub_out(fonts, Motion::LeftWord),
                Key::Delete if mods.command => self.rub_out(fonts, Motion::RightWord),
                Key::Backspace => self.act(fonts, Action::Backspace),
                Key::Delete => self.act(fonts, Action::Delete),
                Key::Escape => self.editor.set_selection(Selection::None),
                Key::Left if mods.command => self.motion(fonts, Motion::LeftWord, mods.shift),
                Key::Right if mods.command => self.motion(fonts, Motion::RightWord, mods.shift),
                Key::Left => self.motion(fonts, Motion::Left, mods.shift),
                Key::Right => self.motion(fonts, Motion::Right, mods.shift),
                Key::Up => self.motion(fonts, Motion::Up, mods.shift),
                Key::Down => self.motion(fonts, Motion::Down, mods.shift),
                // The ends of the whole message rather than of the line, which
                // in a box that grows to eight lines is a different place.
                Key::Home if mods.command => self.motion(fonts, Motion::BufferStart, mods.shift),
                Key::End if mods.command => self.motion(fonts, Motion::BufferEnd, mods.shift),
                Key::Home => self.along_the_row(fonts, false, mods.shift),
                Key::End => self.along_the_row(fonts, true, mods.shift),
                _ => {}
            }
        }

        // Back a step, and forward again. Both spellings of redo, because both
        // are in use and a reader has one of them in their fingers.
        if input.chord(Key::Char('z')) && !mods.shift {
            self.walk(fonts, true);
        }
        if (input.chord(Key::Char('z')) && mods.shift) || input.chord(Key::Char('y')) {
            self.walk(fonts, false);
        }
        if input.chord(Key::Char('a')) {
            // Anchored at the start and moved to the end, which is what a
            // selection *is* here: there is no "select everything" action.
            self.editor.set_cursor(Cursor::new(0, 0));
            self.editor
                .set_selection(Selection::Normal(self.editor.cursor()));
            self.motion(fonts, Motion::BufferEnd, true);
        }
        if input.chord(Key::Char('c'))
            && let Some(text) = self.editor.copy_selection()
        {
            *clipboard = text;
        }
        if input.chord(Key::Char('x'))
            && let Some(text) = self.editor.copy_selection()
        {
            *clipboard = text;
            self.editor.delete_selection();
            self.touched = true;
        }
        if input.chord(Key::Char('v')) && !clipboard.is_empty() {
            let pasted = clipboard.clone();
            self.editor.insert_string(&pasted, None);
            self.touched = true;
        }

        // Typed text last, and only when no chord claimed the frame: a platform
        // that reports `Ctrl+V` as both a chord and the letter "v" would
        // otherwise paste and then type a v.
        if !input.typed().is_empty() && !mods.command {
            for character in input.typed().chars() {
                // Control characters are not text. A newline arrives as Enter,
                // which has already been decided above.
                if !character.is_control() {
                    self.act(fonts, Action::Insert(character));
                    self.touched = true;
                }
            }
        }

        if sent.is_some() {
            self.clear(fonts);
        }
        // Last, after every way the text or the caret can have moved this
        // frame: typing, deleting, pasting, undoing, or walking with the
        // arrows.
        self.follow_caret();
        sent
    }

    /// Does one thing to the text, and remembers that it did.
    ///
    /// The change is opened and closed around every edit rather than around a
    /// run of them, so undo steps back one action: a reader who typed a word
    /// and pressed undo expects the word gone, not the sentence. An action
    /// that changed nothing -- a motion, an escape -- hands back no change and
    /// leaves the stacks alone.
    fn act(&mut self, fonts: &mut Fonts, action: Action) {
        let system = fonts.system_mut();
        self.editor.start_change();
        self.editor.action(system, action);
        let Some(change) = self
            .editor
            .finish_change()
            .filter(|one| !one.items.is_empty())
        else {
            return;
        };
        // A different future has been chosen: what was undone before it cannot
        // be redone without throwing this away.
        self.undone.clear();
        match self
            .done
            .last_mut()
            .filter(|last| carries_on(last, &change))
        {
            Some(last) => last.items.extend(change.items),
            None => self.done.push(change),
        }
    }

    /// Steps back one edit, or forward again.
    ///
    /// Reversing a change and applying it is what undoing one *is* here --
    /// `cosmic-text` hands back what an edit was and can turn it round, so
    /// nothing has to be kept but the changes themselves.
    fn walk(&mut self, fonts: &mut Fonts, back: bool) {
        let from = match back {
            true => &mut self.done,
            false => &mut self.undone,
        };
        let Some(change) = from.pop() else {
            return;
        };
        // Both stacks hold changes the way they were made, and only undoing
        // turns one round. Reversing on the way forward as well applies the
        // undo a second time instead of redoing it -- which walks the cursors
        // off the text they were recorded against, and lands inside a letter
        // rather than between two.
        let mut applied = change.clone();
        if back {
            applied.reverse();
        }
        self.editor.apply_change(&applied);
        self.editor.shape_as_needed(fonts.system_mut(), false);
        match back {
            true => self.undone.push(change),
            false => self.done.push(change),
        }
    }

    /// Moves the caret, extending the selection when shift is held.
    ///
    /// The anchor is the caller's to manage: the editor moves a cursor, and
    /// whether that drags a selection behind it is a decision above it.
    /// Rubs out as far as one motion reaches.
    ///
    /// There is no "delete a word" action: a selection out to the word
    /// boundary and then a backspace is what one is. Anything already selected
    /// goes instead, which is what a reader who selected something and pressed
    /// it meant.
    fn rub_out(&mut self, fonts: &mut Fonts, motion: Motion) {
        if self.editor.selection() == Selection::None {
            self.motion(fonts, motion, true);
        }
        self.act(fonts, Action::Backspace);
    }

    /// The start or end of the row the caret is on, rather than of the whole
    /// paragraph it belongs to.
    ///
    /// `Motion::Home` reads as the one and is the other. Its body is
    /// `cursor.index = 0` on the *logical* line -- byte for byte what
    /// `ParagraphStart` does -- so on a soft-wrapped paragraph it leaves the
    /// row the reader is on. Measured on a long message: Home moved the caret
    /// four rows up the box, which reads as the text having jumped rather than
    /// the caret having gone home.
    ///
    /// Every other editor puts Home at the start of the row under the caret.
    /// Reaching the whole message is what Ctrl+Home and Ctrl+End are for, and
    /// those already do it.
    fn along_the_row(&mut self, fonts: &mut Fonts, end: bool, extend: bool) {
        let cursor = self.editor.cursor();
        let system = fonts.system_mut();
        let Some(mut at) = self
            .editor
            .with_buffer_mut(|buffer| buffer.layout_cursor(system, cursor))
        else {
            return;
        };
        // Past the last glyph is the end of the row: `LayoutCursor` falls back
        // to the row's own end when the index is not a glyph it holds.
        at.glyph = match end {
            true => usize::MAX,
            false => 0,
        };
        self.motion(fonts, Motion::LayoutCursor(at), extend);
    }

    fn motion(&mut self, fonts: &mut Fonts, motion: Motion, extend: bool) {
        if extend {
            if self.editor.selection() == Selection::None {
                self.editor
                    .set_selection(Selection::Normal(self.editor.cursor()));
            }
        } else {
            self.editor.set_selection(Selection::None);
        }
        self.act(fonts, Action::Motion(motion));
    }

    /// Empties it, as sending does.
    pub fn clear(&mut self, fonts: &mut Fonts) {
        self.editor.with_buffer_mut(|buffer| {
            buffer.set_text("", &Attrs::new(), Shaping::Advanced, None);
        });
        self.editor.set_cursor(Cursor::new(0, 0));
        self.editor.set_selection(Selection::None);
        // Reshaped now, or the next height would still be the sent message's
        // and the box would stay tall with nothing in it.
        self.editor.shape_as_needed(fonts.system_mut(), false);
        self.touched = false;
        self.scroll = 0.0;
    }

    /// Puts text in it, with the caret at the end.
    ///
    /// At the end rather than the start, because the reason to open a box that
    /// already has words in it is to change what they say, and the end is where
    /// somebody rereading their own sentence stops.
    pub fn fill(&mut self, text: &str, fonts: &mut Fonts) {
        self.editor.with_buffer_mut(|buffer| {
            buffer.set_text(text, &Attrs::new(), Shaping::Advanced, None);
        });
        self.editor.shape_as_needed(fonts.system_mut(), false);
        let end = self.editor.with_buffer(|buffer| {
            let line = buffer.lines.len().saturating_sub(1);
            Cursor::new(line, buffer.lines[line].text().len())
        });
        self.editor.set_cursor(end);
        self.editor.set_selection(Selection::None);
        // A draft comes back with the caret at its end, so the window on it
        // has to be at the end too.
        self.scroll = 0.0;
        self.follow_caret();
        // Already said something, so an empty box reads as emptied on purpose
        // rather than never filled.
        self.touched = true;
    }

    /// Draws the box, the text, the selection and the caret.
    pub fn draw(&self, into: &mut Canvas<'_>, within: Rect, focused: bool) {
        let Canvas {
            scene,
            painter,
            fonts,
            palette,
        } = into;
        let strip = self.strip(within);
        let outer = self.box_of(within);
        let inner = self.inner(within);

        // The strip a message box sits on, which is the foot of the panel and
        // its own surface. A field has none: the app's `header` carries no
        // background of its own, and filling one here draws a band across the
        // pane that stops short of the button beside it.
        if !self.plain {
            scene.fill(strip.x, strip.y, strip.width, strip.height, palette.ground);
        }
        // The border is one rectangle behind another, which is the only
        // outline this renderer draws -- and a focused box has to be visibly
        // different from an unfocused one.
        //
        // Behind rather than four bars along the edges: a bar has square ends,
        // so four of them around a rounded box leave the corners open.
        let edge: [u8; 4] = if focused {
            [palette.signal[0], palette.signal[1], palette.signal[2], 200]
        } else {
            palette.rule
        };
        // And a shadow under it, which the border is not a substitute for:
        // one says which box has the keyboard, the other says the box is in
        // front of the conversation rather than part of it. A field inside a
        // panel gets none -- the panel it sits in is already the thing that
        // floats, and a shadow inside a shadow is just dirt.
        let panel = match self.plain {
            true => Panel::flat(outer, BOX),
            false => Panel::floating(outer, BOX, DROP),
        };
        panel.edge(edge).fill(palette.surface).draw(scene);

        if self.is_empty() {
            let said = self.hint(fonts, inner.width);
            let glyphs = painter.run(
                fonts,
                &said,
                inner.x,
                inner.y,
                matterless_paint::Run {
                    size: SIZE,
                    line_height: LINE,
                    bold: false,
                    mono: false,
                    wrap: f32::MAX,
                    icon: false,
                    smooth: false,
                },
            );
            scene.glyphs(glyphs, palette.faint, palette.faint);
        }

        // Everything below is a window onto the text rather than the whole
        // of it: a box of eight lines holding twenty has lines above and
        // below, and nothing else says where the window ends.
        scene.clip_to(inner.x, inner.y, inner.width, self.lines() as f32 * LINE);

        // The selection goes down first, or it would cover the letters it is
        // meant to be behind.
        let scroll = self.scroll;
        for (x, y, width) in self.selection_marks(within) {
            scene.fill(
                x,
                y,
                width,
                LINE,
                [palette.ink[0], palette.ink[1], palette.ink[2], 60],
            );
        }

        self.editor.with_buffer(|buffer| {
            let glyphs = matterless_paint::placed_glyphs(buffer, inner.x, inner.y - scroll);
            scene.glyphs(glyphs, palette.ink, palette.faint);
        });

        // Solid rather than blinking: a blink needs a clock and a redraw of its
        // own, and this window only draws when something happens.
        if focused && let Some((x, y)) = self.caret(within) {
            scene.fill(
                x,
                y,
                1.5,
                LINE,
                [palette.ink[0], palette.ink[1], palette.ink[2], 255],
            );
        }

        // Back to the box itself, which is what the caller clipped to and
        // what the marks below are drawn in.
        scene.clip_to(strip.x, strip.y, strip.width, strip.height);

        // A paperclip and a Send, for the readers who would rather press a
        // button than learn that Enter sends and a drop attaches. Nothing here
        // can be reached any other way than by the keyboard otherwise.
        //
        // Not on a field: there is nothing to attach to a query, and a Send
        // beside one says the wrong thing about what return does.
        if self.plain {
            return;
        }
        if self.tools_for == Tools::Writing {
            let attach = self.attach(within);
            let clip = painter.run(
                fonts,
                CLIP,
                attach.x + 5.0,
                attach.y + 2.0,
                // Larger than the words: a mark is a picture, and the font
                // rasterises one at about two thirds of the size asked for.
                Run::mark(16.0),
            );
            scene.glyphs(clip, palette.soft, palette.faint);
        }

        // The way out, beside the way on. Quiet, because it is the one of the
        // two that undoes something, and a way out drawn as loudly as the way
        // on is an invitation to press the wrong one.
        if let Some(cancel) = self.cancel(within) {
            scene.rounded(
                cancel.x,
                cancel.y,
                cancel.width,
                cancel.height,
                palette.raised,
                5.0,
            );
            let label = painter.run(
                fonts,
                "Cancel",
                cancel.x + 9.0,
                cancel.y + 4.0,
                Run::label(f32::MAX),
            );
            scene.glyphs(label, palette.soft, palette.faint);
        }

        // Lit only when there is something to send: a button that does nothing
        // is a button that has to be tried to find out.
        let send = self.send(within);
        let ready = !self.is_empty();
        scene.rounded(
            send.x,
            send.y,
            send.width,
            send.height,
            if ready {
                [palette.signal[0], palette.signal[1], palette.signal[2], 255]
            } else {
                palette.raised
            },
            5.0,
        );
        let label = painter.run(
            fonts,
            match self.tools_for {
                Tools::Writing => "Send",
                Tools::Editing => "Save",
            },
            send.x + 12.0,
            send.y + 4.0,
            Run::label(f32::MAX),
        );
        scene.glyphs(
            label,
            if ready {
                [palette.ground[0], palette.ground[1], palette.ground[2]]
            } else {
                palette.faint
            },
            palette.faint,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::{Composer, Rect};
    use matterless_layout::Fonts;
    /// The hint fits the box it is drawn in, at every width the box gets.
    ///
    /// Reported as the placeholder "not cropped properly at the new width".
    /// Measured at five: it does fit. The first measurement said it did not,
    /// because the probe asked at fifteen points where the box sets its
    /// words at fourteen -- so the style is taken from the box's own
    /// constants here rather than written out a second time.
    #[test]
    fn the_hint_fits_the_box_at_every_width() {
        let mut fonts = Fonts::new();
        let style = matterless_layout::Style {
            size: super::SIZE,
            line_height: super::LINE,
            bold: false,
            italic: false,
            mono: false,
        };
        for width in [900.0_f32, 620.0, 400.0, 300.0, 200.0] {
            let mut composer = Composer::new("composer");
            composer.lay_out(&mut fonts, width);
            let room = composer.inner(Rect::new(0.0, 0.0, width, 800.0)).width;
            let said = composer.hint(&mut fonts, room);
            let drawn = matterless_layout::extent_of(&mut fonts, &said, f32::MAX, style).width;
            assert!(
                drawn <= room,
                "at {width} the hint is {drawn:.1} wide in {room:.1} of room: {said:?}"
            );
        }
    }

    /// A box narrowed is a box re-wrapped, and as tall as that makes it.
    ///
    /// The other half of the same report: the box "does not scale properly
    /// when the window is resized or the thread pane opens". What that would
    /// look like is a box still shaped for the width it used to have, so it
    /// is compared against one built at the new width and nothing else.
    #[test]
    fn narrowing_the_box_reshapes_it_to_the_width_it_has_now() {
        let mut fonts = Fonts::new();
        let long = "the quick brown fox jumps over the lazy dog and keeps on going \
                    well past the end of any one line in this box";

        let mut narrowed = Composer::new("composer");
        narrowed.lay_out(&mut fonts, 900.0);
        narrowed.fill(long, &mut fonts);
        narrowed.lay_out(&mut fonts, 900.0);
        let wide_lines = narrowed.lines();
        narrowed.lay_out(&mut fonts, 400.0);

        let mut fresh = Composer::new("composer");
        fresh.lay_out(&mut fonts, 400.0);
        fresh.fill(long, &mut fonts);
        fresh.lay_out(&mut fonts, 400.0);

        assert!(
            narrowed.lines() > wide_lines,
            "narrowing did not re-wrap: {wide_lines} lines before and after"
        );
        assert_eq!(
            (narrowed.lines(), narrowed.height()),
            (fresh.lines(), fresh.height()),
            "a box narrowed is not the box it would have been built as"
        );
    }

    /// A waiting attachment gives itself room, and gives it back.
    ///
    /// The box is measured before it is laid out, so a shelf that took no
    /// height would draw its names over the tools underneath.
    #[test]
    fn what_is_waiting_makes_the_box_taller() {
        let mut composer = Composer::new("composer");
        let empty = composer.height();
        composer.waiting = vec!["holiday.png".to_string()];
        let carrying = composer.height();
        assert!(
            carrying > empty,
            "{carrying} is no taller than {empty} with a file waiting"
        );

        composer.waiting.push("map.pdf".to_string());
        assert_eq!(
            composer.height(),
            carrying,
            "a second file asked for a second row"
        );

        composer.waiting.clear();
        assert_eq!(composer.height(), empty, "the room was not given back");
    }

    /// Every chip sits on the shelf, side by side, however many there are.
    ///
    /// The one that matters is the crowded case: ten files share the width
    /// rather than the tenth being drawn off the end of the box, where the
    /// reader can neither read it nor take it off again.
    #[test]
    fn the_chips_stay_on_their_shelf() {
        let mut composer = Composer::new("composer");
        let within = Rect::new(0.0, 0.0, 600.0, composer.height());
        for count in [1_usize, 3, 10] {
            composer.waiting = (0..count).map(|at| format!("{at}.png")).collect();
            let within = Rect::new(within.x, within.y, within.width, composer.height());
            let shelf = composer.shelf(within);
            assert!(
                shelf.y >= composer.over(within).bottom(),
                "the shelf of {count} sits over the words"
            );
            assert!(
                shelf.bottom() <= composer.tools(within).y + 0.01,
                "the shelf of {count} sits over the tools"
            );
            let mut edge = shelf.x;
            for at in 0..count {
                let chip = composer.waiting_at(within, at).expect("a chip to draw");
                assert!(
                    chip.x >= edge,
                    "chip {at} of {count} overlaps the one before"
                );
                assert!(
                    chip.x + chip.width <= shelf.x + shelf.width + 0.01,
                    "chip {at} of {count} runs off the shelf"
                );
                assert!(
                    chip.y >= shelf.y && chip.y + chip.height <= shelf.y + shelf.height + 0.01,
                    "chip {at} of {count} sits off the shelf's row"
                );
                edge = chip.x + chip.width;
            }
            assert!(
                composer.waiting_at(within, count).is_none(),
                "a chip was offered past the last one"
            );
        }
    }

    /// The hint is cut to the box, with an ellipsis where it was cut.
    ///
    /// Drawn at its full width and left to the clip, it ended mid-letter: open
    /// a thread and the message box narrows, and the hint read "Shift+Enter
    /// for a ne" -- which does not read as a hint too long for its box, it
    /// reads as a hint that is wrong.
    #[test]
    fn a_hint_too_long_for_its_box_says_so_rather_than_stopping() {
        let mut fonts = Fonts::new();
        let composer = Composer::new("composer");
        let wide = composer.hint(&mut fonts, 400.0);
        assert!(
            wide.ends_with("new line)"),
            "{wide:?} was cut at a width with room"
        );

        let narrow = composer.hint(&mut fonts, 60.0);
        assert!(narrow.len() < wide.len(), "{narrow:?} was not cut");
        assert!(
            narrow.ends_with('\u{2026}'),
            "{narrow:?} does not say it was cut"
        );
    }

    /// No room at all is not the same as a box with nothing in it.
    #[test]
    fn a_box_with_no_room_says_nothing_rather_than_guessing() {
        let mut fonts = Fonts::new();
        let composer = Composer::new("composer");
        assert_eq!(composer.hint(&mut fonts, 0.0), "");
    }
}
