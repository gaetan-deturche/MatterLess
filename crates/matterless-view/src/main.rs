//! A window showing the message list, drawn on Vulkan.
//!
//! Standalone on purpose. It is the same layout and the same draw list the app
//! will use, in a window of its own, so the rendering can be judged before it
//! replaces anything -- and so a scroll can be tried against the thing that was
//! meant to fix scrolling.
//!
//! Run it with `cargo run -p matterless-view`.

use matterless_layout::Fonts;
use matterless_layout::row::{RowLayout, Theme, lay_out};
use matterless_paint::{Painter, Palette, Scene};
use matterless_ui::input::{Event as UiEvent, Input, Key, Mods};
// Aliased: `Node` is a markdown node in this file already, and a box here.
use matterless_render::markdown::Node;
use matterless_render::{PostRow, Row};
use matterless_ui::{Axis, Node as Boxed, Placed, Rect, Size};
use matterless_view::composer::{self, Composer};
use matterless_view::header::{self, Header};
use matterless_view::live::Update;
use matterless_view::sidebar::{Canvas, Entry, Sidebar};
use matterless_view::stream::Stream;
use std::sync::Arc;
use winit::application::ApplicationHandler;
use winit::event::{MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::window::{Window, WindowId};

fn post(author: &str, nodes: Vec<Node>) -> PostRow {
    PostRow {
        post_id: format!("p-{author}-{}", nodes.len()),
        root_id: String::new(),
        author_id: format!("u-{author}"),
        author_name: author.into(),
        create_at: 0,
        update_at: 0,
        edited: false,
        nodes: Arc::new(nodes),
        reactions: Vec::new(),
        files: Vec::new(),
        attachments: Vec::new(),
        avatar_at: 0,
        bot: false,
        body_is_attachment_only: false,
        pending: false,
        failed: false,
        pinned: false,
        saved: false,
        following: false,
        previews: Vec::new(),
    }
}

fn para(text: &str) -> Node {
    Node::Paragraph {
        children: vec![Node::Text { value: text.into() }],
    }
}

/// Enough rows to scroll through, with the shapes that used to be mismeasured:
/// long wraps, mixed weight, mentions, lists and code.
fn conversation() -> Vec<Row> {
    let mut rows = Vec::new();
    for day in 0..6 {
        rows.push(Row::DateSeparator {
            epoch_day: 20_340 + day,
        });
        rows.push(Row::Post {
            post: post("simon.odwyer", vec![para("Yo!")]),
        });
        rows.push(Row::Continuation {
            post: post(
                "simon.odwyer",
                vec![para(
                    "I would like to raise this limit on swarm please, to fifty or perhaps a \
                     hundred, and it appears to live in the configuration file under the data \
                     directory. Do you have access to that in the tools team, or is it more a \
                     question for the people who look after the servers themselves?",
                )],
            ),
        });
        rows.push(Row::Post {
            post: post(
                "claudio.redavid",
                vec![
                    Node::Paragraph {
                        children: vec![
                            Node::Text {
                                value: "ah that is ".into(),
                            },
                            Node::Strong {
                                children: vec![Node::Text {
                                    value: "a good question".into(),
                                }],
                            },
                            Node::Text {
                                value: " -- ask ".into(),
                            },
                            Node::UserMention {
                                username: "olivier.gaertner".into(),
                                everyone: false,
                            },
                            Node::Text {
                                value: ", he installed it".into(),
                            },
                        ],
                    },
                    Node::List {
                        ordered: false,
                        items: vec![
                            vec![para("we have access to the P4 server")],
                            vec![para("but not to where the plugin lives")],
                        ],
                    },
                    Node::CodeBlock {
                        language: Some("php".into()),
                        value: "'max_files' => 100,\n'expand_all' => true,".into(),
                    },
                ],
            ),
        });
    }
    rows
}

/// The sidebar's width. Fixed, as it is in the app today.
const SIDEBAR: f32 = 260.0;
/// The thread pane's width when one is open.
const THREAD: f32 = 420.0;
/// What the thread pane's reply box answers to.
const THREAD_COMPOSER: &str = "thread-composer";

struct App {
    window: Option<Arc<Window>>,
    surface: Option<wgpu::Surface<'static>>,
    view: Option<matterless_view::View>,
    format: wgpu::TextureFormat,
    /// The same surface read without its sRGB encoding, which is what the
    /// pipeline writes through.
    plain: wgpu::TextureFormat,
    size: (u32, u32),
    fonts: Fonts,
    painter: Painter,
    /// The open channel.
    stream: Stream,
    /// The open thread, when there is one. A second stream rather than a
    /// second kind of panel: a thread is the same rows in a narrower column.
    thread: Option<Stream>,
    palette: Palette,
    /// The store, kept open: switching channel is a read, not a reload.
    store: Option<Arc<matterless_store::Store>>,
    /// True while the socket is up. Drawn in the header, because a client that
    /// has quietly stopped receiving looks exactly like a quiet channel.
    connected: bool,
    /// Who the reader is, empty until the server says. Decides unread counts
    /// and which half of a direct message names it.
    me: String,
    sidebar: Sidebar,
    input: Input,
    placed: Vec<Placed>,
    composer: Composer,
    /// The thread pane's own reply box. Kept across a close so a half-written
    /// reply survives glancing back at the channel, and cleared when a
    /// different thread opens.
    thread_composer: Composer,
    /// Copy and paste within this window. Crossing to another process needs a
    /// platform clipboard, which is a dependency this window does not have yet.
    clipboard: String,
    /// The socket thread, once it is up. Sends go through it.
    link: Option<matterless_view::live::Link>,
    /// Messages written but not yet confirmed, held in memory and nowhere else:
    /// a guess must never reach SQLite.
    outstanding: Arc<matterless_render::pending::PendingPosts>,
    /// Pictures already asked for, so a face on screen is fetched once rather
    /// than on every frame it is visible.
    asked: std::collections::HashSet<String>,
    /// Decoded pictures waiting to go into the atlas, which only the thread
    /// that owns the GPU may touch.
    arrived: Vec<(String, u32, u32, Vec<u8>)>,
    /// What a clicked notification does. Held once and shared with every toast,
    /// because each one outlives the call that raised it.
    clicked: Option<Arc<matterless_view::toast::Clicked>>,
    /// Jumping to a conversation by name, which is how a reader gets around a
    /// hundred and fourteen channels without hunting the sidebar.
    switcher: matterless_view::switcher::Switcher,
    /// Finding a message already said, answered from the local store.
    search: matterless_view::search::Search,
    /// Picking a reaction, and the row it is anchored to.
    picker: matterless_view::picker::Picker,
    picked_near: matterless_ui::Rect,
    /// Changing a message, in the row it sits in.
    edit: matterless_view::edit::Edit,
    /// Who is typing, and when this window last said that it was.
    typing: matterless_view::typing::Typing,
    said_typing: Option<std::time::Instant>,
    /// Who is around, and the set last asked about.
    presence: std::collections::HashMap<String, String>,
    asked_about: Vec<String>,
    /// Saved or pinned messages, whichever was last asked for.
    listing: matterless_view::listing::Listing,
    /// How far back the open channel is read. Grows as older pages arrive.
    depth: u32,
    /// True while a page of history is in flight, so the same page is not asked
    /// for once per frame while the reader sits at the top.
    loading_older: bool,
    /// False once the beginning of the channel has been reached.
    more_history: bool,
}

/// The height kept for the "somebody is typing" line, above each composer.
const TYPING: f32 = 16.0;

/// How many emoji names are asked about in one frame. A channel full of them
/// must not turn one frame into fifty requests.
const LOOKUPS: usize = 12;

impl App {
    /// Real messages if the store can be read, and the sample if not.
    ///
    /// Falling back rather than refusing to start: the sample is what proves the
    /// renderer, and it should still be reachable on a machine with no database
    /// -- but the real rows are what prove the port, so they are tried first.
    fn feed() -> (String, Vec<Row>) {
        // Positional arguments only, with flags and their values dropped:
        // taking `--snapshot` as a database path made `Store::open` create a
        // file by that name, which then had no messages in it.
        let mut positional = Vec::new();
        let mut args = std::env::args().skip(1);
        while let Some(arg) = args.next() {
            if arg.starts_with("--") {
                args.next();
            } else {
                positional.push(arg);
            }
        }
        let mut positional = positional.into_iter();
        let path = positional
            .next()
            .map(std::path::PathBuf::from)
            .or_else(matterless_view::feed::default_store);
        let channel = positional.next();
        let me = std::env::var("MATTERLESS_ME").unwrap_or_default();
        match path
            .as_deref()
            .map(|path| matterless_view::feed::rows_from(path, channel.clone(), &me))
        {
            Some(Ok((channel, rows))) => {
                println!("channel {channel}: {} rows from the store", rows.len());
                (channel, rows)
            }
            Some(Err(why)) => {
                println!("the sample conversation ({why})");
                ("sample".to_string(), conversation())
            }
            None => {
                println!("the sample conversation (no store path)");
                ("sample".to_string(), conversation())
            }
        }
    }

    fn new() -> Self {
        // The store is opened once and kept: switching channel is a read.
        let store = matterless_view::feed::default_store()
            .and_then(|path| matterless_view::feed::open(&path).ok())
            .map(Arc::new);
        // Without a reader there is no membership row, and the store answers
        // with each channel's *total* message count -- which looks like an
        // unread badge of four thousand. No reader, no counts.
        let me = std::env::var("MATTERLESS_ME").unwrap_or_default();
        if me.is_empty() {
            // Only until the socket signs in, which answers the same question
            // properly. Worth saying because the first paint happens before
            // that: with no reader, neither half of a `<id>__<id>` slug can be
            // ruled out, so a direct message may be named after the wrong one.
            println!(
                "no reader yet: counts stay blank and a direct message may be \
                 named after the wrong half of its pair until the socket signs in"
            );
        }
        let entries = Self::entries(store.as_deref(), &me);
        let sidebar = Sidebar::new(entries);
        let (channel, rows) = Self::feed();
        let mut app = Self {
            window: None,
            surface: None,
            view: None,
            format: wgpu::TextureFormat::Bgra8UnormSrgb,
            plain: wgpu::TextureFormat::Bgra8Unorm,
            size: (1000, 760),
            fonts: Fonts::new(),
            painter: Painter::new(),
            stream: {
                let mut stream = Stream::new("stream");
                // Looked up here as well as in `open_channel`, because the
                // first channel never goes through it: without this every
                // custom emoji on the channel the app opens with drew as an
                // empty box until the reader left it and came back.
                if let Some(store) = store.as_deref() {
                    stream.custom = matterless_view::feed::custom_emoji(store, &rows);
                }
                stream.rows = rows;
                stream
            },
            thread: None,
            palette: Palette::default(),
            store,
            sidebar,
            input: Input::default(),
            placed: Vec::new(),
            connected: false,
            me: me.clone(),
            composer: Composer::new(composer::NAME),
            thread_composer: {
                let mut reply = Composer::new(THREAD_COMPOSER);
                reply.placeholder = "Reply".to_string();
                reply
            },
            clipboard: String::new(),
            link: None,
            outstanding: Arc::new(matterless_render::pending::PendingPosts::default()),
            asked: std::collections::HashSet::new(),
            arrived: Vec::new(),
            clicked: None,
            switcher: matterless_view::switcher::Switcher::default(),
            search: matterless_view::search::Search::default(),
            picker: matterless_view::picker::Picker::default(),
            picked_near: matterless_ui::Rect::new(0.0, 0.0, 0.0, 0.0),
            edit: matterless_view::edit::Edit::default(),
            typing: matterless_view::typing::Typing::default(),
            said_typing: None,
            presence: std::collections::HashMap::new(),
            asked_about: Vec::new(),
            listing: matterless_view::listing::Listing::default(),
            depth: matterless_view::feed::PAGE,
            loading_older: false,
            more_history: true,
        };
        app.sidebar.selected = Some(channel);
        // Focused before anything is clicked: a chat window that needs a click
        // before it will accept typing is a chat window that feels broken.
        app.input.focus_on(composer::NAME);
        app
    }

    /// The window as boxes: a fixed sidebar, the channel column, and the thread
    /// pane beside it when one is open.
    fn shell(&self) -> Vec<Placed> {
        let mut row = Boxed::new("shell", Size::Grow(1.0))
            .axis(Axis::Row)
            .with(Boxed::new("sidebar-panel", Size::Fixed(SIDEBAR)))
            .with(
                Boxed::new("channel", Size::Grow(1.0))
                    .axis(Axis::Column)
                    .with(Boxed::new("header", Size::Fixed(header::HEIGHT)))
                    .with(Boxed::new("stream", Size::Grow(1.0)))
                    .with(Boxed::new(
                        composer::NAME,
                        Size::Fixed(self.composer.height()),
                    )),
            );
        if self.thread.is_some() {
            row = row.with(
                Boxed::new("thread-panel", Size::Fixed(THREAD))
                    .axis(Axis::Column)
                    .with(Boxed::new("thread-header", Size::Fixed(header::HEIGHT)))
                    .with(Boxed::new("thread", Size::Grow(1.0)))
                    .with(Boxed::new(
                        THREAD_COMPOSER,
                        Size::Fixed(self.thread_composer.height()),
                    )),
            );
        }
        matterless_ui::solve::solve(
            &row,
            Rect::new(0.0, 0.0, self.size.0 as f32, self.size.1 as f32),
        )
    }

    fn sidebar_rect(&self) -> Rect {
        Rect::new(0.0, 0.0, SIDEBAR, self.size.1 as f32)
    }

    /// Everything right of the sidebar, thread pane included.
    fn column_rect(&self) -> Rect {
        Rect::new(
            SIDEBAR,
            0.0,
            (self.size.0 as f32 - SIDEBAR).max(0.0),
            self.size.1 as f32,
        )
    }

    /// The thread pane's own column, when a thread is open.
    fn thread_rect(&self) -> Option<Rect> {
        let column = self.column_rect();
        self.thread.as_ref()?;
        // Never more than half: on a narrow window a fixed pane would leave the
        // conversation it belongs to too thin to read.
        let width = THREAD.min(column.width * 0.5);
        Some(Rect::new(
            column.right() - width,
            column.y,
            width,
            column.height,
        ))
    }

    /// The channel's column: everything right of the sidebar that the thread
    /// pane has not taken.
    fn channel_rect(&self) -> Rect {
        let column = self.column_rect();
        match self.thread_rect() {
            Some(thread) => Rect::new(column.x, column.y, thread.x - column.x, column.height),
            None => column,
        }
    }

    /// The stream, between the header above it and the composer below.
    fn stream_rect(&self) -> Rect {
        let above = self.composer.above(header::below(self.channel_rect()));
        Rect::new(above.x, above.y, above.width, above.height - TYPING)
    }

    /// The line above the composer saying somebody is writing.
    ///
    /// Reserved whether or not anybody is, so the conversation does not jump a
    /// line every time somebody starts and stops.
    fn typing_rect(&self) -> Rect {
        let above = self.composer.above(header::below(self.channel_rect()));
        Rect::new(above.x, above.bottom() - TYPING, above.width, TYPING)
    }

    /// The strip the composer sits in, at the foot of the channel's column.
    fn composer_rect(&self) -> Rect {
        self.composer.strip(header::below(self.channel_rect()))
    }

    /// Everything in the thread pane below its header: the replies and the box
    /// to write one in.
    fn thread_body(&self) -> Option<Rect> {
        Some(header::below(self.thread_rect()?))
    }

    /// The thread's replies, above its reply box.
    fn thread_stream_rect(&self) -> Option<Rect> {
        let above = self.thread_composer.above(self.thread_body()?);
        Some(Rect::new(
            above.x,
            above.y,
            above.width,
            above.height - TYPING,
        ))
    }

    /// The same line, for the thread pane.
    fn thread_typing_rect(&self) -> Option<Rect> {
        let above = self.thread_composer.above(self.thread_body()?);
        Some(Rect::new(
            above.x,
            above.bottom() - TYPING,
            above.width,
            TYPING,
        ))
    }

    fn thread_composer_rect(&self) -> Option<Rect> {
        Some(self.thread_composer.strip(self.thread_body()?))
    }

    /// The open channel's name, which is what the header says.
    fn title(&self) -> String {
        let Some(open) = self.sidebar.selected.as_deref() else {
            return String::new();
        };
        self.sidebar
            .entries
            .iter()
            .find_map(|entry| match entry {
                Entry::Channel { id, label, .. } if id == open => Some(label.clone()),
                _ => None,
            })
            // The sample feed has no sidebar row behind it, and an id is a poor
            // name but better than an empty strip.
            .unwrap_or_else(|| open.to_string())
    }

    fn redraw(&self) {
        if let Some(window) = &self.window {
            window.request_redraw();
        }
    }

    /// The sidebar's rows: the reader's own groups, flattened into one list.
    ///
    /// Built here rather than inline because it is built twice -- once from
    /// whatever was known at startup, and again once the server has said who
    /// the reader is, which is what makes the unread counts real.
    fn entries(store: Option<&matterless_store::Store>, me: &str) -> Vec<Entry> {
        let groups = store
            .map(|store| matterless_view::sidebar_feed::groups(store, me))
            .unwrap_or_default();
        // Without a reader there is no membership row, and the store answers
        // with each channel's *total* message count -- which looks like an
        // unread badge of four thousand.
        let counted = !me.is_empty();
        let mut entries: Vec<Entry> = Vec::new();
        for group in groups {
            if group.channels.is_empty() {
                continue;
            }
            // Named by its team when one contributes: two teams each bring a
            // "Favorites" and a "Channels", and unqualified they read as
            // duplicates of each other.
            entries.push(Entry::Heading {
                label: if group.team_name.is_empty() {
                    group.display_name
                } else {
                    format!("{} -- {}", group.display_name, group.team_name)
                },
            });
            for channel in group.channels {
                entries.push(Entry::Channel {
                    direct: channel.channel_type == "D" || channel.channel_type == "G",
                    // Only a one-to-one has a single other person; a group has
                    // several and no dot could stand for all of them.
                    counterpart: if channel.channel_type == "D" {
                        channel.counterpart_id.clone()
                    } else {
                        None
                    },
                    id: channel.id,
                    label: channel.display_name,
                    unread: if counted { channel.unread } else { 0 },
                    mentions: if counted { channel.mentions } else { 0 },
                    muted: channel.muted,
                });
            }
        }
        println!(
            "sidebar: {} groups, {} channels -- {}",
            entries
                .iter()
                .filter(|entry| matches!(entry, Entry::Heading { .. }))
                .count(),
            entries
                .iter()
                .filter(|entry| matches!(entry, Entry::Channel { .. }))
                .count(),
            entries
                .iter()
                .filter_map(|entry| match entry {
                    Entry::Heading { label } => Some(label.as_str()),
                    Entry::Channel { .. } => None,
                })
                .collect::<Vec<&str>>()
                .join(" > ")
        );
        entries
    }

    /// Rebuilds the sidebar now that the reader is known.
    ///
    /// Names a direct message after the right half of its pair and gives every
    /// row a real unread count -- neither of which was answerable before the
    /// server said who the token belongs to.
    fn signed_in(&mut self, id: &str) {
        self.me = id.to_string();
        let open = self.sidebar.selected.clone();
        let scroll = self.sidebar.scroll;
        self.sidebar = Sidebar::new(Self::entries(self.store.as_deref(), id));
        self.sidebar.selected = open;
        self.sidebar.scroll = scroll;
        // The channel already on screen never went through `open_channel`, so
        // this is its one chance to be reconciled: it was drawn from the store
        // before there was a connection to check it against.
        if let (Some(channel), Some(link)) = (&self.sidebar.selected, self.link.as_ref()) {
            link.send(matterless_view::live::Ask::Looking {
                channel_id: channel.clone(),
            });
            link.send(matterless_view::live::Ask::Refresh {
                channel_id: channel.clone(),
            });
        }
    }

    /// Opens the socket, if there is a store and a session to open it with.
    ///
    /// Refusing to start is not a failure worth stopping for: the window still
    /// draws everything the database holds, which is what it did before there
    /// was a socket at all.
    fn connect(&mut self, proxy: winit::event_loop::EventLoopProxy<Update>) {
        let Some(store) = self.store.clone() else {
            println!("no database, so nothing to keep up to date");
            return;
        };
        let Some(path) = matterless_view::feed::default_store() else {
            return;
        };
        let Some(server) = matterless_view::live::stored_server(&path) else {
            println!("no server.txt beside the database; staying offline");
            return;
        };
        let Some(token) = matterless_view::live::stored_token() else {
            println!("no session in the keychain; staying offline");
            return;
        };
        // The click handler is built here because it needs the same proxy the
        // socket thread wakes the window with: a toast fires its callback on a
        // thread of its own, and this is how the answer gets home.
        let waking = proxy.clone();
        self.clicked = Some(Arc::new(Box::new(move |channel_id: String| {
            let _ = waking.send_event(matterless_view::live::Update::Activated(channel_id));
        })));
        println!("connecting to {server}");
        self.link = Some(matterless_view::live::start(
            store,
            server,
            token,
            Proxy(proxy),
        ));
    }

    /// Applies what the socket reported.
    fn apply(&mut self, update: Update) {
        match update {
            Update::Connected(up) => {
                self.connected = up;
                println!("socket {}", if up { "connected" } else { "lost" });
            }
            Update::SignedIn { id, username } => {
                println!("reader is {username}");
                self.signed_in(&id);
            }
            Update::Listed { title, found } => {
                // Only if it is still the list being looked at: a slow answer
                // must not reopen a panel the reader has already shut, or
                // refill one they have since asked something else of.
                if self.listing.title == title {
                    self.listing.fill(found);
                }
            }
            Update::Statuses(found) => {
                for (user_id, status) in found {
                    self.presence.insert(user_id, status);
                }
            }
            Update::Older { channel_id, more } => {
                self.more_history = more;
                self.loading_older = false;
                if self.sidebar.selected.as_deref() == Some(channel_id.as_str()) {
                    // Deeper, so the read that follows brings back what was
                    // just stored rather than the same newest page again.
                    self.depth += 200;
                    self.hold_place();
                }
            }
            Update::Activated(channel_id) => {
                // Clicking a notification is the reader saying they want to be
                // looking at that conversation.
                if let Some(window) = &self.window {
                    window.focus_window();
                }
                self.sidebar.selected = Some(channel_id.clone());
                self.open_channel(&channel_id);
            }
            Update::Picture {
                key,
                width,
                height,
                rgba,
            } => self.arrived.push((key, width, height, rgba)),
            Update::SendSettled {
                pending_post_id,
                channel_id,
                failed,
            } => {
                if failed {
                    // Kept on screen, marked, rather than losing what was
                    // written: the reader can see it did not go.
                    self.outstanding.mark_failed(&pending_post_id);
                    eprintln!("a message did not send");
                } else {
                    // The socket echo has usually already stored the real post
                    // and this is the second of two answers; dropping the guess
                    // twice is harmless.
                    self.outstanding.resolve(&pending_post_id);
                }
                if self.sidebar.selected.as_deref() == Some(channel_id.as_str()) {
                    self.reread_channel(&channel_id);
                }
                if let Some(root) = self.open_root() {
                    self.reread_thread(&root);
                }
            }
            Update::Failed(why) => {
                self.connected = false;
                eprintln!("staying offline: {why}");
            }
            Update::Changed(deltas) => {
                self.announce(&deltas);
                // The engine has already written every one of these. The only
                // question left is whether anything on screen is now stale.
                let open = self.sidebar.selected.clone().unwrap_or_default();
                let channels = matterless_view::live::touched(&deltas);
                if channels.iter().any(|channel| channel == &open) {
                    self.reread_channel(&open);
                }
                if let Some(root) = self.open_root()
                    && matterless_view::live::touches_thread(&deltas, &root)
                {
                    self.reread_thread(&root);
                }
                // A reaction names its post and no channel, so whether it
                // matters is a question only the window can answer. Without
                // this a pill the reader just added stayed invisible until
                // they left the channel and came back.
                for delta in &deltas {
                    if let matterless_sync::Delta::StatusChanged { user_id, status } = delta {
                        self.presence.insert(user_id.clone(), status.clone());
                    }
                    if let matterless_sync::Delta::Typing {
                        channel_id,
                        user_id,
                        root_id,
                    } = delta
                    {
                        // Never this reader: the server echoes their own
                        // signal back, and "someone is typing" about yourself
                        // is nonsense.
                        if user_id != &self.me {
                            // Counts, never who: this is the loudest signal on
                            // the socket and the log is not a record of who
                            // talks to whom.
                            let before = self.typing.count(channel_id, root_id);
                            self.typing.note(channel_id, root_id, user_id);
                            let now = self.typing.count(channel_id, root_id);
                            if now != before {
                                println!("{now} typing in {channel_id}");
                            }
                        }
                    }
                }
                let reacted = matterless_view::live::reacted(&deltas);
                if !reacted.is_empty() {
                    if self.stream.holds_any(&reacted) {
                        self.reread_channel(&open);
                    }
                    if let Some(root) = self.open_root()
                        && self
                            .thread
                            .as_ref()
                            .is_some_and(|thread| thread.holds_any(&reacted))
                    {
                        self.reread_thread(&root);
                    }
                }
                // The emoji table arriving is not a reason to replan four
                // hundred messages, so `touched` names no channel for it. It
                // is a reason to look up the names again: a pill drawn before
                // the table landed found nothing behind its name and stayed
                // blank for as long as the channel was open.
                if matterless_view::live::renames_emoji(&deltas) {
                    self.rename_emoji();
                }
            }
        }
        self.redraw();
    }

    /// Asks who is around, when the set of people worth asking about changes.
    ///
    /// Derived from the sidebar rather than from the messages on screen: the
    /// sidebar is where presence is actually read, because whether somebody is
    /// around decides whether you write to them now. The rows in a channel
    /// change constantly while the people in them almost never do, so asking
    /// from there would be a request per arriving message.
    fn ask_who_is_around(&mut self) {
        let Some(link) = self.link.as_ref() else {
            return;
        };
        let mut wanted: Vec<String> = self
            .sidebar
            .entries
            .iter()
            .filter_map(|entry| match entry {
                matterless_view::sidebar::Entry::Channel { counterpart, .. } => counterpart.clone(),
                _ => None,
            })
            .collect();
        wanted.sort();
        wanted.dedup();
        // An unchanged set costs no request, which is the whole point: this is
        // called from the frame loop.
        if wanted.is_empty() || wanted == self.asked_about {
            return;
        }
        self.asked_about = wanted.clone();
        link.send(matterless_view::live::Ask::Statuses { user_ids: wanted });
    }

    /// Tells the server this reader is typing, no more than now and then.
    ///
    /// Every keystroke would be a message per character on the busiest signal
    /// there is, and the other clients hold what they hear for six seconds --
    /// so once every three says the same thing for a fraction of the traffic.
    fn say_typing(&mut self, root_id: &str) {
        const EVERY: std::time::Duration = std::time::Duration::from_secs(3);
        if self.said_typing.is_some_and(|last| last.elapsed() < EVERY) {
            return;
        }
        let (Some(channel), Some(link)) = (self.sidebar.selected.clone(), self.link.as_ref())
        else {
            return;
        };
        self.said_typing = Some(std::time::Instant::now());
        link.send(matterless_view::live::Ask::Typing {
            channel_id: channel,
            root_id: root_id.to_string(),
        });
    }

    /// Names any reaction that can be neither drawn nor fetched.
    ///
    /// Names only, never message text. A pill with no character and no picture
    /// is an empty box, and the tally is what says whether that is a name
    /// nobody has asked about or one the server answered "not custom" for.
    fn report_blank_pills(&self) {
        let blank: Vec<&str> = self
            .stream
            .rows
            .iter()
            .filter_map(|row| match row {
                Row::Post { post } | Row::Continuation { post } => Some(post),
                _ => None,
            })
            .flat_map(|post| &post.reactions)
            .filter(|reaction| reaction.unicode.is_none())
            .map(|reaction| reaction.emoji.as_str())
            .filter(|name| !self.stream.custom.contains_key(*name))
            .collect();
        if !blank.is_empty() {
            println!("no character and no picture for: {}", blank.join(", "));
        }
    }

    /// Asks the server about emoji names nobody has asked about yet.
    ///
    /// A few at a time, and remembered either way, so a channel full of
    /// unknown names is a handful of requests spread over a few frames rather
    /// than fifty at once. Whatever is left over goes on the next frame.
    fn name_emoji(&mut self) {
        let (Some(store), Some(link)) = (self.store.clone(), self.link.as_ref()) else {
            return;
        };
        let mut names = matterless_view::feed::unknown_emoji(&store, &self.stream.rows);
        if let Some(thread) = self.thread.as_ref() {
            names.extend(matterless_view::feed::unknown_emoji(&store, &thread.rows));
        }
        let asking: Vec<String> = names
            .into_iter()
            // The asked set is what stops the same name going out once a frame
            // while the answer is still in flight.
            .filter(|name| self.asked.insert(format!("emoji-name/{name}")))
            .take(LOOKUPS)
            .collect();
        if !asking.is_empty() {
            link.send(matterless_view::live::Ask::NameEmoji { names: asking });
        }
    }

    /// Looks up the custom emoji on screen again, and asks for any new picture.
    fn rename_emoji(&mut self) {
        let Some(store) = self.store.clone() else {
            return;
        };
        self.stream.custom = matterless_view::feed::custom_emoji(&store, &self.stream.rows);
        if let Some(thread) = self.thread.as_mut() {
            thread.custom = matterless_view::feed::custom_emoji(&store, &thread.rows);
        }
        self.report_blank_pills();
        self.want_faces();
    }

    /// Sends a message, showing it before the server has agreed to it.
    ///
    /// The guess goes on screen first and the request second: a chat client
    /// that waits for a round trip before showing what you typed feels broken
    /// on any connection worse than a good one.
    fn post_message(&mut self, channel_id: &str, root_id: &str, message: String) {
        let Some(link) = self.link.as_ref() else {
            eprintln!("offline, so nothing was sent");
            return;
        };
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|since| since.as_millis() as i64)
            .unwrap_or_default();
        let pending_post_id = matterless_render::pending::PendingPosts::new_id(&self.me, now);
        // The length, never the text.
        println!(
            "sending {} characters to {channel_id} (threaded: {})",
            message.chars().count(),
            !root_id.is_empty()
        );
        self.outstanding
            .insert(matterless_render::pending::PendingPost {
                pending_post_id: pending_post_id.clone(),
                channel_id: channel_id.to_string(),
                root_id: root_id.to_string(),
                message: message.clone(),
                create_at: now,
                failed: false,
                files: Vec::new(),
            });
        link.send(matterless_view::live::Ask::Send {
            pending_post_id,
            channel_id: channel_id.to_string(),
            root_id: root_id.to_string(),
            message,
        });
        // Painted immediately, and pinned to the bottom: sending is the one
        // case where the reader definitely wants to be looking at the newest
        // message, because they just wrote it.
        self.reread_channel(channel_id);
        let within = self.stream_rect();
        self.stream.to_bottom(within);
        if let Some(root) = self.open_root() {
            self.reread_thread(&root);
        }
    }

    /// Raises a notification for anything that earned one.
    ///
    /// The decision was made in the sync engine, by the same `notify::decide`
    /// the app runs -- muting, mentions, the reader's own messages and their
    /// do-not-disturb are all already accounted for. Nothing is reconsidered
    /// here; the flag on the delta is the answer.
    fn announce(&mut self, deltas: &[matterless_sync::Delta]) {
        let Some(store) = self.store.clone() else {
            return;
        };
        // A message in the channel being looked at is one the reader can
        // already see, so interrupting them about it is noise. The app leaves
        // this to window focus, which this window cannot ask about yet.
        let open = self.sidebar.selected.clone().unwrap_or_default();
        for delta in deltas {
            let matterless_sync::Delta::PostUpserted {
                post_id,
                channel_id,
                notify: true,
                ..
            } = delta
            else {
                continue;
            };
            if channel_id == &open {
                continue;
            }
            let said = matterless_sync::notify::announce(&store, post_id);
            let (title, body) = matterless_view::toast::wording(&said);
            // The count, never the words: a notification carries the message
            // and the log must not.
            println!(
                "notifying about {channel_id} ({} characters, named: {})",
                said.preview.chars().count(),
                said.resolved
            );
            let Some(clicked) = self.clicked.clone() else {
                continue;
            };
            matterless_view::toast::raise(channel_id, &title, &body, clicked);
        }
    }

    /// Asks for the faces on screen that have not been asked for yet.
    ///
    /// Called after anything that changes what is visible -- a scroll, a new
    /// message, a channel switch. The asked set is what keeps that from being a
    /// request per frame.
    fn want_faces(&mut self) {
        let Some(link) = self.link.as_ref() else {
            return;
        };
        let mut wanted = self.stream.faces(self.stream_rect());
        if let (Some(thread), Some(within)) = (&self.thread, self.thread_stream_rect()) {
            wanted.extend(thread.faces(within));
        }
        // The picker's grid is mostly custom emoji, which are pictures like any
        // other and go through the same asked set.
        wanted.extend(self.picker.wants());
        for (key, width, height) in wanted {
            if self.asked.insert(key.clone()) {
                link.send(matterless_view::live::Ask::Fetch { key, width, height });
            }
        }
    }

    /// Sends a failed message again, keeping its pending id.
    ///
    /// The same id, so the row does not jump: it is the one already on screen
    /// being tried again, not a second attempt appearing beneath the first.
    fn retry(&mut self, pending_post_id: &str) {
        let (Some(link), Some(held)) = (self.link.as_ref(), self.outstanding.get(pending_post_id))
        else {
            return;
        };
        println!("retrying a message to {}", held.channel_id);
        // Unmarked before it goes, so the row stops saying it failed while it
        // is in flight.
        self.outstanding
            .insert(matterless_render::pending::PendingPost {
                failed: false,
                ..held.clone()
            });
        link.send(matterless_view::live::Ask::Send {
            pending_post_id: held.pending_post_id.clone(),
            channel_id: held.channel_id.clone(),
            root_id: held.root_id.clone(),
            message: held.message.clone(),
        });
        let channel = held.channel_id.clone();
        self.reread_channel(&channel);
    }

    /// Re-reads the channel now that older history is behind it, keeping the
    /// reader on the message they were looking at.
    ///
    /// Older messages are added *above*, so the scroll has to grow by exactly
    /// what was added or the view jumps by a page. That is the problem the DOM
    /// list never solved and it is arithmetic here, because every height is
    /// known before anything is drawn.
    fn hold_place(&mut self) {
        let Some(store) = self.store.clone() else {
            return;
        };
        let Some(channel) = self.sidebar.selected.clone() else {
            return;
        };
        let before = self.stream.total();
        let outstanding = self.outstanding.for_channel(&channel);
        match matterless_view::feed::rows_of(&store, &channel, &self.me, &outstanding, self.depth) {
            Ok(rows) => {
                self.stream.me = self.me.clone();
                self.stream.custom = matterless_view::feed::custom_emoji(&store, &rows);
                self.stream.rows = rows;
                self.relayout();
                let grew = self.stream.total() - before;
                let within = self.stream_rect();
                self.stream.scroll =
                    (self.stream.scroll + grew).clamp(0.0, self.stream.reach(within));
                println!("older history: the list grew by {grew:.0}px");
            }
            Err(why) => eprintln!("{channel}: {why}"),
        }
    }

    /// Asks for another page of history when the reader nears the top.
    ///
    /// Guarded so the same page is not asked for once per frame while they sit
    /// there, and stopped for good once the channel has no more to give.
    fn want_older(&mut self) {
        if self.loading_older || !self.more_history {
            return;
        }
        let within = self.stream_rect();
        // Within a screenful of the top, so the page is on its way before the
        // reader arrives at the end of what is there.
        if self.stream.scroll > within.height {
            return;
        }
        let (Some(link), Some(channel)) = (self.link.as_ref(), self.sidebar.selected.clone())
        else {
            return;
        };
        self.loading_older = true;
        link.send(matterless_view::live::Ask::LoadOlder {
            channel_id: channel,
        });
    }

    /// Does what a toolbar button asked for.
    ///
    /// Two of them never reach the server: opening a thread is this window's
    /// own business, and a permalink is a string.
    fn act(&mut self, action: matterless_view::actions::Action, post_id: String, on: bool) {
        use matterless_view::actions::Action;
        match action {
            Action::React => {
                // Anchored under the message, so the grid says which one it
                // would react to without needing a title that says so.
                let stream = self.stream_rect();
                let row = self.stream.row_rect(&post_id, stream);
                self.picked_near = row
                    .map(|row| matterless_ui::Rect::new(row.x + 60.0, row.bottom(), 0.0, 0.0))
                    .unwrap_or(stream);
                self.picker.show(&post_id, &mut self.fonts, &mut self.input);
            }
            Action::Thread => {
                let root = self
                    .stream
                    .rows
                    .iter()
                    .find_map(|row| match row {
                        Row::Post { post } | Row::Continuation { post }
                            if post.post_id == post_id =>
                        {
                            Some(if post.root_id.is_empty() {
                                post.post_id.clone()
                            } else {
                                post.root_id.clone()
                            })
                        }
                        _ => None,
                    })
                    .unwrap_or(post_id);
                self.open_thread(&root);
            }
            Action::Edit => {
                // The raw markdown from the store, not the rendered body: what
                // was typed is what should come back into the box.
                let Some(said) = self
                    .store
                    .as_ref()
                    .and_then(|store| store.post(&post_id).ok().flatten())
                else {
                    eprintln!("{post_id}: no copy to edit");
                    return;
                };
                let mut input = std::mem::take(&mut self.input);
                self.edit
                    .show(&post_id, &said.message, &mut self.fonts, &mut input);
                self.input = input;
            }
            Action::Link => {
                // Into this window's own clipboard, which is where everything
                // else it copies goes until there is a platform one.
                self.clipboard = format!("/pl/{post_id}");
                println!("copied a permalink");
            }
            other => {
                if let Some(link) = self.link.as_ref() {
                    link.send(matterless_view::live::Ask::Act {
                        action: other,
                        post_id,
                        on,
                    });
                }
            }
        }
    }

    /// The root of the open thread, if one is open.
    fn open_root(&self) -> Option<String> {
        let name = &self.thread.as_ref()?.name;
        name.strip_prefix("thread/").map(str::to_string)
    }

    /// Re-reads the open channel, keeping the reader where they were.
    ///
    /// Pinned to the bottom only when it was already there: a message arriving
    /// while somebody is reading history must not drag them away from it.
    fn reread_channel(&mut self, channel: &str) {
        let Some(store) = self.store.clone() else {
            return;
        };
        let within = self.stream_rect();
        let was_at_end = self.stream.scroll >= self.stream.reach(within) - 1.0;
        match matterless_view::feed::rows_of(
            &store,
            channel,
            &self.me,
            &self.outstanding.for_channel(channel),
            self.depth,
        ) {
            Ok(rows) => {
                self.stream.me = self.me.clone();
                self.stream.custom = matterless_view::feed::custom_emoji(&store, &rows);
                self.stream.rows = rows;
                self.relayout();
                if was_at_end {
                    let within = self.stream_rect();
                    self.stream.to_bottom(within);
                }
            }
            Err(why) => eprintln!("{channel}: {why}"),
        }
    }

    /// Re-reads the open thread the same way.
    fn reread_thread(&mut self, root_id: &str) {
        let Some(within) = self.thread_stream_rect() else {
            return;
        };
        let was_at_end = self
            .thread
            .as_ref()
            .is_some_and(|thread| thread.scroll >= thread.reach(within) - 1.0);
        // Rebuilt through the same path the click takes, so a live reply and an
        // opened thread are planned identically.
        let open = self.thread.take();
        self.open_thread(root_id);
        if !was_at_end && let (Some(old), Some(new)) = (open, self.thread.as_mut()) {
            new.scroll = old.scroll;
        }
    }

    /// Every box a pointer can land on.
    ///
    /// One list rather than one per handler: a click and a wheel turn have to
    /// be tested against the same boxes, and building them separately is how
    /// the two come to disagree about what is under the pointer.
    fn targets(&self) -> Vec<Placed> {
        let mut boxes = self.shell();
        boxes.extend(self.sidebar.boxes(self.sidebar_rect()));
        // The hovered row is what decides whether a toolbar exists, and it is
        // read from the frame just gone: a toolbar the pointer is already on
        // must stay under it.
        boxes.extend(
            self.stream
                .boxes(self.stream_rect(), self.stream.hovered(&self.input)),
        );
        if let (Some(thread), Some(rect)) = (&self.thread, self.thread_stream_rect()) {
            boxes.extend(thread.boxes(rect, thread.hovered(&self.input)));
        }
        boxes.extend(self.picker.boxes(self.picked_near, self.stream_rect()));
        boxes.extend(self.listing.boxes(self.stream_rect()));
        if let Some(row) = self.edited_row() {
            boxes.extend(self.edit.boxes(row, self.stream_rect()));
        }
        boxes
    }

    /// Where the message being edited sits, if it is still on screen.
    fn edited_row(&self) -> Option<matterless_ui::Rect> {
        let post_id = self.edit.for_post.as_ref()?;
        self.stream.row_rect(post_id, self.stream_rect())
    }

    /// Opens a thread beside the channel, or closes the one that is open.
    ///
    /// Read straight from the store like everything else here: the root, its
    /// replies, and the same planner the channel uses, so the thread's rows are
    /// the rows the app would draw rather than an approximation of them.
    fn open_thread(&mut self, root_id: &str) {
        let Some(store) = self.store.as_ref() else {
            return;
        };
        // Clicking the open thread again closes it, which is what the twisty
        // in every other client does.
        if self
            .thread
            .as_ref()
            .is_some_and(|open| open.name == thread_name(root_id))
        {
            self.close_thread();
            return;
        }
        let Ok(Some(root)) = store.post(root_id) else {
            eprintln!("thread {root_id}: no root in the local store");
            return;
        };
        let replies = store.thread_replies(root_id).unwrap_or_default();
        let mut people: Vec<String> = std::iter::once(root.user_id.clone())
            .chain(replies.iter().map(|reply| reply.user_id.clone()))
            .collect();
        people.sort();
        people.dedup();
        let known = store.users_by_ids(&people).unwrap_or_default();

        let mut options =
            matterless_render::PlanOptions::new(matterless_core::model::ThreadMode::Flat, &self.me);
        options.utc_offset_minutes = matterless_view::clock::utc_offset_minutes();
        options.author_names = known
            .iter()
            .map(|(id, user)| (id.clone(), user.username.clone()))
            .collect();
        options.author_avatars = known
            .iter()
            .map(|(id, user)| (id.clone(), user.last_picture_update))
            .collect();

        let mut stream = Stream::new(thread_name(root_id));
        stream.rows = matterless_render::plan_thread(&root, &replies, &options);
        stream.custom = matterless_view::feed::custom_emoji(store, &stream.rows);
        println!("thread {root_id}: {} rows", stream.rows.len());
        // A reply half-written to one thread does not belong in another. It
        // survives closing and reopening the same one, which is the case worth
        // keeping.
        self.thread_composer.clear(&mut self.fonts);
        self.thread = Some(stream);
        // Writing is what the pane is for, so it opens focused.
        self.input.focus_on(THREAD_COMPOSER);
        // The pane takes width from the channel, so both have to be laid out
        // again before anything is drawn against the old one.
        self.relayout();
        if let Some(within) = self.thread_stream_rect()
            && let Some(thread) = self.thread.as_mut()
        {
            thread.to_bottom(within);
        }
    }

    fn close_thread(&mut self) {
        if self.thread.take().is_some() {
            self.relayout();
        }
    }

    /// Hands the frame's input to the widgets that want it.
    fn react(&mut self) {
        // The editor first of all: it is the only panel that is part of a
        // message rather than in front of one, and while it is open the keys
        // belong to it rather than to the composer at the bottom.
        if self.edit.open() {
            let mut input = std::mem::take(&mut self.input);
            if input.struck(Key::Escape) {
                self.edit.hide(&mut input);
                self.input = input;
                return;
            }
            let within = self.stream_rect();
            let row = self
                .edited_row()
                .unwrap_or_else(|| matterless_ui::Rect::new(within.x, within.y, within.width, 0.0));
            let saved = self
                .edit
                .react(&mut self.fonts, &input, row, within, &mut self.clipboard);
            if let Some(message) = saved {
                let post_id = self.edit.for_post.clone().unwrap_or_default();
                self.edit.hide(&mut input);
                self.input = input;
                if let Some(link) = self.link.as_ref() {
                    link.send(matterless_view::live::Ask::Edit { post_id, message });
                }
                return;
            }
            self.input = input;
            return;
        }

        // A list of messages stands in front of the conversation, like the
        // switcher and search do, so while it is up the keys belong to it.
        if self.listing.open() {
            let mut input = std::mem::take(&mut self.input);
            if input.struck(Key::Escape) {
                self.listing.hide();
                input.focus_on(composer::NAME);
                self.input = input;
                return;
            }
            let chosen = self.listing.react(&input);
            if let Some(found) = chosen {
                self.listing.hide();
                input.focus_on(composer::NAME);
                self.input = input;
                // Opened at the conversation it was said in. Landing on the
                // message itself needs an anchor the window cannot ask for
                // yet, so it opens where the reader can find it rather than
                // pretending.
                self.sidebar.selected = Some(found.channel_id.clone());
                self.open_channel(&found.channel_id);
                return;
            }
            self.input = input;
            return;
        }

        // The picker first: it is the smallest of the panels and the only one
        // opened from a message, so while it is up the keys belong to it.
        if self.picker.open() {
            let mut input = std::mem::take(&mut self.input);
            if input.struck(Key::Escape) {
                self.picker.hide(&mut input);
                self.input = input;
                return;
            }
            let within = self.stream_rect();
            let store = self.store.clone();
            let chosen = self.picker.react(
                &mut self.fonts,
                &input,
                self.picked_near,
                within,
                &mut self.clipboard,
                store.as_deref(),
            );
            self.want_faces();
            if let Some(emoji) = chosen {
                let post_id = self.picker.for_post.clone().unwrap_or_default();
                self.picker.hide(&mut input);
                self.input = input;
                if let Some(link) = self.link.as_ref() {
                    link.send(matterless_view::live::Ask::React {
                        post_id,
                        emoji,
                        // Always adding: the picker is how a reaction that is
                        // not on the message yet gets there. Taking one off is
                        // what the pill underneath is for.
                        on: true,
                    });
                }
                return;
            }
            self.input = input;
            return;
        }

        // Search first and alone while it is open, for the same reason the
        // switcher is: it is a thing the reader is doing instead of reading.
        if self.search.open {
            let mut input = std::mem::take(&mut self.input);
            if input.struck(Key::Escape) {
                self.search.hide(&mut input);
                self.input = input;
                return;
            }
            let within = self.stream_rect();
            let store = self.store.clone();
            let hit = store.as_ref().and_then(|store| {
                self.search.react(
                    &mut self.fonts,
                    &input,
                    within,
                    &mut self.clipboard,
                    store,
                    &self.me,
                )
            });
            if let Some(hit) = hit {
                self.search.hide(&mut input);
                self.input = input;
                // Opened at the channel it was said in. Landing on the message
                // itself needs an anchor the window cannot ask for yet, so it
                // opens where the reader can find it rather than pretending.
                self.sidebar.selected = Some(hit.channel_id.clone());
                self.open_channel(&hit.channel_id);
                return;
            }
            self.input = input;
            return;
        }

        // The switcher first and alone while it is open: it is a thing the
        // reader is doing instead of reading, so the boxes behind it must not
        // take the same keys.
        if self.switcher.open {
            let mut input = std::mem::take(&mut self.input);
            if input.struck(Key::Escape) {
                self.switcher.hide(&mut input);
                self.input = input;
                return;
            }
            let within = self.stream_rect();
            let entries = self.sidebar.entries.clone();
            let chosen = self.switcher.react(
                &mut self.fonts,
                &input,
                within,
                &mut self.clipboard,
                &entries,
            );
            if let Some(channel) = chosen {
                self.switcher.hide(&mut input);
                self.input = input;
                self.sidebar.selected = Some(channel.clone());
                self.open_channel(&channel);
                return;
            }
            self.input = input;
            return;
        }

        let was = (self.composer.height(), self.thread_composer.height());

        // Both boxes are offered the frame. Each one checks whether it holds
        // focus, so only the one the reader is in takes the keystrokes.
        let within = header::below(self.channel_rect());
        let sent = self
            .composer
            .react(&mut self.fonts, &self.input, within, &mut self.clipboard);
        let width = self.channel_rect().width;
        self.composer.lay_out(&mut self.fonts, width);

        let replied = if let Some(body) = self.thread_body() {
            let replied =
                self.thread_composer
                    .react(&mut self.fonts, &self.input, body, &mut self.clipboard);
            self.thread_composer.lay_out(&mut self.fonts, body.width);
            replied
        } else {
            None
        };

        // A keystroke in either box is this reader typing, which the other
        // clients want to know. Which box says whether it is the channel or
        // the thread, because that is what the signal carries.
        if !self.input.typed().is_empty() {
            let root = match self.input.focus() {
                Some(THREAD_COMPOSER) => self.open_root().unwrap_or_default(),
                _ => String::new(),
            };
            self.say_typing(&root);
        }

        // Only when a box actually grew or shrank. Every keystroke would
        // otherwise re-lay-out four hundred messages to learn nothing moved.
        if (self.composer.height(), self.thread_composer.height()) != was {
            self.relayout();
        }

        // Sent after the boxes are settled: posting re-reads the channel, and
        // doing that with a composer mid-resize would lay the stream out
        // against a height that is about to change.
        if let Some(text) = sent
            && let Some(channel) = self.sidebar.selected.clone()
        {
            self.post_message(&channel, "", text);
        }
        if let Some(text) = replied
            && let Some(root) = self.open_root()
        {
            // A reply goes to the channel the thread is in, which is the one
            // being looked at.
            let channel = self.sidebar.selected.clone().unwrap_or_default();
            self.post_message(&channel, &root, text);
        }
    }

    /// Reads a channel and lays it out, then shows its newest message.
    fn open_channel(&mut self, channel: &str) {
        let Some(store) = self.store.clone() else {
            return;
        };
        match matterless_view::feed::rows_of(
            &store,
            channel,
            &self.me,
            &self.outstanding.for_channel(channel),
            self.depth,
        ) {
            Ok(rows) => {
                self.stream.me = self.me.clone();
                self.stream.custom = matterless_view::feed::custom_emoji(&store, &rows);
                self.stream.rows = rows;
                self.report_blank_pills();
                // A thread from the channel just left has nothing to do with
                // the one just opened.
                self.thread = None;
                // A different channel starts from its newest page: the depth
                // reached in the last one says nothing about this one.
                self.depth = matterless_view::feed::PAGE;
                self.more_history = true;
                self.loading_older = false;
                self.relayout();
                let within = self.stream_rect();
                self.stream.to_bottom(within);
                // Opening a channel is reading it, and a channel that stays
                // unread however long it is looked at makes the sidebar a lie.
                if let Some(link) = self.link.as_ref() {
                    link.send(matterless_view::live::Ask::MarkRead {
                        channel_id: channel.to_string(),
                    });
                    // And it is the channel on screen, which is what keeps a
                    // message the reader is watching arrive from interrupting
                    // them about itself.
                    link.send(matterless_view::live::Ask::Looking {
                        channel_id: channel.to_string(),
                    });
                    // Opening it is also the moment to find out what happened
                    // to it while nobody was connected. The rows already on
                    // screen are the local copy; this is what corrects them.
                    link.send(matterless_view::live::Ask::Refresh {
                        channel_id: channel.to_string(),
                    });
                }
            }
            Err(why) => eprintln!("{channel}: {why}"),
        }
    }

    /// Everything the frame draws, in one scene.
    fn scene(&mut self) -> Scene {
        let mut scene = Scene::default();
        let sidebar = self.sidebar_rect();
        let strip = header::strip(self.column_rect());
        let stream = self.stream_rect();

        scene.clip_to(sidebar.x, sidebar.y, sidebar.width, sidebar.height);
        let boxes = self.sidebar.boxes(sidebar);
        let mut canvas = Canvas {
            scene: &mut scene,
            painter: &mut self.painter,
            fonts: &mut self.fonts,
            palette: &self.palette,
        };
        self.sidebar
            .draw(&mut canvas, &boxes, sidebar, &self.input, &self.presence);

        // Read before the painter is borrowed, and given its own layer after: a
        // name too long for the strip is cut by the clip rather than running
        // along the top of the first message.
        // Said in the header while it is down, and silent while it is up: a
        // client that has quietly stopped receiving is indistinguishable from a
        // quiet channel, and that is the state worth naming.
        let header = Header::new(if self.connected {
            self.title()
        } else {
            format!("{} (offline)", self.title())
        });
        scene.clip_to(strip.x, strip.y, strip.width, strip.height);
        let mut canvas = Canvas {
            scene: &mut scene,
            painter: &mut self.painter,
            fonts: &mut self.fonts,
            palette: &self.palette,
        };
        header.draw(&mut canvas, strip);

        scene.clip_to(stream.x, stream.y, stream.width, stream.height);
        let mut canvas = Canvas {
            scene: &mut scene,
            painter: &mut self.painter,
            fonts: &mut self.fonts,
            palette: &self.palette,
        };
        self.stream.draw(&mut canvas, stream, &self.input);

        // The thread pane: its own header and its own clip, so a reply cannot
        // spill into the conversation it came from.
        if let (Some(thread), Some(pane)) = (&self.thread, self.thread_rect()) {
            let strip = header::strip(pane);
            // Above the reply box, not the whole pane: replies drawn behind it
            // would show through the box's own margin.
            let rows = self.thread_composer.above(header::below(pane));
            let title = Header::new(format!("Thread -- {} replies", thread.rows.len()));
            scene.clip_to(strip.x, strip.y, strip.width, strip.height);
            let mut canvas = Canvas {
                scene: &mut scene,
                painter: &mut self.painter,
                fonts: &mut self.fonts,
                palette: &self.palette,
            };
            title.draw(&mut canvas, strip);

            scene.clip_to(rows.x, rows.y, rows.width, rows.height);
            let mut canvas = Canvas {
                scene: &mut scene,
                painter: &mut self.painter,
                fonts: &mut self.fonts,
                palette: &self.palette,
            };
            thread.draw(&mut canvas, rows, &self.input);

            if let Some(strip) = self.thread_composer_rect() {
                scene.clip_to(strip.x, strip.y, strip.width, strip.height);
                let focused = self.input.focus() == Some(THREAD_COMPOSER);
                let body = header::below(pane);
                let mut canvas = Canvas {
                    scene: &mut scene,
                    painter: &mut self.painter,
                    fonts: &mut self.fonts,
                    palette: &self.palette,
                };
                self.thread_composer.draw(&mut canvas, body, focused);
            }
        }

        // The line above each composer. Drawn before the composers so it is
        // under them, which is where a reserved strip belongs.
        let open = self.sidebar.selected.clone().unwrap_or_default();
        let root = self.open_root().unwrap_or_default();
        let lines = [
            (self.typing_rect(), self.typing.line(&open, "")),
            (
                self.thread_typing_rect()
                    .unwrap_or(Rect::new(0.0, 0.0, 0.0, 0.0)),
                self.open_root()
                    .and_then(|root| self.typing.line(&open, &root)),
            ),
        ];
        let _ = root;
        for (rect, said) in lines {
            let Some(said) = said else { continue };
            if rect.width <= 0.0 {
                continue;
            }
            scene.clip_to(rect.x, rect.y, rect.width, rect.height);
            let mut canvas = Canvas {
                scene: &mut scene,
                painter: &mut self.painter,
                fonts: &mut self.fonts,
                palette: &self.palette,
            };
            let Canvas {
                scene,
                painter,
                fonts,
                palette,
            } = &mut canvas;
            let glyphs = painter.run(
                fonts,
                &said,
                rect.x + 24.0,
                rect.y,
                matterless_paint::Run::label(f32::MAX),
            );
            scene.glyphs(glyphs, palette.faint, palette.faint);
        }

        // Its own layer last, so the caret and the box sit over the stream
        // rather than under a message that scrolled into the strip.
        let composer = self.composer_rect();
        scene.clip_to(composer.x, composer.y, composer.width, composer.height);
        let focused = self.input.focus() == Some(composer::NAME);
        let within = header::below(self.channel_rect());
        let mut canvas = Canvas {
            scene: &mut scene,
            painter: &mut self.painter,
            fonts: &mut self.fonts,
            palette: &self.palette,
        };
        self.composer.draw(&mut canvas, within, focused);

        // Last, and over the whole window: the switcher covers what it stands
        // in front of rather than sitting beside it.
        if self.switcher.open {
            let stream = self.stream_rect();
            let field = self.switcher.field(stream);
            scene.clip_to(0.0, 0.0, self.size.0 as f32, self.size.1 as f32);
            let mut canvas = Canvas {
                scene: &mut scene,
                painter: &mut self.painter,
                fonts: &mut self.fonts,
                palette: &self.palette,
            };
            self.switcher.draw(&mut canvas, stream);
            self.switcher.query.draw(&mut canvas, field, true);
        }
        if self.listing.open() {
            let stream = self.stream_rect();
            scene.clip_to(0.0, 0.0, self.size.0 as f32, self.size.1 as f32);
            let mut canvas = Canvas {
                scene: &mut scene,
                painter: &mut self.painter,
                fonts: &mut self.fonts,
                palette: &self.palette,
            };
            self.listing.draw(&mut canvas, stream);
        }
        if self.search.open {
            let stream = self.stream_rect();
            let field = self.search.field(stream);
            scene.clip_to(0.0, 0.0, self.size.0 as f32, self.size.1 as f32);
            let mut canvas = Canvas {
                scene: &mut scene,
                painter: &mut self.painter,
                fonts: &mut self.fonts,
                palette: &self.palette,
            };
            self.search.draw(&mut canvas, stream);
            self.search.query.draw(&mut canvas, field, true);
        }
        // In the row rather than over the window, so it is clipped to the
        // stream like the message it stands in place of.
        if self.edit.open()
            && let Some(row) = self.edited_row()
        {
            scene.clip_to(stream.x, stream.y, stream.width, stream.height);
            let mut canvas = Canvas {
                scene: &mut scene,
                painter: &mut self.painter,
                fonts: &mut self.fonts,
                palette: &self.palette,
            };
            self.edit.draw(&mut canvas, row, stream);
        }

        // Over everything, including the toolbar it was opened from: it is a
        // small panel and whatever it covers is not what the reader is doing.
        if self.picker.open() {
            let near = self.picked_near;
            let field = self.picker.field(near, stream);
            scene.clip_to(0.0, 0.0, self.size.0 as f32, self.size.1 as f32);
            let mut canvas = Canvas {
                scene: &mut scene,
                painter: &mut self.painter,
                fonts: &mut self.fonts,
                palette: &self.palette,
            };
            self.picker.draw(&mut canvas, near, stream);
            self.picker.query.draw(&mut canvas, field, true);
        }
        scene
    }

    /// Lays the whole conversation out for the current width.
    ///
    /// Once per width change, never per frame: the heights do not depend on the
    /// scroll position, which is the property that makes this list honest.
    fn relayout(&mut self) {
        // The composer is shaped first: it decides its own height, and the
        // stream gets what is left, so its width has to be settled before the
        // rows are laid out against it.
        let width = self.channel_rect().width;
        self.composer.lay_out(&mut self.fonts, width);

        let stream = self.stream_rect();
        self.stream.lay_out(&mut self.fonts, stream.width);
        self.stream.clamp(stream);

        if let Some(pane) = self.thread_rect() {
            self.thread_composer.lay_out(&mut self.fonts, pane.width);
        }
        if let Some(within) = self.thread_stream_rect()
            && let Some(thread) = self.thread.as_mut()
        {
            thread.lay_out(&mut self.fonts, within.width);
            thread.clamp(within);
        }
    }
}

/// Puts a just-created window at the bottom of the stack, without focus.
///
/// `with_active(false)` asks not to be *activated*, which is not the same as
/// asking not to be *raised*: the window still arrives in front of whatever
/// the reader is looking at. This is what the app does in `reveal_quietly`,
/// and for the same reason.
#[cfg(windows)]
fn behind(window: &winit::window::Window) {
    use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
    let Ok(handle) = window.window_handle() else {
        return;
    };
    let RawWindowHandle::Win32(win32) = handle.as_raw() else {
        return;
    };
    use windows::Win32::Foundation::HWND;
    use windows::Win32::UI::WindowsAndMessaging::{
        HWND_BOTTOM, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_SHOWWINDOW, SetWindowPos,
    };
    let hwnd = HWND(win32.hwnd.get() as *mut std::ffi::c_void);
    let placed = unsafe {
        SetWindowPos(
            hwnd,
            Some(HWND_BOTTOM),
            0,
            0,
            0,
            0,
            SWP_NOACTIVATE | SWP_NOMOVE | SWP_NOSIZE | SWP_SHOWWINDOW,
        )
    };
    match placed {
        Ok(()) => println!("opened behind everything else (MATTERLESS_QUIET)"),
        Err(error) => eprintln!("could not open quietly: {error}"),
    }
}

#[cfg(not(windows))]
fn behind(_window: &winit::window::Window) {}

impl ApplicationHandler<Update> for App {
    /// Waits for the next thing to happen, or for a typing line to go stale.
    ///
    /// `Wait` alone would leave "someone is typing" on screen until something
    /// else woke the window, which on a quiet channel is the person who
    /// stopped typing sending their message -- or never.
    fn about_to_wait(&mut self, events: &ActiveEventLoop) {
        let (expired, next) = self.typing.forget_stale();
        // Only when a line actually went: asking for a frame whenever one is
        // merely live would redraw every frame for as long as anybody types.
        if expired {
            self.redraw();
        }
        events.set_control_flow(match next {
            Some(expires) => ControlFlow::WaitUntil(expires),
            None => ControlFlow::Wait,
        });
    }

    fn resumed(&mut self, events: &ActiveEventLoop) {
        // Opened without taking focus when asked, which is what makes it
        // usable next to the work it is being compared against: a window that
        // seizes the keyboard every time it starts interrupts whoever is
        // watching it.
        let quiet = std::env::var_os("MATTERLESS_QUIET").is_some();
        let window = Arc::new(
            events
                .create_window(
                    Window::default_attributes()
                        .with_title("MatterLess -- list on Vulkan")
                        .with_active(!quiet)
                        .with_inner_size(winit::dpi::LogicalSize::new(
                            self.size.0 as f64,
                            self.size.1 as f64,
                        )),
                )
                .expect("a window"),
        );
        if quiet {
            behind(&window);
        }

        // Vulkan by name rather than whatever the platform prefers, which on
        // Windows would be DX12.
        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
            backends: wgpu::Backends::VULKAN,
            ..Default::default()
        });
        let surface = instance
            .create_surface(Arc::clone(&window))
            .expect("a surface");
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::LowPower,
            compatible_surface: Some(&surface),
            force_fallback_adapter: false,
        }))
        .expect("a Vulkan adapter");
        let (device, queue) =
            pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default()))
                .expect("a device");

        let capabilities = surface.get_capabilities(&adapter);
        self.format = capabilities.formats[0];
        self.plain = matterless_view::plain(self.format);
        let physical = window.inner_size();
        self.size = (physical.width.max(1), physical.height.max(1));
        surface.configure(
            &device,
            &wgpu::SurfaceConfiguration {
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                format: self.format,
                width: self.size.0,
                height: self.size.1,
                present_mode: wgpu::PresentMode::AutoVsync,
                alpha_mode: capabilities.alpha_modes[0],
                view_formats: vec![self.plain],
                desired_maximum_frame_latency: 2,
            },
        );

        println!(
            "adapter: {} ({:?}), surface {:?} drawn through {:?}",
            adapter.get_info().name,
            adapter.get_info().backend,
            self.format,
            self.plain
        );
        self.view = Some(matterless_view::View::new(device, queue, self.plain));
        self.surface = Some(surface);
        self.window = Some(window);
        self.relayout();
    }

    /// What the socket reported, delivered on the thread that owns the window.
    fn user_event(&mut self, _events: &ActiveEventLoop, update: Update) {
        self.apply(update);
    }

    fn window_event(&mut self, events: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => events.exit(),
            WindowEvent::Resized(size) => {
                self.size = (size.width.max(1), size.height.max(1));
                if let (Some(surface), Some(view)) = (&self.surface, &self.view) {
                    surface.configure(
                        &view.device,
                        &wgpu::SurfaceConfiguration {
                            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                            format: self.format,
                            width: self.size.0,
                            height: self.size.1,
                            present_mode: wgpu::PresentMode::AutoVsync,
                            alpha_mode: wgpu::CompositeAlphaMode::Auto,
                            view_formats: vec![self.plain],
                            desired_maximum_frame_latency: 2,
                        },
                    );
                }
                // A width change is the case the DOM list could never do
                // cleanly: here the new heights are known before the frame is
                // drawn, so there is nothing to correct afterwards.
                self.relayout();
                if let Some(window) = &self.window {
                    window.request_redraw();
                }
            }
            WindowEvent::CursorMoved { position, .. } => {
                self.placed = self.targets();
                let boxes = self.placed.clone();
                self.input.apply(
                    UiEvent::PointerMoved {
                        x: position.x as f32,
                        y: position.y as f32,
                    },
                    &boxes,
                );
                // A held press is a drag, which selects text in the composer.
                if self.input.pressed().is_some() {
                    self.react();
                }
                self.redraw();
            }
            WindowEvent::CursorLeft { .. } => {
                self.input.apply(UiEvent::PointerLeft, &[]);
                self.redraw();
            }
            WindowEvent::ModifiersChanged(changed) => {
                let held = changed.state();
                self.input.apply(
                    UiEvent::Modifiers(Mods {
                        shift: held.shift_key(),
                        // Resolved here so no widget has to ask what platform
                        // it is on: Command is the chord key on macOS, Ctrl
                        // everywhere else.
                        command: if cfg!(target_os = "macos") {
                            held.super_key()
                        } else {
                            held.control_key()
                        },
                        alt: held.alt_key(),
                    }),
                    &[],
                );
            }
            WindowEvent::KeyboardInput { event, .. } => {
                let down = event.state == winit::event::ElementState::Pressed;
                if let Some(key) = named(&event.logical_key) {
                    self.input.apply(UiEvent::Key { key, down }, &[]);
                }
                // What the platform composed, which is not the keys struck: an
                // accented letter is two keys and one insertion, and a chord
                // produces a key and no text at all.
                if down && let Some(text) = &event.text {
                    self.input.apply(UiEvent::Typed(text.to_string()), &[]);
                }
                if down && self.input.chord(Key::Char('k')) {
                    let mut input = std::mem::take(&mut self.input);
                    self.switcher.show(&mut self.fonts, &mut input);
                    self.input = input;
                }
                // Saved is reader-wide; pinned belongs to the channel in
                // front of them, which is why one takes a channel and the
                // other does not.
                if down && self.input.chord(Key::Char('s')) {
                    self.listing.expect("Saved");
                    if let Some(link) = self.link.as_ref() {
                        link.send(matterless_view::live::Ask::Saved);
                    }
                }
                if down
                    && self.input.chord(Key::Char('p'))
                    && let Some(channel) = self.sidebar.selected.clone()
                {
                    self.listing.expect("Pinned");
                    if let Some(link) = self.link.as_ref() {
                        link.send(matterless_view::live::Ask::Pinned {
                            channel_id: channel,
                        });
                    }
                }
                if down && self.input.chord(Key::Char('f')) {
                    let mut input = std::mem::take(&mut self.input);
                    self.search.show(&mut self.fonts, &mut input);
                    self.input = input;
                }
                if down {
                    // Escape closes the thread when the composer has nothing
                    // to clear, which is the only key the shell claims.
                    if self.input.struck(Key::Escape) && self.thread.is_some() {
                        self.close_thread();
                    }
                    self.react();
                    self.redraw();
                }
            }
            WindowEvent::MouseInput { state, button, .. } => {
                if button != winit::event::MouseButton::Left {
                    return;
                }
                let boxes = self.targets();
                let event = if state == winit::event::ElementState::Pressed {
                    UiEvent::PointerPressed
                } else {
                    UiEvent::PointerReleased
                };
                self.input.apply(event, &boxes);
                let within = self.sidebar_rect();
                if let Some(channel) = self.sidebar.react(&self.input, &boxes, within) {
                    self.open_channel(&channel);
                }
                // A message opens its thread. Taken before the composer reacts,
                // because opening one narrows the column the composer sits in.
                let stream = self.stream_rect();
                match self.stream.react(&self.input, &boxes, stream) {
                    Some(matterless_view::stream::Chose::Thread(root)) => self.open_thread(&root),
                    Some(matterless_view::stream::Chose::Retry(pending)) => self.retry(&pending),
                    Some(matterless_view::stream::Chose::Act {
                        action,
                        post_id,
                        on,
                    }) => self.act(action, post_id, on),
                    Some(matterless_view::stream::Chose::React { post_id, emoji, on }) => {
                        if let Some(link) = self.link.as_ref() {
                            link.send(matterless_view::live::Ask::React { post_id, emoji, on });
                        }
                    }
                    None => {}
                }
                self.react();
                self.redraw();
            }
            WindowEvent::MouseWheel { delta, .. } => {
                let by = match delta {
                    MouseScrollDelta::LineDelta(_, lines) => {
                        lines * self.stream.theme.line_height * 3.0
                    }
                    MouseScrollDelta::PixelDelta(position) => position.y as f32,
                };
                let boxes = self.targets();
                self.input.apply(UiEvent::Wheel { x: 0.0, y: by }, &boxes);
                // One wheel, five panels, and the pointer decides which of them
                // it belongs to -- which is what `wheel_over` is for. Each panel
                // asks about itself, so a fixed strip simply takes the turn and
                // does nothing with it.
                let sidebar = self.sidebar_rect();
                self.sidebar.react(&self.input, &boxes, sidebar);
                let stream = self.stream_rect();
                self.stream.react(&self.input, &boxes, stream);
                self.want_older();
                if let Some(within) = self.thread_stream_rect()
                    && let Some(thread) = self.thread.as_mut()
                {
                    thread.react(&self.input, &boxes, within);
                }
                self.redraw();
            }
            WindowEvent::RedrawRequested => {
                if self.surface.is_none() || self.view.is_none() {
                    return;
                }
                // Pictures go into the atlas here, on the thread that owns the
                // GPU and before the scene names them: uploading after the
                // draw list is built would show a face one frame late.
                if !self.arrived.is_empty()
                    && let Some(view) = self.view.as_mut()
                {
                    let mut placed = 0;
                    let mut refused = 0;
                    for (key, width, height, rgba) in self.arrived.drain(..) {
                        match view
                            .atlas
                            .put_image(&view.queue, &key, &rgba, width, height)
                        {
                            Some(_) => placed += 1,
                            // The atlas is full. Worth saying, because the
                            // symptom is faces that stop appearing partway
                            // through a long scroll and nothing else.
                            None => refused += 1,
                        }
                    }
                    println!("atlas: {placed} pictures in, {refused} refused");
                }
                // What is on screen may have changed since the last frame.
                self.want_faces();
                self.name_emoji();
                self.ask_who_is_around();
                let scene = self.scene();
                let size = self.size;
                let ground = self.palette.ground;
                let plain = self.plain;
                let (Some(surface), Some(view)) = (&self.surface, &mut self.view) else {
                    return;
                };
                let Ok(frame) = surface.get_current_texture() else {
                    return;
                };
                // Through the plain view, so the palette is not encoded twice.
                let target = frame.texture.create_view(&wgpu::TextureViewDescriptor {
                    format: Some(plain),
                    ..Default::default()
                });
                view.draw_scene(&target, &mut self.fonts, &scene, size, ground);
                frame.present();
                // A frame's worth of input has been acted on.
                self.input.settle();
            }
            _ => {}
        }
    }
}

/// Renders the feed to a file and exits, with no window and no GPU.
///
/// The same layout and the same draw list the window uses -- only the last step
/// differs -- so this is how the port gets checked on a machine with no display,
/// and how a page of real messages gets compared against the DOM list.
fn snapshot(path: &std::path::Path, width: u32) -> Result<(), String> {
    let mut fonts = Fonts::new();
    let mut painter = Painter::new();
    let palette = Palette::default();
    let theme = Theme {
        width: width as f32,
        ..Theme::default()
    };
    let (channel, rows) = App::feed();
    let laid: Vec<RowLayout> = rows
        .iter()
        .map(|row| lay_out(&mut fonts, row, &theme))
        .collect();
    let total: f32 = laid.iter().map(|row| row.height).sum();
    // Capped: a channel of four hundred messages is taller than any image
    // viewer wants, and the top of it is enough to judge the rendering.
    let height = total.min(4000.0).ceil().max(1.0) as u32;

    let mut canvas = matterless_paint::Canvas::new(width, height, palette.ground);
    let mut top = 0.0_f32;
    for row in &laid {
        if top > height as f32 {
            break;
        }
        painter.paint_row(&mut canvas, &mut fonts, row, top, &theme, &palette);
        top += row.height;
    }

    let file = std::fs::File::create(path).map_err(|error| error.to_string())?;
    let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), width, height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder
        .write_header()
        .map_err(|error| error.to_string())?
        .write_image_data(&canvas.pixels)
        .map_err(|error| error.to_string())?;
    println!(
        "{channel}: {} rows, {total:.0}px tall, written to {}",
        laid.len(),
        path.display()
    );
    Ok(())
}

fn main() {
    // `--snapshot <file>` instead of a window, for a headless check.
    let args: Vec<String> = std::env::args().collect();
    if let Some(at) = args.iter().position(|arg| arg == "--snapshot") {
        let path = args
            .get(at + 1)
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| std::path::PathBuf::from("list.png"));
        if let Err(why) = snapshot(&path, 1000) {
            eprintln!("snapshot: {why}");
            std::process::exit(1);
        }
        return;
    }

    // A loop that carries its own message type, so the socket thread can hand
    // work to the thread that owns the window rather than touching it.
    let events = EventLoop::<Update>::with_user_event()
        .build()
        .expect("an event loop");
    events.set_control_flow(ControlFlow::Wait);
    let mut app = App::new();
    app.connect(events.create_proxy());
    events.run_app(&mut app).expect("the event loop");
}

/// Wakes the window from the socket thread.
///
/// The proxy is the only thing the two threads share, and it carries an
/// already-decided update rather than a lock on anything the window draws from.
struct Proxy(winit::event_loop::EventLoopProxy<Update>);

impl matterless_view::live::Wake for Proxy {
    fn wake(&self, update: Update) {
        // Fails only once the loop has exited, which is not worth reporting on
        // the way out.
        let _ = self.0.send_event(update);
    }
}

/// The keys this app acts on, from what the platform reported.
///
/// Deliberately partial: everything else is text, and text arrives already
/// composed. A letter is mapped too, but only so a chord like Ctrl+C can be
/// recognised -- it is never how typing gets in.
fn named(key: &winit::keyboard::Key) -> Option<Key> {
    use winit::keyboard::{Key as Pressed, NamedKey};
    Some(match key {
        Pressed::Named(NamedKey::Enter) => Key::Enter,
        Pressed::Named(NamedKey::Escape) => Key::Escape,
        Pressed::Named(NamedKey::Tab) => Key::Tab,
        Pressed::Named(NamedKey::Backspace) => Key::Backspace,
        Pressed::Named(NamedKey::Delete) => Key::Delete,
        Pressed::Named(NamedKey::ArrowLeft) => Key::Left,
        Pressed::Named(NamedKey::ArrowRight) => Key::Right,
        Pressed::Named(NamedKey::ArrowUp) => Key::Up,
        Pressed::Named(NamedKey::ArrowDown) => Key::Down,
        Pressed::Named(NamedKey::Home) => Key::Home,
        Pressed::Named(NamedKey::End) => Key::End,
        Pressed::Named(NamedKey::PageUp) => Key::PageUp,
        Pressed::Named(NamedKey::PageDown) => Key::PageDown,
        // Lowercased so a chord is the same key with or without shift.
        Pressed::Character(text) => Key::Char(text.chars().next()?.to_ascii_lowercase()),
        _ => return None,
    })
}

/// What a thread panel answers to. Keyed by root so reopening the same thread
/// can be told from opening a different one.
fn thread_name(root_id: &str) -> String {
    format!("thread/{root_id}")
}
