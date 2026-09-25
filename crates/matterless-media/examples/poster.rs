//! Makes a video's poster, as the message list would, and says how it went.
//!
//!     cargo run -p matterless-media --example poster -- <file> [width] [height]

fn main() {
    let mut args = std::env::args().skip(1);
    let Some(path) = args.next() else {
        eprintln!("usage: poster <file> [width] [height]");
        return;
    };
    let width: u32 = args
        .next()
        .and_then(|said| said.parse().ok())
        .unwrap_or(420);
    let height: u32 = args
        .next()
        .and_then(|said| said.parse().ok())
        .unwrap_or(236);
    let began = std::time::Instant::now();
    match matterless_media::poster(std::path::Path::new(&path), width, height) {
        Ok(rgba) => println!(
            "poster {width}x{height}, {} bytes, in {:?}",
            rgba.len(),
            began.elapsed()
        ),
        Err(why) => println!("no poster: {why}"),
    }
}
