// Pass 1: composite AMOS screens (layers) onto the virtual Amiga display.
// Pass 2: scale the virtual display onto the window surface.

struct LayerUniforms {
    // Area covered by the layer, in display units.
    band: vec4<f32>,       // x, y, w, h
    // Area showing the bitmap, in display units.
    window: vec4<f32>,     // x, y, w, h
    // Source origin (screen offset) and display units per pixel.
    src: vec4<f32>,        // src_x, src_y, scale_x, scale_y
    // Bitmap size, palette rows and transparency.
    info: vec4<i32>,       // width, height, palette_rows, transparent (-1 = none)
    display: vec4<f32>,    // display_w, display_h, format (0 = indexed, 1 = rgba), unused
};

@group(0) @binding(0) var<uniform> layer: LayerUniforms;
@group(0) @binding(1) var indexed_tex: texture_2d<u32>;
@group(0) @binding(2) var rgba_tex: texture_2d<f32>;
@group(0) @binding(3) var palette_tex: texture_2d<f32>;

struct LayerOut {
    @builtin(position) position: vec4<f32>,
};

@vertex
fn layer_vs(@builtin(vertex_index) vi: u32) -> LayerOut {
    let corner = vec2<f32>(f32(vi & 1u), f32((vi >> 1u) & 1u));
    let p = layer.band.xy + corner * layer.band.zw;
    var out: LayerOut;
    let ndc = p / layer.display.xy * 2.0 - 1.0;
    out.position = vec4<f32>(ndc.x, -ndc.y, 0.0, 1.0);
    return out;
}

@fragment
fn layer_fs(in: LayerOut) -> @location(0) vec4<f32> {
    let pos = floor(in.position.xy);
    var row = 0;
    if (layer.info.z > 1) {
        row = clamp(i32(floor((pos.y - layer.band.y) / 2.0)), 0, layer.info.z - 1);
    }
    let win = layer.window;
    let inside = pos.x >= win.x && pos.x < win.x + win.z && pos.y >= win.y && pos.y < win.y + win.w;
    if (!inside) {
        if (layer.info.w >= 0) {
            discard;
        }
        return textureLoad(palette_tex, vec2<i32>(0, row), 0);
    }
    let rel = pos - win.xy;
    let px = vec2<i32>(floor(rel / layer.src.zw) + layer.src.xy);
    if (px.x < 0 || px.y < 0 || px.x >= layer.info.x || px.y >= layer.info.y) {
        discard;
    }
    if (layer.display.z > 0.5) {
        let c = textureLoad(rgba_tex, px, 0);
        if (c.a == 0.0) {
            discard;
        }
        return c;
    }
    let index = i32(textureLoad(indexed_tex, px, 0).r);
    if (index == layer.info.w) {
        discard;
    }
    return textureLoad(palette_tex, vec2<i32>(index, row), 0);
}

// ---------------------------------------------------------------------------

struct BlitUniforms {
    // Destination rectangle in surface pixels.
    rect: vec4<f32>,
    // Source texture size, surface size.
    sizes: vec4<f32>,
    // Shown part of the source texture (x, y, w, h in texels).
    src: vec4<f32>,
};

@group(0) @binding(0) var<uniform> blit: BlitUniforms;
@group(0) @binding(1) var display_tex: texture_2d<f32>;
@group(0) @binding(2) var display_sampler: sampler;

struct BlitOut {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn blit_vs(@builtin(vertex_index) vi: u32) -> BlitOut {
    let corner = vec2<f32>(f32(vi & 1u), f32((vi >> 1u) & 1u));
    let p = blit.rect.xy + corner * blit.rect.zw;
    let ndc = p / blit.sizes.zw * 2.0 - 1.0;
    var out: BlitOut;
    out.position = vec4<f32>(ndc.x, -ndc.y, 0.0, 1.0);
    out.uv = corner;
    return out;
}

// "Sharp bilinear" filtering: nearest neighbour inside source pixels, linear
// blending only across the boundary, so non integer scales stay crisp.
@fragment
fn blit_fs(in: BlitOut) -> @location(0) vec4<f32> {
    let tex_size = blit.sizes.xy;
    let texel = blit.src.xy + in.uv * blit.src.zw;
    let scale = blit.rect.zw / blit.src.zw;
    let texel_floor = floor(texel);
    let frac = fract(texel);
    let region = clamp(0.5 / scale, vec2<f32>(0.0), vec2<f32>(0.5));
    let center_dist = frac - 0.5;
    let f = (center_dist - clamp(center_dist, -region, region)) * scale + 0.5;
    let uv = (texel_floor + f) / tex_size;
    return textureSampleLevel(display_tex, display_sampler, uv, 0.0);
}
