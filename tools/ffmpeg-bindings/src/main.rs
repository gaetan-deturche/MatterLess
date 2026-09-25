//! Writes ffmpeg's structs out as Rust, types only: the functions are loaded
//! by name at run time, so nothing here is linked against.

fn main() {
    let here = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let root = here.join("../..");
    let include = root.join("third_party/ffmpeg/include");
    let bindings = bindgen::Builder::default()
        .header(here.join("wrapper.h").to_string_lossy())
        .clang_arg(format!("-I{}", include.display()))
        .allowlist_type("AV(FormatContext|CodecContext|Codec|CodecParameters|Stream|Packet|Frame|Rational|BufferRef|HWDeviceContext|Dictionary|ChannelLayout|IOContext|InputFormat|Class)")
        .allowlist_type("SwrContext")
        .allowlist_type("AV(MediaType|PixelFormat|SampleFormat|HWDeviceType|CodecID|Discard|Rounding)")
        .allowlist_var("AV_NOPTS_VALUE|AV_TIME_BASE|AVSEEK_FLAG_.*|AV_CODEC_FLAG_.*")
        .default_enum_style(bindgen::EnumVariation::Consts)
        .prepend_enum_name(false)
        .layout_tests(false)
        .derive_debug(false)
        .derive_default(true)
        .generate_comments(false)
        .generate()
        .expect("bindings");
    let out = root.join("crates/matterless-media/src/ffmpeg_types.rs");
    bindings.write_to_file(&out).expect("written");
    println!("wrote {}", out.display());
}
