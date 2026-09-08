use super::*;
use matterless_core::model::{Channel, ChannelMember, Post, PostMetadata, Preference, Reaction};

fn post(id: &str, channel: &str, create_at: Timestamp, update_at: Timestamp) -> Post {
    Post {
        id: id.into(),
        channel_id: channel.into(),
        user_id: "author".into(),
        root_id: String::new(),
        create_at,
        update_at,
        edit_at: 0,
        delete_at: 0,
        message: format!("message {id}"),
        post_type: String::new(),
        file_ids: Vec::new(),
        props: serde_json::Value::Null,
        metadata: PostMetadata::default(),
        pending_post_id: String::new(),
        is_pinned: false,
    }
}

fn store() -> Store {
    Store::open_in_memory().expect("open in-memory store")
}

#[test]
fn migration_sets_the_version() {
    let store = store();
    let version: i64 = store
        .lock()
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .unwrap();
    assert_eq!(version, schema::TARGET_VERSION);
}

#[test]
fn first_write_inserts_and_the_same_post_again_is_not_a_delta() {
    let store = store();
    let one = post("p1", "c1", 100, 100);

    let first = store.upsert_posts(std::slice::from_ref(&one)).unwrap();
    assert_eq!(first[0].change, PostChange::Inserted);

    // The websocket and a REST page both delivering it must not double-count.
    let second = store.upsert_posts(&[one]).unwrap();
    assert_eq!(second[0].change, PostChange::Unchanged);
}

#[test]
fn an_edit_updates_and_a_stale_copy_never_overwrites() {
    let store = store();
    store.upsert_posts(&[post("p1", "c1", 100, 100)]).unwrap();

    let mut edited = post("p1", "c1", 100, 200);
    edited.message = "edited text".into();
    edited.edit_at = 200;
    let outcome = store.upsert_posts(&[edited]).unwrap();
    assert_eq!(outcome[0].change, PostChange::Updated);
    assert_eq!(store.post("p1").unwrap().unwrap().message, "edited text");

    // A replayed older version arriving late must lose.
    let mut stale = post("p1", "c1", 100, 150);
    stale.message = "stale text".into();
    let outcome = store.upsert_posts(&[stale]).unwrap();
    assert_eq!(outcome[0].change, PostChange::Unchanged);
    assert_eq!(
        store.post("p1").unwrap().unwrap().message,
        "edited text",
        "a stale update_at must not overwrite newer content"
    );
}

#[test]
fn deletion_is_a_tombstone_not_a_removal() {
    let store = store();
    store.upsert_posts(&[post("root", "c1", 100, 100)]).unwrap();

    let mut deleted = post("root", "c1", 100, 300);
    deleted.delete_at = 300;
    let outcome = store.upsert_posts(&[deleted]).unwrap();
    assert_eq!(outcome[0].change, PostChange::Tombstoned);

    let held = store.post("root").unwrap().expect("row still present");
    assert!(held.is_deleted(), "the row must survive as a placeholder");
}

#[test]
fn channel_page_is_newest_first_and_pages_backwards() {
    let store = store();
    store
        .upsert_posts(&[
            post("p1", "c1", 100, 100),
            post("p2", "c1", 200, 200),
            post("p3", "c1", 300, 300),
            post("other", "c2", 250, 250),
        ])
        .unwrap();

    let page = store.channel_page("c1", None, 10).unwrap();
    let ids: Vec<&str> = page.iter().map(|p| p.id.as_str()).collect();
    assert_eq!(ids, vec!["p3", "p2", "p1"]);

    let older = store.channel_page("c1", Some(200), 10).unwrap();
    assert_eq!(older.len(), 1);
    assert_eq!(older[0].id, "p1");
}

#[test]
fn replies_are_reachable_from_their_root() {
    let store = store();
    let mut reply = post("r1", "c1", 200, 200);
    reply.root_id = "p1".into();
    store
        .upsert_posts(&[post("p1", "c1", 100, 100), reply])
        .unwrap();

    let replies = store.thread_replies("p1").unwrap();
    assert_eq!(replies.len(), 1);
    assert_eq!(replies[0].id, "r1");
}

#[test]
fn reactions_and_files_come_from_post_metadata() {
    let store = store();
    let mut with_extras = post("p1", "c1", 100, 100);
    with_extras.metadata.reactions = vec![Reaction {
        user_id: "u1".into(),
        post_id: "p1".into(),
        emoji_name: "thumbsup".into(),
        create_at: 110,
    }];
    store.upsert_posts(&[with_extras]).unwrap();

    let reactions = store.reactions("p1").unwrap();
    assert_eq!(reactions.len(), 1);
    assert_eq!(reactions[0].emoji_name, "thumbsup");
}

#[test]
fn unread_is_derived_from_the_two_counters() {
    let store = store();
    store
        .upsert_channels(&[Channel {
            id: "c1".into(),
            team_id: "t1".into(),
            channel_type: "O".into(),
            name: "town".into(),
            display_name: "Town Square".into(),
            total_msg_count: 100,
            total_msg_count_root: 60,
            last_post_at: 500,
            delete_at: 0,
        }])
        .unwrap();

    let mut notify_props = HashMap::new();
    notify_props.insert("mark_unread".to_string(), "mention".to_string());
    store
        .upsert_channel_members(&[ChannelMember {
            channel_id: "c1".into(),
            user_id: "me".into(),
            last_viewed_at: 400,
            msg_count: 90,
            msg_count_root: 55,
            mention_count: 2,
            mention_count_root: 1,
            notify_props,
        }])
        .unwrap();

    let unread = store.unread("c1", "me").unwrap().expect("membership known");
    assert_eq!(unread.messages, 10);
    assert_eq!(unread.messages_root, 5);
    assert_eq!(unread.mentions, 2);
    assert!(unread.muted, "mark_unread=mention means muted");

    assert!(
        store.unread("c1", "someone-else").unwrap().is_none(),
        "unknown membership must be None, not zero"
    );
}

#[test]
fn local_search_finds_a_post_and_skips_tombstones() {
    let store = store();
    let mut interesting = post("p1", "c1", 100, 100);
    interesting.message = "the renderer crashed on Nanite shading".into();
    let mut removed = post("p2", "c1", 200, 200);
    removed.message = "Nanite something deleted".into();
    removed.delete_at = 250;
    store.upsert_posts(&[interesting, removed]).unwrap();

    let hits = store.search("Nanite", 10).unwrap();
    assert_eq!(hits.len(), 1, "the tombstoned post must not surface");
    assert_eq!(hits[0].id, "p1");
}

#[test]
fn search_index_follows_an_edit() {
    let store = store();
    store.upsert_posts(&[post("p1", "c1", 100, 100)]).unwrap();

    let mut edited = post("p1", "c1", 100, 200);
    edited.message = "now mentions Lumen".into();
    store.upsert_posts(&[edited]).unwrap();

    assert_eq!(store.search("Lumen", 10).unwrap().len(), 1);
    assert!(
        store.search("message", 10).unwrap().is_empty(),
        "the old text must leave the index"
    );
}

#[test]
fn resync_order_puts_the_active_channel_first_then_unread() {
    let store = store();
    let channel = |id: &str, total: i64, last_post_at: Timestamp| Channel {
        id: id.into(),
        team_id: "t1".into(),
        channel_type: "O".into(),
        name: id.into(),
        display_name: id.into(),
        total_msg_count: total,
        total_msg_count_root: total,
        last_post_at,
        delete_at: 0,
    };
    store
        .upsert_channels(&[
            channel("quiet", 10, 100),
            channel("unread", 20, 200),
            channel("active", 5, 50),
            channel("recent", 10, 900),
        ])
        .unwrap();

    let member = |channel_id: &str, seen: i64| ChannelMember {
        channel_id: channel_id.into(),
        user_id: "me".into(),
        last_viewed_at: 0,
        msg_count: seen,
        msg_count_root: seen,
        mention_count: 0,
        mention_count_root: 0,
        notify_props: HashMap::new(),
    };
    store
        .upsert_channel_members(&[
            member("quiet", 10),  // fully read
            member("unread", 15), // 5 unread
            member("active", 5),  // fully read
            member("recent", 10), // fully read
        ])
        .unwrap();

    let order = store.resync_order(Some("active"), "me").unwrap();
    assert_eq!(order[0], "active", "the visible channel is resynced first");
    assert_eq!(order[1], "unread", "then anything with unread messages");
    assert_eq!(
        order[2], "recent",
        "then by recency, so the trickle is useful-first"
    );
}

#[test]
fn sync_state_round_trips() {
    let store = store();
    assert!(store.sync_state("c1").unwrap().is_none());

    store
        .set_sync_state(&SyncState {
            channel_id: "c1".into(),
            synced_from: 100,
            synced_to: 900,
            oldest_post_id: "p1".into(),
            newest_post_id: "p9".into(),
            reached_beginning: true,
            last_reconciled_at: 1000,
        })
        .unwrap();

    let held = store.sync_state("c1").unwrap().unwrap();
    assert_eq!(held.synced_from, 100);
    assert_eq!(held.synced_to, 900);
    assert!(held.reached_beginning);
}

#[test]
fn websocket_state_round_trips() {
    let store = store();
    assert_eq!(store.ws_state().unwrap(), (String::new(), 0));
    store.set_ws_state("conn-1", 42, 1000).unwrap();
    assert_eq!(store.ws_state().unwrap(), ("conn-1".to_string(), 42));
}

#[test]
fn preferences_round_trip_and_a_missing_one_is_none() {
    let store = store();
    // Reading before anything is written must be None, not an error -- the table
    // now exists from migration 1 rather than being created on first write.
    assert!(
        store
            .preference("me", "display_settings", "collapsed_reply_threads")
            .unwrap()
            .is_none()
    );

    store
        .upsert_preferences(&[Preference {
            user_id: "me".into(),
            category: "display_settings".into(),
            name: "collapsed_reply_threads".into(),
            value: "on".into(),
        }])
        .unwrap();
    assert_eq!(
        store
            .preference("me", "display_settings", "collapsed_reply_threads")
            .unwrap()
            .as_deref(),
        Some("on")
    );

    // An upsert of the same key updates rather than duplicating.
    store
        .upsert_preferences(&[Preference {
            user_id: "me".into(),
            category: "display_settings".into(),
            name: "collapsed_reply_threads".into(),
            value: "off".into(),
        }])
        .unwrap();
    assert_eq!(
        store
            .preference("me", "display_settings", "collapsed_reply_threads")
            .unwrap()
            .as_deref(),
        Some("off")
    );
}

#[test]
fn the_sidebar_comes_from_one_query() {
    let store = store();
    store
        .upsert_channels(&[
            Channel {
                id: "busy".into(),
                team_id: "t1".into(),
                channel_type: "O".into(),
                name: "busy".into(),
                display_name: "Busy".into(),
                total_msg_count: 50,
                total_msg_count_root: 50,
                last_post_at: 900,
                delete_at: 0,
            },
            Channel {
                id: "quiet".into(),
                team_id: "t1".into(),
                channel_type: "O".into(),
                name: "quiet".into(),
                display_name: "Quiet".into(),
                total_msg_count: 5,
                total_msg_count_root: 5,
                last_post_at: 100,
                delete_at: 0,
            },
        ])
        .unwrap();
    store
        .upsert_channel_members(&[ChannelMember {
            channel_id: "busy".into(),
            user_id: "me".into(),
            last_viewed_at: 0,
            msg_count: 42,
            msg_count_root: 42,
            mention_count: 3,
            mention_count_root: 1,
            notify_props: HashMap::new(),
        }])
        .unwrap();

    let sidebar = store.channels_with_unread("me").unwrap();
    assert_eq!(sidebar.len(), 2, "most recently active first");
    assert_eq!(sidebar[0].0.id, "busy");
    assert_eq!(sidebar[0].1.messages, 8);
    assert_eq!(sidebar[0].1.mentions, 3);
    // A channel with no membership row must not report phantom unread.
    assert_eq!(sidebar[1].0.id, "quiet");
    assert_eq!(sidebar[1].1.messages, 5, "no member row means all unseen");
}

#[test]
fn thread_summaries_group_replies_by_root() {
    let store = store();
    let mut first = post("r1", "c1", 200, 200);
    first.root_id = "root".into();
    first.user_id = "amy".into();
    let mut second = post("r2", "c1", 300, 300);
    second.root_id = "root".into();
    second.user_id = "bob".into();
    let mut removed = post("r3", "c1", 400, 400);
    removed.root_id = "root".into();
    removed.delete_at = 500;

    store
        .upsert_posts(&[post("root", "c1", 100, 100), first, second, removed])
        .unwrap();

    let summaries = store
        .thread_summaries_for("c1", &["root".to_string()])
        .unwrap();
    let (count, last_reply_at, participants) = summaries.get("root").unwrap();
    assert_eq!(*count, 2, "the deleted reply must not be counted");
    assert_eq!(*last_reply_at, 300);
    assert_eq!(participants.len(), 2);

    // A root outside the window costs nothing and returns nothing.
    assert!(
        store
            .thread_summaries_for("c1", &["not-on-screen".to_string()])
            .unwrap()
            .is_empty()
    );
    assert!(store.thread_summaries_for("c1", &[]).unwrap().is_empty());
}

/// Collapsed threads discard every reply, so reading them is pure waste: three
/// JSON columns deserialised per post that can never become a row.
#[test]
fn a_roots_only_page_skips_replies_in_sql() {
    let store = store();
    let mut reply = post("r1", "c1", 200, 200);
    reply.root_id = "root".into();
    let mut second = post("r2", "c1", 300, 300);
    second.root_id = "root".into();
    store
        .upsert_posts(&[
            post("root", "c1", 100, 100),
            reply,
            second,
            post("other", "c1", 400, 400),
        ])
        .unwrap();

    let everything = store.channel_page("c1", None, 100).unwrap();
    assert_eq!(everything.len(), 4);

    let roots = store
        .channel_page_filtered("c1", None, None, 100, true)
        .unwrap();
    let ids: Vec<&str> = roots.iter().map(|post| post.id.as_str()).collect();
    assert_eq!(ids, vec!["other", "root"], "newest first, replies excluded");

    // The limit now means roughly what the reader sees, rather than being
    // diluted by replies that get filtered out afterwards.
    let one = store
        .channel_page_filtered("c1", None, None, 1, true)
        .unwrap();
    assert_eq!(one.len(), 1);
    assert_eq!(one[0].id, "other");
}

#[test]
fn the_badge_counts_mentions_and_followed_channels_but_not_muted_ones() {
    let store = store();
    let channel = |id: &str, total: i64| Channel {
        id: id.into(),
        team_id: "t1".into(),
        channel_type: "O".into(),
        name: id.into(),
        display_name: id.into(),
        total_msg_count: total,
        total_msg_count_root: total,
        last_post_at: 100,
        delete_at: 0,
    };
    let member = |id: &str, seen: i64, mentions: i64, props: &[(&str, &str)]| ChannelMember {
        channel_id: id.into(),
        user_id: "me".into(),
        last_viewed_at: 0,
        msg_count: seen,
        msg_count_root: seen,
        mention_count: mentions,
        mention_count_root: mentions,
        notify_props: props
            .iter()
            .map(|(key, value)| (key.to_string(), value.to_string()))
            .collect(),
    };

    store
        .upsert_channels(&[
            channel("quiet", 10),    // unread, no mentions -> dot only
            channel("tagged", 10),   // 2 mentions
            channel("followed", 10), // desktop=all, 5 unread
            channel("muted", 10),    // muted, ignored entirely
        ])
        .unwrap();
    store
        .upsert_channel_members(&[
            member("quiet", 7, 0, &[]),
            member("tagged", 8, 2, &[]),
            member("followed", 5, 0, &[("desktop", "all")]),
            member("muted", 0, 4, &[("mark_unread", "mention")]),
        ])
        .unwrap();

    let state = store.badge_state("me", false).unwrap();
    assert!(
        state.any_unread,
        "quiet has unread, so the dot is warranted"
    );
    assert_eq!(state.mentions, 2, "the muted channel's 4 are excluded");
    assert_eq!(state.followed_unread, 5);
    assert_eq!(
        state.attention(),
        7,
        "2 mentions + 5 unread in the followed channel; the muted channel's          4 mentions are deliberately excluded"
    );
}

/// Regression: inheriting the account-wide level made the badge read 33 where
/// the official client read 1, because Mattermost ships that level as `all`.
#[test]
fn a_channel_on_default_counts_mentions_only_whatever_the_account_says() {
    let store = store();
    store
        .upsert_channels(&[Channel {
            id: "c1".into(),
            team_id: "t1".into(),
            channel_type: "O".into(),
            name: "c1".into(),
            display_name: "c1".into(),
            total_msg_count: 10,
            total_msg_count_root: 10,
            last_post_at: 100,
            delete_at: 0,
        }])
        .unwrap();
    let mut props = HashMap::new();
    props.insert("desktop".to_string(), "default".to_string());
    store
        .upsert_channel_members(&[ChannelMember {
            channel_id: "c1".into(),
            user_id: "me".into(),
            last_viewed_at: 0,
            msg_count: 6,
            msg_count_root: 6,
            mention_count: 0,
            mention_count_root: 0,
            notify_props: props,
        }])
        .unwrap();

    let viewer = |level: &str| {
        let mut notify = HashMap::new();
        notify.insert("desktop".to_string(), level.to_string());
        store
            .upsert_users(&[User {
                id: "me".into(),
                username: "me".into(),
                first_name: String::new(),
                last_name: String::new(),
                nickname: String::new(),
                email: String::new(),
                last_picture_update: 0,
                notify_props: notify,
                roles: String::new(),
            }])
            .unwrap();
    };

    // 4 unread, no mentions: the badge count stays empty at every account
    // level, because this channel never asked for every message.
    for level in ["all", "mention", "none"] {
        viewer(level);
        let state = store.badge_state("me", false).unwrap();
        assert_eq!(
            state.attention(),
            0,
            "account level {level} must not leak in"
        );
        assert!(state.any_unread, "the dot still shows: there IS unread");
    }
}

#[test]
fn nothing_unread_means_no_badge_at_all() {
    let store = store();
    store
        .upsert_channels(&[Channel {
            id: "c1".into(),
            team_id: "t1".into(),
            channel_type: "O".into(),
            name: "c1".into(),
            display_name: "c1".into(),
            total_msg_count: 10,
            total_msg_count_root: 10,
            last_post_at: 100,
            delete_at: 0,
        }])
        .unwrap();
    store
        .upsert_channel_members(&[ChannelMember {
            channel_id: "c1".into(),
            user_id: "me".into(),
            last_viewed_at: 0,
            msg_count: 10,
            msg_count_root: 10,
            mention_count: 0,
            mention_count_root: 0,
            notify_props: HashMap::new(),
        }])
        .unwrap();
    let state = store.badge_state("me", false).unwrap();
    assert!(!state.any_unread);
    assert_eq!(state.attention(), 0);
}

#[test]
fn the_post_above_a_page_is_the_newest_one_older_than_it() {
    let store = store();
    let posts: Vec<Post> = (1..=5)
        .map(|n| post(&format!("p{n}"), "c1", n * 1000, n * 1000))
        .collect();
    store.upsert_posts(&posts).unwrap();

    // Paging back from p3: the row above it is p2, not p1 and not p4.
    let above = store.post_above("c1", 3000, false).unwrap().unwrap();
    assert_eq!(above.id, "p2");

    // The oldest page has nothing above it.
    assert!(store.post_above("c1", 1000, false).unwrap().is_none());
}

/// The newest page is defined by a floor, not by a count: "the newest N posts"
/// slides forward as messages arrive and drops posts off its bottom, which
/// leaves a hole between it and the page below.
#[test]
fn a_floor_pins_a_page_while_newer_posts_arrive() {
    let store = store();
    let posts: Vec<Post> = (1..=4)
        .map(|n| post(&format!("p{n}"), "c1", n * 1000, n * 1000))
        .collect();
    store.upsert_posts(&posts).unwrap();

    // A page floored at p2 holds p2 upwards, whatever its limit allows.
    let page = store
        .channel_page_filtered("c1", None, Some(2000), 100, false)
        .unwrap();
    let ids: Vec<&str> = page.iter().map(|post| post.id.as_str()).collect();
    assert_eq!(ids, vec!["p4", "p3", "p2"]);

    // A newer post joins that same page rather than pushing p2 out of it.
    store.upsert_posts(&[post("p5", "c1", 5000, 5000)]).unwrap();
    let page = store
        .channel_page_filtered("c1", None, Some(2000), 100, false)
        .unwrap();
    assert_eq!(page.len(), 4, "p2 must still be in its page");
    assert_eq!(page.first().unwrap().id, "p5");

    // And the page below is unaffected, so the seam has no gap.
    let below = store
        .channel_page_filtered("c1", Some(2000), None, 100, false)
        .unwrap();
    assert_eq!(
        below
            .iter()
            .map(|post| post.id.as_str())
            .collect::<Vec<_>>(),
        vec!["p1"]
    );
}

// ---- threads ---------------------------------------------------------------

fn user_thread(root_id: &str, unread_replies: i64, unread_mentions: i64) -> UserThread {
    UserThread {
        id: root_id.into(),
        reply_count: 7,
        last_reply_at: 9000,
        last_viewed_at: 5000,
        unread_replies,
        unread_mentions,
        is_urgent: false,
        delete_at: 0,
        post: post(root_id, "c1", 1000, 1000),
        participants: None,
    }
}

#[test]
fn followed_threads_round_trip_with_the_servers_own_counts() {
    let store = store();
    store
        .upsert_threads(&[user_thread("r1", 3, 1), user_thread("r2", 0, 0)])
        .unwrap();

    let states = store
        .thread_states_for(&["r1".to_string(), "r2".to_string(), "missing".to_string()])
        .unwrap();
    assert_eq!(states.len(), 2, "a thread nobody follows has no row");
    let one = states.get("r1").unwrap();
    assert!(one.following);
    assert_eq!(one.unread_replies, 3);
    assert_eq!(one.unread_mentions, 1);
    assert_eq!(one.reply_count, 7);
}

#[test]
fn reading_a_thread_clears_its_unread_and_never_rewinds_the_watermark() {
    let store = store();
    store.upsert_threads(&[user_thread("r1", 4, 2)]).unwrap();

    assert!(store.mark_thread_viewed("r1", 9000).unwrap());
    let state = store.thread_states_for(&["r1".to_string()]).unwrap();
    let state = state.get("r1").unwrap();
    assert_eq!((state.unread_replies, state.unread_mentions), (0, 0));
    assert_eq!(state.last_viewed_at, 9000);

    // A later refresh carrying an older watermark must not undo the read.
    store.upsert_threads(&[user_thread("r1", 0, 0)]).unwrap();
    let state = store.thread_states_for(&["r1".to_string()]).unwrap();
    assert_eq!(state.get("r1").unwrap().last_viewed_at, 9000);
}

#[test]
fn a_thread_event_can_move_the_counts_without_a_fetch() {
    let store = store();
    store.upsert_threads(&[user_thread("r1", 0, 0)]).unwrap();
    store
        .record_thread_activity("r1", "c1", 8, 12_000, 1, 0)
        .unwrap();

    let states = store.thread_states_for(&["r1".to_string()]).unwrap();
    let state = states.get("r1").unwrap();
    assert_eq!(state.unread_replies, 1);
    assert_eq!(state.reply_count, 8);
    assert_eq!(state.last_reply_at, 12_000);
}

/// Thread unread is deliberately its own total: with collapsed threads on a
/// reply never bumps channel unread, so folding these together would blend two
/// models of read state.
#[test]
fn thread_unread_totals_count_only_followed_and_live_threads() {
    let store = store();
    store
        .upsert_threads(&[user_thread("r1", 3, 1), user_thread("r2", 2, 0)])
        .unwrap();
    assert_eq!(store.thread_unread_totals().unwrap(), (5, 1));

    store.set_thread_following("r2", false).unwrap();
    assert_eq!(
        store.thread_unread_totals().unwrap(),
        (3, 1),
        "an unfollowed thread stops demanding anything"
    );
}

/// Thread mentions belong on the badge, and are additive: with collapsed
/// threads a reply never touches `channel_members.mention_count`, so the two
/// counts describe different things.
#[test]
fn the_badge_adds_thread_mentions_to_channel_mentions() {
    let store = store();
    store
        .upsert_channels(&[Channel {
            id: "c1".into(),
            team_id: "t1".into(),
            channel_type: "O".into(),
            name: "c1".into(),
            display_name: "c1".into(),
            total_msg_count: 10,
            total_msg_count_root: 10,
            last_post_at: 100,
            delete_at: 0,
        }])
        .unwrap();
    store
        .upsert_channel_members(&[ChannelMember {
            channel_id: "c1".into(),
            user_id: "me".into(),
            last_viewed_at: 0,
            msg_count: 8,
            msg_count_root: 8,
            mention_count: 2,
            mention_count_root: 2,
            notify_props: HashMap::new(),
        }])
        .unwrap();
    store.upsert_threads(&[user_thread("r1", 4, 3)]).unwrap();

    let badge = store.badge_state("me", false).unwrap();
    assert_eq!(badge.mentions, 2, "the channel's own mentions");
    assert_eq!(badge.thread_mentions, 3, "counted separately by the server");
    assert_eq!(badge.attention(), 5);
}

/// A followed thread with unread replies is something unread, even when the
/// channel it lives in has been read -- which is the case the whole thread
/// workstream exists for.
#[test]
fn unread_thread_replies_alone_light_the_dot() {
    let store = store();
    store
        .upsert_channels(&[Channel {
            id: "c1".into(),
            team_id: "t1".into(),
            channel_type: "O".into(),
            name: "c1".into(),
            display_name: "c1".into(),
            total_msg_count: 10,
            total_msg_count_root: 10,
            last_post_at: 100,
            delete_at: 0,
        }])
        .unwrap();
    store
        .upsert_channel_members(&[ChannelMember {
            channel_id: "c1".into(),
            user_id: "me".into(),
            last_viewed_at: 0,
            msg_count: 10,
            msg_count_root: 10,
            mention_count: 0,
            mention_count_root: 0,
            notify_props: HashMap::new(),
        }])
        .unwrap();
    assert!(
        !store.badge_state("me", false).unwrap().any_unread,
        "channel is read"
    );

    store.upsert_threads(&[user_thread("r1", 2, 0)]).unwrap();
    let badge = store.badge_state("me", false).unwrap();
    assert!(badge.any_unread, "two unread replies in a followed thread");
    assert_eq!(badge.attention(), 0, "unread replies are not a mention");
}

/// A version number is not proof: a build that stamped `user_version = 4`
/// without running migration 4 left a database with no `threads` table, and the
/// version gate then skipped the migration forever.
#[test]
fn a_stamped_but_missing_table_is_repaired_rather_than_trusted() {
    let store = store();
    {
        let connection = store.connection.lock().unwrap();
        connection.execute_batch("DROP TABLE threads;").unwrap();
        // Exactly the state the bug left behind: current version, absent table.
        connection
            .pragma_update(None, "user_version", 4_i64)
            .unwrap();
        assert!(
            connection
                .query_row(
                    "SELECT COUNT(*) FROM sqlite_master WHERE name = 'threads'",
                    [],
                    |row| row.get::<_, i64>(0)
                )
                .unwrap()
                == 0
        );
        crate::schema::prepare(&connection).unwrap();
    }
    // The repair ran, so thread queries work again.
    assert_eq!(store.thread_unread_totals().unwrap(), (0, 0));
}

/// The bug the user caught: with collapsed threads on, a channel was counting
/// replies -- including replies in threads they do not follow -- because unread
/// came from the all-posts counters instead of the root ones.
#[test]
fn collapsed_threads_count_roots_only() {
    let store = store();
    store
        .upsert_channels(&[Channel {
            id: "c1".into(),
            team_id: "t1".into(),
            channel_type: "O".into(),
            name: "c1".into(),
            display_name: "c1".into(),
            // Ten posts arrived, of which two were roots: the rest are replies.
            total_msg_count: 10,
            total_msg_count_root: 2,
            last_post_at: 100,
            delete_at: 0,
        }])
        .unwrap();
    store
        .upsert_channel_members(&[ChannelMember {
            channel_id: "c1".into(),
            user_id: "me".into(),
            last_viewed_at: 0,
            msg_count: 0,
            msg_count_root: 0,
            mention_count: 3,
            mention_count_root: 1,
            notify_props: HashMap::new(),
        }])
        .unwrap();

    let unread = store.unread("c1", "me").unwrap().unwrap();
    assert_eq!(
        unread.visible(true),
        (2, 1),
        "collapsed: two unread roots, one root mention"
    );
    assert_eq!(
        unread.visible(false),
        (10, 3),
        "flat: the replies are in the stream, so they count"
    );

    // And the badge follows the same rule.
    assert_eq!(store.badge_state("me", true).unwrap().mentions, 1);
    assert_eq!(store.badge_state("me", false).unwrap().mentions, 3);
}

// ---- the sidebar the reader arranged --------------------------------------

fn category(
    id: &str,
    team: &str,
    kind: &str,
    name: &str,
    order: i64,
    sorting: &str,
    channels: &[&str],
) -> SidebarCategory {
    SidebarCategory {
        id: id.into(),
        team_id: team.into(),
        category_type: kind.into(),
        display_name: name.into(),
        sort_order: order,
        sorting: sorting.into(),
        muted: false,
        collapsed: false,
        channel_ids: channels.iter().map(|id| id.to_string()).collect(),
    }
}

#[test]
fn categories_come_back_in_sidebar_order_with_their_channels() {
    let store = store();
    store
        .upsert_sidebar(
            "t1",
            &[
                category("fav1", "t1", "favorites", "Favorites", 0, "manual", &["c3"]),
                category("cus1", "t1", "custom", "Horde", 10, "manual", &["c1", "c2"]),
            ],
        )
        .unwrap();

    let sidebar = store.sidebar().unwrap();
    assert_eq!(sidebar.len(), 2);
    assert_eq!(sidebar[0].0.display_name, "Favorites");
    assert_eq!(sidebar[1].0.display_name, "Horde");
    assert_eq!(
        sidebar[1].1,
        vec!["c1".to_string(), "c2".to_string()],
        "a manual category keeps the order the reader put it in"
    );
}

/// A channel moved out of a category has to stop being in it: upserting alone
/// would leave it in both, and it would appear twice in the sidebar.
#[test]
fn re_storing_a_team_replaces_its_categories_rather_than_adding_to_them() {
    let store = store();
    store
        .upsert_sidebar(
            "t1",
            &[category(
                "cus1",
                "t1",
                "custom",
                "Horde",
                10,
                "manual",
                &["c1", "c2"],
            )],
        )
        .unwrap();
    store
        .upsert_sidebar(
            "t1",
            &[category(
                "cus1",
                "t1",
                "custom",
                "Horde",
                10,
                "manual",
                &["c2"],
            )],
        )
        .unwrap();

    let sidebar = store.sidebar().unwrap();
    assert_eq!(sidebar.len(), 1);
    assert_eq!(sidebar[0].1, vec!["c2".to_string()], "c1 moved out");
}

/// Each team is stored on its own, so refreshing one cannot drop the other's.
#[test]
fn teams_do_not_overwrite_each_other() {
    let store = store();
    store
        .upsert_sidebar(
            "t1",
            &[category("a", "t1", "channels", "Channels", 10, "", &["c1"])],
        )
        .unwrap();
    store
        .upsert_sidebar(
            "t2",
            &[category("b", "t2", "channels", "Channels", 10, "", &["c9"])],
        )
        .unwrap();
    store
        .upsert_sidebar(
            "t1",
            &[category(
                "a",
                "t1",
                "channels",
                "Channels",
                10,
                "",
                &["c1", "c2"],
            )],
        )
        .unwrap();

    let sidebar = store.sidebar().unwrap();
    assert_eq!(sidebar.len(), 2);
    let teams: Vec<&str> = sidebar.iter().map(|(c, _)| c.team_id.as_str()).collect();
    assert!(teams.contains(&"t1") && teams.contains(&"t2"));
}

// ---- reactions -------------------------------------------------------------
//
// They live inside `post.metadata`, so a `reaction_added` event has to patch
// that JSON. For a while it only emitted a delta: the plan was rebuilt from the
// old metadata and the reaction appeared after the next REST fetch, not before.

#[test]
fn a_reaction_is_stored_against_the_post_and_read_back() {
    let store = store();
    store.upsert_posts(&[post("p1", "c1", 1000, 1000)]).unwrap();

    assert!(store.add_reaction("p1", "amy", "tada", 2000).unwrap());
    let held = store.post("p1").unwrap().unwrap();
    assert_eq!(held.metadata.reactions.len(), 1);
    assert_eq!(held.metadata.reactions[0].emoji_name, "tada");
    assert_eq!(held.metadata.reactions[0].user_id, "amy");
}

/// The same person cannot react twice with the same emoji, and the echo of
/// one's own optimistic reaction must not cost a re-render.
#[test]
fn the_same_reaction_twice_changes_nothing() {
    let store = store();
    store.upsert_posts(&[post("p1", "c1", 1000, 1000)]).unwrap();
    assert!(store.add_reaction("p1", "amy", "tada", 2000).unwrap());
    assert!(
        !store.add_reaction("p1", "amy", "tada", 3000).unwrap(),
        "no change, so no delta"
    );
    assert_eq!(
        store.post("p1").unwrap().unwrap().metadata.reactions.len(),
        1
    );
}

#[test]
fn removing_a_reaction_leaves_everyone_elses() {
    let store = store();
    store.upsert_posts(&[post("p1", "c1", 1000, 1000)]).unwrap();
    store.add_reaction("p1", "amy", "tada", 2000).unwrap();
    store.add_reaction("p1", "bob", "tada", 2100).unwrap();
    store.add_reaction("p1", "amy", "eyes", 2200).unwrap();

    assert!(store.remove_reaction("p1", "amy", "tada").unwrap());
    let held = store.post("p1").unwrap().unwrap();
    let mine: Vec<&str> = held
        .metadata
        .reactions
        .iter()
        .map(|reaction| reaction.emoji_name.as_str())
        .collect();
    assert_eq!(mine.len(), 2);
    assert!(
        held.metadata
            .reactions
            .iter()
            .any(|r| r.user_id == "bob" && r.emoji_name == "tada")
    );
    assert!(
        !store.remove_reaction("p1", "amy", "tada").unwrap(),
        "already gone"
    );
}

/// A reaction changes nothing about the message body, and `update_at` is the
/// markdown cache's key: bumping it would throw away a parsed tree for nothing.
#[test]
fn reacting_does_not_touch_the_posts_update_at() {
    let store = store();
    store.upsert_posts(&[post("p1", "c1", 1000, 1000)]).unwrap();
    store.add_reaction("p1", "amy", "tada", 5000).unwrap();
    assert_eq!(store.post("p1").unwrap().unwrap().update_at, 1000);
}

/// A reaction can arrive for a post this client has never held.
#[test]
fn a_reaction_to_an_unknown_post_is_not_an_error() {
    let store = store();
    assert!(!store.add_reaction("missing", "amy", "tada", 1).unwrap());
}

// ---- custom emoji ----------------------------------------------------------

/// The negative answer matters as much as the positive one: without it every
/// `:tada:` in every channel would ask the server again on every render.
#[test]
fn emoji_answers_are_remembered_including_the_negative_ones() {
    let store = store();
    store.remember_emoji("bongo", "abc123").unwrap();
    store.remember_emoji("tada", "").unwrap();

    let known = store
        .known_emoji(&["bongo".into(), "tada".into(), "never_asked".into()])
        .unwrap();
    assert_eq!(known.get("bongo").map(String::as_str), Some("abc123"));
    assert_eq!(
        known.get("tada").map(String::as_str),
        Some(""),
        "asked, and it is standard"
    );
    assert!(!known.contains_key("never_asked"), "so the caller asks");
}

#[test]
fn an_emoji_that_becomes_custom_is_updated_rather_than_duplicated() {
    let store = store();
    store.remember_emoji("later", "").unwrap();
    store.remember_emoji("later", "id9").unwrap();
    let known = store.known_emoji(&["later".into()]).unwrap();
    assert_eq!(known.len(), 1);
    assert_eq!(known.get("later").map(String::as_str), Some("id9"));
}

/// These run the completion queries against a real database on purpose.
///
/// They exist because the first version of them compiled, passed clippy, passed
/// every other test, and failed at runtime with `ESCAPE expression must be a
/// single character`: `'\'` inside a Rust string is an escaped *quote*, so the
/// SQL read `ESCAPE ''`. Nothing but executing the statement could have caught
/// it -- so now something does.
mod completion {
    use super::*;

    fn person(id: &str, username: &str, first: &str, last: &str) -> matterless_core::User {
        matterless_core::User {
            id: id.into(),
            username: username.into(),
            first_name: first.into(),
            last_name: last.into(),
            nickname: String::new(),
            email: format!("{username}@example.test"),
            last_picture_update: 0,
            notify_props: Default::default(),
            roles: String::new(),
        }
    }

    fn room(id: &str, name: &str, display: &str, last_post_at: Timestamp) -> Channel {
        Channel {
            id: id.into(),
            team_id: "t1".into(),
            channel_type: "O".into(),
            name: name.into(),
            display_name: display.into(),
            total_msg_count: 0,
            total_msg_count_root: 0,
            last_post_at,
            delete_at: 0,
        }
    }

    #[test]
    fn people_are_found_by_a_scattered_query() {
        let store = store();
        store
            .upsert_users(&[
                person("u1", "gaetan.deturche", "Gaetan", "Deturche"),
                person("u2", "guillaume.dupont", "Guillaume", "Dupont"),
                person("u3", "arthur.trouslard", "Arthur", "Trouslard"),
            ])
            .expect("users");

        let initials = store.users_matching("gdt", 8).expect("query runs");
        assert_eq!(
            initials.first().map(|user| user.username.as_str()),
            Some("gaetan.deturche"),
            "got {:?}",
            initials.iter().map(|u| &u.username).collect::<Vec<_>>()
        );

        // A real prefix still lands on the obvious answer.
        let prefix = store.users_matching("arth", 8).expect("query runs");
        assert_eq!(
            prefix.first().map(|user| user.username.as_str()),
            Some("arthur.trouslard")
        );

        // A real name, not just the username.
        let by_name = store.users_matching("dupont", 8).expect("query runs");
        assert_eq!(
            by_name.first().map(|user| user.username.as_str()),
            Some("guillaume.dupont")
        );

        assert!(
            store
                .users_matching("zzz", 8)
                .expect("query runs")
                .is_empty()
        );
        // Nothing typed: `@` alone offers people rather than an empty list.
        assert_eq!(store.users_matching("", 8).expect("query runs").len(), 3);
    }

    #[test]
    fn an_underscore_is_a_character_not_a_wildcard() {
        // The escape clause exists for this: without it `_` matches any single
        // character and the list stops meaning anything.
        let store = store();
        store
            .upsert_users(&[person("u1", "a_b", "A", "B"), person("u2", "axb", "A", "X")])
            .expect("users");

        let found = store.users_matching("a_b", 8).expect("query runs");
        assert_eq!(
            found.len(),
            1,
            "got {:?}",
            found.iter().map(|u| &u.username).collect::<Vec<_>>()
        );
        assert_eq!(found[0].username, "a_b");
    }

    #[test]
    fn channels_rank_word_starts_then_recency() {
        let store = store();
        store
            .upsert_channels(&[
                room("c1", "material-review", "Design | Material Review", 100),
                room("c2", "moto", "Moto", 900),
                room("c3", "off-topic", "Off-Topic", 500),
            ])
            .expect("channels");

        let scattered = store.channels_matching("mtrv", 8).expect("query runs");
        assert_eq!(
            scattered.first().map(|channel| channel.id.as_str()),
            Some("c1"),
            "got {:?}",
            scattered
                .iter()
                .map(|c| &c.display_name)
                .collect::<Vec<_>>()
        );

        let by_slug = store.channels_matching("offtop", 8).expect("query runs");
        assert_eq!(
            by_slug.first().map(|channel| channel.id.as_str()),
            Some("c3")
        );

        // Equally good matches are separated by which one is alive.
        let everything = store.channels_matching("o", 8).expect("query runs");
        assert!(!everything.is_empty());
    }

    #[test]
    fn custom_emoji_are_found_the_same_way() {
        let store = store();
        store.remember_emoji("catsus", "e1").expect("emoji");
        store
            .remember_emoji("albertpiciousfront", "e2")
            .expect("emoji");
        // An empty id means "asked, it is standard", and must never be offered
        // as a custom emoji.
        store.remember_emoji("tada", "").expect("emoji");

        let found = store.custom_emoji_matching("cts", 8).expect("query runs");
        assert_eq!(found.first().map(|(name, _)| name.as_str()), Some("catsus"));

        let standard = store.custom_emoji_matching("tada", 8).expect("query runs");
        assert!(standard.is_empty(), "a standard name is not a custom emoji");
    }
}

#[test]
fn a_local_search_honours_from_in_and_dates() {
    use matterless_core::search::SearchQuery;

    let store = store();
    store
        .upsert_users(&[
            matterless_core::User {
                id: "amy".into(),
                username: "amy.smith".into(),
                first_name: "Amy".into(),
                last_name: "Smith".into(),
                nickname: String::new(),
                email: "amy@example.test".into(),
                last_picture_update: 0,
                notify_props: Default::default(),
                roles: String::new(),
            },
            matterless_core::User {
                id: "bob".into(),
                username: "bob.jones".into(),
                first_name: "Bob".into(),
                last_name: "Jones".into(),
                nickname: String::new(),
                email: "bob@example.test".into(),
                last_picture_update: 0,
                notify_props: Default::default(),
                roles: String::new(),
            },
        ])
        .expect("users");
    store
        .upsert_channels(&[Channel {
            id: "c1".into(),
            team_id: "t1".into(),
            channel_type: "O".into(),
            name: "town-square".into(),
            display_name: "Town Square".into(),
            total_msg_count: 0,
            total_msg_count_root: 0,
            last_post_at: 0,
            delete_at: 0,
        }])
        .expect("channel");

    // Two authors, two days, one word in common.
    let day = 86_400_000i64;
    let mut first = post(
        "p1",
        "c1",
        20_700 * day + 3_600_000,
        20_700 * day + 3_600_000,
    );
    first.user_id = "amy".into();
    first.message = "the budget looks wrong".into();
    let mut second = post(
        "p2",
        "c1",
        20_701 * day + 3_600_000,
        20_701 * day + 3_600_000,
    );
    second.user_id = "bob".into();
    second.message = "the budget is fine".into();
    store.upsert_posts(&[first, second]).expect("posts");

    let found = |typed: &str| {
        store
            .search_query(&SearchQuery::parse(typed, 0), 20)
            .expect("query runs")
            .into_iter()
            .map(|post| post.id)
            .collect::<Vec<_>>()
    };

    assert_eq!(found("budget"), vec!["p2", "p1"], "newest first");
    assert_eq!(
        found("from:amy.smith budget"),
        vec!["p1"],
        "the author filters"
    );
    // A modifier alone is a search: no FTS match, just a filter.
    assert_eq!(found("from:bob.jones"), vec!["p2"]);
    assert_eq!(found("in:town-square budget").len(), 2, "by slug");
    assert_eq!(
        found("in:\"Town Square\" budget").len(),
        0,
        "quotes are not stripped here"
    );
    assert_eq!(found("on:2026-09-04 budget"), vec!["p1"], "a single day");
    assert_eq!(
        found("after:2026-09-04 budget"),
        vec!["p2"],
        "exclusive of the day named"
    );
    assert_eq!(found("before:2026-09-05 budget"), vec!["p1"]);
    assert_eq!(
        found("from:amy.smith in:town-square on:2026-09-04"),
        vec!["p1"],
        "all three"
    );

    // Something unknown finds nothing rather than everything, which is the
    // difference between an empty result and a silently ignored filter.
    assert!(found("from:nobody budget").is_empty());
    assert!(found("in:nowhere budget").is_empty());

    // And the FTS syntax hazard is still handled: this must run, not throw.
    assert!(found("say \"hello\" -Wall").is_empty());
}
