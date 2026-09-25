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
    /// People the store had not met are in it now, so anything named from it
    /// is named differently: a direct message's row, an author over a message,
    /// a name taken back out of a group's label.
    Met,
    /// A sign-in came back with a session.
    ///
    /// The token is an `AuthToken` rather than a `String` so that it keeps its
    /// redacted `Debug`: this enum derives one, and every other variant is
    /// something it is fine to print.
    SessionOpened {
        token: AuthToken,
        user_id: String,
        username: String,
        /// The server it is a session on, as the window will store it.
        server: String,
    },
    /// A sign-in was refused, in the server's own words where there are any.
    SignInRefused {
        why: String,
        /// The account has MFA on and the code is what was missing, which is a
        /// different thing to say than "that did not work".
        needs_a_code: bool,
    },
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
    /// A newer build is there, and here is what it would take to install it.
    ///
    /// Offered, never taken: nothing has been fetched at this point beyond the
    /// manifest saying it exists.
    Updatable(crate::update::Offer),
    /// A look somebody asked for came back, with whatever it found -- nothing
    /// newer, something newer, or a reason it could not tell.
    ///
    /// Carries the whole outcome rather than only the good half, because the
    /// reader is waiting on the answer either way.
    Checked(Result<Option<crate::update::Offer>, String>),
    /// Installing the build the reader accepted did not work.
    UpdateFailed(String),
    /// A file is on the server and waiting for a message to claim it.
    ///
    /// Boxed because a `FileInfo` is the largest thing this enum carries and
    /// every other variant would be sized by it.
    Attached {
        channel_id: String,
        root_id: String,
        file: Box<matterless_core::model::FileInfo>,
        /// Which of the files on their way this one was.
        ///
        /// The window puts a tile up the moment something is dropped, before
        /// any of it has left the machine, and the path is what tells it
        /// which tile just became real. Matching on the name instead would
        /// mistake two drops of `image.png` for each other.
        path: std::path::PathBuf,
    },
    /// A file that was on its way is not going to arrive.
    ///
    /// Said out loud rather than left silent, because the tile for it is
    /// already on screen: without this the reader is left looking at a
    /// picture that will never finish, with no way to take it off.
    NotAttached {
        channel_id: String,
        root_id: String,
        path: std::path::PathBuf,
        why: String,
    },
    /// The tray icon was used.
    ///
    /// It arrives the same way, even though the shell delivers it on this very
    /// thread: going through the proxy means the window is not being changed
    /// from inside a window procedure, halfway through winit's own dispatch.
    Tray(crate::tray::Act),
    /// Older history arrived and is in the store. `more` is false once the
    /// beginning of the channel has been reached, so the window stops asking.
    Older { channel_id: String, more: bool },
    /// Somebody's record was fetched again and is in the store.
    Person { user_id: String },
    /// Who is around: each person, their status, and when they were last
    /// active -- which is what "last online 7 min. ago" is worked out from.
    Statuses(Vec<(String, String, i64)>),
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
    /// The same, for a picture with more than one frame in it.
    ///
    /// Its own update rather than a longer `Picture`, because almost nothing
    /// is animated and every avatar in the window would otherwise carry an
    /// empty list of frames across a channel. The first frame arrives as an
    /// ordinary `Picture` beside this, so a build that did nothing with these
    /// would still draw what it draws today.
    Moving {
        key: String,
        width: u32,
        height: u32,
        /// Every frame and how long it lasts, in order.
        frames: Vec<(Vec<u8>, std::time::Duration)>,
    },
    /// The picture a reader opened, full size, for a texture of its own.
    ///
    /// Apart from `Picture` because it does not go into the atlas: one of
    /// these is two thousand pixels across and the atlas is a shared sheet
    /// with no eviction.
    Looked {
        file_id: String,
        /// The size it was fitted to, so it can be remembered at that size.
        within: (u32, u32),
        width: u32,
        height: u32,
        rgba: Vec<u8>,
        /// Every frame at that size, for a picture that moves.
        frames: Option<Vec<(Vec<u8>, std::time::Duration)>>,
    },
    /// And the picture could not be had, so the viewer can say so rather than
    /// showing an empty window.
    LookFailed { file_id: String, why: String },
    /// A video is on the disk, ready to be opened.
    Film {
        file_id: String,
        path: std::path::PathBuf,
    },
    /// A text file a reader opened, as text.
    Text { file_id: String, text: String },
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
        /// Files already uploaded and waiting for this post to claim them.
        file_ids: Vec<String>,
    },
    /// Put a file on the server and say so, without posting anything.
    ///
    /// What both gestures do: a file pasted into the box and a file dropped on
    /// the window alike wait with whatever is being written until the reader
    /// sends them. A drop used to be the whole errand, upload *and* post, and
    /// there is deliberately no request for that any more -- posting a file
    /// the moment it lands is the one thing a reader cannot take back.
    Attach {
        channel_id: String,
        root_id: String,
        path: std::path::PathBuf,
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
    /// Fetch a video to the disk, to be played from there.
    Film { file_id: String },
    /// Fetch a text file, to be read.
    Read { file_id: String },
    /// Fetch one attachment at full size, to be looked at.
    ///
    /// `within` is how big the window is: the picture is scaled down to fit it
    /// on the way in, because a photograph from a phone is four thousand
    /// pixels across and nothing on screen is.
    Look {
        file_id: String,
        /// The original rather than the server's re-encoded preview, which is
        /// right for anything it does not re-encode -- a GIF, an SVG.
        original: bool,
        /// A picture a message links to: `file_id` is its URL.
        linked: bool,
        within: (u32, u32),
    },
    /// Put these pictures on the disk before anybody asks for them.
    ///
    /// For faces, which are the ones that make a channel look half-drawn: a
    /// face is about 24KB and the link measured 220KB/s, so the first sighting
    /// of thirty people costs three seconds however it is arranged. Fetched
    /// ahead of time it costs nothing anybody is waiting on.
    ///
    /// Nothing is decoded and nothing is drawn -- this only fills the disk
    /// cache, so the fetch that follows a real sighting is a file read.
    Warm { keys: Vec<String> },
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
    /// Make a whole channel unread again, from its newest message down.
    ///
    /// Which is what the app means by it: the channel goes back to unread
    /// from there, rather than every message in it being forgotten.
    Unseen { channel_id: String },
    /// Move a channel into one of the team's sidebar categories.
    ///
    /// Favouriting is this with the favourites category as the target, which
    /// is why there is no separate ask for it.
    Move {
        channel_id: String,
        team_id: String,
        category_id: String,
    },
    /// Be told about one message again later.
    Remind { post_id: String, when: i64 },
    /// Everything said in one thread.
    ///
    /// A followed thread can live in a channel this window has never opened,
    /// so its replies are not in the store and nothing else would ever fetch
    /// them: the channel's own refresh brings the recent page of that channel,
    /// and a thread older than that page is not in it.
    Thread { root_id: String, channel_id: String },
    /// Join a public channel, then open it.
    Join { channel_id: String },
    /// Find or create the conversation with one person, then open it.
    Direct { user_id: String },
    /// Open the conversation between these people, starting it if it is not
    /// there. Two or more others; one is a `Direct`.
    Group { user_ids: Vec<String> },
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
    /// Fetch one person's record again, whether or not the store has met them.
    ///
    /// For the card a name opens: a record is otherwise fetched only the
    /// first time somebody is met, so a position or a name changed since --
    /// or a field this build did not keep when it was fetched -- would never
    /// arrive.
    Person { user_id: String },
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
    /// Tell the server this thread has been read, and clear it here.
    ReadThread { root_id: String },
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
        /// A fingerprint of the bytes already drawn under this key, when the
        /// shelf drew another version of the face while this was on its way.
        ///
        /// Carried rather than looked up again on arrival: by then the disk may
        /// hold a copy somebody else wrote since -- the warm-up, or the same
        /// face under its other key -- and taking that for what is on screen
        /// left faces that were never drawn at all.
        shown: Option<u64>,
    },
}

/// The window's end of the socket thread. Dropping it closes the connection.
#[derive(Clone)]
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
pub trait Wake: Clone + Send + Sync + 'static {
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

/// Keeps the session, so the next run does not ask again.
///
/// The keychain rather than a file beside the database: it is a thirty-day
/// bearer credential for somebody's account, and the database's own folder is
/// somewhere anything running as this user can read.
pub fn remember_token(token: &str) -> Result<(), String> {
    let entry = keyring::Entry::new("matterless", "session")
        .map_err(|error| format!("no keychain entry: {error}"))?;
    entry
        .set_password(token)
        .map_err(|error| format!("the keychain refused the session: {error}"))?;
    // Read back, because a write that reports success and does not keep the
    // value is exactly what this machine's credential store does: the entry is
    // listed, and asking for its password answers nothing usable. A caller
    // told the session was kept would go on to read it and find nothing.
    match entry.get_password() {
        Ok(kept) if kept == token => Ok(()),
        Ok(_) => Err("the keychain kept something other than the session".to_string()),
        Err(error) => Err(format!(
            "the keychain took the session and lost it: {error}"
        )),
    }
}

/// Keeps the server, beside the database, where `stored_server` reads it.
pub fn remember_server(database: &std::path::Path, server: &str) -> Result<(), String> {
    let directory = database
        .parent()
        .ok_or_else(|| format!("{} has no folder", database.display()))?;
    std::fs::create_dir_all(directory)
        .map_err(|error| format!("could not make {}: {error}", directory.display()))?;
    std::fs::write(directory.join("server.txt"), server)
        .map_err(|error| format!("could not write server.txt: {error}"))
}

/// The server this install is pointed at, from the file beside the database.
///
/// A sandbox run with no `server.txt` of its own borrows the installed
/// program's. The address is not the reader's data -- it is which server this
/// machine talks to, and the session behind it is in the keyring, which both
/// builds share anyway. Without this a dev build would come up on the sign-in
/// screen every time its store was new, which is the sort of friction that
/// gets a separation like this quietly undone.
pub fn stored_server(database: &std::path::Path) -> Option<String> {
    let named = |directory: &std::path::Path| -> Option<String> {
        let held = std::fs::read_to_string(directory.join("server.txt")).ok()?;
        let trimmed = held.trim().to_string();
        (!trimmed.is_empty()).then_some(trimmed)
    };
    named(database.parent()?).or_else(|| {
        let installed = crate::feed::installed_store()?;
        named(installed.parent()?)
    })
}

/// Signs in, on a thread of its own, and reports what came back.
///
/// Returns immediately, like `start`. A login is a network round trip against
/// a host somebody has just typed, which is the one request in this program
/// most likely to take the full thirty-second timeout -- doing it on the
/// thread that owns the window would freeze the form mid-keystroke, on the
/// one screen where the reader has no other way to tell it is working.
///
/// The password is taken by value and dropped with the thread: nothing here
/// holds it past the one request it is for.
pub fn sign_in(
    server: String,
    login: String,
    password: String,
    code: Option<String>,
    wake: impl Wake,
) {
    std::thread::spawn(move || {
        let runtime = match tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        {
            Ok(runtime) => runtime,
            Err(error) => {
                wake.wake(Update::SignInRefused {
                    why: format!("no runtime: {error}"),
                    needs_a_code: false,
                });
                return;
            }
        };
        runtime.block_on(async move {
            let client = match RestClient::new(&server) {
                Ok(client) => client,
                Err(error) => {
                    wake.wake(Update::SignInRefused {
                        why: said_plainly(&error),
                        needs_a_code: false,
                    });
                    return;
                }
            };
            let who = match client.login(&login, &password, code.as_deref()).await {
                Ok(who) => who,
                Err(error) => {
                    wake.wake(Update::SignInRefused {
                        why: said_plainly(&error),
                        needs_a_code: matches!(error, matterless_core::Error::MfaRequired),
                    });
                    return;
                }
            };
            let Some(token) = client.token() else {
                // `login` sets the token from the response header and fails
                // without one, so this cannot happen -- but the alternative to
                // saying so is an `expect` on a network response.
                wake.wake(Update::SignInRefused {
                    why: "the server signed us in without a session token".to_string(),
                    needs_a_code: false,
                });
                return;
            };
            wake.wake(Update::SessionOpened {
                token,
                user_id: who.id,
                username: who.username,
                server,
            });
        });
    });
}

/// What to put under the form when a sign-in did not work.
///
/// The `Display` on these is written for a log: "api 401 [store.sql_user.get_for_login.app_error]"
/// is exact and says nothing to somebody who has just mistyped a password. The
/// cases a reader can actually do something about get a sentence; everything
/// else keeps the original, because a wrong guess at what went wrong is worse
/// than an ugly true one.
fn said_plainly(error: &matterless_core::Error) -> String {
    use matterless_core::Error;
    match error {
        Error::Url(_) => "That does not look like a server address.".to_string(),
        Error::MfaRequired => "That account needs its one-time code.".to_string(),
        Error::SessionExpired => "That login or password was not accepted.".to_string(),
        Error::RateLimited { retry_after_secs } => {
            format!("Too many attempts. Try again in {retry_after_secs}s.")
        }
        Error::Transport(inner) if inner.is_connect() || inner.is_timeout() => {
            format!("Could not reach that server: {inner}")
        }
        Error::Api(envelope, _) if !envelope.message.is_empty() => envelope.message.clone(),
        other => other.to_string(),
    }
}

/// Runs one request on a thread with a runtime of its own.
///
/// Three things in this file are a thread, a runtime and one request that has
/// no business on the socket: signing in, asking what the newest build is, and
/// fetching it. Two of those have to work when there is no socket at all.
fn apart<Work>(named: &'static str, work: impl FnOnce() -> Work + Send + 'static)
where
    Work: std::future::Future<Output = ()>,
{
    std::thread::spawn(move || {
        match tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        {
            Ok(runtime) => runtime.block_on(work()),
            Err(error) => eprintln!("no runtime to {named} with: {error}"),
        }
    });
}

/// How long between one look for a newer build and the next.
///
/// This window is left open for days, so looking once at start-up means an
/// install can be a week behind and never hear about it. Two hours puts a
/// build released in the morning in front of somebody before they go home,
/// and costs twelve requests a day for a manifest of about three kilobytes.
///
/// The reason it is not shorter is the reason it used to be once only: an
/// update is not urgent, and a client that keeps raising the subject is a
/// client talking about itself rather than about anything the reader came
/// for. A version already refused is never raised again, which is what makes
/// a repeating check bearable at all -- see `put_off` in the window.
pub const HOW_OFTEN: std::time::Duration = std::time::Duration::from_secs(2 * 60 * 60);

/// Asks the release host whether there is a newer build, now and then again.
///
/// Nothing to do with the session. `looked` builds a client of its own
/// precisely because this is a request to a release host and has no business
/// carrying a Mattermost token -- and then it was queued on the socket, which
/// does not exist until there is a store, a server and a session. So the one
/// install most in need of an update, the one that cannot sign in, was the
/// only one that never asked. It never failed a check; it never made one.
///
/// One thread that sleeps, rather than a timer the window has to hold: the
/// window draws when something happens, and nothing happening is exactly the
/// state this has to work in.
/// Looks once, because somebody asked.
///
/// Its own call rather than a nudge to the loop above: what the reader wants
/// is an answer, and "already the newest build" is an answer. The loop has
/// nowhere to say that -- it only ever speaks when there is something to
/// offer, which is right for a check nobody asked for and useless for one
/// somebody is waiting on.
///
/// Unconditional, unlike the periodic look, which a dev build skips: asking
/// is the whole point of pressing it.
pub fn look_once(wake: impl Wake) {
    apart("look for an update now", move || async move {
        wake.wake(Update::Checked(looked().await));
    });
}

pub fn look_for_updates(every: std::time::Duration, wake: impl Wake) {
    apart("look for an update", move || async move {
        loop {
            match looked().await {
                Ok(Some(offer)) => {
                    println!("{} is available", offer.version);
                    wake.wake(Update::Updatable(offer));
                }
                Ok(None) => println!("already the newest build"),
                // A failed check is not a failed start, and not a reason to
                // stop looking: the client runs perfectly well on the build it
                // has, and the next look is two hours away.
                Err(error) => eprintln!("could not look for an update: {error}"),
            }
            tokio::time::sleep(every).await;
        }
    });
}

/// Fetches, checks and runs the installer the reader has just accepted.
///
/// Does not come back when it works: the installer replaces this executable,
/// so the process has to be gone before it can. Only a reader's press reaches
/// here -- nothing is downloaded by the looking.
///
/// Off the socket for the same reason as the looking, and it matters more
/// here: an offer this window could show and not install would be worse than
/// one it never made.
pub fn install_update(offer: crate::update::Offer, wake: impl Wake) {
    apart("install an update", move || async move {
        match fetched(&offer).await {
            Ok(bytes) => match crate::update::install(&bytes, &offer.version) {
                Ok(_) => unreachable!("the installer took over"),
                Err(error) => {
                    eprintln!("installing {}: {error}", offer.version);
                    wake.wake(Update::UpdateFailed(error));
                }
            },
            Err(error) => {
                eprintln!("fetching {}: {error}", offer.version);
                wake.wake(Update::UpdateFailed(error));
            }
        }
    });
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
    let rest = match RestClient::new(&server).map(std::sync::Arc::new) {
        Ok(rest) => rest,
        Err(error) => {
            wake.wake(Update::Failed(format!("{server}: {error}")));
            return;
        }
    };
    rest.set_token(AuthToken::Session(token.clone()));

    // Opened once for the life of the socket thread, which is the only thing
    // that fetches a picture. `None` when there is nowhere to put it: a cache
    // that cannot be opened is slow, not broken.
    let pictures = crate::filecache::shared();
    // How many pictures may be in flight at once.
    //
    // They used to go one at a time, because every arm of this loop awaits in
    // place -- so a send or an incoming event queued behind every face in the
    // channel. That is what this fixes, and it is worth being exact about
    // what it does *not*:
    //
    // **The pictures do not arrive sooner in total.** Measured on 29 faces
    // with a cold cache: 3204ms serialised, 3184ms six at a time. Each
    // request simply takes six times as long (120ms alone, 739ms with five
    // others), so the server hands over about one picture per 110ms however
    // many are asked for. Decoding is not the cost either -- 3ms a picture,
    // 101ms for all of them.
    //
    // What does change is how long any *one* picture waits: behind at most
    // five others rather than behind all twenty-eight. A face that appears
    // while a channel is still loading used to be last in a queue nobody
    // could jump.
    //
    // Six rather than all of them: a browser opens six connections a host for
    // the same reason, and the measurement says more would not help anyway.
    let fetching = std::sync::Arc::new(tokio::sync::Semaphore::new(WHILE_FETCHING));

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
    // Kept, before the window is told. The reader is a user like any other and
    // everything that names one reads the store: their own name over the
    // sidebar, their name taken back out of a group conversation's label, the
    // face beside what they said. None of it had anywhere to read from on a
    // store this window filled by itself -- so the strip said "signing in"
    // long after it had, which is the fallback for a name that is not there.
    //
    // Before the wake rather than after, because the wake is what rebuilds the
    // sidebar that reads it.
    if let Err(error) = store.upsert_users(std::slice::from_ref(&me)) {
        eprintln!("storing the reader: {error}");
    }
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

    // Held apart from the engine's for the work that runs on tasks of its
    // own and has to read or write the store while the loop does something
    // else.
    let kept = Arc::clone(&store);
    let engine = SyncEngine::new(store);
    let mut context = reader_context(me, engine.store());
    println!(
        "following {} threads, replies elsewhere stay quiet",
        context.followed_threads.len()
    );

    let (signals_tx, mut signals) = tokio::sync::mpsc::channel(1024);
    let handle = session.spawn(signals_tx);

    // Everybody the window has asked about, asked about again on a beat.
    let mut watched: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    let mut again = tokio::time::interval(STATUSES_AGAIN);
    again.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    // The first beat is immediate, and sign-in has just asked.
    again.tick().await;
    // Channels a membership pull was made for and did not bring back.
    let mut strangers: std::collections::HashSet<String> = std::collections::HashSet::new();

    loop {
        tokio::select! {
            _ = again.tick() => {
                let everybody: Vec<String> = watched.iter().cloned().collect();
                for slice in everybody.chunks(STATUSES_AT_ONCE) {
                    let (rest, wake, slice) = (rest.clone(), wake.clone(), slice.to_vec());
                    tokio::spawn(async move { who_is_around(&rest, &slice, &wake).await });
                }
            }
            // Sends and socket traffic on one thread: a send has to be able to
            // go out while the socket is quiet, and its echo has to be able to
            // arrive while a send is in flight.
            ask = inbox.recv() => {
                let Some(ask) = ask else { break };
                match ask {
                    Ask::Send { pending_post_id, channel_id, root_id, message, file_ids } => {
                        let request = matterless_core::model::NewPost {
                            channel_id: &channel_id,
                            message: &message,
                            root_id: &root_id,
                            pending_post_id: &pending_post_id,
                            file_ids: &file_ids,
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
                            // From this message down, which is what the app
                            // means by marking one unread: the channel goes
                            // back to being unread from here, not everywhere.
                            Action::Unread => {
                                rest.set_post_unread(&me_id, &post_id).await.map(|_| ())
                            }
                            // Answered in the window: none of these needs the
                            // server here. React opens a picker, Edit opens a
                            // box, Follow and Remind are asks of their own,
                            // and copying never leaves the machine.
                            Action::React
                            | Action::Edit
                            | Action::Forward
                            | Action::Thread
                            | Action::Follow
                            | Action::Remind
                            | Action::CopyText
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
                    Ask::Film { file_id } => {
                        let rest = rest.clone();
                        let wake = wake.clone();
                        tokio::spawn(async move {
                            let Some(path) = film_path(&file_id) else {
                                return wake.wake(Update::LookFailed {
                                    file_id,
                                    why: "nowhere to keep it".to_string(),
                                });
                            };
                            match rest.fetch_bytes(&format!("/files/{file_id}")).await {
                                Ok(Some((bytes, _))) => {
                                    let kept = path.clone();
                                    let written = off_the_loop(move || keep_film(&kept, &bytes)).await;
                                    match written {
                                        Some(Ok(())) => wake.wake(Update::Film { file_id, path }),
                                        Some(Err(why)) => wake.wake(Update::LookFailed {
                                            file_id,
                                            why: why.to_string(),
                                        }),
                                        None => {}
                                    }
                                }
                                Ok(None) => wake.wake(Update::LookFailed {
                                    file_id,
                                    why: "the server has no such file".to_string(),
                                }),
                                Err(error) => wake.wake(Update::LookFailed {
                                    file_id,
                                    why: error.to_string(),
                                }),
                            }
                        });
                    }
                    Ask::Download { file_id, name } => {
                        // A linked picture comes from its own host, without
                        // the session; everything else is this server's file.
                        let route = match file_id.starts_with("https://") {
                            true => file_id.clone(),
                            false => format!("/files/{file_id}"),
                        };
                        match fetch_route(&rest, &route).await {
                            Ok(Some((bytes, _))) => match keep(&downloads(), &name, &bytes) {
                                Ok(path) => println!("kept {}", path.display()),
                                Err(error) => eprintln!("keeping {name}: {error}"),
                            },
                            Ok(None) => eprintln!("{name}: the server sent nothing"),
                            Err(error) => eprintln!("fetching {name}: {error}"),
                        }
                    }
                    Ask::Attach {
                        channel_id,
                        root_id,
                        path,
                    } => match put(&rest, &channel_id, &path).await {
                        Some(file) => wake.wake(Update::Attached {
                            channel_id,
                            root_id,
                            file: Box::new(file),
                            path,
                        }),
                        None => wake.wake(Update::NotAttached {
                            channel_id,
                            root_id,
                            why: format!("{} did not go up", path.display()),
                            path,
                        }),
                    },
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
                    Ask::Unseen { channel_id } => {
                        // Marked from the newest message down, which needs the
                        // newest message: the store's copy may be a session
                        // old, so the server is asked for it.
                        match rest.posts(&channel_id, 1).await {
                            Ok(newest) => match newest.order.first() {
                                // An empty channel has nothing to be unread
                                // about, and saying so is not a failure.
                                None => println!("{channel_id} is empty"),
                                Some(post_id) => {
                                    match rest.set_post_unread(&me_id, post_id).await {
                                        Ok(member) => {
                                            let store = engine.store();
                                            if let Err(error) =
                                                store.upsert_channel_members(&[member])
                                            {
                                                eprintln!("storing the membership: {error}");
                                            }
                                            println!("{channel_id} marked unread");
                                            if let Ok((_, mode)) =
                                                membership(&rest, engine.store(), &me_id).await
                                            {
                                                wake.wake(Update::Membership(mode));
                                            }
                                        }
                                        Err(error) => {
                                            eprintln!("marking {channel_id} unread: {error}")
                                        }
                                    }
                                }
                            },
                            Err(error) => eprintln!("reading {channel_id}: {error}"),
                        }
                    }
                    Ask::Move {
                        channel_id,
                        team_id,
                        category_id,
                    } => {
                        match moved(&rest, &me_id, &team_id, &channel_id, &category_id).await {
                            Ok(()) => {
                                println!("moved {channel_id}");
                                if let Ok((_, mode)) =
                                    membership(&rest, engine.store(), &me_id).await
                                {
                                    wake.wake(Update::Membership(mode));
                                }
                            }
                            Err(error) => eprintln!("moving {channel_id}: {error}"),
                        }
                    }
                    Ask::Remind { post_id, when } => {
                        match rest.set_reminder(&me_id, &post_id, when).await {
                            Ok(()) => println!("reminder set on {post_id}"),
                            Err(error) => eprintln!("setting a reminder: {error}"),
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
                    // Several people at once, which the server keys a channel
                    // on: the same group asked for twice is the same channel,
                    // so this both creates and finds.
                    Ask::Group { mut user_ids } => {
                        // The reader is part of their own conversation. The
                        // server keys the channel on its whole membership, so
                        // leaving oneself out asks for a different one.
                        if !user_ids.contains(&me_id) {
                            user_ids.push(me_id.clone());
                        }
                        match rest.group_channel(&user_ids).await {
                            Ok(channel) => {
                                if let Err(error) =
                                    engine.store().upsert_channels(std::slice::from_ref(&channel))
                                {
                                    eprintln!("storing a new conversation: {error}");
                                }
                                if let Ok((_, mode)) =
                                    membership(&rest, engine.store(), &me_id).await
                                {
                                    wake.wake(Update::Membership(mode));
                                }
                                wake.wake(Update::Reached {
                                    channel_id: channel.id,
                                });
                            }
                            Err(error) => eprintln!("starting a conversation: {error}"),
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
                    Ask::Statuses { user_ids } => {
                        // Who they are, before whether they are here. Nothing
                        // in this window has ever written a user record except
                        // the switcher, when somebody is searched for by name
                        // -- so a store this window filled by itself labels
                        // every direct message and every author by their id,
                        // which is what a conversation looks like when nobody
                        // in it has a name.
                        //
                        // These ids and no others: the caller's set is the
                        // sidebar's conversations and whoever is on screen,
                        // which is exactly the set whose names are wanted. And
                        // only the ones the store has not met, so this costs
                        // one request on a fresh store and nothing after.
                        //
                        // On a task of its own: awaited here, it waited behind
                        // every refresh ahead of it in the queue and held up
                        // every one behind it, and a dot is the one thing on
                        // screen that is out of date the moment it is late.
                        watched.extend(user_ids.iter().cloned());
                        let (rest, store, wake) = (rest.clone(), Arc::clone(&kept), wake.clone());
                        tokio::spawn(async move {
                            met(&rest, &store, &user_ids, &wake).await;
                            who_is_around(&rest, &user_ids, &wake).await;
                        });
                    }
                    Ask::Typing {
                        channel_id,
                        root_id,
                    } => handle.typing(&channel_id, &root_id),
                    Ask::Person { user_id } => {
                        let (rest, store, wake) = (rest.clone(), Arc::clone(&kept), wake.clone());
                        tokio::spawn(async move {
                            match rest.users_by_ids(std::slice::from_ref(&user_id)).await {
                                Ok(people) if !people.is_empty() => {
                                    if let Err(error) = store.upsert_users(&people) {
                                        eprintln!("keeping a person: {error}");
                                        return;
                                    }
                                    wake.wake(Update::Person { user_id });
                                }
                                Ok(_) => {}
                                Err(error) => eprintln!("asking about a person: {error}"),
                            }
                        });
                    }
                    Ask::Thread {
                        root_id,
                        channel_id,
                    } => match rest.thread(&root_id).await {
                        Ok(list) => {
                            // A backfill: these are messages the reader has
                            // had for a while rather than messages arriving,
                            // so none of them may interrupt anybody.
                            let quiet =
                                SyncContext::new(context.me.clone(), ThreadMode::Collapsed);
                            match engine.apply_post_list(
                                &channel_id,
                                &list,
                                Arrival::Backfill,
                                &quiet,
                                0,
                            ) {
                                Ok(deltas) => {
                                    // Who wrote them, before the window draws
                                    // them: a thread in a channel never opened
                                    // is full of people never met.
                                    learn(&rest, engine.store(), &wrote(engine.store(), &deltas))
                                        .await;
                                    println!("thread {root_id}: {} posts", list.order.len());
                                    if !deltas.is_empty() {
                                        wake.wake(Update::Changed(deltas));
                                    }
                                }
                                Err(error) => eprintln!("reading thread {root_id}: {error}"),
                            }
                        }
                        Err(error) => eprintln!("fetching thread {root_id}: {error}"),
                    },
                    Ask::Refresh { channel_id } => {
                        if !known(engine.store(), &channel_id) {
                            continue;
                        }
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
                    Ask::ReadThread { root_id } => {
                        let store = engine.store();
                        let channel_id = store.thread_channel(&root_id).ok().flatten().unwrap_or_default();
                        // The route is scoped by team: the thread's channel's,
                        // or for a direct or group conversation, which belongs
                        // to none, any team this reader is on.
                        let team = store
                            .channel(&channel_id)
                            .ok()
                            .flatten()
                            .map(|channel| channel.team_id)
                            .filter(|team| !team.is_empty())
                            .or_else(|| {
                                store
                                    .teams()
                                    .ok()
                                    .and_then(|teams| teams.first().map(|team| team.id.clone()))
                            })
                            .unwrap_or_default();
                        // The server's clock, from the newest thing in the
                        // thread: it is compared against post times.
                        let read_to = store
                            .thread_replies(&root_id)
                            .unwrap_or_default()
                            .iter()
                            .map(|reply| reply.create_at)
                            .chain(store.post(&root_id).ok().flatten().map(|root| root.create_at))
                            .max()
                            .unwrap_or(0);
                        if let Err(error) = rest.mark_thread_read(&me_id, &team, &root_id, read_to).await {
                            eprintln!("marking thread {root_id} read: {error}");
                            continue;
                        }
                        if let Err(error) = store.set_thread_read(&root_id, read_to, 0, 0) {
                            eprintln!("keeping thread {root_id} read: {error}");
                        }
                        wake.wake(Update::Changed(vec![Delta::ThreadChanged { root_id, channel_id }]));
                    }
                    Ask::MarkRead { channel_id } => {
                        if !known(engine.store(), &channel_id) {
                            continue;
                        }
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
                        match engine
                            .store()
                            .mark_channel_viewed(&channel_id, &me_id, watermark)
                        {
                            // Said out loud, because the window rebuilds its
                            // sidebar from a delta and nothing else. The store
                            // was right and the badge stayed on screen: opening
                            // a channel cleared its counters here a moment
                            // after the sidebar had already been rebuilt from
                            // the old ones, and nothing asked again.
                            Ok(true) => {
                                let unread = engine
                                    .store()
                                    .unread(&channel_id, &me_id)
                                    .ok()
                                    .flatten()
                                    .unwrap_or_default();
                                wake.wake(Update::Changed(vec![Delta::UnreadChanged {
                                    channel_id,
                                    unread,
                                }]));
                            }
                            Ok(false) => {}
                            Err(error) => {
                                eprintln!("recording that {channel_id} was read: {error}")
                            }
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
                    Ask::Look {
                        file_id,
                        original,
                        linked,
                        within,
                    } => {
                        let route = if linked {
                            file_id.clone()
                        } else if original {
                            format!("/files/{file_id}")
                        } else {
                            format!("/files/{file_id}/preview")
                        };
                        let key = looked_key(&file_id, original, linked);
                        // A task of its own, like any picture: awaited here, a
                        // full-size original held every send and every
                        // refresh behind its download and then its decode.
                        let rest = rest.clone();
                        let wake = wake.clone();
                        tokio::spawn(async move {
                            match fetch_route(&rest, &route).await {
                                Ok(Some((bytes, content_type))) => {
                                    let fitted = off_the_loop(move || {
                                        let fitted = fit(&bytes, within.0.max(1), within.1.max(1))
                                            .map(|(width, height, rgba)| {
                                                let frames = reel_of(&bytes, width, height);
                                                (width, height, rgba, frames)
                                            });
                                        // Kept once it has decoded, so the next
                                        // time it is opened needs no network.
                                        if fitted.is_some()
                                            && let Some(held) = crate::filecache::looked()
                                        {
                                            held.write(&key, &bytes, &content_type);
                                        }
                                        (fitted, bytes.len(), kind_of(&bytes))
                                    })
                                    .await;
                                    match fitted {
                                        Some((Some((width, height, rgba, frames)), _, _)) => {
                                            wake.wake(Update::Looked {
                                                file_id,
                                                within,
                                                width,
                                                height,
                                                rgba,
                                                frames,
                                            })
                                        }
                                        Some((None, length, kind)) => wake.wake(Update::LookFailed {
                                            file_id,
                                            why: format!("{length} bytes of {kind} could not be decoded"),
                                        }),
                                        None => {}
                                    }
                                }
                                Ok(None) => wake.wake(Update::LookFailed {
                                    file_id,
                                    why: "the server has no such file".to_string(),
                                }),
                                Err(error) => wake.wake(Update::LookFailed {
                                    file_id,
                                    why: error.to_string(),
                                }),
                            }
                        });
                    }
                    Ask::Read { file_id } => {
                        let rest = rest.clone();
                        let wake = wake.clone();
                        tokio::spawn(async move {
                            let update = match rest.fetch_bytes(&format!("/files/{file_id}")).await {
                                Ok(Some((bytes, _))) => match text_of(&bytes) {
                                    Some(text) => Update::Text { file_id, text },
                                    None => Update::LookFailed {
                                        file_id,
                                        why: "it is not text".to_string(),
                                    },
                                },
                                Ok(None) => Update::LookFailed {
                                    file_id,
                                    why: "the server has no such file".to_string(),
                                },
                                Err(error) => Update::LookFailed {
                                    file_id,
                                    why: error.to_string(),
                                },
                            };
                            wake.wake(update);
                        });
                    }
                    // Behind everything the reader is actually looking at.
                    // One task for the whole list rather than one each: this
                    // is work nobody is waiting for, and a hundred tasks
                    // queued on a semaphore is a hundred tasks the runtime
                    // has to keep.
                    Ask::Warm { keys } => {
                        let rest = rest.clone();
                        let pictures = pictures.clone();
                        let fetching = fetching.clone();
                        tokio::spawn(async move {
                            let Some(held) = pictures else { return };
                            let mut got = 0usize;
                            for key in keys {
                                if held.read(&key).is_some() {
                                    continue;
                                }
                                let Some(route) = route_for(&key) else {
                                    continue;
                                };
                                // Only while nothing in front is waiting. A
                                // semaphore hands permits out in order and has
                                // no notion of priority, so the test is
                                // whether every permit is free -- which is
                                // exactly "no picture the reader asked for is
                                // in flight".
                                while fetching.available_permits() < WHILE_FETCHING {
                                    tokio::time::sleep(std::time::Duration::from_millis(250)).await;
                                }
                                let Ok(_room) = fetching.acquire().await else {
                                    return;
                                };
                                if let Ok(Some((bytes, kind))) = rest.fetch_bytes(&route).await {
                                    held.write(&key, &bytes, &kind);
                                    got += 1;
                                }
                            }
                            if got > 0 {
                                println!("{got} pictures are on the disk before anybody asked");
                            }
                        });
                    }
                    Ask::Fetch {
                        key,
                        width,
                        height,
                        shown,
                    } => {
                        // A video's first picture: made here, from the ends of
                        // the file, since the server keeps none.
                        if let Some(file_id) = key.strip_prefix("poster/") {
                            let file_id = file_id.to_string();
                            let (rest, wake, pictures, fetching) =
                                (rest.clone(), wake.clone(), pictures.clone(), fetching.clone());
                            tokio::spawn(async move {
                                let Ok(_room) = fetching.acquire().await else {
                                    return;
                                };
                                match poster_of(&rest, &file_id, width, height).await {
                                    Ok(png) => {
                                        off_the_loop(move || {
                                            take_in(&key, &png, "image/png", width, height, shown, pictures.as_deref(), &wake)
                                        })
                                        .await;
                                    }
                                    Err(why) => eprintln!("{key}: {why}"),
                                }
                            });
                            continue;
                        }
                        let Some(route) = route_for(&key) else {
                            continue;
                        };
                        // Off the loop, and several at once. Every arm here
                        // awaits in place, so a picture used to be fetched
                        // one at a time *and* to hold up sends and socket
                        // traffic behind it: 29 faces on opening a channel, a
                        // median of 120ms each, 3.2 seconds before the last
                        // one appeared. The disk cache never helped with that
                        // -- the bytes were already local on the second
                        // sighting; what was slow was the queue.
                        let rest = rest.clone();
                        let wake = wake.clone();
                        let pictures = pictures.clone();
                        let fetching = fetching.clone();
                        tokio::spawn(async move {
                            // Still read here, as well as on the shelf: a
                            // fetch asked for from anywhere else, or one the
                            // shelf passed along while another reader was
                            // writing the same file, should not go out twice.
                            let held = {
                                let (pictures, key, wake) = (pictures.clone(), key.clone(), wake.clone());
                                off_the_loop(move || {
                                    pictures
                                        .as_ref()
                                        .and_then(|held| held.read(on_disk(&key)))
                                        .is_some_and(|(bytes, _)| hand_over(&key, &bytes, width, height, &wake))
                                })
                                .await
                            };
                            if held == Some(true) {
                                return;
                            }
                                                        // Held until there is room to go out. Dropped at
                            // the end of the task, which is what bounds this.
                            let _room = match fetching.acquire().await {
                                Ok(room) => room,
                                // The semaphore is only closed when the whole
                                // session is going away.
                                Err(_) => return,
                            };
                            match fetch_route(&rest, &route).await {
                                Ok(Some((bytes, kind))) => {
                                    off_the_loop(move || {
                                        take_in(&key, &bytes, &kind, width, height, shown, pictures.as_deref(), &wake)
                                    })
                                    .await;
                                }
                                // A person with no picture is not an error, and
                                // a silent gap is the right drawing for one.
                                Ok(None) => {}
                                Err(error) => eprintln!("{key}: {error}"),
                            }
                        });
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
                    Signal::Event { event, .. } => {
                        // Every event by name, when asked for. Nothing else
                        // says what the socket actually delivered, so a report
                        // of "the window did not react" cannot be told from
                        // "the server never said so" -- which is exactly the
                        // fork a deleted message that stayed on screen turned
                        // on. `Event::Other` is the interesting one: it is an
                        // event this program has a name for and no answer to.
                        if std::env::var_os("MATTERLESS_EVENTS").is_some() {
                            // Said apart, because `Other` *carries* the name it
                            // could not answer -- so printing the name alone
                            // cannot tell "decoded and acted on" from "arrived
                            // and understood as nothing", which is the whole
                            // question when something on screen does not move.
                            match &event {
                                matterless_core::Event::Other { name } => {
                                    println!("socket: {name} -- nothing was made of it")
                                }
                                // With its timestamps, because whether a
                                // delete is honoured turns on them and the
                                // server does not fill them in the way the
                                // name suggests.
                                matterless_core::Event::PostDeleted(post) => println!(
                                    "socket: post_deleted {} delete_at {} update_at {}",
                                    post.id, post.delete_at, post.update_at
                                ),
                                event => println!("socket: {}", event.name()),
                            }
                        }
                        match engine.apply_event(&event, &context) {
                        Ok(deltas) if !deltas.is_empty() => {
                            // Who wrote them, before the window is told.
                            //
                            // Everything that names somebody reads the store,
                            // and a message can arrive from a person this
                            // window has never met -- the first message in a
                            // channel nobody has opened, or from somebody who
                            // joined today. The notification is raised from
                            // these deltas, so learning the name afterwards is
                            // too late: it had already said the id out loud.
                            learn(&rest, engine.store(), &wrote(engine.store(), &deltas)).await;
                            // Following or unfollowing a thread changes which
                            // replies may interrupt, so the set is re-read
                            // rather than left as it was at startup.
                            if deltas
                                .iter()
                                .any(|delta| matches!(delta, Delta::ThreadChanged { .. }))
                            {
                                context.followed_threads = followed(engine.store());
                            }
                            // Joined or left something, or a thread named a
                            // channel never stored. The same pull a leave
                            // makes: channels, categories, followed threads
                            // and their roots. Once per channel that the pull
                            // does not bring, so a thread somewhere the reader
                            // is not a member cannot ask on every reply.
                            let asked: Vec<String> = deltas
                                .iter()
                                .filter_map(|delta| match delta {
                                    Delta::MembershipChanged { channel_id }
                                        if !strangers.contains(channel_id) =>
                                    {
                                        Some(channel_id.clone())
                                    }
                                    _ => None,
                                })
                                .collect();
                            if !asked.is_empty() {
                                let me_id = context.me.id.clone();
                                match membership(&rest, engine.store(), &me_id).await {
                                    Ok((_, mode)) => {
                                        println!("membership changed: {} channel(s)", asked.len());
                                        context.followed_threads = followed(engine.store());
                                        wake.wake(Update::Membership(mode));
                                    }
                                    Err(error) => eprintln!("membership after a change: {error}"),
                                }
                                for channel_id in asked {
                                    if engine.store().channel(&channel_id).ok().flatten().is_none() {
                                        strangers.insert(channel_id);
                                    }
                                }
                            }
                            wake.wake(Update::Changed(deltas));
                        }
                        Ok(_) => {}
                        Err(error) => eprintln!("applying {}: {error}", event.name()),
                        }
                    }
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
/// What a picture is kept under on the disk: its name without a size on the
/// end. A face drawn large and the same face drawn small are two pictures in
/// the atlas and one file, fetched once.
pub fn on_disk(key: &str) -> &str {
    key.split_once('@').map_or(key, |(file, _)| file)
}

/// Where a linked picture's key leads. Filled by `linked_key` from URLs the
/// server measured, so a key only ever resolves to one of those.
static LINKS: std::sync::LazyLock<std::sync::Mutex<std::collections::HashMap<String, String>>> =
    std::sync::LazyLock::new(Default::default);

/// The key for a picture a message links to: a hash of its URL, because the
/// URL itself is often longer than a file name may be on this system.
pub fn linked_key(url: &str) -> String {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    url.hash(&mut hasher);
    let key = format!("link/{:016x}", hasher.finish());
    LINKS
        .lock()
        .unwrap_or_else(|held| held.into_inner())
        .insert(key.clone(), url.to_string());
    key
}

/// The most a linked picture may weigh. A GIF from the picker is well under a
/// megabyte; this is for the one that is somebody's screen recording.
const LINKED_BYTES: usize = 32 * 1024 * 1024;

/// A file's bytes as text: UTF-8, or UTF-16 when it opens with that byte
/// order mark, which is what PowerShell writes a redirected log in. Nothing
/// when it holds a zero byte, which text never does.
pub fn text_of(bytes: &[u8]) -> Option<String> {
    let units = |little: bool| -> String {
        let wide: Vec<u16> = bytes[2..]
            .as_chunks::<2>()
            .0
            .iter()
            .map(|&pair| match little {
                true => u16::from_le_bytes(pair),
                false => u16::from_be_bytes(pair),
            })
            .collect();
        String::from_utf16_lossy(&wide)
    };
    match bytes {
        [0xFF, 0xFE, ..] => Some(units(true)),
        [0xFE, 0xFF, ..] => Some(units(false)),
        _ if bytes.contains(&0) => None,
        [0xEF, 0xBB, 0xBF, rest @ ..] => Some(String::from_utf8_lossy(rest).into_owned()),
        _ => Some(String::from_utf8_lossy(bytes).into_owned()),
    }
}

/// Fetches whatever a route names: this server's for a path, the picture's own
/// host -- without the session -- for a linked one.
async fn fetch_route(
    rest: &RestClient,
    route: &str,
) -> matterless_core::Result<Option<(Vec<u8>, String)>> {
    match route.starts_with("https://") {
        true => rest.fetch_outside(route, LINKED_BYTES).await,
        false => rest.fetch_bytes(route).await,
    }
}

pub fn route_for(key: &str) -> Option<String> {
    let path = key.split('?').next()?;
    let (kind, id) = path.trim_start_matches('/').split_once('/')?;
    if id.is_empty() || !id.chars().all(|c| c.is_ascii_alphanumeric()) {
        return None;
    }
    match kind {
        // Somewhere else, and only where `linked_key` said.
        "link" => LINKS
            .lock()
            .unwrap_or_else(|held| held.into_inner())
            .get(path)
            .cloned(),
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

/// How many pictures may be on the wire at once.
const WHILE_FETCHING: usize = 6;

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
        // A delete counts the same way an arrival does. Without this a
        // message deleted while its thread was open stayed on the pane: the
        // channel behind it was redrawn and the pane was never asked.
        Delta::PostTombstoned {
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

    /// UTF-8 with or without its mark, and UTF-16 either way round, which is
    /// what a PowerShell redirect writes; and nothing for a file with a zero
    /// byte in it, which is not text.
    #[test]
    fn a_text_file_is_read_in_the_encoding_it_says() {
        assert_eq!(super::text_of(b"plain").as_deref(), Some("plain"));
        assert_eq!(
            super::text_of(&[0xEF, 0xBB, 0xBF, b'h', b'i']).as_deref(),
            Some("hi")
        );
        assert_eq!(
            super::text_of(&[0xFF, 0xFE, b'h', 0, b'i', 0]).as_deref(),
            Some("hi")
        );
        assert_eq!(
            super::text_of(&[0xFE, 0xFF, 0, b'h', 0, b'i']).as_deref(),
            Some("hi")
        );
        assert_eq!(super::text_of(&[b'P', b'K', 3, 4, 0, 0]), None);
    }

    /// A face drawn large is its own picture in the atlas and the same file on
    /// the disk and on the server: asked for at another size, it is neither
    /// downloaded nor kept a second time.
    #[test]
    fn a_face_at_another_size_is_the_same_file() {
        let small = crate::stream::avatar_key("u1abc", 7);
        let large = crate::stream::avatar_key_sized("u1abc", 7, 120);
        assert_ne!(small, large, "the atlas would hand back the small one");
        assert_eq!(super::on_disk(&large), small);
        assert_eq!(super::on_disk(&small), small);
        assert_eq!(super::route_for(&large), super::route_for(&small));
        assert!(super::route_for(&large).is_some());
    }

    /// A moving picture of `count` frames at `wide` by `tall`, as GIF bytes.
    fn a_gif(count: u32, wide: u32, tall: u32) -> Vec<u8> {
        let mut bytes = Vec::new();
        {
            let mut encoder = image::codecs::gif::GifEncoder::new(&mut bytes);
            for at in 0..count {
                let frame = image::RgbaImage::from_fn(wide, tall, |x, y| {
                    image::Rgba([(x + at * 7) as u8, (y * 3) as u8, (x ^ y) as u8, 255])
                });
                encoder
                    .encode_frame(image::Frame::new(frame))
                    .expect("a frame");
            }
        }
        bytes
    }

    /// A GIF opened in the viewer comes with every frame at the size it is
    /// shown, so it plays there; a still picture comes with none.
    #[test]
    fn a_gif_in_the_viewer_brings_its_frames() {
        let gif = a_gif(4, 120, 80);
        let frames = super::reel_of(&gif, 60, 40).expect("it moves");
        assert_eq!(frames.len(), 4);
        assert!(frames.iter().all(|(rgba, _)| rgba.len() == 60 * 40 * 4));
        assert!(
            super::reel_of(&a_gif(1, 120, 80), 60, 40).is_none(),
            "one frame is a still"
        );
    }

    /// A linked picture is kept under its URL's hash, the same key the message
    /// list fetches it under -- not under a route of this server's.
    #[test]
    fn a_linked_picture_is_kept_under_its_link() {
        let url = "https://media1.giphy.com/media/JfHY/200.gif?cid=1";
        assert_eq!(super::looked_key(url, false, true), super::linked_key(url));
        assert_eq!(super::looked_key("f1", true, false), "file/f1");
        assert_eq!(super::looked_key("f1", false, false), "preview/f1");
    }

    /// The frames of a GIF are scaled by the method now, for the same reason a
    /// still is. Same filter, same pixels.
    #[test]
    fn a_gif_frame_is_scaled_by_the_filter_it_replaced() {
        use image::AnimationDecoder;
        let gif = a_gif(3, 96, 64);
        let ours = super::frames_of(&gif, 24, 16).expect("it moves");
        let before: Vec<Vec<u8>> = image::codecs::gif::GifDecoder::new(std::io::Cursor::new(&gif))
            .expect("a gif")
            .into_frames()
            .map(|frame| {
                image::imageops::thumbnail(&frame.expect("a frame").into_buffer(), 24, 16)
                    .into_raw()
            })
            .collect();
        assert_eq!(ours.len(), before.len());
        for ((pixels, _), expected) in ours.iter().zip(&before) {
            assert_eq!(pixels, expected);
        }
    }

    /// The scale moved from `imageops::thumbnail` to the method so the dev
    /// build would run it optimised. It must still be the same filter: the
    /// same pixels out, whatever the picture was stored as.
    #[test]
    fn the_scale_is_the_filter_it_replaced() {
        let mut rgba = image::RgbaImage::new(128, 128);
        for (x, y, pixel) in rgba.enumerate_pixels_mut() {
            *pixel = image::Rgba([
                (x * 2) as u8,
                (y * 2) as u8,
                ((x ^ y) * 3) as u8,
                (x + y) as u8,
            ]);
        }
        let rgb = image::DynamicImage::ImageRgba8(rgba.clone()).to_rgb8();
        for stored in [
            image::DynamicImage::ImageRgba8(rgba),
            image::DynamicImage::ImageRgb8(rgb),
        ] {
            let mut png = Vec::new();
            stored
                .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
                .expect("encoded");
            let (wide, tall, ours) = super::decode(&png, 28, 28).expect("decoded");
            let before = image::imageops::thumbnail(&stored.to_rgba8(), 28, 28);
            assert_eq!((wide, tall), (28, 28));
            assert_eq!(ours, before.into_raw(), "{:?}", stored.color());
        }
    }

    #[test]
    fn a_face_key_gives_up_its_version() {
        assert_eq!(
            super::stem_of("avatar/abc?v=17").as_deref(),
            Some("avatar/abc?v=")
        );
        assert_eq!(
            super::stem_of("avatar/abc?v=0").as_deref(),
            Some("avatar/abc?v=")
        );
        // Nothing else is versioned this way.
        assert_eq!(super::stem_of("emoji/abc"), None);
        assert_eq!(super::stem_of("team/abc?v=3"), None);
        assert_eq!(super::stem_of("mini/file-1/thumbnail"), None);
    }

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

    /// A delete reaches the thread pane it happened in.
    ///
    /// `touches_thread` listed arrivals, thread changes and reactions and not
    /// tombstones, so a reply deleted while its thread was open stayed on the
    /// pane: the channel behind it was redrawn and the pane was never asked.
    /// The root counts too -- deleting the message a pane hangs from is very
    /// much something that pane is showing.
    #[test]
    fn a_delete_reaches_the_thread_it_happened_in() {
        let gone = |post_id: &str, root_id: &str| Delta::PostTombstoned {
            post_id: post_id.into(),
            channel_id: "c1".into(),
            root_id: root_id.into(),
        };
        assert!(touches_thread(&[gone("reply", "root")], "root"));
        assert!(touches_thread(&[gone("root", "")], "root"));
        assert!(
            !touches_thread(&[gone("reply", "elsewhere")], "root"),
            "another thread's delete redrew this pane"
        );
        assert!(!touches_thread(&[gone("loose", "")], "root"));
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

/// Every frame of a picture that moves, at the size the atlas will hold them.
///
/// `None` for anything with one frame in it, which is almost everything: an
/// avatar, a thumbnail, a still emoji. Only a GIF is looked at, because a GIF
/// is what a team makes a moving emoji out of -- the other formats this build
/// can read are animated so rarely that opening every one of them to find out
/// would cost more than it saves.
///
/// Scaled the same way `decode` scales, and frame by frame rather than as a
/// whole: the frames go into one slot in the atlas, one after another, and a
/// slot is one size.
fn frames_of(bytes: &[u8], width: u32, height: u32) -> Option<Vec<(Vec<u8>, std::time::Duration)>> {
    use image::AnimationDecoder;
    if kind_of(bytes) != "gif" {
        return None;
    }
    let decoder = image::codecs::gif::GifDecoder::new(std::io::Cursor::new(bytes)).ok()?;
    let mut frames = Vec::new();
    for frame in decoder.into_frames() {
        let Ok(frame) = frame else {
            // Whatever was read before the file went wrong is still a loop,
            // and a short loop is better than a still picture.
            break;
        };
        let (numerator, denominator) = frame.delay().numer_denom_ms();
        let delay = std::time::Duration::from_micros(
            (u64::from(numerator) * 1000) / u64::from(denominator.max(1)).max(1),
        );
        let mut rgba = frame.into_buffer();
        let wanted = (
            width.clamp(1, MAX_SIDE).min(rgba.width()),
            height.clamp(1, MAX_SIDE).min(rgba.height()),
        );
        if (rgba.width(), rgba.height()) != wanted {
            // The method, not `imageops::thumbnail`, for the reason `decode`
            // gives -- and here it is paid once a frame.
            rgba = image::DynamicImage::ImageRgba8(rgba)
                .thumbnail_exact(wanted.0, wanted.1)
                .into_rgba8();
        }
        frames.push((rgba.into_raw(), delay));
        // A loop longer than this is somebody's video, not an emoji, and the
        // frames are held in memory for as long as the picture is on screen.
        if frames.len() >= 240 {
            break;
        }
    }
    (frames.len() > 1).then_some(frames)
}

/// Decodes a picture to straight RGBA at its own size.
///
/// Scaled down to what the atlas can hold rather than refused: an avatar comes
/// back at whatever size the server keeps, and a face that would not fit is
/// better small than missing.
/// Runs decoding on a thread made for blocking, and waits for it there.
///
/// Not on the socket thread's own: it is one thread for everything, and a task
/// that decodes without awaiting holds it for as long as that takes. A 144-frame
/// GIF and a 104-frame one did exactly that while the reader was opening a
/// channel with nothing kept locally, and its rows arrived seconds later,
/// behind the frames and behind every refresh that had queued up meanwhile.
async fn off_the_loop<T: Send + 'static>(work: impl FnOnce() -> T + Send + 'static) -> Option<T> {
    tokio::task::spawn_blocking(work).await.ok()
}

/// A picture that has just been fetched: decoded, kept, and handed over.
#[allow(clippy::too_many_arguments)]
fn take_in(
    key: &str,
    bytes: &[u8],
    kind: &str,
    width: u32,
    height: u32,
    shown: Option<u64>,
    pictures: Option<&crate::filecache::FileCache>,
    wake: &impl Wake,
) {
    let Some(decoded) = decode(bytes, width, height) else {
        // Named by what actually came back: a picture this build has no
        // decoder for and a picture that is really an error page fail the same
        // way, and "could not be decoded" said neither.
        return eprintln!(
            "{key}: could not be decoded, {} bytes of {}",
            bytes.len(),
            kind_of(bytes)
        );
    };
    // Kept only once it has decoded: bytes this build cannot read are worth
    // nothing on the next start either.
    if let Some(held) = pictures {
        held.write(on_disk(key), bytes, kind);
    }
    // The same picture under a version the cache had not seen: kept, so the
    // next start finds it under the key it will ask with, but not drawn again
    // over itself.
    if shown == Some(fingerprint(bytes)) {
        return;
    }
    deliver(key, bytes, width, height, decoded, wake);
}

/// Decodes a picture and wakes the window with it. False when the bytes are
/// not a picture this build can read.
///
/// Shared by the two readers -- the socket thread and the shelf -- so that a
/// GIF plays whichever of them got to it.
fn hand_over(key: &str, bytes: &[u8], width: u32, height: u32, wake: &impl Wake) -> bool {
    let Some(decoded) = decode(bytes, width, height) else {
        return false;
    };
    deliver(key, bytes, width, height, decoded, wake);
    true
}

/// Wakes the window with a decoded picture, and with its frames if it moves.
fn deliver(
    key: &str,
    bytes: &[u8],
    width: u32,
    height: u32,
    (wide, tall, rgba): (u32, u32, Vec<u8>),
    wake: &impl Wake,
) {
    let moves = frames_of(bytes, width, height);
    // The still frame first, so a picture is on screen whether or not anything
    // plays it.
    wake.wake(Update::Picture {
        key: key.to_string(),
        width: wide,
        height: tall,
        rgba,
    });
    if let Some(frames) = moves {
        wake.wake(Update::Moving {
            key: key.to_string(),
            width: wide,
            height: tall,
            frames,
        });
    }
}

/// How many pictures are read off the disk at once.
///
/// Threads of the window's own rather than the socket thread's: a picture
/// already on the disk has no business queueing behind the network. Measured at
/// a start with every face already cached, before this existed: 216ms before
/// the socket loop so much as looked at the queue -- it was inside `rest.me()`
/// -- and then 91ms of decoding serialised behind it, because a current-thread
/// runtime decodes one picture at a time. 312ms to put up a face that was
/// sitting in a file the whole time.
///
/// Decoding is the work, not the disk: 1194us to decode a face against 281us
/// to read it. So the count is about cores. Forty-nine faces and emoji, read
/// and decoded, wall clock: 26ms on three, 13ms on six, 9ms on twelve -- and
/// twelve spends 133ms of thread time to save those 4ms, where six spends 89ms.
/// Six, then: about a frame, without taking the machine over.
const READERS: usize = 6;

/// The pictures already on this disk.
///
/// Asked first for everything: a hit never touches the socket thread, and a
/// miss is passed along to it, so the window has one call to make either way.
pub struct Shelf {
    wants: std::sync::mpsc::Sender<(String, u32, u32)>,
    looks: std::sync::mpsc::Sender<(String, bool, bool, (u32, u32))>,
}

impl Shelf {
    /// Queues a picture to be read, decoded and handed to the window.
    pub fn want(&self, key: String, width: u32, height: u32) -> bool {
        self.wants.send((key, width, height)).is_ok()
    }

    /// Queues a picture opened in the viewer: off the disk when it is there,
    /// from the server when it is not.
    pub fn look(&self, file_id: String, original: bool, linked: bool, within: (u32, u32)) -> bool {
        self.looks.send((file_id, original, linked, within)).is_ok()
    }
}

/// What an opened picture is kept under on the disk: the rendition and the id,
/// named as the routes name them -- and a linked one by its URL's hash.
fn looked_key(file_id: &str, original: bool, linked: bool) -> String {
    match (linked, original) {
        (true, _) => linked_key(file_id),
        (false, true) => format!("file/{file_id}"),
        (false, false) => format!("preview/{file_id}"),
    }
}

/// The most the frames of one picture in the viewer may take. A GIF from the
/// picker is a few megabytes of frames; a screen recording at window size is
/// hundreds, and past this its loop is cut short rather than the memory spent.
const REEL_BYTES: usize = 192 * 1024 * 1024;

/// Every frame of a moving picture at exactly `width` by `height`, for the
/// viewer. `None` for anything with one frame, which is nearly everything.
fn reel_of(bytes: &[u8], width: u32, height: u32) -> Option<Vec<(Vec<u8>, std::time::Duration)>> {
    use image::AnimationDecoder;
    if kind_of(bytes) != "gif" {
        return None;
    }
    let decoder = image::codecs::gif::GifDecoder::new(std::io::Cursor::new(bytes)).ok()?;
    let mut frames = Vec::new();
    let mut held = 0usize;
    for frame in decoder.into_frames() {
        let Ok(frame) = frame else { break };
        let (numerator, denominator) = frame.delay().numer_denom_ms();
        let delay = std::time::Duration::from_micros(
            (u64::from(numerator) * 1000) / u64::from(denominator.max(1)).max(1),
        );
        let mut rgba = frame.into_buffer();
        if (rgba.width(), rgba.height()) != (width, height) {
            rgba = image::imageops::thumbnail(&rgba, width.max(1), height.max(1));
        }
        held += rgba.as_raw().len();
        frames.push((rgba.into_raw(), delay));
        if held > REEL_BYTES || frames.len() >= 240 {
            break;
        }
    }
    (frames.len() > 1).then_some(frames)
}

/// Asks the server who of these is around, and tells the window.
async fn who_is_around(rest: &RestClient, user_ids: &[String], wake: &impl Wake) {
    match rest.statuses_by_ids(user_ids).await {
        Ok(found) => {
            println!(
                "asked about {} people, {} answered",
                user_ids.len(),
                found.len()
            );
            wake.wake(Update::Statuses(
                found
                    .into_iter()
                    .map(|status| (status.user_id, status.status, status.last_activity_at))
                    .collect(),
            ));
        }
        Err(error) => eprintln!("asking who is around: {error}"),
    }
}

/// How often everybody asked about is asked about again.
///
/// A status is volatile and the socket does not carry everybody's changes,
/// so it is asked for rather than waited for: once for everybody at sign-in,
/// then again on this beat. The official web client polls statuses every
/// minute too.
const STATUSES_AGAIN: std::time::Duration = std::time::Duration::from_secs(60);

/// How many people one status request names.
///
/// Everybody is asked about at sign-in and again on the beat; a hundred people
/// is one request, and a server with thousands gets several rather than one it
/// might refuse for its size.
pub const STATUSES_AT_ONCE: usize = 200;

/// Enough to tell whether two pictures are the same bytes.
fn fingerprint(bytes: &[u8]) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    bytes.hash(&mut hasher);
    hasher.finish()
}

/// A face's key without the version on the end, for finding another of the
/// same person. Only faces: nothing else is versioned this way.
fn stem_of(key: &str) -> Option<String> {
    let (path, _) = key.split_once("?v=")?;
    path.starts_with("avatar/").then(|| format!("{path}?v="))
}

/// Opens the disk cache on the window's behalf.
///
/// The link is held so a miss can be passed along; nothing else here talks to
/// the server.
pub fn shelf(link: Link, wake: impl Wake) -> Shelf {
    let (wants, asked) = std::sync::mpsc::channel::<(String, u32, u32)>();
    let asked = Arc::new(std::sync::Mutex::new(asked));
    let pictures = crate::filecache::shared();
    for _ in 0..READERS {
        let asked = asked.clone();
        let pictures = pictures.clone();
        let wake = wake.clone();
        let link = link.clone();
        std::thread::spawn(move || {
            loop {
                // The lock is held only long enough to take one, so the three
                // threads share the queue rather than each owning a part of it:
                // a thread reading a slow picture must not strand the ones
                // behind it.
                let taken = {
                    let taking = asked.lock().unwrap_or_else(|held| held.into_inner());
                    taking.recv()
                };
                let Ok((key, width, height)) = taken else {
                    return;
                };
                let read = pictures.as_ref().and_then(|held| held.read(on_disk(&key)));
                match read {
                    Some((bytes, _)) if hand_over(&key, &bytes, width, height, &wake) => {}
                    // Either nothing on the disk or nothing readable there.
                    _ => {
                        // Another version of the same face, if there is one.
                        // Shown straight away rather than left blank: what is
                        // held is the right person, and the version in the key
                        // decides nothing here -- the ask carries a zero
                        // whenever the store has not heard of them yet, which
                        // is not the same as the face being out of date.
                        let shown = stem_of(&key)
                            .and_then(|stem| {
                                pictures.as_ref().and_then(|held| held.read_like(&stem))
                            })
                            .filter(|(bytes, _)| hand_over(&key, bytes, width, height, &wake))
                            .map(|(bytes, _)| fingerprint(&bytes));
                        // And asked for all the same, because whether a new
                        // picture has been set is the server's to answer. What
                        // comes back replaces the face only if it differs.
                        link.send(Ask::Fetch {
                            key,
                            width,
                            height,
                            shown,
                        });
                    }
                }
            }
        });
    }
    // The viewer's reader, one of its own: a two-thousand-pixel picture takes
    // tens of milliseconds to decode, and the faces behind it must not wait.
    let (looks, looked) = std::sync::mpsc::channel::<(String, bool, bool, (u32, u32))>();
    std::thread::spawn(move || {
        let held = crate::filecache::looked();
        for (file_id, original, linked, within) in looked {
            let fitted = held
                .as_ref()
                .and_then(|held| held.read(&looked_key(&file_id, original, linked)))
                .and_then(|(bytes, _)| {
                    let (width, height, rgba) = fit(&bytes, within.0.max(1), within.1.max(1))?;
                    Some((width, height, rgba, reel_of(&bytes, width, height)))
                });
            match fitted {
                Some((width, height, rgba, frames)) => wake.wake(Update::Looked {
                    file_id,
                    within,
                    width,
                    height,
                    rgba,
                    frames,
                }),
                None => {
                    link.send(Ask::Look {
                        file_id,
                        original,
                        linked,
                        within,
                    });
                }
            }
        }
    });
    Shelf { wants, looks }
}

fn decode(bytes: &[u8], width: u32, height: u32) -> Option<(u32, u32, Vec<u8>)> {
    let mut decoded = image::load_from_memory(bytes).ok()?;
    // Never enlarged: a picture smaller than its box stays its own size and the
    // sampler stretches it, which costs nothing and keeps the atlas small.
    let wanted = (
        width.clamp(1, MAX_SIDE).min(decoded.width()),
        height.clamp(1, MAX_SIDE).min(decoded.height()),
    );
    if (decoded.width(), decoded.height()) != wanted {
        // A box filter on the way in, which is a better downscale than the
        // bilinear one the sampler would do on the way out -- and it is done
        // once rather than every frame.
        //
        // The method rather than `imageops::thumbnail`: that one is generic, so
        // it is compiled into this crate, which the dev build leaves
        // unoptimised -- 1057us to scale a 128px face to 28px, against 28us in
        // a release build. This one is compiled inside `image`, which the dev
        // build does optimise. Scaled before the conversion, too, so there are
        // fewer pixels to convert.
        decoded = decoded.thumbnail_exact(wanted.0, wanted.1);
    }
    let rgba = decoded.to_rgba8();
    Some((rgba.width(), rgba.height(), rgba.into_raw()))
}

/// One picture, decoded and scaled down to fit a box of `width` by `height`.
///
/// Aspect kept, and never enlarged: a picture smaller than the window stays
/// its own size rather than being blown up into a soft one. Unlike `decode`
/// there is no small ceiling, because this is the picture a reader has asked
/// to look at -- the window's own size is the ceiling, and the device's texture
/// limit is checked where it is uploaded.
fn fit(bytes: &[u8], width: u32, height: u32) -> Option<(u32, u32, Vec<u8>)> {
    let decoded = image::load_from_memory(bytes).ok()?;
    let rgba = decoded.to_rgba8();
    let (was, tall) = (rgba.width().max(1), rgba.height().max(1));
    let scale = (width as f32 / was as f32)
        .min(height as f32 / tall as f32)
        .min(1.0);
    if scale >= 1.0 {
        return Some((rgba.width(), rgba.height(), rgba.into_raw()));
    }
    let wanted = (
        ((was as f32 * scale) as u32).max(1),
        ((tall as f32 * scale) as u32).max(1),
    );
    let smaller = image::imageops::thumbnail(&rgba, wanted.0, wanted.1);
    Some((smaller.width(), smaller.height(), smaller.into_raw()))
}

#[cfg(test)]
mod fitting {
    /// A picture larger than the window comes back smaller, with its shape
    /// kept; one smaller than the window is left alone rather than blown up.
    #[test]
    fn a_picture_is_scaled_to_fit_and_never_past_its_own_size() {
        let wide = image::RgbaImage::from_pixel(400, 100, image::Rgba([255, 0, 0, 255]));
        let mut bytes = std::io::Cursor::new(Vec::new());
        image::DynamicImage::ImageRgba8(wide)
            .write_to(&mut bytes, image::ImageFormat::Png)
            .expect("encodes");
        let bytes = bytes.into_inner();

        let (width, height, rgba) = super::fit(&bytes, 200, 200).expect("decodes");
        assert_eq!((width, height), (200, 50), "the shape is kept");
        assert_eq!(rgba.len() as u32, width * height * 4);

        let (width, height, _) = super::fit(&bytes, 4000, 4000).expect("decodes");
        assert_eq!((width, height), (400, 100), "never enlarged");
    }
}

/// The mini preview a post carries, decoded.
///
/// Base64 in the message metadata rather than a file to fetch: the server puts
/// about a kilobyte of JPEG on every image attachment precisely so a client can
/// show something before it asks for anything. Drawn at whatever size the box
/// is, which is why nothing is resized here -- it is sixteen pixels across and
/// meant to be stretched.
pub fn mini(encoded: &str) -> Option<(u32, u32, Vec<u8>)> {
    use base64::Engine;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(encoded)
        .ok()?;
    let rgba = image::load_from_memory(&bytes).ok()?.to_rgba8();
    Some((rgba.width(), rgba.height(), rgba.into_raw()))
}

#[cfg(test)]
mod minis {
    /// A 2x2 JPEG, the shape a mini preview arrives in.
    const TINY: &str = concat!(
        "/9j/4AAQSkZJRgABAQAAAQABAAD/2wBDAA0JCgsKCA0LCgsODg0PEyAVExISEyccHhcgLikxMC4pLS",
        "wzOko+MzZGNywtQFdBRkxOUlNSMj5aYVpQYEpRUk//2wBDAQ4ODhMREyYVFSZPNS01T09PT09PT09P",
        "T09PT09PT09PT09PT09PT09PT09PT09PT09PT09PT09PT09PT09PT0//wAARCAACAAIDASIAAhEBAx",
        "EB/8QAHwAAAQUBAQEBAQEAAAAAAAAAAAECAwQFBgcICQoL/8QAtRAAAgEDAwIEAwUFBAQAAAF9AQID",
        "AAQRBRIhMUEGE1FhByJxFDKBkaEII0KxwRVS0fAkM2JyggkKFhcYGRolJicoKSo0NTY3ODk6Q0RFRk",
        "dISUpTVFVWV1hZWmNkZWZnaGlqc3R1dnd4eXqDhIWGh4iJipKTlJWWl5iZmqKjpKWmp6ipqrKztLW2",
        "t7i5usLDxMXGx8jJytLT1NXW19jZ2uHi4+Tl5ufo6erx8vP09fb3+Pn6/8QAHwEAAwEBAQEBAQEBAQ",
        "AAAAAAAAECAwQFBgcICQoL/8QAtREAAgECBAQDBAcFBAQAAQJ3AAECAxEEBSExBhJBUQdhcRMiMoEI",
        "FEKRobHBCSMzUvAVYnLRChYkNOEl8RcYGRomJygpKjU2Nzg5OkNERUZHSElKU1RVVldYWVpjZGVmZ2",
        "hpanN0dXZ3eHl6goOEhYaHiImKkpOUlZaXmJmaoqOkpaanqKmqsrO0tba3uLm6wsPExcbHyMnK0tPU",
        "1dbX2Nna4uPk5ebn6Onq8vP09fb3+Pn6/9oADAMBAAIRAxEAPwDm3dg7YZup70UUV2Q+FHoT+Jn/2Q",
        "==",
    );

    /// A mini preview is read from the message rather than fetched, so a
    /// broken one has to fail quietly -- there is no request to report.
    #[test]
    fn a_mini_preview_decodes_and_rubbish_does_not() {
        let (width, height, rgba) = super::mini(TINY).expect("a tiny jpeg decodes");
        assert_eq!((width, height), (2, 2));
        assert_eq!(rgba.len(), 2 * 2 * 4);
        assert!(super::mini("not base64 at all !!").is_none());
        assert!(super::mini("").is_none());
    }
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
    /// A linked picture's key leads to the URL it was made from, and a key of
    /// that shape nobody made leads nowhere.
    #[test]
    fn a_linked_key_leads_only_where_it_was_made() {
        let url = "https://media0.giphy.com/media/H1YuBxdnlHITC/200.gif?cid=abc&ct=g";
        let key = super::linked_key(url);
        assert!(key.len() < 40, "a file name, not a URL: {key}");
        assert_eq!(route_for(&key).as_deref(), Some(url));
        assert_eq!(route_for("link/0123456789abcdef"), None);
    }

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

#[cfg(test)]
mod pictures {
    use super::{decode, kind_of};

    /// A 2x2 lossless WebP.
    ///
    /// Inline rather than a file, because what it is testing is which decoders
    /// this binary was *built* with -- and that is a line in `Cargo.toml`, not
    /// anything a fixture on disk would notice.
    const WEBP: &[u8] = &[
        0x52, 0x49, 0x46, 0x46, 0x20, 0x00, 0x00, 0x00, 0x57, 0x45, 0x42, 0x50, 0x56, 0x50, 0x38,
        0x4C, 0x13, 0x00, 0x00, 0x00, 0x2F, 0x01, 0x40, 0x00, 0x10, 0x0F, 0x10, 0xFB, 0x3F, 0xFF,
        0x0F, 0xFC, 0x8F, 0x0A, 0x23, 0x10, 0xD1, 0xFF, 0x10, 0x00,
    ];

    /// WebP is a format this server actually serves, so it has to be one this
    /// build can read.
    ///
    /// Every custom emoji on it comes back as WebP, and `image` was built
    /// without that decoder: the picture was fetched, refused, and the reader
    /// saw the blank room reserved for it. Nothing failed loudly -- the name
    /// resolved, the row was the right height, and the emoji was simply not
    /// there.
    #[test]
    fn a_webp_picture_can_be_read() {
        assert_eq!(kind_of(WEBP), "webp", "the fixture is what it claims to be");
        let (width, height, rgba) = decode(WEBP, 64, 64).expect("webp decodes");
        // Never enlarged: 2x2 asked for at 64 stays 2x2.
        assert_eq!((width, height), (2, 2));
        assert_eq!(rgba.len(), 2 * 2 * 4);
    }

    /// And a picture that is really an error page is still refused.
    #[test]
    fn markup_is_not_a_picture() {
        assert_eq!(kind_of(b"<html>"), "markup, not a picture");
        assert!(decode(b"<html><body>no</body></html>", 16, 16).is_none());
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
                        root_id: post.root_id,
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
        // Nothing was searched for, so a preview is the opening of the
        // message, which is what these lists have always shown.
        found: crate::listing::found_for(store, posts, me_id, &[]),
    }
}

/// Pulls every channel and membership this reader has, into the store.
///
/// Teams first, because a channel is asked for per team and a direct message
/// comes back under every one of them -- so they are deduplicated by id or the
/// same conversation is stored several times over.
/// Asks the release what the newest build is.
///
/// Its own client rather than the one the server session uses: this is a
/// request to a release host, and it has no business carrying a Mattermost
/// token.
async fn looked() -> Result<Option<crate::update::Offer>, String> {
    let Some(target) = crate::update::target() else {
        return Err("no manifest key for this platform".to_string());
    };
    let client = reqwest::Client::builder()
        .user_agent(concat!("MatterLess/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|error| error.to_string())?;
    let body = client
        .get(crate::update::ENDPOINT)
        .send()
        .await
        .map_err(|error| error.to_string())?
        .error_for_status()
        .map_err(|error| error.to_string())?
        .text()
        .await
        .map_err(|error| error.to_string())?;
    let manifest: crate::update::Manifest = serde_json::from_str(&body)
        .map_err(|error| format!("the manifest will not parse: {error}"))?;
    Ok(crate::update::offered(
        &manifest,
        &target,
        crate::update::running(),
    ))
}

/// Fetches the installer and checks it against the key before handing it back.
///
/// The check is here rather than at the call site so there is no path that
/// reaches the bytes without it: unverified bytes never leave this function.
async fn fetched(offer: &crate::update::Offer) -> Result<Vec<u8>, String> {
    let client = reqwest::Client::builder()
        .user_agent(concat!("MatterLess/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|error| error.to_string())?;
    let bytes = client
        .get(&offer.url)
        .send()
        .await
        .map_err(|error| error.to_string())?
        .error_for_status()
        .map_err(|error| error.to_string())?
        .bytes()
        .await
        .map_err(|error| error.to_string())?;
    println!("fetched {} bytes for {}", bytes.len(), offer.version);
    crate::update::verified(&bytes, &offer.signature, crate::update::PUBKEY)?;
    println!("the download is signed by the key this build trusts");
    Ok(bytes.to_vec())
}

/// Moves one channel into one of a team's sidebar categories.
///
/// The server replaces a category wholesale, `channel_ids` and all, so moving
/// one channel means sending two of them back: the one losing it and the one
/// gaining it. Sending only the gainer leaves the channel in both.
async fn moved(
    rest: &matterless_core::rest::RestClient,
    me_id: &str,
    team_id: &str,
    channel_id: &str,
    category_id: &str,
) -> matterless_core::Result<()> {
    let held = rest.sidebar_categories(me_id, team_id).await?;
    let mut changed: Vec<matterless_core::model::SidebarCategory> = Vec::new();
    for category in &held.categories {
        let holds = category.channel_ids.iter().any(|id| id == channel_id);
        let wanted = category.id == category_id;
        if holds == wanted {
            continue;
        }
        let mut copy = category.clone();
        if wanted {
            // Newest first within the category it arrives in, which is where
            // the official client puts it too.
            copy.channel_ids.insert(0, channel_id.to_string());
        } else {
            copy.channel_ids.retain(|id| id != channel_id);
        }
        changed.push(copy);
    }
    if changed.is_empty() {
        return Ok(());
    }
    rest.update_sidebar_categories(me_id, team_id, &changed)
        .await
}

/// Fetches and keeps the user records the store is missing.
///
/// Answers nothing: the store is the answer, and the window is woken so that
/// what it draws from the store is drawn again.
async fn met(
    rest: &matterless_core::rest::RestClient,
    store: &Store,
    user_ids: &[String],
    wake: &impl Wake,
) {
    if learn(rest, store, user_ids).await {
        wake.wake(Update::Met);
    }
}

/// Fetches and keeps the user records the store is missing, quietly.
///
/// Answers whether it learned anything, so a caller can decide whether the
/// window needs telling. The one that does not is a message arriving: what
/// reads those names is about to run anyway, and it has to run *after* this
/// rather than be sent back to do it again.
async fn learn(
    rest: &matterless_core::rest::RestClient,
    store: &Store,
    user_ids: &[String],
) -> bool {
    let unknown: Vec<String> = match store.users_by_ids(user_ids) {
        Ok(known) => user_ids
            .iter()
            .filter(|id| !known.contains_key(*id))
            .cloned()
            .collect(),
        // A store that cannot be asked is a store worth telling.
        Err(_) => user_ids.to_vec(),
    };
    if unknown.is_empty() {
        return false;
    }
    match rest.users_by_ids(&unknown).await {
        Ok(people) if people.is_empty() => false,
        Ok(people) => {
            println!("met {} people for the first time", people.len());
            match store.upsert_users(&people) {
                Ok(()) => true,
                Err(error) => {
                    eprintln!("storing the people: {error}");
                    false
                }
            }
        }
        Err(error) => {
            eprintln!("asking who these people are: {error}");
            false
        }
    }
}

/// Who wrote the posts these deltas are about, as far as the store knows.
fn wrote(store: &Store, deltas: &[Delta]) -> Vec<String> {
    let mut who: Vec<String> = deltas
        .iter()
        .filter_map(|delta| match delta {
            Delta::PostUpserted { post_id, .. } => store.post(post_id).ok().flatten(),
            _ => None,
        })
        .map(|post| post.user_id)
        .filter(|id| !id.is_empty())
        .collect();
    who.sort();
    who.dedup();
    who
}

/// Whether this is a conversation the store has heard of.
///
/// The window can be showing one the server has never had: with no session it
/// draws an invented sample under an invented id, and it goes on showing that
/// for the moment between signing in and the channel list arriving. Asking a
/// server to mark `sample` read, or for its recent posts, is a round trip that
/// can only come back 400 -- which is exactly what a first sign-in did, twice.
fn known(store: &Store, channel_id: &str) -> bool {
    matches!(store.channel(channel_id), Ok(Some(_)))
}

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
        // How this reader arranged that team: their Favorites, their
        // Channels, whatever they made themselves, and the order inside each.
        //
        // Never fetched here before. The window read the categories out of
        // the store and nothing in the window ever put them there, so a store
        // that had not been filled by some other tool arranged every
        // conversation under one heading called "Other" -- which is what a
        // first sign-in got, because a first sign-in is the one case where
        // nothing else has been near the database.
        match rest.sidebar_categories(me_id, &team.id).await {
            Ok(held) => {
                if let Err(error) = store.upsert_sidebar(&team.id, &held.categories) {
                    eprintln!("storing the categories for {}: {error}", team.id);
                }
            }
            Err(error) => eprintln!("the categories for {}: {error}", team.id),
        }
        // And the threads this reader follows in it.
        //
        // Never asked for before. The threads table was filled only as replies
        // happened to arrive down the socket, so the Threads list held
        // whatever this window had been open for -- three of them, against a
        // hundred and seventy-two followed -- rather than what the reader
        // actually follows. The list has always been ordered by last activity;
        // it simply had almost nothing to order.
        match rest.my_threads(me_id, &team.id, FOLLOWED, false).await {
            Ok(page) => {
                // The roots as well as the threads. A followed thread can live
                // in a channel this window has never opened, and without its
                // root there is nothing to put in the row: `followed_threads`
                // joins the posts table for the message and comes back empty.
                let roots: Vec<matterless_core::model::Post> =
                    page.threads.iter().map(|one| one.post.clone()).collect();
                if let Err(error) = store.keep_thread_roots(&roots) {
                    eprintln!("storing the thread roots for {}: {error}", team.id);
                }
                if let Err(error) = store.upsert_threads(&page.threads) {
                    eprintln!("storing the threads for {}: {error}", team.id);
                }
                println!("following {} threads in {}", page.threads.len(), team.id);
            }
            Err(error) => eprintln!("the followed threads for {}: {error}", team.id),
        }
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
    // Kept, not just read. These were fetched only to work the thread mode out
    // of and then dropped on the floor -- so every other preference the window
    // asks the store for came back empty. Two that matter: the one that lifts
    // unread conversations into their own group at the top, and the order the
    // teams go in. Without the first there is no Unreads heading at all and
    // every unread channel stays where it lives, which reads as the heading
    // having missed them.
    if let Err(error) = store.upsert_preferences(&preferences) {
        eprintln!("storing the preferences: {error}");
    }
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
                // A channel has no single face, whoever is in it.
                face: None,
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
            id: person.id.clone(),
            label: format!("@{} -- message", person.username),
            direct: true,
            reach: crate::switcher::Reach::Direct,
            face: Some(crate::switcher::Face {
                user_id: person.id,
                avatar_at: person.last_picture_update,
            }),
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
///
/// Nothing in the window does this in one gesture any more -- a dropped file
/// joins the message being written, like a pasted one. It is kept for the
/// `drop_a_file` probe, which is how the round trip is exercised against a
/// real server without a window.
pub async fn upload(
    rest: &matterless_core::rest::RestClient,
    engine: &SyncEngine,
    context: &SyncContext,
    channel_id: &str,
    root_id: &str,
    path: &std::path::Path,
) {
    let Some(file) = put(rest, channel_id, path).await else {
        return;
    };
    let name = file.name.clone();
    let file_ids = [file.id];
    // No message of its own: a file dropped on the window *is* the message.
    // A file pasted into the box waits there instead -- see `Ask::Attach`.
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

/// Puts one file on the server and says what it became.
///
/// The half both errands share: dropping a file posts it at once and pasting
/// one leaves it waiting, but each has to get the bytes up there first and
/// each refuses the same things for the same reasons.
async fn put(
    rest: &matterless_core::rest::RestClient,
    channel_id: &str,
    path: &std::path::Path,
) -> Option<matterless_core::model::FileInfo> {
    let name = path.file_name().and_then(|name| name.to_str())?;
    match std::fs::metadata(path) {
        Ok(held) if held.len() > LARGEST => {
            eprintln!(
                "{name} is {} MB, which is too large",
                held.len() / 1_048_576
            );
            return None;
        }
        Err(error) => {
            eprintln!("{name}: {error}");
            return None;
        }
        _ => {}
    }
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) => {
            eprintln!("{name}: {error}");
            return None;
        }
    };
    println!("sending {name}, {} bytes", bytes.len());

    let sent = match rest.upload_file(channel_id, name, &bytes, None).await {
        Ok(sent) => sent,
        Err(error) => {
            eprintln!("uploading {name}: {error}");
            return None;
        }
    };
    let file = sent.file_infos.into_iter().next();
    if file.is_none() {
        eprintln!("{name} uploaded but the server named no file");
    }
    file
}

/// Where this machine keeps what a person downloads.
/// How much of each end of a video is fetched for its first picture: the start
/// holds the picture, and the end holds the index when an MP4 put it there.
const POSTER_ENDS: u64 = 2 * 1024 * 1024;

/// A video's first picture, `width` by `height`, as PNG bytes for the cache.
///
/// From the video itself when it is on the disk; otherwise from its two ends,
/// fetched into a file of its full length with nothing in between -- enough
/// for ffmpeg to find the first picture, a few megabytes where the whole film
/// is tens. The server does not always honour a range, though: the same file
/// came back whole on one request and in part on the next. A whole answer is
/// taken as the file, and kept, so playing it later is instant.
async fn poster_of(
    rest: &RestClient,
    file_id: &str,
    width: u32,
    height: u32,
) -> Result<Vec<u8>, String> {
    let (width, height) = (width.max(1), height.max(1));
    let kept = film_path(file_id);
    if let Some(path) = kept.clone().filter(|path| path.is_file()) {
        return off_the_loop(move || picture_of(&path, width, height))
            .await
            .unwrap_or_else(|| Err("the poster was not made".to_string()));
    }
    let route = format!("/files/{file_id}");
    let fetch = |range: String| {
        let route = route.clone();
        async move {
            rest.fetch_range(&route, &range)
                .await
                .map_err(|why| why.to_string())?
                .ok_or_else(|| "the server has no such file".to_string())
        }
    };
    let head = fetch(format!("bytes=0-{}", POSTER_ENDS - 1)).await?;
    let total = head
        .content_range
        .as_deref()
        .and_then(|said| said.rsplit('/').next())
        .and_then(|total| total.parse::<u64>().ok());
    let made = match total {
        Some(total) if total > POSTER_ENDS => {
            let from = total.saturating_sub(POSTER_ENDS).max(POSTER_ENDS);
            let tail = fetch(format!("bytes={from}-{}", total - 1)).await?;
            match tail.content_range.is_some() {
                true => Made::Ends {
                    head: head.bytes,
                    from,
                    tail: tail.bytes,
                    total,
                },
                false => Made::Whole(tail.bytes),
            }
        }
        // All of it, whether it was asked for in part or not.
        _ => Made::Whole(head.bytes),
    };
    let file_id = file_id.to_string();
    off_the_loop(move || match made {
        Made::Whole(bytes) => match kept {
            Some(path) => {
                keep_film(&path, &bytes).map_err(|why| why.to_string())?;
                picture_of(&path, width, height)
            }
            None => ends_to_picture(&file_id, &bytes, None, bytes.len() as u64, width, height),
        },
        Made::Ends {
            head,
            from,
            tail,
            total,
        } => ends_to_picture(&file_id, &head, Some((from, &tail)), total, width, height),
    })
    .await
    .unwrap_or_else(|| Err("the poster was not made".to_string()))
}

/// What a poster is made from: the whole file, or its two ends.
enum Made {
    Whole(Vec<u8>),
    Ends {
        head: Vec<u8>,
        from: u64,
        tail: Vec<u8>,
        total: u64,
    },
}

/// Lays the two ends of a video out in a file of its length, and makes the
/// poster from that.
fn ends_to_picture(
    file_id: &str,
    head: &[u8],
    tail: Option<(u64, &[u8])>,
    total: u64,
    width: u32,
    height: u32,
) -> Result<Vec<u8>, String> {
    use std::io::{Seek, SeekFrom, Write};
    let path = std::env::temp_dir().join(format!("matterless-poster-{file_id}"));
    let made = (|| {
        let mut file = std::fs::File::create(&path).map_err(|why| why.to_string())?;
        file.set_len(total).map_err(|why| why.to_string())?;
        file.write_all(head).map_err(|why| why.to_string())?;
        if let Some((from, bytes)) = tail {
            file.seek(SeekFrom::Start(from))
                .map_err(|why| why.to_string())?;
            file.write_all(bytes).map_err(|why| why.to_string())?;
        }
        drop(file);
        picture_of(&path, width, height)
    })();
    let _ = std::fs::remove_file(&path);
    made
}

/// The first picture of the video at `path`, as PNG bytes.
fn picture_of(path: &std::path::Path, width: u32, height: u32) -> Result<Vec<u8>, String> {
    let rgba = matterless_media::poster(path, width, height)?;
    let picture = image::RgbaImage::from_raw(width, height, rgba)
        .ok_or_else(|| "a picture of the wrong size".to_string())?;
    let mut png = std::io::Cursor::new(Vec::new());
    image::DynamicImage::ImageRgba8(picture)
        .write_to(&mut png, image::ImageFormat::Png)
        .map_err(|why| why.to_string())?;
    Ok(png.into_inner())
}

/// How much disk the videos a reader played may keep between them.
const FILM_BYTES: u64 = 2 * 1024 * 1024 * 1024;

/// Where a video is kept: by its id alone. ffmpeg tells what a file is from
/// what is in it, and the poster, which has only the id, keeps a video it
/// happened to be sent whole under the same name the player looks for.
pub fn film_path(file_id: &str) -> Option<std::path::PathBuf> {
    if file_id.is_empty() || !file_id.chars().all(|c| c.is_ascii_alphanumeric()) {
        return None;
    }
    let folder = crate::feed::pictures_dir()?.parent()?.join("films");
    Some(folder.join(format!("{file_id}.video")))
}

/// Writes a video beside the others, then lets the ones played longest ago go
/// until they fit.
fn keep_film(path: &std::path::Path, bytes: &[u8]) -> std::io::Result<()> {
    let Some(folder) = path.parent() else {
        return Ok(());
    };
    std::fs::create_dir_all(folder)?;
    let partial = path.with_extension("partial");
    std::fs::write(&partial, bytes)?;
    std::fs::rename(&partial, path)?;
    let mut held: Vec<(std::time::SystemTime, u64, std::path::PathBuf)> =
        std::fs::read_dir(folder)?
            .flatten()
            .filter_map(|item| {
                let metadata = item.metadata().ok()?;
                metadata.is_file().then(|| {
                    (
                        metadata.modified().unwrap_or(std::time::UNIX_EPOCH),
                        metadata.len(),
                        item.path(),
                    )
                })
            })
            .collect();
    held.sort();
    let mut total: u64 = held.iter().map(|(_, bytes, _)| bytes).sum();
    for (_, bytes, old) in held {
        if total <= FILM_BYTES || old == path {
            continue;
        }
        if std::fs::remove_file(&old).is_ok() {
            total -= bytes;
        }
    }
    Ok(())
}

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
