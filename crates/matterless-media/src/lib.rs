//! Playing the videos people attach.
//!
//! ffmpeg does the reading and decoding -- the picture on the GPU through
//! D3D11VA, into textures on the window's own device -- and is loaded at run
//! time from its DLLs, so a machine without them still runs the program.

#![cfg(windows)]

mod api;
mod audio;
mod ffmpeg;
mod player;
mod poster;
mod previews;

pub use player::{Frame, Player};
pub use poster::poster;
pub use previews::Preview;

/// Whether videos can be played here, and if not, why not.
pub fn available() -> Result<(), String> {
    api::api().map(|_| ())
}
