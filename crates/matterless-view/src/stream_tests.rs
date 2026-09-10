//! What the stream has to get right about rows and the pointer.

use super::stream::{Chose, Stream};
use matterless_layout::Fonts;
use matterless_render::{PostRow, Row, markdown::Node};
use matterless_ui::Rect;
use matterless_ui::input::{Event, Input};
use std::sync::Arc;

fn post(id: &str, root: &str) -> PostRow {
    PostRow {
        post_id: id.into(),
        root_id: root.into(),
        author_id: "u1".into(),
        author_name: "someone".into(),
        create_at: 0,
        update_at: 0,
        edited: false,
        nodes: Arc::new(vec![Node::Paragraph {
            children: vec![Node::Text {
                value: "a line of text".into(),
            }],
        }]),
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

fn panel() -> Rect {
    Rect::new(260.0, 44.0, 600.0, 400.0)
}

fn conversation(fonts: &mut Fonts) -> Stream {
    let mut stream = Stream::new("stream");
    stream.rows = vec![
        Row::DateSeparator { epoch_day: 20_340 },
        Row::Post {
            post: post("root", ""),
        },
        Row::Post {
            post: post("reply", "root"),
        },
    ];
    stream.lay_out(fonts, panel().width);
    stream
}

/// A root is its own thread; a reply belongs to the root it hangs from.
#[test]
fn a_row_knows_which_thread_it_opens() {
    let mut fonts = Fonts::new();
    let stream = conversation(&mut fonts);
    assert_eq!(stream.root_of(1).as_deref(), Some("root"));
    assert_eq!(stream.root_of(2).as_deref(), Some("root"));
}

/// A date separator has no thread, and clicking one must do nothing rather
/// than open whichever thread was last under the pointer.
#[test]
fn a_separator_opens_nothing() {
    let mut fonts = Fonts::new();
    let stream = conversation(&mut fonts);
    assert_eq!(stream.root_of(0), None);
    assert_eq!(stream.root_of(99), None);
}

#[test]
fn clicking_a_message_opens_its_thread() {
    let mut fonts = Fonts::new();
    let mut stream = conversation(&mut fonts);
    let within = panel();
    let placed = stream.boxes(within, None);
    let row = placed
        .iter()
        .find(|item| item.name == "stream/row/1")
        .expect("the first message is placed");

    let mut input = Input::default();
    let at = (row.rect.x + 10.0, row.rect.y + row.rect.height / 2.0);
    input.apply(Event::PointerMoved { x: at.0, y: at.1 }, &placed);
    input.apply(Event::PointerPressed, &placed);
    input.apply(Event::PointerReleased, &placed);
    assert_eq!(
        stream.react(&input, &placed, within),
        Some(Chose::Thread("root".to_string()))
    );
}

/// Rows off the bottom are not placed at all, so a click where one would have
/// been finds nothing.
#[test]
fn only_the_visible_rows_are_placed() {
    let mut fonts = Fonts::new();
    let mut stream = Stream::new("stream");
    stream.rows = (0..400)
        .map(|index| Row::Post {
            post: post(&format!("p{index}"), ""),
        })
        .collect();
    stream.lay_out(&mut fonts, panel().width);

    let placed = stream.boxes(panel(), None);
    let rows = placed.len() - 1;
    assert!(rows > 0, "something is on screen");
    assert!(
        rows < 400,
        "a four-hundred-row channel placed all of them ({rows})"
    );
}

/// Two panels at once, so their rows must not answer to each other's names.
#[test]
fn a_thread_panel_names_its_rows_apart_from_the_channel() {
    let mut fonts = Fonts::new();
    let mut thread = conversation(&mut fonts);
    thread.name = "thread/root".to_string();
    let placed = thread.boxes(panel(), None);
    assert!(placed.iter().any(|item| item.name == "thread/root/row/1"));
    assert!(!placed.iter().any(|item| item.name.starts_with("stream/")));
}

#[test]
fn the_wheel_scrolls_the_panel_it_is_over_and_stops_at_the_end() {
    let mut fonts = Fonts::new();
    let mut stream = Stream::new("stream");
    stream.rows = (0..200)
        .map(|index| Row::Post {
            post: post(&format!("p{index}"), ""),
        })
        .collect();
    stream.lay_out(&mut fonts, panel().width);
    let within = panel();
    let placed = stream.boxes(within, None);

    let mut input = Input::default();
    input.apply(Event::PointerMoved { x: 400.0, y: 200.0 }, &placed);
    input.apply(Event::Wheel { x: 0.0, y: -120.0 }, &placed);
    stream.react(&input, &placed, within);
    assert_eq!(stream.scroll, 120.0);

    input.settle();
    input.apply(
        Event::Wheel {
            x: 0.0,
            y: -1_000_000.0,
        },
        &placed,
    );
    stream.react(&input, &placed, within);
    assert_eq!(stream.scroll, stream.reach(within));
}

/// Opening a conversation shows its newest message, not its oldest.
#[test]
fn a_conversation_opens_at_the_bottom() {
    let mut fonts = Fonts::new();
    let mut stream = Stream::new("stream");
    stream.rows = (0..100)
        .map(|index| Row::Post {
            post: post(&format!("p{index}"), ""),
        })
        .collect();
    stream.lay_out(&mut fonts, panel().width);
    stream.to_bottom(panel());
    assert!(stream.scroll > 0.0);
    assert_eq!(stream.scroll, stream.reach(panel()));
}
