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
}

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
        self.composer.above(header::below(self.channel_rect()))
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
        Some(self.thread_composer.above(self.thread_body()?))
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
            }
        }
        self.redraw();
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
        for key in wanted {
            if self.asked.insert(key.clone()) {
                link.send(matterless_view::live::Ask::Fetch { key });
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
        ) {
            Ok(rows) => {
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
        boxes.extend(self.stream.boxes(self.stream_rect()));
        if let (Some(thread), Some(rect)) = (&self.thread, self.thread_stream_rect()) {
            boxes.extend(thread.boxes(rect));
        }
        boxes
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
        ) {
            Ok(rows) => {
                self.stream.rows = rows;
                // A thread from the channel just left has nothing to do with
                // the one just opened.
                self.thread = None;
                self.relayout();
                let within = self.stream_rect();
                self.stream.to_bottom(within);
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
        self.sidebar.draw(&mut canvas, &boxes, sidebar, &self.input);

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

impl ApplicationHandler<Update> for App {
    fn resumed(&mut self, events: &ActiveEventLoop) {
        let window = Arc::new(
            events
                .create_window(
                    Window::default_attributes()
                        .with_title("MatterLess -- list on Vulkan")
                        .with_inner_size(winit::dpi::LogicalSize::new(
                            self.size.0 as f64,
                            self.size.1 as f64,
                        )),
                )
                .expect("a window"),
        );

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
                if let Some(root) = self.stream.react(&self.input, &boxes, stream) {
                    self.open_thread(&root);
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
