//! What a width change costs: shaping again, or only wrapping again.
//!
//! Against what messages actually hold -- hard line breaks, emoji out of the
//! bundled face, and the inline styles markdown turns into spans -- because a
//! paragraph of plain Latin text is the one case where everything is cheap.

use cosmic_text::{Attrs, Buffer, Family, Metrics, Shaping, Weight};
use matterless_layout::Fonts;

const PLAIN: &str = "J'ai pas mal changer le tool de SDF pour mieux gerer les lignes a la \
     distance, pour eviter le probleme de lignes qui deviennent trop proche a la \
     distance j'ai rajouter un system qui va supprimer des lignes dans les mip quand \
     elle devienne trop proche.";

const BROKEN: &str = "Hello,\nj'ai pas mal changer le tool de SDF pour mieux gerer les \
     lignes a la distance.\n\nOn est donc maintenant capable de garder des lignes sharp \
     meme de loin.\nje vous ai mis quelques screen de comparaison sur le channel.";

const WITH_EMOJI: &str = "Passez une bonne semaine ! \u{1F44B}\u{1F3FC} \u{2728} Merci a tous \
     \u{1F64F} pour le travail sur la release \u{1F680}, et bravo \u{1F44D} pour le coup de \
     main sur les crashs \u{1F41B} de la semaine derniere \u{1F602}\u{1F389}";

fn rounds() -> u32 {
    120
}

/// Shapes and wraps from nothing, alternating between two widths.
fn fresh(fonts: &mut Fonts, text: &str, rich: bool) -> std::time::Duration {
    let began = std::time::Instant::now();
    for round in 0..rounds() {
        let width = 620.0 - (round % 2) as f32 * 320.0;
        let mut buffer = Buffer::new(fonts.system_mut(), Metrics::new(14.0, 21.0));
        let mut shaped = buffer.borrow_with(fonts.system_mut());
        shaped.set_size(Some(width), None);
        match rich {
            true => set_rich(&mut shaped, text),
            false => shaped.set_text(text, &Attrs::new(), Shaping::Advanced, None),
        }
        shaped.shape_until_scroll(false);
        std::hint::black_box(shaped.layout_runs().count());
    }
    began.elapsed() / rounds()
}

/// Keeps the text and changes only the width.
fn rewrapped(fonts: &mut Fonts, text: &str, rich: bool) -> std::time::Duration {
    let mut buffer = Buffer::new(fonts.system_mut(), Metrics::new(14.0, 21.0));
    {
        let mut shaped = buffer.borrow_with(fonts.system_mut());
        match rich {
            true => set_rich(&mut shaped, text),
            false => shaped.set_text(text, &Attrs::new(), Shaping::Advanced, None),
        }
    }
    let began = std::time::Instant::now();
    for round in 0..rounds() {
        let width = 620.0 - (round % 2) as f32 * 320.0;
        let mut shaped = buffer.borrow_with(fonts.system_mut());
        shaped.set_size(Some(width), None);
        shaped.shape_until_scroll(false);
        std::hint::black_box(shaped.layout_runs().count());
    }
    began.elapsed() / rounds()
}

/// What markdown comes out as: one line, several styles.
fn set_rich(shaped: &mut cosmic_text::BorrowedWithFontSystem<'_, Buffer>, text: &str) {
    let bold = Attrs::new().weight(Weight::BOLD);
    let mono = Attrs::new().family(Family::Monospace);
    let plain = Attrs::new();
    let third = text.len() / 3;
    let (one, rest) = text.split_at(text.floor_char_boundary(third));
    let (two, three) = rest.split_at(rest.floor_char_boundary(third));
    shaped.set_rich_text(
        [(one, bold), (two, mono), (three, plain.clone())],
        &plain,
        Shaping::Advanced,
        None,
    );
}

fn main() {
    let mut fonts = Fonts::new();
    for (name, text, rich) in [
        ("a paragraph", PLAIN, false),
        ("hard line breaks", BROKEN, false),
        ("emoji", WITH_EMOJI, false),
        ("markdown's spans", PLAIN, true),
    ] {
        let from_nothing = fresh(&mut fonts, text, rich);
        let again = rewrapped(&mut fonts, text, rich);
        println!(
            "{name:18} shaped {from_nothing:>10.1?}   wrapped again {again:>9.1?}   {:5.1}%",
            again.as_secs_f64() / from_nothing.as_secs_f64() * 100.0
        );
    }
}
