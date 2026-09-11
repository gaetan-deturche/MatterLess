//! Puts the application's icon inside the executable.
//!
//! `Window::with_window_icon` sets an icon with `WM_SETICON`, which cannot
//! happen until there is a window to set it on. Until then Windows has only
//! the executable's own resources to go on -- and a plain `cargo build` embeds
//! none, so the taskbar button and the title bar open with the blank sheet
//! that means "some program" and flip to the real icon a moment later.
//!
//! Embedding it here is what the app's bundler does for the Tauri build, and
//! it is the only thing that can: the icon has to be in the file before the
//! process starts.

fn main() {
    println!("cargo:rerun-if-changed=resources/icons/icon.ico");
    // Only on Windows, and only for a build hosted there: the resource format
    // is a Win32 one, and cross-compiling would need a toolchain that can
    // write it.
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let mut resource = winresource::WindowsResource::new();
    resource.set_icon("resources/icons/icon.ico");
    // A failure here is not worth stopping a build over: the window still sets
    // its own icon once it exists, so the cost is the flicker this removes.
    if let Err(error) = resource.compile() {
        println!("cargo:warning=could not embed the icon: {error}");
    }
}
