//! What each widget puts in the scene, as a digest.
//!
//! For changing drawing code without changing what is drawn: run it, make the
//! change, run it again, and the digests either match or they do not.
//!
//! A screenshot cannot answer this. Two launches of a live client scroll
//! differently and the conversation moves underneath -- a comparison across
//! the panel port showed a 247-level difference that turned out to be seventy
//! pixels of scroll. And half of what is here never reaches a screenshot at
//! all: a menu wants a right-click, a tooltip wants a wait, the message
//! toolbar wants a hovered row.
//!
//! Boxes only. Glyph positions carry nothing about a panel and would drown it.

use matterless_paint::{Painter, Palette, Piece, Scene};
use matterless_ui::Rect;
use matterless_ui::input::Input;
use matterless_view::{composer, menu, rail, tooltip, whats_new};
use matterless_widgets::Canvas;

fn window() -> Rect {
    Rect::new(0.0, 0.0, 1000.0, 700.0)
}

fn main() {
    let mut fonts = matterless_layout::Fonts::new();
    let mut painter = Painter::new();
    let palette = Palette::default();
    let input = Input::default();

    for (what, pieces) in [
        ("menu", {
            let mut scene = Scene::default();
            let mut shown = menu::Menu::default();
            shown.show(
                "a-channel",
                menu::Anchor::At(200.0, 200.0),
                menu::Style::channel(),
                vec![
                    menu::Item::new("one", "The first thing"),
                    menu::Item::rule(),
                    menu::Item::new("two", "The second thing"),
                ],
            );
            let mut canvas = Canvas {
                scene: &mut scene,
                painter: &mut painter,
                fonts: &mut fonts,
                palette: &palette,
            };
            shown.draw(&mut canvas, window(), &input);
            scene
        }),
        ("tooltip", {
            let mut scene = Scene::default();
            let mut tip = tooltip::Tooltip::default();
            tip.follows(Some("a-button"), Some((300.0, 300.0)), |_| {
                Some("What this button is for".to_string())
            });
            // The wait is over: the tooltip only draws once it has ripened.
            std::thread::sleep(tooltip::DWELL + std::time::Duration::from_millis(20));
            let _ = tip.shown();
            let mut canvas = Canvas {
                scene: &mut scene,
                painter: &mut painter,
                fonts: &mut fonts,
                palette: &palette,
            };
            tip.draw(&mut canvas, window());
            scene
        }),
        ("composer", {
            let mut scene = Scene::default();
            let mut box_of = composer::Composer::new(composer::NAME);
            box_of.lay_out(&mut fonts, 600.0);
            let mut canvas = Canvas {
                scene: &mut scene,
                painter: &mut painter,
                fonts: &mut fonts,
                palette: &palette,
            };
            box_of.draw(&mut canvas, Rect::new(20.0, 600.0, 600.0, 80.0), true);
            scene
        }),
        ("whats_new", {
            let mut scene = Scene::default();
            let mut panel = whats_new::WhatsNew::default();
            panel.show("0.1.6", "- one thing\n- another thing");
            panel.measure(&mut fonts, window());
            let mut canvas = Canvas {
                scene: &mut scene,
                painter: &mut painter,
                fonts: &mut fonts,
                palette: &palette,
            };
            panel.draw(&mut canvas, &input, window());
            scene
        }),
    ] {
        println!("{what:10} {}", digest(&pieces));
    }

    // The two in the stream only draw while a row is hovered -- the toolbar
    // over a message, and the faces behind its react button. Neither reaches a
    // screenshot, and that file is the one with the paint-order history.
    let mut stream = matterless_view::stream::Stream::new("stream");
    stream.rows = vec![matterless_render::Row::Post {
        post: post("only-one"),
    }];
    let within = Rect::new(260.0, 44.0, 600.0, 400.0);
    stream.lay_out(&mut fonts, within.width);
    let boxes = stream.boxes(within, Some(0));
    let mut input = Input::default();
    let over = boxes
        .iter()
        .find(|placed| placed.name.contains("/tool/react"))
        .expect("a toolbar button is placed");
    input.apply(
        matterless_ui::input::Event::PointerMoved {
            x: over.rect.x + over.rect.width / 2.0,
            y: over.rect.y + over.rect.height / 2.0,
        },
        &boxes,
    );
    let mut scene = Scene::default();
    {
        let mut canvas = Canvas {
            scene: &mut scene,
            painter: &mut painter,
            fonts: &mut fonts,
            palette: &palette,
        };
        stream.draw(&mut canvas, within, &input);
    }
    println!("{:10} {}", "stream", digest(&scene));

    let _ = rail::Rail::default();
}

/// One message, enough to hang a toolbar off.
fn post(id: &str) -> matterless_render::PostRow {
    matterless_render::PostRow {
        post_id: id.into(),
        root_id: String::new(),
        author_id: "u1".into(),
        author_name: "someone".into(),
        create_at: 0,
        update_at: 0,
        edited: false,
        nodes: std::sync::Arc::new(vec![matterless_render::markdown::Node::Paragraph {
            children: vec![matterless_render::markdown::Node::Text {
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

/// Every box the scene holds, in order, to three decimals. Text is left out:
/// glyph positions carry no information about a panel and would drown it.
fn digest(scene: &Scene) -> String {
    let mut said = Vec::new();
    for layer in &scene.layers {
        for piece in &layer.pieces {
            if let Piece::Fill {
                x,
                y,
                width,
                height,
                colour,
                radius,
                softness,
            } = piece
            {
                said.push(format!(
                    "{x:.3},{y:.3},{width:.3},{height:.3},{colour:?},{radius:.3},{softness:.3}"
                ));
            }
        }
    }
    format!("{} boxes  {:016x}", said.len(), hashed(&said.join("|")))
}

/// Any stable hash will do; this one is here so the digest fits on a line.
fn hashed(text: &str) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in text.as_bytes() {
        hash ^= *byte as u64;
        hash = hash.wrapping_mul(0x100_0000_01b3);
    }
    hash
}
