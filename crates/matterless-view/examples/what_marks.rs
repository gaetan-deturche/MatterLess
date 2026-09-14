//! What one of the interface's marks actually measures at a given size.
//!
//! An icon family fills the size it is asked for; the symbol font it replaced
//! drew at about two thirds of it. Every size in the interface came down when
//! the family changed, and this is what said by how much -- the marks arrived
//! a size too big and guessing at the correction would have been another
//! round of looking at screenshots.
//!
//! `Run::mark` shapes at twice the size and draws the result down, so what
//! lands on screen is half of what is rasterised. Both are reported.

use cosmic_text::{Attrs, Buffer, Metrics, Shaping, SwashCache};

fn main() {
    let mut fonts = matterless_layout::Fonts::new();
    let mut cache = SwashCache::new();
    // What a mark actually measures at each size it is asked for. `Run::mark`
    // shapes at twice and draws down, so the number on screen is half of what
    // comes back here.
    for size in [11.0f32, 12.0, 13.0, 14.0, 15.0, 16.0, 18.0, 20.0, 22.0] {
        let mut widest = 0;
        let mut tallest = 0;
        for one in [
            matterless_layout::marks::PINNED,
            matterless_layout::marks::BELL,
            matterless_layout::marks::THREADS,
            matterless_layout::marks::ATTACH,
        ] {
            let mut buffer = Buffer::new(fonts.system_mut(), Metrics::new(size * 2.0, size * 2.8));
            let mut shaped = buffer.borrow_with(fonts.system_mut());
            shaped.set_text(
                one,
                &Attrs::new().family(cosmic_text::Family::Name(matterless_layout::marks::FAMILY)),
                Shaping::Advanced,
                None,
            );
            shaped.shape_until_scroll(false);
            let key = shaped
                .layout_runs()
                .flat_map(|run| run.glyphs.iter())
                .map(|glyph| glyph.physical((0.0, 0.0), 1.0).cache_key)
                .next();
            if let Some(image) = key.and_then(|key| cache.get_image_uncached(fonts.system_mut(), key))
            {
                widest = widest.max(image.placement.width);
                tallest = tallest.max(image.placement.height);
            }
        }
        println!(
            "Run::mark({size:>4}) -> rasterised {widest}x{tallest}, drawn {}x{}",
            widest / 2,
            tallest / 2
        );
    }
}
