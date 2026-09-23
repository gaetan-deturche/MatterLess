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
    /// What is attached and waiting to go with the next message.
    ///
    /// Set by the window, which is what holds the uploads and decides which
    /// conversation they belong to. The box only has to say they are there
    /// and offer to take one off -- a file waiting invisibly is worse than
    /// one sent by accident, because nobody can undo what they cannot see.
    pub waiting: Vec<Waiting>,
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
    /// Where the spinner on a file still going up has got to, in turns.
    ///
    /// Set by the window, which owns the clock. A widget that read one would
    /// draw a different frame every time it was asked, and this one is asked
    /// more than once for a single frame.
    pub spinning: f32,
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
/// Draws lucide's `loader` at a turn of `phase`, out of what the renderer has.
///
/// Eight spokes, brightest at the head and fading behind it, which is the
/// mark's own shape. Dots rather than the glyph itself because nothing here
/// can rotate: every quad and every texture lookup is axis-aligned, so
/// spinning a glyph would mean a transform in the shader. A ring of rounded
/// fills with a travelling brightness is the same picture with none of that.
///
/// The phase is turns, not radians, so the window can hand over a clock
/// without either side agreeing about pi.
fn spinner(
    scene: &mut matterless_paint::Scene,
    x: f32,
    y: f32,
    phase: f32,
    palette: &matterless_paint::Palette,
) {
    const SPOKES: usize = 8;
    const RING: f32 = 9.0;
    const DOT: f32 = 2.6;
    let turn = std::f32::consts::TAU;
    for spoke in 0..SPOKES {
        let along = spoke as f32 / SPOKES as f32;
        // How far behind the head this spoke is, as a fraction of a turn.
        let behind = (along - phase).rem_euclid(1.0);
        // Brightest at the head, down to a quarter at the tail: the eye reads
        // the gradient as the direction of travel.
        let lit = 1.0 - behind * 0.75;
        let angle = along * turn;
        let ink = palette.soft;
        scene.rounded(
            x + angle.cos() * RING - DOT,
            y + angle.sin() * RING - DOT,
            DOT * 2.0,
            DOT * 2.0,
            [ink[0], ink[1], ink[2], (255.0 * lit) as u8],
            DOT,
        );
    }
}

/// One thing waiting to go with the next message.
///
/// A name is enough for a document and not enough for a picture: somebody who
/// has just pasted three screenshots is choosing between `image.png`,
/// `image (1).png` and `image (2).png`, which is no choice at all.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Waiting {
    pub name: String,
    /// What the atlas calls its picture, when it has one.
    ///
    /// Decided by the same `FileRef` the conversation uses, so the tray and
    /// the message it becomes never disagree about which rendition a file has
    /// -- the doc on `FileRef::from_info` has asked for this since it was
    /// written.
    pub picture: Option<String>,
    /// The box the server's own numbers give it, for keeping the tile's shape.
    pub shape: (f32, f32),
    /// Still going up: the tile is on screen and the file is not there yet.
    ///
    /// A drop used to show nothing at all until the upload answered and its
    /// thumbnail came back -- several seconds of a window that had visibly
    /// done nothing with what was dropped on it. The tile goes up first and
    /// fills in afterwards.
    pub on_its_way: bool,
}

impl Waiting {
    /// Wide over tall, and 1.0 for anything that did not say.
    fn aspect(&self) -> f32 {
        let (wide, tall) = self.shape;
        match wide > 0.0 && tall > 0.0 {
            true => wide / tall,
            false => 1.0,
        }
    }

    /// Whether it gets a tile rather than a chip.
    ///
    /// A file on its way gets one before anybody knows whether it is a
    /// picture: the tile is the feedback, and a row that changed height when
    /// the answer came back would move the whole box under the reader.
    fn tiled(&self) -> bool {
        self.picture.is_some() || self.on_its_way
    }
}

/// The row of waiting attachments, and the widest one of them.
const WAITING: f32 = 26.0;
/// The row when one of them is a picture: the picture, and its name under it.
///
/// Both, because either alone fails somewhere. Three files called
/// `image.png`, `image (1).png` and `image (2).png` cannot be told apart by
/// name; two screenshots of the same window cannot be told apart by a
/// sixty-pixel preview.
const TILE: f32 = 80.0;
/// The line under a picture that says what it is called.
const TILE_NAME: f32 = 16.0;
/// Narrow enough for a tall picture, wide enough for a few characters of its
/// name: a portrait screenshot would otherwise be a slot with one letter in it.
const TILE_NARROWEST: f32 = 64.0;
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
            spinning: 0.0,
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

    /// The `@name` being typed at the caret, if one is.
    ///
    /// Answers the sigil that opened it and what has been typed after it, so
    /// a caller can offer the people it might mean. `None` the moment the run
    /// stops being one: a space ends it, moving the caret out of it ends it,
    /// and so does deleting back past the sigil.
    ///
    /// The run is read *back* from the caret rather than forward from the
    /// sigil, because that is the question being asked -- "what is being
    /// typed here" -- and reading forward would offer names while the caret
    /// sat at the far end of the line.
    ///
    /// A sigil needs whitespace before it or nothing at all. Without that,
    /// an email address offers a list of people halfway through it, and so
    /// does every `a@b` anybody writes.
    pub fn being_named(&self, sigils: &[char]) -> Option<(char, String)> {
        let cursor = self.editor.cursor();
        let line = self.editor.with_buffer(|buffer| {
            buffer
                .lines
                .get(cursor.line)
                .map(|line| line.text().to_string())
        })?;
        let before = line.get(..cursor.index)?;
        // Back to the first character that cannot be part of a name.
        let (at, sigil) = before
            .char_indices()
            .rev()
            .find(|(_, c)| c.is_whitespace() || sigils.contains(c))?;
        if !sigils.contains(&sigil) {
            return None;
        }
        // What sits in front of the sigil decides whether it opens anything.
        let opens = before[..at]
            .chars()
            .next_back()
            .is_none_or(char::is_whitespace);
        if !opens {
            return None;
        }
        Some((sigil, before[at + sigil.len_utf8()..].to_string()))
    }

    /// Puts `name` in place of the run `being_named` answered about.
    ///
    /// The whole run including its sigil, and a space after, because a name
    /// chosen is a name finished -- and a caller that had to add the space
    /// itself would be a caller that could forget.
    pub fn name_it(&mut self, fonts: &mut Fonts, sigil: char, said: &str, name: &str) {
        // Once per character of what was typed, plus the sigil, which is what
        // the editor offers: it owns the undo history, and a text set behind
        // its back is a step that cannot be reversed.
        for _ in 0..said.chars().count() + 1 {
            self.act(fonts, Action::Backspace);
        }
        for character in format!("{sigil}{name} ").chars() {
            self.act(fonts, Action::Insert(character));
        }
        self.touched = true;
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
        for (at, chip) in self.tray(within).into_iter().enumerate() {
            let under = input.hovered() == Some(format!("{}/unattach/{at}", self.name).as_str());
            scene.rounded(
                chip.x,
                chip.y,
                chip.width,
                chip.height,
                if under { palette.hover } else { palette.raised },
                5.0,
            );
            // A picture is shown rather than named. Nothing is drawn until the
            // bytes arrive, which leaves the plate above standing in for it --
            // the same rule the conversation follows, and the reason the tray
            // does not change size when one lands.
            if self.waiting[at].tiled() {
                let shown = chip.height - TILE_NAME;
                // On `on_its_way`, not on "is there a picture": a key is
                // chosen the moment the upload answers and the bytes are a
                // fetch behind it, so matching on the key drew an image the
                // atlas could not draw and no spinner either -- a dark square
                // with nothing happening on it.
                match self.waiting[at]
                    .picture
                    .clone()
                    .filter(|_| !self.waiting[at].on_its_way)
                {
                    Some(key) => scene.extend([matterless_paint::Piece::Image {
                        x: chip.x,
                        y: chip.y,
                        width: chip.width,
                        height: shown,
                        key,
                        radius: 5.0,
                    }]),
                    // Nothing to draw yet, so the plate and a spinner on it.
                    // A dark tile alone says "something is here" and not
                    // "something is happening", and what is being waited
                    // through is seconds of upload.
                    None => {
                        scene.rounded(chip.x, chip.y, chip.width, shown, palette.ground, 5.0);
                        spinner(
                            scene,
                            chip.x + chip.width / 2.0,
                            chip.y + shown / 2.0,
                            self.spinning,
                            palette,
                        );
                    }
                }
                // Its name under it. A preview alone is not enough -- two
                // screenshots of the same window look alike at this size --
                // and a name alone is not either, which is what the tray used
                // to be.
                let said = matterless_layout::elided(
                    fonts,
                    &self.waiting[at].name,
                    (chip.width - 8.0).max(8.0),
                    matterless_layout::Style {
                        size: 11.0,
                        line_height: 14.0,
                        bold: false,
                        italic: false,
                        mono: false,
                    },
                );
                let glyphs = painter.run(
                    fonts,
                    &said,
                    chip.x + 4.0,
                    chip.y + shown + 1.0,
                    Run {
                        size: 11.0,
                        line_height: 14.0,
                        wrap: f32::MAX,
                        ..Run::label(f32::MAX)
                    },
                );
                scene.glyphs(glyphs, palette.soft, palette.faint);
                // Over the picture, and always readable: the corner it sits in
                // belongs to whatever was photographed.
                scene.rounded(
                    chip.right() - 19.0,
                    chip.y + 3.0,
                    16.0,
                    16.0,
                    palette.ground,
                    8.0,
                );
                let cross = painter.run(
                    fonts,
                    matterless_layout::marks::CLOSE,
                    chip.right() - 16.0,
                    chip.y + 4.0,
                    Run::mark(11.0),
                );
                scene.glyphs(
                    cross,
                    if under { palette.ink } else { palette.soft },
                    palette.faint,
                );
                continue;
            }
            let room = (chip.width - 22.0).max(10.0);
            let said = matterless_layout::elided(
                fonts,
                &self.waiting[at].name,
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
                chip.y + (chip.height - 16.0) / 2.0,
                Run::label(f32::MAX),
            );
            scene.glyphs(glyphs, palette.soft, palette.faint);
            // The way to take it off, on its own end of the chip.
            let cross = painter.run(
                fonts,
                matterless_layout::marks::CLOSE,
                chip.right() - 15.0,
                chip.y + (chip.height - 14.0) / 2.0,
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
    /// eliding is for. A picture among them makes the row deep enough to see
    /// one in -- all of them, because a row of two heights reads as two rows.
    fn held(&self) -> f32 {
        if self.plain || self.waiting.is_empty() {
            return 0.0;
        }
        match self.waiting.iter().any(|held| held.tiled()) {
            true => TILE,
            false => WAITING,
        }
    }

    /// Where the waiting attachments sit: between the words and the tools.
    fn shelf(&self, within: Rect) -> Rect {
        let tools = self.tools(within);
        Rect::new(tools.x, tools.y - self.held(), tools.width, self.held())
    }

    /// Where every one of them sits on that shelf.
    ///
    /// Laid out in one place and indexed by `waiting_at`, because the drawing,
    /// the hit boxes and the press all have to agree about it -- and with
    /// pictures in the row the widths are no longer all the same, so "the nth
    /// of n equal shares" stopped being an answer.
    ///
    /// The gaps and the margins come out of the width before it is shared, so
    /// that however many are waiting the last one ends inside the box: a chip
    /// drawn past the edge can be neither read nor pressed off again.
    fn tray(&self, within: Rect) -> Vec<Rect> {
        let shelf = self.shelf(within);
        let count = self.waiting.len();
        if shelf.height <= 0.0 || count == 0 {
            return Vec::new();
        }
        let tall = shelf.height - 6.0;
        let room = (shelf.width - CHIP_MARGIN * 2.0 - CHIP_GAP * (count - 1) as f32).max(0.0);
        let share = (room / count as f32).min(CHIP);
        let mut at = shelf.x + CHIP_MARGIN;
        let mut places = Vec::with_capacity(count);
        for held in &self.waiting {
            // A picture keeps its own shape, so a tall screenshot is not
            // stretched into a wide box -- measured against the picture's own
            // part of the tile, not the name's. Never wider than its share:
            // the row has to end inside the box whatever is in it.
            let wide = match held.tiled() {
                true => {
                    ((tall - TILE_NAME) * held.aspect()).clamp(TILE_NARROWEST.min(share), share)
                }
                false => share,
            };
            places.push(Rect::new(at, shelf.y + 3.0, wide, tall));
            at += wide + CHIP_GAP;
        }
        places
    }

    /// Where one of them sits, for the boxes and the drawing.
    fn waiting_at(&self, within: Rect, at: usize) -> Option<Rect> {
        self.tray(within).get(at).copied()
    }

    /// The pictures the tray needs before it can show anything.
    ///
    /// The same shape every other widget answers with, so the window gathers
    /// them all the same way: what to fetch, and how big it will be drawn.
    pub fn wants(&self) -> Vec<(String, u32, u32)> {
        self.waiting
            .iter()
            .filter_map(|held| {
                let key = held.picture.clone()?;
                let (wide, tall) = held.shape;
                Some((key, wide.max(1.0) as u32, tall.max(1.0) as u32))
            })
            .collect()
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
    use super::{Composer, Rect, Waiting};
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

    /// A file waiting by name alone, which is what a document is.
    fn named(name: &str) -> Waiting {
        Waiting {
            name: name.to_string(),
            ..Default::default()
        }
    }

    /// A picture waiting, which is what is shown rather than named.
    fn pictured(name: &str, wide: f32, tall: f32) -> Waiting {
        Waiting {
            name: name.to_string(),
            picture: Some(format!("thumb/{name}")),
            shape: (wide, tall),
            on_its_way: false,
        }
    }

    /// A file dropped a moment ago and not yet on the server.
    fn going_up(name: &str) -> Waiting {
        Waiting {
            name: name.to_string(),
            on_its_way: true,
            ..Default::default()
        }
    }

    /// A picture waiting is shown rather than named, at its own shape.
    ///
    /// Reported against the official client: the tray listed `image.png`,
    /// `image (1).png` and `image (2).png`, which is no way to tell three
    /// screenshots apart.
    #[test]
    fn a_waiting_picture_is_shown_at_its_own_shape() {
        let mut composer = Composer::new("composer");
        composer.waiting = vec![named("map.pdf")];
        let by_name = composer.height();
        composer.waiting = vec![pictured("holiday.png", 120.0, 60.0)];
        let shown = composer.height();
        assert!(
            shown > by_name,
            "{shown} leaves no more room for a picture than {by_name} does for a name"
        );

        let within = Rect::new(0.0, 0.0, 600.0, composer.height());
        let tile = composer.waiting_at(within, 0).expect("a tile");
        // Against the picture's own part of the tile: the name under it is
        // the tray's, not the picture's, and measuring the shape against the
        // whole tile would squash every preview by a line.
        let shown = tile.height - super::TILE_NAME;
        assert!(
            (tile.width / shown - 2.0).abs() < 0.1,
            "{tile:?} is not the shape the server gave it"
        );
        // And it asks for exactly the rendition the conversation would.
        assert_eq!(
            composer.wants(),
            vec![("thumb/holiday.png".to_string(), 120, 60)]
        );
    }

    /// Pictures and names in one tray still make one row, inside the box.
    #[test]
    fn a_mixed_tray_stays_on_one_shelf() {
        let mut composer = Composer::new("composer");
        composer.waiting = vec![
            pictured("a.png", 100.0, 100.0),
            named("b.pdf"),
            pictured("tall.png", 40.0, 120.0),
        ];
        let within = Rect::new(0.0, 0.0, 600.0, composer.height());
        let shelf = composer.shelf(within);
        let tray = composer.tray(within);
        assert_eq!(tray.len(), 3);
        for (at, tile) in tray.iter().enumerate() {
            assert!(
                tile.right() <= shelf.right() + 0.5,
                "{at} at {tile:?} runs past {shelf:?}"
            );
            assert!(tile.y >= shelf.y && tile.bottom() <= shelf.bottom() + 0.5);
        }
        assert!(
            tray.windows(2)
                .all(|pair| (pair[0].y - pair[1].y).abs() < 0.01),
            "the tray is two rows deep"
        );
    }

    /// And the tile really carries the picture, in the box the tray gave it.
    ///
    /// A gesture cannot be driven from here -- nothing can be dropped or
    /// pasted into this window from a test -- so the scene is built and the
    /// pieces read back. What this catches is the pair that has to agree: the
    /// key the atlas is asked for, and the rect it is drawn in.
    #[test]
    fn the_tile_draws_the_picture_the_tray_asked_for() {
        use matterless_paint::{Painter, Palette, Piece, Scene};

        let mut fonts = Fonts::new();
        let mut composer = Composer::new("composer");
        composer.waiting = vec![pictured("holiday.png", 120.0, 60.0), named("map.pdf")];
        let within = Rect::new(0.0, 0.0, 600.0, composer.height());
        let tile = composer.waiting_at(within, 0).expect("a tile");

        let mut scene = Scene::default();
        let mut painter = Painter::new();
        let palette = Palette::default();
        composer.draw_over(
            &mut matterless_widgets::Canvas {
                scene: &mut scene,
                painter: &mut painter,
                fonts: &mut fonts,
                palette: &palette,
            },
            &matterless_ui::input::Input::default(),
            within,
        );

        let drawn: Vec<(String, f32, f32, f32, f32)> = scene
            .layers
            .iter()
            .flat_map(|layer| layer.pieces.iter())
            .filter_map(|piece| match piece {
                Piece::Image {
                    key,
                    x,
                    y,
                    width,
                    height,
                    ..
                } => Some((key.clone(), *x, *y, *width, *height)),
                _ => None,
            })
            .collect();
        assert_eq!(
            drawn,
            vec![(
                "thumb/holiday.png".to_string(),
                tile.x,
                tile.y,
                tile.width,
                tile.height - super::TILE_NAME
            )],
            "the picture is drawn somewhere other than its own tile"
        );
        // And the name is under it rather than over it, which is the whole
        // reason the tile is deeper than the picture.
        // The positive claim, rather than "nothing above it": the box draws
        // its own words too, and a minimum over every glyph answers about
        // whichever of them sits highest.
        let under = scene
            .layers
            .iter()
            .flat_map(|layer| layer.pieces.iter())
            .filter_map(|piece| match piece {
                Piece::Text { glyphs, .. } => Some(glyphs),
                _ => None,
            })
            .flatten()
            .any(|glyph| {
                let (gx, gy) = (glyph.x as f32, glyph.y as f32);
                gx >= tile.x
                    && gx <= tile.right()
                    && gy >= tile.y + tile.height - super::TILE_NAME - 1.0
                    && gy <= tile.bottom() + 1.0
            });
        assert!(under, "the tile has no name under its picture");
    }

    /// A file still going up already has its tile, at the size it will keep.
    ///
    /// Reported 2026-09-23: dropping a picture showed nothing for several
    /// seconds -- the upload, and then a round trip for its thumbnail -- so a
    /// drop looked like it had missed. The tile is the feedback, and it has
    /// to be the same tile afterwards: a row that changed height when the
    /// answer came back would move the whole box under the reader.
    #[test]
    fn a_file_on_its_way_already_has_its_tile() {
        let mut composer = Composer::new("composer");
        composer.waiting = vec![going_up("holiday.png")];
        let while_waiting = composer.height();
        let within = Rect::new(0.0, 0.0, 600.0, while_waiting);
        let tile = composer
            .waiting_at(within, 0)
            .expect("a tile while it goes up");

        // The same file, once the server has answered.
        composer.waiting = vec![pictured("holiday.png", 120.0, 60.0)];
        assert_eq!(
            composer.height(),
            while_waiting,
            "the box changed height when the upload finished"
        );
        let landed = composer.waiting_at(within, 0).expect("a tile");
        assert_eq!(
            (tile.y, tile.height),
            (landed.y, landed.height),
            "the tile moved when its picture arrived"
        );
    }

    /// The spinner turns, and only while something is going up.
    ///
    /// What it guards is the pair that makes an animation visible at all: the
    /// picture has to change with the phase, and the window has to be woken
    /// to draw it. This half is the picture -- that the same tile, drawn at
    /// two phases, is not the same set of pieces.
    #[test]
    fn the_spinner_turns_while_a_file_goes_up() {
        use matterless_paint::{Painter, Palette, Piece, Scene};

        let mut fonts = Fonts::new();
        let mut composer = Composer::new("composer");
        composer.waiting = vec![going_up("holiday.png")];
        let within = Rect::new(0.0, 0.0, 600.0, composer.height());

        let drawn = |composer: &Composer, fonts: &mut Fonts| -> Vec<u8> {
            let mut scene = Scene::default();
            let mut painter = Painter::new();
            let palette = Palette::default();
            composer.draw_over(
                &mut matterless_widgets::Canvas {
                    scene: &mut scene,
                    painter: &mut painter,
                    fonts,
                    palette: &palette,
                },
                &matterless_ui::input::Input::default(),
                within,
            );
            // The alphas of the round fills, in order: the spinner is the
            // only thing here whose brightness changes.
            scene
                .layers
                .iter()
                .flat_map(|layer| layer.pieces.iter())
                .filter_map(|piece| match piece {
                    Piece::Fill { colour, radius, .. } if *radius > 2.0 && *radius < 3.0 => {
                        Some(colour[3])
                    }
                    _ => None,
                })
                .collect()
        };

        composer.spinning = 0.0;
        let head = drawn(&composer, &mut fonts);
        assert_eq!(head.len(), 8, "eight spokes, as the mark has");
        composer.spinning = 0.5;
        let half = drawn(&composer, &mut fonts);
        assert_ne!(head, half, "the spinner is the same at every phase");
        // Back where it started: a turn is a turn.
        composer.spinning = 1.0;
        assert_eq!(drawn(&composer, &mut fonts), head);

        // And nothing turns once the picture has arrived.
        composer.waiting = vec![pictured("holiday.png", 120.0, 60.0)];
        assert!(
            drawn(&composer, &mut fonts).is_empty(),
            "a picture that has landed is still drawing a spinner"
        );
    }

    /// A picture the atlas cannot draw yet keeps its spinner.
    ///
    /// The gap reported 2026-09-23 with four files at once: the upload
    /// answers, so the tile is no longer "on its way", and its thumbnail is
    /// another round trip behind that -- leaving a dark square with no
    /// spinner and no picture. Whether to spin is `on_its_way`, which the
    /// window keeps true until the atlas holds the key; it is not "has a
    /// picture been chosen", which is true the moment the upload lands.
    #[test]
    fn a_tile_whose_picture_has_not_arrived_still_spins() {
        use matterless_paint::{Painter, Palette, Piece, Scene};

        let mut fonts = Fonts::new();
        let mut composer = Composer::new("composer");
        // What the window hands over between the two: the rendition is known
        // and the bytes are not here.
        composer.waiting = vec![Waiting {
            on_its_way: true,
            ..pictured("holiday.png", 120.0, 60.0)
        }];
        let within = Rect::new(0.0, 0.0, 600.0, composer.height());

        let mut scene = Scene::default();
        let mut painter = Painter::new();
        let palette = Palette::default();
        composer.draw_over(
            &mut matterless_widgets::Canvas {
                scene: &mut scene,
                painter: &mut painter,
                fonts: &mut fonts,
                palette: &palette,
            },
            &matterless_ui::input::Input::default(),
            within,
        );
        let spokes = scene
            .layers
            .iter()
            .flat_map(|layer| layer.pieces.iter())
            .filter(|piece| {
                matches!(piece, Piece::Fill { radius, .. } if *radius > 2.0 && *radius < 3.0)
            })
            .count();
        assert_eq!(spokes, 8, "a tile with nothing to draw is not saying so");
    }

    /// A waiting attachment gives itself room, and gives it back.
    ///
    /// The box is measured before it is laid out, so a shelf that took no
    /// height would draw its names over the tools underneath.
    #[test]
    fn what_is_waiting_makes_the_box_taller() {
        let mut composer = Composer::new("composer");
        let empty = composer.height();
        composer.waiting = vec![named("holiday.png")];
        let carrying = composer.height();
        assert!(
            carrying > empty,
            "{carrying} is no taller than {empty} with a file waiting"
        );

        composer.waiting.push(named("map.pdf"));
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
            composer.waiting = (0..count).map(|at| named(&format!("{at}.png"))).collect();
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
