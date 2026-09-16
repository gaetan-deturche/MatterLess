//! Whether Direct3D resizes its swapchain as slowly as Vulkan does.
//!
//! On this machine `vkCreateSwapchainKHR` takes about a hundred milliseconds
//! on the integrated chip and eight on the card. That is a large enough
//! difference to want to know whose fault it is: the driver's Vulkan window
//! path, or something underneath both -- the compositor, or the power
//! management that a laptop chip does and a desktop card does not.
//!
//! So the same thing through DXGI, on the same adapters, with the same shape
//! of test: a swapchain, resized a hundred and twenty times, timed. Nothing is
//! drawn. If Direct3D is fast where Vulkan is slow, the fault is in one
//! driver's WSI; if both are slow, it is below them.
//!
//!     cargo run -p matterless-view --release --example what_d3d_costs

fn main() {
    #[cfg(windows)]
    match measure() {
        Ok(()) => {}
        Err(why) => eprintln!("could not measure it: {why}"),
    }
    #[cfg(not(windows))]
    println!("Direct3D is a Windows thing");
}

#[cfg(windows)]
fn measure() -> Result<(), String> {
    use std::time::{Duration, Instant};
    use windows::Win32::Foundation::HWND;
    use windows::Win32::Graphics::Direct3D::D3D_DRIVER_TYPE_UNKNOWN;
    use windows::Win32::Graphics::Direct3D11::{
        D3D11_CREATE_DEVICE_FLAG, D3D11_SDK_VERSION, D3D11CreateDevice, ID3D11Device,
    };
    use windows::Win32::Graphics::Dxgi::Common::{
        DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_MODE_DESC, DXGI_SAMPLE_DESC,
    };
    use windows::Win32::Graphics::Dxgi::{
        CreateDXGIFactory1, DXGI_SWAP_CHAIN_DESC, DXGI_SWAP_EFFECT_FLIP_DISCARD,
        DXGI_USAGE_RENDER_TARGET_OUTPUT, IDXGIAdapter, IDXGIFactory1, IDXGISwapChain,
    };

    // A window to present to. Not shown -- the Vulkan measurement was the same
    // whether the window was visible or not, and this only has to be the same
    // shape of test.
    let event_loop = winit::event_loop::EventLoop::new().map_err(|why| why.to_string())?;
    #[allow(deprecated)]
    let window = event_loop
        .create_window(
            winit::window::Window::default_attributes()
                .with_title("what d3d costs")
                .with_inner_size(winit::dpi::PhysicalSize::new(1000, 760)),
        )
        .map_err(|why| why.to_string())?;
    let handle = match raw_window_handle::HasWindowHandle::window_handle(&window)
        .map_err(|why| why.to_string())?
        .as_raw()
    {
        raw_window_handle::RawWindowHandle::Win32(win32) => HWND(win32.hwnd.get() as *mut _),
        _ => return Err("not a Win32 window".to_string()),
    };

    let factory: IDXGIFactory1 = unsafe { CreateDXGIFactory1() }.map_err(|why| why.to_string())?;
    let mut at = 0;
    while let Ok(adapter) = unsafe { factory.EnumAdapters(at) } {
        at += 1;
        let Ok(about) = (unsafe { adapter.GetDesc() }) else {
            continue;
        };
        let named = String::from_utf16_lossy(&about.Description)
            .trim_end_matches('\0')
            .to_string();
        // Software adapters answer instantly and say nothing about a driver.
        if named.contains("Microsoft Basic") {
            continue;
        }

        let mut device: Option<ID3D11Device> = None;
        let made = unsafe {
            D3D11CreateDevice(
                &adapter as &IDXGIAdapter,
                // Unknown, because the adapter is named: asking for hardware
                // with an adapter in hand is an error.
                D3D_DRIVER_TYPE_UNKNOWN,
                windows::Win32::Foundation::HMODULE::default(),
                D3D11_CREATE_DEVICE_FLAG(0),
                None,
                D3D11_SDK_VERSION,
                Some(&mut device),
                None,
                None,
            )
        };
        let (Ok(()), Some(device)) = (made, device) else {
            println!("{named}: no device");
            continue;
        };

        let how = DXGI_SWAP_CHAIN_DESC {
            BufferDesc: DXGI_MODE_DESC {
                Width: 1000,
                Height: 760,
                Format: DXGI_FORMAT_B8G8R8A8_UNORM,
                ..Default::default()
            },
            SampleDesc: DXGI_SAMPLE_DESC {
                Count: 1,
                Quality: 0,
            },
            BufferUsage: DXGI_USAGE_RENDER_TARGET_OUTPUT,
            BufferCount: 2,
            OutputWindow: handle,
            Windowed: true.into(),
            SwapEffect: DXGI_SWAP_EFFECT_FLIP_DISCARD,
            ..Default::default()
        };
        let mut chain: Option<IDXGISwapChain> = None;
        if unsafe { factory.CreateSwapChain(&device, &how, &mut chain) }.is_err() {
            println!("{named}: no swapchain");
            continue;
        }
        let Some(chain) = chain else { continue };

        // The same swing of widths the Vulkan measurement uses.
        let mut took = Duration::ZERO;
        let mut worst = Duration::ZERO;
        let rounds = 120;
        for round in 0..rounds {
            let swing = round % 120;
            let by = if swing < 60 { swing } else { 120 - swing };
            let width = 900 + by * 4;
            let began = Instant::now();
            let done = unsafe {
                chain.ResizeBuffers(
                    2,
                    width as u32,
                    760,
                    DXGI_FORMAT_B8G8R8A8_UNORM,
                    windows::Win32::Graphics::Dxgi::DXGI_SWAP_CHAIN_FLAG(0),
                )
            };
            let each = began.elapsed();
            if done.is_err() {
                println!("{named}: a resize was refused");
                break;
            }
            took += each;
            worst = worst.max(each);
        }
        println!(
            "{named}: {rounds} resizes, {:.2}ms each, worst {:.2}ms",
            took.as_secs_f64() * 1000.0 / rounds as f64,
            worst.as_secs_f64() * 1000.0,
        );
    }
    Ok(())
}
