//! A sheet of marks, drawn, so one can be chosen by looking at it.
//!
//! The constants in `marks` are four hex digits each and the bundled font holds
//! the whole of lucide behind them, so picking one by name out of a list is
//! picking blind -- the first mark this rail wore was `UNREAD`, which is a
//! picture of an envelope, sitting directly above a button that is also an
//! envelope.
//!
//!     cargo run -p matterless-view --example what_marks_look_like
//!
//! Writes `marks.png` beside the target directory: the mark, at the size the
//! interface draws it and again at four times, with its lucide name under it.
//! Codepoints come from the command line as bare hex, or the sheet falls back
//! to everything `marks` already names.

use cosmic_text::{Attrs, Buffer, Family, Metrics, Shaping, SwashCache};
use matterless_layout::Fonts;

/// The tile the interface draws a rail mark on, so a candidate is judged at
/// the size it will actually be.
const TILE: u32 = 30;
const SHOWN: f32 = 15.0;
const BIG: f32 = 60.0;
const ROW: u32 = 96;
const COLUMN: u32 = 220;

fn main() {
    let asked: Vec<(String, char)> = std::env::args()
        .skip(1)
        .filter_map(|arg| {
            let code = u32::from_str_radix(arg.trim_start_matches("U+"), 16).ok()?;
            Some((arg.clone(), char::from_u32(code)?))
        })
        .collect();

    let marks: Vec<(String, char)> = if asked.is_empty() { named() } else { asked };

    let mut fonts = Fonts::new();
    let mut cache = SwashCache::new();
    let wide = COLUMN * 2;
    let tall = ROW * (marks.len() as u32).div_ceil(2).max(1) + 20;
    let mut pixels = vec![0u8; (wide * tall * 4) as usize];
    // The app's own ground, so a mark is judged against what it will sit on.
    let (ground, _) = pixels.as_chunks_mut::<4>();
    for pixel in ground {
        *pixel = [12, 18, 24, 255];
    }

    for (at, (label, mark)) in marks.iter().enumerate() {
        let column = (at as u32 % 2) * COLUMN;
        let row = (at as u32 / 2) * ROW + 10;
        // The tile, as the rail draws it.
        fill(
            &mut pixels,
            wide,
            column + 10,
            row + 10,
            TILE,
            TILE,
            [27, 39, 52, 255],
        );
        draw(
            &mut fonts,
            &mut cache,
            &mut pixels,
            wide,
            tall,
            &mark.to_string(),
            SHOWN,
            column + 10 + TILE / 2,
            row + 10 + TILE / 2,
        );
        draw(
            &mut fonts,
            &mut cache,
            &mut pixels,
            wide,
            tall,
            &mark.to_string(),
            BIG,
            column + 80,
            row + 25,
        );
        say(
            &mut fonts,
            &mut cache,
            &mut pixels,
            wide,
            tall,
            label,
            column + 10,
            row + 52,
        );
    }

    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../target")
        .join("marks.png");
    let file = std::fs::File::create(&path).expect("create the sheet");
    let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), wide, tall);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder
        .write_header()
        .expect("header")
        .write_image_data(&pixels)
        .expect("pixels");
    println!("{} marks -> {}", marks.len(), path.display());
}

/// Every mark `marks` already names, paired with what it is called there.
fn named() -> Vec<(String, char)> {
    use matterless_layout::marks as m;
    [
        ("THREADS", m::THREADS),
        ("PINNED", m::PINNED),
        ("SAVED", m::SAVED),
        ("ADD_PEOPLE", m::ADD_PEOPLE),
        ("BELL", m::BELL),
        ("BELL_OFF", m::BELL_OFF),
        ("LEAVE", m::LEAVE),
        ("CLOSE", m::CLOSE),
        ("ATTACH", m::ATTACH),
        ("REACT", m::REACT),
        ("REPLY", m::REPLY),
        ("MORE", m::MORE),
        ("UNREAD", m::UNREAD),
        ("TO_UNREAD", m::TO_UNREAD),
        ("DIRECTS", m::DIRECTS),
        ("FAVOURITE", m::FAVOURITE),
        ("FOLDER", m::FOLDER),
        ("LINK", m::LINK),
        ("NEW", m::NEW),
        ("BACK", m::BACK),
        ("NEXT", m::NEXT),
        ("SAVE_FILE", m::SAVE_FILE),
        ("DELETE", m::DELETE),
        ("EDIT", m::EDIT),
        ("COPY", m::COPY),
        ("REMIND", m::REMIND),
        ("FORWARD", m::FORWARD),
        ("SEARCH", m::SEARCH),
    ]
    .into_iter()
    .filter_map(|(name, mark)| Some((name.to_string(), mark.chars().next()?)))
    .collect()
}

fn fill(pixels: &mut [u8], wide: u32, x: u32, y: u32, w: u32, h: u32, colour: [u8; 4]) {
    for row in y..(y + h) {
        for column in x..(x + w) {
            let at = ((row * wide + column) * 4) as usize;
            if at + 4 <= pixels.len() {
                pixels[at..at + 4].copy_from_slice(&colour);
            }
        }
    }
}

/// One mark, centred on `(x, y)`, in the icon font at `size`.
#[allow(clippy::too_many_arguments)]
fn draw(
    fonts: &mut Fonts,
    cache: &mut SwashCache,
    pixels: &mut [u8],
    wide: u32,
    tall: u32,
    mark: &str,
    size: f32,
    x: u32,
    y: u32,
) {
    ink(
        fonts,
        cache,
        pixels,
        wide,
        tall,
        mark,
        size,
        Some(matterless_layout::marks::FAMILY),
        x,
        y,
        true,
    );
}

/// A name, in the text font, left-aligned.
#[allow(clippy::too_many_arguments)]
fn say(
    fonts: &mut Fonts,
    cache: &mut SwashCache,
    pixels: &mut [u8],
    wide: u32,
    tall: u32,
    said: &str,
    x: u32,
    y: u32,
) {
    ink(
        fonts, cache, pixels, wide, tall, said, 13.0, None, x, y, false,
    );
}

/// Shapes and rasterises straight onto the buffer.
///
/// `Run::mark` shapes at twice the size and draws the result down; this is a
/// sheet for looking at rather than a frame, so it rasterises at the size
/// asked for and leaves it there.
#[allow(clippy::too_many_arguments)]
fn ink(
    fonts: &mut Fonts,
    cache: &mut SwashCache,
    pixels: &mut [u8],
    wide: u32,
    tall: u32,
    text: &str,
    size: f32,
    family: Option<&str>,
    x: u32,
    y: u32,
    centred: bool,
) {
    let mut buffer = Buffer::new(fonts.system_mut(), Metrics::new(size, size * 1.4));
    let mut shaped = buffer.borrow_with(fonts.system_mut());
    let attrs = match family {
        Some(name) => Attrs::new().family(Family::Name(name)),
        None => Attrs::new(),
    };
    shaped.set_text(text, &attrs, Shaping::Advanced, None);
    shaped.shape_until_scroll(false);

    let placed: Vec<_> = shaped
        .layout_runs()
        .flat_map(|run| {
            let line_y = run.line_y;
            run.glyphs.iter().map(move |glyph| (glyph.clone(), line_y))
        })
        .collect();

    for (glyph, line_y) in placed {
        let physical = glyph.physical((0.0, 0.0), 1.0);
        let Some(image) = cache.get_image_uncached(fonts.system_mut(), physical.cache_key) else {
            continue;
        };
        let (left, top) = match centred {
            true => (
                x as i32 - image.placement.width as i32 / 2,
                y as i32 - image.placement.height as i32 / 2,
            ),
            false => (
                x as i32 + physical.x + image.placement.left,
                y as i32 + line_y as i32 - image.placement.top,
            ),
        };
        for (at, alpha) in image.data.iter().enumerate() {
            let column = left + (at as u32 % image.placement.width.max(1)) as i32;
            let row = top + (at as u32 / image.placement.width.max(1)) as i32;
            if column < 0 || row < 0 || column >= wide as i32 || row >= tall as i32 {
                continue;
            }
            let into = ((row as u32 * wide + column as u32) * 4) as usize;
            let over = u32::from(*alpha);
            for channel in 0..3 {
                let was = u32::from(pixels[into + channel]);
                let ink = 227u32;
                pixels[into + channel] = ((ink * over + was * (255 - over)) / 255) as u8;
            }
        }
    }
}
