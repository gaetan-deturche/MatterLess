//! The rules a sidebar has to get right, none of which were covered while this
//! lived inside a Tauri command.

use super::*;

fn channel(id: &str, kind: &str, name: &str, display: &str) -> Channel {
    Channel {
        id: id.into(),
        team_id: String::new(),
        channel_type: kind.into(),
        name: name.into(),
        display_name: display.into(),
        total_msg_count: 0,
        total_msg_count_root: 0,
        last_post_at: 0,
        delete_at: 0,
    }
}

fn summary(id: &str, display: &str, last_post_at: i64) -> ChannelSummary {
    ChannelSummary {
        counterpart_id: None,
        id: id.into(),
        team_id: "t1".into(),
        display_name: display.into(),
        channel_type: "O".into(),
        last_post_at,
        unread: 0,
        mentions: 0,
        muted: false,
    }
}

fn category(id: &str, kind: &str, sorting: &str, order: i64) -> SidebarCategory {
    SidebarCategory {
        id: id.into(),
        display_name: id.into(),
        category_type: kind.into(),
        team_id: "t1".into(),
        sorting: sorting.into(),
        sort_order: order,
        muted: false,
        collapsed: false,
        channel_ids: Vec::new(),
    }
}

// ------------------------------------------------------------- direct messages

#[test]
fn a_direct_message_is_named_after_the_other_person() {
    let names = HashMap::from([("them".to_string(), "Simon O'Dwyer".to_string())]);
    let dm = channel("c1", "D", "me__them", "");
    assert_eq!(label(&dm, "me", &names), "Simon O'Dwyer");
}

/// The slug's halves are not ordered, so the reader may be either one.
#[test]
fn the_counterpart_is_found_whichever_half_the_reader_is() {
    assert_eq!(counterpart("me__them", "me"), Some("them".into()));
    assert_eq!(counterpart("them__me", "me"), Some("them".into()));
}

#[test]
fn a_note_to_self_is_called_you() {
    let dm = channel("c1", "D", "me__me", "");
    assert_eq!(label(&dm, "me", &HashMap::new()), "You");
}

/// The failure the native window shows today: with nobody resolved, the label
/// must at least be an id and never the raw `id__id` slug.
#[test]
fn an_unresolved_counterpart_falls_back_to_the_id_not_the_slug() {
    let dm = channel("c1", "D", "me__them", "");
    assert_eq!(label(&dm, "me", &HashMap::new()), "them");
}

#[test]
fn a_named_channel_keeps_its_name() {
    let open = channel("c1", "O", "town-square", "Town Square");
    assert_eq!(label(&open, "me", &HashMap::new()), "Town Square");
}

/// A group message has several people, so it keeps its generated name until
/// the participants are known.
#[test]
fn a_group_message_is_not_treated_as_a_direct_one() {
    let group = channel("c1", "G", "abc123", "");
    assert_eq!(label(&group, "me", &HashMap::new()), "abc123");
    assert_eq!(counterpart("abc123", "me"), None);
}

// -------------------------------------------------------------------- ordering

#[test]
fn each_category_is_sorted_on_its_own_terms() {
    let mut manual = vec![summary("b", "beta", 1), summary("a", "alpha", 2)];
    let ordered = manual.clone();
    sort_group(&mut manual, "manual");
    assert_eq!(manual, ordered, "manual keeps the reader's own order");

    let mut recent = vec![summary("a", "alpha", 1), summary("b", "beta", 2)];
    sort_group(&mut recent, "recent");
    assert_eq!(recent[0].id, "b", "recent leads with the newest");

    // Empty means the server left it at its default.
    let mut default = vec![summary("b", "Beta", 1), summary("a", "alpha", 2)];
    sort_group(&mut default, "");
    assert_eq!(
        default[0].id, "a",
        "the default is alphabetical, and caseless"
    );
}

#[test]
fn favourites_lead_and_direct_messages_trail() {
    let summaries = vec![
        summary("fav", "favourite", 0),
        summary("chan", "channel", 0),
        summary("dm", "direct", 0),
    ];
    let categories = vec![
        (category("Channels", "channels", "", 1), vec!["chan".into()]),
        (
            category("Directs", "direct_messages", "recent", 2),
            vec!["dm".into()],
        ),
        (
            category("Favorites", "favorites", "", 0),
            vec!["fav".into()],
        ),
    ];
    let groups = arrange(
        summaries,
        categories,
        &HashMap::new(),
        &HashMap::from([("t1".to_string(), 0usize)]),
    );
    let order: Vec<&str> = groups
        .iter()
        .map(|group| group.category_type.as_str())
        .collect();
    assert_eq!(order, vec!["favorites", "channels", "direct_messages"]);
}

/// The duplication that turns 114 channels into 172 if the groups are summed:
/// every team returns the same direct-message conversations.
#[test]
fn direct_messages_from_every_team_are_listed_once() {
    let summaries = vec![summary("dm", "direct", 0)];
    let mut second = category("Directs", "direct_messages", "recent", 0);
    second.team_id = "t2".into();
    let categories = vec![
        (
            category("Directs", "direct_messages", "recent", 0),
            vec!["dm".into()],
        ),
        (second, vec!["dm".into()]),
    ];
    let groups = arrange(summaries, categories, &HashMap::new(), &HashMap::new());
    assert_eq!(groups.len(), 1);
    assert_eq!(groups[0].channels.len(), 1);
}

/// A channel joined seconds ago is in no category until the server says so, and
/// must still be reachable.
#[test]
fn a_channel_in_no_category_is_still_listed() {
    let summaries = vec![summary("chan", "channel", 0), summary("new", "joined", 0)];
    let categories = vec![(category("Channels", "channels", "", 0), vec!["chan".into()])];
    let groups = arrange(summaries, categories, &HashMap::new(), &HashMap::new());
    let other = groups
        .iter()
        .find(|group| group.id == "uncategorised")
        .expect("the ungrouped channel is reachable");
    assert_eq!(other.channels.len(), 1);
    assert_eq!(other.channels[0].id, "new");
}

#[test]
fn teams_follow_the_order_the_reader_dragged_them_into() {
    let order = team_order("t2,t1", &["t1".into(), "t2".into(), "t3".into()]);
    assert_eq!(order.get("t2"), Some(&0));
    assert_eq!(order.get("t1"), Some(&1));
    // Not mentioned by the preference, so it keeps its place at the end.
    assert_eq!(order.get("t3"), Some(&2));
}
