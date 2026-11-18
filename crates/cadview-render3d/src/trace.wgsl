// The presentation render: a progressive path tracer in one compute
// shader, on plain WebGPU (no ray-tracing extensions).
//
// Each invocation owns one pixel and adds `size.w` samples to that
// pixel's running sum in `accum`, one sample at a time, so the sum (and the
// image) is the same however the samples are split into passes. Sample `s`
// of pixel `p` draws its random numbers from PCG hashes of (p, s, seed)
// only: same scene, same samples, same seed, same bytes.
//
// Every decision a second backend must agree on (camera, environment,
// sun, materials and their patterns, exposure, tone curve) comes from
// cadview_core::scene3d and is mirrored here; the formulas are documented
// there. What is this backend's own: the two-level BVH, the BSDF sampling,
// next-event estimation of the sun with multiple importance sampling,
// Russian roulette and the clamp on indirect light.

struct Globals {
    // Camera. origin.w: 1 perspective, 0 orthographic. right.w and up.w:
    // half the view's width and height (tangents for perspective, mm for
    // orthographic). back: unit vector toward the viewer.
    cam_origin: vec4<f32>,
    cam_right: vec4<f32>,
    cam_up: vec4<f32>,
    cam_back: vec4<f32>,
    // width, height, first sample of this pass, samples in this pass.
    size: vec4<u32>,
    // seed, max bounces, flags (1 ground, 2 sun, 4 clip "all"), clip planes.
    params: vec4<u32>,
    // Unit direction toward the sun, w: cosine of its angular radius.
    sun_dir: vec4<f32>,
    // Radiance inside the sun's disc, w: its solid angle.
    sun_radiance: vec4<f32>,
    zenith: vec4<f32>,
    horizon: vec4<f32>,
    nadir: vec4<f32>,
    // Ground albedo, w: ground height.
    ground: vec4<f32>,
    // x: clamp on the luminance of indirect contributions (0 = none).
    misc: vec4<f32>,
    // x: instances.
    counts: vec4<u32>,
    clip: array<vec4<f32>, 4>,
};

struct Node {
    lo: vec3<f32>,
    first: u32,
    hi: vec3<f32>,
    count: u32,
};

// A triangle in its mesh's frame: first corner and two edges; w holds the
// three vertex indices (bitcast) for the normals.
struct Tri {
    p0: vec4<f32>,
    e1: vec4<f32>,
    e2: vec4<f32>,
};

struct Instance {
    to_object: mat4x4<f32>,
    to_world: mat4x4<f32>,
    // x: root node of the mesh's BVH, y: material index, z and w unused.
    info: vec4<u32>,
};

struct Mat {
    // rgb base colour, a metallic
    base: vec4<f32>,
    // roughness, transmission, ior, clearcoat
    surface: vec4<f32>,
    // clearcoat roughness, pattern kind (0 flat, 1 bands, 2 brushed), -, -
    coat: vec4<f32>,
    emission: vec4<f32>,
    // pattern axis (mesh frame), w: spacing (bands) or scale (brushed)
    pat_axis: vec4<f32>,
    // pattern origin (mesh frame), w: seed (bitcast)
    pat_origin: vec4<f32>,
    // bands colour, w: strength (bands) or streaks (brushed)
    pat_color: vec4<f32>,
    // x: warp (bands), y: anisotropy (brushed)
    pat_misc: vec4<f32>,
    // section colour of cut faces
    section: vec4<f32>,
};

@group(0) @binding(0) var<uniform> g: Globals;
@group(0) @binding(1) var<storage, read> tlas: array<Node>;
@group(0) @binding(2) var<storage, read> blas: array<Node>;
@group(0) @binding(3) var<storage, read> tris: array<Tri>;
@group(0) @binding(4) var<storage, read> normals: array<vec4<f32>>;
@group(0) @binding(5) var<storage, read> instances: array<Instance>;
@group(0) @binding(6) var<storage, read> materials: array<Mat>;
@group(0) @binding(7) var<storage, read_write> accum: array<vec4<f32>>;

const PI: f32 = 3.14159265358979;
const INF: f32 = 3.0e38;
// Traversal stack: the build caps the depth at 40 (bvh::MAX_DEPTH).
const STACK: u32 = 42u;

// ── Random numbers ─────────────────────────────────────────────────────

// cadview_core::scene3d::pcg_hash.
fn pcg(x: u32) -> u32 {
    let state = x * 747796405u + 2891336453u;
    let word = ((state >> ((state >> 28u) + 4u)) ^ state) * 277803737u;
    return (word >> 22u) ^ word;
}

var<private> rng: u32;

fn rand() -> f32 {
    rng = pcg(rng);
    return f32(rng >> 8u) * (1.0 / 16777216.0);
}

// ── Patterns (cadview_core::scene3d::value_noise, pattern_albedo) ──────

fn lattice(i: vec3<i32>, seed: u32) -> f32 {
    let u = bitcast<vec3<u32>>(i);
    let h = pcg(u.x ^ pcg(u.y ^ pcg(u.z ^ pcg(seed))));
    return f32(h >> 8u) * (1.0 / 16777216.0);
}

fn value_noise(p: vec3<f32>, seed: u32) -> f32 {
    let f = floor(p);
    let i = vec3<i32>(f);
    let t = p - f;
    let s = t * t * t * (t * (t * 6.0 - 15.0) + 10.0);
    let x00 = mix(lattice(i, seed), lattice(i + vec3<i32>(1, 0, 0), seed), s.x);
    let x10 = mix(lattice(i + vec3<i32>(0, 1, 0), seed), lattice(i + vec3<i32>(1, 1, 0), seed), s.x);
    let x01 = mix(lattice(i + vec3<i32>(0, 0, 1), seed), lattice(i + vec3<i32>(1, 0, 1), seed), s.x);
    let x11 = mix(lattice(i + vec3<i32>(0, 1, 1), seed), lattice(i + vec3<i32>(1, 1, 1), seed), s.x);
    return mix(mix(x00, x10, s.y), mix(x01, x11, s.y), s.z);
}

fn streak_noise(m: Mat, p: vec3<f32>, scale: f32, circular: bool) -> f32 {
    let q = p - m.pat_origin.xyz;
    let along = dot(q, m.pat_axis.xyz);
    let across = q - m.pat_axis.xyz * along;
    let k = 1.0 / max(scale, 1e-6);
    let seed = bitcast<u32>(m.pat_origin.w) ^ 0x9e3779b9u;
    if (circular) {
        return value_noise(vec3<f32>(length(across) * k, along * k, 0.0), seed);
    }
    return value_noise(vec3<f32>(across.x * k, across.y * k, across.z * k + along * k * 0.02), seed);
}

fn bands_weight(m: Mat, p: vec3<f32>) -> f32 {
    let q = p - m.pat_origin.xyz;
    let along = dot(q, m.pat_axis.xyz);
    let across = q - m.pat_axis.xyz * along;
    let spacing = max(m.pat_axis.w, 1e-6);
    let k = 1.0 / (4.0 * spacing);
    let n = value_noise(
        vec3<f32>(across.x * k, across.y * k, across.z * k + along * k * 0.125),
        bitcast<u32>(m.pat_origin.w),
    );
    let r = length(across) + m.pat_misc.x * (2.0 * n - 1.0);
    let ph = r / spacing;
    let f = ph - floor(ph);
    return f * f * f;
}

struct Surface {
    albedo: vec3<f32>,
    metallic: f32,
    // Roughness along and across the pattern axis.
    r_along: f32,
    r_across: f32,
    transmission: f32,
    ior: f32,
    clearcoat: f32,
    coat_roughness: f32,
    emission: vec3<f32>,
    // Unit pattern axis in world space (the anisotropy direction).
    axis: vec3<f32>,
};

fn surface_at(m: Mat, p_mesh: vec3<f32>, to_world: mat4x4<f32>) -> Surface {
    var s: Surface;
    let base = clamp(m.base.rgb, vec3<f32>(0.0), vec3<f32>(1.0));
    s.albedo = base;
    s.metallic = clamp(m.base.a, 0.0, 1.0);
    let r = clamp(m.surface.x, 0.0, 1.0);
    s.r_along = r;
    s.r_across = r;
    s.transmission = clamp(m.surface.y, 0.0, 1.0);
    s.ior = max(m.surface.z, 1.0);
    s.clearcoat = clamp(m.surface.w, 0.0, 1.0);
    s.coat_roughness = clamp(m.coat.x, 0.0, 1.0);
    s.emission = max(m.emission.rgb, vec3<f32>(0.0));
    s.axis = normalize((to_world * vec4<f32>(m.pat_axis.xyz, 0.0)).xyz);
    let kind = u32(m.coat.y + 0.5);
    if (kind == 1u) {
        let st = clamp(m.pat_color.w, 0.0, 1.0);
        let w = st * bands_weight(m, p_mesh);
        let gr = streak_noise(m, p_mesh, 0.2 * m.pat_axis.w, false);
        s.albedo = mix(base, clamp(m.pat_color.rgb, vec3<f32>(0.0), vec3<f32>(1.0)), w) * (1.0 - 0.12 * st * gr);
    } else if (kind == 2u) {
        let st = clamp(m.pat_color.w, 0.0, 1.0);
        let circular = m.pat_misc.z > 0.5;
        if (circular) {
            // cadview_core::scene3d::streak_direction: the tangent of the
            // circle about the axis.
            let t = cross(m.pat_axis.xyz, p_mesh - m.pat_origin.xyz);
            if (dot(t, t) > 1e-12) {
                s.axis = normalize((to_world * vec4<f32>(t, 0.0)).xyz);
            }
        }
        let sn = streak_noise(m, p_mesh, m.pat_axis.w, circular);
        s.albedo = base * (1.0 - 0.25 * st * sn);
        let rr = clamp(r * (1.0 + 0.6 * st * (sn - 0.5)), 0.0, 1.0);
        s.r_across = rr;
        s.r_along = rr * (1.0 - 0.8 * clamp(m.pat_misc.y, 0.0, 1.0));
    }
    return s;
}

// ── Environment (cadview_core::scene3d::Environment::radiance) ─────────

fn dome(d: vec3<f32>) -> vec3<f32> {
    if (d.z >= 0.0) {
        return mix(g.horizon.rgb, g.zenith.rgb, sqrt(min(d.z, 1.0)));
    }
    return g.nadir.rgb;
}

fn in_sun(d: vec3<f32>) -> bool {
    return (g.params.z & 2u) != 0u && dot(d, g.sun_dir.xyz) >= g.sun_dir.w;
}

// ── Intersection ───────────────────────────────────────────────────────

// 1 / d with zero components replaced by a tiny number, so slab tests of
// axis-aligned rays stay finite (0 * inf would be NaN).
fn safe_inv(d: vec3<f32>) -> vec3<f32> {
    let tiny = vec3<f32>(1e-30);
    return 1.0 / select(d, tiny, abs(d) < tiny);
}

// Entry distance of the ray into a box, or INF when it misses within tmax.
fn box_hit(o: vec3<f32>, inv_d: vec3<f32>, lo: vec3<f32>, hi: vec3<f32>, tmax: f32) -> f32 {
    let t0 = (lo - o) * inv_d;
    let t1 = (hi - o) * inv_d;
    let tn = min(t0, t1);
    let tf = max(t0, t1);
    let enter = max(max(tn.x, tn.y), max(tn.z, 0.0));
    let leave = min(min(tf.x, tf.y), min(tf.z, tmax));
    if (enter <= leave) {
        return enter;
    }
    return INF;
}

fn clip_count() -> u32 {
    return g.params.w;
}

// Is world point p cut away?
fn removed(p: vec3<f32>) -> bool {
    let n = clip_count();
    if (n == 0u) {
        return false;
    }
    let all = (g.params.z & 4u) != 0u;
    var beyond = 0u;
    for (var i = 0u; i < n; i++) {
        let c = g.clip[i];
        if (dot(c.xyz, p) + c.w > 0.0) {
            beyond++;
        }
    }
    if (all) {
        return beyond == n;
    }
    return beyond > 0u;
}

struct Hit {
    t: f32,
    inst: u32,
    tri: u32,
    u: f32,
    v: f32,
};

// Möller-Trumbore in the mesh frame; returns (t, u, v) or t = INF.
fn tri_hit(o: vec3<f32>, d: vec3<f32>, tri: Tri, tmax: f32) -> vec3<f32> {
    let e1 = tri.e1.xyz;
    let e2 = tri.e2.xyz;
    let pv = cross(d, e2);
    let det = dot(e1, pv);
    if (abs(det) < 1e-30) {
        return vec3<f32>(INF, 0.0, 0.0);
    }
    let inv = 1.0 / det;
    let tv = o - tri.p0.xyz;
    let u = dot(tv, pv) * inv;
    // A small slack on the edges, so rays through a shared edge do not slip
    // between two triangles (Möller-Trumbore is not watertight).
    let slack = 1e-6;
    if (u < -slack || u > 1.0 + slack) {
        return vec3<f32>(INF, 0.0, 0.0);
    }
    let qv = cross(tv, e1);
    let v = dot(d, qv) * inv;
    if (v < -slack || u + v > 1.0 + slack) {
        return vec3<f32>(INF, 0.0, 0.0);
    }
    let t = dot(e2, qv) * inv;
    if (t <= 0.0 || t >= tmax) {
        return vec3<f32>(INF, 0.0, 0.0);
    }
    return vec3<f32>(t, u, v);
}

// Closest hit (any_hit false) or any hit (true) within tmax, both levels.
// The ray direction is not renormalised in the mesh frame (placements are
// rigid, so t means the same distance in both).
fn trace(o: vec3<f32>, d: vec3<f32>, tmax_in: f32, any_hit: bool) -> Hit {
    var best: Hit;
    best.t = tmax_in;
    best.inst = 0xffffffffu;
    let inv_d = safe_inv(d);
    var stack: array<u32, STACK>;
    var sp = 0u;
    var node = 0u;
    loop {
        let n = tlas[node];
        var next_set = false;
        if (n.count > 0u) {
            // A leaf holds one instance.
            let ii = n.first;
            let inst = instances[ii];
            let lo = (inst.to_object * vec4<f32>(o, 1.0)).xyz;
            let ld = (inst.to_object * vec4<f32>(d, 0.0)).xyz;
            let inv_ld = safe_inv(ld);
            var bstack: array<u32, STACK>;
            var bsp = 0u;
            var bn = inst.info.x;
            loop {
                let m = blas[bn];
                var bnext = false;
                if (m.count > 0u) {
                    for (var k = 0u; k < m.count; k++) {
                        let ti = m.first + k;
                        let h = tri_hit(lo, ld, tris[ti], best.t);
                        if (h.x < best.t) {
                            if (clip_count() > 0u && removed(o + d * h.x)) {
                                continue;
                            }
                            best.t = h.x;
                            best.inst = ii;
                            best.tri = ti;
                            best.u = h.y;
                            best.v = h.z;
                            if (any_hit) {
                                return best;
                            }
                        }
                    }
                } else {
                    let a = m.first;
                    let ta = box_hit(lo, inv_ld, blas[a].lo, blas[a].hi, best.t);
                    let tb = box_hit(lo, inv_ld, blas[a + 1u].lo, blas[a + 1u].hi, best.t);
                    if (ta < INF && tb < INF) {
                        if (ta <= tb) {
                            bstack[bsp] = a + 1u;
                            bn = a;
                        } else {
                            bstack[bsp] = a;
                            bn = a + 1u;
                        }
                        bsp++;
                        bnext = true;
                    } else if (ta < INF) {
                        bn = a;
                        bnext = true;
                    } else if (tb < INF) {
                        bn = a + 1u;
                        bnext = true;
                    }
                }
                if (!bnext) {
                    var more = false;
                    loop {
                        if (bsp == 0u) {
                            break;
                        }
                        bsp--;
                        bn = bstack[bsp];
                        if (box_hit(lo, inv_ld, blas[bn].lo, blas[bn].hi, best.t) < INF) {
                            more = true;
                            break;
                        }
                    }
                    if (!more) {
                        break;
                    }
                }
            }
        } else {
            let a = n.first;
            let ta = box_hit(o, inv_d, tlas[a].lo, tlas[a].hi, best.t);
            let tb = box_hit(o, inv_d, tlas[a + 1u].lo, tlas[a + 1u].hi, best.t);
            if (ta < INF && tb < INF) {
                if (ta <= tb) {
                    stack[sp] = a + 1u;
                    node = a;
                } else {
                    stack[sp] = a;
                    node = a + 1u;
                }
                sp++;
                next_set = true;
            } else if (ta < INF) {
                node = a;
                next_set = true;
            } else if (tb < INF) {
                node = a + 1u;
                next_set = true;
            }
        }
        if (!next_set) {
            // Children were pushed with the distance at push time; a later,
            // closer hit makes some of them useless, so test again.
            loop {
                if (sp == 0u) {
                    return best;
                }
                sp--;
                node = stack[sp];
                if (box_hit(o, inv_d, tlas[node].lo, tlas[node].hi, best.t) < INF) {
                    break;
                }
            }
        }
    }
    return best;
}

// The root TLAS node is tested before traversal starts.
fn trace_scene(o: vec3<f32>, d: vec3<f32>, tmax: f32, any_hit: bool) -> Hit {
    var none: Hit;
    none.t = tmax;
    none.inst = 0xffffffffu;
    if (g.counts.x == 0u) {
        return none;
    }
    if (box_hit(o, safe_inv(d), tlas[0].lo, tlas[0].hi, tmax) == INF) {
        return none;
    }
    return trace(o, d, tmax, any_hit);
}

// Distance to the ground plane from above, or INF.
fn ground_hit(o: vec3<f32>, d: vec3<f32>) -> f32 {
    if ((g.params.z & 1u) == 0u || d.z >= 0.0 || o.z <= g.ground.w) {
        return INF;
    }
    return (g.ground.w - o.z) / d.z;
}

// Wächter and Binder, "A Fast and Robust Method for Avoiding
// Self-Intersection" (Ray Tracing Gems, 2019): move p off the surface by a
// few units in the last place along n, so the offset scales with |p|.
fn offset_ray(p: vec3<f32>, n: vec3<f32>) -> vec3<f32> {
    let of_i = vec3<i32>(256.0 * n);
    let pi = bitcast<vec3<f32>>(bitcast<vec3<i32>>(p) + select(of_i, -of_i, p < vec3<f32>(0.0)));
    return select(pi, p + n * (1.0 / 65536.0), abs(p) < vec3<f32>(1.0 / 32.0));
}

// ── BSDF ───────────────────────────────────────────────────────────────

fn lum(c: vec3<f32>) -> f32 {
    return dot(c, vec3<f32>(0.2126, 0.7152, 0.0722));
}

// Exact Fresnel reflectance of a dielectric for unpolarised light; `eta`
// is n_incident / n_transmitted, `c` = cos of the incident angle.
fn fresnel_dielectric(c_in: f32, eta: f32) -> f32 {
    let c = clamp(c_in, 0.0, 1.0);
    let s2 = eta * eta * (1.0 - c * c);
    if (s2 >= 1.0) {
        return 1.0;
    }
    let ct = sqrt(1.0 - s2);
    let rs = (eta * c - ct) / (eta * c + ct);
    let rp = (c - eta * ct) / (c + eta * ct);
    return 0.5 * (rs * rs + rp * rp);
}

fn schlick(f0: vec3<f32>, c: f32) -> vec3<f32> {
    let k = pow(1.0 - clamp(c, 0.0, 1.0), 5.0);
    return f0 + (vec3<f32>(1.0) - f0) * k;
}

// Anisotropic GGX in a tangent frame (x along t, y along b, z along n).
fn ggx_d(h: vec3<f32>, ax: f32, ay: f32) -> f32 {
    let x = h.x / ax;
    let y = h.y / ay;
    let k = x * x + y * y + h.z * h.z;
    return 1.0 / (PI * ax * ay * k * k);
}

fn ggx_lambda(w: vec3<f32>, ax: f32, ay: f32) -> f32 {
    let z2 = w.z * w.z;
    if (z2 <= 0.0) {
        return INF;
    }
    let a2 = (ax * ax * w.x * w.x + ay * ay * w.y * w.y) / z2;
    return 0.5 * (sqrt(1.0 + a2) - 1.0);
}

fn ggx_g1(w: vec3<f32>, ax: f32, ay: f32) -> f32 {
    return 1.0 / (1.0 + ggx_lambda(w, ax, ay));
}

fn ggx_g2(v: vec3<f32>, l: vec3<f32>, ax: f32, ay: f32) -> f32 {
    return 1.0 / (1.0 + ggx_lambda(v, ax, ay) + ggx_lambda(l, ax, ay));
}

// Heitz, "Sampling the GGX Distribution of Visible Normals" (JCGT 2018).
fn sample_vndf(v: vec3<f32>, ax: f32, ay: f32, u1: f32, u2: f32) -> vec3<f32> {
    let vh = normalize(vec3<f32>(ax * v.x, ay * v.y, v.z));
    let lensq = vh.x * vh.x + vh.y * vh.y;
    var t1 = vec3<f32>(1.0, 0.0, 0.0);
    if (lensq > 0.0) {
        t1 = vec3<f32>(-vh.y, vh.x, 0.0) * inverseSqrt(lensq);
    }
    let t2 = cross(vh, t1);
    let r = sqrt(u1);
    let phi = 2.0 * PI * u2;
    let p1 = r * cos(phi);
    var p2 = r * sin(phi);
    let s = 0.5 * (1.0 + vh.z);
    p2 = (1.0 - s) * sqrt(1.0 - p1 * p1) + s * p2;
    let nh = t1 * p1 + t2 * p2 + vh * sqrt(max(0.0, 1.0 - p1 * p1 - p2 * p2));
    return normalize(vec3<f32>(ax * nh.x, ay * nh.y, max(0.0, nh.z)));
}

// Solid-angle pdf of reflecting v into l by VNDF sampling.
fn vndf_pdf(v: vec3<f32>, l: vec3<f32>, ax: f32, ay: f32) -> f32 {
    if (v.z <= 0.0 || l.z <= 0.0) {
        return 0.0;
    }
    let h = normalize(v + l);
    return ggx_g1(v, ax, ay) * ggx_d(h, ax, ay) / (4.0 * v.z);
}

fn alpha_of(r: f32) -> f32 {
    return max(r * r, 1e-3);
}

// The opaque lobes at one point, in a tangent frame.
struct Lobes {
    t: vec3<f32>,
    b: vec3<f32>,
    n: vec3<f32>,
    albedo: vec3<f32>,
    metallic: f32,
    ax: f32,
    ay: f32,
    ac: f32,
    eta: f32,
    clearcoat: f32,
    p_coat: f32,
    p_spec: f32,
};

fn to_local(l: Lobes, w: vec3<f32>) -> vec3<f32> {
    return vec3<f32>(dot(w, l.t), dot(w, l.b), dot(w, l.n));
}

fn make_lobes(s: Surface, n: vec3<f32>, wo: vec3<f32>) -> Lobes {
    var l: Lobes;
    l.n = n;
    var t = s.axis - n * dot(s.axis, n);
    if (dot(t, t) < 1e-8) {
        // Duff et al., "Building an Orthonormal Basis, Revisited" (2017).
        let sg = select(-1.0, 1.0, n.z >= 0.0);
        let a = -1.0 / (sg + n.z);
        let bb = n.x * n.y * a;
        t = vec3<f32>(1.0 + sg * n.x * n.x * a, sg * bb, -sg * n.x);
    }
    l.t = normalize(t);
    l.b = cross(n, l.t);
    l.albedo = s.albedo;
    l.metallic = s.metallic;
    l.ax = alpha_of(s.r_along);
    l.ay = alpha_of(s.r_across);
    l.ac = alpha_of(s.coat_roughness);
    l.eta = 1.0 / s.ior;
    l.clearcoat = s.clearcoat;
    let nv = max(dot(n, wo), 1e-4);
    l.p_coat = s.clearcoat * fresnel_dielectric(nv, 1.0 / 1.5);
    let fd = fresnel_dielectric(nv, l.eta);
    let spec = mix(vec3<f32>(fd), schlick(s.albedo, nv), s.metallic);
    let diff = s.albedo * (1.0 - s.metallic) * (1.0 - fd);
    let ws = lum(spec);
    let wd = lum(diff);
    l.p_spec = select(ws / (ws + wd), 0.5, ws + wd <= 0.0);
    return l;
}

// BSDF times cos(theta_l), and the pdf of sampling l, for local v and l.
fn eval_lobes(l: Lobes, v: vec3<f32>, w: vec3<f32>) -> vec4<f32> {
    if (v.z <= 0.0 || w.z <= 0.0) {
        return vec4<f32>(0.0);
    }
    let h = normalize(v + w);
    let vh = max(dot(v, h), 0.0);
    // Base: metal or dielectric specular plus diffuse.
    let fd_h = fresnel_dielectric(vh, l.eta);
    let f_spec = mix(vec3<f32>(fd_h), schlick(l.albedo, vh), l.metallic);
    let spec = f_spec * ggx_d(h, l.ax, l.ay) * ggx_g2(v, w, l.ax, l.ay) / (4.0 * v.z * w.z);
    let diff = l.albedo * (1.0 - l.metallic) * (1.0 - fresnel_dielectric(v.z, l.eta)) / PI;
    let coat_v = l.clearcoat * fresnel_dielectric(v.z, 1.0 / 1.5);
    var f = (spec + diff) * (1.0 - coat_v);
    if (l.clearcoat > 0.0) {
        let fc = l.clearcoat * fresnel_dielectric(vh, 1.0 / 1.5);
        f += vec3<f32>(fc * ggx_d(h, l.ac, l.ac) * ggx_g2(v, w, l.ac, l.ac) / (4.0 * v.z * w.z));
    }
    let pdf = l.p_coat * vndf_pdf(v, w, l.ac, l.ac)
        + (1.0 - l.p_coat) * (l.p_spec * vndf_pdf(v, w, l.ax, l.ay) + (1.0 - l.p_spec) * w.z / PI);
    return vec4<f32>(f * w.z, pdf);
}

// Sample a direction (local); returns it with w = 0 when sampling failed.
fn sample_lobes(l: Lobes, v: vec3<f32>) -> vec4<f32> {
    let u0 = rand();
    let u1 = rand();
    let u2 = rand();
    var w: vec3<f32>;
    if (u0 < l.p_coat) {
        let h = sample_vndf(v, l.ac, l.ac, u1, u2);
        w = reflect(-v, h);
    } else if ((u0 - l.p_coat) < (1.0 - l.p_coat) * l.p_spec) {
        let h = sample_vndf(v, l.ax, l.ay, u1, u2);
        w = reflect(-v, h);
    } else {
        // Cosine-weighted hemisphere.
        let r = sqrt(u1);
        let phi = 2.0 * PI * u2;
        w = vec3<f32>(r * cos(phi), r * sin(phi), sqrt(max(0.0, 1.0 - u1)));
    }
    if (w.z <= 0.0) {
        return vec4<f32>(0.0);
    }
    return vec4<f32>(w, 1.0);
}

fn power_heuristic(a: f32, b: f32) -> f32 {
    let a2 = a * a;
    let b2 = b * b;
    if (a2 + b2 <= 0.0) {
        return 0.0;
    }
    return a2 / (a2 + b2);
}

// A uniform direction in the sun's cone.
fn sample_sun() -> vec3<f32> {
    let c = g.sun_dir.w;
    let z = 1.0 - rand() * (1.0 - c);
    let r = sqrt(max(0.0, 1.0 - z * z));
    let phi = 2.0 * PI * rand();
    let n = g.sun_dir.xyz;
    let sg = select(-1.0, 1.0, n.z >= 0.0);
    let a = -1.0 / (sg + n.z);
    let bb = n.x * n.y * a;
    let t = vec3<f32>(1.0 + sg * n.x * n.x * a, sg * bb, -sg * n.x);
    let b = vec3<f32>(bb, sg + n.y * n.y * a, -n.y);
    return normalize(t * (r * cos(phi)) + b * (r * sin(phi)) + n * z);
}

// ── Paths ──────────────────────────────────────────────────────────────

var<private> radiance: vec3<f32>;

// Add a contribution reached after `bounces` scattering events. Light
// after two or more (indirect light) is clamped in luminance, which is
// where fireflies come from (a rare path to the sun through glass or off
// glossy metal); direct light and what the camera sees are never clamped.
fn add(c: vec3<f32>, bounces: u32) {
    var v = max(c, vec3<f32>(0.0));
    let cl = g.misc.x;
    if (bounces >= 2u && cl > 0.0) {
        let y = lum(v);
        if (y > cl) {
            v = v * (cl / y);
        }
    }
    radiance += v;
}

fn camera_ray(px: vec2<f32>) -> array<vec3<f32>, 2> {
    let w = f32(g.size.x);
    let h = f32(g.size.y);
    let ndc = vec2<f32>(2.0 * px.x / w - 1.0, 1.0 - 2.0 * px.y / h);
    let x = ndc.x * g.cam_right.w;
    let y = ndc.y * g.cam_up.w;
    if (g.cam_origin.w > 0.5) {
        let d = normalize(-g.cam_back.xyz + g.cam_right.xyz * x + g.cam_up.xyz * y);
        return array<vec3<f32>, 2>(g.cam_origin.xyz, d);
    }
    let o = g.cam_origin.xyz + g.cam_right.xyz * x + g.cam_up.xyz * y;
    return array<vec3<f32>, 2>(o, -g.cam_back.xyz);
}

fn trace_path(px: vec2<f32>) {
    let ray = camera_ray(px);
    var o = ray[0];
    var d = ray[1];
    var throughput = vec3<f32>(1.0);
    // Pdf of the BSDF sample that made this ray (0: a delta event or the
    // camera), for weighting a sun it hits against sun sampling.
    var prev_pdf = 0.0;
    let max_bounces = g.params.y;
    for (var bounce = 0u; bounce <= max_bounces; bounce++) {
        let hit = trace_scene(o, d, INF, false);
        let tg = ground_hit(o, d);
        var p: vec3<f32>;
        var n: vec3<f32>;
        var ng: vec3<f32>;
        var s: Surface;
        if (hit.inst == 0xffffffffu && tg == INF) {
            var c = dome(d);
            if (in_sun(d)) {
                var w = 1.0;
                if (prev_pdf > 0.0) {
                    w = power_heuristic(prev_pdf, 1.0 / g.sun_radiance.w);
                }
                c += g.sun_radiance.rgb * w;
            }
            add(throughput * c, bounce);
            return;
        }
        if (tg < hit.t) {
            p = o + d * tg;
            p.z = g.ground.w;
            n = vec3<f32>(0.0, 0.0, 1.0);
            ng = n;
            s.albedo = g.ground.rgb;
            s.metallic = 0.0;
            s.r_along = 1.0;
            s.r_across = 1.0;
            s.transmission = 0.0;
            s.ior = 1.0;
            s.clearcoat = 0.0;
            s.coat_roughness = 1.0;
            s.emission = vec3<f32>(0.0);
            s.axis = vec3<f32>(1.0, 0.0, 0.0);
        } else {
            let inst = instances[hit.inst];
            let tri = tris[hit.tri];
            let m = materials[inst.info.y];
            let w0 = 1.0 - hit.u - hit.v;
            let pm = tri.p0.xyz + tri.e1.xyz * hit.u + tri.e2.xyz * hit.v;
            p = (inst.to_world * vec4<f32>(pm, 1.0)).xyz;
            ng = normalize((inst.to_world * vec4<f32>(cross(tri.e1.xyz, tri.e2.xyz), 0.0)).xyz);
            let i0 = bitcast<u32>(tri.p0.w);
            let i1 = bitcast<u32>(tri.e1.w);
            let i2 = bitcast<u32>(tri.e2.w);
            let nm = normals[i0].xyz * w0 + normals[i1].xyz * hit.u + normals[i2].xyz * hit.v;
            n = normalize((inst.to_world * vec4<f32>(nm, 0.0)).xyz);
            s = surface_at(m, pm, inst.to_world);
            // Seen through a cut: the inside of a closed part. Shade it as
            // the cap on the plane the ray last crossed, flat section colour.
            if (clip_count() > 0u && dot(ng, d) > 0.0) {
                let cap = cap_crossing(o, d, hit.t);
                if (cap.w > 0.0) {
                    p = o + d * cap.w;
                    // The plane's normal points into the removed side,
                    // where the ray came from.
                    ng = cap.xyz;
                    n = ng;
                    s.albedo = m.section.rgb;
                    s.metallic = 0.0;
                    s.r_along = 1.0;
                    s.r_across = 1.0;
                    s.transmission = 0.0;
                    s.ior = 1.0;
                    s.clearcoat = 0.0;
                    s.emission = vec3<f32>(0.0);
                }
            }
        }
        let wo = -d;
        let entering = dot(ng, d) < 0.0;
        // Normals facing the side the ray came from.
        let ngo = select(-ng, ng, entering);
        var ns = select(-n, n, dot(n, wo) >= 0.0);
        if (dot(ns, ngo) <= 0.0) {
            ns = ngo;
        }
        add(throughput * s.emission, bounce);
        if (bounce == max_bounces) {
            return;
        }

        // Glass: Fresnel picks reflection or refraction; a delta event.
        if (rand() < s.transmission * (1.0 - s.metallic)) {
            let eta = select(s.ior, 1.0 / s.ior, entering);
            let a = alpha_of(s.r_across);
            var mn = ns;
            var tl: Lobes;
            if (s.r_across > 0.03) {
                tl.n = ns;
                var tt = vec3<f32>(1.0, 0.0, 0.0);
                if (abs(ns.x) > 0.9) {
                    tt = vec3<f32>(0.0, 1.0, 0.0);
                }
                tl.t = normalize(tt - ns * dot(tt, ns));
                tl.b = cross(ns, tl.t);
                let vl = to_local(tl, wo);
                let hl = sample_vndf(vl, a, a, rand(), rand());
                mn = tl.t * hl.x + tl.b * hl.y + ns * hl.z;
            }
            let ci = dot(wo, mn);
            let fr = fresnel_dielectric(ci, eta);
            var wi: vec3<f32>;
            var through = false;
            if (rand() < fr) {
                wi = reflect(d, mn);
            } else {
                wi = refract(d, mn, eta);
                if (dot(wi, wi) < 1e-12) {
                    wi = reflect(d, mn);
                } else {
                    through = true;
                }
            }
            if (through) {
                if (dot(wi, ngo) >= 0.0) {
                    return;
                }
                if (entering) {
                    throughput *= s.albedo;
                }
                o = offset_ray(p, -ngo);
            } else {
                if (dot(wi, ngo) <= 0.0) {
                    return;
                }
                o = offset_ray(p, ngo);
            }
            if (s.r_across > 0.03) {
                throughput *= ggx_g1(to_local(tl, select(wi, -wi, through)), a, a);
            }
            d = wi;
            prev_pdf = 0.0;
        } else {
            let lobes = make_lobes(s, ns, wo);
            let vl = to_local(lobes, wo);
            let origin = offset_ray(p, ngo);
            // Next-event estimation of the sun, weighted against BSDF
            // sampling by the power heuristic.
            if ((g.params.z & 2u) != 0u) {
                let wl = sample_sun();
                if (dot(wl, ngo) > 0.0 && dot(wl, ns) > 0.0) {
                    let blocked = trace_scene(origin, wl, INF, true).inst != 0xffffffffu
                        || ground_hit(origin, wl) < INF;
                    if (!blocked) {
                        let e = eval_lobes(lobes, vl, to_local(lobes, wl));
                        let pl = 1.0 / g.sun_radiance.w;
                        let wgt = power_heuristic(pl, e.w);
                        add(throughput * e.rgb * g.sun_radiance.rgb * (wgt / pl), bounce + 1u);
                    }
                }
            }
            let smp = sample_lobes(lobes, vl);
            if (smp.w == 0.0) {
                return;
            }
            let e = eval_lobes(lobes, vl, smp.xyz);
            if (e.w <= 0.0) {
                return;
            }
            let wi = lobes.t * smp.x + lobes.b * smp.y + lobes.n * smp.z;
            if (dot(wi, ngo) <= 0.0) {
                return;
            }
            throughput *= e.rgb / e.w;
            prev_pdf = e.w;
            o = origin;
            d = wi;
        }
        // Russian roulette from the fourth bounce on.
        if (bounce >= 3u) {
            let q = min(max(throughput.r, max(throughput.g, throughput.b)), 0.95);
            if (rand() >= q) {
                return;
            }
            throughput /= q;
        }
    }
}

// For a ray that hit the inside of a part at t_hit: where it came into the
// kept region through a clip plane (xyz: that plane's unit normal, w: the
// distance), or w = 0 when it did not cross one before the hit.
fn cap_crossing(o: vec3<f32>, d: vec3<f32>, t_hit: f32) -> vec4<f32> {
    let n = clip_count();
    let all = (g.params.z & 4u) != 0u;
    var best = vec4<f32>(0.0);
    var best_t = select(0.0, INF, all);
    for (var i = 0u; i < n; i++) {
        let c = g.clip[i];
        let dn = dot(c.xyz, d);
        // Crossing from beyond (removed side) to the kept side.
        if (dn >= 0.0) {
            continue;
        }
        let t = -(dot(c.xyz, o) + c.w) / dn;
        if (t <= 0.0 || t >= t_hit) {
            continue;
        }
        // Any: the last such crossing; All: the first.
        if ((!all && t > best_t) || (all && t < best_t)) {
            best_t = t;
            best = vec4<f32>(c.xyz, t);
        }
    }
    return best;
}

@compute @workgroup_size(8, 8, 1)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    if (id.x >= g.size.x || id.y >= g.size.y) {
        return;
    }
    let pixel = id.y * g.size.x + id.x;
    var sum = accum[pixel];
    let seed = pcg(g.params.x);
    for (var k = 0u; k < g.size.w; k++) {
        let sample = g.size.z + k;
        rng = pcg(pixel ^ pcg(sample ^ seed));
        radiance = vec3<f32>(0.0);
        let px = vec2<f32>(f32(id.x) + rand(), f32(id.y) + rand());
        trace_path(px);
        // A sample that went wrong (not finite) counts as black.
        let ok = all(radiance == radiance) && all(abs(radiance) < vec3<f32>(1e30));
        sum += vec4<f32>(select(vec3<f32>(0.0), radiance, ok), 1.0);
    }
    accum[pixel] = sum;
}
