//! Where the time in shaping a channel actually goes.
//!
//! Opening a channel of crash reports shapes a quarter of a million characters
//! and stalls the window for whole seconds, which is more than the number of
//! characters seems to deserve. This shapes the same text several ways and
//! prints what each costs, so the answer is a measurement rather than a guess
//! about which line of `line_count` is the expensive one.

use cosmic_text::{Attrs, Buffer, Family, Metrics, Shaping};
use matterless_layout::Fonts;
use std::time::Instant;

/// Text shaped like a message list: ordinary prose, a stack trace, a path, a
/// line of code. Repeated to the size of a real channel.
fn corpus() -> Vec<String> {
    let samples = [
        "Hello tout le monde, je me demandais si ca vous interesserait de refaire une passe groupee de posts artstations pour toutes nos dernieres saisons ?",
        r"E:\Dev\Curiosity_Main\Engine\Binaries\Win64\UnrealEditor-D3D12RHI.dll",
        "FD3D12DynamicRHI::ProcessInterruptQueue::'2'::<T>::operator()",
        "Total occurences: 68 | Occurences in the last 24h: 1 | Users impacted: 12",
        "the build is green again after the shader recompile, thanks for the quick look",
        r"C:\Windows\System32\KERNELBASE.dll RaiseException FRunnableThreadWin::Run",
    ];
    let mut out = Vec::new();
    while out.len() < 1200 {
        for one in samples {
            out.push(one.to_string());
        }
    }
    out
}

const WIDTH: f32 = 660.0;

fn main() {
    let lines = corpus();
    let chars: usize = lines.iter().map(|line| line.chars().count()).sum();
    println!("{} paragraphs, {chars} characters\n", lines.len());
    let metrics = Metrics::new(14.0, 21.0);

    let plain = Attrs::new();
    let mono = Attrs::new().family(Family::Monospace);

    fresh("a fresh buffer each time, advanced", &lines, chars, metrics, &plain, Shaping::Advanced);
    kept("one buffer kept, advanced", &lines, chars, metrics, &plain, Shaping::Advanced);
    fresh("a fresh buffer each time, basic", &lines, chars, metrics, &plain, Shaping::Basic);
    fresh("a fresh buffer each time, monospace", &lines, chars, metrics, &mono, Shaping::Advanced);

    // The monospace family resolved once, by name, instead of asked for as a
    // generic every time: if this is the difference, the cost is the lookup
    // and not the shaping.
    let named = fonts_mono_name();
    let by_name = Attrs::new().family(Family::Name(&named));
    fresh(
        &format!("monospace by name ({named})"),
        &lines,
        chars,
        metrics,
        &by_name,
        Shaping::Advanced,
    );

    // The same paragraph over and over, to show what the caches hold.
    let one = vec![lines[0].clone(); lines.len()];
    let over = one.iter().map(|line| line.chars().count()).sum();
    fresh("one paragraph, repeated", &one, over, metrics, &plain, Shaping::Advanced);

    // And the cost of the buffer with no text in it at all, which is the
    // per-paragraph overhead the channel pays four hundred times over.
    let mut fonts = Fonts::new();
    let began = Instant::now();
    for _ in &lines {
        let buffer = Buffer::new(fonts.system_mut(), metrics);
        std::hint::black_box(buffer.size());
    }
    say("an empty buffer, made and dropped", began, 0);
}

/// What the system actually gives back for `Family::Monospace`.
fn fonts_mono_name() -> String {
    let mut fonts = Fonts::new();
    let system = fonts.system_mut();
    let query = cosmic_text::fontdb::Query {
        families: &[cosmic_text::fontdb::Family::Monospace],
        ..Default::default()
    };
    system
        .db()
        .query(&query)
        .and_then(|id| system.db().face(id).map(|face| face.families[0].0.clone()))
        .unwrap_or_else(|| "Consolas".to_string())
}

fn fresh(
    what: &str,
    lines: &[String],
    chars: usize,
    metrics: Metrics,
    attrs: &Attrs<'_>,
    shaping: Shaping,
) {
    let mut fonts = Fonts::new();
    let began = Instant::now();
    for line in lines {
        let mut buffer = Buffer::new(fonts.system_mut(), metrics);
        let mut buffer = buffer.borrow_with(fonts.system_mut());
        buffer.set_size(Some(WIDTH), None);
        buffer.set_text(line, attrs, shaping, None);
        buffer.shape_until_scroll(false);
        std::hint::black_box(buffer.layout_runs().count());
    }
    say(what, began, chars);
}

fn kept(
    what: &str,
    lines: &[String],
    chars: usize,
    metrics: Metrics,
    attrs: &Attrs<'_>,
    shaping: Shaping,
) {
    let mut fonts = Fonts::new();
    let began = Instant::now();
    let mut buffer = Buffer::new(fonts.system_mut(), metrics);
    for line in lines {
        let mut buffer = buffer.borrow_with(fonts.system_mut());
        buffer.set_size(Some(WIDTH), None);
        buffer.set_text(line, attrs, shaping, None);
        buffer.shape_until_scroll(false);
        std::hint::black_box(buffer.layout_runs().count());
    }
    say(what, began, chars);
}

fn say(what: &str, began: Instant, chars: usize) {
    let took = began.elapsed();
    if chars == 0 {
        println!("{what:<38} {:>7}ms", took.as_millis());
        return;
    }
    println!(
        "{what:<38} {:>7}ms  {:>10.0} chars/s",
        took.as_millis(),
        chars as f64 / took.as_secs_f64()
    );
}
