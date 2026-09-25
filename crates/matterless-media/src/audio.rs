//! The sound of a video, to the reader's default output.

use crate::player::Sound;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use std::sync::{Arc, Mutex};

/// Opens the default output and starts pulling from `sound`, telling it the
/// rate to resample to. `None` when there is nowhere to play: the picture
/// still plays, silently.
pub(crate) fn open(sound: Arc<Mutex<Sound>>) -> Option<cpal::Stream> {
    let host = cpal::default_host();
    let device = host.default_output_device()?;
    let supported = device.default_output_config().ok()?;
    // What Windows' shared mixer takes. Anything else would need converting
    // here, in the callback, where there is no time for it.
    if supported.sample_format() != cpal::SampleFormat::F32 {
        return None;
    }
    let channels = supported.channels() as usize;
    if let Ok(mut held) = sound.lock() {
        held.rate = supported.sample_rate();
    }
    let pulled = sound.clone();
    let stream = device
        .build_output_stream(
            supported.config(),
            move |data: &mut [f32], _| {
                // Never waited on: a callback that blocks is a click in the
                // sound. A busy queue is one buffer of silence.
                let Ok(mut held) = pulled.try_lock() else {
                    data.fill(0.0);
                    return;
                };
                if !held.playing {
                    data.fill(0.0);
                    return;
                }
                let loud = if held.muted { 0.0 } else { 1.0 };
                for chunk in data.chunks_mut(channels.max(1)) {
                    let left = held.samples.pop_front().unwrap_or(0.0) * loud;
                    let right = held.samples.pop_front().unwrap_or(0.0) * loud;
                    match chunk.len() {
                        1 => chunk[0] = (left + right) / 2.0,
                        _ => {
                            for (at, sample) in chunk.iter_mut().enumerate() {
                                *sample = match at {
                                    0 => left,
                                    1 => right,
                                    _ => 0.0,
                                };
                            }
                        }
                    }
                }
            },
            |why| eprintln!("sound: {why}"),
            None,
        )
        .ok()?;
    stream.play().ok()?;
    Some(stream)
}
