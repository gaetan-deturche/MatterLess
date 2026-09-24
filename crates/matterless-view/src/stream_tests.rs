//! What the stream has to get right about rows and the pointer.

use super::stream::{Anchor, Chose, Stream};
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

/// The row being edited makes room, and gives up its words to do it.
///
/// Reported against the official client: there the row grows, the name and
/// the time stay above the box, and the conversation below is pushed down.
/// Here the editor floated at the row's own height, so it covered the name
/// over it and the messages under it -- 4.5 of the screenshots.
#[test]
fn the_edited_row_makes_room_for_the_box() {
    let mut fonts = Fonts::new();
    let mut stream = conversation(&mut fonts);
    let tall = 180.0_f32;
    let was = stream.total();
    let row_was = stream
        .row_rect("root", panel())
        .expect("the row is on screen")
        .height;

    stream.editing = Some(("root".to_string(), tall));
    stream.lay_out(&mut fonts, panel().width);

    let row = stream.row_rect("root", panel()).expect("still on screen");
    assert!(
        row.height > row_was,
        "the row did not grow: {} then {}",
        row_was,
        row.height
    );
    assert!(
        stream.total() > was,
        "the conversation did not get taller, so nothing was pushed down"
    );

    // The editor's own space is under the name rather than over it.
    let under = stream
        .editing_rect("root", panel())
        .expect("somewhere to put the box");
    assert!(
        under.y > row.y,
        "the box would be drawn over the name and the time"
    );
    assert_eq!(under.bottom(), row.bottom());
    assert!(
        (under.height - tall).abs() < 0.01,
        "{} of room for a box that asked for {tall}",
        under.height
    );
}

/// And it gives the room back, with its words, when the editor closes.
///
/// The layout cache is keyed on the row and not on what is being done to it,
/// so a trimmed layout left in there would come back as a short, wordless
/// row for the rest of the session.
#[test]
fn closing_the_editor_gives_the_row_its_words_back() {
    let mut fonts = Fonts::new();
    let mut stream = conversation(&mut fonts);
    let was = stream.total();
    let blocks = stream.laid[1].blocks.len();

    stream.editing = Some(("root".to_string(), 180.0));
    stream.lay_out(&mut fonts, panel().width);
    stream.editing = None;
    stream.lay_out(&mut fonts, panel().width);

    assert_eq!(stream.total(), was, "the conversation kept the extra room");
    assert_eq!(
        stream.laid[1].blocks.len(),
        blocks,
        "the row came back out of the cache with its words missing"
    );
}

/// The room the row makes is exactly the room the box takes, first time.
///
/// Reported twice. The arithmetic was right from the start -- room and panel
/// agree to the pixel at any given height -- and the fault was the order:
/// the row was told how tall the box was *before* the box had been laid out
/// at the width it was about to get, and a height is a number of lines,
/// which is an answer about a width. Measured, the row was told 116 where
/// the box then drew 136, and nothing re-shaped it -- so the box sat 20
/// pixels taller than its hole, over the name above it and the message
/// below, for as long as it was open.
///
/// So this runs the window's order twice and insists the first pass is
/// already right. A test that only checked the second would have passed
/// against the bug.
#[test]
fn the_room_the_row_makes_is_the_room_the_box_takes() {
    let mut fonts = Fonts::new();
    let mut stream = conversation(&mut fonts);
    let mut edit = crate::edit::Edit::new();
    let mut input = Input::default();
    edit.show(
        "root",
        "je l'avais review, mais il a change un truc la dessus apres parce \
         qu'il setais plante sur un nom de methode",
        &mut fonts,
        &mut input,
    );

    let within = panel();
    let mut first = None;
    for pass in 0..2 {
        // The width the box is about to get, then its height, then the row.
        let width = stream
            .editing_rect("root", within)
            .or_else(|| stream.row_rect("root", within))
            .map(|room| room.width)
            .unwrap_or(within.width);
        edit.box_of.lay_out(&mut fonts, width);
        stream.editing = Some(("root".to_string(), edit.height()));
        stream.lay_out(&mut fonts, within.width);

        let row = stream.row_rect("root", within).expect("a row");
        let room = stream.editing_rect("root", within).expect("room in it");
        let panel_of = edit.rect(room, within);

        assert_eq!(
            (panel_of.y, panel_of.height),
            (room.y, room.height),
            "pass {pass}: the box is not the size of the hole made for it"
        );
        assert!(
            room.y >= row.y + crate::stream::AVATAR_TOP + crate::stream::AVATAR,
            "pass {pass}: the box starts inside the face beside the name"
        );
        assert!(
            panel_of.bottom() <= row.bottom() + 0.01,
            "pass {pass}: the box runs past the row, into the message below"
        );
        match first {
            None => first = Some((room.y, room.height)),
            Some(was) => assert_eq!(
                was,
                (room.y, room.height),
                "the first pass was wrong and the second one settled it"
            ),
        }
    }
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

/// The toolbar stays up under a pointer that is not moving.
///
/// Reported off a screen recording: the strip flickered on and off every
/// frame while the pointer sat on it. The cause is a loop, and the loop is
/// what this reproduces -- the toolbar exists only on the hovered row, so a
/// button that does not count as its row takes the toolbar away the moment
/// the pointer reaches it; the pointer then lands on the row again, which
/// brings the toolbar back under the pointer, which takes it away.
///
/// The three quick faces were the ones missing from `hovered`, and they are
/// the first three buttons on the strip -- the ones most likely to be
/// pressed. So this walks *every* button rather than the one that was
/// reported, because the next one added is the next one to be forgotten.
#[test]
fn the_toolbar_stays_up_under_a_still_pointer() {
    let mut fonts = Fonts::new();
    let mut stream = conversation(&mut fonts);
    let within = panel();
    stream.favourites = vec![
        ("a".into(), "\u{1F44D}".into()),
        ("b".into(), "\u{1F44C}".into()),
        ("c".into(), "\u{1F382}".into()),
    ];

    let row = 1;
    let mut top = within.y + stream.theme.pad_top - stream.scroll;
    for laid in stream.laid.iter().take(row) {
        top += laid.height;
    }

    let mut buttons: Vec<(String, Rect)> = stream
        .favourites_at_for_test(row, top, within)
        .into_iter()
        .map(|(at, rect)| (format!("quick face {at}"), rect))
        .collect();
    buttons.extend(
        stream
            .tools(row, top, within)
            .into_iter()
            .map(|(tool, rect)| (format!("{tool:?}"), rect)),
    );
    assert!(
        buttons.len() >= 4,
        "a strip of {} is not enough to prove anything",
        buttons.len()
    );

    for (what, rect) in buttons {
        let at = (rect.x + rect.width / 2.0, rect.y + rect.height / 2.0);
        let mut input = Input::default();
        let mut hovered = None;
        // Four frames of the window's own order -- boxes built from the hover
        // it has, then the same pointer applied again -- which is enough for
        // a two-frame oscillation to show itself.
        for frame in 0..4 {
            let placed = stream.boxes(within, hovered);
            input.apply(Event::PointerMoved { x: at.0, y: at.1 }, &placed);
            hovered = stream.hovered(&input);
            assert_eq!(
                hovered,
                Some(row),
                "frame {frame}: the pointer on {what} stopped counting as its row, \
                 so the toolbar went out from under it (it was on {:?})",
                input.hovered()
            );
        }
    }
}

/// A conversation trails off at whichever end has more past it.
///
/// A list cut off square at the top of its panel says nothing about whether
/// there is anything above it, and the one thing a reader wants to know
/// about a wall of text is which way it goes on. But only the end that has
/// something past it: both at once on a conversation that fits the panel
/// would be two shadows cast by nothing.
#[test]
fn the_conversation_trails_off_at_the_end_that_has_more() {
    use matterless_paint::{Painter, Palette, Piece, Scene, Solid};

    let mut fonts = Fonts::new();
    let within = panel();

    // Enough rows to overflow the panel several times over.
    let ends = |stream: &mut Stream, fonts: &mut Fonts| {
        let mut painter = Painter::new();
        let mut scene = Scene::default();
        stream.draw(
            &mut crate::sidebar::Canvas {
                scene: &mut scene,
                painter: &mut painter,
                fonts,
                palette: &Palette::default(),
            },
            within,
            &Input::default(),
            &std::collections::HashMap::new(),
        );
        let mut top = false;
        let mut bottom = false;
        for layer in &scene.layers {
            for piece in &layer.pieces {
                if let Piece::Fade { solid, height, .. } = piece
                    // The cut-listing fade is a block inside a row and is
                    // never the full depth of an end.
                    && *height >= 20.0
                {
                    match solid {
                        Solid::Top => top = true,
                        Solid::Bottom => bottom = true,
                    }
                }
            }
        }
        (top, bottom)
    };

    let mut stream = wordy(&mut fonts, within.width);
    assert!(
        stream.reach(within) > 0.0,
        "the fixture fits the panel, so there is no end to trail off"
    );

    stream.scroll = 0.0;
    assert_eq!(
        ends(&mut stream, &mut fonts),
        (false, true),
        "at the top of the list: nothing above, more below"
    );

    stream.to_bottom(within);
    assert_eq!(
        ends(&mut stream, &mut fonts),
        (true, false),
        "at the end of the list: more above, nothing below"
    );

    stream.scroll = stream.reach(within) / 2.0;
    assert_eq!(
        ends(&mut stream, &mut fonts),
        (true, true),
        "in the middle: it goes on both ways"
    );
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
    assert_eq!(after.0, held.0, "a different message is across the middle");
    assert!(
        (after.1 - held.1).abs() < 1.0,
        "the same message, at a different height in it: {} against {}",
        after.1,
        held.1
    );
}

/// Opening a thread holds the "N replies" line, not the message above it.
///
/// That line is what was pressed, and the message it hangs under is often
/// several wrapped lines whose height changes in the narrower column -- so
/// anchoring the message pins the message's top and lets the line slide out
/// from under the pointer. The same complaint as holding the top row, one row
/// further down.
#[test]
fn the_replies_line_is_what_a_thread_opens_from() {
    let mut fonts = Fonts::new();
    let mut stream = Stream::new("stream");
    let wordy_post = |at: usize| {
        let mut said = post(&format!("p{at}"), "");
        said.nodes = Arc::new(vec![Node::Paragraph {
            children: vec![Node::Text {
                value: "a sentence with enough words in it to wrap ".repeat(6),
            }],
        }]);
        Row::Post { post: said }
    };
    stream.rows = (0..20).map(wordy_post).collect();
    // The message whose replies get asked for, and the line that asks.
    stream.rows.push(wordy_post(20));
    stream.rows.push(Row::ThreadFooter {
        root_id: "p20".into(),
        reply_count: 15,
        last_reply_at: 0,
        participants: Vec::new(),
        unread_replies: 0,
        unread_mentions: 0,
        following: true,
    });
    stream.rows.extend((21..40).map(wordy_post));

    let wide = panel();
    stream.lay_out(&mut fonts, wide.width);
    // With the pair somewhere in the middle, where there is something to lose.
    stream.scroll = stream.reach(wide) / 2.0;
    let (line, message) = ("footer/p20", "post/p20");
    let by_line = stream.holding_row(line, wide).expect("no replies line");
    let by_message = stream.holding_row(message, wide).expect("no message");

    // The column a thread pane leaves behind.
    let narrow = Rect::new(wide.x, wide.y, wide.width * 0.6, wide.height);
    stream.lay_out(&mut fonts, narrow.width);

    // What anchoring the message would have done to the line.
    stream.hold(Some(by_message), narrow);
    let adrift = stream.holding_row(line, narrow).expect("gone");
    assert!(
        (adrift.1 - by_line.1).abs() > 1.0,
        "anchoring the message already held the line still, so this proves nothing"
    );

    stream.hold(Some(by_line.clone()), narrow);
    let after = stream.holding_row(line, narrow).expect("gone");
    assert!(
        (after.1 - by_line.1).abs() < 1.0,
        "the replies line moved: {} against {}",
        after.1,
        by_line.1
    );
}

/// Opening a thread and closing it again leaves the channel where it started.
///
/// The two do different things on purpose. Opening honours the press and
/// holds the line that was clicked, which moves the column. Closing has no
/// press to honour, so what it owes the reader is the place they were in
/// before the pane took half of it -- and that place has to be remembered
/// across the open, because by then the column is somewhere else.
#[test]
fn opening_a_thread_and_closing_it_leaves_the_channel_where_it_was() {
    let mut fonts = Fonts::new();
    let wide = panel();
    let mut stream = wordy(&mut fonts, wide.width);
    stream.scroll = stream.reach(wide) / 2.0;
    let started = stream.scroll;

    // Where the channel was, kept for the way back.
    let was = stream.anchor(wide);
    // The press being honoured: some row other than the one the middle would
    // have picked, the way a footer further down the panel is.
    let pressed = stream
        .holding_row("post/p25", wide)
        .expect("not in the channel");

    // The pane opens, taking width, and the pressed line is held.
    let narrow = Rect::new(wide.x, wide.y, wide.width * 0.6, wide.height);
    stream.lay_out(&mut fonts, narrow.width);
    stream.hold(Some(pressed), narrow);
    assert!(
        (stream.scroll - started).abs() > 1.0,
        "the pane opening left the channel where it was, so this proves nothing"
    );

    // And closes again.
    stream.lay_out(&mut fonts, wide.width);
    stream.anchored(was, wide);
    assert!(
        (stream.scroll - started).abs() < 1.0,
        "the channel came back to {} instead of {started}",
        stream.scroll
    );
}

/// A long conversation of rows that wrap differently in a narrower column.
fn wordy(fonts: &mut Fonts, width: f32) -> Stream {
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
    stream.lay_out(fonts, width);
    stream
}

/// At either end it is an edge that is being read against, and no row can
/// stand in for one.
#[test]
fn the_anchor_is_an_edge_at_either_end_and_a_row_between() {
    let mut fonts = Fonts::new();
    let within = panel();
    let mut stream = wordy(&mut fonts, within.width);

    stream.to_bottom(within);
    assert_eq!(stream.anchor(within), Anchor::End);
    stream.scroll = 0.0;
    assert_eq!(stream.anchor(within), Anchor::Start);
    stream.scroll = stream.reach(within) / 2.0;
    assert!(matches!(stream.anchor(within), Anchor::Row(Some(_))));
}

/// The newest message is held against the bottom edge, which is the one place
/// it is ever supposed to be.
///
/// Holding a row instead leaves it adrift of that edge by however much the
/// rows below the held one have changed -- and a reader at the newest message
/// is the common case, so this was most of what still moved on a resize.
#[test]
fn the_newest_message_stays_against_the_bottom() {
    let mut fonts = Fonts::new();
    let wide = panel();
    let mut stream = wordy(&mut fonts, wide.width);
    stream.to_bottom(wide);
    let anchor = stream.anchor(wide);
    // What holding a row would have done, for comparison. Taken now, before
    // anything moves, which is when an anchor is ever taken.
    let by_row = stream.holding(wide);

    let narrow = Rect::new(wide.x, wide.y, wide.width * 0.6, wide.height);
    stream.lay_out(&mut fonts, narrow.width);
    stream.hold(by_row, narrow);
    assert!(
        stream.behind(narrow) > 1.0,
        "a row anchor already kept it against the bottom, so this proves nothing"
    );

    stream.anchored(anchor, narrow);
    assert!(
        stream.behind(narrow) <= 1.0,
        "the newest message sits {}px off the bottom edge",
        stream.behind(narrow)
    );
}

/// A taller panel keeps the middle on the same message, not the top edge.
///
/// Nothing re-wraps when only the height changes, so every row keeps its
/// place in the list and the whole question is where the panel's own middle
/// lands. Measuring the anchor from the top edge made a panel growing
/// downwards hold its rows against the top: the same rule the X axis follows
/// stopped being followed the moment the axis changed.
#[test]
fn a_taller_panel_keeps_the_middle_on_the_same_message() {
    let mut fonts = Fonts::new();
    let short = panel();
    let mut stream = wordy(&mut fonts, short.width);
    stream.scroll = stream.reach(short) / 2.0;

    let middle = stream.holding(short).expect("nothing to hold").0;
    let anchor = stream.anchor(short);

    // The window's bottom edge dragged down, which moves the panel's middle
    // and leaves every row exactly as it was.
    let tall = Rect::new(short.x, short.y, short.width, short.height + 200.0);
    stream.anchored(anchor, tall);

    assert_eq!(
        stream.holding(tall).expect("nothing to hold").0,
        middle,
        "a different message lies across the middle of the taller panel"
    );
}

/// A shorter panel keeps the newest message against the bottom.
///
/// Reported from the window: dragging the bottom edge *up* while at the end
/// of a channel threw the reader off it. A shorter panel reaches further, so
/// the distance from the newest message grows by exactly the height removed
/// -- which is why this has to be asked before the panel changes and not
/// after. Asked after, the reader looks like somebody who had scrolled up by
/// that much, and gets held by a row like one.
#[test]
fn a_shorter_panel_keeps_the_newest_message_against_the_bottom() {
    let mut fonts = Fonts::new();
    let tall = panel();
    let mut stream = wordy(&mut fonts, tall.width);
    stream.to_bottom(tall);

    let anchor = stream.anchor(tall);
    assert_eq!(anchor, Anchor::End, "at the end and not seen to be");

    let short = Rect::new(tall.x, tall.y, tall.width, tall.height - 200.0);
    // What the same question asked too late would have answered.
    assert!(
        stream.anchor(short) != Anchor::End,
        "the shorter panel still looks like the end, so this proves nothing"
    );

    stream.anchored(anchor, short);
    assert!(
        stream.behind(short) <= 1.0,
        "the newest message sits {}px off the bottom edge",
        stream.behind(short)
    );
}

/// A conversation knows how long it is before it has measured most of it.
///
/// The whole point of the remembered heights: a channel opens on its last
/// dozen rows and the hundred above them are shaped over the next fifth of a
/// second -- measured at 30 to 220ms on 125 of 466 openings -- and until they
/// are, the length is a growing number. A scrollbar drawn against a growing
/// number shrinks under the reader; held back until it settles, it appears
/// out of nowhere six to thirteen frames in. Knowing the length answers both.
#[test]
fn a_remembered_length_is_known_before_the_rows_are_measured() {
    let mut fonts = Fonts::new();
    let within = panel();
    let stream = wordy(&mut fonts, within.width);
    // Everything measured: this is what the reader would have seen last time,
    // and what would have been written down.
    let whole = stream.total();
    let measured: std::collections::HashMap<String, f32> = stream.measured().into_iter().collect();
    assert!(
        measured.len() > 12,
        "the fixture is too short to prove anything"
    );

    // Opened afresh, the way a channel does: only the newest end is shaped.
    let mut opened = Stream::new("stream");
    opened.rows = stream.rows.clone();
    opened.plan(stream.rows.clone(), true);
    opened.lay_out(&mut fonts, within.width);
    opened.cover(&mut fonts, within.width, within);
    assert!(opened.waiting() > 0, "the whole channel was shaped at once");
    assert!(
        !opened.knows_its_length(),
        "it claims to know a length it has not been told"
    );
    let shaped_only = opened.total();

    // Told what it measured to last time.
    opened.foresee(&measured);
    assert!(opened.knows_its_length());
    assert!(
        (opened.total() - whole).abs() < 1.0,
        "remembered length {} against the real {whole}",
        opened.total()
    );
    assert!(
        opened.total() > shaped_only,
        "the remembered rows added nothing"
    );

    // And one row it has never heard of is enough to stop it claiming.
    let mut partial = measured.clone();
    partial.remove(&opened.waiting_keys()[0]);
    opened.foresee(&partial);
    assert!(
        !opened.knows_its_length(),
        "a conversation half remembered is claiming to know its length"
    );
}

/// A remembered height stands in for a row until the row is measured, and
/// then it has to leave. It did not: the rows were shaped and counted, and the
/// remembered length stayed on top of them -- a conversation-length void above
/// the first message, and a scrollbar measuring a channel twice as long.
#[test]
fn a_remembered_length_leaves_as_the_rows_are_shaped() {
    let mut fonts = Fonts::new();
    let within = panel();
    let stream = wordy(&mut fonts, within.width);
    let whole = stream.total();
    let measured: std::collections::HashMap<String, f32> = stream.measured().into_iter().collect();

    let mut opened = Stream::new("stream");
    opened.plan(stream.rows.clone(), true);
    opened.lay_out(&mut fonts, within.width);
    opened.cover(&mut fonts, within.width, within);
    opened.foresee(&measured);
    assert!(opened.waiting() > 0);

    while opened.waiting() > 0 {
        opened.fill(&mut fonts, within.width, 8);
        assert!(
            (opened.total() - whole).abs() < 1.0,
            "{} rows still waiting, and the length is {} against {whole}",
            opened.waiting(),
            opened.total()
        );
    }
}

/// A channel opened on its newest rows, with the rest waiting and their
/// heights remembered: the rows it has are drawn below all of that.
fn opened_with_a_memory(fonts: &mut Fonts, within: Rect) -> Stream {
    let whole = wordy(fonts, within.width);
    let measured: std::collections::HashMap<String, f32> = whole.measured().into_iter().collect();
    let mut opened = Stream::new("stream");
    opened.plan(whole.rows.clone(), true);
    opened.lay_out(fonts, within.width);
    opened.foresee(&measured);
    assert!(
        opened.waiting() > 0 && opened.knows_its_length(),
        "the fixture is not what it says"
    );
    opened
}

/// Taking the reader to a row lands them on that row, however much of the
/// channel above it is still only remembered. It started counting from the
/// top of what was shaped, so the rail's "where you stopped reading" aimed the
/// whole remembered length too high and the divider was never on screen.
#[test]
fn going_to_a_row_lands_on_it_while_the_rest_is_remembered() {
    let mut fonts = Fonts::new();
    let within = panel();
    let mut stream = opened_with_a_memory(&mut fonts, within);
    let key = stream.measured()[2].0.clone();
    let above = 30.0;
    assert!(stream.to_row(&key, above, within));
    // A strip whose middle is half a pixel below `above`: the row asked for
    // starts at `above`, so the strip's middle is half a pixel into it.
    let (held, under) = stream
        .holding(Rect::new(
            within.x,
            within.y,
            within.width,
            2.0 * above + 1.0,
        ))
        .expect("something at the top");
    assert_eq!(
        held, key,
        "the panel's top is not at the row that was asked for"
    );
    assert!(
        (under - 0.5).abs() < 1.0,
        "the row starts {}px from where it should",
        under - 0.5
    );
}

/// Putting a reader back after a relayout puts them where they were, while
/// the rows above are only remembered.
#[test]
fn a_reader_is_put_back_where_they_were_while_the_rest_is_remembered() {
    let mut fonts = Fonts::new();
    let within = panel();
    let mut stream = opened_with_a_memory(&mut fonts, within);
    stream.scroll = (stream.reach(within) - 300.0).max(0.0);
    let before = stream.holding(within).expect("a row across the middle");
    stream.hold(Some(before.clone()), within);
    let after = stream.holding(within).expect("a row across the middle");
    assert_eq!(after.0, before.0, "a different row across the middle");
    assert!(
        (after.1 - before.1).abs() < 0.5,
        "moved by {}px",
        after.1 - before.1
    );
}

/// An attachment's colour is what the integration wrote, in any of the forms
/// integrations write it -- and nothing at all for anything else, so a typo
/// falls back to the quiet bar rather than to black.
#[test]
fn an_attachment_colour_is_read_as_written() {
    use super::stream::attachment_colour;
    assert_eq!(attachment_colour("#e01e5a"), Some([0xe0, 0x1e, 0x5a, 0xff]));
    assert_eq!(attachment_colour("#f00"), Some([0xff, 0x00, 0x00, 0xff]));
    assert_eq!(
        attachment_colour(" #36A64F "),
        Some([0x36, 0xa6, 0x4f, 0xff])
    );
    assert!(attachment_colour("danger").is_some());
    assert!(attachment_colour("Good").is_some());
    assert_eq!(attachment_colour("#12345"), None);
    assert_eq!(attachment_colour("red"), None);
    assert_eq!(attachment_colour("#gg0000"), None);
}

/// The next conversation is not the last one. Opening a channel with nothing
/// waiting kept the length remembered for the channel before it, and drew its
/// one message that far down the panel.
#[test]
fn a_new_plan_forgets_the_last_conversations_length() {
    let mut fonts = Fonts::new();
    let within = panel();
    let stream = wordy(&mut fonts, within.width);
    let measured: std::collections::HashMap<String, f32> = stream.measured().into_iter().collect();

    let mut opened = Stream::new("stream");
    opened.plan(stream.rows.clone(), true);
    opened.lay_out(&mut fonts, within.width);
    opened.foresee(&measured);
    assert!(opened.total() > opened.laid.iter().map(|row| row.height).sum::<f32>());

    // A short conversation: nothing waits.
    let short: Vec<_> = stream.rows[stream.rows.len() - 2..].to_vec();
    opened.plan(short, true);
    opened.lay_out(&mut fonts, within.width);
    assert_eq!(opened.waiting(), 0);
    let shaped: f32 = opened.laid.iter().map(|row| row.height).sum();
    assert!(
        (opened.total() - shaped).abs() < 0.5,
        "two rows measuring {shaped} report a length of {}",
        opened.total()
    );
}

/// Changing what a height *means* empties the kept rows.
///
/// The text size is going to be the reader's to choose. The width was already
/// a key of the row cache and the rest of the theme was not, so a size changed
/// under a warm cache would have handed back every height measured at the old
/// one -- confidently, and wrong by a line per row.
#[test]
fn a_different_text_size_is_a_different_set_of_heights() {
    let mut fonts = Fonts::new();
    let within = panel();
    let mut stream = wordy(&mut fonts, within.width);
    let at_default = stream.total();
    assert!(at_default > 0.0);

    // What a reader picking a larger size would do to the theme.
    let mut bigger = stream.theme;
    bigger.body_size += 3.0;
    bigger.line_height += 4.0;
    assert_ne!(
        stream.theme.fingerprint(),
        bigger.fingerprint(),
        "a text size the reader can change is not in the fingerprint"
    );

    // The same rows, measured again at the bigger size, must be taller --
    // which can only happen if the cache let go of the old answers.
    stream.theme_for_test(bigger);
    stream.relay_seen(&mut fonts, within.width, within);
    let at_bigger = stream.total();
    assert!(
        at_bigger > at_default,
        "the conversation is {at_bigger} at the larger size against {at_default} at the smaller"
    );
}

/// A reader part-way up keeps the foot of the conversation, not the middle.
///
/// The case the anchor alone gets wrong, and the one that was reported: a
/// message box growing a line takes that line off the bottom of the panel,
/// and holding the row across the middle keeps the words still while the
/// newest message slides under the box. In a conversation the bottom is the
/// edge that matters -- a growing box must not cover what is being replied
/// to. The window adds the height it lost to the scroll; this is that sum.
#[test]
fn a_panel_losing_its_foot_keeps_what_was_at_the_bottom() {
    let mut fonts = Fonts::new();
    let tall = panel();
    let mut stream = wordy(&mut fonts, tall.width);
    stream.to_bottom(tall);
    // Part-way up, so neither edge answers for the reader.
    stream.scroll -= 150.0;
    assert!(stream.behind(tall) > 1.0 && stream.scroll > 1.0);
    let behind = stream.behind(tall);

    // The box grows by a line: the panel loses twenty pixels from its foot.
    let short = Rect::new(tall.x, tall.y, tall.width, tall.height - 20.0);
    let held = stream.anchor(tall);
    stream.anchored(held, short);
    // What the window then does, and the whole of this fix: the distance
    // from the newest message is what is kept, rather than the height that
    // was lost -- the anchor has already moved the scroll by half of that.
    let reach = stream.reach(short);
    stream.scroll = (reach - behind).clamp(0.0, reach);

    assert!(
        (stream.behind(short) - behind).abs() < 1.0,
        "the foot moved: {behind}px behind before, {}px after",
        stream.behind(short)
    );
}

/// And the top of the channel is the same in reverse.
///
/// Where holding a row is not merely useless but wrong: the rows above the
/// held one growing taller pushes the scroll up to keep it still, and the
/// first message in the channel slides off the top of the panel.
#[test]
fn the_top_of_the_channel_stays_at_the_top() {
    let mut fonts = Fonts::new();
    let wide = panel();
    let mut stream = wordy(&mut fonts, wide.width);
    stream.scroll = 0.0;
    let anchor = stream.anchor(wide);
    let by_row = stream.holding(wide);

    let narrow = Rect::new(wide.x, wide.y, wide.width * 0.6, wide.height);
    stream.lay_out(&mut fonts, narrow.width);
    stream.hold(by_row, narrow);
    assert!(
        stream.scroll > 1.0,
        "a row anchor already stayed at the top, so this proves nothing"
    );

    stream.anchored(anchor, narrow);
    assert_eq!(
        stream.scroll, 0.0,
        "the first message is {}px above the top edge",
        stream.scroll
    );
}

/// A named message stays put, not merely whichever row the anchor would pick.
///
/// Whatever is held is the only thing that does not move; everything else
/// slides by however much the rows between it and the anchor have changed. So
/// the message whose replies had just been asked for still slid down the
/// screen, being a few rows off the one being held.
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
    // Not the row the anchor would pick: one further down, the way a message
    // somebody pointed at usually is.
    let top = stream.holding(wide).expect("nothing to hold").0;
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
    // What holding the anchor's own row would have done, for comparison.
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

/// The reader's most-used sit on the strip, and one press reacts with one.
///
/// Which is the whole of why they are there: a press, a row of faces and
/// another press is three for the reaction anybody makes most often.
#[test]
fn a_most_used_face_reacts_in_one_press() {
    let mut fonts = Fonts::new();
    let mut stream = conversation(&mut fonts);
    stream.favourites = vec![
        ("+1".to_string(), "\u{1f44d}".to_string()),
        ("tada".to_string(), "\u{1f389}".to_string()),
    ];
    assert_eq!(
        click_hovering(&mut stream, 1, "stream/row/1/quick/1"),
        Some(Chose::React {
            post_id: "root".to_string(),
            emoji: "tada".to_string(),
            on: true,
        })
    );
    // And they sit before the controls, which is where a press lands first.
    let placed = stream.boxes(panel(), Some(1));
    let face = placed
        .iter()
        .find(|one| one.name == "stream/row/1/quick/0")
        .expect("no face on the strip");
    let react = placed
        .iter()
        .find(|one| one.name == "stream/row/1/tool/react")
        .expect("no react button");
    assert!(
        face.rect.right() <= react.rect.x,
        "the faces are not before the controls"
    );
}

/// A row keeps its toolbar while something it opened is still up.
///
/// The row of quick faces and the emoji grid both hang off a button on that
/// toolbar, and both are reached by moving the pointer off it -- so a toolbar
/// that answered only the pointer went away as soon as either was aimed at,
/// leaving the panel hanging off nothing. Which is the same argument as a
/// button inside a row counting as that row, one step further out.
#[test]
fn a_row_keeps_its_toolbar_while_its_own_panel_is_up() {
    let mut fonts = Fonts::new();
    let mut stream = conversation(&mut fonts);
    let input = Input::default();

    // Nothing open and the pointer nowhere: no row is hovered.
    assert_eq!(stream.hovered(&input), None);

    // The grid, which the app raises and tells the panel about.
    click_hovering(&mut stream, 1, "stream/row/1/tool/react");
    stream.held = Some("root".to_string());
    assert_eq!(
        stream.hovered(&input),
        Some(1),
        "the toolbar went away when the grid appeared"
    );

    // Put away, and the row answers the pointer again -- which is nowhere.
    stream.held = None;
    assert_eq!(stream.hovered(&input), None);
}

/// The react button opens the whole grid, carrying the button it hangs off.
///
/// It used to open a row of seven faces with the grid behind *them* -- a press
/// to reach a shortlist and another to leave it. The reader's own most-used
/// are on the strip itself now, which answers that question better and beside
/// it, so the shortlist was a second answer to a question already answered.
#[test]
fn the_react_button_opens_the_grid_under_itself() {
    let mut fonts = Fonts::new();
    let mut stream = conversation(&mut fonts);
    let button = stream
        .boxes(panel(), Some(1))
        .into_iter()
        .find(|placed| placed.name == "stream/row/1/tool/react")
        .expect("no button to react from")
        .rect;
    assert_eq!(
        click_hovering(&mut stream, 1, "stream/row/1/tool/react"),
        Some(Chose::Pick {
            post_id: "root".to_string(),
            under: button,
        })
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

/// Two pictures are drawn beside each other, and each where its own block
/// says.
///
/// The layout packs them along a shelf now; the drawing used to place every
/// picture at the left edge of the column and would have stacked them on top
/// of one another -- both drawn in the same place, and a press landing on
/// whichever was tested first.
#[test]
fn two_pictures_are_drawn_side_by_side() {
    use matterless_paint::{Painter, Palette, Piece, Scene};

    let picture = |id: &str, width: i32, height: i32| matterless_render::FileRef {
        id: id.into(),
        name: format!("{id}.png"),
        extension: "png".into(),
        size: 4096,
        mime_type: "image/png".into(),
        width: 1280,
        height: 720,
        image: true,
        video: false,
        variant: matterless_render::ImageVariant::Thumb,
        mini_preview: None,
        box_width: width,
        box_height: height,
        archived: false,
    };

    let mut fonts = Fonts::new();
    let mut painter = Painter::new();
    let mut scene = Scene::default();
    let mut shown = post("p1", "");
    shown.files = vec![picture("f1", 120, 100), picture("f2", 90, 60)];

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

    let box_of = |wanted: &str| {
        scene
            .layers
            .iter()
            .flat_map(|layer| layer.pieces.iter())
            .find_map(|piece| match piece {
                Piece::Image {
                    x,
                    y,
                    width,
                    height,
                    key,
                    ..
                } if key == wanted => Some((*x, *y, *width, *height)),
                _ => None,
            })
            .unwrap_or_else(|| panic!("{wanted} is drawn"))
    };

    let (first_x, first_y, first_width, _) = box_of("thumb/f1");
    let (second_x, second_y, second_width, _) = box_of("thumb/f2");

    assert_eq!(first_y, second_y, "the same shelf");
    assert!(
        second_x >= first_x + first_width,
        "the second starts where the first ends: {second_x} against {first_x} + {first_width}"
    );
    assert_eq!(first_width, 120.0);
    assert_eq!(second_width, 90.0);
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

/// The unread mark brought into view, a little below the top edge so what it
/// follows is still visible: a divider hard against the top reads as the
/// beginning of the channel rather than as a line drawn through it.
#[test]
fn the_unread_mark_can_be_scrolled_to() {
    let mut fonts = Fonts::new();
    let mut stream = Stream::new("stream");
    let mut rows: Vec<Row> = (0..60)
        .map(|at| Row::Post {
            post: post(&format!("p{at}"), "something said"),
        })
        .collect();
    rows.insert(40, Row::UnreadDivider);
    stream.rows = rows;
    stream.lay_out(&mut fonts, panel().width);

    let within = panel();
    assert!(stream.to_row(crate::stream::DIVIDER, 72.0, within));

    // Where it landed: the mark's own top, 72 below the panel's.
    let mark = stream
        .laid
        .iter()
        .take(40)
        .map(|laid| laid.height)
        .sum::<f32>();
    assert!(
        (stream.scroll - (mark + stream.theme.pad_top - 72.0)).abs() < 0.5,
        "the mark sits 72 below the top: scroll {} against {}",
        stream.scroll,
        mark + stream.theme.pad_top - 72.0
    );
}

/// A channel with nothing unread has no mark in its plan, and says so rather
/// than scrolling somewhere arbitrary -- which is what lets the caller decide
/// to go to the end instead.
#[test]
fn a_channel_with_no_mark_says_so() {
    let mut fonts = Fonts::new();
    let mut stream = Stream::new("stream");
    stream.rows = (0..20)
        .map(|at| Row::Post {
            post: post(&format!("p{at}"), "something said"),
        })
        .collect();
    stream.lay_out(&mut fonts, panel().width);
    let before = stream.scroll;

    assert!(!stream.to_row(crate::stream::DIVIDER, 72.0, panel()));
    assert_eq!(stream.scroll, before, "and nothing moved");
}

/// A mark near the top of a short history cannot be pushed 72 pixels down --
/// there is nothing above it to scroll away -- so it clamps rather than
/// scrolling to a negative offset.
#[test]
fn a_mark_at_the_top_clamps_rather_than_overscrolling() {
    let mut fonts = Fonts::new();
    let mut stream = Stream::new("stream");
    let mut rows: Vec<Row> = (0..40)
        .map(|at| Row::Post {
            post: post(&format!("p{at}"), "something said"),
        })
        .collect();
    rows.insert(0, Row::UnreadDivider);
    stream.rows = rows;
    stream.lay_out(&mut fonts, panel().width);

    assert!(stream.to_row(crate::stream::DIVIDER, 72.0, panel()));
    assert_eq!(stream.scroll, 0.0);
}

/// Going back to a conversation does not measure it again.
///
/// The cache used to be rebuilt from whatever was laid out that time, so
/// leaving a channel threw away every row of it and coming back shaped all two
/// hundred from nothing -- a fifth of a second, for messages that had not
/// changed since they were measured a minute earlier. It is the same work,
/// against the same width, for the same rows.
#[test]
fn coming_back_to_a_channel_reuses_what_was_shaped() {
    let mut fonts = Fonts::new();
    let mut stream = Stream::new("stream");
    let first: Vec<Row> = (0..30)
        .map(|at| Row::Post {
            post: post(&format!("a{at}"), "something said in the first"),
        })
        .collect();
    let second: Vec<Row> = (0..30)
        .map(|at| Row::Post {
            post: post(&format!("b{at}"), "something said in the second"),
        })
        .collect();

    stream.rows = first.clone();
    stream.lay_out(&mut fonts, panel().width);
    assert_eq!(stream.reused(), 0, "nothing was known the first time");

    // Away to another conversation, which shapes its own rows...
    stream.rows = second;
    stream.lay_out(&mut fonts, panel().width);
    assert_eq!(stream.reused(), 0);

    // ...and back again, which should cost nothing at all.
    stream.rows = first;
    stream.lay_out(&mut fonts, panel().width);
    assert_eq!(
        stream.reused(),
        30,
        "every row was measured again after a single channel switch"
    );
}

/// What is kept is bounded, and what goes is what nobody has been back to.
#[test]
fn the_kept_rows_do_not_grow_without_end() {
    let mut fonts = Fonts::new();
    let mut stream = Stream::new("stream");
    // Driven past a small cap rather than the real one, which would mean
    // shaping twenty thousand rows to prove an arithmetic rule.
    stream.kept_cap = 250;
    for channel in 0..8 {
        stream.rows = (0..100)
            .map(|at| Row::Post {
                post: post(&format!("c{channel}-{at}"), "something said"),
            })
            .collect();
        stream.lay_out(&mut fonts, panel().width);
        assert!(
            stream.kept_rows() <= stream.kept_cap + 100,
            "{} rows kept after {channel} conversations",
            stream.kept_rows()
        );
    }
    // The one in front of the reader is whole, however much was dropped.
    stream.lay_out(&mut fonts, panel().width);
    assert_eq!(
        stream.reused(),
        100,
        "the conversation on screen was evicted from under it"
    );
}

/// A conversation left behind keeps its newest rows, not all of them.
///
/// Somebody who has scrolled a thousand messages back into a channel and gone
/// elsewhere should not hold the whole of it: what a reader returns to is the
/// end of a conversation, and reaching that far into the history twice is rare
/// enough to pay for again.
#[test]
fn a_channel_left_behind_keeps_only_its_newest_rows() {
    let mut fonts = Fonts::new();
    let mut stream = Stream::new("stream");
    // Room enough that the outer bound is not what binds here: the inner
    // rule is the one under test, and a cap below it would drop the whole
    // conversation before the trim could keep any of it.
    stream.kept_cap = 2_000;
    // One long scrollback, then away to something else.
    let deep: Vec<Row> = (0..900)
        .map(|at| Row::Post {
            post: post(&format!("deep{at:03}"), "something said long ago"),
        })
        .collect();
    stream.rows = deep.clone();
    stream.lay_out(&mut fonts, panel().width);
    stream.rows = vec![Row::Post {
        post: post("elsewhere", "a different channel"),
    }];
    stream.lay_out(&mut fonts, panel().width);

    // Back to the end of the long one, which is what a reader returns to.
    stream.rows = deep[400..].to_vec();
    stream.lay_out(&mut fonts, panel().width);
    assert_eq!(
        stream.reused(),
        500,
        "the newest rows of the channel were not kept"
    );

    // And the far end of it was let go, as it should have been.
    stream.rows = deep[..400].to_vec();
    stream.lay_out(&mut fonts, panel().width);
    assert_eq!(stream.reused(), 0, "the oldest rows were kept after all");
}

/// Opening the thread pane does not cost the conversation behind it.
///
/// The pane narrows this column and closing it widens it back, which is the
/// commonest thing anybody does in this window. A height is only true for the
/// width it was measured at, so the two sizes are held apart rather than one
/// replacing the other -- held one at a time, the pane cost a full re-shape of
/// the conversation each way, twice per look.
#[test]
fn the_thread_pane_does_not_throw_the_conversation_away() {
    let mut fonts = Fonts::new();
    let mut stream = Stream::new("stream");
    stream.rows = (0..40)
        .map(|at| Row::Post {
            post: post(&format!("p{at}"), "something said in a channel"),
        })
        .collect();
    let wide = panel().width;
    let narrow = wide - 320.0;

    stream.lay_out(&mut fonts, wide);
    assert_eq!(stream.reused(), 0, "nothing was known to begin with");
    // The pane opens: a width never seen, so everything is measured.
    stream.lay_out(&mut fonts, narrow);
    assert_eq!(stream.reused(), 0);
    // And closing it comes back to rows that were measured at this width.
    stream.lay_out(&mut fonts, wide);
    assert_eq!(
        stream.reused(),
        40,
        "closing the pane re-shaped the channel"
    );
    stream.lay_out(&mut fonts, narrow);
    assert_eq!(stream.reused(), 40, "and opening it again did too");
    assert_eq!(stream.kept_widths(), 2);

    // A drag through a size settles somewhere new, and the sizes the window
    // keeps returning to are the ones kept.
    for width in [wide - 40.0, wide - 80.0, wide - 120.0] {
        stream.lay_out(&mut fonts, width);
    }
    assert!(
        stream.kept_widths() <= 3,
        "{} widths held",
        stream.kept_widths()
    );
}

/// The bar stays away until the conversation's height is known.
///
/// A channel opens on its last dozen rows and the rest are measured behind the
/// window, so the reach the thumb is sized against grows for about a fifth of
/// a second: measured a frame apart on a switch, 1368 then 1388 then 1408. The
/// text does not move -- it is pinned to the end -- but the thumb starts sized
/// for what has been measured and shrinks as the truth arrives, which is what
/// a reader sees as the bar glitching on every channel they open.
#[test]
fn the_scrollbar_waits_for_the_height_to_be_known() {
    let mut fonts = Fonts::new();
    let mut stream = Stream::new("stream");
    let rows: Vec<Row> = (0..40)
        .map(|at| Row::Post {
            post: post(&format!("p{at}"), "something said"),
        })
        .collect();
    // As a channel is opened: the tail is shaped and the rest is waiting.
    stream.plan(rows, true);
    stream.lay_out(&mut fonts, panel().width);
    assert!(stream.waiting() > 0, "the rest is still to be measured");
    let named = |stream: &Stream| {
        stream
            .boxes(panel(), None)
            .into_iter()
            .any(|placed| placed.name.contains("scrollbar"))
    };
    assert!(
        !named(&stream),
        "a bar sized against a height nobody has yet"
    );

    // And once every row has been measured it is there, with something true
    // to say.
    while stream.waiting() > 0 {
        stream.fill(&mut fonts, panel().width, 40);
    }
    assert!(named(&stream), "the bar never came back");
}

/// The "New messages" line survives a pointer resting on the message under it.
///
/// It has no height of its own and its block is lifted half its depth into the
/// join, so a light laid down when the row below took its turn covered the rule
/// and the lower half of the words. That row is the first unread message, which
/// is the one a reader coming back is most likely to be pointing at.
#[test]
fn the_unread_line_is_not_covered_by_the_light_under_the_message() {
    use matterless_paint::{Painter, Palette, Piece, Scene};

    let mut fonts = Fonts::new();
    let within = panel();
    let mut stream = Stream::new("stream");
    stream.rows = vec![
        Row::Post {
            post: post("read", ""),
        },
        Row::UnreadDivider,
        Row::Post {
            post: post("fresh", ""),
        },
    ];
    stream.lay_out(&mut fonts, within.width);

    let placed = stream.boxes(within, None);
    let row = placed
        .iter()
        .find(|item| item.name == "stream/row/2")
        .expect("the first unread message is placed");
    let mut input = Input::default();
    input.apply(
        Event::PointerMoved {
            x: row.rect.x + 10.0,
            y: row.rect.y + row.rect.height / 2.0,
        },
        &placed,
    );
    assert_eq!(
        stream.hovered(&input),
        Some(2),
        "the fixture is not pointing at the row it means to"
    );

    let palette = Palette::default();
    let mut painter = Painter::new();
    let mut scene = Scene::default();
    stream.draw(
        &mut crate::sidebar::Canvas {
            scene: &mut scene,
            painter: &mut painter,
            fonts: &mut fonts,
            palette: &palette,
        },
        within,
        &input,
        &std::collections::HashMap::new(),
    );

    let mut light = None;
    let mut mark = None;
    for (at, piece) in scene
        .layers
        .iter()
        .flat_map(|layer| layer.pieces.iter())
        .enumerate()
    {
        if let Piece::Fill { colour, height, .. } = piece {
            if *colour == palette.hover && light.is_none() {
                light = Some(at);
            }
            // The rules either side of the words: one pixel deep, in the
            // colour the unread line alone is drawn in.
            if *height == 1.0 && colour[..3] == palette.flag[..3] && mark.is_none() {
                mark = Some(at);
            }
        }
    }
    let light = light.expect("the hovered row has a light under it");
    let mark = mark.expect("the unread line draws its rule");
    assert!(
        light < mark,
        "the light ({light}) is drawn over the line ({mark})"
    );
}
