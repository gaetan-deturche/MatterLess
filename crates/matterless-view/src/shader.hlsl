// One pipeline for glyphs, solid rectangles and pictures.
//
// A rectangle samples the letters sheet's opaque corner texel, so its coverage
// is 1 and its colour comes through unchanged. Everything on screen is a quad
// and every quad goes through here.
//
// The sheets are bound together rather than one at a time, and each quad names
// the one it wants. Binding them in turn meant a draw call per switch -- a row
// is a fill, its words, a face and its words again -- which is thirty-odd draws
// for a frame that is otherwise five. Four textures is well inside every
// device's limit, so there is nothing to be gained from an array of them.
//
// Written in HLSL rather than translated into it. It was WGSL while the window
// drew through wgpu, and kept being WGSL through the Vulkan port because naga
// could turn it into SPIR-V -- but naga's HLSL is written against D3D12's
// sampler heaps, which Direct3D 11 has no idea about. One shader in the
// language the device speaks beats two that have to be kept saying the same
// thing.

cbuffer Viewport : register(b0) {
    // Pixels to clip space, and how far the list is scrolled.
    float2 viewport_size;
    float viewport_scroll;
    float viewport_pad;
};

// The sheets, in the order `Sheet` lists them, and then the one picture a
// reader has opened -- which has a texture to itself because it is far too
// large to share one.
Texture2D<float4> letters : register(t0);
Texture2D<float4> faces : register(t1);
Texture2D<float4> pictures : register(t2);
Texture2D<float4> shown : register(t3);

// Point, for a glyph, which is rasterised at the size it is drawn.
SamplerState atlas_sampler : register(s0);
// Linear, for a picture scaled to whatever box the layout reserved.
SamplerState smooth_sampler : register(s1);

struct In {
    float2 position : POSITION;
    float2 uv : TEXCOORD0;
    float4 colour : COLOR0;
    // Which sheet to sample. It also decides the sampler: the letters sheet
    // holds glyphs and the white texel, which want point sampling, and
    // everything else is a picture scaled to a box, which wants linear.
    uint sheet : TEXCOORD1;
    // Where this corner sits relative to the quad's middle, and how big half
    // the quad is. Together they let the fragment know where it is inside the
    // rectangle without the rectangle being uploaded twice.
    float2 local : TEXCOORD2;
    float2 half_size : TEXCOORD3;
    // Zero for a square corner, which is every glyph and most fills.
    float radius : TEXCOORD4;
    // How far the edge fades. One pixel is a crisp shape; twenty is a shadow.
    float softness : TEXCOORD5;
};

struct Out {
    float4 clip : SV_POSITION;
    float2 uv : TEXCOORD0;
    float4 colour : COLOR0;
    nointerpolation uint sheet : TEXCOORD1;
    float2 local : TEXCOORD2;
    float2 half_size : TEXCOORD3;
    float radius : TEXCOORD4;
    float softness : TEXCOORD5;
};

Out vertex(In input) {
    Out output;
    // Scrolling happens here rather than by rebuilding the geometry: the list
    // is the same quads at a different offset.
    float2 onscreen = float2(input.position.x, input.position.y - viewport_scroll);
    // Pixels, top-left origin, to clip space.
    float2 ndc = float2(
        onscreen.x / viewport_size.x * 2.0 - 1.0,
        1.0 - onscreen.y / viewport_size.y * 2.0
    );
    output.clip = float4(ndc, 0.0, 1.0);
    output.uv = input.uv;
    output.colour = input.colour;
    output.sheet = input.sheet;
    output.local = input.local;
    output.half_size = input.half_size;
    output.radius = input.radius;
    output.softness = input.softness;
    return output;
}

// How far a point is outside a rectangle with rounded corners. Negative
// inside, zero on the edge. The standard rounded-box distance: shrink the box
// by the radius, measure to that, and give back the radius.
float rounded_box(float2 at, float2 half_size, float radius) {
    // Not `point`: HLSL keeps that for a geometry shader's primitive type,
    // and the error it gives is about `abs` taking no arguments.
    float2 corner = abs(at) - half_size + float2(radius, radius);
    return length(max(corner, float2(0.0, 0.0))) + min(max(corner.x, corner.y), 0.0) - radius;
}

float4 fragment(Out input) : SV_Target0 {
    // A letter is white in its sheet with its coverage in the alpha, so this
    // multiply tints it. An emoji carries its own colour and arrives with a
    // white vertex, so the same multiply leaves it alone.
    //
    // `SampleLevel` rather than `Sample`: a sample that picks its own mip is
    // only defined in uniform control flow, and which sheet a quad wants is
    // decided per quad. Nothing here has mips, so asking for level zero
    // outright costs nothing and is allowed anywhere.
    float4 texel;
    if (input.sheet == 0u) {
        texel = letters.SampleLevel(atlas_sampler, input.uv, 0.0);
    } else if (input.sheet == 1u) {
        texel = faces.SampleLevel(smooth_sampler, input.uv, 0.0);
    } else if (input.sheet == 2u) {
        texel = pictures.SampleLevel(smooth_sampler, input.uv, 0.0);
    } else if (input.sheet == 3u) {
        texel = shown.SampleLevel(smooth_sampler, input.uv, 0.0);
    } else {
        // The letters sheet again, filtered: a mark is rasterised at twice the
        // size it is drawn so that its detail survives as shades rather than
        // being hinted into hard stems, and drawing it down is what asks for
        // the other sampler.
        texel = letters.SampleLevel(smooth_sampler, input.uv, 0.0);
    }
    // A corner is cut here rather than by building the shape out of quads: the
    // rectangle is already a quad, and this costs one distance per pixel of it
    // instead of a mesh per rounded thing on screen. One pixel of falloff, so
    // the curve has an edge rather than a staircase.
    //
    // Skipped entirely at radius zero, which is every glyph: the atlas already
    // carries a letter's coverage, and softening the quad it sits on would eat
    // the outermost row of it.
    //
    // A shadow is the same shape with a wide falloff, which the distance
    // already gives: `box-shadow` blurred over twenty pixels is this function
    // with `softness` at twenty instead of one. No second pass, no blur
    // kernel, and no texture for something the geometry already describes.
    float coverage = 1.0;
    if (input.radius > 0.0 || input.softness > 1.0) {
        float distance = rounded_box(input.local, input.half_size, input.radius);
        float fade = max(input.softness, 1.0) * 0.5;
        coverage = 1.0 - smoothstep(-fade, fade, distance);
    }
    return float4(texel.rgb * input.colour.rgb, texel.a * input.colour.a * coverage);
}
