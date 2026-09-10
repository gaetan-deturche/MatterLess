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
    // Every timestamp in the store is UTC. Without this the day a message
    // belongs to is decided in UTC too, and a conversation that ran across
    // midnight local time is separated in the wrong place.
    options.utc_offset_minutes = crate::clock::utc_offset_minutes();
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
    // Every timestamp in the store is UTC. Without this the day a message
    // belongs to is decided in UTC too, and a conversation that ran across
    // midnight local time is separated in the wrong place.
    options.utc_offset_minutes = crate::clock::utc_offset_minutes();
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

/// Every emoji name a page of rows mentions that has no character of its own.
fn named_by(rows: &[Row]) -> Vec<String> {
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
    wanted
}

/// The custom emoji a page of rows uses, by name to the id behind their image.
///
/// A standard emoji is a character the parser already resolved. A custom one is
/// an image behind the session token and has no character at all, so without
/// this a reaction reads ":bongo:" and a message body says the name out loud.
pub fn custom_emoji(store: &Store, rows: &[Row]) -> HashMap<String, String> {
    store
        .known_emoji(&named_by(rows))
        .unwrap_or_default()
        .into_iter()
        // An empty id means the store looked and it is a standard one, which is
        // a real answer and not a miss.
        .filter(|(_, id)| !id.is_empty())
        .collect()
}

/// The names on this page nobody has asked the server about yet.
///
/// Absent from the table entirely, which is not the same as present with an
/// empty id: that is the remembered answer "the server does not know this as
/// custom", and asking again would be asking a question already answered.
pub fn unknown_emoji(store: &Store, rows: &[Row]) -> Vec<String> {
    let wanted = named_by(rows);
    let known = store.known_emoji(&wanted).unwrap_or_default();
    wanted
        .into_iter()
        .filter(|name| !known.contains_key(name))
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

#[cfg(test)]
mod tests {
    use super::*;
    use matterless_render::{PostRow, ReactionSummary};
    use std::sync::Arc;

    fn reacted_with(names: &[&str]) -> Vec<Row> {
        vec![Row::Post {
            post: PostRow {
                post_id: "p1".into(),
                root_id: String::new(),
                author_id: "u1".into(),
                author_name: "ada".into(),
                create_at: 1,
                update_at: 1,
                edited: false,
                nodes: Arc::new(Vec::new()),
                reactions: names
                    .iter()
                    .map(|name| ReactionSummary {
                        emoji: (*name).to_string(),
                        count: 1,
                        mine: false,
                        // No character, so it is either custom or a name the
                        // table has never heard of -- the two this asks about.
                        unicode: None,
                        names: Vec::new(),
                    })
                    .collect(),
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
            },
        }]
    }

    /// A name nobody has asked about is asked about; one already answered is
    /// not. Without the second half every frame would ask the server the same
    /// question again, and the answer to "is `53` an emoji" does not change.
    #[test]
    fn only_names_nobody_has_asked_about_are_asked_about() {
        let store = Store::open_in_memory().expect("a store");
        store.remember_emoji("bongo", "abc123").expect("custom");
        store.remember_emoji("53", "").expect("not custom");
        let rows = reacted_with(&["bongo", "53", "shipit"]);

        assert_eq!(unknown_emoji(&store, &rows), vec!["shipit".to_string()]);
        // And the one with an id behind it is the one that can be drawn.
        let known = custom_emoji(&store, &rows);
        assert_eq!(known.get("bongo").map(String::as_str), Some("abc123"));
        assert!(!known.contains_key("53"));
    }
}
