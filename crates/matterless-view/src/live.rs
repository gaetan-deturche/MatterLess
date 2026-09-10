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
use matterless_core::ws::{Signal, WsSession};
use matterless_core::{ThreadMode, User};
use matterless_store::Store;
use matterless_sync::{Delta, SyncContext, SyncEngine};
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
    let _handle = session.spawn(signals_tx);

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
                            // server. React opens a picker, and the emoji it
                            // chooses arrives later as its own ask.
                            Action::React | Action::Thread | Action::Link => Ok(()),
                        };
                        match done {
                            // The socket echoes the change, which is what
                            // redraws the row. Nothing to do here but say it
                            // went.
                            Ok(()) => println!("{} on {post_id}", action.slug()),
                            Err(error) => eprintln!("{} on {post_id}: {error}", action.slug()),
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
