//! What opening a channel costs, channel by channel.
//!
//! "Certain channels" hang for a moment when they are opened, and the window
//! is the wrong place to find out why: everything between the click and the
//! frame happens on the one thread, so all that can be seen from outside is
//! that it stopped. This does the same two pieces of work -- reading the plan
//! out of the store, and shaping every row of it -- and times them apart.
//!
//! Counts and milliseconds. No message text.

use matterless_layout::row::Theme;
use std::time::Instant;

fn main() {
    let Some(store) = matterless_view::feed::default_store()
        .and_then(|path| matterless_store::Store::open(&path).ok())
    else {
        eprintln!("no store");
        return;
    };
    // Only decides which messages count as the reader's own, which changes
    // nothing about what a row costs to shape.
    let me = std::env::var("MATTERLESS_ME").unwrap_or_default();
    let mut fonts = matterless_layout::Fonts::new();
    let theme = Theme {
        width: 660.0,
        ..Theme::default()
    };

    let wanted: Vec<String> = std::env::args().skip(1).collect();
    let channels: Vec<(String, String)> = if wanted.is_empty() {
        store
            .channels_with_unread(&me)
            .unwrap_or_default()
            .into_iter()
            .take(30)
            .map(|(channel, _)| (channel.id, channel.display_name))
            .collect()
    } else {
        wanted.into_iter().map(|id| (id.clone(), id)).collect()
    };

    println!("{:>7} {:>7} {:>6} {:>6}  channel", "plan", "shape", "rows", "chars");
    let mut worst: Vec<(u128, String)> = Vec::new();
    for (id, name) in channels {
        let began = Instant::now();
        let Ok(rows) = matterless_view::feed::rows_of(
            &store,
            &id,
            &me,
            &[],
            matterless_view::feed::PAGE,
            0,
        ) else {
            continue;
        };
        let planned = began.elapsed();

        let began = Instant::now();
        let laid: Vec<_> = rows
            .iter()
            .map(|row| matterless_layout::row::lay_out(&mut fonts, row, &theme))
            .collect();
        let shaped = began.elapsed();

        // How much text there was to shape, which is what a row costs.
        let chars: usize = laid
            .iter()
            .flat_map(|row| row.blocks.iter())
            .flat_map(|block| block.spans.iter())
            .map(|span| span.text.chars().count())
            .sum();
        println!(
            "{:>6}ms {:>6}ms {:>6} {:>6}  {name}",
            planned.as_millis(),
            shaped.as_millis(),
            rows.len(),
            chars
        );
        worst.push((planned.as_millis() + shaped.as_millis(), name));
    }
    worst.sort_by_key(|worst| std::cmp::Reverse(worst.0));
    println!("\nslowest:");
    for (cost, name) in worst.iter().take(5) {
        println!("{cost:>6}ms  {name}");
    }
}
