//! Where rasterised glyphs and fetched pictures live on the GPU.
//!
//! A glyph is rasterised once and then drawn as a textured quad, however many
//! times it appears. The alternative -- uploading a bitmap per glyph per frame --
//! is what makes text renderers slow, and a chat window draws the same few
//! hundred glyphs over and over. A picture is the same idea with the bitmap
//! coming off the network instead of out of a font.
//!
//! **Three sheets, not one.** They are three textures rather than three bands
//! of one because what they hold has nothing in common but its format:
//!
//! * **Letters.** A font at a handful of sizes. Bounded, long-lived, and
//!   nothing reclaims: a letter that will not rasterise is a hole in a
//!   sentence, so this sheet is sized never to need to.
//! * **Faces.** Avatars, team icons, custom emoji, and the mini previews that
//!   stand in for a picture on its way. All small, all repeated, all wanted
//!   again the moment a reader scrolls back.
//! * **Pictures.** Attachments, drawn in a box five hundred pixels wide. Large,
//!   and passed once.
//!
//! Sharing one sheet made the third crowd out the second: sixteen screenshots
//! take a whole 2048-pixel band, and a reader passes sixteen in a few seconds
//! of scrolling -- so the avatars they had been looking at all along were what
//! got thrown away to make room for pictures they were scrolling past. Apart, a
//! channel of screenshots costs the faces beside them nothing.
//!
//! The allocator is a shelf: rows of whatever height the tallest thing in them
//! needs, filled left to right. It wastes some space at the end of each row,
//! which is the right trade for things that are similar in height.
//!
//! Both picture sheets reclaim, and what they keep is what is on screen. When
//! one fills, everything not drawn in the last few frames is thrown away and
//! the survivors are packed again from the top -- which is why their pixels are
//! kept on this side as well: repacking moves them, and asking the server for
//! them a second time to put them somewhere else would be a request to answer
//! a question already answered.

use cosmic_text::{CacheKey, SwashCache, SwashContent};
use matterless_layout::Fonts;
use std::collections::HashMap;

/// Which sheet a quad samples.
///
/// The renderer binds one texture at a time, so this is what a run of quads is
/// grouped by -- in the order they were built, never sorted, because the scene
/// is painted in the order it is built and grouping by texture would reorder
/// it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sheet {
    Letters,
    Faces,
    Pictures,
}

/// How big each sheet is, in texels.
///
/// Letters get more room than the single band ever gave them, because nothing
/// reclaims there. Faces are small and repeated, so a megapixel is hundreds of
/// them. Pictures get the largest sheet and still reclaim, because no size is
/// enough for an unbounded scroll.
const LETTERS: (u32, u32) = (2048, 1024);
const FACES: (u32, u32) = (1024, 1024);
const PICTURES: (u32, u32) = (2048, 2048);

impl Sheet {
    /// How wide and tall this one is, which is what turns a slot into texture
    /// coordinates.
    pub fn size(self) -> (u32, u32) {
        match self {
            Sheet::Letters => LETTERS,
            Sheet::Faces => FACES,
            Sheet::Pictures => PICTURES,
        }
    }

    /// Which sheet a picture belongs on, by the route it came from.
    ///
    /// A mini preview goes with the faces rather than with the picture it
    /// stands in for: it is a kilobyte, and keeping it after its picture has
    /// been thrown away means a reader scrolling back sees the blur they saw
    /// the first time rather than a blank.
    fn of(key: &str) -> Self {
        match key.split_once('/').map(|(kind, _)| kind) {
            Some("avatar" | "team" | "emoji" | "mini") => Sheet::Faces,
            _ => Sheet::Pictures,
        }
    }
}

/// Where one glyph sits in its sheet, and how far it is drawn from the pen.
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

/// One picture in a sheet: where it is, what it is, and when it was last
/// drawn.
struct Held {
    slot: Slot,
    /// Its pixels, kept so the sheet can be packed again without asking the
    /// server for them a second time. Bounded by the sheet itself, and only
    /// ever what is actually in there.
    pixels: Vec<u8>,
    /// The frame it was last drawn in, which is the whole of the policy: what
    /// is on screen stays, and everything else is what makes room.
    used: u64,
}

/// A shelf allocator over one sheet.
///
/// The three are one of these rather than three copies of the same six lines.
/// They were two copies once, and the copies already disagreed about whether
/// the last row counted.
#[derive(Debug)]
struct Pen {
    wide: u32,
    tall: u32,
    x: u32,
    y: u32,
    shelf: u32,
    full: bool,
}

impl Pen {
    fn new(size: (u32, u32)) -> Self {
        Self {
            wide: size.0,
            tall: size.1,
            // Past the white texel at the origin, which a solid rectangle
            // samples.
            x: 1,
            y: 1,
            shelf: 0,
            full: false,
        }
    }

    /// Back to the top, with nothing in it. What a repack starts from.
    fn reset(&mut self) {
        self.x = 1;
        self.y = 1;
        self.shelf = 0;
        self.full = false;
    }

    /// Finds room on the current shelf, or opens the next one.
    fn reserve(&mut self, width: u32, height: u32) -> Option<Slot> {
        if self.full || width > self.wide {
            return None;
        }
        if self.x + width > self.wide {
            self.y += self.shelf + 1;
            self.x = 1;
            self.shelf = 0;
        }
        if self.y + height >= self.tall {
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
/// Pictures are written into a sheet before the frame that uses them is built,
/// so the freshest mark available when room is needed is the frame before this
/// one. Three is that, with a frame either side to spare.
const KEEP: u64 = 3;

/// One texture, and the shelf over it.
struct Surface {
    texture: wgpu::Texture,
    view: wgpu::TextureView,
    pen: Pen,
}

impl Surface {
    fn new(device: &wgpu::Device, label: &str, size: (u32, u32)) -> Self {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some(label),
            size: wgpu::Extent3d {
                width: size.0,
                height: size.1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            // Colour and coverage, not coverage alone. A letter is stored as
            // white with the coverage in its alpha, so the vertex tints it; an
            // emoji is stored as it is, and its vertex is white so nothing
            // tints it. One format, one sampler, both kinds of glyph -- where
            // a coverage-only atlas drew every emoji as a white silhouette.
            // Plain rather than sRGB: these bytes are already sRGB and the
            // frame is written without a second encoding, so sampling must not
            // decode.
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        Self {
            texture,
            view,
            pen: Pen::new(size),
        }
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
}

/// A sheet of pictures, and what is in it.
struct Store {
    surface: Surface,
    images: HashMap<String, Option<Held>>,
    /// Thrown away and not yet reported, so whoever asked for them knows to
    /// ask again if they come back on screen.
    forgotten: Vec<String>,
    what: &'static str,
}

impl Store {
    fn new(device: &wgpu::Device, what: &'static str, size: (u32, u32)) -> Self {
        Self {
            surface: Surface::new(device, what, size),
            images: HashMap::new(),
            forgotten: Vec::new(),
            what,
        }
    }

    /// The slot a picture was written to, marking it as drawn.
    fn image(&mut self, key: &str, now: u64) -> Option<Slot> {
        let held = self.images.get_mut(key)?.as_mut()?;
        held.used = now;
        Some(held.slot)
    }

    fn put(
        &mut self,
        queue: &wgpu::Queue,
        key: &str,
        rgba: &[u8],
        width: u32,
        height: u32,
        now: u64,
    ) -> Option<Slot> {
        if let Some(held) = self.images.get(key) {
            return held.as_ref().map(|held| held.slot);
        }
        if width == 0 || height == 0 || rgba.len() < (width * height * 4) as usize {
            self.images.insert(key.to_string(), None);
            return None;
        }
        let slot = match self.surface.pen.reserve(width, height) {
            Some(slot) => slot,
            None => {
                // Out of room: throw away everything that is not on screen and
                // pack what is left again. A picture that cannot be had even
                // then is one larger than the sheet itself.
                self.make_room(queue, now);
                match self.surface.pen.reserve(width, height) {
                    Some(slot) => slot,
                    None => {
                        eprintln!("no room in {} for a {width}x{height} picture", self.what);
                        self.images.insert(key.to_string(), None);
                        return None;
                    }
                }
            }
        };
        self.surface
            .write(queue, slot.x, slot.y, width, height, rgba);
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
                used: now,
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
    /// them into less of the sheet, and the room that frees is the room the
    /// picture that triggered this needs.
    ///
    /// Nothing is refetched. The pixels are kept on this side precisely so
    /// that moving a picture costs an upload rather than a round trip, and so
    /// that what a reader is looking at does not blink while it is replaced by
    /// itself.
    fn make_room(&mut self, queue: &wgpu::Queue, now: u64) {
        let before = self.images.len();
        // Absences go too: they are the record of a question already answered,
        // and after this the answer may be different.
        let forgotten = &mut self.forgotten;
        self.images.retain(|key, held| {
            let keeping = held.as_ref().is_some_and(|held| held.used + KEEP >= now);
            if !keeping {
                forgotten.push(key.clone());
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
        self.surface.pen.reset();
        let mut kept = 0;
        for (key, held) in survivors {
            let Slot { width, height, .. } = held.slot;
            let Some(slot) = self.surface.pen.reserve(width, height) else {
                // It fitted a moment ago and does not now, which can only mean
                // the sheet is genuinely full of things being looked at.
                self.forgotten.push(key);
                continue;
            };
            self.surface
                .write(queue, slot.x, slot.y, width, height, &held.pixels);
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
        println!("{}: packed again, {kept} kept of {before}", self.what);
    }
}

pub struct Atlas {
    letters: Surface,
    slots: HashMap<CacheKey, Option<Slot>>,
    faces: Store,
    pictures: Store,
    /// Frames drawn, which is what `used` counts in.
    now: u64,
}

impl Atlas {
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue) -> Self {
        let atlas = Self {
            letters: Surface::new(device, "letters", LETTERS),
            slots: HashMap::new(),
            faces: Store::new(device, "faces", FACES),
            pictures: Store::new(device, "pictures", PICTURES),
            now: 0,
        };
        // One opaque white texel at the origin of the letters sheet, so a solid
        // rectangle is the same pipeline as a glyph: it samples this and
        // multiplies by its colour.
        atlas
            .letters
            .write(queue, 0, 0, 1, 1, &[255, 255, 255, 255]);
        atlas
    }

    /// What the pipeline binds, one per sheet.
    pub fn view(&self, sheet: Sheet) -> &wgpu::TextureView {
        match sheet {
            Sheet::Letters => &self.letters.view,
            Sheet::Faces => &self.faces.surface.view,
            Sheet::Pictures => &self.pictures.surface.view,
        }
    }

    /// The slot a picture was written to, and which sheet it is on.
    ///
    /// Marks it as drawn, which is what keeps it: the policy is "what is on
    /// screen stays", and this is the only place that knows a picture is.
    pub fn image(&mut self, key: &str) -> Option<(Sheet, Slot)> {
        let sheet = Sheet::of(key);
        let slot = match sheet {
            Sheet::Faces => self.faces.image(key, self.now),
            _ => self.pictures.image(key, self.now),
        }?;
        Some((sheet, slot))
    }

    /// A frame has been drawn.
    ///
    /// Counted rather than timed because it is only ever compared against
    /// itself: a window that is redrawing is a window whose pictures are being
    /// asked for, and one that is not does not age.
    pub fn drew(&mut self) {
        self.now += 1;
    }

    /// How many pictures are held, across both sheets that hold any.
    pub fn pictures_held(&self) -> usize {
        self.faces.images.values().flatten().count()
            + self.pictures.images.values().flatten().count()
    }

    /// The pictures thrown away since this was last asked.
    ///
    /// Whoever asked the server for one keeps its own record of having asked,
    /// so that it asks once rather than once a frame. That record has to be
    /// torn up when a picture is thrown away, or a reader who scrolls back up
    /// to it finds a gap that nothing will ever fill.
    pub fn forgotten(&mut self) -> Vec<String> {
        let mut all = std::mem::take(&mut self.faces.forgotten);
        all.append(&mut self.pictures.forgotten);
        all
    }

    /// Writes a decoded picture into whichever sheet it belongs on.
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
        let now = self.now;
        match Sheet::of(key) {
            Sheet::Faces => self.faces.put(queue, key, rgba, width, height, now),
            _ => self.pictures.put(queue, key, rgba, width, height, now),
        }
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

        let Some(slot) = self.letters.pen.reserve(width, height) else {
            // Out of room. Cached as absent rather than retried every frame:
            // a missing glyph is a visible gap, and a rasterise per frame is a
            // stutter that would hide the cause. Nothing reclaims here -- see
            // the note at the top on why this sheet is sized not to need to.
            eprintln!("no room in letters for a {width}x{height} glyph");
            self.slots.insert(key, None);
            return None;
        };
        self.letters
            .write(queue, slot.x, slot.y, width, height, &pixels);
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

    /// A pen fills its sheet and stops at the edge of it, never past.
    ///
    /// The halves of the old single texture were separate copies of this and
    /// already disagreed about whether the last row counted: one tested
    /// against the texture's side and the other against the split, which is
    /// the same line written twice and wrong once.
    #[test]
    fn a_pen_stays_inside_its_own_sheet() {
        let mut pen = Pen::new(PICTURES);
        let mut given = Vec::new();
        while let Some(slot) = pen.reserve(300, 200) {
            given.push(slot);
        }
        assert!(given.len() > 10, "only {} fitted", given.len());
        for slot in &given {
            assert!(slot.y + slot.height < PICTURES.1, "{slot:?} runs off it");
            assert!(slot.x + slot.width <= PICTURES.0, "{slot:?} runs off it");
        }
        for (at, slot) in given.iter().enumerate() {
            for other in &given[at + 1..] {
                assert!(!overlap(slot, other), "{slot:?} overlaps {other:?}");
            }
        }
    }

    /// And once it is full it stays full until it is reset.
    #[test]
    fn a_full_pen_gives_nothing_until_it_is_reset() {
        let mut pen = Pen::new(FACES);
        while pen.reserve(1000, 600).is_some() {}
        assert!(pen.reserve(1, 1).is_none(), "a full pen gave room");
        pen.reset();
        let slot = pen.reserve(1, 1).expect("a reset pen has room");
        assert_eq!((slot.x, slot.y), (1, 1));
    }

    /// Nothing is ever written over the white texel a solid rectangle samples.
    #[test]
    fn the_white_texel_at_the_origin_is_never_allocated() {
        let mut pen = Pen::new(LETTERS);
        let slot = pen.reserve(8, 8).expect("room");
        assert!(slot.x >= 1 && slot.y >= 1, "{slot:?} is on the white texel");
    }

    /// Tallest first, which is what makes a repack worth doing.
    ///
    /// A shelf is as tall as the tallest thing in it, so one tall picture in a
    /// row of short ones costs the whole row its height. Sorting does not make
    /// more of them *fit* -- the survivors of an eviction fitted a moment ago
    /// by definition -- it packs them into less of the sheet, and the room that
    /// frees is the room the picture that triggered the eviction needs.
    #[test]
    fn packing_tallest_first_leaves_more_of_the_sheet_free() {
        // Three to a shelf, and every third one tall: in the order they
        // arrived that is one tall picture in every shelf.
        let sizes: Vec<(u32, u32)> = (0..12)
            .map(|at| if at % 3 == 0 { (660, 300) } else { (660, 40) })
            .collect();

        let mut arrived = Pen::new(PICTURES);
        for (width, height) in &sizes {
            assert!(arrived.reserve(*width, *height).is_some(), "they all fit");
        }

        let mut sorted = sizes.clone();
        sorted.sort_by_key(|(_, height)| std::cmp::Reverse(*height));
        let mut tallest = Pen::new(PICTURES);
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

    /// A scroll through a channel of screenshots must not cost the avatars
    /// beside them, which is the whole reason there are three sheets.
    #[test]
    fn what_is_small_and_repeated_is_kept_apart_from_what_is_large_and_passed() {
        assert_eq!(Sheet::of("avatar/abc123"), Sheet::Faces);
        assert_eq!(Sheet::of("team/abc123"), Sheet::Faces);
        assert_eq!(Sheet::of("emoji/abc123"), Sheet::Faces);
        // The blur that stands in for a picture outlives the picture: a reader
        // scrolling back to an evicted one sees what they saw the first time
        // rather than a blank.
        assert_eq!(Sheet::of("mini/abc123"), Sheet::Faces);

        assert_eq!(Sheet::of("thumb/abc123"), Sheet::Pictures);
        assert_eq!(Sheet::of("preview/abc123"), Sheet::Pictures);
        assert_eq!(Sheet::of("file/abc123"), Sheet::Pictures);
        // Anything unrecognised goes with the large and the passing, which is
        // the sheet that reclaims hardest.
        assert_eq!(Sheet::of("something/else"), Sheet::Pictures);
        assert_eq!(Sheet::of("nonsense"), Sheet::Pictures);
    }

    /// Letters get more room than the one band ever gave them, because nothing
    /// reclaims there and a letter that will not rasterise is a hole in a
    /// sentence.
    #[test]
    fn the_letters_sheet_is_not_smaller_than_the_band_it_replaces() {
        let band = 2048 * (768 - 1);
        assert!(
            LETTERS.0 * LETTERS.1 >= band,
            "letters lost room: {} against {band}",
            LETTERS.0 * LETTERS.1
        );
    }
}
