//! One video, decoded on the GPU and handed over frame by frame.
//!
//! A thread of its own reads the file and decodes: the picture through
//! D3D11VA into textures on the window's own device, the sound into floats
//! for the audio output. The window asks each frame which picture is due and
//! draws it; nothing here draws.
//!
//! The clock is the wall clock while playing, restarted at the first frame
//! shown after opening or seeking -- so a slow start or a slow seek never
//! plays the opening frames late and skips them. The sound runs beside it
//! from its own queue.

use crate::api::{Api, api, error};
use crate::ffmpeg::*;
use std::collections::VecDeque;
use std::ffi::{CString, c_int, c_void};
use std::path::Path;
use std::ptr::{null, null_mut};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};
use windows::Win32::Graphics::Direct3D11::{ID3D11Device, ID3D11Multithread};
use windows::core::Interface;

/// How many decoded pictures are held ahead of the one showing. Each is a
/// texture out of the decoder's pool, so the pool is made this much bigger.
const QUEUE: usize = 8;
/// How far past what is decoded an aim is still reached by decoding on rather
/// than by seeking to the keyframe before it, in seconds.
const REACH: f64 = 3.0;
/// How much of the run-up to a dragged-to place is shown after a seek.
const RUN_UP: f64 = 0.25;
/// Kept before a place landed on: a frame's length at twenty a second.
const LANDING: f64 = 0.05;
/// How much sound is decoded ahead, in seconds.
const AHEAD: f64 = 1.0;
/// How many compressed pictures may wait to be decoded while the sound is
/// read ahead: several seconds of video at any ordinary rate.
const PENDING: usize = 600;

/// A decoded picture: one slice of a texture array on the window's device.
pub struct Frame {
    frame: *mut AVFrame,
    /// When it is shown, in seconds from the start of the video.
    pub pts: f64,
    /// The `ID3D11Texture2D` it is in, borrowed for as long as this lives.
    pub texture: *mut c_void,
    /// Which slice of that texture array.
    pub index: u32,
    pub width: u32,
    pub height: u32,
}

// The texture is the window's device's, and the frame only keeps it alive.
unsafe impl Send for Frame {}

impl Drop for Frame {
    fn drop(&mut self) {
        if let Ok(api) = api() {
            unsafe { (api.av_frame_free)(&mut self.frame) };
        }
    }
}

/// The sound waiting to be played, as interleaved stereo.
pub(crate) struct Sound {
    pub samples: VecDeque<f32>,
    pub playing: bool,
    /// Played in silence: consumed at the same pace, so unmuting is in step.
    pub muted: bool,
    pub rate: u32,
}

/// What the decoding thread and the window share.
struct State {
    frames: VecDeque<Frame>,
    playing: bool,
    /// When the clock last started, while playing.
    since: Option<Instant>,
    /// Where the clock stood when it last started or stopped.
    offset: f64,
    /// A keyframe seek for the decoding thread: to the keyframe before this.
    seek: Option<f64>,
    /// Where the reader is pointing while dragging or landing: the newest
    /// picture at or before it is shown, whatever the clock says.
    aim: Option<f64>,
    /// The aim is where it is to stop: once its picture is shown, the clock
    /// starts from there.
    landing: bool,
    /// Whether to play once landed: what it was doing when the aim began.
    resume: Option<bool>,
    /// After a keyframe seek, the run-up from the keyframe is not shown: the
    /// last picture stays until the decoder is nearly there, rather than the
    /// picture flashing back to the keyframe and racing forward on every step
    /// of a drag backwards.
    hide_before: f64,
    /// The picture last handed out, and the last one decoded.
    shown: f64,
    decoded: f64,
    /// Where the last seek was sent: the keyframe it starts decoding from is
    /// at or before this.
    floor: f64,
    /// Sound before this is thrown away: after a seek, the keyframe the
    /// pictures start from is before where the sound should.
    skip_until: f64,
    /// A frame is wanted even while paused: the first after opening or a seek.
    first: bool,
    /// Everything in the file has been read and decoded.
    read_all: bool,
    finished: bool,
    quit: bool,
    failed: Option<String>,
    duration: f64,
    /// The picture's own size, once the decoder is open.
    size: Option<(u32, u32)>,
}

struct Shared {
    state: Mutex<State>,
    /// Wakes the decoding thread: room in the queue, a seek, or quitting.
    stir: Condvar,
    sound: Arc<Mutex<Sound>>,
}

/// A raw pointer that may cross to the decoding thread.
struct Handed(*mut c_void);
unsafe impl Send for Handed {}

/// A video being played.
pub struct Player {
    shared: Arc<Shared>,
    thread: Option<std::thread::JoinHandle<()>>,
    /// Small pictures of the whole video, decoded ahead for dragging.
    previews: Arc<Mutex<crate::previews::Previews>>,
    stop_previews: Arc<std::sync::atomic::AtomicBool>,
    filler: Option<std::thread::JoinHandle<()>>,
    _output: Option<cpal::Stream>,
    /// Held so the lock ffmpeg is given outlives the thread using it.
    _multithread: ID3D11Multithread,
}

impl Player {
    /// Opens a video file for playing on `device`, paused on its first frame.
    ///
    /// The device is made safe to use from two threads: ffmpeg decodes into
    /// it from its own while the window draws with it.
    pub fn open(path: &Path, device: &ID3D11Device) -> Result<Self, String> {
        api()?;
        let context = unsafe { device.GetImmediateContext() }.map_err(|why| why.to_string())?;
        let multithread: ID3D11Multithread = context.cast().map_err(|why| why.to_string())?;
        let _ = unsafe { multithread.SetMultithreadProtected(true) };

        let sound = Arc::new(Mutex::new(Sound {
            samples: VecDeque::new(),
            playing: false,
            muted: false,
            rate: 0,
        }));
        let output = crate::audio::open(sound.clone());
        let shared = Arc::new(Shared {
            state: Mutex::new(State {
                frames: VecDeque::new(),
                playing: false,
                since: None,
                offset: 0.0,
                seek: None,
                aim: None,
                landing: false,
                resume: None,
                hide_before: f64::NEG_INFINITY,
                shown: f64::NEG_INFINITY,
                decoded: f64::NEG_INFINITY,
                floor: 0.0,
                skip_until: 0.0,
                first: true,
                read_all: false,
                finished: false,
                quit: false,
                failed: None,
                duration: 0.0,
                size: None,
            }),
            stir: Condvar::new(),
            sound,
        });
        // One reference for ffmpeg, which releases it with the device context.
        let handed = Handed(device.clone().into_raw());
        let lock = Handed(multithread.as_raw());
        let path = path.to_path_buf();
        let preview_path = path.clone();
        let thread = {
            let shared = shared.clone();
            std::thread::Builder::new()
                .name("video".to_string())
                .spawn(move || {
                    let (handed, lock) = (handed, lock);
                    if let Err(why) = decode(&path, handed.0, lock.0, &shared) {
                        eprintln!("video {}: {why}", path.display());
                        let mut state = lock_state(&shared);
                        state.failed = Some(why);
                    }
                })
                .map_err(|why| why.to_string())?
        };
        let previews = Arc::new(Mutex::new(crate::previews::Previews::default()));
        let stop_previews = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let filler = {
            let (previews, stop, path) = (previews.clone(), stop_previews.clone(), preview_path);
            std::thread::Builder::new()
                .name("video previews".to_string())
                .spawn(move || {
                    if let Err(why) = crate::previews::fill(&path, &previews, &stop) {
                        eprintln!("video previews {}: {why}", path.display());
                    }
                })
                .ok()
        };
        Ok(Self {
            shared,
            thread: Some(thread),
            previews,
            stop_previews,
            filler,
            _output: output,
            _multithread: multithread,
        })
    }

    /// Why it cannot be played, once that is known.
    pub fn failed(&self) -> Option<String> {
        lock_state(&self.shared).failed.clone()
    }

    pub fn duration(&self) -> f64 {
        lock_state(&self.shared).duration
    }

    pub fn position(&self) -> f64 {
        position(&lock_state(&self.shared))
    }

    pub fn playing(&self) -> bool {
        lock_state(&self.shared).playing
    }

    /// Still on its way to where it was dragged or sent.
    pub fn aiming(&self) -> bool {
        lock_state(&self.shared).aim.is_some()
    }

    pub fn play(&self) {
        let finished = {
            let mut state = lock_state(&self.shared);
            if state.aim.is_some() {
                // Once it has got where it is going.
                state.resume = Some(true);
                return;
            }
            if !state.playing && !state.finished {
                state.playing = true;
                state.since = Some(Instant::now());
            }
            state.finished
        };
        // From the start again, the way every player answers play at the end.
        if finished {
            self.seek(0.0);
            lock_state(&self.shared).resume = Some(true);
            return;
        }
        set_sound(&self.shared, true);
    }

    pub fn pause(&self) {
        let mut state = lock_state(&self.shared);
        if state.aim.is_some() {
            state.resume = Some(false);
            return;
        }
        state.offset = position(&state);
        state.since = None;
        state.playing = false;
        drop(state);
        set_sound(&self.shared, false);
    }

    pub fn toggle(&self) {
        let (aiming, resume, playing) = {
            let state = lock_state(&self.shared);
            (state.aim.is_some(), state.resume, state.playing)
        };
        match (aiming, resume == Some(true) || playing) {
            (_, true) => self.pause(),
            (_, false) => self.play(),
        }
    }

    pub fn set_muted(&self, muted: bool) {
        if let Ok(mut sound) = self.shared.sound.lock() {
            sound.muted = muted;
        }
    }

    /// Goes to `seconds` from the start, exactly, and carries on as it was --
    /// or, after a drag, as it was when the drag began.
    pub fn seek(&self, seconds: f64) {
        self.aim_at(seconds, true);
    }

    /// Follows the pointer along the track: paused, silent, showing the
    /// newest picture at or before it. Forward it only decodes on from where
    /// it is, which is what makes a drag through a long run between keyframes
    /// move rather than jump; back, it seeks to the keyframe before and runs
    /// up to it. `seek` lands it.
    ///
    /// When a picture decoded ahead is near enough, that is the answer: shown
    /// at once, the decoder left alone, and the player held paused as a drag
    /// holds it. `seek` lands it on the exact frame.
    pub fn scrub(&self, seconds: f64) -> Option<crate::Preview> {
        let near = self
            .previews
            .lock()
            .ok()
            .and_then(|previews| previews.near(seconds));
        let Some(preview) = near else {
            self.aim_at(seconds, false);
            return None;
        };
        let mut state = lock_state(&self.shared);
        if state.resume.is_none() {
            state.resume = Some(state.playing);
        }
        if state.playing {
            state.offset = position(&state);
            state.since = None;
            state.playing = false;
        }
        state.aim = None;
        state.landing = false;
        drop(state);
        set_sound(&self.shared, false);
        Some(preview)
    }

    /// How far into the video the pictures decoded ahead reach.
    pub fn previewed(&self) -> f64 {
        self.previews
            .lock()
            .map_or(0.0, |previews| previews.reach())
    }

    /// The picture's own size, once known.
    pub fn size(&self) -> Option<(u32, u32)> {
        lock_state(&self.shared).size
    }

    fn aim_at(&self, seconds: f64, landing: bool) {
        let mut state = lock_state(&self.shared);
        let to = seconds.clamp(0.0, state.duration.max(0.0));
        if state.resume.is_none() {
            state.resume = Some(state.playing);
        }
        if state.playing {
            state.offset = position(&state);
            state.since = None;
            state.playing = false;
        }
        state.finished = false;
        // Reached by decoding on: the decoder will still come to it -- it is
        // at or past the oldest picture waiting, or past the last one decoded,
        // or past where a seek in progress started -- and it is not so far on
        // that the keyframe before it would be quicker. Behind the picture
        // showing does not matter: dragging back through a run the decoder
        // is still working along costs no second seek, which is what made a
        // drag backwards start again from the same keyframe on every step.
        let head = match (state.frames.front(), state.decoded.is_finite()) {
            (Some(frame), _) => frame.pts,
            (None, true) => state.decoded + 0.0005,
            (None, false) => state.floor,
        };
        let ahead = state.decoded.max(state.floor).max(state.shown);
        // Or it is already showing: nothing waiting is before it and what is
        // on screen is -- the decoder has simply run on past it.
        let showing = state.shown <= to + 0.001 && head > to;
        let reachable =
            state.seek.is_none() && (showing || to + 0.0005 >= head) && to <= ahead + REACH;
        if !reachable {
            state.seek = Some(to);
            state.floor = to;
            state.frames.clear();
            state.decoded = f64::NEG_INFINITY;
        }
        // Only the newest pictures before it matter; the rest are dropped as
        // they are decoded. Landing keeps the frame before it, which is the
        // one to stop on when no frame falls exactly there.
        state.hide_before = to - if landing { LANDING } else { RUN_UP };
        state.aim = Some(to);
        state.landing = landing;
        if landing {
            state.skip_until = to;
        }
        drop(state);
        if let Ok(mut sound) = self.shared.sound.lock() {
            sound.playing = false;
            sound.samples.clear();
        }
        self.shared.stir.notify_all();
    }

    /// The picture to show now, if a new one is due.
    pub fn due(&self) -> Option<Frame> {
        let mut state = lock_state(&self.shared);
        if state.first {
            let frame = state.frames.pop_front()?;
            state.first = false;
            state.shown = frame.pts;
            // The clock starts on what is shown, not on when it was asked for.
            state.offset = frame.pts;
            state.since = state.playing.then(Instant::now);
            self.shared.stir.notify_all();
            return Some(frame);
        }
        if let Some(aim) = state.aim {
            let mut chosen = None;
            let mut took = false;
            while state
                .frames
                .front()
                .is_some_and(|frame| frame.pts <= aim + 0.001)
            {
                took = true;
                let frame = state.frames.pop_front();
                if frame
                    .as_ref()
                    .is_some_and(|frame| frame.pts >= state.hide_before)
                {
                    chosen = frame;
                }
            }
            if let Some(frame) = chosen.as_ref() {
                state.shown = frame.pts;
            }
            if took {
                self.shared.stir.notify_all();
            }
            let past = state
                .frames
                .front()
                .is_some_and(|frame| frame.pts > aim + 0.001)
                || (state.read_all && state.frames.is_empty());
            if state.landing && past && state.seek.is_none() {
                state.aim = None;
                state.landing = false;
                state.offset = aim;
                let resume = state.resume.take() == Some(true);
                state.playing = resume;
                state.since = resume.then(Instant::now);
                drop(state);
                set_sound(&self.shared, resume);
            }
            return chosen;
        }
        if !state.playing {
            return None;
        }
        let now = position(&state);
        let mut chosen = None;
        while state.frames.front().is_some_and(|frame| frame.pts <= now) {
            chosen = state.frames.pop_front();
        }
        if let Some(frame) = chosen.as_ref() {
            state.shown = frame.pts;
            self.shared.stir.notify_all();
        }
        if state.read_all && state.frames.is_empty() && chosen.is_none() {
            // The end: stopped there, and play starts it over.
            state.offset = now.min(state.duration.max(now));
            state.since = None;
            state.playing = false;
            state.finished = true;
            drop(state);
            set_sound(&self.shared, false);
        }
        chosen
    }

    /// When the next picture comes due, while playing.
    pub fn next_at(&self) -> Option<Instant> {
        let state = lock_state(&self.shared);
        if state.first {
            return (!state.frames.is_empty()).then(Instant::now);
        }
        // Aiming: now if a picture is ready, and soon otherwise, since the
        // decoder is working towards it.
        if let Some(aim) = state.aim {
            let ready = state
                .frames
                .front()
                .is_some_and(|frame| frame.pts <= aim + 0.001);
            return Some(match ready || state.landing {
                true => Instant::now(),
                false => Instant::now() + Duration::from_millis(4),
            });
        }
        if !state.playing {
            return None;
        }
        let next = state.frames.front()?.pts;
        let wait = (next - position(&state)).max(0.0);
        Some(Instant::now() + Duration::from_secs_f64(wait))
    }
}

impl Drop for Player {
    fn drop(&mut self) {
        self.stop_previews
            .store(true, std::sync::atomic::Ordering::Relaxed);
        if let Some(filler) = self.filler.take() {
            let _ = filler.join();
        }
        lock_state(&self.shared).quit = true;
        self.shared.stir.notify_all();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn lock_state(shared: &Shared) -> std::sync::MutexGuard<'_, State> {
    shared.state.lock().unwrap_or_else(|held| held.into_inner())
}

fn position(state: &State) -> f64 {
    match state.since {
        Some(at) if state.playing => state.offset + at.elapsed().as_secs_f64(),
        _ => state.offset,
    }
}

fn set_sound(shared: &Shared, playing: bool) {
    if let Ok(mut sound) = shared.sound.lock() {
        sound.playing = playing;
    }
}

/// ffmpeg's lock around the device, as the device's own.
unsafe extern "C" fn enter(lock: *mut c_void) {
    if let Some(multithread) = unsafe { ID3D11Multithread::from_raw_borrowed(&lock) } {
        unsafe { multithread.Enter() };
    }
}

unsafe extern "C" fn leave(lock: *mut c_void) {
    if let Some(multithread) = unsafe { ID3D11Multithread::from_raw_borrowed(&lock) } {
        unsafe { multithread.Leave() };
    }
}

/// Picks the GPU's surfaces out of what the decoder offers, or nothing.
unsafe extern "C" fn on_the_gpu(
    _: *mut AVCodecContext,
    offered: *const AVPixelFormat,
) -> AVPixelFormat {
    let mut at = offered;
    while !at.is_null() && unsafe { *at } != AV_PIX_FMT_NONE {
        if unsafe { *at } == AV_PIX_FMT_D3D11 {
            return AV_PIX_FMT_D3D11;
        }
        at = unsafe { at.add(1) };
    }
    AV_PIX_FMT_NONE
}

fn check(api: &Api, code: c_int, doing: &str) -> Result<c_int, String> {
    match code < 0 {
        true => Err(format!("{doing}: {}", error(api, code))),
        false => Ok(code),
    }
}

/// Everything ffmpeg was given, freed however `decode` leaves.
struct Owned<'a> {
    api: &'a Api,
    format: *mut AVFormatContext,
    video: *mut AVCodecContext,
    audio: *mut AVCodecContext,
    hardware: *mut AVBufferRef,
    resample: *mut SwrContext,
    packet: *mut AVPacket,
    frame: *mut AVFrame,
    /// Compressed pictures read but not yet decoded.
    pending: VecDeque<*mut AVPacket>,
}

impl Owned<'_> {
    fn drop_pending(&mut self) {
        for mut packet in self.pending.drain(..) {
            if !packet.is_null() {
                unsafe { (self.api.av_packet_free)(&mut packet) };
            }
        }
    }
}

impl Drop for Owned<'_> {
    fn drop(&mut self) {
        self.drop_pending();
        let api = self.api;
        unsafe {
            if !self.frame.is_null() {
                (api.av_frame_free)(&mut self.frame);
            }
            if !self.packet.is_null() {
                (api.av_packet_free)(&mut self.packet);
            }
            if !self.resample.is_null() {
                (api.swr_free)(&mut self.resample);
            }
            if !self.video.is_null() {
                (api.avcodec_free_context)(&mut self.video);
            }
            if !self.audio.is_null() {
                (api.avcodec_free_context)(&mut self.audio);
            }
            if !self.hardware.is_null() {
                (api.av_buffer_unref)(&mut self.hardware);
            }
            if !self.format.is_null() {
                (api.avformat_close_input)(&mut self.format);
            }
        }
    }
}

fn seconds(stamp: i64, base: AVRational) -> f64 {
    match stamp == AV_NOPTS_VALUE || base.den == 0 {
        true => 0.0,
        false => stamp as f64 * base.num as f64 / base.den as f64,
    }
}

/// The decoding thread: open, then read and decode until told to stop.
fn decode(
    path: &Path,
    device: *mut c_void,
    lock: *mut c_void,
    shared: &Shared,
) -> Result<(), String> {
    let api = api()?;
    let mut owned = Owned {
        api,
        format: null_mut(),
        video: null_mut(),
        audio: null_mut(),
        hardware: null_mut(),
        resample: null_mut(),
        packet: null_mut(),
        frame: null_mut(),
        pending: VecDeque::new(),
    };
    let name = CString::new(path.to_string_lossy().as_bytes()).map_err(|why| why.to_string())?;
    unsafe {
        check(
            api,
            (api.avformat_open_input)(&mut owned.format, name.as_ptr(), null(), null_mut()),
            "opening",
        )?;
        check(
            api,
            (api.avformat_find_stream_info)(owned.format, null_mut()),
            "reading",
        )?;

        // The picture, decoded on the window's device.
        let mut codec: *const AVCodec = null();
        let video = check(
            api,
            (api.av_find_best_stream)(owned.format, AVMEDIA_TYPE_VIDEO, -1, -1, &mut codec, 0),
            "finding the picture",
        )?;
        // ffmpeg prefers dav1d for AV1, which decodes only on the CPU; its own
        // decoder is the one with the GPU path.
        if !codec.is_null() && (*codec).id == AV_CODEC_ID_AV1 {
            let own = (api.avcodec_find_decoder_by_name)(c"av1".as_ptr());
            if !own.is_null() {
                codec = own;
            }
        }
        let video_stream = *(*owned.format).streams.add(video as usize);
        let video_base = (*video_stream).time_base;
        owned.video = (api.avcodec_alloc_context3)(codec);
        check(
            api,
            (api.avcodec_parameters_to_context)(owned.video, (*video_stream).codecpar),
            "the picture's settings",
        )?;
        (*owned.video).pkt_timebase = video_base;
        owned.hardware = (api.av_hwdevice_ctx_alloc)(AV_HWDEVICE_TYPE_D3D11VA);
        if owned.hardware.is_null() {
            return Err("no D3D11VA in this ffmpeg".to_string());
        }
        let context = (*owned.hardware).data as *mut AVHWDeviceContext;
        let d3d = (*context).hwctx as *mut AVD3D11VADeviceContext;
        (*d3d).device = device;
        (*d3d).lock = Some(enter);
        (*d3d).unlock = Some(leave);
        (*d3d).lock_ctx = lock;
        check(
            api,
            (api.av_hwdevice_ctx_init)(owned.hardware),
            "the GPU decoder",
        )?;
        (*owned.video).hw_device_ctx = (api.av_buffer_ref)(owned.hardware);
        (*owned.video).get_format = Some(on_the_gpu);
        (*owned.video).extra_hw_frames = QUEUE as c_int + 8;
        check(
            api,
            (api.avcodec_open2)(owned.video, codec, null_mut()),
            "opening the picture",
        )?;
        lock_state(shared).size = Some((
            (*owned.video).width.max(1) as u32,
            (*owned.video).height.max(1) as u32,
        ));

        // The sound, if there is any and anywhere to play it.
        let rate = shared.sound.lock().map(|sound| sound.rate).unwrap_or(0);
        let mut sound_codec: *const AVCodec = null();
        let audio = (api.av_find_best_stream)(
            owned.format,
            AVMEDIA_TYPE_AUDIO,
            -1,
            video,
            &mut sound_codec,
            0,
        );
        let mut audio_base = AVRational { num: 0, den: 1 };
        if audio >= 0 && rate > 0 {
            let audio_stream = *(*owned.format).streams.add(audio as usize);
            audio_base = (*audio_stream).time_base;
            owned.audio = (api.avcodec_alloc_context3)(sound_codec);
            (api.avcodec_parameters_to_context)(owned.audio, (*audio_stream).codecpar);
            (*owned.audio).pkt_timebase = audio_base;
            if (api.avcodec_open2)(owned.audio, sound_codec, null_mut()) < 0 {
                (api.avcodec_free_context)(&mut owned.audio);
            } else {
                let mut stereo = AVChannelLayout::default();
                (api.av_channel_layout_default)(&mut stereo, 2);
                let made = (api.swr_alloc_set_opts2)(
                    &mut owned.resample,
                    &stereo,
                    AV_SAMPLE_FMT_FLT,
                    rate as c_int,
                    &(*owned.audio).ch_layout,
                    (*owned.audio).sample_fmt,
                    (*owned.audio).sample_rate,
                    0,
                    null_mut(),
                );
                (api.av_channel_layout_uninit)(&mut stereo);
                if made < 0 || (api.swr_init)(owned.resample) < 0 {
                    (api.avcodec_free_context)(&mut owned.audio);
                }
            }
        }

        let duration = match (*owned.format).duration > 0 {
            true => (*owned.format).duration as f64 / AV_TIME_BASE as f64,
            false => seconds((*video_stream).duration, video_base),
        };
        lock_state(shared).duration = duration;

        owned.packet = (api.av_packet_alloc)();
        owned.frame = (api.av_frame_alloc)();
        if owned.packet.is_null() || owned.frame.is_null() {
            return Err("out of memory".to_string());
        }
        let has_sound = !owned.audio.is_null();
        // The file has no more packets to give.
        let mut read_to_end = false;

        loop {
            // What the window asked for.
            let room = {
                let mut state = lock_state(shared);
                if state.quit {
                    return Ok(());
                }
                if let Some(to) = state.seek.take() {
                    drop(state);
                    (api.av_seek_frame)(
                        owned.format,
                        -1,
                        (to * AV_TIME_BASE as f64) as i64,
                        AVSEEK_FLAG_BACKWARD as c_int,
                    );
                    (api.avcodec_flush_buffers)(owned.video);
                    if has_sound {
                        (api.avcodec_flush_buffers)(owned.audio);
                    }
                    owned.drop_pending();
                    read_to_end = false;
                    let mut state = lock_state(shared);
                    state.frames.clear();
                    state.decoded = f64::NEG_INFINITY;
                    state.read_all = false;
                    continue;
                }
                state.frames.len() < QUEUE
            };
            // A waiting picture is decoded only when there is room for it: a
            // decoded one holds a surface out of the GPU's pool, and the pool
            // is only so big. A null packet is the end of the file.
            if room && let Some(mut packet) = owned.pending.pop_front() {
                if packet.is_null() {
                    (api.avcodec_send_packet)(owned.video, null());
                    take_pictures(api, &mut owned, video_base, shared)?;
                    if has_sound {
                        (api.avcodec_send_packet)(owned.audio, null());
                        take_sound(api, &mut owned, audio_base, rate, shared);
                    }
                    lock_state(shared).read_all = true;
                } else {
                    if (api.avcodec_send_packet)(owned.video, packet) >= 0 {
                        take_pictures(api, &mut owned, video_base, shared)?;
                    }
                    (api.av_packet_free)(&mut packet);
                }
                continue;
            }
            // Read on only while the sound wants more, or the pictures do and
            // nothing is waiting for them.
            let sound_full = !has_sound
                || shared
                    .sound
                    .lock()
                    .map(|sound| sound.samples.len() as f64 >= AHEAD * rate as f64 * 2.0)
                    .unwrap_or(true);
            let enough = sound_full && (!room || !owned.pending.is_empty());
            if read_to_end || enough || owned.pending.len() >= PENDING {
                let state = lock_state(shared);
                let _ = shared
                    .stir
                    .wait_timeout(state, Duration::from_millis(20))
                    .map_err(|_| ());
                continue;
            }

            if (api.av_read_frame)(owned.format, owned.packet) < 0 {
                read_to_end = true;
                owned.pending.push_back(null_mut());
                continue;
            }
            let stream = (*owned.packet).stream_index;
            if stream == video {
                let kept = (api.av_packet_clone)(owned.packet);
                if !kept.is_null() {
                    owned.pending.push_back(kept);
                }
            } else if has_sound
                && stream == audio
                && (api.avcodec_send_packet)(owned.audio, owned.packet) >= 0
            {
                take_sound(api, &mut owned, audio_base, rate, shared);
            }
            (api.av_packet_unref)(owned.packet);
        }
    }
}

/// Every picture the decoder has ready, onto the queue.
unsafe fn take_pictures(
    api: &Api,
    owned: &mut Owned,
    base: AVRational,
    shared: &Shared,
) -> Result<(), String> {
    loop {
        let frame = unsafe { (api.av_frame_alloc)() };
        if frame.is_null() {
            return Err("out of memory".to_string());
        }
        let mut frame = frame;
        if unsafe { (api.avcodec_receive_frame)(owned.video, frame) } < 0 {
            unsafe { (api.av_frame_free)(&mut frame) };
            return Ok(());
        }
        if unsafe { (*frame).format } != AV_PIX_FMT_D3D11 {
            unsafe { (api.av_frame_free)(&mut frame) };
            return Err("this video cannot be decoded on this GPU".to_string());
        }
        let pts = seconds(unsafe { (*frame).best_effort_timestamp }, base);
        let mut state = lock_state(shared);
        state.decoded = pts;
        // The run-up after a seek is dropped here, as it comes out: through
        // the queue it would go at the pace the window draws, not the GPU's.
        if pts + 0.001 < state.hide_before {
            drop(state);
            unsafe { (api.av_frame_free)(&mut frame) };
            continue;
        }
        let picture = unsafe {
            Frame {
                frame,
                pts,
                texture: (*frame).data[0] as *mut c_void,
                index: (*frame).data[1] as usize as u32,
                width: (*frame).width.max(0) as u32,
                height: (*frame).height.max(0) as u32,
            }
        };
        state.frames.push_back(picture);
    }
}

/// Every stretch of sound the decoder has ready, resampled onto the queue.
unsafe fn take_sound(api: &Api, owned: &mut Owned, base: AVRational, rate: u32, shared: &Shared) {
    let skip_until = lock_state(shared).skip_until;
    loop {
        if unsafe { (api.avcodec_receive_frame)(owned.audio, owned.frame) } < 0 {
            return;
        }
        let frame = owned.frame;
        let pts = seconds(unsafe { (*frame).best_effort_timestamp }, base);
        let count = unsafe { (*frame).nb_samples };
        let from = unsafe { (*owned.audio).sample_rate }.max(1);
        let room = (count as i64 * rate as i64 / from as i64) as c_int + 256;
        let mut out = vec![0f32; room as usize * 2];
        let outs = [out.as_mut_ptr() as *mut u8];
        let made = unsafe {
            (api.swr_convert)(
                owned.resample,
                outs.as_ptr(),
                room,
                (*frame).extended_data as *const *const u8,
                count,
            )
        };
        if made > 0 && pts + 0.001 >= skip_until {
            out.truncate(made as usize * 2);
            if let Ok(mut sound) = shared.sound.lock() {
                sound.samples.extend(out);
            }
        }
    }
}
