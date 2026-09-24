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
pub mod clip;
pub mod clock;
pub mod composer;
#[cfg(test)]
mod composer_tests;
pub mod d3d;
pub mod d3d_draw;
pub mod edit;
pub mod feed;
pub mod filecache;
pub mod glide;
pub mod header;
pub mod identity;
pub mod listing;
pub mod live;
pub mod menu;
pub mod moving;
pub mod offer;
pub mod open;
pub mod pack;
pub mod picker;
pub mod places;
pub mod profile;
pub mod rail;
pub mod rest;
pub mod scrollbar;
pub mod search;
pub mod settings;
pub mod sidebar;
pub mod sidebar_feed;
pub mod signin;
pub mod spell;
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

use crate::d3d::Gpu;
use crate::d3d_draw::{Bound, Viewport};
use atlas::{Atlas, Sheet};
use cosmic_text::SwashCache;
use matterless_layout::Fonts;
use matterless_paint::Piece;
use windows::Win32::Foundation::{HWND, RECT};
use windows::Win32::Graphics::Direct3D::D3D11_PRIMITIVE_TOPOLOGY_TRIANGLELIST;
use windows::Win32::Graphics::Direct3D11::*;
use windows::Win32::Graphics::Dxgi::Common::{DXGI_FORMAT_R8G8B8A8_UNORM, DXGI_SAMPLE_DESC};
use windows::Win32::Graphics::Dxgi::DXGI_PRESENT;

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
    gpu: &Gpu,
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
            Piece::Fade {
                x,
                y,
                width,
                height,
                colour,
                solid,
            } => {
                let rgba = [
                    colour[0] as f32 / 255.0,
                    colour[1] as f32 / 255.0,
                    colour[2] as f32 / 255.0,
                    colour[3] as f32 / 255.0,
                ];
                // Straight alpha, so the see-through end keeps the colour and
                // only loses its opacity. Blended the other way it would fade
                // towards black, which on this palette is nearly the ground
                // and so looks right until it is drawn over a picture.
                let gone = [rgba[0], rgba[1], rgba[2], 0.0];
                let (top, bottom) = match solid {
                    matterless_paint::Solid::Top => (rgba, gone),
                    matterless_paint::Solid::Bottom => (gone, rgba),
                };
                fading(
                    into,
                    [*x, *y, *x + *width, *y + *height],
                    [solid_uv[0], solid_uv[1], solid_uv[0], solid_uv[1]],
                    top,
                    bottom,
                    Sheet::Letters as u32,
                    0.0,
                    1.0,
                );
            }
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
                    // A mark is rasterised at twice the size it is drawn and
                    // filtered down, and a third of a pixel does not survive
                    // being halved -- so those stay flat whatever the setting.
                    let banded = atlas.subpixel && glyph.scale >= 1.0;
                    let Some(slot) = atlas.slot(gpu, fonts, cache, glyph.key, banded) else {
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
                        } else if slot.subpixel {
                            SUBPIXEL_LETTERS
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
    fading(into, rect, uv, colour, colour, sheet, radius, softness);
}

/// The same, with a colour at the top and another at the bottom.
///
/// A gradient costs nothing here: the fragment already interpolates whatever
/// the corners carry, so a fade is one quad rather than a stack of strips or a
/// texture. Used where a cut code block trails off into its own ground.
#[allow(clippy::too_many_arguments)]
fn fading(
    into: &mut Vec<Vertex>,
    rect: [f32; 4],
    uv: [f32; 4],
    top: [f32; 4],
    bottom: [f32; 4],
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
        colour: match y <= y0 {
            true => top,
            false => bottom,
        },
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

/// The same sheet again, holding a channel of coverage in each of the three
/// rather than white with one coverage in the alpha.
///
/// Its own index because the fragment cannot tell the two apart by looking:
/// both are four bytes in the letters sheet, and a subpixel mask read as a
/// white-plus-alpha one is a white letter with the wrong edges.
const SUBPIXEL_LETTERS: u32 = 5;

/// One layer's vertices and the rectangle they are clipped to.
///
/// A layer is one draw. Every sheet is bound at once and each quad names the
/// one it samples, so nothing inside a layer needs splitting -- only the
/// scissor changes, and that is what a layer is.
type Span = (std::ops::Range<u32>, Clip);

/// The GPU side of the list: a device, a swapchain, a pipeline and an atlas.
pub struct View {
    pub gpu: Gpu,
    bound: Bound,
    pub atlas: Atlas,
    cache: SwashCache,
    vertices: Option<ID3D11Buffer>,
    capacity: usize,
    /// The picture a reader has opened, in a texture of its own.
    ///
    /// One at a time, replaced when another is opened and dropped when the
    /// viewer shuts. Not in a sheet: they are packed for things drawn at the
    /// size they were fetched, and a picture opened full size is neither.
    shown: Option<ID3D11ShaderResourceView>,
    /// What stands in the fourth slot while nothing is open. A slot left empty
    /// is a shader reading from nothing, which draws a black rectangle.
    nothing: ID3D11ShaderResourceView,
}

impl View {
    /// Opens Direct3D for a window and builds everything that draws into it.
    pub fn new(window: HWND, size: (u32, u32)) -> Result<Self, String> {
        let gpu = Gpu::new(window, size)?;
        let bound = Bound::new(&gpu)?;
        let atlas = Atlas::new(&gpu);
        let nothing = one_texel(&gpu)?;
        Ok(Self {
            gpu,
            bound,
            atlas,
            cache: SwashCache::new(),
            vertices: None,
            capacity: 0,
            shown: None,
            nothing,
        })
    }

    /// Builds the buffers again for a new size.
    pub fn resize(&mut self, size: (u32, u32)) -> Result<(), String> {
        self.gpu.resize(size)
    }

    /// Puts a picture that has arrived into the sheets.
    ///
    /// Here rather than on the atlas so the caller does not have to hold the
    /// device as well: everything about the GPU is this view's.
    pub fn put_image(
        &mut self,
        key: &str,
        rgba: &[u8],
        width: u32,
        height: u32,
    ) -> Option<atlas::Slot> {
        self.atlas.put_image(&self.gpu, key, rgba, width, height)
    }

    /// The next frame of a picture that moves, over the one showing.
    pub fn put_frame(&mut self, key: &str, rgba: &[u8], width: u32, height: u32) -> bool {
        self.atlas.put_frame(&self.gpu, key, rgba, width, height)
    }

    /// Puts a picture in a texture of its own, for the viewer.
    pub fn show(&mut self, width: u32, height: u32, rgba: &[u8]) {
        let wanted = (width as usize) * (height as usize) * 4;
        if rgba.len() < wanted || width == 0 || height == 0 {
            return;
        }
        let how = D3D11_TEXTURE2D_DESC {
            Width: width,
            Height: height,
            MipLevels: 1,
            ArraySize: 1,
            Format: DXGI_FORMAT_R8G8B8A8_UNORM,
            SampleDesc: DXGI_SAMPLE_DESC {
                Count: 1,
                Quality: 0,
            },
            Usage: D3D11_USAGE_DEFAULT,
            BindFlags: D3D11_BIND_SHADER_RESOURCE.0 as u32,
            ..Default::default()
        };
        // Given its pixels as it is made, since they are all there already.
        let first = D3D11_SUBRESOURCE_DATA {
            pSysMem: rgba.as_ptr() as *const _,
            SysMemPitch: width * 4,
            SysMemSlicePitch: 0,
        };
        let mut texture: Option<ID3D11Texture2D> = None;
        if unsafe {
            self.gpu
                .device
                .CreateTexture2D(&how, Some(&first), Some(&mut texture))
        }
        .is_err()
        {
            return;
        }
        let Some(texture) = texture else { return };
        let mut view: Option<ID3D11ShaderResourceView> = None;
        if unsafe {
            self.gpu
                .device
                .CreateShaderResourceView(&texture, None, Some(&mut view))
        }
        .is_err()
        {
            return;
        }
        self.shown = view;
    }

    pub fn stop_showing(&mut self) {
        self.shown = None;
    }

    /// Draws one frame and presents it.
    ///
    /// `waiting` is whether to hold the frame back until the screen is ready
    /// for it. Every frame does, bar one: a frame drawn from inside a resize,
    /// where the window itself is waiting on this to come back and a
    /// sixtieth of a second spent watching for the vertical blank is a
    /// sixtieth the edge spends behind the pointer.
    pub fn draw_scene(
        &mut self,
        fonts: &mut Fonts,
        scene: &matterless_paint::Scene,
        size: (u32, u32),
        ground: [u8; 4],
        waiting: bool,
    ) {
        let Some(target) = self.gpu.target.clone() else {
            return;
        };
        // One buffer for the frame, with each layer's span remembered.
        let mut quads: Vec<Vertex> = Vec::new();
        let mut spans: Vec<Span> = Vec::new();
        // And the one picture that samples its own texture, kept apart because
        // it is built after the rest of the frame. Last in the buffer and last
        // in the pass: it is an overlay over the whole window, so there is
        // nothing it should be drawn under.
        let mut shown: Vec<Span> = Vec::new();
        for layer in &scene.layers {
            let from = quads.len() as u32;
            vertices_of(
                &layer.pieces,
                &self.gpu,
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

        // Cloned rather than borrowed: the context is a handle, and holding a
        // borrow of it would stop the buffer being grown while it is bound.
        let context = self.gpu.context.clone();
        self.gpu.ground(ground);
        // Cleared first, whatever is drawn after.
        let clear = [
            ground[0] as f32 / 255.0,
            ground[1] as f32 / 255.0,
            ground[2] as f32 / 255.0,
            1.0,
        ];
        unsafe {
            context.ClearRenderTargetView(&target, &clear);
            context.OMSetRenderTargets(Some(&[Some(target.clone())]), None);
            context.RSSetViewports(Some(&[self.gpu.viewport()]));
        }

        if !quads.is_empty() && self.room_for(quads.len()) {
            self.write(&quads);
            let sheets = [
                Some(self.atlas.view(Sheet::Letters)),
                Some(self.atlas.view(Sheet::Faces)),
                Some(self.atlas.view(Sheet::Pictures)),
                Some(match self.shown.as_ref() {
                    Some(view) => view.clone(),
                    None => self.nothing.clone(),
                }),
            ];
            let viewport = Viewport {
                size: [size.0 as f32, size.1 as f32],
                // Scrolling is baked into the positions by whoever built the
                // scene; a panel that scrolls is not the renderer's business.
                scroll: 0.0,
                padding: 0.0,
            };
            self.write_uniform(&viewport);

            let stride = std::mem::size_of::<Vertex>() as u32;
            let offset = 0u32;
            unsafe {
                context.IASetInputLayout(&self.bound.layout);
                context.IASetPrimitiveTopology(D3D11_PRIMITIVE_TOPOLOGY_TRIANGLELIST);
                context.IASetVertexBuffers(
                    0,
                    1,
                    Some(&self.vertices.clone()),
                    Some(&stride),
                    Some(&offset),
                );
                context.VSSetShader(&self.bound.vertex, None);
                context.PSSetShader(&self.bound.fragment, None);
                context.VSSetConstantBuffers(0, Some(&[Some(self.bound.uniform.clone())]));
                // Every texture at once, and each quad names the one it wants:
                // the only thing left that splits a frame into draws is the
                // scissor.
                context.PSSetShaderResources(0, Some(&sheets));
                context.PSSetSamplers(
                    0,
                    Some(&[
                        Some(self.bound.point.clone()),
                        Some(self.bound.smooth.clone()),
                    ]),
                );
                context.OMSetBlendState(&self.bound.blend, None, 0xffff_ffff);
                context.RSSetState(&self.bound.raster);
            }
            // The opened picture last, over everything, for the same reason it
            // is built last.
            for (range, clip) in spans.into_iter().chain(shown) {
                let Some(scissor) = fits(clip, size) else {
                    continue;
                };
                unsafe {
                    context.RSSetScissorRects(Some(&[scissor]));
                    context.Draw(range.end - range.start, range.start);
                }
            }
        }

        // Vsync, because a chat window has nothing to gain from drawing faster
        // than the screen shows it -- except while it is being resized, when
        // what it has to gain is the frame arriving before the compositor
        // shows the window at its new size.
        let _ = unsafe { self.gpu.chain.Present(u32::from(waiting), DXGI_PRESENT(0)) };
        // A frame has been drawn, which is what the atlas ages its pictures
        // by: everything on screen was just asked for, so anything that was
        // not is a frame older than the things it competes with for room.
        self.atlas.drew();
    }

    /// Grows the vertex buffer if the frame has outgrown it.
    ///
    /// Only ever upwards: a conversation that once needed this many quads will
    /// need them again the moment it is scrolled back to.
    fn room_for(&mut self, quads: usize) -> bool {
        if self.vertices.is_some() && quads <= self.capacity {
            return true;
        }
        let capacity = quads.next_power_of_two().max(4096);
        let how = D3D11_BUFFER_DESC {
            ByteWidth: (capacity * std::mem::size_of::<Vertex>()) as u32,
            Usage: D3D11_USAGE_DYNAMIC,
            BindFlags: D3D11_BIND_VERTEX_BUFFER.0 as u32,
            CPUAccessFlags: D3D11_CPU_ACCESS_WRITE.0 as u32,
            ..Default::default()
        };
        let mut made: Option<ID3D11Buffer> = None;
        if unsafe { self.gpu.device.CreateBuffer(&how, None, Some(&mut made)) }.is_err() {
            return false;
        }
        self.vertices = made;
        self.capacity = capacity;
        self.vertices.is_some()
    }

    /// Copies the frame's quads in, discarding what was there.
    ///
    /// `DISCARD` rather than a second buffer: it tells the runtime the old
    /// contents are not wanted, so it hands back memory the device has
    /// finished with instead of waiting for it.
    fn write(&self, quads: &[Vertex]) {
        let Some(buffer) = self.vertices.as_ref() else {
            return;
        };
        let mut into = D3D11_MAPPED_SUBRESOURCE::default();
        if unsafe {
            self.gpu
                .context
                .Map(buffer, 0, D3D11_MAP_WRITE_DISCARD, 0, Some(&mut into))
        }
        .is_err()
        {
            return;
        }
        let bytes = as_bytes(quads);
        // SAFETY: the buffer was made at least this big, and `Map` gives a
        // pointer to all of it.
        unsafe {
            std::ptr::copy_nonoverlapping(bytes.as_ptr(), into.pData as *mut u8, bytes.len());
            self.gpu.context.Unmap(buffer, 0);
        }
    }

    fn write_uniform(&self, viewport: &Viewport) {
        let mut into = D3D11_MAPPED_SUBRESOURCE::default();
        if unsafe {
            self.gpu.context.Map(
                &self.bound.uniform,
                0,
                D3D11_MAP_WRITE_DISCARD,
                0,
                Some(&mut into),
            )
        }
        .is_err()
        {
            return;
        }
        let bytes = viewport_bytes(viewport);
        // SAFETY: the buffer is exactly this size.
        unsafe {
            std::ptr::copy_nonoverlapping(bytes.as_ptr(), into.pData as *mut u8, bytes.len());
            self.gpu.context.Unmap(&self.bound.uniform, 0);
        }
    }
}

/// One transparent texel, to stand in the fourth slot while nothing is open.
fn one_texel(gpu: &Gpu) -> Result<ID3D11ShaderResourceView, String> {
    let how = D3D11_TEXTURE2D_DESC {
        Width: 1,
        Height: 1,
        MipLevels: 1,
        ArraySize: 1,
        Format: DXGI_FORMAT_R8G8B8A8_UNORM,
        SampleDesc: DXGI_SAMPLE_DESC {
            Count: 1,
            Quality: 0,
        },
        Usage: D3D11_USAGE_DEFAULT,
        BindFlags: D3D11_BIND_SHADER_RESOURCE.0 as u32,
        ..Default::default()
    };
    let nothing = [0u8; 4];
    let first = D3D11_SUBRESOURCE_DATA {
        pSysMem: nothing.as_ptr() as *const _,
        SysMemPitch: 4,
        SysMemSlicePitch: 0,
    };
    let mut texture: Option<ID3D11Texture2D> = None;
    unsafe {
        gpu.device
            .CreateTexture2D(&how, Some(&first), Some(&mut texture))
    }
    .map_err(|why| why.to_string())?;
    let texture = texture.ok_or("no texture")?;
    let mut view: Option<ID3D11ShaderResourceView> = None;
    unsafe {
        gpu.device
            .CreateShaderResourceView(&texture, None, Some(&mut view))
    }
    .map_err(|why| why.to_string())?;
    view.ok_or_else(|| "no view".to_string())
}

/// A clip rectangle, clamped to what is actually on the surface.
///
/// A scissor outside it is refused rather than a wrong pixel, and a panel can
/// be dragged past the edge.
fn fits(clip: Clip, size: (u32, u32)) -> Option<RECT> {
    let x = clip.0.max(0.0).min(size.0 as f32) as i32;
    let y = clip.1.max(0.0).min(size.1 as f32) as i32;
    let width = (clip.2.min(size.0 as f32 - x as f32)).max(0.0) as i32;
    let height = (clip.3.min(size.1 as f32 - y as f32)).max(0.0) as i32;
    if width == 0 || height == 0 {
        return None;
    }
    Some(RECT {
        left: x,
        top: y,
        right: x + width,
        bottom: y + height,
    })
}
