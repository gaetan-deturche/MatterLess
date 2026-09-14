//! What a page of real messages asks for in custom emoji, and what it gets.
//!
//! Read-only, against the dev build's own database, through the same
//! `custom_emoji` the window calls -- so a name missing here is a name missing
//! on screen.

use matterless_paint::{Painter, Palette, Piece};
use matterless_render::Row;

fn main() {
    let wanted = std::env::args().nth(1);
    let path = matterless_view::feed::default_store().expect("APPDATA");
    let store = matterless_view::feed::open(&path).expect("a store");

    let (channel_id, rows) =
        matterless_view::feed::rows_from(&path, wanted.clone(), "").expect("rows");
    println!("channel {channel_id}: {} rows", rows.len());

    let custom = matterless_view::feed::custom_emoji(&store, &rows);
    let unknown = matterless_view::feed::unknown_emoji(&store, &rows);
    println!("custom resolved: {custom:?}");
    println!("still unknown: {unknown:?}");

    // And what the painter would actually emit for each row that names one.
    let mut fonts = matterless_layout::Fonts::new();
    let mut painter = Painter::new();
    let theme = matterless_layout::row::Theme::default();
    for row in &rows {
        let laid = matterless_layout::row::lay_out(&mut fonts, row, &theme);
        let names: Vec<String> = laid
            .blocks
            .iter()
            .flat_map(|block| &block.spans)
            .filter_map(|span| span.emoji.clone())
            .collect();
        if names.is_empty() {
            continue;
        }
        let pieces = painter.pieces_of(
            &mut fonts,
            &laid,
            0.0,
            &theme,
            &Palette::default(),
            &custom,
        );
        let images: Vec<&String> = pieces
            .iter()
            .filter_map(|piece| match piece {
                Piece::Image { key, .. } => Some(key),
                _ => None,
            })
            .collect();
        let who = match row {
            Row::Post { post } | Row::Continuation { post } => post.post_id.clone(),
            _ => String::new(),
        };
        println!("{who}: names {names:?} -> images {images:?}");
    }
}
