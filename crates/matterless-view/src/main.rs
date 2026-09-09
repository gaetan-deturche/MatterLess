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
use matterless_ui::input::{Event as UiEvent, Input};
// Aliased: `Node` is a markdown node in this file already, and a box here.
use matterless_render::markdown::Node;
use matterless_render::{PostRow, Row};
use matterless_ui::{Axis, Node as Boxed, Placed, Rect, Size};
use matterless_view::sidebar::{Canvas, Entry, Sidebar};
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

struct App {
    window: Option<Arc<Window>>,
    surface: Option<wgpu::Surface<'static>>,
    view: Option<matterless_view::View>,
    format: wgpu::TextureFormat,
    size: (u32, u32),
    fonts: Fonts,
    painter: Painter,
    rows: Vec<Row>,
    laid: Vec<RowLayout>,
    theme: Theme,
    palette: Palette,
    scroll: f32,
    /// The store, kept open: switching channel is a read, not a reload.
    store: Option<matterless_store::Store>,
    sidebar: Sidebar,
    input: Input,
    placed: Vec<Placed>,
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
            .and_then(|path| matterless_view::feed::open(&path).ok());
        let listed = store
            .as_ref()
            .map(|store| matterless_view::feed::channels(store, ""))
            .unwrap_or_default();
        let sidebar = Sidebar::new(
            listed
                .into_iter()
                .map(|channel| Entry::Channel {
                    id: channel.id,
                    label: channel.label,
                    unread: channel.unread,
                    mentions: channel.mentions,
                    muted: channel.muted,
                })
                .collect(),
        );
        let (channel, rows) = Self::feed();
        let mut app = Self {
            window: None,
            surface: None,
            view: None,
            format: wgpu::TextureFormat::Bgra8UnormSrgb,
            size: (1000, 760),
            fonts: Fonts::new(),
            painter: Painter::new(),
            rows,
            laid: Vec::new(),
            theme: Theme::default(),
            palette: Palette::default(),
            scroll: 0.0,
            store,
            sidebar,
            input: Input::default(),
            placed: Vec::new(),
        };
        app.sidebar.selected = Some(channel);
        app
    }

    /// The window as boxes: a fixed sidebar, and the stream taking the rest.
    fn shell(&self) -> Vec<Placed> {
        let tree = Boxed::new("shell", Size::Grow(1.0))
            .axis(Axis::Row)
            .with(Boxed::new("sidebar-panel", Size::Fixed(SIDEBAR)))
            .with(Boxed::new("stream", Size::Grow(1.0)));
        matterless_ui::solve::solve(
            &tree,
            Rect::new(0.0, 0.0, self.size.0 as f32, self.size.1 as f32),
        )
    }

    fn sidebar_rect(&self) -> Rect {
        Rect::new(0.0, 0.0, SIDEBAR, self.size.1 as f32)
    }

    fn stream_rect(&self) -> Rect {
        Rect::new(
            SIDEBAR,
            0.0,
            (self.size.0 as f32 - SIDEBAR).max(0.0),
            self.size.1 as f32,
        )
    }

    fn redraw(&self) {
        if let Some(window) = &self.window {
            window.request_redraw();
        }
    }

    /// Reads a channel and lays it out, then shows its newest message.
    fn open_channel(&mut self, channel: &str) {
        let Some(store) = self.store.as_ref() else {
            return;
        };
        match matterless_view::feed::rows_of(store, channel, "") {
            Ok(rows) => {
                self.rows = rows;
                self.relayout();
                let total: f32 = self.laid.iter().map(|row| row.height).sum();
                self.scroll = (total - self.stream_rect().height).max(0.0);
            }
            Err(why) => eprintln!("{channel}: {why}"),
        }
    }

    /// Everything the frame draws, in one scene.
    fn scene(&mut self) -> Scene {
        let mut scene = Scene::default();
        let sidebar = self.sidebar_rect();
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

        scene.clip_to(stream.x, stream.y, stream.width, stream.height);
        let mut top = stream.y - self.scroll;
        for row in &self.laid {
            let bottom = top + row.height;
            if bottom >= stream.y && top <= stream.bottom() {
                let pieces =
                    self.painter
                        .pieces_of(&mut self.fonts, row, top, &self.theme, &self.palette);
                // Shifted into the stream's column: the row plan is laid out
                // from zero and knows nothing of the panel it lands in.
                scene.extend(pieces.into_iter().map(|piece| shift(piece, stream.x)));
            }
            top = bottom;
        }
        scene
    }

    /// Lays the whole conversation out for the current width.
    ///
    /// Once per width change, never per frame: the heights do not depend on the
    /// scroll position, which is the property that makes this list honest.
    fn relayout(&mut self) {
        self.theme = Theme {
            width: self.stream_rect().width,
            ..Theme::default()
        };
        self.laid = self
            .rows
            .iter()
            .map(|row| lay_out(&mut self.fonts, row, &self.theme))
            .collect();
        let total: f32 = self.laid.iter().map(|row| row.height).sum();
        let reach = (total - self.size.1 as f32).max(0.0);
        self.scroll = self.scroll.clamp(0.0, reach);
    }
}

impl ApplicationHandler for App {
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
                view_formats: Vec::new(),
                desired_maximum_frame_latency: 2,
            },
        );

        println!(
            "adapter: {} ({:?})",
            adapter.get_info().name,
            adapter.get_info().backend
        );
        self.view = Some(matterless_view::View::new(device, queue, self.format));
        self.surface = Some(surface);
        self.window = Some(window);
        self.relayout();
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
                            view_formats: Vec::new(),
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
                self.placed = self.shell();
                let mut boxes = self.placed.clone();
                boxes.extend(self.sidebar.boxes(self.sidebar_rect()));
                self.input.apply(
                    UiEvent::PointerMoved {
                        x: position.x as f32,
                        y: position.y as f32,
                    },
                    &boxes,
                );
                self.redraw();
            }
            WindowEvent::CursorLeft { .. } => {
                self.input.apply(UiEvent::PointerLeft, &[]);
                self.redraw();
            }
            WindowEvent::MouseInput { state, button, .. } => {
                if button != winit::event::MouseButton::Left {
                    return;
                }
                let boxes = self.sidebar.boxes(self.sidebar_rect());
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
                self.redraw();
            }
            WindowEvent::MouseWheel { delta, .. } => {
                let by = match delta {
                    MouseScrollDelta::LineDelta(_, lines) => lines * self.theme.line_height * 3.0,
                    MouseScrollDelta::PixelDelta(position) => position.y as f32,
                };
                let boxes = self.sidebar.boxes(self.sidebar_rect());
                self.input.apply(UiEvent::Wheel { x: 0.0, y: by }, &boxes);
                let within = self.sidebar_rect();
                // The sidebar takes it when the pointer is over the sidebar;
                // otherwise the stream does. One wheel, two panels, and the
                // pointer decides -- which is what `wheel_over` is for.
                let over_sidebar = self
                    .input
                    .wheel_over(&boxes, |name| name == "sidebar")
                    .is_some();
                if over_sidebar {
                    self.sidebar.react(&self.input, &boxes, within);
                } else {
                    let total: f32 = self.laid.iter().map(|row| row.height).sum();
                    let reach = (total - self.stream_rect().height).max(0.0);
                    self.scroll = (self.scroll - by).clamp(0.0, reach);
                }
                self.redraw();
            }
            WindowEvent::RedrawRequested => {
                if self.surface.is_none() || self.view.is_none() {
                    return;
                }
                let scene = self.scene();
                let size = self.size;
                let ground = self.palette.ground;
                let (Some(surface), Some(view)) = (&self.surface, &mut self.view) else {
                    return;
                };
                let Ok(frame) = surface.get_current_texture() else {
                    return;
                };
                let target = frame
                    .texture
                    .create_view(&wgpu::TextureViewDescriptor::default());
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

/// Moves a piece sideways into its panel.
fn shift(piece: matterless_paint::Piece, by: f32) -> matterless_paint::Piece {
    use matterless_paint::Piece;
    match piece {
        Piece::Fill {
            x,
            y,
            width,
            height,
            colour,
        } => Piece::Fill {
            x: x + by,
            y,
            width,
            height,
            colour,
        },
        Piece::Text { glyphs, ink, faint } => Piece::Text {
            glyphs: glyphs
                .into_iter()
                .map(|glyph| matterless_paint::PlacedGlyph {
                    x: glyph.x + by as i32,
                    ..glyph
                })
                .collect(),
            ink,
            faint,
        },
    }
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

    let events = EventLoop::new().expect("an event loop");
    events.set_control_flow(ControlFlow::Wait);
    events.run_app(&mut App::new()).expect("the event loop");
}
