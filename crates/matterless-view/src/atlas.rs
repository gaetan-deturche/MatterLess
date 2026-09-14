//! Where rasterised glyphs live on the GPU.
//!
//! A glyph is rasterised once and then drawn as a textured quad, however many
//! times it appears. The alternative -- uploading a bitmap per glyph per frame --
//! is what makes text renderers slow, and a chat window draws the same few
//! hundred glyphs over and over.
//!
//! The allocator is a shelf: rows of a fixed height, filled left to right. It
//! wastes some space at the end of each row, which is the right trade for
//! glyphs -- they are small, similar in height, and there are only so many of
//! them in a font at one size.
//!
//! Pictures are not like that, and the picture half reclaims. A single
//! screenshot is drawn in a box five hundred pixels wide, so sixteen of them
//! fill the half -- and a reader scrolling a channel of screenshots passes
//! sixteen in a few seconds. Without reclaiming, the seventeenth and everything
//! after it simply never appears, which is a failure with no symptom but a
//! blank.
//!
//! What it keeps is what is on screen. When the half fills, everything not
//! drawn in the last few frames is thrown away and the survivors are packed
//! again from the top -- which is why their pixels are kept on this side as
//! well: repacking moves them, and asking the server for them a second time
//! to put them somewhere else would be a request to answer a question already
//! answered.

use cosmic_text::{CacheKey, SwashCache, SwashContent};
use matterless_layout::Fonts;
use std::collections::HashMap;

/// The side of the atlas texture. 2048 is 16 MB of RGBA, which is nothing on
/// any card that runs Vulkan and is room for both halves below.
pub const SIDE: u32 = 2048;

/// Where the glyphs stop and the pictures start.
///
/// The two are fenced apart rather than sharing one allocator, because they
/// exhaust at wildly different rates: a channel full of screenshots would take
/// the whole atlas and then letters would stop rasterising, which is a baffling
/// failure to be handed. Glyphs are tiny, so the smaller half is theirs and it
/// still holds several thousand of them.
const SPLIT: u32 = 768;

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

/// One picture in the atlas: where it is, what it is, and when it was last
/// drawn.
struct Held {
    slot: Slot,
    /// Its pixels, kept so the half can be packed again without asking the
    /// server for them a second time. Bounded by the half itself -- ten
    /// megabytes at the very most, and only ever what is actually in there.
    pixels: Vec<u8>,
    /// The frame it was last drawn in, which is the whole of the policy: what
    /// is on screen stays, and everything else is what makes room.
    used: u64,
}

/// A shelf allocator over one band of the texture.
///
/// Rows of whatever height the tallest thing in them needs, filled left to
/// right. Both halves are one of these, which is what keeps the two from
/// drifting apart -- they were separate copies of the same six lines, and the
/// copies already disagreed about whether the last row counted.
#[derive(Debug)]
struct Pen {
    /// The band this fills: `[top, bottom)`.
    top: u32,
    bottom: u32,
    x: u32,
    y: u32,
    shelf: u32,
    full: bool,
}

impl Pen {
    fn new(top: u32, bottom: u32) -> Self {
        Self {
            top,
            bottom,
            x: 1,
            y: top,
            shelf: 0,
            full: false,
        }
    }

    /// Back to the top, with nothing in it. What a repack starts from.
    fn reset(&mut self) {
        self.x = 1;
        self.y = self.top;
        self.shelf = 0;
        self.full = false;
    }

    /// Finds room on the current shelf, or opens the next one.
    fn reserve(&mut self, width: u32, height: u32) -> Option<Slot> {
        if self.full || width > SIDE {
            return None;
        }
        if self.x + width > SIDE {
            self.y += self.shelf + 1;
            self.x = 1;
            self.shelf = 0;
        }
        if self.y + height >= self.bottom {
            self.full = true;
            return None;
        }
        let slot = Slot {
            x: self.x,
            y: self.y,
            width,
            height,
            left: 0,
            top: 0,
            colour: false,
        };
        self.x += width + 1;
        self.shelf = self.shelf.max(height);
        Some(slot)
    }
}

/// How many frames back still counts as "on screen".
///
/// Pictures are written into the atlas before the frame that uses them is
/// built, so the freshest mark available when room is needed is the frame
/// before this one. Three is that, with a frame either side to spare.
const KEEP: u64 = 3;

pub struct Atlas {
    texture: wgpu::Texture,
    pub view: wgpu::TextureView,
    slots: HashMap<CacheKey, Option<Slot>>,
    /// Pictures -- avatars, attachments, emoji -- keyed by the route they came
    /// from. They share the glyph texture because a picture is exactly what a
    /// colour emoji already is: RGBA sampled with a white vertex, through the
    /// same pipeline and in the same draw call.
    images: HashMap<String, Option<Held>>,
    /// Frames drawn, which is what `used` counts in.
    now: u64,
    /// Pictures thrown away and not yet reported, so whoever asked for them
    /// knows to ask again if they come back on screen.
    forgotten: Vec<String>,
    /// Glyphs above the split, pictures below. Separate pens rather than a
    /// shared one, so a channel full of screenshots cannot take the room
    /// letters need.
    letters: Pen,
    pictures: Pen,
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
            images: HashMap::new(),
            now: 0,
            forgotten: Vec::new(),
            letters: Pen::new(1, SPLIT),
            pictures: Pen::new(SPLIT, SIDE),
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

    /// The slot a picture was written to, if it has been.
    ///
    /// Marks it as drawn, which is what keeps it: the policy is "what is on
    /// screen stays", and this is the only place that knows a picture is.
    pub fn image(&mut self, key: &str) -> Option<Slot> {
        let now = self.now;
        let held = self.images.get_mut(key)?.as_mut()?;
        held.used = now;
        Some(held.slot)
    }

    /// A frame has been drawn.
    ///
    /// Counted rather than timed because it is only ever compared against
    /// itself: a window that is redrawing is a window whose pictures are being
    /// asked for, and one that is not does not age.
    pub fn drew(&mut self) {
        self.now += 1;
    }

    /// How many pictures are in the atlas.
    pub fn pictures_held(&self) -> usize {
        self.images.values().flatten().count()
    }

    /// The pictures thrown away since this was last asked.
    ///
    /// Whoever asked the server for one keeps its own record of having asked,
    /// so that it asks once rather than once a frame. That record has to be
    /// torn up when a picture is thrown away, or a reader who scrolls back up
    /// to it finds a gap that nothing will ever fill.
    pub fn forgotten(&mut self) -> Vec<String> {
        std::mem::take(&mut self.forgotten)
    }

    /// Writes a decoded picture into the atlas.
    ///
    /// `rgba` is straight, not premultiplied, which is what the blend expects
    /// and what every decoder here produces.
    pub fn put_image(
        &mut self,
        queue: &wgpu::Queue,
        key: &str,
        rgba: &[u8],
        width: u32,
        height: u32,
    ) -> Option<Slot> {
        if let Some(held) = self.images.get(key) {
            return held.as_ref().map(|held| held.slot);
        }
        if width == 0 || height == 0 || rgba.len() < (width * height * 4) as usize {
            self.images.insert(key.to_string(), None);
            return None;
        }
        let slot = match self.pictures.reserve(width, height) {
            Some(slot) => slot,
            None => {
                // Out of room: throw away everything that is not on screen and
                // pack what is left again. A picture that cannot be had even
                // then is one larger than the half itself.
                self.make_room(queue);
                match self.pictures.reserve(width, height) {
                    Some(slot) => slot,
                    None => {
                        eprintln!("no room in the atlas for a {width}x{height} picture");
                        self.images.insert(key.to_string(), None);
                        return None;
                    }
                }
            }
        };
        self.write(queue, slot.x, slot.y, width, height, rgba);
        // Its own colours, so the vertex must not tint it.
        let slot = Slot {
            colour: true,
            ..slot
        };
        self.images.insert(
            key.to_string(),
            Some(Held {
                slot,
                pixels: rgba[..(width * height * 4) as usize].to_vec(),
                used: self.now,
            }),
        );
        Some(slot)
    }

    /// Throws away the pictures nobody is looking at and packs the rest again.
    ///
    /// Packed tallest first, which is what a shelf allocator wants: a shelf is
    /// as tall as the tallest thing in it, so one tall picture in a row of
    /// short ones costs the whole row its height. That does not make more of
    /// them fit -- the survivors fitted a moment ago by definition -- it packs
    /// them into less of the band, and the room that frees is the room the
    /// picture that triggered this needs.
    ///
    /// Nothing is refetched. The pixels are kept on this side precisely so
    /// that moving a picture costs an upload rather than a round trip, and so
    /// that what a reader is looking at does not blink while it is replaced by
    /// itself.
    fn make_room(&mut self, queue: &wgpu::Queue) {
        let now = self.now;
        let before = self.images.len();
        // Absences go too: they are the record of a question already answered,
        // and after this the answer may be different.
        self.images.retain(|key, held| {
            let keeping = held.as_ref().is_some_and(|held| held.used + KEEP >= now);
            if !keeping {
                self.forgotten.push(key.clone());
            }
            keeping
        });
        let mut survivors: Vec<(String, Held)> = self
            .images
            .drain()
            .filter_map(|(key, held)| held.map(|held| (key, held)))
            .collect();
        survivors.sort_by(|left, right| {
            right
                .1
                .slot
                .height
                .cmp(&left.1.slot.height)
                .then_with(|| left.0.cmp(&right.0))
        });
        self.pictures.reset();
        let mut kept = 0;
        for (key, held) in survivors {
            let Slot { width, height, .. } = held.slot;
            let Some(slot) = self.pictures.reserve(width, height) else {
                // It fitted a moment ago and does not now, which can only mean
                // the half is genuinely full of things being looked at.
                continue;
            };
            self.write(queue, slot.x, slot.y, width, height, &held.pixels);
            kept += 1;
            self.images.insert(
                key,
                Some(Held {
                    slot: Slot {
                        colour: true,
                        ..slot
                    },
                    ..held
                }),
            );
        }
        println!("atlas: packed again, {kept} pictures kept of {before}");
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

        let Some(slot) = self.letters.reserve(width, height) else {
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

}

#[cfg(test)]
mod tests {
    use super::*;

    /// Whether two slots overlap, which nothing in one pen ever may.
    fn overlap(left: &Slot, right: &Slot) -> bool {
        left.x < right.x + right.width
            && right.x < left.x + left.width
            && left.y < right.y + right.height
            && right.y < left.y + left.height
    }

    /// A pen fills its band and stops at the bottom of it, never past.
    ///
    /// The two halves used to be separate copies of this and already disagreed
    /// about whether the last row counted: one tested against `SIDE` and the
    /// other against `SPLIT`, which is the same line written twice and wrong
    /// once.
    #[test]
    fn a_pen_stays_inside_its_own_band() {
        let mut pen = Pen::new(SPLIT, SIDE);
        let mut given = Vec::new();
        while let Some(slot) = pen.reserve(300, 200) {
            given.push(slot);
        }
        assert!(given.len() > 10, "only {} fitted", given.len());
        for slot in &given {
            assert!(slot.y >= SPLIT, "{slot:?} is in the glyphs' half");
            assert!(slot.y + slot.height < SIDE, "{slot:?} runs off the bottom");
            assert!(slot.x + slot.width <= SIDE, "{slot:?} runs off the side");
        }
        for (at, slot) in given.iter().enumerate() {
            for other in &given[at + 1..] {
                assert!(!overlap(slot, other), "{slot:?} overlaps {other:?}");
            }
        }
    }

    /// And once it is full it stays full until it is reset, rather than
    /// creeping into the half above.
    #[test]
    fn a_full_pen_gives_nothing_until_it_is_reset() {
        let mut pen = Pen::new(SPLIT, SIDE);
        while pen.reserve(2000, 600).is_some() {}
        assert!(pen.reserve(1, 1).is_none(), "a full pen gave room");
        pen.reset();
        let slot = pen.reserve(1, 1).expect("a reset pen has room");
        assert_eq!((slot.x, slot.y), (1, SPLIT));
    }

    /// Tallest first, which is what makes a repack worth doing.
    ///
    /// A shelf is as tall as the tallest thing in it, so one tall picture in a
    /// row of short ones costs the whole row its height. Sorting does not make
    /// more of them *fit* -- the survivors of an eviction fitted a moment ago
    /// by definition -- it packs them into less of the band, and the room that
    /// frees is the room the picture that triggered the eviction needs.
    #[test]
    fn packing_tallest_first_leaves_more_of_the_band_free() {
        // Three to a shelf, and every third one tall: in the order they
        // arrived that is one tall picture in every shelf.
        let sizes: Vec<(u32, u32)> = (0..12)
            .map(|at| if at % 3 == 0 { (660, 300) } else { (660, 40) })
            .collect();

        let mut arrived = Pen::new(SPLIT, SIDE);
        for (width, height) in &sizes {
            assert!(arrived.reserve(*width, *height).is_some(), "they all fit");
        }

        let mut sorted = sizes.clone();
        sorted.sort_by_key(|(_, height)| std::cmp::Reverse(*height));
        let mut tallest = Pen::new(SPLIT, SIDE);
        for (width, height) in &sorted {
            assert!(tallest.reserve(*width, *height).is_some(), "they all fit");
        }

        assert!(
            tallest.y < arrived.y,
            "tallest first reached {} and the order they arrived reached {}",
            tallest.y,
            arrived.y
        );
    }

    /// What counts as "on screen" is the last few frames, and the frame a
    /// picture is written in is not yet one of them.
    ///
    /// Pictures go in before the frame that uses them is built, so the
    /// freshest mark available when room is needed is the frame before this
    /// one. A rule that kept only `used == now` would throw away the very
    /// pictures it was making room for.
    #[test]
    fn the_frame_a_picture_is_written_in_still_counts_as_on_screen() {
        let keeps = |used: u64, now: u64| used + KEEP >= now;
        assert!(keeps(10, 10), "written this frame");
        assert!(keeps(9, 10), "drawn in the frame before");
        assert!(keeps(7, 10), "three frames back is the edge");
        assert!(!keeps(6, 10), "and four is past it");
    }
}
