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

/// Narrowing the column leaves the reader looking at the same message.
///
/// A scroll is a number of pixels and every row is re-measured when the column
/// changes, so the same number lands somewhere else. Opening a thread narrows
/// the conversation beside it, and the message whose replies had just been
/// asked for was the first thing to slide out from under the pointer.
#[test]
fn a_narrower_column_keeps_the_reader_where_they_were() {
    let mut fonts = Fonts::new();
    let mut stream = Stream::new("stream");
    stream.rows = (0..40)
        .map(|at| {
            let mut said = post(&format!("p{at}"), "");
            // Long enough that a narrower column wraps it differently, which
            // is the whole reason the pixels stop meaning what they meant.
            said.nodes = Arc::new(vec![Node::Paragraph {
                children: vec![Node::Text {
                    value: "a sentence with enough words in it to wrap ".repeat(6),
                }],
            }]);
            Row::Post { post: said }
        })
        .collect();

    let wide = panel();
    stream.lay_out(&mut fonts, wide.width);
    // Somewhere in the middle: at either end there is nothing to lose.
    stream.scroll = stream.reach(wide) / 2.0;
    let held = stream.holding(wide).expect("nothing to hold");

    // The column a thread pane leaves behind.
    let narrow = Rect::new(wide.x, wide.y, wide.width * 0.6, wide.height);
    stream.lay_out(&mut fonts, narrow.width);
    stream.clamp(narrow);
    let adrift = stream.holding(narrow).expect("nothing to hold");
    assert_ne!(
        adrift.0, held.0,
        "the column got narrower and nothing moved, so this proves nothing"
    );

    stream.hold(Some(held.clone()), narrow);
    let after = stream.holding(narrow).expect("nothing to hold");
    assert_eq!(after.0, held.0, "a different message is under the top edge");
    assert!(
        (after.1 - held.1).abs() < 1.0,
        "the same message, at a different height in it: {} against {}",
        after.1,
        held.1
    );
}

/// A named message stays put, not merely whatever was at the top.
///
/// Holding the top row keeps the top row still and lets everything below it
/// move, because the rows between are re-measured too -- so the message whose
/// replies had just been asked for still slid down the screen.
#[test]
fn a_named_message_is_the_one_that_stays_put() {
    let mut fonts = Fonts::new();
    let mut stream = Stream::new("stream");
    stream.rows = (0..40)
        .map(|at| {
            let mut said = post(&format!("p{at}"), "");
            said.nodes = Arc::new(vec![Node::Paragraph {
                children: vec![Node::Text {
                    value: "a sentence with enough words in it to wrap ".repeat(6),
                }],
            }]);
            Row::Post { post: said }
        })
        .collect();

    let wide = panel();
    stream.lay_out(&mut fonts, wide.width);
    stream.scroll = stream.reach(wide) / 2.0;
    // Not the row at the top: one further down, the way a message somebody
    // pointed at usually is.
    let top = stream.holding(wide).expect("nothing at the top").0;
    let named = format!(
        "post/p{}",
        top.strip_prefix("post/p")
            .unwrap()
            .parse::<usize>()
            .unwrap()
            + 3
    );
    let anchored = stream.holding_row(&named, wide).expect("not on screen");

    let narrow = Rect::new(wide.x, wide.y, wide.width * 0.6, wide.height);
    stream.lay_out(&mut fonts, narrow.width);
    stream.clamp(narrow);
    // What holding the top would have done, for comparison.
    stream.hold(stream.holding(wide), narrow);
    let by_top = stream.holding_row(&named, narrow).expect("gone");
    assert!(
        (by_top.1 - anchored.1).abs() > 1.0,
        "holding the top already kept it still, so this proves nothing"
    );

    stream.hold(Some(anchored.clone()), narrow);
    let after = stream.holding_row(&named, narrow).expect("gone");
    assert!(
        (after.1 - anchored.1).abs() < 1.0,
        "the named message moved: {} against {}",
        after.1,
        anchored.1
    );
}

/// A press says where it was, not where that person is first named.
///
/// The same person is named many times in a conversation, and the card that
/// opens has to point at the name under the pointer. Looked up by the press
/// afterwards it answered with the first of them, so pressing a name near the
/// bottom of the screen opened a card at the top of it.
#[test]
fn a_press_carries_the_words_that_were_pressed() {
    use matterless_paint::{Painter, Palette, Scene};

    let mut fonts = Fonts::new();
    let mut painter = Painter::new();
    let mut scene = Scene::default();
    let mut stream = Stream::new("stream");
    // Two messages naming the same person, so the press and the person are
    // not the same question.
    stream.rows = (0..2)
        .map(|at| {
            let mut said = post(&format!("p{at}"), "");
            said.nodes = Arc::new(vec![Node::Paragraph {
                children: vec![Node::UserMention {
                    username: "ada".into(),
                    everyone: false,
                }],
            }]);
            Row::Post { post: said }
        })
        .collect();
    stream.lay_out(&mut fonts, panel().width);
    // Presses are recorded as the rows are drawn, so nothing is pressable
    // until a frame has been built.
    stream.draw(
        &mut crate::sidebar::Canvas {
            scene: &mut scene,
            painter: &mut painter,
            fonts: &mut fonts,
            palette: &Palette::default(),
        },
        panel(),
        &Input::default(),
        &std::collections::HashMap::new(),
    );

    let placed = stream.boxes(panel(), None);
    let box_of = |name: &str| {
        placed
            .iter()
            .find(|item| item.name == name)
            .unwrap_or_else(|| panic!("{name} is placed"))
            .rect
    };
    let (first, second) = (box_of("stream/press/0"), box_of("stream/press/1"));
    assert!(
        second.y > first.y,
        "the two names are in the same place: {first:?} {second:?}"
    );

    let Some(Chose::Press { press, at }) = click(&mut stream, "stream/press/1") else {
        panic!("the second name was not pressable")
    };
    assert_eq!(press, matterless_layout::row::Press::Person("ada".into()));
    assert_eq!(
        at,
        Some(second),
        "the press pointed at the first `@ada` rather than the one pressed"
    );
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
        Some(Chose::Press {
            press: matterless_layout::row::Press::Link("https://example.com/a".to_string()),
            at: None,
        })
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
        Some(Chose::Press {
            press: matterless_layout::row::Press::Post {
                channel_id: "c9".to_string(),
                post_id: "quoted".to_string(),
            },
            at: None,
        })
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

    assert!(stream.to_post(&mut fonts, "p30", within));
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
    assert!(!stream.to_post(&mut fonts, "never-loaded", within));
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
        &std::collections::HashMap::new(),
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

/// Shaping a channel of crash reports costs a second, and the window replans
/// far more often than the conversation changes: opening it, the socket
/// signing in, a reaction landing, a resize. Six full passes in the first
/// seconds of the window's life, measured, before any of this existed.
#[test]
fn a_replan_reshapes_only_what_changed() {
    let mut fonts = Fonts::new();
    let mut stream = conversation(&mut fonts);
    let within = panel();
    let rows = stream.rows.len();
    assert_eq!(stream.reused(), 0, "the first layout has nothing to reuse");

    // The same plan again -- which is what almost every replan is.
    stream.lay_out(&mut fonts, within.width);
    assert_eq!(stream.reused(), rows, "all of it");

    // One message edited: that row and no other.
    let Some(Row::Post { post }) = stream.rows.get_mut(1) else {
        unreachable!("the second row is a post")
    };
    post.edited = true;
    post.nodes = Arc::new(vec![Node::Paragraph {
        children: vec![Node::Text {
            value: "a much longer line of text than the one that was there before,                     long enough to wrap onto a second line and change the height"
                .into(),
        }],
    }]);
    stream.lay_out(&mut fonts, within.width);
    assert_eq!(stream.reused(), rows - 1);

    // A row arriving above the others must not push them out of the cache:
    // matching on position would miss every row below the new one.
    stream.rows.insert(0, Row::UnreadDivider);
    stream.lay_out(&mut fonts, within.width);
    assert_eq!(stream.reused(), rows, "everything but the new row");
}

/// A narrower column wraps differently, so nothing shaped for the old one says
/// anything about the new one.
#[test]
fn a_different_width_reuses_nothing() {
    let mut fonts = Fonts::new();
    let mut stream = conversation(&mut fonts);
    stream.lay_out(&mut fonts, panel().width);
    assert_eq!(stream.reused(), stream.rows.len());
    stream.lay_out(&mut fonts, 420.0);
    assert_eq!(stream.reused(), 0);
}

/// A separator says "Today", and the row behind it is only a day number. A
/// window left open across midnight would keep yesterday's word for it.
#[test]
fn a_new_day_reshapes_the_separators() {
    let mut fonts = Fonts::new();
    let mut stream = conversation(&mut fonts);
    stream.lay_out(&mut fonts, panel().width);
    assert_eq!(stream.reused(), stream.rows.len());
    // Midnight: the same rows, a day later.
    let tomorrow = stream.theme.today + 1;
    let offset = stream.theme.utc_offset_minutes;
    stream.lay_out_on(&mut fonts, panel().width, tomorrow, offset);
    assert_eq!(stream.reused(), 0);
}

/// A conversation long enough that shaping all of it is worth avoiding.
fn many(count: usize) -> Vec<Row> {
    (0..count)
        .map(|n| Row::Post {
            post: post(&format!("p{n}"), ""),
        })
        .collect()
}

/// Shaping four hundred messages before the window can draw any of them cost a
/// second and a half in a channel of crash reports -- for messages nobody had
/// scrolled to. A conversation opens at its newest end, so that is the end
/// that has to be measured.
#[test]
fn opening_shapes_the_newest_end_and_leaves_the_rest_waiting() {
    let mut fonts = Fonts::new();
    let mut stream = Stream::new("stream");
    let within = panel();
    stream.plan(many(200), true);
    stream.lay_out(&mut fonts, within.width);
    stream.cover(&mut fonts, within.width, within);

    assert!(stream.waiting() > 0, "all of it was shaped anyway");
    assert_eq!(
        stream.waiting() + stream.rows.len(),
        200,
        "no row was lost between the two halves"
    );
    // Enough to fill the panel, which is the whole requirement: the reader
    // must never see the end of what has been measured.
    assert!(
        stream.total() >= within.height,
        "{} for a panel of {}",
        stream.total(),
        within.height
    );
    let Some(Row::Post { post }) = stream.rows.last() else {
        unreachable!("the last row is a post")
    };
    assert_eq!(post.post_id, "p199", "shaped from the wrong end");
}

/// The rest arrives in order, oldest last, and says how much taller it made
/// the list -- which is what the window needs to hold the reader's place.
#[test]
fn what_is_waiting_arrives_in_order_and_says_how_much_it_added() {
    let mut fonts = Fonts::new();
    let mut stream = Stream::new("stream");
    let within = panel();
    stream.plan(many(200), true);
    stream.lay_out(&mut fonts, within.width);
    let shaped = stream.rows.len();
    let before = stream.total();

    let grew = stream.fill(&mut fonts, within.width, 10);
    assert_eq!(stream.rows.len(), shaped + 10);
    assert_eq!(stream.waiting(), 200 - shaped - 10);
    assert!(grew > 0.0);
    assert!((stream.total() - before - grew).abs() < 0.5, "{grew}");

    let Some(Row::Post { post }) = stream.rows.first() else {
        unreachable!("the first row is a post")
    };
    assert_eq!(
        post.post_id,
        format!("p{}", 200 - shaped - 10),
        "the slice went on the wrong end"
    );
}

/// A page of older history arriving under somebody reading the top of the
/// channel: what is under their eye could be any of it, so all of it is
/// measured.
#[test]
fn a_reader_away_from_the_newest_message_gets_all_of_it_shaped() {
    let mut fonts = Fonts::new();
    let mut stream = Stream::new("stream");
    stream.plan(many(200), false);
    assert_eq!(stream.waiting(), 0);
    stream.lay_out(&mut fonts, panel().width);
    assert_eq!(stream.rows.len(), 200);
}

/// What the conversation *mentions* is not a question about what is on
/// screen. Asked over the shaped rows alone, it would have been answered about
/// the last dozen messages.
#[test]
fn the_plan_is_whole_even_while_part_of_it_is_unshaped() {
    let mut stream = Stream::new("stream");
    stream.plan(many(200), true);
    assert!(stream.waiting() > 0);
    assert_eq!(stream.planned().count(), 200);
}

/// A plan arriving while the last one still has rows waiting must not leave
/// them behind it -- least of all the plan that asks for all of it to be
/// shaped, which would otherwise hold a second copy of a channel it had just
/// been told to measure whole.
#[test]
fn a_fresh_plan_drops_what_the_last_one_was_still_waiting_on() {
    let mut stream = Stream::new("stream");
    stream.plan(many(200), true);
    assert!(stream.waiting() > 0);

    stream.plan(many(30), false);
    assert_eq!(stream.waiting(), 0);
    assert_eq!(stream.planned().count(), 30);

    stream.plan(many(200), true);
    assert!(stream.waiting() > 0);
    stream.plan(many(40), true);
    assert_eq!(stream.planned().count(), 40);
}

/// Following a link to a message the window has planned but not yet shaped.
///
/// Unshaped rows have no height and so nowhere to scroll to, and the answer
/// "further back than this channel is loaded" would have been a lie about a
/// message sitting in memory.
#[test]
fn a_message_still_waiting_to_be_shaped_can_still_be_jumped_to() {
    let mut fonts = Fonts::new();
    let mut stream = Stream::new("stream");
    let within = panel();
    stream.plan(many(200), true);
    stream.lay_out(&mut fonts, within.width);
    let waiting = stream.waiting();
    assert!(waiting > 0);

    assert!(stream.to_post(&mut fonts, "p7", within), "not found");
    assert!(
        stream.row_rect("p7", within).is_some(),
        "found but not brought into view"
    );
    // Only as far as it had to go, and no further: the rest of the channel is
    // still the window's to shape behind the reader.
    assert!(stream.waiting() < waiting);

    // And a message that really is not here still says so, rather than
    // shaping the whole channel looking for it twice.
    assert!(!stream.to_post(&mut fonts, "never-loaded", within));
}

/// A reaction on a message the reader has not scrolled back to yet still
/// belongs on the page. Asked over the shaped rows alone, the window would
/// have skipped the re-read and kept a stale pill there.
#[test]
fn a_message_still_waiting_to_be_shaped_is_still_held() {
    let mut stream = Stream::new("stream");
    stream.plan(many(200), true);
    assert!(stream.waiting() > 0);
    assert!(stream.holds_any(&["p3".to_string()]));
    assert!(!stream.holds_any(&["p999".to_string()]));
}
