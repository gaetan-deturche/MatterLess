//! One picture out of a video, for the message it is attached to.
//!
//! Decoded on the CPU: it is one frame, the GPU's surfaces would have to be
//! read back to put it in the atlas, and a poster is wanted for every video on
//! screen rather than the one being played.

use crate::api::{Api, api, error};
use crate::ffmpeg::*;
use std::ffi::{CString, c_int, c_void};
use std::path::Path;
use std::ptr::{null, null_mut};

/// `SWS_AREA`: the scaler for making a picture smaller.
const SWS_AREA: c_int = 1 << 5;

/// How many packets are read looking for the first picture before giving up.
const LOOKED: usize = 600;

/// The first picture of the video at `path`, fitted into `width` by `height`
/// with its shape kept and the rest left black: straight RGBA, exactly that
/// size, so the box it is drawn in never stretches it.
pub fn poster(path: &Path, width: u32, height: u32) -> Result<Vec<u8>, String> {
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
        held.codec = (api.avcodec_alloc_context3)(decoder);
        check(
            api,
            (api.avcodec_parameters_to_context)(held.codec, (*stream).codecpar),
            "settings",
        )?;
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

        let mut found = false;
        for _ in 0..LOOKED {
            if (api.av_read_frame)(held.format, held.packet) < 0 {
                // The end of what is here: whatever the decoder holds.
                (api.avcodec_send_packet)(held.codec, null());
                found = (api.avcodec_receive_frame)(held.codec, held.frame) >= 0;
                break;
            }
            if (*held.packet).stream_index == video
                && (api.avcodec_send_packet)(held.codec, held.packet) >= 0
                && (api.avcodec_receive_frame)(held.codec, held.frame) >= 0
            {
                found = true;
            }
            (api.av_packet_unref)(held.packet);
            if found {
                break;
            }
        }
        if !found {
            return Err("no picture in the first part of the file".to_string());
        }

        let frame = held.frame;
        let (from_width, from_height) = ((*frame).width.max(1), (*frame).height.max(1));
        let scale = (width as f64 / from_width as f64).min(height as f64 / from_height as f64);
        let fitted = (
            ((from_width as f64 * scale).round() as c_int).clamp(1, width as c_int),
            ((from_height as f64 * scale).round() as c_int).clamp(1, height as c_int),
        );
        held.scaler = (api.sws_get_context)(
            from_width,
            from_height,
            (*frame).format,
            fitted.0,
            fitted.1,
            AV_PIX_FMT_RGBA,
            SWS_AREA,
            null_mut(),
            null_mut(),
            null(),
        );
        if held.scaler.is_null() {
            return Err("no scaler for this picture".to_string());
        }
        let mut picture = vec![0u8; fitted.0 as usize * fitted.1 as usize * 4];
        let into = [picture.as_mut_ptr()];
        let stride = [fitted.0 * 4];
        (api.sws_scale)(
            held.scaler,
            (*frame).data.as_ptr() as *const *const u8,
            (*frame).linesize.as_ptr(),
            0,
            from_height,
            into.as_ptr(),
            stride.as_ptr(),
        );

        // Centred on black, the size of the box.
        let mut canvas = vec![0u8; width as usize * height as usize * 4];
        for pixel in canvas.chunks_exact_mut(4) {
            pixel[3] = 255;
        }
        let (left, top) = (
            (width as usize - fitted.0 as usize) / 2,
            (height as usize - fitted.1 as usize) / 2,
        );
        let row = fitted.0 as usize * 4;
        for y in 0..fitted.1 as usize {
            let to = ((top + y) * width as usize + left) * 4;
            canvas[to..to + row].copy_from_slice(&picture[y * row..(y + 1) * row]);
        }
        Ok(canvas)
    }
}

fn check(api: &Api, code: c_int, doing: &str) -> Result<c_int, String> {
    match code < 0 {
        true => Err(format!("{doing}: {}", error(api, code))),
        false => Ok(code),
    }
}

/// What ffmpeg was given, freed however `poster` leaves.
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
