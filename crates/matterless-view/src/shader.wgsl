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
// Linear, for a picture scaled to whatever box the layout reserved. A glyph is
// rasterised at the size it is drawn and wants the point sampler above.
@group(0) @binding(3) var smooth_sampler: sampler;

struct In {
    @location(0) position: vec2<f32>,
    @location(1) uv: vec2<f32>,
    @location(2) colour: vec4<f32>,
    @location(3) filtered: f32,
    // Where this corner sits relative to the quad's middle, and how big half
    // the quad is. Together they let the fragment know where it is inside the
    // rectangle without the rectangle being uploaded twice.
    @location(4) local: vec2<f32>,
    @location(5) half_size: vec2<f32>,
    // Zero for a square corner, which is every glyph and most fills.
    @location(6) radius: f32,
};

struct Out {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) colour: vec4<f32>,
    @location(2) filtered: f32,
    @location(3) local: vec2<f32>,
    @location(4) half_size: vec2<f32>,
    @location(5) radius: f32,
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
    out.filtered = in.filtered;
    out.local = in.local;
    out.half_size = in.half_size;
    out.radius = in.radius;
    return out;
}

// How far a point is outside a rectangle with rounded corners. Negative
// inside, zero on the edge. The standard rounded-box distance: shrink the box
// by the radius, measure to that, and give back the radius.
fn rounded_box(point: vec2<f32>, half_size: vec2<f32>, radius: f32) -> f32 {
    let corner = abs(point) - half_size + vec2<f32>(radius);
    return length(max(corner, vec2<f32>(0.0))) + min(max(corner.x, corner.y), 0.0) - radius;
}

@fragment
fn fragment(in: Out) -> @location(0) vec4<f32> {
    // A letter is white in the atlas with its coverage in the alpha, so this
    // multiply tints it. An emoji carries its own colour and arrives with a
    // white vertex, so the same multiply leaves it alone.
    // Both are sampled and one is chosen, rather than branching: a texture
    // sample inside non-uniform control flow is undefined, and a select is
    // free next to the fetches it picks between.
    let sharp = textureSample(atlas, atlas_sampler, in.uv);
    let soft = textureSample(atlas, smooth_sampler, in.uv);
    let texel = select(sharp, soft, in.filtered > 0.5);
    // A corner is cut here rather than by building the shape out of quads: the
    // rectangle is already a quad, and this costs one distance per pixel of it
    // instead of a mesh per rounded thing on screen. One pixel of falloff, so
    // the curve has an edge rather than a staircase.
    //
    // Skipped entirely at radius zero, which is every glyph: the atlas already
    // carries a letter's coverage, and softening the quad it sits on would eat
    // the outermost row of it.
    var coverage = 1.0;
    if in.radius > 0.0 {
        let distance = rounded_box(in.local, in.half_size, in.radius);
        coverage = 1.0 - smoothstep(-0.5, 0.5, distance);
    }
    return vec4<f32>(texel.rgb * in.colour.rgb, texel.a * in.colour.a * coverage);
}
