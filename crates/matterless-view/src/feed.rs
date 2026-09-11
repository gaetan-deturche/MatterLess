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
    Ok(plan_channel(
        &posts,
        &summaries(store, channel_id, &posts),
        &options,
    ))
}

/// What each root on this page has hanging off it.
///
/// Stubbed to an empty map for a long time, which is why the thread footer --
/// built by the planner, measured by the layout -- had never once appeared:
/// with no summary a root grows no footer, and the only way left to open a
/// thread was to press the whole message.
fn summaries(
    store: &Store,
    channel_id: &str,
    posts: &[matterless_core::Post],
) -> HashMap<String, matterless_render::ThreadSummary> {
    let roots: Vec<String> = posts
        .iter()
        .filter(|post| post.root_id.is_empty())
        .map(|post| post.id.clone())
        .collect();
    let mut summaries: HashMap<String, matterless_render::ThreadSummary> = store
        .thread_summaries_for(channel_id, &roots)
        .unwrap_or_default()
        .into_iter()
        .map(|(root_id, (reply_count, last_reply_at, participants))| {
            (
                root_id,
                matterless_render::ThreadSummary {
                    reply_count,
                    last_reply_at,
                    participants,
                    ..Default::default()
                },
            )
        })
        .collect();

    // Read state is the server's, and it can describe a thread whose replies
    // are not held locally at all -- so a state without a local summary still
    // earns an entry, or the footer that explains an unread badge would never
    // be built.
    for (root_id, state) in store.thread_states_for(&roots).unwrap_or_default() {
        let summary = summaries.entry(root_id).or_default();
        summary.reply_count = summary.reply_count.max(state.reply_count);
        summary.last_reply_at = summary.last_reply_at.max(state.last_reply_at);
        summary.unread_replies = state.unread_replies;
        summary.unread_mentions = state.unread_mentions;
        summary.following = state.following;
    }
    summaries
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

/// A link to one message that will open anywhere.
///
/// The team is part of the path even though a message belongs to a channel,
/// which is how Mattermost's own links are shaped. A direct message has no
/// team of its own, and any team the reader is on resolves the link -- which
/// is what the official client does too.
///
/// `None` when the store cannot name a team, rather than a link with a hole in
/// it: a URL that 404s is worse than an action that says it cannot.
/// A link to a conversation itself, for the clipboard.
///
/// By name rather than by id, the way the app builds it: a channel link the
/// reader can read tells them where it goes before they follow it.
pub fn channel_link(store: &Store, server: &str, channel_id: &str) -> Option<String> {
    let channel = store.channel(channel_id).ok().flatten()?;
    // A direct message belongs to no team; any team the reader is on resolves
    // the link, as it does for a post permalink.
    let team = if channel.team_id.is_empty() {
        store.any_team_name().ok().flatten()?
    } else {
        store.team_name(&channel.team_id).ok().flatten()?
    };
    Some(format!(
        "{}/{team}/channels/{}",
        server.trim_end_matches('/'),
        channel.name
    ))
}

pub fn permalink(store: &Store, server: &str, post_id: &str) -> Option<String> {
    let post = store.post(post_id).ok().flatten()?;
    let channel = store.channel(&post.channel_id).ok().flatten()?;
    let team = if channel.team_id.is_empty() {
        store.any_team_name().ok().flatten()?
    } else {
        store.team_name(&channel.team_id).ok().flatten()?
    };
    Some(format!(
        "{}/{team}/pl/{post_id}",
        server.trim_end_matches('/')
    ))
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

    fn stored(store: &Store, channel_id: &str, team_id: &str) {
        let channel: matterless_core::model::Channel = serde_json::from_value(serde_json::json!({
            "id": channel_id, "team_id": team_id, "type": "O", "name": "dev",
        }))
        .expect("a channel");
        store.upsert_channels(&[channel]).expect("stored");
        store
            .upsert_posts(&[said("p1", channel_id)])
            .expect("stored");
    }

    fn said(id: &str, channel_id: &str) -> matterless_core::Post {
        serde_json::from_value(serde_json::json!({
            "id": id, "channel_id": channel_id, "user_id": "u1",
            "create_at": 1, "update_at": 1, "message": "hello",
        }))
        .expect("a post")
    }

    fn team(id: &str, name: &str) -> matterless_core::model::Team {
        serde_json::from_value(serde_json::json!({
            "id": id, "name": name, "display_name": name,
        }))
        .expect("a team")
    }

    /// The shape Mattermost's own links have, and the trailing slash a server
    /// address may or may not carry.
    #[test]
    fn a_permalink_names_the_team_the_channel_is_in() {
        let store = Store::open_in_memory().expect("a store");
        store
            .upsert_teams(&[team("t1", "voyager"), team("t2", "northwind")])
            .expect("teams");
        stored(&store, "c1", "t1");

        assert_eq!(
            permalink(&store, "https://chat.invalid", "p1").as_deref(),
            Some("https://chat.invalid/voyager/pl/p1")
        );
        assert_eq!(
            permalink(&store, "https://chat.invalid/", "p1").as_deref(),
            Some("https://chat.invalid/voyager/pl/p1")
        );
    }

    /// A direct message has no team of its own, so any team the reader is on
    /// resolves the link rather than the link being unbuildable.
    #[test]
    fn a_direct_message_borrows_a_team() {
        let store = Store::open_in_memory().expect("a store");
        store.upsert_teams(&[team("t2", "northwind")]).expect("teams");
        stored(&store, "c1", "");
        assert_eq!(
            permalink(&store, "https://chat.invalid", "p1").as_deref(),
            Some("https://chat.invalid/northwind/pl/p1")
        );
    }

    /// No team at all means no link, rather than one with a hole where the
    /// team should be: a URL that 404s is worse than an action that says it
    /// cannot.
    #[test]
    fn no_team_means_no_link() {
        let store = Store::open_in_memory().expect("a store");
        stored(&store, "c1", "");
        assert_eq!(permalink(&store, "https://chat.invalid", "p1"), None);
        // And a message this store has never met is not a link either.
        assert_eq!(permalink(&store, "https://chat.invalid", "nope"), None);
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
