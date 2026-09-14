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
        Row::ThreadFooter {
            root_id: "root".into(),
            reply_count: 2,
            last_reply_at: 0,
            participants: vec![matterless_render::ThreadFace {
                user_id: "u1".into(),
                name: "someone".into(),
                avatar_at: 0,
            }],
            unread_replies: 0,
            unread_mentions: 0,
            following: true,
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

/// Clicks the middle of a placed row and answers what the stream made of it.
fn click(stream: &mut Stream, name: &str) -> Option<Chose> {
    let within = panel();
    let placed = stream.boxes(within, None);
    let row = placed
        .iter()
        .find(|item| item.name == name)
        .unwrap_or_else(|| panic!("{name} is placed"));
    let mut input = Input::default();
    let at = (row.rect.x + 10.0, row.rect.y + row.rect.height / 2.0);
    input.apply(Event::PointerMoved { x: at.0, y: at.1 }, &placed);
    input.apply(Event::PointerPressed, &placed);
    input.apply(Event::PointerReleased, &placed);
    stream.react(&input, &placed, within)
}

/// The footer is the way in, and the only one.
#[test]
fn the_footer_opens_the_thread() {
    let mut fonts = Fonts::new();
    let mut stream = conversation(&mut fonts);
    assert_eq!(
        click(&mut stream, "stream/row/3"),
        Some(Chose::Thread("root".to_string()))
    );
}

/// A conversation of one message carrying both kinds of preview card.
fn carded(fonts: &mut Fonts) -> Stream {
    let mut stream = Stream::new("stream");
    let mut said = post("root", "");
    said.previews = vec![
        matterless_render::Preview::Page {
            url: "https://example.com/a".into(),
            title: "A page".into(),
            description: "what it is about".into(),
            site_name: "example.com".into(),
            image: None,
        },
        matterless_render::Preview::Permalink {
            post_id: "quoted".into(),
            channel_id: "c9".into(),
            channel_label: "somewhere".into(),
            author_name: "somebody".into(),
            create_at: 0,
            nodes: vec![Node::Paragraph {
                children: vec![Node::Text {
                    value: "what they said".into(),
                }],
            }],
        },
    ];
    stream.rows = vec![Row::Post { post: said }];
    stream.lay_out(fonts, panel().width);
    stream
}

/// A card is a link, and the whole card is it -- not just the words inside.
///
/// It was drawn and nothing else: pressing one did nothing at all, which for a
/// quoted message means the one thing it exists to offer was missing.
#[test]
fn a_page_card_opens_its_link() {
    let mut fonts = Fonts::new();
    let mut stream = carded(&mut fonts);
    assert_eq!(
        click(&mut stream, "stream/row/0/preview/0"),
        Some(Chose::Press(matterless_layout::row::Press::Link(
            "https://example.com/a".to_string()
        )))
    );
}

/// And a quoted card goes to the message it quotes, which is somewhere inside
/// the app rather than out of it.
#[test]
fn a_quoted_card_goes_to_the_message() {
    let mut fonts = Fonts::new();
    let mut stream = carded(&mut fonts);
    assert_eq!(
        click(&mut stream, "stream/row/0/preview/1"),
        Some(Chose::Press(matterless_layout::row::Press::Post {
            channel_id: "c9".to_string(),
            post_id: "quoted".to_string(),
        }))
    );
}

/// Following one means seeing what it was said among, so the message lands
/// with the conversation above it still on screen.
#[test]
fn following_a_message_leaves_the_conversation_above_it() {
    let mut fonts = Fonts::new();
    let mut stream = Stream::new("stream");
    stream.rows = (0..60)
        .map(|at| Row::Post {
            post: post(&format!("p{at}"), ""),
        })
        .collect();
    stream.lay_out(&mut fonts, panel().width);
    let within = panel();

    assert!(stream.to_post("p30", within));
    let rect = stream
        .row_rect("p30", within)
        .expect("it was scrolled into view");
    assert!(rect.y >= within.y && rect.bottom() <= within.bottom());
    assert!(
        rect.y > within.y + 10.0,
        "not jammed against the top, or its conversation is gone"
    );

    // A message that is not loaded cannot be scrolled to, and saying so is
    // what lets the caller leave the reader where they were.
    assert!(!stream.to_post("never-loaded", within));
}

/// Clicks the middle of a box while one row is hovered, which is what makes
/// that row's controls exist at all.
fn click_hovering(stream: &mut Stream, hovered: usize, name: &str) -> Option<Chose> {
    let within = panel();
    let placed = stream.boxes(within, Some(hovered));
    let target = placed
        .iter()
        .find(|item| item.name == name)
        .unwrap_or_else(|| panic!("{name} is placed"));
    let at = (
        target.rect.x + target.rect.width / 2.0,
        target.rect.y + target.rect.height / 2.0,
    );
    let mut input = Input::default();
    input.apply(Event::PointerMoved { x: at.0, y: at.1 }, &placed);
    input.apply(Event::PointerPressed, &placed);
    input.apply(Event::PointerReleased, &placed);
    stream.react(&input, &placed, within)
}

/// Three controls on the hovered message, not eight.
///
/// The port had every action as its own flat button across the top of the row,
/// which is the menu spilled into the message. The app offers react, reply and
/// more, and everything else is behind the third.
#[test]
fn a_hovered_message_offers_three_controls() {
    let mut fonts = Fonts::new();
    let stream = conversation(&mut fonts);
    let named: Vec<String> = stream
        .boxes(panel(), Some(1))
        .into_iter()
        .map(|placed| placed.name)
        .filter(|name| name.contains("/tool/"))
        .collect();
    assert_eq!(
        named,
        vec![
            "stream/row/1/tool/react",
            "stream/row/1/tool/reply",
            "stream/row/1/tool/more",
        ]
    );
    // And none at all on a message nobody is pointing at.
    assert!(
        !stream
            .boxes(panel(), None)
            .iter()
            .any(|placed| placed.name.contains("/tool/"))
    );
}

/// Reply opens the thread, which is what the row used to do when the whole
/// message was a button.
#[test]
fn reply_opens_the_thread() {
    let mut fonts = Fonts::new();
    let mut stream = conversation(&mut fonts);
    assert_eq!(
        click_hovering(&mut stream, 1, "stream/row/1/tool/reply"),
        Some(Chose::Thread("root".to_string()))
    );
}

/// The react button opens the quick faces rather than reacting: it is a way in
/// to seven of them and the search behind them, and reacting on the first
/// press would mean the reader never got to choose.
#[test]
fn react_opens_the_faces_rather_than_reacting() {
    let mut fonts = Fonts::new();
    let mut stream = conversation(&mut fonts);
    assert_eq!(
        click_hovering(&mut stream, 1, "stream/row/1/tool/react"),
        None
    );
    let open = stream.boxes(panel(), Some(1));
    assert!(open.iter().any(|placed| placed.name == "stream/faces/0"));

    // And then a face is the reaction it stands for.
    assert_eq!(
        click_hovering(&mut stream, 1, "stream/faces/0"),
        Some(Chose::React {
            post_id: "root".to_string(),
            emoji: "+1".to_string(),
            on: true,
        })
    );
    assert!(
        !stream
            .boxes(panel(), Some(1))
            .iter()
            .any(|placed| placed.name == "stream/faces/0"),
        "answered, so they are gone"
    );
}

/// The last face is not a face: it opens the whole picker, which is what
/// anything outside the quick seven needs.
#[test]
fn the_end_of_the_quick_row_opens_the_picker() {
    let mut fonts = Fonts::new();
    let mut stream = conversation(&mut fonts);
    click_hovering(&mut stream, 1, "stream/row/1/tool/react");
    let last = crate::actions::QUICK.len();
    assert_eq!(
        click_hovering(&mut stream, 1, &format!("stream/faces/{last}")),
        Some(Chose::Act {
            action: crate::actions::Action::React,
            post_id: "root".to_string(),
            on: true,
        })
    );
}

/// A click anywhere else puts the faces away, the way a `details` closes when
/// the page is clicked.
#[test]
fn the_faces_close_when_something_else_is_pressed() {
    let mut fonts = Fonts::new();
    let mut stream = conversation(&mut fonts);
    click_hovering(&mut stream, 1, "stream/row/1/tool/react");
    click_hovering(&mut stream, 1, "stream/faces/elsewhere");
    assert!(
        !stream
            .boxes(panel(), Some(1))
            .iter()
            .any(|placed| placed.name.starts_with("stream/faces")),
        "the catcher shut them"
    );
}

/// More asks for the menu, and says which button to hang it under: by the time
/// the shell reacts, the pointer has already moved.
#[test]
fn more_asks_for_the_menu_under_itself() {
    let mut fonts = Fonts::new();
    let mut stream = conversation(&mut fonts);
    let button = stream
        .boxes(panel(), Some(1))
        .into_iter()
        .find(|placed| placed.name == "stream/row/1/tool/more")
        .expect("the button");
    match click_hovering(&mut stream, 1, "stream/row/1/tool/more") {
        Some(Chose::More { post_id, under }) => {
            assert_eq!(post_id, "root");
            assert_eq!(under, button.rect);
        }
        other => panic!("{other:?}"),
    }
}

/// A message is not a button. It was one, and that swallowed every click on a
/// mention or a link inside it -- while giving no hint that pressing a
/// sentence would do anything at all.
#[test]
fn clicking_a_message_does_nothing() {
    let mut fonts = Fonts::new();
    let mut stream = conversation(&mut fonts);
    assert_eq!(click(&mut stream, "stream/row/1"), None);
    assert_eq!(click(&mut stream, "stream/row/2"), None);
}

/// The way back, offered only when there is somewhere to go back from.
///
/// A button that appears while somebody is reading the last few messages is
/// exactly the thing they do not want jumping into the middle of it.
#[test]
fn the_way_back_appears_only_once_the_reader_has_left() {
    let mut fonts = Fonts::new();
    let mut stream = conversation(&mut fonts);
    let within = panel();
    stream.to_bottom(within);
    assert_eq!(stream.behind(within), 0.0);
    assert!(stream.to_newest(within).is_none());

    // Far enough back that the newest message is out of sight.
    stream.scroll = 0.0;
    if stream.reach(within) > within.height * 0.75 {
        assert!(stream.to_newest(within).is_some());
    }
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
/// A picture's box is filled, then the mini preview, then the picture -- in
/// that order, because the scene is painted in the order it is built.
///
/// Drawn one on top of the other rather than one instead of the other: nothing
/// has to notice the moment the real bytes land, and nothing has to be taken
/// away when they do. Get the order wrong and the blurred kilobyte covers the
/// picture it was standing in for -- which is the same mistake this file has
/// now made in five other places.
#[test]
fn a_picture_is_drawn_over_its_own_placeholder() {
    use matterless_paint::{Painter, Palette, Piece, Scene};

    let mut fonts = Fonts::new();
    let mut painter = Painter::new();
    let mut scene = Scene::default();
    let mut shown = post("p1", "");
    shown.files = vec![matterless_render::FileRef {
        id: "f1".into(),
        name: "shot.png".into(),
        extension: "png".into(),
        size: 4096,
        mime_type: "image/png".into(),
        width: 1280,
        height: 720,
        image: true,
        video: false,
        variant: matterless_render::ImageVariant::Preview,
        mini_preview: Some("/9j/pretend".into()),
        box_width: 320,
        box_height: 180,
        archived: false,
    }];
    let mut stream = Stream::new("stream");
    stream.rows = vec![Row::Post { post: shown }];
    stream.lay_out(&mut fonts, panel().width);
    stream.draw(
        &mut crate::sidebar::Canvas {
            scene: &mut scene,
            painter: &mut painter,
            fonts: &mut fonts,
            palette: &Palette::default(),
        },
        panel(),
        &Input::default(),
    );

    let pieces: Vec<&Piece> = scene
        .layers
        .iter()
        .flat_map(|layer| layer.pieces.iter())
        .collect();
    let at = |wanted: &str| {
        pieces
            .iter()
            .position(|piece| matches!(piece, Piece::Image { key, .. } if key == wanted))
    };
    let mini = at("mini/f1").expect("the placeholder is drawn");
    let real = at("preview/f1").expect("the picture is drawn");
    assert!(
        mini < real,
        "the placeholder is painted at {mini}, over the picture at {real}"
    );
}

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
