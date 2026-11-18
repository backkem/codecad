// Screen-space ambient occlusion (cadview_core::scene3d::Occlusion).
//
// fs_ao: for each pixel, the eye-space point from depth sample 0 and a
// normal from its neighbours; the core's 16 kernel points, turned about the
// normal by the core's 4 x 4 rotation pattern, scaled by the radius. A
// kernel point counts as occluded when the surface drawn at its pixel lies
// in front of it by more than the bias, weighted by the core's depth range
// (so a part far in front does not darken what is behind it).
// Visibility = 1 - strength * share.
// fs_blur: the mean over a 4 x 4 block (every rotation once), skipping
// pixels on other surfaces. fs_composite: multiply the colour by it.
// Fully open pixels come out as exactly 1 and are left untouched.

struct Ao {
    proj: mat4x4<f32>,
    inv_proj: mat4x4<f32>,
    kernel: array<vec4<f32>, 16>,
    // xy: (cos, sin) of the rotation for pixel (x % 4, y % 4), index y * 4 + x.
    rot: array<vec4<f32>, 16>,
    // x: radius; y: strength; z: bias; w: 1 for perspective, 0 orthographic.
    params: vec4<f32>,
    // xy: image size in pixels; zw: depth range in radii (full weight up
    // to z, none from w).
    size: vec4<f32>,
};

@group(0) @binding(0) var<uniform> a: Ao;
@group(0) @binding(1) var depth_tex: texture_depth_multisampled_2d;
@group(0) @binding(2) var ao_in: texture_2d<f32>;

@vertex
fn vs_full(@builtin(vertex_index) vi: u32) -> @builtin(position) vec4<f32> {
    let x = f32(i32(vi & 1u) * 4 - 1);
    let y = f32(i32(vi >> 1u) * 4 - 1);
    return vec4<f32>(x, y, 0.0, 1.0);
}

fn inside(p: vec2<i32>) -> bool {
    return p.x >= 0 && p.y >= 0 && p.x < i32(a.size.x) && p.y < i32(a.size.y);
}

// Eye-space position at pixel `p`; w = 0 where nothing was drawn.
fn eye_at(p: vec2<i32>) -> vec4<f32> {
    let d = textureLoad(depth_tex, p, 0);
    let ndc = vec2<f32>(
        (f32(p.x) + 0.5) / a.size.x * 2.0 - 1.0,
        1.0 - (f32(p.y) + 0.5) / a.size.y * 2.0,
    );
    let e = a.inv_proj * vec4<f32>(ndc, d, 1.0);
    var valid = 1.0;
    if (d >= 1.0) {
        valid = 0.0;
    }
    return vec4<f32>(e.xyz / e.w, valid);
}

fn neighbour(p: vec2<i32>) -> vec4<f32> {
    if (!inside(p)) {
        return vec4<f32>(0.0);
    }
    return eye_at(p);
}

// The step to the neighbour on the same surface: of the two sides, the one
// with the smaller depth change.
fn tangent(c: vec3<f32>, lo: vec4<f32>, hi: vec4<f32>) -> vec3<f32> {
    let d_hi = hi.xyz - c;
    let d_lo = c - lo.xyz;
    if (hi.w == 0.0) {
        return d_lo * lo.w;
    }
    if (lo.w != 0.0 && abs(d_lo.z) < abs(d_hi.z)) {
        return d_lo;
    }
    return d_hi;
}

@fragment
fn fs_ao(@builtin(position) frag: vec4<f32>) -> @location(0) vec4<f32> {
    let p = vec2<i32>(frag.xy);
    let c = eye_at(p);
    if (c.w == 0.0) {
        return vec4<f32>(1.0, 0.0, 0.0, 1.0);
    }
    let dx = tangent(c.xyz, neighbour(p - vec2<i32>(1, 0)), neighbour(p + vec2<i32>(1, 0)));
    // Pixel y grows downward, eye y upward: "down" first keeps the cross
    // product's orientation; it is flipped toward the viewer anyway.
    let dy = tangent(c.xyz, neighbour(p + vec2<i32>(0, 1)), neighbour(p - vec2<i32>(0, 1)));
    var n = cross(dx, dy);
    let len = length(n);
    if (!(len > 0.0)) {
        return vec4<f32>(1.0, 0.0, 0.0, 1.0);
    }
    n = n / len;
    var view = vec3<f32>(0.0, 0.0, 1.0);
    if (a.params.w > 0.5) {
        view = normalize(-c.xyz);
    }
    if (dot(n, view) < 0.0) {
        n = -n;
    }
    var helper = vec3<f32>(1.0, 0.0, 0.0);
    if (abs(n.x) > 0.9) {
        helper = vec3<f32>(0.0, 1.0, 0.0);
    }
    let t0 = normalize(cross(helper, n));
    let b0 = cross(n, t0);
    let r = a.rot[(u32(p.y) % 4u) * 4u + u32(p.x) % 4u].xy;
    let t = t0 * r.x + b0 * r.y;
    let b = cross(n, t);
    let radius = a.params.x;
    var occ = 0.0;
    for (var i = 0u; i < 16u; i++) {
        let k = a.kernel[i].xyz;
        let s = c.xyz + (t * k.x + b * k.y + n * k.z) * radius;
        let cs = a.proj * vec4<f32>(s, 1.0);
        if (cs.w <= 0.0) {
            continue;
        }
        let ndc = cs.xy / cs.w;
        let q = vec2<i32>(floor(vec2<f32>((ndc.x + 1.0) * 0.5 * a.size.x, (1.0 - ndc.y) * 0.5 * a.size.y)));
        if (!inside(q)) {
            continue;
        }
        let o = eye_at(q);
        if (o.w == 0.0) {
            continue;
        }
        if (o.z - s.z > a.params.z) {
            let x = abs(o.z - c.z) / radius;
            occ = occ + clamp((a.size.w - x) / max(a.size.w - a.size.z, 1e-6), 0.0, 1.0);
        }
    }
    let v = 1.0 - a.params.y * occ / 16.0;
    return vec4<f32>(v, 0.0, 0.0, 1.0);
}

@fragment
fn fs_blur(@builtin(position) frag: vec4<f32>) -> @location(0) vec4<f32> {
    let p = vec2<i32>(frag.xy);
    let c = eye_at(p);
    if (c.w == 0.0) {
        return vec4<f32>(1.0, 0.0, 0.0, 1.0);
    }
    let reach = a.size.z * a.params.x;
    var sum = 0.0;
    var count = 0.0;
    for (var y = -2; y < 2; y++) {
        for (var x = -2; x < 2; x++) {
            let q = p + vec2<i32>(x, y);
            if (!inside(q)) {
                continue;
            }
            let o = eye_at(q);
            if (o.w == 0.0 || abs(o.z - c.z) > reach) {
                continue;
            }
            sum = sum + textureLoad(ao_in, q, 0).r;
            count = count + 1.0;
        }
    }
    return vec4<f32>(sum / max(count, 1.0), 0.0, 0.0, 1.0);
}

@fragment
fn fs_composite(@builtin(position) frag: vec4<f32>) -> @location(0) vec4<f32> {
    let v = textureLoad(ao_in, vec2<i32>(frag.xy), 0).r;
    if (v >= 1.0) {
        discard;
    }
    return vec4<f32>(v, v, v, 1.0);
}
