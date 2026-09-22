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

use crate::d3d::Gpu;
use cosmic_text::{CacheKey, SwashCache, SwashContent};
use matterless_layout::Fonts;
use std::collections::HashMap;
use windows::Win32::Graphics::Direct3D11::{
    D3D11_BIND_SHADER_RESOURCE, D3D11_BOX, D3D11_TEXTURE2D_DESC, D3D11_USAGE_DEFAULT,
    ID3D11ShaderResourceView, ID3D11Texture2D,
};
use windows::Win32::Graphics::Dxgi::Common::{DXGI_FORMAT_R8G8B8A8_UNORM, DXGI_SAMPLE_DESC};

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

/// How much a glyph's coverage is opened up before it is stored.
///
/// The window blends in gamma space -- the swapchain is `B8G8R8A8_UNORM`, not
/// `_SRGB` -- which is the usual choice for interface text and the reason it
/// has to be paid for here. Half coverage written as 128 is not half the light
/// of full coverage; on a dark panel it lands nearer a quarter, so the edges of
/// every stem come out thinner than the shape they were rasterised from and
/// light text on dark reads as spindly.
///
/// The curve gives that back. It is applied here rather than in the fragment
/// shader because this is the one place a glyph's pixels are decided: a
/// rectangle samples the sheet's opaque texel and a colour emoji carries its
/// own alpha, and neither wants this done to it.
const COVERAGE_GAMMA: f32 = 1.4;

fn weighted(coverage: u8) -> u8 {
    // The ends are exact: nothing is not a little something, and a filled
    // texel stays filled.
    if coverage == 0 || coverage == 255 {
        return coverage;
    }
    let part = f32::from(coverage) / 255.0;
    (part.powf(1.0 / COVERAGE_GAMMA) * 255.0)
        .round()
        .clamp(0.0, 255.0) as u8
}

/// Smears each subpixel's coverage across its neighbours, and answers RGB
/// triples.
///
/// Without this, subpixel antialiasing is coloured fringes. A stem landing on
/// one channel and not the two beside it lights that channel alone, and what
/// the eye sees at the edge of every letter is orange on one side and blue on
/// the other rather than a sharper letter.
///
/// The filter is the one ClearType and FreeType both use in some form: a
/// five-tap weighted average along the row of subpixels, which spreads a
/// third of a pixel's worth of light over a whole pixel's width. The colour
/// does not disappear -- it is what carries the extra resolution -- but it
/// stops being the thing you notice.
///
/// Three taps, not five. FreeType calls this one "light" and defaults to it,
/// and the reason is measurable: the five-tap `[1, 2, 3, 2, 1] / 9` reaches two
/// subpixels either side -- two thirds of a whole pixel -- and took the
/// steepest edges in a line of text from 163 to 142 while widening the run of
/// pixels that are neither ink nor panel from 8.8% to 12.4%. That is the
/// difference between antialiasing and a blur, and it is what somebody means
/// when they say sharper text came out soft.
///
/// One subpixel either side is a third of a pixel: enough to carry a channel's
/// light into its neighbours, not enough to smear the stem.
fn spread(data: &[u8], width: usize, height: usize) -> Vec<u8> {
    const TAPS: [u32; 3] = [1, 1, 1];
    const TOTAL: u32 = 3;
    let across = width * 3;
    let mut out = vec![0u8; across * height];
    for row in 0..height {
        // The row as one run of subpixel samples, which is what the filter
        // works along: red, green and blue are three places on the line rather
        // than three channels of one place.
        let from = row * width * 4;
        let line: Vec<u8> = data[from..from + width * 4]
            .as_chunks::<4>()
            .0
            .iter()
            .flat_map(|texel| [texel[0], texel[1], texel[2]])
            .collect();
        for at in 0..across {
            let mut sum = 0u32;
            for (tap, weight) in TAPS.iter().enumerate() {
                // Past either end is no coverage, which is true: there is
                // nothing there.
                let near = at as isize + tap as isize - (TAPS.len() as isize / 2);
                if near >= 0 && (near as usize) < across {
                    sum += u32::from(line[near as usize]) * weight;
                }
            }
            out[row * across + at] = (sum / TOTAL).min(255) as u8;
        }
    }
    out
}

/// Rasterises one glyph with a channel of coverage each.
///
/// The same rasterisation cosmic-text does, asked for in a different format.
/// Its `SwashCache` renders `Format::Alpha` and offers no way to say
/// otherwise, so the scaler is built here -- font, size, hinting, the
/// fractional offset the cache key carries, and the fake italic -- and the
/// only line that differs is the format.
///
/// `None` for a glyph with no pixels, which a space is, and which is a real
/// answer rather than a failure.
fn subpixel_image(
    fonts: &mut Fonts,
    context: &mut swash::scale::ScaleContext,
    key: CacheKey,
) -> Option<cosmic_text::SwashImage> {
    use swash::scale::{Render, Source, StrikeWith};
    use swash::zeno::{Angle, Format, Transform, Vector};

    let font = fonts.system_mut().get_font(key.font_id, key.font_weight)?;
    let mut scaler = context
        .builder(font.as_swash())
        .size(f32::from_bits(key.font_size_bits))
        .hint(
            !key.flags
                .contains(cosmic_text::CacheKeyFlags::DISABLE_HINTING),
        )
        .build();
    // Where inside the pixel the glyph starts, which is what makes two "a"s a
    // third of a pixel apart look like two "a"s rather than one shifted.
    let offset = match key.flags.contains(cosmic_text::CacheKeyFlags::PIXEL_FONT) {
        true => Vector::new(key.x_bin.as_float().round(), key.y_bin.as_float().round()),
        false => Vector::new(key.x_bin.as_float(), key.y_bin.as_float()),
    };
    Render::new(&[
        Source::ColorOutline(0),
        Source::ColorBitmap(StrikeWith::BestFit),
        Source::Outline,
    ])
    // The one line this function exists for.
    .format(Format::Subpixel)
    .offset(offset)
    .transform(
        match key.flags.contains(cosmic_text::CacheKeyFlags::FAKE_ITALIC) {
            true => Some(Transform::skew(
                Angle::from_degrees(14.0),
                Angle::from_degrees(0.0),
            )),
            false => None,
        },
    )
    .render(&mut scaler, key.glyph_id)
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
    /// The three channels hold coverage of their own rather than the glyph
    /// being white with one coverage in its alpha.
    ///
    /// Which is what subpixel antialiasing is: a stem that covers the red
    /// third of a pixel and not the other two is drawn as a third of a pixel
    /// rather than as a third of the light of a whole one. It needs the
    /// fragment to hand each channel its own coverage, which is why it also
    /// needs a second colour out of the shader and a blend that reads it.
    pub subpixel: bool,
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
            subpixel: false,
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
    texture: ID3D11Texture2D,
    view: ID3D11ShaderResourceView,
    pen: Pen,
}

impl Surface {
    fn new(gpu: &Gpu, size: (u32, u32)) -> Self {
        // Colour and coverage, not coverage alone. A letter is stored as white
        // with the coverage in its alpha, so the vertex tints it; an emoji is
        // stored as it is, and its vertex is white so nothing tints it. One
        // format, one sampler, both kinds of glyph -- where a coverage-only
        // atlas drew every emoji as a white silhouette. Straight rather than
        // sRGB: these bytes are already sRGB and the frame is written without
        // a second encoding, so sampling must not decode.
        let how = D3D11_TEXTURE2D_DESC {
            Width: size.0,
            Height: size.1,
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
        let mut texture: Option<ID3D11Texture2D> = None;
        unsafe { gpu.device.CreateTexture2D(&how, None, Some(&mut texture)) }.expect("a sheet");
        let texture = texture.expect("a sheet");
        let mut view: Option<ID3D11ShaderResourceView> = None;
        unsafe {
            gpu.device
                .CreateShaderResourceView(&texture, None, Some(&mut view))
        }
        .expect("a view of the sheet");
        Self {
            texture,
            view: view.expect("a view of the sheet"),
            pen: Pen::new(size),
        }
    }

    /// Copies a rectangle of pixels in.
    ///
    /// Straight into the texture rather than through a staging buffer of our
    /// own: the runtime keeps one and knows when the device is finished with
    /// it, which is the whole difference between this and doing it by hand.
    fn write(&self, gpu: &Gpu, x: u32, y: u32, width: u32, height: u32, data: &[u8]) {
        if width == 0 || height == 0 {
            return;
        }
        let wanted = (width * height * 4) as usize;
        let Some(bytes) = data.get(..wanted) else {
            return;
        };
        let where_to = D3D11_BOX {
            left: x,
            top: y,
            front: 0,
            right: x + width,
            bottom: y + height,
            back: 1,
        };
        unsafe {
            gpu.context.UpdateSubresource(
                &self.texture,
                0,
                Some(&where_to),
                bytes.as_ptr() as *const _,
                width * 4,
                0,
            )
        };
    }
}

/// A sheet of pictures, and what is in it./// A sheet of pictures, and what is in it.
struct Store {
    surface: Surface,
    images: HashMap<String, Option<Held>>,
    /// Thrown away and not yet reported, so whoever asked for them knows to
    /// ask again if they come back on screen.
    forgotten: Vec<String>,
    what: &'static str,
}

impl Store {
    fn new(gpu: &Gpu, what: &'static str, size: (u32, u32)) -> Self {
        Self {
            surface: Surface::new(gpu, size),
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

    /// Writes new pixels into the slot a key already holds.
    ///
    /// `put` answers a key it already has with the slot it already gave and
    /// writes nothing, which is right for everything this atlas holds but one:
    /// nothing changes under its key here -- a new avatar is a new key -- and
    /// a picture that moves is exactly the exception. Every frame after the
    /// first went in through `put`, was recognised as a key already held, and
    /// was dropped on the floor; the loop played perfectly and the screen
    /// showed frame one.
    ///
    /// Answers whether it could. A frame of a different size than the slot is
    /// not a frame of the same picture, and goes back to being a new picture.
    fn replace(&mut self, gpu: &Gpu, key: &str, rgba: &[u8], width: u32, height: u32) -> bool {
        let Some(Some(held)) = self.images.get_mut(key) else {
            return false;
        };
        let needs = (width * height * 4) as usize;
        if held.slot.width != width || held.slot.height != height || rgba.len() < needs {
            return false;
        }
        let (x, y) = (held.slot.x, held.slot.y);
        // Kept on this side as well, because a repack moves what is here and
        // rewrites it from these bytes: without this the sheet would go back
        // to whichever frame the picture arrived on the next time it filled.
        held.pixels = rgba[..needs].to_vec();
        self.surface.write(gpu, x, y, width, height, rgba);
        true
    }

    fn put(
        &mut self,
        gpu: &Gpu,
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
                self.make_room(gpu, now);
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
        self.surface.write(gpu, slot.x, slot.y, width, height, rgba);
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
    fn make_room(&mut self, gpu: &Gpu, now: u64) {
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
                .write(gpu, slot.x, slot.y, width, height, &held.pixels);
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

/// What a glyph was rasterised for.
///
/// The mode is part of the key because the same glyph is a different bitmap
/// under each, and because a mark is always asked for flat: it is rasterised
/// at twice the size it is drawn and filtered down, and there is no sense in
/// which a channel's third of a pixel survives being halved.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct Rasterised {
    key: CacheKey,
    subpixel: bool,
}

pub struct Atlas {
    letters: Surface,
    slots: HashMap<Rasterised, Option<Slot>>,
    /// Reused across glyphs: it holds the scaled outlines a font has already
    /// been asked for, and building one per glyph throws that away.
    scaler: swash::scale::ScaleContext,
    /// Whether letters are rasterised with a channel of coverage each.
    ///
    /// Here rather than passed per glyph because it is a property of how this
    /// sheet was filled: every glyph in it was rasterised one way or the
    /// other, and changing it means the sheet no longer matches what the
    /// shader is told to expect.
    pub subpixel: bool,
    faces: Store,
    pictures: Store,
    /// Frames drawn, which is what `used` counts in.
    now: u64,
}

impl Atlas {
    pub fn new(gpu: &Gpu) -> Self {
        let atlas = Self {
            letters: Surface::new(gpu, LETTERS),
            slots: HashMap::new(),
            scaler: swash::scale::ScaleContext::new(),
            subpixel: false,
            faces: Store::new(gpu, "faces", FACES),
            pictures: Store::new(gpu, "pictures", PICTURES),
            now: 0,
        };
        // One opaque white texel at the origin of the letters sheet, so a solid
        // rectangle is the same pipeline as a glyph: it samples this and
        // multiplies by its colour.
        atlas.letters.write(gpu, 0, 0, 1, 1, &[255, 255, 255, 255]);
        atlas
    }

    /// Changes how letters are rasterised, and forgets the ones already done.
    ///
    /// Answers whether anything changed, so a caller does not redraw for an
    /// answer it already had. What is forgotten is only the record of where
    /// each glyph sits: the pixels stay in the sheet, because nothing reclaims
    /// there. One switch costs a second copy of whatever is on screen, which
    /// this sheet has room for -- and a reader who toggles it all afternoon
    /// gets a full sheet and glyphs that stop arriving, which is the trade the
    /// note at the top of this file already makes for a sheet that is sized
    /// never to need reclaiming.
    pub fn rasterise_subpixel(&mut self, want: bool) -> bool {
        if self.subpixel == want {
            return false;
        }
        self.subpixel = want;
        self.slots.clear();
        true
    }

    /// What the pipeline binds, one per sheet.
    pub fn view(&self, sheet: Sheet) -> ID3D11ShaderResourceView {
        match sheet {
            Sheet::Letters => self.letters.view.clone(),
            Sheet::Faces => self.faces.surface.view.clone(),
            Sheet::Pictures => self.pictures.surface.view.clone(),
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

    /// Whether a picture asked for in frame `used` counts as still being drawn
    /// in frame `now`.
    ///
    /// One frame of slack and no more. A picture goes into the atlas before
    /// the frame that draws it, so the freshest mark a caller can see between
    /// frames is the one before this -- and anything older is a picture the
    /// window has stopped drawing.
    fn recently(used: u64, now: u64) -> bool {
        used + 1 >= now
    }

    /// Whether the frame just built actually asked for this picture.
    ///
    /// Held is not drawn. A picture stays in the atlas until something needs
    /// the room, so "is it in there" answers yes for every emoji the reader
    /// has scrolled past today -- and asked that question, a moving picture
    /// nobody can see goes on costing an upload and a woken window for each
    /// frame of its loop.
    ///
    /// The mark that `image` leaves is the real answer: it names the frame the
    /// picture was last asked for in. One frame of slack, because a picture is
    /// put in before the frame that draws it.
    ///
    /// Without marking it used, which is the point of it being separate from
    /// `image`: this is asked *about* a picture rather than for one, and a
    /// mark made here would keep a picture alive on the strength of nothing
    /// but the asking.
    pub fn drawn(&self, key: &str) -> bool {
        let sheet = match Sheet::of(key) {
            Sheet::Faces => &self.faces,
            _ => &self.pictures,
        };
        sheet
            .images
            .get(key)
            .and_then(|held| held.as_ref())
            .is_some_and(|held| Self::recently(held.used, self.now))
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
    /// The next frame of a picture that moves, into the slot it already has.
    ///
    /// Answers whether the picture was there to be written over. It will not
    /// be once the sheet has reclaimed it, and a frame of a picture nobody is
    /// drawing is nothing to put anywhere.
    pub fn put_frame(
        &mut self,
        gpu: &Gpu,
        key: &str,
        rgba: &[u8],
        width: u32,
        height: u32,
    ) -> bool {
        match Sheet::of(key) {
            Sheet::Faces => self.faces.replace(gpu, key, rgba, width, height),
            _ => self.pictures.replace(gpu, key, rgba, width, height),
        }
    }

    pub fn put_image(
        &mut self,
        gpu: &Gpu,
        key: &str,
        rgba: &[u8],
        width: u32,
        height: u32,
    ) -> Option<Slot> {
        let now = self.now;
        match Sheet::of(key) {
            Sheet::Faces => self.faces.put(gpu, key, rgba, width, height, now),
            _ => self.pictures.put(gpu, key, rgba, width, height, now),
        }
    }

    /// The slot for a glyph, rasterising it the first time it is asked for.
    ///
    /// `None` means the glyph has no pixels -- a space, or something the font
    /// draws as nothing -- which is a real answer and is cached as one.
    pub fn slot(
        &mut self,
        gpu: &Gpu,
        fonts: &mut Fonts,
        cache: &mut SwashCache,
        key: CacheKey,
        subpixel: bool,
    ) -> Option<Slot> {
        let asked = Rasterised { key, subpixel };
        if let Some(held) = self.slots.get(&asked) {
            return *held;
        }
        // Through `cache` for a flat mask, which is what it offers: cosmic-text
        // renders `Format::Alpha` and gives no way to ask for anything else. A
        // subpixel mask is the same rasterisation with a different format, so
        // that path is spelled out here rather than done without.
        let image = match subpixel {
            false => cache.get_image_uncached(fonts.system_mut(), key),
            true => subpixel_image(fonts, &mut self.scaler, key),
        };
        let Some(image) = image else {
            self.slots.insert(asked, None);
            return None;
        };
        let width = image.placement.width;
        let height = image.placement.height;
        if width == 0 || height == 0 {
            self.slots.insert(asked, None);
            return None;
        }

        // A mask becomes white with its coverage in the alpha, so the vertex
        // colour decides what shade the letter is. A colour bitmap is kept
        // exactly as it is.
        let coloured = matches!(image.content, SwashContent::Color);
        let banded = matches!(image.content, SwashContent::SubpixelMask);
        let pixels: Vec<u8> = if coloured {
            image.data.clone()
        } else if banded {
            // Four bytes to a pixel already, and each of the first three is a
            // channel's own coverage -- filtered across its neighbours first,
            // then curved like any other coverage. The fourth becomes the most
            // any channel is covered, which is what anything reading this as
            // one number wants: the alpha the window blends by.
            let filtered = spread(&image.data, width as usize, height as usize);
            filtered
                .as_chunks::<3>()
                .0
                .iter()
                .flat_map(|texel| {
                    let (r, g, b) = (weighted(texel[0]), weighted(texel[1]), weighted(texel[2]));
                    [r, g, b, r.max(g).max(b)]
                })
                .collect()
        } else {
            image
                .data
                .iter()
                .flat_map(|coverage| [255, 255, 255, weighted(*coverage)])
                .collect()
        };

        let Some(slot) = self.letters.pen.reserve(width, height) else {
            // Out of room. Cached as absent rather than retried every frame:
            // a missing glyph is a visible gap, and a rasterise per frame is a
            // stutter that would hide the cause. Nothing reclaims here -- see
            // the note at the top on why this sheet is sized not to need to.
            eprintln!("no room in letters for a {width}x{height} glyph");
            self.slots.insert(asked, None);
            return None;
        };
        self.letters
            .write(gpu, slot.x, slot.y, width, height, &pixels);
        let slot = Slot {
            left: image.placement.left,
            top: image.placement.top,
            colour: coloured,
            subpixel: banded,
            ..slot
        };
        self.slots.insert(asked, Some(slot));
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

    /// Held is not drawn, and a moving picture asks the difference.
    ///
    /// A picture stays in the atlas until something needs the room, so "is it
    /// in there" answers yes for every emoji a reader has scrolled past today.
    /// A loop advanced on that answer costs an upload and a woken window for
    /// each of its frames while nobody can see it -- which, with two animated
    /// emoji fetched for a channel and neither on screen, is what it did.
    #[test]
    fn only_what_the_last_frame_asked_for_counts_as_drawn() {
        let drawn = Atlas::recently;
        assert!(drawn(10, 10), "asked for in the frame being built");
        assert!(
            drawn(9, 10),
            "asked for in the one before, and put in before that"
        );
        assert!(
            !drawn(8, 10),
            "two frames back is a picture nothing is drawing"
        );
        // And what is kept is a far longer memory than what is drawn: the two
        // answer different questions and must not be the same number.
        assert!(9 + KEEP >= 10 && !drawn(6, 10));
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
