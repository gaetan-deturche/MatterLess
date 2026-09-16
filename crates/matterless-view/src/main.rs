//! A window showing the message list, drawn on Vulkan.
//!
//! Standalone on purpose. It is the same layout and the same draw list the app
//! will use, in a window of its own, so the rendering can be judged before it
//! replaces anything -- and so a scroll can be tried against the thing that was
//! meant to fix scrolling.
//!
//! Run it with `cargo run -p matterless-view`.

// Keeps the console window from appearing behind the window on Windows release
// builds; dev builds keep it, so the diagnostics this prints are visible.
//
// The app has carried this since it was written. Without it a release build
// opens a black console first, with a taskbar button of its own, and the
// window arrives behind it.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

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
            post: post("ada", vec![para("Yo!")]),
        });
        rows.push(Row::Continuation {
            post: post(
                "ada",
                vec![para(
                    "Could we raise the cache size the test runner is allowed, to a couple of \
                     gigabytes or so? It looks as though it is set in the file beside the \
                     runner rather than anywhere obvious, and I would rather not guess at it \
                     on a machine everybody shares.",
                )],
            ),
        });
        rows.push(Row::Post {
            post: post(
                "ben",
                vec![
                    Node::Paragraph {
                        children: vec![
                            Node::Text {
                                value: "that is ".into(),
                            },
                            Node::Strong {
                                children: vec![Node::Text {
                                    value: "a fair question".into(),
                                }],
                            },
                            Node::Text {
                                value: " -- ask ".into(),
                            },
                            Node::UserMention {
                                username: "cara".into(),
                                everyone: false,
                            },
                            Node::Text {
                                value: ", she set it up".into(),
                            },
                        ],
                    },
                    Node::List {
                        ordered: false,
                        items: vec![
                            vec![para("we can reach the runner itself")],
                            vec![para("but not the box its cache lives on")],
                        ],
                    },
                    Node::CodeBlock {
                        language: Some("php".into()),
                        value: "cache_size = "2GiB"\nkeep_days = 14".into(),
                    },
                ],
            ),
        });
    }
    rows
}

/// The strip of teams down the far left.
///
/// Its own column rather than a row inside the sidebar: a team is a different
/// kind of thing from a conversation, and a list holding both means two things
/// at once.
const RAIL: f32 = 48.0;
/// The sidebar's width. Fixed, as it is in the app today.
const SIDEBAR: f32 = 260.0;
/// The thread pane's width when one is open.
const THREAD: f32 = 420.0;
/// What the thread pane's reply box answers to.
const THREAD_COMPOSER: &str = "thread-composer";

/// Draws frames on its own, so a measurement does not need a hand on a wheel.
///
/// The window renders on demand: it waits, and a frame happens because
/// something arrived or somebody moved. That is right for a client and wrong
/// for measuring one -- the first attempt to find out where a frame goes was
/// read off whatever the server happened to send, and the sample only grew
/// when somebody scrolled.
///
/// `MATTERLESS_FRAMES=<n>` draws n frames back to back instead, prints what
/// they cost and quits. The first `WARM` of them are thrown away: a window's
/// opening frames carry the fonts being read and the atlas being filled, which
/// are real costs and are not what a frame costs.
struct Driver {
    left: u32,
    warm: u32,
}

impl Driver {
    /// How many frames to draw before the tally is started.
    const WARM: u32 = 30;

    fn asked() -> Option<Self> {
        let left = std::env::var("MATTERLESS_FRAMES")
            .ok()?
            .parse::<u32>()
            .ok()?;
        println!(
            "drawing {left} frames after {} warm (MATTERLESS_FRAMES)",
            Self::WARM
        );
        Some(Self {
            left,
            warm: Self::WARM,
        })
    }

    /// Counts the frame just drawn. `false` once there are none left to draw.
    fn drew(&mut self) -> bool {
        if self.warm > 0 {
            self.warm -= 1;
            // Everything up to here was the window opening, not a frame.
            if self.warm == 0 {
                matterless_view::timing::forget();
            }
            return true;
        }
        self.left = self.left.saturating_sub(1);
        self.left > 0
    }
}

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
    /// Set only by `MATTERLESS_FRAMES`, and the reason the window stops
    /// waiting between frames.
    driver: Option<Driver>,
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
    said_typing: matterless_view::typing::Sending,
    /// Who is around, and the set last asked about.
    presence: std::collections::HashMap<String, String>,
    asked_about: Vec<String>,
    /// Saved or pinned messages, whichever was last asked for. An aside,
    /// read against whatever conversation is open.
    listing: matterless_view::listing::Listing,
    /// The threads this reader follows, which is a place to go rather than an
    /// aside: it fills the conversation's own column, chosen from the sidebar
    /// like a channel.
    followed: matterless_view::listing::Listing,
    /// Who somebody is, when their name has been pressed.
    profile: matterless_view::profile::Profile,
    /// The teams, down the far left.
    rail: matterless_view::rail::Rail,
    /// Where this window is signed in, for building a link to a message.
    server: String,
    /// How this reader reads threads, which decides both what the sidebar
    /// counts and whether a reply is a row in the channel.
    threads: matterless_core::model::ThreadMode,
    /// Where the reader had got to in the open channel *before* they opened
    /// it, which is where the "New messages" divider goes.
    ///
    /// Held rather than read each time the channel is replanned: opening a
    /// channel marks it read, which moves the watermark in the store to the
    /// newest post. Read afresh it would always be the end of the
    /// conversation, and the divider would be placed below everything and so
    /// never drawn.
    viewed_at: i64,
    /// How long the divider has left before it goes, once the reader has had
    /// a chance to look at what it marks.
    rest: matterless_view::rest::Rest,
    /// When the window started shaping the top of the channel behind itself,
    /// and how much there was of it. For the line it prints when it is done.
    behind: Option<(std::time::Instant, usize)>,
    /// Which channel `viewed_at` was read for, so re-opening the one already
    /// open leaves it alone.
    ///
    /// A channel is opened again for reasons that have nothing to do with the
    /// reader: the socket signing in refreshes the membership, which replans
    /// the conversation because a thread mode decides what is a row in it.
    /// Without this the divider appeared for a second and then went, every
    /// time -- read once when the watermark was still the reader's, read again
    /// a moment later when it was the newest post.
    viewed_in: String,
    /// How far back the open channel is read. Grows as older pages arrive.
    depth: u32,
    /// True while a page of history is in flight, so the same page is not asked
    /// for once per frame while the reader sits at the top.
    loading_older: bool,
    /// False once the beginning of the channel has been reached.
    more_history: bool,
    /// A newer build, once the release has said there is one.
    offered: matterless_view::updater_bar::Bar,
    /// What the offered release said about itself, while it is being read.
    whats_new: matterless_view::whats_new::WhatsNew,
    /// What it is offering, kept so accepting it needs no second look.
    offer: Option<matterless_view::update::Offer>,
    /// The notification area, which is what lets the window be shut.
    tray: matterless_view::tray::Tray,
    /// A way to wake the window from the tray's own window procedure.
    waker: Option<winit::event_loop::EventLoopProxy<Update>>,
    /// The taskbar button: its overlay badge, and its flashing.
    taskbar: matterless_view::taskbar::Taskbar,
    /// Whether this window has the keyboard, which decides both whether an
    /// arriving message interrupts and whether the button should be flashing.
    focused: bool,
    /// What the button under the pointer is for, once it has been rested on.
    tooltip: matterless_view::tooltip::Tooltip,
    /// The picture a reader has opened, over everything else.
    viewer: matterless_view::viewer::Viewer,
    /// The menu currently open, whichever of the two it is.
    ///
    /// One at a time and one field: a right-click on the sidebar while the
    /// message menu is open should replace it, not stack a second one over it.
    menu: matterless_view::menu::Menu,
}

/// How many followed threads the list holds. Well past what anybody reads in
/// one sitting, and the store answers instantly either way.
const THREADS: u32 = 200;

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
        let opening = matterless_view::timing::watch("opening the store", 0, "");
        let store = matterless_view::feed::default_store()
            .and_then(|path| matterless_view::feed::open(&path).ok())
            .map(Arc::new);
        drop(opening);
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
        let listing = matterless_view::timing::watch("building the sidebar", 0, "");
        let entries = Self::entries(
            store.as_deref(),
            &me,
            matterless_core::model::ThreadMode::Collapsed,
            ("", "", false),
        );
        let sidebar = Sidebar::new(entries);
        drop(listing);
        let reading = matterless_view::timing::watch("reading the first channel", 0, "");
        let (channel, rows) = Self::feed();
        drop(reading);
        let mut app = Self {
            driver: Driver::asked(),
            window: None,
            surface: None,
            view: None,
            format: wgpu::TextureFormat::Bgra8UnormSrgb,
            plain: wgpu::TextureFormat::Bgra8Unorm,
            size: (1000, 760),
            fonts: {
                // Building this scans the system's font directories.
                let _loading = matterless_view::timing::watch("loading the fonts", 0, "");
                Fonts::new()
            },
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
                // The window opens at the newest message, so the rest of the
                // conversation is shaped behind it rather than before the
                // window can be drawn at all.
                stream.plan(rows, true);
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
                reply.placeholder =
                    "Reply... (Enter to send, Shift+Enter for a new line)".to_string();
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
            said_typing: matterless_view::typing::Sending::default(),
            presence: std::collections::HashMap::new(),
            asked_about: Vec::new(),
            listing: matterless_view::listing::Listing::default(),
            viewer: matterless_view::viewer::Viewer::default(),
            followed: matterless_view::listing::Listing::new("followed"),
            profile: matterless_view::profile::Profile::default(),
            rail: matterless_view::rail::Rail::default(),
            server: String::new(),
            // Until the server is asked. Collapsed is what this deployment
            // uses and what every other part of the window assumed outright,
            // so it is the assumption made once and in the open.
            threads: matterless_core::model::ThreadMode::Collapsed,
            viewed_at: 0,
            viewed_in: String::new(),
            rest: matterless_view::rest::Rest::default(),
            behind: None,
            depth: matterless_view::feed::PAGE,
            loading_older: false,
            more_history: true,
            menu: matterless_view::menu::Menu::default(),
            tooltip: matterless_view::tooltip::Tooltip::default(),
            offered: matterless_view::updater_bar::Bar::default(),
            whats_new: matterless_view::whats_new::WhatsNew::default(),
            offer: None,
            tray: matterless_view::tray::Tray::default(),
            waker: None,
            taskbar: matterless_view::taskbar::Taskbar::default(),
            // Assumed until the platform says otherwise, which it does on the
            // first focus event: a window that starts out believing it is
            // ignored would flash at the first message.
            focused: true,
        };
        app.sidebar.selected = Some(channel);
        // Focused before anything is clicked: a chat window that needs a click
        // before it will accept typing is a chat window that feels broken.
        app.input.focus_on(composer::NAME);
        app
    }

    /// The window as boxes: a notice across the top when there is one, then a
    /// fixed sidebar, the channel column, and the thread pane beside it when
    /// one is open.
    fn shell(&self) -> Vec<Placed> {
        let mut row = Boxed::new("shell", Size::Grow(1.0))
            .axis(Axis::Row)
            .with(Boxed::new("rail", Size::Fixed(RAIL)))
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
        // The notice takes its own room off the top rather than covering
        // anything: what it is interrupting is the thing the reader came for.
        let whole = Boxed::new("window", Size::Grow(1.0))
            .axis(Axis::Column)
            .with(Boxed::new(
                matterless_view::updater_bar::NAME,
                Size::Fixed(self.offered.height()),
            ))
            .with(row);
        matterless_ui::solve::solve(
            &whole,
            Rect::new(0.0, 0.0, self.size.0 as f32, self.size.1 as f32),
        )
    }

    /// The strip across the top, which is empty when nothing is offered.
    fn notice_rect(&self) -> Rect {
        Rect::new(0.0, 0.0, self.size.0 as f32, self.offered.height())
    }

    /// Everything under the notice, which is the whole window when there is
    /// none. What every pane measures itself against.
    fn below_notice(&self) -> Rect {
        let taken = self.offered.height();
        Rect::new(
            0.0,
            taken,
            self.size.0 as f32,
            (self.size.1 as f32 - taken).max(0.0),
        )
    }

    /// The whole window, which is what a floating panel is clamped inside.
    fn window_rect(&self) -> Rect {
        Rect::new(0.0, 0.0, self.size.0 as f32, self.size.1 as f32)
    }

    fn rail_rect(&self) -> Rect {
        let under = self.below_notice();
        Rect::new(under.x, under.y, RAIL, under.height)
    }

    fn sidebar_rect(&self) -> Rect {
        let under = self.below_notice();
        Rect::new(under.x + RAIL, under.y, SIDEBAR, under.height)
    }

    /// Everything right of the sidebar, thread pane included.
    fn column_rect(&self) -> Rect {
        let under = self.below_notice();
        Rect::new(
            under.x + RAIL + SIDEBAR,
            under.y,
            (under.width - RAIL - SIDEBAR).max(0.0),
            under.height,
        )
    }

    /// The column on the right, when one of the lists is open.
    ///
    /// Search, saved, pinned and threads all live here; only one at a time,
    /// because they are all answers to "show me messages from somewhere else"
    /// and two of them side by side would be two answers to one question.
    fn aside_rect(&self) -> Option<Rect> {
        (self.search.open || self.listing.open())
            .then(|| matterless_view::aside::rect(self.column_rect()))
    }

    /// The thread pane's own column, when a thread is open.
    fn thread_rect(&self) -> Option<Rect> {
        let column = self.column_rect();
        self.thread.as_ref()?;
        // Both at once is one pane too many for any window this runs in, so
        // the list wins -- it is the one that was just asked for. The thread
        // is not closed, only hidden: shutting the list brings it back where
        // the reader left it.
        if self.aside_rect().is_some() {
            return None;
        }
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

    /// The channel's column: everything right of the sidebar that the list or
    /// the thread pane has not taken.
    fn channel_rect(&self) -> Rect {
        let column = self.column_rect();
        match self.aside_rect().or_else(|| self.thread_rect()) {
            Some(taken) => Rect::new(column.x, column.y, taken.x - column.x, column.height),
            None => column,
        }
    }

    /// Whether the conversation's column is showing the followed threads
    /// rather than a conversation.
    fn on_threads(&self) -> bool {
        self.followed.open()
    }

    /// The threads list's own column: everything under the header.
    ///
    /// No room kept for a composer, because there is nothing here to write to
    /// -- a thread is replied to by opening it.
    fn followed_rect(&self) -> Rect {
        header::below(self.channel_rect())
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
        if self.on_threads() {
            return "Threads".to_string();
        }
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
    fn entries(
        store: Option<&matterless_store::Store>,
        me: &str,
        threads: matterless_core::model::ThreadMode,
        who: (&str, &str, bool),
    ) -> Vec<Entry> {
        let groups = store
            .map(|store| matterless_view::sidebar_feed::groups(store, me, threads))
            .unwrap_or_default();
        // Without a reader there is no membership row, and the store answers
        // with each channel's *total* message count -- which looks like an
        // unread badge of four thousand.
        let counted = !me.is_empty();
        let (name, status, live) = who;
        // When each of these people last changed their picture, in one query
        // rather than one per row: a hundred and fifteen conversations is a
        // hundred and fifteen round trips to answer the same question, and it
        // is asked again every time the sidebar is rebuilt.
        //
        // The version is part of what the picture is called, so it has to be
        // the same one the conversation uses -- otherwise the same face is
        // fetched twice and held twice under two names.
        let faces: std::collections::HashMap<String, i64> = {
            let mut wanted: Vec<String> = groups
                .iter()
                .flat_map(|group| group.channels.iter())
                .filter_map(|channel| channel.counterpart_id.clone())
                .collect();
            wanted.sort();
            wanted.dedup();
            store
                .and_then(|store| store.users_by_ids(&wanted).ok())
                .map(|found| {
                    found
                        .into_iter()
                        .map(|(id, user)| (id, user.last_picture_update))
                        .collect()
                })
                .unwrap_or_default()
        };
        // Said once, at the top, rather than in whichever channel happens to
        // be open: who the reader is has nothing to do with which conversation
        // they are reading.
        let mut entries: Vec<Entry> = vec![Entry::Me {
            name: name.to_string(),
            status: status.to_string(),
            live,
        }];
        // First in the list, because with collapsed threads a reply never
        // touches its channel's counters: this is the only row in the sidebar
        // that can say a thread is waiting.
        if counted {
            let (unread, mentions) = store
                .and_then(|store| store.thread_unread_totals().ok())
                .unwrap_or((0, 0));
            entries.push(Entry::Threads { unread, mentions });
        }
        let row = |channel: matterless_sidebar::ChannelSummary| Entry::Channel {
            counterpart_avatar_at: channel
                .counterpart_id
                .as_deref()
                .and_then(|who| faces.get(who).copied())
                .unwrap_or(0),
            direct: channel.channel_type == "D" || channel.channel_type == "G",
            private: channel.channel_type == "P",
            // Only a one-to-one has a single other person; a group has several
            // and no dot could stand for all of them.
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
        };

        // Whatever is unread, lifted out of wherever it lives and put at the
        // top. Muted channels stay put however much they hold: muting says
        // "do not interrupt me", and moving one to the front is interrupting.
        let mut groups = groups;
        if counted && store.is_some_and(|store| lifts_unreads(store, me)) {
            let mut waiting: Vec<matterless_sidebar::ChannelSummary> = Vec::new();
            for group in &mut groups {
                group.channels.retain(|channel| {
                    let asking = channel.unread > 0 && !channel.muted;
                    if asking {
                        waiting.push(channel.clone());
                    }
                    !asking
                });
            }
            if !waiting.is_empty() {
                waiting.sort_by_key(|channel| std::cmp::Reverse(channel.last_post_at));
                entries.push(Entry::Heading {
                    label: "Unreads".to_string(),
                    directs: false,
                });
                entries.extend(waiting.into_iter().map(row));
            }
        }

        // A team's name once, above the groups belonging to it, rather than on
        // each of them: two teams each bring a "Favorites" and a "Channels",
        // and unqualified they read as duplicates -- but qualifying every one
        // says the team's name four times down the list.
        let mut team = String::new();
        for group in groups {
            if group.channels.is_empty() {
                continue;
            }
            if group.team_name != team {
                team = group.team_name.clone();
                if !team.is_empty() {
                    entries.push(Entry::Team {
                        id: group.team_id.clone(),
                        label: team.clone(),
                    });
                }
            }
            entries.push(Entry::Heading {
                directs: group.category_type == "direct_messages",
                label: group.display_name,
            });
            entries.extend(group.channels.into_iter().map(row));
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
                    Entry::Heading { label, .. } | Entry::Team { label, .. } =>
                        Some(label.as_str()),
                    _ => None,
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
        self.rebuild_sidebar();
        // The counts were read from a store that may be a whole session old,
        // and nothing about them follows from the posts held here: they are
        // the server's arithmetic. Asked for before the channel is opened, so
        // the answer lands on a sidebar that is already drawn.
        if let Some(link) = self.link.as_ref() {
            link.send(matterless_view::live::Ask::Membership);
        }

        // Opened properly now that there is a reader and a connection. The
        // window draws a channel before either exists, so the one on screen at
        // startup had never been through `open_channel` -- and every single
        // thing that happens there was quietly missing for it. Its custom
        // emoji went unresolved, it was never reconciled with the server, and
        // it was never marked read, so its unread count climbed for as long as
        // the reader kept looking at it. Three separate bugs from one path
        // that skipped the other.
        if let Some(channel) = self.sidebar.selected.clone() {
            self.open_channel(&channel);
        }
    }

    /// Opens the socket, if there is a store and a session to open it with.
    ///
    /// Refusing to start is not a failure worth stopping for: the window still
    /// draws everything the database holds, which is what it did before there
    /// was a socket at all.
    fn connect(&mut self, proxy: winit::event_loop::EventLoopProxy<Update>) {
        // Kept for the tray, which has to reach the window from a window
        // procedure of its own.
        self.waker = Some(proxy.clone());
        // Looked for once, here, rather than on a timer: an update is not
        // urgent, and asking again mid-session would interrupt reading to talk
        // about the client rather than about anything the reader came for.
        let looking = matterless_view::update::asks();
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
        self.server = server.clone();
        let link = matterless_view::live::start(store, server, token, Proxy(proxy));
        if looking {
            link.send(matterless_view::live::Ask::LookForUpdate);
        } else {
            println!("no update check in a dev build");
            self.pretend_offered();
        }
        self.link = Some(link);
    }

    /// Puts a made-up offer on screen, in a dev build, when asked.
    ///
    /// A dev build never looks for an update, which means the one piece of
    /// interface nobody can ever see while working on it is the one that asks
    /// somebody to restart. `MATTERLESS_OFFER=0.1.6` puts the card up, and
    /// `MATTERLESS_OFFER_NOTES=<file>` gives it a change list to show.
    ///
    /// Nothing is installable: the offer has no real url, so accepting it
    /// fails -- which is the other half of the card worth looking at.
    fn pretend_offered(&mut self) {
        let Some(version) = std::env::var("MATTERLESS_OFFER")
            .ok()
            .filter(|it| !it.is_empty())
        else {
            return;
        };
        let notes = std::env::var("MATTERLESS_OFFER_NOTES")
            .ok()
            .and_then(|path| std::fs::read_to_string(path).ok())
            .unwrap_or_default();
        println!("pretending {version} is available (MATTERLESS_OFFER)");
        self.apply(Update::Updatable(matterless_view::update::Offer {
            version,
            url: String::new(),
            signature: String::new(),
            notes,
        }));
    }

    /// Applies what the socket reported.
    fn apply(&mut self, update: Update) {
        match update {
            Update::Connected(up) => {
                self.connected = up;
                println!("socket {}", if up { "connected" } else { "lost" });
                // The strip at the top of the sidebar says so, so it has to be
                // rebuilt to stop saying the opposite.
                self.rebuild_sidebar();
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
            Update::Discovered { query, found } => self.switcher.offer(&query, found),
            Update::Reached { channel_id } => {
                self.sidebar.selected = Some(channel_id.clone());
                self.open_channel(&channel_id);
            }
            Update::Membership(mode) => {
                // Only when it actually moved. This arrives once per sign-in
                // carrying the mode the window is already in, and replanning
                // on it shaped the open channel a second time for nothing --
                // a whole second, in a channel of crash reports, a second
                // after the first one.
                let moved = self.threads != mode;
                self.threads = mode;
                self.rebuild_sidebar();
                // The counts are not the only thing the mode decides: a reply
                // is a row in the channel under one and not under the other,
                // so the conversation is replanned when it does change.
                if moved && let Some(channel) = self.sidebar.selected.clone() {
                    self.open_channel(&channel);
                }
            }
            Update::Statuses(found) => {
                let mine = found.iter().any(|(user_id, _)| user_id == &self.me);
                for (user_id, status) in found {
                    self.presence.insert(user_id, status);
                }
                // The reader's own presence is on the strip, so learning it is
                // a reason to draw the strip again.
                if mine {
                    self.rebuild_sidebar();
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
                // looking at that conversation -- which means the window, and
                // by now it may be hidden in the tray rather than merely
                // behind something.
                self.raise();
                self.sidebar.selected = Some(channel_id.clone());
                self.open_channel(&channel_id);
            }
            Update::Picture {
                key,
                width,
                height,
                rgba,
            } => self.arrived.push((key, width, height, rgba)),
            // Straight to a texture of its own rather than into the atlas --
            // and only if it is still the one being looked at, since a reader
            // flicking through outruns the network.
            Update::Looked {
                file_id,
                width,
                height,
                rgba,
            } => {
                if self
                    .viewer
                    .current()
                    .is_some_and(|one| one.file_id == file_id)
                    && let Some(view) = self.view.as_mut()
                {
                    view.show(width, height, &rgba);
                    self.viewer.arrived(&file_id, (width, height));
                }
            }
            Update::LookFailed { file_id, why } => {
                eprintln!("looking at {file_id}: {why}");
                self.viewer.gave_up(&file_id, &why);
            }
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
                // The counts in the sidebar are arithmetic on two numbers the
                // server keeps, and they move without a single post arriving:
                // reading a channel here, or on a phone, changes them. Rebuilt
                // whenever one does, or a badge stays on screen after the
                // messages behind it have been read.
                if deltas
                    .iter()
                    .any(|delta| matches!(delta, matterless_sync::Delta::UnreadChanged { .. }))
                {
                    self.rebuild_sidebar();
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
            Update::Updatable(offer) => {
                self.offered.offer(&offer.version, &offer.notes);
                self.offer = Some(offer);
                let notice = self.notice_rect();
                self.offered.measure(&mut self.fonts, notice);
                // Everything under it is a row shorter now, which is a
                // question for the layout rather than for this.
                self.relayout();
            }
            Update::UpdateFailed(why) => {
                self.offered.failed(&why);
                let window = self.notice_rect();
                self.offered.measure(&mut self.fonts, window);
            }
            // Answered before this, in `user_event`, because it is the one
            // update that can end the loop and this has no way to say so.
            Update::Tray(_) => {}
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
        // The reader themselves, because their own presence is on the strip
        // at the top of the sidebar and nothing else would ask for it.
        wanted.push(self.me.clone());
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
    /// The rule is `typing::Sending`. Asked *after* the conversation is known,
    /// or a keystroke with no socket to send on would count as having sent.
    fn say_typing(&mut self, root_id: &str) {
        let (Some(channel), Some(link)) = (self.sidebar.selected.clone(), self.link.as_ref())
        else {
            return;
        };
        if !self
            .said_typing
            .due(&channel, root_id, std::time::Instant::now())
        {
            return;
        }
        link.send(matterless_view::live::Ask::Typing {
            channel_id: channel,
            root_id: root_id.to_string(),
        });
    }

    /// Follows something somebody wrote: a link, a person, a conversation.
    fn press(&mut self, press: matterless_layout::row::Press) {
        use matterless_layout::row::Press;
        match press {
            Press::Link(href) => {
                matterless_view::open::link(&href);
            }
            // By name, because that is all a `~channel` in a message carries.
            // A name this store has never met is not an error worth a dialog:
            // it is a channel the reader is not in.
            Press::Channel(name) => {
                match self
                    .store
                    .as_ref()
                    .and_then(|store| store.channel_by_name(&name).ok().flatten())
                {
                    Some(channel) => {
                        self.sidebar.selected = Some(channel.id.clone());
                        self.open_channel(&channel.id);
                    }
                    None => println!("no channel called {name} in the local store"),
                }
            }
            Press::Person(username) => self.show_profile(&username),
            // Inside the app rather than out of it: the message is on this
            // server and in this store, so following a quoted card is a scroll
            // and not a browser.
            Press::Post {
                channel_id,
                post_id,
            } => {
                if self.sidebar.selected.as_deref() != Some(channel_id.as_str()) {
                    self.sidebar.selected = Some(channel_id.clone());
                    self.open_channel(&channel_id);
                }
                let within = self.stream_rect();
                if !self.stream.to_post(&mut self.fonts, &post_id, within) {
                    // Older than the pages loaded so far. Saying so beats
                    // leaving the reader somewhere arbitrary and silent.
                    println!("{post_id} is further back than this channel is loaded");
                }
            }
        }
    }

    /// Opens a card on somebody, from what the store already holds.
    ///
    /// Anchored to the words that were pressed, which the stream recorded when
    /// it drew them -- the card has to point at the name it is about, and a
    /// mention appears many times in a conversation.
    fn show_profile(&mut self, username: &str) {
        let Some(user) = self
            .store
            .as_ref()
            .and_then(|store| store.user_by_username(username).ok().flatten())
        else {
            println!("nobody called {username} in the local store");
            return;
        };
        let near = self
            .stream
            .pressed_rect(&matterless_layout::row::Press::Person(username.to_string()))
            .unwrap_or_else(|| self.stream_rect());
        self.profile.show(
            matterless_view::profile::Card {
                full_name: format!("{} {}", user.first_name, user.last_name)
                    .trim()
                    .to_string(),
                // This client does not model a job title, so the nickname
                // is what it has that is worth a line. Empty far more often
                // than not, which the card is built to handle.
                nickname: user.nickname.clone(),
                status: self.presence.get(&user.id).cloned().unwrap_or_default(),
                avatar_at: user.last_picture_update,
                username: user.username,
                user_id: user.id,
            },
            near,
        );
        self.want_faces();
    }

    /// What the strip offers for the conversation on screen.
    ///
    /// A direct message cannot be left -- the server would refuse -- so the
    /// button is not there rather than there and refused.
    fn header_offers(&self) -> Vec<header::Act> {
        let direct = self
            .sidebar
            .selected
            .as_ref()
            .and_then(|id| self.store.as_ref()?.channel(id).ok().flatten())
            .is_some_and(|channel| channel.channel_type == "D" || channel.channel_type == "G");
        header::offered(direct)
    }

    /// Does what a button inside a composer says.
    ///
    /// Attach asks the platform for a file, which this window cannot do yet --
    /// so it says what it would need rather than doing nothing and leaving the
    /// reader to wonder whether the press registered.
    fn pressed_in_composer(&mut self, button: composer::Button, root_id: &str) {
        match button {
            composer::Button::Send => {
                let box_of = if root_id.is_empty() {
                    &mut self.composer
                } else {
                    &mut self.thread_composer
                };
                let text = box_of.text().trim().to_string();
                if text.is_empty() {
                    return;
                }
                box_of.clear(&mut self.fonts);
                if let Some(channel) = self.sidebar.selected.clone() {
                    self.post_message(&channel, root_id, text);
                }
            }
            composer::Button::Attach => {
                println!("drop a file on the window to send it");
            }
        }
    }

    /// The reader's own name, as the store knows it.
    fn my_name(&self) -> String {
        self.store
            .as_ref()
            .and_then(|store| store.users_by_ids(std::slice::from_ref(&self.me)).ok())
            .and_then(|known| known.get(&self.me).map(|user| user.username.clone()))
            .unwrap_or_else(|| "signing in".to_string())
    }

    /// Whether the reader follows the thread on screen.
    ///
    /// From the store rather than remembered here: following is changed from
    /// this window and from every other client the reader has.
    fn following_open_thread(&self) -> bool {
        let (Some(root), Some(store)) = (self.open_root(), self.store.as_ref()) else {
            return false;
        };
        store
            .thread_states_for(std::slice::from_ref(&root))
            .unwrap_or_default()
            .get(&root)
            .is_some_and(|state| state.following)
    }

    /// Whether the conversation on screen is muted, as the sidebar has it.
    fn muted(&self) -> bool {
        let Some(open) = self.sidebar.selected.as_deref() else {
            return false;
        };
        self.sidebar.entries.iter().any(|entry| {
            matches!(
                entry,
                matterless_view::sidebar::Entry::Channel { id, muted, .. }
                    if id == open && *muted
            )
        })
    }

    /// Which strip button the pointer is on, from the boxes just placed.
    fn on_header(&self) -> Option<header::Act> {
        self.input
            .hovered()
            .and_then(|name| name.strip_prefix("header/"))
            .and_then(header::Act::from_slug)
    }

    /// Does what a strip button says.
    fn act_on_header(&mut self, act: header::Act) {
        match act {
            // The same place the sidebar's own row goes, rather than a second
            // way of showing the same list somewhere else.
            header::Act::Threads => {
                self.sidebar.selected = Some(matterless_view::sidebar::THREADS.to_string());
                self.open_channel(matterless_view::sidebar::THREADS);
            }
            header::Act::Saved => {
                self.listing.expect("Saved");
                if let Some(link) = self.link.as_ref() {
                    link.send(matterless_view::live::Ask::Saved);
                }
            }
            header::Act::Pinned => {
                if let (Some(channel), Some(link)) =
                    (self.sidebar.selected.clone(), self.link.as_ref())
                {
                    self.listing.expect("Pinned");
                    link.send(matterless_view::live::Ask::Pinned {
                        channel_id: channel,
                    });
                }
            }
            header::Act::Close => self.close_thread(),
            header::Act::Follow => {
                let Some(root) = self.open_root() else {
                    return;
                };
                let following = self.following_open_thread();
                if let Some(link) = self.link.as_ref() {
                    link.send(matterless_view::live::Ask::Follow {
                        root_id: root,
                        following: !following,
                    });
                }
            }
            header::Act::Add => {
                let Some(channel) = self.sidebar.selected.clone() else {
                    return;
                };
                let mut input = std::mem::take(&mut self.input);
                self.switcher.show(&mut self.fonts, &mut input);
                self.switcher
                    .instead(matterless_view::switcher::Asking::Add(channel));
                self.input = input;
            }
            header::Act::Mute => {
                let muted = self.muted();
                if let (Some(channel), Some(link)) =
                    (self.sidebar.selected.clone(), self.link.as_ref())
                {
                    link.send(matterless_view::live::Ask::Mute {
                        channel_id: channel,
                        // What it becomes, decided here rather than waited
                        // for: the button has to change the instant it is
                        // pressed, as the reaction pills do.
                        muted: !muted,
                    });
                }
            }
            header::Act::Leave => {
                if let (Some(channel), Some(link)) =
                    (self.sidebar.selected.clone(), self.link.as_ref())
                {
                    link.send(matterless_view::live::Ask::Leave {
                        channel_id: channel,
                    });
                }
            }
        }
    }

    /// A link to this message that will open anywhere.
    /// The menu behind a message's `...`.
    ///
    /// Built from the row on screen rather than from the store: what it offers
    /// has to agree with what the reader can see, and a row that has just been
    /// saved should say so before the server has answered.
    fn offer_message_menu(&mut self, post_id: &str, under: Rect) {
        use matterless_view::actions::Action;
        use matterless_view::menu::{Item, Style, Tint};
        let Some(post) = self
            .stream
            .rows
            .iter()
            .chain(self.thread.iter().flat_map(|thread| thread.rows.iter()))
            .find_map(|row| match row {
                Row::Post { post } | Row::Continuation { post } if post.post_id == post_id => {
                    Some(post)
                }
                _ => None,
            })
        else {
            return;
        };
        let mine = post.author_id == self.me;
        let said = |action: Action, on: bool| Item::new(action.slug(), action.said(on));
        let mut items = vec![
            said(Action::Follow, post.following),
            said(Action::Unread, false),
            said(Action::Save, post.saved),
            said(Action::Pin, post.pinned),
        ];
        if mine {
            // Only on the reader's own: the server refuses either on anybody
            // else's, and offering what will be refused is a worse answer than
            // not offering it.
            items.push(Item::rule());
            items.push(said(Action::Edit, false));
            items.push(said(Action::Delete, false).asks(vec![
                // Asked, not assumed: a delete cannot be undone and the item
                // above it is one slip away.
                Item::new("delete.now", "Delete").tinted(Tint::Danger),
                Item::new("delete.keep", "Keep"),
            ]));
        }
        items.push(said(Action::Forward, false));
        items.push(
            said(Action::Remind, false).asks(
                matterless_view::actions::reminders(matterless_view::clock::minutes_today())
                    .into_iter()
                    .map(|(label, seconds)| Item::new(&format!("remind.{seconds}"), label))
                    .collect(),
            ),
        );
        items.push(Item::rule());
        items.push(said(Action::CopyText, false));
        items.push(said(Action::Link, false));
        self.menu.show(
            post_id,
            matterless_view::menu::Anchor::Under(under),
            Style::message(),
            items,
        );
    }

    /// The menu a right-click on a channel row offers.
    ///
    /// Items that cannot apply are left out rather than drawn dead: a direct
    /// message has no members to add and cannot be left -- the server refuses
    /// -- and a row that never becomes available is a worse answer than no row.
    fn offer_channel_menu(&mut self) {
        use matterless_view::menu::{Item, Style, Tint};
        let Some(channel_id) = self
            .input
            .contexted()
            .and_then(|name| name.strip_prefix("sidebar/channel/"))
            .map(str::to_string)
        else {
            return;
        };
        let Some((x, y)) = self.input.pointer_at() else {
            return;
        };
        let Some(store) = self.store.as_deref() else {
            return;
        };
        let Ok(Some(channel)) = store.channel(&channel_id) else {
            return;
        };
        // A conversation rather than a channel: no membership to manage.
        let conversation = channel.channel_type == "D" || channel.channel_type == "G";
        let muted = self.sidebar.entries.iter().any(|entry| {
            matches!(entry, Entry::Channel { id, muted, .. } if *id == channel_id && *muted)
        });

        let categories = store.sidebar().unwrap_or_default();
        let kind = |wanted: &str| {
            categories
                .iter()
                .position(|(category, _)| {
                    category.category_type == wanted && category.team_id == channel.team_id
                })
                .map(|at| &categories[at])
        };
        let favourites = kind("favorites");
        // Where un-favouriting puts a channel back.
        let plain = kind("channels");
        let favourite = favourites.is_some_and(|(_, held)| held.contains(&channel_id));

        let mut items = vec![
            Item::new("channel.unread", "Mark as Unread").marked(matterless_layout::marks::UNREAD),
        ];
        if let (Some((favourites, _)), Some((plain, _))) = (favourites, plain) {
            let into = if favourite { &plain.id } else { &favourites.id };
            items.push(
                Item::new(
                    &format!("channel.move.{into}"),
                    if favourite {
                        "Remove from Favorites"
                    } else {
                        "Favorite"
                    },
                )
                .marked(matterless_layout::marks::FAVOURITE),
            );
        }
        items.push(
            Item::new(
                "channel.mute",
                if muted {
                    "Unmute Channel"
                } else {
                    "Mute Channel"
                },
            )
            .marked(matterless_layout::marks::BELL_OFF),
        );
        // Everywhere it could go, minus wherever it already is.
        let targets: Vec<Item> = categories
            .iter()
            .filter(|(category, held)| {
                category.team_id == channel.team_id
                    && category.category_type != "direct_messages"
                    && !held.contains(&channel_id)
            })
            .map(|(category, _)| {
                Item::new(
                    &format!("channel.move.{}", category.id),
                    &category.display_name,
                )
            })
            .collect();
        if !targets.is_empty() {
            items.push(Item::rule());
            items.push(
                Item::new("channel.move", "Move to\u{2026}")
                    .marked(matterless_layout::marks::FOLDER)
                    .nests(targets),
            );
        }
        items.push(Item::rule());
        items.push(Item::new("channel.link", "Copy Link").marked(matterless_layout::marks::LINK));
        if !conversation {
            items.push(
                Item::new("channel.add", "Add Members")
                    .marked(matterless_layout::marks::ADD_PEOPLE),
            );
            items.push(Item::rule());
            items.push(
                Item::new("channel.leave", "Leave Channel")
                    .marked(matterless_layout::marks::LEAVE)
                    .tinted(Tint::Flag),
            );
        }
        self.menu.show(
            &channel_id,
            matterless_view::menu::Anchor::At(x, y),
            Style::channel(),
            items,
        );
    }

    /// Runs whichever item was chosen.
    ///
    /// The channel menu's ids carry a prefix and the message menu's do not,
    /// which is what says which of the two answered: both offer a "Mark as
    /// Unread" and they mean different things by it.
    fn act_on_menu(&mut self, about: &str, chosen: &str) {
        use matterless_view::actions::Action;
        if let Some(rest) = chosen.strip_prefix("channel.") {
            self.act_on_channel_menu(about, rest);
            return;
        }
        // Answered by doing nothing, which is what keeping it means.
        if chosen == "delete.keep" {
            return;
        }
        if let Some(seconds) = chosen
            .strip_prefix("remind.")
            .and_then(|seconds| seconds.parse::<i64>().ok())
        {
            if let Some(link) = self.link.as_ref() {
                link.send(matterless_view::live::Ask::Remind {
                    post_id: about.to_string(),
                    when: matterless_view::clock::now() + seconds,
                });
            }
            return;
        }
        let action = match chosen {
            "delete.now" => Action::Delete,
            slug => match Action::from_slug(slug) {
                Some(action) => action,
                None => return,
            },
        };
        let post = self
            .stream
            .rows
            .iter()
            .chain(self.thread.iter().flat_map(|thread| thread.rows.iter()))
            .find_map(|row| match row {
                Row::Post { post } | Row::Continuation { post } if post.post_id == about => {
                    Some(post.clone())
                }
                _ => None,
            });
        match action {
            // What the toggle becomes, decided here rather than by the server:
            // the row has to change the instant it is chosen.
            Action::Save => {
                let on = !post.is_some_and(|post| post.saved);
                self.act(action, about.to_string(), on);
            }
            Action::Pin => {
                let on = !post.is_some_and(|post| post.pinned);
                self.act(action, about.to_string(), on);
            }
            Action::Follow => {
                let following = !post.as_ref().is_some_and(|post| post.following);
                let root = post
                    .map(|post| {
                        if post.root_id.is_empty() {
                            post.post_id
                        } else {
                            post.root_id
                        }
                    })
                    .unwrap_or_else(|| about.to_string());
                if let Some(link) = self.link.as_ref() {
                    link.send(matterless_view::live::Ask::Follow {
                        root_id: root,
                        following,
                    });
                }
            }
            Action::CopyText => match self
                .store
                .as_ref()
                .and_then(|store| store.post(about).ok().flatten())
            {
                // The raw markdown, not the rendered body: what was typed is
                // what somebody pasting it elsewhere means to carry.
                Some(said) => {
                    self.clipboard = said.message;
                    println!("copied a message");
                }
                None => eprintln!("no copy of {about} to read"),
            },
            other => self.act(other, about.to_string(), true),
        }
    }

    /// Runs one item of the channel menu.
    fn act_on_channel_menu(&mut self, channel_id: &str, chosen: &str) {
        use matterless_view::live::Ask;
        if let Some(category_id) = chosen.strip_prefix("move.") {
            let team_id = self
                .store
                .as_ref()
                .and_then(|store| store.channel(channel_id).ok().flatten())
                .map(|channel| channel.team_id)
                .unwrap_or_default();
            if let Some(link) = self.link.as_ref() {
                link.send(Ask::Move {
                    channel_id: channel_id.to_string(),
                    team_id,
                    category_id: category_id.to_string(),
                });
            }
            return;
        }
        match chosen {
            "unread" => {
                if let Some(link) = self.link.as_ref() {
                    link.send(Ask::Unseen {
                        channel_id: channel_id.to_string(),
                    });
                }
            }
            "mute" => {
                let muted = self.sidebar.entries.iter().any(|entry| {
                    matches!(entry, Entry::Channel { id, muted, .. } if id == channel_id && *muted)
                });
                if let Some(link) = self.link.as_ref() {
                    link.send(Ask::Mute {
                        channel_id: channel_id.to_string(),
                        muted: !muted,
                    });
                }
            }
            "link" => match self.store.as_deref().and_then(|store| {
                matterless_view::feed::channel_link(store, &self.server, channel_id)
            }) {
                Some(link) => {
                    self.clipboard = link;
                    println!("copied a channel link");
                }
                None => eprintln!("no team to build a link from"),
            },
            "add" => {
                // The same picker the strip's own button opens, so there is
                // one way of choosing somebody rather than two that drift.
                let mut input = std::mem::take(&mut self.input);
                self.switcher.show(&mut self.fonts, &mut input);
                self.switcher
                    .instead(matterless_view::switcher::Asking::Add(
                        channel_id.to_string(),
                    ));
                self.input = input;
            }
            "leave" => {
                if let Some(link) = self.link.as_ref() {
                    link.send(Ask::Leave {
                        channel_id: channel_id.to_string(),
                    });
                }
            }
            _ => {}
        }
    }

    /// Runs whatever a click on a conversation asked for.
    ///
    /// One place for both panels: the channel and the thread draw the same
    /// rows with the same controls, and two copies of this is how they would
    /// come to disagree about what pressing one of them does.
    fn acted(&mut self, chose: Option<matterless_view::stream::Chose>) {
        use matterless_view::stream::Chose;
        match chose {
            Some(Chose::Thread(root)) => self.open_thread(&root),
            Some(Chose::Retry(pending)) => self.retry(&pending),
            Some(Chose::Act {
                action,
                post_id,
                on,
            }) => self.act(action, post_id, on),
            Some(Chose::Press(press)) => self.press(press),
            Some(Chose::Save { file_id, name }) => {
                if let Some(link) = self.link.as_ref() {
                    link.send(matterless_view::live::Ask::Download { file_id, name });
                }
            }
            Some(Chose::React { post_id, emoji, on }) => {
                if let Some(link) = self.link.as_ref() {
                    link.send(matterless_view::live::Ask::React { post_id, emoji, on });
                }
            }
            Some(Chose::More { post_id, under }) => self.offer_message_menu(&post_id, under),
            Some(Chose::Look { file_id, post_id }) => self.look_at(&post_id, &file_id),
            None => {}
        }
    }

    /// Opens the conversation with these people, starting it if there is not
    /// one yet.
    ///
    /// One other person is a direct message and several are a group, which is
    /// the only difference between them: the reader picked who to talk to, and
    /// the number decides what kind of conversation that is. Both need the
    /// server before there is anything to open, so the window hears back
    /// rather than guessing an id.
    fn talk_to(&mut self, user_ids: Vec<String>) {
        let Some(link) = self.link.as_ref() else {
            return;
        };
        let sent = match user_ids.len() {
            0 => return,
            1 => link.send(matterless_view::live::Ask::Direct {
                user_id: user_ids[0].clone(),
            }),
            _ => link.send(matterless_view::live::Ask::Group { user_ids }),
        };
        if !sent {
            eprintln!("the socket is not up, so there is nobody to ask");
        }
    }

    /// Opens one of a message's pictures, full size.
    ///
    /// Every picture on the message goes to the viewer, not just the one
    /// pressed: stepping through them must not need the stream again, and
    /// whether there are others is what decides if it offers to.
    fn look_at(&mut self, post_id: &str, file_id: &str) {
        let all = self
            .stream
            .rows
            .iter()
            .chain(self.thread.iter().flat_map(|thread| thread.rows.iter()))
            .find_map(|row| match row {
                Row::Post { post } | Row::Continuation { post } if post.post_id == post_id => {
                    Some(matterless_view::stream::looking_at(post))
                }
                _ => None,
            })
            .unwrap_or_default();
        if let Some(one) = self.viewer.show(all, file_id) {
            self.fetch_looked(&one);
        }
    }

    /// Asks for the picture the viewer is showing, at the size this window can
    /// actually draw.
    fn fetch_looked(&self, one: &matterless_view::viewer::Looking) {
        let Some(link) = self.link.as_ref() else {
            return;
        };
        link.send(matterless_view::live::Ask::Look {
            file_id: one.file_id.clone(),
            original: one.original,
            within: self.size,
        });
    }

    /// What the box under the pointer is for.
    ///
    /// The port's answer to the `title` on nearly every control in the app.
    /// Each panel knows its own marks, so the words come from the module that
    /// draws them and this only says which panel a name belongs to.
    ///
    /// `None` for everything that explains itself. A tooltip over a sentence
    /// somebody wrote is an interruption, not help.
    fn explains(&self, name: &str) -> Option<String> {
        // The strip, whose buttons are the ones with nothing but a mark on
        // them: a pin, a bookmark, a bell and a door.
        if let Some(slug) = name.strip_prefix("header/") {
            if slug == "find" {
                return None;
            }
            let act = header::Act::from_slug(slug)?;
            let on = match act {
                header::Act::Mute => self.muted(),
                header::Act::Follow => self.following_open_thread(),
                _ => false,
            };
            return Some(act.explains(on).to_string());
        }
        // A team is its name, and the envelope is what it stands for.
        if let Some(id) = name.strip_prefix("rail/") {
            if id == matterless_view::rail::DIRECTS {
                return Some("Direct messages".to_string());
            }
            return self
                .rail
                .teams
                .iter()
                .find(|team| team.id == id)
                .map(|team| team.name.clone());
        }
        if name == "sidebar/me" {
            return Some("Your status".to_string());
        }
        self.explains_in(&self.stream, name).or_else(|| {
            self.thread
                .as_ref()
                .and_then(|thread| self.explains_in(thread, name))
        })
    }

    /// The same question for one conversation, since there can be two on
    /// screen and both name their boxes after themselves.
    fn explains_in(&self, stream: &Stream, name: &str) -> Option<String> {
        let rest = name.strip_prefix(&format!("{}/", stream.name))?;
        // A face on the quick row is the emoji it stands for, by name: a
        // picture of a face does not say what reacting with it means.
        if let Some(at) = rest.strip_prefix("faces/") {
            let at = at.parse::<usize>().ok()?;
            return Some(match matterless_view::actions::QUICK.get(at) {
                Some((emoji, _)) => format!(":{emoji}:"),
                None => "More reactions".to_string(),
            });
        }
        // A pressable run of words. A mention and a channel say where they
        // lead; a link says where it goes, which the app leaves to the
        // browser's status bar and this window has nowhere else to put.
        if let Some(at) = rest.strip_prefix("press/") {
            let at = at.parse::<usize>().ok()?;
            let (press, _) = stream.presses_seen().get(at)?;
            return Some(match press {
                matterless_layout::row::Press::Person(_) => "Show profile".to_string(),
                matterless_layout::row::Press::Channel(_) => "Go to channel".to_string(),
                matterless_layout::row::Press::Post { .. } => "Go to the message".to_string(),
                matterless_layout::row::Press::Link(href) => href.clone(),
            });
        }
        let (index, what) = rest.strip_prefix("row/")?.split_once('/')?;
        let index = index.parse::<usize>().ok()?;
        if let Some(slug) = what.strip_prefix("tool/") {
            return Some(
                matterless_view::actions::Tool::from_slug(slug)?
                    .explains()
                    .to_string(),
            );
        }
        let (Row::Post { post } | Row::Continuation { post }) = stream.rows.get(index)? else {
            return None;
        };
        // Who left a reaction, which is the whole reason a pill is worth
        // hovering: the count says how many and nothing says who.
        if let Some(ordinal) = what.strip_prefix("reaction/") {
            let reaction = post.reactions.get(ordinal.parse::<usize>().ok()?)?;
            return Some(matterless_view::tooltip::reacted_by(
                &reaction.emoji,
                reaction.count,
                &reaction.names,
            ));
        }
        // An attachment, by the name it was uploaded under: the card shows a
        // truncated name and the reader cannot widen it.
        if let Some(ordinal) = what.strip_prefix("save/") {
            let file = post.files.get(ordinal.parse::<usize>().ok()?)?;
            return Some(format!("Save {}", file.name));
        }
        // A quoted card leads somewhere inside the app, which is worth saying;
        // a page card is its own link and says so in its title already.
        if let Some(ordinal) = what.strip_prefix("preview/") {
            return match post.previews.get(ordinal.parse::<usize>().ok()?)? {
                matterless_render::Preview::Permalink { .. } => {
                    Some("Go to the message".to_string())
                }
                matterless_render::Preview::Page { url, .. } => Some(url.clone()),
            };
        }
        None
    }

    /// Redraws the taskbar overlay from the store.
    ///
    /// From the store rather than from anything passed in: the badge has to
    /// agree with the sidebar, and both of them have to agree with the
    /// notification policy -- muted channels contribute to none of the three.
    ///
    /// Cheap to call, and called often. `Taskbar::show` compares against what
    /// it last handed the shell, so the common case -- a message that changes
    /// nothing about what is waiting -- costs one comparison.
    fn update_badge(&mut self) {
        let Some(store) = self.store.as_deref() else {
            return;
        };
        if self.me.is_empty() {
            // Without a reader every channel's *total* count looks unread, and
            // the badge would open on a number in the thousands.
            return;
        }
        let collapsed = self.threads == matterless_core::model::ThreadMode::Collapsed;
        let Ok(state) = store.badge_state(&self.me, collapsed) else {
            return;
        };
        let attention = state.attention();
        let scale = self
            .window
            .as_ref()
            .map(|window| window.scale_factor())
            .unwrap_or(1.0);
        let overlay = matterless_view::badge::wanted(attention, state.any_unread, scale);
        let told = matterless_view::badge::described(attention, state.any_unread);
        if self
            .taskbar
            .show(raw_window(self.window.as_ref()), overlay, &told)
        {
            println!(
                "badge: {attention} wanting an answer, unread {}",
                state.any_unread
            );
        }
    }

    /// Puts the icon in the notification area, once.
    ///
    /// Answers whether it is there, which decides what the close button means:
    /// on a machine with no notification area, hiding the window would leave
    /// no way back to it and no way out.
    fn tray_ready(&mut self) -> bool {
        let Some(waker) = self.waker.clone() else {
            return false;
        };
        self.tray.show(move |act| {
            // Through the proxy rather than acting here: this is called from
            // inside a window procedure, part-way through winit's own dispatch,
            // and the window is not ours to change at that moment.
            let _ = waker.send_event(Update::Tray(act));
        })
    }

    /// Shows the window and puts it in front, from wherever it had got to.
    ///
    /// All three, because only all three work: it may be hidden, minimised, or
    /// simply behind everything, and a tray icon that sometimes does nothing is
    /// worse than one that is not there.
    fn raise(&mut self) {
        let Some(window) = self.window.as_ref() else {
            return;
        };
        window.set_visible(true);
        window.set_minimized(false);
        window.focus_window();
        self.taskbar.calm(raw_window(self.window.as_ref()));
    }

    /// Runs whatever the tray was asked for.
    fn act_on_tray(&mut self, act: matterless_view::tray::Act, events: &ActiveEventLoop) {
        use matterless_view::tray::{Act, startup};
        match act {
            Act::Open => self.raise(),
            Act::Startup => {
                let now = startup::set(!startup::enabled());
                println!("start with windows: {now}");
            }
            Act::Quit => {
                println!("quitting from the tray");
                // Taken down by hand rather than left to `Drop`: the event loop
                // does not unwind, so nothing else would remove the icon and
                // the shell would leave a dead one behind.
                self.tray.hide();
                events.exit();
            }
        }
    }

    fn permalink(&self, post_id: &str) -> Option<String> {
        matterless_view::feed::permalink(self.store.as_deref()?, &self.server, post_id)
    }

    /// Sends a link to one message into another conversation.
    ///
    /// A link rather than a copy of the words, which is what the official
    /// client does and the honest thing besides: the message stays one
    /// message, with one author and one set of replies, and the forward is a
    /// pointer to it rather than a second copy that can drift.
    fn forward(&mut self, post_id: &str, channel_id: &str) {
        match self.permalink(post_id) {
            Some(link) => self.post_message(channel_id, "", link),
            None => eprintln!("no team to build a link from"),
        }
    }

    /// Re-reads the sidebar, keeping where the reader is and how far they had
    /// scrolled: a list that jumps to the top every time a count changes is
    /// worse than one showing a stale number.
    fn rebuild_sidebar(&mut self) {
        // The taskbar carries the same counts as the list, so they are settled
        // together: a badge recomputed anywhere else is a badge that disagrees
        // with the sidebar under it.
        self.update_badge();
        // The teams beside it, from the same entries the sidebar just got:
        // a rail counting something the list does not show would be two
        // answers to one question.
        let mut teams: Vec<matterless_view::rail::Tile> = self
            .store
            .as_ref()
            .map(|store| {
                store
                    .teams()
                    .unwrap_or_default()
                    .into_iter()
                    .map(|team| matterless_view::rail::Tile {
                        id: team.id,
                        name: if team.display_name.is_empty() {
                            team.name
                        } else {
                            team.display_name
                        },
                        unread: 0,
                        mentions: 0,
                        directs: false,
                    })
                    .collect()
            })
            .unwrap_or_default();
        // What is waiting in each, summed from the rows themselves. A muted
        // channel contributes nothing: muting says "do not interrupt me", and
        // a dot on the team is an interruption at one remove.
        let mut directs = matterless_view::rail::Tile {
            id: matterless_view::rail::DIRECTS.to_string(),
            name: matterless_view::rail::ENVELOPE.to_string(),
            unread: 0,
            mentions: 0,
            directs: true,
        };
        if let Some(store) = self.store.as_ref() {
            for entry in &self.sidebar.entries {
                let matterless_view::sidebar::Entry::Channel {
                    id,
                    unread,
                    mentions,
                    muted,
                    direct,
                    ..
                } = entry
                else {
                    continue;
                };
                if *muted {
                    continue;
                }
                let into = if *direct {
                    Some(&mut directs)
                } else {
                    store
                        .channel(id)
                        .ok()
                        .flatten()
                        .and_then(|channel| {
                            teams.iter().position(|team| team.id == channel.team_id)
                        })
                        .map(|at| &mut teams[at])
                };
                if let Some(tile) = into {
                    tile.unread += unread;
                    tile.mentions += mentions;
                }
            }
        }
        teams.push(directs);
        self.rail.teams = teams;
        self.rail.chosen = self
            .sidebar
            .selected
            .as_ref()
            .and_then(|id| self.store.as_ref()?.channel(id).ok().flatten())
            .map(|channel| channel.team_id);
        let open = self.sidebar.selected.clone();
        let scroll = self.sidebar.scroll;
        let name = self.my_name();
        let status = self.presence.get(&self.me).cloned().unwrap_or_default();
        self.sidebar = Sidebar::new(Self::entries(
            self.store.as_deref(),
            &self.me,
            self.threads,
            (&name, &status, self.connected),
        ));
        self.sidebar.selected = open;
        self.sidebar.scroll = scroll;
    }

    /// Says when the local copy of a channel is behind what the server says.
    ///
    /// The unread count is arithmetic on two numbers the server keeps -- the
    /// channel's total and this reader's seen count -- and neither has
    /// anything to do with which posts are held here. So a channel can read
    /// "13 unread" while the newest message on screen is a week old, and the
    /// badge and the conversation are both telling the truth about different
    /// things. Worth saying out loud, because the two disagreeing is exactly
    /// what a reader would call a bug.
    fn report_hole(&self, channel: &str, store: &matterless_store::Store) {
        let (Ok(newest), Ok(Some(known))) = (store.newest_post_at(channel), store.channel(channel))
        else {
            return;
        };
        if known.last_post_at > newest {
            let behind = (known.last_post_at - newest) / 60_000;
            println!("{channel}: the local copy is {behind} minutes behind the server");
        }
    }

    /// Names any reaction that can be neither drawn nor fetched.
    ///
    /// Names only, never message text. A pill with no character and no picture
    /// is an empty box, and the tally is what says whether that is a name
    /// nobody has asked about or one the server answered "not custom" for.
    fn report_blank_pills(&self) {
        let blank: Vec<&str> = self
            .stream
            .planned()
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
        // A message in the channel being looked at *while looking at it* is
        // one the reader can already see, so interrupting them about it is
        // noise. Focus is half of that rule: the same channel behind another
        // window is not being read.
        let open = if self.focused {
            self.sidebar.selected.clone().unwrap_or_default()
        } else {
            String::new()
        };
        let mut asked = false;
        let mut named = false;
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
            let said = matterless_sync::notify::announce(&store, post_id, &self.me);
            let (title, body) = matterless_view::toast::wording(&said);
            // The count, never the words: a notification carries the message
            // and the log must not.
            println!(
                "notifying about {channel_id} ({} characters, named: {})",
                said.preview.chars().count(),
                said.resolved
            );
            asked = true;
            named |= said.resolved;
            let Some(clicked) = self.clicked.clone() else {
                continue;
            };
            matterless_view::toast::raise(channel_id, &title, &body, clicked);
        }
        // The button flashes for the same things the toast fires for, and only
        // while the window is not being looked at: asking for attention you
        // already have is how an app becomes irritating. Something that named
        // the reader keeps flashing until it is seen; anything else is one
        // nudge.
        if asked && !self.focused {
            self.taskbar
                .ask_for_attention(raw_window(self.window.as_ref()), named);
        }
        // Whatever arrived changed what is waiting.
        self.update_badge();
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
        wanted.extend(self.profile.wants());
        wanted.extend(self.rail.wants());
        wanted.extend(self.sidebar.wants());
        for (key, width, height) in wanted {
            if self.asked.insert(key.clone()) {
                link.send(matterless_view::live::Ask::Fetch { key, width, height });
            }
        }
        self.want_minis();
    }

    /// The mini previews on screen, decoded straight into the atlas.
    ///
    /// No request and no socket: the bytes came with the message. They go
    /// through the same `arrived` queue a fetched picture does, because the
    /// thread that owns the GPU is the only one that may touch the atlas.
    ///
    /// Counted in the same `asked` set, which is what keeps a kilobyte of JPEG
    /// from being decoded again on every frame it is visible.
    fn want_minis(&mut self) {
        let mut wanted = self.stream.minis(self.stream_rect());
        if let (Some(thread), Some(within)) = (&self.thread, self.thread_stream_rect()) {
            wanted.extend(thread.minis(within));
        }
        for (key, encoded) in wanted {
            if self.asked.insert(key.clone())
                && let Some((width, height, rgba)) = matterless_view::live::mini(&encoded)
            {
                self.arrived.push((key, width, height, rgba));
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
        match matterless_view::feed::rows_of(
            &store,
            &channel,
            &self.me,
            &outstanding,
            self.depth,
            self.viewed_at,
        ) {
            Ok(rows) => {
                self.stream.me = self.me.clone();
                self.stream.custom = matterless_view::feed::custom_emoji(&store, &rows);
                // Never lazily here: this is a page arriving under somebody
                // reading the top of the channel, and what is under their eye
                // is the oldest end rather than the newest.
                self.stream.plan(rows, false);
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
        // Rows already planned and not yet shaped are the top of the channel
        // arriving: the reader has not reached the end of what is here, the
        // list is simply still growing towards them.
        if self.loading_older || !self.more_history || self.stream.waiting() > 0 {
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
            Action::Link => match self.permalink(&post_id) {
                // Into this window's own clipboard, which is where everything
                // else it copies goes until there is a platform one.
                Some(link) => {
                    self.clipboard = link;
                    println!("copied a permalink");
                }
                None => eprintln!("no team to build a link from"),
            },
            Action::Forward => {
                let mut input = std::mem::take(&mut self.input);
                self.switcher.show(&mut self.fonts, &mut input);
                self.switcher
                    .instead(matterless_view::switcher::Asking::Forward(post_id));
                self.input = input;
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
        let _reread = matterless_view::timing::watch("re-reading the open channel", 0, "");
        self.recall_watermark(channel, &store);
        let within = self.stream_rect();
        let was_at_end = self.stream.scroll >= self.stream.reach(within) - 1.0;
        match matterless_view::feed::rows_of(
            &store,
            channel,
            &self.me,
            &self.outstanding.for_channel(channel),
            self.depth,
            self.viewed_at,
        ) {
            Ok(rows) => {
                self.stream.me = self.me.clone();
                self.stream.custom = matterless_view::feed::custom_emoji(&store, &rows);
                self.stream.plan(rows, was_at_end);
                self.watch_divider();
                self.relayout();
                if was_at_end {
                    let within = self.stream_rect();
                    self.stream.cover(&mut self.fonts, within.width, within);
                    self.stream.to_bottom(within);
                }
            }
            Err(why) => eprintln!("{channel}: {why}"),
        }
    }

    /// Where the "New messages" divider goes in `channel`.
    ///
    /// The rule itself is `feed::watermark`; this is the part that has to
    /// touch the store.
    fn recall_watermark(&mut self, channel: &str, store: &matterless_store::Store) {
        let seen = store
            .last_viewed_at(channel, &self.me)
            .ok()
            .flatten()
            .unwrap_or(0);
        let held = (self.viewed_in == channel).then_some(self.viewed_at);
        self.viewed_in = channel.to_string();
        self.viewed_at = matterless_view::feed::watermark(held, seen);
        // A watermark that moved back is the reader marking a message unread,
        // and the divider they asked for is not on a clock.
        if held.is_some_and(|held| self.viewed_at < held) {
            self.rest.stop();
        }
    }

    /// Starts the clock on a divider that has just been planned.
    ///
    /// Asked after every plan rather than only on the way in: a channel is
    /// replanned while it is open -- a reply arrives, the membership refreshes
    /// -- and a clock that was only ever started once would be left running
    /// against a divider that had gone, or never started for one that stayed.
    fn watch_divider(&mut self) {
        let shown = self
            .stream
            .rows
            .iter()
            .any(|row| matches!(row, Row::UnreadDivider));
        match (shown, self.rest.wakes().is_some()) {
            (true, false) => self.rest.begin(std::time::Instant::now(), self.focused),
            (false, _) => self.rest.stop(),
            (true, true) => {}
        }
    }

    /// Takes the divider away, the reader having seen it.
    ///
    /// The row is lifted out rather than the channel replanned: it is one line
    /// of text, and rereading costs the whole conversation its shaping -- two
    /// and a half seconds in a channel of crash reports, four seconds after
    /// opening it, which is a worse fault than the one this fixes.
    ///
    /// Zero is the planner's word for "no divider", and `watermark` keeps it:
    /// nothing short of leaving the channel brings it back.
    fn forget_divider(&mut self) {
        self.viewed_at = 0;
        let at = self
            .stream
            .rows
            .iter()
            .position(|row| matches!(row, Row::UnreadDivider));
        if let Some(at) = at {
            self.stream.forget_row(at);
            self.redraw();
        }
    }

    /// Shapes a slice of whatever the open channel is still waiting on.
    ///
    /// Answers whether there is more to do, so the window comes straight back
    /// for it: nothing else is going to wake it for work it set itself.
    ///
    /// A budget rather than a count, because a row is anything from one line
    /// to a screenful of code. Well under a frame, so a keystroke or a wheel
    /// turn never waits on more than one slice.
    fn shape_some(&mut self) -> bool {
        const BUDGET: std::time::Duration = std::time::Duration::from_millis(8);
        if self.stream.waiting() == 0 {
            return false;
        }
        let behind = self
            .behind
            .get_or_insert_with(|| (std::time::Instant::now(), self.stream.waiting()));
        let (since, rows) = (behind.0, behind.1);
        let within = self.stream_rect();
        let at_end = self.stream.scroll >= self.stream.reach(within) - 1.0;
        let began = std::time::Instant::now();
        let mut grew = 0.0;
        while self.stream.waiting() > 0 && began.elapsed() < BUDGET {
            grew += self.stream.fill(&mut self.fonts, within.width, 8);
        }
        // Rows appearing above must not push what is being read downwards.
        // Pinned to the end when that is where they were, and held in place
        // by the height that arrived when it is not.
        match at_end {
            true => self.stream.to_bottom(within),
            false => {
                self.stream.scroll =
                    (self.stream.scroll + grew).clamp(0.0, self.stream.reach(within));
            }
        }
        // The rows on screen have not moved, so the frame is only worth asking
        // for once there is nothing left to add -- and then for the scrollbar,
        // which has been growing the whole time.
        if self.stream.waiting() == 0 {
            println!(
                "shaped the rest of the channel behind the window: {rows} rows in {}ms",
                since.elapsed().as_millis()
            );
            self.behind = None;
            // The divider can be further back than a screenful of unread
            // messages, in which case it was still waiting when the plan
            // asked. Left unasked, its clock would never start and it would
            // stay on the channel for good.
            self.watch_divider();
            self.redraw();
            return false;
        }
        true
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
        boxes.extend(self.rail.boxes(self.rail_rect()));
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
        boxes.extend(header::boxes(self.column_rect(), &self.header_offers()));
        if let Some(pane) = self.thread_rect() {
            boxes.extend(header::boxes(pane, &header::for_thread()));
        }
        if self.on_threads() {
            // Deeper than the conversation's own rows, which are not drawn --
            // and shallower than anything that floats over the column.
            boxes.extend(self.followed.boxes_in(self.followed_rect(), 3));
        } else {
            boxes.extend(self.composer.boxes_in(header::below(self.channel_rect())));
        }
        if let Some(body) = self.thread_body() {
            boxes.extend(self.thread_composer.boxes_in(body));
        }
        boxes.extend(self.picker.boxes(self.picked_near, self.stream_rect()));
        if let Some(pane) = self.aside_rect() {
            boxes.extend(self.listing.boxes(pane));
            boxes.extend(self.search.boxes(pane));
        }
        boxes.extend(self.profile.boxes(self.stream_rect()));
        if let Some(row) = self.edited_row() {
            boxes.extend(self.edit.boxes(row, self.stream_rect()));
        }
        // Over everything, including a menu: it is the whole window.
        boxes.extend(self.viewer.boxes(self.window_rect()));
        // Last and deepest: a menu is over everything, and its catcher covers
        // the window so a click beside it shuts it rather than reaching what
        // it is covering.
        boxes.extend(self.menu.boxes(self.window_rect()));
        // The strip has its own room off the top of the window rather than
        // floating over anything, so it sits with the panes rather than above
        // them -- but its buttons still have to beat whatever the shell put
        // behind it.
        boxes.extend(self.offered.boxes(self.notice_rect()));
        // And the change list over all of it, including a menu: it covers the
        // window, and a press beside it shuts it.
        boxes.extend(self.whats_new.boxes(self.window_rect()));
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
        // Where the offer sits, before anything asks what is under the
        // pointer: it is measured rather than computed per frame because
        // measuring needs the fonts and a hit test does not have them.
        // The panel over everything, first and alone: it covers the window,
        // so nothing behind it may take the same press.
        if self.whats_new.open() {
            let window = self.window_rect();
            self.whats_new.measure(&mut self.fonts, window);
            if self.whats_new.react(&self.input) {
                self.input = Input::default();
            }
            return;
        }
        if self.offered.open() {
            let notice = self.notice_rect();
            self.offered.measure(&mut self.fonts, notice);
            match self.offered.react(&self.input) {
                Some(matterless_view::updater_bar::Chose::Install) => {
                    match (self.offer.clone(), self.link.as_ref()) {
                        (Some(offer), Some(link)) => {
                            println!("installing {}", offer.version);
                            link.send(matterless_view::live::Ask::InstallUpdate(offer));
                        }
                        _ => self.offered.failed("there is nothing to install"),
                    }
                    // Measured again: the button says something else now.
                    self.offered.measure(&mut self.fonts, notice);
                }
                Some(matterless_view::updater_bar::Chose::Later) => {
                    println!("the update was put off");
                    self.offer = None;
                    // The strip has given its row back, so everything under it
                    // is taller than it was.
                    self.relayout();
                }
                Some(matterless_view::updater_bar::Chose::News) => {
                    let notes = self.offered.notes().to_string();
                    let version = self
                        .offer
                        .as_ref()
                        .map(|offer| offer.version.clone())
                        .unwrap_or_default();
                    self.whats_new.show(&version, &notes);
                    let window = self.window_rect();
                    self.whats_new.measure(&mut self.fonts, window);
                }
                None => {}
            }
        }
        // The viewer first of all and alone: it covers the window, so nothing
        // behind it may take the same press or the same key.
        if self.viewer.open() {
            let mut input = std::mem::take(&mut self.input);
            let did = self.viewer.react(&input);
            match did {
                Some(matterless_view::viewer::Did::Close) => {
                    self.viewer.hide();
                    if let Some(view) = self.view.as_mut() {
                        view.stop_showing();
                    }
                    input.focus_on(composer::NAME);
                }
                Some(matterless_view::viewer::Did::Show(one)) => {
                    // The last one's texture goes now rather than when the next
                    // arrives: holding it would leave the old picture on screen
                    // under a name that is no longer its own.
                    if let Some(view) = self.view.as_mut() {
                        view.stop_showing();
                    }
                    self.fetch_looked(&one);
                }
                Some(matterless_view::viewer::Did::Save { file_id, name }) => {
                    if let Some(link) = self.link.as_ref() {
                        link.send(matterless_view::live::Ask::Download { file_id, name });
                    }
                }
                None => {}
            }
            self.input = input;
            return;
        }

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

        // The card is dismissed rather than interacted with: it says who
        // somebody is and has nothing to press. Escape, or a click anywhere
        // that is not on it -- which is what a reader expects of a popover and
        // means it never has to be closed deliberately.
        if self.profile.open()
            && (self.input.struck(Key::Escape)
                || self
                    .input
                    .clicked()
                    .is_some_and(|name| name != matterless_view::profile::NAME))
        {
            self.profile.hide();
        }

        // The followed threads, filling the conversation's own column. Choosing
        // one goes where it was said and opens it, which is the whole reason
        // the list exists: under collapsed threads a reply is a row nowhere
        // else, so this is the only way to reach one.
        if self.on_threads() {
            let input = std::mem::take(&mut self.input);
            let boxes = self.placed.clone();
            let within = self.followed_rect();
            let did = self.followed.react_in(&input, &boxes, within);
            self.input = input;
            if let Some(matterless_view::listing::Did::Open(found)) = did {
                let root = found.root_id.clone();
                self.sidebar.selected = Some(found.channel_id.clone());
                self.open_channel(&found.channel_id);
                if !root.is_empty() {
                    self.open_thread(&root);
                }
                return;
            }
        }

        // A list of messages sits *beside* the conversation rather than over
        // it, so unlike the switcher it does not take the frame: a click on the
        // sidebar, a turn of the wheel over the channel, and typing a reply all
        // still work while it is open. Its own rows are its own because a click
        // resolves to one box, which is what the depths are for.
        if self.listing.open() {
            let mut input = std::mem::take(&mut self.input);
            if input.struck(Key::Escape) {
                self.listing.hide();
                input.focus_on(composer::NAME);
                self.input = input;
                return;
            }
            let pane = matterless_view::aside::rect(self.column_rect());
            let boxes = self.placed.clone();
            let did = self.listing.react(&input, &boxes, pane);
            if matches!(did, Some(matterless_view::listing::Did::Close)) {
                self.listing.hide();
                input.focus_on(composer::NAME);
                self.input = input;
                return;
            }
            if let Some(matterless_view::listing::Did::Open(found)) = did {
                self.listing.hide();
                input.focus_on(composer::NAME);
                self.input = input;
                // Opened at the conversation it was said in. Landing on the
                // message itself needs an anchor the window cannot ask for
                // yet, so it opens where the reader can find it rather than
                // pretending.
                self.sidebar.selected = Some(found.channel_id.clone());
                self.open_channel(&found.channel_id);
                // And the thread, when the row is about one -- which is the
                // whole point of a list of threads, and right for a saved
                // reply too.
                if !found.root_id.is_empty() {
                    self.open_thread(&found.root_id);
                }
                return;
            }
            self.input = input;
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

        // Search sits beside the conversation too, and for the better reason:
        // a result is a line out of context, and the context is the channel it
        // is read against. It keeps the keyboard while its own field has the
        // focus, and leaves the rest of the frame alone.
        if self.search.open {
            let mut input = std::mem::take(&mut self.input);
            if input.struck(Key::Escape) {
                self.search.hide(&mut input);
                self.input = input;
                return;
            }
            let pane = matterless_view::aside::rect(self.column_rect());
            let boxes = self.placed.clone();
            let store = self.store.clone();
            self.search.scrolled(&input, &boxes, pane);
            let did = store.as_ref().and_then(|store| {
                self.search.react(
                    &mut self.fonts,
                    &input,
                    pane,
                    &mut self.clipboard,
                    store,
                    &self.me,
                )
            });
            if matches!(did, Some(matterless_view::search::Did::Close)) {
                self.search.hide(&mut input);
                self.input = input;
                return;
            }
            if let Some(matterless_view::search::Did::Open(hit)) = did {
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
            let picked = self.switcher.react(
                &mut self.fonts,
                &input,
                within,
                &mut self.clipboard,
                &entries,
            );
            // Everybody picked, for the one question that takes more than one
            // name. Nothing else can answer with several, so nothing else has
            // to be consulted about which question this was.
            if let Some(matterless_view::switcher::Chose::These(these)) = picked {
                self.switcher.hide(&mut input);
                self.input = input;
                self.talk_to(these.iter().map(|one| one.id.clone()).collect());
                return;
            }
            if let Some(matterless_view::switcher::Chose::One(reached)) = picked {
                use matterless_view::switcher::{Asking, Reach};
                // Which of the three questions this was is remembered on the
                // switcher rather than guessed here.
                let asking = self.switcher.asking.clone();
                self.switcher.hide(&mut input);
                self.input = input;
                match (asking, reached.reach) {
                    // A message can only be forwarded somewhere the reader can
                    // already write, so the other two reaches are no answer.
                    (Asking::Forward(post_id), Reach::Open) => self.forward(&post_id, &reached.id),
                    (Asking::Forward(_), _) => {
                        eprintln!("that is not somewhere to forward to yet")
                    }
                    // Only a person can be added to a channel, which is why
                    // this list offers no conversations while it is asking.
                    (Asking::Add(channel_id), Reach::Direct) => {
                        if let Some(link) = self.link.as_ref() {
                            link.send(matterless_view::live::Ask::AddMember {
                                channel_id,
                                user_id: reached.id,
                            });
                        }
                    }
                    (Asking::Add(_), _) => eprintln!("that is not somebody to add"),
                    (Asking::Jump, Reach::Open) => {
                        self.sidebar.selected = Some(reached.id.clone());
                        self.open_channel(&reached.id);
                    }
                    // Both need the server before there is anything to open,
                    // so the window hears back rather than guessing an id.
                    (Asking::Jump, Reach::Join) => {
                        if let Some(link) = self.link.as_ref() {
                            link.send(matterless_view::live::Ask::Join {
                                channel_id: reached.id,
                            });
                        }
                    }
                    (Asking::Jump, Reach::Direct) => {
                        if let Some(link) = self.link.as_ref() {
                            link.send(matterless_view::live::Ask::Direct {
                                user_id: reached.id,
                            });
                        }
                    }
                    // One name, for the question that wanted several: a
                    // conversation with one other person is a direct message.
                    (Asking::Start, Reach::Direct) => self.talk_to(vec![reached.id]),
                    (Asking::Start, _) => eprintln!("that is not somebody to talk to"),
                }
                return;
            }
            // Whatever the letters could still mean, asked once per query
            // rather than once per keystroke.
            if let Some(query) = self.switcher.to_ask()
                && let Some(link) = self.link.as_ref()
            {
                link.send(matterless_view::live::Ask::Discover { query });
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

        // The two buttons in each box. Send does what return does, and attach
        // does what a drop does -- both exist for a reader who has not been
        // told about either.
        if let Some(button) = self.composer.pressed(&self.input) {
            self.pressed_in_composer(button, "");
        }
        if self.thread_body().is_some()
            && let Some(button) = self.thread_composer.pressed(&self.input)
        {
            let root = self.open_root().unwrap_or_default();
            self.pressed_in_composer(button, &root);
        }

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
        let _open = matterless_view::timing::watch("opening a channel", 0, "");
        // The threads row is a place to go, not a channel to read: it fills
        // the same column, so opening it is the same gesture, but there is no
        // conversation to load and nothing to mark read.
        if channel == matterless_view::sidebar::THREADS {
            self.followed.expect("Threads");
            self.followed.fill(matterless_view::listing::followed(
                &store, &self.me, THREADS,
            ));
            self.thread = None;
            return;
        }
        self.followed.hide();
        // Before anything marks it read, which is what this is for.
        self.recall_watermark(channel, &store);
        match matterless_view::feed::rows_of(
            &store,
            channel,
            &self.me,
            &self.outstanding.for_channel(channel),
            self.depth,
            self.viewed_at,
        ) {
            Ok(rows) => {
                self.stream.me = self.me.clone();
                self.stream.custom = matterless_view::feed::custom_emoji(&store, &rows);
                // Lazily: the next thing this does is show the newest message,
                // so the top of the conversation can be shaped behind it.
                self.stream.plan(rows, true);
                self.watch_divider();
                self.report_blank_pills();
                self.report_hole(channel, &store);
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
                self.stream.cover(&mut self.fonts, within.width, within);
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

        // Each piece of a frame, so the table says where one goes rather than
        // only that it was slow. Two spaces in front of the name because they
        // are the halves of "building the frame", which is the line above them.
        let probe = matterless_view::timing::watch("  the rail", 0, "");
        let rail = self.rail_rect();
        scene.clip_to(rail.x, rail.y, rail.width, rail.height);
        {
            let mut canvas = Canvas {
                scene: &mut scene,
                painter: &mut self.painter,
                fonts: &mut self.fonts,
                palette: &self.palette,
            };
            self.rail.draw(&mut canvas, rail, &self.input);
        }
        drop(probe);

        let probe = matterless_view::timing::watch("  the sidebar", 0, "");
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
        drop(probe);
        let probe = matterless_view::timing::watch("  the header", 0, "");

        // Read before the painter is borrowed, and given its own layer after: a
        // name too long for the strip is cut by the clip rather than running
        // along the top of the first message.
        // The title and nothing else. Whether this window is still hearing
        // anything is said at the top of the sidebar now, where it belongs: it
        // is a fact about the connection rather than about the conversation.
        let on_strip = self.on_header();
        let mut header = Header::new(self.title());
        header.offered = self.header_offers();
        header.muted = self.muted();
        if self.on_threads() {
            // The same mark the sidebar row carries, so the two read as the
            // same place rather than as a channel that happens to be called
            // Threads.
            header.sigil = matterless_layout::marks::THREADS;
            header.sigil_is_mark = true;
        }
        scene.clip_to(strip.x, strip.y, strip.width, strip.height);
        let mut canvas = Canvas {
            scene: &mut scene,
            painter: &mut self.painter,
            fonts: &mut self.fonts,
            palette: &self.palette,
        };
        header.draw(&mut canvas, strip, on_strip);
        drop(probe);

        let probe = matterless_view::timing::watch("  the stream", self.stream.rows.len(), "rows");
        if self.on_threads() {
            // The list instead of the conversation, in the same column: it was
            // chosen from the sidebar the way a channel is, so it opens where
            // a channel opens.
            let within = self.followed_rect();
            scene.clip_to(within.x, within.y, within.width, within.height);
            let mut canvas = Canvas {
                scene: &mut scene,
                painter: &mut self.painter,
                fonts: &mut self.fonts,
                palette: &self.palette,
            };
            self.followed.draw_in(&mut canvas, &self.input, within);
        } else {
            scene.clip_to(stream.x, stream.y, stream.width, stream.height);
            let mut canvas = Canvas {
                scene: &mut scene,
                painter: &mut self.painter,
                fonts: &mut self.fonts,
                palette: &self.palette,
            };
            self.stream.draw(&mut canvas, stream, &self.input);
        }
        drop(probe);
        let probe = matterless_view::timing::watch("  the thread pane", 0, "");

        // The thread pane: its own header and its own clip, so a reply cannot
        // spill into the conversation it came from.
        // Mutable because a stream records where it drew each link, which is
        // what the next frame hit-tests against.
        let following = self.following_open_thread();
        if let (Some(pane), Some(thread)) = (self.thread_rect(), self.thread.as_mut()) {
            // The pane's own frame: `.pane { border-left: 1px solid var(--rule);
            // background: var(--ground) }`. Without the rule it ran into the
            // conversation it came from with nothing between them.
            scene.clip_to(pane.x, pane.y, pane.width, pane.height);
            scene.fill(pane.x, pane.y, pane.width, pane.height, self.palette.ground);
            scene.fill(pane.x, pane.y, 1.0, pane.height, self.palette.rule);
            let strip = header::strip(pane);
            // Above the reply box, not the whole pane: replies drawn behind it
            // would show through the box's own margin.
            let rows = self.thread_composer.above(header::below(pane));
            let mut title = Header::new(format!("Thread -- {} replies", thread.rows.len()));
            // Whether its replies keep interrupting the reader, and a way out.
            // Nothing on the channel's strip applies to one thread.
            title.offered = header::for_thread();
            title.muted = following;
            scene.clip_to(strip.x, strip.y, strip.width, strip.height);
            let mut canvas = Canvas {
                scene: &mut scene,
                painter: &mut self.painter,
                fonts: &mut self.fonts,
                palette: &self.palette,
            };
            title.draw(&mut canvas, strip, on_strip);

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

        drop(probe);
        let probe = matterless_view::timing::watch("  the composers", 0, "");
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
        // rather than under a message that scrolled into the strip. Not at all
        // while the threads are up: there is nothing there to reply to.
        if self.on_threads() {
            drop(probe);
            return scene;
        }
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
        drop(probe);
        let _probe = matterless_view::timing::watch("  the overlays", 0, "");

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
        if let Some(pane) = self.aside_rect() {
            scene.clip_to(0.0, 0.0, self.size.0 as f32, self.size.1 as f32);
            let mut canvas = Canvas {
                scene: &mut scene,
                painter: &mut self.painter,
                fonts: &mut self.fonts,
                palette: &self.palette,
            };
            self.listing.draw(&mut canvas, &self.input, pane);
            let field = self.search.field(pane);
            self.search.draw(&mut canvas, &self.input, pane);
            if self.search.open {
                self.search.query.draw(&mut canvas, field, true);
            }
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

        // Over the conversation and under nothing: a card is the answer to a
        // question the reader just asked, so whatever it covers is not what
        // they are looking at.
        if self.profile.open() {
            scene.clip_to(0.0, 0.0, self.size.0 as f32, self.size.1 as f32);
            let mut canvas = Canvas {
                scene: &mut scene,
                painter: &mut self.painter,
                fonts: &mut self.fonts,
                palette: &self.palette,
            };
            self.profile.draw(&mut canvas, stream);
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
        if self.offered.open() {
            scene.clip_to(0.0, 0.0, self.size.0 as f32, self.size.1 as f32);
            let notice = self.notice_rect();
            let mut canvas = Canvas {
                scene: &mut scene,
                painter: &mut self.painter,
                fonts: &mut self.fonts,
                palette: &self.palette,
            };
            self.offered.draw(&mut canvas, &self.input, notice);
        }
        // The picture a reader opened, over the window and everything in it.
        // Under only the menu and the tooltip, which are the two things that
        // are always over whatever they are about.
        if self.viewer.open() {
            let window = self.window_rect();
            scene.clip_to(0.0, 0.0, self.size.0 as f32, self.size.1 as f32);
            let mut canvas = Canvas {
                scene: &mut scene,
                painter: &mut self.painter,
                fonts: &mut self.fonts,
                palette: &self.palette,
            };
            self.viewer.draw(&mut canvas, &self.input, window);
        }
        // The change list over the picture: it was asked for from a strip that
        // sits above everything, and it covers the window while it is read.
        if self.whats_new.open() {
            let window = self.window_rect();
            scene.clip_to(0.0, 0.0, self.size.0 as f32, self.size.1 as f32);
            let mut canvas = Canvas {
                scene: &mut scene,
                painter: &mut self.painter,
                fonts: &mut self.fonts,
                palette: &self.palette,
            };
            self.whats_new.draw(&mut canvas, &self.input, window);
        }
        // The menu over even that: it is the thing the reader just asked for.
        if self.menu.open() {
            let window = self.window_rect();
            scene.clip_to(0.0, 0.0, self.size.0 as f32, self.size.1 as f32);
            let mut canvas = Canvas {
                scene: &mut scene,
                painter: &mut self.painter,
                fonts: &mut self.fonts,
                palette: &self.palette,
            };
            self.menu.draw(&mut canvas, window, &self.input);
        }
        // Last of everything, because it explains whatever is on top: a label
        // about a menu item drawn under the menu is a label about nothing.
        if self.tooltip.shown().is_some() {
            let window = self.window_rect();
            scene.clip_to(0.0, 0.0, self.size.0 as f32, self.size.1 as f32);
            let mut canvas = Canvas {
                scene: &mut scene,
                painter: &mut self.painter,
                fonts: &mut self.fonts,
                palette: &self.palette,
            };
            self.tooltip.draw(&mut canvas, window);
        }
        scene
    }

    /// Points the tooltip at whatever the pointer is now on.
    ///
    /// Called wherever the hover can have changed rather than once a frame:
    /// the box under the pointer is settled by `input`, and asking again while
    /// nothing has moved would restart the wait on every redraw -- so a
    /// tooltip would never appear at all.
    fn watch_pointer(&mut self) {
        let hovered = self.input.hovered().map(str::to_string);
        let explained = hovered
            .as_deref()
            .and_then(|name| self.explains(name))
            .unwrap_or_default();
        let at = self.input.pointer_at();
        self.tooltip.follows(hovered.as_deref(), at, |_| {
            (!explained.is_empty()).then_some(explained)
        });
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
        {
            let _shaping = matterless_view::timing::watch(
                "shaping the channel",
                self.stream.rows.len(),
                "rows",
            );
            self.stream.lay_out(&mut self.fonts, stream.width);
        }
        self.stream.clamp(stream);

        if let Some(pane) = self.thread_rect() {
            self.thread_composer.lay_out(&mut self.fonts, pane.width);
        }
        if let Some(within) = self.thread_stream_rect()
            && let Some(thread) = self.thread.as_mut()
        {
            let _shaping =
                matterless_view::timing::watch("shaping the thread", thread.rows.len(), "rows");
            thread.lay_out(&mut self.fonts, within.width);
            thread.clamp(within);
        }
    }
}

/// The application's own icon, at `side` pixels square.
///
/// Windows keeps two icons per window and uses them for different things: the
/// small one is the title bar, the big one is the taskbar button and Alt-Tab.
/// They have to be set separately -- `with_window_icon` sets only the small
/// one, which is why the title bar was right while the taskbar button stayed
/// on the blank sheet that says "some program".
///
/// Scaled here rather than handed over at 512 and left to Windows: an icon
/// resized once by a proper filter into the size it will be shown at is the
/// same argument the badge and the tray picture make.
fn window_icon(side: u32) -> Option<winit::window::Icon> {
    // Decoded once and kept. Two sizes are asked for, and decoding a 512px
    // picture twice cost most of the time the window took to be created --
    // measured at 187ms of 199ms, which is a fifth of a second of nothing on
    // screen for a picture that has not changed.
    static FULL: std::sync::OnceLock<Option<image::RgbaImage>> = std::sync::OnceLock::new();
    let full = FULL
        .get_or_init(|| {
            const ICON: &[u8] = include_bytes!("../resources/icons/icon.png");
            image::load_from_memory(ICON)
                .inspect_err(|error| eprintln!("the window icon would not decode: {error}"))
                .ok()
                .map(|decoded| decoded.into_rgba8())
        })
        .as_ref()?;
    // A box filter rather than Lanczos: this is a 16- or 32-fold reduction,
    // where a windowed sinc costs a great deal and shows nothing for it.
    let scaled = image::imageops::thumbnail(full, side, side);
    winit::window::Icon::from_rgba(scaled.into_raw(), side, side)
        .inspect_err(|error| eprintln!("the window icon was refused: {error}"))
        .ok()
}

/// What Windows asks for at 100% scaling: 16 for the title bar, 32 for the
/// taskbar. Both are resampled by the shell for a higher DPI, which is a
/// better trade than handing it one size and letting it guess the other.
const SMALL_ICON: u32 = 16;
const BIG_ICON: u32 = 32;

/// The attributes a window starts from.
///
/// Split out because the taskbar's icon is a Windows-only attribute: the
/// window carries two icons there, and setting only the one winit's portable
/// call reaches leaves the taskbar button on the default.
#[cfg(windows)]
fn window_attributes() -> winit::window::WindowAttributes {
    use winit::platform::windows::WindowAttributesExtWindows;
    Window::default_attributes().with_taskbar_icon(window_icon(BIG_ICON))
}

#[cfg(not(windows))]
fn window_attributes() -> winit::window::WindowAttributes {
    Window::default_attributes()
}

/// This window's handle, for the Win32 calls that need one.
///
/// Zero when there is no window yet or the platform has no such thing, which
/// every caller treats as "do nothing": the badge and the flashing are both
/// decoration over a window that already works.
fn raw_window(window: Option<&Arc<Window>>) -> matterless_view::taskbar::RawWindow {
    #[cfg(windows)]
    {
        use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
        let Some(Ok(handle)) = window.map(|window| window.window_handle()) else {
            return 0;
        };
        let RawWindowHandle::Win32(win32) = handle.as_raw() else {
            return 0;
        };
        win32.hwnd.get()
    }
    #[cfg(not(windows))]
    {
        let _ = window;
        0
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
        // Straight back for the next one, with nothing waited on: a measured
        // frame is one this program asked for rather than one the server
        // happened to cause.
        if self.driver.is_some() {
            self.redraw();
            return events.set_control_flow(ControlFlow::Poll);
        }
        let (expired, next) = self.typing.forget_stale();
        // Only when a line actually went: asking for a frame whenever one is
        // merely live would redraw every frame for as long as anybody types.
        if expired {
            self.redraw();
        }
        // A tooltip appears without anything happening -- the pointer stops,
        // and half a second later there are words. Nothing else would wake the
        // window for that, so the wait is cut short for it and the frame that
        // draws it is asked for here.
        if self.tooltip.ripened() {
            self.redraw();
        }
        // The same trick for the unread divider: it goes with nothing having
        // happened, so the window has to be woken to draw the frame without
        // it.
        if self.rest.ripened(std::time::Instant::now()) {
            self.forget_divider();
        }
        let filling = self.shape_some();
        let next = [next, self.tooltip.wakes(), self.rest.wakes()]
            .into_iter()
            .flatten()
            .min();
        events.set_control_flow(match (filling, next) {
            // Straight back here for the next slice. Nothing else will wake
            // the window for work it is doing on its own.
            (true, _) => ControlFlow::Poll,
            (false, Some(expires)) => ControlFlow::WaitUntil(expires),
            (false, None) => ControlFlow::Wait,
        });
    }

    fn resumed(&mut self, events: &ActiveEventLoop) {
        let began = std::time::Instant::now();
        // Opened without taking focus when asked, which is what makes it
        // usable next to the work it is being compared against: a window that
        // seizes the keyboard every time it starts interrupts whoever is
        // watching it.
        let quiet = std::env::var_os("MATTERLESS_QUIET").is_some();
        // Created hidden, and shown at the bottom of this function once there
        // is something to see.
        //
        // Not cosmetic: the shell asks a window for its icon with `WM_GETICON`,
        // which is a `SendMessage` and blocks on the owner pumping messages.
        // Setting up Vulkan below takes well over a second and pumps nothing,
        // so a window shown first gets a taskbar button the shell cannot ask
        // about -- it gives up, draws the generic "some program" icon, and
        // corrects itself only once the app starts answering. The icons were
        // set all along; nobody was there to hand them over.
        let window = Arc::new(
            events
                .create_window(
                    window_attributes()
                        .with_title("MatterLess -- list on Vulkan")
                        .with_window_icon(window_icon(SMALL_ICON))
                        .with_active(!quiet)
                        .with_visible(false)
                        .with_inner_size(winit::dpi::LogicalSize::new(
                            self.size.0 as f64,
                            self.size.1 as f64,
                        )),
                )
                .expect("a window"),
        );
        self.window = Some(Arc::clone(&window));
        // Before anything else can want it: the close button means one thing
        // with a tray and another without, and the answer must not depend on
        // how far through starting up the reader got.
        if !self.tray_ready() {
            eprintln!("no notification area: closing the window will quit");
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
        // The integrated chip in preference to the card, deliberately.
        //
        // A chat window has no business waking a discrete GPU: it draws a few
        // hundred quads when somebody scrolls, and on a machine doing real
        // work on that card it should stay out of the way.
        //
        // It is not free, and the cost is worth knowing before somebody
        // "fixes" this. Where the display hangs off a discrete card -- which
        // is the usual desktop arrangement -- every finished frame is copied
        // across to be shown. Measured on a machine with an RTX 5070 beside an
        // integrated Radeon, that was 350ms of extra swapchain creation at
        // start-up, and the surface came back in the other channel order,
        // which is the display saying whose it is. `--example what_gpu` prints
        // what any given machine offers and what each costs.
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::LowPower,
            compatible_surface: Some(&surface),
            force_fallback_adapter: false,
        }))
        .expect("a Vulkan adapter");
        let (device, queue) =
            pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default()))
                .expect("a device");

        let ready = began.elapsed();
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
        // Already held, from before the tray was told about it.
        debug_assert!(self.window.is_some());
        self.relayout();
        // Enough of the newest end to fill the panel, so the first frame is a
        // conversation rather than a dozen rows in an empty column. The rest
        // of it is shaped behind the window once it is up.
        let within = self.stream_rect();
        self.stream.cover(&mut self.fonts, within.width, within);
        self.stream.to_bottom(within);
        // What the window waits on before it can be shown, since it now waits
        // on all of it: a swapchain is most of it and belongs to the driver,
        // and the rest is this program's to keep honest.
        println!(
            "ready in {}ms, of which {}ms was Vulkan up to the device",
            began.elapsed().as_millis(),
            ready.as_millis()
        );
        // Everything is ready, so the window can be seen -- and the message
        // loop is about to start, so the shell's question about the icon will
        // be answered rather than timed out.
        if quiet {
            behind(&window);
        } else {
            window.set_visible(true);
        }
    }

    /// What the socket reported, delivered on the thread that owns the window.
    fn user_event(&mut self, events: &ActiveEventLoop, update: Update) {
        // The tray is the one update that can end the loop, so it is answered
        // here rather than in `apply`, which has no way to say so.
        if let Update::Tray(act) = update {
            self.act_on_tray(act, events);
            return;
        }
        self.apply(update);
    }

    fn window_event(&mut self, events: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => {
                // Closing HIDES the window, and the tray is what says so.
                //
                // Notifications only exist while this process runs -- there is
                // no push proxy for a desktop client -- so quitting on the
                // close button would quietly turn them off, which is the one
                // thing that lets somebody keep the webapp closed. Quitting is
                // still one click away, in the tray menu.
                //
                // Unless there is no tray to be in, in which case the close
                // button is the only way out and has to remain one.
                if self.tray_ready() {
                    if let Some(window) = self.window.as_ref() {
                        window.set_visible(false);
                    }
                    println!("hidden to the tray");
                } else {
                    events.exit();
                }
            }
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
                self.watch_pointer();
                self.redraw();
            }
            // A file dragged onto the window goes to the conversation under
            // the pointer -- the thread if one is open and the pointer is in
            // it, the channel otherwise. Dropping is the whole gesture: no
            // dialog to open, and no dependency for one.
            WindowEvent::DroppedFile(path) => {
                let Some(channel_id) = self.sidebar.selected.clone() else {
                    eprintln!("no conversation to send that to");
                    return;
                };
                let over_thread = self.thread_rect().zip(self.input.pointer_at()).is_some_and(
                    |(pane, (x, y))| {
                        x >= pane.x && x <= pane.right() && y >= pane.y && y <= pane.bottom()
                    },
                );
                let root_id = if over_thread {
                    self.open_root().unwrap_or_default()
                } else {
                    String::new()
                };
                if let Some(link) = self.link.as_ref() {
                    link.send(matterless_view::live::Ask::Upload {
                        channel_id,
                        root_id,
                        path,
                    });
                }
            }
            WindowEvent::Focused(focused) => {
                self.focused = focused;
                // A channel left open behind an editor is not being read.
                self.rest.focus(std::time::Instant::now(), focused);
                if focused {
                    // Looked at, so it has been answered: the flashing stops
                    // even though whatever caused it may still be unread. The
                    // badge is what carries that, and it stays.
                    self.taskbar.calm(raw_window(self.window.as_ref()));
                }
                // Which conversation counts as "being read" depends on this, so
                // the socket thread has to hear about it.
                if let Some(link) = self.link.as_ref() {
                    link.send(matterless_view::live::Ask::Looking {
                        channel_id: if focused {
                            self.sidebar.selected.clone().unwrap_or_default()
                        } else {
                            String::new()
                        },
                    });
                }
            }
            WindowEvent::CursorLeft { .. } => {
                self.input.apply(UiEvent::PointerLeft, &[]);
                self.watch_pointer();
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
                // A menu goes away on Escape before anything else reads the
                // keystroke, and takes the keystroke with it: closing a menu
                // and the thread pane behind it with one press is two answers
                // to one question.
                if down && self.menu.open() {
                    self.menu.react(&self.input);
                    if !self.menu.open() {
                        self.input.settle();
                        self.react();
                        self.redraw();
                        return;
                    }
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
                // Answered from the store, so it is filled the moment it
                // opens rather than after a round trip.
                if down && self.input.chord(Key::Char('t')) {
                    self.listing.expect("Threads");
                    if let Some(store) = self.store.clone() {
                        let found = matterless_view::listing::followed(&store, &self.me, THREADS);
                        println!("Threads: {} followed", found.len());
                        self.listing.fill(found);
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
                // The other button asks what can be done here. Only the
                // sidebar answers, which is where the app puts its own.
                if button == winit::event::MouseButton::Right {
                    if state != winit::event::ElementState::Pressed {
                        return;
                    }
                    let boxes = self.targets();
                    self.input.apply(UiEvent::Contexted, &boxes);
                    self.offer_channel_menu();
                    self.react();
                    self.redraw();
                    return;
                }
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
                // The menu first, and alone: while one is open its catcher
                // covers the window, so everything under it is out of reach
                // until it has been answered or dismissed.
                if self.menu.open() {
                    if let Some((about, chosen)) = self.menu.react(&self.input) {
                        self.act_on_menu(&about, &chosen);
                    }
                    self.react();
                    self.redraw();
                    return;
                }
                // A team on the rail takes the reader to the first
                // conversation it holds: a team is not itself somewhere to be,
                // and landing on nothing would be a press that did nothing.
                // A team is somewhere in the list rather than somewhere to
                // be, so the rail takes the reader there: it scrolls the
                // sidebar to that team's heading and leaves the conversation
                // they were reading open. Opening a channel they did not ask
                // for would be answering a question they did not put.
                if let Some(team) = self.rail.react(&self.input) {
                    let within = self.sidebar_rect();
                    self.sidebar.scroll_to(&team, within);
                }
                // The one button in the list, before the rows: a press on it
                // is not a press on the heading behind it.
                if self.input.clicked_on(matterless_view::sidebar::NEW) {
                    let mut input = std::mem::take(&mut self.input);
                    self.switcher.show(&mut self.fonts, &mut input);
                    self.switcher
                        .instead(matterless_view::switcher::Asking::Start);
                    self.input = input;
                    self.redraw();
                    return;
                }
                let within = self.sidebar_rect();
                if let Some(channel) = self.sidebar.react(&self.input, &boxes, within) {
                    self.open_channel(&channel);
                }
                // The strip's own buttons, which are the only visible way to
                // reach the lists: a keystroke nobody has been told about is
                // not a feature anybody has.
                if let Some(pressed) = self
                    .input
                    .clicked()
                    .and_then(|name| name.strip_prefix("header/"))
                    .map(str::to_string)
                {
                    // The field opens the same panel the keystroke does, so
                    // there is one search rather than two that drift.
                    if pressed == "find" {
                        let mut input = std::mem::take(&mut self.input);
                        self.search.show(&mut self.fonts, &mut input);
                        self.input = input;
                    } else if let Some(act) = header::Act::from_slug(&pressed) {
                        self.act_on_header(act);
                    }
                }
                // A message opens its thread. Taken before the composer reacts,
                // because opening one narrows the column the composer sits in.
                let stream = self.stream_rect();
                let chose = self.stream.react(&self.input, &boxes, stream);
                self.acted(chose);
                // And the same in the thread pane, which draws the same rows
                // with the same controls on them: without this its toolbar was
                // there to be hovered and did nothing when it was pressed.
                let chose = match (self.thread_stream_rect(), self.thread.as_mut()) {
                    (Some(within), Some(thread)) => thread.react(&self.input, &boxes, within),
                    _ => None,
                };
                self.acted(chose);
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
                // The list on the right, and the one filling the column, each
                // take the turn when the pointer is over them -- the same way
                // every other panel does.
                if self.aside_rect().is_some() || self.on_threads() {
                    self.react();
                }
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
                            // Bigger than the picture half itself, now that
                            // the half makes room rather than filling up.
                            None => refused += 1,
                        }
                    }
                    println!("atlas: {placed} pictures in, {refused} refused");
                }
                // Anything the atlas threw away is something this window must
                // be willing to ask for again. Without this the record of
                // having asked outlives the picture, and a reader scrolling
                // back up finds a gap that nothing ever fills.
                if let Some(view) = self.view.as_mut() {
                    for key in view.atlas.forgotten() {
                        self.asked.remove(&key);
                    }
                }
                // What is on screen may have changed since the last frame.
                self.want_faces();
                self.name_emoji();
                self.ask_who_is_around();
                // The two halves of a frame, timed apart. Building the draw
                // list is this program's work and is the half worth fixing;
                // handing it to the GPU and waiting for the swapchain is
                // mostly the driver's, and a slow one there usually means
                // vsync rather than anything here.
                let building = matterless_view::timing::watch(
                    "building the frame",
                    self.stream.rows.len(),
                    "rows",
                );
                let scene = self.scene();
                drop(building);
                let _drawing = matterless_view::timing::watch("drawing the frame", 0, "");
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
                if let Some(driver) = self.driver.as_mut()
                    && !driver.drew()
                {
                    drop(_drawing);
                    println!("{}", matterless_view::timing::table());
                    events.exit();
                }
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
    // Before anything opens a connection. `reqwest` names its own provider,
    // but the WebSocket goes through `tokio-tungstenite`, which asks rustls to
    // pick one -- and rustls panics rather than choose when the binary carries
    // both. `aws-lc-rs` is the one reqwest already uses.
    if rustls::crypto::aws_lc_rs::default_provider()
        .install_default()
        .is_err()
    {
        println!("a rustls crypto provider was already installed");
    }
    // Before any notification, and before the window: an AppUserModelID is
    // what the shell attributes a toast to, and it is a claim about the whole
    // process rather than about one message. Says what it decided, because
    // "why does this say PowerShell" is otherwise unanswerable from outside.
    matterless_view::identity::claimed();
    // `MATTERLESS_TOAST=<text>` raises one and stops. The notification path is
    // otherwise only reachable by persuading somebody to send a message, which
    // is a poor way to check what a notification looks like -- and whose name
    // is on it is exactly the thing that was wrong.
    if let Some(text) = std::env::var_os("MATTERLESS_TOAST") {
        let text = text.to_string_lossy().to_string();
        let clicked: std::sync::Arc<matterless_view::toast::Clicked> =
            std::sync::Arc::new(Box::new(|channel| println!("clicked, for {channel}")));
        let shown = matterless_view::toast::raise("a-channel", "MatterLess", &text, clicked);
        println!("raised a notification: {shown}");
        // A toast is handed to the shell and drawn by it, so this process has
        // to outlive the handover.
        std::thread::sleep(std::time::Duration::from_secs(6));
        return;
    }

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

/// Whether this reader wants unread conversations lifted into their own group.
///
/// A preference rather than a choice made here: the app reads the same one,
/// and a sidebar that rearranges itself differently in two clients of one
/// server is worse than either arrangement.
fn lifts_unreads(store: &matterless_store::Store, me: &str) -> bool {
    store
        .preference(me, "sidebar_settings", "show_unread_section")
        .ok()
        .flatten()
        .as_deref()
        == Some("true")
}
