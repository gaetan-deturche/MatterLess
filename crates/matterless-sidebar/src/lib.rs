//! What each channel is called, and the order the channels sit in.
//!
//! Both are decisions rather than lookups. A direct message has no name of its
//! own -- the server sends an empty `display_name` and a `<userA>__<userB>`
//! slug -- so what a reader expects to see has to be worked out from who the
//! other party is. And the order is the reader's own: the server's categories,
//! each sorted on its own terms, teams in the order they dragged them into.
//!
//! This lives apart from any shell because two of them need it. It was written
//! inside the Tauri command layer, which the native window cannot reach, so
//! that window listed channels flat by recency and labelled direct messages
//! with their raw slug. Nothing here knows about a window, a database or a
//! network: it takes what has been read and answers what to draw.

use matterless_core::model::{Channel, SidebarCategory};
use serde::Serialize;
use std::collections::{HashMap, HashSet};

/// One channel, ready to draw.
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct ChannelSummary {
    /// The other person in a direct message, whose avatar labels it.
    ///
    /// `None` for a channel and for a group message: a group has several
    /// people, so it gets an icon rather than a face.
    pub counterpart_id: Option<String>,
    pub id: String,
    pub team_id: String,
    pub display_name: String,
    pub channel_type: String,
    pub last_post_at: i64,
    pub unread: i64,
    pub mentions: i64,
    pub muted: bool,
}

/// One drawn section of the sidebar.
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct Group {
    pub id: String,
    pub display_name: String,
    /// The server's own order within a team, which breaks ties between several
    /// custom categories.
    pub sort_order: i64,
    /// `favorites`, `custom`, `channels` or `direct_messages`.
    pub category_type: String,
    pub team_id: String,
    /// Shown when more than one team contributes groups.
    pub team_name: String,
    pub collapsed: bool,
    pub channels: Vec<ChannelSummary>,
}

/// A direct message has no display name of its own: the name is
/// `<userA>__<userB>`, and what a person expects to see is the other party.
pub fn counterpart(channel_name: &str, me_id: &str) -> Option<String> {
    let (left, right) = channel_name.split_once("__")?;
    if left == right {
        // A note-to-self channel: both halves are you.
        return Some(me_id.to_string());
    }
    Some(if left == me_id { right } else { left }.to_string())
}

/// What to call a channel, resolving a direct message to the person.
///
/// `names` maps counterpart ids to display names, and is the caller's to supply
/// because filling it in may mean asking the server.
pub fn label(channel: &Channel, me_id: &str, names: &HashMap<String, String>) -> String {
    if !channel.display_name.is_empty() {
        // A group conversation's display name is its whole membership, the
        // reader included -- and reading your own name in the list of people
        // you are talking to is noise in every row of the sidebar. Dropped
        // when the caller has resolved who the reader is; left alone when it
        // has not, rather than guessing at a comma-separated list.
        if channel.channel_type == "G"
            && let Some(mine) = names.get(me_id)
        {
            return without(&channel.display_name, mine);
        }
        return channel.display_name.clone();
    }
    if channel.channel_type == "D" {
        return match counterpart(&channel.name, me_id) {
            Some(id) if id == me_id => "You".to_string(),
            Some(id) => names.get(&id).cloned().unwrap_or(id),
            None => channel.name.clone(),
        };
    }
    // Group DMs fall back to their generated name until participants are named.
    channel.name.clone()
}

/// One name taken out of a comma-separated list of them.
///
/// Matched whole rather than as a substring: "ada" appears inside
/// "adam.smith", and removing it there would leave "m.smith".
fn without(names: &str, mine: &str) -> String {
    let kept: Vec<&str> = names
        .split(',')
        .map(str::trim)
        .filter(|name| !name.is_empty() && *name != mine)
        .collect();
    // Everyone in it is the reader, which a note to self of several is not --
    // so the original is a better answer than nothing at all.
    if kept.is_empty() {
        return names.to_string();
    }
    kept.join(", ")
}

/// Orders one group on the terms its category asks for.
pub fn sort_group(channels: &mut [ChannelSummary], sorting: &str) {
    match sorting {
        // Already in the reader's own order, straight from `channel_ids`.
        "manual" => {}
        "recent" => channels.sort_by_key(|channel| std::cmp::Reverse(channel.last_post_at)),
        // Empty means the server left it at its default, which is alphabetical.
        _ => channels.sort_by(|left, right| {
            left.display_name
                .to_lowercase()
                .cmp(&right.display_name.to_lowercase())
        }),
    }
}

/// The reader's team order, from the preference they set by dragging the rail.
///
/// The API returns teams in its own order, so without this preference the
/// sidebar can disagree with every other client they use. Anything the
/// preference does not mention keeps the order it arrived in, after the teams
/// that were arranged.
pub fn team_order(preference: &str, teams: &[String]) -> HashMap<String, usize> {
    let mut order: HashMap<String, usize> = preference
        .split(',')
        .map(str::trim)
        .filter(|id| !id.is_empty())
        .enumerate()
        .map(|(index, id)| (id.to_string(), index))
        .collect();
    let mut next = order.len();
    for team in teams {
        if !order.contains_key(team) {
            order.insert(team.clone(), next);
            next += 1;
        }
    }
    order
}

/// The sidebar, grouped the way the reader arranged it on the server.
///
/// Two things the raw API makes easy to get wrong:
///
/// * **the direct-messages category is returned per team**, holding the same
///   conversations each time, so those categories are merged into one group --
///   the same duplication that turns 114 channels into 172 if summed;
/// * **`sorting` differs per category** (`manual`, `recent`, or empty for the
///   server's alphabetical default), so each group is ordered on its own terms
///   rather than all one way.
pub fn arrange(
    summaries: Vec<ChannelSummary>,
    categories: Vec<(SidebarCategory, Vec<String>)>,
    team_names: &HashMap<String, String>,
    team_order: &HashMap<String, usize>,
) -> Vec<Group> {
    let by_id: HashMap<String, ChannelSummary> = summaries
        .into_iter()
        .map(|row| (row.id.clone(), row))
        .collect();

    let mut groups: Vec<Group> = Vec::new();
    let mut directs: Option<Group> = None;
    let mut placed: HashSet<String> = HashSet::new();

    for (category, channel_ids) in categories {
        let mut channels: Vec<ChannelSummary> = channel_ids
            .iter()
            .filter_map(|id| by_id.get(id).cloned())
            .collect();
        sort_group(&mut channels, &category.sorting);

        if category.category_type == "direct_messages" {
            // Merged across teams: the same conversations, listed once.
            let group = directs.get_or_insert_with(|| Group {
                id: "direct_messages".to_string(),
                display_name: "Direct messages".to_string(),
                sort_order: category.sort_order,
                category_type: category.category_type.clone(),
                team_id: String::new(),
                team_name: String::new(),
                collapsed: category.collapsed,
                channels: Vec::new(),
            });
            for channel in channels {
                if placed.insert(channel.id.clone()) {
                    group.channels.push(channel);
                }
            }
            continue;
        }

        for channel in &channels {
            placed.insert(channel.id.clone());
        }
        groups.push(Group {
            id: category.id,
            display_name: category.display_name,
            sort_order: category.sort_order,
            category_type: category.category_type,
            team_name: team_names
                .get(&category.team_id)
                .cloned()
                .unwrap_or_default(),
            team_id: category.team_id,
            collapsed: category.collapsed,
            channels,
        });
    }

    // Favourites first, then the reader's own groups, then the ungrouped
    // remainder -- and direct messages last, appended below.
    //
    // By category *kind* rather than by name: sorting alphabetically put
    // "Channels" above "Favorites", which is the opposite of what a sidebar
    // should lead with. The server's `sort_order` breaks ties, so several custom
    // categories keep the order they were arranged in.
    groups.sort_by_key(|group| {
        let kind = match group.category_type.as_str() {
            "favorites" => 0,
            "custom" => 1,
            "channels" => 2,
            _ => 3,
        };
        (
            team_order
                .get(&group.team_id)
                .copied()
                .unwrap_or(usize::MAX),
            kind,
            group.sort_order,
            group.display_name.clone(),
        )
    });

    // DMs last, as they are in each team's own order.
    if let Some(mut group) = directs {
        sort_group(&mut group.channels, "recent");
        groups.push(group);
    }

    // Anything the categories did not mention still has to be reachable: a
    // channel joined seconds ago is in no category until the server says so.
    let mut loose: Vec<ChannelSummary> = by_id
        .into_values()
        .filter(|channel| !placed.contains(&channel.id))
        .collect();
    if !loose.is_empty() {
        sort_group(&mut loose, "recent");
        tracing::debug!(count = loose.len(), "channels outside any category");
        groups.push(Group {
            id: "uncategorised".to_string(),
            display_name: "Other".to_string(),
            // Ungrouped, so it sits with the ungrouped: after the reader's own
            // categories, before direct messages.
            sort_order: i64::MAX,
            category_type: "channels".to_string(),
            team_id: String::new(),
            team_name: String::new(),
            collapsed: false,
            channels: loose,
        });
    }

    groups
}

#[cfg(test)]
mod tests;
