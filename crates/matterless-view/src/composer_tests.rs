//! What the composer has to get right, checked without a window.
//!
//! Every one of these drives the same `react` the event loop drives, so what is
//! asserted here is what typing into the real box does.

use super::composer::{Composer, NAME};
use matterless_layout::Fonts;
use matterless_ui::Rect;
use matterless_ui::input::{Event, Input, Key, Mods};

/// The strip's own numbers, so a test says what it means rather than a literal.
const LINE: f32 = 20.0;
const PADDING: f32 = 10.0;
const MARGIN: f32 = 12.0;
const MAX_LINES: usize = 8;

fn panel() -> Rect {
    Rect::new(260.0, 44.0, 740.0, 700.0)
}

/// Focused and laid out, as the shell leaves it before a frame.
fn ready(fonts: &mut Fonts) -> (Composer, Input) {
    let mut composer = Composer::new(NAME);
    composer.lay_out(fonts, panel().width);
    let mut input = Input::default();
    input.focus_on(NAME);
    (composer, input)
}

fn press(input: &mut Input, key: Key) {
    input.apply(Event::Key { key, down: true }, &[]);
}

fn holding(input: &mut Input, mods: Mods) {
    input.apply(Event::Modifiers(mods), &[]);
}

fn shift() -> Mods {
    Mods {
        shift: true,
        ..Mods::default()
    }
}

fn command() -> Mods {
    Mods {
        command: true,
        ..Mods::default()
    }
}

/// One frame: give the input to the composer, then settle and reshape as the
/// event loop does.
fn frame(composer: &mut Composer, fonts: &mut Fonts, input: &mut Input) -> Option<String> {
    let sent = composer.react(fonts, input, panel(), &mut String::new());
    input.settle();
    composer.lay_out(fonts, panel().width);
    sent
}

fn type_text(composer: &mut Composer, fonts: &mut Fonts, input: &mut Input, text: &str) {
    input.apply(Event::Typed(text.to_string()), &[]);
    frame(composer, fonts, input);
}

#[test]
fn typing_reaches_the_buffer() {
    let mut fonts = Fonts::new();
    let (mut composer, mut input) = ready(&mut fonts);
    type_text(&mut composer, &mut fonts, &mut input, "hello");
    assert_eq!(composer.text(), "hello");
    assert!(composer.touched);
}

/// The convention every chat client shares, and the opposite of what a text
/// area does on its own -- which is why the composer decides it, not the editor.
#[test]
fn enter_sends_and_shift_enter_breaks_the_line() {
    let mut fonts = Fonts::new();
    let (mut composer, mut input) = ready(&mut fonts);
    type_text(&mut composer, &mut fonts, &mut input, "one");

    holding(&mut input, shift());
    press(&mut input, Key::Enter);
    assert_eq!(
        frame(&mut composer, &mut fonts, &mut input),
        None,
        "shift+enter must not send"
    );
    assert_eq!(composer.text(), "one\n");

    holding(&mut input, Mods::default());
    press(&mut input, Key::Enter);
    assert_eq!(
        frame(&mut composer, &mut fonts, &mut input),
        Some("one\n".to_string())
    );
    // Sending empties it, or the next message would carry the last one.
    assert_eq!(composer.text(), "");
}

/// Whitespace is not a message, and sending one would post an empty row.
#[test]
fn enter_on_nothing_sends_nothing() {
    let mut fonts = Fonts::new();
    let (mut composer, mut input) = ready(&mut fonts);
    type_text(&mut composer, &mut fonts, &mut input, "   ");
    press(&mut input, Key::Enter);
    assert_eq!(frame(&mut composer, &mut fonts, &mut input), None);
}

#[test]
fn backspace_removes_the_last_character() {
    let mut fonts = Fonts::new();
    let (mut composer, mut input) = ready(&mut fonts);
    type_text(&mut composer, &mut fonts, &mut input, "hey");
    press(&mut input, Key::Backspace);
    frame(&mut composer, &mut fonts, &mut input);
    assert_eq!(composer.text(), "he");
}

/// Deleting must not split a character. Working in graphemes is the whole
/// reason the editor owns the buffer rather than this holding a String.
#[test]
fn backspace_takes_a_whole_character_not_a_byte() {
    let mut fonts = Fonts::new();
    let (mut composer, mut input) = ready(&mut fonts);
    type_text(&mut composer, &mut fonts, &mut input, "café");
    press(&mut input, Key::Backspace);
    frame(&mut composer, &mut fonts, &mut input);
    assert_eq!(composer.text(), "caf");
}

#[test]
fn select_all_then_cut_empties_it_into_the_clipboard() {
    let mut fonts = Fonts::new();
    let (mut composer, mut input) = ready(&mut fonts);
    type_text(&mut composer, &mut fonts, &mut input, "take this");

    let mut clipboard = String::new();
    holding(&mut input, command());
    press(&mut input, Key::Char('a'));
    press(&mut input, Key::Char('x'));
    composer.react(&mut fonts, &input, panel(), &mut clipboard);
    input.settle();
    composer.lay_out(&mut fonts, panel().width);
    assert_eq!(clipboard, "take this");
    assert_eq!(composer.text(), "");

    press(&mut input, Key::Char('v'));
    composer.react(&mut fonts, &input, panel(), &mut clipboard);
    assert_eq!(composer.text(), "take this");
}

/// A chord is a command, not text. A platform reporting Ctrl+V as both would
/// otherwise paste and then type a v.
#[test]
fn a_chord_does_not_also_type_its_letter() {
    let mut fonts = Fonts::new();
    let (mut composer, mut input) = ready(&mut fonts);
    let mut clipboard = "pasted".to_string();
    holding(&mut input, command());
    press(&mut input, Key::Char('v'));
    input.apply(Event::Typed("v".to_string()), &[]);
    composer.react(&mut fonts, &input, panel(), &mut clipboard);
    assert_eq!(composer.text(), "pasted");
}

/// Typing must not reach a composer the reader is not in.
#[test]
fn an_unfocused_composer_ignores_the_keyboard() {
    let mut fonts = Fonts::new();
    let mut composer = Composer::new(NAME);
    composer.lay_out(&mut fonts, panel().width);
    let mut input = Input::default();
    input.focus_on("stream");
    input.apply(Event::Typed("hello".to_string()), &[]);
    composer.react(&mut fonts, &input, panel(), &mut String::new());
    assert_eq!(composer.text(), "");
}

/// The box grows with the message and gives the height back when sent, or the
/// stream above it would stay short after a long one.
#[test]
fn the_box_grows_with_the_message_and_shrinks_again() {
    let mut fonts = Fonts::new();
    let (mut composer, mut input) = ready(&mut fonts);
    let one_line = composer.height();

    for _ in 0..4 {
        holding(&mut input, shift());
        press(&mut input, Key::Enter);
        frame(&mut composer, &mut fonts, &mut input);
    }
    assert!(
        composer.height() > one_line,
        "four line breaks should have made it taller"
    );

    composer.clear(&mut fonts);
    composer.lay_out(&mut fonts, panel().width);
    assert_eq!(composer.height(), one_line);
}

/// However long the message, the composer must not eat the conversation.
#[test]
fn it_stops_growing_at_the_cap() {
    let mut fonts = Fonts::new();
    let (mut composer, mut input) = ready(&mut fonts);
    // Up to the cap it grows a line at a time.
    for _ in 0..MAX_LINES - 1 {
        holding(&mut input, shift());
        press(&mut input, Key::Enter);
        frame(&mut composer, &mut fonts, &mut input);
    }
    let capped = composer.height();
    assert!(capped > LINE + PADDING * 2.0 + MARGIN * 2.0, "it grew");

    // Past it, nothing moves. The property rather than the formula: the box
    // has a row of buttons along its bottom now, and a test that spells out
    // its height has to be edited every time anything is added to it.
    for _ in 0..MAX_LINES * 2 {
        holding(&mut input, shift());
        press(&mut input, Key::Enter);
        frame(&mut composer, &mut fonts, &mut input);
    }
    assert_eq!(composer.height(), capped);
}

/// The strip and the space above it tile the panel exactly, or a gap would
/// show the ground through between the stream and the composer.
#[test]
fn the_composer_and_the_stream_tile_the_panel() {
    let mut fonts = Fonts::new();
    let mut composer = Composer::new(NAME);
    composer.lay_out(&mut fonts, panel().width);
    assert_eq!(composer.above(panel()).bottom(), composer.strip(panel()).y);
    assert_eq!(composer.strip(panel()).bottom(), panel().bottom());
}

/// Two boxes on screen at once, so a keystroke has to reach exactly one. The
/// channel's composer and the thread's are the same widget under two names,
/// and without the name they would both take every key.
#[test]
fn only_the_focused_box_of_two_takes_the_keystrokes() {
    let mut fonts = Fonts::new();
    let mut channel = Composer::new(NAME);
    let mut thread = Composer::new("thread-composer");
    channel.lay_out(&mut fonts, panel().width);
    thread.lay_out(&mut fonts, panel().width);

    let mut input = Input::default();
    input.focus_on("thread-composer");
    input.apply(Event::Typed("a reply".to_string()), &[]);
    channel.react(&mut fonts, &input, panel(), &mut String::new());
    thread.react(&mut fonts, &input, panel(), &mut String::new());

    assert_eq!(thread.text(), "a reply");
    assert_eq!(channel.text(), "", "the channel's box is not focused");
}

/// Sending from one box must not empty the other: a draft in the channel has
/// to survive replying in a thread.
#[test]
fn sending_a_reply_leaves_the_channel_draft_alone() {
    let mut fonts = Fonts::new();
    let mut channel = Composer::new(NAME);
    let mut thread = Composer::new("thread-composer");
    channel.lay_out(&mut fonts, panel().width);
    thread.lay_out(&mut fonts, panel().width);

    let mut input = Input::default();
    input.focus_on(NAME);
    input.apply(Event::Typed("half a thought".to_string()), &[]);
    channel.react(&mut fonts, &input, panel(), &mut String::new());
    input.settle();

    input.focus_on("thread-composer");
    input.apply(Event::Typed("done".to_string()), &[]);
    thread.react(&mut fonts, &input, panel(), &mut String::new());
    input.settle();
    press(&mut input, Key::Enter);
    let sent = thread.react(&mut fonts, &input, panel(), &mut String::new());

    assert_eq!(sent, Some("done".to_string()));
    assert_eq!(thread.text(), "");
    assert_eq!(channel.text(), "half a thought");
}

/// A focused field shows a caret, empty or not.
///
/// Reported missing from the search box and the channel switcher. Both are
/// plain `Composer`s and both are drawn focused, so what this pins is the one
/// thing that could still be false: that a field with nothing typed in it has
/// a laid-out line for the caret to sit on.
#[test]
fn a_field_has_a_caret_before_anything_is_typed() {
    let mut fonts = Fonts::new();
    let mut field = Composer::new("query").plain();
    field.lay_out(&mut fonts, panel().width);
    let at = field
        .caret(panel())
        .expect("a focused field with no caret to draw");
    let inner = panel();
    assert!(
        at.0 >= inner.x && at.1 >= inner.y,
        "the caret is outside the box: {at:?}"
    );

    // And it moves with what is typed, rather than staying at the left edge.
    field.fill("jump to", &mut fonts);
    field.lay_out(&mut fonts, panel().width);
    let typed = field.caret(panel()).expect("no caret after typing");
    assert!(
        typed.0 > at.0,
        "the caret did not follow the words: {at:?} then {typed:?}"
    );
}
