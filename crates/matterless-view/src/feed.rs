//! Real messages, read from the app's own database.
//!
//! A hardcoded conversation proves the renderer draws. It does not prove the
//! port: the heights that went wrong in the DOM list went wrong on real
//! messages -- long wraps, mentions, code, one-word replies, five hundred rows
//! of them -- and those are the only ones worth measuring against.
//!
//! Read-only, and through the same `plan_channel` the app uses, so the rows here
//! are the rows the app would draw rather than an approximation of them.

use matterless_core::model::ThreadMode;
use matterless_render::pending::PendingPost;
use matterless_render::{PlanOptions, Row, plan_channel};
use matterless_store::Store;
use std::collections::HashMap;
use std::path::Path;

/// How many messages a channel opens with. Well past a screenful, so scrolling
/// has somewhere to go, and far short of the whole history, which is unbounded.
pub const PAGE: u32 = 400;

/// Where the dev build keeps its database.
pub fn default_store() -> Option<std::path::PathBuf> {
    let base = std::env::var_os("APPDATA")?;
    Some(
        Path::new(&base)
            .join("com.gaetandeturche.matterless.dev")
            .join("matterless.db"),
    )
}

/// Opens the store, read-only.
pub fn open(path: &Path) -> Result<Store, String> {
    if !path.is_file() {
        return Err(format!("{} is not a database", path.display()));
    }
    Store::open(path).map_err(|error| format!("open {}: {error}", path.display()))
}

/// The channel with the most recent message, which is the one worth opening.
fn busiest(store: &Store) -> Option<String> {
    // Every channel the reader is in, newest first. The user id is only used to
    // join the membership, and an unknown one simply finds nothing -- which is
    // why the caller is told to pass the real one.
    store
        .channels_with_unread("")
        .ok()
        .and_then(|channels| channels.first().map(|(channel, _)| channel.id.clone()))
}

/// Reads a page of a channel and plans it into rows.
///
/// `me_id` only decides which messages count as the reader's own and which
/// mentions are theirs; an empty one draws every message as somebody else's,
/// which is wrong in styling but not in height.
/// Reads a channel from an already-open store and plans it.
pub fn rows_of(
    store: &Store,
    channel_id: &str,
    me_id: &str,
    outstanding: &[PendingPost],
    depth: u32,
) -> Result<Vec<Row>, String> {
    let mut posts = store
        .channel_page(channel_id, None, depth)
        .map_err(|error| format!("read {channel_id}: {error}"))?;
    let mut authors: Vec<String> = posts.iter().map(|post| post.user_id.clone()).collect();
    authors.sort();
    authors.dedup();
    let people = store.users_by_ids(&authors).unwrap_or_default();
    let mut options = PlanOptions::new(ThreadMode::Collapsed, me_id);
    options.author_names = people
        .iter()
        .map(|(id, user)| (id.clone(), user.username.clone()))
        .collect();
    options.author_avatars = people
        .iter()
        .map(|(id, user)| (id.clone(), user.last_picture_update))
        .collect();

    // Optimistic sends are merged here so they reach the screen through the
    // same plan as everything else. A guess is never written to SQLite: if the
    // send fails, or the window dies mid-flight, the database looks exactly as
    // it did before.
    for held in outstanding {
        options.pending.insert(held.pending_post_id.clone());
        if held.failed {
            options.failed.insert(held.pending_post_id.clone());
        }
        posts.push(held.as_post(me_id));
    }
    Ok(plan_channel(&posts, &HashMap::new(), &options))
}

pub fn rows_from(
    path: &Path,
    channel_id: Option<String>,
    me_id: &str,
) -> Result<(String, Vec<Row>), String> {
    // Checked rather than left to `Store::open`, which would create an empty
    // database at whatever path it was handed and then report no messages in it.
    if !path.is_file() {
        return Err(format!("{} is not a database", path.display()));
    }
    let store = Store::open(path).map_err(|error| format!("open {}: {error}", path.display()))?;
    let channel_id = channel_id
        .or_else(|| busiest(&store))
        .ok_or_else(|| "no channel in the store; pass one as an argument".to_string())?;

    let posts = store
        .channel_page(&channel_id, None, PAGE)
        .map_err(|error| format!("read {channel_id}: {error}"))?;
    if posts.is_empty() {
        return Err(format!("{channel_id} has no messages in the local store"));
    }

    // The names come from the store rather than the post: which name to show is
    // a server-side preference, and the plan wants it already resolved.
    let authors: Vec<String> = {
        let mut ids: Vec<String> = posts.iter().map(|post| post.user_id.clone()).collect();
        ids.sort();
        ids.dedup();
        ids
    };
    let people = store.users_by_ids(&authors).unwrap_or_default();

    let mut options = PlanOptions::new(ThreadMode::Collapsed, me_id);
    options.author_names = people
        .iter()
        .map(|(id, user)| (id.clone(), user.username.clone()))
        .collect();
    options.author_avatars = people
        .iter()
        .map(|(id, user)| (id.clone(), user.last_picture_update))
        .collect();

    // No thread summaries: a footer is a fixed-height row, and its absence
    // changes what is drawn but not whether the heights are right.
    let rows = plan_channel(&posts, &HashMap::new(), &options);
    Ok((channel_id, rows))
}

/// The custom emoji a page of rows uses, by name to the id behind their image.
///
/// A standard emoji is a character the parser already resolved. A custom one is
/// an image behind the session token and has no character at all, so without
/// this a reaction reads ":bongo:" and a message body says the name out loud.
pub fn custom_emoji(store: &Store, rows: &[Row]) -> HashMap<String, String> {
    let mut wanted: Vec<String> = Vec::new();
    for row in rows {
        let (Row::Post { post } | Row::Continuation { post }) = row else {
            continue;
        };
        for reaction in &post.reactions {
            if reaction.unicode.is_none() {
                wanted.push(reaction.emoji.clone());
            }
        }
        wanted.extend(named_in(&post.nodes));
    }
    wanted.sort();
    wanted.dedup();
    store
        .known_emoji(&wanted)
        .unwrap_or_default()
        .into_iter()
        // An empty id means the store looked and it is a standard one, which is
        // a real answer and not a miss.
        .filter(|(_, id)| !id.is_empty())
        .collect()
}

/// Every `:name:` in a parsed body that had no character of its own.
fn named_in(nodes: &[matterless_render::markdown::Node]) -> Vec<String> {
    use matterless_render::markdown::Node;
    let mut found = Vec::new();
    for node in nodes {
        match node {
            Node::Emoji { name, unicode } if unicode.is_none() => found.push(name.clone()),
            Node::Paragraph { children }
            | Node::Heading { children, .. }
            | Node::Blockquote { children }
            | Node::Strong { children }
            | Node::Emphasis { children }
            | Node::Strike { children } => found.extend(named_in(children)),
            Node::List { items, .. } => {
                for item in items {
                    found.extend(named_in(item));
                }
            }
            _ => {}
        }
    }
    found
}
