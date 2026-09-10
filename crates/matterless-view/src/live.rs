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
    /// Fetch a picture. The key is the window's own name for it, and the route
    /// is derived from it here so the window never builds a server path.
    Fetch { key: String },
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
    // Flat rather than the reader's own preference: this window draws a channel
    // flat, and a context that disagreed with the drawing would count unreads
    // against rows that are not there.
    let context = SyncContext::new(me, ThreadMode::Flat);

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
                    Ask::Fetch { key } => {
                        let Some(route) = route_for(&key) else {
                            continue;
                        };
                        match rest.fetch_bytes(&route).await {
                            Ok(Some((bytes, _))) => match decode(&bytes) {
                                Some((width, height, rgba)) => wake.wake(Update::Picture {
                                    key,
                                    width,
                                    height,
                                    rgba,
                                }),
                                None => eprintln!("{key}: could not be decoded"),
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
                        Ok(deltas) if !deltas.is_empty() => wake.wake(Update::Changed(deltas)),
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
fn decode(bytes: &[u8]) -> Option<(u32, u32, Vec<u8>)> {
    let decoded = image::load_from_memory(bytes).ok()?;
    let mut rgba = decoded.to_rgba8();
    if rgba.width() > MAX_SIDE || rgba.height() > MAX_SIDE {
        rgba = image::imageops::thumbnail(
            &rgba,
            rgba.width().min(MAX_SIDE),
            rgba.height().min(MAX_SIDE),
        );
    }
    Some((rgba.width(), rgba.height(), rgba.into_raw()))
}

/// The largest picture worth putting in a shared atlas. A face is drawn at 28
/// pixels, so anything past this is detail nothing will ever see.
const MAX_SIDE: u32 = 128;

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
