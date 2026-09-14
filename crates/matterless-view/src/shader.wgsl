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
// device's limit, so there is nothing to be gained from a binding array and no
// reason to leave WGSL for it.

struct Viewport {
    // Pixels to clip space, and how far the list is scrolled.
    size: vec2<f32>,
    scroll: f32,
    _pad: f32,
};

@group(0) @binding(0) var<uniform> viewport: Viewport;
// Point, for a glyph, which is rasterised at the size it is drawn.
@group(0) @binding(1) var atlas_sampler: sampler;
// Linear, for a picture scaled to whatever box the layout reserved.
@group(0) @binding(2) var smooth_sampler: sampler;
// The sheets, in the order `Sheet` lists them, and then the one picture a
// reader has opened -- which has a texture to itself because it is far too
// large to share one.
@group(0) @binding(3) var letters: texture_2d<f32>;
@group(0) @binding(4) var faces: texture_2d<f32>;
@group(0) @binding(5) var pictures: texture_2d<f32>;
@group(0) @binding(6) var shown: texture_2d<f32>;

struct In {
    @location(0) position: vec2<f32>,
    @location(1) uv: vec2<f32>,
    @location(2) colour: vec4<f32>,
    // Which sheet to sample. It also decides the sampler: the letters sheet
    // holds glyphs and the white texel, which want point sampling, and
    // everything else is a picture scaled to a box, which wants linear.
    @location(3) sheet: u32,
    // Where this corner sits relative to the quad's middle, and how big half
    // the quad is. Together they let the fragment know where it is inside the
    // rectangle without the rectangle being uploaded twice.
    @location(4) local: vec2<f32>,
    @location(5) half_size: vec2<f32>,
    // Zero for a square corner, which is every glyph and most fills.
    @location(6) radius: f32,
    // How far the edge fades. One pixel is a crisp shape; twenty is a shadow.
    @location(7) softness: f32,
};

struct Out {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) colour: vec4<f32>,
    @location(2) @interpolate(flat) sheet: u32,
    @location(3) local: vec2<f32>,
    @location(4) half_size: vec2<f32>,
    @location(5) radius: f32,
    @location(6) softness: f32,
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
    out.sheet = in.sheet;
    out.local = in.local;
    out.half_size = in.half_size;
    out.radius = in.radius;
    out.softness = in.softness;
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
    // A letter is white in its sheet with its coverage in the alpha, so this
    // multiply tints it. An emoji carries its own colour and arrives with a
    // white vertex, so the same multiply leaves it alone.
    //
    // `textureSampleLevel` rather than `textureSample`: a sample that picks its
    // own mip is only defined in uniform control flow, and which sheet a quad
    // wants is decided per quad. Nothing here has mips, so asking for level
    // zero outright costs nothing and is allowed anywhere.
    var texel: vec4<f32>;
    if in.sheet == 0u {
        texel = textureSampleLevel(letters, atlas_sampler, in.uv, 0.0);
    } else if in.sheet == 1u {
        texel = textureSampleLevel(faces, smooth_sampler, in.uv, 0.0);
    } else if in.sheet == 2u {
        texel = textureSampleLevel(pictures, smooth_sampler, in.uv, 0.0);
    } else {
        texel = textureSampleLevel(shown, smooth_sampler, in.uv, 0.0);
    }
    // A corner is cut here rather than by building the shape out of quads: the
    // rectangle is already a quad, and this costs one distance per pixel of it
    // instead of a mesh per rounded thing on screen. One pixel of falloff, so
    // the curve has an edge rather than a staircase.
    //
    // Skipped entirely at radius zero, which is every glyph: the atlas already
    // carries a letter's coverage, and softening the quad it sits on would eat
    // the outermost row of it.
    // A shadow is the same shape with a wide falloff, which the distance
    // already gives: `box-shadow` blurred over twenty pixels is this function
    // with `softness` at twenty instead of one. No second pass, no blur
    // kernel, and no texture for something the geometry already describes.
    var coverage = 1.0;
    if in.radius > 0.0 || in.softness > 1.0 {
        let distance = rounded_box(in.local, in.half_size, in.radius);
        let fade = max(in.softness, 1.0) * 0.5;
        coverage = 1.0 - smoothstep(-fade, fade, distance);
    }
    return vec4<f32>(texel.rgb * in.colour.rgb, texel.a * in.colour.a * coverage);
}
