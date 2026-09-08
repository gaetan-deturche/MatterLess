//! The sync engine: merges the REST backfill and the live websocket into the
//! store, then emits deltas describing only what actually changed.
//!
//! Two properties the plan requires and the types enforce:
//!
//! * **Every delta says how it arrived.** `Arrival::Live` vs `Backfill` vs
//!   `Resync` is what stops a ten-minute catch-up raising two hundred toasts.
//! * **Ephemeral traffic never reaches the post store.** Typing measured 72-93%
//!   of websocket events across three runs; `Delta::Typing` is returned for the
//!   UI to route into its own map and is never written to SQLite.

pub mod notify;

use matterless_core::model::{Post, PostList, ThreadMode, Timestamp, User};
use matterless_core::ws::Event;
use matterless_store::{PostChange, Store, StoreError, SyncState, Unread};
use notify::{MentionVerdict, NotifyContext, NotifyDecision};
use std::collections::HashMap;
use std::sync::Arc;

#[derive(Debug, thiserror::Error)]
pub enum SyncError {
    #[error(transparent)]
    Store(#[from] StoreError),
    #[error(transparent)]
    Core(#[from] matterless_core::Error),
}

pub type Result<T> = std::result::Result<T, SyncError>;

/// How a post reached us. Notifications only ever fire on `Live`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Arrival {
    /// A websocket event while connected.
    Live,
    /// Paging history the user scrolled to.
    Backfill,
    /// A catch-up sweep after a reconnect. Legitimately new to us, but the user
    /// was not present for it, so it must not interrupt.
    Resync,
}

impl Arrival {
    pub fn may_notify(self) -> bool {
        self == Arrival::Live
    }
}

#[derive(Debug, Clone)]
pub enum Delta {
    /// A post was written. `in_stream` is false for thread context that must be
    /// stored but never placed in the channel view.
    PostUpserted {
        post_id: String,
        channel_id: String,
        root_id: String,
        arrival: Arrival,
        change: PostChange,
        in_stream: bool,
        mention: MentionVerdict,
        notify: bool,
    },
    PostTombstoned {
        post_id: String,
        channel_id: String,
    },
    UnreadChanged {
        channel_id: String,
        unread: Unread,
    },
    ReactionsChanged {
        post_id: String,
    },
    /// Ephemeral. Route to a separate store scope; never persist.
    Typing {
        channel_id: String,
        user_id: String,
        /// The thread being typed in, empty for the channel itself: a reply
        /// being typed belongs to its thread, not to the stream behind it.
        root_id: String,
    },
    /// Ephemeral.
    StatusChanged {
        user_id: String,
        status: String,
    },
    /// A preference changed at runtime. When `affects_thread_mode` is set the
    /// caller must re-resolve `ThreadMode`: collapsed threads change the data
    /// model, not just a display option.
    PreferencesChanged {
        affects_thread_mode: bool,
    },
    SidebarChanged {
        team_id: String,
    },
    CustomEmojiChanged,
    /// A cached profile went stale (name or avatar).
    UserChanged {
        user_id: String,
    },
    /// The socket came back without a resume, so history has a hole.
    ResyncRequired,
    /// A followed thread changed: new replies, read elsewhere, or (un)followed.
    ///
    /// Carries the channel because the footer row that shows it lives in that
    /// channel's plan -- the UI refetches the plan, as it does for every other
    /// change, rather than being handed a row.
    ThreadChanged {
        root_id: String,
        channel_id: String,
    },
}

/// Runtime state the decision functions need. Kept small and cheap to clone.
pub struct SyncContext {
    pub me: User,
    pub thread_mode: ThreadMode,
    pub active_channel: Option<String>,
    pub window_focused: bool,
    pub status: String,
    /// Channel id -> the member notify_props for this user.
    pub channel_notify_props: HashMap<String, HashMap<String, String>>,
    /// Thread roots the user follows, for the collapsed-threads rule.
    pub followed_threads: std::collections::HashSet<String>,
}

impl SyncContext {
    pub fn new(me: User, thread_mode: ThreadMode) -> Self {
        Self {
            me,
            thread_mode,
            active_channel: None,
            window_focused: true,
            status: "online".to_string(),
            channel_notify_props: HashMap::new(),
            followed_threads: std::collections::HashSet::new(),
        }
    }

    fn notify_context<'a>(
        &'a self,
        channel_id: &str,
        root_id: &str,
        empty: &'a HashMap<String, String>,
    ) -> NotifyContext<'a> {
        NotifyContext {
            me: &self.me,
            channel_notify_props: self.channel_notify_props.get(channel_id).unwrap_or(empty),
            status: &self.status,
            active_channel: self.active_channel.as_deref(),
            window_focused: self.window_focused,
            thread_mode: self.thread_mode,
            following_thread: !root_id.is_empty() && self.followed_threads.contains(root_id),
        }
    }
}

/// Where a reconnect's catch-up should start, and what can wait.
///
/// Phase 0 proved resume is never honoured here, so every reconnect is a full
/// resync. At ~53 ms a channel fetch and a limiter of 8/s, sweeping all 114
/// active channels uniformly is ~14 s of requests -- so it is ordered, split,
/// and the tail is fetched lazily on first open.
#[derive(Debug, Clone, Default)]
pub struct ResyncPlan {
    /// Fetch before the UI is considered current. Kept tiny on purpose.
    pub immediate: Vec<String>,
    /// Background trickle, useful-first.
    pub trickle: Vec<String>,
    /// Left until the channel is actually opened.
    pub lazy: Vec<String>,
}

const IMMEDIATE_BUDGET: usize = 3;
const TRICKLE_BUDGET: usize = 20;

pub struct SyncEngine {
    store: Arc<Store>,
    empty_props: HashMap<String, String>,
}

impl SyncEngine {
    pub fn new(store: Arc<Store>) -> Self {
        Self {
            store,
            empty_props: HashMap::new(),
        }
    }

    pub fn store(&self) -> &Arc<Store> {
        &self.store
    }

    /// Merges a REST page. `in_stream` distinguishes the ordered posts from the
    /// thread context the same response carries -- 31% of it, when measured.
    pub fn apply_post_list(
        &self,
        channel_id: &str,
        list: &PostList,
        arrival: Arrival,
        context: &SyncContext,
        now: Timestamp,
    ) -> Result<Vec<Delta>> {
        let mut posts: Vec<Post> = Vec::with_capacity(list.posts.len());
        posts.extend(list.posts.values().cloned());
        // Oldest first, so a root is written before its reply.
        posts.sort_by(|left, right| {
            left.create_at
                .cmp(&right.create_at)
                .then_with(|| left.id.cmp(&right.id))
        });

        let outcomes = self.store.upsert_posts(&posts)?;
        let mut deltas = Vec::new();

        for (post, outcome) in posts.iter().zip(outcomes.iter()) {
            if outcome.change == PostChange::Unchanged {
                continue;
            }
            let in_stream = list.order.iter().any(|id| id == &post.id);
            deltas.push(self.post_delta(post, outcome.change, arrival, in_stream, context));
        }

        self.record_sync_state(channel_id, list, now)?;
        if let Some(unread) = self.store.unread(channel_id, &context.me.id)? {
            deltas.push(Delta::UnreadChanged {
                channel_id: channel_id.to_string(),
                unread,
            });
        }
        Ok(deltas)
    }

    /// Applies one websocket event. Ephemeral events return a delta but touch
    /// no table.
    pub fn apply_event(&self, event: &Event, context: &SyncContext) -> Result<Vec<Delta>> {
        match event {
            Event::Posted { post, channel_id } => {
                let outcomes = self
                    .store
                    .upsert_posts(std::slice::from_ref(post.as_ref()))?;
                let change = outcomes
                    .first()
                    .map(|outcome| outcome.change)
                    .unwrap_or(PostChange::Unchanged);
                if change == PostChange::Unchanged {
                    // Already held: a replay after a reconnect, not a new post.
                    return Ok(Vec::new());
                }
                let delta = self.post_delta(post, change, Arrival::Live, true, context);
                if change == PostChange::Inserted {
                    // Keeps the counters unread and the badge are derived from
                    // in step with the server between REST refreshes. The
                    // mention verdict is the one the notification policy just
                    // reached, so a badge can never disagree with a toast.
                    let mentions_me = matches!(
                        &delta,
                        Delta::PostUpserted { mention, .. } if mention.is_mention()
                    );
                    self.store.record_arrival(
                        channel_id,
                        !post.is_reply(),
                        post.user_id == context.me.id,
                        mentions_me,
                    )?;
                }
                let mut deltas = vec![delta];
                if let Some(unread) = self.store.unread(channel_id, &context.me.id)? {
                    deltas.push(Delta::UnreadChanged {
                        channel_id: channel_id.clone(),
                        unread,
                    });
                }
                Ok(deltas)
            }
            Event::PostEdited(post) => {
                let outcomes = self
                    .store
                    .upsert_posts(std::slice::from_ref(post.as_ref()))?;
                let change = outcomes
                    .first()
                    .map(|outcome| outcome.change)
                    .unwrap_or(PostChange::Unchanged);
                if change == PostChange::Unchanged {
                    return Ok(Vec::new());
                }
                // An edit never notifies, whatever it now says.
                Ok(vec![Delta::PostUpserted {
                    post_id: post.id.clone(),
                    channel_id: post.channel_id.clone(),
                    root_id: post.root_id.clone(),
                    arrival: Arrival::Live,
                    change,
                    in_stream: true,
                    mention: MentionVerdict::None,
                    notify: false,
                }])
            }
            Event::PostDeleted(post) => {
                self.store
                    .upsert_posts(std::slice::from_ref(post.as_ref()))?;
                Ok(vec![Delta::PostTombstoned {
                    post_id: post.id.clone(),
                    channel_id: post.channel_id.clone(),
                }])
            }
            // The store has to be patched, not just announced: reactions live
            // in the post's metadata, so a rebuild that skipped this read the
            // old set and showed nothing until the next REST fetch.
            Event::ReactionAdded(reaction) => {
                let changed = self.store.add_reaction(
                    &reaction.post_id,
                    &reaction.user_id,
                    &reaction.emoji_name,
                    reaction.create_at,
                )?;
                if !changed {
                    // Our own optimistic reaction, echoed back.
                    return Ok(Vec::new());
                }
                Ok(vec![Delta::ReactionsChanged {
                    post_id: reaction.post_id.clone(),
                }])
            }
            Event::ReactionRemoved(reaction) => {
                let changed = self.store.remove_reaction(
                    &reaction.post_id,
                    &reaction.user_id,
                    &reaction.emoji_name,
                )?;
                if !changed {
                    return Ok(Vec::new());
                }
                Ok(vec![Delta::ReactionsChanged {
                    post_id: reaction.post_id.clone(),
                }])
            }
            Event::Typing {
                channel_id,
                user_id,
                root_id,
            } => Ok(vec![Delta::Typing {
                channel_id: channel_id.clone(),
                user_id: user_id.clone(),
                root_id: root_id.clone(),
            }]),
            Event::StatusChange { user_id, status } => Ok(vec![Delta::StatusChanged {
                user_id: user_id.clone(),
                status: status.clone(),
            }]),
            // Read state, possibly set by another device.
            Event::ChannelsViewed { channel_times } => {
                let mut deltas = Vec::new();
                for (channel_id, viewed_at) in channel_times {
                    self.store
                        .mark_channel_viewed(channel_id, &context.me.id, *viewed_at)?;
                    if let Some(unread) = self.store.unread(channel_id, &context.me.id)? {
                        deltas.push(Delta::UnreadChanged {
                            channel_id: channel_id.clone(),
                            unread,
                        });
                    }
                }
                Ok(deltas)
            }
            Event::ChannelViewed { channel_id } => {
                // No timestamp on this one; the counters are what matter.
                self.store
                    .mark_channel_viewed(channel_id, &context.me.id, 0)?;
                let mut deltas = Vec::new();
                if let Some(unread) = self.store.unread(channel_id, &context.me.id)? {
                    deltas.push(Delta::UnreadChanged {
                        channel_id: channel_id.clone(),
                        unread,
                    });
                }
                Ok(deltas)
            }
            Event::PreferencesChanged { preferences } => {
                self.store.upsert_preferences(preferences)?;
                let affects_thread_mode = preferences.iter().any(|preference| {
                    preference.category == "display_settings"
                        && preference.name == "collapsed_reply_threads"
                });
                Ok(vec![Delta::PreferencesChanged {
                    affects_thread_mode,
                }])
            }
            Event::SidebarCategoriesUpdated { team_id } => Ok(vec![Delta::SidebarChanged {
                team_id: team_id.clone(),
            }]),
            Event::EmojiAdded { .. } => Ok(vec![Delta::CustomEmojiChanged]),
            Event::UserUpdated { user_id } => Ok(vec![Delta::UserChanged {
                user_id: user_id.clone(),
            }]),
            // Per-thread read state, which channel unread cannot express: with
            // collapsed threads on, a reply never bumps a channel's counters,
            // so these counts are the only thing that can explain an unread
            // thread. They are the server's numbers, applied verbatim.
            Event::ThreadUpdated { thread_id, thread } => {
                let channel_id = match thread {
                    Some(thread) => {
                        self.store
                            .upsert_threads(std::slice::from_ref(thread.as_ref()))?;
                        thread.post.channel_id.clone()
                    }
                    // No thread attached: nothing to store, but the footer for
                    // that root still has to be re-read.
                    None => self.store.thread_channel(thread_id)?.unwrap_or_default(),
                };
                Ok(vec![Delta::ThreadChanged {
                    root_id: thread_id.clone(),
                    channel_id,
                }])
            }
            Event::ThreadReadChanged {
                thread_id,
                channel_id,
                timestamp,
                unread_replies,
                unread_mentions,
            } => {
                if thread_id.is_empty() {
                    // Every thread in the team was marked read.
                    let cleared = self.store.mark_all_threads_read(*timestamp)?;
                    tracing::debug!(cleared, "all threads marked read");
                    return Ok(Vec::new());
                }
                self.store.set_thread_read(
                    thread_id,
                    *timestamp,
                    *unread_replies,
                    *unread_mentions,
                )?;
                let channel_id = if channel_id.is_empty() {
                    self.store.thread_channel(thread_id)?.unwrap_or_default()
                } else {
                    channel_id.clone()
                };
                Ok(vec![Delta::ThreadChanged {
                    root_id: thread_id.clone(),
                    channel_id,
                }])
            }
            Event::ThreadFollowChanged {
                thread_id,
                following,
                ..
            } => {
                self.store.set_thread_following(thread_id, *following)?;
                Ok(vec![Delta::ThreadChanged {
                    root_id: thread_id.clone(),
                    channel_id: self.store.thread_channel(thread_id)?.unwrap_or_default(),
                }])
            }
            Event::Hello(_) => Ok(Vec::new()),
            Event::Other { .. } => Ok(Vec::new()),
        }
    }

    fn post_delta(
        &self,
        post: &Post,
        change: PostChange,
        arrival: Arrival,
        in_stream: bool,
        context: &SyncContext,
    ) -> Delta {
        let notify_context =
            context.notify_context(&post.channel_id, &post.root_id, &self.empty_props);
        let decision: NotifyDecision = notify::decide(post, &notify_context);
        Delta::PostUpserted {
            post_id: post.id.clone(),
            channel_id: post.channel_id.clone(),
            root_id: post.root_id.clone(),
            arrival,
            change,
            in_stream,
            mention: decision.mention,
            // A backfill or resync is never allowed to interrupt, however
            // strongly the post mentions you.
            notify: decision.notify && arrival.may_notify() && change == PostChange::Inserted,
        }
    }

    fn record_sync_state(&self, channel_id: &str, list: &PostList, now: Timestamp) -> Result<()> {
        let Some(newest) = list.newest_in_order() else {
            return Ok(());
        };
        let oldest = list.oldest_in_order().unwrap_or(newest);
        let held = self.store.sync_state(channel_id)?.unwrap_or(SyncState {
            channel_id: channel_id.to_string(),
            ..SyncState::default()
        });

        let state = SyncState {
            channel_id: channel_id.to_string(),
            synced_from: if held.synced_from == 0 {
                oldest.create_at
            } else {
                held.synced_from.min(oldest.create_at)
            },
            synced_to: held.synced_to.max(newest.create_at),
            oldest_post_id: if held.synced_from == 0 || oldest.create_at < held.synced_from {
                oldest.id.clone()
            } else {
                held.oldest_post_id
            },
            newest_post_id: if newest.create_at >= held.synced_to {
                newest.id.clone()
            } else {
                held.newest_post_id
            },
            // `prev_post_id` empty means the server had nothing older to give.
            reached_beginning: held.reached_beginning || list.prev_post_id.is_empty(),
            last_reconciled_at: now,
        };
        self.store.set_sync_state(&state)?;
        Ok(())
    }

    /// Orders and splits the post-reconnect catch-up.
    pub fn plan_resync(&self, context: &SyncContext) -> Result<ResyncPlan> {
        let ordered = self
            .store
            .resync_order(context.active_channel.as_deref(), &context.me.id)?;
        let mut plan = ResyncPlan::default();
        for (index, channel_id) in ordered.into_iter().enumerate() {
            if index < IMMEDIATE_BUDGET {
                plan.immediate.push(channel_id);
            } else if index < IMMEDIATE_BUDGET + TRICKLE_BUDGET {
                plan.trickle.push(channel_id);
            } else {
                plan.lazy.push(channel_id);
            }
        }
        Ok(plan)
    }

    /// The `?since=` cursor for a channel: where our contiguous history ends.
    pub fn catch_up_cursor(&self, channel_id: &str) -> Result<Timestamp> {
        Ok(self
            .store
            .sync_state(channel_id)?
            .map(|state| state.synced_to)
            .unwrap_or(0))
    }
}

#[cfg(test)]
mod tests;
