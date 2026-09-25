//! A window showing the message list, drawn on Direct3D 11.
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
use matterless_view::stream::{Anchor, Stream};
use std::sync::Arc;
use winit::application::ApplicationHandler;
use winit::event::{MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::window::{Window, WindowId};

/// The first day of the sample, as days since the epoch.
///
/// Fixed rather than "today": a sample that moves means a snapshot taken on
/// one day and compared on the next differs in six date separators and
/// nothing else.
const SAMPLE_DAY: i64 = 20_340;

fn post(author: &str, at: (i64, u32, u32), nodes: Vec<Node>) -> PostRow {
    let (day, hour, minute) = at;
    PostRow {
        // The day and the time are in it because they are what makes it one
        // of six. Named after the author and the length of what was said, it
        // was the same id on all six days -- and the stream keys a row by its
        // id, so hovering one of them meant hovering all six.
        post_id: format!("p-{author}-{day}-{hour}{minute:02}"),
        root_id: String::new(),
        author_id: format!("u-{author}"),
        author_name: author.into(),
        create_at: ((SAMPLE_DAY + day) * 86_400 + i64::from(hour) * 3_600 + i64::from(minute) * 60)
            * 1_000,
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

/// Enough rows to scroll through, with the shapes that used to be
/// mismeasured: long wraps, mixed weight, mentions, lists and code.
///
/// Invented, and deliberately so. This was a real exchange between real
/// colleagues -- their usernames, what they were asking each other, and the
/// configuration they were discussing -- pasted in as convenient test data
/// and then compiled into a public repository and shipped inside an
/// installer. Sample data is published data. Nothing here names anybody.
///
/// Two exchanges over six days rather than one over six. A window with no
/// store shows this and nothing else, and the same words under six different
/// dates do not read as six days of a conversation; they read as a window
/// drawing the same day over and over, which is a bug report waiting to
/// happen. Both carry every shape, so what each is here to exercise is on
/// screen either way.
fn conversation() -> Vec<Row> {
    let mut rows = Vec::new();
    for day in 0..6 {
        rows.push(Row::DateSeparator {
            epoch_day: SAMPLE_DAY + day,
        });
        match day % 2 {
            0 => cache_day(&mut rows, day),
            _ => hardware_day(&mut rows, day),
        }
    }
    rows
}

/// One day of the sample: a question about a machine everybody shares.
fn cache_day(rows: &mut Vec<Row>, day: i64) {
    rows.push(Row::Post {
        post: post("ada", (day, 9, 14), vec![para("Morning!")]),
    });
    rows.push(Row::Continuation {
        post: post(
            "ada",
            (day, 9, 15),
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
            (day, 9, 31),
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
                    language: Some("toml".into()),
                    value: "cache_size = \"2GiB\"\nkeep_days = 14".into(),
                },
            ],
        ),
    });
}

/// The other day of the sample: a machine slower than it was.
fn hardware_day(rows: &mut Vec<Row>, day: i64) {
    rows.push(Row::Post {
        post: post(
            "cara",
            (day, 10, 2),
            vec![para("Is anyone else seeing this?")],
        ),
    });
    rows.push(Row::Continuation {
        post: post(
            "cara",
            (day, 10, 3),
            vec![para(
                "The overnight run took two hours and forty minutes, against fifty-odd \
                 minutes for the same commit last week. Nothing in the tree explains it, \
                 so I would rather rule the machine out before anybody starts bisecting a \
                 slowdown that is not in the code at all.",
            )],
        ),
    });
    rows.push(Row::Post {
        post: post(
            "ben",
            (day, 10, 18),
            vec![
                Node::Paragraph {
                    children: vec![
                        Node::Text {
                            value: "that will be the disk -- it has been ".into(),
                        },
                        Node::Strong {
                            children: vec![Node::Text {
                                value: "read-only since Tuesday".into(),
                            }],
                        },
                        Node::Text {
                            value: ", which ".into(),
                        },
                        Node::UserMention {
                            username: "ada".into(),
                            everyone: false,
                        },
                        Node::Text {
                            value: " found the hard way".into(),
                        },
                    ],
                },
                Node::List {
                    ordered: false,
                    items: vec![
                        vec![para("every write falls back to the network share")],
                        vec![para("and the share is a long way from that rack")],
                    ],
                },
                Node::CodeBlock {
                    language: Some("toml".into()),
                    value: "scratch = \"/mnt/local/build\"\nfallback = \"//store/build\"".into(),
                },
            ],
        ),
    });
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
/// What the side pane is called when the pointer is on its edge.
const GRIP: &str = "pane-grip";
/// What the thread pane's reply box answers to.
const THREAD_COMPOSER: &str = "thread-composer";

/// One thing a reader does, kept up for as long as it is being measured.
///
/// A window redrawing the same picture is the cheapest frame it will ever
/// have: nothing is reshaped, no row is laid out again, every picture is
/// already in the atlas. Measuring only that says a frame costs a fraction of
/// a millisecond and hides everything a reader would actually feel.
#[derive(Clone, Copy, PartialEq, Debug)]
enum Act {
    /// The floor: the same scene, drawn again.
    Still,
    /// Down for a stretch and back up. Rows leave the plan and come back,
    /// which is what a store that only ever grows would hide.
    Scrolling,
    /// Round the sidebar. The dearest thing this program does on a keystroke,
    /// and the one the reader complained about first.
    Switching,
    /// Into the composer, until it wraps, then cleared: a box of one line and
    /// a box of four are not the same work.
    Typing,
    /// Open a thread, and close it. A second stream beside the first.
    Threading,
    /// Drag the window's edge, which is the dearest thing anybody can do to
    /// it: every width rebuilds the swapchain.
    ///
    /// Driven rather than asked for, because measuring it by hand costs
    /// somebody a drag per reading and answers one question at a time.
    Resizing,
    /// The same, downwards. Nothing re-wraps when only the height changes, so
    /// this is the axis that says whether the reader is being held or merely
    /// left where they were -- and while the act above was the only one, that
    /// question went unasked twice.
    Heightening,
}

impl Act {
    /// Every act, in the order a run measures them: cheapest first, so the
    /// table above reads as the floor the ones below it are measured against.
    const EVERY: [Act; 7] = [
        Act::Still,
        Act::Scrolling,
        Act::Switching,
        Act::Typing,
        Act::Threading,
        Act::Resizing,
        Act::Heightening,
    ];

    fn what(self) -> &'static str {
        match self {
            Act::Still => "still",
            Act::Scrolling => "scrolling",
            Act::Switching => "switching channel",
            Act::Typing => "typing",
            Act::Threading => "opening a thread",
            Act::Resizing => "dragging the edge",
            Act::Heightening => "dragging the bottom edge",
        }
    }
}

/// What the driver's typing act types, over and over.
///
/// Named because it is also how what the driver typed is recognised and
/// cleared afterwards: a draft made of nothing but this word was left by a
/// measurement, never by the reader.
const DRIVER_TYPES: &str = "mesure ";

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
    /// Which act is being measured, as an index into `Act::EVERY`.
    at: usize,
    /// Frames to draw for each act.
    each: u32,
    /// Frames left in this one.
    left: u32,
    /// Frames of warm-up left before the tally is started.
    warm: u32,
    /// Frames into this act, warm-up included, which is what the acts that
    /// have a rhythm -- scroll down then up, type then clear -- count on.
    tick: u32,
    /// How many times the act being measured actually did its thing.
    ///
    /// Counted because an act that quietly does nothing reads exactly like an
    /// act that is free: the first scrolling run turned the wheel with the
    /// pointer over no panel, nothing moved, and the table reported the cost
    /// of a still frame under the word "scrolling".
    did: u32,
}

impl Driver {
    /// How many frames to draw before the tally is started.
    ///
    /// Not only for the window opening: an act has its own warm-up. The first
    /// frame after a channel is switched to has every one of its rows to lay
    /// out, and counting that as the cost of switching would say switching is
    /// what a first frame costs.
    const WARM: u32 = 30;

    fn asked() -> Option<Self> {
        let each = std::env::var("MATTERLESS_FRAMES")
            .ok()?
            .parse::<u32>()
            .ok()?;
        println!(
            "{each} frames for each of {} acts, {} warm before each (MATTERLESS_FRAMES)",
            Act::EVERY.len(),
            Self::WARM
        );
        Some(Self {
            at: 0,
            each,
            left: each,
            warm: Self::WARM,
            tick: 0,
            did: 0,
        })
    }

    fn act(&self) -> Option<Act> {
        Act::EVERY.get(self.at).copied()
    }

    fn over(&self) -> bool {
        self.at >= Act::EVERY.len()
    }

    /// The very first frame of an act, which is when it is put back to where
    /// every act begins.
    ///
    /// Without this each act inherited whatever the one before it left: the
    /// channel-switching act wandered off down the sidebar, and the thread act
    /// after it landed in a conversation with no thread in it and measured
    /// nothing at all while reporting a number.
    fn fresh(&self) -> bool {
        self.warm == Self::WARM
    }

    /// Whether what this frame did counts. The warm-up's work is real and is
    /// not what the act costs.
    fn counting(&self) -> bool {
        self.warm == 0
    }

    /// Counts the frame just drawn, and names the act if that was its last.
    ///
    /// The name is what the caller prints the table under: a run answers
    /// "what does scrolling cost" rather than "what does a frame cost", which
    /// is the question anybody actually has. The count comes back with it
    /// rather than being left to be read off the driver, which is where the
    /// first version put it -- after this had already reset it, so every act
    /// reported having done nothing.
    fn drew(&mut self) -> Option<(&'static str, u32)> {
        let act = self.act()?;
        // The clock runs through the warm-up too, so an act keeps its rhythm
        // from its first frame. Held still, the channel-switching act asked
        // for the same switch on every one of its thirty warm frames.
        self.tick += 1;
        if self.warm > 0 {
            self.warm -= 1;
            // What came before was the window opening, or the first frames of
            // this act, and neither is what the act costs.
            if self.warm == 0 {
                matterless_view::timing::forget();
            }
            return None;
        }
        self.left -= 1;
        if self.left > 0 {
            return None;
        }
        self.at += 1;
        self.left = self.each;
        self.warm = Self::WARM;
        self.tick = 0;
        let did = std::mem::take(&mut self.did);
        Some((act.what(), did))
    }
}

struct App {
    window: Option<Arc<Window>>,
    /// Kept so another surface can be made for the window without starting
    /// over. `MATTERLESS_SURFACE=new` builds one instead of rebuilding the
    /// one there is, to find out which of the two is cheaper.
    view: Option<matterless_view::View>,
    /// The same surface read without its sRGB encoding, which is what the
    /// pipeline writes through.
    size: (u32, u32),
    fonts: Fonts,
    painter: Painter,
    /// The open channel.
    stream: Stream,
    /// The open thread, when there is one. A second stream rather than a
    /// second kind of panel: a thread is the same rows in a narrower column.
    thread: Option<Stream>,
    /// Where the channel was before the thread pane took half of it.
    parked: Option<Parked>,
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
    /// The channel every driven act starts from, remembered the first time one
    /// runs so that the acts are measured against each other rather than
    /// against wherever the act before them wandered to.
    home: Option<String>,
    /// When the window last changed size, while it is still changing.
    ///
    /// A drag reports a size on every frame of itself and each one shapes what
    /// is on screen; the whole conversation is shaped once, when the edge
    /// stops, to settle the heights of everything that is not.
    resizing: Option<std::time::Instant>,
    /// A size that has arrived and not been built for yet.
    ///
    /// Held rather than applied, and applied only where the surface is rebuilt
    /// for it. `self.size` is what every rectangle in the window is measured
    /// from and the surface is what they are drawn into: moved apart, even for
    /// one frame, the window lays itself out for a size it is not yet, and the
    /// scissor for a panel 1001 wide lands in a target 1000 wide -- which is
    /// not a wrong pixel, it is a validation error and a dead window.
    ///
    /// Answered at the top of the next frame rather than as each size lands.
    /// A drag reports sizes faster than the window can draw them, so the ones
    /// it passes through between two frames are not worth building for: this
    /// field holds the latest and the frame takes it. Building inside the
    /// handler instead left the window blocked in there while the system
    /// queued the next dozen sizes, every one of which was then paid for in
    /// turn.
    sized: Option<(u32, u32)>,
    input: Input,
    placed: Vec<Placed>,
    composer: Composer,
    /// Half-written messages, by the conversation they were being written to.
    ///
    /// A draft belongs to its conversation: what somebody began saying in one
    /// channel makes no sense in the next, and is exactly what they want back
    /// when they return to it. So a box is not cleared on the way out -- it is
    /// put away, and brought back when the reader is.
    ///
    /// In memory only. A draft is a thought in progress rather than a
    /// document, and one handed back a week after the window was last open
    /// would be a surprise rather than a convenience.
    drafts: std::collections::HashMap<String, String>,
    /// Files already on the server, waiting for a message to carry them.
    ///
    /// Under the conversation they were pasted into, exactly as a draft is:
    /// a picture meant for one channel makes no more sense in the next than
    /// half a sentence does, and it should be there on the way back.
    ///
    /// The whole `FileInfo` rather than the id, because the guess row drawn
    /// the moment a message is sent shows the attachments too, and only the
    /// server's copy says how big a picture is.
    attached: std::collections::HashMap<String, Vec<matterless_core::model::FileInfo>>,
    /// The box heights the conversation was last laid out against.
    ///
    /// `reacted` checks for a box that changed size and returns early on half
    /// a dozen paths before it gets there -- a picker open, a mention list
    /// up, a pane taking the frame -- so the frame asks too. It is the one
    /// place every path ends at.
    laid_against: (f32, f32),
    /// When this window started, which is what the spinner's phase is taken
    /// from: any fixed instant will do, and one that never changes means two
    /// spinners on screen turn together.
    began: std::time::Instant,
    /// Files dropped and not yet on the server, by conversation.
    ///
    /// The tray is drawn from these as well as from `attached`, so a tile
    /// appears on the frame a file is dropped rather than when the upload
    /// answers -- which for anything large was several seconds of a window
    /// that had visibly done nothing with what was dropped on it.
    attaching: std::collections::HashMap<String, Vec<std::path::PathBuf>>,
    /// Where each box is writing to, so its text can be put away under the
    /// right name when the reader moves.
    ///
    /// Kept beside the boxes rather than read off what is open, because the
    /// thread pane is taken away and rebuilt every time anybody posts in it --
    /// so by the time the question is asked there is nothing left to ask.
    writing_to: Option<String>,
    replying_to: Option<String>,
    /// The thread pane's own reply box. Kept across a close so a half-written
    /// reply survives glancing back at the channel, and cleared when a
    /// different thread opens.
    thread_composer: Composer,
    /// When and where the pointer was last pressed, and how many presses in a
    /// row that was, so a double or triple press can be told from two.
    pressed_before: Option<(std::time::Instant, (f32, f32), u32)>,
    /// What was last copied, which every widget that copies or pastes is
    /// handed. Joined to the system's clipboard in `react`, so the two agree
    /// without a dozen widgets each having to know there is a system.
    clipboard: String,
    /// The socket thread, once it is up. Sends go through it.
    link: Option<matterless_view::live::Link>,
    /// The pictures already on this disk, read on threads of their own. Every
    /// picture is asked of this first; a miss goes down the link from there.
    shelf: Option<matterless_view::live::Shelf>,
    /// The pictures opened lately, decoded, so opening one again is instant.
    remembered: matterless_view::viewer::Remembered,
    /// The frames of the picture in the viewer, when it moves, by its file id.
    looked_moving: matterless_view::moving::Moving,
    /// Messages written but not yet confirmed, held in memory and nowhere else:
    /// a guess must never reach SQLite.
    outstanding: Arc<matterless_render::pending::PendingPosts>,
    /// Pictures already asked for, so a face on screen is fetched once rather
    /// than on every frame it is visible.
    asked: std::collections::HashSet<String>,
    /// Decoded pictures waiting to go into the atlas, which only the thread
    /// that owns the GPU may touch.
    arrived: Vec<(String, u32, u32, Vec<u8>)>,
    /// The way in, when there is no session. Kept rather than made when it is
    /// needed: a refused sign-in has to keep what was typed, and a form built
    /// fresh each frame would not.
    signin: matterless_view::signin::SignIn,
    /// A version the reader has already said no to.
    ///
    /// Kept because the looking repeats now. "Not now" has to mean it, or a
    /// window left open all day asks again every two hours about the build it
    /// was told to stop asking about -- which is worse than never looking
    /// twice, and is why it only ever looked once.
    put_off: Option<String>,
    /// Whether the looking has been started.
    ///
    /// `connect` runs again after a sign-in, and a second call would leave two
    /// threads asking the same question for as long as the window is open.
    watching: bool,
    /// What this copy of the program has been told to do.
    settings: matterless_view::settings::Settings,
    /// A turn of the wheel still arriving.
    glide: matterless_view::glide::Glide,
    /// What the two boxes said when they were last looked at, and when that
    /// changed, for putting a draft away once the typing stops.
    ///
    /// The text rather than a flag set where a key is handled: a box takes
    /// characters, pastes, drops, deletions, an emoji off the grid and a name
    /// off the completion list, and a flag would have to be set in every one
    /// of those places and would be forgotten in one.
    watched: (String, String),
    typed_at: Option<std::time::Instant>,
    /// The pictures that move, and where in their loops they are.
    moving: matterless_view::moving::Moving,
    /// Which channel the open thread belongs to.
    ///
    /// Not whatever the column is showing. A thread opened from the Threads
    /// list is read beside that list rather than beside its own conversation,
    /// so "the channel being looked at" stops being the channel being replied
    /// to -- and a reply sent to the wrong one goes to the wrong people.
    thread_in: Option<String>,
    /// The session this run holds, once somebody has signed in.
    ///
    /// Held here rather than read back out of the keychain, because signing in
    /// is the one moment this program has a token in its hand and does not
    /// need to ask anybody for it. It used to ask: `opened_a_session` wrote
    /// the token away and then `connect` read it again, so a keychain that
    /// took the write and would not give it back sent the reader straight to
    /// the sign-in screen they had just come from -- with the password
    /// cleared, and nothing on screen saying anything had gone wrong, because
    /// as far as either half was concerned nothing had.
    session: Option<String>,
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
    /// What to call each person heard typing, by id.
    ///
    /// Filled once, the first time somebody is heard from, and read on every
    /// frame the line is up. Typing is the loudest signal on the socket --
    /// between 72% and 93% of it -- but a person repeats it every three
    /// seconds while they keep going, so a lookup per person costs one read
    /// per typing run rather than one per signal.
    typing_names: std::collections::HashMap<String, String>,
    /// Who is around, and everybody ever asked about -- so a face is asked
    /// about once, and the socket thread's beat keeps it current after that.
    presence: std::collections::HashMap<String, String>,
    /// When each of them was last active, as the server said with their status.
    active_at: std::collections::HashMap<String, i64>,
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
    /// Why this window is not talking to a server, when it is not.
    ///
    /// Each of these was a line printed to a console nobody sees, and the
    /// window said "offline" and nothing else -- so a fresh install looked
    /// exactly like a slow connection, and there was no way to tell from the
    /// screen which of three quite different things was missing.
    offline: Option<&'static str>,
    /// The people or channels a half-typed name could mean.
    ///
    /// Not `offer`, which is a new version waiting to be installed. Two
    /// things called the offer in one window is one too many.
    naming: matterless_view::offer::Offer,
    /// The conversations this reader has been in, for the back button.
    visited: matterless_view::places::Places,
    /// Whether the channel being opened is a step through `visited` rather
    /// than a new arrival, so a step does not record itself as one.
    stepping: bool,
    /// How wide the side pane is, as the reader has left it.
    ///
    /// One number for the thread pane and for the list, because they are one
    /// edge as far as anybody looking at the window is concerned -- only ever
    /// one of them is up, and two that did not line up read as the window
    /// having moved something while the reader was not looking.
    ///
    /// Held here rather than in either pane: it belongs to the window, so
    /// searching after reading a thread finds the edge where it was left.
    pane_width: f32,
}

/// How many followed threads the list holds. Well past what anybody reads in
/// one sitting, and the store answers instantly either way.
const THREADS: u32 = 200;

/// How far down the pill is allowed to reach, given the conversation above
/// the box and the room that box keeps clear of itself.
///
/// The strip and the conversation tile exactly, so the conversation's bottom
/// edge is the top of the composer's strip -- and the box starts a margin
/// below that. Those pixels are ground with nothing in them, so the pill
/// sits in them and covers that much less of what is being read. Measured:
/// twelve of the pill's twenty-four.
///
/// It stops exactly where the box starts, so none of the box is covered
/// however tall the box has grown.
fn floor_of(above: Rect, margin: f32) -> f32 {
    above.bottom() + margin
}

/// The pill saying somebody is writing: how tall, how far in, and the room
/// inside it.
///
/// Taller than the sixteen the strip used to be, because nothing is reserved
/// for it any more -- it floats over the conversation, so its height costs
/// nothing and a thirteen-point line wants the room.
const TYPING: f32 = 24.0;
const TYPING_INSET: f32 = 24.0;
const TYPING_PADDING: f32 = 10.0;
const TYPING_DROP: f32 = 8.0;

/// How far below the top edge the unread mark is brought, so what it
/// follows is still visible above it. About three lines.
const ABOVE: f32 = 72.0;

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
            // Nothing is known yet -- not the reader, not whether there is a
            // server to reach. `connect` settles it on the first frame and
            // rebuilds this.
            Who {
                name: "",
                status: "",
                live: false,
                offline: None,
            },
        );
        let sidebar = Sidebar::new(entries);
        drop(listing);
        let reading = matterless_view::timing::watch("reading the first channel", 0, "");
        let (channel, rows) = Self::feed();
        drop(reading);
        let mut app = Self {
            driver: Driver::asked(),
            home: None,
            resizing: None,
            parked: None,
            sized: None,
            window: None,
            view: None,
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
            drafts: std::collections::HashMap::new(),
            attached: std::collections::HashMap::new(),
            attaching: std::collections::HashMap::new(),
            began: std::time::Instant::now(),
            laid_against: (0.0, 0.0),
            writing_to: None,
            replying_to: None,
            pressed_before: None,
            clipboard: String::new(),
            link: None,
            shelf: None,
            remembered: matterless_view::viewer::Remembered::default(),
            looked_moving: matterless_view::moving::Moving::default(),
            outstanding: Arc::new(matterless_render::pending::PendingPosts::default()),
            asked: std::collections::HashSet::new(),
            arrived: Vec::new(),
            signin: matterless_view::signin::SignIn::default(),
            settings: matterless_view::settings::Settings::default(),
            glide: matterless_view::glide::Glide::default(),
            watched: (String::new(), String::new()),
            typed_at: None,
            moving: matterless_view::moving::Moving::default(),
            thread_in: None,
            put_off: None,
            watching: false,
            session: None,
            clicked: None,
            switcher: matterless_view::switcher::Switcher::default(),
            search: matterless_view::search::Search::default(),
            picker: matterless_view::picker::Picker::default(),
            picked_near: matterless_ui::Rect::new(0.0, 0.0, 0.0, 0.0),
            edit: matterless_view::edit::Edit::default(),
            typing: matterless_view::typing::Typing::default(),
            said_typing: matterless_view::typing::Sending::default(),
            typing_names: std::collections::HashMap::new(),
            pane_width: matterless_view::aside::WIDTH,
            // Until `connect` says otherwise, which it does on the first
            // frame: a window that has not tried yet is not offline.
            offline: None,
            naming: matterless_view::offer::Offer::default(),
            visited: matterless_view::places::Places::default(),
            stepping: false,
            presence: std::collections::HashMap::new(),
            active_at: std::collections::HashMap::new(),
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
        // Whichever pane has the right of the column, at the width it is
        // actually drawn at -- the same choice `channel_rect` makes when it
        // decides how much room the conversation has left.
        //
        // A pane that is drawn and not in here leaves every box in the channel
        // running on underneath it: the wheel over a list of search results
        // scrolled the message box behind it, because `wheel_over` asks
        // whether a matching box is under the pointer rather than whether it
        // is the topmost one. And a pane in here that is not drawn narrows
        // them for nothing, which is what a thread did while a list was open
        // over it.
        if let Some(pane) = self.aside_rect() {
            row = row.with(Boxed::new("aside-panel", Size::Fixed(pane.width)));
        } else if let Some(pane) = self.thread_rect() {
            row = row.with(
                // Its own width, not `THREAD`: a narrow window gives a thread
                // half the column and no more, and a box built from the
                // constant would hang off the edge of the one on screen.
                Boxed::new("thread-panel", Size::Fixed(pane.width))
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
        (self.search.open() || self.listing.open())
            .then(|| matterless_view::aside::rect(self.column_rect(), self.pane_width))
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
        // The same edge the list uses, by the same rule: one pane at a time
        // and one width between them, so opening a thread after a search does
        // not move the edge the reader just placed.
        Some(matterless_view::aside::rect(column, self.pane_width))
    }

    /// The channel's title bar, above its conversation.
    ///
    /// From `channel_rect`, which is the whole of the fix this replaced:
    /// asking `column_rect` put the strip across the thread pane as well, and
    /// `place` lays the buttons in from the *right* edge of what it is given
    /// -- so opening a thread slid every one of them under the pane, where
    /// the pane's own header was then drawn over them. Reported as the thread
    /// header hiding the channel's buttons, which is exactly what it was.
    ///
    /// The strip, the conversation and the message box now all measure from
    /// the same rect, which is the only way they can agree.
    fn header_rect(&self) -> Rect {
        header::strip(self.channel_rect())
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
    ///
    /// All of it when there is no box: room kept for something that is not
    /// drawn is a band of ground along the bottom of the conversation, and
    /// the conversation is the only thing on screen in that case.
    fn stream_rect(&self) -> Rect {
        let within = header::below(self.channel_rect());
        match self.can_write() {
            true => self.composer.above(within),
            false => within,
        }
    }

    /// Where the pill saying somebody is writing floats.
    ///
    /// Over the foot of the conversation rather than in a strip of its own.
    /// The strip was reserved whether or not anybody was typing, so that the
    /// conversation would not jump a line when somebody started -- which
    /// bought that at the price of a line of the conversation, always, in
    /// every channel, for a line that is up for seconds at a time. A pill
    /// over the last message costs nothing when there is nothing to say.
    fn typing_rect(&self) -> Rect {
        let above = self.composer.above(header::below(self.channel_rect()));
        Rect::new(
            above.x,
            floor_of(above, self.composer.margin()) - TYPING,
            above.width,
            TYPING,
        )
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

    /// The same line, for the thread pane.
    fn thread_typing_rect(&self) -> Option<Rect> {
        let above = self.thread_composer.above(self.thread_body()?);
        Some(Rect::new(
            above.x,
            floor_of(above, self.thread_composer.margin()) - TYPING,
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
        // What is on screen with no store is the invented sample, and it is
        // not a channel: it is in nobody's sidebar, on no server, and named
        // after the fallback rather than after a place. Called "sample"
        // behind a hash it read as a channel by that name, which is the one
        // reading of it that is not true.
        if self.store.is_none() {
            return "Sample conversation".to_string();
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
            // A channel the sidebar has not caught up with yet. An id is a
            // poor name but better than an empty strip.
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
        who: Who<'_>,
    ) -> Vec<Entry> {
        let groups = store
            .map(|store| matterless_view::sidebar_feed::groups(store, me, threads))
            .unwrap_or_default();
        // Without a reader there is no membership row, and the store answers
        // with each channel's *total* message count -- which looks like an
        // unread badge of four thousand.
        let counted = !me.is_empty();
        let Who {
            name,
            status,
            live,
            offline,
        } = who;
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
            offline: offline.map(str::to_string),
            version: matterless_view::update::running().to_string(),
        }];
        // First in the list, because with collapsed threads a reply never
        // touches its channel's counters: this is the only row in the sidebar
        // that can say a thread is waiting.
        if counted {
            let (unread, mentions) = store
                .and_then(|store| store.thread_unread_totals().ok())
                .unwrap_or((0, 0));
            entries.push(Entry::Threads { unread, mentions });
            // Under it, and only when there are any. A row saying nought
            // takes a line of the list to say nothing.
            let drafts = store
                .and_then(|store| store.drafts(me).ok())
                .map(|kept| kept.len())
                .unwrap_or(0);
            if drafts > 0 {
                entries.push(Entry::Drafts { count: drafts });
            }
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
                    label: matterless_view::sidebar::UNREADS.to_string(),
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
        // Before the channel is opened, so the box it opens with is filled
        // from what was left rather than emptied and filled a frame later.
        self.recall_drafts();
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
        //
        // And before everything below it, which is the point. This used to be
        // queued on the socket, so every one of the three returns under this
        // line -- no store, no server, no session -- took the update check
        // away with it. An install that cannot sign in is the one that most
        // needs a newer build, and it was the only one that never asked.
        if !self.watching {
            self.watching = true;
            if matterless_view::update::asks() {
                matterless_view::live::look_for_updates(
                    matterless_view::live::HOW_OFTEN,
                    Proxy(proxy.clone()),
                );
            } else {
                println!("no update check in a dev build");
                self.pretend_offered();
            }
        }
        // Each of these is said on screen as well as here. They are three
        // different problems with three different answers, and "offline" on
        // its own is the one word that fits all of them and helps with none.
        let store = match self.store.clone() {
            Some(store) => store,
            // No database is not the same as nothing to be done about it.
            // It is a sandbox build's first start, and it is also what a
            // reader who deleted theirs is left with -- which used to be
            // unrecoverable: the window said "offline" for ever while holding
            // a session and a server address that would have filled a new one
            // in seconds.
            None => match self.made_a_store() {
                Some(store) => store,
                None => {
                    println!("no database, so nothing to keep up to date");
                    self.stayed_offline("no message store");
                    return;
                }
            },
        };
        let Some(path) = matterless_view::feed::default_store() else {
            self.stayed_offline("nowhere to keep a message store");
            return;
        };
        let Some(server) = matterless_view::live::stored_server(&path) else {
            println!("no server.txt beside the database; staying offline");
            self.stayed_offline("no server to talk to");
            return;
        };
        // What this run already holds first, and only then what was kept from
        // a previous one.
        let Some(token) = self
            .session
            .clone()
            .or_else(matterless_view::live::stored_token)
        else {
            println!("no session in the keychain; staying offline");
            self.stayed_offline("no session to sign in with");
            return;
        };
        self.offline = None;
        // The click handler is built here because it needs the same proxy the
        // socket thread wakes the window with: a toast fires its callback on a
        // thread of its own, and this is how the answer gets home.
        let waking = proxy.clone();
        self.clicked = Some(Arc::new(Box::new(move |channel_id: String| {
            let _ = waking.send_event(matterless_view::live::Update::Activated(channel_id));
        })));
        println!("connecting to {server}");
        self.server = server.clone();
        self.link = Some(matterless_view::live::start(
            store.clone(),
            server,
            token,
            Proxy(proxy.clone()),
        ));
        self.shelf = self
            .link
            .as_ref()
            .map(|link| matterless_view::live::shelf(link.clone(), Proxy(proxy)));
        // Every face this machine has heard of, fetched behind everything the
        // reader is looking at. A face is about 24KB and the link measures
        // 220KB/s, so the first sighting of thirty people costs three seconds
        // whatever order they are asked for in -- the only way for that not to
        // be a wait is for it to have happened already.
        let mut keys: Vec<String> = match store.everyone_with_a_picture() {
            Ok(people) => people
                .into_iter()
                .map(|(id, stamp)| matterless_view::stream::avatar_key(&id, stamp))
                .collect(),
            Err(error) => {
                eprintln!("asking who has a picture: {error}");
                Vec::new()
            }
        };
        let faces = keys.len();
        // Everybody is asked about once, now, rather than as each face
        // appears. A dot was asked for as its face came into view and drawn
        // only once the answer was back -- a couple of seconds of faces with no
        // dot on a busy start, in every channel opened. Asked up front, a
        // channel opened later finds them answered; the socket thread asks
        // again on a beat, because a status does not stay true for long. In
        // slices, because one request naming everybody a big server knows is
        // one a server may refuse.
        if let (Some(link), Ok(people)) = (self.link.as_ref(), store.everyone_with_a_picture()) {
            let everybody: Vec<String> = people.into_iter().map(|(id, _)| id).collect();
            for slice in everybody.chunks(matterless_view::live::STATUSES_AT_ONCE) {
                link.send(matterless_view::live::Ask::Statuses {
                    user_ids: slice.to_vec(),
                });
            }
            self.asked_about.extend(everybody);
        }
        // The team's own emoji as well. They are pictures like any other, and
        // one missing is a gap in the middle of a sentence rather than a face
        // beside it -- the more noticeable of the two, and the reader saw
        // both still loading when only the faces were warmed.
        match store.every_custom_emoji() {
            Ok(emoji) => keys.extend(
                emoji
                    .into_iter()
                    .map(|id| matterless_view::stream::emoji_key(&id)),
            ),
            Err(error) => eprintln!("asking which emoji exist: {error}"),
        }
        // And every face and emoji already on this disk straight into the
        // atlas, not just onto the disk. A picture otherwise reaches the atlas
        // only when a row first asks for it, so the first frame of every
        // channel was drawn without them and they arrived a frame or two
        // later -- which reads as loading however fast the read is. The emoji
        // were left out at first, and the reader saw them still arriving in
        // the reactions.
        //
        // Only what is held: one that is not would be a fetch, and a start
        // with an empty cache would put all of them ahead of what the reader
        // is looking at. Those are what the warm-up below is for.
        if let Some(shelf) = self.shelf.as_ref() {
            let held = matterless_view::filecache::shared();
            let face = matterless_view::stream::AVATAR as u32;
            let emoji = self.stream.emoji_side();
            let mut preloaded = 0;
            for (at, key) in keys.iter().enumerate() {
                let side = if at < faces { face } else { emoji };
                if held.as_ref().is_some_and(|held| held.holds(key))
                    && self.asked.insert(key.clone())
                {
                    shelf.want(key.clone(), side, side);
                    preloaded += 1;
                }
            }
            println!("{preloaded} faces and emoji into the atlas before anything asks");
        }
        if !keys.is_empty() {
            println!(
                "warming {faces} faces and {} emoji onto the disk, behind everything else",
                keys.len() - faces
            );
            if let Some(link) = self.link.as_ref() {
                link.send(matterless_view::live::Ask::Warm { keys });
            }
        }
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
            Update::Attached {
                channel_id,
                root_id,
                file,
                path,
            } => {
                let under = match root_id.is_empty() {
                    true => channel_id,
                    false => thread_name(&root_id),
                };
                println!("{} is attached and waiting", file.name);
                self.no_longer_on_its_way(&under, &path);
                self.attached.entry(under).or_default().push(*file);
                self.relayout();
                self.redraw();
            }
            Update::NotAttached {
                channel_id,
                root_id,
                path,
                why,
            } => {
                let under = match root_id.is_empty() {
                    true => channel_id,
                    false => thread_name(&root_id),
                };
                eprintln!("{why}");
                self.no_longer_on_its_way(&under, &path);
                self.relayout();
                self.redraw();
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
                // A window that has just signed in for the first time is still
                // showing the invented sample, and `sample` is not a channel
                // any server has: it went out as a channel id in everything
                // the open conversation asks for and came back 400. This is
                // the moment there are real channels to show instead.
                if self.left_the_sample() {
                    return;
                }
                // The counts are not the only thing the mode decides: a reply
                // is a row in the channel under one and not under the other,
                // so the conversation is replanned when it does change.
                if moved && let Some(channel) = self.sidebar.selected.clone() {
                    self.open_channel(&channel);
                }
            }
            Update::Met => {
                // The sidebar labels its conversations from the store, and so
                // does the plan for the one that is open: both were built
                // before these names existed.
                //
                // Re-read rather than reopened. Opening a channel puts the
                // reader at its newest message, so every name learnt -- which
                // happens whenever somebody new comes into view -- threw a
                // reader who had scrolled away back to the bottom.
                self.rebuild_sidebar();
                if let Some(channel) = self.sidebar.selected.clone() {
                    self.reread_channel(&channel);
                }
            }
            Update::Person { user_id } => {
                // Refreshed in place, if the card is still open on them. Not
                // through `show_profile`, which would ask about them again.
                let fresh = self
                    .profile
                    .of
                    .as_ref()
                    .filter(|card| card.user_id == user_id)
                    .and_then(|card| {
                        self.store
                            .as_ref()?
                            .user_by_username(&card.username)
                            .ok()
                            .flatten()
                    });
                if let (Some(user), Some(card)) = (fresh, self.profile.of.as_mut()) {
                    card.full_name = format!("{} {}", user.first_name, user.last_name)
                        .trim()
                        .to_string();
                    card.nickname = user.nickname;
                    card.email = user.email;
                    card.position = user.position;
                    card.avatar_at = user.last_picture_update;
                }
            }
            Update::Statuses(found) => {
                let mine = found.iter().any(|(user_id, _, _)| user_id == &self.me);
                for (user_id, status, active) in found {
                    if active > 0 {
                        self.active_at.insert(user_id.clone(), active);
                    }
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
            Update::Moving {
                key,
                width,
                height,
                frames,
            } => {
                println!("{key} moves: {} frames", frames.len());
                self.moving.keep(
                    &key,
                    matterless_view::moving::Reel::new(
                        width,
                        height,
                        frames,
                        std::time::Instant::now(),
                    ),
                );
            }
            // Straight to a texture of its own rather than into the atlas --
            // and only if it is still the one being looked at, since a reader
            // flicking through outruns the network.
            Update::Looked {
                file_id,
                within,
                width,
                height,
                rgba,
                frames,
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
                match frames {
                    // Played in the viewer. Not remembered as a still: opening
                    // it again from memory would show a GIF that does not move.
                    Some(frames) => {
                        self.looked_moving = Default::default();
                        self.looked_moving.keep(
                            &file_id,
                            matterless_view::moving::Reel::new(
                                width,
                                height,
                                frames,
                                std::time::Instant::now(),
                            ),
                        );
                    }
                    None => self.remembered.keep(&file_id, within, width, height, rgba),
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
            Update::SessionOpened {
                token,
                user_id,
                username,
                server,
            } => {
                match self.opened_a_session(&token, &user_id, &username, &server) {
                    // Straight on to the socket, with the session that has
                    // just been made: the reader signed in to read something,
                    // and a window that then sat there until it was restarted
                    // would have asked for a password to do nothing with.
                    Ok(()) => {
                        if let Some(waker) = self.waker.clone() {
                            self.connect(waker);
                        }
                    }
                    Err(why) => {
                        eprintln!("the session could not be kept: {why}");
                        self.signin.trying = false;
                        self.signin.failed = Some(why);
                    }
                }
            }
            Update::SignInRefused { why, needs_a_code } => {
                eprintln!("sign-in refused: {why}");
                self.signin.trying = false;
                match needs_a_code {
                    true => {
                        let mut input = std::mem::take(&mut self.input);
                        self.signin.ask_for_a_code(&mut input);
                        self.input = input;
                    }
                    false => self.signin.failed = Some(why),
                }
            }
            Update::Changed(deltas) => {
                self.read_what_arrived(&deltas);
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
                            self.learn_a_name(user_id);
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
                // Said no to already. The looking repeats, so this arrives
                // again every couple of hours until the reader either takes it
                // or a newer one comes along -- and re-raising something
                // somebody has dismissed is how a client becomes the thing
                // being read rather than the thing it is read in.
                if !worth_raising(self.put_off.as_deref(), &offer.version) {
                    return;
                }
                self.offered.offer(&offer.version, &offer.notes);
                self.offer = Some(offer);
                let notice = self.notice_rect();
                self.offered.measure(&mut self.fonts, notice);
                // Everything under it is a row shorter now, which is a
                // question for the layout rather than for this.
                self.relayout();
            }
            Update::Checked(found) => {
                // Answered where it was asked, and then handed on: a build
                // that is there is a build the strip along the top should
                // offer, whoever went looking for it.
                let newer = match found {
                    Ok(Some(offer)) => {
                        self.settings
                            .looked(format!("MatterLess {} is ready to install.", offer.version));
                        Some(offer)
                    }
                    Ok(None) => {
                        self.settings.looked("This is the newest build.");
                        None
                    }
                    Err(why) => {
                        eprintln!("could not look for an update: {why}");
                        self.settings
                            .looked("Could not reach the place builds are kept.");
                        None
                    }
                };
                // Before the offer is handed on, because that path relays out
                // the whole window: the panel has just dropped its placement
                // and would be missing from the frame in between.
                let window = self.window_rect();
                self.settings.measure(&mut self.fonts, window);
                if let Some(offer) = newer {
                    self.apply(Update::Updatable(offer));
                }
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
    /// The sidebar's direct messages, and whoever is on screen.
    ///
    /// This asked only about the sidebar for a long time, on the grounds that
    /// the rows in a channel change constantly while the people in them almost
    /// never do. True, and it meant a face in a conversation could never have
    /// a dot: nothing had asked how that person was. The set is deduped and
    /// compared before anything is sent, so the rows changing under an
    /// unchanged cast still costs no request.
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
        // Whoever is in view, in the conversation and in the thread beside it.
        wanted.extend(self.stream.who_is_here(self.stream_rect()));
        // And whoever the switcher is offering, who may be somebody this
        // reader has never written to and so is in no conversation at all.
        wanted.extend(self.switcher.who_is_here());
        if let (Some(within), Some(thread)) = (self.thread_stream_rect(), self.thread.as_ref()) {
            wanted.extend(thread.who_is_here(within));
        }
        // The reader themselves, because their own presence is on the strip
        // at the top of the sidebar and nothing else would ask for it.
        wanted.push(self.me.clone());
        // Only whoever nobody has asked about yet. Everybody the store knew was
        // asked about at sign-in and the socket thread asks again on a beat,
        // so what is left is somebody new -- and asking again every time the
        // faces on screen changed was 705 requests in one walk of the sidebar.
        wanted.retain(|id| !self.presence.contains_key(id) && !self.asked_about.contains(id));
        wanted.sort();
        wanted.dedup();
        if wanted.is_empty() {
            return;
        }
        self.asked_about.extend(wanted.iter().cloned());
        link.send(matterless_view::live::Ask::Statuses { user_ids: wanted });
    }

    /// Tells the server this reader is typing, no more than now and then.
    ///
    /// The rule is `typing::Sending`. Asked *after* the conversation is known,
    /// or a keystroke with no socket to send on would count as having sent.
    fn say_typing(&mut self, root_id: &str) {
        // In a thread, the thread's own channel -- the same conversation the
        // reply will go to, and not necessarily the one on screen.
        let where_to = match root_id.is_empty() {
            true => self.sidebar.selected.clone(),
            false => self.thread_in.clone(),
        };
        let (Some(channel), Some(link)) = (where_to, self.link.as_ref()) else {
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
    fn press(&mut self, press: matterless_layout::row::Press, at: Option<Rect>) {
        use matterless_layout::row::Press;
        match press {
            // Answered by the panel it was pressed in, which is the only thing
            // that knows how tall the row becomes. It never reaches here.
            Press::Whole(_) => {}
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
            Press::Person(username) => self.show_profile(&username, at),
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
    fn show_profile(&mut self, username: &str, at: Option<Rect>) {
        let Some(user) = self
            .store
            .as_ref()
            .and_then(|store| store.user_by_username(username).ok().flatten())
        else {
            println!("nobody called {username} in the local store");
            return;
        };
        // Where the reader actually pressed. Asking the stream where this
        // person is named answers with the first place they are, which is not
        // where the pointer was: the card opened at the top of the screen
        // whatever was clicked further down it.
        let near = at
            .or_else(|| {
                self.stream
                    .pressed_rect(&matterless_layout::row::Press::Person(username.to_string()))
            })
            .unwrap_or_else(|| self.stream_rect());
        self.profile.show(
            matterless_view::profile::Card {
                full_name: format!("{} {}", user.first_name, user.last_name)
                    .trim()
                    .to_string(),
                nickname: user.nickname.clone(),
                status: self.presence.get(&user.id).cloned().unwrap_or_default(),
                avatar_at: user.last_picture_update,
                email: user.email.clone(),
                position: user.position.clone(),
                last_active: self.active_at.get(&user.id).copied().unwrap_or(0),
                username: user.username,
                user_id: user.id.clone(),
            },
            near,
        );
        // Shown from what is held, and asked about again behind it: a record
        // is fetched only the first time somebody is met, so their position
        // and anything changed since would otherwise never arrive.
        if let Some(link) = self.link.as_ref() {
            link.send(matterless_view::live::Ask::Person { user_id: user.id });
        }
        self.want_faces();
    }

    /// What the strip offers for the conversation on screen.
    ///
    /// A direct message cannot be left -- the server would refuse -- so the
    /// button is not there rather than there and refused.
    /// The field on the strip, when the strip is wide enough to hold one.
    ///
    /// From the channel's column, as the drawing and the hit test do. With a
    /// thread open there is no room for a 240-wide field beside the buttons,
    /// and `find` says so rather than squeezing it: a field crowded between
    /// the name on its left and the buttons on its right is worse than a
    /// keystroke.
    fn strip_field(&self) -> Option<Rect> {
        header::find(self.channel_rect(), &self.header_offers())
    }

    /// Where the query is typed.
    ///
    /// The strip's own field while the list hangs under it, and the pane's
    /// header once the pane is open -- because the pane covers the right of
    /// the strip, measured: a 240-wide field at x 546 under a pane starting at
    /// 682. A box a third hidden behind a panel is the bug this set out to
    /// fix, wearing a different hat.
    fn search_field(&self, pane: Rect) -> Rect {
        match self.search.open() {
            true => self.search.in_pane(pane),
            false => self
                .strip_field()
                // The room round the box rather than the box, because the
                // strip has none to spare -- see `Composer::around`.
                .map(|field| self.search.query.around(field))
                .unwrap_or_else(|| self.search.in_pane(pane)),
        }
    }

    fn header_offers(&self) -> Vec<header::Act> {
        // Nothing, when there is no store to answer any of them. Search
        // searches it, Saved and Pinned and Threads are lists out of it, and
        // muting or leaving is a message to a server this window has not
        // reached. A row of buttons that do nothing is worse than an empty
        // strip: it takes a press each to find that out.
        //
        // The field goes with them. `fit` offers one only when `Search` is
        // among the acts, so this is the whole of it.
        if self.store.is_none() {
            return Vec::new();
        }
        let direct = self
            .sidebar
            .selected
            .as_ref()
            .and_then(|id| self.store.as_ref()?.channel(id).ok().flatten())
            .is_some_and(|channel| channel.channel_type == "D" || channel.channel_type == "G");
        header::offered(direct)
    }

    /// Whether there is anywhere to write.
    ///
    /// No store is no conversation: what is on screen then is the sample,
    /// which is something to look at rather than somewhere to talk. And the
    /// threads list has nothing to reply to, which is the case this already
    /// had.
    ///
    /// A box that cannot send is worse than no box. It invites a message,
    /// takes it, lights its own Send button, and then swallows the lot with a
    /// line in a console nobody is reading.
    fn can_write(&self) -> bool {
        self.store.is_some() && !self.on_threads()
    }

    /// Does what a button inside a composer says.
    ///
    /// Attach asks the platform for a file, which this window cannot do yet --
    /// so it says what it would need rather than doing nothing and leaving the
    /// reader to wonder whether the press registered.
    fn pressed_in_composer(&mut self, button: composer::Button, root_id: &str) {
        match button {
            // The editor's own two, which never reach here: it answers its
            // buttons itself, because only it knows which message they are
            // about. Named rather than caught by a wildcard, so adding a
            // third button to a box is a compile error somebody has to read.
            composer::Button::Save | composer::Button::Cancel => {}
            composer::Button::Send => {
                let carrying = self.carrying(root_id);
                let box_of = if root_id.is_empty() {
                    &mut self.composer
                } else {
                    &mut self.thread_composer
                };
                let text = box_of.text().trim().to_string();
                // An empty box with a picture waiting in it is worth sending:
                // the picture is the message, which is what a drop does too.
                if text.is_empty() && !carrying {
                    return;
                }
                box_of.clear(&mut self.fonts);
                if let Some(channel) = self.sidebar.selected.clone() {
                    self.post_message(&channel, root_id, text);
                }
            }
            composer::Button::Attach => {
                println!("drop a file on the window, or paste one into the box");
            }
            // Straight into the box's own text. Markdown is what the server
            // stores and what this window already draws, so a formatting
            // button is a text edit and there is no second representation of
            // a message for anything to keep in step.
            composer::Button::Mark(how) => {
                let box_of = if root_id.is_empty() {
                    &mut self.composer
                } else {
                    &mut self.thread_composer
                };
                box_of.mark_up(how, &mut self.fonts);
                self.relayout();
            }
            composer::Button::Unattach(at) => {
                let under = match root_id.is_empty() {
                    true => self.sidebar.selected.clone().unwrap_or_default(),
                    false => thread_name(root_id),
                };
                // Off the message only. The upload stays on the server, where
                // nothing claims it and nothing is served from it -- the same
                // state a send that failed leaves behind.
                if let Some(held) = self.attached.get_mut(&under)
                    && at < held.len()
                {
                    let gone = held.remove(at);
                    println!("{} is no longer going with this message", gone.name);
                }
                self.relayout();
            }
        }
    }

    /// Hands each box what it is carrying.
    ///
    /// The window holds the uploads, because they belong to a conversation
    /// rather than to a box -- the box is emptied and refilled every time the
    /// reader moves, and what they pasted has to still be there when they
    /// come back. So they are copied down at the one point both boxes are
    /// measured.
    ///
    /// A picture is resolved through the same `FileRef` the conversation
    /// uses, at the same gallery size, so the tray shows the rendition the
    /// message will show and the two can never disagree about which one a
    /// file has. It is also the only honest way to ask "is this a picture":
    /// a mime type alone once called a 4.5MB bitmap an image the server had
    /// declined to decode, and it was laid out in a box nothing by nothing.
    fn show_what_is_attached(&mut self) {
        let going_up = |paths: Option<&Vec<std::path::PathBuf>>| {
            paths
                .map(|paths| {
                    paths
                        .iter()
                        .map(|path| composer::Waiting {
                            name: path
                                .file_name()
                                .map(|name| name.to_string_lossy().to_string())
                                .unwrap_or_default(),
                            picture: None,
                            shape: (1.0, 1.0),
                            on_its_way: true,
                        })
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default()
        };
        let named = |held: Option<&Vec<matterless_core::model::FileInfo>>| {
            held.map(|files| {
                files
                    .iter()
                    .map(|file| {
                        let shown = matterless_render::FileRef::from_info(
                            file,
                            matterless_render::FileLayout {
                                gallery: true,
                                ..Default::default()
                            },
                        );
                        composer::Waiting {
                            name: file.name.clone(),
                            picture: shown
                                .image
                                .then(|| matterless_view::stream::picture_key(&shown)),
                            shape: (shown.box_width as f32, shown.box_height as f32),
                            on_its_way: false,
                        }
                    })
                    .collect()
            })
            .unwrap_or_default()
        };
        let channel = self.sidebar.selected.clone().unwrap_or_default();
        // Arrived first, then the ones still going up: a tile that jumped
        // along the row when its upload finished would move the cross the
        // reader was about to press.
        let mut waiting: Vec<composer::Waiting> = named(self.attached.get(&channel));
        waiting.extend(going_up(self.attaching.get(&channel)));
        self.composer.waiting = waiting;
        let thread = self.open_root().map(|root| thread_name(&root));
        self.thread_composer.waiting = match thread {
            Some(name) => {
                let mut waiting: Vec<composer::Waiting> = named(self.attached.get(&name));
                waiting.extend(going_up(self.attaching.get(&name)));
                waiting
            }
            None => Vec::new(),
        };
    }

    /// Takes a file off the on-its-way list, whichever way it ended.
    ///
    /// By path rather than by name: two drops of `image.png` from different
    /// folders are two files, and matching on the name would take the wrong
    /// tile away.
    fn no_longer_on_its_way(&mut self, under: &str, path: &std::path::Path) {
        if let Some(going) = self.attaching.get_mut(under) {
            if let Some(at) = going.iter().position(|held| held == path) {
                going.remove(at);
            }
            if going.is_empty() {
                self.attaching.remove(under);
            }
        }
    }

    /// Whether anything is waiting to be sent with the next message here.
    fn carrying(&self, root_id: &str) -> bool {
        let under = match root_id.is_empty() {
            true => self.sidebar.selected.clone().unwrap_or_default(),
            false => thread_name(root_id),
        };
        self.attached
            .get(&under)
            .is_some_and(|held| !held.is_empty())
    }

    /// The pill for one line: where it sits, and what fits in it.
    ///
    /// Measured rather than given the width of the column. A pill is only a
    /// pill if it ends where the sentence does -- run the full width and it
    /// is the band this replaced, wearing a rounded corner.
    ///
    /// `None` when there is no room to draw one at all, which is a column
    /// narrower than its own margins.
    fn typing_pill(
        fonts: &mut matterless_layout::Fonts,
        strip: Rect,
        said: &str,
    ) -> Option<(Rect, String)> {
        let room = strip.width - TYPING_INSET * 2.0 - TYPING_PADDING * 2.0;
        if room <= 0.0 {
            return None;
        }
        let style = matterless_view::listing::label(false);
        let said = matterless_layout::elided(fonts, said, room, style);
        let width = matterless_layout::extent_of(fonts, &said, f32::MAX, style).width
            + TYPING_PADDING * 2.0;
        // Centred, not left-aligned. It sat at `TYPING_INSET`, which is where
        // the full-width band's words used to start -- right for a line
        // running the width of the column, and wrong for a pill, which has
        // two ends and reads as pinned to whichever one it touches.
        //
        // Never past its own margin on a column too narrow for it, which is
        // what `max` is for: a centred thing wider than its room would be
        // centred off both edges at once.
        let left = (strip.x + (strip.width - width) / 2.0).max(strip.x + TYPING_INSET);
        Some((Rect::new(left, strip.y, width, strip.height), said))
    }

    /// Looks one person up, if this window has not already.
    ///
    /// Kept rather than asked for again because the line is drawn every
    /// frame: the read belongs to the signal arriving, not to the drawing.
    /// A miss is not recorded, so somebody the store has not heard of yet is
    /// asked about again next time rather than being nameless for the run.
    fn learn_a_name(&mut self, user_id: &str) {
        if self.typing_names.contains_key(user_id) {
            return;
        }
        let known = self
            .store
            .as_ref()
            .and_then(|store| {
                store
                    .users_by_ids(std::slice::from_ref(&user_id.to_string()))
                    .ok()
            })
            .and_then(|known| known.get(user_id).map(|user| user.username.clone()));
        if let Some(name) = known {
            self.typing_names.insert(user_id.to_string(), name);
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
                self.show_the_list("Saved");
                if let Some(link) = self.link.as_ref() {
                    link.send(matterless_view::live::Ask::Saved);
                }
            }
            header::Act::Pinned => {
                if let Some(channel) = self.sidebar.selected.clone() {
                    self.show_the_list("Pinned");
                    if let Some(link) = self.link.as_ref() {
                        link.send(matterless_view::live::Ask::Pinned {
                            channel_id: channel,
                        });
                    }
                }
            }
            // No field on the strip to type into, so the pane's own is what
            // this opens. Which is where the query goes once the pane is up
            // anyway -- the pane covers the right of the strip, so the two
            // are never both in use.
            header::Act::Search => {
                let mut input = std::mem::take(&mut self.input);
                self.search.widen(&mut input);
                self.input = input;
            }
            // Whatever the strip could not hold, as a menu under the mark.
            // Asked at the moment of the press rather than remembered: the
            // window can have been resized since it was drawn, and a menu of
            // what used to be folded would offer the wrong things.
            header::Act::More => {
                let strip = header::fit(self.channel_rect(), &self.header_offers());
                let Some((_, under)) = strip
                    .shown
                    .iter()
                    .find(|(act, _)| *act == header::Act::More)
                else {
                    return;
                };
                let muted = self.muted();
                let items: Vec<matterless_view::menu::Item> = strip
                    .folded
                    .iter()
                    .map(|act| {
                        matterless_view::menu::Item::new(
                            &format!("header.{}", act.slug()),
                            act.explains(muted),
                        )
                        .marked(act.label(muted))
                    })
                    .collect();
                self.menu.show(
                    "header",
                    matterless_view::menu::Anchor::Under(*under),
                    matterless_view::menu::Style::channel(),
                    items,
                );
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

    /// Suggestions for the misspelled word under a right-click in a message
    /// box, and a way to say it is right.
    fn offer_spelling_menu(&mut self) {
        use matterless_view::menu::{Item, Style};
        let Some(name) = self.input.contexted().map(str::to_string) else {
            return;
        };
        let Some((x, y)) = self.input.pointer_at() else {
            return;
        };
        let found = if name == composer::NAME {
            self.composer
                .misspelled_at(header::below(self.channel_rect()), x, y)
        } else if name == THREAD_COMPOSER {
            self.thread_body()
                .and_then(|body| self.thread_composer.misspelled_at(body, x, y))
        } else if name == matterless_view::edit::NAME {
            self.edited_row().and_then(|row| {
                let field = self.edit.field(row, self.stream_rect());
                self.edit.box_of.misspelled_at(field, x, y)
            })
        } else {
            return;
        };
        let (Some((line, range, word, sentence)), Some(speller)) =
            (found, matterless_view::spell::speller())
        else {
            return;
        };
        let offered = speller.suggest(&word, &sentence);
        let mut items: Vec<Item> = offered
            .iter()
            .map(|one| Item::new(&format!("spell.use.{one}"), one))
            .collect();
        if items.is_empty() {
            items.push(Item::new("spell.none", "No suggestions"));
        }
        items.push(Item::rule());
        items.push(Item::new("spell.learn", "Add to Dictionary"));
        // Which box, where in it, and the word -- handed back with the choice.
        let about = [
            name,
            line.to_string(),
            range.start.to_string(),
            range.end.to_string(),
            word,
        ]
        .join("\u{1f}");
        self.menu.show(
            &about,
            matterless_view::menu::Anchor::At(x, y),
            Style::message(),
            items,
        );
    }

    /// A spelling suggestion taken, or a word taught.
    fn act_on_spelling(&mut self, about: &str, chosen: &str) {
        let parts: Vec<&str> = about.split('\u{1f}').collect();
        let [name, line, start, end, word] = parts.as_slice() else {
            return;
        };
        let (Ok(line), Ok(start), Ok(end)) = (line.parse(), start.parse(), end.parse()) else {
            return;
        };
        if chosen == "learn" {
            matterless_view::spell::learn_all([word.to_string()]);
            // Kept, so it is still right after a restart.
            if let Some(store) = self.store.as_ref() {
                let mut kept: Vec<String> = store
                    .setting(matterless_view::spell::OWN_WORDS)
                    .ok()
                    .flatten()
                    .map(|saved| saved.lines().map(str::to_string).collect())
                    .unwrap_or_default();
                if !kept.iter().any(|one| one.eq_ignore_ascii_case(word)) {
                    kept.push(word.to_string());
                }
                if let Err(error) =
                    store.remember_setting(matterless_view::spell::OWN_WORDS, &kept.join("\n"))
                {
                    eprintln!("keeping a word: {error}");
                }
            }
            println!("a word was added to the dictionary");
        } else if let Some(with) = chosen.strip_prefix("use.") {
            let box_of = match *name {
                composer::NAME => &mut self.composer,
                THREAD_COMPOSER => &mut self.thread_composer,
                matterless_view::edit::NAME => &mut self.edit.box_of,
                _ => return,
            };
            box_of.replace_word(&mut self.fonts, line, start..end, with);
        }
        self.relayout();
        self.redraw();
    }

    /// Runs whichever item was chosen.
    ///
    /// The channel menu's ids carry a prefix and the message menu's do not,
    /// which is what says which of the two answered: both offer a "Mark as
    /// Unread" and they mean different things by it.
    fn act_on_menu(&mut self, about: &str, chosen: &str) {
        use matterless_view::actions::Action;
        if let Some(rest) = chosen.strip_prefix("spell.") {
            self.act_on_spelling(about, rest);
            return;
        }
        if let Some(rest) = chosen.strip_prefix("channel.") {
            self.act_on_channel_menu(about, rest);
            return;
        }
        // The strip's own overflow, which does exactly what the button it
        // stands in for would have done.
        if let Some(rest) = chosen.strip_prefix("header.") {
            if let Some(act) = header::Act::from_slug(rest) {
                self.act_on_header(act);
            }
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
                    self.copy(said.message);
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
                    self.copy(link);
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
            // The panel has already changed what it measures; this is the
            // measuring, which needs the fonts it does not have.
            Some(Chose::Reshape) => self.relayout(),
            // Under the button that was pressed, which is what the quick row
            // of faces was sitting on before it made room for the whole grid.
            Some(Chose::Pick { post_id, under }) => {
                self.picked_near = under;
                self.picker.show(&post_id, &mut self.fonts, &mut self.input);
            }
            Some(Chose::Thread(root)) => self.open_thread(&root),
            Some(Chose::Retry(pending)) => self.retry(&pending),
            Some(Chose::Act {
                action,
                post_id,
                on,
            }) => self.act(action, post_id, on),
            Some(Chose::Press { press, at }) => self.press(press, at),
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
    ///
    /// Memory first, then the disk, then the server: a picture opened again
    /// took seconds each time, fetched afresh behind whatever the socket
    /// thread was busy with.
    fn fetch_looked(&mut self, one: &matterless_view::viewer::Looking) {
        if let Some((width, height, rgba)) = self.remembered.get(&one.file_id, self.size)
            && let Some(view) = self.view.as_mut()
        {
            view.show(width, height, rgba);
            self.viewer.arrived(&one.file_id, (width, height));
            return;
        }
        if let Some(shelf) = self.shelf.as_ref()
            && shelf.look(one.file_id.clone(), one.original, one.linked, self.size)
        {
            return;
        }
        let Some(link) = self.link.as_ref() else {
            return;
        };
        link.send(matterless_view::live::Ask::Look {
            file_id: one.file_id.clone(),
            original: one.original,
            linked: one.linked,
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
            if id == matterless_view::rail::UNREAD {
                return Some("Unread channels".to_string());
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
        // A pressable run of words. A mention and a channel say where they
        // lead; a link says where it goes, which the app leaves to the
        // browser's status bar and this window has nowhere else to put.
        if let Some(at) = rest.strip_prefix("press/") {
            let at = at.parse::<usize>().ok()?;
            let (press, _) = stream.presses_seen().get(at)?;
            return Some(match press {
                // The words say it themselves -- "Show the other 40 lines" --
                // so a tooltip repeating them is one more thing to read.
                matterless_layout::row::Press::Whole(_) => return None,
                matterless_layout::row::Press::Person(_) => "Show profile".to_string(),
                matterless_layout::row::Press::Channel(_) => "Go to channel".to_string(),
                matterless_layout::row::Press::Post { .. } => "Go to the message".to_string(),
                matterless_layout::row::Press::Link(href) => href.clone(),
            });
        }
        let (index, what) = rest.strip_prefix("row/")?.split_once('/')?;
        let index = index.parse::<usize>().ok()?;
        // One of the reader's most-used on the strip: the emoji it stands
        // for, by name -- a picture of a face does not say what reacting with
        // it means.
        if let Some(at) = what.strip_prefix("quick/") {
            let at = at.parse::<usize>().ok()?;
            let (emoji, _) = stream.favourites.get(at)?;
            return Some(format!(":{emoji}:"));
        }
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
                        kind: matterless_view::rail::Kind::Team,
                    })
                    .collect()
            })
            .unwrap_or_default();
        // What is waiting in each, summed from the rows themselves. A muted
        // channel contributes nothing: muting says "do not interrupt me", and
        // a dot on the team is an interruption at one remove.
        let mut directs = matterless_view::rail::Tile {
            id: matterless_view::rail::DIRECTS.to_string(),
            // A name rather than a picture: the tile draws a mark of its own
            // now, and this is what the tooltip and the tests read.
            name: "Direct messages".to_string(),
            unread: 0,
            mentions: 0,
            kind: matterless_view::rail::Kind::Directs,
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
        // Last, under everything that is somewhere to go: it is about the
        // program rather than about any conversation in it.
        teams.push(matterless_view::rail::Tile {
            id: matterless_view::rail::SETTINGS.to_string(),
            name: "Settings".to_string(),
            unread: 0,
            mentions: 0,
            kind: matterless_view::rail::Kind::Settings,
        });
        // Above the teams, because it is not one of them: it is the one square
        // that does something to the conversation already open rather than
        // taking the reader to another.
        teams.insert(
            0,
            matterless_view::rail::Tile {
                id: matterless_view::rail::UNREAD.to_string(),
                name: "Unread".to_string(),
                unread: 0,
                mentions: 0,
                kind: matterless_view::rail::Kind::Unread,
            },
        );
        self.rail.teams = teams;
        self.rail.chosen = self
            .sidebar
            .selected
            .as_ref()
            .and_then(|id| self.store.as_ref()?.channel(id).ok().flatten())
            .map(|channel| channel.team_id);
        let open = self.sidebar.selected.clone();
        let scroll = self.sidebar.scroll;
        // "Signing in" is only true while something is: with no session and
        // no server there is nothing to sign in to, and a window that says it
        // is trying for ever is worse than one that says what is missing.
        let name = match self.offline {
            Some(_) if self.me.is_empty() => "Not signed in".to_string(),
            _ => self.my_name(),
        };
        let status = self.presence.get(&self.me).cloned().unwrap_or_default();
        self.sidebar = Sidebar::new(Self::entries(
            self.store.as_deref(),
            &self.me,
            self.threads,
            Who {
                name: &name,
                status: &status,
                live: self.connected,
                offline: self.offline,
            },
        ));
        self.sidebar.selected = open;
        self.sidebar.scroll = scroll;
    }

    /// Records why the window stayed offline, and puts the way in on screen.
    ///
    /// The reason still goes in the sidebar, under the reader's name, for the
    /// moment a session is lost rather than never had. What a reader sees on
    /// a cold start with nothing at all is the sign-in screen, which is the
    /// only one of the two they can do anything about.
    fn stayed_offline(&mut self, why: &'static str) {
        self.offline = Some(why);
        self.rebuild_sidebar();
        self.signin.start(&mut self.input);
        // Whatever is already there is filled in, so a reader who has been
        // here before is only asked for what is actually missing.
        if let Some(path) = matterless_view::feed::default_store()
            && let Some(server) = matterless_view::live::stored_server(&path)
        {
            self.signin.server.set(&server);
        }
        self.want_the_logo();
        self.pretend_typed();
    }

    /// Fills the form in, in a dev build, when asked.
    ///
    /// The same argument `pretend_offered` makes: a form has states nobody
    /// working on it can see without a real account on a real server -- the
    /// button live rather than spent, the field for a one-time code, the line
    /// under it that says what went wrong. `MATTERLESS_SIGNIN=host|login|word`
    /// puts something in each box.
    ///
    /// A dev build only, and the password is whatever was put on a command
    /// line: this is for looking at the screen, not for signing in.
    fn pretend_typed(&mut self) {
        let Ok(said) = std::env::var("MATTERLESS_SIGNIN") else {
            return;
        };
        if !cfg!(debug_assertions) {
            eprintln!("MATTERLESS_SIGNIN is ignored outside a dev build");
            return;
        }
        let mut parts = said.split('|');
        if let Some(server) = parts.next() {
            self.signin.server.set(server);
        }
        if let Some(login) = parts.next() {
            self.signin.login.set(login);
        }
        if let Some(password) = parts.next() {
            self.signin.password.set(password);
        }
        self.signin.wants_code = std::env::var("MATTERLESS_SIGNIN_CODE").is_ok();
        self.signin.failed = std::env::var("MATTERLESS_SIGNIN_FAILED").ok();
    }

    /// Opens a real conversation, if the one on screen is not one.
    ///
    /// Answers whether it did. What a window shows before it has ever signed
    /// in is the sample, under a channel id no server knows -- and signing in
    /// does not by itself change which conversation is open, so without this
    /// the reader watched 116 channels arrive into the sidebar beside a
    /// conversation that was still invented.
    ///
    /// The first channel the sidebar offers, rather than a fresh guess at the
    /// busiest: the sidebar is already the reader's own arrangement, and with
    /// unread conversations lifted to the top its first row is the one most
    /// worth opening.
    fn left_the_sample(&mut self) -> bool {
        let Some(store) = self.store.clone() else {
            return false;
        };
        let real = self
            .sidebar
            .selected
            .as_deref()
            .is_some_and(|id| matches!(store.channel(id), Ok(Some(_))));
        if real {
            return false;
        }
        let Some(first) = self.sidebar.entries.iter().find_map(|entry| match entry {
            Entry::Channel { id, .. } => Some(id.clone()),
            _ => None,
        }) else {
            return false;
        };
        println!("leaving the sample for {first}");
        self.sidebar.selected = Some(first.clone());
        self.open_channel(&first);
        true
    }

    /// Whether there is anything to see: a window that is up rather than one
    /// that has the reader's attention.
    ///
    /// Not `focused`, which was the first answer and the wrong one: a window
    /// read beside an editor is not focused, and a picture that stops moving
    /// whenever the pointer goes elsewhere reads as a picture that has broken
    /// rather than as a window being frugal. Minimised or in the tray, there
    /// is nothing to draw for and this goes quiet.
    fn on_show(&self) -> bool {
        self.window
            .as_ref()
            .is_some_and(|window| !window.is_minimized().unwrap_or(false))
    }

    /// When the window next has to wake for a picture that moves.
    ///
    /// `None` whenever nothing is moving on screen, which is nearly always --
    /// and that is what keeps this out of the way: the window then waits for
    /// something to happen rather than for a clock, exactly as it did before
    /// any picture moved.
    fn playing(&self) -> Option<std::time::Instant> {
        if !self.on_show() {
            return None;
        }
        let now = std::time::Instant::now();
        // The picture in the viewer, which is in a texture of its own.
        let viewed = self.viewer.current().map(|one| one.file_id.as_str());
        let looked = self.looked_moving.wakes(now, |key| Some(key) == viewed);
        if self.moving.is_empty() {
            return looked;
        }
        let view = self.view.as_ref()?;
        let atlas = self.moving.wakes(now, |key| view.atlas.drawn(key));
        match (looked, atlas) {
            (Some(looked), Some(atlas)) => Some(looked.min(atlas)),
            (looked, atlas) => looked.or(atlas),
        }
    }

    /// Puts a half-written message away a couple of seconds after the typing
    /// stops, and says when to come back if it is not time yet.
    ///
    /// A draft used to reach the store only when the conversation was left,
    /// which is a fine moment to save one and a poor one to rely on: anything
    /// that ends the window without a channel change -- an update installing
    /// itself, a machine going down, the program falling over -- took whatever
    /// was in the box with it.
    ///
    /// Read off the text rather than hooked into the keyboard, because a box
    /// changes in a dozen ways: a paste, a drop, a deletion, an emoji off the
    /// grid, a name off the completion list. Comparing what it says now with
    /// what it said when it was last looked at catches all of them and cannot
    /// be forgotten in one.
    fn settled_draft(&mut self) -> Option<std::time::Instant> {
        let said = (self.composer.text(), self.thread_composer.text());
        if said != self.watched {
            self.watched = said;
            // Not "somebody typed a moment ago" but "somebody is typing":
            // every change pushes the clock out again, so what this measures
            // is the pause at the end rather than the first keystroke.
            self.typed_at = Some(std::time::Instant::now());
        }
        let at = self.typed_at?;
        if at.elapsed() < WRITTEN {
            return Some(at + WRITTEN);
        }
        self.typed_at = None;
        // Under the conversation each box belongs to, and both, because a
        // reply in the pane is as much a draft as a message in the channel.
        let (channel, thread) = (self.writing_to.clone(), self.replying_to.clone());
        let (said, replied) = (self.watched.0.clone(), self.watched.1.clone());
        self.park(channel, said);
        self.park(thread, replied);
        None
    }

    /// Whether a picture on screen is showing a frame it has outlasted.
    fn overdue(&self) -> bool {
        let viewed = self.viewer.current().map(|one| one.file_id.as_str());
        if self.on_show()
            && self
                .looked_moving
                .overdue(std::time::Instant::now(), |key| Some(key) == viewed)
        {
            return true;
        }
        if !self.on_show() || self.moving.is_empty() {
            return false;
        }
        let Some(view) = self.view.as_ref() else {
            return false;
        };
        self.moving
            .overdue(std::time::Instant::now(), |key| view.atlas.drawn(key))
    }

    /// Queues whatever frame each moving picture is due.
    ///
    /// Only what the atlas is still holding, which is what "on screen" means
    /// here: a picture is put in before the frame that draws it and thrown out
    /// a few frames after the last one that did, so the atlas already answers
    /// the question and answers it without walking the scene.
    ///
    /// And only while the window is being looked at. A conversation left open
    /// behind an editor should cost nothing at all, and a loop nobody can see
    /// is a texture upload and a woken thread for each frame of it.
    fn played(&mut self) {
        // The picture in the viewer: its next frame, as the texture it is drawn
        // from.
        if self.on_show() {
            let viewed = self.viewer.current().map(|one| one.file_id.clone());
            let due = self.looked_moving.due(std::time::Instant::now(), |key| {
                viewed.as_deref() == Some(key)
            });
            if let Some(view) = self.view.as_mut() {
                for (_, width, height, rgba) in due {
                    view.show(width, height, &rgba);
                }
            }
        }
        if !self.on_show() || self.moving.is_empty() {
            return;
        }
        let Some(view) = self.view.as_ref() else {
            return;
        };
        let due = self
            .moving
            .due(std::time::Instant::now(), |key| view.atlas.drawn(key));
        let Some(view) = self.view.as_mut() else {
            return;
        };
        // Over the frame showing rather than into the queue the arriving
        // pictures use: that queue ends at `put_image`, which answers a key it
        // already holds with the slot it already gave and writes nothing. A
        // frame of a picture already on screen is the one thing in this window
        // that has to overwrite what is under its own key.
        for (key, width, height, rgba) in due {
            view.put_frame(&key, &rgba, width, height);
        }
    }

    /// Goes and looks for a newer build, because the reader asked.
    ///
    /// The periodic look needs a session to have been opened before it starts,
    /// and a dev build never starts it at all. This one needs neither: it is
    /// somebody pressing a button, and the answer is the same whether or not
    /// the window ever reached a server.
    fn look_for_a_build(&mut self) {
        let Some(waker) = self.waker.clone() else {
            // No event loop to answer on, which is the window not being up
            // yet. Nothing can have pressed the button.
            self.settings.looked("Nothing to look with yet.");
            return;
        };
        matterless_view::live::look_once(Proxy(waker));
    }

    /// Draws text the way the reader asked, and remembers that they did.
    ///
    /// The glyphs already rasterised are the wrong ones now -- a sheet of
    /// flat coverage read as three channels is a letter with the wrong edges
    /// -- so the atlas forgets them and the window is laid out again against
    /// the ones that replace them.
    fn draw_text_as(&mut self, mode: matterless_view::settings::Text) {
        let want = mode == matterless_view::settings::Text::Subpixel;
        let changed = self
            .view
            .as_mut()
            .is_some_and(|view| view.atlas.rasterise_subpixel(want));
        if let Some(store) = self.store.as_ref()
            && let Err(error) =
                store.remember_setting(matterless_view::settings::Text::SETTING, mode.stored())
        {
            eprintln!("keeping the text setting: {error}");
        }
        if changed {
            println!("text is drawn {}", mode.stored());
            self.relayout();
        }
    }

    /// Whether the window is asking to be signed in rather than drawing a
    /// conversation.
    fn signing_in(&self) -> bool {
        self.offline.is_some()
    }

    /// Puts the greyscale mark in the queue the atlas is filled from.
    ///
    /// Through `arrived` like every other picture, because the thread that
    /// owns the GPU is the only one that may touch the atlas -- and counted in
    /// `asked`, so it is decoded once rather than on every frame it is
    /// visible.
    fn want_the_logo(&mut self) {
        let key = matterless_view::signin::LOGO_KEY.to_string();
        if self.asked.insert(key.clone())
            && let Some((width, height, rgba)) = matterless_view::signin::logo()
        {
            self.arrived.push((key, width, height, rgba));
        }
    }

    /// Takes the session a sign-in came back with.
    ///
    /// Everything here is the first run this window never had: the database
    /// it will keep messages in does not exist yet, nor the folder round it,
    /// nor the file naming the server. `feed::open` refuses a path that is not
    /// already a file, which is why the store is opened here rather than
    /// through it.
    /// Marks every tile whose picture the atlas cannot draw yet.
    ///
    /// Answered from the atlas rather than from the upload, because "the
    /// server has the file" and "this window can show it" are two different
    /// things with a fetch between them.
    fn still_waiting(view: Option<&matterless_view::View>, box_of: &mut composer::Composer) {
        for held in &mut box_of.waiting {
            if let Some(key) = held.picture.as_deref() {
                held.on_its_way = !view.is_some_and(|view| view.atlas.holds(key));
            }
        }
    }

    /// Whether any tile in either box is still waiting for something.
    fn anything_loading(&self) -> bool {
        [&self.composer, &self.thread_composer]
            .into_iter()
            .any(|box_of| box_of.waiting.iter().any(|held| held.on_its_way))
    }

    /// Makes an empty store where this build keeps one, for a start that has
    /// everything except a database.
    ///
    /// Only when it can actually be filled. An empty database with nobody to
    /// sign in as is a worse answer than saying there is none: the window
    /// would look signed out *and* claim to have a store, and the next start
    /// would find a file and stop asking why it is empty.
    fn made_a_store(&mut self) -> Option<Arc<matterless_store::Store>> {
        let path = matterless_view::feed::default_store()?;
        matterless_view::live::stored_server(&path)?;
        self.session
            .clone()
            .or_else(matterless_view::live::stored_token)?;
        if let Err(error) = std::fs::create_dir_all(path.parent()?) {
            eprintln!("could not make {}: {error}", path.display());
            return None;
        }
        match matterless_store::Store::open(&path) {
            Ok(store) => {
                println!("no store yet, so one was made at {}", path.display());
                let store = Arc::new(store);
                self.store = Some(store.clone());
                Some(store)
            }
            Err(error) => {
                eprintln!("could not make a store at {}: {error}", path.display());
                None
            }
        }
    }

    fn opened_a_session(
        &mut self,
        token: &matterless_core::auth::AuthToken,
        user_id: &str,
        username: &str,
        server: &str,
    ) -> Result<(), String> {
        let path = matterless_view::feed::default_store()
            .ok_or_else(|| "there is nowhere to keep a message store".to_string())?;
        let folder = path
            .parent()
            .ok_or_else(|| format!("{} has no folder", path.display()))?;
        std::fs::create_dir_all(folder)
            .map_err(|error| format!("could not make {}: {error}", folder.display()))?;
        let store = matterless_store::Store::open(&path)
            .map_err(|error| format!("could not open {}: {error}", path.display()))?;
        matterless_view::live::remember_server(&path, server)?;
        // Held for this run before anything is asked to keep it for the next.
        self.session = Some(token.bearer().to_string());
        // And keeping it is allowed to fail. It only decides whether the next
        // launch has to ask again -- this one is signed in either way, and
        // refusing to go on because a keychain would not take a write is
        // refusing over something the reader has already done successfully.
        if let Err(error) = matterless_view::live::remember_token(token.bearer()) {
            eprintln!("{error} -- signed in for this run, and it will ask again next time");
        }
        println!("signed in as {username}, store at {}", path.display());
        self.store = Some(Arc::new(store));
        self.me = user_id.to_string();
        self.signin.trying = false;
        self.signin.failed = None;
        // The password is not kept a moment past the request it was for.
        self.signin.password.set("");
        self.offline = None;
        self.rebuild_sidebar();
        Ok(())
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
        // Taken before anything is sent: whatever was pasted into this
        // conversation goes with this message and with no other, and a send
        // that fails carries them on its retry rather than losing them.
        let under = match root_id.is_empty() {
            true => channel_id.to_string(),
            false => thread_name(root_id),
        };
        let files = self.attached.remove(&under).unwrap_or_default();
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
                files: files.clone(),
            });
        link.send(matterless_view::live::Ask::Send {
            pending_post_id,
            channel_id: channel_id.to_string(),
            root_id: root_id.to_string(),
            message,
            file_ids: files.iter().map(|file| file.id.clone()).collect(),
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
    /// A message that lands in the conversation on screen has been read, so
    /// the server is told.
    ///
    /// `open_channel` was the only place that ever said a channel had been
    /// read, which marked it on the way *in* and never again: the badge lit
    /// while somebody sat in the conversation watching the message arrive,
    /// and stayed lit until they left and came back.
    ///
    /// The same bug, in a different path, was found once before -- the
    /// channel the window starts on had never been through `open_channel`,
    /// "so its unread count climbed for as long as the reader kept looking at
    /// it". That one was cured by routing start-up through `open_channel`,
    /// which left this one standing.
    ///
    /// The condition is `announce`'s, deliberately: the same open-and-focused
    /// test that decides a message needs no notification *because the reader
    /// can already see it*. If the two disagreed, a message could be both too
    /// visible to interrupt anybody about and too unseen to count as read,
    /// which is the worst of each.
    ///
    /// Any post, not only the ones worth a notification. `announce` walks
    /// `notify: true` alone, and a message that does not interrupt -- the
    /// reader's own, or one in a muted conversation -- is still a message
    /// that has been read.
    fn read_what_arrived(&mut self, deltas: &[matterless_sync::Delta]) {
        // Focus is half of the rule: the same channel behind another window is
        // not being read.
        if !self.focused {
            return;
        }
        let Some(open) = self.sidebar.selected.clone() else {
            return;
        };
        // Once for the batch rather than once for each message in it: a busy
        // conversation would otherwise be a request per post.
        let arrived = deltas.iter().any(|delta| {
            matches!(
                delta,
                matterless_sync::Delta::PostUpserted { channel_id, .. } if channel_id == &open
            )
        });
        if arrived {
            self.read_what_is_open();
        }
    }

    /// Says the conversation on screen has been read, if anything in it has
    /// not been.
    ///
    /// Asked of the store first so that sitting in a channel that is already
    /// read costs nothing: this is reached from a focus change and from every
    /// batch of messages, and a request each time would be traffic to say
    /// what the server already believes.
    fn read_what_is_open(&mut self) {
        let (Some(store), Some(open), Some(link)) = (
            self.store.as_ref(),
            self.sidebar.selected.clone(),
            self.link.as_ref(),
        ) else {
            return;
        };
        // The threads row fills the same column but is not a conversation, so
        // there is nothing to mark.
        if open == matterless_view::sidebar::THREADS {
            return;
        }
        let waiting = store
            .unread(&open, &self.me)
            .ok()
            .flatten()
            .is_some_and(|unread| unread.messages > 0 || unread.mentions > 0);
        if !waiting {
            return;
        }
        link.send(matterless_view::live::Ask::MarkRead { channel_id: open });
    }

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
        // A window opened quietly is one opened beside the real client -- a
        // build from the tree, run to be measured. It raised every toast the
        // installed client was raising at the same moment, so the reader got
        // each notification twice, the second for a message already read.
        // It still decides, and says so in the log; it does not interrupt.
        let quiet = std::env::var_os("MATTERLESS_QUIET").is_some();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|since| since.as_millis() as i64)
            .unwrap_or(0);
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
            // And how old the message is, which is what tells a live one
            // from a message the reader has had for hours being announced
            // again at startup.
            let age = store
                .post(post_id)
                .ok()
                .flatten()
                .map(|post| (now - post.create_at) / 1000);
            println!(
                "notifying about {channel_id} ({} characters, named: {}, {} seconds old){}",
                said.preview.chars().count(),
                said.resolved,
                age.map_or_else(|| "?".to_string(), |age| age.to_string()),
                if quiet { ", quietly" } else { "" }
            );
            if quiet {
                continue;
            }
            asked = true;
            let Some(clicked) = self.clicked.clone() else {
                continue;
            };
            matterless_view::toast::raise(channel_id, &title, &body, clicked);
        }
        // The button lights for the same things the toast fires for, and only
        // while the window is not being looked at: asking for attention you
        // already have is how an app becomes irritating. Whether the reader
        // was named is the badge's to say -- the button has one state, and
        // "lit twice as hard" is not a thing a taskbar can do.
        if asked && !self.focused {
            self.taskbar
                .ask_for_attention(raw_window(self.window.as_ref()));
        }
        // Whatever arrived changed what is waiting.
        self.update_badge();
    }

    /// Asks for the faces on screen that have not been asked for yet.
    ///
    /// Called after anything that changes what is visible -- a scroll, a new
    /// message, a channel switch. The asked set is what keeps that from being a
    /// request per frame.
    /// Reads which reactions this reader uses most, for the toolbar.
    ///
    /// Counted from the store rather than kept as a list of recents: the
    /// answer is already in the reactions table, it survives a restart with
    /// nothing written down, and it cannot drift from the truth because it is
    /// the truth. Asked when the conversation changes rather than per frame --
    /// it is a query, and the answer only moves when somebody reacts.
    fn recall_favourites(&mut self) {
        let Some(store) = self.store.as_ref() else {
            return;
        };
        let many = matterless_view::actions::FAVOURITES as u32;
        let favourites: Vec<(String, String)> = store
            .favourite_emoji(&self.me, many)
            .unwrap_or_default()
            .into_iter()
            // Only the ones that can be drawn as a character. A custom emoji
            // is a picture the atlas has to be holding, which the strip has
            // nowhere to wait for.
            .filter_map(|name| {
                let face = matterless_render::emoji::character_for(&name)?;
                Some((name, face))
            })
            .collect();
        self.stream.favourites = favourites.clone();
        if let Some(thread) = self.thread.as_mut() {
            thread.favourites = favourites;
        }
    }

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
        wanted.extend(self.switcher.wants());
        // The tray shows a picture rather than naming it, so it needs the same
        // thumbnail the conversation would.
        wanted.extend(self.composer.wants());
        wanted.extend(self.thread_composer.wants());
        for (key, width, height) in wanted {
            if self.asked.insert(key.clone()) {
                match self.shelf.as_ref() {
                    // The disk first, always: a face that is already here
                    // appears in the frame it is asked for rather than a third
                    // of a second later, and the socket thread never hears
                    // about it.
                    Some(shelf) => {
                        shelf.want(key, width, height);
                    }
                    None => {
                        link.send(matterless_view::live::Ask::Fetch {
                            key,
                            width,
                            height,
                            shown: None,
                        });
                    }
                }
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
            // The ones it already had: a retry claims the same uploads, which
            // are still on the server whether or not the post ever landed.
            file_ids: held.files.iter().map(|file| file.id.clone()).collect(),
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
                // From the menu rather than from the quick row, which carries
                // its own anchor. Under the *top* of the message: its bottom
                // is where a crash notice's stack trace ends, which is off the
                // screen, and a panel pushed back on from there covers the
                // message it was opened from.
                let stream = self.stream_rect();
                let row = self.stream.row_rect(&post_id, stream);
                self.picked_near = row
                    .map(|row| matterless_ui::Rect::new(row.x + 60.0, row.y, 0.0, 0.0))
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
                self.open_the_editor(&post_id, &said.message);
            }
            Action::Link => match self.permalink(&post_id) {
                Some(link) => {
                    self.copy(link);
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
        // Where the reader is, taken before the rows are replaced. Taken
        // after, by `relayout`, it was measured against the new rows and the
        // old layouts, came back as nothing, and every re-read put a reader in
        // the middle of the channel at its foot -- any message landing, any
        // edit, any reaction, any name learnt. The same "asked too late" fault
        // as the drag anchor, the stale hit box and the growing box.
        let held = self.anchors();
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
                self.shape(Shaping::Everything, held);
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
            // Written down now it is all measured, so the next opening of
            // this channel knows its length from the first frame.
            self.remember_heights();
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
        // Over everything, so nothing behind it can be pressed through it.
        if self.settings.open() {
            return self.settings.boxes(self.window_rect());
        }
        if self.signing_in() {
            let mut boxes = self.signin.boxes(self.below_notice());
            // After, so the strip is the innermost match along the top: `at`
            // takes the last box holding the point, not the shallowest.
            boxes.extend(self.offered.boxes(self.notice_rect()));
            return boxes;
        }
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
        boxes.extend(header::boxes(self.channel_rect(), &self.header_offers()));
        if let Some(pane) = self.thread_rect() {
            boxes.extend(header::boxes(pane, &header::for_thread()));
        }
        if self.on_threads() {
            // Deeper than the conversation's own rows, which are not drawn --
            // and shallower than anything that floats over the column.
            boxes.extend(self.followed.boxes_in(self.followed_rect(), 3));
        } else if self.can_write() {
            boxes.extend(self.composer.boxes_in(header::below(self.channel_rect())));
        }
        if let Some(body) = self.thread_body() {
            boxes.extend(self.thread_composer.boxes_in(body));
        }
        if let Some((_, box_of, within)) = self.naming_in() {
            boxes.extend(self.naming.boxes(box_of, within));
        }
        boxes.extend(self.picker.boxes(self.picked_near, self.picker_within()));
        // The list under the strip has no pane of its own, so its boxes are
        // placed from the column rather than from one.
        if self.search.asking()
            && let Some(field) = self.strip_field()
        {
            boxes.extend(self.search.boxes(matterless_view::search::Shown {
                pane: matterless_view::aside::rect(self.column_rect(), self.pane_width),
                field: self.search.query.around(field),
            }));
        }
        if let Some(pane) = self.aside_rect() {
            boxes.extend(self.listing.boxes(pane));
            boxes.extend(self.search.boxes(matterless_view::search::Shown {
                pane,
                field: self.search_field(pane),
            }));
        }
        // The edge itself, over whatever is drawn on either side of it: it
        // is six pixels wide and everything it overlaps is something else's,
        // so a shallower box would take every press that landed on it.
        if let Some(pane) = self.aside_rect().or_else(|| self.thread_rect()) {
            boxes.push(Placed {
                name: GRIP.to_string(),
                rect: matterless_view::aside::grip(pane),
                depth: 6,
            });
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
        // The room the row made, under its name: the editor goes *in* the
        // conversation, so the header above it is the message's own.
        self.stream
            .editing_rect(post_id, self.stream_rect())
            .or_else(|| {
                self.thread
                    .as_ref()
                    .zip(self.thread_stream_rect())
                    .and_then(|(thread, within)| thread.editing_rect(post_id, within))
            })
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
        // Taken from the root rather than from the column, which may be
        // showing the Threads list and not a conversation at all.
        self.thread_in = Some(root.channel_id.clone());
        // And everything said in it, because the store may hold only the root.
        //
        // A followed thread can live in a channel this window has never
        // opened: `my_threads` brings the root so the row has something to
        // say, and nothing brings the replies. Opening one showed the message
        // it started from and an empty pane under a heading that said how many
        // replies there were. Asked for every time, like the channel's own
        // refresh -- the pane draws from the store now and fills in when the
        // answer lands, which is what this window does everywhere.
        if let Some(link) = self.link.as_ref() {
            link.send(matterless_view::live::Ask::Thread {
                root_id: root_id.to_string(),
                channel_id: root.channel_id.clone(),
            });
        }
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
        // The same reactions on its toolbars as on the channel's. A thread is
        // a second panel of the same messages, and a pane built fresh each
        // time one opens starts with none of what the app already knows.
        stream.favourites = self.stream.favourites.clone();
        println!("thread {root_id}: {} rows", stream.rows.len());
        // The reply being written goes with the thread being left, and
        // whatever was left in this one comes back. It used to be cleared
        // outright -- and this is the path a thread is *reopened* by:
        // `reread_thread` takes the pane away and builds it again through here
        // every time anybody posts in it. So a reader typing a reply lost it
        // the moment somebody else answered, which is exactly when they were
        // most likely to be typing.
        //
        // Put away under the root the reply was begun for rather than under
        // whatever pane is open, because by here that pane has already been
        // taken. Reopening the same thread parks and restores the same draft,
        // which is the no-op it should be.
        let leaving = self.replying_to.take();
        self.park(leaving, self.thread_composer.text());
        self.replying_to = Some(thread_name(root_id));
        self.resume(Which::Thread, &thread_name(root_id));
        self.thread = Some(stream);
        // Writing is what the pane is for, so it opens focused.
        self.input.focus_on(THREAD_COMPOSER);
        // The pane takes width from the channel, so both have to be laid out
        // again before anything is drawn against the old one.
        //
        // Anchored on the line whose replies were asked for. A press is a
        // stronger word about where somebody is looking than the middle of the
        // panel is, but only at the moment it happens -- so this is the one
        // place a row is named from outside, and it is named once.
        let anchored = self.asked_from(root_id);
        // Where the channel is now, to come back to when the pane closes.
        let was = self.anchors().0;
        self.relayout();
        if anchored.is_some() {
            let within = self.stream_rect();
            self.stream.hold(anchored, within);
        }
        self.parked = Some(Parked {
            was,
            left_at: self.stream.scroll,
        });
        if let Some(within) = self.thread_stream_rect()
            && let Some(thread) = self.thread.as_mut()
        {
            thread.to_bottom(within);
        }
    }

    fn close_thread(&mut self) {
        let Some(thread) = self.thread.take() else {
            return;
        };
        let _ = thread;
        // With no thread open there is no thread's channel, and a stale one
        // would send the next reply somewhere nobody is looking.
        self.thread_in = None;
        // Back to where the channel was before the pane took half of it --
        // which is not the same thing as the line that opened it. Closing is
        // nobody pointing at anything: there is no press to honour, so what
        // matters is that the column ends up where it started.
        //
        // Only if the reader has not moved it since. Scrolling the channel
        // while reading the thread makes this stale, and coming back to a
        // place they have left is worse than any of the alternatives -- so it
        // is dropped, and `relayout` holds them where they actually are.
        // Changing channel leaves nothing to find either way.
        let parked = self
            .parked
            .take()
            .filter(|parked| (parked.left_at - self.stream.scroll).abs() < 1.0);
        self.relayout();
        if let Some(parked) = parked {
            let within = self.stream_rect();
            self.stream.anchored(parked.was, within);
        }
    }

    /// Opens one of the side lists, leaving the conversation where it was.
    ///
    /// The pane takes its width from the channel, so the rows have to be laid
    /// out again -- and the anchor has to be taken *before* the column
    /// changes, or it describes a panel the reader is no longer in. That is
    /// the thread pane's rule and the list had none of it: it opened, the
    /// conversation was drawn at a width it had not been measured for, and it
    /// appeared to scroll.
    ///
    /// Nothing is held when the column does not actually change -- a list
    /// opening over a thread takes the pane the thread already had -- because
    /// the parked place belongs to whoever moved the column first.
    fn show_the_list(&mut self, title: &str) {
        let before = self.channel_rect().width;
        let was = self.anchors().0;
        self.listing.expect(title);
        if (self.channel_rect().width - before).abs() < 0.5 {
            return;
        }
        self.relayout();
        self.parked = Some(Parked {
            was,
            left_at: self.stream.scroll,
        });
    }

    /// Shuts the side list and gives the conversation its column back.
    ///
    /// The mirror of `show_the_list`, and the same rule as closing a thread:
    /// the place is put back only if the reader has not moved it since, and
    /// `relayout` holds them where they actually are otherwise.
    fn hide_the_list(&mut self) {
        if !self.listing.open() {
            return;
        }
        let before = self.channel_rect().width;
        self.listing.hide();
        if (self.channel_rect().width - before).abs() < 0.5 {
            return;
        }
        let parked = self
            .parked
            .take()
            .filter(|parked| (parked.left_at - self.stream.scroll).abs() < 1.0);
        self.relayout();
        if let Some(parked) = parked {
            let within = self.stream_rect();
            self.stream.anchored(parked.was, within);
        }
    }

    /// What to hold on to when a thread opens or closes beside the channel.
    ///
    /// The "N replies" line, and not the message it hangs under. That line is
    /// what was pressed to get here, and the message above it can be a dozen
    /// wrapped lines whose height changes in the narrower column -- so holding
    /// the message pinned the top of the message and let the line the reader
    /// had actually clicked slide out from under the pointer, which is the
    /// same complaint as holding the top row, one row further down.
    ///
    /// The message itself when there is no such line: a thread opened from a
    /// message that has no replies yet has no footer to hold.
    fn asked_from(&self, root_id: &str) -> Option<(String, f32)> {
        let within = self.stream_rect();
        self.stream
            .holding_row(&format!("footer/{root_id}"), within)
            .or_else(|| self.stream.holding_row(&format!("post/{root_id}"), within))
    }

    /// How many presses in quick succession this one is, if more than one.
    ///
    /// Counted here because it is a question about a clock, and the input
    /// layer has none. Half a second and four pixels: the interval every
    /// desktop uses, and near enough to the same spot that a reader aiming at
    /// one word has not moved on to another.
    fn repeated(&mut self) -> Option<u32> {
        const APART: std::time::Duration = std::time::Duration::from_millis(500);
        const NEAR: f32 = 4.0;
        let at = self.input.pointer_at()?;
        let now = std::time::Instant::now();
        let again = match self.pressed_before.take() {
            Some((when, was, many))
                if now.duration_since(when) < APART
                    && (was.0 - at.0).abs() <= NEAR
                    && (was.1 - at.1).abs() <= NEAR =>
            {
                many + 1
            }
            _ => 1,
        };
        self.pressed_before = Some((now, at, again));
        (again > 1).then_some(again)
    }

    /// The room the emoji grid is kept inside.
    ///
    /// The whole column rather than the channel's own half of it: the grid can
    /// be opened from a message in the thread pane as readily as from one
    /// beside it, and clamped to the channel it would be pushed left off the
    /// message it belongs to -- or off the pane entirely.
    fn picker_within(&self) -> matterless_ui::Rect {
        self.column_rect()
    }

    /// Puts a half-written message away under the conversation it was for.
    ///
    /// An empty one is forgotten rather than kept: a reader who cleared the
    /// box meant to clear it, and a blank draft coming back is the same as no
    /// draft coming back with extra bookkeeping behind it.
    fn park(&mut self, under: Option<String>, text: String) {
        let Some(under) = under else {
            return;
        };
        // The Threads list and the Drafts list are rows of the sidebar and not
        // conversations: there is no box on either to write in, and text kept
        // under one came back as a draft "in a conversation" nobody could open.
        if under == matterless_view::sidebar::THREADS || under == matterless_view::sidebar::DRAFTS {
            return;
        }
        // To the store as well as to the window. Held only here they were
        // thrown away by every restart -- and this window restarts itself to
        // install an update, so somebody who had written three paragraphs and
        // gone to lunch came back to nothing.
        //
        // Not before the reader is known: the row carries whose draft it is,
        // and one written with nobody's id could never be honoured.
        // With whatever is attached to that conversation. The tray already
        // outlives a channel switch; what it did not outlive was the window,
        // so a message begun with a screenshot came back without it and
        // nothing said it had ever been there.
        let carried = self.attached.get(&under).cloned().unwrap_or_default();
        // Never while the driver is measuring. Its typing act writes into the
        // box and its switching act carries that from channel to channel, and
        // kept, it left "mesure mesure ..." drafts in five of the reader's
        // conversations -- in the store the installed client reads too.
        if !self.me.is_empty()
            && self.driver.is_none()
            && let Some(store) = self.store.as_ref()
            && let Err(error) = store.keep_draft(
                &self.me,
                &under,
                &text,
                &carried,
                matterless_view::clock::now() * 1_000,
            )
        {
            eprintln!("keeping a draft: {error}");
        }
        let had = self.drafts.contains_key(&under);
        match text.trim().is_empty() {
            true => self.drafts.remove(&under),
            false => self.drafts.insert(under.clone(), text),
        };
        // The sidebar's Drafts row counts what the store holds, and is built
        // only when the sidebar is: a draft left by switching channel showed
        // there whenever something else next rebuilt it, seconds later.
        if had != self.drafts.contains_key(&under) {
            self.rebuild_sidebar();
        }
    }

    /// Throws away every kept draft that is nothing but what the driver types.
    ///
    /// A run no longer keeps any; this is for the ones runs kept before that,
    /// and it is exact rather than a guess -- one word, repeated, which is
    /// the driver's and not something a reader writes.
    fn clear_what_the_driver_typed(&mut self) {
        let Some(store) = self.store.as_ref() else {
            return;
        };
        let typed = DRIVER_TYPES.trim();
        let left = match store.drafts(&self.me) {
            Ok(drafts) => drafts,
            Err(error) => return eprintln!("reading the drafts: {error}"),
        };
        let mut cleared = 0;
        for draft in left {
            let words: Vec<&str> = draft.message.split_whitespace().collect();
            if words.is_empty()
                || words.iter().any(|word| *word != typed)
                || !draft.files.is_empty()
            {
                continue;
            }
            match store.keep_draft(
                &self.me,
                &draft.conversation,
                "",
                &[],
                matterless_view::clock::now() * 1_000,
            ) {
                Ok(()) => cleared += 1,
                Err(error) => eprintln!("clearing a draft the driver left: {error}"),
            }
        }
        if cleared > 0 {
            println!("cleared {cleared} drafts the driver had typed");
        }
    }

    /// Brings back everything left half written, from the last time.
    ///
    /// Once the reader is known, because the rows carry whose they are. Never
    /// over what is already in hand: a draft parked this run is the newer of
    /// the two, and the store is only a memory of what happened before.
    fn recall_drafts(&mut self) {
        // The words this reader taught the spell checker, back with the rest of
        // what they left behind.
        if let Some(saved) = self.store.as_ref().and_then(|store| {
            store
                .setting(matterless_view::spell::OWN_WORDS)
                .ok()
                .flatten()
        }) {
            matterless_view::spell::learn_all(saved.lines().map(str::to_string));
        }
        let Some(store) = self.store.as_ref() else {
            return;
        };
        let kept = store.drafts(&self.me).unwrap_or_default();
        let mut brought = 0usize;
        let mut files = 0usize;
        for draft in kept {
            // What was attached comes back the same way and under the same
            // rule: never over what this run already holds.
            if !draft.files.is_empty()
                && let std::collections::hash_map::Entry::Vacant(slot) =
                    self.attached.entry(draft.conversation.clone())
            {
                files += slot.insert(draft.files).len();
            }
            if let std::collections::hash_map::Entry::Vacant(slot) =
                self.drafts.entry(draft.conversation)
                && !draft.message.is_empty()
            {
                slot.insert(draft.message);
                brought += 1;
            }
        }
        if brought > 0 || files > 0 {
            println!("{brought} drafts came back, carrying {files} attachment(s)");
        }
    }

    /// Brings back what was left for this conversation, or empties the box.
    ///
    /// Emptied through `clear` rather than filled with nothing, because
    /// filling marks the box as written in -- and a box nobody has touched is
    /// not the same as one they emptied.
    fn resume(&mut self, box_of: Which, under: &str) {
        let draft = self.drafts.get(under).cloned();
        let fonts = &mut self.fonts;
        let box_of = match box_of {
            Which::Channel => &mut self.composer,
            Which::Thread => &mut self.thread_composer,
            // The editor keeps no draft: it is changing a message that has
            // already been said, and what it opens with is that message.
            // Asking it to resume one would put a draft from somewhere else
            // over what somebody wrote.
            Which::Editing => return,
        };
        match draft {
            Some(draft) => box_of.fill(&draft, fonts),
            None => box_of.clear(fonts),
        }
    }

    /// Copies text, into this window and into every other one.
    fn copy(&mut self, text: String) {
        matterless_view::clip::write(&text);
        self.clipboard = text;
    }

    /// Hands the frame's input to the widgets that want it, with the system's
    /// clipboard joined to this window's own around it.
    ///
    /// Pulled in before anything can paste and pushed back out if anything
    /// copied. Only on a frame where a paste is actually being asked for:
    /// opening the clipboard takes a lock every other application waits on,
    /// and taking it once a frame would be taking it for nothing.
    ///
    /// Around the whole pass rather than inside it, because the pass returns
    /// early in a dozen places -- a panel that covers the window answers the
    /// frame and nothing behind it gets a look.
    fn react(&mut self) {
        // A picture or a file is attached rather than typed, and then the
        // words the reader had copied before must not be poured into the box
        // as well -- so they are put aside for the pass and given back after.
        let mut aside = None;
        if self.input.chord(Key::Char('v')) {
            match matterless_view::clip::held() {
                Some(matterless_view::clip::Held::Words(said)) => self.clipboard = said,
                Some(held) => {
                    self.attach_held(held);
                    aside = Some(std::mem::take(&mut self.clipboard));
                }
                None => {}
            }
        }
        let before = self.clipboard.clone();
        self.reacted();
        if self.clipboard != before {
            matterless_view::clip::write(&self.clipboard);
        }
        if let Some(words) = aside {
            self.clipboard = words;
        }
    }

    /// The strip across the top, and what its buttons do.
    ///
    /// Its own method because two screens carry it: the conversation, and the
    /// sign-in screen -- which is the one that most needs it, being what a
    /// window shows when it cannot reach the server at all.
    fn answered_the_strip(&mut self) {
        if !self.offered.open() {
            return;
        }
        let notice = self.notice_rect();
        self.offered.measure(&mut self.fonts, notice);
        match self.offered.react(&self.input) {
            Some(matterless_view::updater_bar::Chose::Install) => {
                // The waker rather than the socket. Installing never needed a
                // Mattermost session -- it is a fetch from a release host and
                // a check against a key -- and asking for one meant the offer
                // could be shown to a window that could not take it.
                match (self.offer.clone(), self.waker.clone()) {
                    (Some(offer), Some(waker)) => {
                        println!("installing {}", offer.version);
                        matterless_view::live::install_update(offer, Proxy(waker));
                    }
                    _ => self.offered.failed("there is nothing to install"),
                }
                // Measured again: the button says something else now.
                self.offered.measure(&mut self.fonts, notice);
            }
            Some(matterless_view::updater_bar::Chose::Later) => {
                println!("the update was put off");
                self.put_off = self.offer.as_ref().map(|offer| offer.version.clone());
                self.offer = None;
                // The strip has given its row back, so everything under it is
                // taller than it was.
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

    /// Tries the sign-in the reader asked for.
    fn sign_in(&mut self) {
        let Some(waker) = self.waker.clone() else {
            self.signin.failed = Some("this window has no way to hear back".to_string());
            return;
        };
        let server = self.signin.server_url();
        let code = self
            .signin
            .wants_code
            .then(|| self.signin.code.text.trim().to_string());
        self.signin.trying = true;
        self.signin.failed = None;
        println!("signing in to {server}");
        matterless_view::live::sign_in(
            server,
            self.signin.login.text.trim().to_string(),
            self.signin.password.text.clone(),
            code,
            Proxy(waker),
        );
    }

    /// Sends what was pasted to the conversation being written in.
    ///
    /// The box with the keyboard rather than the one under the pointer, which
    /// is where a *drop* goes: a paste belongs to whoever is typing, and they
    /// may well be reading somewhere else while they do it.
    fn attach_held(&mut self, held: matterless_view::clip::Held) {
        let Some(channel_id) = self.sidebar.selected.clone() else {
            eprintln!("no conversation to attach that to");
            return;
        };
        let root_id = match self.input.focus() == Some(THREAD_COMPOSER) {
            true => self.open_root().unwrap_or_default(),
            false => String::new(),
        };
        let paths = match held {
            matterless_view::clip::Held::Files(paths) => paths,
            matterless_view::clip::Held::Picture { bytes, extension } => {
                match Self::spill(&bytes, extension) {
                    Some(path) => vec![path],
                    None => return,
                }
            }
            // Handled by the caller, which is the only one that can put words
            // where the keyboard is.
            matterless_view::clip::Held::Words(_) => return,
        };
        let Some(link) = self.link.as_ref() else {
            return;
        };
        for path in paths {
            // Attached, not sent. A file dropped on the window is the whole
            // message; one pasted into the box waits there with whatever is
            // being written until the reader sends both.
            link.send(matterless_view::live::Ask::Attach {
                channel_id: channel_id.clone(),
                root_id: root_id.clone(),
                path,
            });
        }
    }

    /// Writes pasted bytes somewhere the uploader can read them.
    ///
    /// A picture on the clipboard has no file behind it and the uploader
    /// names a file by its path, so one has to exist. In a directory of its
    /// own so the name can be the plain `image.png` the official client
    /// sends, rather than something with a clock in it -- what the reader
    /// sees on the message is this name.
    ///
    /// Left where it lands. It is the temporary directory, which is the one
    /// place on the machine that is swept without being asked.
    fn spill(bytes: &[u8], extension: &str) -> Option<std::path::PathBuf> {
        let now = matterless_view::clock::now();
        let room = std::env::temp_dir().join(format!("matterless-paste-{now}"));
        if let Err(error) = std::fs::create_dir_all(&room) {
            eprintln!("keeping the pasted picture: {error}");
            return None;
        }
        let path = room.join(format!("image.{extension}"));
        if let Err(error) = std::fs::write(&path, bytes) {
            eprintln!("keeping the pasted picture: {error}");
            return None;
        }
        Some(path)
    }

    /// Opens the editor on a message, and re-shapes the row for it.
    ///
    /// The row gives up its words for the box's height, and `shape` is the
    /// only thing that arranges that -- it runs from `relayout` and nowhere
    /// else. Opening the editor without asking for one left the box floating
    /// at the row's own height, over the name above it and the message
    /// below, which is what was reported twice.
    ///
    /// A pair with `shut_the_editor`, so neither can be done without the
    /// other half being done too.
    fn open_the_editor(&mut self, post_id: &str, said: &str) {
        let mut input = std::mem::take(&mut self.input);
        self.edit.show(post_id, said, &mut self.fonts, &mut input);
        self.input = input;
        self.relayout();
        self.redraw();
    }

    /// Shuts it, and gives the row its words back.
    fn shut_the_editor(&mut self, input: &mut Input) {
        self.edit.hide(input);
        self.relayout();
        self.redraw();
    }

    /// Notes that the reader has arrived somewhere, for the back button.
    ///
    /// Nothing is recorded while stepping. A step is a move *through* the
    /// history, and a history that recorded its own steps could never be
    /// walked backwards: every press would append the place it had just come
    /// from and stand still.
    fn remember_the_place(&mut self, channel: &str) {
        if !self.stepping {
            self.visited.arrived(channel);
        }
    }

    /// Goes back a place, or on to one already visited.
    ///
    /// `on` rather than `forward`: `Action::Forward` already means forwarding
    /// a message to somebody, and two things called forward in one window is
    /// one too many.
    fn stepped(&mut self, on: bool) {
        let wanted = match on {
            true => self.visited.on(),
            false => self.visited.back(),
        };
        let Some(channel) = wanted.map(str::to_string) else {
            return;
        };
        // Through the same `open_channel` a click takes, with a flag rather
        // than an opener of its own: the two would otherwise drift about what
        // opening a conversation involves, and it involves a dozen things.
        self.stepping = true;
        self.sidebar.selected = Some(channel.clone());
        self.open_channel(&channel);
        self.stepping = false;
        println!(
            "stepped {} to {channel}",
            match on {
                true => "on",
                false => "back",
            }
        );
    }

    /// The box a name is being typed into, and the panel it is drawn in.
    ///
    /// `None` when none of them has the keyboard, which is when nothing can
    /// be being typed anywhere.
    ///
    /// The editor counts. It is a composer like the other two -- the same
    /// widget under a third name -- and a reader changing a message to name
    /// somebody wants the same list they would have had writing it. Leaving
    /// it out was not a decision, it was the two boxes being the only two
    /// anybody thought of.
    fn naming_in(&self) -> Option<(Which, Rect, Rect)> {
        let channel = header::below(self.channel_rect());
        match self.input.focus() {
            Some(composer::NAME) => Some((Which::Channel, self.composer.strip(channel), channel)),
            Some(THREAD_COMPOSER) => {
                let body = self.thread_body()?;
                Some((Which::Thread, self.thread_composer.strip(body), body))
            }
            Some(matterless_view::edit::NAME) => {
                let stream = self.stream_rect();
                let row = self.edited_row()?;
                Some((Which::Editing, self.edit.field(row, stream), stream))
            }
            _ => None,
        }
    }

    /// Offers whatever the half-typed name at the caret could mean.
    ///
    /// Answered from the store alone, which is what makes it worth having:
    /// a list that arrives after a round trip arrives after the next letter
    /// has been typed, and a reader who has to wait for it will have finished
    /// the name by hand.
    fn offer_names(&mut self) {
        let Some((which, _, _)) = self.naming_in() else {
            self.naming.hide();
            return;
        };
        let box_of = match which {
            Which::Channel => &self.composer,
            Which::Thread => &self.thread_composer,
            Which::Editing => &self.edit.box_of,
        };
        let Some((sigil, said)) = box_of.being_named(&matterless_view::offer::SIGILS) else {
            self.naming.hide();
            return;
        };
        let Some(store) = self.store.as_ref() else {
            return;
        };
        let found: Vec<matterless_view::offer::Suggestion> = match sigil {
            matterless_view::offer::CHANNELS => store
                .channels_matching(&said, 6)
                .unwrap_or_default()
                .into_iter()
                // A direct message has no name anybody can type, so it is not
                // something a `~` could ever mean.
                .filter(|channel| channel.channel_type == "O" || channel.channel_type == "P")
                .map(|channel| matterless_view::offer::Suggestion {
                    insert: channel.name,
                    label: channel.display_name,
                    face: None,
                })
                .collect(),
            _ => store
                .users_matching(&said, 6)
                .unwrap_or_default()
                .into_iter()
                .map(|user| {
                    // Read by their real name where there is one, and
                    // mentioned by the name the server knows: a mention of
                    // "Amy Jones" reaches nobody.
                    let real = format!("{} {}", user.first_name, user.last_name);
                    let real = real.trim().to_string();
                    matterless_view::offer::Suggestion {
                        label: match real.is_empty() {
                            true => user.username.clone(),
                            false => format!("{real}  {}", user.username),
                        },
                        face: Some(matterless_view::switcher::Face {
                            user_id: user.id,
                            avatar_at: user.last_picture_update,
                        }),
                        insert: user.username,
                    }
                })
                .collect(),
        };
        self.naming.show(sigil, &said, found);
    }

    /// Answers the offer, and takes the keys it used off the box.
    ///
    /// Before the boxes see the keyboard. Return here means "this name", and
    /// left to the box it would mean "send the message" -- which is the same
    /// trap Up in an empty box was, and the reason `Input::took` exists.
    fn answered_the_offer(&mut self) -> bool {
        if !self.naming.open() {
            return false;
        }
        let mut input = std::mem::take(&mut self.input);
        let chose = self.naming.react(&input);
        // Every key it claims, whether or not it acted on one: an arrow that
        // walked the list must not also walk the caret.
        for key in matterless_view::offer::Offer::CLAIMS {
            input.took(key);
        }
        self.input = input;
        match chose {
            Some(matterless_view::offer::Chose::Name(one)) => {
                let Some((which, _, _)) = self.naming_in() else {
                    return false;
                };
                let (sigil, said) = (self.naming.sigil, self.naming.said.clone());
                // Split off so the box and the fonts are two borrows of two
                // fields rather than one of the whole window.
                let fonts = &mut self.fonts;
                let box_of = match which {
                    Which::Channel => &mut self.composer,
                    Which::Thread => &mut self.thread_composer,
                    Which::Editing => &mut self.edit.box_of,
                };
                box_of.name_it(fonts, sigil, &said, &one.insert);
                self.naming.hide();
                self.relayout();
                self.redraw();
                true
            }
            Some(matterless_view::offer::Chose::Nothing) => {
                self.naming.hide();
                self.redraw();
                true
            }
            None => false,
        }
    }

    /// Goes to the next conversation with something waiting in it.
    ///
    /// The sidebar decides which -- it is the one that holds the list in the
    /// reader's own order and knows what is muted. This opens it and scrolls
    /// the list so the row can be seen, because arriving somewhere without
    /// the list following reads as the window having jumped.
    fn walk_to_unread(&mut self, on: bool) {
        let Some(channel) = self.sidebar.next_unread(on).map(str::to_string) else {
            println!("nothing unread to walk to");
            return;
        };
        self.sidebar.selected = Some(channel.clone());
        self.open_channel(&channel);
        let within = self.sidebar_rect();
        self.sidebar.scroll_to(&channel, within);
        println!("walked to {channel}");
    }

    /// Up in an empty message box opens the last thing this reader said.
    ///
    /// Answered here rather than in the box, and the key is taken so the box
    /// never sees it. `Composer::react` reads `Key::Up` as a caret move and
    /// cannot tell whether it should be: it can reach neither the store nor
    /// the conversation, and does not know who the reader is. The window is
    /// the one that can see all three.
    ///
    /// Only Up on its own. Shift+Up selects and Ctrl+Up walks by word, and a
    /// box being selected in is not an idle one.
    ///
    /// Only an empty box, and empty includes what is waiting to be sent: a
    /// box with a pasted picture in it and nothing typed already sends on
    /// Enter, so it is not idle either.
    fn edit_the_last_thing_said(&mut self) -> bool {
        if !self.input.mods().bare() || !self.input.struck(Key::Up) {
            return false;
        }
        // Whichever box has the keyboard, and nothing if neither has.
        let root_id = match self.input.focus() {
            Some(composer::NAME) => String::new(),
            Some(THREAD_COMPOSER) => self.open_root().unwrap_or_default(),
            _ => return false,
        };
        let box_of = match root_id.is_empty() {
            true => &self.composer,
            false => &self.thread_composer,
        };
        if !box_of.text().trim().is_empty() || self.carrying(&root_id) {
            return false;
        }
        let Some(post_id) = self.last_thing_i_said(&root_id) else {
            return false;
        };
        // Taken only now that there is something to do with it: an empty box
        // in a conversation this reader has never written in leaves Up alone
        // rather than swallowing it to no effect.
        self.input.took(Key::Up);
        self.act(matterless_view::actions::Action::Edit, post_id, false);
        true
    }

    /// The newest message in this conversation that this reader can edit.
    ///
    /// From the rows rather than the store: they are the conversation as it
    /// is on screen, already in order and already without the deleted ones,
    /// and the newest message is always among them -- a channel opens at its
    /// end.
    ///
    /// The thread's own rows when a reply is being written, or the channel's.
    /// Otherwise replying in a thread would open the last thing said in the
    /// channel behind it, which is a different message and quite possibly a
    /// different day.
    ///
    /// Only one the store has a copy of, which is the part worth the read. A
    /// message still on its way is a row like any other -- `as_post` gives it
    /// this reader's own `author_id` and its `pending_post_id` for an id --
    /// and there is nothing to edit behind that id. Unchecked, sending a
    /// message and pressing Up, which is exactly when somebody wants this,
    /// would take the key and answer with a line in the log. So a post the
    /// server has not confirmed is passed over for the one before it.
    fn last_thing_i_said(&self, root_id: &str) -> Option<String> {
        let rows = match root_id.is_empty() {
            true => &self.stream.rows,
            false => &self.thread.as_ref()?.rows,
        };
        let store = self.store.as_ref()?;
        rows.iter()
            .rev()
            .filter_map(|row| match row {
                matterless_render::Row::Post { post }
                | matterless_render::Row::Continuation { post } => Some(post),
                _ => None,
            })
            .filter(|post| post.author_id == self.me)
            .find(|post| {
                store
                    .post(&post.post_id)
                    .map(|held| held.is_some())
                    .unwrap_or(false)
            })
            .map(|post| post.post_id.clone())
    }

    /// Moves the pane's edge while its grip is held.
    ///
    /// Answered before anything else that reads the pointer: every rect in
    /// the column is measured from this width, so a frame that reacted first
    /// and resized second would answer the press against the layout the
    /// reader has just finished moving.
    ///
    /// `pressed` rather than `hovered`, which is what makes it a drag: the
    /// name sticks to the box the button went down on, so the pointer can
    /// leave the six pixels it started in -- and it will, immediately.
    fn dragged_the_pane(&mut self) -> bool {
        if self.input.pressed() != Some(GRIP) {
            return false;
        }
        let Some((x, _)) = self.input.pointer_at() else {
            return false;
        };
        let column = self.column_rect();
        // From the right edge of the column, because that is the edge the
        // pane is pinned to. `aside::rect` does the clamping, so the width
        // kept here is what the reader asked for rather than what fitted --
        // widen the window again and they get the pane they dragged.
        let wanted = (column.right() - x).max(0.0);
        if (wanted - self.pane_width).abs() < 0.5 {
            return false;
        }
        self.pane_width = wanted;
        self.relayout();
        self.redraw();
        true
    }

    /// Hands the frame's input to the widgets that want it.
    fn reacted(&mut self) {
        // Before everything, and alone: with no session this is the whole
        // window, and nothing behind it is drawn for a press to reach. Except
        // the strip that offers a newer build, which belongs to every screen
        // and to this one most of all.
        if self.signing_in() {
            self.answered_the_strip();
            let window = self.below_notice();
            self.signin.measure(&mut self.fonts, window);
            if self
                .signin
                .react(&mut self.fonts, &mut self.input, &mut self.clipboard)
                .is_some()
            {
                self.sign_in();
            }
            return;
        }
        // The pane's edge, before anything else reads the pointer: every
        // rect in the column is measured from its width, so reacting first
        // would answer this frame against the layout the drag has just
        // finished moving.
        if self.dragged_the_pane() {
            return;
        }
        // Before the boxes see the keyboard, because it takes the key off
        // them: Up in an empty box is the window's to answer.
        if self.edit_the_last_thing_said() {
            return;
        }
        // And before them for the same reason: while a list of names is up,
        // Return means "this one" rather than "send this".
        if self.answered_the_offer() {
            return;
        }
        // Where the offer sits, before anything asks what is under the
        // pointer: it is measured rather than computed per frame because
        // measuring needs the fonts and a hit test does not have them.
        // Over everything and alone, for the same reason the release notes
        // are: it covers the window, so nothing behind it may take the press.
        if self.settings.open() {
            let window = self.window_rect();
            self.settings.measure(&mut self.fonts, window);
            let mut input = std::mem::take(&mut self.input);
            let did = self.settings.react(&mut input);
            self.input = input;
            // Only once the panel has actually answered something, which is
            // the whole of why it could not be pressed: clearing the frame's
            // input unconditionally threw away the box the button went *down*
            // on, so when the button came up there was nothing for the release
            // to agree with -- and a click is a press and a release agreeing.
            // Every press in here died on the frame it was made.
            if let Some(did) = did {
                match did {
                    matterless_view::settings::Did::Text(mode) => self.draw_text_as(mode),
                    matterless_view::settings::Did::Kept(kept) => {
                        if let Some(held) = matterless_view::filecache::looked() {
                            held.set_budget(kept.bytes());
                        }
                        if let Some(store) = self.store.as_ref()
                            && let Err(error) = store.remember_setting(
                                matterless_view::settings::Kept::SETTING,
                                &kept.stored(),
                            )
                        {
                            eprintln!("keeping the picture budget: {error}");
                        }
                    }
                    matterless_view::settings::Did::Look => self.look_for_a_build(),
                    matterless_view::settings::Did::Close => {}
                }
                self.input = Input::default();
                // Answering changes what the card says and therefore how tall
                // it is, and the panel drops its placement when that happens.
                // Nothing else measures it -- a draw draws what was measured
                // -- so without this the card is not there at all until the
                // next press, which reads as the panel blinking.
                self.settings.measure(&mut self.fonts, window);
            }
            return;
        }
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
        self.answered_the_strip();
        // The viewer first of all and alone: it covers the window, so nothing
        // behind it may take the same press or the same key.
        if self.viewer.open() {
            let mut input = std::mem::take(&mut self.input);
            let did = self.viewer.react(&input);
            match did {
                Some(matterless_view::viewer::Did::Close) => {
                    self.viewer.hide();
                    self.looked_moving = Default::default();
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
                self.shut_the_editor(&mut input);
                self.input = input;
                return;
            }
            let within = self.stream_rect();
            let row = self
                .edited_row()
                .unwrap_or_else(|| matterless_ui::Rect::new(within.x, within.y, within.width, 0.0));
            // The buttons in the box, before the box reads the frame: Cancel
            // is a way out and must not also put a caret somewhere.
            match self.edit.pressed(&input) {
                Some(composer::Button::Cancel) => {
                    self.shut_the_editor(&mut input);
                    self.input = input;
                    return;
                }
                // The same errand Enter runs, so there is one way of saving
                // rather than a button that does nearly what the key does.
                Some(composer::Button::Save) => {
                    let message = self.edit.box_of.text();
                    let post_id = self.edit.for_post.clone().unwrap_or_default();
                    self.shut_the_editor(&mut input);
                    self.input = input;
                    if let Some(link) = self.link.as_ref() {
                        link.send(matterless_view::live::Ask::Edit { post_id, message });
                    }
                    return;
                }
                _ => {}
            }
            let was = self.edit.height();
            let saved = self
                .edit
                .react(&mut self.fonts, &input, row, within, &mut self.clipboard);
            // A message that wraps to another line is a row that has to make
            // more room for it. Nothing else here re-shapes, so a box left to
            // grow on its own grows over its neighbours.
            if (self.edit.height() - was).abs() > 0.5 {
                self.input = std::mem::take(&mut input);
                self.relayout();
                self.redraw();
                return;
            }
            if let Some(message) = saved {
                let post_id = self.edit.for_post.clone().unwrap_or_default();
                self.shut_the_editor(&mut input);
                self.input = input;
                if let Some(link) = self.link.as_ref() {
                    link.send(matterless_view::live::Ask::Edit { post_id, message });
                }
                return;
            }
            self.input = input;
            return;
        }

        // The one thing the card offers: a conversation with whoever it is
        // about, opened the way the switcher opens one -- the server makes or
        // finds the direct channel, and the window lands in it when it does.
        if self.input.clicked() == Some(matterless_view::profile::MESSAGE)
            && let (Some(card), Some(link)) = (self.profile.of.as_ref(), self.link.as_ref())
        {
            link.send(matterless_view::live::Ask::Direct {
                user_id: card.user_id.clone(),
            });
        }
        // Dismissed by Escape, by its cross, by its button once that has done
        // its work, or by a click anywhere that is not on it -- which is what a
        // reader expects of a popover and means it never has to be closed
        // deliberately.
        let dismissed = self.input.struck(Key::Escape)
            || self
                .input
                .clicked()
                .is_some_and(|name| name != matterless_view::profile::NAME);
        // `opening` first and always, so the flag is cleared whether or not
        // anything was dismissed this frame -- left standing it would swallow
        // the next click instead of this one.
        if self.profile.open() && !self.profile.opening() && dismissed {
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
                // Beside the list, not instead of it. Choosing a thread used
                // to open its channel in the column and the thread next to it,
                // which threw the list away to show a conversation nobody
                // asked for -- and made the list a place you pass through once
                // rather than one you work down.
                let root = found.root_id.clone();
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
                input.focus_on(composer::NAME);
                self.input = input;
                self.hide_the_list();
                return;
            }
            let pane = matterless_view::aside::rect(self.column_rect(), self.pane_width);
            let boxes = self.placed.clone();
            let did = self.listing.react(&input, &boxes, pane);
            if let Some(matterless_view::listing::Did::Clear(found)) = did {
                self.input = input;
                self.clear_draft(&found);
                self.redraw();
                return;
            }
            if matches!(did, Some(matterless_view::listing::Did::Close)) {
                input.focus_on(composer::NAME);
                self.input = input;
                self.hide_the_list();
                return;
            }
            if let Some(matterless_view::listing::Did::Open(found)) = did {
                input.focus_on(composer::NAME);
                self.input = input;
                self.hide_the_list();
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
            // Escape, or a press anywhere that is not on it: a popover opened
            // from a message has no close button and should not need one.
            //
            // `opening` first and always, so the flag is cleared whether or
            // not anything dismissed it this frame. The press that opens the
            // picker is still in this frame's input, so left standing it would
            // be read as a press outside and shut it again -- which is the
            // whole of why the profile card could not be opened by clicking.
            let opening = self.picker.opening();
            let elsewhere = input
                .clicked()
                .is_some_and(|name| !matterless_view::picker::owns(name));
            if input.struck(Key::Escape) || (elsewhere && !opening) {
                self.picker.hide(&mut input);
                self.input = input;
                return;
            }
            let within = self.picker_within();
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
        if self.search.busy() {
            let mut input = std::mem::take(&mut self.input);
            if input.struck(Key::Escape) {
                self.search.hide(&mut input);
                self.input = input;
                return;
            }
            // A press anywhere but in the box or its list puts the list away.
            // The reader is doing something else, and a panel hanging over the
            // conversation they went back to is in the way.
            if self.search.asking()
                && let Some(pressed) = input.pressed()
                && !pressed.starts_with(matterless_view::search::NAME)
                && pressed != "header/find"
            {
                self.search.hide(&mut input);
                self.input = input;
                return;
            }
            let pane = matterless_view::aside::rect(self.column_rect(), self.pane_width);
            let boxes = self.placed.clone();
            let store = self.store.clone();
            self.search.scrolled(&input, &boxes, pane);
            let field = self.search_field(pane);
            let here = self.sidebar.selected.clone();
            let did = store.as_ref().and_then(|store| {
                self.search.react(
                    &mut self.fonts,
                    &input,
                    matterless_view::search::Shown { pane, field },
                    &mut self.clipboard,
                    matterless_view::search::Against {
                        store,
                        me: &self.me,
                        here: here.as_deref().unwrap_or_default(),
                    },
                )
            });
            if matches!(did, Some(matterless_view::search::Did::Close)) {
                self.search.hide(&mut input);
                self.input = input;
                return;
            }
            // Return, with nothing picked out of the list: the rest of them.
            if matches!(did, Some(matterless_view::search::Did::Widen)) {
                self.search.widen(&mut input);
                self.input = input;
                self.react();
                self.redraw();
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
        // Not when there is nowhere to write. The box holds focus from the
        // first frame, so that a chat window takes typing without a click
        // first; with no box drawn, that focus would swallow every keystroke
        // into somewhere the reader cannot see.
        let sent = match self.can_write() {
            false => None,
            true => {
                // Before it reacts, so a press on the bar does not also land
                // in the words behind it.
                self.composer.dragged(&self.input, within);
                let sent =
                    self.composer
                        .react(&mut self.fonts, &self.input, within, &mut self.clipboard);
                let width = self.channel_rect().width;
                self.composer.lay_out(&mut self.fonts, width);
                sent
            }
        };

        let replied = if let Some(body) = self.thread_body() {
            self.thread_composer.dragged(&self.input, body);
            let replied =
                self.thread_composer
                    .react(&mut self.fonts, &self.input, body, &mut self.clipboard);
            self.thread_composer.lay_out(&mut self.fonts, body.width);
            replied
        } else {
            None
        };

        // After the boxes, because what is being typed is what they have
        // just been told: the caret has moved by now and the run under it is
        // what the list is a list of.
        self.offer_names();
        // The two buttons in each box. Send does what return does, and attach
        // does what a drop does -- both exist for a reader who has not been
        // told about either.
        if self.can_write()
            && let Some(button) = self.composer.pressed(&self.input)
        {
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
            self.box_changed_size();
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
            // A reply goes to the channel the thread is in, which is not
            // always the one being looked at: from the Threads list it is a
            // conversation the column is not showing.
            let channel = self.thread_in.clone().unwrap_or_default();
            self.post_message(&channel, &root, text);
        }
    }

    /// Brings the unread mark into view.
    ///
    /// A little below the top edge rather than against it, so the tail of what
    /// was already read stays visible: a divider hard against the top reads as
    /// the beginning of the channel rather than as a line drawn through it.
    ///
    /// Nothing new to show is not nothing to do. With no mark in the plan the
    /// place to be is the end of the conversation -- that is where the reader
    /// has got to, and where the next message will arrive -- so the shortcut
    /// always moves rather than sometimes doing nothing at all, which reads as
    /// a key that does not work.
    ///
    /// This channel only. Walking on to the next unread *conversation* is a
    /// different gesture, about the sidebar rather than about the page, and
    /// folding it in here would mean a key that sometimes scrolls and
    /// sometimes takes you somewhere else.
    fn show_unread_mark(&mut self) {
        let within = self.stream_rect();
        // Three lines or so, and never more than a quarter of a short panel.
        let above = ABOVE.min(within.height / 4.0);
        if !self
            .stream
            .to_row(matterless_view::stream::DIVIDER, above, within)
        {
            self.stream.to_bottom(within);
        }
        self.react();
        self.redraw();
    }

    /// Shows what is half written, as a list beside the conversation.
    ///
    /// The same pane Saved and Pinned use. A draft has no message to go to,
    /// and it does not need one: `Did::Open` takes the reader to the
    /// conversation and the thread and never looks at the post id, so an
    /// empty one is the right answer rather than a missing one.
    fn open_drafts(&mut self) {
        let Some(store) = self.store.clone() else {
            return;
        };
        self.show_the_list("Drafts");
        // The one list whose rows can be thrown away from it.
        self.listing.clearable = true;
        let named = |conversation: &str| -> (String, String) {
            // A reply's conversation is the thread's own name, which carries
            // the root it hangs from.
            match conversation.strip_prefix(THREAD_PREFIX) {
                Some(root) => (String::new(), root.to_string()),
                None => (conversation.to_string(), String::new()),
            }
        };
        let found: Vec<matterless_view::listing::Found> = store
            .drafts(&self.me)
            .unwrap_or_default()
            .into_iter()
            .map(|draft| {
                let (channel_id, root_id) = named(&draft.conversation);
                // A reply names the conversation its root is in, which needs
                // the post: without it the row would say nothing about where
                // the draft belongs.
                let channel_id = match channel_id.is_empty() {
                    false => channel_id,
                    true => store
                        .post(&root_id)
                        .ok()
                        .flatten()
                        .map(|root| root.channel_id)
                        .unwrap_or_default(),
                };
                let channel = store
                    .channel(&channel_id)
                    .ok()
                    .flatten()
                    .map(|channel| match channel.display_name.is_empty() {
                        true => channel.name,
                        false => channel.display_name,
                    })
                    .unwrap_or_else(|| "a conversation".to_string());
                matterless_view::listing::Found {
                    post_id: String::new(),
                    channel_id,
                    channel,
                    author: String::new(),
                    // A draft that is only a picture still has to say what it
                    // is: with nothing typed, the row would otherwise be a
                    // channel name and a blank line.
                    preview: match (draft.message.trim().is_empty(), draft.files.first()) {
                        (true, Some(file)) => match draft.files.len() {
                            1 => file.name.clone(),
                            more => format!("{} and {} more", file.name, more - 1),
                        },
                        _ => matterless_sync::notify::preview_of(&draft.message),
                    },
                    note: match root_id.is_empty() {
                        true => String::new(),
                        false => "a reply".to_string(),
                    },
                    root_id,
                }
            })
            .collect();
        println!("Drafts: {} unfinished", found.len());
        self.listing.fill(found);
    }

    /// Throws away a draft from the Drafts list, and whatever was attached to it.
    ///
    /// Out of the box as well, when the box is showing it: a draft cleared from
    /// the list and still sitting in the message box is not cleared. The list
    /// is read again, so the row goes.
    fn clear_draft(&mut self, found: &matterless_view::listing::Found) {
        let conversation = match found.root_id.is_empty() {
            true => found.channel_id.clone(),
            false => format!("{THREAD_PREFIX}{}", found.root_id),
        };
        // Its attachments first: `park` keeps a draft that still has any.
        self.attached.remove(&conversation);
        self.park(Some(conversation.clone()), String::new());
        let open = self.sidebar.selected.as_deref() == Some(conversation.as_str());
        let open_thread = self
            .open_root()
            .is_some_and(|root| format!("{THREAD_PREFIX}{root}") == conversation);
        if open {
            self.composer.clear(&mut self.fonts);
        }
        if open_thread {
            self.thread_composer.clear(&mut self.fonts);
        }
        println!("cleared a draft");
        self.open_drafts();
        self.rebuild_sidebar();
        self.relayout();
    }

    /// Reads a channel and lays it out, then shows its newest message.
    fn open_channel(&mut self, channel: &str) {
        let Some(store) = self.store.clone() else {
            return;
        };
        let _open = matterless_view::timing::watch("opening a channel", 0, "");
        // The drafts row opens a pane beside the conversation and leaves the
        // reader in the one they are reading, so it is answered before
        // anything that records where they are: it is not a place, and
        // writing it down as one overwrote `left_off` -- which is what the
        // next start reopens -- and put it in the back-and-forward history.
        if channel == matterless_view::sidebar::DRAFTS {
            self.open_drafts();
            return;
        }
        // Written on the way in rather than on the way out, so a window that
        // is killed still knows where somebody was. Nothing depends on it this
        // run: it is read once, at the next start.
        //
        // Not before the session is known. The row carries who it belongs to
        // and is checked against their membership when it is read back, so one
        // written with nobody's id would be a row that can never be honoured
        // -- and it would have written over the one from last time.
        if !self.me.is_empty()
            && channel != matterless_view::sidebar::THREADS
            && let Err(error) = store.leave_off(
                &self.me,
                channel,
                // Milliseconds, because every other time in this store is:
                // `clock::now` answers seconds, and a column that quietly
                // disagreed with the rest would read as 1970 to whoever
                // looked at it next.
                matterless_view::clock::now() * 1_000,
            )
        {
            eprintln!("remembering where we are: {error}");
        }
        self.remember_the_place(channel);
        self.recall_favourites();
        // A place in the conversation being left behind. The scroll check
        // would drop it anyway, and a row of one channel is not a row of
        // another, but neither of those is a reason to carry it across.
        self.parked = None;
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
        // How tall this channel's rows were drawn, before its rows are
        // replaced. Waiting for the background shaping to finish writes
        // nothing at all for a reader who keeps moving -- measured: a run
        // that switched channels every forty frames never once got there.
        self.remember_heights();
        // What was being written here goes with the channel being left, and
        // whatever was left in the one being opened comes back.
        let leaving = self.writing_to.take();
        self.park(leaving, self.composer.text());
        self.writing_to = Some(channel.to_string());
        self.resume(Which::Channel, channel);
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
                // How long this conversation is, before a single row above
                // the panel has been measured. Without it the channel opens
                // knowing the height of its last dozen messages and finds out
                // about the rest over the next fifth of a second -- which is
                // the scrollbar appearing late, measured at 30 to 220ms on
                // 125 of 466 openings.
                self.foresee_heights();
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

    /// The pointer, moved to where it now is.
    ///
    /// Its own method for the same reason the wheel is: the driver has to put
    /// the pointer somewhere before it turns one, and a wheel is answered by
    /// whichever panel the pointer is over.
    fn point_at(&mut self, x: f32, y: f32) {
        self.placed = self.targets();
        let boxes = self.placed.clone();
        self.input.apply(UiEvent::PointerMoved { x, y }, &boxes);
        // A held press is a drag, which selects text in the composer.
        if self.input.pressed().is_some() {
            self.react();
        }
        self.watch_pointer();
        self.redraw();
    }

    /// Takes a turn of the wheel, to be given over the next few frames.
    ///
    /// Its own method because the driver turns the wheel too, and a
    /// measurement of scrolling that went down a path of its own would be a
    /// measurement of the path of its own.
    fn wheel_by(&mut self, by: f32) {
        self.glide.push(by, std::time::Instant::now());
        // The frame that draws is the one that moves it, so there has to be
        // one: this window draws when something happens, and what is
        // happening now is happening over time rather than at an event.
        self.redraw();
    }

    /// Hands over however much of the turn belongs to this frame.
    ///
    /// Answers whether there is more to come, so the caller knows to ask for
    /// another frame. A glide that stopped being asked would leave the rest of
    /// the distance undelivered and the conversation short of where the wheel
    /// was turned to.
    fn glided(&mut self) -> bool {
        if !self.glide.travelling() {
            return false;
        }
        let step = self.glide.taken(std::time::Instant::now());
        if step != 0.0 {
            self.scrolled_by(step);
        }
        self.glide.travelling()
    }

    /// A turn of the wheel, wherever the pointer is.
    fn scrolled_by(&mut self, by: f32) {
        let boxes = self.targets();
        self.input.apply(UiEvent::Wheel { x: 0.0, y: by }, &boxes);
        // One wheel, five panels, and the pointer decides which of them
        // it belongs to -- which is what `wheel_over` is for. Each panel
        // asks about itself, so a fixed strip simply takes the turn and
        // does nothing with it.
        // A floating panel over the conversation takes the turn while it is
        // up, and takes it alone: the picker's grid scrolls and the
        // conversation behind it stays where the reader left it. `wheel_over`
        // asks whether a matching box is under the pointer rather than whether
        // it is the topmost one, so the stream would otherwise scroll too --
        // under a panel the reader is looking at.
        if self.picker.open() {
            self.react();
            self.redraw();
            return;
        }
        let sidebar = self.sidebar_rect();
        self.sidebar.react(&self.input, &boxes, sidebar);
        // The list on the right, and the one filling the column, each
        // take the turn when the pointer is over them -- the same way
        // every other panel does.
        if self.aside_rect().is_some() || self.on_threads() {
            self.react();
        }
        // The message boxes are panels too, once one holds more than it
        // shows. Answered here rather than in `react`, which a wheel turn
        // does not reach unless a picker or a side pane happens to be open --
        // so the box took the turn in two windows out of three and ignored it
        // in the ordinary one.
        let writing = header::below(self.channel_rect());
        self.composer.wheeled(&self.input, &boxes, writing);
        if let Some(body) = self.thread_body() {
            self.thread_composer.wheeled(&self.input, &boxes, body);
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

    /// Does what the act being measured says, once.
    ///
    /// Everything here goes through what an event would call rather than
    /// reaching into a panel: a driver with a shortcut measures the shortcut.
    fn drive(&mut self, act: Act, tick: u32, fresh: bool) -> u32 {
        if fresh {
            // Where the reader was when the run began, so every act reads the
            // same conversation.
            match self.home.clone() {
                Some(home) => self.open_channel(&home),
                None => self.home = self.sidebar.selected.clone(),
            }
        }
        let mut did = 0;
        match act {
            // Nothing, on purpose: the floor every other act is read against.
            Act::Still => did += 1,
            Act::Scrolling => {
                // Over the conversation first. A wheel is answered by the
                // panel the pointer is over, and a driver that never moved one
                // turned the wheel at nothing: the first run of this act
                // reported the cost of a still frame and called it scrolling.
                let stream = self.stream_rect();
                self.point_at(
                    stream.x + stream.width / 2.0,
                    stream.y + stream.height / 2.0,
                );
                let line = self.stream.theme.line_height;
                // Thirty frames down, thirty back up, so the same rows leave
                // the plan and return rather than the list running off the end
                // and sitting there.
                let before = self.stream.scroll;
                self.wheel_by(match (tick / 30) % 2 {
                    0 => -line * 3.0,
                    _ => line * 3.0,
                });
                // Only when the list actually moved. At the end of a
                // conversation the wheel is refused, which is right and is
                // not scrolling.
                if self.stream.scroll != before {
                    did += 1;
                }
            }
            Act::Switching => {
                // Not every frame: a channel that is switched away from before
                // its first frame is drawn is never drawn at all.
                if tick.is_multiple_of(40)
                    && let Some(next) = self.another_channel()
                {
                    // Selected as a click selects it: `another_channel` walks
                    // on from the selection, and without this it found the
                    // same next channel every time -- one channel reopened
                    // twenty-five times, never one that had not been opened.
                    self.sidebar.selected = Some(next.clone());
                    self.open_channel(&next);
                    did += 1;
                }
            }
            Act::Typing => {
                // The composer only hears what is aimed at it, and nothing has
                // clicked it.
                self.input.focus_on(composer::NAME);
                let before = self.composer.text().len();
                match tick.is_multiple_of(90) {
                    true => self.composer.clear(&mut self.fonts),
                    false => {
                        self.input
                            .apply(UiEvent::Typed(DRIVER_TYPES.to_string()), &[]);
                        self.react();
                    }
                }
                if self.composer.text().len() != before {
                    did += 1;
                }
                self.redraw();
            }
            Act::Threading => {
                // Open, and eight frames later the same call closes it again,
                // which is what pressing the footer twice does. Eight rather
                // than forty because a run is commonly shorter than forty, and
                // a period longer than the run measured the opening only --
                // which is how closing came to be the one path here that
                // nothing ever drove.
                if tick.is_multiple_of(8)
                    && let Some(root) = self.a_thread()
                {
                    self.open_thread(&root);
                    did += 1;
                }
            }
            Act::Resizing => {
                // A width that moves every frame, the way a hand on the edge
                // moves it. Back and forth so it never runs off the screen.
                let swing = (tick % 120) as i32;
                let by = match swing < 60 {
                    true => swing,
                    false => 120 - swing,
                };
                let wide = 900 + by * 4;
                if let Some(window) = self.window.as_ref() {
                    let asked = window.request_inner_size(winit::dpi::PhysicalSize::new(
                        wide as u32,
                        self.size.1,
                    ));
                    // Some systems answer straight away rather than through an
                    // event; either way the window has been asked to change.
                    let _ = asked;
                    did += 1;
                }
            }
            Act::Heightening => {
                // The bottom edge, up and back down. Held at the newest
                // message throughout: a shorter panel reaches further, so a
                // reader who was against the bottom edge is the case this is
                // here to keep honest.
                let swing = (tick % 120) as i32;
                let by = match swing < 60 {
                    true => swing,
                    false => 120 - swing,
                };
                let tall = 500 + by * 4;
                if let Some(window) = self.window.as_ref() {
                    let asked = window.request_inner_size(winit::dpi::PhysicalSize::new(
                        self.size.0,
                        tall as u32,
                    ));
                    let _ = asked;
                    did += 1;
                }
            }
        }
        did
    }

    /// A channel other than the open one, walking the sidebar and coming back
    /// round. `None` when the reader is in one channel and no others.
    fn another_channel(&self) -> Option<String> {
        let every: Vec<&String> = self
            .sidebar
            .entries
            .iter()
            .filter_map(|entry| match entry {
                matterless_view::sidebar::Entry::Channel { id, .. } => Some(id),
                _ => None,
            })
            .collect();
        let open = self.sidebar.selected.as_deref().unwrap_or_default();
        let at = every.iter().position(|id| id.as_str() == open);
        let next = match at {
            Some(at) => every.get(at + 1).or(every.first()),
            None => every.first(),
        };
        next.map(|id| (*id).clone())
    }

    /// A thread in the open channel, if the reader can see one.
    fn a_thread(&self) -> Option<String> {
        self.stream.rows.iter().find_map(|row| match row {
            matterless_render::Row::ThreadFooter { root_id, .. } => Some(root_id.clone()),
            _ => None,
        })
    }

    /// Builds the surface for whatever size has arrived, if one has.
    ///
    /// Called from the frame rather than from the wait. Dragging a window's
    /// border on this system runs a message loop of the operating system's
    /// own, and `about_to_wait` is not reached again until the border is let
    /// go -- so a window that resized itself there simply stopped resizing
    /// while it was being resized.
    ///
    /// Once per frame and every frame. A drag reports sizes faster than this
    /// can draw them, so the ones it passed through on the way are not worth
    /// building for -- but the one it is in when a frame is about to be drawn
    /// always is. `self.size` moves here and nowhere else, beside the surface
    /// it has to agree with.
    ///
    /// There was a throttle here, and it is what left the contents trailing
    /// the window's own frame: sized for a rebuild that cost a tenth of a
    /// second, it answered three of every forty sizes a drag reported. On
    /// Direct3D the rebuild is 1.00ms and the reshaping 1.37ms, against a
    /// frame of 11.89ms that is mostly waiting for the screen. There is
    /// nothing left to spread out.
    fn take_the_size(&mut self) {
        let Some(size) = self.sized.take() else {
            return;
        };
        let _resizing = matterless_view::timing::watch("resizing the window", 0, "");
        // Where the reader is, measured against the panel they are still
        // looking at. This has to happen before the size moves: an anchor read
        // against the new panel and put back against that same panel is an
        // identity when nothing re-wraps, and nothing re-wraps when only the
        // height changed -- which is the whole of why a vertical drag held the
        // rows against the top edge however the anchor itself was written.
        let held = self.anchors();
        self.size = size;
        {
            let _surfacing = matterless_view::timing::watch("  reconfiguring the surface", 0, "");
            if let Some(view) = self.view.as_mut()
                && let Err(why) = view.resize(size)
            {
                eprintln!("could not resize the surface: {why}");
            }
        }
        // Re-wrapped as the edge moves rather than once it stops, and by the
        // same path that will run when it does stop -- only over less of the
        // conversation. The rest is settled once the dragging ends.
        self.shape(Shaping::WhatShows, held);
        self.resizing = Some(std::time::Instant::now());
    }

    /// Builds one frame and hands it to the GPU.
    ///
    /// The two halves are timed apart. Building the draw list is this
    /// program's work and is the half worth fixing; handing it over and
    /// waiting for the swapchain is mostly the driver's, and a slow one there
    /// usually means vsync rather than anything here.
    ///
    /// `waiting` is passed down to the present: false only for the frame drawn
    /// from inside a resize, which the window is waiting on.
    fn paint(&mut self, waiting: bool) {
        let building =
            matterless_view::timing::watch("building the frame", self.stream.rows.len(), "rows");
        // A box that grew without the conversation being told is a box drawn
        // over the last message.
        if (self.composer.height(), self.thread_composer.height()) != self.laid_against {
            self.box_changed_size();
        }
        self.re_aim();
        // Where the spinner has got to. Per frame, and not in the layout
        // pass: `show_what_is_attached` runs from `relayout`, so a phase set
        // there is the phase at the last time something changed shape -- the
        // window woke, drew, and drew the same picture, which is a spinner
        // that does not spin.
        let spinning = self.began.elapsed().as_secs_f32() / SPIN.as_secs_f32() % 1.0;
        self.composer.spinning = spinning;
        self.thread_composer.spinning = spinning;
        // A tile stops being "on its way" the moment the upload answers, and
        // its thumbnail is another round trip behind that -- so for a second
        // it was a dark square with no spinner and no picture, which is what
        // attaching four files at once showed. The spinner outlasts the
        // upload and stops when there is something to draw.
        Self::still_waiting(self.view.as_ref(), &mut self.composer);
        Self::still_waiting(self.view.as_ref(), &mut self.thread_composer);
        let scene = self.scene();
        drop(building);
        let _drawing = matterless_view::timing::watch("drawing the frame", 0, "");
        let size = self.size;
        let ground = self.palette.ground;
        let Some(view) = self.view.as_mut() else {
            return;
        };
        view.draw_scene(&mut self.fonts, &scene, size, ground, waiting);
        // A frame's worth of input has been acted on.
        self.input.settle();
    }

    /// The sign-in screen, and nothing behind it.
    ///
    /// Its own scene rather than a panel over the usual one: with no session
    /// there is no sidebar, no channel and no conversation -- what would be
    /// behind it is the invented sample, and a sign-in box floating over that
    /// says the sample is the reader's.
    fn signin_scene(&mut self) -> Scene {
        let mut scene = Scene::default();
        let window = self.below_notice();
        self.signin.measure(&mut self.fonts, window);
        let mut canvas = Canvas {
            scene: &mut scene,
            painter: &mut self.painter,
            fonts: &mut self.fonts,
            palette: &self.palette,
        };
        self.signin.draw(&mut canvas, &self.input, window);
        // Over it, because a window that cannot sign in is exactly the one
        // that may be too old to. Drawn after, so the strip is not inside the
        // card's own background.
        if self.offered.open() {
            let notice = self.notice_rect();
            let mut canvas = Canvas {
                scene: &mut scene,
                painter: &mut self.painter,
                fonts: &mut self.fonts,
                palette: &self.palette,
            };
            self.offered.draw(&mut canvas, &self.input, notice);
        }
        scene
    }

    /// Everything the frame draws, in one scene.
    fn scene(&mut self) -> Scene {
        if self.signing_in() {
            return self.signin_scene();
        }
        let mut scene = Scene::default();
        let sidebar = self.sidebar_rect();
        let strip = self.header_rect();
        // No test can reach this one: it needs a window with a thread open in
        // it. The strip's buttons are laid in from its right edge, so a strip
        // that reaches under the pane puts every one of them there.
        debug_assert!(
            self.thread_rect()
                .is_none_or(|pane| strip.right() <= pane.x + 0.5),
            "the channel's strip runs under the thread pane"
        );
        // An open editor whose row has not been told to make room for it.
        //
        // No test can reach this either, and it is the fault that got past
        // two of them: the arithmetic for the row was right and correct in
        // isolation, and nothing ever called it, because `Action::Edit`
        // opened the editor without asking for a re-layout. What the tests
        // checked was the order this file was *supposed* to use.
        debug_assert!(
            !self.edit.open()
                || self.edit.for_post == self.stream.editing.as_ref().map(|(id, _)| id.clone())
                || self
                    .thread
                    .as_ref()
                    .is_some_and(|thread| thread.editing.is_some()),
            "the editor is open and no row has made room for it"
        );
        let stream = self.stream_rect();
        // With nowhere to write, the conversation has the column down to the
        // bottom edge. Room kept for a box that is not drawn is a band of bare
        // ground under the last message, and on the window this guards -- no
        // store, so the sample and nothing else -- that band is most of what
        // there is to look at.
        debug_assert!(
            self.can_write() || (stream.bottom() - self.channel_rect().bottom()).abs() < 0.5,
            "no message box, and the conversation still stops short of the bottom"
        );

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
        let strip_field = self.strip_field();
        let typed_in = self.search.asking() && strip_field.is_some();
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
        if self.store.is_none() {
            // Nothing, for the same reason the name changed: a hash says
            // "public channel on a server", and there is no server.
            header.sigil = "";
        }
        scene.clip_to(strip.x, strip.y, strip.width, strip.height);
        let mut canvas = Canvas {
            scene: &mut scene,
            painter: &mut self.painter,
            fonts: &mut self.fonts,
            palette: &self.palette,
        };
        header.draw(&mut canvas, strip, on_strip, typed_in);
        // Over the strip rather than in it. The strip draws a placeholder while
        // the search is shut and leaves the room empty once it is open, so this
        // is the box itself -- the one the reader clicked, with the caret in
        // it, where the caret used to appear on the far side of the window.
        if let Some(field) = strip_field.filter(|_| typed_in) {
            let within = self.search.query.around(field);
            self.search.query.draw(&mut canvas, within, true);
        }
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
            self.stream
                .draw(&mut canvas, stream, &self.input, &self.presence);
        }
        drop(probe);
        let probe = matterless_view::timing::watch("  the thread pane", 0, "");

        // The thread pane: its own header and its own clip, so a reply cannot
        // spill into the conversation it came from.
        // Mutable because a stream records where it drew each link, which is
        // what the next frame hit-tests against.
        let following = self.following_open_thread();
        // Read before the pane is: who is around is a fact about the window,
        // and drawing the thread holds the rest of it.
        let presence_of = self.presence.clone();
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
            // Never the box being typed into: the query is on the channel's
            // own strip, and this one is the thread's title bar.
            title.draw(&mut canvas, strip, on_strip, false);

            scene.clip_to(rows.x, rows.y, rows.width, rows.height);
            let mut canvas = Canvas {
                scene: &mut scene,
                painter: &mut self.painter,
                fonts: &mut self.fonts,
                palette: &self.palette,
            };
            thread.draw(&mut canvas, rows, &self.input, &presence_of);

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
                self.thread_composer
                    .draw_over(&mut canvas, &self.input, body);
            }
        }

        drop(probe);
        let probe = matterless_view::timing::watch("  the composers", 0, "");
        // The message box, its own layer, so the caret and the box sit over
        // the stream rather than under a message that scrolled into the
        // strip. Not at all while the threads are up, and not at all with no
        // store: neither is somewhere to write.
        if self.can_write() {
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
            self.composer.draw_over(&mut canvas, &self.input, within);
        }

        // A pill over the foot of each conversation, and partly over the
        // blank the box keeps above itself -- which is why it is drawn after
        // the boxes rather than before them: the composer fills its whole
        // strip with ground, so a pill under it would be painted out. It
        // ends exactly where the box begins, so none of the box is covered.
        let open = self.sidebar.selected.clone().unwrap_or_default();
        let lines = [
            (
                self.typing_rect(),
                self.typing.line(&open, "", &self.typing_names),
            ),
            (
                self.thread_typing_rect()
                    .unwrap_or(Rect::new(0.0, 0.0, 0.0, 0.0)),
                self.open_root()
                    .and_then(|root| self.typing.line(&open, &root, &self.typing_names)),
            ),
        ];
        for (strip, said) in lines {
            let Some(said) = said else { continue };
            if strip.width <= 0.0 {
                continue;
            }
            let Some((pill, said)) = Self::typing_pill(&mut self.fonts, strip, &said) else {
                continue;
            };
            // Wide enough for the shadow, which reaches past the pill on
            // every side. Clipped to the strip alone it is cut off square,
            // which is the one thing a shadow must not be.
            scene.clip_to(
                strip.x,
                strip.y - TYPING_DROP,
                strip.width,
                strip.height + TYPING_DROP * 2.0,
            );
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
            // A floating panel, not a fill: it sits over a message now, and
            // what says a thing is in front of the window rather than part of
            // it is the hairline, not the darkness under it.
            matterless_widgets::Panel::floating(pill, pill.height / 2.0, TYPING_DROP)
                .edge(palette.rule)
                .fill(palette.surface)
                .draw(scene);
            let glyphs = painter.run(
                fonts,
                &said,
                pill.x + TYPING_PADDING,
                pill.y + (pill.height - 18.0) / 2.0,
                matterless_paint::Run::label(f32::MAX),
            );
            scene.glyphs(glyphs, palette.soft, palette.faint);
        }

        // Over both boxes, because it is in front of the one being typed in
        // and has to be readable over the conversation behind it.
        if let Some((_, box_of, within)) = self.naming_in() {
            scene.clip_to(0.0, 0.0, self.size.0 as f32, self.size.1 as f32);
            let mut canvas = Canvas {
                scene: &mut scene,
                painter: &mut self.painter,
                fonts: &mut self.fonts,
                palette: &self.palette,
            };
            self.naming.draw(&mut canvas, box_of, within, &self.input);
        }

        drop(probe);
        // Every overlay after this is drawn on the Threads view too. It used to
        // stop here when the view was open, which left the Drafts pane an empty
        // band beside the list -- its room taken, nothing drawn in it -- and the
        // switcher, the profile card, Settings, menus and tooltips invisible.
        let _probe = matterless_view::timing::watch("  the overlays", 0, "");

        // The list under the strip's field, over the conversation: it hangs
        // below a strip that clips its own drawing, so it is drawn out here
        // where nothing cuts it off.
        if let Some(field) = self.strip_field().filter(|_| self.search.asking()) {
            scene.clip_to(0.0, 0.0, self.size.0 as f32, self.size.1 as f32);
            let mut canvas = Canvas {
                scene: &mut scene,
                painter: &mut self.painter,
                fonts: &mut self.fonts,
                palette: &self.palette,
            };
            self.search.draw_droplist(&mut canvas, &self.input, field);
        }

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
            self.switcher.draw(&mut canvas, stream, &self.presence);
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
            self.search.draw(&mut canvas, &self.input, pane);
            if self.search.open() {
                let field = self.search.in_pane(pane);
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
            self.edit.draw(&mut canvas, &self.input, row, stream);
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
            let within = self.picker_within();
            let field = self.picker.field(near, within);
            scene.clip_to(0.0, 0.0, self.size.0 as f32, self.size.1 as f32);
            let mut canvas = Canvas {
                scene: &mut scene,
                painter: &mut self.painter,
                fonts: &mut self.fonts,
                palette: &self.palette,
            };
            self.picker.draw(&mut canvas, &self.input, near, within);
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
        if self.settings.open() {
            scene.clip_to(0.0, 0.0, self.size.0 as f32, self.size.1 as f32);
            let window = self.window_rect();
            let mut canvas = Canvas {
                scene: &mut scene,
                painter: &mut self.painter,
                fonts: &mut self.fonts,
                palette: &self.palette,
            };
            self.settings.draw(&mut canvas, &self.input, window);
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

    /// Asks again what the pointer is on, against the boxes this frame will
    /// be drawn from.
    ///
    /// The pointer does not move because a list got shorter, and `hovered` is
    /// only ever set by a move -- so a row sliding under a still pointer (the
    /// next unread conversation, once the one above it has been read) stayed
    /// dark, kept the arrow, and took no press. Every layout change ends in a
    /// frame, so the frame is the one place that catches all of them.
    fn re_aim(&mut self) {
        // Never while a button is held. A press that opens something under the
        // pointer -- the settings panel over the gear that opened it -- would
        // otherwise be released onto a box it did not begin on, and a press
        // that disagrees with its release is not a click at all. A held press
        // that the reader actually drags is re-aimed by the move itself.
        if self.input.pointer_at().is_none() || self.input.pressed().is_some() {
            return;
        }
        let was = self.input.hovered().map(str::to_string);
        let boxes = self.targets();
        self.input.aim(&boxes);
        self.placed = boxes;
        // Only when it changed. `watch_pointer` restarts the tooltip's wait,
        // so asking it once a frame is a tooltip that never appears.
        if self.input.hovered() != was.as_deref() {
            self.watch_pointer();
        }
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
        // A hand over whatever answers a press. Everything in this window
        // looked alike under the pointer: a link, the message holding it and
        // the empty margin beside them were one flat surface, and the only way
        // to find out whether a thing could be pressed was to press it.
        let over = hovered.as_deref().is_some_and(|name| self.pressable(name));
        // An edge that can be dragged says so with the arrows, not the hand:
        // a hand means "this answers a press", and pressing the edge does
        // nothing at all. The drag is the whole of what it is for.
        let on_the_edge = hovered.as_deref() == Some(GRIP) || self.input.pressed() == Some(GRIP);
        if let Some(window) = self.window.as_ref() {
            window.set_cursor(match (on_the_edge, over) {
                (true, _) => winit::window::CursorIcon::ColResize,
                (false, true) => winit::window::CursorIcon::Pointer,
                (false, false) => winit::window::CursorIcon::Default,
            });
        }
        let at = self.input.pointer_at();
        self.tooltip.follows(hovered.as_deref(), at, |_| {
            (!explained.is_empty()).then_some(explained)
        });
    }

    /// Whether pressing what the pointer is over would do anything.
    ///
    /// A list of what *is* pressable rather than what is not, because the
    /// surfaces are not the opposite of the controls: a row in the
    /// conversation is neither. It catches the pointer so the toolbar knows
    /// which message to hang off, and pressing it does nothing at all -- a
    /// hand over every message would be a lie told the whole length of the
    /// window.
    ///
    /// The alternative was a field on `Placed` saying so, set at each of the
    /// forty places one is built. That is the tidier answer and a great deal
    /// of edits for a cursor; this is one function, and the day a control
    /// stops offering a hand is the day somebody adds a name to it.
    fn pressable(&self, name: &str) -> bool {
        // Anything the window can say something about does something. The two
        // are written next to each other and gain their entries together.
        if self.explains(name).is_some() {
            return true;
        }
        // And the controls whose purpose is written on them, so they never
        // needed explaining: rows in a panel, and the buttons on a box.
        const PANELS: [&str; 7] = [
            matterless_view::menu::NAME,
            matterless_view::picker::NAME,
            matterless_view::switcher::NAME,
            matterless_view::listing::NAME,
            matterless_view::whats_new::NAME,
            matterless_view::updater_bar::NAME,
            matterless_view::viewer::NAME,
        ];
        const BUTTONS: [&str; 4] = ["/send", "/attach", "/close", "/newest"];
        name.starts_with("sidebar/channel/")
            || name.starts_with("sidebar/team/")
            || name == matterless_view::sidebar::NEW
            || PANELS
                .iter()
                .any(|panel| name.starts_with(&format!("{panel}/")))
            || BUTTONS.iter().any(|button| name.ends_with(button))
    }

    /// Tells the conversation how tall the rows it has not measured were, from
    /// the last time this machine drew them.
    ///
    /// One query for the whole channel. What comes back is heights and
    /// nothing else -- the blocks are needed only for the dozen rows actually
    /// on screen, and a `RowLayout` carries a copy of the message's text,
    /// which is several hundred bytes against thirty.
    fn foresee_heights(&mut self) {
        let Some(store) = self.store.as_ref() else {
            return;
        };
        let width = self.stream.laid_at_width().round() as u32;
        let fingerprint = self.stream.fingerprint();
        let wanted: Vec<String> = self.stream.waiting_keys();
        if wanted.is_empty() {
            return;
        }
        match store.heights_of(width, &fingerprint, &wanted) {
            Ok(known) => {
                self.stream.foresee(&known);
                println!(
                    "the channel's length was remembered for {} of {} rows not yet measured",
                    known.len(),
                    wanted.len()
                );
            }
            Err(error) => eprintln!("reading remembered heights: {error}"),
        }
    }

    /// Writes down how tall every measured row was drawn.
    ///
    /// After the width has settled and never during a drag: a height measured
    /// against a column somebody is still moving is an answer about a width
    /// nobody ended up at, and it would fill the table with them.
    fn remember_heights(&mut self) {
        let Some(store) = self.store.as_ref() else {
            return;
        };
        if self.resizing.is_some() {
            return;
        }
        let width = self.stream.laid_at_width().round() as u32;
        let fingerprint = self.stream.fingerprint();
        let measured = self.stream.measured();
        if measured.is_empty() {
            return;
        }
        if let Err(error) = store.keep_heights(
            width,
            &fingerprint,
            &measured,
            matterless_view::clock::now() * 1_000,
        ) {
            eprintln!("remembering heights: {error}");
        }
    }

    /// Lays out again for a box that grew or shrank, keeping the foot of the
    /// conversation where it was.
    ///
    /// The relayout alone is not enough, and this is the half that was
    /// reported twice: the anchor holds the row across the *middle* for a
    /// reader who is not at either end, so a panel losing height from the
    /// bottom keeps that row still and slides the newest message under the
    /// box. In a conversation the bottom is the edge that matters -- a
    /// growing message box must not cover what is being replied to.
    ///
    /// The distance from the newest message is what is kept, not the height
    /// that was lost: the anchor has already moved the scroll by half of that
    /// to hold the middle row, and adding the loss on top overshot by ten.
    ///
    /// Measured against the panel the reader was **looking at**, which is not
    /// what `stream_rect` answers: the box has already grown by the time this
    /// runs. Asking the new panel and restoring against the new panel is the
    /// same question twice and hands back the scroll unchanged -- which is
    /// exactly what the first attempt did, and why the report came back. The
    /// same "asked too late" fault as the drag anchor and the stale hit box.
    fn box_changed_size(&mut self) {
        let now = self.stream_rect();
        let grew = self.composer.height() - self.laid_against.0;
        let was = Rect::new(now.x, now.y, now.width, now.height + grew);
        let behind = self.stream.behind(was);
        self.relayout();
        let within = self.stream_rect();
        let reach = self.stream.reach(within);
        if reach <= 0.0 {
            return;
        }
        self.stream.scroll = (reach - behind).clamp(0.0, reach);
    }

    /// Lays the whole conversation out for the current width.
    fn relayout(&mut self) {
        let held = self.anchors();
        self.shape(Shaping::Everything, held);
    }

    /// Where the reader is in each panel, for putting them back afterwards.
    ///
    /// Every row is about to be measured against a different column, so the
    /// scroll -- which is a number of pixels -- will point somewhere else once
    /// that is done. Opening a thread narrows the conversation beside it, and
    /// the message whose replies had just been asked for was the first thing
    /// to slide out from under the pointer.
    ///
    /// Which place each panel keeps depends on where its reader is sitting:
    /// `Stream::anchor` decides that, between the two edges and the row across
    /// the middle.
    fn anchors(&self) -> Anchors {
        (
            self.stream.anchor(self.stream_rect()),
            self.thread_stream_rect()
                .and_then(|within| Some(self.thread.as_ref()?.anchor(within))),
        )
    }

    /// Measures the window's contents against the column they now have.
    ///
    /// One path for both of the times this happens, which is the point of it.
    /// A drag shapes only what shows, because a channel is a hundred and
    /// eighty messages of cosmic-text and a screenful is a dozen; when the
    /// edge stops, the rest catch up. That is the *only* difference between
    /// them. Everything else -- which width each panel gets, the order they
    /// are measured in, and where the reader is put afterwards -- is this one
    /// body, so that letting go of the edge cannot rearrange anything the drag
    /// had already settled.
    ///
    /// The anchors are the caller's to take, and are not taken here, because
    /// *when* they were taken is the whole of whether they work: they have to
    /// describe the panel the reader is still looking at. Measured against the
    /// panel they are about to be in, and put back against that same panel,
    /// they say nothing at all -- so the one caller that changes the window's
    /// size takes them first and hands them over.
    fn shape(&mut self, how: Shaping, (held, thread_held): Anchors) {
        // What each box is carrying, before it is measured: an attachment
        // waiting in it is a row of its height.
        self.show_what_is_attached();
        // And which row has to make room for the editor, before the rows are
        // laid out: the row gives up its words for the box's height, so the
        // height has to be settled first.
        //
        // Which means shaping the box here, at the width it is about to be
        // given. Its height is a number of lines and a number of lines is an
        // answer about a width -- so asked before that, it is the height the
        // box had in the column it used to be in. Measured: the row was told
        // 116 where the box then drew 136, and nothing re-shaped it, so the
        // box overlapped the name above it and the message below for as long
        // as it was open. The width does not depend on the height, so there
        // is no circle here to break.
        if self.edit.open() {
            let width = self
                .edited_row()
                .map(|room| room.width)
                .unwrap_or_else(|| self.stream_rect().width);
            self.edit.box_of.lay_out(&mut self.fonts, width);
        }
        let editing = self
            .edit
            .for_post
            .clone()
            .map(|post_id| (post_id, self.edit.height()));
        self.stream.editing = editing.clone();
        if let Some(thread) = self.thread.as_mut() {
            thread.editing = editing;
        }
        // The composer is shaped first: it decides its own height, and the
        // stream gets what is left, so its width has to be settled before the
        // rows are laid out against it.
        let width = self.channel_rect().width;
        self.composer.lay_out(&mut self.fonts, width);

        self.laid_against = (self.composer.height(), self.thread_composer.height());
        let stream = self.stream_rect();
        {
            let _shaping = matterless_view::timing::watch(
                how.of("shaping the channel", "  shaping what shows of it"),
                self.stream.rows.len(),
                "rows",
            );
            match how {
                Shaping::WhatShows => self
                    .stream
                    .relay_seen(&mut self.fonts, stream.width, stream),
                Shaping::Everything => self.stream.lay_out(&mut self.fonts, stream.width),
            }
        }
        self.stream.anchored(held, stream);

        if let Some(pane) = self.thread_rect() {
            self.thread_composer.lay_out(&mut self.fonts, pane.width);
        }
        if let Some(within) = self.thread_stream_rect()
            && let Some(thread) = self.thread.as_mut()
        {
            let _shaping = matterless_view::timing::watch(
                how.of("shaping the thread", "  shaping what shows of the thread"),
                thread.rows.len(),
                "rows",
            );
            match how {
                Shaping::WhatShows => thread.relay_seen(&mut self.fonts, within.width, within),
                Shaping::Everything => thread.lay_out(&mut self.fonts, within.width),
            }
            if let Some(thread_held) = thread_held {
                thread.anchored(thread_held, within);
            }
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
    Window::default_attributes()
        .with_taskbar_icon(window_icon(BIG_ICON))
        // Everything in here is drawn through the swapchain, so the surface
        // the system keeps for GDI to paint into is only ever seen where the
        // swapchain has not covered yet -- the strip a drag has just added,
        // which nothing paints, and which showed as a white band.
        .with_no_redirection_bitmap(true)
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
        if let Some(driver) = self.driver.as_ref() {
            if driver.act().is_none() {
                // Every act measured, and the exit already asked for.
                return events.set_control_flow(ControlFlow::Wait);
            }
            // Only asked for here. The act itself belongs to the frame: this
            // runs whenever the loop has nothing left to deliver, which is
            // several times per frame, and an act driven from here happened
            // seven times for every one it was supposed to.
            self.redraw();
            return events.set_control_flow(ControlFlow::Poll);
        }
        // The rest of the conversation, once the edge has stopped moving.
        // Asked for here because this already runs between every pair of sizes
        // a drag reports, so there is no timer to keep.
        if let Some(since) = self.resizing
            && since.elapsed() >= SETTLE
        {
            self.resizing = None;
            self.relayout();
            self.redraw();
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
        // A picture with more than one frame asks to be woken when the frame
        // showing runs out, rather than the window redrawing continuously for
        // as long as one is on screen: a loop is ten frames a second and a
        // window is sixty, so five of every six of those frames would draw
        // the picture that is already there.
        // A draft goes to the store once the typing has stopped, rather than
        // waiting for the conversation to be left.
        let written = self.settled_draft();
        let playing = self.playing();
        // Asked as "is a frame other than the one showing due", not as "has
        // the wake time passed": the wake is the end of the frame showing, so
        // by the time it fires the answer to the second question is always no.
        if self.overdue() {
            self.redraw();
        }
        let filling = self.shape_some();
        // A spinner is the one thing on screen that moves with nothing
        // happening, so the window has to ask for its own frames -- and only
        // while something is actually going up.
        let spinning = (!self.attaching.is_empty() || self.anything_loading())
            .then(|| std::time::Instant::now() + SPIN / SPOKES_A_TURN);
        if spinning.is_some() {
            self.redraw();
        }
        let next = [
            next,
            written,
            playing,
            spinning,
            self.tooltip.wakes(),
            self.rest.wakes(),
            // So the window wakes to finish shaping even when the drag ends
            // with nothing else to wake it.
            self.resizing.map(|since| since + SETTLE),
        ]
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
                        // The name and nothing else. It read "MatterLess -- list on Vulkan",
                        // which was a note to whoever was porting the renderer -- it has not
                        // been Vulkan since the Direct3D port, and a title bar is the one
                        // piece of a window every screenshot carries.
                        .with_title("MatterLess")
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

        // Direct3D. Everything the window draws through is built here and
        // owned by the view: the device, the swapchain, the pipeline and the
        // sheets.
        let handle = match raw_window_handle::HasWindowHandle::window_handle(&window)
            .expect("a window handle")
            .as_raw()
        {
            raw_window_handle::RawWindowHandle::Win32(win32) => {
                windows::Win32::Foundation::HWND(win32.hwnd.get() as *mut _)
            }
            _ => return events.exit(),
        };
        let view = match matterless_view::View::new(handle, self.size) {
            Ok(view) => view,
            Err(why) => {
                eprintln!("could not open Direct3D: {why}");
                return events.exit();
            }
        };
        println!("drawing through Direct3D, {}x{}", self.size.0, self.size.1);
        self.view = Some(view);
        // How this reader has asked for text to be drawn, before a glyph is
        // rasterised: applying it later would throw away a sheet's worth of
        // letters on the first frame.
        let asked = matterless_view::settings::Text::read(
            self.store
                .as_ref()
                .and_then(|store| {
                    store
                        .setting(matterless_view::settings::Text::SETTING)
                        .ok()
                        .flatten()
                })
                .as_deref(),
        );
        self.settings.text = asked;
        // How much opened pictures may keep on disk, applied before any is.
        let kept = matterless_view::settings::Kept::read(
            self.store
                .as_ref()
                .and_then(|store| {
                    store
                        .setting(matterless_view::settings::Kept::SETTING)
                        .ok()
                        .flatten()
                })
                .as_deref(),
        );
        self.settings.kept = kept;
        if let Some(held) = matterless_view::filecache::looked() {
            held.set_budget(kept.bytes());
        }
        if let Some(view) = self.view.as_mut() {
            view.atlas
                .rasterise_subpixel(asked == matterless_view::settings::Text::Subpixel);
        }
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
        println!("ready in {}ms", began.elapsed().as_millis());
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
                // Minimising is reported as a resize to nothing -- measured:
                // 0x0 on the way down, then the real size on the way back --
                // and a conversation re-wrapped to a column one pixel wide is
                // worse than wasted work. The anchor that puts the reader
                // back where they were is taken against that panel on the way
                // out and read against it on the way in, and an anchor
                // measured against a panel nobody is looking at says nothing
                // at all: that is the frame of broken layout after a restore.
                // It also spent a slot of the three-width row cache on a
                // width no reader will ever see.
                if size.width == 0 || size.height == 0 {
                    return;
                }
                self.sized = Some((size.width, size.height));
                // Drawn here, before this handler comes back, rather than left
                // for the next frame. The window is already the new size when
                // this runs, and the compositor will show it at that size with
                // whatever the swapchain last held -- which, a frame behind,
                // is a picture too small for it. That is the black strip along
                // the edge being dragged: not something drawn wrongly but a
                // frame that has not arrived yet.
                //
                // So it arrives now. The cost is the window sitting in here
                // for the length of a frame -- some six milliseconds, against
                // the tenth of a second that made this deferred in the first
                // place -- and the present does not wait for the screen,
                // because the thing waiting for this frame is the resize.
                //
                // Not while the driver is measuring: it asks for the sizes
                // itself, from inside the frame, and would be re-entered here.
                match self.driver.is_none() {
                    true => {
                        self.take_the_size();
                        self.paint(false);
                    }
                    false => self.redraw(),
                }
            }
            WindowEvent::CursorMoved { position, .. } => {
                self.point_at(position.x as f32, position.y as f32);
            }
            // A file dragged onto the window joins the message being written
            // in the conversation under the pointer -- the thread if one is
            // open and the pointer is in it, the channel otherwise.
            //
            // It used to be the whole message and posted at once, which is the
            // one thing a reader cannot take back. A drop and a paste mean the
            // same thing now: here is a file for what I am writing. Sending is
            // still the reader's own gesture.
            WindowEvent::DroppedFile(path) => {
                let Some(channel_id) = self.sidebar.selected.clone() else {
                    eprintln!("nowhere to attach that");
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
                    link.send(matterless_view::live::Ask::Attach {
                        channel_id: channel_id.clone(),
                        root_id: root_id.clone(),
                        path: path.clone(),
                    });
                    // On screen now, not when the server answers. A large
                    // file is seconds of upload and then another round trip
                    // for its thumbnail, and until both were done the window
                    // showed nothing at all -- so a drop looked like it had
                    // missed.
                    let under = match root_id.is_empty() {
                        true => channel_id,
                        false => thread_name(&root_id),
                    };
                    self.attaching.entry(under).or_default().push(path);
                    self.relayout();
                    self.redraw();
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
                    // And what arrived behind the window has now been come
                    // back to. Without this the badge stayed lit for as long
                    // as somebody read, because nothing between opening a
                    // channel and leaving it ever said it had been read --
                    // and alt-tabbing away and back is the commonest way to
                    // be handed messages you then sit and read.
                    self.read_what_is_open();
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
                // Where the reader stopped reading, which is the one place
                // in a long channel that is hard to find by hand.
                if down && self.input.chord(Key::Char('u')) {
                    self.show_unread_mark();
                }
                // On to the next conversation with something in it, which is
                // the other half of the same errand and deliberately not the
                // same gesture: one control that sometimes scrolls the page
                // and sometimes takes you somewhere else is two controls
                // wearing one coat. Alt+Shift, as the official client has it.
                let mods = self.input.mods();
                if down && mods.alt && mods.shift && !mods.command {
                    for (key, on) in [(Key::Down, true), (Key::Up, false)] {
                        if self.input.struck(key) {
                            self.walk_to_unread(on);
                        }
                    }
                }
                // Answered from the store, so it is filled the moment it
                // opens rather than after a round trip.
                if down && self.input.chord(Key::Char('t')) {
                    self.show_the_list("Threads");
                    if let Some(store) = self.store.clone() {
                        let found = matterless_view::listing::followed(&store, &self.me, THREADS);
                        println!("Threads: {} followed", found.len());
                        self.listing.fill(found);
                    }
                }
                if down && self.input.chord(Key::Char('f')) {
                    let mut input = std::mem::take(&mut self.input);
                    // The same thing the box does, when there is a box. On a
                    // window too narrow for one there is nowhere to hang a
                    // list, so the pane is the whole of the search.
                    match self.strip_field().is_some() {
                        true => self.search.peek(&mut self.fonts, &mut input),
                        false => self.search.show(&mut self.fonts, &mut input),
                    }
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
                    self.offer_spelling_menu();
                    self.react();
                    self.redraw();
                    return;
                }
                // The two buttons under the thumb, which every browser and
                // the official client answer with the previous and next
                // place. They were dropped here along with everything else
                // that is not the left button.
                if matches!(
                    button,
                    winit::event::MouseButton::Back | winit::event::MouseButton::Forward
                ) {
                    if state != winit::event::ElementState::Pressed {
                        return;
                    }
                    self.stepped(button == winit::event::MouseButton::Forward);
                    return;
                }
                if button != winit::event::MouseButton::Left {
                    return;
                }
                let boxes = self.targets();
                let down = state == winit::event::ElementState::Pressed;
                let event = match down {
                    true => UiEvent::PointerPressed,
                    false => UiEvent::PointerReleased,
                };
                self.input.apply(event, &boxes);
                // Whether this press carries on from the last one, which is a
                // question about a clock and so is answered here rather than
                // in the input layer.
                if down && let Some(again) = self.repeated() {
                    self.input.apply(UiEvent::PointerRepeated(again), &boxes);
                }
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
                if let Some(pressed) = self.rail.react(&self.input) {
                    // Every other square on the rail is somewhere to be, and
                    // this one is something to do -- so it is answered before
                    // the sidebar is scrolled to a heading that does not exist.
                    // What it does is bring the conversations with something
                    // waiting into view in the list, as the reader asked of
                    // it; where they stopped reading inside one is `u`.
                    if pressed == matterless_view::rail::UNREAD {
                        let within = self.sidebar_rect();
                        if !self.sidebar.scroll_to_unread(within) {
                            println!("nothing unread to scroll to");
                        }
                    } else if pressed == matterless_view::rail::SETTINGS {
                        self.settings.show();
                        let window = self.window_rect();
                        self.settings.measure(&mut self.fonts, window);
                    } else {
                        let within = self.sidebar_rect();
                        self.sidebar.scroll_to(&pressed, within);
                    }
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
                let stood_in = self.sidebar.selected.clone();
                if let Some(channel) = self.sidebar.react(&self.input, &boxes, within) {
                    self.open_channel(&channel);
                    // The drafts row lists conversations to go back to rather
                    // than being one, so the reader has not moved: the strip
                    // keeps the channel's name and the sidebar keeps its mark.
                    // Pressing the row otherwise left the header reading
                    // "# drafts" with no way back to the name.
                    if channel == matterless_view::sidebar::DRAFTS {
                        self.sidebar.selected = stood_in;
                    }
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
                    // The box on the strip is a real field: it takes the
                    // keyboard and hangs what matched under it, and Return is
                    // what asks for the pane. Somebody looking for one message
                    // gets it without the window rearranging around them.
                    //
                    // Only while it is shut: a second click in it is a reader
                    // putting the caret somewhere, and starting again would
                    // throw away what they had typed.
                    if pressed == "find" {
                        if !self.search.busy() {
                            let mut input = std::mem::take(&mut self.input);
                            self.search.peek(&mut self.fonts, &mut input);
                            self.input = input;
                        }
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
                self.wheel_by(by);
            }
            WindowEvent::RedrawRequested => {
                if self.view.is_none() {
                    return;
                }
                // Before anything measures itself against the window.
                self.take_the_size();
                // However much of a turn of the wheel belongs to this frame,
                // before the frame is built from where things now are. Asking
                // for the next one here rather than at the end, because what
                // follows can return early and the rest of the distance would
                // be left undelivered.
                if self.glided() {
                    self.redraw();
                }
                // Whichever message has the emoji grid open keeps its toolbar,
                // though the pointer has left it for the grid. Derived once a
                // frame rather than set beside every `show` and `hide`, so it
                // cannot be left standing after the grid has gone.
                let held = self.picker.for_post.clone();
                self.stream.held = held.clone();
                if let Some(thread) = self.thread.as_mut() {
                    thread.held = held;
                }
                // Once, here, before the frame it is measured by.
                if let Some(driver) = self.driver.as_ref()
                    && let Some(act) = driver.act()
                {
                    let (tick, fresh, counting) = (driver.tick, driver.fresh(), driver.counting());
                    let did = self.drive(act, tick, fresh);
                    if let Some(driver) = self.driver.as_mut()
                        && counting
                    {
                        driver.did += did;
                    }
                }
                // Whatever frame a moving picture is due, into the same queue
                // the still ones use: a frame is the same size as the one
                // before it and goes into the same slot, so nothing above this
                // knows a picture moved.
                self.played();
                // Pictures go into the atlas here, on the thread that owns the
                // GPU and before the scene names them: uploading after the
                // draw list is built would show a face one frame late.
                if !self.arrived.is_empty()
                    && let Some(view) = self.view.as_mut()
                {
                    let mut placed = 0;
                    let mut refused = 0;
                    for (key, width, height, rgba) in self.arrived.drain(..) {
                        match view.put_image(&key, &rgba, width, height) {
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
                self.paint(true);
                if let Some(driver) = self.driver.as_mut()
                    && let Some((act, did)) = driver.drew()
                {
                    println!(
                        "\n== {act}, {did} times =={}",
                        matterless_view::timing::table()
                    );
                    if did == 0 {
                        println!("  nothing happened -- this act measured a still frame");
                    }
                    matterless_view::timing::forget();
                    if driver.over() {
                        self.clear_what_the_driver_typed();
                        events.exit();
                    }
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
        // The reader's own clock, as the window uses. `Theme::default` carries
        // an offset of zero, so a snapshot drew every time in UTC -- which is
        // two hours out here, and the whole point of a snapshot is comparing
        // it against a client that is not.
        today: matterless_view::clock::today(),
        utc_offset_minutes: matterless_view::clock::utc_offset_minutes(),
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
    // The spelling dictionaries, on a thread of their own from the start: they
    // take a few tens of milliseconds, and the box underlines nothing until
    // they are there rather than making the first frame wait for them.
    matterless_view::spell::load_in_background();
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
/// Cloneable because a picture is fetched on a task of its own now, and each
/// one has to be able to wake the window when its bytes land. winit's proxy is
/// itself a handle, so a clone is another handle to the same loop.
#[derive(Clone)]
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
/// What the row at the top of the sidebar says about the reader.
///
/// A struct rather than a tuple of three strings and a flag: it gained a
/// fourth field and the call site was already four positional arguments of
/// which two were `&str`, which is the shape where an argument quietly goes
/// in the wrong slot.
struct Who<'a> {
    name: &'a str,
    status: &'a str,
    /// Whether this window is hearing anything.
    live: bool,
    /// Why it is not, when it is not.
    offline: Option<&'static str>,
}

/// Which box a message is being typed into.
///
/// Two of them keep a draft; the third is the editor, which is changing a
/// message that has already been said and so has nothing to park. It is here
/// because it is a composer like the other two and everything that asks "what
/// is being typed" has to be able to name it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Which {
    Channel,
    Thread,
    Editing,
}

/// Where the reader was in the conversation and in the thread beside it, if
/// one is open. Taken before anything moves and handed to `shape`.
type Anchors = (Anchor, Option<Anchor>);

/// Where the channel was when a thread took half of it.
///
/// Opening a thread narrows the column and holds the line that was pressed,
/// which moves the channel: the press is the best thing to honour at that
/// moment, but it is not where the reader had been. Closing the pane has no
/// press to honour, so what it owes them is the column they started with.
struct Parked {
    /// The place the channel held before the pane opened.
    was: Anchor,
    /// And where the pane's opening left the scroll, so that a reader who has
    /// since moved the channel themselves can be told from one who has not.
    /// Theirs is the newer word about where they are.
    left_at: f32,
}

/// How much of the conversation a pass measures.
///
/// The only thing the two passes differ in. Both go through `shape`, so a
/// drag and the settle that follows it cannot lay the window out differently:
/// whatever was on screen while the edge moved is where it stays when the
/// edge is let go, and the settle only fills in what was never looked at.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Shaping {
    /// The rows on screen, for a width that is still changing. A channel is a
    /// hundred and eighty messages of cosmic-text and a screenful is a dozen.
    WhatShows,
    /// All of it, once the edge has stopped.
    Everything,
}

impl Shaping {
    /// Picks the name this pass goes under in the timing table.
    fn of(self, everything: &'static str, some: &'static str) -> &'static str {
        match self {
            Shaping::Everything => everything,
            Shaping::WhatShows => some,
        }
    }
}

/// How long after the last size before the rest of the conversation is
/// shaped. Short enough to feel like part of letting go of the edge.
const SETTLE: std::time::Duration = std::time::Duration::from_millis(120);

/// How long one turn of a loading spinner takes.
///
/// Slow enough not to strobe, fast enough to read as working. The window
/// wakes once per spoke rather than once per frame: eight frames a turn is
/// what the eye needs, and sixty would be drawing the same picture over.
const SPIN: std::time::Duration = std::time::Duration::from_millis(1_100);
/// How many frames one turn is drawn in.
const SPOKES_A_TURN: u32 = 8;

/// How long a box has to be still before what is in it is written down.
///
/// Long enough that an ordinary sentence is one write rather than thirty, and
/// short enough that somebody who stops to think has already been saved.
const WRITTEN: std::time::Duration = std::time::Duration::from_secs(2);

/// What a thread's conversation is called, and the one place it is spelled.
///
/// A draft's key is this for a reply and the channel's id for a message, so
/// reading one back needs the prefix as well as the joining.
const THREAD_PREFIX: &str = "thread/";

fn thread_name(root_id: &str) -> String {
    format!("{THREAD_PREFIX}{root_id}")
}

/// Whether an offer is worth putting on screen, given what was refused.
///
/// Only the same version is held back. A reader who says "not now" to one
/// build has said nothing about the next, and treating it as "stop telling me
/// about updates" would quietly strand them on whatever they happened to
/// refuse -- which, with the looking now repeating, is the difference between
/// a client that respects an answer and one that has been switched off.
///
/// Whether it is newer at all is settled before this, in `update::offered`.
fn worth_raising(put_off: Option<&str>, version: &str) -> bool {
    put_off != Some(version)
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

#[cfg(test)]
mod tests {
    use super::{Act, App, Driver, Rect, TYPING, TYPING_INSET, TYPING_PADDING, worth_raising};

    /// "Not now" means this one, and only this one.
    ///
    /// The looking repeats every couple of hours now, so an offer the reader
    /// has dismissed would otherwise come back all day. The other half matters
    /// as much: refusing one build must not stop the next from being offered,
    /// or a reader who said "not now" once is never told about anything again.
    #[test]
    fn a_refused_version_is_not_raised_again_and_a_newer_one_is() {
        assert!(worth_raising(None, "0.3.5"), "nothing was refused");
        assert!(
            !worth_raising(Some("0.3.5"), "0.3.5"),
            "the version just dismissed came straight back"
        );
        assert!(
            worth_raising(Some("0.3.5"), "0.3.6"),
            "refusing one build silenced every build after it"
        );
    }

    /// The pill ends where the sentence does.
    ///
    /// Which is the whole of what makes it a pill rather than the band it
    /// replaced: given the width of the column it would be the same strip
    /// wearing a rounded corner, and it floats over a message now, so every
    /// pixel of it that is not words is a pixel of the conversation covered
    /// for nothing.
    #[test]
    fn the_pill_is_as_wide_as_what_it_says() {
        let mut fonts = matterless_layout::Fonts::new();
        let strip = Rect::new(100.0, 400.0, 600.0, TYPING);

        let (short, _) = App::typing_pill(&mut fonts, strip, "amy is typing")
            .expect("a pill in a column with room");
        let (long, _) = App::typing_pill(&mut fonts, strip, "amy, ben and cara are typing")
            .expect("a pill in a column with room");

        assert!(
            long.width > short.width,
            "{} is no wider than {} for a longer sentence",
            long.width,
            short.width
        );
        assert!(
            long.width < strip.width,
            "the pill took the whole column, which is the band again"
        );
        assert_eq!(short.y, strip.y);
        assert_eq!(short.height, strip.height);

        // Centred: the same room to the left of it as to the right. It was
        // pinned to `TYPING_INSET`, which is where a full-width band's words
        // began -- right for a line and wrong for a pill, which has two ends
        // and reads as stuck to whichever one it touches.
        for pill in [short, long] {
            let left = pill.x - strip.x;
            let right = strip.right() - pill.right();
            assert!(
                (left - right).abs() < 0.01,
                "{left} of room on the left and {right} on the right"
            );
        }
    }

    /// A pill too wide to centre keeps its margin rather than both edges.
    ///
    /// Centring a thing wider than its room puts it off both ends at once,
    /// which on this strip means over the thread pane on one side.
    #[test]
    fn a_pill_with_no_room_to_centre_keeps_its_margin() {
        let mut fonts = matterless_layout::Fonts::new();
        let strip = Rect::new(100.0, 0.0, 240.0, TYPING);
        let (pill, _) =
            App::typing_pill(&mut fonts, strip, "amy, ben and cara are typing").expect("a pill");
        assert!(
            pill.x >= strip.x + TYPING_INSET - 0.01,
            "the pill was centred off the left edge of the column"
        );
    }

    /// A column too narrow for the sentence cuts it rather than overflowing.
    ///
    /// The pill is drawn over the conversation, so one running past the edge
    /// of the column would sit over the thread pane beside it.
    #[test]
    fn a_narrow_column_cuts_the_sentence() {
        let mut fonts = matterless_layout::Fonts::new();
        let wide = Rect::new(0.0, 0.0, 600.0, TYPING);
        let narrow = Rect::new(0.0, 0.0, 200.0, TYPING);
        let said = "amy, ben and cara are typing";

        let (_, whole) = App::typing_pill(&mut fonts, wide, said).expect("a pill");
        let (pill, cut) = App::typing_pill(&mut fonts, narrow, said).expect("a pill");

        assert_eq!(whole, said, "a column with room should not cut anything");
        assert!(cut.len() < said.len(), "{cut:?} was not cut to the column");
        assert!(
            pill.right() <= narrow.right(),
            "the pill runs past the column it is drawn in"
        );
    }

    /// A column narrower than the pill's own margins draws no pill at all.
    #[test]
    fn a_column_with_no_room_draws_nothing() {
        let mut fonts = matterless_layout::Fonts::new();
        let none = Rect::new(0.0, 0.0, TYPING_INSET * 2.0 + TYPING_PADDING * 2.0, TYPING);
        assert!(App::typing_pill(&mut fonts, none, "amy is typing").is_none());
    }

    /// A driver with no window, wound by hand.
    fn driver(each: u32) -> Driver {
        Driver {
            at: 0,
            each,
            left: each,
            warm: Driver::WARM,
            tick: 0,
            did: 0,
        }
    }

    /// Winds a whole act and reports what it was called and what it counted.
    ///
    /// Every one of the three things this got wrong -- the count read after it
    /// had been reset, the warm-up's work counted as the act's, the clock held
    /// still so an act fired on every warm frame -- is a sequencing mistake
    /// that is invisible in a table and obvious here.
    fn wind(driver: &mut Driver, each: u32) -> (&'static str, u32) {
        let mut done = None;
        for _ in 0..Driver::WARM + each {
            if driver.counting() {
                driver.did += 1;
            }
            done = driver.drew().or(done);
        }
        done.expect("an act that never ended")
    }

    /// The warm-up is not part of what the act cost.
    #[test]
    fn only_the_measured_frames_are_counted() {
        let mut driver = driver(50);
        let (what, did) = wind(&mut driver, 50);
        assert_eq!(what, Act::Still.what());
        assert_eq!(did, 50, "the warm-up was counted with the act");
    }

    /// The clock runs from the act's first frame, warm-up included, so an act
    /// that does something every fortieth frame does it at the same rate
    /// before and after the measurement starts.
    #[test]
    fn the_clock_never_stands_still() {
        let mut driver = driver(10);
        let mut ticks = Vec::new();
        for _ in 0..Driver::WARM + 10 {
            ticks.push(driver.tick);
            driver.drew();
        }
        assert_eq!(ticks[0], 0);
        assert_eq!(ticks[1], 1, "held still through the warm-up: {ticks:?}");
        assert_eq!(
            ticks.last(),
            Some(&(Driver::WARM + 9)),
            "the clock stopped somewhere: {ticks:?}"
        );
    }

    /// Every act is measured, once, in order, and then the run is over.
    #[test]
    fn a_run_is_every_act_and_then_the_end() {
        let mut driver = driver(5);
        let mut named = Vec::new();
        for _ in 0..Act::EVERY.len() {
            named.push(wind(&mut driver, 5).0);
        }
        let expected: Vec<&str> = Act::EVERY.iter().map(|act| act.what()).collect();
        assert_eq!(named, expected);
        assert!(driver.over(), "the run did not end");
        assert_eq!(driver.act(), None, "an act past the last of them");
        assert_eq!(driver.drew(), None, "it kept going after the end");
    }

    /// The first frame of an act is the one that puts the window back where
    /// every act starts, and no other frame is.
    #[test]
    fn only_an_acts_first_frame_is_a_fresh_one() {
        let mut driver = driver(5);
        assert!(driver.fresh(), "the very first frame");
        driver.drew();
        assert!(!driver.fresh(), "still fresh a frame later");
        // The rest of this act exactly, so the next frame is the next act's
        // first and nothing has been taken out of it.
        for _ in 0..Driver::WARM + 5 - 1 {
            driver.drew();
        }
        assert!(driver.fresh(), "the next act did not start fresh");
    }
}
