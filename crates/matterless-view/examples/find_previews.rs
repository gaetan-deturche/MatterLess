//! Which conversations here actually carry a link or permalink card.
//!
//! A preview is the server's doing: it fetches a link's metadata and attaches
//! an embed, or it does not. So an empty-looking channel proves nothing about
//! the card drawing, and this says where to look instead -- the window takes a
//! channel id as its second argument.
//!
//! Channel names and preview kinds, never message text.

fn main() {
    let Some(store) = matterless_view::feed::default_store()
        .and_then(|path| matterless_store::Store::open(&path).ok())
    else {
        eprintln!("no store");
        return;
    };
    // Every channel the store knows, by asking for a match everything shares.
    let channels = store.channels_matching("", 500).unwrap_or_default();
    println!("scanning {} conversations", channels.len());

    let mut found = 0usize;
    for channel in channels {
        let posts = store
            .channel_page(&channel.id, None, 200)
            .unwrap_or_default();
        let kinds: Vec<&str> = posts
            .iter()
            .flat_map(|post| post.metadata.embeds.iter())
            .map(|embed| embed.embed_type.as_str())
            .filter(|kind| *kind == "opengraph" || *kind == "permalink")
            .collect();
        if kinds.is_empty() {
            continue;
        }
        found += 1;
        let pages = kinds.iter().filter(|kind| **kind == "opengraph").count();
        println!(
            "{} -- {} ({pages} pages, {} permalinks)",
            channel.id,
            if channel.display_name.is_empty() {
                channel.name.clone()
            } else {
                channel.display_name.clone()
            },
            kinds.len() - pages
        );
    }
    if found == 0 {
        println!("nothing in this store carries a card");
    }
}
