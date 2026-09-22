//! The same emoji from every candidate face, at the sizes the window draws.

use cosmic_text::{Attrs, Buffer, Family, Metrics, Shaping, SwashCache};
use matterless_layout::Fonts;

const SHOWN: [&str; 8] = [
    "\u{1F44D}",
    "\u{2764}\u{FE0F}",
    "\u{1F44B}",
    "\u{1F680}",
    "\u{1F602}",
    "\u{1F389}",
    "\u{1F41B}",
    "\u{2705}",
];
const SIZES: [f32; 2] = [13.0, 26.0];
/// The room each emoji is given in the sheet, and the gap under a row.
const CELL: u32 = 34;
const ROW: u32 = 34;

fn main() {
    let mut fonts = Fonts::new();
    let mut cache = SwashCache::new();
    let mut families: Vec<String> = vec![matterless_layout::EMOJI_FAMILY.to_string()];
    for path in std::env::args().skip(1) {
        let bytes = std::fs::read(&path).expect("a font");
        let before: Vec<_> = fonts
            .system_mut()
            .db()
            .faces()
            .map(|face| face.id)
            .collect();
        fonts.system_mut().db_mut().load_font_data(bytes);
        let named: Vec<String> = fonts
            .system_mut()
            .db()
            .faces()
            .filter(|face| !before.contains(&face.id))
            .map(|face| face.families[0].0.clone())
            .collect();
        println!("{path} brought {named:?}");
        if let Some(name) = named.first() {
            families.push(name.clone());
        }
    }

    // Which face an emoji reaches when nothing asks for one by name, which
    // is how every message in this window shapes.
    for emoji in SHOWN {
        let mut buffer = Buffer::new(fonts.system_mut(), Metrics::new(13.0, 18.0));
        let mut shaped = buffer.borrow_with(fonts.system_mut());
        shaped.set_size(Some(f32::MAX), None);
        shaped.set_text(emoji, &Attrs::new(), Shaping::Advanced, None);
        shaped.shape_until_scroll(false);
        let ids: Vec<_> = shaped
            .layout_runs()
            .flat_map(|line| line.glyphs.iter())
            .map(|glyph| glyph.font_id)
            .collect();
        for id in ids {
            let family = fonts
                .system_mut()
                .db()
                .face(id)
                .map(|face| face.families[0].0.clone())
                .unwrap_or_default();
            println!("fallback for {emoji}: {family}");
        }
    }

    let width = CELL * SHOWN.len() as u32;
    let height = ROW * (SIZES.len() * families.len()) as u32;
    let mut pixels = vec![0u8; (width * height * 4) as usize];
    let mut at_row = 0;
    for family in &families {
        for size in SIZES {
            for (column, emoji) in SHOWN.iter().enumerate() {
                let mut buffer = Buffer::new(fonts.system_mut(), Metrics::new(size, size * 1.4));
                let mut shaped = buffer.borrow_with(fonts.system_mut());
                shaped.set_size(Some(f32::MAX), None);
                let attrs = Attrs::new().family(Family::Name(family));
                shaped.set_text(emoji, &attrs, Shaping::Advanced, None);
                shaped.shape_until_scroll(false);
                let keys: Vec<_> = shaped
                    .layout_runs()
                    .flat_map(|line| line.glyphs.iter())
                    .map(|glyph| glyph.physical((0.0, 0.0), 1.0).cache_key)
                    .collect();
                for key in keys {
                    let Some(image) = cache.get_image_uncached(fonts.system_mut(), key) else {
                        continue;
                    };
                    println!(
                        "{emoji} in {family} at {size}: {:?} {}x{}",
                        image.content, image.placement.width, image.placement.height
                    );
                    let left = column as u32 * CELL + 3;
                    let top = at_row * ROW + 4;
                    for y in 0..image.placement.height {
                        for x in 0..image.placement.width {
                            let from = ((y * image.placement.width + x) * 4) as usize;
                            let (to_x, to_y) = (left + x, top + y);
                            if to_x >= width || to_y >= height {
                                continue;
                            }
                            let to = ((to_y * width + to_x) * 4) as usize;
                            if image.data.len() < from + 4 {
                                continue;
                            }
                            pixels[to..to + 4].copy_from_slice(&image.data[from..from + 4]);
                        }
                    }
                }
            }
            println!("row {at_row}: {family} at {size}");
            at_row += 1;
        }
    }
    image::save_buffer(
        "emoji-sheet.png",
        &pixels,
        width,
        height,
        image::ColorType::Rgba8,
    )
    .expect("wrote it");
    println!("wrote emoji-sheet.png, {width}x{height}");
}
