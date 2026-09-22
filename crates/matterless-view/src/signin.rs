//! The way in, for a window that has no session yet.
//!
//! Until now there was none. A build with no token in the keychain and no
//! `server.txt` beside its database drew the sample conversation, said
//! "offline" in the corner, and offered nothing that would change that: the
//! only way to give this window a session was to run the probe tool, let it
//! mint one, and put the file where `live::stored_token` looks. That is a
//! first run nobody outside this repository could complete.
//!
//! So this is what a window with no session shows, and it shows it instead of
//! the conversation rather than over it. There is nothing behind it worth
//! dimming -- the conversation behind an unsigned-in window is invented data,
//! and a sign-in box floating over a fake conversation says the fake one is
//! yours.
//!
//! The fields are not `Composer`s, which every other box in this window is.
//! A password has to be drawn as something other than what it says, and a
//! `Composer` is a `cosmic-text` editor: its glyphs, its caret and its
//! selection all come from one shaped buffer of the real text, so masking it
//! means either lying to the editor about its own contents or drawing bullets
//! at the letters' widths. Neither is worth it for four single-line fields
//! with no selection and no wrapping, which is what these are.

use matterless_layout::Fonts;
use matterless_paint::Run;
use matterless_ui::input::{Input, Key};
use matterless_ui::{Placed, Rect};
use matterless_widgets::{Button, Canvas, Laid, Named, Panel, Row};

pub const NAME: &str = "signin";

/// What this widget's hit boxes are called. The join and its inverse in one
/// place, so the box it registers and the press it answers cannot disagree.
fn named() -> Named {
    Named::new(NAME)
}

/// How wide the card is, and how much air is inside it.
const WIDTH: f32 = 380.0;
const PAD: f32 = 24.0;
const CORNER: f32 = 10.0;
const DROP: f32 = 18.0;

/// The mark above the card, at the size it is drawn.
const LOGO: f32 = 88.0;
/// Between the mark and the card. Enough that they read as two things.
const LOGO_GAP: f32 = 20.0;

const TITLE_SIZE: f32 = 17.0;
const SAID_SIZE: f32 = 12.5;
const LABEL_SIZE: f32 = 11.5;
const FIELD_SIZE: f32 = 13.5;

/// One field: its label, the box under it, and the gap to the next.
const LABEL_LINE: f32 = 16.0;
const FIELD_HEIGHT: f32 = 32.0;
const FIELD_PAD: f32 = 9.0;
const FIELD_GAP: f32 = 14.0;
/// How wide the caret is.
///
/// Named because the scroll has to know it: a field pushed far enough left to
/// put the caret hard against its right edge put the caret's own width past
/// that edge, where the clip ate it. Which is the one moment it is needed --
/// somebody typing at the end of a line longer than the box.
const CARET: f32 = 1.5;
const BUTTON_HEIGHT: f32 = 30.0;
/// Around the block of fields: under the words above it, over the row below.
const GAP: f32 = 16.0;
/// What the name and the line under it take.
///
/// Fixed rather than measured. The card's height and the places of the things
/// in it were two sums over the same pieces, one measuring the runs and the
/// other reading these numbers. They agreed, as it happens -- but agreeing by
/// arithmetic nobody checked is not the same as being one sum.
const TITLE_LINE: f32 = TITLE_SIZE * 1.4;
const SAID_LINE: f32 = SAID_SIZE * 1.5;

/// What the logo is called in the atlas.
///
/// On the pictures sheet rather than the faces one, which `atlas::Sheet::of`
/// decides from this prefix. It is one 128px picture that never changes, and
/// the faces sheet is for the hundreds of small ones that do.
pub const LOGO_KEY: &str = "signin/logo";

/// One line somebody types into.
///
/// No selection: a login form is four short strings, and a selection is a
/// second caret, a drag, a set of chords and a hit test for each of them.
/// Everything that makes a field usable without one -- the arrows, Home and
/// End, Backspace and Delete, paste -- is here.
#[derive(Debug, Clone)]
pub struct Field {
    /// What this box answers to, inside the widget.
    pub slug: &'static str,
    /// Above the box, saying what goes in it.
    pub label: &'static str,
    /// Inside it while it is empty.
    pub hint: &'static str,
    pub text: String,
    /// Whether it is drawn as bullets. What is typed is unaffected.
    pub masked: bool,
    /// Where the caret is, as a byte index into `text`.
    caret: usize,
    /// The other end of the selection, as a byte index.
    ///
    /// Equal to the caret when nothing is selected, which is most of the time
    /// and is why there is no `Option` here: every edit has to collapse the
    /// selection anyway, and "both ends in the same place" already says that.
    anchor: usize,
}

impl Field {
    pub fn new(slug: &'static str, label: &'static str, hint: &'static str) -> Self {
        Self {
            slug,
            label,
            hint,
            text: String::new(),
            masked: false,
            caret: 0,
            anchor: 0,
        }
    }

    pub fn masked(mut self) -> Self {
        self.masked = true;
        self
    }

    pub fn is_empty(&self) -> bool {
        self.text.trim().is_empty()
    }

    /// What is drawn, which is the text itself or one bullet per character.
    ///
    /// Per character rather than per byte: a password is allowed to contain
    /// anything a keyboard can produce, and `len()` on one with an accent in
    /// it would show more bullets than were typed.
    fn shown(&self) -> String {
        self.shown_upto(self.text.len())
    }

    /// What is drawn for the text up to byte `at`.
    ///
    /// One function rather than three, because every measurement this field
    /// makes -- the caret, each end of the selection, the scroll -- is the
    /// width of some prefix of what is *shown*, and a masked field shows
    /// something other than what it holds.
    fn shown_upto(&self, at: usize) -> String {
        let upto = &self.text[..at];
        match self.masked {
            true => "\u{2022}".repeat(upto.chars().count()),
            false => upto.to_string(),
        }
    }

    /// The selection, low end first. Both the same means none.
    fn span(&self) -> (usize, usize) {
        match self.caret <= self.anchor {
            true => (self.caret, self.anchor),
            false => (self.anchor, self.caret),
        }
    }

    pub fn has_selection(&self) -> bool {
        self.caret != self.anchor
    }

    /// What is selected, as the reader typed it.
    ///
    /// `None` from a masked field even when something is selected. A password
    /// box that hands its contents to the clipboard is a password box that
    /// shows the password, by a longer route -- no browser does it, and the
    /// selection is there to be replaced or deleted, not read.
    pub fn selection(&self) -> Option<String> {
        let (from, to) = self.span();
        (from != to && !self.masked).then(|| self.text[from..to].to_string())
    }

    /// Takes the selected run out, and leaves the caret where it was.
    fn cut_selection(&mut self) -> bool {
        let (from, to) = self.span();
        if from == to {
            return false;
        }
        self.text.drain(from..to);
        self.caret = from;
        self.anchor = from;
        true
    }

    /// Puts both ends of the selection at `at`.
    fn put(&mut self, at: usize) {
        self.caret = at;
        self.anchor = at;
    }

    /// Moves the caret, keeping the anchor when the reader is extending.
    fn go(&mut self, at: usize, extend: bool) {
        self.caret = at;
        if !extend {
            self.anchor = at;
        }
    }

    /// The byte index one character before the caret, or the caret itself when
    /// it is already at the start.
    fn back_one(&self) -> usize {
        self.text[..self.caret]
            .char_indices()
            .next_back()
            .map(|(at, _)| at)
            .unwrap_or(0)
    }

    /// The byte index one character after the caret, or the caret itself when
    /// it is already at the end.
    fn forward_one(&self) -> usize {
        self.text[self.caret..]
            .chars()
            .next()
            .map(|one| self.caret + one.len_utf8())
            .unwrap_or(self.caret)
    }

    pub fn set(&mut self, text: &str) {
        self.text = text.to_string();
        self.put(self.text.len());
    }

    /// Selects everything, with the caret at the end.
    pub fn select_all(&mut self) {
        self.anchor = 0;
        self.caret = self.text.len();
    }

    /// Selects the run of non-space around `at`, which is what a double press
    /// on a word means.
    fn select_word_at(&mut self, at: usize) {
        let from = self.text[..at]
            .char_indices()
            .rev()
            .take_while(|(_, one)| !one.is_whitespace())
            .map(|(index, _)| index)
            .last()
            .unwrap_or(at);
        let to = self.text[at..]
            .char_indices()
            .take_while(|(_, one)| !one.is_whitespace())
            .map(|(index, one)| at + index + one.len_utf8())
            .last()
            .unwrap_or(at);
        self.anchor = from;
        self.caret = to;
    }

    fn insert(&mut self, said: &str) {
        // A pasted server url arrives with its newline still on it more often
        // than not, and a single-line field that accepts one grows a line it
        // will never show.
        let clean: String = said.chars().filter(|one| !one.is_control()).collect();
        // The selection goes first even when nothing is being put in its
        // place: what a reader means by selecting and pressing a dead key is
        // still "not this".
        self.cut_selection();
        if clean.is_empty() {
            return;
        }
        self.text.insert_str(self.caret, &clean);
        self.put(self.caret + clean.len());
    }

    fn style() -> matterless_layout::Style {
        matterless_layout::Style {
            size: FIELD_SIZE,
            line_height: FIELD_SIZE * 1.4,
            bold: false,
            italic: false,
            mono: false,
        }
    }

    /// How wide what is drawn is, up to byte `at`.
    fn upto(&self, fonts: &mut Fonts, at: usize) -> f32 {
        matterless_layout::extent_of(fonts, &self.shown_upto(at), f32::MAX, Self::style()).width
    }

    /// Which character boundary `x` is nearest, `x` measured from the left of
    /// the text rather than of the box.
    ///
    /// Every boundary is measured. A field holds a hostname or a password, so
    /// this is a few dozen measurements on a press, and the alternative --
    /// stepping by an assumed character width -- is wrong for every font that
    /// is not monospaced, which is the one this draws in.
    fn boundary_at(&self, fonts: &mut Fonts, x: f32) -> usize {
        let mut best = 0;
        let mut closest = f32::MAX;
        let mut at = 0;
        loop {
            let away = (self.upto(fonts, at) - x).abs();
            if away < closest {
                closest = away;
                best = at;
            }
            match self.text[at..].chars().next() {
                Some(one) => at += one.len_utf8(),
                None => break,
            }
        }
        best
    }

    /// How far the text is pushed left so the caret stays in the box.
    ///
    /// Worked out each frame rather than kept: it is a function of the text,
    /// the caret and the width, all three of which the caller already has, and
    /// a stored one is a fourth thing that can disagree with them.
    fn offset(&self, fonts: &mut Fonts, inner: f32) -> f32 {
        let caret = self.upto(fonts, self.caret);
        (caret + CARET - inner).max(0.0)
    }

    /// Where inside the text a press at window `x` landed.
    fn pressed_at(&self, fonts: &mut Fonts, box_of: Rect, x: f32) -> usize {
        let inner = box_of.inset(FIELD_PAD);
        let offset = self.offset(fonts, inner.width);
        self.boundary_at(fonts, x - inner.x + offset)
    }

    /// A press, a drag, or a second and third press in the same place.
    pub fn pointer(&mut self, fonts: &mut Fonts, input: &Input, box_of: Rect, named: &Named) {
        let Some((x, _)) = input.pointer_at() else {
            return;
        };
        let mine = named.of(self.slug);
        if input.pressed_now() == Some(mine.as_str()) {
            let at = self.pressed_at(fonts, box_of, x);
            match input.clicks() {
                // A word, then the lot. What every other box on this machine
                // does, and the reason a reader double-presses a hostname
                // rather than dragging across it.
                2 => self.select_word_at(at),
                n if n >= 3 => self.select_all(),
                _ => self.go(at, input.mods().shift),
            }
            return;
        }
        // Still held, from a press that began in this box: the caret follows
        // and the anchor stays where the press put it, which is a selection
        // being dragged out.
        if input.pressed() == Some(mine.as_str()) {
            let at = self.pressed_at(fonts, box_of, x);
            self.go(at, true);
        }
    }

    /// Takes the frame's keystrokes, if this field holds the keyboard.
    ///
    /// The keys are *taken* rather than read: the window reads the same input,
    /// and an Escape both closed a panel and cleared a field the one time both
    /// were listening.
    /// `clipboard` is the window's, not the platform's.
    ///
    /// The same contract the message box has: the window owns one string,
    /// syncs it with the system clipboard around the frame, and hands it to
    /// whatever is being typed in. Reaching for the real clipboard in here
    /// would make every test of this field depend on -- and clobber -- what
    /// the reader happened to have copied.
    fn react(&mut self, input: &mut Input, clipboard: &mut String) {
        let extend = input.mods().shift;
        if input.chord(Key::Char('a')) {
            self.select_all();
        }
        if input.chord(Key::Char('c'))
            && let Some(said) = self.selection()
        {
            *clipboard = said;
        }
        if input.chord(Key::Char('x'))
            && let Some(said) = self.selection()
        {
            *clipboard = said;
            self.cut_selection();
        }
        if input.took(Key::Backspace) && !self.cut_selection() && self.caret > 0 {
            let back = self.back_one();
            self.text.drain(back..self.caret);
            self.put(back);
        }
        if input.took(Key::Delete) && !self.cut_selection() {
            let forward = self.forward_one();
            self.text.drain(self.caret..forward);
        }
        if input.took(Key::Left) {
            // A selection collapses to the end the caret is moving towards,
            // rather than stepping one off wherever the caret happens to be.
            match self.has_selection() && !extend {
                true => self.put(self.span().0),
                false => self.go(self.back_one(), extend),
            }
        }
        if input.took(Key::Right) {
            match self.has_selection() && !extend {
                true => self.put(self.span().1),
                false => self.go(self.forward_one(), extend),
            }
        }
        if input.took(Key::Home) {
            self.go(0, extend);
        }
        if input.took(Key::End) {
            self.go(self.text.len(), extend);
        }
        if input.chord(Key::Char('v')) && !clipboard.is_empty() {
            let pasted = clipboard.clone();
            self.insert(&pasted);
        }
        // Typed text last, and only when no chord claimed the frame. A
        // platform that reports `Ctrl+V` as both a chord and the letter "v"
        // would otherwise paste and then type a v -- which the message box
        // already knew and this box had to learn the same way.
        if !input.typed().is_empty() && !input.mods().command {
            let typed = input.typed().to_string();
            self.insert(&typed);
        }
    }

    /// `within` is what the clip goes back to.
    ///
    /// A field with more text than box narrows the clip to its own inside, and
    /// a clip is the scene's, not this box's: left narrowed, it swallowed
    /// every panel drawn after it. On screen that was three fields with their
    /// text and no boxes round it, and a Sign in button that was not there at
    /// all -- all of it drawn, none of it inside the last field's inside.
    fn draw(&self, into: &mut Canvas<'_>, box_of: Rect, within: Rect, focused: bool) {
        let Canvas {
            scene,
            painter,
            fonts,
            palette,
        } = into;
        let edge = match focused {
            true => [palette.signal[0], palette.signal[1], palette.signal[2], 200],
            false => palette.rule,
        };
        Panel::flat(box_of, 6.0)
            .edge(edge)
            .fill(palette.raised)
            .draw(scene);

        let inner = box_of.inset(FIELD_PAD);
        let baseline = box_of.y + (box_of.height - FIELD_SIZE * 1.4) / 2.0;
        if self.text.is_empty() {
            let glyphs = painter.run(
                fonts,
                self.hint,
                inner.x,
                baseline,
                Run::label(inner.width).sized(FIELD_SIZE),
            );
            scene.glyphs(glyphs, palette.faint, palette.faint);
            // A field with nothing in it still shows where typing would go.
            if focused {
                scene.fill(
                    inner.x,
                    baseline,
                    CARET,
                    FIELD_SIZE * 1.4,
                    [palette.ink[0], palette.ink[1], palette.ink[2], 255],
                );
            }
            return;
        }

        // Everything past here is a window onto the line, for a server name
        // longer than the box: without the clip it runs out over the card.
        let offset = self.offset(fonts, inner.width);
        scene.clip_to(inner.x, box_of.y, inner.width, box_of.height);

        // The selection first, or it would cover the letters it is meant to
        // be behind.
        let (from, to) = self.span();
        if from != to {
            let left = self.upto(fonts, from) - offset;
            let right = self.upto(fonts, to) - offset;
            scene.fill(
                inner.x + left,
                baseline,
                right - left,
                FIELD_SIZE * 1.4,
                [palette.ink[0], palette.ink[1], palette.ink[2], 60],
            );
        }

        let glyphs = painter.run(
            fonts,
            &self.shown(),
            inner.x - offset,
            baseline,
            Run::label(f32::MAX).sized(FIELD_SIZE),
        );
        scene.glyphs(glyphs, palette.ink, palette.faint);
        if focused {
            let caret = self.upto(fonts, self.caret);
            scene.fill(
                inner.x + caret - offset,
                baseline,
                CARET,
                FIELD_SIZE * 1.4,
                [palette.ink[0], palette.ink[1], palette.ink[2], 255],
            );
        }
        scene.clip_to(within.x, within.y, within.width, within.height);
    }
}

/// What the window should do about what just happened in here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Act {
    /// Everything is filled in and the reader asked to go.
    SignIn,
}

/// The sign-in screen.
#[derive(Debug)]
pub struct SignIn {
    pub server: Field,
    pub login: Field,
    pub password: Field,
    pub code: Field,
    /// Whether the server has asked for a one-time code. False until it does:
    /// a field for a code most accounts do not have is a field most readers
    /// would stop and wonder about.
    pub wants_code: bool,
    /// A sign-in is in flight, so nothing may be pressed and the button says
    /// so.
    pub trying: bool,
    /// What went wrong last time, in the server's own words where there are
    /// any.
    pub failed: Option<String>,
    /// The card and the row at its foot, measured together.
    placed: Option<(Rect, Rect, Laid)>,
}

impl Default for SignIn {
    fn default() -> Self {
        Self {
            server: Field::new("server", "Server", "mattermost.example.com"),
            login: Field::new("login", "Email or username", ""),
            password: Field::new("password", "Password", "").masked(),
            code: Field::new("code", "One-time code", "123456"),
            wants_code: false,
            trying: false,
            failed: None,
            placed: None,
        }
    }
}

impl SignIn {
    /// The fields that are on screen, in the order the keyboard walks them.
    fn fields(&self) -> Vec<&Field> {
        let mut fields = vec![&self.server, &self.login, &self.password];
        if self.wants_code {
            fields.push(&self.code);
        }
        fields
    }

    fn field_mut(&mut self, slug: &str) -> Option<&mut Field> {
        match slug {
            "server" => Some(&mut self.server),
            "login" => Some(&mut self.login),
            "password" => Some(&mut self.password),
            "code" if self.wants_code => Some(&mut self.code),
            _ => None,
        }
    }

    /// Whether there is enough to try with.
    ///
    /// The code counts only once it has been asked for, so the button does not
    /// go dead the moment the server says an account has MFA.
    pub fn ready(&self) -> bool {
        !self.server.is_empty()
            && !self.login.is_empty()
            && !self.password.is_empty()
            && (!self.wants_code || !self.code.is_empty())
    }

    /// The server, as something `Url::parse` will take.
    ///
    /// Nobody types a scheme into a box that says `mattermost.example.com`,
    /// and `Url::parse` refuses a host on its own -- so a reader who typed
    /// exactly what the hint showed them got "relative URL without a base",
    /// which is not about anything they can see.
    pub fn server_url(&self) -> String {
        let said = self.server.text.trim().trim_end_matches('/');
        match said.contains("://") {
            true => said.to_string(),
            false => format!("https://{said}"),
        }
    }

    /// Says the server wants a code, and puts the keyboard in the field for
    /// it.
    pub fn ask_for_a_code(&mut self, input: &mut Input) {
        self.wants_code = true;
        self.code.text.clear();
        self.failed = Some("That account needs its one-time code.".to_string());
        input.focus_on(named().of("code"));
    }

    /// The first field with nothing in it, which is where the keyboard starts.
    pub fn start(&self, input: &mut Input) {
        let first = self
            .fields()
            .iter()
            .find(|field| field.is_empty())
            .map(|field| field.slug)
            .unwrap_or("server");
        input.focus_on(named().of(first));
    }

    /// Which field holds the keyboard, if one does.
    fn focused(&self, input: &Input) -> Option<&'static str> {
        let held = named().slug(input.focus()?)?;
        self.fields()
            .iter()
            .find(|field| field.slug == held)
            .map(|field| field.slug)
    }

    /// Moves the keyboard on one field, wrapping at the end.
    fn walk(&self, input: &mut Input, by: isize) {
        let fields = self.fields();
        let at = self
            .focused(input)
            .and_then(|slug| fields.iter().position(|field| field.slug == slug))
            .unwrap_or(0) as isize;
        let count = fields.len() as isize;
        let next = ((at + by) % count + count) % count;
        input.focus_on(named().of(fields[next as usize].slug));
    }

    pub fn measure(&mut self, fonts: &mut Fonts, window: Rect) {
        self.placed = Some(self.place(fonts, window));
    }

    fn laid(&self) -> Option<(Rect, Rect, &Laid)> {
        let (card, logo, row) = self.placed.as_ref()?;
        Some((*card, *logo, row))
    }

    /// How far down the card the first label sits.
    fn under_the_name() -> f32 {
        PAD + TITLE_LINE + SAID_LINE + GAP
    }

    /// The block of labels and boxes, from the first label to the last box.
    fn fields_tall(&self) -> f32 {
        let count = self.fields().len() as f32;
        count * (LABEL_LINE + FIELD_HEIGHT) + (count - 1.0) * FIELD_GAP
    }

    /// The gap and the line that says what went wrong, when there is one.
    ///
    /// Measured, because this is the one run on the card whose length is not
    /// known here: it can be a sentence of this program's own or whatever the
    /// server said, and a long one wraps.
    fn failed_tall(&self, fonts: &mut Fonts) -> f32 {
        let Some(said) = self.failed.as_deref() else {
            return 0.0;
        };
        FIELD_GAP
            + matterless_layout::extent_of(fonts, said, WIDTH - PAD * 2.0, style(SAID_SIZE)).height
    }

    /// How tall the card is, from what is in it.
    fn card_height(&self, fonts: &mut Fonts) -> f32 {
        Self::under_the_name()
            + self.fields_tall()
            + self.failed_tall(fonts)
            + GAP
            + BUTTON_HEIGHT
            + PAD
    }

    /// The card, the mark above it, and the row at its foot.
    ///
    /// The pair is centred together rather than the card alone: a card in the
    /// middle with a logo above it sits visibly low, because the logo is part
    /// of what the eye is centring.
    fn place(&self, fonts: &mut Fonts, window: Rect) -> (Rect, Rect, Laid) {
        let width = WIDTH.min(window.width - PAD * 2.0).max(0.0);
        let height = self.card_height(fonts);
        // The mark is only worth showing where there is room for it and the
        // card both. A short window keeps the card.
        let with_logo = LOGO + LOGO_GAP + height <= window.height - PAD * 2.0;
        let together = match with_logo {
            true => LOGO + LOGO_GAP + height,
            false => height,
        };
        let top = window.y + ((window.height - together) / 2.0).max(PAD);
        let logo = Rect::new(
            window.x + (window.width - LOGO) / 2.0,
            top,
            match with_logo {
                true => LOGO,
                false => 0.0,
            },
            match with_logo {
                true => LOGO,
                false => 0.0,
            },
        );
        let card = Rect::new(
            window.x + (window.width - width) / 2.0,
            top + logo.height + if with_logo { LOGO_GAP } else { 0.0 },
            width,
            height,
        );
        let row = Row::new(
            NAME,
            Rect::new(
                card.x,
                card.bottom() - PAD - BUTTON_HEIGHT,
                card.width,
                BUTTON_HEIGHT,
            ),
        )
        .pad(PAD)
        .depth(52)
        .enabled(self.ready() && !self.trying)
        .button(Button::primary("go", self.button_says()))
        .measure(fonts);
        (card, logo, row)
    }

    fn button_says(&self) -> &'static str {
        match self.trying {
            true => "Signing in...",
            false => "Sign in",
        }
    }

    /// Where each field's box goes inside the card.
    fn boxes_of(&self, card: Rect) -> Vec<(&'static str, Rect)> {
        let wrap = card.width - PAD * 2.0;
        let mut y = card.y + Self::under_the_name();
        let mut placed = Vec::new();
        for field in self.fields() {
            placed.push((
                field.slug,
                Rect::new(card.x + PAD, y + LABEL_LINE, wrap, FIELD_HEIGHT),
            ));
            y += LABEL_LINE + FIELD_HEIGHT + FIELD_GAP;
        }
        placed
    }

    pub fn boxes(&self, window: Rect) -> Vec<Placed> {
        let Some((card, _, row)) = self.laid() else {
            return Vec::new();
        };
        // The whole window first, so a press beside the card takes the
        // keyboard out of whichever field had it, and the card over it so a
        // press inside does not.
        let mut placed = vec![
            Placed {
                name: NAME.to_string(),
                rect: window,
                depth: 50,
            },
            named().at("card", card, 51),
        ];
        for (slug, rect) in self.boxes_of(card) {
            placed.push(named().at(slug, rect, 52));
        }
        placed.extend(row.boxes());
        placed
    }

    /// Takes the frame, and answers whether the reader asked to sign in.
    pub fn react(
        &mut self,
        fonts: &mut Fonts,
        input: &mut Input,
        clipboard: &mut String,
    ) -> Option<Act> {
        if self.trying {
            // Nothing is taken while one is in flight, including the
            // keystrokes: a reader who typed into a field whose value had
            // already been sent would be editing a request that has gone.
            return None;
        }
        // A press in a field takes the keyboard and puts the caret where it
        // landed; the same press held and moved drags a selection out. On the
        // press rather than on the click, because a selection is finished
        // before the button comes back up.
        let card = self.laid().map(|(card, _, _)| card);
        let boxes = card.map(|card| self.boxes_of(card)).unwrap_or_default();
        let holding = input.pressed().map(str::to_string);
        let starting = input.pressed_now().map(str::to_string);
        for (slug, rect) in boxes {
            let mine = named().of(slug);
            if holding.as_deref() != Some(mine.as_str()) {
                continue;
            }
            if starting.as_deref() == Some(mine.as_str()) {
                input.focus_on(mine.clone());
            }
            if let Some(field) = self.field_mut(slug) {
                field.pointer(fonts, input, rect, &named());
            }
        }
        // Tab walks the fields, and Shift+Tab walks back. Taken before the
        // field sees it, or a Tab would be typed into one.
        if input.struck(Key::Tab) {
            let back = input.mods().shift;
            input.took(Key::Tab);
            self.walk(input, if back { -1 } else { 1 });
        }
        let pressed = self.laid().and_then(|(_, _, row)| row.clicked(input)) == Some("go");
        // Return anywhere in the form, which is what everybody presses. It
        // goes on to the next field when there is one to fill rather than
        // sending half a form.
        let entered = input.took(Key::Enter);
        if let Some(slug) = self.focused(input)
            && let Some(field) = self.field_mut(slug)
        {
            field.react(input, clipboard);
        }
        if entered && !self.ready() {
            self.walk(input, 1);
            return None;
        }
        (pressed || entered).then_some(Act::SignIn)
    }

    pub fn draw(&self, into: &mut Canvas<'_>, input: &Input, window: Rect) {
        let Some((card, logo, row)) = self.laid() else {
            return;
        };
        let row = row.clone();
        let focused = self.focused(input);
        let boxes = self.boxes_of(card);
        let failed = self.failed.clone();
        {
            let Canvas {
                scene,
                painter,
                fonts,
                palette,
            } = into;
            scene.fill(
                window.x,
                window.y,
                window.width,
                window.height,
                palette.ground,
            );
            // The mark, in grey. Colour on it would be the one bright thing on
            // a screen whose point is that nothing has arrived yet.
            if logo.width > 0.0 {
                scene.extend([matterless_paint::Piece::Image {
                    x: logo.x,
                    y: logo.y,
                    width: logo.width,
                    height: logo.height,
                    key: LOGO_KEY.to_string(),
                    radius: 0.0,
                }]);
            }
            Panel::floating(card, CORNER, DROP)
                .edge(palette.rule)
                .fill(palette.surface)
                .draw(scene);

            let glyphs = painter.run(
                fonts,
                "MatterLess",
                card.x + PAD,
                card.y + PAD,
                Run::label(card.width - PAD * 2.0).sized(TITLE_SIZE).bold(),
            );
            scene.glyphs(glyphs, palette.ink, palette.faint);
            let glyphs = painter.run(
                fonts,
                SAID,
                card.x + PAD,
                card.y + PAD + TITLE_SIZE * 1.4,
                Run::label(card.width - PAD * 2.0).sized(SAID_SIZE),
            );
            scene.glyphs(glyphs, palette.faint, palette.faint);

            for (slug, rect) in &boxes {
                let label = self
                    .fields()
                    .iter()
                    .find(|field| field.slug == *slug)
                    .map(|field| field.label)
                    .unwrap_or("");
                let glyphs = painter.run(
                    fonts,
                    label,
                    rect.x,
                    rect.y - LABEL_LINE,
                    Run::label(rect.width).sized(LABEL_SIZE),
                );
                scene.glyphs(glyphs, palette.soft, palette.faint);
            }

            if let Some(said) = failed.as_deref()
                && let Some((_, last)) = boxes.last()
            {
                let glyphs = painter.run(
                    fonts,
                    said,
                    card.x + PAD,
                    last.bottom() + FIELD_GAP,
                    Run::label(card.width - PAD * 2.0).sized(SAID_SIZE),
                );
                scene.glyphs(glyphs, palette.danger, palette.danger);
            }
        }

        for (slug, rect) in boxes {
            let field = self.fields().into_iter().find(|field| field.slug == slug);
            if let Some(field) = field {
                field.draw(into, rect, window, focused == Some(slug));
            }
        }
        row.draw(into, input);
    }
}

/// Under the name on the card.
const SAID: &str = "Sign in to your Mattermost server.";

fn style(size: f32) -> matterless_layout::Style {
    matterless_layout::Style {
        size,
        line_height: size * 1.4,
        bold: false,
        italic: false,
        mono: false,
    }
}

/// The app's own icon, in grey, at the size the card wants it.
///
/// Decoded once: it is a 512px picture that never changes, and this is called
/// from the frame that uploads it.
pub fn logo() -> Option<(u32, u32, Vec<u8>)> {
    const ICON: &[u8] = include_bytes!("../resources/icons/icon.png");
    const SIDE: u32 = 128;
    let decoded = image::load_from_memory(ICON).ok()?.into_rgba8();
    let mut small = image::imageops::thumbnail(&decoded, SIDE, SIDE);
    for pixel in small.pixels_mut() {
        // Rec. 601 luma, which is what `image`'s own grayscale uses. The alpha
        // is left alone: the icon is a shape on nothing, and flattening its
        // alpha would put a grey square on the screen.
        let [red, green, blue, alpha] = pixel.0;
        let grey = (0.299 * f32::from(red) + 0.587 * f32::from(green) + 0.114 * f32::from(blue))
            .round()
            .clamp(0.0, 255.0) as u8;
        pixel.0 = [grey, grey, grey, alpha];
    }
    Some((SIDE, SIDE, small.into_raw()))
}

#[cfg(test)]
mod tests {
    use super::*;

    use matterless_ui::input::{Event, Mods};

    fn press(input: &mut Input, key: Key) {
        input.apply(Event::Key { key, down: true }, &[]);
    }

    fn holding(input: &mut Input, mods: Mods) {
        input.apply(Event::Modifiers(mods), &[]);
    }

    fn command() -> Mods {
        Mods {
            command: true,
            ..Mods::default()
        }
    }

    fn shift() -> Mods {
        Mods {
            shift: true,
            ..Mods::default()
        }
    }

    /// A field with `said` in it and everything selected.
    fn all_of(said: &str) -> Field {
        let mut field = Field::new("server", "Server", "");
        field.set(said);
        field.select_all();
        field
    }

    /// Windows reports `Ctrl+V` as a chord *and* as the letter "v".
    ///
    /// So a paste put the clipboard in and then typed a v after it, which is
    /// exactly what somebody pasting a password into this form got -- and a
    /// masked field shows one bullet per character, so the only sign of it was
    /// that the sign-in failed. The message box already guarded against this;
    /// this box was written without the guard and earned the same bug.
    #[test]
    fn a_paste_does_not_also_type_the_v() {
        let mut field = Field::new("server", "Server", "");
        let mut input = Input::default();
        holding(&mut input, command());
        press(&mut input, Key::Char('v'));
        input.apply(Event::Typed("v".to_string()), &[]);
        let mut clipboard = "mattermost.example.com".to_string();
        field.react(&mut input, &mut clipboard);
        assert_eq!(field.text, "mattermost.example.com");
    }

    #[test]
    fn typing_over_a_selection_replaces_it() {
        let mut field = all_of("wrong.example.com");
        let mut input = Input::default();
        input.apply(Event::Typed("right.example.com".to_string()), &[]);
        field.react(&mut input, &mut String::new());
        assert_eq!(field.text, "right.example.com");
    }

    #[test]
    fn select_all_then_backspace_empties_it() {
        let mut field = all_of("mattermost.example.com");
        let mut input = Input::default();
        press(&mut input, Key::Backspace);
        field.react(&mut input, &mut String::new());
        assert_eq!(field.text, "");
    }

    /// Ctrl+A reaches the field, and it is a chord rather than the letter.
    #[test]
    fn a_chord_selects_everything() {
        let mut field = Field::new("login", "Login", "");
        field.set("ada");
        let mut input = Input::default();
        holding(&mut input, command());
        press(&mut input, Key::Char('a'));
        input.apply(Event::Typed("a".to_string()), &[]);
        field.react(&mut input, &mut String::new());
        assert!(field.has_selection(), "Ctrl+A selected nothing");
        assert_eq!(field.text, "ada", "Ctrl+A typed an a as well");
    }

    /// A password never reaches the clipboard, selected or not.
    #[test]
    fn a_masked_field_will_not_be_copied() {
        let mut field = Field::new("password", "Password", "").masked();
        field.set("hunter2");
        field.select_all();
        assert!(field.has_selection());
        assert_eq!(field.selection(), None);

        let mut input = Input::default();
        holding(&mut input, command());
        press(&mut input, Key::Char('c'));
        let mut clipboard = String::new();
        field.react(&mut input, &mut clipboard);
        assert_eq!(clipboard, "", "the password reached the clipboard");
    }

    /// Left and Right collapse a selection to the end they point at, rather
    /// than stepping one character off wherever the caret happens to be.
    #[test]
    fn an_arrow_collapses_a_selection_to_the_end_it_points_at() {
        let mut field = all_of("ada");
        let mut input = Input::default();
        press(&mut input, Key::Left);
        field.react(&mut input, &mut String::new());
        assert!(!field.has_selection());
        assert_eq!(field.caret, 0);

        let mut field = all_of("ada");
        let mut input = Input::default();
        press(&mut input, Key::Right);
        field.react(&mut input, &mut String::new());
        assert!(!field.has_selection());
        assert_eq!(field.caret, 3);
    }

    #[test]
    fn shift_and_an_arrow_extends_rather_than_moves() {
        let mut field = Field::new("login", "Login", "");
        field.set("ada");
        let mut input = Input::default();
        holding(&mut input, shift());
        press(&mut input, Key::Left);
        field.react(&mut input, &mut String::new());
        assert_eq!(field.selection().as_deref(), Some("a"));
    }

    fn filled() -> SignIn {
        let mut form = SignIn::default();
        form.server.set("mattermost.example.com");
        form.login.set("ada");
        form.password.set("hunter2");
        form
    }

    #[test]
    fn a_bare_host_is_given_a_scheme() {
        let form = filled();
        assert_eq!(form.server_url(), "https://mattermost.example.com");
    }

    #[test]
    fn a_scheme_that_was_typed_is_kept() {
        let mut form = filled();
        form.server.set("http://localhost:8065/");
        // The trailing slash goes too: `Url::join` puts the api path on the
        // end either way, and keeping it makes the printed server name differ
        // from the one in `server.txt` by a character.
        assert_eq!(form.server_url(), "http://localhost:8065");
    }

    #[test]
    fn the_password_is_drawn_as_bullets_and_kept_as_itself() {
        let mut field = Field::new("password", "Password", "").masked();
        field.set("p\u{e4}ss");
        assert_eq!(field.text, "p\u{e4}ss");
        // Four characters, not the five bytes an accented one takes.
        assert_eq!(field.shown(), "\u{2022}\u{2022}\u{2022}\u{2022}");
    }

    #[test]
    fn a_pasted_line_loses_its_newline() {
        let mut field = Field::new("server", "Server", "");
        field.insert("mattermost.example.com\n");
        assert_eq!(field.text, "mattermost.example.com");
    }

    #[test]
    fn the_caret_walks_by_character_not_by_byte() {
        let mut field = Field::new("login", "Login", "");
        field.set("\u{e9}t\u{e9}");
        assert_eq!(field.caret, field.text.len());
        field.caret = field.back_one();
        // One character back from the end of "ete" with accents is two bytes,
        // and a caret that moved one would split the last letter.
        assert_eq!(&field.text[field.caret..], "\u{e9}");
    }

    /// Every part of the card inside it, in order, with nothing sitting on
    /// anything else.
    ///
    /// The height and the places were two sums over the same pieces. They
    /// agreed, but nothing said so, and the card grows a field and a line of
    /// its own accord -- an error under four boxes is a different card from
    /// the three-box one this was written against.
    /// Nothing the card fills is drawn outside the clip it landed in.
    ///
    /// This is the fault that reached the screen. A field with more text than
    /// box narrows the scene's clip to its own inside so the words do not run
    /// out over the card -- and never widened it again. Everything filled
    /// after it was drawn into that same narrow strip: the next field's panel,
    /// the one after that, and the Sign in button, all of them in the scene
    /// and none of them on screen. What a reader saw was three fields' worth
    /// of text with no boxes round it and no button at all.
    ///
    /// Stated as the general thing rather than as those three: a filled shape
    /// outside its own layer's clip is a shape nobody will ever see, and there
    /// is no reason to draw one.
    #[test]
    fn nothing_filled_is_drawn_outside_the_clip_it_lands_in() {
        let mut fonts = Fonts::new();
        let window = Rect::new(0.0, 0.0, 1002.0, 792.0);
        let mut form = filled();
        // Longer than the box, so it is a field that narrows the clip -- and
        // not the last one, so there is something after it to lose.
        form.server
            .set("mattermost.a-very-long-hostname-that-will-not-fit.example.com");
        form.wants_code = true;
        form.code.set("123456");
        form.failed = Some("That login or password was not accepted.".to_string());
        form.measure(&mut fonts, window);

        let mut scene = matterless_paint::Scene::default();
        let mut painter = matterless_paint::Painter::new();
        let palette = matterless_paint::Palette::default();
        let mut input = Input::default();
        form.start(&mut input);
        let mut canvas = Canvas {
            scene: &mut scene,
            painter: &mut painter,
            fonts: &mut fonts,
            palette: &palette,
        };
        form.draw(&mut canvas, &input, window);

        let mut outside = Vec::new();
        for layer in &scene.layers {
            let (cx, cy, cw, ch) = layer.clip;
            for piece in &layer.pieces {
                let matterless_paint::Piece::Fill {
                    x,
                    y,
                    width,
                    height,
                    ..
                } = piece
                else {
                    continue;
                };
                // A shadow is drawn bigger than what casts it on purpose.
                if *x < cx - 0.5
                    || *y < cy - 0.5
                    || x + width > cx + cw + 0.5
                    || y + height > cy + ch + 0.5
                {
                    outside.push((*x, *y, *width, *height, layer.clip));
                }
            }
        }
        assert!(
            outside.is_empty(),
            "{} filled shapes were drawn where nothing will show them: {outside:?}",
            outside.len()
        );
    }

    #[test]
    fn the_card_holds_what_it_is_measured_for() {
        let mut fonts = Fonts::new();
        let window = Rect::new(0.0, 0.0, 1002.0, 792.0);
        for code in [false, true] {
            for failed in [None, Some("That login or password was not accepted.")] {
                let mut form = filled();
                form.wants_code = code;
                form.failed = failed.map(str::to_string);
                form.measure(&mut fonts, window);
                let (card, _, row) = form.laid().expect("the card was measured");
                let boxes = form.boxes_of(card);
                assert_eq!(boxes.len(), if code { 4 } else { 3 });

                let mut floor = card.y + PAD;
                for (slug, rect) in &boxes {
                    assert!(
                        rect.y - LABEL_LINE >= floor - 0.5,
                        "{slug} sits on what is above it"
                    );
                    assert!(
                        rect.bottom() <= card.bottom() - PAD + 0.5,
                        "{slug} runs out of the bottom of the card"
                    );
                    floor = rect.bottom();
                }
                let go = row.rect("go").expect("the button was placed");
                assert!(
                    go.y >= floor + form.failed_tall(&mut fonts) - 0.5,
                    "the button sits on the last field, or on the line under it"
                );
                assert!(
                    go.bottom() <= card.bottom() - PAD + 0.5,
                    "the button runs out of the bottom of the card"
                );
                // And the card is no taller than what it holds: a card sized
                // for more than is in it is the same fault the other way up,
                // and it is the half that showed as a band of nothing.
                assert!(
                    card.bottom() - PAD - go.bottom() < GAP,
                    "the card is {:.1}px taller than what it holds",
                    card.bottom() - PAD - go.bottom()
                );
            }
        }
    }

    #[test]
    fn nothing_is_ready_until_every_field_has_something() {
        let mut form = SignIn::default();
        assert!(!form.ready(), "an empty form offered to sign in");
        form.server.set("mattermost.example.com");
        form.login.set("ada");
        assert!(!form.ready(), "a form with no password offered to sign in");
        form.password.set("hunter2");
        assert!(form.ready());
    }

    #[test]
    fn a_code_is_only_required_once_it_has_been_asked_for() {
        let mut form = filled();
        assert!(form.ready());
        form.wants_code = true;
        assert!(!form.ready(), "the code was asked for and not required");
        form.code.set("123456");
        assert!(form.ready());
    }
}
