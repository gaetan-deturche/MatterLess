//! Which characters this machine's fonts draw as line art, and which as a
//! coloured bitmap.
//!
//! The interface's own marks have to be the first kind. A control is not
//! content: a row of little pictures across the top of the strip competes with
//! the conversation for the eye. The variation selector that is supposed to
//! ask for the monochrome form (U+FE0E) is ignored by this font stack, so the
//! only way to get one is to pick a character that has no coloured form at
//! all -- and the only way to know which those are is to rasterise them and
//! look at what came back.
//!
//! `colour` is a bitmap out of the emoji font. `line art` is a coverage mask,
//! which the interface tints like a letter. `nothing` is a character no font
//! here has, which would draw as a hole.

use cosmic_text::{Attrs, Buffer, Metrics, Shaping, SwashCache, SwashContent};

fn main() {
    let mut fonts = matterless_layout::Fonts::new();
    let mut cache = SwashCache::new();

    // Every mark the interface uses, and the candidates for the ones that are
    // still coloured.
    let wanted: Vec<(&str, Vec<char>)> = vec![
        ("threads", vec!['\u{1f9f5}', '\u{2630}', '\u{2261}']),
        ("pinned", vec!['\u{1f4cc}', '\u{1f588}', '\u{2691}', '\u{2316}', '\u{26b2}']),
        ("saved", vec!['\u{1f516}', '\u{2690}', '\u{2691}', '\u{2605}', '\u{2606}']),
        ("add", vec!['\u{1f464}', '\u{2295}', '\u{263b}', '\u{26b2}', '\u{2687}']),
        ("bell", vec!['\u{1f514}', '\u{1f56d}', '\u{237e}', '\u{2407}', '\u{1f56c}']),
        ("bell off", vec!['\u{1f515}', '\u{2298}', '\u{29b8}', '\u{2718}']),
        ("leave", vec!['\u{1f6aa}', '\u{23cf}', '\u{21aa}', '\u{238b}', '\u{21b6}', '\u{2b8c}']),
        ("react", vec!['\u{263a}', '\u{263b}', '\u{1f642}', '\u{2b58}']),
        ("attach", vec!['\u{1f4ce}', '\u{1f587}', '\u{1f5dc}', '\u{2398}', '\u{1f5ce}', '\u{1f5c8}']),
        ("folder", vec!['\u{1f5c0}', '\u{1f4c1}', '\u{1f5bf}', '\u{25b1}']),
        ("link", vec!['\u{1f517}', '\u{26d3}', '\u{29c9}', '\u{2384}', '\u{1f5d7}']),
    ];

    for (what, candidates) in wanted {
        println!("{what}:");
        for one in candidates {
            let mut buffer = Buffer::new(fonts.system_mut(), Metrics::new(15.0, 20.0));
            let mut shaped = buffer.borrow_with(fonts.system_mut());
            shaped.set_text(&one.to_string(), &Attrs::new(), Shaping::Advanced, None);
            shaped.shape_until_scroll(false);
            let key = shaped
                .layout_runs()
                .flat_map(|run| run.glyphs.iter())
                .map(|glyph| glyph.physical((0.0, 0.0), 1.0).cache_key)
                .next();
            let said = match key.and_then(|key| cache.get_image_uncached(fonts.system_mut(), key)) {
                None => "nothing".to_string(),
                Some(image) if image.placement.width == 0 => "nothing".to_string(),
                Some(image) => {
                    let kind = match image.content {
                        SwashContent::Color => "colour",
                        _ => "line art",
                    };
                    format!(
                        "{kind} {}x{}",
                        image.placement.width, image.placement.height
                    )
                }
            };
            println!("  U+{:04X} {one}  {said}", one as u32);
        }
    }
}
