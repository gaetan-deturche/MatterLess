//! Where rasterised glyphs live on the GPU.
//!
//! A glyph is rasterised once and then drawn as a textured quad, however many
//! times it appears. The alternative -- uploading a bitmap per glyph per frame --
//! is what makes text renderers slow, and a chat window draws the same few
//! hundred glyphs over and over.
//!
//! The allocator is a shelf: rows of a fixed height, filled left to right. It
//! wastes some space at the end of each row and cannot reclaim anything, which
//! is the right trade for glyphs -- they are small, similar in height, and there
//! are only so many of them in a font at one size.

use cosmic_text::{CacheKey, SwashCache, SwashContent};
use matterless_layout::Fonts;
use std::collections::HashMap;

/// The side of the atlas texture. 1024 holds several thousand glyphs at chat
/// sizes, which is every glyph the window will ever ask for.
pub const SIDE: u32 = 1024;

/// Where one glyph sits in the atlas, and how far it is drawn from the pen.
#[derive(Debug, Clone, Copy)]
pub struct Slot {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
    /// The bitmap's offset from the glyph's origin, which is not the same as
    /// its position: a `y` that ignores this puts every letter on its own
    /// baseline.
    pub left: i32,
    pub top: i32,
    /// The glyph carries its own colour -- an emoji -- so nothing may tint it.
    pub colour: bool,
}

pub struct Atlas {
    texture: wgpu::Texture,
    pub view: wgpu::TextureView,
    slots: HashMap<CacheKey, Option<Slot>>,
    /// The shelf being filled, and where in it.
    pen_x: u32,
    shelf_y: u32,
    shelf_height: u32,
    full: bool,
}

impl Atlas {
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue) -> Self {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("glyph atlas"),
            size: wgpu::Extent3d {
                width: SIDE,
                height: SIDE,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            // Colour and coverage, not coverage alone. A letter is stored as
            // white with the coverage in its alpha, so the vertex tints it; an
            // emoji is stored as it is, and its vertex is white so nothing
            // tints it. One format, one sampler, both kinds of glyph -- where
            // a coverage-only atlas drew every emoji as a white silhouette. Plain
            // rather than sRGB: these bytes are already sRGB and the frame is
            // written without a second encoding, so sampling must not decode.
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let atlas = Self {
            texture,
            view,
            slots: HashMap::new(),
            pen_x: 1,
            shelf_y: 1,
            shelf_height: 0,
            full: false,
        };
        // One opaque white texel at the origin, so a solid rectangle is the
        // same pipeline as a glyph: it samples this and multiplies by its
        // colour.
        atlas.write(queue, 0, 0, 1, 1, &[255, 255, 255, 255]);
        atlas
    }

    fn write(&self, queue: &wgpu::Queue, x: u32, y: u32, width: u32, height: u32, data: &[u8]) {
        if width == 0 || height == 0 {
            return;
        }
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &self.texture,
                mip_level: 0,
                origin: wgpu::Origin3d { x, y, z: 0 },
                aspect: wgpu::TextureAspect::All,
            },
            data,
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
    }

    /// The slot for a glyph, rasterising it the first time it is asked for.
    ///
    /// `None` means the glyph has no pixels -- a space, or something the font
    /// draws as nothing -- which is a real answer and is cached as one.
    pub fn slot(
        &mut self,
        queue: &wgpu::Queue,
        fonts: &mut Fonts,
        cache: &mut SwashCache,
        key: CacheKey,
    ) -> Option<Slot> {
        if let Some(held) = self.slots.get(&key) {
            return *held;
        }
        let image = cache.get_image_uncached(fonts.system_mut(), key);
        let Some(image) = image else {
            self.slots.insert(key, None);
            return None;
        };
        let width = image.placement.width;
        let height = image.placement.height;
        if width == 0 || height == 0 {
            self.slots.insert(key, None);
            return None;
        }

        // A mask becomes white with its coverage in the alpha, so the vertex
        // colour decides what shade the letter is. A colour bitmap is kept
        // exactly as it is.
        let coloured = matches!(image.content, SwashContent::Color);
        let pixels: Vec<u8> = if coloured {
            image.data.clone()
        } else {
            image
                .data
                .iter()
                .flat_map(|coverage| [255, 255, 255, *coverage])
                .collect()
        };

        let Some(slot) = self.reserve(width, height) else {
            // Out of room. Cached as absent rather than retried every frame:
            // a missing glyph is a visible gap, and a rasterise per frame is a
            // stutter that would hide the cause.
            self.slots.insert(key, None);
            return None;
        };
        self.write(queue, slot.x, slot.y, width, height, &pixels);
        let slot = Slot {
            left: image.placement.left,
            top: image.placement.top,
            colour: coloured,
            ..slot
        };
        self.slots.insert(key, Some(slot));
        Some(slot)
    }

    /// Finds room on the current shelf, or opens the next one.
    fn reserve(&mut self, width: u32, height: u32) -> Option<Slot> {
        if self.full || width > SIDE {
            return None;
        }
        if self.pen_x + width > SIDE {
            self.shelf_y += self.shelf_height + 1;
            self.pen_x = 1;
            self.shelf_height = 0;
        }
        if self.shelf_y + height >= SIDE {
            self.full = true;
            return None;
        }
        let slot = Slot {
            x: self.pen_x,
            y: self.shelf_y,
            width,
            height,
            left: 0,
            top: 0,
            colour: false,
        };
        self.pen_x += width + 1;
        self.shelf_height = self.shelf_height.max(height);
        Some(slot)
    }
}
