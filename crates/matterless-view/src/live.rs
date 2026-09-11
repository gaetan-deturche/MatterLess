//! The window's connection to the server.
//!
//! Until now this window was a snapshot: it opened the database, drew what was
//! in it, and never learned anything again. This is the half that makes it a
//! chat client -- the socket, the sync engine, and a way to wake a winit event
//! loop from the thread they run on.
//!
//! Nothing here decides what a change *means*. `matterless-sync` already does
//! that, and it is the same engine the app runs, so a message arriving here and
//! a message arriving there are the same message with the same unread count and
//! the same notification verdict. This only carries the answer across a thread
//! boundary.

use matterless_core::RestClient;
use matterless_core::auth::AuthToken;
use matterless_core::model::PostList;
use matterless_core::ws::{Signal, WsSession};
use matterless_core::{ThreadMode, User};
use matterless_store::Store;
use matterless_sync::{Arrival, Delta, SyncContext, SyncEngine};
use std::sync::Arc;

/// What the socket thread tells the window.
#[derive(Debug)]
pub enum Update {
    /// The socket came up or went down. A level, not an edge: a window that
    /// starts after the socket is already up would otherwise never learn it.
    Connected(bool),
    /// What changed, already applied to the store.
    Changed(Vec<Delta>),
    /// The connection could not be made at all, with the reason.
    Failed(String),
    /// Who the token belongs to. The window needs this to count unreads and to
    /// tell which half of a direct message is the reader, and asking the server
    /// is the only way to know it without being told.
    SignedIn { id: String, username: String },
    /// A send came back. `failed` keeps the row on screen, marked, rather than
    /// losing what was written.
    SendSettled {
        pending_post_id: String,
        channel_id: String,
        failed: bool,
    },
    /// A notification was clicked, naming the conversation to open. Raised
    /// from whatever thread the platform fires its callback on, and delivered
    /// like everything else on the one that owns the window.
    Activated(String),
    /// Older history arrived and is in the store. `more` is false once the
    /// beginning of the channel has been reached, so the window stops asking.
    Older { channel_id: String, more: bool },
    /// Who is around, by user id.
    Statuses(Vec<(String, String)>),
    /// What else a name could mean, for the query that was asked.
    Discovered {
        query: String,
        found: Vec<crate::switcher::Match>,
    },
    /// A conversation is ready to be opened, having been joined or created.
    Reached { channel_id: String },
    /// The sidebar's channels and counts have been refreshed in the store,
    /// and here is how this reader reads threads.
    Membership(matterless_core::model::ThreadMode),
    /// A titled list of messages, for the panel that asked for it.
    Listed {
        title: String,
        found: Vec<crate::listing::Found>,
    },
    /// A picture arrived, decoded to straight RGBA and ready for the atlas.
    ///
    /// Decoded on the socket thread rather than the drawing one: a JPEG is
    /// milliseconds of work, and doing it between frames is how a scroll
    /// stutters when somebody with a photograph comes into view.
    Picture {
        key: String,
        width: u32,
        height: u32,
        rgba: Vec<u8>,
    },
}

/// What the window asks the socket thread to do.
#[derive(Debug)]
pub enum Ask {
    /// Post a message. The pending id is the window's own, so the row it
    /// already drew and the answer that comes back name the same thing.
    Send {
        pending_post_id: String,
        channel_id: String,
        root_id: String,
        message: String,
    },
    /// Save, pin or delete one message.
    Act {
        action: crate::actions::Action,
        post_id: String,
        on: bool,
    },
    /// Add or remove a reaction.
    React {
        post_id: String,
        emoji: String,
        on: bool,
    },
    /// Keep a file somebody attached, next to the reader's other downloads.
    Download { file_id: String, name: String },
    /// Send a file that was dropped on the window.
    ///
    /// The path rather than the bytes: reading a hundred and fifty megabytes
    /// on the thread that draws would stall the window for as long as the disk
    /// took, and the socket thread is already the one that waits for things.
    Upload {
        channel_id: String,
        root_id: String,
        path: std::path::PathBuf,
    },
    /// What else this name could mean, beyond the conversations already held.
    ///
    /// Public channels the reader is not in, and people they may never have
    /// written to. Asked of the server because neither is in the local store
    /// by definition.
    Discover { query: String },
    /// Keep hearing about a thread, or stop.
    Follow { root_id: String, following: bool },
    /// Put somebody else in a channel.
    AddMember { channel_id: String, user_id: String },
    /// Stop a conversation counting unread, or start again.
    Mute { channel_id: String, muted: bool },
    /// Stop being in a channel.
    Leave { channel_id: String },
    /// Join a public channel, then open it.
    Join { channel_id: String },
    /// Find or create the conversation with one person, then open it.
    Direct { user_id: String },
    /// Every channel and membership, from the server into the store.
    ///
    /// The unread counts are arithmetic on two numbers the server keeps -- a
    /// channel's total and this reader's seen count -- and neither has
    /// anything to do with which posts are held locally. A cached sidebar can
    /// be a whole session old, so those numbers have to be reconciled or the
    /// badges describe a conversation nobody is having any more.
    Membership,
    /// The messages this reader has saved, across every conversation.
    Saved,
    /// The messages pinned in one channel, for everyone.
    Pinned { channel_id: String },
    /// Who is around, for the conversations on screen.
    ///
    /// Batched rather than one call per person: the sidebar asks about every
    /// direct conversation at once, and that is one request rather than forty.
    Statuses { user_ids: Vec<String> },
    /// Say that this reader is typing, so the other clients can show it.
    ///
    /// Fire and forget: nothing depends on it arriving, and a typing signal
    /// that misses is worth strictly less than the round trip to confirm it.
    Typing { channel_id: String, root_id: String },
    /// Bring a channel's recent history back in line with the server.
    ///
    /// The window reads a local store, so anything that happened while it was
    /// not connected -- a message deleted, a message edited -- it never hears
    /// about: the server simply stops mentioning a deleted post, and a stale
    /// local copy stays on screen forever. This is the visit that heals that,
    /// and it is what the app does on every channel open.
    Refresh { channel_id: String },
    /// Change what a message says.
    ///
    /// The socket echoes the edit back, which is what redraws the row, so
    /// nothing is written here beyond what the server agreed to.
    Edit { post_id: String, message: String },
    /// Resolve these emoji names against the server, once each.
    ///
    /// The answer is remembered either way: a name the server does not know as
    /// custom is a standard one too new for the generated table, or a scanner
    /// artefact like the `53` in `12:53:`, and neither is worth asking twice.
    NameEmoji { names: Vec<String> },
    /// Which conversation the reader is looking at.
    ///
    /// The notification rules ask: a message in the channel already on screen
    /// does not interrupt somebody who can see it.
    Looking { channel_id: String },
    /// Tell the server this channel has been seen, and record the watermark.
    ///
    /// Without it a channel stays unread however long it is looked at, and the
    /// sidebar is permanently wrong.
    MarkRead { channel_id: String },
    /// Fetch a page of older history and store it.
    LoadOlder { channel_id: String },
    /// Fetch a picture, decoded to fit the box the layout reserved for it.
    ///
    /// The size is asked for rather than settled later because a picture scaled
    /// on the way in is scaled once, by a proper filter, into exactly the space
    /// it will occupy -- where scaling it on the way out means the atlas holds
    /// pixels nothing will ever show.
    Fetch {
        key: String,
        width: u32,
        height: u32,
    },
}

/// The window's end of the socket thread. Dropping it closes the connection.
pub struct Link {
    asks: tokio::sync::mpsc::UnboundedSender<Ask>,
}

impl Link {
    /// Queues a message to send. Fails only once the socket thread has gone.
    pub fn send(&self, ask: Ask) -> bool {
        self.asks.send(ask).is_ok()
    }
}

/// Anything that can be woken from the socket thread.
///
/// A trait rather than winit's proxy directly, so the plumbing can be tested
/// and so this crate does not need a window to compile.
pub trait Wake: Send + 'static {
    fn wake(&self, update: Update);
}

/// Where the session lives once the app has signed in.
///
/// Read rather than asked for: the token is a thirty-day bearer credential the
/// app already holds, and this window has no business collecting a password to
/// mint a second one.
pub fn stored_token() -> Option<String> {
    let entry = keyring::Entry::new("matterless", "session").ok()?;
    match entry.get_password() {
        Ok(token) if !token.is_empty() => Some(token),
        // The same minted file the app imports from, for a machine that has
        // never signed in through the app itself.
        _ => minted_token(),
    }
}

fn minted_token() -> Option<String> {
    for candidate in [
        "Claude/.mm_token.json",
        "../../Claude/.mm_token.json",
        "tools/.mm_token.json",
        "../../tools/.mm_token.json",
    ] {
        let Ok(raw) = std::fs::read_to_string(candidate) else {
            continue;
        };
        let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&raw) else {
            continue;
        };
        if let Some(token) = parsed.get("token").and_then(|value| value.as_str())
            && !token.is_empty()
        {
            return Some(token.to_string());
        }
    }
    None
}

/// The server this install is pointed at, from the file beside the database.
pub fn stored_server(database: &std::path::Path) -> Option<String> {
    let directory = database.parent()?;
    let held = std::fs::read_to_string(directory.join("server.txt")).ok()?;
    let trimmed = held.trim().to_string();
    (!trimmed.is_empty()).then_some(trimmed)
}

/// Opens the socket on a thread of its own and reports what arrives.
///
/// Returns immediately. The window keeps drawing from the store while this
/// connects, which is the point: a cold start paints from SQLite and the socket
/// catches it up, rather than the reader waiting on a network round trip.
pub fn start(store: Arc<Store>, server: String, token: String, wake: impl Wake) -> Link {
    let (asks, inbox) = tokio::sync::mpsc::unbounded_channel();
    std::thread::spawn(move || {
        let runtime = match tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        {
            Ok(runtime) => runtime,
            Err(error) => {
                wake.wake(Update::Failed(format!("no runtime: {error}")));
                return;
            }
        };
        runtime.block_on(run(store, server, token, wake, inbox));
    });
    Link { asks }
}

async fn run(
    store: Arc<Store>,
    server: String,
    token: String,
    wake: impl Wake,
    mut inbox: tokio::sync::mpsc::UnboundedReceiver<Ask>,
) {
    let rest = match RestClient::new(&server) {
        Ok(rest) => rest,
        Err(error) => {
            wake.wake(Update::Failed(format!("{server}: {error}")));
            return;
        }
    };
    rest.set_token(AuthToken::Session(token.clone()));

    // Who the reader is, which the notification and unread decisions need and
    // which also proves the token is still good before a socket is opened.
    let me: User = match rest.me().await {
        Ok(me) => me,
        Err(error) => {
            wake.wake(Update::Failed(format!(
                "the session is not usable: {error}"
            )));
            return;
        }
    };
    let me_id = me.id.clone();
    println!("signed in as {}", me.username);
    wake.wake(Update::SignedIn {
        id: me.id.clone(),
        username: me.username.clone(),
    });

    let session = match WsSession::new(&rest.base_url(), AuthToken::Session(token)) {
        Ok(session) => session,
        Err(error) => {
            wake.wake(Update::Failed(format!("websocket url: {error}")));
            return;
        }
    };

    let engine = SyncEngine::new(store);
    let mut context = reader_context(me, engine.store());
    println!(
        "following {} threads, replies elsewhere stay quiet",
        context.followed_threads.len()
    );

    let (signals_tx, mut signals) = tokio::sync::mpsc::channel(1024);
    let handle = session.spawn(signals_tx);

    loop {
        tokio::select! {
            // Sends and socket traffic on one thread: a send has to be able to
            // go out while the socket is quiet, and its echo has to be able to
            // arrive while a send is in flight.
            ask = inbox.recv() => {
                let Some(ask) = ask else { break };
                match ask {
                    Ask::Send { pending_post_id, channel_id, root_id, message } => {
                        let request = matterless_core::model::NewPost {
                            channel_id: &channel_id,
                            message: &message,
                            root_id: &root_id,
                            pending_post_id: &pending_post_id,
                            file_ids: &[],
                        };
                        let failed = match rest.create_post(&request).await {
                            Ok(post) => {
                                // Stored here rather than waited for: the echo
                                // usually beats this reply, and whichever wins
                                // the other one is a no-op.
                                let event = matterless_core::Event::Posted {
                                    channel_id: post.channel_id.clone(),
                                    post: Box::new(post),
                                };
                                if let Err(error) = engine.apply_event(&event, &context) {
                                    eprintln!("storing a confirmed send: {error}");
                                }
                                false
                            }
                            Err(error) => {
                                eprintln!("send failed: {error}");
                                true
                            }
                        };
                        wake.wake(Update::SendSettled {
                            pending_post_id,
                            channel_id,
                            failed,
                        });
                    }
                    Ask::React { post_id, emoji, on } => {
                        let done = if on {
                            rest.add_reaction(&me_id, &post_id, &emoji).await.map(|_| ())
                        } else {
                            rest.remove_reaction(&me_id, &post_id, &emoji).await
                        };
                        match done {
                            // The server echoes the change on the socket, which
                            // is what actually redraws the pill. Nothing to do
                            // here but say it went.
                            Ok(()) => println!("reacted to {post_id}"),
                            Err(error) => eprintln!("reacting to {post_id}: {error}"),
                        }
                    }
                    Ask::Act {
                        action,
                        post_id,
                        on,
                    } => {
                        use crate::actions::Action;
                        let done = match action {
                            Action::Save => rest.set_post_saved(&me_id, &post_id, on).await,
                            Action::Pin => rest.set_post_pinned(&post_id, on).await,
                            Action::Delete => rest.delete_post(&post_id).await,
                            // Answered in the window: none of these needs the
                            // server. React opens a picker and Edit opens a
                            // box, and what either produces arrives later as
                            // its own ask.
                            Action::React
                            | Action::Edit
                            | Action::Forward
                            | Action::Thread
                            | Action::Link => Ok(()),
                        };
                        match done {
                            // The socket echoes the change, which is what
                            // redraws the row. Nothing to do here but say it
                            // went.
                            Ok(()) => println!("{} on {post_id}", action.slug()),
                            Err(error) => eprintln!("{} on {post_id}: {error}", action.slug()),
                        }
                    }
                    Ask::Download { file_id, name } => {
                        match rest.fetch_bytes(&format!("/files/{file_id}")).await {
                            Ok(Some((bytes, _))) => match keep(&downloads(), &name, &bytes) {
                                Ok(path) => println!("kept {}", path.display()),
                                Err(error) => eprintln!("keeping {name}: {error}"),
                            },
                            Ok(None) => eprintln!("{name}: the server sent nothing"),
                            Err(error) => eprintln!("fetching {name}: {error}"),
                        }
                    }
                    Ask::Upload {
                        channel_id,
                        root_id,
                        path,
                    } => {
                        upload(&rest, &engine, &context, &channel_id, &root_id, &path).await;
                    }
                    Ask::Discover { query } => {
                        let found = discover(&rest, engine.store(), &me_id, &query).await;
                        println!("{} other ways to read \"{query}\"", found.len());
                        wake.wake(Update::Discovered { query, found });
                    }
                    Ask::Follow { root_id, following } => {
                        // The team comes from the thread's own channel: a
                        // thread belongs to one, and the route is scoped by it.
                        let team = engine
                            .store()
                            .post(&root_id)
                            .ok()
                            .flatten()
                            .and_then(|post| engine.store().channel(&post.channel_id).ok().flatten())
                            .map(|channel| channel.team_id)
                            .unwrap_or_default();
                        match rest.follow_thread(&me_id, &team, &root_id, following).await {
                            Ok(()) => {
                                println!(
                                    "{} {root_id}",
                                    if following { "following" } else { "unfollowed" }
                                );
                                // Which threads may interrupt has just changed,
                                // and the rule reads a set held on this thread.
                                context.followed_threads = followed(engine.store());
                            }
                            Err(error) => eprintln!("following {root_id}: {error}"),
                        }
                    }
                    Ask::AddMember {
                        channel_id,
                        user_id,
                    } => {
                        // The same call that joins: adding somebody is putting
                        // a member on a channel, and which member is the only
                        // difference between doing it to yourself and to
                        // somebody else.
                        match rest.join_channel(&channel_id, &user_id).await {
                            Ok(_) => println!("added {user_id} to {channel_id}"),
                            Err(error) => eprintln!("adding {user_id}: {error}"),
                        }
                    }
                    Ask::Mute { channel_id, muted } => {
                        match rest.set_channel_muted(&channel_id, &me_id, muted).await {
                            Ok(()) => {
                                println!(
                                    "{channel_id} is now {}",
                                    if muted { "muted" } else { "unmuted" }
                                );
                                // Muting is a membership setting, and the
                                // sidebar reads it from the same rows the
                                // counts come from.
                                if let Ok((_, mode)) =
                                    membership(&rest, engine.store(), &me_id).await
                                {
                                    wake.wake(Update::Membership(mode));
                                }
                            }
                            Err(error) => eprintln!("muting {channel_id}: {error}"),
                        }
                    }
                    Ask::Leave { channel_id } => {
                        match rest.leave_channel(&channel_id, &me_id).await {
                            Ok(()) => {
                                println!("left {channel_id}");
                                // The sidebar has a row fewer in it, and only
                                // a fresh membership pull will show that.
                                if let Ok((_, mode)) =
                                    membership(&rest, engine.store(), &me_id).await
                                {
                                    wake.wake(Update::Membership(mode));
                                }
                            }
                            Err(error) => eprintln!("leaving {channel_id}: {error}"),
                        }
                    }
                    Ask::Join { channel_id } => {
                        match rest.join_channel(&channel_id, &me_id).await {
                            Ok(_) => {
                                println!("joined {channel_id}");
                                // The sidebar has a new row in it, and only a
                                // fresh membership pull will show it.
                                if let Ok((counted, mode)) =
                                    membership(&rest, engine.store(), &me_id).await
                                {
                                    println!("{counted} channels after joining");
                                    wake.wake(Update::Membership(mode));
                                }
                                wake.wake(Update::Reached { channel_id });
                            }
                            Err(error) => eprintln!("joining {channel_id}: {error}"),
                        }
                    }
                    Ask::Direct { user_id } => match rest.direct_channel(&me_id, &user_id).await {
                        Ok(channel) => {
                            // Stored before it is opened: the window reads the
                            // conversation out of the store, and a channel it
                            // has never heard of reads as empty.
                            if let Err(error) = engine.store().upsert_channels(std::slice::from_ref(&channel)) {
                                eprintln!("storing a new conversation: {error}");
                            }
                            if let Ok((_, mode)) = membership(&rest, engine.store(), &me_id).await {
                                wake.wake(Update::Membership(mode));
                            }
                            wake.wake(Update::Reached {
                                channel_id: channel.id,
                            });
                        }
                        Err(error) => eprintln!("opening a conversation: {error}"),
                    },
                    Ask::Membership => match membership(&rest, engine.store(), &me_id).await {
                        Ok((counted, mode)) => {
                            println!("{counted} channels refreshed, threads are {mode:?}");
                            context.thread_mode = mode;
                            wake.wake(Update::Membership(mode));
                        }
                        Err(error) => eprintln!("refreshing the sidebar: {error}"),
                    },
                    Ask::Saved => {
                        // From the server rather than the store: a message
                        // saved from another client was never seen here, and a
                        // list that only knows about this window's own saves
                        // would be quietly wrong.
                        match rest.flagged_posts(&me_id, LISTED).await {
                            Ok(list) => wake.wake(listed("Saved", engine.store(), list, &me_id)),
                            Err(error) => eprintln!("listing saved messages: {error}"),
                        }
                    }
                    Ask::Pinned { channel_id } => match rest.pinned_posts(&channel_id).await {
                        Ok(list) => wake.wake(listed("Pinned", engine.store(), list, &me_id)),
                        Err(error) => eprintln!("listing pinned messages: {error}"),
                    },
                    Ask::Statuses { user_ids } => match rest.statuses_by_ids(&user_ids).await {
                        Ok(found) => {
                            println!("asked about {} people, {} answered", user_ids.len(), found.len());
                            wake.wake(Update::Statuses(
                                found
                                    .into_iter()
                                    .map(|status| (status.user_id, status.status))
                                    .collect(),
                            ));
                        }
                        Err(error) => eprintln!("asking who is around: {error}"),
                    },
                    Ask::Typing {
                        channel_id,
                        root_id,
                    } => handle.typing(&channel_id, &root_id),
                    Ask::Refresh { channel_id } => {
                        match rest.posts(&channel_id, RECENT).await {
                            Ok(list) => {
                                // A backfill, so nothing in it can notify:
                                // these are messages the reader has had for a
                                // while, not messages arriving.
                                let quiet =
                                    SyncContext::new(context.me.clone(), ThreadMode::Collapsed);
                                let mut deltas = match engine.apply_post_list(
                                    &channel_id,
                                    &list,
                                    Arrival::Backfill,
                                    &quiet,
                                    0,
                                ) {
                                    Ok(deltas) => deltas,
                                    Err(error) => {
                                        eprintln!("refreshing {channel_id}: {error}");
                                        continue;
                                    }
                                };
                                deltas.extend(returned(engine.store(), &list));
                                deltas.extend(vanished(engine.store(), &channel_id, &list));
                                if !deltas.is_empty() {
                                    println!(
                                        "{channel_id}: {} changes from the server",
                                        deltas.len()
                                    );
                                    wake.wake(Update::Changed(deltas));
                                }
                            }
                            Err(error) => eprintln!("refreshing {channel_id}: {error}"),
                        }
                    }
                    Ask::Edit { post_id, message } => {
                        // The length, never the text.
                        match rest.patch_post(&post_id, &message).await {
                            Ok(_) => println!(
                                "edited {post_id} to {} characters",
                                message.chars().count()
                            ),
                            Err(error) => eprintln!("editing {post_id}: {error}"),
                        }
                    }
                    Ask::NameEmoji { names } => {
                        let mut learned = 0usize;
                        for name in &names {
                            let id = rest
                                .emoji_by_name(name)
                                .await
                                .ok()
                                .flatten()
                                .map(|emoji| emoji.id)
                                .unwrap_or_default();
                            if !id.is_empty() {
                                learned += 1;
                            }
                            if let Err(error) = engine.store().remember_emoji(name, &id) {
                                eprintln!("remembering :{name}: {error}");
                            }
                        }
                        if learned > 0 {
                            println!("{learned} of {} names are custom emoji", names.len());
                            // The same delta the socket raises when somebody
                            // adds one, so the window has one way to hear that
                            // the table changed rather than two.
                            wake.wake(Update::Changed(vec![Delta::CustomEmojiChanged]));
                        }
                    }
                    Ask::Looking { channel_id } => {
                        context.active_channel = Some(channel_id);
                        // A window with focus it cannot measure is better
                        // assumed focused: the alternative is interrupting
                        // somebody about the channel they are reading.
                        context.window_focused = true;
                    }
                    Ask::MarkRead { channel_id } => {
                        if let Err(error) = rest.view_channel(&me_id, &channel_id).await {
                            // Not worth surfacing: the next look at the channel
                            // tries again.
                            eprintln!("marking {channel_id} read: {error}");
                            continue;
                        }
                        // The server's own clock, taken from the newest post
                        // rather than read locally: this is compared against
                        // post times later, and mixing clocks would eventually
                        // hide the unread divider for good on a machine whose
                        // clock runs fast.
                        let watermark = engine.store().newest_post_at(&channel_id).unwrap_or(0);
                        if let Err(error) =
                            engine
                                .store()
                                .mark_channel_viewed(&channel_id, &me_id, watermark)
                        {
                            eprintln!("recording that {channel_id} was read: {error}");
                        }
                    }
                    Ask::LoadOlder { channel_id } => {
                        let held = engine.store().sync_state(&channel_id).ok().flatten();
                        let Some(held) = held else { continue };
                        if held.reached_beginning || held.oldest_post_id.is_empty() {
                            wake.wake(Update::Older {
                                channel_id,
                                more: false,
                            });
                            continue;
                        }
                        match rest
                            .posts_before(&channel_id, &held.oldest_post_id, 200)
                            .await
                        {
                            Ok(list) => {
                                let more = !list.prev_post_id.is_empty();
                                let fetched = list.order.len();
                                if let Err(error) = engine.apply_post_list(
                                    &channel_id,
                                    &list,
                                    matterless_sync::Arrival::Backfill,
                                    &context,
                                    0,
                                ) {
                                    eprintln!("storing older history: {error}");
                                }
                                println!("{channel_id}: {fetched} older messages, more: {more}");
                                wake.wake(Update::Older { channel_id, more });
                            }
                            Err(error) => eprintln!("older history for {channel_id}: {error}"),
                        }
                    }
                    Ask::Fetch { key, width, height } => {
                        let Some(route) = route_for(&key) else {
                            continue;
                        };
                        match rest.fetch_bytes(&route).await {
                            Ok(Some((bytes, _))) => match decode(&bytes, width, height) {
                                Some((width, height, rgba)) => wake.wake(Update::Picture {
                                    key,
                                    width,
                                    height,
                                    rgba,
                                }),
                                // Named by what actually came back: a picture
                                // this build has no decoder for and a picture
                                // that is really an error page fail the same
                                // way, and "could not be decoded" said neither.
                                None => eprintln!(
                                    "{key}: could not be decoded, {} bytes of {}",
                                    bytes.len(),
                                    kind_of(&bytes)
                                ),
                            },
                            // A person with no picture is not an error, and a
                            // silent gap is the right drawing for one.
                            Ok(None) => {}
                            Err(error) => eprintln!("{key}: {error}"),
                        }
                    }
                }
            }
            signal = signals.recv() => {
                let Some(signal) = signal else { break };
                match signal {
                    Signal::Connected { .. } => wake.wake(Update::Connected(true)),
                    Signal::Disconnected { .. } => wake.wake(Update::Connected(false)),
                    // The socket came back without resuming, so history has a
                    // hole. The window redraws from the store, which is the
                    // honest answer until it can refetch the gap itself.
                    Signal::ResyncRequired => {
                        wake.wake(Update::Changed(vec![Delta::ResyncRequired]))
                    }
                    Signal::Event { event, .. } => match engine.apply_event(&event, &context) {
                        Ok(deltas) if !deltas.is_empty() => {
                            // Following or unfollowing a thread changes which
                            // replies may interrupt, so the set is re-read
                            // rather than left as it was at startup.
                            if deltas
                                .iter()
                                .any(|delta| matches!(delta, Delta::ThreadChanged { .. }))
                            {
                                context.followed_threads = followed(engine.store());
                            }
                            wake.wake(Update::Changed(deltas));
                        }
                        Ok(_) => {}
                        Err(error) => eprintln!("applying {}: {error}", event.name()),
                    },
                }
            }
        }
    }
}

/// The server route a picture key names, or `None` if it names nothing.
///
/// A pure function, so the routing is testable without a network or a token,
/// and so nothing a window asks for can leave the routes listed here. Ids are
/// server-generated tokens; anything else is malformed and gets no request.
pub fn route_for(key: &str) -> Option<String> {
    let path = key.split('?').next()?;
    let (kind, id) = path.trim_start_matches('/').split_once('/')?;
    if id.is_empty() || !id.chars().all(|c| c.is_ascii_alphanumeric()) {
        return None;
    }
    match kind {
        "avatar" => Some(format!("/users/{id}/image")),
        "team" => Some(format!("/teams/{id}/image")),
        "emoji" => Some(format!("/emoji/{id}/image")),
        // The thumbnail is what a message list wants: measured at 20 KB against
        // a 474 KB original on this server.
        "thumb" => Some(format!("/files/{id}/thumbnail")),
        "preview" => Some(format!("/files/{id}/preview")),
        "file" => Some(format!("/files/{id}")),
        _ => None,
    }
}

/// Which channels a batch of changes touched, so only an open one is re-read.
///
/// A channel nobody is looking at still had its post stored -- that is the
/// engine's job and it has already happened. This only answers what has to be
/// drawn again.
pub fn touched(deltas: &[Delta]) -> Vec<String> {
    let mut channels: Vec<String> = deltas
        .iter()
        .filter_map(|delta| match delta {
            Delta::PostUpserted { channel_id, .. }
            | Delta::PostTombstoned { channel_id, .. }
            | Delta::UnreadChanged { channel_id, .. }
            | Delta::ThreadChanged { channel_id, .. } => Some(channel_id.clone()),
            _ => None,
        })
        .collect();
    channels.sort();
    channels.dedup();
    channels
}

/// Whether a batch changed anything a *thread* pane is showing.
pub fn touches_thread(deltas: &[Delta], root_id: &str) -> bool {
    deltas.iter().any(|delta| match delta {
        Delta::PostUpserted {
            root_id: root,
            post_id,
            ..
        } => root == root_id || post_id == root_id,
        Delta::ThreadChanged { root_id: root, .. } => root == root_id,
        Delta::ReactionsChanged { post_id } => post_id == root_id,
        _ => false,
    })
}

/// The messages whose reactions changed.
///
/// Separate from `touched` because the delta carries no channel: only the
/// window knows whether one of these is on screen, and without asking it a
/// reaction stayed invisible until the reader left the channel and came back.
pub fn reacted(deltas: &[Delta]) -> Vec<String> {
    let mut posts: Vec<String> = deltas
        .iter()
        .filter_map(|delta| match delta {
            Delta::ReactionsChanged { post_id } => Some(post_id.clone()),
            _ => None,
        })
        .collect();
    posts.sort();
    posts.dedup();
    posts
}

/// Whether the custom emoji table changed under what is on screen.
///
/// Separate from `touched`, which deliberately names no channel for this: the
/// table arriving is not a reason to replan four hundred messages, but it is a
/// reason to look the names up again. A pill drawn before the table landed
/// found nothing behind its name and stayed blank until the channel was left.
pub fn renames_emoji(deltas: &[Delta]) -> bool {
    deltas
        .iter()
        .any(|delta| matches!(delta, Delta::CustomEmojiChanged))
}

#[cfg(test)]
mod tests {
    use super::*;
    use matterless_store::{PostChange, Unread};

    /// Both of them, because a name is stored by whichever client sent it.
    const SEPARATORS: char = '\\';
    use matterless_sync::Arrival;
    use matterless_sync::notify::MentionVerdict;

    fn posted(channel: &str, root: &str, post: &str) -> Delta {
        Delta::PostUpserted {
            post_id: post.into(),
            channel_id: channel.into(),
            root_id: root.into(),
            arrival: Arrival::Live,
            change: PostChange::Inserted,
            in_stream: true,
            mention: MentionVerdict::None,
            notify: false,
        }
    }

    #[test]
    fn only_the_channels_that_changed_are_named() {
        let deltas = vec![
            posted("one", "", "p1"),
            posted("one", "", "p2"),
            Delta::UnreadChanged {
                channel_id: "two".into(),
                unread: Unread::default(),
            },
            Delta::StatusChanged {
                user_id: "u1".into(),
                status: "online".into(),
            },
        ];
        assert_eq!(touched(&deltas), vec!["one".to_string(), "two".to_string()]);
    }

    /// Ephemeral changes redraw nothing: a status or a typing indicator is not
    /// a reason to re-read four hundred messages.
    #[test]
    fn ephemeral_changes_name_no_channel() {
        let deltas = vec![
            Delta::Typing {
                channel_id: "one".into(),
                user_id: "u1".into(),
                root_id: String::new(),
            },
            Delta::CustomEmojiChanged,
        ];
        assert!(touched(&deltas).is_empty());
    }

    /// Ephemeral for planning is not ephemeral for drawing: the emoji table is
    /// the one delta that redraws nothing and still changes what is on screen.
    #[test]
    fn the_emoji_table_is_ephemeral_but_not_ignorable() {
        let deltas = vec![Delta::CustomEmojiChanged];
        assert!(touched(&deltas).is_empty());
        assert!(renames_emoji(&deltas));
        assert!(!renames_emoji(&[posted("one", "", "p1")]));
    }

    /// The two halves of what decides whether a reply interrupts somebody.
    ///
    /// Hardcoding `Flat` here once turned every reply in every thread into a
    /// notification, because the rule that spares an unfollowed thread only
    /// applies under collapsed ones. An empty followed set is the other half:
    /// it would go quiet about the threads the reader is actually in.
    #[test]
    fn the_reader_reads_collapsed_and_knows_what_they_follow() {
        let store = Store::open_in_memory().expect("a store");
        // Through serde, because these are wire types with no other
        // constructor and every field but the id has a default.
        let thread: matterless_core::model::UserThread =
            serde_json::from_value(serde_json::json!({ "id": "root", "last_reply_at": 20,
                                "post": { "id": "root", "channel_id": "c1" } }))
            .expect("a thread");
        store.upsert_threads(&[thread]).expect("a followed thread");

        let me: matterless_core::model::User =
            serde_json::from_value(serde_json::json!({ "id": "u1" })).expect("a user");
        let context = reader_context(me, &store);
        assert_eq!(context.thread_mode, ThreadMode::Collapsed);
        assert!(context.followed_threads.contains("root"));
        assert!(!context.followed_threads.contains("some-other-root"));
    }

    /// A file name is a string somebody else's client stored, so it must not
    /// be able to decide where the file lands.
    ///
    /// Where it lands, rather than what it is called: `..` in the middle of a
    /// name is harmless once there is no separator around it, and asserting on
    /// the name would be asserting on the spelling of the guard rather than on
    /// what the guard is for.
    #[test]
    fn a_file_name_cannot_choose_where_it_lands() {
        let folder = std::env::temp_dir().join("matterless-keeps");
        std::fs::create_dir_all(&folder).expect("a folder");
        let landed = |name: &str| {
            let path = keep(&folder, name, b"x").expect("written");
            let parent = path.parent().map(std::path::Path::to_path_buf);
            let called = path
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or_default()
                .to_string();
            let _ = std::fs::remove_file(&path);
            (parent, called)
        };

        for hostile in [
            "../../evil.txt".to_string(),
            "..".to_string(),
            "/etc/passwd".to_string(),
            format!("C:{SEPARATORS}Windows{SEPARATORS}hosts"),
            String::new(),
        ] {
            let (parent, called) = landed(&hostile);
            assert_eq!(parent.as_deref(), Some(folder.as_path()), "{hostile}");
            assert!(!called.is_empty(), "{hostile} landed with no name");
        }

        // An ordinary name survives intact, or this is a rename rather than a
        // guard.
        assert_eq!(
            landed("renderer-divergence.html").1,
            "renderer-divergence.html"
        );
    }

    /// Two files of one name are two files. Replacing the one a reader kept
    /// earlier is the kind of thing they only notice afterwards.
    #[test]
    fn a_second_file_of_the_same_name_does_not_replace_the_first() {
        let folder = std::env::temp_dir().join("matterless-twice");
        std::fs::create_dir_all(&folder).expect("a folder");
        let first = keep(&folder, "notes.txt", b"one").expect("written");
        let second = keep(&folder, "notes.txt", b"two").expect("written");
        assert_ne!(first, second);
        assert_eq!(std::fs::read(&first).expect("still there"), b"one");
        let _ = std::fs::remove_file(&first);
        let _ = std::fs::remove_file(&second);
    }

    /// A query is whatever somebody typed. Anything but the unreserved set has
    /// to be escaped, or a space ends the parameter and the server answers a
    /// different question than the one asked.
    #[test]
    fn a_query_parameter_survives_what_a_reader_types() {
        assert_eq!(encoded("voyager"), "voyager");
        assert_eq!(encoded("two words"), "two%20words");
        assert_eq!(encoded("a&b=c"), "a%26b%3Dc");
        assert_eq!(encoded("../../etc"), "..%2F..%2Fetc");
        assert_eq!(encoded("café"), "caf%C3%A9");
    }

    /// A reaction names its post and nothing else -- no channel, which is why
    /// the window has to decide whether it matters.
    #[test]
    fn a_reaction_names_the_message_it_is_on() {
        let deltas = vec![
            Delta::ReactionsChanged {
                post_id: "p1".into(),
            },
            Delta::ReactionsChanged {
                post_id: "p1".into(),
            },
            posted("c", "", "p2"),
        ];
        assert_eq!(reacted(&deltas), vec!["p1".to_string()]);
        assert!(touched(&deltas).iter().all(|channel| channel != "p1"));
    }

    fn said(id: &str, at: i64) -> matterless_core::Post {
        serde_json::from_value(serde_json::json!({
            "id": id, "channel_id": "c1", "user_id": "u1",
            "create_at": at, "update_at": at, "message": id,
        }))
        .expect("a post")
    }

    /// What the server answers with, built the way it arrives: a map plus the
    /// order, newest first, which is the shape the endpoint actually returns.
    fn page(posts: &[matterless_core::Post]) -> PostList {
        serde_json::from_value(serde_json::json!({
            "order": posts.iter().map(|post| post.id.clone()).collect::<Vec<_>>(),
            "posts": posts
                .iter()
                .map(|post| (post.id.clone(), serde_json::to_value(post).expect("json")))
                .collect::<serde_json::Map<_, _>>(),
        }))
        .expect("a page")
    }

    /// A page from the server proves a message is gone only for the span it
    /// actually covers. Anything older than the page is simply not in it, and
    /// tombstoning on that would erase history the reader still has.
    #[test]
    fn only_messages_the_page_covers_can_be_called_gone() {
        let store = Store::open_in_memory().expect("a store");
        let held = [
            said("p0", 50),
            said("p1", 100),
            said("p2", 200),
            said("p3", 300),
        ];
        store.upsert_posts(&held).expect("stored");

        // The server still has p1 and p3. p2 fell inside the page and is not
        // in it; p0 is older than the page began.
        let list = page(&[said("p3", 300), said("p1", 100)]);

        let gone = vanished(&store, "c1", &list);
        assert_eq!(gone.len(), 1);
        assert!(matches!(
            &gone[0],
            Delta::PostTombstoned { post_id, .. } if post_id == "p2"
        ));
        assert_ne!(store.post("p2").unwrap().unwrap().delete_at, 0);
        assert_eq!(store.post("p0").unwrap().unwrap().delete_at, 0);
        assert_eq!(store.post("p1").unwrap().unwrap().delete_at, 0);
    }

    /// The page's own window is `order`. A thread root dragged in by a recent
    /// reply can be months older, and taking the range from it would make 200
    /// messages evidence about a month of history -- tombstoning everything in
    /// between that this page had no room for.
    #[test]
    fn an_old_root_pulled_in_by_a_reply_does_not_widen_the_window() {
        let store = Store::open_in_memory().expect("a store");
        store
            .upsert_posts(&[said("ancient", 10), said("between", 50), said("p1", 100)])
            .expect("stored");

        // The server answered with today's page, plus the old root a reply in
        // it belongs to -- which is in `posts` but not in `order`.
        let mut list = page(&[said("p1", 100)]);
        list.posts.insert("ancient".into(), said("ancient", 10));

        assert!(vanished(&store, "c1", &list).is_empty());
        assert_eq!(store.post("between").unwrap().unwrap().delete_at, 0);
    }

    /// The inference has to be reversible, or one wrong bound hides somebody's
    /// messages for good. A page listing a post alive puts it back.
    #[test]
    fn a_message_the_server_still_has_comes_back() {
        let store = Store::open_in_memory().expect("a store");
        store.upsert_posts(&[said("p1", 100)]).expect("stored");
        store.tombstone_post("p1", 101).expect("tombstoned");
        assert_ne!(store.post("p1").unwrap().unwrap().delete_at, 0);

        let put_back = returned(&store, &page(&[said("p1", 100)]));
        assert_eq!(put_back.len(), 1);
        assert_eq!(store.post("p1").unwrap().unwrap().delete_at, 0);
        // And only once: a second visit has nothing left to correct.
        assert!(returned(&store, &page(&[said("p1", 100)])).is_empty());
    }

    /// An empty page is not evidence that a channel is empty: a request that
    /// came back with nothing must not erase what is already held.
    #[test]
    fn an_empty_page_calls_nothing_gone() {
        let store = Store::open_in_memory().expect("a store");
        store.upsert_posts(&[said("p1", 100)]).expect("stored");
        assert!(vanished(&store, "c1", &page(&[])).is_empty());
        assert_eq!(store.post("p1").unwrap().unwrap().delete_at, 0);
    }

    /// A reply belongs to its thread, and so does the root itself.
    #[test]
    fn a_thread_notices_its_own_replies() {
        assert!(touches_thread(&[posted("c", "root", "reply")], "root"));
        assert!(touches_thread(&[posted("c", "", "root")], "root"));
        assert!(!touches_thread(&[posted("c", "other", "reply")], "root"));
    }
}

/// Decodes a picture to straight RGBA at its own size.
///
/// Scaled down to what the atlas can hold rather than refused: an avatar comes
/// back at whatever size the server keeps, and a face that would not fit is
/// better small than missing.
fn decode(bytes: &[u8], width: u32, height: u32) -> Option<(u32, u32, Vec<u8>)> {
    let decoded = image::load_from_memory(bytes).ok()?;
    let mut rgba = decoded.to_rgba8();
    // Never enlarged: a picture smaller than its box stays its own size and the
    // sampler stretches it, which costs nothing and keeps the atlas small.
    let wanted = (
        width.clamp(1, MAX_SIDE).min(rgba.width()),
        height.clamp(1, MAX_SIDE).min(rgba.height()),
    );
    if (rgba.width(), rgba.height()) != wanted {
        // A box filter on the way in, which is a better downscale than the
        // bilinear one the sampler would do on the way out -- and it is done
        // once rather than every frame.
        rgba = image::imageops::thumbnail(&rgba, wanted.0, wanted.1);
    }
    Some((rgba.width(), rgba.height(), rgba.into_raw()))
}

/// What a downloaded picture actually is, by its first bytes.
///
/// Only for saying so in a message: the decoder sniffs for itself.
fn kind_of(bytes: &[u8]) -> &'static str {
    match bytes {
        [0x89, b'P', b'N', b'G', ..] => "png",
        [0xFF, 0xD8, ..] => "jpeg",
        [b'G', b'I', b'F', ..] => "gif",
        [
            b'R',
            b'I',
            b'F',
            b'F',
            _,
            _,
            _,
            _,
            b'W',
            b'E',
            b'B',
            b'P',
            ..,
        ] => "webp",
        [b'<', ..] | [0xEF, 0xBB, 0xBF, b'<', ..] => "markup, not a picture",
        [b'{', ..] => "json, not a picture",
        _ => "something unrecognised",
    }
}

/// The largest picture worth putting in a shared atlas.
///
/// The server's thumbnails come back around this size and a message list never
/// draws one larger, so this is a guard against a surprise rather than a
/// resize anything normally hits.
const MAX_SIDE: u32 = 640;

#[cfg(test)]
mod routes {
    use super::route_for;

    #[test]
    fn a_key_names_the_route_it_came_from() {
        assert_eq!(
            route_for("avatar/abc123?v=17").as_deref(),
            Some("/users/abc123/image")
        );
        assert_eq!(
            route_for("thumb/file9").as_deref(),
            Some("/files/file9/thumbnail")
        );
    }

    /// Nothing a window asks for may leave the routes listed here: an id is a
    /// server-generated token, and anything else gets no request made for it.
    #[test]
    fn a_malformed_key_asks_for_nothing() {
        assert_eq!(
            route_for("team/abc123").as_deref(),
            Some("/teams/abc123/image")
        );
        assert_eq!(route_for("avatar/../../etc/passwd"), None);
        assert_eq!(route_for("avatar/"), None);
        assert_eq!(route_for("avatar"), None);
        assert_eq!(route_for("secrets/abc123"), None);
        assert_eq!(route_for("avatar/abc-123"), None);
    }
}

/// How this window reads, for the notification rules.
///
/// Collapsed, matching how the channel is actually planned and drawn. Flat was
/// wrong twice over: it disagreed with the drawing, and it silently disabled
/// the rule in `decide` that keeps a reply in a thread nobody follows from
/// interrupting them -- that rule only applies under collapsed threads, so
/// every reply in every thread was notifying.
fn reader_context(me: matterless_core::User, store: &Store) -> SyncContext {
    let mut context = SyncContext::new(me, ThreadMode::Collapsed);
    context.followed_threads = followed(store);
    context
}

/// The thread roots this reader follows.
///
/// Read from the store rather than left empty, which is what decides whether a
/// reply interrupts somebody: under collapsed threads a reply notifies only if
/// the thread is followed or it names them outright. An empty set is a safe
/// answer -- nothing interrupts -- but a wrong one, because a thread the reader
/// is actually in should.
fn followed(store: &Store) -> std::collections::HashSet<String> {
    store
        .followed_threads(FOLLOWED)
        .unwrap_or_default()
        .into_iter()
        .map(|thread| thread.root_id)
        .collect()
}

/// How many followed threads are held. Well past what anybody follows at once.
const FOLLOWED: u32 = 500;

/// How much recent history one visit reconciles. What the official client asks
/// for on a channel switch, and one request rather than a paged walk.
const RECENT: u32 = 200;

/// Tombstones the messages the server no longer has.
///
/// A deleted post is not reported as deleted -- the server simply stops
/// listing it -- so the only way to learn of one missed while disconnected is
/// that it is absent from a page that covers its time. Bounded to the range
/// the page actually spans, and to roots, which is what the stream draws:
/// outside that range absence proves nothing, and a reply can be absent from a
/// channel page while still existing under an older root.
///
/// Tombstoned rather than deleted. It is an inference, and a row that can be
/// corrected by the next thing the server says is a safer answer than one that
/// is gone.
fn vanished(store: &Store, channel_id: &str, list: &PostList) -> Vec<Delta> {
    // Bounded by `order`, not by `posts`. The map carries thread roots dragged
    // in by a recent reply, which can be months older than the page itself --
    // taking the range from them would claim a page of 200 messages was
    // evidence about a month of history, and tombstone everything between.
    // `order` is the contiguous window, and only over that does absence mean
    // anything.
    let (Some(newest), Some(oldest)) = (
        list.order.first().and_then(|id| list.posts.get(id)),
        list.oldest_in_order(),
    ) else {
        return Vec::new();
    };
    let (newest, oldest) = (newest.create_at, oldest.create_at);
    // A page that did not reach back far enough proves nothing about anything
    // older than it, and `has_next` says the server had more to give.
    let held = store
        .channel_page_filtered(channel_id, Some(newest + 1), Some(oldest), RECENT * 2, true)
        .unwrap_or_default();
    held.into_iter()
        .filter(|post| post.delete_at == 0 && !list.posts.contains_key(&post.id))
        .filter_map(|post| {
            // The server's own clock is not available here, and the value is
            // only ever compared against zero.
            match store.tombstone_post(&post.id, post.create_at + 1) {
                Ok(true) => {
                    println!("{} is gone from the server", post.id);
                    Some(Delta::PostTombstoned {
                        post_id: post.id,
                        channel_id: post.channel_id,
                    })
                }
                Ok(false) => None,
                Err(error) => {
                    eprintln!("tombstoning {}: {error}", post.id);
                    None
                }
            }
        })
        .collect()
}

/// Undoes a tombstone the server disagrees with.
///
/// `vanished` infers a delete from absence, and an inference can be wrong: a
/// page that did not reach as far back as it looked, or a bound taken from the
/// wrong end of the list. This is what makes that recoverable -- a message the
/// server hands back alive is put back, so a mistake lasts until the next
/// visit rather than for good.
fn returned(store: &Store, list: &PostList) -> Vec<Delta> {
    list.posts
        .values()
        .filter(|post| post.delete_at == 0)
        .filter_map(|post| match store.restore_post(&post.id) {
            Ok(true) => {
                println!("{} is still there after all", post.id);
                Some(Delta::PostUpserted {
                    post_id: post.id.clone(),
                    channel_id: post.channel_id.clone(),
                    root_id: post.root_id.clone(),
                    arrival: Arrival::Backfill,
                    change: matterless_store::PostChange::Updated,
                    in_stream: !post.is_reply(),
                    mention: matterless_sync::notify::MentionVerdict::None,
                    // A message that was always there is not news.
                    notify: false,
                })
            }
            Ok(false) => None,
            Err(error) => {
                eprintln!("restoring {}: {error}", post.id);
                None
            }
        })
        .collect()
}

/// How many messages one list holds. Long enough that scrolling it is the
/// exception, short enough that the panel is not a second conversation.
const LISTED: u32 = 60;

/// Turns a server list into rows the panel can draw.
///
/// Newest first, which is the order both endpoints answer in and the order a
/// list of things you saved is worth reading in.
fn listed(title: &str, store: &Store, list: PostList, me_id: &str) -> Update {
    let mut posts: Vec<matterless_core::Post> = list
        .order
        .iter()
        .filter_map(|id| list.posts.get(id).cloned())
        // A tombstone is not something to show in a list of saved messages:
        // the message is gone, and the save outliving it says nothing.
        .filter(|post| post.delete_at == 0)
        .collect();
    // `order` is the server's, but a pinned list arrives unordered often
    // enough that sorting here is cheaper than trusting it.
    posts.sort_by_key(|post| std::cmp::Reverse(post.create_at));
    println!("{title}: {} messages", posts.len());
    Update::Listed {
        title: title.to_string(),
        found: crate::listing::found_for(store, posts, me_id),
    }
}

/// Pulls every channel and membership this reader has, into the store.
///
/// Teams first, because a channel is asked for per team and a direct message
/// comes back under every one of them -- so they are deduplicated by id or the
/// same conversation is stored several times over.
async fn membership(
    rest: &matterless_core::rest::RestClient,
    store: &Store,
    me_id: &str,
) -> matterless_core::Result<(usize, ThreadMode)> {
    let teams = rest.my_teams().await?;
    let mut channels = Vec::new();
    let mut members = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for team in &teams {
        for channel in rest.my_channels(&team.id).await? {
            if channel.delete_at == 0 && seen.insert(channel.id.clone()) {
                channels.push(channel);
            }
        }
        members.extend(rest.my_channel_members(&team.id).await?);
    }
    let counted = channels.len();
    if let Err(error) = store
        .upsert_teams(&teams)
        .and_then(|()| store.upsert_channels(&channels))
        .and_then(|()| store.upsert_channel_members(&members))
    {
        eprintln!("storing the sidebar: {error}");
    }
    // Both halves, because neither answers it alone: the server said
    // `default_off` here while the account said `on`, and the account wins.
    // With `resolve`, a missing server setting means threads are off entirely
    // -- so this has to be asked for rather than assumed.
    let config = rest.client_config().await.unwrap_or_default();
    let preferences = rest.preferences(me_id).await.unwrap_or_default();
    Ok((
        counted,
        matterless_core::resolve_thread_mode(&config, &preferences),
    ))
}

/// Percent-encodes a value for one query parameter.
///
/// Everything but the unreserved set, which is the only safe rule when the
/// value is whatever somebody typed into a box.
fn encoded(value: &str) -> String {
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

/// How many of each kind a discovery offers. A switcher shows eight rows in
/// total, so more than this is answering a question nobody can see.
const DISCOVERED: u32 = 10;

/// What else a name could mean: channels not joined, and people not written to.
///
/// The reader's own conversations are matched locally and instantly; this is
/// only the rest, so anything the store already holds is filtered out rather
/// than offered twice under two different verbs.
async fn discover(
    rest: &matterless_core::rest::RestClient,
    store: &Store,
    me_id: &str,
    query: &str,
) -> Vec<crate::switcher::Match> {
    let wanted = query.to_lowercase();
    let mut found = Vec::new();

    for team in store.teams().unwrap_or_default() {
        let channels = rest
            .public_channels(&team.id, 0, 200)
            .await
            .unwrap_or_default();
        for channel in channels {
            // Already in it, so the sidebar match covers it.
            if store.channel(&channel.id).ok().flatten().is_some() {
                continue;
            }
            if !channel.display_name.to_lowercase().contains(&wanted)
                && !channel.name.to_lowercase().contains(&wanted)
            {
                continue;
            }
            found.push(crate::switcher::Match {
                id: channel.id,
                label: format!("{} -- join", channel.display_name),
                direct: false,
                reach: crate::switcher::Reach::Join,
            });
            if found.len() >= DISCOVERED as usize {
                break;
            }
        }
    }

    // People, by whatever the server suggests for the letters typed. Encoded,
    // because a query is whatever the reader typed: a space or an ampersand in
    // it would otherwise end the parameter and the server would answer a
    // different question.
    let people = rest
        .autocomplete_users(&format!("/users/autocomplete?name={}", encoded(&wanted)))
        .await
        .unwrap_or_default();
    // Remembered, so a conversation opened with one of them is labelled by
    // their name rather than their id: the sidebar resolves a direct message's
    // counterpart out of the store, and somebody met for the first time here
    // is not in it yet.
    if let Err(error) = store.upsert_users(&people) {
        eprintln!("storing the people found: {error}");
    }
    for person in people.into_iter().take(DISCOVERED as usize) {
        if person.id == me_id {
            continue;
        }
        found.push(crate::switcher::Match {
            id: person.id,
            label: format!("@{} -- message", person.username),
            direct: true,
            reach: crate::switcher::Reach::Direct,
        });
    }
    found
}

/// The largest file this window will send.
///
/// The server's own limit is in its client config and is larger than this on
/// this deployment; the point of a cap here is that a file is read into memory
/// whole, so an accidental drop of something enormous is refused rather than
/// spending a gigabyte finding out the server would refuse it too.
const LARGEST: u64 = 100 * 1024 * 1024;

/// Sends a file, then a message carrying it.
///
/// Two steps because the server's are two: the bytes go up and come back with
/// an id, and the post is created with that id in `file_ids`. An upload with
/// no post attached is orphaned rather than broken, which is why a failed send
/// here costs nothing but disk on the server.
pub async fn upload(
    rest: &matterless_core::rest::RestClient,
    engine: &SyncEngine,
    context: &SyncContext,
    channel_id: &str,
    root_id: &str,
    path: &std::path::Path,
) {
    let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
        eprintln!("that file has no name this window can send");
        return;
    };
    match std::fs::metadata(path) {
        Ok(held) if held.len() > LARGEST => {
            eprintln!(
                "{name} is {} MB, which is too large",
                held.len() / 1_048_576
            );
            return;
        }
        Err(error) => {
            eprintln!("{name}: {error}");
            return;
        }
        _ => {}
    }
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) => {
            eprintln!("{name}: {error}");
            return;
        }
    };
    println!("sending {name}, {} bytes", bytes.len());

    let sent = match rest.upload_file(channel_id, name, &bytes, None).await {
        Ok(sent) => sent,
        Err(error) => {
            eprintln!("uploading {name}: {error}");
            return;
        }
    };
    let file_ids: Vec<String> = sent.file_infos.into_iter().map(|file| file.id).collect();
    if file_ids.is_empty() {
        eprintln!("{name} uploaded but the server named no file");
        return;
    }
    // No message of its own: the file is the message. A caption would need a
    // composer that knows a drop is coming, which is a different feature.
    let request = matterless_core::model::NewPost {
        channel_id,
        message: "",
        root_id,
        pending_post_id: "",
        file_ids: &file_ids,
    };
    match rest.create_post(&request).await {
        Ok(post) => {
            let event = matterless_core::Event::Posted {
                channel_id: post.channel_id.clone(),
                post: Box::new(post),
            };
            if let Err(error) = engine.apply_event(&event, context) {
                eprintln!("storing the message that carries {name}: {error}");
            }
        }
        Err(error) => eprintln!("sending {name}: {error}"),
    }
}

/// Where this machine keeps what a person downloads.
fn downloads() -> std::path::PathBuf {
    let home = std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .map(std::path::PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    let folder = home.join("Downloads");
    if folder.is_dir() { folder } else { home }
}

/// Writes a downloaded file into a folder.
///
/// The name is taken apart and rebuilt rather than used as it came: it is a
/// string the server stored on somebody else's say-so, and a path separator or
/// a `..` in it would put the file somewhere nobody asked for.
fn keep(folder: &std::path::Path, name: &str, bytes: &[u8]) -> std::io::Result<std::path::PathBuf> {
    let safe: String = name
        .chars()
        .map(|character| {
            if character.is_alphanumeric() || matches!(character, '.' | '-' | '_' | ' ') {
                character
            } else {
                '_'
            }
        })
        .collect();
    let safe = safe.trim_matches(['.', ' ']).to_string();
    let safe = if safe.is_empty() {
        "attachment".to_string()
    } else {
        safe
    };

    // Never over something already there. A second copy of a file is a second
    // file, and silently replacing one the reader kept earlier is the kind of
    // thing they would only notice afterwards.
    let mut path = folder.join(&safe);
    let mut attempt = 1;
    while path.exists() {
        let (stem, extension) = safe.rsplit_once('.').unwrap_or((safe.as_str(), ""));
        path = folder.join(if extension.is_empty() {
            format!("{stem} ({attempt})")
        } else {
            format!("{stem} ({attempt}).{extension}")
        });
        attempt += 1;
    }
    std::fs::write(&path, bytes)?;
    Ok(path)
}
