//! Which adapters this machine offers, and which one the window would take.
//!
//! The window asks for one by preference and prints the name it got, which
//! says nothing about what it passed over. On a machine with both an
//! integrated and a discrete GPU that is the difference between the chip in
//! the processor and the card in the slot.
//!
//!     cargo run -p matterless-view --example what_gpu

fn main() {
    #[cfg(windows)]
    match look() {
        Ok(()) => {}
        Err(why) => eprintln!("could not ask Direct3D: {why}"),
    }
    #[cfg(not(windows))]
    println!("Direct3D is a Windows thing");
}

#[cfg(windows)]
fn look() -> Result<(), String> {
    use windows::Win32::Graphics::Dxgi::{
        CreateDXGIFactory1, DXGI_GPU_PREFERENCE_MINIMUM_POWER, IDXGIAdapter, IDXGIFactory1,
        IDXGIFactory6,
    };
    use windows::core::Interface;

    let factory: IDXGIFactory1 = unsafe { CreateDXGIFactory1() }.map_err(|why| why.to_string())?;
    let mut at = 0;
    while let Ok(adapter) = unsafe { factory.EnumAdapters(at) } {
        at += 1;
        let Ok(about) = (unsafe { adapter.GetDesc() }) else {
            continue;
        };
        let named = String::from_utf16_lossy(&about.Description)
            .trim_end_matches(char::from(0))
            .to_string();
        // Memory of its own says little: a chip in the processor can be given
        // a carve-out by the firmware, and this machine's has two gigabytes.
        println!(
            "{named}, {} MB of its own",
            about.DedicatedVideoMemory / (1024 * 1024)
        );
    }
    // The one the window takes: what Windows calls the power saver.
    if let Some(saver) = factory
        .cast::<IDXGIFactory6>()
        .ok()
        .and_then(|six| unsafe {
            six.EnumAdapterByGpuPreference::<IDXGIAdapter>(0, DXGI_GPU_PREFERENCE_MINIMUM_POWER)
                .ok()
        })
        .and_then(|adapter| unsafe { adapter.GetDesc() }.ok())
    {
        println!(
            "the window takes: {}",
            String::from_utf16_lossy(&saver.Description).trim_end_matches(char::from(0))
        );
    }
    Ok(())
}
