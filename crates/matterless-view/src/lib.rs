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

pub mod atlas;
pub mod feed;

use atlas::{Atlas, SIDE};
use cosmic_text::SwashCache;
use matterless_layout::Fonts;
use matterless_layout::row::{RowLayout, Theme};
use matterless_paint::{Palette, Piece};

/// One corner of a quad, in pixels and atlas coordinates.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct Vertex {
    pub position: [f32; 2],
    pub uv: [f32; 2],
    pub colour: [f32; 4],
}

impl Vertex {
    const LAYOUT: wgpu::VertexBufferLayout<'static> = wgpu::VertexBufferLayout {
        array_stride: std::mem::size_of::<Vertex>() as wgpu::BufferAddress,
        step_mode: wgpu::VertexStepMode::Vertex,
        attributes: &wgpu::vertex_attr_array![0 => Float32x2, 1 => Float32x2, 2 => Float32x4],
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
            Piece::Fill {
                x,
                y,
                width,
                height,
                colour,
            } => {
                let rgba = [
                    colour[0] as f32 / 255.0,
                    colour[1] as f32 / 255.0,
                    colour[2] as f32 / 255.0,
                    colour[3] as f32 / 255.0,
                ];
                push_quad(
                    into,
                    [*x, *y, *x + *width, *y + *height],
                    [solid_uv[0], solid_uv[1], solid_uv[0], solid_uv[1]],
                    rgba,
                );
            }
            Piece::Text { glyphs, ink } => {
                let rgba = [
                    ink[0] as f32 / 255.0,
                    ink[1] as f32 / 255.0,
                    ink[2] as f32 / 255.0,
                    1.0,
                ];
                for glyph in glyphs {
                    let Some(slot) = atlas.slot(queue, fonts, cache, glyph.key) else {
                        continue;
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

fn push_quad(into: &mut Vec<Vertex>, rect: [f32; 4], uv: [f32; 4], colour: [f32; 4]) {
    let [x0, y0, x1, y1] = rect;
    let [u0, v0, u1, v1] = uv;
    let corners = [
        Vertex {
            position: [x0, y0],
            uv: [u0, v0],
            colour,
        },
        Vertex {
            position: [x1, y0],
            uv: [u1, v0],
            colour,
        },
        Vertex {
            position: [x1, y1],
            uv: [u1, v1],
            colour,
        },
        Vertex {
            position: [x0, y0],
            uv: [u0, v0],
            colour,
        },
        Vertex {
            position: [x1, y1],
            uv: [u1, v1],
            colour,
        },
        Vertex {
            position: [x0, y1],
            uv: [u0, v1],
            colour,
        },
    ];
    into.extend_from_slice(&corners);
}

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
        // Nearest, not linear: a glyph rasterised at the size it is drawn does
        // not want filtering, which only blurs it.
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("atlas"),
            mag_filter: wgpu::FilterMode::Nearest,
            min_filter: wgpu::FilterMode::Nearest,
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
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::NonFiltering),
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

    /// Draws the rows that fall inside the viewport.
    ///
    /// Only those rows: the list is unbounded, and building geometry for
    /// forty thousand messages to show thirty of them is the same mistake
    /// virtualising the DOM was there to avoid.
    #[allow(clippy::too_many_arguments)]
    pub fn draw(
        &mut self,
        target: &wgpu::TextureView,
        fonts: &mut Fonts,
        painter: &mut matterless_paint::Painter,
        rows: &[RowLayout],
        scroll: f32,
        size: (u32, u32),
        theme: &Theme,
        palette: &Palette,
    ) {
        let viewport = Viewport {
            size: [size.0 as f32, size.1 as f32],
            scroll,
            padding: 0.0,
        };
        self.queue
            .write_buffer(&self.uniform, 0, viewport_bytes(&viewport));

        let mut quads: Vec<Vertex> = Vec::new();
        let mut top = 0.0_f32;
        let bottom = scroll + size.1 as f32;
        for row in rows {
            let height = row.height;
            if top + height >= scroll && top <= bottom {
                let pieces = painter.pieces_of(fonts, row, top, theme, palette);
                vertices_of(
                    &pieces,
                    &self.queue,
                    fonts,
                    &mut self.cache,
                    &mut self.atlas,
                    &mut quads,
                );
            }
            top += height;
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
                label: Some("list"),
            });
        {
            let ground = wgpu::Color {
                r: palette.ground[0] as f64 / 255.0,
                g: palette.ground[1] as f64 / 255.0,
                b: palette.ground[2] as f64 / 255.0,
                a: 1.0,
            };
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("list"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: target,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(ground),
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
                pass.draw(0..quads.len() as u32, 0..1);
            }
        }
        self.queue.submit(Some(encoder.finish()));
    }
}
