//! The ffmpeg libraries, found and opened at run time.
//!
//! Loaded rather than linked: a machine without the DLLs still runs the
//! program, it just cannot play a video -- and the DLLs stay separate files a
//! reader can replace, which is what the LGPL asks.

use crate::ffmpeg::*;
use std::ffi::{c_char, c_int, c_void};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use windows::Win32::Foundation::HMODULE;
use windows::Win32::System::LibraryLoader::{
    GetProcAddress, LOAD_WITH_ALTERED_SEARCH_PATH, LoadLibraryExW,
};
use windows::core::{PCSTR, PCWSTR};

/// The DLLs, dependencies first, named for the major versions the structs in
/// `ffmpeg` were generated from.
const LIBRARIES: [&str; 5] = [
    "avutil-61.dll",
    "swresample-7.dll",
    "swscale-10.dll",
    "avcodec-63.dll",
    "avformat-63.dll",
];

/// Every function this crate calls, each one looked up by name.
pub struct Api {
    pub avformat_open_input: unsafe extern "C" fn(
        *mut *mut AVFormatContext,
        *const c_char,
        *const AVInputFormat,
        *mut *mut AVDictionary,
    ) -> c_int,
    pub avformat_find_stream_info:
        unsafe extern "C" fn(*mut AVFormatContext, *mut *mut AVDictionary) -> c_int,
    pub avformat_close_input: unsafe extern "C" fn(*mut *mut AVFormatContext),
    pub av_read_frame: unsafe extern "C" fn(*mut AVFormatContext, *mut AVPacket) -> c_int,
    pub av_seek_frame: unsafe extern "C" fn(*mut AVFormatContext, c_int, i64, c_int) -> c_int,
    pub av_find_best_stream: unsafe extern "C" fn(
        *mut AVFormatContext,
        AVMediaType,
        c_int,
        c_int,
        *mut *const AVCodec,
        c_int,
    ) -> c_int,
    pub avcodec_find_decoder_by_name: unsafe extern "C" fn(*const c_char) -> *const AVCodec,
    pub avcodec_alloc_context3: unsafe extern "C" fn(*const AVCodec) -> *mut AVCodecContext,
    pub avcodec_parameters_to_context:
        unsafe extern "C" fn(*mut AVCodecContext, *const AVCodecParameters) -> c_int,
    pub avcodec_open2:
        unsafe extern "C" fn(*mut AVCodecContext, *const AVCodec, *mut *mut AVDictionary) -> c_int,
    pub avcodec_send_packet: unsafe extern "C" fn(*mut AVCodecContext, *const AVPacket) -> c_int,
    pub avcodec_receive_frame: unsafe extern "C" fn(*mut AVCodecContext, *mut AVFrame) -> c_int,
    pub avcodec_free_context: unsafe extern "C" fn(*mut *mut AVCodecContext),
    pub avcodec_flush_buffers: unsafe extern "C" fn(*mut AVCodecContext),
    pub av_packet_alloc: unsafe extern "C" fn() -> *mut AVPacket,
    pub av_packet_free: unsafe extern "C" fn(*mut *mut AVPacket),
    pub av_packet_unref: unsafe extern "C" fn(*mut AVPacket),
    pub av_packet_clone: unsafe extern "C" fn(*const AVPacket) -> *mut AVPacket,
    pub av_frame_alloc: unsafe extern "C" fn() -> *mut AVFrame,
    pub av_frame_free: unsafe extern "C" fn(*mut *mut AVFrame),
    pub av_hwdevice_ctx_alloc: unsafe extern "C" fn(AVHWDeviceType) -> *mut AVBufferRef,
    pub av_hwdevice_ctx_init: unsafe extern "C" fn(*mut AVBufferRef) -> c_int,
    pub av_buffer_ref: unsafe extern "C" fn(*const AVBufferRef) -> *mut AVBufferRef,
    pub av_buffer_unref: unsafe extern "C" fn(*mut *mut AVBufferRef),
    pub av_channel_layout_default: unsafe extern "C" fn(*mut AVChannelLayout, c_int),
    pub av_channel_layout_uninit: unsafe extern "C" fn(*mut AVChannelLayout),
    pub av_log_set_level: unsafe extern "C" fn(c_int),
    pub av_strerror: unsafe extern "C" fn(c_int, *mut c_char, usize) -> c_int,
    pub swr_alloc_set_opts2: unsafe extern "C" fn(
        *mut *mut SwrContext,
        *const AVChannelLayout,
        AVSampleFormat,
        c_int,
        *const AVChannelLayout,
        AVSampleFormat,
        c_int,
        c_int,
        *mut c_void,
    ) -> c_int,
    pub swr_init: unsafe extern "C" fn(*mut SwrContext) -> c_int,
    pub swr_convert: unsafe extern "C" fn(
        *mut SwrContext,
        *const *mut u8,
        c_int,
        *const *const u8,
        c_int,
    ) -> c_int,
    pub swr_free: unsafe extern "C" fn(*mut *mut SwrContext),
    /// The scaler, whose context is opaque: nothing here reads inside it.
    pub sws_get_context: unsafe extern "C" fn(
        c_int,
        c_int,
        AVPixelFormat,
        c_int,
        c_int,
        AVPixelFormat,
        c_int,
        *mut c_void,
        *mut c_void,
        *const f64,
    ) -> *mut c_void,
    pub sws_scale: unsafe extern "C" fn(
        *mut c_void,
        *const *const u8,
        *const c_int,
        c_int,
        c_int,
        *const *mut u8,
        *const c_int,
    ) -> c_int,
    pub sws_free_context: unsafe extern "C" fn(*mut c_void),
}

/// The libraries, opened once. `Err` says why there is no video, for the
/// viewer to show rather than a picture that never arrives.
pub fn api() -> Result<&'static Api, String> {
    static LOADED: OnceLock<Result<Api, String>> = OnceLock::new();
    LOADED
        .get_or_init(|| {
            let found = folders()
                .into_iter()
                .find(|folder| folder.join(LIBRARIES[0]).is_file())
                .ok_or_else(|| format!("ffmpeg is not installed ({} not found)", LIBRARIES[0]))?;
            let modules = LIBRARIES
                .iter()
                .map(|name| open(&found.join(name)))
                .collect::<Result<Vec<HMODULE>, String>>()?;
            let [avutil, swresample, swscale, avcodec, avformat] = modules[..] else {
                return Err("ffmpeg's libraries".to_string());
            };
            let api = unsafe {
                Api {
                    avformat_open_input: find(avformat, "avformat_open_input\0")?,
                    avformat_find_stream_info: find(avformat, "avformat_find_stream_info\0")?,
                    avformat_close_input: find(avformat, "avformat_close_input\0")?,
                    av_read_frame: find(avformat, "av_read_frame\0")?,
                    av_seek_frame: find(avformat, "av_seek_frame\0")?,
                    av_find_best_stream: find(avformat, "av_find_best_stream\0")?,
                    avcodec_find_decoder_by_name: find(avcodec, "avcodec_find_decoder_by_name\0")?,
                    avcodec_alloc_context3: find(avcodec, "avcodec_alloc_context3\0")?,
                    avcodec_parameters_to_context: find(
                        avcodec,
                        "avcodec_parameters_to_context\0",
                    )?,
                    avcodec_open2: find(avcodec, "avcodec_open2\0")?,
                    avcodec_send_packet: find(avcodec, "avcodec_send_packet\0")?,
                    avcodec_receive_frame: find(avcodec, "avcodec_receive_frame\0")?,
                    avcodec_free_context: find(avcodec, "avcodec_free_context\0")?,
                    avcodec_flush_buffers: find(avcodec, "avcodec_flush_buffers\0")?,
                    av_packet_alloc: find(avcodec, "av_packet_alloc\0")?,
                    av_packet_free: find(avcodec, "av_packet_free\0")?,
                    av_packet_unref: find(avcodec, "av_packet_unref\0")?,
                    av_packet_clone: find(avcodec, "av_packet_clone\0")?,
                    av_frame_alloc: find(avutil, "av_frame_alloc\0")?,
                    av_frame_free: find(avutil, "av_frame_free\0")?,
                    av_hwdevice_ctx_alloc: find(avutil, "av_hwdevice_ctx_alloc\0")?,
                    av_hwdevice_ctx_init: find(avutil, "av_hwdevice_ctx_init\0")?,
                    av_buffer_ref: find(avutil, "av_buffer_ref\0")?,
                    av_buffer_unref: find(avutil, "av_buffer_unref\0")?,
                    av_channel_layout_default: find(avutil, "av_channel_layout_default\0")?,
                    av_channel_layout_uninit: find(avutil, "av_channel_layout_uninit\0")?,
                    av_log_set_level: find(avutil, "av_log_set_level\0")?,
                    av_strerror: find(avutil, "av_strerror\0")?,
                    swr_alloc_set_opts2: find(swresample, "swr_alloc_set_opts2\0")?,
                    swr_init: find(swresample, "swr_init\0")?,
                    swr_convert: find(swresample, "swr_convert\0")?,
                    swr_free: find(swresample, "swr_free\0")?,
                    sws_get_context: find(swscale, "sws_getContext\0")?,
                    sws_scale: find(swscale, "sws_scale\0")?,
                    sws_free_context: find(swscale, "sws_freeContext\0")?,
                }
            };
            // Quiet: ffmpeg's own chatter goes to stderr, where it reads as the
            // program's. Errors still come back through the return codes.
            unsafe { (api.av_log_set_level)(16) };
            Ok(api)
        })
        .as_ref()
        .map_err(Clone::clone)
}

/// Where the DLLs may be, in order: beside the program, which is where the
/// installer puts them; a folder named by `MATTERLESS_FFMPEG`; and, in a debug
/// build, the copy the repository's setup fetched.
fn folders() -> Vec<PathBuf> {
    let mut folders = Vec::new();
    if let Some(beside) = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(Path::to_path_buf))
    {
        folders.push(beside);
    }
    if let Some(named) = std::env::var_os("MATTERLESS_FFMPEG") {
        folders.push(PathBuf::from(named));
    }
    if cfg!(debug_assertions) {
        folders.push(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../third_party/ffmpeg/bin"));
    }
    folders
}

/// Opens one DLL, looking for what it depends on beside it rather than beside
/// the program.
fn open(path: &Path) -> Result<HMODULE, String> {
    let wide: Vec<u16> = path
        .as_os_str()
        .to_string_lossy()
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();
    unsafe { LoadLibraryExW(PCWSTR(wide.as_ptr()), None, LOAD_WITH_ALTERED_SEARCH_PATH) }
        .map_err(|why| format!("{}: {why}", path.display()))
}

/// One function, by its null-terminated name.
///
/// # Safety
///
/// `T` must be the function's real signature: nothing can check it.
unsafe fn find<T: Copy>(module: HMODULE, name: &str) -> Result<T, String> {
    let found = unsafe { GetProcAddress(module, PCSTR(name.as_ptr())) }
        .ok_or_else(|| format!("ffmpeg has no {}", name.trim_end_matches('\0')))?;
    Ok(unsafe { std::mem::transmute_copy(&found) })
}

/// An ffmpeg error code, in words.
pub fn error(api: &Api, code: c_int) -> String {
    let mut buffer = [0 as c_char; 128];
    unsafe { (api.av_strerror)(code, buffer.as_mut_ptr(), buffer.len()) };
    unsafe { std::ffi::CStr::from_ptr(buffer.as_ptr()) }
        .to_string_lossy()
        .into_owned()
}
