// Working-quality raster shading. Mirrors cadview_core::scene3d::shade and
// tone_map; the target is Rgba8UnormSrgb, so the hardware sRGB-encodes the
// linear output.

struct Globals {
    view_proj: mat4x4<f32>,
    inv_view_proj: mat4x4<f32>,
    // xyz: eye point (w = 1, perspective) or direction toward the viewer (w = 0).
    eye: vec4<f32>,
    // xyz: unit direction toward the key light; w: key intensity.
    key: vec4<f32>,
    // xyz: unit direction toward the fill light; w: fill intensity.
    fill: vec4<f32>,
    // x: ambient; y, z: viewport size in pixels; w: edge half-width in pixels.
    params: vec4<f32>,
    // rgb: linear edge colour; w: pull toward the eye for edges (world units).
    edge: vec4<f32>,
    // rgb: linear outline colour; w: outline half-width in pixels.
    outline: vec4<f32>,
    // Clip planes (n, w): removed where dot(n, p) + w > 0.
    clip: array<vec4<f32>, 4>,
    // x: number of clip planes; y: width of the cut line in pixels.
    clip_info: vec4<f32>,
    // rgb: linear ground colour; w: ground height.
    ground: vec4<f32>,
    // xy: ground centre; z, w: inner and outer fade radius.
    ground_fade: vec4<f32>,
    // x: half the side of the ground square.
    ground_size: vec4<f32>,
    // rgb: linear background colour.
    background: vec4<f32>,
};

struct Item {
    mvp: mat4x4<f32>,
    model: mat4x4<f32>,
    // rgb: linear base colour; a: metallic.
    base: vec4<f32>,
    // x: roughness; y: specular exponent.
    mat: vec4<f32>,
    // rgb: linear section colour.
    section: vec4<f32>,
};

@group(0) @binding(0) var<uniform> g: Globals;
@group(0) @binding(1) var<storage, read> items: array<Item>;

struct MeshOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) world: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) @interpolate(flat) item: u32,
};

@vertex
fn vs_mesh(
    @location(0) p: vec3<f32>,
    @location(1) n: vec3<f32>,
    @builtin(instance_index) ii: u32,
) -> MeshOut {
    var o: MeshOut;
    o.pos = items[ii].mvp * vec4<f32>(p, 1.0);
    o.world = (items[ii].model * vec4<f32>(p, 1.0)).xyz;
    o.normal = (items[ii].model * vec4<f32>(n, 0.0)).xyz;
    o.item = ii;
    return o;
}

// Khronos PBR Neutral, as cadview_core::scene3d::tone_map.
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

// cadview_core::scene3d::shade, tone mapped. `n` faces the viewer.
fn shade(n: vec3<f32>, world: vec3<f32>, it: u32) -> vec3<f32> {
    let v = normalize(g.eye.xyz - world * g.eye.w);
    let l = g.key.xyz;
    let lf = g.fill.xyz;
    let base = clamp(items[it].base.rgb, vec3<f32>(0.0), vec3<f32>(1.0));
    let metallic = clamp(items[it].base.a, 0.0, 1.0);
    let s = items[it].mat.y;
    let kd = base * (1.0 - metallic);
    let f0 = vec3<f32>(0.04) + (base - vec3<f32>(0.04)) * metallic;
    var c = g.params.x * (kd + f0) + g.fill.w * kd * max(dot(n, lf), 0.0);
    let n_l = dot(n, l);
    if (n_l > 0.0) {
        var h = v + l;
        if (length(h) > 1e-6) {
            h = normalize(h);
        } else {
            h = n;
        }
        let n_h = max(dot(n, h), 0.0);
        let v_h = clamp(dot(v, h), 0.0, 1.0);
        let fres = f0 + (vec3<f32>(1.0) - f0) * pow(1.0 - v_h, 5.0);
        let spec = fres * ((s + 8.0) / 8.0) * pow(n_h, s);
        c = c + g.key.w * (kd + spec) * n_l;
    }
    return tone_map(c);
}

// No clip planes: both sides of every triangle, back faces with the normal
// flipped (an open or wrongly oriented mesh still shades).
@fragment
fn fs_mesh(in: MeshOut, @builtin(front_facing) front: bool) -> @location(0) vec4<f32> {
    var n = normalize(in.normal);
    if (!front) {
        n = -n;
    }
    return vec4<f32>(shade(n, in.world, in.item), 1.0);
}

fn clipped(w: vec3<f32>) -> bool {
    let count = u32(g.clip_info.x);
    for (var i = 0u; i < count; i++) {
        if (dot(g.clip[i].xyz, w) + g.clip[i].w > 0.0) {
            return true;
        }
    }
    return false;
}

// With clip planes, front faces: removed on the far side of a plane, and a
// line of the edge colour where a plane cuts the face.
@fragment
fn fs_mesh_clip(in: MeshOut) -> @location(0) vec4<f32> {
    // Distances and their screen derivatives first, in uniform control flow.
    var removed = false;
    var cut_px = 1e9;
    for (var i = 0u; i < 4u; i++) {
        let d = dot(g.clip[i].xyz, in.world) + g.clip[i].w;
        let fw = fwidth(d);
        if (f32(i) < g.clip_info.x) {
            if (d > 0.0) {
                removed = true;
            }
            cut_px = min(cut_px, -d / max(fw, 1e-20));
        }
    }
    if (removed) {
        discard;
    }
    var c = shade(normalize(in.normal), in.world, in.item);
    let w = g.clip_info.y;
    if (w > 0.0) {
        let cover = clamp(w + 0.5 - cut_px, 0.0, 1.0);
        c = mix(c, g.edge.rgb, cover);
    }
    return vec4<f32>(c, 1.0);
}

// With clip planes, back faces: seen only through a cut (meshes are
// closed), so they are the cut face. Flat section colour; drawn at the back
// face's depth, which fs_cut_depth then moves onto the cut.
@fragment
fn fs_cap(in: MeshOut) -> @location(0) vec4<f32> {
    if (clipped(in.world)) {
        discard;
    }
    return vec4<f32>(items[in.item].section.rgb, 1.0);
}

@vertex
fn vs_full(@builtin(vertex_index) vi: u32) -> @builtin(position) vec4<f32> {
    let x = f32(i32(vi & 1u) * 4 - 1);
    let y = f32(i32(vi >> 1u) * 4 - 1);
    return vec4<f32>(x, y, 0.0, 1.0);
}

// On cut-face pixels (stencil 2): the depth where the view ray through the
// pixel enters the kept region, i.e. the depth of the cut itself. Rays that
// enter through no plane keep their depth.
@fragment
fn fs_cut_depth(@builtin(position) frag: vec4<f32>) -> @builtin(frag_depth) f32 {
    let ndc = vec2<f32>(frag.x / g.params.y * 2.0 - 1.0, 1.0 - frag.y / g.params.z * 2.0);
    let h0 = g.inv_view_proj * vec4<f32>(ndc, 0.0, 1.0);
    let h1 = g.inv_view_proj * vec4<f32>(ndc, 1.0, 1.0);
    let p0 = h0.xyz / h0.w;
    let d = h1.xyz / h1.w - p0;
    let count = u32(g.clip_info.x);
    var s = -1.0;
    for (var i = 0u; i < count; i++) {
        let nd = dot(g.clip[i].xyz, d);
        let f0 = dot(g.clip[i].xyz, p0) + g.clip[i].w;
        if (nd < 0.0 && f0 > 0.0) {
            s = max(s, f0 / -nd);
        }
    }
    if (s < 0.0) {
        discard;
    }
    let c = g.view_proj * vec4<f32>(p0 + d * s, 1.0);
    return clamp(c.z / c.w, 0.0, 1.0);
}

// Ground: a square at the ground height, flat colour fading into the
// background with distance from the centre (cadview_core FrameGround).

struct GroundOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) world: vec3<f32>,
};

@vertex
fn vs_ground(@builtin(vertex_index) vi: u32) -> GroundOut {
    var corners = array<vec2<f32>, 6>(
        vec2<f32>(-1.0, -1.0),
        vec2<f32>(1.0, -1.0),
        vec2<f32>(1.0, 1.0),
        vec2<f32>(-1.0, -1.0),
        vec2<f32>(1.0, 1.0),
        vec2<f32>(-1.0, 1.0),
    );
    var o: GroundOut;
    let xy = g.ground_fade.xy + corners[vi] * g.ground_size.x;
    o.world = vec3<f32>(xy, g.ground.w);
    o.pos = g.view_proj * vec4<f32>(o.world, 1.0);
    return o;
}

@fragment
fn fs_ground(in: GroundOut) -> @location(0) vec4<f32> {
    let d = length(in.world.xy - g.ground_fade.xy);
    let a = g.ground_fade.z;
    let b = g.ground_fade.w;
    let t = clamp((d - a) / max(b - a, 1e-12), 0.0, 1.0);
    let s = t * t * (3.0 - 2.0 * t);
    return vec4<f32>(g.ground.rgb + (g.background.rgb - g.ground.rgb) * s, 1.0);
}

// Edges: one instance per segment, six vertices forming a screen-space
// quad of the edge width with square caps.

fn toward_eye(w: vec3<f32>) -> vec3<f32> {
    return normalize(g.eye.xyz - w * g.eye.w);
}

struct EdgeOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) world: vec3<f32>,
};

fn edge_vertex(vi: u32, a: vec3<f32>, b: vec3<f32>, it: u32, hw: f32) -> EdgeOut {
    var corners = array<vec2<f32>, 6>(
        vec2<f32>(0.0, -1.0),
        vec2<f32>(1.0, -1.0),
        vec2<f32>(1.0, 1.0),
        vec2<f32>(0.0, -1.0),
        vec2<f32>(1.0, 1.0),
        vec2<f32>(0.0, 1.0),
    );
    let c = corners[vi];
    let model = items[it].model;
    let wa = (model * vec4<f32>(a, 1.0)).xyz;
    let wb = (model * vec4<f32>(b, 1.0)).xyz;
    let ca = g.view_proj * vec4<f32>(wa + toward_eye(wa) * g.edge.w, 1.0);
    let cb = g.view_proj * vec4<f32>(wb + toward_eye(wb) * g.edge.w, 1.0);
    let half_vp = g.params.yz * 0.5;
    let sa = ca.xy / ca.w * half_vp;
    let sb = cb.xy / cb.w * half_vp;
    var dir = sb - sa;
    if (length(dir) < 1e-6) {
        dir = vec2<f32>(1.0, 0.0);
    } else {
        dir = normalize(dir);
    }
    let side = vec2<f32>(-dir.y, dir.x);
    let off = side * (c.y * hw) + dir * ((c.x * 2.0 - 1.0) * hw);
    var clip = ca;
    var o: EdgeOut;
    o.world = wa;
    if (c.x > 0.5) {
        clip = cb;
        o.world = wb;
    }
    o.pos = vec4<f32>(clip.xy + off / half_vp * clip.w, clip.z, clip.w);
    return o;
}

@vertex
fn vs_edge(
    @builtin(vertex_index) vi: u32,
    @location(0) a: vec3<f32>,
    @location(1) b: vec3<f32>,
    @location(2) it: u32,
) -> EdgeOut {
    return edge_vertex(vi, a, b, it, g.params.w);
}

@vertex
fn vs_outline(
    @builtin(vertex_index) vi: u32,
    @location(0) a: vec3<f32>,
    @location(1) b: vec3<f32>,
    @location(2) it: u32,
) -> EdgeOut {
    return edge_vertex(vi, a, b, it, g.outline.w);
}

@fragment
fn fs_edge(in: EdgeOut) -> @location(0) vec4<f32> {
    if (clipped(in.world)) {
        discard;
    }
    return vec4<f32>(g.edge.rgb, 1.0);
}

@fragment
fn fs_outline(in: EdgeOut) -> @location(0) vec4<f32> {
    if (clipped(in.world)) {
        discard;
    }
    return vec4<f32>(g.outline.rgb, 1.0);
}
