//! Direct3D 11: the device, the swapchain, and the target drawn into.
//!
//! This window drew through wgpu, then through Vulkan by hand, and is here
//! because of one measurement. Resizing rebuilds the swapchain, and on this
//! machine's integrated chip that costs:
//!
//! | | `vkCreateSwapchainKHR` | `ResizeBuffers` |
//! |---|---|---|
//! | AMD Radeon (integrated) | 97.7ms | 1.9ms |
//! | NVIDIA RTX 5070 | 8.1ms | 0.6ms |
//!
//! Fifty times cheaper on the same adapter, so it is not the compositor and it
//! is not power management -- both of those would charge Direct3D the same.
//! It is AMD's Vulkan window path, and there is nothing above it that helps:
//! present mode, frame latency, `oldSwapchain` and building a second surface
//! were each measured and none of them moves it.
//!
//! Which is also why nobody writes this down. Desktop applications draw with
//! Direct3D, where the call costs two milliseconds; Vulkan on Windows is games,
//! and a game builds its swapchain once rather than on every frame of a drag.

use windows::Win32::Foundation::HWND;
use windows::Win32::Graphics::Direct3D::{D3D_DRIVER_TYPE_UNKNOWN, D3D_FEATURE_LEVEL_11_0};
use windows::Win32::Graphics::Direct3D11::{
    D3D11_CREATE_DEVICE_FLAG, D3D11_SDK_VERSION, D3D11_VIEWPORT, D3D11CreateDevice, ID3D11Device,
    ID3D11DeviceContext, ID3D11RenderTargetView, ID3D11Texture2D,
};
use windows::Win32::Graphics::Dxgi::Common::{
    DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_MODE_DESC, DXGI_SAMPLE_DESC,
};
use windows::Win32::Graphics::Dxgi::{
    CreateDXGIFactory1, DXGI_SWAP_CHAIN_DESC, DXGI_SWAP_CHAIN_FLAG, DXGI_SWAP_EFFECT_FLIP_DISCARD,
    DXGI_USAGE_RENDER_TARGET_OUTPUT, IDXGIAdapter, IDXGIFactory1, IDXGISwapChain,
};

/// How many buffers the swapchain holds: one on screen, one being drawn.
const BUFFERS: u32 = 2;

/// The device, the context it is driven through, and the window's swapchain.
pub struct Gpu {
    pub device: ID3D11Device,
    pub context: ID3D11DeviceContext,
    pub chain: IDXGISwapChain,
    /// The view of whichever buffer is being drawn into, rebuilt with the
    /// swapchain because it names a texture that the resize replaces.
    pub target: Option<ID3D11RenderTargetView>,
    pub size: (u32, u32),
}

impl Gpu {
    /// Opens Direct3D for a window that already exists.
    ///
    /// The adapter is chosen the way it always was: the chip in the processor
    /// in preference to the card in the slot. A chat window has no business
    /// waking a discrete GPU to draw a few hundred quads, and on Direct3D that
    /// preference costs nothing -- which was the whole trouble with Vulkan.
    pub fn new(window: HWND, size: (u32, u32)) -> Result<Self, String> {
        let factory: IDXGIFactory1 =
            unsafe { CreateDXGIFactory1() }.map_err(|why| why.to_string())?;
        let adapter = pick(&factory);

        let mut device: Option<ID3D11Device> = None;
        let mut context: Option<ID3D11DeviceContext> = None;
        unsafe {
            D3D11CreateDevice(
                adapter.as_ref(),
                // Unknown when an adapter is named, which is what naming one
                // means: asking for hardware as well is an error.
                match adapter.is_some() {
                    true => D3D_DRIVER_TYPE_UNKNOWN,
                    false => windows::Win32::Graphics::Direct3D::D3D_DRIVER_TYPE_HARDWARE,
                },
                windows::Win32::Foundation::HMODULE::default(),
                D3D11_CREATE_DEVICE_FLAG(0),
                Some(&[D3D_FEATURE_LEVEL_11_0]),
                D3D11_SDK_VERSION,
                Some(&mut device),
                None,
                Some(&mut context),
            )
        }
        .map_err(|why| why.to_string())?;
        let (Some(device), Some(context)) = (device, context) else {
            return Err("no Direct3D device".to_string());
        };

        let how = DXGI_SWAP_CHAIN_DESC {
            BufferDesc: DXGI_MODE_DESC {
                Width: size.0,
                Height: size.1,
                // Straight eight-bit channels, never the sRGB view of them:
                // the palette is written in the numbers it means and the frame
                // is written without a second encoding, so a surface that
                // decodes on the way in is how every colour came out pale.
                Format: DXGI_FORMAT_B8G8R8A8_UNORM,
                ..Default::default()
            },
            SampleDesc: DXGI_SAMPLE_DESC {
                Count: 1,
                Quality: 0,
            },
            BufferUsage: DXGI_USAGE_RENDER_TARGET_OUTPUT,
            BufferCount: BUFFERS,
            OutputWindow: window,
            Windowed: true.into(),
            // Flip, because the alternative is a blit the compositor has to do
            // again on the way to the screen.
            SwapEffect: DXGI_SWAP_EFFECT_FLIP_DISCARD,
            ..Default::default()
        };
        let mut chain: Option<IDXGISwapChain> = None;
        unsafe { factory.CreateSwapChain(&device, &how, &mut chain) }
            .ok()
            .map_err(|why| why.to_string())?;
        let chain = chain.ok_or("no swapchain")?;

        let mut gpu = Self {
            device,
            context,
            chain,
            target: None,
            size,
        };
        gpu.retarget()?;
        Ok(gpu)
    }

    /// Points the target view at the swapchain's current back buffer.
    fn retarget(&mut self) -> Result<(), String> {
        let back: ID3D11Texture2D =
            unsafe { self.chain.GetBuffer(0) }.map_err(|why| why.to_string())?;
        let mut view: Option<ID3D11RenderTargetView> = None;
        unsafe {
            self.device
                .CreateRenderTargetView(&back, None, Some(&mut view))
        }
        .map_err(|why| why.to_string())?;
        self.target = view;
        Ok(())
    }

    /// Builds the buffers again for a new size.
    ///
    /// The target view has to go first: it names the buffer being replaced,
    /// and `ResizeBuffers` refuses while anything still holds one.
    pub fn resize(&mut self, size: (u32, u32)) -> Result<(), String> {
        if size.0 == 0 || size.1 == 0 {
            return Ok(());
        }
        self.target = None;
        unsafe { self.context.OMSetRenderTargets(None, None) };
        unsafe {
            self.chain.ResizeBuffers(
                BUFFERS,
                size.0,
                size.1,
                DXGI_FORMAT_B8G8R8A8_UNORM,
                DXGI_SWAP_CHAIN_FLAG(0),
            )
        }
        .map_err(|why| why.to_string())?;
        self.size = size;
        self.retarget()
    }

    /// The whole window, for the one viewport everything is drawn under.
    pub fn viewport(&self) -> D3D11_VIEWPORT {
        D3D11_VIEWPORT {
            TopLeftX: 0.0,
            TopLeftY: 0.0,
            Width: self.size.0 as f32,
            Height: self.size.1 as f32,
            MinDepth: 0.0,
            MaxDepth: 1.0,
        }
    }
}

/// The adapter to draw on: integrated first, then anything else.
///
/// `None` when nothing can be told apart, which asks Direct3D to choose.
fn pick(factory: &IDXGIFactory1) -> Option<IDXGIAdapter> {
    let mut best: Option<(IDXGIAdapter, u32)> = None;
    let mut at = 0;
    while let Ok(adapter) = unsafe { factory.EnumAdapters(at) } {
        at += 1;
        let Ok(about) = (unsafe { adapter.GetDesc() }) else {
            continue;
        };
        let named = String::from_utf16_lossy(&about.Description);
        // The software one draws nothing anybody wants to look at.
        if named.contains("Microsoft Basic") {
            continue;
        }
        // No memory of its own is what an integrated chip looks like from
        // here: it shares the machine's.
        let rank = u32::from(about.DedicatedVideoMemory > 0);
        if best.as_ref().is_none_or(|(_, was)| rank < *was) {
            best = Some((adapter, rank));
        }
    }
    best.map(|(adapter, _)| adapter)
}
