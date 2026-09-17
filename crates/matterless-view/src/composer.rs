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
    /// True once anything has been typed, so an empty buffer can be told from
    /// one the reader has emptied on purpose.
    pub touched: bool,
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

/// A button inside the box.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Button {
    /// Pick a file, for a reader who would rather not drag one.
    Attach,
    /// Send what is typed, for a reader who would rather not press return.
    Send,
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
/// The row they sit on, along the bottom of the box.
const TOOLS: f32 = 34.0;
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
            touched: false,
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

    /// Where the caret sits inside the box, if it has one to show.
    ///
    /// Answered here rather than worked out while drawing, so a test can ask.
    /// `None` when the editor has no laid-out line to put it on, which is what
    /// a field that has never been laid out looks like.
    pub fn caret(&self, within: Rect) -> Option<(f32, f32)> {
        let inner = self.inner(within);
        let (x, y) = self.editor.cursor_position()?;
        Some((inner.x + x as f32, inner.y + y as f32))
    }

    /// How many lines the text occupies, capped at what the box will show.
    fn lines(&self) -> usize {
        self.editor
            .with_buffer(|buffer| buffer.layout_runs().count())
            .clamp(1, MAX_LINES)
    }

    /// The height the strip needs, which grows with the message.
    pub fn height(&self) -> f32 {
        let tools = if self.plain { 0.0 } else { TOOLS };
        self.lines() as f32 * LINE + self.padding() * 2.0 + self.margin() * 2.0 + tools
    }

    /// Inside the box, and around it. Tighter on a field than on a message.
    fn padding(&self) -> f32 {
        if self.plain { FIELD_PADDING } else { PADDING }
    }

    fn margin(&self) -> f32 {
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
    pub fn send(&self, within: Rect) -> Rect {
        let tools = self.tools(within);
        Rect::new(tools.right() - SEND - 6.0, tools.y + 4.0, SEND, BUTTON)
    }

    /// The two buttons, so a pointer can land on them.
    pub fn boxes_in(&self, within: Rect) -> Vec<Placed> {
        if self.plain {
            return Vec::new();
        }
        vec![
            Placed {
                name: format!("{}/attach", self.name),
                rect: self.attach(within),
                depth: 3,
            },
            Placed {
                name: format!("{}/send", self.name),
                rect: self.send(within),
                depth: 3,
            },
        ]
    }

    /// What a press on one of them means, if it landed on one.
    pub fn pressed(&self, input: &Input) -> Option<Button> {
        let clicked = input.clicked()?;
        match clicked.strip_prefix(&format!("{}/", self.name))? {
            "attach" => Some(Button::Attach),
            "send" => Some(Button::Send),
            _ => None,
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
            let at = ((x - inner.x) as i32, (y - inner.y) as i32);
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
                    if !text.trim().is_empty() {
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
                Key::Home => self.motion(fonts, Motion::Home, mods.shift),
                Key::End => self.motion(fonts, Motion::End, mods.shift),
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

        // The selection goes down first, or it would cover the letters it is
        // meant to be behind.
        if let Some((start, end)) = self.editor.selection_bounds() {
            self.editor.with_buffer(|buffer| {
                for run in buffer.layout_runs() {
                    for (x, width) in run.highlight(start, end) {
                        scene.fill(
                            inner.x + x,
                            inner.y + run.line_top,
                            width,
                            LINE,
                            [palette.ink[0], palette.ink[1], palette.ink[2], 60],
                        );
                    }
                }
            });
        }

        self.editor.with_buffer(|buffer| {
            let glyphs = matterless_paint::placed_glyphs(buffer, inner.x, inner.y);
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

        // A paperclip and a Send, for the readers who would rather press a
        // button than learn that Enter sends and a drop attaches. Nothing here
        // can be reached any other way than by the keyboard otherwise.
        //
        // Not on a field: there is nothing to attach to a query, and a Send
        // beside one says the wrong thing about what return does.
        if self.plain {
            return;
        }
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
            "Send",
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
    use super::Composer;
    use matterless_layout::Fonts;

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
