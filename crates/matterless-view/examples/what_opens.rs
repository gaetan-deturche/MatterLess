//! What opening a channel costs, channel by channel.
//!
//! "Certain channels" hung for a moment when they were opened, and the window
//! is the wrong place to find out why: everything between the click and the
//! frame happens on the one thread, so all that can be seen from outside is
//! that it stopped.
//!
//! Three numbers, because the window now does the work in two parts. `plan` is
//! reading the rows out of the store. `open` is shaping enough of the newest
//! end to fill the panel, which is what the reader waits for. `whole` is the
//! rest of the conversation, which is shaped behind the window afterwards and
//! nobody waits for -- kept here because it is what the first two used to cost
//! together, and because it is the number that grows without anyone noticing.
//!
//! Counts and milliseconds. No message text.

use matterless_ui::Rect;
use matterless_view::stream::Stream;
use std::time::Instant;

/// A panel the size of the window's own.
const PANEL: Rect = Rect {
    x: 260.0,
    y: 44.0,
    width: 700.0,
    height: 640.0,
};

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

    println!(
        "{:>7} {:>7} {:>7} {:>6} {:>6}  channel",
        "plan", "open", "whole", "shown", "rows"
    );
    let mut worst: Vec<(u128, String)> = Vec::new();
    for (id, name) in channels {
        let began = Instant::now();
        let Ok(rows) =
            matterless_view::feed::rows_of(&store, &id, &me, &[], matterless_view::feed::PAGE, 0)
        else {
            continue;
        };
        let planned = began.elapsed();
        let count = rows.len();

        // What the reader waits for: the newest end, and enough of it.
        let mut stream = Stream::new("stream");
        stream.custom = matterless_view::feed::custom_emoji(&store, &rows);
        let began = Instant::now();
        stream.plan(rows, true);
        stream.lay_out(&mut fonts, PANEL.width);
        stream.cover(&mut fonts, PANEL.width, PANEL);
        let opened = began.elapsed();
        let shown = stream.rows.len();

        // And the rest, which happens behind the window.
        let began = Instant::now();
        while stream.waiting() > 0 {
            stream.fill(&mut fonts, PANEL.width, 24);
        }
        let whole = began.elapsed();

        println!(
            "{:>6}ms {:>6}ms {:>6}ms {:>6} {:>6}  {name}",
            planned.as_millis(),
            opened.as_millis(),
            whole.as_millis(),
            shown,
            count
        );
        worst.push((planned.as_millis() + opened.as_millis(), name));
    }
    worst.sort_by_key(|worst| std::cmp::Reverse(worst.0));
    println!("\nslowest to open:");
    for (cost, name) in worst.iter().take(5) {
        println!("{cost:>6}ms  {name}");
    }
}
