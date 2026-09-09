use super::*;
use matterless_core::model::{Channel, ChannelMember, PostMetadata, Preference};
use matterless_core::ws::Event;

fn engine() -> SyncEngine {
    SyncEngine::new(Arc::new(Store::open_in_memory().unwrap()))
}

fn me() -> User {
    let mut notify_props = HashMap::new();
    notify_props.insert("desktop".into(), "all".into());
    User {
        id: "me".into(),
        username: "ada".into(),
        first_name: "Gaetan".into(),
        last_name: "Deturche".into(),
        nickname: String::new(),
        email: String::new(),
        last_picture_update: 0,
        notify_props,
        roles: String::new(),
    }
}

fn post(id: &str, create_at: Timestamp, root_id: &str) -> Post {
    Post {
        id: id.into(),
        channel_id: "c1".into(),
        user_id: "someone".into(),
        root_id: root_id.into(),
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

/// One ordered post plus the thread root it needs, exactly the shape the server
/// returns.
fn list_with_context() -> PostList {
    let mut posts = HashMap::new();
    posts.insert("reply".to_string(), post("reply", 200, "root"));
    posts.insert("root".to_string(), post("root", 100, ""));
    PostList {
        order: vec!["reply".to_string()],
        posts,
        next_post_id: String::new(),
        prev_post_id: "older".to_string(),
        has_next: false,
    }
}

#[test]
fn context_posts_are_stored_but_kept_out_of_the_stream() {
    let engine = engine();
    let context = SyncContext::new(me(), ThreadMode::Flat);
    let deltas = engine
        .apply_post_list(
            "c1",
            &list_with_context(),
            Arrival::Backfill,
            &context,
            1_000,
        )
        .unwrap();

    let upserts: Vec<(&String, bool)> = deltas
        .iter()
        .filter_map(|delta| match delta {
            Delta::PostUpserted {
                post_id, in_stream, ..
            } => Some((post_id, *in_stream)),
            _ => None,
        })
        .collect();
    assert_eq!(upserts.len(), 2, "both posts are written");
    assert!(
        upserts
            .iter()
            .any(|(id, in_stream)| *id == "reply" && *in_stream)
    );
    assert!(
        upserts
            .iter()
            .any(|(id, in_stream)| *id == "root" && !*in_stream),
        "the root is stored but must not be placed in the channel stream"
    );
    // Both are readable afterwards.
    assert!(engine.store().post("root").unwrap().is_some());
}

#[test]
fn a_backfill_never_notifies_however_hard_it_mentions_you() {
    let engine = engine();
    let context = SyncContext::new(me(), ThreadMode::Flat);

    let mut mentions_me = post("p1", 100, "");
    mentions_me.message = "@ada can you look".into();
    let mut posts = HashMap::new();
    posts.insert("p1".to_string(), mentions_me);
    let list = PostList {
        order: vec!["p1".to_string()],
        posts,
        next_post_id: String::new(),
        prev_post_id: String::new(),
        has_next: false,
    };

    for arrival in [Arrival::Backfill, Arrival::Resync] {
        let scratch = SyncEngine::new(Arc::new(Store::open_in_memory().unwrap()));
        let deltas = scratch
            .apply_post_list("c1", &list, arrival, &context, 1_000)
            .unwrap();
        let notified = deltas.iter().any(|delta| match delta {
            Delta::PostUpserted { notify, .. } => *notify,
            _ => false,
        });
        assert!(
            !notified,
            "{arrival:?} must never raise a toast -- this is what stops 200 \
             notifications after a laptop wakes"
        );
    }

    // The same post arriving live does notify.
    let deltas = engine
        .apply_post_list("c1", &list, Arrival::Live, &context, 1_000)
        .unwrap();
    assert!(deltas.iter().any(|delta| match delta {
        Delta::PostUpserted {
            notify, mention, ..
        } => *notify && *mention == MentionVerdict::Direct,
        _ => false,
    }));
}

#[test]
fn a_replayed_live_event_produces_no_delta() {
    let engine = engine();
    let context = SyncContext::new(me(), ThreadMode::Flat);
    let event = Event::Posted {
        post: Box::new(post("p1", 100, "")),
        channel_id: "c1".into(),
    };

    let first = engine.apply_event(&event, &context).unwrap();
    assert!(
        first
            .iter()
            .any(|delta| matches!(delta, Delta::PostUpserted { .. })),
        "the first delivery is a real delta"
    );

    let second = engine.apply_event(&event, &context).unwrap();
    assert!(
        !second
            .iter()
            .any(|delta| matches!(delta, Delta::PostUpserted { .. })),
        "a reconnect replaying the same post must be silent"
    );
}

#[test]
fn typing_and_status_never_touch_the_store() {
    let engine = engine();
    let context = SyncContext::new(me(), ThreadMode::Flat);

    let typing = Event::Typing {
        channel_id: "c1".into(),
        user_id: "someone".into(),
        root_id: "root9".into(),
    };
    let deltas = engine.apply_event(&typing, &context).unwrap();
    // The thread survives the mapping: it is what tells the shell whether to
    // show this under the channel or in the thread pane.
    match &deltas[0] {
        Delta::Typing { root_id, .. } => assert_eq!(root_id, "root9"),
        other => panic!("expected Typing, got {other:?}"),
    }

    let status = Event::StatusChange {
        user_id: "someone".into(),
        status: "away".into(),
    };
    let deltas = engine.apply_event(&status, &context).unwrap();
    assert!(matches!(deltas[0], Delta::StatusChanged { .. }));

    // Nothing was written; the posts table is still empty.
    let count: i64 = engine.store().channel_page("c1", None, 100).unwrap().len() as i64;
    assert_eq!(count, 0);
}

#[test]
fn an_edit_is_never_a_notification() {
    let engine = engine();
    let context = SyncContext::new(me(), ThreadMode::Flat);

    let mut original = post("p1", 100, "");
    original.message = "nothing to see".into();
    engine
        .apply_event(
            &Event::Posted {
                post: Box::new(original.clone()),
                channel_id: "c1".into(),
            },
            &context,
        )
        .unwrap();

    let mut edited = original;
    edited.message = "@ada now it mentions you".into();
    edited.update_at = 500;
    edited.edit_at = 500;
    let deltas = engine
        .apply_event(&Event::PostEdited(Box::new(edited)), &context)
        .unwrap();

    assert!(deltas.iter().any(|delta| matches!(
        delta,
        Delta::PostUpserted {
            change: PostChange::Updated,
            ..
        }
    )));
    assert!(
        !deltas.iter().any(|delta| match delta {
            Delta::PostUpserted { notify, .. } => *notify,
            _ => false,
        }),
        "editing a post to add a mention must not raise a toast"
    );
}

#[test]
fn deletion_emits_a_tombstone_delta() {
    let engine = engine();
    let context = SyncContext::new(me(), ThreadMode::Flat);
    engine
        .apply_event(
            &Event::Posted {
                post: Box::new(post("p1", 100, "")),
                channel_id: "c1".into(),
            },
            &context,
        )
        .unwrap();

    let mut deleted = post("p1", 100, "");
    deleted.update_at = 300;
    deleted.delete_at = 300;
    let deltas = engine
        .apply_event(&Event::PostDeleted(Box::new(deleted)), &context)
        .unwrap();
    assert!(matches!(deltas[0], Delta::PostTombstoned { .. }));
}

#[test]
fn sync_state_tracks_the_contiguous_range_and_the_beginning() {
    let engine = engine();
    let context = SyncContext::new(me(), ThreadMode::Flat);

    engine
        .apply_post_list(
            "c1",
            &list_with_context(),
            Arrival::Backfill,
            &context,
            7_000,
        )
        .unwrap();
    let state = engine.store().sync_state("c1").unwrap().unwrap();
    assert_eq!(state.synced_to, 200);
    assert_eq!(state.newest_post_id, "reply");
    assert!(
        !state.reached_beginning,
        "prev_post_id was set, so older history exists"
    );
    assert_eq!(engine.catch_up_cursor("c1").unwrap(), 200);

    // A page with no prev_post_id means we have reached the start of history.
    let mut posts = HashMap::new();
    posts.insert("first".to_string(), post("first", 10, ""));
    let oldest_page = PostList {
        order: vec!["first".to_string()],
        posts,
        next_post_id: String::new(),
        prev_post_id: String::new(),
        has_next: false,
    };
    engine
        .apply_post_list("c1", &oldest_page, Arrival::Backfill, &context, 8_000)
        .unwrap();
    let state = engine.store().sync_state("c1").unwrap().unwrap();
    assert_eq!(state.synced_from, 10, "the range grew backwards");
    assert_eq!(state.synced_to, 200, "and kept its newest end");
    assert!(state.reached_beginning);
}

#[test]
fn resync_is_split_so_a_reconnect_is_not_114_blocking_fetches() {
    let engine = engine();
    let mut context = SyncContext::new(me(), ThreadMode::Flat);
    context.active_channel = Some("focus".to_string());

    let mut channels = Vec::new();
    let mut members = Vec::new();
    for index in 0..40 {
        let id = format!("c{index:02}");
        channels.push(Channel {
            id: id.clone(),
            team_id: "t1".into(),
            channel_type: "O".into(),
            name: id.clone(),
            display_name: id.clone(),
            total_msg_count: 10,
            total_msg_count_root: 10,
            last_post_at: 1_000 - index,
            delete_at: 0,
        });
        members.push(ChannelMember {
            channel_id: id,
            user_id: "me".into(),
            last_viewed_at: 0,
            msg_count: 10,
            msg_count_root: 10,
            mention_count: 0,
            mention_count_root: 0,
            notify_props: HashMap::new(),
        });
    }
    channels.push(Channel {
        id: "focus".into(),
        team_id: "t1".into(),
        channel_type: "O".into(),
        name: "focus".into(),
        display_name: "focus".into(),
        total_msg_count: 1,
        total_msg_count_root: 1,
        last_post_at: 1,
        delete_at: 0,
    });
    engine.store().upsert_channels(&channels).unwrap();
    engine.store().upsert_channel_members(&members).unwrap();

    let plan = engine.plan_resync(&context).unwrap();
    assert_eq!(plan.immediate.len(), IMMEDIATE_BUDGET);
    assert_eq!(
        plan.immediate[0], "focus",
        "the channel on screen is fetched before anything else"
    );
    assert_eq!(plan.trickle.len(), TRICKLE_BUDGET);
    assert!(
        !plan.lazy.is_empty(),
        "the tail must be deferred, not fetched on reconnect"
    );
    assert_eq!(
        plan.immediate.len() + plan.trickle.len() + plan.lazy.len(),
        41
    );
}

/// Muting is a membership property, and the decision has to read it from where
/// membership actually lives.
///
/// `notify::decide` was already tested against injected props and passed, while
/// the app notified on every muted channel for months: the context field those
/// props were supposed to arrive in was never written outside the probes. This
/// test puts the setting only in the store, which is the one place the app
/// keeps it.
#[test]
fn a_muted_channel_does_not_notify_from_the_store_alone() {
    let engine = engine();
    let context = SyncContext::new(me(), ThreadMode::Flat);
    engine
        .store()
        .upsert_channels(&[Channel {
            id: "c1".into(),
            team_id: "t1".into(),
            channel_type: "O".into(),
            name: "c1".into(),
            display_name: "c1".into(),
            total_msg_count: 0,
            total_msg_count_root: 0,
            last_post_at: 0,
            delete_at: 0,
        }])
        .unwrap();

    let mut muted = HashMap::new();
    // What Mattermost calls muted: only a mention marks it unread.
    muted.insert("mark_unread".to_string(), "mention".to_string());
    engine
        .store()
        .upsert_channel_members(&[ChannelMember {
            channel_id: "c1".into(),
            user_id: "me".into(),
            last_viewed_at: 0,
            msg_count: 0,
            msg_count_root: 0,
            mention_count: 0,
            mention_count_root: 0,
            notify_props: muted,
        }])
        .unwrap();

    let deltas = engine
        .apply_event(
            &Event::Posted {
                post: Box::new(post("p1", 100, "")),
                channel_id: "c1".into(),
            },
            &context,
        )
        .unwrap();

    let notified = deltas.iter().any(|delta| match delta {
        Delta::PostUpserted { notify, .. } => *notify,
        _ => false,
    });
    assert!(
        !notified,
        "a plain message in a muted channel must not notify"
    );
}

#[test]
fn unread_delta_follows_a_live_post() {
    let engine = engine();
    let context = SyncContext::new(me(), ThreadMode::Flat);
    engine
        .store()
        .upsert_channels(&[Channel {
            id: "c1".into(),
            team_id: "t1".into(),
            channel_type: "O".into(),
            name: "c1".into(),
            display_name: "c1".into(),
            total_msg_count: 5,
            total_msg_count_root: 5,
            last_post_at: 100,
            delete_at: 0,
        }])
        .unwrap();
    engine
        .store()
        .upsert_channel_members(&[ChannelMember {
            channel_id: "c1".into(),
            user_id: "me".into(),
            last_viewed_at: 0,
            msg_count: 3,
            msg_count_root: 3,
            mention_count: 1,
            mention_count_root: 0,
            notify_props: HashMap::new(),
        }])
        .unwrap();

    let deltas = engine
        .apply_event(
            &Event::Posted {
                post: Box::new(post("p1", 100, "")),
                channel_id: "c1".into(),
            },
            &context,
        )
        .unwrap();

    let unread = deltas.iter().find_map(|delta| match delta {
        Delta::UnreadChanged { unread, .. } => Some(*unread),
        _ => None,
    });
    let unread = unread.expect("a live post must report the new unread count");
    // 5 held + this arrival = 6, against 3 seen. This asserted 2 before
    // `record_arrival` existed, which was the bug: the post was stored but the
    // counter unread derives from never moved, so a badge never changed.
    assert_eq!(unread.messages, 3);
    assert_eq!(unread.mentions, 1);
}

#[test]
fn a_read_on_another_device_clears_the_unread_here() {
    let engine = engine();
    let context = SyncContext::new(me(), ThreadMode::Flat);
    engine
        .store()
        .upsert_channels(&[Channel {
            id: "c1".into(),
            team_id: "t1".into(),
            channel_type: "O".into(),
            name: "c1".into(),
            display_name: "c1".into(),
            total_msg_count: 20,
            total_msg_count_root: 20,
            last_post_at: 500,
            delete_at: 0,
        }])
        .unwrap();
    engine
        .store()
        .upsert_channel_members(&[ChannelMember {
            channel_id: "c1".into(),
            user_id: "me".into(),
            last_viewed_at: 100,
            msg_count: 12,
            msg_count_root: 12,
            mention_count: 4,
            mention_count_root: 2,
            notify_props: HashMap::new(),
        }])
        .unwrap();

    let before = engine.store().unread("c1", "me").unwrap().unwrap();
    assert_eq!((before.messages, before.mentions), (8, 4));

    // The shape the live server actually sends: channel id -> viewed-at ms.
    let deltas = engine
        .apply_event(
            &Event::ChannelsViewed {
                channel_times: vec![("c1".to_string(), 9_999)],
            },
            &context,
        )
        .unwrap();

    let reported = deltas
        .iter()
        .find_map(|delta| match delta {
            Delta::UnreadChanged { unread, .. } => Some(*unread),
            _ => None,
        })
        .expect("a read elsewhere must report the new unread count");
    assert_eq!(reported.messages, 0, "reading on another device clears it");
    assert_eq!(reported.mentions, 0);

    let after = engine.store().unread("c1", "me").unwrap().unwrap();
    assert_eq!(after.messages, 0, "and it is persisted, not just reported");
}

#[test]
fn a_runtime_preference_change_flags_the_thread_mode() {
    let engine = engine();
    let context = SyncContext::new(me(), ThreadMode::Flat);

    let flip = Event::PreferencesChanged {
        preferences: vec![Preference {
            user_id: "me".into(),
            category: "display_settings".into(),
            name: "collapsed_reply_threads".into(),
            value: "off".into(),
        }],
    };
    let deltas = engine.apply_event(&flip, &context).unwrap();
    assert!(matches!(
        deltas[0],
        Delta::PreferencesChanged {
            affects_thread_mode: true
        }
    ));

    // An unrelated preference must not force a re-resolve.
    let unrelated = Event::PreferencesChanged {
        preferences: vec![Preference {
            user_id: "me".into(),
            category: "display_settings".into(),
            name: "use_military_time".into(),
            value: "true".into(),
        }],
    };
    let deltas = engine.apply_event(&unrelated, &context).unwrap();
    assert!(matches!(
        deltas[0],
        Delta::PreferencesChanged {
            affects_thread_mode: false
        }
    ));
}

/// The bug behind a badge that never moved: the post was stored but the counter
/// unread is derived from was not.
#[test]
fn a_live_post_moves_the_unread_count() {
    let engine = engine();
    let context = SyncContext::new(me(), ThreadMode::Flat);
    engine
        .store()
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
    engine
        .store()
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
    assert_eq!(
        engine.store().unread("c1", "me").unwrap().unwrap().messages,
        0
    );

    let deltas = engine
        .apply_event(
            &Event::Posted {
                post: Box::new(post("from-someone", 200, "")),
                channel_id: "c1".into(),
            },
            &context,
        )
        .unwrap();
    let reported = deltas
        .iter()
        .find_map(|delta| match delta {
            Delta::UnreadChanged { unread, .. } => Some(*unread),
            _ => None,
        })
        .expect("a live post must report unread");
    assert_eq!(reported.messages, 1, "someone else posting is one unread");
}

/// Sending is reading: the server advances both counters, and mirroring only one
/// would leave a phantom unread for every message sent.
#[test]
fn my_own_post_does_not_count_as_unread() {
    let engine = engine();
    let context = SyncContext::new(me(), ThreadMode::Flat);
    engine
        .store()
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
    engine
        .store()
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

    let mut mine = post("mine", 200, "");
    mine.user_id = "me".into();
    engine
        .apply_event(
            &Event::Posted {
                post: Box::new(mine),
                channel_id: "c1".into(),
            },
            &context,
        )
        .unwrap();

    assert_eq!(
        engine.store().unread("c1", "me").unwrap().unwrap().messages,
        0,
        "my own message must not appear as unread to me"
    );
}
