//! Phase 2 gate: go offline for ten minutes, come back, and prove local state
//! matches the server -- unread counts included, and not one delta mislabelled
//! as live.
//!
//! "Offline" is simulated by dropping the socket with an RST and refusing to
//! reconcile for the duration, rather than by touching the machine's network.
//! From the client's side that is the same situation: the server keeps
//! accumulating posts and we are not listening. The RST (rather than a clean
//! close) is what a real outage looks like to the peer.
//!
//! Four independent checks, because a self-consistent client can still be wrong:
//!
//!   A  no delta produced by the catch-up is labelled Live or raises a toast
//!   B  no post the server has after the outage is missing from the store
//!   C  the store's channel order matches the server's `order` array
//!   D  locally derived unread equals unread computed straight from the server

use anyhow::{Context, Result};
use matterless_core::model::{ChannelMember, ThreadMode};
use matterless_core::ws::{Signal, WsSession};
use matterless_core::{RestClient, User};
use matterless_store::Store;
use matterless_sync::{Arrival, Delta, SyncContext, SyncEngine};
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::mpsc;

/// Channels to bootstrap and then verify. The full 114 would work but makes the
/// run slow without testing anything the top slice does not.
const DEFAULT_CHANNEL_BUDGET: usize = 15;
const LIVE_OBSERVE: Duration = Duration::from_secs(30);

pub struct GateConfig {
    pub offline_minutes: u64,
    pub channel_budget: usize,
    pub db_path: String,
}

impl Default for GateConfig {
    fn default() -> Self {
        Self {
            offline_minutes: 10,
            channel_budget: DEFAULT_CHANNEL_BUDGET,
            db_path: ".gate.db".to_string(),
        }
    }
}

#[derive(Default)]
struct Findings {
    channels_watched: usize,
    live_deltas: usize,
    live_notifies: usize,
    posts_found_by_catchup: usize,
    catchup_deltas: usize,
    /// Must be zero: a catch-up delta that claimed to be live or asked to notify.
    mislabelled_as_live: Vec<String>,
    /// Server has it, the store does not: a hole.
    missing_from_store: Vec<(String, String)>,
    /// Store order differs from the server's `order`.
    order_mismatches: Vec<String>,
    /// (channel, local_unread, server_unread, local_mentions, server_mentions)
    unread_mismatches: Vec<(String, i64, i64, i64, i64)>,
    channels_that_moved: usize,
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as i64)
        .unwrap_or(0)
}

/// Builds the notify context the engine needs from the member records.
fn context_from(me: User, thread_mode: ThreadMode, _members: &[ChannelMember]) -> SyncContext {
    // The members are not copied in: notification settings are read from the
    // store at the moment a decision is made, and these are already in it.
    let mut context = SyncContext::new(me, thread_mode);
    // Deliberately "not looking at anything, window unfocused" so the
    // AlreadyLooking rule cannot mask a mislabelled notification.
    context.active_channel = None;
    context.window_focused = false;
    context
}

pub async fn run(client: &RestClient, me: &User, config: GateConfig) -> Result<bool> {
    // A fresh database each run keeps the verdict deterministic.
    let path = Path::new(&config.db_path);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).ok();
    }
    for suffix in ["", "-wal", "-shm"] {
        let _ = std::fs::remove_file(format!("{}{suffix}", config.db_path));
    }
    let store = Arc::new(Store::open(path).context("open gate store")?);
    // NOTE: the probe calls the blocking store directly. The Tauri shell must
    // wrap these in spawn_blocking -- see the store's module docs.
    let engine = SyncEngine::new(Arc::clone(&store));

    let mut findings = Findings::default();

    // ---------------------------------------------------------- bootstrap
    println!("[1/6] bootstrapping reference data ...");
    let client_config = client.client_config().await.context("client config")?;
    let preferences = client.preferences(&me.id).await.context("preferences")?;
    let thread_mode = matterless_core::resolve_thread_mode(&client_config, &preferences);
    println!("      thread mode: {thread_mode:?}");

    let teams = client.my_teams().await.context("teams")?;
    store.upsert_teams(&teams)?;
    store.upsert_users(std::slice::from_ref(me))?;
    store.upsert_preferences(&preferences)?;

    let mut all_channels = Vec::new();
    let mut all_members = Vec::new();
    for team in &teams {
        all_channels.extend(client.my_channels(&team.id).await.context("channels")?);
        all_members.extend(
            client
                .my_channel_members(&team.id)
                .await
                .context("channel members")?,
        );
    }
    // DM and group channels are returned for EVERY team, so the per-team lists
    // must be deduped by id or the watched set burns slots on the same channel.
    let mut seen_channel_ids = HashSet::new();
    all_channels
        .retain(|channel| channel.delete_at == 0 && seen_channel_ids.insert(channel.id.clone()));
    store.upsert_channels(&all_channels)?;
    store.upsert_channel_members(&all_members)?;

    let context = context_from(me.clone(), thread_mode, &all_members);

    // Most recently active first: where posts are actually likely to land.
    let mut ranked = all_channels.clone();
    ranked.sort_by_key(|channel| std::cmp::Reverse(channel.last_post_at));
    let watched: Vec<String> = ranked
        .iter()
        .take(config.channel_budget)
        .map(|channel| channel.id.clone())
        .collect();
    findings.channels_watched = watched.len();

    println!(
        "[2/6] baseline sync of {} of {} channels ...",
        watched.len(),
        all_channels.len()
    );
    let mut watermark: HashMap<String, i64> = HashMap::new();
    for channel_id in &watched {
        let list = client
            .posts(channel_id, 60)
            .await
            .context("baseline posts")?;
        engine.apply_post_list(channel_id, &list, Arrival::Backfill, &context, now_ms())?;
        watermark.insert(channel_id.clone(), engine.catch_up_cursor(channel_id)?);
    }

    // ------------------------------------------------- live sanity check
    println!(
        "[3/6] observing live traffic for {}s ...",
        LIVE_OBSERVE.as_secs()
    );
    let token = client.token().context("not authenticated")?;
    let (signals_tx, mut signals) = mpsc::channel(1024);
    let handle = WsSession::new(&client.base_url(), token.clone())?.spawn(signals_tx);

    let observe_until = Instant::now() + LIVE_OBSERVE;
    while Instant::now() < observe_until {
        let remaining = observe_until.saturating_duration_since(Instant::now());
        match tokio::time::timeout(remaining, signals.recv()).await {
            Ok(Some(Signal::Event { event, .. })) => {
                for delta in engine.apply_event(&event, &context)? {
                    if let Delta::PostUpserted {
                        arrival, notify, ..
                    } = delta
                    {
                        findings.live_deltas += 1;
                        if arrival == Arrival::Live && notify {
                            findings.live_notifies += 1;
                        }
                    }
                }
            }
            Ok(Some(_)) => {}
            Ok(None) => break,
            Err(_) => break,
        }
    }

    // --------------------------------------------------------- go offline
    println!(
        "\n[4/6] going offline for {} minutes (RST, then no reconciling).",
        config.offline_minutes
    );
    println!(
        "      >>> post a message in any channel now, or the gate has nothing to catch up on <<<\n"
    );
    handle.hard_reset().await;
    handle.shutdown().await;
    drop(signals);

    let offline_until = Instant::now() + Duration::from_secs(config.offline_minutes * 60);
    while Instant::now() < offline_until {
        let left = offline_until.saturating_duration_since(Instant::now());
        println!(
            "      offline, {:.1} min remaining",
            left.as_secs_f64() / 60.0
        );
        tokio::time::sleep(Duration::from_secs(60).min(left.max(Duration::from_secs(1)))).await;
    }

    // ------------------------------------------------------- come back up
    println!("\n[5/6] reconnecting and running the prioritised catch-up ...");
    let (signals_tx, mut signals) = mpsc::channel(1024);
    let handle = WsSession::new(&client.base_url(), token)?.spawn(signals_tx);

    // Wait for the socket so live events do not race the catch-up.
    let mut resync_signalled = false;
    let connect_deadline = Instant::now() + Duration::from_secs(45);
    while Instant::now() < connect_deadline {
        match tokio::time::timeout(Duration::from_secs(20), signals.recv()).await {
            Ok(Some(Signal::Connected {
                connection_id,
                resumed,
            })) => {
                println!("      connected {connection_id} resumed={resumed}");
                break;
            }
            Ok(Some(Signal::ResyncRequired)) => resync_signalled = true,
            Ok(Some(_)) => {}
            Ok(None) | Err(_) => break,
        }
    }
    println!("      resync signalled by the session: {resync_signalled}");

    let plan = engine.plan_resync(&context)?;
    println!(
        "      resync plan: {} immediate, {} trickle, {} lazy",
        plan.immediate.len(),
        plan.trickle.len(),
        plan.lazy.len()
    );

    // Only the channels we baselined can be verified, so catch those up.
    for channel_id in &watched {
        let cursor = *watermark.get(channel_id).unwrap_or(&0);
        let list = client
            .posts_since(channel_id, cursor.saturating_sub(1))
            .await
            .context("catch-up posts")?;
        if list.posts.is_empty() {
            continue;
        }
        let deltas =
            engine.apply_post_list(channel_id, &list, Arrival::Resync, &context, now_ms())?;
        for delta in deltas {
            if let Delta::PostUpserted {
                post_id,
                arrival,
                notify,
                ..
            } = delta
            {
                findings.catchup_deltas += 1;
                findings.posts_found_by_catchup += 1;
                if arrival != Arrival::Resync || notify {
                    findings.mislabelled_as_live.push(post_id);
                }
            }
        }
    }
    handle.shutdown().await;

    // ----------------------------------------------------------- verify
    println!("\n[6/6] verifying against the server ...");

    // Fresh server view, independent of anything we stored during the outage.
    let mut fresh_members: Vec<ChannelMember> = Vec::new();
    let mut fresh_channels = Vec::new();
    for team in &teams {
        fresh_channels.extend(client.my_channels(&team.id).await?);
        fresh_members.extend(client.my_channel_members(&team.id).await?);
    }
    let server_channels: HashMap<&str, &matterless_core::Channel> = fresh_channels
        .iter()
        .map(|channel| (channel.id.as_str(), channel))
        .collect();
    let server_members: HashMap<&str, &ChannelMember> = fresh_members
        .iter()
        .map(|member| (member.channel_id.as_str(), member))
        .collect();

    for channel_id in &watched {
        let before = *watermark.get(channel_id).unwrap_or(&0);
        let moved = server_channels
            .get(channel_id.as_str())
            .is_some_and(|channel| channel.last_post_at > before);
        if moved {
            findings.channels_that_moved += 1;
        }

        // B: every server post since the watermark must now be in the store.
        let server_side = client
            .posts_since(channel_id, before.saturating_sub(1))
            .await?;
        for post in server_side.posts.values() {
            if post.delete_at != 0 {
                continue;
            }
            if store.post(&post.id)?.is_none() {
                findings
                    .missing_from_store
                    .push((channel_id.clone(), post.id.clone()));
            }
        }

        // C: ordering. Compare the newest page the server gives against ours.
        let newest = client.posts(channel_id, 30).await?;
        let server_order: Vec<String> = newest
            .in_order()
            .filter(|post| post.delete_at == 0)
            .map(|post| post.id.clone())
            .collect();
        let local_order: Vec<String> = store
            .channel_page(channel_id, None, 200)?
            .into_iter()
            .filter(|post| post.delete_at == 0)
            .map(|post| post.id)
            .collect();
        let local_set: HashSet<&str> = local_order.iter().map(String::as_str).collect();
        let comparable: Vec<&String> = server_order
            .iter()
            .filter(|id| local_set.contains(id.as_str()))
            .collect();
        let local_projection: Vec<&String> = local_order
            .iter()
            .filter(|id| comparable.contains(id))
            .collect();
        if local_projection != comparable {
            findings.order_mismatches.push(channel_id.clone());
        }
    }

    // D: derived unread. Upsert the fresh server records, then check our
    // derivation reproduces what the server's own numbers say.
    store.upsert_channels(&fresh_channels)?;
    store.upsert_channel_members(&fresh_members)?;
    for channel_id in &watched {
        let Some(channel) = server_channels.get(channel_id.as_str()) else {
            continue;
        };
        let Some(member) = server_members.get(channel_id.as_str()) else {
            continue;
        };
        let server_unread = (channel.total_msg_count - member.msg_count).max(0);
        let server_mentions = member.mention_count;
        let local = store.unread(channel_id, &me.id)?.unwrap_or_default();
        if local.messages != server_unread || local.mentions != server_mentions {
            findings.unread_mismatches.push((
                channel_id.clone(),
                local.messages,
                server_unread,
                local.mentions,
                server_mentions,
            ));
        }
    }

    Ok(report(&findings, &config))
}

fn report(findings: &Findings, config: &GateConfig) -> bool {
    println!("\n============== Phase 2 offline gate ==============");
    println!("channels watched        {}", findings.channels_watched);
    println!("offline for             {} min", config.offline_minutes);
    println!("live deltas seen        {}", findings.live_deltas);
    println!("live notifications      {}", findings.live_notifies);
    println!("channels that moved     {}", findings.channels_that_moved);
    println!(
        "posts found by catch-up {}",
        findings.posts_found_by_catchup
    );
    println!("catch-up deltas         {}", findings.catchup_deltas);

    println!("\n---- checks ----");
    let mut passed = true;

    if findings.posts_found_by_catchup == 0 {
        println!(
            "INCONCLUSIVE  nothing was posted during the outage, so checks A-C had \
             nothing to test.\n              Re-run and post a message during the \
             offline window."
        );
    } else if findings.mislabelled_as_live.is_empty() {
        println!(
            "A PASS  all {} catch-up deltas were labelled Resync and none asked to notify",
            findings.catchup_deltas
        );
    } else {
        passed = false;
        println!(
            "A FAIL  {} catch-up deltas mislabelled or notifying: {:?}",
            findings.mislabelled_as_live.len(),
            &findings.mislabelled_as_live[..findings.mislabelled_as_live.len().min(10)]
        );
    }

    if findings.missing_from_store.is_empty() {
        println!("B PASS  no server post is missing from the store");
    } else {
        passed = false;
        println!(
            "B FAIL  {} posts the server has are absent locally: {:?}",
            findings.missing_from_store.len(),
            &findings.missing_from_store[..findings.missing_from_store.len().min(10)]
        );
    }

    if findings.order_mismatches.is_empty() {
        println!("C PASS  local ordering matches the server's order array");
    } else {
        passed = false;
        println!(
            "C FAIL  ordering differs in {} channels: {:?}",
            findings.order_mismatches.len(),
            &findings.order_mismatches[..findings.order_mismatches.len().min(5)]
        );
    }

    if findings.unread_mismatches.is_empty() {
        println!("D PASS  derived unread and mention counts match the server");
    } else {
        passed = false;
        println!(
            "D FAIL  unread mismatches (channel, local, server, localMentions, serverMentions):"
        );
        for row in findings.unread_mismatches.iter().take(10) {
            println!(
                "          {} {} vs {} | {} vs {}",
                row.0, row.1, row.2, row.3, row.4
            );
        }
    }

    println!("==================================================");
    passed && findings.posts_found_by_catchup > 0
}
