//! The one task that owns mutable sync state.
//!
//! `SyncContext` changes as the user switches channel or the window loses focus,
//! and it is read on every websocket event. Sharing it behind a lock would mean
//! holding a guard across an await on the very hot path. Giving it to a single
//! task and talking to that task over a channel makes the hazard unreachable.
//!
//! Deltas sent to the UI deliberately carry **no post content** -- only the
//! channel that changed. The frontend then asks for that channel's row plan, so
//! there is exactly one way a row reaches the screen.

use crate::pending::PendingPosts;
use matterless_core::ws::{Signal, WsSession};
use matterless_core::{AuthToken, Event, RestClient, User};
use matterless_sync::{Arrival, Delta, SyncContext, SyncEngine};
use serde::Serialize;
use std::sync::Arc;
use std::time::Instant;
use tauri::ipc::Channel;
use tokio::sync::mpsc;

/// What the UI tells the engine.
pub enum EngineMsg {
    Subscribe(Channel<UiDelta>),
    SetActiveChannel(Option<String>),
    SetFocus(bool),
    /// Sent once the account is known, which is what starts the websocket.
    Start {
        me: Box<User>,
        thread_mode: matterless_core::ThreadMode,
    },
    /// A channel's history was refreshed over REST by a command.
    Refreshed(String),
    /// This reader's own presence changed, so the notification policy has to
    /// know: the do-not-disturb rule is decided against it, and until this
    /// existed `SyncContext::status` stayed at its "online" default forever --
    /// which meant setting yourself to Do Not Disturb suppressed nothing.
    OwnStatus(String),
    /// The reader is typing. Throttled here rather than in the shell, so there
    /// is one answer to "how often does this go out" -- the server asks for one
    /// every `TimeBetweenUserTypingUpdatesMilliseconds` (5000 ms here).
    Typing {
        channel_id: String,
        root_id: String,
    },
    /// A command changed a channel's unread locally (marking it read).
    Unread {
        channel_id: String,
        messages: i64,
        mentions: i64,
        muted: bool,
    },
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum UiDelta {
    /// "This channel's plan changed." No content: the UI refetches the plan.
    Channel {
        channel_id: String,
        /// Milliseconds from the websocket frame being read to this delta being
        /// emitted -- the Rust half of the websocket-to-glyph measurement.
        /// `None` when the change came from a REST refresh, where there is no
        /// frame to measure from.
        ws_ms: Option<f64>,
        /// Wall clock at emit, so the shell can price the IPC hop it cannot see
        /// the start of. Coarse by nature: it is the one number here that
        /// crosses a clock domain.
        emitted_at_ms: Option<f64>,
    },
    Unread {
        channel_id: String,
        messages: i64,
        mentions: i64,
        muted: bool,
    },
    /// The reader rearranged their sidebar, here or elsewhere.
    ///
    /// Carries the team so the shell can say what changed in the log; the
    /// regroup itself asks for every team, because a channel can be dragged
    /// between categories and only the server knows the result.
    Sidebar { team_id: String },
    /// Somebody's presence. Ephemeral, and its own scope in the store: a dot
    /// changing colour must never invalidate a message row.
    Status { user_id: String, status: String },
    /// Ephemeral, and routed to its own store scope. Typing measured 72-93% of
    /// all traffic, so it must never touch the message list.
    Typing {
        channel_id: String,
        user_id: String,
        /// The thread being typed in, empty for the channel itself.
        root_id: String,
    },
    /// A post that earned a notification. The whole policy already ran in
    /// Rust -- own posts, system messages, DND, muting, the level, unfollowed
    /// threads and the channel being read while focused are all already
    /// excluded, so the shell raises this unconditionally.
    ///
    /// Unlike every other delta this one carries content, deliberately: a toast
    /// is a one-shot side effect rather than a render, so there is no second
    /// render path to create, and a round trip here would cost latency at the
    /// one moment it is most visible.
    Notify {
        channel_id: String,
        post_id: String,
        /// Who to name in the toast, already resolved.
        author: String,
        /// Where it landed, for the toast's title -- or what kind of
        /// conversation it is, when naming it would just repeat the author.
        channel: String,
        /// A short single-line preview.
        preview: String,
        /// A direct or group message, where the person *is* the conversation.
        direct: bool,
    },
    /// A followed thread gained a reply, or its read state moved.
    ThreadChanged { root_id: String, channel_id: String },
    Connection {
        connected: bool,
        /// True when history may have a hole and a catch-up is needed.
        resync: bool,
    },
}

/// Trims a message to one short line for a toast. Newlines and long bodies
/// both make Windows truncate unhelpfully, so it is done here where the whole
/// text is available.
/// What to call the conversation in a toast, and whether it is a direct one.
///
/// A DM has no display name of its own and its `name` is the pair of user ids,
/// so falling back to the name put a wall of hex in the title. Naming the *kind*
/// of conversation instead is what the official client does, and it avoids
/// repeating the person, who is already in the body.
fn conversation_label(channel: Option<matterless_core::Channel>) -> (String, bool) {
    match channel {
        Some(channel) if channel.channel_type == "D" => ("Direct Message".to_string(), true),
        Some(channel) if channel.channel_type == "G" => ("Group Message".to_string(), true),
        Some(channel) if !channel.display_name.is_empty() => (channel.display_name, false),
        // A channel with no display name at all: its name is at least readable.
        Some(channel) => (channel.name, false),
        // Not held locally yet. An empty title leaves the shell to fall back.
        None => (String::new(), false),
    }
}

/// What a toast shows. Assembled in Rust because deciding it needs the store.
#[derive(Debug, Default)]
struct ToastText {
    author: String,
    /// Who wrote it, so an unresolved name can be fetched.
    author_id: String,
    /// False when `author` is standing in as a raw id because the local store
    /// has never met this person -- which a fresh install has not, for almost
    /// everybody.
    resolved: bool,
    /// The conversation's name, or what kind of conversation it is.
    channel: String,
    preview: String,
    /// A direct or group message, where the person is the conversation.
    direct: bool,
}

/// Milliseconds since the epoch, matching what `Date.now()` reads in the shell.
fn wall_clock_ms() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|since| since.as_secs_f64() * 1000.0)
        .unwrap_or(0.0)
}

fn preview_of(message: &str) -> String {
    let single_line: String = message
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    let mut short: String = single_line.chars().take(140).collect();
    if single_line.chars().count() > 140 {
        short.push('…');
    }
    short
}

struct Runner {
    engine: Arc<SyncEngine>,
    rest: Arc<RestClient>,
    pending: Arc<PendingPosts>,
    context: Option<SyncContext>,
    subscriber: Option<Channel<UiDelta>>,
    pending_active: Option<String>,
    pending_focus: bool,
    /// Remembered so a subscriber that arrives after the socket is already up
    /// still learns the truth. Without this the badge is a race.
    /// Whether the websocket is up. Shared with `AppState` so `bootstrap` can
    /// report the current state rather than assuming "offline".
    connected: Arc<std::sync::atomic::AtomicBool>,
    /// When "typing" last went out per conversation.
    last_typing: TypingThrottle,
}

/// Rate-limits outbound typing, per conversation.
///
/// Its own type so the rule is testable without a clock: `now` is passed in.
/// Per conversation rather than globally, because typing in a thread and typing
/// in its channel are different claims -- the server carries `parent_id` to
/// tell them apart.
#[derive(Default)]
struct TypingThrottle {
    sent: std::collections::HashMap<String, std::time::Instant>,
}

impl TypingThrottle {
    fn is_due(&mut self, channel_id: &str, root_id: &str, now: std::time::Instant) -> bool {
        let key = if root_id.is_empty() {
            channel_id.to_string()
        } else {
            format!("{channel_id}/{root_id}")
        };
        match self.sent.get(&key) {
            Some(sent) if now.duration_since(*sent) < TYPING_EVERY => false,
            _ => {
                self.sent.insert(key, now);
                true
            }
        }
    }
}

/// How often a typing notice goes out per conversation. The server publishes
/// its own interval as `TimeBetweenUserTypingUpdatesMilliseconds` (5000 ms
/// here) and the official client honours it; sending faster is noise every
/// other member's client has to handle.
const TYPING_EVERY: std::time::Duration = std::time::Duration::from_millis(5000);

impl Runner {
    /// Whether the reader has collapsed threads on, which decides whether a
    /// reply counts towards its channel.
    fn collapsed(&self) -> bool {
        self.context
            .as_ref()
            .map(|context| context.thread_mode == matterless_core::ThreadMode::Collapsed)
            .unwrap_or(true)
    }

    fn emit(&self, delta: UiDelta) {
        // A notification carries the message itself, and the log never holds
        // message text: its length says as much about "did the preview build"
        // without keeping a word of it.
        if let UiDelta::Notify {
            channel_id,
            post_id,
            author,
            preview,
            direct,
            ..
        } = &delta
        {
            tracing::debug!(
                %channel_id,
                %post_id,
                %author,
                preview_chars = preview.chars().count(),
                direct,
                "emitting a notification"
            );
        } else {
            tracing::debug!(delta = ?delta, "emitting");
        }
        if let Some(subscriber) = &self.subscriber
            && let Err(error) = subscriber.send(delta)
        {
            tracing::warn!(%error, "the ui delta channel is gone");
        }
    }

    /// What a toast says, read from the store.
    ///
    /// Falls back to ids rather than failing: a notification with an ugly name
    /// still tells you something happened, where no notification tells you
    /// nothing.
    fn notification_text(&self, post_id: &str) -> ToastText {
        let store = self.engine.store();
        let Ok(Some(post)) = store.post(post_id) else {
            return ToastText::default();
        };
        let known = store
            .users_by_ids(std::slice::from_ref(&post.user_id))
            .ok()
            .and_then(|found| found.get(&post.user_id).map(|user| user.username.clone()));

        let (channel, direct) = conversation_label(store.channel(&post.channel_id).ok().flatten());
        ToastText {
            resolved: known.is_some(),
            author: known.unwrap_or_else(|| post.user_id.clone()),
            author_id: post.user_id.clone(),
            channel,
            preview: preview_of(&post.message),
            direct,
        }
    }

    /// Fetches a stranger's name, then sends the notification.
    ///
    /// Only for an author the store has never seen: on a fresh install that is
    /// most people, and falling back to the raw id put `jpztezkyefyydkp79o63`
    /// in a toast where the name belonged. One request, off the hot path, and
    /// the toast waits for it rather than being shown twice.
    fn notify_once_named(&self, channel_id: String, post_id: String, text: ToastText) {
        let Some(subscriber) = self.subscriber.clone() else {
            return;
        };
        let rest = Arc::clone(&self.rest);
        let store = Arc::clone(self.engine.store());
        tauri::async_runtime::spawn(async move {
            let mut author = text.author;
            match rest
                .users_by_ids(std::slice::from_ref(&text.author_id))
                .await
            {
                Ok(fetched) => {
                    if let Some(found) = fetched.iter().find(|user| user.id == text.author_id) {
                        author = found.username.clone();
                    }
                    let held = fetched.clone();
                    // Kept, so the next message from this person needs no fetch.
                    let _ = tokio::task::spawn_blocking(move || store.upsert_users(&held)).await;
                }
                Err(error) => {
                    // The id still says something happened, which is better
                    // than staying silent about a message.
                    tracing::warn!(%error, "could not name the author of a notification");
                }
            }
            let delta = UiDelta::Notify {
                channel_id,
                post_id,
                author,
                channel: text.channel,
                preview: text.preview,
                direct: text.direct,
            };
            if let Err(error) = subscriber.send(delta) {
                tracing::warn!(%error, "the ui delta channel is gone");
            }
        });
    }

    /// Translates engine deltas into the narrow set the UI needs.
    ///
    /// `read_at` is `Some` only for deltas a websocket frame caused, and is what
    /// makes the shell's paint measurement comparable to the moment the bytes
    /// arrived.
    fn forward(&mut self, deltas: Vec<Delta>, read_at: Option<Instant>) {
        let mut touched: Option<String> = None;
        for delta in deltas {
            match delta {
                Delta::PostUpserted {
                    channel_id,
                    notify,
                    post_id,
                    arrival,
                    ..
                } => {
                    touched = Some(channel_id.clone());
                    // `notify` already accounts for arrival, mentions, muting,
                    // focus and thread following; nothing is re-decided here.
                    if notify && arrival == Arrival::Live {
                        let text = self.notification_text(&post_id);
                        if text.resolved {
                            self.emit(UiDelta::Notify {
                                channel_id,
                                post_id,
                                author: text.author,
                                channel: text.channel,
                                preview: text.preview,
                                direct: text.direct,
                            });
                        } else {
                            self.notify_once_named(channel_id, post_id, text);
                        }
                    }
                }
                Delta::PostTombstoned { channel_id, .. } => touched = Some(channel_id),
                Delta::UnreadChanged { channel_id, unread } => {
                    // In the reader's mode: under collapsed threads a reply is
                    // not channel unread.
                    let (messages, mentions) = unread.visible(self.collapsed());
                    self.emit(UiDelta::Unread {
                        channel_id,
                        messages,
                        mentions,
                        muted: unread.muted,
                    })
                }
                // A thread's counts live on its footer row, which is part of
                // its channel's plan -- so this is a plan change like any
                // other, and goes through the one render path.
                Delta::ThreadChanged {
                    root_id,
                    channel_id,
                } => {
                    // Two things change, not one. The footer's counts are part
                    // of the channel's plan, and the threads view is its own
                    // list ordered by activity -- so a reply can move any row
                    // in it, not just this one.
                    self.emit(UiDelta::ThreadChanged {
                        root_id,
                        channel_id: channel_id.clone(),
                    });
                    if !channel_id.is_empty() {
                        touched = Some(channel_id);
                    }
                }
                Delta::ReactionsChanged { post_id } => {
                    // Reactions live on a row, so the plan is what changes.
                    if let Ok(Some(post)) = self.engine.store().post(&post_id) {
                        touched = Some(post.channel_id);
                    }
                }
                Delta::Typing {
                    channel_id,
                    user_id,
                    root_id,
                } => self.emit(UiDelta::Typing {
                    channel_id,
                    user_id,
                    root_id,
                }),
                Delta::ResyncRequired => self.emit(UiDelta::Connection {
                    connected: true,
                    resync: true,
                }),
                // Categories moved: no message row changes, but the sidebar's
                // grouping did, and it is server-side state rather than
                // something the shell can work out.
                Delta::SidebarChanged { team_id } => self.emit(UiDelta::Sidebar { team_id }),
                Delta::StatusChanged { user_id, status } => {
                    // Somebody's dot moves. Not a row change, so it does not go
                    // near the plan -- but the reader's *own* status is also the
                    // do-not-disturb rule's input, and it can be changed from
                    // another device.
                    if let Some(context) = self.context.as_mut()
                        && context.me.id == user_id
                    {
                        context.status = status.clone();
                    }
                    self.emit(UiDelta::Status { user_id, status });
                }
                // Preference and emoji changes are surfaced in a later slice;
                // they do not alter a message row.
                Delta::PreferencesChanged { .. }
                | Delta::CustomEmojiChanged
                | Delta::UserChanged { .. } => {}
            }
        }
        if let Some(channel_id) = touched {
            self.emit(UiDelta::Channel {
                channel_id,
                ws_ms: read_at.map(|read_at| read_at.elapsed().as_secs_f64() * 1000.0),
                emitted_at_ms: read_at.map(|_| wall_clock_ms()),
            });
        }
    }

    fn typing_is_due(&mut self, channel_id: &str, root_id: &str) -> bool {
        self.last_typing
            .is_due(channel_id, root_id, std::time::Instant::now())
    }

    fn handle_message(
        &mut self,
        message: EngineMsg,
    ) -> Option<(User, matterless_core::ThreadMode)> {
        match message {
            EngineMsg::Subscribe(channel) => {
                self.subscriber = Some(channel);
                // Replay current state rather than waiting for it to change.
                self.emit(UiDelta::Connection {
                    connected: self.connected.load(std::sync::atomic::Ordering::Relaxed),
                    resync: false,
                });
                None
            }
            EngineMsg::SetActiveChannel(channel_id) => {
                self.pending_active = channel_id.clone();
                if let Some(context) = self.context.as_mut() {
                    context.active_channel = channel_id;
                }
                None
            }
            EngineMsg::SetFocus(focused) => {
                self.pending_focus = focused;
                if let Some(context) = self.context.as_mut() {
                    context.window_focused = focused;
                }
                None
            }
            EngineMsg::OwnStatus(status) => {
                if let Some(context) = self.context.as_mut() {
                    tracing::info!(status = %status, "own presence");
                    context.status = status;
                }
                None
            }
            EngineMsg::Typing { .. } => {
                // Nothing to do here: sending needs the socket, which the run
                // loop owns, so it intercepts this one before we see it. This
                // arm exists for the phase before the socket is up, where a
                // keystroke has nowhere to go.
                None
            }
            EngineMsg::Refreshed(channel_id) => {
                self.emit(UiDelta::Channel {
                    channel_id,
                    // A REST refresh, not a frame: nothing to time from.
                    ws_ms: None,
                    emitted_at_ms: None,
                });
                None
            }
            EngineMsg::Unread {
                channel_id,
                messages,
                mentions,
                muted,
            } => {
                self.emit(UiDelta::Unread {
                    channel_id,
                    messages,
                    mentions,
                    muted,
                });
                None
            }
            EngineMsg::Start { me, thread_mode } => Some((*me, thread_mode)),
        }
    }
}

pub fn spawn(
    engine: Arc<SyncEngine>,
    rest: Arc<RestClient>,
    pending: Arc<PendingPosts>,
    mut from_ui: mpsc::Receiver<EngineMsg>,
    connected: Arc<std::sync::atomic::AtomicBool>,
) {
    // `tauri::async_runtime::spawn`, not `tokio::spawn`: this is called from
    // `setup()`, which runs on the main thread before a tokio reactor is
    // installed. Tauri owns the runtime and hands out this handle.
    tauri::async_runtime::spawn(async move {
        let mut runner = Runner {
            engine,
            rest,
            pending,
            context: None,
            subscriber: None,
            pending_active: None,
            pending_focus: true,
            last_typing: TypingThrottle::default(),
            // Shared with the command layer: a connection delta is an *edge*,
            // and a shell that starts after the socket is already up would
            // otherwise never learn the level.
            connected,
        };

        // Wait for the account before opening a socket: without a SyncContext
        // there is nothing to decide notifications against.
        let (me, thread_mode) = loop {
            let Some(message) = from_ui.recv().await else {
                return;
            };
            if let Some(started) = runner.handle_message(message) {
                break started;
            }
        };

        let mut context = SyncContext::new(me, thread_mode);
        context.active_channel = runner.pending_active.clone();
        context.window_focused = runner.pending_focus;
        runner.context = Some(context);

        let Some(AuthToken::Session(token)) = runner.rest.token() else {
            tracing::error!("no session token; the websocket cannot start");
            return;
        };
        let Ok(session) = WsSession::new(&runner.rest.base_url(), AuthToken::Session(token)) else {
            tracing::error!("could not derive the websocket url");
            return;
        };
        let (signals_tx, mut signals) = mpsc::channel(1024);
        let handle = session.spawn(signals_tx);

        loop {
            tokio::select! {
                message = from_ui.recv() => {
                    let Some(message) = message else { break };
                    // Intercepted rather than handled on the runner: this is the
                    // one message that needs the socket, and the socket lives
                    // here.
                    if let EngineMsg::Typing { channel_id, root_id } = &message {
                        if runner.typing_is_due(channel_id, root_id) {
                            handle.typing(channel_id, root_id);
                            tracing::debug!(channel = %channel_id, threaded = !root_id.is_empty(),
                                "typing sent");
                        }
                        continue;
                    }
                    runner.handle_message(message);
                }
                signal = signals.recv() => {
                    let Some(signal) = signal else { break };
                    match signal {
                        Signal::Connected { .. } => {
                            runner
                                .connected
                                .store(true, std::sync::atomic::Ordering::Relaxed);
                            runner.emit(UiDelta::Connection {
                                connected: true, resync: false });
                        }
                        Signal::Disconnected { .. } => {
                            runner
                                .connected
                                .store(false, std::sync::atomic::Ordering::Relaxed);
                            runner.emit(UiDelta::Connection {
                                connected: false, resync: false });
                        }
                        Signal::ResyncRequired => runner.emit(UiDelta::Connection {
                            connected: true, resync: true }),
                        Signal::Event { event, read_at, .. } => {
                            tracing::debug!(event = event.name(), "websocket event");
                            // Our own send, echoed back: the optimistic row has
                            // served its purpose and the real post takes over.
                            if let Event::Posted { post, .. } = &event
                                && !post.pending_post_id.is_empty()
                                && let Some(channel_id) =
                                    runner.pending.resolve(&post.pending_post_id)
                            {
                                tracing::debug!(%channel_id, "confirmed an optimistic send");
                            }
                            let Some(context) = runner.context.as_ref() else { continue };
                            // Blocking store work, but bounded: one post.
                            match runner.engine.apply_event(&event, context) {
                                Ok(deltas) => runner.forward(deltas, Some(read_at)),
                                Err(error) => tracing::warn!(%error, "applying an event failed"),
                            }
                            if let Event::Posted { channel_id, .. } = &event {
                                tracing::debug!(%channel_id, "live post applied");
                            }
                        }
                    }
                }
            }
        }
        handle.shutdown().await;
    });
}

#[cfg(test)]
mod tests {

    #[test]
    fn typing_goes_out_once_per_interval_per_conversation() {
        let mut throttle = TypingThrottle::default();
        let start = std::time::Instant::now();

        assert!(
            throttle.is_due("c1", "", start),
            "the first keystroke sends"
        );
        assert!(
            !throttle.is_due("c1", "", start + std::time::Duration::from_millis(4_999)),
            "and nothing else does until the interval is up"
        );
        assert!(throttle.is_due("c1", "", start + std::time::Duration::from_millis(5_001)));

        // A thread is its own conversation: typing a reply must not be silenced
        // by having just typed in the channel behind it.
        assert!(throttle.is_due("c1", "root9", start));
        assert!(!throttle.is_due("c1", "root9", start));
        assert!(throttle.is_due("c2", "", start), "nor by another channel");
    }
    use super::*;

    fn channel(channel_type: &str, display_name: &str, name: &str) -> matterless_core::Channel {
        matterless_core::Channel {
            id: "c1".into(),
            team_id: "t1".into(),
            channel_type: channel_type.into(),
            name: name.into(),
            display_name: display_name.into(),
            total_msg_count: 0,
            total_msg_count_root: 0,
            last_post_at: 0,
            delete_at: 0,
        }
    }

    /// The reported bug: a DM's `name` is `<id>__<id>`, and using it as the
    /// toast title showed a wall of hex where the conversation should be.
    #[test]
    fn a_direct_message_is_named_by_its_kind_not_by_its_ids() {
        let dm = channel(
            "D",
            "",
            "7gwdwjg1zjf7tb5xxdp6ieazgr__ufwu3fbgutnu5ep8odumg4tzzr",
        );
        let (label, direct) = conversation_label(Some(dm));
        assert_eq!(label, "Direct Message");
        assert!(direct, "the shell marks the author with @ for these");
    }

    #[test]
    fn a_group_message_says_so_too() {
        let (label, direct) = conversation_label(Some(channel("G", "", "abc123def456")));
        assert_eq!(label, "Group Message");
        assert!(direct);
    }

    /// A normal channel has a name worth showing, and the author is not the
    /// conversation, so nothing is marked.
    #[test]
    fn a_channel_is_named_by_its_display_name() {
        let (label, direct) =
            conversation_label(Some(channel("O", "Builds | Alerts", "builds--alerts")));
        assert_eq!(label, "Builds | Alerts");
        assert!(!direct);
    }

    #[test]
    fn a_channel_without_a_display_name_falls_back_to_its_slug() {
        let (label, _) = conversation_label(Some(channel("P", "", "secret-project")));
        assert_eq!(label, "secret-project");
    }

    /// A post can arrive for a channel the store has never seen. A toast with a
    /// blank title still says something happened; failing would say nothing.
    #[test]
    fn an_unknown_channel_yields_an_empty_label_rather_than_failing() {
        let (label, direct) = conversation_label(None);
        assert!(label.is_empty());
        assert!(!direct);
    }
}
