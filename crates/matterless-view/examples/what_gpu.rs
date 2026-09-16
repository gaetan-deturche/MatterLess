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
    use windows::Win32::Graphics::Dxgi::{CreateDXGIFactory1, IDXGIFactory1};

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
        // No memory of its own is what an integrated chip looks like from
        // here: it shares the machine's. That is the one the window prefers.
        let kind = match about.DedicatedVideoMemory > 0 {
            true => "discrete",
            false => "integrated -- preferred",
        };
        println!(
            "{named} ({kind}), {} MB of its own",
            about.DedicatedVideoMemory / (1024 * 1024)
        );
    }
    Ok(())
}
