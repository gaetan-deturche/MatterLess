//! Plays a video for a few seconds without a window, and says what came out:
//! whether ffmpeg loaded, how big the pictures are, how many arrived and when.
//!
//!     cargo run -p matterless-media --example play -- <file> [seconds] [--sound]
//!
//! Silent unless `--sound` is given: a check run should not play somebody's
//! video out loud on their machine.

use std::time::{Duration, Instant};
use windows::Win32::Graphics::Direct3D::{D3D_DRIVER_TYPE_HARDWARE, D3D_FEATURE_LEVEL_11_0};
use windows::Win32::Graphics::Direct3D11::{
    D3D11_CREATE_DEVICE_VIDEO_SUPPORT, D3D11_SDK_VERSION, D3D11CreateDevice, ID3D11Device,
};

fn main() {
    let mut args = std::env::args().skip(1);
    let Some(path) = args.next() else {
        eprintln!("usage: play <file> [seconds]");
        return;
    };
    let rest: Vec<String> = args.collect();
    let seconds: f64 = rest
        .iter()
        .find_map(|said| said.parse().ok())
        .unwrap_or(3.0);
    let sound = rest.iter().any(|said| said == "--sound");
    // `--seek=<seconds>`: jumps there a second in, and says where it landed.
    let seek: Option<f64> = rest
        .iter()
        .find_map(|said| said.strip_prefix("--seek=").and_then(|at| at.parse().ok()));
    if let Err(why) = matterless_media::available() {
        eprintln!("no ffmpeg: {why}");
        return;
    }
    let mut device: Option<ID3D11Device> = None;
    unsafe {
        D3D11CreateDevice(
            None,
            D3D_DRIVER_TYPE_HARDWARE,
            Default::default(),
            D3D11_CREATE_DEVICE_VIDEO_SUPPORT,
            Some(&[D3D_FEATURE_LEVEL_11_0]),
            D3D11_SDK_VERSION,
            Some(&mut device),
            None,
            None,
        )
    }
    .expect("a device");
    let device = device.expect("a device");
    // `--drag` / `--drag-back`: a second's drag across the video, one position
    // a frame, then let go -- what the reader saw on the way: how many
    // different pictures, and the widest jump in the video between two.
    if let Some(back) = rest.iter().find_map(|said| match said.as_str() {
        "--drag" => Some(false),
        "--drag-back" => Some(true),
        _ => None,
    }) {
        let player =
            matterless_media::Player::open(std::path::Path::new(&path), &device).expect("opened");
        player.set_muted(true);
        while player.due().is_none() {
            std::thread::sleep(Duration::from_millis(2));
        }
        // The small pictures decoded ahead, which a drag uses when near.
        let filling = Instant::now();
        while player.previewed() + 0.5 < player.duration()
            && filling.elapsed() < Duration::from_secs(30)
        {
            std::thread::sleep(Duration::from_millis(10));
        }
        println!(
            "previews reach {:.2}s of {:.2}s after {:?}",
            player.previewed(),
            player.duration(),
            filling.elapsed()
        );
        let (start, end) = if back { (12.0, 2.0) } else { (2.0, 12.0) };
        if back {
            player.seek(start);
            while player.due().is_none() {
                std::thread::sleep(Duration::from_millis(2));
            }
        }
        let began = Instant::now();
        let mut seen: Vec<f64> = Vec::new();
        for step in 0..60 {
            let to = start + (end - start) * step as f64 / 59.0;
            if let Some(preview) = player.scrub(to) {
                seen.push(preview.pts);
            }
            let until = began + Duration::from_millis(16 * (step + 1));
            while Instant::now() < until {
                if let Some(frame) = player.due() {
                    seen.push(frame.pts);
                }
                std::thread::sleep(Duration::from_millis(1));
            }
        }
        let released = Instant::now();
        player.seek(end);
        let mut landed = seen.last().copied().unwrap_or(f64::NAN);
        while player.aiming() && released.elapsed() < Duration::from_secs(5) {
            if let Some(frame) = player.due() {
                landed = frame.pts;
            }
            std::thread::sleep(Duration::from_millis(1));
        }
        if player.aiming() {
            println!("never landed");
        }
        let mut distinct = seen.clone();
        distinct.dedup_by(|a, b| (*a - *b).abs() < 1e-6);
        let widest = distinct
            .windows(2)
            .map(|pair| (pair[1] - pair[0]).abs())
            .fold(0.0f64, f64::max);
        println!(
            "{} drag: {} pictures, {} different, widest jump {widest:.2}s of video; let go, picture at {landed:.3} after {:?}",
            if back { "backward" } else { "forward" },
            seen.len(),
            distinct.len(),
            released.elapsed()
        );
        return;
    }
    let opened = Instant::now();
    let player =
        matterless_media::Player::open(std::path::Path::new(&path), &device).expect("opened");
    player.set_muted(!sound);
    player.play();
    let mut shown = 0;
    let mut first = None;
    let mut sought = false;
    let mut landed = false;
    while opened.elapsed().as_secs_f64() < seconds + 1.0 {
        if let Some(why) = player.failed() {
            eprintln!("failed: {why}");
            return;
        }
        if !sought
            && let Some(to) = seek
            && opened.elapsed() > Duration::from_secs(1)
        {
            sought = true;
            player.seek(to);
            println!("seek to {to} at {:?}", opened.elapsed());
        }
        if let Some(frame) = player.due() {
            if sought && !landed {
                landed = true;
                println!(
                    "after the seek: picture at pts {:.3}, {:?}",
                    frame.pts,
                    opened.elapsed()
                );
            }
            if first.is_none() {
                first = Some(opened.elapsed());
                println!(
                    "first picture {}x{} at {:?} (pts {:.3})",
                    frame.width,
                    frame.height,
                    opened.elapsed(),
                    frame.pts
                );
            }
            shown += 1;
        }
        std::thread::sleep(Duration::from_millis(2));
    }
    println!(
        "{shown} pictures in {seconds}s, position {:.2} of {:.2}s",
        player.position(),
        player.duration()
    );
}
