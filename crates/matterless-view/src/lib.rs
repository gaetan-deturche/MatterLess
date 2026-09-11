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
pub mod atlas;
pub mod badge;
pub mod clock;
pub mod composer;
#[cfg(test)]
mod composer_tests;
pub mod edit;
pub mod feed;
pub mod header;
pub mod listing;
pub mod live;
pub mod menu;
pub mod open;
pub mod picker;
pub mod profile;
pub mod rail;
pub mod scrollbar;
pub mod search;
pub mod sidebar;
pub mod sidebar_feed;
pub mod stream;
#[cfg(test)]
mod stream_tests;
pub mod switcher;
pub mod taskbar;
pub mod toast;
pub mod tooltip;
pub mod tray;
pub mod typing;
pub mod update;
pub mod updater_bar;

use atlas::{Atlas, SIDE};
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
    /// Which sampler this quad wants: 0 for a glyph, 1 for a picture. Carried
    /// per vertex because both kinds are in one draw call, and splitting the
    /// call by sampler would cost more state changes than a float does.
    pub filtered: f32,
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
            0 => Float32x2, 1 => Float32x2, 2 => Float32x4, 3 => Float32,
            4 => Float32x2, 5 => Float32x2, 6 => Float32, 7 => Float32,
        ],
    };
}

/// Casts the vertices to bytes for upload.
///
/// Hand-rolled rather than pulling in `bytemuck`: `Vertex` is plain floats with
/// no padding, so its layout is exactly what the shader declares.
fn as_bytes(vertices: &[Vertex]) -> &[u8] {
    // SAFETY: `Vertex` is `repr(C)` and contains only `f32`, so it has no
    // padding and no invalid bit patterns.
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
    let side = SIDE as f32;
    // Half a texel into the opaque corner, so no neighbour bleeds in.
    let solid_uv = [0.5 / side, 0.5 / side];
    for piece in pieces {
        match piece {
            // Nothing to draw: a press box is where a pointer may land, and
            // its words are already in the text piece beside it.
            Piece::Press { .. } => {}
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
                    0.0,
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
                let Some(slot) = atlas.image(key) else {
                    continue;
                };
                // Half a texel in on every side. Filtering at the exact edge of
                // a slot pulls in the gap between slots, which is transparent,
                // so every picture would be drawn with a faded border.
                quad(
                    into,
                    [*x, *y, *x + *width, *y + *height],
                    [
                        (slot.x as f32 + 0.5) / side,
                        (slot.y as f32 + 0.5) / side,
                        ((slot.x + slot.width) as f32 - 0.5) / side,
                        ((slot.y + slot.height) as f32 - 0.5) / side,
                    ],
                    // White, so the picture keeps its own colours.
                    [1.0, 1.0, 1.0, 1.0],
                    1.0,
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
                    // ignoring them puts every letter on its own baseline.
                    let x0 = (glyph.x + slot.left) as f32;
                    let y0 = (glyph.y - slot.top) as f32;
                    push_quad(
                        into,
                        [x0, y0, x0 + slot.width as f32, y0 + slot.height as f32],
                        [
                            slot.x as f32 / side,
                            slot.y as f32 / side,
                            (slot.x + slot.width) as f32 / side,
                            (slot.y + slot.height) as f32 / side,
                        ],
                        rgba,
                    );
                }
            }
        }
    }
}

/// A quad sampled point-for-point, which is what a glyph and a fill want.
fn push_quad(into: &mut Vec<Vertex>, rect: [f32; 4], uv: [f32; 4], colour: [f32; 4]) {
    quad(into, rect, uv, colour, 0.0, 0.0, 1.0);
}

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
    filtered: f32,
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
        filtered,
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

/// One layer's vertices and the rectangle they are clipped to.
type Span = (std::ops::Range<u32>, (f32, f32, f32, f32));

/// The GPU side of the list: a device, a pipeline, an atlas and a vertex buffer.
pub struct View {
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    pipeline: wgpu::RenderPipeline,
    bindings: wgpu::BindGroup,
    uniform: wgpu::Buffer,
    vertices: wgpu::Buffer,
    capacity: usize,
    pub atlas: Atlas,
    cache: SwashCache,
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
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let bindings = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("list"),
            layout: &layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: uniform.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&atlas.view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::Sampler(&smooth),
                },
            ],
        });
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
        }
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
        }
        self.queue.submit(Some(encoder.finish()));
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
