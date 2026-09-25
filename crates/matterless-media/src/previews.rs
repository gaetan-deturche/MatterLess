//! Small pictures of a whole video, decoded ahead, for dragging through it.
//!
//! What makes a drag slow is not reading the file but decoding it: a picture
//! between two keyframes seconds apart is every picture from the first one on.
//! So the whole video is decoded once, on the CPU and beside the player, and
//! pictures are kept evenly spread through it -- scaled down, as many as fit
//! the budget. A drag then shows the nearest of them at once, and the player
//! lands on the exact frame at full size when the drag is let go.

use crate::api::{Api, api, error};
use crate::ffmpeg::*;
use std::ffi::{CString, c_int, c_void};
use std::path::Path;
use std::ptr::{null, null_mut};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

/// How much memory the pictures of one video may take.
pub const BUDGET: usize = 512 * 1024 * 1024;
/// The largest a picture is kept: enough to recognise where you are.
const LARGEST: (u32, u32) = (854, 480);
/// `SWS_AREA`: the scaler for making a picture smaller.
const SWS_AREA: c_int = 1 << 5;

/// One kept picture.
#[derive(Clone)]
pub struct Preview {
    pub pts: f64,
    pub width: u32,
    pub height: u32,
    pub rgba: Arc<Vec<u8>>,
}

/// The pictures kept so far, in order.
#[derive(Default)]
pub(crate) struct Previews {
    kept: Vec<Preview>,
    /// How far apart they are meant to be, in seconds.
    every: f64,
}

impl Previews {
    /// How far into the video the kept pictures reach, in seconds.
    pub fn reach(&self) -> f64 {
        self.kept.last().map_or(0.0, |last| last.pts)
    }

    /// The kept picture nearest `seconds`, if the pictures reach that far.
    pub fn near(&self, seconds: f64) -> Option<Preview> {
        let last = self.kept.last()?;
        if seconds > last.pts + self.every.max(0.05) {
            return None;
        }
        let at = self.kept.partition_point(|one| one.pts < seconds);
        let after = self.kept.get(at);
        let before = at.checked_sub(1).and_then(|at| self.kept.get(at));
        match (before, after) {
            (Some(before), Some(after)) => Some(
                match seconds - before.pts <= after.pts - seconds {
                    true => before,
                    false => after,
                }
                .clone(),
            ),
            (Some(one), None) | (None, Some(one)) => Some(one.clone()),
            (None, None) => None,
        }
    }
}

/// Decodes the video at `path` from start to end, keeping pictures into
/// `into` until the budget is spent or `stop` is set.
pub(crate) fn fill(path: &Path, into: &Mutex<Previews>, stop: &AtomicBool) -> Result<(), String> {
    let api = api()?;
    let mut held = Held {
        api,
        format: null_mut(),
        codec: null_mut(),
        packet: null_mut(),
        frame: null_mut(),
        scaler: null_mut(),
    };
    let name = CString::new(path.to_string_lossy().as_bytes()).map_err(|why| why.to_string())?;
    unsafe {
        check(
            api,
            (api.avformat_open_input)(&mut held.format, name.as_ptr(), null(), null_mut()),
            "opening",
        )?;
        check(
            api,
            (api.avformat_find_stream_info)(held.format, null_mut()),
            "reading",
        )?;
        let mut decoder: *const AVCodec = null();
        let video = check(
            api,
            (api.av_find_best_stream)(held.format, AVMEDIA_TYPE_VIDEO, -1, -1, &mut decoder, 0),
            "finding the picture",
        )?;
        let stream = *(*held.format).streams.add(video as usize);
        let base = (*stream).time_base;
        held.codec = (api.avcodec_alloc_context3)(decoder);
        check(
            api,
            (api.avcodec_parameters_to_context)(held.codec, (*stream).codecpar),
            "settings",
        )?;
        // Every core: this is work nobody is watching, done as fast as it goes.
        (*held.codec).thread_count = 0;
        check(
            api,
            (api.avcodec_open2)(held.codec, decoder, null_mut()),
            "opening the decoder",
        )?;
        held.packet = (api.av_packet_alloc)();
        held.frame = (api.av_frame_alloc)();
        if held.packet.is_null() || held.frame.is_null() {
            return Err("out of memory".to_string());
        }

        let duration = match (*held.format).duration > 0 {
            true => (*held.format).duration as f64 / AV_TIME_BASE as f64,
            false => 0.0,
        };
        let (width, height) = ((*held.codec).width.max(1), (*held.codec).height.max(1));
        let scale = (LARGEST.0 as f64 / width as f64)
            .min(LARGEST.1 as f64 / height as f64)
            .min(1.0);
        let small = (
            ((width as f64 * scale).round() as c_int).max(1),
            ((height as f64 * scale).round() as c_int).max(1),
        );
        let bytes = small.0 as usize * small.1 as usize * 4;
        let room = (BUDGET / bytes).max(1);
        let every = match duration > 0.0 {
            true => duration / room as f64,
            false => 0.0,
        };
        if let Ok(mut previews) = into.lock() {
            previews.every = every;
        }
        let mut next = f64::NEG_INFINITY;
        let mut kept = 0usize;
        let mut draining = false;

        while !stop.load(Ordering::Relaxed) && kept < room {
            if !draining {
                if (api.av_read_frame)(held.format, held.packet) < 0 {
                    (api.avcodec_send_packet)(held.codec, null());
                    draining = true;
                } else {
                    if (*held.packet).stream_index == video {
                        (api.avcodec_send_packet)(held.codec, held.packet);
                    }
                    (api.av_packet_unref)(held.packet);
                }
            }
            loop {
                if (api.avcodec_receive_frame)(held.codec, held.frame) < 0 {
                    break;
                }
                let frame = held.frame;
                let stamp = (*frame).best_effort_timestamp;
                let pts = match stamp == AV_NOPTS_VALUE || base.den == 0 {
                    true => 0.0,
                    false => stamp as f64 * base.num as f64 / base.den as f64,
                };
                if pts < next {
                    continue;
                }
                next = pts + every;
                if held.scaler.is_null() {
                    held.scaler = (api.sws_get_context)(
                        (*frame).width,
                        (*frame).height,
                        (*frame).format,
                        small.0,
                        small.1,
                        AV_PIX_FMT_RGBA,
                        SWS_AREA,
                        null_mut(),
                        null_mut(),
                        null(),
                    );
                    if held.scaler.is_null() {
                        return Err("no scaler for this video".to_string());
                    }
                }
                let mut rgba = vec![0u8; bytes];
                let out = [rgba.as_mut_ptr()];
                let stride = [small.0 * 4];
                (api.sws_scale)(
                    held.scaler,
                    (*frame).data.as_ptr() as *const *const u8,
                    (*frame).linesize.as_ptr(),
                    0,
                    (*frame).height,
                    out.as_ptr(),
                    stride.as_ptr(),
                );
                if let Ok(mut previews) = into.lock() {
                    previews.kept.push(Preview {
                        pts,
                        width: small.0 as u32,
                        height: small.1 as u32,
                        rgba: Arc::new(rgba),
                    });
                }
                kept += 1;
            }
            if draining {
                break;
            }
        }
    }
    Ok(())
}

fn check(api: &Api, code: c_int, doing: &str) -> Result<c_int, String> {
    match code < 0 {
        true => Err(format!("{doing}: {}", error(api, code))),
        false => Ok(code),
    }
}

/// What ffmpeg was given, freed however `fill` leaves.
struct Held<'a> {
    api: &'a Api,
    format: *mut AVFormatContext,
    codec: *mut AVCodecContext,
    packet: *mut AVPacket,
    frame: *mut AVFrame,
    scaler: *mut c_void,
}

impl Drop for Held<'_> {
    fn drop(&mut self) {
        let api = self.api;
        unsafe {
            if !self.scaler.is_null() {
                (api.sws_free_context)(self.scaler);
            }
            if !self.frame.is_null() {
                (api.av_frame_free)(&mut self.frame);
            }
            if !self.packet.is_null() {
                (api.av_packet_free)(&mut self.packet);
            }
            if !self.codec.is_null() {
                (api.avcodec_free_context)(&mut self.codec);
            }
            if !self.format.is_null() {
                (api.avformat_close_input)(&mut self.format);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kept(at: &[f64]) -> Previews {
        Previews {
            kept: at
                .iter()
                .map(|&pts| Preview {
                    pts,
                    width: 1,
                    height: 1,
                    rgba: Arc::new(vec![0; 4]),
                })
                .collect(),
            every: 0.5,
        }
    }

    /// The nearest kept picture, either side -- and nothing past the last one
    /// kept, where the decoding has not got to yet.
    #[test]
    fn the_nearest_picture_is_given_and_nothing_beyond() {
        let previews = kept(&[0.0, 0.5, 1.0, 1.5]);
        assert_eq!(previews.near(0.6).map(|one| one.pts), Some(0.5));
        assert_eq!(previews.near(0.9).map(|one| one.pts), Some(1.0));
        assert_eq!(previews.near(1.9).map(|one| one.pts), Some(1.5));
        assert!(previews.near(2.5).is_none(), "not decoded that far yet");
        assert!(Previews::default().near(0.0).is_none());
    }
}
