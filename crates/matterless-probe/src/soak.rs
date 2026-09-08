//! Phase 1 gate, and its replay.
//!
//! The first version kept its own ledger of what the websocket delivered, which
//! made it structurally unable to fail the way the app would: it never touched
//! `PostChange::Unchanged`, the production dedup. Everything now goes through the
//! real store and engine, and two questions are asked of two different layers:
//!
//!   1  **dedup** -- did any post ever produce two `Inserted` deltas? The
//!      engine's job, and the failure a reconnect causes.
//!   2  **transport** -- did a strict sweep find a post the socket never
//!      delivered? The socket's job.
//!
//! A live run can record itself (`--record`), and a recording replays in seconds
//! with no network (`--replay`). Iterating on the checks against a real session
//! should not cost an hour a time -- which is how two bugs in this file survived
//! as long as they did.

use crate::recording::{Record, Recorder, read_all};
use anyhow::{Context, Result};
use matterless_core::model::{
    Channel, ChannelMember, Post, PostList, Preference, Team, ThreadMode, Timestamp, User,
};
use matterless_core::ws::{Signal, WsSession};
use matterless_core::{Event, RestClient};
use matterless_store::{PostChange, Store, SyncState};
use matterless_sync::{Arrival, Delta, SyncContext, SyncEngine};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::mpsc;

const RECONCILE_EVERY: Duration = Duration::from_secs(60);
const DRAIN_GRACE: Duration = Duration::from_secs(6);

pub struct SoakConfig {
    pub minutes: u64,
    /// 0 disables the forced disconnect.
    pub rst_every_minutes: u64,
    pub only_channel: Option<String>,
    pub record_path: Option<String>,
}

struct ChannelWatch {
    label: String,
    last_post_at: Timestamp,
    total_msg_count: i64,
    /// Server-time watermark at bootstrap. Only posts created strictly after
    /// this were ever owed; anything at or before it predates our connection.
    /// Server time throughout, so clock skew cannot distort the comparison.
    owed_after: Timestamp,
}

/// The posts a `?since=` response says the socket owed us.
///
/// Both exclusions caused a false failure when they were missing:
///
/// * **Thread context.** The response map carries each reply's root, often
///   created long before the window. 84% of posts here are replies, so this is
///   the common case rather than an edge one; `in_order()` excludes them.
/// * **The boundary post.** A sweep asks for `since = watermark - 1`, so the
///   post sitting exactly on the watermark returns every time -- and it existed
///   before we were listening.
///
/// Inferring "owed" from "the store inserted it" gets both wrong, which is how a
/// perfectly healthy run reported twelve phantom losses.
fn owed_posts(list: &PostList, owed_after: Timestamp) -> Vec<&Post> {
    list.in_order()
        .filter(|post| post.create_at > owed_after && !post.is_deleted())
        .collect()
}

#[derive(Default)]
struct Ledger {
    /// post id -> number of `Inserted` deltas. Two means dedup failed.
    inserted: HashMap<String, u32>,
    duplicate_deltas: Vec<String>,
    /// Every post id the socket delivered, taken from the raw event rather than
    /// a delta -- a redelivery yields no delta but still proves transport, which
    /// keeps the miss check free of races.
    seen_live: HashSet<String>,
    /// Owed, and found by a sweep with no disconnect since the previous one.
    miss_candidates: Vec<(String, String)>,
    /// Every owed post: the honest sample size for the transport check.
    owed: HashSet<String>,
    events: HashMap<String, u64>,
    channel_hits: HashMap<String, u32>,
    connections: u32,
    forced_resets: u32,
    resyncs_required: u32,
    reconcile_cycles: u32,
    strict_cycles: u32,
    targeted_fetches: u32,
}

fn now_ms() -> Timestamp {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as Timestamp)
        .unwrap_or(0)
}

fn label_for(channel: &Channel, team_name: &str) -> String {
    let name = if channel.display_name.is_empty() {
        channel.name.as_str()
    } else {
        channel.display_name.as_str()
    };
    format!("{team_name}/{name}")
}

fn thread_mode_from(text: &str) -> ThreadMode {
    match text {
        "Collapsed" => ThreadMode::Collapsed,
        "Disabled" => ThreadMode::Disabled,
        _ => ThreadMode::Flat,
    }
}

/// Shared by the live run and a replay, so both exercise identical logic.
struct Session {
    engine: SyncEngine,
    context: SyncContext,
    watches: HashMap<String, ChannelWatch>,
    ledger: Ledger,
}

impl Session {
    fn new(
        me: &User,
        thread_mode: ThreadMode,
        teams: &[Team],
        channels: &[Channel],
        members: &[ChannelMember],
        preferences: &[Preference],
    ) -> Result<Self> {
        let store = Arc::new(Store::open_in_memory().context("open soak store")?);
        let engine = SyncEngine::new(Arc::clone(&store));
        store.upsert_teams(teams)?;
        store.upsert_users(std::slice::from_ref(me))?;
        store.upsert_preferences(preferences)?;
        store.upsert_channels(channels)?;
        store.upsert_channel_members(members)?;

        // Seed each channel's watermark where it already is, so a sweep only
        // reports posts made from now on. Without this the first sweep to touch
        // a channel fetches from timestamp zero and imports its whole history as
        // newly inserted. Costs no requests: last_post_at is already here.
        let bootstrapped_at = now_ms();
        for channel in channels {
            store.set_sync_state(&SyncState {
                channel_id: channel.id.clone(),
                synced_from: channel.last_post_at,
                synced_to: channel.last_post_at,
                oldest_post_id: String::new(),
                newest_post_id: String::new(),
                reached_beginning: false,
                last_reconciled_at: bootstrapped_at,
            })?;
        }

        let team_names: HashMap<&str, &str> = teams
            .iter()
            .map(|team| (team.id.as_str(), team.name.as_str()))
            .collect();
        let watches = channels
            .iter()
            .map(|channel| {
                let team = team_names
                    .get(channel.team_id.as_str())
                    .copied()
                    .unwrap_or("dm");
                (
                    channel.id.clone(),
                    ChannelWatch {
                        label: label_for(channel, team),
                        last_post_at: channel.last_post_at,
                        total_msg_count: channel.total_msg_count,
                        owed_after: channel.last_post_at,
                    },
                )
            })
            .collect();

        let mut context = SyncContext::new(me.clone(), thread_mode);
        for member in members {
            context
                .channel_notify_props
                .insert(member.channel_id.clone(), member.notify_props.clone());
        }
        context.window_focused = false;

        Ok(Self {
            engine,
            context,
            watches,
            ledger: Ledger::default(),
        })
    }

    fn record_delta(&mut self, delta: &Delta) {
        let Delta::PostUpserted {
            post_id,
            channel_id,
            change,
            ..
        } = delta
        else {
            return;
        };
        if *change != PostChange::Inserted {
            // An edit legitimately upserts again; only a second *insert* of the
            // same post is a dedup failure.
            return;
        }
        let count = self.ledger.inserted.entry(post_id.clone()).or_insert(0);
        *count += 1;
        if *count == 2 {
            self.ledger.duplicate_deltas.push(post_id.clone());
        }
        if *count == 1 {
            *self
                .ledger
                .channel_hits
                .entry(channel_id.clone())
                .or_insert(0) += 1;
        }
    }

    fn on_event(&mut self, event: &Event) -> Result<()> {
        *self
            .ledger
            .events
            .entry(event.name().to_string())
            .or_insert(0) += 1;
        if let Event::Posted { post, .. } = event {
            self.ledger.seen_live.insert(post.id.clone());
        }
        let deltas = self.engine.apply_event(event, &self.context)?;
        for delta in &deltas {
            self.record_delta(delta);
        }
        Ok(())
    }

    fn on_rest_since(&mut self, channel_id: &str, list: &PostList, strict: bool) -> Result<()> {
        let deltas = self.engine.apply_post_list(
            channel_id,
            list,
            Arrival::Resync,
            &self.context,
            now_ms(),
        )?;
        for delta in &deltas {
            self.record_delta(delta);
        }

        let owed_after = self
            .watches
            .get(channel_id)
            .map(|watch| watch.owed_after)
            .unwrap_or(0);
        for post in owed_posts(list, owed_after) {
            self.ledger.owed.insert(post.id.clone());
            if strict && !self.ledger.seen_live.contains(&post.id) {
                self.ledger
                    .miss_candidates
                    .push((channel_id.to_string(), post.id.clone()));
            }
        }
        Ok(())
    }

    fn report(&self, elapsed: Duration, resets_expected: bool) -> bool {
        let ledger = &self.ledger;
        println!("\n=========== Phase 1 soak (hardened) ===========");
        println!("ran for            {:.1} min", elapsed.as_secs_f64() / 60.0);
        println!("channels watched   {}", self.watches.len());
        println!("posts inserted     {}", ledger.inserted.len());
        println!(
            "  of which owed    {}  (after the watermark, in-stream)",
            ledger.owed.len()
        );
        println!("delivered live     {}", ledger.seen_live.len());
        println!("connections        {}", ledger.connections);
        println!("forced resets      {}", ledger.forced_resets);
        println!("resyncs required   {}", ledger.resyncs_required);
        println!(
            "reconcile cycles   {} ({} strict, {} targeted fetches)",
            ledger.reconcile_cycles, ledger.strict_cycles, ledger.targeted_fetches
        );

        let total: u64 = ledger.events.values().sum();
        let mut names: Vec<_> = ledger.events.iter().collect();
        names.sort_by(|left, right| right.1.cmp(left.1));
        println!("\nevents by type ({total} total):");
        for (name, count) in names {
            let share = if total > 0 {
                (*count as f64) * 100.0 / (total as f64)
            } else {
                0.0
            };
            println!("  {name:<26} {count:>6}  {share:>5.1}%");
        }

        if !ledger.channel_hits.is_empty() {
            let mut busiest: Vec<_> = ledger.channel_hits.iter().collect();
            busiest.sort_by(|left, right| right.1.cmp(left.1));
            println!("\nwhere the posts landed:");
            for (channel_id, count) in busiest.iter().take(12) {
                let label = self
                    .watches
                    .get(*channel_id)
                    .map(|watch| watch.label.clone())
                    .unwrap_or_else(|| (*channel_id).clone());
                println!("  {label:<46} {count:>4}");
            }
        }

        // A candidate is a real loss only if the socket never delivered it.
        let missed: Vec<&(String, String)> = ledger
            .miss_candidates
            .iter()
            .filter(|(_, post_id)| !ledger.seen_live.contains(post_id))
            .collect();

        println!("\n---- checks ----");
        let mut passed = true;
        if ledger.inserted.is_empty() {
            println!("INCONCLUSIVE  no post arrived at all; nothing was exercised");
            return false;
        }

        if ledger.duplicate_deltas.is_empty() {
            println!(
                "1 PASS  dedup: no post produced two Inserted deltas ({} posts, {} resets)",
                ledger.inserted.len(),
                ledger.forced_resets
            );
        } else {
            passed = false;
            println!(
                "1 FAIL  dedup: {} posts inserted twice: {:?}",
                ledger.duplicate_deltas.len(),
                &ledger.duplicate_deltas[..ledger.duplicate_deltas.len().min(10)]
            );
        }

        if missed.is_empty() {
            println!(
                "2 PASS  transport: all {} owed posts arrived live ({} strict cycles)",
                ledger.owed.len(),
                ledger.strict_cycles
            );
        } else {
            passed = false;
            println!(
                "2 FAIL  transport: {} of {} owed posts never arrived live:",
                missed.len(),
                ledger.owed.len()
            );
            for (channel_id, post_id) in missed.iter().take(10) {
                let label = self
                    .watches
                    .get(channel_id)
                    .map(|watch| watch.label.clone())
                    .unwrap_or_else(|| channel_id.clone());
                println!("          {post_id} in {label}");
            }
        }

        if resets_expected && ledger.forced_resets == 0 {
            println!(
                "NOTE  no reset fired -- the run was shorter than the interval, so \
                 recovery is still unproven"
            );
        }
        println!("===============================================");
        passed
    }
}

/// Cheap sweep: two requests per team reveal which channels moved, and only
/// those get a post fetch.
async fn reconcile(
    client: &RestClient,
    session: &mut Session,
    teams: &[Team],
    recorder: &mut Recorder,
    strict: bool,
) -> Result<()> {
    session.ledger.reconcile_cycles += 1;
    if strict {
        session.ledger.strict_cycles += 1;
    }
    recorder.write(Record::Cycle { strict })?;

    let mut moved = Vec::new();
    for team in teams {
        for channel in client.my_channels(&team.id).await? {
            let Some(watch) = session.watches.get_mut(&channel.id) else {
                continue;
            };
            if channel.last_post_at > watch.last_post_at
                || channel.total_msg_count != watch.total_msg_count
            {
                moved.push(channel.id.clone());
                watch.last_post_at = channel.last_post_at.max(watch.last_post_at);
                watch.total_msg_count = channel.total_msg_count;
            }
        }
    }

    for channel_id in moved {
        session.ledger.targeted_fetches += 1;
        let cursor = session.engine.catch_up_cursor(&channel_id)?;
        let list = match client
            .posts_since(&channel_id, cursor.saturating_sub(1))
            .await
        {
            Ok(list) => list,
            Err(error) => {
                eprintln!("reconcile fetch {channel_id} failed: {error}");
                continue;
            }
        };
        if list.posts.is_empty() {
            continue;
        }
        recorder.write(Record::RestSince {
            channel_id: channel_id.clone(),
            list: list.clone(),
        })?;
        session.on_rest_since(&channel_id, &list, strict)?;
    }
    Ok(())
}

pub async fn run(client: &RestClient, me: &User, config: SoakConfig) -> Result<bool> {
    let client_config = client.client_config().await.context("client config")?;
    let preferences = client.preferences(&me.id).await.context("preferences")?;
    let thread_mode = matterless_core::resolve_thread_mode(&client_config, &preferences);
    let teams = client.my_teams().await.context("teams")?;

    let mut channels: Vec<Channel> = Vec::new();
    let mut members: Vec<ChannelMember> = Vec::new();
    let mut seen = HashSet::new();
    for team in &teams {
        for channel in client.my_channels(&team.id).await.context("channels")? {
            if channel.delete_at != 0 {
                continue;
            }
            if config
                .only_channel
                .as_deref()
                .is_some_and(|wanted| wanted != channel.id)
            {
                continue;
            }
            // DMs come back for every team, so dedupe by id.
            if seen.insert(channel.id.clone()) {
                channels.push(channel);
            }
        }
        members.extend(
            client
                .my_channel_members(&team.id)
                .await
                .context("members")?,
        );
    }

    let mut session = Session::new(me, thread_mode, &teams, &channels, &members, &preferences)?;
    let mut recorder = Recorder::create(config.record_path.as_deref())?;
    recorder.write(Record::Bootstrap {
        me: me.clone(),
        teams: teams.clone(),
        channels: channels.clone(),
        members: members.clone(),
        preferences: preferences.clone(),
        thread_mode: format!("{thread_mode:?}"),
    })?;

    println!("thread mode: {thread_mode:?}");
    println!(
        "watching {} channels from their current watermark",
        session.watches.len()
    );
    println!("every delivery goes through the real store and engine");
    if config.rst_every_minutes > 0 {
        println!(
            "forcing an RST every {} min to exercise reconnect + resync",
            config.rst_every_minutes
        );
    }
    if let Some(path) = config.record_path.as_deref() {
        println!("recording to {path} (message text redacted)");
    }

    let token = client.token().context("not authenticated")?;
    let (signals_tx, mut signals) = mpsc::channel(4096);
    let handle = WsSession::new(&client.base_url(), token)?.spawn(signals_tx);

    let started = Instant::now();
    let finish_at = started + Duration::from_secs(config.minutes * 60);
    let mut disconnected_since_reconcile = false;

    let mut reconcile_timer = tokio::time::interval(RECONCILE_EVERY);
    reconcile_timer.tick().await;
    // An interval of zero is invalid, so park the disabled case far out.
    let reset_period = if config.rst_every_minutes == 0 {
        Duration::from_secs(u64::from(u32::MAX))
    } else {
        Duration::from_secs(config.rst_every_minutes * 60)
    };
    let mut reset_timer = tokio::time::interval(reset_period);
    reset_timer.tick().await;

    while Instant::now() < finish_at {
        tokio::select! {
            signal = signals.recv() => {
                let Some(signal) = signal else { break };
                match signal {
                    Signal::Connected { connection_id, resumed } => {
                        session.ledger.connections += 1;
                        recorder.write(Record::Connected {
                            connection_id: connection_id.clone(), resumed })?;
                        println!("[{:>5.1}m] connected {connection_id} resumed={resumed}",
                                 started.elapsed().as_secs_f64() / 60.0);
                    }
                    Signal::Disconnected { reason } => {
                        disconnected_since_reconcile = true;
                        recorder.write(Record::Disconnected { reason: reason.clone() })?;
                        println!("[{:>5.1}m] disconnected: {reason}",
                                 started.elapsed().as_secs_f64() / 60.0);
                    }
                    Signal::ResyncRequired => {
                        session.ledger.resyncs_required += 1;
                        recorder.write(Record::Resync)?;
                    }
                    Signal::Event { event, .. } => {
                        if recorder.is_recording() {
                            recorder.write(match &event {
                                Event::Posted { post, channel_id } => Record::Posted {
                                    post: (**post).clone(), channel_id: channel_id.clone() },
                                Event::PostEdited(post) => Record::Edited {
                                    post: (**post).clone() },
                                Event::PostDeleted(post) => Record::Deleted {
                                    post: (**post).clone() },
                                other => Record::OtherEvent { name: other.name().to_string() },
                            })?;
                        }
                        session.on_event(&event)?;
                    }
                }
            }

            _ = reconcile_timer.tick() => {
                let strict = !disconnected_since_reconcile;
                reconcile(client, &mut session, &teams, &mut recorder, strict).await?;
                disconnected_since_reconcile = false;
            }

            _ = reset_timer.tick(), if config.rst_every_minutes > 0 => {
                session.ledger.forced_resets += 1;
                println!("[{:>5.1}m] forcing an RST",
                         started.elapsed().as_secs_f64() / 60.0);
                handle.hard_reset().await;
            }
        }
    }

    println!("\ndraining in-flight events ...");
    let drain_until = Instant::now() + DRAIN_GRACE;
    while Instant::now() < drain_until {
        match tokio::time::timeout(Duration::from_secs(1), signals.recv()).await {
            Ok(Some(Signal::Event { event, .. })) => {
                if recorder.is_recording()
                    && let Event::Posted { post, channel_id } = &event
                {
                    recorder.write(Record::Posted {
                        post: (**post).clone(),
                        channel_id: channel_id.clone(),
                    })?;
                }
                session.on_event(&event)?;
            }
            Ok(Some(_)) => {}
            Ok(None) => break,
            Err(_) => {}
        }
    }
    // Never strict: a post from seconds ago may legitimately not have arrived.
    reconcile(client, &mut session, &teams, &mut recorder, false).await?;

    handle.shutdown().await;
    Ok(session.report(started.elapsed(), config.rst_every_minutes > 0))
}

/// Re-runs a recorded session with no network. Seconds, not an hour.
pub fn replay(path: &str) -> Result<bool> {
    let records = read_all(path)?;
    let started = Instant::now();

    let Some(Record::Bootstrap {
        me,
        teams,
        channels,
        members,
        preferences,
        thread_mode,
    }) = records.first()
    else {
        anyhow::bail!("{path}: the first record must be a bootstrap");
    };

    let mut session = Session::new(
        me,
        thread_mode_from(thread_mode),
        teams,
        channels,
        members,
        preferences,
    )?;
    println!("replaying {} records from {path}", records.len());
    println!("thread mode: {thread_mode}");

    let mut strict = true;
    for record in records.iter().skip(1) {
        match record {
            Record::Bootstrap { .. } => {}
            Record::Connected { .. } => session.ledger.connections += 1,
            Record::Disconnected { .. } => session.ledger.forced_resets += 1,
            Record::Resync => session.ledger.resyncs_required += 1,
            Record::Cycle { strict: is_strict } => {
                strict = *is_strict;
                session.ledger.reconcile_cycles += 1;
                if strict {
                    session.ledger.strict_cycles += 1;
                }
            }
            Record::Posted { post, channel_id } => session.on_event(&Event::Posted {
                post: Box::new(post.clone()),
                channel_id: channel_id.clone(),
            })?,
            Record::Edited { post } => {
                session.on_event(&Event::PostEdited(Box::new(post.clone())))?
            }
            Record::Deleted { post } => {
                session.on_event(&Event::PostDeleted(Box::new(post.clone())))?
            }
            Record::OtherEvent { name } => {
                *session.ledger.events.entry(name.clone()).or_insert(0) += 1;
            }
            Record::RestSince { channel_id, list } => {
                session.ledger.targeted_fetches += 1;
                session.on_rest_since(channel_id, list, strict)?;
            }
        }
    }

    let resets = session.ledger.forced_resets > 0;
    println!("replayed in {:.2}s", started.elapsed().as_secs_f64());
    Ok(session.report(started.elapsed(), resets))
}

#[cfg(test)]
mod tests {
    use super::*;
    use matterless_core::model::PostMetadata;

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
            message: String::new(),
            post_type: String::new(),
            file_ids: Vec::new(),
            props: serde_json::Value::Null,
            metadata: PostMetadata::default(),
            pending_post_id: String::new(),
            is_pinned: false,
        }
    }

    /// The exact shape of a sweep response: one new reply in `order`, its old
    /// root along as context, and the boundary post sitting on the watermark.
    fn sweep_response(watermark: Timestamp) -> PostList {
        let mut posts = HashMap::new();
        posts.insert("boundary".to_string(), post("boundary", watermark, ""));
        posts.insert("ancient_root".to_string(), post("ancient_root", 5, ""));
        posts.insert(
            "new_reply".to_string(),
            post("new_reply", watermark + 100, "ancient_root"),
        );
        PostList {
            order: vec!["new_reply".to_string(), "boundary".to_string()],
            posts,
            next_post_id: String::new(),
            prev_post_id: String::new(),
            has_next: false,
        }
    }

    #[test]
    fn only_posts_made_after_the_watermark_are_owed() {
        let response = sweep_response(1_000);
        let owed = owed_posts(&response, 1_000);
        let ids: Vec<&str> = owed.iter().map(|post| post.id.as_str()).collect();
        assert_eq!(
            ids,
            vec!["new_reply"],
            "the boundary post and the old root were never owed"
        );
    }

    #[test]
    fn thread_context_is_never_owed_even_when_recent() {
        // A root created after the watermark but arriving only as context is
        // still not part of the stream, so the socket did not owe it here.
        let mut posts = HashMap::new();
        posts.insert("root".to_string(), post("root", 2_000, ""));
        posts.insert("reply".to_string(), post("reply", 2_100, "root"));
        let list = PostList {
            order: vec!["reply".to_string()],
            posts,
            next_post_id: String::new(),
            prev_post_id: String::new(),
            has_next: false,
        };
        let ids: Vec<&str> = owed_posts(&list, 1_000)
            .iter()
            .map(|post| post.id.as_str())
            .collect();
        assert_eq!(ids, vec!["reply"]);
    }

    #[test]
    fn tombstones_are_not_owed() {
        let mut gone = post("gone", 2_000, "");
        gone.delete_at = 2_500;
        let mut posts = HashMap::new();
        posts.insert("gone".to_string(), gone);
        let list = PostList {
            order: vec!["gone".to_string()],
            posts,
            next_post_id: String::new(),
            prev_post_id: String::new(),
            has_next: false,
        };
        assert!(owed_posts(&list, 1_000).is_empty());
    }

    #[test]
    fn an_empty_sweep_owes_nothing() {
        let list = PostList {
            order: Vec::new(),
            posts: HashMap::new(),
            next_post_id: String::new(),
            prev_post_id: String::new(),
            has_next: false,
        };
        assert!(owed_posts(&list, 0).is_empty());
    }
}
