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
///
/// At the width it has now, and at the one it is given next. Opening the
/// thread pane narrows this panel with a message already in the box, which
/// re-wraps it and makes it taller -- and the stream is handed whatever is
/// left, so a box measured for the old width either overlaps the
/// conversation or leaves a band of ground under it.
#[test]
fn the_composer_and_the_stream_tile_the_panel() {
    let mut fonts = Fonts::new();
    let mut composer = Composer::new(NAME);
    composer.lay_out(&mut fonts, panel().width);
    assert_eq!(composer.above(panel()).bottom(), composer.strip(panel()).y);
    assert_eq!(composer.strip(panel()).bottom(), panel().bottom());

    // A message long enough to wrap when the panel loses half its width,
    // which is roughly what the thread pane costs.
    composer.fill(
        "the quick brown fox jumps over the lazy dog and keeps on going well \
         past the end of any one line in this box",
        &mut fonts,
    );
    composer.lay_out(&mut fonts, panel().width);
    // Taken before the panel narrows: `strip` answers from the height the
    // box has now, whatever rect it is handed, so asking it about the wide
    // panel afterwards gives the narrow answer twice.
    let wide = composer.strip(panel()).height;

    let half = Rect::new(panel().x, panel().y, panel().width / 2.0, panel().height);
    composer.lay_out(&mut fonts, half.width);
    assert_eq!(composer.above(half).bottom(), composer.strip(half).y);
    assert_eq!(composer.strip(half).bottom(), half.bottom());
    assert!(
        composer.strip(half).height > wide,
        "the box did not grow when the panel narrowed under it: {} then {}",
        wide,
        composer.strip(half).height
    );
}

/// The box keeps a band of blank ground above itself, and says how wide.
///
/// The typing pill sits in that band, which is the whole of how it covers
/// half as much of the conversation -- so the band has to be real, the box
/// must not reach into it, and `margin` has to be the number that describes
/// it. Asserted here rather than trusted, because the pill is measured from
/// the conversation's bottom edge and the box is measured from the strip's
/// top, and nothing else makes those two agree.
#[test]
fn the_box_leaves_a_band_of_ground_above_itself() {
    let mut fonts = Fonts::new();
    let mut composer = Composer::new(NAME);
    composer.lay_out(&mut fonts, panel().width);

    let band = composer.margin();
    assert!(
        band > 0.0,
        "there is no room above the box to put anything in"
    );

    let strip = composer.strip(panel());
    assert_eq!(
        composer.above(panel()).bottom(),
        strip.y,
        "the conversation and the strip do not meet, so the band is not where the pill looks for it"
    );
    assert_eq!(
        strip.height - composer.box_height(),
        band * 2.0,
        "the box does not sit a margin in from both edges of its strip"
    );
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

/// Undo steps back one edit, and redo forward again.
///
/// A message half written is exactly where losing a word hurts, and nothing
/// answered `Ctrl+Z` at all. Changes rather than copies of the text: the
/// editor hands back what an edit was and can reverse one.
#[test]
fn undo_steps_back_one_edit_and_redo_forward() {
    let mut fonts = Fonts::new();
    let (mut composer, mut input) = ready(&mut fonts);
    // Two words, which are two steps: a run of typing ends after the space
    // that follows it, so a step is a word rather than a letter.
    type_text(&mut composer, &mut fonts, &mut input, "hello ");
    type_text(&mut composer, &mut fonts, &mut input, "world");
    assert_eq!(composer.text(), "hello world");

    holding(&mut input, command());
    press(&mut input, Key::Char('z'));
    frame(&mut composer, &mut fonts, &mut input);
    assert_eq!(
        composer.text(),
        "hello ",
        "undo took a letter rather than the word"
    );

    press(&mut input, Key::Char('z'));
    frame(&mut composer, &mut fonts, &mut input);
    assert_eq!(composer.text(), "", "the second undo did nothing");

    // And nothing past the beginning, rather than a panic on an empty stack.
    press(&mut input, Key::Char('z'));
    frame(&mut composer, &mut fonts, &mut input);
    assert_eq!(composer.text(), "");

    press(&mut input, Key::Char('y'));
    frame(&mut composer, &mut fonts, &mut input);
    assert_eq!(composer.text(), "hello ", "redo did not come back");
}

/// Typing after an undo throws away what was undone.
///
/// A reader who undid something and then wrote has chosen a different future;
/// offering to redo the one they left would be offering to throw away what
/// they just wrote.
#[test]
fn typing_after_an_undo_forgets_what_was_undone() {
    let mut fonts = Fonts::new();
    let (mut composer, mut input) = ready(&mut fonts);
    type_text(&mut composer, &mut fonts, &mut input, "hello");
    holding(&mut input, command());
    press(&mut input, Key::Char('z'));
    frame(&mut composer, &mut fonts, &mut input);
    assert_eq!(composer.text(), "");

    holding(&mut input, Mods::default());
    type_text(&mut composer, &mut fonts, &mut input, "goodbye");
    holding(&mut input, command());
    press(&mut input, Key::Char('y'));
    frame(&mut composer, &mut fonts, &mut input);
    assert_eq!(
        composer.text(),
        "goodbye",
        "redo reached past what was written after it"
    );
}

/// A word at a time, which is what every other text box does.
#[test]
fn command_backspace_rubs_out_a_word() {
    let mut fonts = Fonts::new();
    let (mut composer, mut input) = ready(&mut fonts);
    type_text(&mut composer, &mut fonts, &mut input, "hello there");
    holding(&mut input, command());
    press(&mut input, Key::Backspace);
    frame(&mut composer, &mut fonts, &mut input);
    assert_eq!(composer.text(), "hello ");
}

/// A field paints its box inside what it is given, so a caller with no room to
/// spare has to hand over the room round the box rather than the box itself.
///
/// The app's header is 44 tall where a one-line field wants 46. Handing over
/// the box's own rect squeezed everything inside it -- measured, a 240x28
/// field came out as a 226x14 border with two pixels of room for a
/// twenty-pixel line, so the words sat across the bottom edge and the box
/// looked broken. `around` is the way back.
#[test]
fn a_field_paints_its_box_where_it_was_asked_to() {
    let mut fonts = Fonts::new();
    let mut field = Composer::new("query").plain();
    field.lay_out(&mut fonts, 240.0);

    // Where the app's header puts it: centred in a 44-tall strip.
    let wanted = Rect::new(546.0, 6.0, 240.0, 32.0);
    let within = field.around(wanted);
    assert!(
        within.height >= field.height(),
        "the room round a 32-tall box is what a field needs: {within:?} against {}",
        field.height()
    );
    assert_eq!(
        field.box_height(),
        wanted.height,
        "and 32 is the box a one-line field paints"
    );

    // The strip itself has no room for that, which is the whole reason: the
    // rect handed over reaches past the strip while the box lands inside it.
    assert!(wanted.y >= 0.0 && wanted.bottom() <= 44.0, "the box fits");
    assert!(within.height > 44.0, "the room round it does not");
}

/// The header's field is the height a one-line field paints, so the two agree
/// without anybody doing arithmetic at the call site.
#[test]
fn the_headers_field_is_the_height_a_field_wants() {
    let mut fonts = Fonts::new();
    let mut field = Composer::new("query").plain();
    field.lay_out(&mut fonts, 240.0);

    let column = Rect::new(336.0, 0.0, 666.0, 792.0);
    let offers = crate::header::offered(false);
    let rect = crate::header::find(column, &offers).expect("a field on the strip");
    assert_eq!(rect.height, field.box_height());
}

/// Past the cap the box shows a window onto the text and follows the caret
/// into it.
///
/// Reported as a box that "doesn't handle long text properly", and the
/// screenshot showed why: the box stopped growing at `MAX_LINES` and the rest
/// of the message carried on down the window, drawn over the Send button and
/// out past the bottom edge. There was no window and no scroll -- `MAX_LINES`
/// said the text scrolled inside the box and nothing ever scrolled it.
#[test]
fn a_message_past_the_cap_scrolls_inside_the_box() {
    let mut fonts = Fonts::new();
    let (mut composer, mut input) = ready(&mut fonts);

    let tall = composer.height();
    for _ in 0..(MAX_LINES * 2) {
        holding(&mut input, shift());
        press(&mut input, Key::Enter);
        frame(&mut composer, &mut fonts, &mut input);
    }

    // The box stopped growing, as it always did.
    assert_eq!(
        composer.height(),
        MAX_LINES as f32 * LINE + PADDING * 2.0 + MARGIN * 2.0 + 34.0,
        "capped at {MAX_LINES} lines"
    );
    assert!(composer.height() > tall);

    // And the caret came with it: it is inside the box rather than somewhere
    // below the window.
    let caret = composer.caret(panel()).expect("a caret");
    let box_of = panel().bottom() - composer.height();
    assert!(
        caret.1 >= box_of && caret.1 + LINE <= panel().bottom(),
        "the caret sits at {} in a box from {box_of} to {}",
        caret.1,
        panel().bottom()
    );
}

/// Walking back up to the top brings the window with it, and walking down
/// again takes it back.
#[test]
fn the_window_follows_the_caret_both_ways() {
    let mut fonts = Fonts::new();
    let (mut composer, mut input) = ready(&mut fonts);
    for _ in 0..(MAX_LINES * 2) {
        holding(&mut input, shift());
        press(&mut input, Key::Enter);
        frame(&mut composer, &mut fonts, &mut input);
    }
    let at_the_end = composer.caret(panel()).expect("a caret").1;

    holding(&mut input, Mods::default());
    for _ in 0..(MAX_LINES * 2) {
        press(&mut input, Key::Up);
        frame(&mut composer, &mut fonts, &mut input);
    }
    let at_the_top = composer.caret(panel()).expect("a caret").1;
    let box_of = panel().bottom() - composer.height();
    assert!(
        at_the_top >= box_of && at_the_top + LINE <= panel().bottom(),
        "walking to the top keeps the caret in the box, not at {at_the_top}"
    );
    assert!(
        at_the_top < at_the_end + LINE,
        "and it is higher in the box than it was at the end"
    );

    for _ in 0..(MAX_LINES * 2) {
        press(&mut input, Key::Down);
        frame(&mut composer, &mut fonts, &mut input);
    }
    let back = composer.caret(panel()).expect("a caret").1;
    assert!(
        back >= box_of && back + LINE <= panel().bottom(),
        "and walking back down keeps it in too, not at {back}"
    );
}

/// Sending empties the box, so the window on it goes back to the top -- a
/// box of one line scrolled eight down would draw nothing at all.
#[test]
fn an_emptied_box_is_scrolled_back_to_the_top() {
    let mut fonts = Fonts::new();
    let (mut composer, mut input) = ready(&mut fonts);
    for _ in 0..(MAX_LINES * 2) {
        holding(&mut input, shift());
        press(&mut input, Key::Enter);
        frame(&mut composer, &mut fonts, &mut input);
    }
    composer.clear(&mut fonts);
    composer.lay_out(&mut fonts, panel().width);

    let caret = composer.caret(panel()).expect("a caret");
    let box_of = panel().bottom() - composer.height();
    assert!(
        (caret.1 - (box_of + MARGIN + PADDING)).abs() < 1.0,
        "the caret is on the first line at {}, box from {box_of}",
        caret.1
    );
}

/// A word picked out of a message is the only thing lit up.
///
/// Reported as a double-click selecting "some unrelated words", and it did:
/// `LayoutRun::highlight` only knows about the two lines a selection ends on,
/// and for any other line both of its tests short-circuit so every character
/// comes back selected. Which lines are in range at all is the caller's to
/// know. A box of one line was right and a box of several lit up every line
/// except the one being selected on.
#[test]
fn a_selection_lights_only_the_lines_it_is_on() {
    let mut fonts = Fonts::new();
    let (mut composer, placed, box_of) = holding_a_message(&mut fonts);

    // Into a word on the last line of the text.
    let x = box_of.x + MARGIN + PADDING + 140.0;
    let y = box_of.y + MARGIN + PADDING + LINE * 5.5;
    let mut input = Input::default();
    input.focus_on(NAME);
    input.apply(Event::PointerMoved { x, y }, &placed);
    input.apply(Event::PointerPressed, &placed);
    input.apply(Event::PointerRepeated(2), &placed);
    composer.react(&mut fonts, &input, panel(), &mut String::new());

    let ((start, _), (end, _)) = composer.bounds().expect("a word is selected");
    assert_eq!(start, end, "a word does not span lines");

    let lit = composer.selection_marks(panel());
    assert_eq!(
        lit.len(),
        1,
        "one word, one mark -- not {:?}",
        lit.iter().map(|(_, y, w)| (*y, *w)).collect::<Vec<_>>()
    );
    // And on the line it was clicked on, not somewhere up the box.
    assert!(
        (lit[0].1 - y).abs() < LINE,
        "the mark is at {} and the click was at {y}",
        lit[0].1
    );
}

/// Nothing outside the selection's own lines is touched, however many lines
/// the box holds.
#[test]
fn the_lines_around_a_selection_are_left_alone() {
    let mut fonts = Fonts::new();
    let (mut composer, placed, box_of) = holding_a_message(&mut fonts);

    let x = box_of.x + MARGIN + PADDING + 140.0;
    let y = box_of.y + MARGIN + PADDING + LINE * 5.5;
    let mut input = Input::default();
    input.focus_on(NAME);
    input.apply(Event::PointerMoved { x, y }, &placed);
    input.apply(Event::PointerPressed, &placed);
    input.apply(Event::PointerRepeated(2), &placed);
    composer.react(&mut fonts, &input, panel(), &mut String::new());

    let ((start, _), (end, _)) = composer.bounds().expect("a word is selected");
    for (_, at, width) in composer.selection_marks(panel()) {
        assert!(width > 0.0);
        let line = ((at - (box_of.y + MARGIN + PADDING)) / LINE).round() as usize;
        assert!(
            line <= end.saturating_sub(start) + 8,
            "a mark landed on line {line}, well outside the selection"
        );
    }
}

/// A message several lines long, in a box that holds it.
fn holding_a_message(fonts: &mut Fonts) -> (Composer, Vec<matterless_ui::Placed>, Rect) {
    let mut composer = Composer::new(NAME);
    composer.fill(
        "clean.\nNot a specific launch flag.\nCaveat on reproducing it\n\nAll three \
         occurrences were launches driven by the harness, which scripts the transitions \
         to skip the start screen. That makes travel begin far earlier than in a normal \
         boot, which plausibly widens this race considerably. It may be hard to repeat \
         by hand and easy to repeat under automation, so a failure to repeat one by \
         hand should not be taken as absence. The same harness has produced the \
         other two, on machines sharing neither their drivers nor their memory.",
        fonts,
    );
    composer.lay_out(fonts, panel().width);
    let box_of = Rect::new(
        panel().x,
        panel().bottom() - composer.height(),
        panel().width,
        composer.height(),
    );
    let placed = vec![matterless_ui::Placed {
        name: NAME.to_string(),
        rect: box_of,
        depth: 0,
    }];
    (composer, placed, box_of)
}

/// The wheel moves the window on the text.
///
/// Reported as "scroll doesn't react to mouse wheel". It did not: the window's
/// wheel path only reacts to the frame when a picker or a side pane is open,
/// so the box heard the turn in two windows out of three and never in the
/// ordinary one.
#[test]
fn the_wheel_moves_the_window_on_the_text() {
    let mut fonts = Fonts::new();
    let (mut composer, placed, _) = holding_a_message(&mut fonts);
    // Filled, so it sits at the bottom of its own text.
    let at_the_end = composer.caret(panel()).expect("a caret").1;

    let mut input = Input::default();
    input.apply(
        Event::PointerMoved {
            x: placed[0].rect.x + 100.0,
            y: placed[0].rect.y + 40.0,
        },
        &placed,
    );
    input.apply(Event::Wheel { x: 0.0, y: 60.0 }, &placed);
    composer.wheeled(&input, &placed, panel());

    let after = composer.caret(panel()).expect("a caret").1;
    assert!(
        after > at_the_end,
        "turning the wheel back should move the text down past the caret: \
         {at_the_end} then {after}"
    );

    // And it stops at the top rather than running on for ever.
    for _ in 0..20 {
        input.apply(Event::Wheel { x: 0.0, y: 60.0 }, &placed);
        composer.wheeled(&input, &placed, panel());
    }
    let top = composer.caret(panel()).expect("a caret").1;
    for _ in 0..5 {
        input.apply(Event::Wheel { x: 0.0, y: 60.0 }, &placed);
        composer.wheeled(&input, &placed, panel());
    }
    assert_eq!(
        composer.caret(panel()).expect("a caret").1,
        top,
        "clamped at the first line"
    );
}

/// The wheel belongs to the panel the pointer is over, like every other one.
#[test]
fn the_wheel_elsewhere_is_not_this_boxs() {
    let mut fonts = Fonts::new();
    let (mut composer, placed, _) = holding_a_message(&mut fonts);
    let before = composer.caret(panel()).expect("a caret").1;

    let mut input = Input::default();
    // Well above the box, over the conversation.
    input.apply(Event::PointerMoved { x: 500.0, y: 100.0 }, &placed);
    input.apply(Event::Wheel { x: 0.0, y: 60.0 }, &placed);
    composer.wheeled(&input, &placed, panel());

    assert_eq!(composer.caret(panel()).expect("a caret").1, before);
}

/// Home and End belong to the row the caret is on, and Ctrl to the whole
/// message.
///
/// Reported as Home and End going "to the top/bottom of what the input shows,
/// but not the actual top/bottom of the message", and they did neither:
/// `Motion::Home` reads as the start of a line and its body is
/// `cursor.index = 0` on the *logical* line, byte for byte what
/// `ParagraphStart` does. On a soft-wrapped paragraph that leaves the row
/// entirely -- measured at four rows up a full box -- which reads as the text
/// having jumped rather than the caret having gone home.
#[test]
fn home_and_end_stay_on_the_row_the_caret_is_on() {
    let mut fonts = Fonts::new();
    let (mut composer, _, box_of) = holding_a_message(&mut fonts);
    let mut input = Input::default();
    input.focus_on(NAME);
    let top = box_of.y + MARGIN + PADDING;

    // `fill` leaves the caret at the very end, on the last row of a wrapped
    // paragraph -- which is where the old Home jumped away from.
    let (was_x, was_y) = composer.caret(panel()).expect("a caret");
    assert!(was_y - top > LINE, "the caret is not on the first row");

    holding(&mut input, Mods::default());
    press(&mut input, Key::Home);
    frame(&mut composer, &mut fonts, &mut input);
    let (home_x, home_y) = composer.caret(panel()).expect("a caret");
    assert_eq!(home_y, was_y, "Home stays on its row");
    assert!(home_x < was_x, "and goes to the start of it");

    press(&mut input, Key::End);
    frame(&mut composer, &mut fonts, &mut input);
    let (end_x, end_y) = composer.caret(panel()).expect("a caret");
    assert_eq!(end_y, was_y, "and so does End");
    assert_eq!(end_x, was_x, "which is where it started");
}

/// Ctrl reaches the ends of the whole message, and brings the window with it.
#[test]
fn ctrl_home_and_end_reach_the_ends_of_the_message() {
    let mut fonts = Fonts::new();
    let (mut composer, _, box_of) = holding_a_message(&mut fonts);
    let mut input = Input::default();
    input.focus_on(NAME);
    let top = box_of.y + MARGIN + PADDING;

    holding(&mut input, command());
    press(&mut input, Key::Home);
    frame(&mut composer, &mut fonts, &mut input);
    let (_, y) = composer.caret(panel()).expect("a caret");
    assert!(
        (y - top).abs() < 1.0,
        "the first row of the message, and the window scrolled to it: {}",
        y - top
    );

    press(&mut input, Key::End);
    frame(&mut composer, &mut fonts, &mut input);
    let (_, y) = composer.caret(panel()).expect("a caret");
    assert!(
        y - top >= (MAX_LINES - 1) as f32 * LINE,
        "and back to the last row: {}",
        y - top
    );
}

/// Shift+Home takes the row up to the caret with it, rather than the
/// paragraph.
#[test]
fn shift_home_selects_back_along_the_row() {
    let mut fonts = Fonts::new();
    let (mut composer, _, _) = holding_a_message(&mut fonts);
    let mut input = Input::default();
    input.focus_on(NAME);

    holding(&mut input, shift());
    press(&mut input, Key::Home);
    frame(&mut composer, &mut fonts, &mut input);

    let picked = composer.selection().expect("something is selected");
    assert!(!picked.is_empty());
    assert!(
        !picked.contains('\n'),
        "one row, not the paragraph: {picked:?}"
    );
}
