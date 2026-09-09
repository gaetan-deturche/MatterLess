// One pipeline for glyphs and for solid rectangles.
//
// A rectangle samples the atlas's opaque corner texel, so its coverage is 1 and
// its colour comes through unchanged. That keeps the whole frame in one draw
// call: a chat row is a few dozen quads, and splitting them by kind would cost
// more in state changes than it saves.

struct Viewport {
    // Pixels to clip space, and how far the list is scrolled.
    size: vec2<f32>,
    scroll: f32,
    _pad: f32,
};

@group(0) @binding(0) var<uniform> viewport: Viewport;
@group(0) @binding(1) var atlas: texture_2d<f32>;
@group(0) @binding(2) var atlas_sampler: sampler;

struct In {
    @location(0) position: vec2<f32>,
    @location(1) uv: vec2<f32>,
    @location(2) colour: vec4<f32>,
};

struct Out {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) colour: vec4<f32>,
};

@vertex
fn vertex(in: In) -> Out {
    var out: Out;
    // Scrolling happens here rather than by rebuilding the geometry: the list
    // is the same quads at a different offset.
    let onscreen = vec2<f32>(in.position.x, in.position.y - viewport.scroll);
    // Pixels, top-left origin, to clip space.
    let ndc = vec2<f32>(
        onscreen.x / viewport.size.x * 2.0 - 1.0,
        1.0 - onscreen.y / viewport.size.y * 2.0,
    );
    out.clip = vec4<f32>(ndc, 0.0, 1.0);
    out.uv = in.uv;
    out.colour = in.colour;
    return out;
}

@fragment
fn fragment(in: Out) -> @location(0) vec4<f32> {
    // A letter is white in the atlas with its coverage in the alpha, so this
    // multiply tints it. An emoji carries its own colour and arrives with a
    // white vertex, so the same multiply leaves it alone.
    let texel = textureSample(atlas, atlas_sampler, in.uv);
    return vec4<f32>(texel.rgb * in.colour.rgb, texel.a * in.colour.a);
}
