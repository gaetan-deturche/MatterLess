//! The shaders everything is drawn with, and the state around them.
//!
//! One pipeline for glyphs, rectangles and pictures: every quad names the
//! sheet it samples, so a row of text and the face beside it are one draw
//! rather than four.

use crate::d3d::Gpu;
use windows::Win32::Graphics::Direct3D::Fxc::{D3DCOMPILE_OPTIMIZATION_LEVEL3, D3DCompile};
use windows::Win32::Graphics::Direct3D::ID3DBlob;
use windows::Win32::Graphics::Direct3D11::*;
use windows::Win32::Graphics::Dxgi::Common::{
    DXGI_FORMAT_R32_FLOAT, DXGI_FORMAT_R32_UINT, DXGI_FORMAT_R32G32_FLOAT,
    DXGI_FORMAT_R32G32B32A32_FLOAT,
};
use windows::core::PCSTR;

/// The shader, as written. Compiled at startup by the runtime compiler that
/// ships with Windows, which takes about a millisecond for something this
/// size and saves carrying a compiler of our own or bytecode in the tree.
const SHADER: &str = include_str!("shader.hlsl");

/// What the shader is given once a frame.
///
/// Matches the `Viewport` block in the shader: two floats of size, the scroll,
/// and a float of padding, because a constant buffer is measured in
/// sixteen-byte lots.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct Viewport {
    pub size: [f32; 2],
    pub scroll: f32,
    pub padding: f32,
}

/// Everything bound to draw with, built once.
pub struct Bound {
    pub vertex: ID3D11VertexShader,
    pub fragment: ID3D11PixelShader,
    pub layout: ID3D11InputLayout,
    pub blend: ID3D11BlendState,
    /// Scissoring on, because a layer is a draw and a clip.
    pub raster: ID3D11RasterizerState,
    pub point: ID3D11SamplerState,
    pub smooth: ID3D11SamplerState,
    pub uniform: ID3D11Buffer,
}

impl Bound {
    pub fn new(gpu: &Gpu) -> Result<Self, String> {
        let vertex_code = compile("vertex", "vs_5_0")?;
        let fragment_code = compile("fragment", "ps_5_0")?;

        let mut vertex: Option<ID3D11VertexShader> = None;
        unsafe {
            gpu.device
                .CreateVertexShader(bytes_of(&vertex_code), None, Some(&mut vertex))
        }
        .map_err(|why| why.to_string())?;
        let mut fragment: Option<ID3D11PixelShader> = None;
        unsafe {
            gpu.device
                .CreatePixelShader(bytes_of(&fragment_code), None, Some(&mut fragment))
        }
        .map_err(|why| why.to_string())?;

        // The fields of `Vertex`, in order and at their offsets. Named by the
        // semantics the shader declares rather than by position, so a field
        // that moves is a compile error over there rather than a wrong colour
        // over here.
        let elements = [
            element(c"POSITION", 0, DXGI_FORMAT_R32G32_FLOAT, 0),
            element(c"TEXCOORD", 0, DXGI_FORMAT_R32G32_FLOAT, 8),
            element(c"COLOR", 0, DXGI_FORMAT_R32G32B32A32_FLOAT, 16),
            element(c"TEXCOORD", 1, DXGI_FORMAT_R32_UINT, 32),
            element(c"TEXCOORD", 2, DXGI_FORMAT_R32G32_FLOAT, 36),
            element(c"TEXCOORD", 3, DXGI_FORMAT_R32G32_FLOAT, 44),
            element(c"TEXCOORD", 4, DXGI_FORMAT_R32_FLOAT, 52),
            element(c"TEXCOORD", 5, DXGI_FORMAT_R32_FLOAT, 56),
        ];
        let mut layout: Option<ID3D11InputLayout> = None;
        unsafe {
            gpu.device
                .CreateInputLayout(&elements, bytes_of(&vertex_code), Some(&mut layout))
        }
        .map_err(|why| why.to_string())?;

        // Straight alpha, which is what every colour in this program is
        // written in and what the CPU snapshot blends with.
        let mut blending = D3D11_BLEND_DESC::default();
        blending.RenderTarget[0] = D3D11_RENDER_TARGET_BLEND_DESC {
            BlendEnable: true.into(),
            // The second colour the fragment writes, not its alpha.
            //
            // It carries a coverage per channel, so a subpixel letter weighs
            // red, green and blue separately against what is already there --
            // which is the whole of what subpixel antialiasing is, and cannot
            // be said with one alpha. Everything else writes its alpha into
            // all three, so this is exactly `SRC_ALPHA / INV_SRC_ALPHA` for
            // every quad that is not a letter.
            SrcBlend: D3D11_BLEND_SRC1_COLOR,
            DestBlend: D3D11_BLEND_INV_SRC1_COLOR,
            BlendOp: D3D11_BLEND_OP_ADD,
            SrcBlendAlpha: D3D11_BLEND_ONE,
            DestBlendAlpha: D3D11_BLEND_INV_SRC1_ALPHA,
            BlendOpAlpha: D3D11_BLEND_OP_ADD,
            RenderTargetWriteMask: D3D11_COLOR_WRITE_ENABLE_ALL.0 as u8,
        };
        let mut blend: Option<ID3D11BlendState> = None;
        unsafe { gpu.device.CreateBlendState(&blending, Some(&mut blend)) }
            .map_err(|why| why.to_string())?;

        let rastering = D3D11_RASTERIZER_DESC {
            FillMode: D3D11_FILL_SOLID,
            CullMode: D3D11_CULL_NONE,
            ScissorEnable: true.into(),
            ..Default::default()
        };
        let mut raster: Option<ID3D11RasterizerState> = None;
        unsafe {
            gpu.device
                .CreateRasterizerState(&rastering, Some(&mut raster))
        }
        .map_err(|why| why.to_string())?;

        // Point for the letters sheet, which holds glyphs rasterised at the
        // size they are drawn and the one opaque texel a plain fill samples.
        // Linear for everything else, which is a picture scaled to a box.
        let point = sampler(gpu, D3D11_FILTER_MIN_MAG_MIP_POINT)?;
        let smooth = sampler(gpu, D3D11_FILTER_MIN_MAG_MIP_LINEAR)?;

        let how = D3D11_BUFFER_DESC {
            ByteWidth: std::mem::size_of::<Viewport>() as u32,
            Usage: D3D11_USAGE_DYNAMIC,
            BindFlags: D3D11_BIND_CONSTANT_BUFFER.0 as u32,
            CPUAccessFlags: D3D11_CPU_ACCESS_WRITE.0 as u32,
            ..Default::default()
        };
        let mut uniform: Option<ID3D11Buffer> = None;
        unsafe { gpu.device.CreateBuffer(&how, None, Some(&mut uniform)) }
            .map_err(|why| why.to_string())?;

        Ok(Self {
            vertex: vertex.ok_or("no vertex shader")?,
            fragment: fragment.ok_or("no fragment shader")?,
            layout: layout.ok_or("no input layout")?,
            blend: blend.ok_or("no blend state")?,
            raster: raster.ok_or("no rasteriser state")?,
            point,
            smooth,
            uniform: uniform.ok_or("no constant buffer")?,
        })
    }
}

/// Compiles one entry point of the shader.
fn compile(entry: &str, profile: &str) -> Result<ID3DBlob, String> {
    let entry = std::ffi::CString::new(entry).map_err(|why| why.to_string())?;
    let profile = std::ffi::CString::new(profile).map_err(|why| why.to_string())?;
    let name = c"shader.hlsl";
    let mut code: Option<ID3DBlob> = None;
    let mut trouble: Option<ID3DBlob> = None;
    let done = unsafe {
        D3DCompile(
            SHADER.as_ptr() as *const _,
            SHADER.len(),
            PCSTR(name.as_ptr() as *const u8),
            None,
            None,
            PCSTR(entry.as_ptr() as *const u8),
            PCSTR(profile.as_ptr() as *const u8),
            D3DCOMPILE_OPTIMIZATION_LEVEL3,
            0,
            &mut code,
            Some(&mut trouble),
        )
    };
    if let Err(why) = done {
        // What the compiler actually said, which is the only useful half.
        let said = trouble
            .as_ref()
            .map(|blob| unsafe {
                let at = blob.GetBufferPointer() as *const u8;
                let many = blob.GetBufferSize();
                String::from_utf8_lossy(std::slice::from_raw_parts(at, many)).to_string()
            })
            .unwrap_or_default();
        return Err(format!("{entry:?} would not compile: {why}\n{said}"));
    }
    code.ok_or_else(|| "the compiler produced nothing".to_string())
}

fn bytes_of(blob: &ID3DBlob) -> &[u8] {
    // SAFETY: the blob owns the bytes and outlives the borrow.
    unsafe {
        std::slice::from_raw_parts(blob.GetBufferPointer() as *const u8, blob.GetBufferSize())
    }
}

fn element(
    name: &'static std::ffi::CStr,
    index: u32,
    format: windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT,
    offset: u32,
) -> D3D11_INPUT_ELEMENT_DESC {
    D3D11_INPUT_ELEMENT_DESC {
        SemanticName: PCSTR(name.as_ptr() as *const u8),
        SemanticIndex: index,
        Format: format,
        InputSlot: 0,
        AlignedByteOffset: offset,
        InputSlotClass: D3D11_INPUT_PER_VERTEX_DATA,
        InstanceDataStepRate: 0,
    }
}

fn sampler(gpu: &Gpu, filter: D3D11_FILTER) -> Result<ID3D11SamplerState, String> {
    let how = D3D11_SAMPLER_DESC {
        Filter: filter,
        // Clamped, so sampling the edge of a slot cannot wrap round to the
        // other side of the sheet.
        AddressU: D3D11_TEXTURE_ADDRESS_CLAMP,
        AddressV: D3D11_TEXTURE_ADDRESS_CLAMP,
        AddressW: D3D11_TEXTURE_ADDRESS_CLAMP,
        MaxLOD: f32::MAX,
        ..Default::default()
    };
    let mut made: Option<ID3D11SamplerState> = None;
    unsafe { gpu.device.CreateSamplerState(&how, Some(&mut made)) }
        .map_err(|why| why.to_string())?;
    made.ok_or_else(|| "no sampler".to_string())
}
