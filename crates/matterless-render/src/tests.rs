use super::*;
use matterless_core::model::{PostMetadata, Reaction};

const DAY: Timestamp = 86_400_000;

fn post(id: &str, author: &str, create_at: Timestamp) -> Post {
    Post {
        id: id.into(),
        channel_id: "c1".into(),
        user_id: author.into(),
        root_id: String::new(),
        create_at,
        update_at: create_at,
        edit_at: 0,
        delete_at: 0,
        message: format!("body of {id}"),
        post_type: String::new(),
        file_ids: Vec::new(),
        props: serde_json::Value::Null,
        metadata: PostMetadata::default(),
        pending_post_id: String::new(),
        is_pinned: false,
    }
}

fn options(mode: ThreadMode) -> PlanOptions {
    PlanOptions::new(mode, "me")
}

fn kinds(rows: &[Row]) -> Vec<&'static str> {
    rows.iter()
        .map(|row| match row {
            Row::DateSeparator { .. } => "sep",
            Row::UnreadDivider => "unread",
            Row::Post { .. } => "post",
            Row::Continuation { .. } => "cont",
            Row::System { .. } => "system",
            Row::DeletedRoot { .. } => "deleted_root",
            Row::ThreadFooter { .. } => "footer",
        })
        .collect()
}

/// Input arrives newest-first, as the store returns it; rows come back in
/// reading order. Getting this backwards would silently invert every channel.
#[test]
fn newest_first_input_becomes_reading_order_rows() {
    let posts = vec![
        post("third", "amy", 3_000),
        post("second", "bob", 2_000),
        post("first", "amy", 1_000),
    ];
    let rows = plan_channel(&posts, &HashMap::new(), &options(ThreadMode::Flat));
    let ids: Vec<&str> = rows
        .iter()
        .filter_map(|row| match row {
            Row::Post { post } | Row::Continuation { post } => Some(post.post_id.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(ids, vec!["first", "second", "third"]);
}

#[test]
fn consecutive_posts_by_one_author_collapse_but_a_gap_breaks_the_run() {
    let posts = vec![
        post("a1", "amy", 0),
        post("a2", "amy", 60_000), // 1 min later
        post("a3", "amy", 60_000 + DEFAULT_COLLAPSE_WINDOW_MS + 1), // past the window
        post("b1", "bob", 60_000 + DEFAULT_COLLAPSE_WINDOW_MS + 2),
    ];
    let rows = plan_channel(&posts, &HashMap::new(), &options(ThreadMode::Flat));
    assert_eq!(
        kinds(&rows),
        vec!["sep", "post", "cont", "post", "post"],
        "a2 continues a1; a3 is too late; b1 is a different author"
    );
}

#[test]
fn a_date_separator_appears_per_local_day_and_breaks_the_run() {
    let mut later = post("a2", "amy", DAY + 1_000);
    later.message = "next day".into();
    let posts = vec![later, post("a1", "amy", 1_000)];

    let rows = plan_channel(&posts, &HashMap::new(), &options(ThreadMode::Flat));
    assert_eq!(
        kinds(&rows),
        vec!["sep", "post", "sep", "post"],
        "a new day must not be a continuation even from the same author"
    );
}

#[test]
fn the_viewer_offset_decides_where_a_day_starts() {
    // 23:30 UTC: the same instant is "tomorrow" for a viewer at +60 minutes.
    let late = post("late", "amy", 23 * 3_600_000 + 30 * 60_000);
    let posts = vec![late];

    let mut utc = options(ThreadMode::Flat);
    utc.utc_offset_minutes = 0;
    let mut ahead = options(ThreadMode::Flat);
    ahead.utc_offset_minutes = 60;

    let day_utc = match &plan_channel(&posts, &HashMap::new(), &utc)[0] {
        Row::DateSeparator { epoch_day } => *epoch_day,
        other => panic!("expected a separator, got {other:?}"),
    };
    let day_ahead = match &plan_channel(&posts, &HashMap::new(), &ahead)[0] {
        Row::DateSeparator { epoch_day } => *epoch_day,
        other => panic!("expected a separator, got {other:?}"),
    };
    assert_eq!(day_ahead, day_utc + 1);
}

#[test]
fn the_unread_divider_lands_before_the_first_unread_post() {
    let posts = vec![post("new", "bob", 3_000), post("old", "amy", 1_000)];
    let mut plan = options(ThreadMode::Flat);
    plan.last_viewed_at = 2_000;

    let rows = plan_channel(&posts, &HashMap::new(), &plan);
    assert_eq!(kinds(&rows), vec!["sep", "post", "unread", "post"]);
}

#[test]
fn no_divider_without_a_last_viewed_time() {
    let posts = vec![post("only", "amy", 1_000)];
    let rows = plan_channel(&posts, &HashMap::new(), &options(ThreadMode::Flat));
    assert!(!kinds(&rows).contains(&"unread"));
}

/// The structural consequence of collapsed threads, and the reason a flat list
/// could not be refined into this later.
#[test]
fn collapsed_threads_keep_replies_out_of_the_stream() {
    let mut reply = post("reply", "bob", 2_000);
    reply.root_id = "root".into();
    let posts = vec![reply, post("root", "amy", 1_000)];

    let mut threads = HashMap::new();
    threads.insert(
        "root".to_string(),
        ThreadSummary {
            reply_count: 1,
            last_reply_at: 2_000,
            participants: vec!["amy".into(), "bob".into()],
            ..Default::default()
        },
    );

    let collapsed = plan_channel(&posts, &threads, &options(ThreadMode::Collapsed));
    assert_eq!(
        kinds(&collapsed),
        vec!["sep", "post", "footer"],
        "the reply folds into a footer under its root"
    );
    match &collapsed[2] {
        Row::ThreadFooter {
            root_id,
            reply_count,
            participants,
            ..
        } => {
            assert_eq!(root_id, "root");
            assert_eq!(*reply_count, 1);
            assert_eq!(participants.len(), 2);
        }
        other => panic!("expected a footer, got {other:?}"),
    }

    // Flat mode puts the same reply back in the stream and grows no footer.
    let flat = plan_channel(&posts, &threads, &options(ThreadMode::Flat));
    assert_eq!(kinds(&flat), vec!["sep", "post", "post"]);
}

#[test]
fn a_thread_pane_is_always_flat_even_when_the_channel_collapses() {
    let root = post("root", "amy", 1_000);
    let mut reply = post("reply", "bob", 2_000);
    reply.root_id = "root".into();

    let rows = plan_thread(&root, &[reply], &options(ThreadMode::Collapsed));
    assert_eq!(
        kinds(&rows),
        vec!["sep", "post", "post"],
        "the pane must show the reply it exists to show"
    );
}

#[test]
fn a_deleted_post_vanishes_but_a_deleted_root_holding_replies_stays() {
    let mut orphan = post("orphan", "amy", 1_000);
    orphan.delete_at = 5_000;
    let mut root = post("root", "amy", 2_000);
    root.delete_at = 5_000;

    let mut threads = HashMap::new();
    threads.insert(
        "root".to_string(),
        ThreadSummary {
            reply_count: 3,
            last_reply_at: 4_000,
            participants: vec!["amy".into()],
            ..Default::default()
        },
    );

    let rows = plan_channel(&[orphan, root], &threads, &options(ThreadMode::Collapsed));
    assert_eq!(
        kinds(&rows),
        vec!["sep", "deleted_root"],
        "the orphan goes, the anchor stays"
    );
}

#[test]
fn system_messages_get_their_own_row_and_never_continue() {
    let mut joined = post("sys", "amy", 2_000);
    joined.post_type = "system_join_channel".into();
    let posts = vec![
        post("after", "amy", 3_000),
        joined,
        post("before", "amy", 1_000),
    ];

    let rows = plan_channel(&posts, &HashMap::new(), &options(ThreadMode::Flat));
    assert_eq!(
        kinds(&rows),
        vec!["sep", "post", "system", "post"],
        "the post after a system row starts a fresh header"
    );
}

#[test]
fn reactions_are_grouped_with_the_viewers_own_marked() {
    let mut post = post("p1", "amy", 1_000);
    post.metadata.reactions = vec![
        Reaction {
            user_id: "bob".into(),
            post_id: "p1".into(),
            emoji_name: "+1".into(),
            create_at: 1,
        },
        Reaction {
            user_id: "me".into(),
            post_id: "p1".into(),
            emoji_name: "+1".into(),
            create_at: 2,
        },
        Reaction {
            user_id: "bob".into(),
            post_id: "p1".into(),
            emoji_name: "eyes".into(),
            create_at: 3,
        },
    ];

    let mut named = options(ThreadMode::Flat);
    named.author_names.insert("bob".into(), "bob.smith".into());

    let rows = plan_channel(&[post], &HashMap::new(), &named);
    let Row::Post { post: row } = &rows[1] else {
        panic!("expected a post row");
    };
    assert_eq!(
        row.reactions,
        vec![
            ReactionSummary {
                emoji: "+1".into(),
                count: 2,
                mine: true,
                unicode: Some("\u{1F44D}".into()),
                // In the order they reacted, and the viewer is "You" rather
                // than their own name -- which is what the tooltip shows.
                names: vec!["bob.smith".into(), "You".into()],
            },
            ReactionSummary {
                emoji: "eyes".into(),
                count: 1,
                mine: false,
                unicode: Some("\u{1F440}".into()),
                names: vec!["bob.smith".into()],
            },
        ],
        "grouped in first-seen order, with the viewer's own flagged"
    );
}

#[test]
fn a_reaction_names_every_reactor() {
    // A busy post carries dozens, and the tooltip names all of them: the
    // question it answers is "who", which a sample does not.
    let mut post = post("p1", "amy", 1_000);
    post.metadata.reactions = (0..14)
        .map(|index| Reaction {
            user_id: format!("u{index}"),
            post_id: "p1".into(),
            emoji_name: "tada".into(),
            create_at: index,
        })
        .collect();

    let rows = plan_channel(&[post], &HashMap::new(), &options(ThreadMode::Flat));
    let Row::Post { post: row } = &rows[1] else {
        panic!("expected a post row");
    };
    assert_eq!(row.reactions[0].count, 14);
    assert_eq!(row.reactions[0].names.len(), 14, "all of them, in order");
    // An unhydrated reactor shows as an id rather than as nothing.
    assert_eq!(row.reactions[0].names[0], "u0");
    assert_eq!(row.reactions[0].names[13], "u13");
}

/// A CI post with an empty body and everything in `props.attachments` -- the
/// case that renders as a blank message if it is ignored.
#[test]
fn a_webhook_post_with_no_message_still_has_a_body() {
    let mut webhook = post("build", "ci-bot", 1_000);
    webhook.message = String::new();
    webhook.props = serde_json::json!({
        "from_webhook": "true",
        "attachments": [{
            "color": "#ff0000",
            "title": "Build 4821 failed",
            "title_link": "https://horde.example/4821",
            "text": "3 tests failed in `NaniteShading`",
            "fields": [{ "title": "Branch", "value": "main", "short": true }]
        }]
    });

    let rows = plan_channel(&[webhook], &HashMap::new(), &options(ThreadMode::Flat));
    let Row::Post { post: row } = &rows[1] else {
        panic!("expected a post row");
    };
    assert!(row.nodes.is_empty(), "the message really is empty");
    assert!(
        row.body_is_attachment_only,
        "so the shell must be told the body lives in the attachment"
    );
    assert_eq!(row.attachments.len(), 1);
    let attachment = &row.attachments[0];
    assert_eq!(attachment.title.as_deref(), Some("Build 4821 failed"));
    assert_eq!(attachment.color.as_deref(), Some("#ff0000"));
    assert!(
        !attachment.text.is_empty(),
        "attachment text is parsed as markdown too"
    );
    assert_eq!(attachment.fields[0].title, "Branch");
    assert!(attachment.fields[0].short);
}

#[test]
fn an_edit_changes_the_cache_key_and_is_flagged() {
    let mut edited = post("p1", "amy", 1_000);
    edited.update_at = 9_000;
    edited.edit_at = 9_000;

    let rows = plan_channel(&[edited], &HashMap::new(), &options(ThreadMode::Flat));
    let Row::Post { post: row } = &rows[1] else {
        panic!("expected a post row");
    };
    assert!(row.edited);
    assert_eq!(row.create_at, 1_000);
    assert_eq!(row.update_at, 9_000, "the render cache keys on this");
}

#[test]
fn an_empty_channel_plans_to_nothing() {
    assert!(plan_channel(&[], &HashMap::new(), &options(ThreadMode::Flat)).is_empty());
}

/// The plan crosses the IPC boundary, so its shape is part of the contract.
#[test]
fn rows_serialise_with_a_kind_tag_the_shell_can_switch_on() {
    let rows = plan_channel(
        &[post("p1", "amy", 1_000)],
        &HashMap::new(),
        &options(ThreadMode::Flat),
    );
    let json = serde_json::to_value(&rows).unwrap();
    let array = json.as_array().unwrap();
    assert_eq!(array[0]["kind"], "date_separator");
    assert_eq!(array[1]["kind"], "post");
    assert_eq!(array[1]["post"]["post_id"], "p1");
    // Nodes carry their own discriminator.
    assert_eq!(array[1]["post"]["nodes"][0]["t"], "paragraph");
}

/// The hazard the payload-first ordering exists to avoid: a separator emitted
/// above a post that turns out to contribute no row at all.
#[test]
fn a_fully_deleted_post_leaves_no_dangling_separator() {
    let mut orphan = post("orphan", "amy", 1_000);
    orphan.delete_at = 5_000;
    let rows = plan_channel(&[orphan], &HashMap::new(), &options(ThreadMode::Flat));
    assert!(
        rows.is_empty(),
        "no visible row means no separator either, got {:?}",
        kinds(&rows)
    );
}

/// `~channel` collides with GFM strikethrough tokenisation, so the scanner has
/// to see across text-run boundaries. Both must work in one message.
#[test]
fn a_channel_link_and_a_strikethrough_coexist() {
    let nodes = match markdown::parse("see ~build-alerts not ~~old-channel~~")
        .into_iter()
        .next()
    {
        Some(markdown::Node::Paragraph { children }) => children,
        other => panic!("expected a paragraph, got {other:?}"),
    };
    assert!(
        nodes.iter().any(|node| matches!(
            node,
            markdown::Node::ChannelLink { name } if name == "build-alerts"
        )),
        "the single-tilde link must survive: {nodes:?}"
    );
    assert!(
        nodes
            .iter()
            .any(|node| matches!(node, markdown::Node::Strike { .. })),
        "and the double-tilde strike must still parse: {nodes:?}"
    );
}

#[test]
fn an_author_resolves_to_a_name_and_falls_back_to_the_id() {
    let mut plan = options(ThreadMode::Flat);
    plan.author_names
        .insert("amy".to_string(), "amy.jones".to_string());

    let rows = plan_channel(
        &[post("known", "amy", 2_000), post("stranger", "zed", 1_000)],
        &HashMap::new(),
        &plan,
    );
    let names: Vec<&str> = rows
        .iter()
        .filter_map(|row| match row {
            Row::Post { post } | Row::Continuation { post } => Some(post.author_name.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(
        names,
        vec!["zed", "amy.jones"],
        "an unknown author shows as its id so the gap is visible, not blank"
    );
}

#[test]
fn the_display_preference_decides_which_name_is_shown() {
    let user = matterless_core::User {
        id: "u1".into(),
        username: "amy.jones".into(),
        first_name: "Amy".into(),
        last_name: "Jones".into(),
        nickname: "Ace".into(),
        email: String::new(),
        last_picture_update: 0,
        notify_props: std::collections::HashMap::new(),
        roles: String::new(),
    };
    assert_eq!(display_name(&user, "username"), "amy.jones");
    assert_eq!(display_name(&user, "full_name"), "Amy Jones");
    assert_eq!(display_name(&user, "nickname_full_name"), "Ace");
    // Anything unrecognised behaves as the measured default.
    assert_eq!(display_name(&user, "something_new"), "amy.jones");
}

/// An optimistic send is a row like any other, marked so the shell can dim it.
/// It reaches the plan from memory, never from SQLite -- that is what keeps a
/// wrong guess from being persisted while still leaving one render path.
#[test]
fn a_pending_send_appears_in_the_plan_and_is_marked() {
    let mut plan = options(ThreadMode::Flat);
    plan.pending.insert("local-1".to_string());

    let mut optimistic = post("local-1", "me", 3_000);
    optimistic.pending_post_id = "local-1".into();

    let rows = plan_channel(
        &[post("real", "amy", 1_000), optimistic],
        &HashMap::new(),
        &plan,
    );
    let flags: Vec<(&str, bool, bool)> = rows
        .iter()
        .filter_map(|row| match row {
            Row::Post { post } | Row::Continuation { post } => {
                Some((post.post_id.as_str(), post.pending, post.failed))
            }
            _ => None,
        })
        .collect();
    assert_eq!(
        flags,
        vec![("real", false, false), ("local-1", true, false)],
        "the pending row sorts last and only it is flagged"
    );
}

#[test]
fn a_failed_send_stays_visible_rather_than_vanishing() {
    let mut plan = options(ThreadMode::Flat);
    plan.pending.insert("local-1".to_string());
    plan.failed.insert("local-1".to_string());

    let rows = plan_channel(&[post("local-1", "me", 3_000)], &HashMap::new(), &plan);
    let Row::Post { post: row } = &rows[1] else {
        panic!("expected a post row");
    };
    assert!(row.pending && row.failed, "so the shell can offer a retry");
}

/// A supplied parse is used verbatim; anything absent is parsed on the spot.
/// This is what lets the caller cache by `post.id + update_at` without the
/// planner knowing a cache exists.
#[test]
fn supplied_parses_are_used_and_gaps_are_filled() {
    let mut plan = options(ThreadMode::Flat);
    plan.parsed.insert(
        "cached".to_string(),
        std::sync::Arc::new(vec![markdown::Node::Text {
            value: "from the cache".into(),
        }]),
    );

    let rows = plan_channel(
        &[post("cached", "amy", 1_000), post("fresh", "amy", 2_000)],
        &HashMap::new(),
        &plan,
    );
    let bodies: Vec<(&str, &[markdown::Node])> = rows
        .iter()
        .filter_map(|row| match row {
            Row::Post { post } | Row::Continuation { post } => {
                Some((post.post_id.as_str(), post.nodes.as_slice()))
            }
            _ => None,
        })
        .collect();

    assert_eq!(
        bodies[0].1,
        &[markdown::Node::Text {
            value: "from the cache".into()
        }],
        "the supplied parse is used as given, not re-derived"
    );
    // The uncached one really was parsed: its body is a paragraph, not raw text.
    assert!(matches!(
        bodies[1].1.first(),
        Some(markdown::Node::Paragraph { .. })
    ));
}

/// A node tree must survive the round trip through the cache.
#[test]
fn a_node_tree_round_trips_through_json() {
    let original = markdown::parse(
        "hey @ada see ~build-alerts :shipit: `code` **bold** https://example.com/x",
    );
    let json = serde_json::to_string(&original).unwrap();
    let restored: Vec<markdown::Node> = serde_json::from_str(&json).unwrap();
    assert_eq!(
        restored, original,
        "a cached render must decode identically"
    );
}

// ---- page seams -----------------------------------------------------------
//
// A paged plan builds each page on its own, so without context the first post
// of a page looks like the start of a fresh run on a fresh day.

const MINUTE: Timestamp = 60_000;

fn preceding(author: &str, create_at: Timestamp) -> Preceding {
    Preceding {
        user_id: author.into(),
        create_at,
    }
}

#[test]
fn a_run_of_messages_continues_across_a_page_seam() {
    let posts = vec![post("second", "amy", 10 * MINUTE)];

    let bare = plan_channel(&posts, &HashMap::new(), &options(ThreadMode::Flat));
    assert_eq!(
        kinds(&bare),
        vec!["sep", "post"],
        "no context, so a fresh run"
    );

    let mut with_context = options(ThreadMode::Flat);
    with_context.preceding = Some(preceding("amy", 10 * MINUTE - 30_000));
    let rows = plan_channel(&posts, &HashMap::new(), &with_context);
    assert_eq!(
        kinds(&rows),
        vec!["cont"],
        "same author 30s earlier on the page above: no header, no separator"
    );
}

#[test]
fn a_different_author_across_a_seam_still_starts_a_row() {
    let posts = vec![post("second", "bob", 10 * MINUTE)];
    let mut options = options(ThreadMode::Flat);
    options.preceding = Some(preceding("amy", 10 * MINUTE - 30_000));
    assert_eq!(
        kinds(&plan_channel(&posts, &HashMap::new(), &options)),
        vec!["post"]
    );
}

#[test]
fn a_new_day_across_a_seam_still_gets_its_separator() {
    let posts = vec![post("next", "amy", DAY + 3_600_000)];
    let mut options = options(ThreadMode::Flat);
    options.preceding = Some(preceding("amy", 3_600_000));
    assert_eq!(
        kinds(&plan_channel(&posts, &HashMap::new(), &options)),
        vec!["sep", "post"],
        "a day boundary inside the seam separates, and breaks the run"
    );
}

/// The divider is one row for the whole channel, not one per page.
#[test]
fn the_unread_divider_is_not_repeated_on_every_page() {
    let posts = vec![post("newest", "amy", 30 * MINUTE)];
    let mut options = options(ThreadMode::Flat);
    options.last_viewed_at = 5 * MINUTE;

    // The page above already holds a post past the watermark, so it carries it.
    options.preceding = Some(preceding("bob", 20 * MINUTE));
    assert!(
        !plan_channel(&posts, &HashMap::new(), &options)
            .iter()
            .any(|row| matches!(row, Row::UnreadDivider)),
        "a second divider"
    );

    // A page whose predecessor is entirely read must show it.
    options.preceding = Some(preceding("bob", MINUTE));
    assert!(
        plan_channel(&posts, &HashMap::new(), &options)
            .iter()
            .any(|row| matches!(row, Row::UnreadDivider)),
        "the divider belongs on this page"
    );
}

// ---- thread footers --------------------------------------------------------

fn summary(reply_count: i64, unread_replies: i64, unread_mentions: i64) -> ThreadSummary {
    ThreadSummary {
        reply_count,
        last_reply_at: 5_000,
        participants: vec!["amy".into()],
        unread_replies,
        unread_mentions,
        following: true,
    }
}

#[test]
fn a_footer_carries_the_servers_unread_counts() {
    let posts = vec![post("root", "amy", 1_000)];
    let threads = HashMap::from([("root".to_string(), summary(7, 3, 1))]);
    let rows = plan_channel(&posts, &threads, &options(ThreadMode::Collapsed));

    let Some(Row::ThreadFooter {
        reply_count,
        unread_replies,
        unread_mentions,
        following,
        ..
    }) = rows
        .iter()
        .find(|row| matches!(row, Row::ThreadFooter { .. }))
    else {
        panic!("no footer: {rows:?}");
    };
    assert_eq!(*reply_count, 7);
    assert_eq!(*unread_replies, 3);
    assert_eq!(*unread_mentions, 1);
    assert!(*following);
}

/// The gap this row exists to close: a followed thread whose replies are not
/// held locally still has to say it has unread ones, or the channel shows a
/// truthful unread badge with nothing on screen to explain it.
#[test]
fn unread_replies_alone_are_enough_to_earn_a_footer() {
    let posts = vec![post("root", "amy", 1_000)];
    let threads = HashMap::from([("root".to_string(), summary(0, 2, 0))]);
    let rows = plan_channel(&posts, &threads, &options(ThreadMode::Collapsed));
    assert_eq!(kinds(&rows), vec!["sep", "post", "footer"]);
}

#[test]
fn a_thread_with_nothing_in_it_grows_no_footer() {
    let posts = vec![post("root", "amy", 1_000)];
    let threads = HashMap::from([("root".to_string(), summary(0, 0, 0))]);
    let rows = plan_channel(&posts, &threads, &options(ThreadMode::Collapsed));
    assert_eq!(kinds(&rows), vec!["sep", "post"]);
}

/// In flat mode the replies are in the stream already, so a footer would be a
/// second copy of them.
#[test]
fn flat_mode_still_has_no_footers_however_unread_the_thread_is() {
    let posts = vec![post("root", "amy", 1_000)];
    let threads = HashMap::from([("root".to_string(), summary(7, 3, 1))]);
    let rows = plan_channel(&posts, &threads, &options(ThreadMode::Flat));
    assert!(
        !rows
            .iter()
            .any(|row| matches!(row, Row::ThreadFooter { .. }))
    );
}

// ---- the thread pane -------------------------------------------------------

#[test]
fn a_thread_pane_is_the_root_then_its_replies_in_order() {
    let root = post("root", "amy", 1_000);
    let replies = vec![post("r1", "bob", 2_000), post("r2", "amy", 3_000)];
    let rows = plan_thread(&root, &replies, &options(ThreadMode::Collapsed));
    let ids: Vec<&str> = rows
        .iter()
        .filter_map(|row| match row {
            Row::Post { post } | Row::Continuation { post } => Some(post.post_id.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(ids, vec!["root", "r1", "r2"]);
    assert!(
        !rows
            .iter()
            .any(|row| matches!(row, Row::ThreadFooter { .. })),
        "a thread does not contain itself"
    );
}

/// The per-thread watermark is what Phase 3 lacked: with nothing to place a
/// divider against it was zeroed, so a thread with unread replies opened with
/// no indication of where they started.
#[test]
fn the_divider_marks_where_the_unread_replies_start() {
    let root = post("root", "amy", 1_000);
    let replies = vec![post("read", "bob", 2_000), post("fresh", "bob", 4_000)];
    let mut options = options(ThreadMode::Collapsed);
    options.last_viewed_at = 3_000;
    let rows = plan_thread(&root, &replies, &options);

    let divider = rows
        .iter()
        .position(|row| matches!(row, Row::UnreadDivider))
        .expect("a thread read up to 3000 has one unread reply");
    let fresh = rows
        .iter()
        .position(|row| match row {
            Row::Post { post } | Row::Continuation { post } => post.post_id == "fresh",
            _ => false,
        })
        .unwrap();
    assert!(
        divider < fresh,
        "the line sits above the first unread reply"
    );
}

#[test]
fn a_fully_read_thread_has_no_divider() {
    let root = post("root", "amy", 1_000);
    let replies = vec![post("r1", "bob", 2_000)];
    let mut options = options(ThreadMode::Collapsed);
    options.last_viewed_at = 9_000;
    let rows = plan_thread(&root, &replies, &options);
    assert!(!rows.iter().any(|row| matches!(row, Row::UnreadDivider)));
}

// ---- bots, webhooks and system messages ------------------------------------
//
// Shapes measured across 5101 posts on a live server:
// 243 bot posts, 14 webhooks, 257 posts with attachments, and 76 whose entire
// body is an attachment.

fn with_props(mut subject: Post, props: serde_json::Value) -> Post {
    subject.props = props;
    subject
}

fn post_row(rows: &[Row]) -> &PostRow {
    rows.iter()
        .find_map(|row| match row {
            Row::Post { post } | Row::Continuation { post } => Some(post),
            _ => None,
        })
        .expect("a post row")
}

fn system_text(rows: &[Row]) -> &str {
    rows.iter()
        .find_map(|row| match row {
            Row::System { text, .. } => Some(text.as_str()),
            _ => None,
        })
        .expect("a system row")
}

#[test]
fn a_webhook_post_is_named_by_the_webhook_not_its_owner() {
    let subject = with_props(
        post("hook", "u1", 1_000),
        serde_json::json!({ "from_webhook": "true", "webhook_display_name": "Build Bot" }),
    );
    let mut options = options(ThreadMode::Flat);
    options
        .author_names
        .insert("u1".to_string(), "some.person".to_string());
    let rows = plan_channel(&[subject], &HashMap::new(), &options);

    let row = post_row(&rows);
    assert_eq!(
        row.author_name, "Build Bot",
        "the owning user's name would credit a colleague with everything CI says"
    );
    assert!(row.bot);
}

/// The other name the API allows. Never seen on this server, but free to honour.
#[test]
fn override_username_is_honoured_too() {
    let subject = with_props(
        post("hook", "u1", 1_000),
        serde_json::json!({ "from_webhook": "true", "override_username": "deploy-bot" }),
    );
    let rows = plan_channel(&[subject], &HashMap::new(), &options(ThreadMode::Flat));
    assert_eq!(post_row(&rows).author_name, "deploy-bot");
}

#[test]
fn a_bot_post_keeps_its_users_name_but_is_marked() {
    let subject = with_props(
        post("botpost", "u1", 1_000),
        serde_json::json!({ "from_bot": "true" }),
    );
    let mut options = options(ThreadMode::Flat);
    options
        .author_names
        .insert("u1".to_string(), "sentry".to_string());
    let rows = plan_channel(&[subject], &HashMap::new(), &options);
    let row = post_row(&rows);
    assert_eq!(row.author_name, "sentry");
    assert!(row.bot, "so the shell can say it is not a person");
}

/// 76 attachments on this server populate `fallback` while leaving `text`
/// empty. Without it those posts render as nothing at all.
#[test]
fn an_attachment_with_only_a_fallback_still_has_a_body() {
    let subject = with_props(
        post("ci", "u1", 1_000),
        serde_json::json!({
            "attachments": [{ "color": "#ff0000", "fallback": "Build 42 failed" }]
        }),
    );
    let rows = plan_channel(&[subject], &HashMap::new(), &options(ThreadMode::Flat));
    let attachment = post_row(&rows).attachments.first().expect("one attachment");
    assert!(!attachment.text.is_empty(), "the fallback is the body");
    assert_eq!(attachment.color.as_deref(), Some("#ff0000"));
}

/// ...but it usually duplicates `pretext` word for word, so it is a last resort
/// rather than an addition.
#[test]
fn a_fallback_is_ignored_when_the_attachment_says_it_another_way() {
    let subject = with_props(
        post("ci", "u1", 1_000),
        serde_json::json!({
            "attachments": [{ "pretext": "Build 42 failed", "fallback": "Build 42 failed" }]
        }),
    );
    let rows = plan_channel(&[subject], &HashMap::new(), &options(ThreadMode::Flat));
    let attachment = post_row(&rows).attachments.first().expect("one attachment");
    assert!(!attachment.pretext.is_empty());
    assert!(attachment.text.is_empty(), "not the same words twice");
}

#[test]
fn a_post_that_is_only_an_attachment_says_so() {
    let mut subject = with_props(
        post("ci", "u1", 1_000),
        serde_json::json!({ "attachments": [{ "text": "Deploy finished" }] }),
    );
    subject.message = String::new();
    let rows = plan_channel(&[subject], &HashMap::new(), &options(ThreadMode::Flat));
    assert!(post_row(&rows).body_is_attachment_only);
}

#[test]
fn system_messages_read_as_sentences() {
    let cases = [
        (
            "system_join_channel",
            serde_json::json!({ "username": "amy" }),
            "amy joined the channel",
        ),
        (
            "system_leave_channel",
            serde_json::json!({ "username": "amy" }),
            "amy left the channel",
        ),
        (
            "system_add_to_channel",
            serde_json::json!({ "username": "amy", "addedUsername": "bob" }),
            "amy added bob to the channel",
        ),
        (
            "system_join_team",
            serde_json::json!({ "username": "amy" }),
            "amy joined the team",
        ),
        (
            "system_header_change",
            serde_json::json!({ "username": "amy", "new_header": "x" }),
            "amy updated the channel header",
        ),
        (
            "system_remove_from_team",
            serde_json::json!({ "removedUsername": "bob" }),
            "bob was removed from the team",
        ),
    ];
    for (post_type, props, expected) in cases {
        let mut subject = with_props(post("s1", "u1", 1_000), props);
        subject.post_type = post_type.to_string();
        subject.message = String::new();
        let rows = plan_channel(&[subject], &HashMap::new(), &options(ThreadMode::Flat));
        assert_eq!(system_text(&rows), expected, "for {post_type}");
    }
}

/// An unknown type must still read as English rather than as a slug: the shell
/// was printing the raw type with its underscores swapped for spaces.
#[test]
fn an_unknown_system_type_is_readable_rather_than_a_slug() {
    let mut subject = with_props(
        post("s1", "u1", 1_000),
        serde_json::json!({ "username": "amy" }),
    );
    subject.post_type = "system_something_new".to_string();
    subject.message = String::new();
    let rows = plan_channel(&[subject], &HashMap::new(), &options(ThreadMode::Flat));
    assert_eq!(system_text(&rows), "amy: something new");
}

/// A system post carrying its own message is quoting the server, which knows
/// better than any guess made here.
#[test]
fn a_system_post_with_a_message_uses_it() {
    let mut subject = post("s1", "u1", 1_000);
    subject.post_type = "system_unfamiliar".to_string();
    subject.message = "the server explained itself".to_string();
    let rows = plan_channel(&[subject], &HashMap::new(), &options(ThreadMode::Flat));
    assert_eq!(system_text(&rows), "the server explained itself");
}

#[test]
fn a_footer_carries_its_participants_with_names_and_avatar_versions() {
    let posts = vec![post("root", "amy", 1_000)];
    let threads = HashMap::from([(
        "root".to_string(),
        ThreadSummary {
            reply_count: 9,
            last_reply_at: 5_000,
            participants: vec!["amy".into(), "bob".into(), "cass".into(), "dee".into()],
            unread_replies: 0,
            unread_mentions: 0,
            following: true,
        },
    )]);
    let mut options = options(ThreadMode::Collapsed);
    options
        .author_names
        .insert("amy".into(), "amy.smith".into());
    options.author_avatars.insert("amy".into(), 4242);

    let rows = plan_channel(&posts, &threads, &options);
    let Some(Row::ThreadFooter { participants, .. }) = rows
        .iter()
        .find(|row| matches!(row, Row::ThreadFooter { .. }))
    else {
        panic!("no footer: {rows:?}");
    };
    assert_eq!(
        participants.len(),
        4,
        "all of them; the shell caps the faces"
    );
    assert_eq!(participants[0].name, "amy.smith", "resolved, not an id");
    assert_eq!(
        participants[0].avatar_at, 4242,
        "so a new picture is a new URL"
    );
    // Someone with no local user row still gets a face, keyed by id.
    assert_eq!(participants[1].name, "bob");
    assert_eq!(participants[1].avatar_at, 0);
}

/// Shapes taken from a live server: an image
/// arrives with dimensions, a preview flag and a mini preview, and a PDF arrives
/// with none of the three -- the fields are absent from its JSON rather than
/// null.
fn image_info(id: &str, width: i32, height: i32) -> matterless_core::model::FileInfo {
    matterless_core::model::FileInfo {
        id: id.into(),
        name: "image.png".into(),
        extension: "png".into(),
        size: 474_641,
        mime_type: "image/png".into(),
        width,
        height,
        has_preview_image: true,
        mini_preview: Some("/9j/tiny".into()),
        post_id: "p1".into(),
        archived: false,
    }
}

/// Everything drawn inline shares the room when there is more than one of it,
/// whatever it is made of.
///
/// A GIF and a video both fetch the *original* file -- a GIF's thumbnail is a
/// still frame, and a video has no thumbnail at all -- and the box used to be
/// picked from that choice, so both were drawn full size beside neighbours the
/// same layout had shrunk to thumbnails.
#[test]
fn a_gif_and_a_video_share_a_gallery_like_a_picture() {
    let gallery = FileLayout {
        gallery: true,
        full_res: false,
        allow_svg: false,
        pixel_ratio: 1.0,
    };
    let mut gif = image_info("f1", 800, 600);
    gif.mime_type = "image/gif".into();
    gif.extension = "gif".into();
    let mut film = image_info("f2", 800, 600);
    film.mime_type = "video/mp4".into();
    film.extension = "mp4".into();
    film.has_preview_image = false;

    let picture = FileRef::from_info(&image_info("f3", 800, 600), gallery);
    let animated = FileRef::from_info(&gif, gallery);
    let video = FileRef::from_info(&film, gallery);

    assert_eq!(
        animated.box_width, picture.box_width,
        "a gif shares the row rather than taking it"
    );
    assert_eq!(video.box_width, picture.box_width, "so does a video");
    // And the rendition each fetches is still its own business.
    assert_eq!(animated.variant, ImageVariant::Original);
    assert_eq!(picture.variant, ImageVariant::Thumb);
    assert!(video.video, "a playable container plays");
}

fn document_info(id: &str) -> matterless_core::model::FileInfo {
    matterless_core::model::FileInfo {
        id: id.into(),
        name: "rapport.pdf".into(),
        extension: "pdf".into(),
        size: 43_172,
        mime_type: "application/pdf".into(),
        width: 0,
        height: 0,
        has_preview_image: false,
        mini_preview: None,
        post_id: "p1".into(),
        archived: false,
    }
}

#[test]
fn an_image_attachment_carries_a_reserved_box() {
    let mut carrier = post("p1", "amy", 1_000);
    carrier.metadata.files = vec![image_info("f1", 1923, 794)];

    let rows = plan_channel(&[carrier], &HashMap::new(), &options(ThreadMode::Flat));
    let Some(Row::Post { post, .. }) = rows.iter().find(|row| matches!(row, Row::Post { .. }))
    else {
        panic!("no post row: {rows:?}");
    };
    let file = &post.files[0];
    assert!(file.image, "an image renders inline");
    assert_eq!(
        file.variant,
        ImageVariant::Preview,
        "a lone image is worth the server's larger rendition"
    );
    assert_eq!(file.mini_preview.as_deref(), Some("/9j/tiny"));
    // 1923x794 into 420x350: width-bound, so the height follows the aspect.
    assert_eq!((file.box_width, file.box_height), (420, 173));
}

#[test]
fn a_lone_image_can_be_drawn_from_the_file_as_uploaded() {
    let mut carrier = post("p1", "amy", 1_000);
    carrier.metadata.files = vec![image_info("f1", 1923, 794)];
    let mut wants_full = options(ThreadMode::Flat);
    wants_full.full_res = true;

    let rows = plan_channel(&[carrier], &HashMap::new(), &wants_full);
    let Some(Row::Post { post, .. }) = rows.iter().find(|row| matches!(row, Row::Post { .. }))
    else {
        panic!("no post row");
    };
    assert_eq!(post.files[0].variant, ImageVariant::Original);
    // The reader's choice changes which bytes are fetched, not the layout.
    assert_eq!(
        (post.files[0].box_width, post.files[0].box_height),
        (420, 173)
    );
}

#[test]
fn several_images_become_a_row_of_thumbnails_at_their_own_size() {
    let mut carrier = post("p1", "amy", 1_000);
    carrier.metadata.files = vec![
        image_info("f1", 1899, 1052),
        image_info("f2", 3072, 4096),
        image_info("f3", 52, 107),
    ];

    let rows = plan_channel(&[carrier], &HashMap::new(), &options(ThreadMode::Flat));
    let Some(Row::Post { post, .. }) = rows.iter().find(|row| matches!(row, Row::Post { .. }))
    else {
        panic!("no post row");
    };
    for file in &post.files {
        assert_eq!(file.variant, ImageVariant::Thumb, "{}", file.id);
    }
    // Exactly the sizes the server returns for these three, measured with
    // measurement: stretching any of them into a 420px box was a
    // 3.5x upscale.
    assert_eq!(
        (post.files[0].box_width, post.files[0].box_height),
        (120, 66)
    );
    assert_eq!(
        (post.files[1].box_width, post.files[1].box_height),
        (75, 100)
    );
    assert_eq!(
        (post.files[2].box_width, post.files[2].box_height),
        (49, 100)
    );
}

#[test]
fn a_thumbnail_box_shrinks_on_a_denser_display() {
    let mut carrier = post("p1", "amy", 1_000);
    carrier.metadata.files = vec![image_info("f1", 1899, 1052), image_info("f2", 1899, 1052)];
    let mut dense = options(ThreadMode::Flat);
    dense.pixel_ratio = 2.0;

    let rows = plan_channel(&[carrier], &HashMap::new(), &dense);
    let Some(Row::Post { post, .. }) = rows.iter().find(|row| matches!(row, Row::Post { .. }))
    else {
        panic!("no post row");
    };
    // 120x66 device pixels is 60x33 CSS pixels at ratio 2: smaller on purpose,
    // because covering 120 CSS pixels with 120 image pixels on this display
    // would be a 2x upscale.
    assert_eq!(
        (post.files[0].box_width, post.files[0].box_height),
        (60, 33)
    );
}

#[test]
fn a_document_attachment_is_a_card_with_no_box() {
    let mut carrier = post("p1", "amy", 1_000);
    carrier.metadata.files = vec![document_info("f2")];

    let rows = plan_channel(&[carrier], &HashMap::new(), &options(ThreadMode::Flat));
    let Some(Row::Post { post, .. }) = rows.iter().find(|row| matches!(row, Row::Post { .. }))
    else {
        panic!("no post row: {rows:?}");
    };
    let file = &post.files[0];
    assert!(!file.image, "a PDF is not drawn as a picture");
    assert_eq!((file.box_width, file.box_height), (0, 0));
    assert_eq!(
        file.size, 43_172,
        "the card shows the size, so it is carried"
    );
}

#[test]
fn an_archived_file_is_not_drawn_as_a_picture() {
    let mut info = image_info("f3", 600, 400);
    info.archived = true;
    let mut carrier = post("p1", "amy", 1_000);
    carrier.metadata.files = vec![info];

    let rows = plan_channel(&[carrier], &HashMap::new(), &options(ThreadMode::Flat));
    let Some(Row::Post { post, .. }) = rows.iter().find(|row| matches!(row, Row::Post { .. }))
    else {
        panic!("no post row: {rows:?}");
    };
    // The bytes are gone from storage, so asking for them would 404 forever.
    assert!(!post.files[0].image);
    assert!(post.files[0].archived);
}

#[test]
fn an_image_whose_stored_flag_was_lost_is_still_drawn_as_one() {
    // The signature of a post written before `has_preview_image` existed: the
    // store re-serialises metadata through these types, so the flag is simply
    // absent and reads as false.
    let mut lossy = image_info("f4", 1000, 500);
    lossy.has_preview_image = false;
    lossy.mini_preview = None;
    let mut carrier = post("p1", "amy", 1_000);
    carrier.metadata.files = vec![lossy];

    let rows = plan_channel(&[carrier], &HashMap::new(), &options(ThreadMode::Flat));
    let Some(Row::Post { post, .. }) = rows.iter().find(|row| matches!(row, Row::Post { .. }))
    else {
        panic!("no post row: {rows:?}");
    };
    assert!(post.files[0].image);
    assert_eq!(post.files[0].variant, ImageVariant::Preview);
}

#[test]
fn an_svg_is_a_file_card_unless_the_server_allows_it() {
    // `EnableSVGs` is false on this server, and the reason is not cosmetic: an
    // SVG is a document that can carry script and external references.
    let mut vector = image_info("f7", 300, 300);
    vector.mime_type = "image/svg+xml".into();
    let mut carrier = post("p1", "amy", 1_000);
    carrier.metadata.files = vec![vector];

    let rows = plan_channel(
        &[carrier.clone()],
        &HashMap::new(),
        &options(ThreadMode::Flat),
    );
    let Some(Row::Post { post, .. }) = rows.iter().find(|row| matches!(row, Row::Post { .. }))
    else {
        panic!("no post row");
    };
    assert!(!post.files[0].image, "drawn as a card, not inline");

    let mut permissive = options(ThreadMode::Flat);
    permissive.allow_svg = true;
    let rows = plan_channel(&[carrier], &HashMap::new(), &permissive);
    let Some(Row::Post { post, .. }) = rows.iter().find(|row| matches!(row, Row::Post { .. }))
    else {
        panic!("no post row");
    };
    assert!(
        post.files[0].image,
        "a server that allows them gets them inline"
    );
    // Still the file itself: an SVG has no thumbnail or preview rendition.
    assert_eq!(post.files[0].variant, ImageVariant::Original);
}

#[test]
fn an_animation_is_always_drawn_from_the_file_itself() {
    // A preview of a GIF is one still frame, so it never takes a thumbnail --
    // even in a gallery, where its neighbours do.
    let mut moving = image_info("f5", 400, 400);
    moving.mime_type = "image/gif".into();
    let mut carrier = post("p1", "amy", 1_000);
    carrier.metadata.files = vec![moving, image_info("f6", 400, 400)];

    let rows = plan_channel(&[carrier], &HashMap::new(), &options(ThreadMode::Flat));
    let Some(Row::Post { post, .. }) = rows.iter().find(|row| matches!(row, Row::Post { .. }))
    else {
        panic!("no post row");
    };
    assert!(post.files[0].image, "still drawn inline");
    assert_eq!(post.files[0].variant, ImageVariant::Original);
    assert_eq!(
        post.files[1].variant,
        ImageVariant::Thumb,
        "its neighbour still takes a thumbnail"
    );
}

#[test]
fn the_image_box_preserves_the_aspect_ratio() {
    const BOX: (i32, i32) = (420, 350);
    // Smaller than the box: left alone rather than blown up.
    assert_eq!(fit_box(120, 80, BOX), (120, 80));
    // Tall: height-bound.
    assert_eq!(fit_box(800, 2000, BOX), (140, 350));
    // Wide: width-bound.
    assert_eq!(fit_box(2000, 800, BOX), (420, 168));
    // Exactly the box.
    assert_eq!(fit_box(420, 350, BOX), (420, 350));
    // A degenerate size reserves nothing instead of dividing by zero.
    assert_eq!(fit_box(0, 0, BOX), (0, 0));
    // An extreme ratio still leaves a pixel to draw in.
    assert_eq!(fit_box(10_000, 1, BOX), (420, 1));
}

#[test]
fn a_page_preview_carries_what_the_card_shows() {
    // The shape captured from this server: dimensions arrive as *strings*.
    let mut carrier = post("p1", "amy", 1_000);
    carrier.metadata.embeds = vec![matterless_core::model::Embed {
        embed_type: "opengraph".into(),
        url: "https://github.com/example/repo".into(),
        data: serde_json::json!({
            "title": "example/repo",
            "description": "A repository",
            "site_name": "GitHub",
            "images": [{ "url": "https://img.example/card.png", "width": "1200", "height": "600" }],
        }),
    }];

    let rows = plan_channel(&[carrier], &HashMap::new(), &options(ThreadMode::Flat));
    let Some(Row::Post { post, .. }) = rows.iter().find(|row| matches!(row, Row::Post { .. }))
    else {
        panic!("no post row");
    };
    match &post.previews[0] {
        Preview::Page {
            title,
            site_name,
            image,
            ..
        } => {
            assert_eq!(title, "example/repo");
            assert_eq!(site_name, "GitHub");
            let image = image.as_ref().expect("an image");
            // 1200x600 into 400x220: width-bound, aspect kept.
            assert_eq!((image.width, image.height), (400, 200));
        }
        other => panic!("expected a page preview, got {other:?}"),
    }
}

#[test]
fn a_bare_link_embed_draws_nothing() {
    // `{type, url}` with no data: the server fetched no metadata, so a card
    // would just repeat the URL already in the message.
    let mut carrier = post("p1", "amy", 1_000);
    carrier.metadata.embeds = vec![matterless_core::model::Embed {
        embed_type: "link".into(),
        url: "https://jcgt.org/published/0015/02/01/".into(),
        data: serde_json::Value::Null,
    }];

    let rows = plan_channel(&[carrier], &HashMap::new(), &options(ThreadMode::Flat));
    let Some(Row::Post { post, .. }) = rows.iter().find(|row| matches!(row, Row::Post { .. }))
    else {
        panic!("no post row");
    };
    assert!(post.previews.is_empty());
}

#[test]
fn a_permalink_preview_quotes_the_message() {
    let mut quoted = post("q1", "bob", 500);
    quoted.message = "the **quoted** message".into();
    quoted.channel_id = "c9".into();

    let mut carrier = post("p1", "amy", 1_000);
    carrier.metadata.embeds = vec![matterless_core::model::Embed {
        embed_type: "permalink".into(),
        // Deliberately empty: a permalink embed carries no `url` at all.
        url: String::new(),
        data: serde_json::json!({
            "post_id": "q1",
            "channel_id": "c9",
            "channel_type": "D",
            "channel_display_name": "",
            "team_name": "",
            "post": serde_json::to_value(&quoted).expect("post as json"),
        }),
    }];
    let mut named = options(ThreadMode::Flat);
    named.author_names.insert("bob".into(), "bob.jones".into());

    let rows = plan_channel(&[carrier], &HashMap::new(), &named);
    let Some(Row::Post { post, .. }) = rows.iter().find(|row| matches!(row, Row::Post { .. }))
    else {
        panic!("no post row");
    };
    match &post.previews[0] {
        Preview::Permalink {
            author_name,
            channel_label,
            nodes,
            post_id,
            ..
        } => {
            assert_eq!(post_id, "q1");
            assert_eq!(author_name, "bob.jones", "resolved, not an id");
            // A direct message has no display name of its own, so it is
            // labelled by its kind rather than by its `id__id` slug.
            assert_eq!(channel_label, "Direct message");
            assert!(!nodes.is_empty(), "the quoted body is parsed");
        }
        other => panic!("expected a permalink preview, got {other:?}"),
    }
}

#[test]
fn a_deleted_message_is_not_quoted() {
    let mut quoted = post("q1", "bob", 500);
    quoted.delete_at = 900;
    let mut carrier = post("p1", "amy", 1_000);
    carrier.metadata.embeds = vec![matterless_core::model::Embed {
        embed_type: "permalink".into(),
        url: String::new(),
        data: serde_json::json!({ "post": serde_json::to_value(&quoted).expect("json") }),
    }];

    let rows = plan_channel(&[carrier], &HashMap::new(), &options(ThreadMode::Flat));
    let Some(Row::Post { post, .. }) = rows.iter().find(|row| matches!(row, Row::Post { .. }))
    else {
        panic!("no post row");
    };
    assert!(post.previews.is_empty(), "a tombstone is not a preview");
}
