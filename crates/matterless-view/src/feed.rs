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
use matterless_render::{PlanOptions, Row, plan_channel};
use matterless_store::Store;
use std::collections::HashMap;
use std::path::Path;

/// How many messages to read. Well past a screenful, so scrolling has somewhere
/// to go, and far short of the whole history, which is unbounded.
const PAGE: u32 = 400;

/// Where the dev build keeps its database.
pub fn default_store() -> Option<std::path::PathBuf> {
    let base = std::env::var_os("APPDATA")?;
    Some(
        Path::new(&base)
            .join("com.gaetandeturche.matterless.dev")
            .join("matterless.db"),
    )
}

/// One channel, as the sidebar needs it.
pub struct Listed {
    pub id: String,
    pub label: String,
    pub unread: i64,
    pub mentions: i64,
    pub muted: bool,
}

/// Every channel in the store, most recently active first.
///
/// The same order the sidebar's "recent" sorting uses, and the only order that
/// makes sense without the server's category arrangement -- which this reads
/// none of, deliberately: it is a feed for the port, not the sidebar the app
/// will eventually have.
pub fn channels(store: &Store, me_id: &str) -> Vec<Listed> {
    store
        .channels_with_unread(me_id)
        .unwrap_or_default()
        .into_iter()
        .map(|(channel, unread)| Listed {
            label: if channel.display_name.is_empty() {
                channel.name.clone()
            } else {
                channel.display_name.clone()
            },
            id: channel.id,
            unread: unread.messages,
            mentions: unread.mentions,
            muted: unread.muted,
        })
        .collect()
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
pub fn rows_of(store: &Store, channel_id: &str, me_id: &str) -> Result<Vec<Row>, String> {
    let posts = store
        .channel_page(channel_id, None, PAGE)
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
