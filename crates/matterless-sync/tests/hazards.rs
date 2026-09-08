//! The specific sequences that break a chat sync engine, as deterministic tests.
//!
//! These exist because the 60-minute live soak was being used as a debugging
//! loop. Every bug it actually caught was a deterministic logic error that
//! belongs here, where it costs milliseconds and is reproducible -- a live run
//! cannot be re-triggered once its traffic is gone.
//!
//! Each test is named for the hazard, not the mechanism.

use matterless_core::model::{
    Channel, ChannelMember, Post, PostList, PostMetadata, ThreadMode, Timestamp, User,
};
use matterless_core::ws::Event;
use matterless_store::{PostChange, Store};
use matterless_sync::{Arrival, Delta, SyncContext, SyncEngine};
use std::collections::HashMap;
use std::sync::Arc;

fn harness() -> (SyncEngine, SyncContext) {
    let store = Arc::new(Store::open_in_memory().unwrap());
    let engine = SyncEngine::new(store);
    let mut notify_props = HashMap::new();
    notify_props.insert("desktop".to_string(), "all".to_string());
    let me = User {
        id: "me".into(),
        username: "ada".into(),
        first_name: "Gaetan".into(),
        last_name: "Deturche".into(),
        nickname: String::new(),
        email: String::new(),
        last_picture_update: 0,
        notify_props,
        roles: String::new(),
    };
    let mut context = SyncContext::new(me, ThreadMode::Flat);
    context.window_focused = false;

    engine
        .store()
        .upsert_channels(&[Channel {
            id: "c1".into(),
            team_id: "t1".into(),
            channel_type: "O".into(),
            name: "c1".into(),
            display_name: "Channel One".into(),
            total_msg_count: 100,
            total_msg_count_root: 100,
            last_post_at: 1_000,
            delete_at: 0,
        }])
        .unwrap();
    engine
        .store()
        .upsert_channel_members(&[ChannelMember {
            channel_id: "c1".into(),
            user_id: "me".into(),
            last_viewed_at: 0,
            msg_count: 100,
            msg_count_root: 100,
            mention_count: 0,
            mention_count_root: 0,
            notify_props: HashMap::new(),
        }])
        .unwrap();
    (engine, context)
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
        message: format!("text of {id}"),
        post_type: String::new(),
        file_ids: Vec::new(),
        props: serde_json::Value::Null,
        metadata: PostMetadata::default(),
        pending_post_id: String::new(),
        is_pinned: false,
    }
}

fn list_of(order: &[&str], extra_context: &[Post]) -> PostList {
    let mut posts = HashMap::new();
    let mut ordered = Vec::new();
    for (index, id) in order.iter().enumerate() {
        let created = 2_000 + (index as Timestamp) * 10;
        posts.insert((*id).to_string(), post(id, created, ""));
        ordered.push((*id).to_string());
    }
    for post in extra_context {
        posts.insert(post.id.clone(), post.clone());
    }
    PostList {
        order: ordered,
        posts,
        next_post_id: String::new(),
        prev_post_id: String::new(),
        has_next: false,
    }
}

fn inserted_ids(deltas: &[Delta]) -> Vec<String> {
    deltas
        .iter()
        .filter_map(|delta| match delta {
            Delta::PostUpserted {
                post_id,
                change: PostChange::Inserted,
                ..
            } => Some(post_id.clone()),
            _ => None,
        })
        .collect()
}

fn live(engine: &SyncEngine, context: &SyncContext, post: Post) -> Vec<Delta> {
    engine
        .apply_event(
            &Event::Posted {
                post: Box::new(post),
                channel_id: "c1".into(),
            },
            context,
        )
        .unwrap()
}

/// The commonest real sequence: the socket delivers a post, then a reconcile
/// sweep returns the same post because it falls inside the `?since=` window.
#[test]
fn hazard_live_then_reconcile_returns_the_same_post() {
    let (engine, context) = harness();
    let arrived = post("p1", 2_000, "");

    let first = live(&engine, &context, arrived.clone());
    assert_eq!(inserted_ids(&first), vec!["p1"]);

    let mut posts = HashMap::new();
    posts.insert("p1".to_string(), arrived);
    let sweep = PostList {
        order: vec!["p1".to_string()],
        posts,
        next_post_id: String::new(),
        prev_post_id: String::new(),
        has_next: false,
    };
    let second = engine
        .apply_post_list("c1", &sweep, Arrival::Resync, &context, 3_000)
        .unwrap();
    assert!(
        inserted_ids(&second).is_empty(),
        "the reconcile must not insert a post the socket already delivered"
    );
}

/// A reconnect with no resume: the catch-up window overlaps posts we already
/// hold, and the new ones must appear exactly once.
#[test]
fn hazard_reconnect_catchup_overlaps_what_we_hold() {
    let (engine, context) = harness();
    for id in ["p1", "p2"] {
        live(&engine, &context, post(id, 2_000, ""));
    }

    // The sweep window deliberately reaches back over p1 and p2.
    let overlapping = list_of(&["p1", "p2", "p3", "p4"], &[]);
    let deltas = engine
        .apply_post_list("c1", &overlapping, Arrival::Resync, &context, 3_000)
        .unwrap();

    let mut new_ids = inserted_ids(&deltas);
    new_ids.sort();
    assert_eq!(
        new_ids,
        vec!["p3".to_string(), "p4".to_string()],
        "only the genuinely new posts may insert"
    );
}

/// Repeated resets, which is what `--rst-every` exercises live. Ten overlapping
/// catch-ups must still yield one insert per post.
#[test]
fn hazard_repeated_resets_never_double_insert() {
    let (engine, context) = harness();
    let sweep = list_of(&["p1", "p2", "p3"], &[]);
    let mut total_inserts = 0;
    for _ in 0..10 {
        let deltas = engine
            .apply_post_list("c1", &sweep, Arrival::Resync, &context, 3_000)
            .unwrap();
        total_inserts += inserted_ids(&deltas).len();
    }
    assert_eq!(
        total_inserts, 3,
        "ten identical catch-ups must insert three posts, once each"
    );
}

/// A `?since=` response carries each reply's root as context. The root is often
/// old and was never part of the window -- it must be stored but must not be
/// treated as part of the stream. Mistaking these for new arrivals is exactly
/// what made the live soak report twelve phantom losses.
#[test]
fn hazard_since_response_carries_old_thread_context() {
    let (engine, context) = harness();
    let ancient_root = post("root", 50, "");
    let mut reply = post("reply", 2_500, "root");
    reply.root_id = "root".into();

    let mut posts = HashMap::new();
    posts.insert("reply".to_string(), reply);
    posts.insert("root".to_string(), ancient_root);
    let list = PostList {
        order: vec!["reply".to_string()],
        posts,
        next_post_id: String::new(),
        prev_post_id: String::new(),
        has_next: false,
    };

    let deltas = engine
        .apply_post_list("c1", &list, Arrival::Resync, &context, 3_000)
        .unwrap();

    let in_stream: Vec<&String> = deltas
        .iter()
        .filter_map(|delta| match delta {
            Delta::PostUpserted {
                post_id,
                in_stream: true,
                ..
            } => Some(post_id),
            _ => None,
        })
        .collect();
    assert_eq!(
        in_stream,
        vec!["reply"],
        "only the ordered post is in-stream"
    );
    assert!(
        engine.store().post("root").unwrap().is_some(),
        "the root must still be stored so the reply can render"
    );
}

/// Deleting then receiving a stale edit: the tombstone must not be resurrected.
#[test]
fn hazard_stale_edit_arrives_after_a_delete() {
    let (engine, context) = harness();
    live(&engine, &context, post("p1", 2_000, ""));

    let mut deleted = post("p1", 2_000, "");
    deleted.update_at = 5_000;
    deleted.delete_at = 5_000;
    engine
        .apply_event(&Event::PostDeleted(Box::new(deleted)), &context)
        .unwrap();
    assert!(engine.store().post("p1").unwrap().unwrap().is_deleted());

    // An edit that was in flight before the delete, arriving after it.
    let mut stale_edit = post("p1", 2_000, "");
    stale_edit.update_at = 3_000;
    stale_edit.message = "resurrected".into();
    engine
        .apply_event(&Event::PostEdited(Box::new(stale_edit)), &context)
        .unwrap();

    let held = engine.store().post("p1").unwrap().unwrap();
    assert!(held.is_deleted(), "a stale edit must not undo a delete");
    assert_ne!(held.message, "resurrected");
}

/// Posts arriving out of order, which happens across a reconnect boundary.
/// Storage order must not affect read order.
#[test]
fn hazard_out_of_order_arrival_still_reads_newest_first() {
    let (engine, context) = harness();
    live(&engine, &context, post("late", 3_000, ""));
    live(&engine, &context, post("early", 1_500, ""));
    live(&engine, &context, post("middle", 2_000, ""));

    let page = engine.store().channel_page("c1", None, 10).unwrap();
    let ids: Vec<&str> = page.iter().map(|post| post.id.as_str()).collect();
    assert_eq!(ids, vec!["late", "middle", "early"]);
}

/// Two posts sharing a millisecond. `create_at` alone is not a total order, so
/// the id has to break the tie or paging can loop or skip.
#[test]
fn hazard_identical_timestamps_are_still_totally_ordered() {
    let (engine, context) = harness();
    live(&engine, &context, post("bbb", 2_000, ""));
    live(&engine, &context, post("aaa", 2_000, ""));

    let page = engine.store().channel_page("c1", None, 10).unwrap();
    let ids: Vec<&str> = page.iter().map(|post| post.id.as_str()).collect();
    assert_eq!(ids, vec!["bbb", "aaa"], "descending id breaks the tie");

    // Paging back from the shared timestamp must not return them again.
    let older = engine.store().channel_page("c1", Some(2_000), 10).unwrap();
    assert!(older.is_empty(), "paging must not repeat a tied timestamp");
}

/// A catch-up burst after waking from sleep: many posts at once, none of which
/// may raise a notification however hard they mention you.
#[test]
fn hazard_wake_from_sleep_burst_is_silent() {
    let (engine, context) = harness();
    let mut posts = HashMap::new();
    let mut order = Vec::new();
    for index in 0..200 {
        let id = format!("p{index:03}");
        let mut shouty = post(&id, 2_000 + index, "");
        shouty.message = "@ada @channel look at this".into();
        posts.insert(id.clone(), shouty);
        order.push(id);
    }
    let burst = PostList {
        order,
        posts,
        next_post_id: String::new(),
        prev_post_id: String::new(),
        has_next: false,
    };

    let deltas = engine
        .apply_post_list("c1", &burst, Arrival::Resync, &context, 3_000)
        .unwrap();
    let notifying = deltas
        .iter()
        .filter(|delta| matches!(delta, Delta::PostUpserted { notify: true, .. }))
        .count();
    assert_eq!(
        notifying, 0,
        "200 catch-up posts must produce zero toasts, mentions and all"
    );
    assert_eq!(
        inserted_ids(&deltas).len(),
        200,
        "but all of them are stored"
    );
}

/// The same burst arriving live is not silent -- otherwise the previous test
/// would pass with notifications simply broken.
#[test]
fn hazard_the_silence_is_not_just_broken_notifications() {
    let (engine, context) = harness();
    let mut shouty = post("p1", 2_000, "");
    shouty.message = "@ada urgent".into();
    let deltas = live(&engine, &context, shouty);
    assert!(
        deltas
            .iter()
            .any(|delta| matches!(delta, Delta::PostUpserted { notify: true, .. })),
        "a live mention must still notify"
    );
}
