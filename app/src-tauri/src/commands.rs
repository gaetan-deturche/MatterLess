//! The IPC surface.
//!
//! Every command is `async`, and every store call is wrapped in
//! `spawn_blocking`. That is not stylistic: a synchronous Tauri command runs on
//! the main thread, so one slow call serialises every other invoke and freezes
//! the webview until they all finish.
//!
//! `attach_file` is the one exception, and only because it has to be:
//! `tauri::ipc::Request` borrows its body, so it cannot be held across an
//! await. It does no blocking work itself -- it copies the bytes, hands them to
//! a spawned task and returns -- and the upload's result arrives as an event.
//!
//! One mechanism per traffic shape, as the plan sets out: a command for a
//! channel's whole row plan, a `Channel` for deltas, and nothing at all for
//! composer keystrokes.

use crate::AppState;
use crate::engine::{EngineMsg, UiDelta};
use crate::pending::{PendingPost, PendingPosts};
use crate::uploads;
use matterless_core::model::ThreadMode;
use matterless_render::markdown::Node;
use matterless_render::{PlanOptions, Row, ThreadSummary, plan_channel};
use matterless_sync::{Arrival, SyncContext};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;
use tauri::ipc::Channel;
use tauri::{Emitter, Manager, State};

type Reply<T> = Result<T, String>;

fn fail(error: impl std::fmt::Display) -> String {
    error.to_string()
}

#[derive(Serialize)]
pub struct ChannelOpened {
    /// `last_viewed_at` as it stands before anything marks the channel read --
    /// where this visit's "New messages" divider belongs.
    pub divider_at: i64,
    /// Local history is too thin to fill the stream, so the caller should ask
    /// for a refresh *after* it has rendered.
    pub needs_refresh: bool,
}

#[derive(Serialize)]
pub struct SessionInfo {
    pub signed_in: bool,
    pub username: String,
    pub server: String,
}

#[derive(Serialize, Clone)]
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

#[derive(Serialize)]
pub struct Bootstrap {
    pub me_id: String,
    pub username: String,
    pub thread_mode: String,
    /// The server's `TeammateNameDisplay`, passed back so row requests resolve
    /// names the same way the sidebar did.
    pub display_mode: String,
    pub channels: Vec<ChannelSummary>,
    /// True when the sidebar came from the local store with no network at all.
    /// The cold-start budget depends on this being the normal case.
    pub from_cache: bool,
    /// The server's `MaxFileSize` in bytes -- 150 MB here. Reported so the
    /// composer can refuse an oversized file before spending the upload.
    pub max_file_size: i64,
    /// This reader's own presence: `online`, `away`, `dnd`, `offline`, `ooo`.
    pub own_status: String,
    /// Whether the websocket is up *now*.
    ///
    /// Reported rather than assumed: connection deltas are edges, and the
    /// socket can come up while bootstrap is still running -- which left the
    /// shell showing "offline" for the rest of the session over a perfectly
    /// live connection.
    pub connected: bool,
}

/// Where the time in a plan build actually goes.
///
/// `build_ms` alone timed four SQL queries and the markdown parse together,
/// which is enough to know there is a problem and useless for knowing which one
/// -- and guessing from a single aggregate is exactly what produced a wrong
/// conclusion about the render cache. Each phase is timed separately now.
#[derive(Serialize, Default)]
pub struct Timings {
    /// Reading the page of posts.
    pub page_ms: f64,
    /// `thread_summaries`: a GROUP BY over the whole channel, so this grows with
    /// stored history rather than with the window.
    pub threads_ms: f64,
    /// Resolving author names.
    pub names_ms: f64,
    /// Reading and writing the render cache.
    pub cache_ms: f64,
    /// Parsing the markdown that was not cached.
    pub parse_ms: f64,
    /// `plan_channel` itself: grouping, separators and row assembly.
    pub plan_ms: f64,
    pub cache_hits: usize,
    pub cache_misses: usize,
}

#[derive(Serialize)]
pub struct RowsPayload {
    pub channel_id: String,
    pub rows: Vec<Row>,
    /// The oldest `create_at` in this window, so the shell can page past it.
    pub oldest_in_window: Option<i64>,
    /// Whether the page came back full, i.e. more posts exist locally below it.
    pub window_full: bool,
    /// The newest `create_at` in this page, which is where the page above it
    /// starts once this one is sealed.
    pub newest_in_window: Option<i64>,
    /// Custom emoji used by this page: name to id, for the image route.
    ///
    /// Only the custom ones. A standard emoji carries its character in the node
    /// itself, resolved at parse time from a static table.
    pub emoji: HashMap<String, String>,
    /// How long the plan took to build, so the latency budget is visible in the
    /// app rather than asserted in a document.
    pub build_ms: f64,
    pub timings: Timings,
}

/// Is there a *usable* session? Holding a token is not the same as the token
/// working -- it may have expired, or been revoked elsewhere -- so this asks the
/// server rather than trusting its own state.
#[tauri::command]
pub async fn session(app: tauri::AppHandle, state: State<'_, AppState>) -> Reply<SessionInfo> {
    // What the reader chose, not what was compiled in: an install with no
    // server yet reports an empty one, which is what puts the sign-in screen
    // into asking for it.
    let server = crate::stored_server(&crate::data_dir(&app)).unwrap_or_default();
    let absent = SessionInfo {
        signed_in: false,
        username: String::new(),
        server: server.clone(),
    };
    if server.is_empty() || state.rest.token().is_none() {
        return Ok(absent);
    }
    // Verified, not remembered: this call's whole job is to say whether the
    // token still works, and a cached identity would answer yes for ever.
    match state.rest.verify_me().await {
        Ok(user) => Ok(SessionInfo {
            signed_in: true,
            username: user.username,
            server,
        }),
        // Expiry is a UI state, not an error: there is no refresh path on this
        // server, so the only recovery is to prompt.
        Err(matterless_core::Error::SessionExpired) => Ok(absent),
        Err(other) => Err(fail(other)),
    }
}

/// Password login. There is no OAuth2 and no personal access token on this
/// server, so this is the only way in -- and the token goes to the keychain.
/// Points this install at a Mattermost server.
///
/// Checked before it is remembered: a typo saved as the server would leave the
/// app unable to reach anything and unable to say why. `/system/ping` is the
/// cheapest thing that proves a Mattermost is listening -- Phase 0 measured it
/// at 9ms warm.
#[tauri::command]
pub async fn set_server(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    url: String,
) -> Reply<String> {
    let tidied = url.trim().trim_end_matches('/').to_string();
    let tidied = if tidied.starts_with("http://") || tidied.starts_with("https://") {
        tidied
    } else {
        // A host typed without a scheme is the common case, and https is the
        // only one worth defaulting to.
        format!("https://{tidied}")
    };

    state.rest.set_base_url(&tidied).map_err(fail)?;
    state.rest.ping().await.map_err(|error| {
        // Left pointing at it anyway: the sign-in screen is about to ask again,
        // and reverting would make the field forget what was typed.
        format!("no Mattermost server answered at {tidied}: {error}")
    })?;

    crate::store_server(&crate::data_dir(&app), &tidied).map_err(|error| error.to_string())?;
    tracing::info!("server chosen");
    Ok(tidied)
}

#[tauri::command]
pub async fn sign_in(
    state: State<'_, AppState>,
    login_id: String,
    password: String,
    mfa_token: Option<String>,
) -> Reply<SignIn> {
    match state
        .rest
        .login(&login_id, &password, mfa_token.as_deref())
        .await
    {
        Ok(user) => {
            if let Some(matterless_core::AuthToken::Session(token)) = state.rest.token() {
                crate::store_token(&token).map_err(fail)?;
            }
            Ok(SignIn::SignedIn {
                username: user.username,
            })
        }
        // Not an error the reader should read: the password was right and the
        // account simply has a second factor. Reported as an outcome so the
        // sign-in screen can ask for the code instead of showing a failure.
        Err(matterless_core::Error::MfaRequired) => Ok(SignIn::MfaRequired),
        Err(other) => Err(fail(other)),
    }
}

/// How a sign-in attempt ended.
///
/// Two *outcomes* rather than a result and an error, because needing a second
/// factor is not a failure -- it is the next step, and the screen that follows
/// is different from the one an error would show.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum SignIn {
    SignedIn { username: String },
    MfaRequired,
}

/// Paints from SQLite first, then refreshes in the background.
///
/// Nothing here awaits the network: Phase 0 measured a real channel fetch at
/// ~53 ms median and 85 ms at the tail, so a cold start that waited for one
/// would miss its budget before drawing anything.
#[tauri::command]
pub async fn bootstrap(state: State<'_, AppState>) -> Reply<Bootstrap> {
    let store = state.store.clone();
    let rest = state.rest.clone();

    let me = rest.me().await.map_err(fail)?;
    let me_id = me.id.clone();
    let username = me.username.clone();

    // Resolve the thread mode from both sources: the server default and the
    // per-user override, which Phase 0 measured disagreeing.
    let client_config = rest.client_config().await.map_err(fail)?;
    // The standard emoji table is generated from the server's own map, which
    // only changes when the server does -- so the two versions are logged
    // together, and a drift after an upgrade is visible rather than showing up
    // as somebody's `:name:` failing to render.
    let (emoji_names, emoji_generated) = matterless_render::emoji::provenance();
    tracing::info!(
        emoji_names,
        emoji_generated,
        server_version = client_config
            .get("Version")
            .map(String::as_str)
            .unwrap_or("unknown"),
        "standard emoji table"
    );
    let max_file_size: i64 = client_config
        .get("MaxFileSize")
        .and_then(|value| value.parse().ok())
        .unwrap_or(0);
    state
        .max_file_size
        .store(max_file_size, std::sync::atomic::Ordering::Relaxed);
    state.allow_svg.store(
        client_config.get("EnableSVGs").map(String::as_str) == Some("true"),
        std::sync::atomic::Ordering::Relaxed,
    );
    // The reader's own presence, for the do-not-disturb rule. Read here rather
    // than waited for: a `status_change` event only arrives when it *changes*,
    // so a session that starts in Do Not Disturb would otherwise notify freely
    // until the reader touched it.
    let own_status = rest
        .my_status(&me_id)
        .await
        .map(|status| status.status)
        .unwrap_or_else(|error| {
            tracing::warn!(%error, "could not read own status; assuming online");
            "online".to_string()
        });
    let preferences = rest.preferences(&me_id).await.map_err(fail)?;
    let thread_mode = matterless_core::resolve_thread_mode(&client_config, &preferences);
    // Every unread count depends on this, so it is recorded once here rather
    // than re-derived per command.
    state.collapsed.store(
        thread_mode == ThreadMode::Collapsed,
        std::sync::atomic::Ordering::Relaxed,
    );

    let cached = {
        let store = store.clone();
        let me_id = me_id.clone();
        tokio::task::spawn_blocking(move || store.channels_with_unread(&me_id))
            .await
            .map_err(fail)?
            .map_err(fail)?
    };
    let from_cache = !cached.is_empty();
    tracing::info!(
        user = %username,
        thread_mode = ?thread_mode,
        from_cache,
        cached_channels = cached.len(),
        "bootstrapping"
    );

    // Every start, not just the first: preferences decide the thread mode, how
    // the sidebar groups, and which counters unread comes from. Storing them
    // only on a cold start left the app running on whatever they were the day
    // it was first launched.
    {
        let store = store.clone();
        let preferences = preferences.clone();
        tokio::task::spawn_blocking(move || store.upsert_preferences(&preferences))
            .await
            .map_err(fail)?
            .map_err(fail)?;
    }

    let summaries = if from_cache {
        cached
    } else {
        // First ever run: there is nothing local, so the sidebar has to wait.
        fetch_membership(&state, &me_id).await?
    };

    let display_mode = name_display_mode(&client_config);

    {
        let store = state.store.clone();
        let me = me.clone();
        tokio::task::spawn_blocking(move || store.upsert_users(&[me]))
            .await
            .map_err(fail)?
            .map_err(fail)?;
    }

    state
        .to_engine
        .send(EngineMsg::Start {
            me: Box::new(me),
            thread_mode,
        })
        .await
        .map_err(fail)?;
    // After `Start`, which is what creates the context this lands in.
    let _ = state
        .to_engine
        .send(EngineMsg::OwnStatus(own_status.clone()))
        .await;

    let channels = summarise(&state, &me_id, &display_mode, summaries).await?;
    Ok(Bootstrap {
        me_id,
        username,
        thread_mode: format!("{thread_mode:?}"),
        display_mode,
        from_cache,
        channels,
        max_file_size,
        own_status,
        connected: state.connected.load(std::sync::atomic::Ordering::Relaxed),
    })
}

/// Every channel and membership, from the server into the store.
///
/// Shared by the cold start and the post-cache refresh: a cached sidebar can be
/// a whole session old, so its unread and mention counts -- and the taskbar
/// badge derived from them -- have to be reconciled against the server.
async fn fetch_membership(
    state: &AppState,
    me_id: &str,
) -> std::result::Result<Vec<(matterless_core::Channel, matterless_store::Unread)>, String> {
    let rest = state.rest.clone();
    let teams = rest.my_teams().await.map_err(fail)?;
    let mut channels = Vec::new();
    let mut members = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for team in &teams {
        for channel in rest.my_channels(&team.id).await.map_err(fail)? {
            // DMs are returned for every team; dedupe by id.
            if channel.delete_at == 0 && seen.insert(channel.id.clone()) {
                channels.push(channel);
            }
        }
        members.extend(rest.my_channel_members(&team.id).await.map_err(fail)?);
    }
    let store = state.store.clone();
    let me_id = me_id.to_string();
    tokio::task::spawn_blocking(move || -> matterless_store::Result<_> {
        store.upsert_teams(&teams)?;
        store.upsert_channels(&channels)?;
        store.upsert_channel_members(&members)?;
        store.channels_with_unread(&me_id)
    })
    .await
    .map_err(fail)?
    .map_err(fail)
}

/// Turns stored channels into sidebar rows, resolving the people behind DMs.
async fn summarise(
    state: &AppState,
    me_id: &str,
    display_mode: &str,
    summaries: Vec<(matterless_core::Channel, matterless_store::Unread)>,
) -> std::result::Result<Vec<ChannelSummary>, String> {
    // A DM's label is the other person, so those users must be local first.
    let counterparts: Vec<String> = summaries
        .iter()
        .filter(|(channel, _)| channel.channel_type == "D")
        .filter_map(|(channel, _)| direct_message_counterpart(&channel.name, me_id))
        .collect();
    hydrate_users(state, counterparts.clone()).await?;
    let names = {
        let store = state.store.clone();
        let mode = display_mode.to_string();
        tokio::task::spawn_blocking(move || store.users_by_ids(&counterparts))
            .await
            .map_err(fail)?
            .map_err(fail)?
            .into_iter()
            .map(|(id, user)| (id, matterless_render::display_name(&user, &mode)))
            .collect::<HashMap<String, String>>()
    };

    let collapsed = state.collapsed.load(std::sync::atomic::Ordering::Relaxed);
    Ok(summaries
        .into_iter()
        .map(|(channel, unread)| {
            // Under collapsed threads a reply belongs to its thread, not to the
            // channel -- the sidebar was counting replies in threads the reader
            // does not even follow.
            let (messages, mentions) = unread.visible(collapsed);
            ChannelSummary {
                display_name: label_for_channel(&channel, me_id, &names),
                counterpart_id: if channel.channel_type == "D" {
                    direct_message_counterpart(&channel.name, me_id)
                } else {
                    None
                },
                id: channel.id,
                team_id: channel.team_id,
                channel_type: channel.channel_type,
                last_post_at: channel.last_post_at,
                unread: messages,
                mentions,
                muted: unread.muted,
            }
        })
        .collect())
}

/// One thread's rows: the root, then its replies.
///
/// Built from SQLite like everything else, so opening a thread paints without
/// waiting for the network; `refresh_thread` reconciles afterwards.
#[tauri::command]
pub async fn thread_rows(
    state: State<'_, AppState>,
    request: ThreadRowsRequest,
) -> Reply<ThreadPayload> {
    let ThreadRowsRequest {
        root_id,
        me_id,
        display_mode,
        utc_offset_minutes,
        full_res,
        pixel_ratio,
    } = request;

    let allow_svg = state.allow_svg.load(std::sync::atomic::Ordering::Relaxed);

    // Authors of the replies have to be resolvable before the plan is built.
    let store = state.store.clone();
    let probe = root_id.clone();
    let authors = tokio::task::spawn_blocking(move || -> matterless_store::Result<Vec<String>> {
        let replies = store.thread_replies(&probe)?;
        let mut ids: Vec<String> = replies.iter().map(|post| post.user_id.clone()).collect();
        // Reactors are named by the pills, and are often nobody's author here.
        ids.extend(
            replies
                .iter()
                .flat_map(|post| post.metadata.reactions.iter())
                .map(|reaction| reaction.user_id.clone()),
        );
        if let Some(root) = store.post(&probe)? {
            ids.extend(
                root.metadata
                    .reactions
                    .iter()
                    .map(|reaction| reaction.user_id.clone()),
            );
            ids.push(root.user_id);
        }
        ids.sort();
        ids.dedup();
        Ok(ids)
    })
    .await
    .map_err(fail)?
    .map_err(fail)?;
    hydrate_users(&state, authors.clone()).await?;

    let store = state.store.clone();
    let renders = state.renders.clone();
    let payload =
        tokio::task::spawn_blocking(move || -> matterless_store::Result<ThreadPayload> {
            let started = Instant::now();
            let Some(root) = store.post(&root_id)? else {
                return Ok(ThreadPayload {
                    root_id,
                    channel_id: String::new(),
                    rows: Vec::new(),
                    reply_count: 0,
                    unread_replies: 0,
                    following: false,
                    build_ms: started.elapsed().as_secs_f64() * 1000.0,
                });
            };
            let replies = store.thread_replies(&root_id)?;
            let state_row = store
                .thread_states_for(std::slice::from_ref(&root_id))?
                .remove(&root_id)
                .unwrap_or_default();

            // Same cache as the channel plan, keyed by post id and update_at: a
            // body already parsed for a footer's root is not parsed again here.
            let mut parsed = HashMap::new();
            for post in std::iter::once(&root).chain(replies.iter()) {
                let nodes = match renders.get(&post.id, post.update_at) {
                    Some(nodes) => nodes,
                    None => {
                        let nodes =
                            std::sync::Arc::new(matterless_render::markdown::parse(&post.message));
                        renders.put(&post.id, post.update_at, std::sync::Arc::clone(&nodes));
                        nodes
                    }
                };
                parsed.insert(post.id.clone(), nodes);
            }

            let mut options = PlanOptions::new(ThreadMode::Collapsed, &me_id);
            options.saved = store.saved_post_ids(&me_id)?;
            if state_row.following {
                options.followed.insert(root_id.clone());
            }
            options.utc_offset_minutes = utc_offset_minutes;
            options.full_res = full_res;
            options.pixel_ratio = pixel_ratio;
            options.allow_svg = allow_svg;
            // The thread's own watermark, so the divider lands above the first
            // reply this reader has not seen.
            options.last_viewed_at = state_row.last_viewed_at;
            options.parsed = parsed;
            options.author_names = store
                .users_by_ids(&authors)?
                .into_iter()
                .map(|(id, user)| (id, matterless_render::display_name(&user, &display_mode)))
                .collect();

            let rows = matterless_render::plan_thread(&root, &replies, &options);
            Ok(ThreadPayload {
                channel_id: root.channel_id.clone(),
                root_id,
                reply_count: replies.len() as i64,
                unread_replies: state_row.unread_replies,
                following: state_row.following,
                rows,
                build_ms: started.elapsed().as_secs_f64() * 1000.0,
            })
        })
        .await
        .map_err(fail)?
        .map_err(fail)?;

    tracing::debug!(
        root = %payload.root_id,
        rows = payload.rows.len(),
        build_ms = payload.build_ms,
        "built a thread plan"
    );
    Ok(payload)
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadRowsRequest {
    pub root_id: String,
    pub me_id: String,
    pub display_mode: String,
    pub utc_offset_minutes: i32,
    #[serde(default)]
    pub full_res: bool,
    #[serde(default = "one")]
    pub pixel_ratio: f32,
}

#[derive(Serialize)]
pub struct ThreadPayload {
    pub root_id: String,
    pub channel_id: String,
    pub rows: Vec<Row>,
    pub reply_count: i64,
    pub unread_replies: i64,
    pub following: bool,
    pub build_ms: f64,
}

/// Fetches a thread from the server and stores it.
///
/// Replies arrive as a backfill: they are older than nothing and newer than
/// nothing in particular, but they must never notify -- the notification for a
/// reply already fired when it arrived live.
#[tauri::command]
pub async fn refresh_thread(state: State<'_, AppState>, root_id: String) -> Reply<usize> {
    let list = state.rest.thread(&root_id).await.map_err(fail)?;
    let fetched = list.order.len();
    let me = state.rest.me().await.map_err(fail)?;
    let channel_id = list
        .posts
        .values()
        .next()
        .map(|post| post.channel_id.clone())
        .unwrap_or_default();

    let engine = state.engine.clone();
    tokio::task::spawn_blocking(move || {
        let context = SyncContext::new(me, ThreadMode::Collapsed);
        engine.apply_post_list(&channel_id, &list, Arrival::Backfill, &context, 0)
    })
    .await
    .map_err(fail)?
    .map_err(fail)?;

    tracing::info!(root = %root_id, fetched, "thread refreshed");
    Ok(fetched)
}

/// Marks a thread read up to its newest reply.
#[tauri::command]
pub async fn mark_thread_read(state: State<'_, AppState>, root_id: String) -> Reply<()> {
    let me = state.rest.me().await.map_err(fail)?;
    let team_id = team_for_thread(&state, &root_id).await?;

    // A server timestamp, never `now_ms()`: it is compared against reply
    // `create_at` afterwards, the same rule channel read state needed.
    let store = state.store.clone();
    let probe = root_id.clone();
    let newest = tokio::task::spawn_blocking(move || -> matterless_store::Result<i64> {
        Ok(store
            .thread_replies(&probe)?
            .iter()
            .map(|post| post.create_at)
            .max()
            .unwrap_or(0))
    })
    .await
    .map_err(fail)?
    .map_err(fail)?;
    if newest == 0 {
        return Ok(());
    }

    state
        .rest
        .mark_thread_read(&me.id, &team_id, &root_id, newest)
        .await
        .map_err(fail)?;
    let store = state.store.clone();
    let probe = root_id.clone();
    tokio::task::spawn_blocking(move || store.mark_thread_viewed(&probe, newest))
        .await
        .map_err(fail)?
        .map_err(fail)?;
    tracing::info!(root = %root_id, marked_at = newest, "thread marked read");
    Ok(())
}

/// Follows or unfollows a thread.
#[tauri::command]
pub async fn set_thread_following(
    state: State<'_, AppState>,
    root_id: String,
    following: bool,
) -> Reply<()> {
    let me = state.rest.me().await.map_err(fail)?;
    let team_id = team_for_thread(&state, &root_id).await?;
    state
        .rest
        .follow_thread(&me.id, &team_id, &root_id, following)
        .await
        .map_err(fail)?;
    let store = state.store.clone();
    let probe = root_id.clone();
    tokio::task::spawn_blocking(move || store.set_thread_following(&probe, following))
        .await
        .map_err(fail)?
        .map_err(fail)?;
    tracing::info!(root = %root_id, following, "thread follow changed");
    Ok(())
}

/// Which team to address a thread action to.
///
/// The route carries a team even for a thread in a DM, where the channel has no
/// team at all -- any team the user belongs to is accepted, so the first one is
/// the fallback rather than an error.
async fn team_for_thread(state: &AppState, root_id: &str) -> std::result::Result<String, String> {
    let store = state.store.clone();
    let probe = root_id.to_string();
    let channel_id = tokio::task::spawn_blocking(move || store.thread_channel(&probe))
        .await
        .map_err(fail)?
        .map_err(fail)?
        .unwrap_or_default();

    if !channel_id.is_empty() {
        let store = state.store.clone();
        let team = tokio::task::spawn_blocking(move || store.channel(&channel_id))
            .await
            .map_err(fail)?
            .map_err(fail)?
            .map(|channel| channel.team_id)
            .unwrap_or_default();
        if !team.is_empty() {
            return Ok(team);
        }
    }
    let teams = state.rest.my_teams().await.map_err(fail)?;
    teams
        .first()
        .map(|team| team.id.clone())
        .ok_or_else(|| "no team to address the thread action to".to_string())
}

/// Adds or removes the viewer's own reaction, optimistically.
///
/// Written to SQLite before the request goes out, because a reaction is the one
/// interaction where the delay is the whole experience -- and reverted if the
/// server refuses, since a reaction that silently did not happen is worse than
/// one that visibly failed.
#[tauri::command]
pub async fn toggle_reaction(
    state: State<'_, AppState>,
    post_id: String,
    emoji: String,
) -> Reply<bool> {
    let me = state.rest.me().await.map_err(fail)?;

    let store = state.store.clone();
    let probe = post_id.clone();
    let viewer = me.id.clone();
    let name = emoji.clone();
    let had = tokio::task::spawn_blocking(move || -> matterless_store::Result<bool> {
        Ok(store
            .post(&probe)?
            .map(|post| {
                post.metadata
                    .reactions
                    .iter()
                    .any(|reaction| reaction.user_id == viewer && reaction.emoji_name == name)
            })
            .unwrap_or(false))
    })
    .await
    .map_err(fail)?
    .map_err(fail)?;

    let adding = !had;
    // Optimistic: the row is rebuilt from the store, so this is what the reader
    // sees before the round trip finishes.
    apply_reaction(&state, &post_id, &me.id, &emoji, adding).await?;
    notify_reaction(&state, &post_id).await;

    let outcome = if adding {
        state
            .rest
            .add_reaction(&me.id, &post_id, &emoji)
            .await
            .map(|_| ())
    } else {
        state.rest.remove_reaction(&me.id, &post_id, &emoji).await
    };

    if let Err(error) = outcome {
        // Put it back the way it was, and say so: a reaction that quietly did
        // not happen leaves the reader believing something untrue.
        tracing::warn!(%error, post = %post_id, emoji = %emoji, "reaction rejected");
        apply_reaction(&state, &post_id, &me.id, &emoji, !adding).await?;
        notify_reaction(&state, &post_id).await;
        return Err(fail(error));
    }

    tracing::debug!(post = %post_id, emoji = %emoji, adding, "reaction toggled");
    Ok(adding)
}

async fn apply_reaction(
    state: &AppState,
    post_id: &str,
    user_id: &str,
    emoji: &str,
    adding: bool,
) -> std::result::Result<(), String> {
    let store = state.store.clone();
    let post_id = post_id.to_string();
    let user_id = user_id.to_string();
    let emoji = emoji.to_string();
    tokio::task::spawn_blocking(move || -> matterless_store::Result<bool> {
        if adding {
            store.add_reaction(&post_id, &user_id, &emoji, now_ms())
        } else {
            store.remove_reaction(&post_id, &user_id, &emoji)
        }
    })
    .await
    .map_err(fail)?
    .map_err(fail)?;
    Ok(())
}

/// Tells the shell the post's row changed, through the one render path.
async fn notify_reaction(state: &AppState, post_id: &str) {
    let store = state.store.clone();
    let probe = post_id.to_string();
    let channel = tokio::task::spawn_blocking(move || store.post(&probe))
        .await
        .ok()
        .and_then(|held| held.ok())
        .flatten()
        .map(|post| post.channel_id);
    if let Some(channel_id) = channel {
        let _ = state.to_engine.send(EngineMsg::Refreshed(channel_id)).await;
    }
}

/// The sidebar, grouped the way the reader arranged it on the server.
///
/// Fetches each team's categories, stores them, and returns the groups ready to
/// draw. Two things the raw API makes easy to get wrong:
///
/// * **the direct-messages category is returned per team**, holding the same
///   conversations each time, so those categories are merged into one group --
///   the same duplication that turns 114 channels into 172 if summed;
/// * **`sorting` differs per category** (`manual`, `recent`, or empty for the
///   server's alphabetical default), so each group is ordered on its own terms
///   rather than all one way.
#[tauri::command]
pub async fn sidebar(
    state: State<'_, AppState>,
    me_id: String,
    display_mode: String,
) -> Reply<SidebarPayload> {
    let teams = state.rest.my_teams().await.map_err(fail)?;
    for team in &teams {
        let categories = state
            .rest
            .sidebar_categories(&me_id, &team.id)
            .await
            .map_err(fail)?;
        let store = state.store.clone();
        let team_id = team.id.clone();
        let held = categories.categories.clone();
        tokio::task::spawn_blocking(move || store.upsert_sidebar(&team_id, &held))
            .await
            .map_err(fail)?
            .map_err(fail)?;
    }

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
    // The reader's own team order, which they set by dragging the team rail.
    // The API returns teams in its own order, so without this preference the
    // sidebar can disagree with every other client they use.
    let arranged = {
        let store = state.store.clone();
        let viewer = me_id.clone();
        tokio::task::spawn_blocking(move || store.preference(&viewer, "teams_order", ""))
            .await
            .map_err(fail)?
            .map_err(fail)?
            .unwrap_or_default()
    };
    let mut team_order: HashMap<String, usize> = arranged
        .split(',')
        .map(str::trim)
        .filter(|id| !id.is_empty())
        .enumerate()
        .map(|(index, id)| (id.to_string(), index))
        .collect();
    // Anything the preference does not mention keeps the API's order, after
    // the teams that were arranged.
    let mut next = team_order.len();
    for team in &teams {
        if !team_order.contains_key(&team.id) {
            team_order.insert(team.id.clone(), next);
            next += 1;
        }
    }

    // Every channel the reader is in, already carrying its unread and its
    // label, so grouping is a rearrangement rather than a second set of reads.
    let summaries = {
        let store = state.store.clone();
        let viewer = me_id.clone();
        tokio::task::spawn_blocking(move || store.channels_with_unread(&viewer))
            .await
            .map_err(fail)?
            .map_err(fail)?
    };
    let rows = summarise(&state, &me_id, &display_mode, summaries).await?;
    let by_id: HashMap<String, ChannelSummary> =
        rows.into_iter().map(|row| (row.id.clone(), row)).collect();

    let stored = {
        let store = state.store.clone();
        tokio::task::spawn_blocking(move || store.sidebar())
            .await
            .map_err(fail)?
            .map_err(fail)?
    };

    let mut groups: Vec<SidebarGroup> = Vec::new();
    let mut directs: Option<SidebarGroup> = None;
    let mut placed: std::collections::HashSet<String> = std::collections::HashSet::new();

    for (category, channel_ids) in stored {
        let mut channels: Vec<ChannelSummary> = channel_ids
            .iter()
            .filter_map(|id| by_id.get(id).cloned())
            .collect();
        sort_group(&mut channels, &category.sorting);

        if category.category_type == "direct_messages" {
            // Merged across teams: the same conversations, listed once.
            let group = directs.get_or_insert_with(|| SidebarGroup {
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
        groups.push(SidebarGroup {
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

    // Whether the reader groups unread channels separately. Reported rather
    // than applied: which channels are unread changes with every message and
    // every read, so the split belongs where that state lives -- applying it
    // here left a channel sitting under "Unreads" after it had been read,
    // until something else triggered a regroup.
    let separate_unreads = {
        let store = state.store.clone();
        let viewer = me_id.clone();
        tokio::task::spawn_blocking(move || {
            store.preference(&viewer, "sidebar_settings", "show_unread_section")
        })
        .await
        .map_err(fail)?
        .map_err(fail)?
        .as_deref()
            == Some("true")
    };

    // Anything the categories did not mention still has to be reachable: a
    // channel joined seconds ago is in no category until the server says so.
    let mut loose: Vec<ChannelSummary> = by_id
        .into_values()
        .filter(|channel| !placed.contains(&channel.id))
        .collect();
    if !loose.is_empty() {
        sort_group(&mut loose, "recent");
        tracing::debug!(count = loose.len(), "channels outside any category");
        groups.push(SidebarGroup {
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

    tracing::info!(
        groups = groups.len(),
        channels = groups
            .iter()
            .map(|group| group.channels.len())
            .sum::<usize>(),
        separate_unreads,
        teams_arranged = !arranged.is_empty(),
        order = groups
            .iter()
            .map(|group| group.display_name.as_str())
            .collect::<Vec<_>>()
            .join(" > "),
        "sidebar grouped"
    );
    Ok(SidebarPayload {
        groups,
        separate_unreads,
    })
}

/// The sidebar's server-side shape, plus how the reader wants it presented.
#[derive(Serialize)]
pub struct SidebarPayload {
    pub groups: Vec<SidebarGroup>,
    /// The reader groups unread channels separately. Applied in the shell,
    /// where "which channels are unread" is live.
    pub separate_unreads: bool,
}

/// Orders one group on the terms its category asks for.
fn sort_group(channels: &mut [ChannelSummary], sorting: &str) {
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

/// One drawn section of the sidebar.
#[derive(Serialize, Clone)]
pub struct SidebarGroup {
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

/// One category of standard emoji, ready to draw.
#[derive(Debug, Clone, Serialize)]
pub struct EmojiCategory {
    pub label: String,
    /// Name and the character it draws as, in the order a reader expects.
    pub emoji: Vec<(String, String)>,
}

/// The standard emoji, grouped for a picker.
///
/// Sent whole and once: 1805 names with their characters is a small payload,
/// and the alternative -- a request per category, or per scroll -- would put a
/// round trip between the reader and a grid of tiles. The custom emoji are not
/// here because the shell already holds them: `refresh_emoji` hands it the
/// entire catalogue at start-up.
#[tauri::command]
pub fn emoji_categories() -> Vec<EmojiCategory> {
    matterless_render::emoji::categories()
        .iter()
        .map(|(label, names)| EmojiCategory {
            label: (*label).to_string(),
            emoji: names
                .iter()
                .filter_map(|name| {
                    matterless_render::emoji::character_for(name)
                        .map(|face| ((*name).to_string(), face))
                })
                .collect(),
        })
        .collect()
}

/// Fetches every custom emoji the server has and remembers them.
///
/// The complete list, rather than resolving names as they turn up in messages:
/// a scan of message text only ever finds what someone has already typed, so a
/// new custom emoji would render as `:name:` the first time it was used. Paged
/// until a short page arrives, which is how the endpoint says it is done.
#[tauri::command]
pub async fn refresh_emoji(state: State<'_, AppState>) -> Reply<HashMap<String, String>> {
    const PER_PAGE: u32 = 200;
    let mut page = 0u32;
    let mut all: Vec<(String, String)> = Vec::new();

    loop {
        let batch = state
            .rest
            .custom_emoji_page(page, PER_PAGE)
            .await
            .map_err(fail)?;
        let count = batch.len();
        all.extend(
            batch
                .into_iter()
                .filter(|emoji| !emoji.name.is_empty())
                .map(|emoji| (emoji.name, emoji.id)),
        );
        // A short page is the last page.
        if (count as u32) < PER_PAGE {
            break;
        }
        page += 1;
        // A server with an unbounded set must not turn a start-up into a
        // crawl; whatever is past this resolves by name on demand.
        if page > EMOJI_PAGES {
            tracing::warn!(pages = page, "stopped paging custom emoji");
            break;
        }
    }

    let store = state.store.clone();
    let held = all.clone();
    tokio::task::spawn_blocking(move || -> matterless_store::Result<()> {
        for (name, id) in &held {
            store.remember_emoji(name, id)?;
        }
        Ok(())
    })
    .await
    .map_err(fail)?
    .map_err(fail)?;

    tracing::info!(custom = all.len(), "custom emoji catalogued");
    // The whole map goes back, not just the count: the shell learns custom
    // emoji from page payloads, and a payload only carries the names that page
    // happened to scan -- so a reply in a thread pane, or a page built before
    // this catalogue landed, would render `:name:` for an emoji the store knew
    // perfectly well. 705 pairs is nothing to hand over once.
    Ok(all.into_iter().collect())
}

/// Pages of custom emoji to read at startup: 200 each, so this is a ceiling of
/// 2000 on a set that is 200-ish here.
const EMOJI_PAGES: u32 = 10;

/// Re-reads the threads this user follows, across every team.
///
/// **Deduped by thread id**: a thread in a DM or group channel is returned for
/// *every* team, so summing the per-team lists double-counts it -- the same trap
/// that once turned 114 channels into 172.
#[tauri::command]
pub async fn refresh_threads(state: State<'_, AppState>, me_id: String) -> Reply<ThreadTotals> {
    let teams = state.rest.my_teams().await.map_err(fail)?;
    let mut threads: Vec<matterless_core::model::UserThread> = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let mut unread_threads = 0i64;
    let mut unread_mentions = 0i64;

    for team in &teams {
        let page = state
            .rest
            .my_threads(&me_id, &team.id, THREADS_WANTED, false)
            .await
            .map_err(fail)?;
        unread_threads += page.total_unread_threads;
        unread_mentions += page.total_unread_mentions;
        for thread in page.threads {
            if seen.insert(thread.id.clone()) {
                threads.push(thread);
            }
        }
    }

    // The roots themselves are posts: storing them is what lets a followed
    // thread be listed for a channel whose history was never opened.
    let roots: Vec<matterless_core::Post> =
        threads.iter().map(|thread| thread.post.clone()).collect();
    let store = state.store.clone();
    let held = threads.clone();
    tokio::task::spawn_blocking(move || -> matterless_store::Result<()> {
        store.upsert_posts(&roots)?;
        store.upsert_threads(&held)?;
        Ok(())
    })
    .await
    .map_err(fail)?
    .map_err(fail)?;

    let totals = ThreadTotals {
        followed: threads.len(),
        unread_threads,
        unread_mentions,
    };
    tracing::info!(
        followed = totals.followed,
        unread_threads,
        unread_mentions,
        "threads refreshed"
    );
    Ok(totals)
}

/// How many followed threads there are, and how much of them is unread.
#[derive(Serialize)]
pub struct ThreadTotals {
    pub followed: usize,
    pub unread_threads: i64,
    pub unread_mentions: i64,
}

/// Emoji names to resolve per build. Each is one request, so this bounds what a
/// channel full of unfamiliar names can cost a single render.
const EMOJI_LOOKUPS: usize = 12;

/// Followed threads to read per team. The server's own tab pages at 25; this is
/// generous because the whole list feeds footer counts, not just a view.
const THREADS_WANTED: u32 = 200;

/// The second half of "paint from SQLite, then refresh": re-reads every
/// membership from the server.
///
/// Without this the cached sidebar keeps last session's counts until each
/// channel is opened, which is how the badge came to read 0 while the official
/// client read 1 -- the unread DM was never in the local counters at all.
#[tauri::command]
pub async fn refresh_membership(
    state: State<'_, AppState>,
    me_id: String,
    display_mode: String,
) -> Reply<Vec<ChannelSummary>> {
    let summaries = fetch_membership(&state, &me_id).await?;
    let channels = summarise(&state, &me_id, &display_mode, summaries).await?;
    tracing::info!(channels = channels.len(), "membership refreshed");
    Ok(channels)
}

/// Which name form the server asks clients to show.
fn name_display_mode(client_config: &HashMap<String, String>) -> String {
    client_config
        .get("TeammateNameDisplay")
        .cloned()
        .unwrap_or_else(|| "username".to_string())
}

/// What to put in the sidebar. A DM has no display name of its own.
fn label_for_channel(
    channel: &matterless_core::Channel,
    me_id: &str,
    names: &HashMap<String, String>,
) -> String {
    if !channel.display_name.is_empty() {
        return channel.display_name.clone();
    }
    if channel.channel_type == "D" {
        let counterpart = direct_message_counterpart(&channel.name, me_id);
        return match counterpart {
            Some(id) if id == me_id => "You".to_string(),
            Some(id) => names.get(&id).cloned().unwrap_or(id),
            None => channel.name.clone(),
        };
    }
    // Group DMs fall back to their generated name until participants are named.
    channel.name.clone()
}

/// Fetches any of these users we do not already hold, in one request.
///
/// An author or a DM counterpart has to be resolvable to a name, and doing that
/// one user at a time is the fastest way to hit the rate limit -- Phase 0 found
/// no `X-RateLimit-*` headers, so there is no way to adapt if we do.
/// Fetches the profiles this page named but does not have, off the render path.
///
/// Then asks the shell to rebuild the channel, so the ids the plan fell back on
/// are replaced by names. Nothing is rebuilt when nobody was missing, which is
/// the usual case: bootstrap has already cached the channel's members, and this
/// only bites for someone who has left, reacted from outside the channel, or
/// been quoted from another one.
fn spawn_hydration(state: &AppState, channel_id: String, ids: Vec<String>) {
    if ids.is_empty() {
        return;
    }
    let store = state.store.clone();
    let rest = state.rest.clone();
    let to_engine = state.to_engine.clone();
    tauri::async_runtime::spawn(async move {
        let started = Instant::now();
        let probe = store.clone();
        let missing = match tokio::task::spawn_blocking(move || probe.missing_user_ids(&ids)).await
        {
            Ok(Ok(missing)) => missing,
            Ok(Err(error)) => {
                tracing::warn!(%error, "could not tell which users are missing");
                return;
            }
            Err(error) => {
                tracing::warn!(%error, "the missing-user check did not finish");
                return;
            }
        };
        if missing.is_empty() {
            return;
        }
        let wanted = missing.len();
        let fetched = match rest.users_by_ids(&missing).await {
            Ok(fetched) => fetched,
            Err(error) => {
                tracing::warn!(%error, wanted, "could not fetch missing users");
                return;
            }
        };
        let got = fetched.len();
        if let Err(error) = tokio::task::spawn_blocking(move || store.upsert_users(&fetched)).await
        {
            tracing::warn!(%error, "storing fetched users did not finish");
            return;
        }
        tracing::info!(
            channel = %channel_id,
            wanted,
            got,
            ms = started.elapsed().as_secs_f64() * 1000.0,
            "hydrated users behind the page"
        );
        // Only now is a rebuild worth what it costs.
        if let Err(error) = to_engine.send(EngineMsg::Refreshed(channel_id)).await {
            tracing::warn!(%error, "could not ask for a rebuild after hydrating");
        }
    });
}

/// Fills in any of `ids` the local store does not know yet.
///
/// Returns how many had to be fetched from the server, which is the difference
/// between a cheap local check and a network round trip on the path that draws
/// the channel.
async fn hydrate_users(state: &AppState, ids: Vec<String>) -> Result<usize, String> {
    if ids.is_empty() {
        return Ok(0);
    }
    let store = state.store.clone();
    let probe = ids.clone();
    let missing = tokio::task::spawn_blocking(move || store.missing_user_ids(&probe))
        .await
        .map_err(fail)?
        .map_err(fail)?;
    if missing.is_empty() {
        return Ok(0);
    }
    let wanted = missing.len();
    let fetched = state.rest.users_by_ids(&missing).await.map_err(fail)?;
    let store = state.store.clone();
    tokio::task::spawn_blocking(move || store.upsert_users(&fetched))
        .await
        .map_err(fail)?
        .map_err(fail)?;
    Ok(wanted)
}

/// A direct message has no display name of its own: the name is
/// `<userA>__<userB>`, and what a person expects to see is the other party.
fn direct_message_counterpart(channel_name: &str, me_id: &str) -> Option<String> {
    let (left, right) = channel_name.split_once("__")?;
    if left == right {
        // A note-to-self channel: both halves are you.
        return Some(me_id.to_string());
    }
    Some(if left == me_id { right } else { left }.to_string())
}

fn thread_mode_from(text: &str) -> ThreadMode {
    match text {
        "Collapsed" => ThreadMode::Collapsed,
        "Disabled" => ThreadMode::Disabled,
        _ => ThreadMode::Flat,
    }
}

/// A **window** of the channel's rows, built in Rust and read from SQLite only.
///
/// Deliberately a window rather than everything held. Growing the read without
/// bound made a plain scroll-back reach 1261 rows, where rebuilding the array on
/// every delta cost ~50 ms in Rust and ~142 ms of paint -- so a single incoming
/// message while scrolled deep cost nearly 200 ms. A fixed window keeps both
/// flat however far back the reader has gone, and scrolling moves the window
/// instead of extending it.
///
/// `anchor` is the newest `create_at` to include; `None` means the newest posts.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RowsRequest {
    pub channel_id: String,
    pub me_id: String,
    pub thread_mode: String,
    pub display_mode: String,
    /// Minutes east of UTC, so day boundaries land where the viewer sees them.
    pub utc_offset_minutes: i32,
    /// Exclusive upper bound on `create_at`: a page holds the newest posts
    /// older than this. `None` asks for the newest page in the channel.
    pub anchor: Option<i64>,
    /// Inclusive lower bound, which is what pins the newest page in place while
    /// messages arrive into it. `None` means "whatever `limit` reaches".
    pub since: Option<i64>,
    /// Where the "New messages" divider goes: `last_viewed_at` as captured when
    /// the channel was opened. Zero means no divider.
    pub unread_since: i64,
    /// Posts per page. Fixed: a page is rebuilt when its newest post changes,
    /// and only the newest page ever changes, so this bounds the work a live
    /// message costs however far back the reader has scrolled.
    pub limit: u32,
    /// Draw a lone image from the file as uploaded rather than the server's
    /// preview rendition -- the reader's setting, so it travels with the
    /// request and the plan is rebuilt when it changes.
    #[serde(default)]
    pub full_res: bool,
    /// The viewer's `devicePixelRatio`. Defaulted rather than required so an
    /// older shell still gets a plan.
    #[serde(default = "one")]
    pub pixel_ratio: f32,
}

/// The default pixel ratio: one image pixel per device pixel.
fn one() -> f32 {
    1.0
}

#[tauri::command]
pub async fn channel_rows(state: State<'_, AppState>, request: RowsRequest) -> Reply<RowsPayload> {
    let RowsRequest {
        channel_id,
        me_id,
        thread_mode,
        display_mode,
        utc_offset_minutes,
        anchor,
        since,
        unread_since,
        limit,
        full_res,
        pixel_ratio,
    } = request;
    // Under collapsed threads a reply is never a row, so it must not be read.
    let roots_only = thread_mode_from(&thread_mode) == ThreadMode::Collapsed;
    let allow_svg = state.allow_svg.load(std::sync::atomic::Ordering::Relaxed);

    // Everyone whose name this page needs, resolvable before the plan is built:
    // the authors of its posts, and the people in its threads -- a footer shows
    // their faces, and they are usually not authors of anything in the window.
    //
    // Both reads happen inside the one blocking task: SQLite work never runs on
    // the async thread.
    // Timed because they sit *outside* the plan's own `build_ms`, which is what
    // the shell measures against: a channel open showed 28ms of unexplained
    // round trip, and one showed 6.6 seconds.
    let authors_at = Instant::now();
    let (posts, preceding, known_authors, page_ms) = {
        let store = state.store.clone();
        let probe = channel_id.clone();
        type Prepared = (
            Vec<matterless_core::model::Post>,
            Option<matterless_render::Preceding>,
            Vec<String>,
            f64,
        );
        tokio::task::spawn_blocking(move || -> matterless_store::Result<Prepared> {
            let read_at = Instant::now();
            let posts = store.channel_page_filtered(&probe, anchor, since, limit, roots_only)?;
            // What sits directly above this page, so a run of messages and a
            // day both survive the seam.
            let preceding = match posts.iter().map(|post| post.create_at).min() {
                Some(oldest) => store.post_above(&probe, oldest, roots_only)?.map(|post| {
                    matterless_render::Preceding {
                        user_id: post.user_id,
                        create_at: post.create_at,
                    }
                }),
                None => None,
            };
            let page_ms = read_at.elapsed().as_secs_f64() * 1000.0;
            let mut ids: Vec<String> = posts.iter().map(|post| post.user_id.clone()).collect();
            // Whoever reacted, too: a pill names them, and reacting to a post
            // does not make somebody an author of anything on this page.
            ids.extend(
                posts
                    .iter()
                    .flat_map(|post| post.metadata.reactions.iter())
                    .map(|reaction| reaction.user_id.clone()),
            );
            // And whoever wrote a message quoted by a permalink preview: the
            // preview names them, and they are usually nobody's author here.
            ids.extend(
                posts
                    .iter()
                    .flat_map(|post| post.metadata.embeds.iter())
                    .filter(|embed| embed.embed_type == "permalink")
                    .filter_map(|embed| {
                        embed
                            .data
                            .get("post")
                            .and_then(|quoted| quoted.get("user_id"))
                            .and_then(serde_json::Value::as_str)
                            .map(str::to_string)
                    }),
            );
            let roots: Vec<String> = posts
                .iter()
                .filter(|post| post.root_id.is_empty())
                .map(|post| post.id.clone())
                .collect();
            ids.extend(store.thread_participant_ids(&roots)?);
            ids.sort();
            ids.dedup();
            // The page itself travels on: reading it a second time inside the
            // plan build cost more than the build did -- measured, 8.3ms of
            // author collection against 6.7ms to build the whole plan.
            Ok((posts, preceding, ids, page_ms))
        })
        .await
        .map_err(fail)?
        .map_err(fail)?
    };
    let authors_ms = authors_at.elapsed().as_secs_f64() * 1000.0;

    // Anyone this page names but the store has never seen is fetched *after* the
    // plan is built, not before.
    //
    // The fetch is a network round trip, and it used to sit on the path that
    // draws the channel: measured, one open spent 6.6 seconds waiting for it
    // while the plan itself took 3.6ms. Every place the plan needs a name falls
    // back to that person's id when it has none, so the page can be drawn now
    // and corrected a moment later.
    spawn_hydration(&state, channel_id.clone(), known_authors.clone());

    // Optimistic sends live in memory and are merged here, so they reach the
    // screen through the same plan as everything else rather than a second path.
    // Only the newest page can hold them: a send is always the newest thing in
    // the channel, and merging it into every page would show it repeatedly.
    let newest_page = anchor.is_none();
    let outstanding = if newest_page {
        state.pending.for_channel(&channel_id)
    } else {
        Vec::new()
    };

    let store = state.store.clone();
    let renders = state.renders.clone();
    let target = channel_id.clone();
    let name_mode = display_mode.clone();
    let author = me_id.clone();

    // The payload, plus the emoji names nobody has asked about yet: they are
    // found inside this closure but resolved outside it, because a plan build
    // must never wait on a request.
    let (payload, emoji_misses) = tokio::task::spawn_blocking(
        move || -> matterless_store::Result<(RowsPayload, Vec<String>)> {
            let started = Instant::now();
            // Read once, above, and handed in.
            let mut timings = Timings {
                page_ms,
                ..Default::default()
            };

            // Only roots in this window can grow a footer, so only they are counted.
            let root_ids: Vec<String> = posts
                .iter()
                .filter(|post| post.root_id.is_empty())
                .map(|post| post.id.clone())
                .collect();

            let threads_phase = Instant::now();
            // Supplied by the caller, held for the visit. This was stubbed to 0
            // for a long time, which is why the divider row -- built, tested and
            // ready -- had never once appeared on screen.
            let member_viewed = unread_since;
            let mut summaries: HashMap<String, ThreadSummary> = store
                .thread_summaries_for(&target, &root_ids)?
                .into_iter()
                .map(|(root_id, (reply_count, last_reply_at, participants))| {
                    (
                        root_id,
                        ThreadSummary {
                            reply_count,
                            last_reply_at,
                            participants,
                            ..Default::default()
                        },
                    )
                })
                .collect();

            // Read state is the server's, and it can describe a thread whose
            // replies are not held locally at all -- so a state without a local
            // summary still earns an entry, or the footer that explains an unread
            // badge would never be built.
            for (root_id, state) in store.thread_states_for(&root_ids)? {
                let summary = summaries.entry(root_id).or_default();
                summary.reply_count = summary.reply_count.max(state.reply_count);
                summary.last_reply_at = summary.last_reply_at.max(state.last_reply_at);
                summary.unread_replies = state.unread_replies;
                summary.unread_mentions = state.unread_mentions;
                summary.following = state.following;
            }
            timings.threads_ms = threads_phase.elapsed().as_secs_f64() * 1000.0;

            // Parsed bodies from the in-process cache; anything missing is parsed
            // once and kept. A post's markdown cannot change without `update_at`
            // moving, so a hit is never stale.
            let cache_phase = Instant::now();
            let mut parsed: HashMap<String, std::sync::Arc<Vec<Node>>> = HashMap::new();
            let mut hits = 0usize;
            let mut misses = 0usize;
            let mut parse_ms = 0.0f64;

            for post in &posts {
                if let Some(nodes) = renders.get(&post.id, post.update_at) {
                    hits += 1;
                    parsed.insert(post.id.clone(), nodes);
                    continue;
                }
                let parse_phase = Instant::now();
                let nodes = std::sync::Arc::new(matterless_render::markdown::parse(&post.message));
                parse_ms += parse_phase.elapsed().as_secs_f64() * 1000.0;
                misses += 1;
                renders.put(&post.id, post.update_at, std::sync::Arc::clone(&nodes));
                parsed.insert(post.id.clone(), nodes);
            }
            timings.cache_ms = cache_phase.elapsed().as_secs_f64() * 1000.0 - parse_ms;
            timings.parse_ms = parse_ms;
            timings.cache_hits = hits;
            timings.cache_misses = misses;

            let names_phase = Instant::now();
            // The per-message actions need three states the posts do not carry:
            // saved is a preference, and following belongs to the thread rather than
            // to the message. Both come from what this build already read.
            let followed: std::collections::HashSet<String> = summaries
                .iter()
                .filter(|(_, summary)| summary.following)
                .map(|(root_id, _)| root_id.clone())
                .collect();
            let saved = store.saved_post_ids(&me_id)?;
            let mut options = PlanOptions::new(thread_mode_from(&thread_mode), &me_id);
            options.saved = saved;
            options.followed = followed;
            options.preceding = preceding;
            options.utc_offset_minutes = utc_offset_minutes;
            options.full_res = full_res;
            options.pixel_ratio = pixel_ratio;
            options.allow_svg = allow_svg;
            options.last_viewed_at = member_viewed;
            options.parsed = parsed;
            let authors = store.users_by_ids(&known_authors)?;
            options.author_avatars = authors
                .iter()
                .map(|(id, user)| (id.clone(), user.last_picture_update))
                .collect();
            options.author_names = authors
                .into_iter()
                .map(|(id, user)| (id, matterless_render::display_name(&user, &name_mode)))
                .collect();
            timings.names_ms = names_phase.elapsed().as_secs_f64() * 1000.0;

            for outstanding_post in &outstanding {
                options
                    .pending
                    .insert(outstanding_post.pending_post_id.clone());
                if outstanding_post.failed {
                    options
                        .failed
                        .insert(outstanding_post.pending_post_id.clone());
                }
            }
            let mut posts = posts;
            posts.extend(
                outstanding
                    .iter()
                    .map(|outstanding_post| outstanding_post.as_post(&author)),
            );

            let plan_phase = Instant::now();
            let rows = plan_channel(&posts, &summaries, &options);
            timings.plan_ms = plan_phase.elapsed().as_secs_f64() * 1000.0;

            // Custom emoji for this page. The store answers for names already
            // asked about -- including the ones that turned out to be standard --
            // and `emoji_misses` carries the rest out for the network to resolve
            // after the plan is built, so a build never waits on a request.
            let mut candidates: Vec<String> = posts
                .iter()
                .flat_map(|post| matterless_render::emoji::custom_candidates(&post.message))
                .collect();
            for post in &posts {
                candidates.extend(
                    post.metadata
                        .reactions
                        .iter()
                        .map(|reaction| reaction.emoji_name.clone()),
                );
            }
            candidates.sort();
            candidates.dedup();
            let known = store.known_emoji(&candidates)?;
            let emoji: HashMap<String, String> = known
                .iter()
                .filter(|(_, id)| !id.is_empty())
                .map(|(name, id)| (name.clone(), id.clone()))
                .collect();
            let emoji_misses: Vec<String> = candidates
                .into_iter()
                .filter(|name| !known.contains_key(name))
                .collect();

            let oldest_in_window = posts.iter().map(|post| post.create_at).min();
            let newest_in_window = posts.iter().map(|post| post.create_at).max();
            Ok((
                RowsPayload {
                    channel_id: target,
                    window_full: posts.len() as u32 >= limit,
                    oldest_in_window,
                    newest_in_window,
                    rows,
                    emoji,
                    build_ms: started.elapsed().as_secs_f64() * 1000.0,
                    timings,
                },
                emoji_misses,
            ))
        },
    )
    .await
    .map_err(fail)?
    .map_err(fail)?;

    // Names nobody has asked about yet, resolved after the plan is built and
    // remembered either way. Bounded per build: a channel full of unknown names
    // must not turn one render into fifty requests, and whatever is left over
    // resolves on the next one.
    if !emoji_misses.is_empty() {
        let asking: Vec<String> = emoji_misses.into_iter().take(EMOJI_LOOKUPS).collect();
        let mut resolved = 0usize;
        for name in &asking {
            let found = state.rest.emoji_by_name(name).await.ok().flatten();
            let id = found.map(|emoji| emoji.id).unwrap_or_default();
            if !id.is_empty() {
                resolved += 1;
            }
            let store = state.store.clone();
            let name = name.clone();
            let _ = tokio::task::spawn_blocking(move || store.remember_emoji(&name, &id)).await;
        }
        // The names the server does not know as custom are either scanner
        // artefacts (`12:53:` yields `53`) or standard emoji newer than the
        // generated table -- which is the one way a real emoji can still render
        // as `:name:`. Logged by name so the difference is decidable: if these
        // read like emoji after a server upgrade, re-run
        // `tools/generate_emoji_table.py`.
        let unmatched: Vec<&str> = asking
            .iter()
            .filter(|name| matterless_render::emoji::codepoints_for(name).is_none())
            .map(String::as_str)
            .collect();
        tracing::debug!(
            asked = asking.len(),
            resolved,
            unmatched = unmatched.join(","),
            "resolved custom emoji names"
        );
        // The plan the reader is looking at was built without them, so it is
        // rebuilt once they are known -- through the one render path.
        if resolved > 0 {
            let _ = state
                .to_engine
                .send(EngineMsg::Refreshed(channel_id.clone()))
                .await;
        }
    }

    // A histogram of row kinds is what makes the log a substitute for a
    // screenshot: it says what the message list actually contains.
    let mut kinds: HashMap<&str, usize> = HashMap::new();
    for row in &payload.rows {
        let kind = match row {
            Row::DateSeparator { .. } => "sep",
            Row::UnreadDivider => "unread",
            Row::Post { post } if post.failed => "post_failed",
            Row::Post { post } if post.pending => "post_pending",
            Row::Post { .. } => "post",
            Row::Continuation { .. } => "cont",
            Row::System { .. } => "system",
            Row::DeletedRoot { .. } => "deleted_root",
            Row::ThreadFooter { .. } => "footer",
        };
        *kinds.entry(kind).or_insert(0) += 1;
    }
    let mut summary: Vec<String> = kinds
        .into_iter()
        .map(|(kind, count)| format!("{kind}={count}"))
        .collect();
    summary.sort();

    // Why a divider was or was not placed. Under collapsed threads an unread
    // *reply* is never a row, so a channel can carry a real unread count with
    // nothing in its stream to mark -- and that is indistinguishable from a
    // broken divider without these two numbers.
    let newest_in_stream = payload
        .rows
        .iter()
        .filter_map(|row| match row {
            Row::Post { post } | Row::Continuation { post } => Some(post.create_at),
            _ => None,
        })
        .max()
        .unwrap_or(0);
    let has_divider = payload
        .rows
        .iter()
        .any(|row| matches!(row, Row::UnreadDivider));
    tracing::info!(
        channel = %channel_id,
        rows = payload.rows.len(),
        roots_only,
        build_ms = payload.build_ms,
        // The page read and author collection, which sit outside the plan's own
        // timings and were once half the cost of an open.
        authors_ms,
        page_ms = payload.timings.page_ms,
        threads_ms = payload.timings.threads_ms,
        names_ms = payload.timings.names_ms,
        cache_ms = payload.timings.cache_ms,
        cached_bodies = state.renders.len(),
        parse_ms = payload.timings.parse_ms,
        plan_ms = payload.timings.plan_ms,
        hits = payload.timings.cache_hits,
        misses = payload.timings.cache_misses,
        kinds = %summary.join(" "),
        unread_since,
        newest_in_stream,
        has_divider,
        stream_has_newer = newest_in_stream > unread_since,
        // Which rendition the plan reserved boxes for, and what the display
        // does: both decide the box, so "why is that image that size" is
        // answerable from the log rather than by guessing.
        full_res,
        pixel_ratio,
        "built a row plan"
    );

    Ok(payload)
}

/// Marks a channel active, fills thin history, and reports where the
/// "New messages" divider belongs.
///
/// Returns `last_viewed_at` as it stands *now*, before anything marks the
/// channel read. The shell holds that value for the visit so the divider stays
/// put: marking read moves `last_viewed_at` to the present, and recomputing
/// from the live value would erase the line while it is still being read.
/// Deliberately does no fetching. It used to page history in before returning,
/// which meant the delta from that fetch could trigger a render while the caller
/// still had no divider anchor -- rendering the channel without its
/// "New messages" line. The caller now renders first and refreshes after.
#[tauri::command]
pub async fn open_channel(state: State<'_, AppState>, channel_id: String) -> Reply<ChannelOpened> {
    state
        .to_engine
        .send(EngineMsg::SetActiveChannel(Some(channel_id.clone())))
        .await
        .map_err(fail)?;

    let me = state.rest.me().await.map_err(fail)?;
    let divider_at = {
        let store = state.store.clone();
        let target = channel_id.clone();
        let viewer = me.id.clone();
        tokio::task::spawn_blocking(move || store.last_viewed_at(&target, &viewer))
            .await
            .map_err(fail)?
            .map_err(fail)?
            .unwrap_or(0)
    };

    // "Not enough to fill the stream", not "nothing at all": under collapsed
    // threads most posts are replies and never become rows, so a channel holding
    // 60 posts can still show a dozen -- and an emptiness test would leave it
    // that way permanently.
    let store = state.store.clone();
    let probe = channel_id.clone();
    let roots = tokio::task::spawn_blocking(move || store.root_count(&probe))
        .await
        .map_err(fail)?
        .map_err(fail)?;

    // Two separate questions, and only the first used to be asked.
    let thin = (roots as usize) < ROOTS_WANTED;
    let stale = state
        .reconciled
        .lock()
        .expect("reconciled")
        .get(&channel_id)
        .is_none_or(|at| at.elapsed() > RECONCILE_AFTER);
    tracing::info!(channel = %channel_id, roots, thin, stale, "channel opened");

    Ok(ChannelOpened {
        divider_at,
        needs_refresh: thin || stale,
    })
}

/// How long a channel's history is trusted after a reconcile.
///
/// Long enough that switching back and forth costs nothing, short enough that
/// a visit heals a hole. One reconcile is a single 200-post request, which is
/// what the official client does on every channel switch.
const RECONCILE_AFTER: std::time::Duration = std::time::Duration::from_secs(60);

/// Marks a channel read, locally and on the server.
///
/// Called after the reader has had the channel in front of them for a few
/// focused seconds -- not on open. The delay is what makes the unread state
/// worth showing at all, and counting only focused time means a window left in
/// the background never silently clears anything.
#[tauri::command]
pub async fn mark_read(state: State<'_, AppState>, channel_id: String) -> Reply<()> {
    let me = state.rest.me().await.map_err(fail)?;
    if let Err(error) = state.rest.view_channel(&me.id, &channel_id).await {
        // Not worth surfacing: the next view attempt will try again.
        tracing::warn!(channel = %channel_id, %error, "mark read failed");
        return Ok(());
    }
    let store = state.store.clone();
    let target = channel_id.clone();
    let viewer = me.id.clone();
    let marked_at = tokio::task::spawn_blocking(move || -> matterless_store::Result<i64> {
        // Server time, not `now_ms()`: this value is later compared against post
        // `create_at`, and mixing clocks would eventually hide the divider for
        // good on a machine whose clock runs fast.
        let watermark = store.newest_post_at(&target)?;
        store.mark_channel_viewed(&target, &viewer, watermark)?;
        Ok(watermark)
    })
    .await
    .map_err(fail)?
    .map_err(fail)?;
    tracing::info!(channel = %channel_id, marked_at, "marked read");
    notify_unread(&state, &channel_id, &me.id).await;
    Ok(())
}

/// Roots wanted on screen before paging stops.
const ROOTS_WANTED: usize = 40;
/// At ~53 ms a page, three pages is the most worth spending on an open.
const MAX_PAGES: usize = 3;

/// Pulls a channel's recent history over REST and folds it into the store.
///
/// Pages until there are enough *roots*, not enough posts: 84% of posts here are
/// replies, and under collapsed threads a reply never enters the stream. One
/// 60-post page yielded 14 visible rows, which reads as a nearly empty channel.
#[tauri::command]
pub async fn refresh(state: State<'_, AppState>, channel_id: String) -> Reply<()> {
    let me = state.rest.me().await.map_err(fail)?;
    let mut before: Option<String> = None;
    // Recorded up front rather than on the way out: a fetch that fails should
    // not be retried on every keystroke-fast channel switch either.
    state
        .reconciled
        .lock()
        .expect("reconciled")
        .insert(channel_id.clone(), std::time::Instant::now());

    for page in 0..MAX_PAGES {
        let list = match &before {
            None => state.rest.posts(&channel_id, 200).await.map_err(fail)?,
            Some(cursor) => state
                .rest
                .posts_before(&channel_id, cursor, 200)
                .await
                .map_err(fail)?,
        };
        let oldest = list.oldest_in_order().map(|post| post.id.clone());
        let exhausted = list.order.is_empty() || list.prev_post_id.is_empty();

        let engine = state.engine.clone();
        let target = channel_id.clone();
        let me_for_page = me.clone();
        tokio::task::spawn_blocking(move || {
            // A backfill, so nothing here can notify whatever it says.
            let context = SyncContext::new(me_for_page, ThreadMode::Flat);
            engine.apply_post_list(&target, &list, Arrival::Backfill, &context, 0)
        })
        .await
        .map_err(fail)?
        .map_err(fail)?;

        let store = state.store.clone();
        let target = channel_id.clone();
        let roots = tokio::task::spawn_blocking(move || store.root_count(&target))
            .await
            .map_err(fail)?
            .map_err(fail)?;
        tracing::debug!(page, roots, "backfilled a page");

        if roots as usize >= ROOTS_WANTED || exhausted {
            break;
        }
        before = oldest;
        if before.is_none() {
            break;
        }
    }

    state
        .to_engine
        .send(EngineMsg::Refreshed(channel_id))
        .await
        .map_err(fail)?;
    Ok(())
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as i64)
        .unwrap_or(0)
}

/// Posts a message, showing it immediately and reconciling afterwards.
///
/// The optimistic row is written to memory only, never to SQLite, so a failed
/// send leaves the database exactly as it was. On success the server's own copy
/// goes into the store and the pending entry is dropped; the websocket echo that
/// follows is then a no-op, because the store recognises the post it already
/// holds. That is the dedup proven live in the Phase 1 soak doing real work.
#[tauri::command]
pub async fn send_post(
    state: State<'_, AppState>,
    channel_id: String,
    message: String,
    root_id: Option<String>,
    file_ids: Option<Vec<String>>,
) -> Reply<String> {
    let trimmed = message.trim();
    // A post with attachments and no words is perfectly ordinary -- dropping an
    // image in is the common case -- so emptiness only disqualifies a post that
    // carries nothing at all.
    let attached: Vec<String> = file_ids.unwrap_or_default();
    if trimmed.is_empty() && attached.is_empty() {
        return Err("an empty message is not sent".into());
    }
    let files = state.uploads.claim(&attached);
    if files.len() != attached.len() {
        return Err("those attachments are no longer held; try attaching again".into());
    }
    let me = state.rest.me().await.map_err(fail)?;
    let pending_post_id = PendingPosts::new_id(&me.id, now_ms());
    // Length, not content: the log must stay free of message text.
    tracing::info!(
        channel = %channel_id,
        pending = %pending_post_id,
        chars = trimmed.chars().count(),
        threaded = root_id.is_some(),
        files = files.len(),
        "sending"
    );

    state.pending.insert(PendingPost {
        pending_post_id: pending_post_id.clone(),
        channel_id: channel_id.clone(),
        root_id: root_id.clone().unwrap_or_default(),
        message: trimmed.to_string(),
        create_at: now_ms(),
        failed: false,
        files,
    });
    // Paint the guess before the request goes out.
    let _ = state
        .to_engine
        .send(EngineMsg::Refreshed(channel_id.clone()))
        .await;

    deliver(&state, &pending_post_id, &channel_id).await?;
    Ok(pending_post_id)
}

/// Retries a send that failed, keeping the same pending id so the row does not
/// jump.
#[tauri::command]
pub async fn retry_post(state: State<'_, AppState>, pending_post_id: String) -> Reply<()> {
    let Some(held) = state.pending.get(&pending_post_id) else {
        return Err("that message is no longer pending".into());
    };
    let channel_id = held.channel_id.clone();
    deliver(&state, &pending_post_id, &channel_id).await
}

/// Throws away a failed send. Only ever reachable for a failed one, because a
/// message must not vanish while it might still land.
#[tauri::command]
pub async fn discard_post(state: State<'_, AppState>, pending_post_id: String) -> Reply<()> {
    if let Some(channel_id) = state.pending.discard(&pending_post_id) {
        let _ = state.to_engine.send(EngineMsg::Refreshed(channel_id)).await;
    }
    Ok(())
}

async fn deliver(state: &AppState, pending_post_id: &str, channel_id: &str) -> Result<(), String> {
    let Some(held) = state.pending.get(pending_post_id) else {
        return Ok(());
    };
    let file_ids: Vec<String> = held.files.iter().map(|file| file.id.clone()).collect();
    let request = matterless_core::model::NewPost {
        channel_id,
        message: &held.message,
        root_id: &held.root_id,
        pending_post_id,
        file_ids: &file_ids,
    };

    match state.rest.create_post(&request).await {
        Ok(post) => {
            // Store the server's copy, then drop the guess.
            let engine = state.engine.clone();
            let me = state.rest.me().await.map_err(fail)?;
            let deltas = tokio::task::spawn_blocking(move || {
                let context = SyncContext::new(me, ThreadMode::Flat);
                engine.apply_event(
                    &matterless_core::Event::Posted {
                        post: Box::new(post.clone()),
                        channel_id: post.channel_id.clone(),
                    },
                    &context,
                )
            })
            .await
            .map_err(fail)?
            .map_err(fail)?;
            let resolved = state.pending.resolve(pending_post_id).is_some();

            // The websocket echo usually beats this reply, in which case it has
            // already stored the post and cleared the pending row -- so redrawing
            // again would be a third rebuild that changes nothing. Only ask for
            // one when this path is what actually altered the view.
            let changed_anything = !deltas.is_empty() || resolved;
            tracing::info!(
                pending = %pending_post_id,
                changed_anything,
                "send confirmed"
            );
            if changed_anything {
                let _ = state
                    .to_engine
                    .send(EngineMsg::Refreshed(channel_id.to_string()))
                    .await;
            }
        }
        Err(error) => {
            tracing::warn!(pending = %pending_post_id, %error, "send failed");
            // Keep the row, marked, rather than losing what was typed.
            state.pending.mark_failed(pending_post_id);
            let _ = state
                .to_engine
                .send(EngineMsg::Refreshed(channel_id.to_string()))
                .await;
            return Err(fail(error));
        }
    }
    Ok(())
}

/// Lets the frontend write into the same timeline as the backend.
///
/// Without this the log tells only half the story: Rust knows what it returned,
/// but only the shell knows what it then did with it. `detail` is free-form so a
/// caller can add structure, but **never message text**.
#[tauri::command]
pub async fn ui_log(level: String, event: String, detail: Option<String>) -> Reply<()> {
    let detail = detail.unwrap_or_default();
    match level.as_str() {
        "error" => tracing::error!(target: "ui", %event, %detail),
        "warn" => tracing::warn!(target: "ui", %event, %detail),
        "debug" => tracing::debug!(target: "ui", %event, %detail),
        _ => tracing::info!(target: "ui", %event, %detail),
    }
    Ok(())
}

/// So the log is findable from inside the app rather than by guessing a path.
#[tauri::command]
pub async fn where_is_the_log(app: tauri::AppHandle) -> Reply<String> {
    Ok(crate::log_path(&app).display().to_string())
}

/// Pushes a channel's current unread to the UI after a local change.
async fn notify_unread(state: &AppState, channel_id: &str, user_id: &str) {
    let store = state.store.clone();
    let target = channel_id.to_string();
    let viewer = user_id.to_string();
    let unread = tokio::task::spawn_blocking(move || store.unread(&target, &viewer)).await;
    if let Ok(Ok(Some(unread))) = unread {
        let (messages, mentions) =
            unread.visible(state.collapsed.load(std::sync::atomic::Ordering::Relaxed));
        let _ = state
            .to_engine
            .send(EngineMsg::Unread {
                channel_id: channel_id.to_string(),
                messages,
                mentions,
                muted: unread.muted,
            })
            .await;
    }
}

/// Fetches the page of history older than what is held.
///
/// Returns whether anything older still exists, so the shell knows when to stop
/// asking. The cursor is the oldest post in our contiguous range -- paging from
/// anywhere else would open a gap.
#[tauri::command]
pub async fn load_older(state: State<'_, AppState>, channel_id: String) -> Reply<bool> {
    let store = state.store.clone();
    let probe = channel_id.clone();
    let held = tokio::task::spawn_blocking(move || store.sync_state(&probe))
        .await
        .map_err(fail)?
        .map_err(fail)?;

    let Some(held) = held else {
        return Ok(false);
    };
    if held.reached_beginning {
        tracing::debug!(channel = %channel_id, "already at the start of history");
        return Ok(false);
    }
    if held.oldest_post_id.is_empty() {
        return Ok(false);
    }

    let list = state
        .rest
        .posts_before(&channel_id, &held.oldest_post_id, 200)
        .await
        .map_err(fail)?;
    let more = !list.prev_post_id.is_empty();
    let fetched = list.order.len();

    let engine = state.engine.clone();
    let me = state.rest.me().await.map_err(fail)?;
    let target = channel_id.clone();
    tokio::task::spawn_blocking(move || {
        let context = SyncContext::new(me, ThreadMode::Flat);
        engine.apply_post_list(&target, &list, Arrival::Backfill, &context, 0)
    })
    .await
    .map_err(fail)?
    .map_err(fail)?;

    // Deliberately no `Refreshed` here. Backfilled posts are older than
    // everything on screen, so nothing in the newest page changed -- and that
    // delta was making the shell rebuild it after every page-back, once per
    // server round trip.
    tracing::info!(channel = %channel_id, fetched, more, "loaded older history");
    Ok(more)
}

#[derive(Serialize)]
pub struct BadgeReport {
    pub any_unread: bool,
    pub attention: i64,
    pub mentions: i64,
    pub thread_mentions: i64,
    pub followed_unread: i64,
}

/// Redraws the taskbar overlay badge from the store.
///
/// Computed in Rust rather than summed in the shell because the rule depends on
/// each channel's `notify_props`, which the sidebar payload does not carry --
/// and a badge that disagreed with the notification policy would be worse than
/// none.
#[tauri::command]
pub async fn update_badge(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    me_id: String,
) -> Reply<BadgeReport> {
    let store = state.store.clone();
    let collapsed = state.collapsed.load(std::sync::atomic::Ordering::Relaxed);
    let badge = tokio::task::spawn_blocking(move || store.badge_state(&me_id, collapsed))
        .await
        .map_err(fail)?
        .map_err(fail)?;

    let attention = badge.attention();
    let window = app.get_webview_window("main");
    // Built at the size Windows will actually draw, because anything else
    // reaches the taskbar through a filtered rescale and looks soft.
    let icon_size = window
        .as_ref()
        .and_then(|window| window.scale_factor().ok())
        .map(crate::taskbar::size_for_scale)
        .unwrap_or(crate::taskbar::BASE_SIZE);
    if let Some(window) = window {
        let overlay = if attention > 0 {
            Some(crate::taskbar::count(attention, icon_size))
        } else if badge.any_unread {
            Some(crate::taskbar::dot(icon_size))
        } else {
            None
        };
        // Windows only; elsewhere this is a no-op rather than an error.
        if let Err(error) = window.set_overlay_icon(overlay) {
            tracing::debug!(%error, "no taskbar overlay on this platform");
        }
    }

    // Both parts are logged: a wrong total is only diagnosable if the log says
    // which rule contributed it.
    tracing::debug!(
        any_unread = badge.any_unread,
        mentions = badge.mentions,
        thread_mentions = badge.thread_mentions,
        followed_unread = badge.followed_unread,
        attention,
        icon_size,
        "badge updated"
    );
    Ok(BadgeReport {
        any_unread: badge.any_unread,
        attention,
        mentions: badge.mentions,
        thread_mentions: badge.thread_mentions,
        followed_unread: badge.followed_unread,
    })
}

/// Raises a Windows toast that reaches the app when clicked.
///
/// In Rust rather than through the notification plugin: that plugin never
/// registers an activation handler, so its toasts were inert. The decision to
/// notify is still made in `notify::decide` -- this only draws it.
#[tauri::command]
pub async fn raise_toast(
    app: tauri::AppHandle,
    channel_id: String,
    title: String,
    body: String,
) -> Reply<()> {
    #[cfg(windows)]
    {
        crate::toast::raise(&app, &channel_id, &title, &body).map_err(|error| {
            tracing::warn!(%error, "the toast could not be raised");
            error.to_string()
        })?;
        // Length only, never the text: the log stays free of message content.
        tracing::info!(channel = %channel_id, chars = body.len(), "toast raised");
    }
    #[cfg(not(windows))]
    {
        let _ = (app, channel_id, title, body);
    }
    Ok(())
}

/// Flashes the taskbar button.
///
/// Only meaningful while the window is unfocused, which the caller checks:
/// asking for attention you already have is how an app becomes irritating.
#[tauri::command]
pub async fn flash_taskbar(app: tauri::AppHandle, urgent: bool) -> Reply<()> {
    if let Some(window) = app.get_webview_window("main") {
        // Critical keeps flashing until the window is looked at, which is right
        // for something that named you; Informational is a single nudge.
        let kind = if urgent {
            tauri::UserAttentionType::Critical
        } else {
            tauri::UserAttentionType::Informational
        };
        window.request_user_attention(Some(kind)).map_err(fail)?;
        tracing::debug!(urgent, "asked for attention");
    }
    Ok(())
}

/// Stops the flashing once the window has been looked at.
#[tauri::command]
pub async fn clear_attention(app: tauri::AppHandle) -> Reply<()> {
    if let Some(window) = app.get_webview_window("main") {
        window.request_user_attention(None).map_err(fail)?;
    }
    Ok(())
}

/// Window focus feeds the "do not notify for the channel you are reading" rule.
#[tauri::command]
pub async fn set_focus(state: State<'_, AppState>, focused: bool) -> Reply<()> {
    state
        .to_engine
        .send(EngineMsg::SetFocus(focused))
        .await
        .map_err(fail)
}

/// One stream for every delta, rather than an event type per mutation.
#[tauri::command]
pub async fn subscribe(state: State<'_, AppState>, channel: Channel<UiDelta>) -> Reply<()> {
    state
        .to_engine
        .send(EngineMsg::Subscribe(channel))
        .await
        .map_err(fail)
}

// -------------------------------------------------------------------- files

/// Uploads one file into a channel, ahead of the post that will carry it.
///
/// Synchronous, and it returns the moment the bytes are in hand: the upload
/// itself runs on the async runtime and reports back with an `attachment`
/// event. That is not a shortcut -- `tauri::ipc::Request` borrows its body, so
/// it cannot cross an await -- but it is also the right shape, because a 20 MB
/// drop takes long enough that the tray has to be able to say "uploading".
///
/// The bytes arrive as a raw IPC body rather than as JSON: a number array would
/// be roughly four characters per byte to serialise, parse and copy.
#[tauri::command]
pub fn attach_file(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    request: tauri::ipc::Request<'_>,
) -> Reply<()> {
    let header = |name: &str| -> String {
        request
            .headers()
            .get(name)
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default()
            .to_string()
    };
    let attach_id = header("x-attach-id");
    let channel_id = header("x-channel-id");
    let filename = uploads::percent_decode(&header("x-file-name"));
    if attach_id.is_empty() || channel_id.is_empty() {
        return Err("an attachment needs a channel and a handle".into());
    }

    let tauri::ipc::InvokeBody::Raw(bytes) = request.body() else {
        return Err("attachments are sent as raw bytes".into());
    };
    let size = bytes.len() as i64;
    let limit = state
        .max_file_size
        .load(std::sync::atomic::Ordering::Relaxed);
    // Enforced here as well as in the shell: the shell's copy of the limit is a
    // convenience, and this is the last place that can refuse before spending
    // the upload.
    if limit > 0 && size > limit {
        return Err(format!(
            "that file is {} MB; this server allows {} MB",
            size / (1024 * 1024),
            limit / (1024 * 1024)
        ));
    }
    if size == 0 {
        return Err("that file is empty".into());
    }

    let bytes = bytes.clone();
    let rest = Arc::clone(&state.rest);
    let holder = Arc::clone(&state.uploads);
    let app = app.clone();
    tracing::info!(
        attach = %attach_id,
        channel = %channel_id,
        bytes = size,
        extension = %extension_of(&filename),
        // Length rather than the name: a filename is content, and the log must
        // stay free of it. Enough to tell "the header arrived" from "the header
        // was mangled", which is the failure this would otherwise hide.
        name_chars = filename.chars().count(),
        "uploading an attachment"
    );

    // Reported as it goes, throttled.
    //
    // A chunk is 64KB, so a 150MB file is 2400 of these -- one IPC message each
    // would cost more than the upload. A tenth of a second is faster than a
    // reader can read a number, and the last chunk always reports so a bar
    // cannot stop short of the end.
    let progress_app = app.clone();
    let progress_id = attach_id.clone();
    let last = std::sync::Mutex::new(std::time::Instant::now() - PROGRESS_EVERY);
    let progress: matterless_core::rest::UploadProgress = Arc::new(move |sent, total| {
        let due = {
            let mut last = last.lock().expect("upload progress");
            if sent >= total || last.elapsed() >= PROGRESS_EVERY {
                *last = std::time::Instant::now();
                true
            } else {
                false
            }
        };
        if due {
            let _ = progress_app.emit(
                "attachment.progress",
                AttachmentProgress {
                    attach_id: progress_id.clone(),
                    sent,
                    total,
                },
            );
        }
    });

    let holder_for_task = Arc::clone(&holder);
    let finished_id = attach_id.clone();
    // The registry keeps its own copy: the task takes the original, and the
    // handle has to be filed under it after the task has been spawned.
    let registered = attach_id.clone();
    let task = tauri::async_runtime::spawn(async move {
        let started = std::time::Instant::now();
        let outcome = rest
            .upload_file(&channel_id, &filename, &bytes, Some(progress))
            .await;
        let payload = match outcome {
            Ok(response) => match response.file_infos.into_iter().next() {
                Some(info) => {
                    tracing::info!(
                        attach = %attach_id,
                        file = %info.id,
                        bytes = info.size,
                        ms = started.elapsed().as_secs_f64() * 1000.0,
                        "attachment uploaded"
                    );
                    // Default layout: the tray shows one chip per file, so
                    // there is no gallery decision to make and no image to
                    // draw at size -- only the name, the size and the
                    // placeholder.
                    let file = matterless_render::FileRef::from_info(
                        &info,
                        matterless_render::FileLayout::default(),
                    );
                    holder.hold(info);
                    AttachmentEvent {
                        attach_id,
                        file: Some(file),
                        error: None,
                    }
                }
                None => AttachmentEvent {
                    attach_id,
                    file: None,
                    error: Some("the server accepted the upload but returned no file".into()),
                },
            },
            Err(error) => {
                tracing::warn!(attach = %attach_id, %error, "attachment upload failed");
                AttachmentEvent {
                    attach_id,
                    file: None,
                    error: Some(error.to_string()),
                }
            }
        };
        holder_for_task.finished(&finished_id);
        let _ = app.emit("attachment", payload);
    });
    state.uploads.sending(&registered, task);
    Ok(())
}

/// How far an upload has got, emitted while it runs.
#[derive(Clone, Serialize)]
pub struct AttachmentProgress {
    pub attach_id: String,
    pub sent: u64,
    pub total: u64,
}

/// How often an upload reports its progress.
const PROGRESS_EVERY: std::time::Duration = std::time::Duration::from_millis(100);

/// Stops an upload that is still on the wire.
///
/// Aborting the task drops the connection, which is the only way to stop one:
/// Mattermost has no resumable upload and no "cancel" endpoint. The server may
/// still finish writing what it already received, and that orphan is not this
/// client's to delete -- but nothing here will claim it, and no post will
/// reference it.
#[tauri::command]
pub async fn cancel_attachment(state: State<'_, AppState>, attach_id: String) -> Reply<bool> {
    let stopped = state.uploads.cancel(&attach_id);
    tracing::info!(attach = %attach_id, stopped, "attachment upload cancelled");
    Ok(stopped)
}

/// How an upload ended. Emitted rather than returned, because the command that
/// started it had to hand back the IPC thread first.
#[derive(Clone, Serialize)]
pub struct AttachmentEvent {
    pub attach_id: String,
    /// The uploaded file, ready for the composer's tray.
    pub file: Option<matterless_render::FileRef>,
    pub error: Option<String>,
}

/// Drops an upload the composer no longer wants to send.
///
/// The bytes stay on the server as an orphan -- there is no delete-file endpoint
/// for a non-admin -- but nothing here holds a claim on them any more.
#[tauri::command]
pub async fn release_attachment(state: State<'_, AppState>, file_id: String) -> Reply<bool> {
    let released = state.uploads.release(&file_id);
    tracing::debug!(file = %file_id, released, "attachment released");
    Ok(released)
}

/// Writes an attachment into the reader's downloads folder.
///
/// A plain link cannot do this: `/files/{id}/link` is 403 on this server (public
/// links are off), and the webview sends no Authorization header, so the bytes
/// have to come through here. The disk cache is asked first, which makes saving
/// something just looked at free.
#[tauri::command]
pub async fn save_attachment(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    file_id: String,
    name: String,
) -> Reply<String> {
    let key = format!("file-{file_id}");
    let cached = {
        let cache = Arc::clone(&state.files);
        let probe = key.clone();
        tokio::task::spawn_blocking(move || cache.read(&probe))
            .await
            .map_err(fail)?
    };
    let bytes = match cached {
        Some((bytes, _)) => bytes,
        None => {
            let (bytes, content_type) = state
                .rest
                .fetch_bytes(&format!("/files/{file_id}"))
                .await
                .map_err(fail)?
                .ok_or_else(|| "that file is no longer on the server".to_string())?;
            let cache = Arc::clone(&state.files);
            let stored = bytes.clone();
            tokio::task::spawn_blocking(move || cache.write(&key, &stored, &content_type))
                .await
                .map_err(fail)?;
            bytes
        }
    };

    let folder = app
        .path()
        .download_dir()
        .map_err(|error| format!("no downloads folder: {error}"))?;
    let destination = unused_path(&folder, &safe_filename(&name));
    let written = bytes.len();
    let target = destination.clone();
    tokio::task::spawn_blocking(move || std::fs::write(&target, bytes))
        .await
        .map_err(fail)?
        .map_err(|error| format!("could not write the file: {error}"))?;
    tracing::info!(file = %file_id, bytes = written, path = %destination.display(), "attachment saved");
    Ok(destination.to_string_lossy().into_owned())
}

/// A filename Windows will accept, from one the server accepted.
///
/// The name came from whoever uploaded it, so it is not assumed to be a legal
/// path component here: a reserved character or a path separator in it would
/// otherwise decide where the file lands.
fn safe_filename(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|character| match character {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '_',
            control if control.is_control() => '_',
            other => other,
        })
        .collect();
    let trimmed = cleaned.trim().trim_matches('.').to_string();
    if trimmed.is_empty() {
        "attachment".to_string()
    } else {
        trimmed
    }
}

/// `name.png`, `name (2).png`, ... -- saving twice keeps both rather than
/// overwriting whatever was there.
fn unused_path(folder: &std::path::Path, name: &str) -> std::path::PathBuf {
    let candidate = folder.join(name);
    if !candidate.exists() {
        return candidate;
    }
    let (stem, extension) = match name.rsplit_once('.') {
        Some((stem, extension)) => (stem.to_string(), format!(".{extension}")),
        None => (name.to_string(), String::new()),
    };
    for attempt in 2..1000 {
        let candidate = folder.join(format!("{stem} ({attempt}){extension}"));
        if !candidate.exists() {
            return candidate;
        }
    }
    folder.join(format!("{stem} (many){extension}"))
}

fn extension_of(name: &str) -> String {
    name.rsplit_once('.')
        .map(|(_, extension)| extension.to_ascii_lowercase())
        .unwrap_or_default()
}

// ---------------------------------------------------------- message actions

/// Edits a message.
///
/// **Not** optimistic in SQLite, deliberately, though the plan called for it:
/// the store rejects a post whose `update_at` is older than what it holds, so
/// writing a *guessed* timestamp locally can make the server's own copy arrive
/// stale and be dropped -- a clock-domain hazard with a silent failure. The
/// shell shows the new text in its editor while the request is in flight, and
/// the store is written from the server's reply, which carries the real
/// `update_at`.
#[tauri::command]
pub async fn edit_post(state: State<'_, AppState>, post_id: String, message: String) -> Reply<()> {
    let trimmed = message.trim();
    if trimmed.is_empty() {
        // Mattermost treats an empty edit as a delete; making that implicit
        // would delete a message the reader only meant to clear.
        return Err("an empty message is a delete; use Delete".into());
    }
    let post = state
        .rest
        .patch_post(&post_id, trimmed)
        .await
        .map_err(fail)?;
    tracing::info!(
        post = %post_id,
        chars = trimmed.chars().count(),
        update_at = post.update_at,
        "message edited"
    );

    let channel_id = post.channel_id.clone();
    let store = state.store.clone();
    tokio::task::spawn_blocking(move || store.upsert_posts(std::slice::from_ref(&post)))
        .await
        .map_err(fail)?
        .map_err(fail)?;
    let _ = state.to_engine.send(EngineMsg::Refreshed(channel_id)).await;
    Ok(())
}

/// Deletes a message.
#[tauri::command]
pub async fn delete_post_now(state: State<'_, AppState>, post_id: String) -> Reply<()> {
    state.rest.delete_post(&post_id).await.map_err(fail)?;

    let store = state.store.clone();
    let probe = post_id.clone();
    let deleted_at = now_ms();
    let channel_id =
        tokio::task::spawn_blocking(move || -> matterless_store::Result<Option<String>> {
            let channel = store.post(&probe)?.map(|post| post.channel_id);
            store.tombstone_post(&probe, deleted_at)?;
            Ok(channel)
        })
        .await
        .map_err(fail)?
        .map_err(fail)?;

    tracing::info!(post = %post_id, "message deleted");
    if let Some(channel_id) = channel_id {
        let _ = state.to_engine.send(EngineMsg::Refreshed(channel_id)).await;
    }
    Ok(())
}

/// Everything a profile card shows, for one person.
#[derive(Debug, Clone, Serialize)]
pub struct Profile {
    pub user_id: String,
    pub username: String,
    /// Their name as this server asks clients to show it.
    pub display_name: String,
    pub full_name: String,
    pub nickname: String,
    pub email: String,
    pub avatar_at: matterless_core::model::Timestamp,
    pub status: String,
    /// True for the reader themselves, so the card can offer notes to self
    /// rather than "send a message".
    pub is_me: bool,
}

/// A profile, by username *or* by user id.
///
/// Both, because the two callers have different things to hand: a clicked
/// `@mention` knows only the name it was written with, while a message row
/// knows the author's id -- and its *displayed* name is whatever
/// `TeammateNameDisplay` says, which on a server set to `full_name` is not a
/// username at all.
///
/// Local first, then the server: a mention of somebody who has never posted in
/// a channel this client has read is exactly the case the users table cannot
/// answer, and it is also the case where a card is most useful.
#[tauri::command]
pub async fn profile(
    state: State<'_, AppState>,
    who: String,
    me_id: String,
    display_mode: String,
) -> Reply<Profile> {
    let wanted = who.trim_start_matches('@').to_string();
    let store = state.store.clone();
    let probe = wanted.clone();
    let held = tokio::task::spawn_blocking(
        move || -> matterless_store::Result<Option<matterless_core::User>> {
            if let Some(user) = store.user_by_username(&probe)? {
                return Ok(Some(user));
            }
            Ok(store
                .users_by_ids(std::slice::from_ref(&probe))?
                .into_values()
                .next())
        },
    )
    .await
    .map_err(fail)?
    .map_err(fail)?;

    let user = match held {
        Some(user) => user,
        None => {
            let fetched = state
                .rest
                .user_by_username(&wanted)
                .await
                .map_err(|error| {
                    // The server's envelope is three sentences of API prose; it
                    // belongs in the log, not in a card six words wide.
                    tracing::info!(%wanted, %error, "no profile for that name");
                    format!("No account for @{wanted}.")
                })?;
            // Remembered, so the next click is local and so their name resolves
            // everywhere else too.
            let store = state.store.clone();
            let copy = fetched.clone();
            let _ = tokio::task::spawn_blocking(move || store.upsert_users(&[copy])).await;
            fetched
        }
    };

    // One status, for one card: the batch endpoint takes a list, and a list of
    // one is still the right call to make.
    let status = state
        .rest
        .statuses_by_ids(std::slice::from_ref(&user.id))
        .await
        .ok()
        .and_then(|found| found.into_iter().next())
        .map(|status| status.status)
        .unwrap_or_default();

    let full_name = format!("{} {}", user.first_name, user.last_name)
        .trim()
        .to_string();
    tracing::debug!(user = %user.id, "profile opened");
    Ok(Profile {
        display_name: matterless_render::display_name(&user, &display_mode),
        is_me: user.id == me_id,
        user_id: user.id,
        username: user.username,
        full_name,
        nickname: user.nickname,
        email: user.email,
        avatar_at: user.last_picture_update,
        status,
    })
}

/// The channel a `~link` names, or nothing when this reader is not in it.
///
/// Nothing rather than an error: a message can name a private channel the
/// reader has no part in, and that is ordinary rather than a failure.
#[tauri::command]
pub async fn channel_by_name(state: State<'_, AppState>, name: String) -> Reply<Option<String>> {
    let store = state.store.clone();
    let wanted = name.trim_start_matches('~').to_string();
    let found = tokio::task::spawn_blocking(move || store.channel_by_name(&wanted))
        .await
        .map_err(fail)?
        .map_err(fail)?;
    Ok(found.map(|channel| channel.id))
}

/// Presence for the people on screen.
///
/// One request for all of them, never one per avatar: Phase 0 found no
/// `X-RateLimit-*` headers on this server, so there is no way to adapt if a
/// busy channel fires sixty requests at once -- the limiter would simply have
/// to eat them.
#[tauri::command]
pub async fn statuses(
    state: State<'_, AppState>,
    user_ids: Vec<String>,
) -> Reply<HashMap<String, String>> {
    if user_ids.is_empty() {
        return Ok(HashMap::new());
    }
    let found = state.rest.statuses_by_ids(&user_ids).await.map_err(fail)?;
    tracing::debug!(
        asked = user_ids.len(),
        got = found.len(),
        "statuses fetched"
    );
    Ok(found
        .into_iter()
        .map(|status| (status.user_id, status.status))
        .collect())
}

/// Sets this reader's own presence.
///
/// Also told to the engine, because the do-not-disturb rule in `notify::decide`
/// is decided against it -- and that context used to sit at its "online"
/// default forever, so choosing Do Not Disturb suppressed nothing at all.
#[tauri::command]
pub async fn set_my_status(
    state: State<'_, AppState>,
    me_id: String,
    status: String,
) -> Reply<String> {
    let updated = state
        .rest
        .set_my_status(&me_id, &status)
        .await
        .map_err(fail)?;
    let _ = state
        .to_engine
        .send(EngineMsg::OwnStatus(updated.status.clone()))
        .await;
    tracing::info!(status = %updated.status, "own status set");
    Ok(updated.status)
}

/// Tells the server this reader is typing.
///
/// Fired on every keystroke and throttled in the engine, so "how often does
/// this go out" has one answer rather than one per caller.
#[tauri::command]
pub async fn send_typing(
    state: State<'_, AppState>,
    channel_id: String,
    root_id: Option<String>,
) -> Reply<()> {
    let _ = state
        .to_engine
        .send(EngineMsg::Typing {
            channel_id,
            root_id: root_id.unwrap_or_default(),
        })
        .await;
    Ok(())
}

/// The message as it was typed, for "Copy Text".
///
/// Fetched on demand rather than carried on every row: the plan hands the shell
/// parsed nodes, and shipping the raw markdown of 200 messages alongside them to
/// serve one menu click would be the wrong trade.
#[tauri::command]
pub async fn post_text(state: State<'_, AppState>, post_id: String) -> Reply<String> {
    let store = state.store.clone();
    let probe = post_id.clone();
    let post = tokio::task::spawn_blocking(move || store.post(&probe))
        .await
        .map_err(fail)?
        .map_err(fail)?
        .ok_or_else(|| "that message is not held locally".to_string())?;
    Ok(post.message)
}

/// A link to the message, in the form the official client uses.
///
/// `<server>/<team name>/pl/<post id>` -- the team's *name*, not its id, which
/// is why this is resolved here: the shell knows the channel, and the mapping
/// from channel to team to name is store work.
/// Asks the server to remind the reader about a post at a given time.
///
/// `target_time` is epoch seconds and comes from the shell: which instant
/// "tomorrow morning" is depends on the reader's clock and timezone, and the
/// shell is where both are known.
#[tauri::command]
pub async fn set_reminder(
    state: State<'_, AppState>,
    post_id: String,
    target_time: i64,
) -> Reply<()> {
    let me = state.rest.me().await.map_err(fail)?;
    state
        .rest
        .set_reminder(&me.id, &post_id, target_time)
        .await
        .map_err(fail)?;
    // The time, not the post's content: the log stays free of message text.
    tracing::info!(post = %post_id, target_time, "reminder set");
    Ok(())
}

#[tauri::command]
pub async fn post_permalink(state: State<'_, AppState>, post_id: String) -> Reply<String> {
    let store = state.store.clone();
    let probe = post_id.clone();
    let team_name =
        tokio::task::spawn_blocking(move || -> matterless_store::Result<Option<String>> {
            let Some(post) = store.post(&probe)? else {
                return Ok(None);
            };
            let Some(channel) = store.channel(&post.channel_id)? else {
                return Ok(None);
            };
            // A direct message has no team of its own; any team the reader is on
            // resolves the link, which is what the official client does too.
            if channel.team_id.is_empty() {
                store.any_team_name()
            } else {
                store.team_name(&channel.team_id)
            }
        })
        .await
        .map_err(fail)?
        .map_err(fail)?
        .ok_or_else(|| "no team to build a link from".to_string())?;

    let base = state.rest.base_url().to_string();
    let link = format!("{}/{team_name}/pl/{post_id}", base.trim_end_matches('/'));
    tracing::debug!(post = %post_id, "permalink built");
    Ok(link)
}

/// Marks the channel unread from this message down.
///
/// The server answers with the membership it ended up with, which is stored
/// straight away: the sidebar count and the badge are drawn from it, and waiting
/// for a refresh would leave both showing the old state.
#[tauri::command]
pub async fn mark_post_unread(
    state: State<'_, AppState>,
    me_id: String,
    post_id: String,
) -> Reply<()> {
    let member = state
        .rest
        .set_post_unread(&me_id, &post_id)
        .await
        .map_err(fail)?;
    let channel_id = member.channel_id.clone();
    tracing::info!(
        post = %post_id,
        channel = %channel_id,
        unread = member.msg_count,
        "marked unread from a message"
    );

    let store = state.store.clone();
    let held = member.clone();
    tokio::task::spawn_blocking(move || store.upsert_channel_members(&[held]))
        .await
        .map_err(fail)?
        .map_err(fail)?;
    // The stream itself gains a divider, so the plan is what changes.
    let _ = state.to_engine.send(EngineMsg::Refreshed(channel_id)).await;
    Ok(())
}

/// Saves or unsaves a message for this reader.
#[tauri::command]
pub async fn set_post_saved(
    state: State<'_, AppState>,
    me_id: String,
    post_id: String,
    saved: bool,
) -> Reply<()> {
    state
        .rest
        .set_post_saved(&me_id, &post_id, saved)
        .await
        .map_err(fail)?;

    let store = state.store.clone();
    let owner = me_id.clone();
    let probe = post_id.clone();
    let channel_id =
        tokio::task::spawn_blocking(move || -> matterless_store::Result<Option<String>> {
            if saved {
                store.set_preference(&owner, "flagged_post", &probe, "true")?;
            } else {
                store.delete_preference(&owner, "flagged_post", &probe)?;
            }
            Ok(store.post(&probe)?.map(|post| post.channel_id))
        })
        .await
        .map_err(fail)?
        .map_err(fail)?;

    tracing::info!(post = %post_id, saved, "message saved state changed");
    if let Some(channel_id) = channel_id {
        let _ = state.to_engine.send(EngineMsg::Refreshed(channel_id)).await;
    }
    Ok(())
}

/// Pins or unpins a message. Channel-wide: everyone sees it.
#[tauri::command]
pub async fn set_post_pinned(
    state: State<'_, AppState>,
    post_id: String,
    pinned: bool,
) -> Reply<()> {
    state
        .rest
        .set_post_pinned(&post_id, pinned)
        .await
        .map_err(fail)?;

    let store = state.store.clone();
    let probe = post_id.clone();
    let channel_id =
        tokio::task::spawn_blocking(move || -> matterless_store::Result<Option<String>> {
            store.set_post_pinned(&probe, pinned)?;
            Ok(store.post(&probe)?.map(|post| post.channel_id))
        })
        .await
        .map_err(fail)?
        .map_err(fail)?;

    tracing::info!(post = %post_id, pinned, "message pin changed");
    if let Some(channel_id) = channel_id {
        let _ = state.to_engine.send(EngineMsg::Refreshed(channel_id)).await;
    }
    Ok(())
}

// ----------------------------------------------------------- suggestions

/// One completion offered to the composer or the switcher.
#[derive(Debug, Clone, Serialize)]
pub struct Suggestion {
    /// What gets inserted: a username, a channel slug, an emoji name.
    pub value: String,
    /// What the row shows.
    pub label: String,
    /// The second line: a real name, a team, an emoji's own glyph.
    pub detail: String,
    /// For an avatar, a channel icon or a custom emoji image.
    pub id: String,
    /// `user` / `channel` / `emoji`, so the shell knows what it is drawing.
    pub kind: String,
    /// A user's `last_picture_update`, so an avatar URL has its version.
    pub avatar_at: matterless_core::model::Timestamp,
    /// A channel's type (`O`, `P`, `D`, `G`) for its icon; empty otherwise.
    pub channel_type: String,
}

/// How many rows a completion list offers. More than a screenful is not a
/// choice, it is a scroll.
const SUGGESTIONS: u32 = 8;

/// Completions for what is being typed, answered from the local store.
///
/// Local-first is the whole design: the users and channels tables already hold
/// everyone this client has seen and every channel the reader is in, so the
/// common case costs one SQLite scan and no request at all. Matching is a
/// *subsequence* -- `al` finds `ada.lovelace` -- with the ranking done in
/// Rust; see `matterless_core::fuzzy`.
///
/// The server is only asked for *people*, and only when the local answer is
/// thin: a name nobody here has posted under is the one case the local table
/// cannot know about. Note the server's own autocomplete is prefix-based, so it
/// answers nothing for a scattered query -- which is fine, because that is
/// exactly the query the local index handles well.
///
/// `limit` is the caller's: the default suits a completion list under a caret,
/// while a browsing grid asks for more. One command either way, so the picker
/// and `:` completion cannot disagree about what exists or how it ranks.
#[tauri::command]
pub async fn suggest(
    state: State<'_, AppState>,
    kind: String,
    query: String,
    channel_id: Option<String>,
    display_mode: String,
    limit: Option<u32>,
) -> Reply<Vec<Suggestion>> {
    let store = state.store.clone();
    let probe = query.clone();
    let wanted = kind.clone();
    let mode = display_mode.clone();
    let cap = limit.unwrap_or(SUGGESTIONS).clamp(1, 200);

    let mut found =
        tokio::task::spawn_blocking(move || -> matterless_store::Result<Vec<Suggestion>> {
            Ok(match wanted.as_str() {
                "user" => store
                    .users_matching(&probe, cap)?
                    .into_iter()
                    .map(|user| Suggestion {
                        value: user.username.clone(),
                        label: user.username.clone(),
                        detail: matterless_render::display_name(&user, "full_name"),
                        avatar_at: user.last_picture_update,
                        id: user.id,
                        kind: "user".into(),
                        channel_type: String::new(),
                    })
                    .collect(),
                "channel" => store
                    .channels_matching(&probe, cap)?
                    .into_iter()
                    .map(|channel| Suggestion {
                        value: channel.name.clone(),
                        label: if channel.display_name.is_empty() {
                            channel.name.clone()
                        } else {
                            channel.display_name.clone()
                        },
                        detail: String::new(),
                        channel_type: channel.channel_type,
                        id: channel.id,
                        kind: "channel".into(),
                        avatar_at: 0,
                    })
                    .collect(),
                "emoji" => {
                    let mut rows: Vec<Suggestion> = store
                        .custom_emoji_matching(&probe, cap)?
                        .into_iter()
                        .map(|(name, emoji_id)| Suggestion {
                            value: name.clone(),
                            label: name,
                            detail: String::new(),
                            id: emoji_id,
                            kind: "emoji".into(),
                            avatar_at: 0,
                            channel_type: String::new(),
                        })
                        .collect();
                    // Standard emoji come from the generated table rather than the
                    // database: 4463 names compiled in, so there is nothing to
                    // query and nothing to keep in step.
                    for name in matterless_render::emoji::names_matching(&probe, cap as usize) {
                        if rows.len() >= cap as usize {
                            break;
                        }
                        rows.push(Suggestion {
                            detail: matterless_render::emoji::character_for(name)
                                .unwrap_or_default(),
                            value: name.to_string(),
                            label: name.to_string(),
                            id: String::new(),
                            kind: "emoji".into(),
                            avatar_at: 0,
                            channel_type: String::new(),
                        });
                    }
                    rows
                }
                other => {
                    tracing::warn!(kind = other, "unknown suggestion kind");
                    Vec::new()
                }
            })
        })
        .await
        .map_err(fail)?
        .map_err(fail)?;

    // The server only for people, only when the local answer is thin, and never
    // for a bare `@`: asking on every keystroke is how a completion list starts
    // flickering between two answers.
    if kind == "user" && found.len() < 3 && query.chars().count() >= 2 {
        let route = match &channel_id {
            Some(channel) => format!(
                "/users/autocomplete?in_channel={channel}&name={}",
                urlencode(&query)
            ),
            None => format!("/users/autocomplete?name={}", urlencode(&query)),
        };
        match state.rest.autocomplete_users(&route).await {
            Ok(users) => {
                let store = state.store.clone();
                let held = users.clone();
                // Remembered, so the next keystroke is local.
                let _ = tokio::task::spawn_blocking(move || store.upsert_users(&held)).await;
                for user in users {
                    if found.iter().any(|row| row.id == user.id) {
                        continue;
                    }
                    found.push(Suggestion {
                        value: user.username.clone(),
                        label: user.username.clone(),
                        detail: matterless_render::display_name(&user, "full_name"),
                        avatar_at: user.last_picture_update,
                        id: user.id,
                        kind: "user".into(),
                        channel_type: String::new(),
                    });
                }
            }
            Err(error) => tracing::debug!(%error, "user autocomplete failed; local only"),
        }
    }

    found.truncate(cap as usize);
    tracing::debug!(kind = %kind, chars = query.chars().count(), rows = found.len(), "suggested");
    let _ = mode;
    Ok(found)
}

/// Percent-encodes a query value.
///
/// By hand because it is four characters of alphabet check: pulling a crate in
/// for one query parameter, or interpolating a name with a `&` in it straight
/// into a URL, are both worse.
fn urlencode(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for byte in value.as_bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(*byte as char)
            }
            other => out.push_str(&format!("%{other:02X}")),
        }
    }
    out
}

/// Opens (or finds) the direct message channel with another person.
///
/// Mattermost creates it on demand and returns the existing one if there is
/// already history, so this is idempotent -- there is no "already exists" case
/// to handle.
/// A public channel the reader could join.
#[derive(Debug, Clone, Serialize)]
pub struct Joinable {
    pub id: String,
    pub name: String,
    pub display_name: String,
    pub purpose: String,
    pub channel_type: String,
    /// True when the reader is already in it: shown, but as "open" rather than
    /// "join", because a browse list that hides them makes the reader wonder
    /// whether the channel exists at all.
    pub joined: bool,
}

/// The public channels of a team, for browsing.
///
/// From the server, not the local table: the whole point is the channels this
/// reader is *not* in, and those have never been synced. Membership is local
/// though, which is what marks the ones already joined.
#[tauri::command]
pub async fn browse_channels(
    state: State<'_, AppState>,
    team_id: String,
    query: String,
) -> Reply<Vec<Joinable>> {
    const PER_PAGE: u32 = 200;
    const PAGES: u32 = 10;
    let mut found: Vec<matterless_core::rest::PublicChannel> = Vec::new();
    for page in 0..PAGES {
        let batch = state
            .rest
            .public_channels(&team_id, page, PER_PAGE)
            .await
            .map_err(fail)?;
        let count = batch.len();
        found.extend(batch);
        if (count as u32) < PER_PAGE {
            break;
        }
    }

    let store = state.store.clone();
    let ids: Vec<String> = found.iter().map(|channel| channel.id.clone()).collect();
    let mine = tokio::task::spawn_blocking(move || store.known_channel_ids(&ids))
        .await
        .map_err(fail)?
        .map_err(fail)?;

    let wanted = query.trim().to_lowercase();
    let mut rows: Vec<Joinable> = found
        .into_iter()
        .filter(|channel| channel.delete_at == 0)
        .filter(|channel| {
            wanted.is_empty()
                || channel.display_name.to_lowercase().contains(&wanted)
                || channel.name.to_lowercase().contains(&wanted)
                || channel.purpose.to_lowercase().contains(&wanted)
        })
        .map(|channel| Joinable {
            joined: mine.contains(&channel.id),
            id: channel.id,
            name: channel.name,
            display_name: channel.display_name,
            purpose: channel.purpose,
            channel_type: channel.channel_type,
        })
        .collect();
    // Ones to join first, then by name: a browse list is for finding something
    // new, and the ones already joined are reachable from the sidebar anyway.
    rows.sort_by(|one, other| {
        one.joined.cmp(&other.joined).then_with(|| {
            one.display_name
                .to_lowercase()
                .cmp(&other.display_name.to_lowercase())
        })
    });
    tracing::info!(team = %team_id, shown = rows.len(), "browsed public channels");
    Ok(rows)
}

/// Joins a public channel and makes it local.
#[tauri::command]
pub async fn join_channel(
    state: State<'_, AppState>,
    channel_id: String,
    me_id: String,
) -> Reply<()> {
    state
        .rest
        .join_channel(&channel_id, &me_id)
        .await
        .map_err(fail)?;
    tracing::info!(channel = %channel_id, "joined a channel");
    Ok(())
}

/// Leaves a channel.
#[tauri::command]
pub async fn leave_channel(
    state: State<'_, AppState>,
    channel_id: String,
    me_id: String,
) -> Reply<()> {
    state
        .rest
        .leave_channel(&channel_id, &me_id)
        .await
        .map_err(fail)?;
    let store = state.store.clone();
    let gone = channel_id.clone();
    // Forgotten locally too, or the sidebar keeps showing a channel the server
    // no longer counts the reader a member of.
    tokio::task::spawn_blocking(move || store.forget_channel(&gone))
        .await
        .map_err(fail)?
        .map_err(fail)?;
    tracing::info!(channel = %channel_id, "left a channel");
    Ok(())
}

/// Finds or creates a group conversation with these people.
#[tauri::command]
pub async fn open_group_message(
    state: State<'_, AppState>,
    me_id: String,
    user_ids: Vec<String>,
) -> Reply<String> {
    // The reader is part of their own group: the server keys the channel on its
    // whole membership, so leaving oneself out asks for a different one.
    let mut everyone = user_ids;
    if !everyone.contains(&me_id) {
        everyone.push(me_id);
    }
    let channel = state.rest.group_channel(&everyone).await.map_err(fail)?;
    let channel_id = channel.id.clone();
    let store = state.store.clone();
    let held = channel.clone();
    tokio::task::spawn_blocking(move || store.upsert_channels(std::slice::from_ref(&held)))
        .await
        .map_err(fail)?
        .map_err(fail)?;
    tracing::info!(channel = %channel_id, people = everyone.len(), "group message opened");
    Ok(channel_id)
}

#[tauri::command]
pub async fn open_direct_message(
    state: State<'_, AppState>,
    me_id: String,
    user_id: String,
) -> Reply<String> {
    let channel = state
        .rest
        .direct_channel(&me_id, &user_id)
        .await
        .map_err(fail)?;
    let channel_id = channel.id.clone();

    let store = state.store.clone();
    let held = channel.clone();
    tokio::task::spawn_blocking(move || store.upsert_channels(std::slice::from_ref(&held)))
        .await
        .map_err(fail)?
        .map_err(fail)?;
    tracing::info!(channel = %channel_id, "direct message opened");
    Ok(channel_id)
}

/// Channels and people in one ranked list, for the quick switcher.
///
/// Both halves are scored by the same function, which is what makes one list
/// out of two sources honest -- a channel and a person can be compared because
/// the number means the same thing. Channels carry a small bonus: they are
/// places the reader is already in, and a switcher is usually reached to go
/// somewhere rather than to start something.
///
/// Direct and group channels are deliberately *not* offered as channels. A DM's
/// stored `name` is the `id__id` pair and its display name is empty, so it can
/// only be matched by nonsense; people cover them instead, and picking a person
/// opens the DM that already exists rather than making a second one.
#[tauri::command]
pub async fn switcher(
    state: State<'_, AppState>,
    query: String,
    display_mode: String,
) -> Reply<Vec<Suggestion>> {
    /// A place you are already in beats a person with the same score.
    const CHANNEL_BONUS: i32 = 12;

    let store = state.store.clone();
    let probe = query.clone();
    let mode = display_mode.clone();

    let mut found = tokio::task::spawn_blocking(
        move || -> matterless_store::Result<Vec<(i32, Suggestion)>> {
            let mut rows: Vec<(i32, Suggestion)> = Vec::new();

            for channel in store.channels_matching(&probe, SWITCHER_ROWS)? {
                if channel.channel_type == "D" || channel.channel_type == "G" {
                    continue;
                }
                let label = if channel.display_name.is_empty() {
                    channel.name.clone()
                } else {
                    channel.display_name.clone()
                };
                let score = matterless_core::fuzzy::best_score(
                    [label.as_str(), channel.name.as_str()],
                    &probe,
                )
                .unwrap_or(0)
                    + CHANNEL_BONUS;
                rows.push((
                    score,
                    Suggestion {
                        value: channel.id.clone(),
                        label,
                        detail: String::new(),
                        id: channel.id,
                        kind: "channel".into(),
                        avatar_at: 0,
                        channel_type: channel.channel_type,
                    },
                ));
            }

            for user in store.users_matching(&probe, SWITCHER_ROWS)? {
                let full = format!("{} {}", user.first_name, user.last_name);
                let score = matterless_core::fuzzy::best_score(
                    [user.username.as_str(), full.trim()],
                    &probe,
                )
                .unwrap_or(0);
                rows.push((
                    score,
                    Suggestion {
                        value: user.id.clone(),
                        label: user.username.clone(),
                        detail: matterless_render::display_name(&user, &mode),
                        avatar_at: user.last_picture_update,
                        id: user.id,
                        kind: "user".into(),
                        channel_type: String::new(),
                    },
                ));
            }
            Ok(rows)
        },
    )
    .await
    .map_err(fail)?
    .map_err(fail)?;

    found.sort_by_key(|(score, _)| std::cmp::Reverse(*score));
    found.truncate(SWITCHER_ROWS as usize);
    tracing::debug!(
        chars = query.chars().count(),
        rows = found.len(),
        "switcher"
    );
    Ok(found.into_iter().map(|(_, row)| row).collect())
}

/// Rows the switcher offers. A list you have to scan is not a shortcut.
const SWITCHER_ROWS: u32 = 10;

/// One search result, ready to draw.
#[derive(Debug, Clone, Serialize)]
pub struct SearchHit {
    pub post_id: String,
    pub channel_id: String,
    /// The channel as the sidebar labels it -- a DM says who it is with rather
    /// than showing its `id__id` name.
    pub channel_label: String,
    pub author_name: String,
    pub create_at: matterless_core::model::Timestamp,
    /// The message, parsed the same way the stream parses it, so a result reads
    /// like the message it points at rather than like markdown source.
    pub nodes: Vec<Node>,
    /// True when this hit came from the local index alone -- the server has not
    /// confirmed it. Only possible while offline.
    pub local_only: bool,
}

/// Results to keep. Two teams answer 100 each; a reader scans the first handful.
const SEARCH_HITS: usize = 50;

/// Searches messages: the local index first, then the server, merged.
///
/// The local pass is what makes typing feel instant -- FTS5 over what has been
/// fetched answers in microseconds -- but it can only ever know part of the
/// history, so the server's answer is the authoritative one and is folded in as
/// it arrives. Posts the server returns are stored, so a search also warms the
/// cache for the channels it touched.
///
/// **Deduped by post id across teams.** A search runs per team and a DM or
/// group channel belongs to every team, so the same message comes back twice --
/// the same trap that once turned 114 channels into 172 and 163 threads into
/// 204.
/// Where a jumped-to post sits, once its surroundings are local.
#[derive(Debug, Clone, Serialize)]
pub struct Island {
    /// The post itself, so the shell can ask for a window that contains it.
    pub create_at: matterless_core::model::Timestamp,
    /// The oldest and newest posts fetched around it, as timestamps: the bounds
    /// of the island, and what says how much context there is on each side.
    pub oldest: matterless_core::model::Timestamp,
    pub newest: matterless_core::model::Timestamp,
    /// True when the window reached the channel's newest post, so the island is
    /// not an island at all -- it joins the live page and there is no gap.
    pub reaches_present: bool,
}

/// Fetches the posts either side of one and stores them.
///
/// Jumping needs context, not a post: a message on its own is unreadable and
/// the reader cannot tell what they are looking at. So a window comes down
/// around it -- and because that window is nowhere near the newest page, what
/// it makes is an *island*, disjoint from the contiguous history this client
/// holds. The gap between the two is the thing that has to be remembered; a
/// client that forgets it renders the two ranges as if they touched, and the
/// scrollback quietly grows a hole.
///
/// Walking backwards a page at a time was the alternative, and is what this
/// replaces: it costs one request per page and simply fails for anything older
/// than the reader's patience.
#[tauri::command]
pub async fn fetch_around(
    state: State<'_, AppState>,
    channel_id: String,
    post_id: String,
    span: u32,
) -> Reply<Island> {
    let started = Instant::now();
    let target = state.rest.post(&post_id).await.map_err(fail)?;
    let before = state
        .rest
        .posts_before(&channel_id, &post_id, span)
        .await
        .map_err(fail)?;
    let after = state
        .rest
        .posts_after(&channel_id, &post_id, span)
        .await
        .map_err(fail)?;

    // `has_next` on the *after* page is the server saying there is more between
    // here and the present. Without it the shell cannot tell an island from a
    // window that happens to reach the end.
    let reaches_present = !after.has_next;

    let mut posts: Vec<matterless_core::Post> = Vec::new();
    posts.extend(before.posts.into_values());
    posts.extend(after.posts.into_values());
    let create_at = target.create_at;
    posts.push(target);
    posts.retain(|post| post.delete_at == 0);

    let oldest = posts
        .iter()
        .map(|post| post.create_at)
        .min()
        .unwrap_or(create_at);
    let newest = posts
        .iter()
        .map(|post| post.create_at)
        .max()
        .unwrap_or(create_at);
    let held = posts.len();
    remember(&state, &posts).await?;

    tracing::info!(
        channel = %channel_id,
        post = %post_id,
        posts = held,
        reaches_present,
        ms = started.elapsed().as_secs_f64() * 1000.0,
        "fetched the window around a post"
    );
    Ok(Island {
        create_at,
        oldest,
        newest,
        reaches_present,
    })
}

/// The reader's saved messages, newest first.
///
/// Fetched from the server rather than read locally: saving is a preference, so
/// the store knows *which* posts are saved but may never have held the posts
/// themselves -- a message saved from a channel this client has not opened is
/// exactly the one worth listing. They are stored on the way through, so the
/// list also warms the cache for opening them.
#[tauri::command]
pub async fn saved_messages(
    state: State<'_, AppState>,
    me_id: String,
    display_mode: String,
) -> Reply<Vec<SearchHit>> {
    let list = state
        .rest
        .flagged_posts(&me_id, SEARCH_HITS as u32)
        .await
        .map_err(fail)?;
    let posts = ordered(list);
    tracing::info!(count = posts.len(), "listed saved messages");
    remember(&state, &posts).await?;
    hits_for(&state, posts, &me_id, &display_mode).await
}

/// The messages pinned in a channel, newest first.
#[tauri::command]
pub async fn pinned_messages(
    state: State<'_, AppState>,
    channel_id: String,
    me_id: String,
    display_mode: String,
) -> Reply<Vec<SearchHit>> {
    let list = state.rest.pinned_posts(&channel_id).await.map_err(fail)?;
    let posts = ordered(list);
    tracing::info!(channel = %channel_id, count = posts.len(), "listed pinned messages");
    remember(&state, &posts).await?;
    hits_for(&state, posts, &me_id, &display_mode).await
}

/// A post list as posts, newest first.
///
/// `order` is the server's own ordering and is the only thing that carries it:
/// `posts` is a map, whose iteration order means nothing.
fn ordered(list: matterless_core::model::PostList) -> Vec<matterless_core::Post> {
    let mut posts: Vec<matterless_core::Post> = list
        .order
        .iter()
        .filter_map(|id| list.posts.get(id).cloned())
        .filter(|post| post.delete_at == 0)
        .collect();
    posts.sort_by_key(|post| std::cmp::Reverse(post.create_at));
    posts
}

/// Stores posts a panel just fetched, so opening one does not fetch it again.
async fn remember(state: &AppState, posts: &[matterless_core::Post]) -> Result<(), String> {
    if posts.is_empty() {
        return Ok(());
    }
    let store = state.store.clone();
    let held: Vec<matterless_core::Post> = posts.to_vec();
    // The outcomes say which were new, which nothing here needs: this exists so
    // opening one of these results reads it from the store rather than the wire.
    tokio::task::spawn_blocking(move || store.upsert_posts(&held))
        .await
        .map_err(fail)?
        .map_err(fail)?;
    Ok(())
}

/// Everything a result row needs, resolved from the store.
///
/// Shared by search, saved messages and pinned messages: all three are lists of
/// posts drawn the same way, and three copies of this would be three places for
/// a channel label or an author name to be resolved differently.
async fn hits_for(
    state: &AppState,
    posts: Vec<matterless_core::Post>,
    me_id: &str,
    display_mode: &str,
) -> Result<Vec<SearchHit>, String> {
    let store = state.store.clone();
    let renders = state.renders.clone();
    let viewer = me_id.to_string();
    let mode = display_mode.to_string();
    tokio::task::spawn_blocking(move || -> matterless_store::Result<Vec<SearchHit>> {
        let author_ids: Vec<String> = posts.iter().map(|post| post.user_id.clone()).collect();
        let authors = store.users_by_ids(&author_ids)?;
        // A DM has no display name of its own, so labelling one needs the
        // counterpart's name -- the same resolution the sidebar does.
        let counterparts: Vec<String> = posts
            .iter()
            .filter_map(|post| store.channel(&post.channel_id).ok().flatten())
            .filter(|channel| channel.channel_type == "D")
            .filter_map(|channel| direct_message_counterpart(&channel.name, &viewer))
            .collect();
        let named = store.users_by_ids(&counterparts)?;
        let names: HashMap<String, String> = named
            .into_iter()
            .map(|(id, user)| (id, matterless_render::display_name(&user, &mode)))
            .collect();

        let mut rows = Vec::with_capacity(posts.len());
        for post in posts {
            let channel_label = match store.channel(&post.channel_id)? {
                Some(channel) => label_for_channel(&channel, &viewer, &names),
                None => post.channel_id.clone(),
            };
            // The same cache the stream uses, keyed by post and edit: a result
            // for a message already on screen costs no parse at all.
            let nodes = match renders.get(&post.id, post.update_at) {
                Some(nodes) => nodes,
                None => {
                    let parsed =
                        std::sync::Arc::new(matterless_render::markdown::parse(&post.message));
                    renders.put(&post.id, post.update_at, std::sync::Arc::clone(&parsed));
                    parsed
                }
            };
            rows.push(SearchHit {
                author_name: authors
                    .get(&post.user_id)
                    .map(|user| matterless_render::display_name(user, &mode))
                    .unwrap_or_else(|| post.user_id.clone()),
                channel_label,
                channel_id: post.channel_id,
                create_at: post.create_at,
                nodes: (*nodes).clone(),
                local_only: false,
                post_id: post.id,
            });
        }
        Ok(rows)
    })
    .await
    .map_err(fail)?
    .map_err(fail)
}

#[tauri::command]
pub async fn search_messages(
    state: State<'_, AppState>,
    query: String,
    me_id: String,
    display_mode: String,
    utc_offset_minutes: i32,
) -> Reply<Vec<SearchHit>> {
    let typed = query.trim().to_string();
    if typed.is_empty() {
        return Ok(Vec::new());
    }
    let started = Instant::now();

    // ---- the local index, immediately ----------------------------------
    //
    // Modifiers included: `from:` names a user this store holds, `in:` a
    // channel it holds, and the dates are a comparison on `create_at`. An
    // earlier version sent those to the server alone on the grounds that "the
    // index knows nothing about authors" -- which was simply untrue, and made
    // the local half useless for exactly the queries a reader repeats.
    let parsed = matterless_core::search::SearchQuery::parse(&typed, utc_offset_minutes);
    let mut posts: Vec<matterless_core::Post> = Vec::new();
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();

    {
        let store = state.store.clone();
        let held = parsed.clone();
        match tokio::task::spawn_blocking(move || store.search_query(&held, SEARCH_HITS as u32))
            .await
            .map_err(fail)?
        {
            Ok(found) => {
                for post in found {
                    if seen.insert(post.id.clone()) {
                        posts.push(post);
                    }
                }
            }
            // A miss, not a failure: the server pass still answers.
            Err(error) => tracing::debug!(%error, "local search skipped"),
        }
    }
    let local_hits = posts.len();

    // ---- the server, across every team ---------------------------------
    let mut from_server = 0usize;
    match state.rest.my_teams().await {
        Ok(teams) => {
            for team in teams {
                match state.rest.search_posts(&team.id, &typed).await {
                    Ok(list) => {
                        let found: Vec<matterless_core::Post> = list
                            .order
                            .iter()
                            .filter_map(|id| list.posts.get(id).cloned())
                            .collect();
                        from_server += found.len();
                        // Stored: they are real posts, and keeping them means
                        // the channel they belong to is that much warmer.
                        let store = state.store.clone();
                        let held = found.clone();
                        let _ = tokio::task::spawn_blocking(move || store.upsert_posts(&held))
                            .await
                            .map_err(fail)?;
                        for post in found {
                            if post.delete_at == 0 && seen.insert(post.id.clone()) {
                                posts.push(post);
                            }
                        }
                    }
                    Err(error) => tracing::warn!(team = %team.id, %error, "search failed"),
                }
            }
        }
        Err(error) => tracing::warn!(%error, "no teams to search"),
    }

    // Newest first, which is what a chat search means by relevance.
    posts.sort_by_key(|post| std::cmp::Reverse(post.create_at));
    posts.truncate(SEARCH_HITS);

    // ---- everything a row needs, resolved here -------------------------
    let hits = hits_for(&state, posts, &me_id, &display_mode).await?;

    tracing::info!(
        chars = typed.chars().count(),
        local = local_hits,
        server = from_server,
        hits = hits.len(),
        filtered = !parsed.from.is_empty()
            || !parsed.in_channels.is_empty()
            || parsed.since.is_some()
            || parsed.until.is_some(),
        ms = started.elapsed().as_secs_f64() * 1000.0,
        "searched"
    );
    Ok(hits)
}
