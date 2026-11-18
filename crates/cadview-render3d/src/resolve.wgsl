// The path tracer's last step: the mean of the accumulated samples, times
// the exposure, through the core's tone curve into the sRGB target (the
// hardware encodes the linear output).

struct Resolve {
    // width, height, -, -
    size: vec4<u32>,
    // x: exposure / samples
    scale: vec4<f32>,
};

@group(0) @binding(0) var<uniform> r: Resolve;
@group(0) @binding(1) var<storage, read> accum: array<vec4<f32>>;

@vertex
fn vs_full(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    let x = f32((i << 1u) & 2u);
    let y = f32(i & 2u);
    return vec4<f32>(x * 2.0 - 1.0, 1.0 - y * 2.0, 0.0, 1.0);
}

// cadview_core::scene3d::tone_map (Khronos PBR Neutral).
fn tone_map(c_in: vec3<f32>) -> vec3<f32> {
    let start = 0.8 - 0.04;
    let desat = 0.15;
    var c = max(c_in, vec3<f32>(0.0));
    let x = min(c.r, min(c.g, c.b));
    var offset = 0.04;
    if (x < 0.08) {
        offset = x - 6.25 * x * x;
    }
    c = c - vec3<f32>(offset);
    let peak = max(c.r, max(c.g, c.b));
    if (peak < start) {
        return c;
    }
    let d = 1.0 - start;
    let new_peak = 1.0 - d * d / (peak + d - start);
    c = c * (new_peak / peak);
    let gg = 1.0 - 1.0 / (desat * (peak - new_peak) + 1.0);
    return mix(c, vec3<f32>(new_peak), gg);
}

@fragment
fn fs_resolve(@builtin(position) pos: vec4<f32>) -> @location(0) vec4<f32> {
    let x = min(u32(pos.x), r.size.x - 1u);
    let y = min(u32(pos.y), r.size.y - 1u);
    let c = accum[y * r.size.x + x].rgb * r.scale.x;
    return vec4<f32>(tone_map(c), 1.0);
}
