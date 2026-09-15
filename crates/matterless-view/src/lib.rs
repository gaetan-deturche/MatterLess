//! The message list, drawn on the GPU.
//!
//! Everything here consumes what the layout already decided: heights come from
//! `matterless-layout`, draw calls from `matterless-paint`. This turns that
//! draw list into vertices and hands it to Vulkan.
//!
//! The reason for it is not speed. It is that a row's height is known before it
//! is drawn, so there is nothing to measure afterwards and nothing to reconcile
//! -- which is what a browser cannot offer and what every scroll artefact in the
//! DOM list came from.

pub mod actions;
pub mod aside;
pub mod atlas;
pub mod badge;
pub mod clock;
pub mod composer;
#[cfg(test)]
mod composer_tests;
pub mod edit;
pub mod feed;
pub mod header;
pub mod identity;
pub mod listing;
pub mod live;
pub mod menu;
pub mod open;
pub mod picker;
pub mod profile;
pub mod rail;
pub mod rest;
pub mod scrollbar;
pub mod search;
pub mod sidebar;
pub mod sidebar_feed;
pub mod stream;
#[cfg(test)]
mod stream_tests;
pub mod switcher;
pub mod taskbar;
pub mod timing;
pub mod toast;
pub mod tooltip;
pub mod tray;
pub mod typing;
pub mod update;
pub mod updater_bar;
pub mod viewer;
pub mod whats_new;

use atlas::{Atlas, Sheet};
use cosmic_text::SwashCache;
use matterless_layout::Fonts;
use matterless_paint::Piece;

/// The format the frame is written through, which is never an sRGB one.
///
/// The palette is authored in sRGB -- the same hex the stylesheet uses -- and
/// both the browser and the CPU snapshot blend those bytes as they stand.
/// Handing them to an sRGB surface encodes them a second time: the ground
/// leaves the shader as 15/255 and lands as 69/255, which is how a near-black
/// panel turns slate grey while the light text barely moves. Drawing through
/// the plain view of the same surface keeps one encoding, stylesheet to screen.
pub fn plain(format: wgpu::TextureFormat) -> wgpu::TextureFormat {
    format.remove_srgb_suffix()
}

/// One corner of a quad, in pixels and atlas coordinates.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct Vertex {
    pub position: [f32; 2],
    pub uv: [f32; 2],
    pub colour: [f32; 4],
    /// Which sheet this quad samples, in the order `Sheet` lists them, with
    /// the opened picture's own texture after them.
    ///
    /// Carried per vertex because all four are bound at once: binding them in
    /// turn meant a draw call per switch, and a message row switches four
    /// times. It decides the sampler too -- the letters sheet is glyphs and
    /// the white texel, which want point sampling, and everything else is a
    /// picture scaled to a box, which wants linear.
    pub sheet: u32,
    /// Where this corner sits relative to the quad's middle, and half the
    /// quad's size. The fragment needs both to know where it is inside the
    /// rectangle, and per-vertex is how it gets there without a second buffer.
    pub local: [f32; 2],
    pub half_size: [f32; 2],
    /// How far the corners are cut. Zero is a square one, which is every
    /// glyph and most fills.
    pub radius: f32,
    /// How far the edge fades. One pixel is a crisp shape; twenty is a
    /// shadow, which is the same shape drawn soft.
    pub softness: f32,
}

impl Vertex {
    const LAYOUT: wgpu::VertexBufferLayout<'static> = wgpu::VertexBufferLayout {
        array_stride: std::mem::size_of::<Vertex>() as wgpu::BufferAddress,
        step_mode: wgpu::VertexStepMode::Vertex,
        attributes: &wgpu::vertex_attr_array![
            0 => Float32x2, 1 => Float32x2, 2 => Float32x4, 3 => Uint32,
            4 => Float32x2, 5 => Float32x2, 6 => Float32, 7 => Float32,
        ],
    };
}

/// Casts the vertices to bytes for upload.
///
/// Hand-rolled rather than pulling in `bytemuck`: `Vertex` is four-byte fields
/// with no padding, so its layout is exactly what the shader declares.
fn as_bytes(vertices: &[Vertex]) -> &[u8] {
    // SAFETY: `Vertex` is `repr(C)` and contains only `f32` and `u32`, so it
    // has no padding and no invalid bit patterns.
    unsafe {
        std::slice::from_raw_parts(
            vertices.as_ptr() as *const u8,
            std::mem::size_of_val(vertices),
        )
    }
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
struct Viewport {
    size: [f32; 2],
    scroll: f32,
    padding: f32,
}

fn viewport_bytes(viewport: &Viewport) -> &[u8] {
    // SAFETY: as above -- plain floats, `repr(C)`.
    unsafe {
        std::slice::from_raw_parts(
            (viewport as *const Viewport) as *const u8,
            std::mem::size_of::<Viewport>(),
        )
    }
}

/// Turns draw pieces into vertices.
///
/// Two triangles per quad, and a solid fill points at the atlas's opaque corner
/// texel so it goes through the same pipeline as a glyph.
pub fn vertices_of(
    pieces: &[Piece],
    queue: &wgpu::Queue,
    fonts: &mut Fonts,
    cache: &mut SwashCache,
    atlas: &mut Atlas,
    into: &mut Vec<Vertex>,
) {
    let letters = Sheet::Letters.size();
    let (wide, tall) = (letters.0 as f32, letters.1 as f32);
    // Half a texel into the opaque corner, so no neighbour bleeds in.
    let solid_uv = [0.5 / wide, 0.5 / tall];
    for piece in pieces {
        match piece {
            // Nothing to draw: a press box is where a pointer may land, and
            // its words are already in the text piece beside it.
            Piece::Press { .. } => {}
            // Drawn, but not from here: it samples a texture of its own rather
            // than the atlas, so it needs its own bind group and therefore its
            // own draw. `draw_scene` picks these out.
            Piece::Shown { .. } => {}
            Piece::Fill {
                x,
                y,
                width,
                height,
                colour,
                radius,
                softness,
            } => {
                let rgba = [
                    colour[0] as f32 / 255.0,
                    colour[1] as f32 / 255.0,
                    colour[2] as f32 / 255.0,
                    colour[3] as f32 / 255.0,
                ];
                quad(
                    into,
                    [*x, *y, *x + *width, *y + *height],
                    [solid_uv[0], solid_uv[1], solid_uv[0], solid_uv[1]],
                    rgba,
                    // The white texel it samples is on the letters sheet.
                    Sheet::Letters as u32,
                    *radius,
                    *softness,
                );
            }
            Piece::Image {
                x,
                y,
                width,
                height,
                key,
                radius,
            } => {
                // Nothing until the bytes have arrived. The layout already
                // reserved the room, so an absent picture leaves a gap rather
                // than a conversation that shifts when it lands.
                let Some((sheet, slot)) = atlas.image(key) else {
                    continue;
                };
                let (wide, tall) = sheet.size();
                let (wide, tall) = (wide as f32, tall as f32);
                // Half a texel in on every side. Filtering at the exact edge of
                // a slot pulls in the gap between slots, which is transparent,
                // so every picture would be drawn with a faded border.
                quad(
                    into,
                    [*x, *y, *x + *width, *y + *height],
                    [
                        (slot.x as f32 + 0.5) / wide,
                        (slot.y as f32 + 0.5) / tall,
                        ((slot.x + slot.width) as f32 - 0.5) / wide,
                        ((slot.y + slot.height) as f32 - 0.5) / tall,
                    ],
                    // White, so the picture keeps its own colours.
                    [1.0, 1.0, 1.0, 1.0],
                    sheet as u32,
                    *radius,
                    1.0,
                );
            }
            Piece::Text {
                glyphs,
                ink,
                faint,
                signal,
            } => {
                let shade = |colour: &[u8; 3]| {
                    [
                        colour[0] as f32 / 255.0,
                        colour[1] as f32 / 255.0,
                        colour[2] as f32 / 255.0,
                        1.0,
                    ]
                };
                let (loud, quiet, followed) = (shade(ink), shade(faint), shade(signal));
                for glyph in glyphs {
                    let Some(slot) = atlas.slot(queue, fonts, cache, glyph.key) else {
                        continue;
                    };
                    // White for a glyph that brought its own colour, so the
                    // shader's tint leaves it as it is.
                    let rgba = if slot.colour {
                        [1.0, 1.0, 1.0, 1.0]
                    } else {
                        match glyph.shade {
                            matterless_paint::Shade::Ink => loud,
                            matterless_paint::Shade::Faint => quiet,
                            matterless_paint::Shade::Signal => followed,
                        }
                    };
                    // `left` and `top` are the bitmap's offset from the pen, and
                    // ignoring them puts every letter on its own baseline. They
                    // scale with it: a mark rasterised at twice has twice the
                    // bearing too.
                    let x0 = glyph.x as f32 + slot.left as f32 * glyph.scale;
                    let y0 = glyph.y as f32 - slot.top as f32 * glyph.scale;
                    let (drawn, tall_as) = (
                        slot.width as f32 * glyph.scale,
                        slot.height as f32 * glyph.scale,
                    );
                    quad(
                        into,
                        [x0, y0, x0 + drawn, y0 + tall_as],
                        [
                            slot.x as f32 / wide,
                            slot.y as f32 / tall,
                            (slot.x + slot.width) as f32 / wide,
                            (slot.y + slot.height) as f32 / tall,
                        ],
                        rgba,
                        // A letter is one texel to one pixel and wants the
                        // point sampler; a mark drawn down from twice its size
                        // is the one thing on this sheet that wants the other.
                        if glyph.scale < 1.0 {
                            SMOOTH_LETTERS
                        } else {
                            Sheet::Letters as u32
                        },
                        0.0,
                        1.0,
                    );
                }
            }
        }
    }
}

/// A quad sampled point-for-point, which is what a glyph and a fill want.
/// Six vertices for one rectangle.
///
/// Each corner carries where it sits relative to the middle and how big half
/// the rectangle is, so the fragment can measure its own distance to a rounded
/// edge. That is three floats a vertex to avoid a second buffer and a second
/// bind group for something every quad already knows about itself.
fn quad(
    into: &mut Vec<Vertex>,
    rect: [f32; 4],
    uv: [f32; 4],
    colour: [f32; 4],
    sheet: u32,
    radius: f32,
    softness: f32,
) {
    let [x0, y0, x1, y1] = rect;
    let [u0, v0, u1, v1] = uv;
    let half_size = [(x1 - x0) / 2.0, (y1 - y0) / 2.0];
    // Never more than half the shorter side: past that the corners meet and
    // the distance stops describing a rectangle at all.
    let radius = radius.min(half_size[0].min(half_size[1])).max(0.0);
    let corner = |x: f32, y: f32, u: f32, v: f32| Vertex {
        position: [x, y],
        uv: [u, v],
        colour,
        sheet,
        local: [x - (x0 + half_size[0]), y - (y0 + half_size[1])],
        half_size,
        radius,
        softness,
    };
    into.extend([
        corner(x0, y0, u0, v0),
        corner(x1, y0, u1, v0),
        corner(x1, y1, u1, v1),
        corner(x0, y0, u0, v0),
        corner(x1, y1, u1, v1),
        corner(x0, y1, u0, v1),
    ]);
}

/// The rectangle a run is clipped to.
type Clip = (f32, f32, f32, f32);

/// Where the opened picture's own texture sits, after the three sheets.
const SHOWN: u32 = 3;

/// The letters sheet again, sampled smooth rather than point.
///
/// Not a fourth texture: the same one, read the other way. Everything on it is
/// one texel to one pixel except a mark that was rasterised at twice its size
/// so it could be drawn down, and that one wants filtering.
const SMOOTH_LETTERS: u32 = 4;

/// Binds everything the pipeline samples: the two samplers, the three sheets,
/// and whatever is open.
///
/// One function because it is called three times -- at startup, when a picture
/// is opened, and when it is closed -- and three copies of a seven-entry
/// descriptor is three places for an entry to go to the wrong binding.
#[allow(clippy::too_many_arguments)]
fn bind_all(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    uniform: &wgpu::Buffer,
    sampler: &wgpu::Sampler,
    smooth: &wgpu::Sampler,
    atlas: &Atlas,
    shown: &wgpu::TextureView,
) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("list"),
        layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::Sampler(sampler),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: wgpu::BindingResource::Sampler(smooth),
            },
            wgpu::BindGroupEntry {
                binding: 3,
                resource: wgpu::BindingResource::TextureView(atlas.view(Sheet::Letters)),
            },
            wgpu::BindGroupEntry {
                binding: 4,
                resource: wgpu::BindingResource::TextureView(atlas.view(Sheet::Faces)),
            },
            wgpu::BindGroupEntry {
                binding: 5,
                resource: wgpu::BindingResource::TextureView(atlas.view(Sheet::Pictures)),
            },
            wgpu::BindGroupEntry {
                binding: 6,
                resource: wgpu::BindingResource::TextureView(shown),
            },
        ],
    })
}

/// One layer's vertices and the rectangle they are clipped to.
///
/// A layer is one draw. Every sheet is bound at once and each quad names the
/// one it samples, so nothing inside a layer needs splitting -- only the
/// scissor changes, and that is what a layer is.
type Span = (std::ops::Range<u32>, Clip);

/// The GPU side of the list: a device, a pipeline, an atlas and a vertex buffer.
pub struct View {
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    pipeline: wgpu::RenderPipeline,
    /// Every texture at once. Bound once a frame, whatever is on screen.
    bindings: wgpu::BindGroup,
    uniform: wgpu::Buffer,
    vertices: wgpu::Buffer,
    capacity: usize,
    pub atlas: Atlas,
    cache: SwashCache,
    /// What the pipeline binds against, kept so a second bind group can be
    /// built for the picture being looked at.
    layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    smooth: wgpu::Sampler,
    /// The picture a reader has opened, in a texture of its own.
    ///
    /// One at a time, replaced when another is opened and dropped when the
    /// viewer shuts. Not in a sheet: they are packed for things drawn at the
    /// size they were fetched, and a picture opened full size is neither.
    shown: Option<wgpu::TextureView>,
    /// What stands in the fourth binding while nothing is open.
    nothing: wgpu::TextureView,
}

impl View {
    /// Builds the pipeline against an already-configured surface format.
    pub fn new(device: wgpu::Device, queue: wgpu::Queue, format: wgpu::TextureFormat) -> Self {
        let atlas = Atlas::new(&device, &queue);
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("list"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shader.wgsl").into()),
        });
        let uniform = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("viewport"),
            size: std::mem::size_of::<Viewport>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        // Two samplers, because the atlas holds two kinds of thing. A glyph is
        // rasterised at the size it is drawn, so filtering it only blurs it. A
        // picture is scaled to whatever box the layout reserved, and point
        // sampling that drops whole rows of pixels -- which is what makes a
        // downscaled screenshot look shattered.
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("atlas"),
            mag_filter: wgpu::FilterMode::Nearest,
            min_filter: wgpu::FilterMode::Nearest,
            ..Default::default()
        });
        let smooth = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("pictures"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            // Clamped, so sampling the edge of a slot cannot wrap round to the
            // other side of the atlas.
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            ..Default::default()
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("list"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                // The three sheets and the opened picture, all bound at once:
                // four textures is well inside any device's limit, and a quad
                // that names the one it wants costs nothing where binding them
                // in turn cost a draw call per switch.
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 4,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 5,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 6,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
            ],
        });
        // Something for the fourth texture to be while nothing is open. A
        // binding cannot be left empty, and one texel is cheaper to keep than
        // a second pipeline for the frames with no picture in them.
        let nothing = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("nothing shown"),
            size: wgpu::Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let nothing = nothing.create_view(&wgpu::TextureViewDescriptor::default());
        let bindings = bind_all(
            &device, &layout, &uniform, &sampler, &smooth, &atlas, &nothing,
        );
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("list"),
            bind_group_layouts: &[&layout],
            push_constant_ranges: &[],
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("list"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vertex"),
                buffers: &[Vertex::LAYOUT],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fragment"),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    // Straight alpha: the coverage decides how much of the ink
                    // lands, which is what antialiased text is.
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
            cache: None,
        });
        let vertices = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("quads"),
            size: 4096 * std::mem::size_of::<Vertex>() as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        Self {
            device,
            queue,
            pipeline,
            bindings,
            uniform,
            vertices,
            capacity: 4096,
            atlas,
            cache: SwashCache::new(),
            layout,
            sampler,
            smooth,
            shown: None,
            nothing,
        }
    }

    /// Puts the picture a reader has opened into a texture of its own.
    ///
    /// Replaces whatever was there: only one is ever looked at, so the last
    /// one's texture is dropped here rather than accumulating. `rgba` is
    /// `width * height * 4` bytes, already scaled to something the window can
    /// draw -- the decoder does that, because the window knows how big it is
    /// and the GPU has a limit this must stay under either way.
    pub fn show(&mut self, width: u32, height: u32, rgba: &[u8]) {
        let side = self.device.limits().max_texture_dimension_2d;
        if width == 0 || height == 0 || width > side || height > side {
            eprintln!("a {width}x{height} picture is not one this device can hold");
            self.shown = None;
            return;
        }
        let texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("shown"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            // Plain, exactly as the sheets are: these bytes are already sRGB
            // and the frame is written without a second encoding, so sampling
            // must not decode. Declaring this one sRGB decoded it on the way
            // out and the same picture came back paler full size than it was
            // in the message it was opened from.
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        self.queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            rgba,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(width * 4),
                rows_per_image: Some(height),
            },
            wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
        );
        self.shown = Some(texture.create_view(&wgpu::TextureViewDescriptor::default()));
        self.rebind();
    }

    /// Binds everything again, which is what a new or dropped picture needs:
    /// the fourth texture is part of the same group as the three sheets.
    fn rebind(&mut self) {
        let shown = self.shown.as_ref().unwrap_or(&self.nothing);
        self.bindings = bind_all(
            &self.device,
            &self.layout,
            &self.uniform,
            &self.sampler,
            &self.smooth,
            &self.atlas,
            shown,
        );
    }

    /// Lets go of it. The texture is freed with the bind group holding it.
    pub fn stop_showing(&mut self) {
        self.shown = None;
        self.rebind();
    }

    /// Draws a whole frame: every panel, each clipped to its own rectangle.
    ///
    /// One pass per layer, because the scissor is set per draw. A window is a
    /// handful of panels, so that is a handful of draws -- and the alternative,
    /// clipping each shape on the CPU, would mean rebuilding geometry whenever
    /// a panel moved.
    pub fn draw_scene(
        &mut self,
        target: &wgpu::TextureView,
        fonts: &mut Fonts,
        scene: &matterless_paint::Scene,
        size: (u32, u32),
        ground: [u8; 4],
    ) {
        let viewport = Viewport {
            size: [size.0 as f32, size.1 as f32],
            // Scrolling is baked into the positions by whoever built the scene;
            // a panel that scrolls is not the renderer's business.
            scroll: 0.0,
            padding: 0.0,
        };
        self.queue
            .write_buffer(&self.uniform, 0, viewport_bytes(&viewport));

        // One buffer for the frame, with each layer's span remembered.
        let mut quads: Vec<Vertex> = Vec::new();
        let mut spans: Vec<Span> = Vec::new();
        // And the one picture that samples its own texture, kept apart because
        // it needs the other bind group. Last in the buffer and last in the
        // pass: it is an overlay over the whole window, so there is nothing it
        // should be drawn under.
        let mut shown: Vec<Span> = Vec::new();
        for layer in &scene.layers {
            let from = quads.len() as u32;
            vertices_of(
                &layer.pieces,
                &self.queue,
                fonts,
                &mut self.cache,
                &mut self.atlas,
                &mut quads,
            );
            let to = quads.len() as u32;
            if to > from {
                spans.push((from..to, layer.clip));
            }
            for piece in &layer.pieces {
                let Piece::Shown {
                    x,
                    y,
                    width,
                    height,
                } = piece
                else {
                    continue;
                };
                let from = quads.len() as u32;
                quad(
                    &mut quads,
                    [*x, *y, *x + *width, *y + *height],
                    [0.0, 0.0, 1.0, 1.0],
                    // White, so the picture keeps its own colours.
                    [1.0, 1.0, 1.0, 1.0],
                    // Its own texture, after the three sheets: a picture
                    // opened full size is far too large to share one.
                    SHOWN,
                    0.0,
                    1.0,
                );
                shown.push((from..quads.len() as u32, layer.clip));
            }
        }

        if quads.len() > self.capacity {
            self.capacity = quads.len().next_power_of_two();
            self.vertices = self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("quads"),
                size: (self.capacity * std::mem::size_of::<Vertex>()) as u64,
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
        }
        if !quads.is_empty() {
            self.queue.write_buffer(&self.vertices, 0, as_bytes(&quads));
        }

        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("frame"),
            });
        {
            let clear = wgpu::Color {
                r: ground[0] as f64 / 255.0,
                g: ground[1] as f64 / 255.0,
                b: ground[2] as f64 / 255.0,
                a: 1.0,
            };
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("frame"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: target,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(clear),
                        store: wgpu::StoreOp::Store,
                    },
                    depth_slice: None,
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            if !quads.is_empty() {
                pass.set_pipeline(&self.pipeline);
                // Once for the frame. Every texture is in this group and each
                // quad names the one it samples, so the only thing left that
                // splits a frame into draws is the scissor.
                pass.set_bind_group(0, &self.bindings, &[]);
                pass.set_vertex_buffer(0, self.vertices.slice(..));
                for (range, clip) in spans {
                    // Clamped to the surface: a scissor outside it is a
                    // validation error, and a panel can be dragged past the edge.
                    let x = clip.0.max(0.0).min(size.0 as f32) as u32;
                    let y = clip.1.max(0.0).min(size.1 as f32) as u32;
                    let width = (clip.2.min(size.0 as f32 - x as f32)).max(0.0) as u32;
                    let height = (clip.3.min(size.1 as f32 - y as f32)).max(0.0) as u32;
                    if width == 0 || height == 0 {
                        continue;
                    }
                    pass.set_scissor_rect(x, y, width, height);
                    pass.draw(range, 0..1);
                }
            }
            // The opened picture. Its own draw only because it is built after
            // the rest of the frame, not because it needs a different binding:
            // it names the fourth texture like anything else names a sheet.
            if self.shown.is_some() && !shown.is_empty() {
                pass.set_pipeline(&self.pipeline);
                pass.set_bind_group(0, &self.bindings, &[]);
                pass.set_vertex_buffer(0, self.vertices.slice(..));
                for (range, clip) in shown {
                    let x = clip.0.max(0.0).min(size.0 as f32) as u32;
                    let y = clip.1.max(0.0).min(size.1 as f32) as u32;
                    let width = (clip.2.min(size.0 as f32 - x as f32)).max(0.0) as u32;
                    let height = (clip.3.min(size.1 as f32 - y as f32)).max(0.0) as u32;
                    if width == 0 || height == 0 {
                        continue;
                    }
                    pass.set_scissor_rect(x, y, width, height);
                    pass.draw(range, 0..1);
                }
            }
        }
        self.queue.submit(Some(encoder.finish()));
        // A frame has been drawn, which is what the atlas ages its pictures by:
        // everything on screen was just asked for, so anything that was not is
        // a frame older than the things it is competing with for room.
        self.atlas.drew();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The invariant the whole palette rests on: nothing is encoded twice.
    #[test]
    fn the_frame_is_never_written_through_an_srgb_view() {
        for format in [
            wgpu::TextureFormat::Bgra8UnormSrgb,
            wgpu::TextureFormat::Rgba8UnormSrgb,
            wgpu::TextureFormat::Bgra8Unorm,
            wgpu::TextureFormat::Rgba8Unorm,
        ] {
            assert!(!plain(format).is_srgb(), "{format:?} stayed sRGB");
        }
    }

    /// The plain view has to be the same texture read differently, or the
    /// surface will not accept it.
    #[test]
    fn a_plain_format_is_left_alone() {
        assert_eq!(
            plain(wgpu::TextureFormat::Bgra8UnormSrgb),
            wgpu::TextureFormat::Bgra8Unorm
        );
        assert_eq!(
            plain(wgpu::TextureFormat::Bgra8Unorm),
            wgpu::TextureFormat::Bgra8Unorm
        );
    }
}
