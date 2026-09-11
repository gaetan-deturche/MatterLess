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

use crate::sidebar::Canvas;
use cosmic_text::{
    Action, Attrs, Buffer, Cursor, Edit, Editor, Metrics, Motion, Selection, Shaping,
};
use matterless_layout::Fonts;
use matterless_paint::Run;
use matterless_ui::input::{Input, Key};
use matterless_ui::{Placed, Rect};

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
/// The margin outside the box, matching the stream's own gutter.
const MARGIN: f32 = 12.0;
/// How tall it is allowed to grow before the text scrolls inside it. Eight
/// lines is a long message; past that the composer would be eating the
/// conversation it is a reply to.
const MAX_LINES: usize = 8;
/// What the stylesheet cuts the box's corners by.
const BOX: f32 = 8.0;
/// The buttons inside the box: one to attach, one to send.
const BUTTON: f32 = 26.0;
const SEND: f32 = 52.0;
/// The row they sit on, along the bottom of the box.
const TOOLS: f32 = 34.0;
/// What the attach button shows. A paperclip, which is what every client uses
/// and what a reader will look for.
const CLIP: &str = "📎";

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
        }
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
        let inner = (width - MARGIN * 2.0 - PADDING * 2.0).max(1.0);
        self.editor.with_buffer_mut(|buffer| {
            buffer.set_size(Some(inner), None);
        });
        self.editor.shape_as_needed(fonts.system_mut(), false);
    }

    /// How many lines the text occupies, capped at what the box will show.
    fn lines(&self) -> usize {
        self.editor
            .with_buffer(|buffer| buffer.layout_runs().count())
            .clamp(1, MAX_LINES)
    }

    /// The height the strip needs, which grows with the message.
    pub fn height(&self) -> f32 {
        self.lines() as f32 * LINE + PADDING * 2.0 + MARGIN * 2.0 + TOOLS
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
            strip.x + MARGIN,
            strip.y + MARGIN,
            (strip.width - MARGIN * 2.0).max(0.0),
            (strip.height - MARGIN * 2.0).max(0.0),
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
        let inner = self.box_of(within).inset(PADDING);
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
            let action = if input.pressed_now() == Some(self.name.as_str()) {
                Action::Click { x: at.0, y: at.1 }
            } else {
                Action::Drag { x: at.0, y: at.1 }
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
                Key::Backspace => self.act(fonts, Action::Backspace),
                Key::Delete => self.act(fonts, Action::Delete),
                Key::Escape => self.editor.set_selection(Selection::None),
                Key::Left if mods.command => self.motion(fonts, Motion::LeftWord, mods.shift),
                Key::Right if mods.command => self.motion(fonts, Motion::RightWord, mods.shift),
                Key::Left => self.motion(fonts, Motion::Left, mods.shift),
                Key::Right => self.motion(fonts, Motion::Right, mods.shift),
                Key::Up => self.motion(fonts, Motion::Up, mods.shift),
                Key::Down => self.motion(fonts, Motion::Down, mods.shift),
                Key::Home => self.motion(fonts, Motion::Home, mods.shift),
                Key::End => self.motion(fonts, Motion::End, mods.shift),
                _ => {}
            }
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

    fn act(&mut self, fonts: &mut Fonts, action: Action) {
        let system = fonts.system_mut();
        self.editor.action(system, action);
    }

    /// Moves the caret, extending the selection when shift is held.
    ///
    /// The anchor is the caller's to manage: the editor moves a cursor, and
    /// whether that drags a selection behind it is a decision above it.
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
        let inner = outer.inset(PADDING);

        scene.fill(strip.x, strip.y, strip.width, strip.height, palette.ground);
        // A border rather than a shadow: one rectangle behind another, which is
        // the only outline this renderer draws -- and a focused box has to be
        // visibly different from an unfocused one.
        //
        // Behind rather than four bars along the edges: a bar has square ends,
        // so four of them around a rounded box leave the corners open.
        let edge: [u8; 4] = if focused {
            [palette.signal[0], palette.signal[1], palette.signal[2], 200]
        } else {
            palette.rule
        };
        scene.rounded(outer.x, outer.y, outer.width, outer.height, edge, BOX);
        scene.rounded(
            outer.x + 1.0,
            outer.y + 1.0,
            outer.width - 2.0,
            outer.height - 2.0,
            palette.surface,
            BOX - 1.0,
        );

        if self.is_empty() {
            let glyphs = painter.run(
                fonts,
                &self.placeholder,
                inner.x,
                inner.y,
                matterless_paint::Run {
                    size: SIZE,
                    line_height: LINE,
                    bold: false,
                    mono: false,
                    wrap: f32::MAX,
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
        if focused && let Some((x, y)) = self.editor.cursor_position() {
            scene.fill(
                inner.x + x as f32,
                inner.y + y as f32,
                1.5,
                LINE,
                [palette.ink[0], palette.ink[1], palette.ink[2], 255],
            );
        }

        // A paperclip and a Send, for the readers who would rather press a
        // button than learn that Enter sends and a drop attaches. Nothing here
        // can be reached any other way than by the keyboard otherwise.
        let attach = self.attach(within);
        let clip = painter.run(
            fonts,
            CLIP,
            attach.x + 6.0,
            attach.y + 4.0,
            Run::label(f32::MAX),
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
