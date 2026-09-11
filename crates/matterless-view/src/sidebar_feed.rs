//! The sidebar as the reader arranged it, read from the local store.
//!
//! The same `matterless_sidebar::arrange` the Tauri command calls, so the two
//! shells cannot drift into different orders or different names for the same
//! conversation. Only the reads differ, because this one has no network.

use matterless_sidebar::{ChannelSummary, Group};
use matterless_store::Store;
use std::collections::HashMap;

/// Every channel, grouped and named the way the reader arranged it.
///
/// What this cannot do is ask the server about somebody it has never met: an
/// unresolved direct-message counterpart falls back to their id rather than the
/// `<id>__<id>` slug, and the next sync fills the name in.
pub fn groups(
    store: &Store,
    me_id: &str,
    threads: matterless_core::model::ThreadMode,
) -> Vec<Group> {
    // Under collapsed threads a reply is not a message in the channel at all:
    // it belongs to its thread, and the server keeps a second pair of counts
    // that say so. Counting every message instead put a badge on six channels
    // the official client showed as read -- five unread in Cutscenes, none of
    // them a root.
    let roots_only = threads == matterless_core::model::ThreadMode::Collapsed;
    let channels = store.channels_with_unread(me_id).unwrap_or_default();

    // A direct message is labelled by the other person, so those users have to
    // be resolved before anything can be named.
    let counterparts: Vec<String> = channels
        .iter()
        .filter(|(channel, _)| channel.channel_type == "D")
        .filter_map(|(channel, _)| matterless_sidebar::counterpart(&channel.name, me_id))
        .collect();
    // The server's `TeammateNameDisplay` reaches the app through bootstrap,
    // which this window does not run. Phase 0 measured this deployment on
    // `username`, which is what an empty mode resolves to.
    let names: HashMap<String, String> = store
        .users_by_ids(&counterparts)
        .unwrap_or_default()
        .into_iter()
        .map(|(id, user)| (id, matterless_render::display_name(&user, "")))
        .collect();

    let summaries: Vec<ChannelSummary> = channels
        .into_iter()
        .map(|(channel, unread)| ChannelSummary {
            display_name: matterless_sidebar::label(&channel, me_id, &names),
            counterpart_id: if channel.channel_type == "D" {
                matterless_sidebar::counterpart(&channel.name, me_id)
            } else {
                None
            },
            id: channel.id,
            team_id: channel.team_id,
            channel_type: channel.channel_type,
            last_post_at: channel.last_post_at,
            unread: if roots_only {
                unread.messages_root
            } else {
                unread.messages
            },
            mentions: if roots_only {
                unread.mentions_root
            } else {
                unread.mentions
            },
            muted: unread.muted,
        })
        .collect();

    let categories = store.sidebar().unwrap_or_default();
    // Every team has its own "Favorites" and its own "Channels", so without
    // these two headings read as duplicates of each other.
    let teams = store.teams().unwrap_or_default();
    let team_names: HashMap<String, String> = teams
        .iter()
        .map(|team| {
            let label = if team.display_name.is_empty() {
                team.name.clone()
            } else {
                team.display_name.clone()
            };
            (team.id.clone(), label)
        })
        .collect();
    let arranged = store
        .preference(me_id, "teams_order", "")
        .ok()
        .flatten()
        .unwrap_or_default();
    let order = matterless_sidebar::team_order(
        &arranged,
        &teams
            .iter()
            .map(|team| team.id.clone())
            .collect::<Vec<String>>(),
    );
    matterless_sidebar::arrange(summaries, categories, &team_names, &order)
}
