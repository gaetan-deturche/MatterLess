//! Draws a conversation and writes it out, so the result can be looked at.
//!
//! Assertions catch a height that is wrong by a line. They do not catch text
//! drawn at the wrong baseline, a gutter applied twice, or a wrap that happens
//! somewhere other than where it was measured -- all of which are obvious in a
//! picture and invisible in a number. The image is written to `target/` so it
//! is never mistaken for something the app ships.

use matterless_layout::Fonts;
use matterless_layout::row::{Theme, lay_out};
use matterless_paint::{Canvas, Painter, Palette};
use matterless_render::markdown::Node;
use matterless_render::{PostRow, Row};
use std::sync::Arc;

fn post(author: &str, nodes: Vec<Node>) -> PostRow {
    PostRow {
        post_id: format!("p-{author}"),
        root_id: String::new(),
        author_id: format!("u-{author}"),
        author_name: author.into(),
        create_at: 0,
        update_at: 0,
        edited: false,
        nodes: Arc::new(nodes),
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

fn para(children: Vec<Node>) -> Node {
    Node::Paragraph { children }
}

fn text(value: &str) -> Node {
    Node::Text {
        value: value.into(),
    }
}

/// The conversation drawn by the snapshot: plain text, a wrap, mixed weight,
/// a mention, a list and a code block.
///
/// Invented, and deliberately so: this was a real exchange between real
/// colleagues, pasted in as convenient test data and then compiled into a
/// public repository. Sample data is published data.
fn conversation() -> Vec<Row> {
    vec![
        Row::DateSeparator { epoch_day: 20_340 },
        Row::Post {
            post: post("ada", vec![para(vec![text("Morning!")])]),
        },
        Row::Continuation {
            post: post(
                "ada",
                vec![para(vec![text(
                    "Could we raise the cache size the test runner is allowed, to a couple of \
                     gigabytes or so? It looks as though it is set in the file beside the \
                     runner rather than anywhere obvious.",
                )])],
            ),
        },
        Row::Post {
            post: post(
                "ben",
                vec![
                    para(vec![
                        text("that is "),
                        Node::Strong {
                            children: vec![text("a fair question")],
                        },
                        text(" -- ask "),
                        Node::UserMention {
                            username: "cara".into(),
                            everyone: false,
                        },
                        text(", she set it up"),
                    ]),
                    Node::List {
                        ordered: false,
                        items: vec![
                            vec![para(vec![text("we can reach the runner itself")])],
                            vec![para(vec![text("but not the box its cache lives on")])],
                        ],
                    },
                    Node::CodeBlock {
                        language: Some("toml".into()),
                        value: "cache_size = \"2GiB\"\nkeep_days = 14".into(),
                    },
                ],
            ),
        },
    ]
}

#[test]
fn a_conversation_is_drawn_where_it_was_measured() {
    let mut fonts = Fonts::new();
    let mut painter = Painter::new();
    let theme = Theme {
        width: 820.0,
        ..Theme::default()
    };
    let palette = Palette::default();

    let rows = conversation();
    let laid: Vec<_> = rows
        .iter()
        .map(|row| lay_out(&mut fonts, row, &theme))
        .collect();
    let total: f32 = laid.iter().map(|row| row.height).sum();

    let mut canvas = Canvas::new(theme.width as u32, total.ceil() as u32, palette.ground);
    let mut top = 0.0_f32;
    for row in &laid {
        painter.paint_row(&mut canvas, &mut fonts, row, top, &theme, &palette);
        top += row.height;
    }

    // Something was actually drawn: a canvas of pure background means the
    // glyphs went somewhere else, which no height assertion would notice.
    let (pixels, _) = canvas.pixels.as_chunks::<4>();
    let drawn = pixels
        .iter()
        .filter(|pixel| pixel[0..3] != palette.ground[0..3])
        .count();
    assert!(
        drawn > 400,
        "too few pixels differ from the background: {drawn}"
    );

    // And it stayed inside the canvas the layout asked for.
    assert_eq!(
        canvas.pixels.len(),
        (canvas.width * canvas.height * 4) as usize
    );

    let path = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join("conversation.png");
    let file = std::fs::File::create(&path).expect("create the snapshot");
    let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), canvas.width, canvas.height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder
        .write_header()
        .expect("png header")
        .write_image_data(&canvas.pixels)
        .expect("png body");
    println!("snapshot: {}", path.display());
}
