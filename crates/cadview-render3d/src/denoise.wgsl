// The presentation render's denoiser: an edge-avoiding a-trous wavelet
// filter (Dammertz et al., "Edge-Avoiding A-Trous Wavelet Transform for
// fast Global Illumination Filtering", HPG 2010) with a luminance term
// scaled by each pixel's own noise (Schied et al., "Spatiotemporal
// Variance-Guided Filtering", HPG 2017; no temporal part).
//
// It filters irradiance, not colour: the mean radiance divided by the mean
// first-hit albedo, so patterns (wood grain, brushed streaks) and texture
// edges come back exactly when the result is multiplied by the albedo
// again. Its guides are what the tracer records exactly for every sample:
// albedo, normal and position at the first surface that is not a mirror or
// clear glass (through those it follows the reflection or refraction, so
// what is seen in them keeps its edges), averaged over the pixel's samples.
//
// Three entry points, run in order: `prepare` (irradiance and the variance
// of its mean), `atrous` once per scale (step 1, 2, 4, 8, 16 pixels) and
// `finish` (times the albedo). Deterministic: every pixel reads fixed
// neighbours in a fixed order.

struct Params {
    // width, height, step (pixels), -
    size: vec4<u32>,
    // luminance (in standard deviations), normal, albedo, plane distance
    // (in pixel footprints per pixel of offset)
    sigma: vec4<f32>,
};

@group(0) @binding(0) var<uniform> P: Params;
// The tracer's running sums: rgb, w = samples.
@group(0) @binding(1) var<storage, read> accum: array<vec4<f32>>;
// Three per pixel: (albedo, luminance^2), (normal, hits), (position, footprint).
@group(0) @binding(2) var<storage, read> aux: array<vec4<f32>>;
@group(0) @binding(3) var<storage, read> src: array<vec4<f32>>;
@group(0) @binding(4) var<storage, read_write> dst: array<vec4<f32>>;

// Albedo below this is taken as this when dividing (black parts).
const ALBEDO_EPS: f32 = 0.02;

fn lum(c: vec3<f32>) -> f32 {
    return dot(c, vec3<f32>(0.2126, 0.7152, 0.0722));
}

struct Guide {
    albedo: vec3<f32>,
    // Mean normal over all samples (misses count as zero, so a pixel half
    // on a part has a normal half as long).
    normal: vec3<f32>,
    // Mean position and footprint over the samples that hit.
    pos: vec3<f32>,
    footprint: f32,
    hit: bool,
};

fn guide(i: u32) -> Guide {
    var g: Guide;
    let n = max(accum[i].w, 1.0);
    let a0 = aux[3u * i];
    let a1 = aux[3u * i + 1u];
    let a2 = aux[3u * i + 2u];
    g.albedo = a0.rgb / n;
    g.normal = a1.xyz / n;
    let h = max(a1.w, 1.0);
    g.pos = a2.xyz / h;
    g.footprint = a2.w / h;
    g.hit = a1.w > 0.0;
    return g;
}

fn divisor(g: Guide) -> vec3<f32> {
    return max(g.albedo, vec3<f32>(ALBEDO_EPS));
}

fn index(x: u32, y: u32) -> u32 {
    return y * P.size.x + x;
}

@compute @workgroup_size(8, 8, 1)
fn prepare(@builtin(global_invocation_id) id: vec3<u32>) {
    if (id.x >= P.size.x || id.y >= P.size.y) {
        return;
    }
    let i = index(id.x, id.y);
    let n = max(accum[i].w, 1.0);
    let c = accum[i].rgb / n;
    let gd = guide(i);
    let a = divisor(gd);
    // Variance of the mean luminance from the sum of squares, turned into
    // irradiance units.
    let l = lum(c);
    let var_sample = max(aux[3u * i].w / n - l * l, 0.0);
    let la = max(lum(a), ALBEDO_EPS);
    let v = var_sample / n / (la * la);
    dst[i] = vec4<f32>(c / a, v);
}

const KERNEL: array<f32, 3> = array<f32, 3>(0.375, 0.25, 0.0625);

@compute @workgroup_size(8, 8, 1)
fn atrous(@builtin(global_invocation_id) id: vec3<u32>) {
    let w = P.size.x;
    let h = P.size.y;
    if (id.x >= w || id.y >= h) {
        return;
    }
    let step = i32(P.size.z);
    let i = index(id.x, id.y);
    let gp = guide(i);
    let cp = src[i];
    // The standard deviation of the centre, from its variance blurred over
    // 3 x 3 (a single pixel's estimate is itself noisy).
    var vs = 0.0;
    var vw = 0.0;
    for (var dy = -1; dy <= 1; dy++) {
        for (var dx = -1; dx <= 1; dx++) {
            let x = i32(id.x) + dx;
            let y = i32(id.y) + dy;
            if (x < 0 || y < 0 || x >= i32(w) || y >= i32(h)) {
                continue;
            }
            let k = select(0.5, 0.25, dx != 0) * select(0.5, 0.25, dy != 0);
            vs += k * src[index(u32(x), u32(y))].w;
            vw += k;
        }
    }
    let vp = max(vs / vw, 0.0);
    let lp = lum(cp.rgb);
    let nl = length(gp.normal);
    let nhat = select(vec3<f32>(0.0), gp.normal / nl, nl > 1e-6);
    let sl = P.sigma.x;
    let sn2 = P.sigma.y * P.sigma.y;
    let sa2 = P.sigma.z * P.sigma.z;
    let sx = P.sigma.w;
    var sum = vec3<f32>(0.0);
    var vsum = 0.0;
    var wsum = 0.0;
    for (var dy = -2; dy <= 2; dy++) {
        for (var dx = -2; dx <= 2; dx++) {
            let x = i32(id.x) + dx * step;
            let y = i32(id.y) + dy * step;
            if (x < 0 || y < 0 || x >= i32(w) || y >= i32(h)) {
                continue;
            }
            let kw = KERNEL[abs(dx)] * KERNEL[abs(dy)];
            let qi = index(u32(x), u32(y));
            let cq = src[qi];
            var wt = kw;
            if (dx != 0 || dy != 0) {
                let gq = guide(qi);
                let dn = gp.normal - gq.normal;
                let da = gp.albedo - gq.albedo;
                var e = dot(dn, dn) / sn2 + dot(da, da) / sa2;
                if (gp.hit && gq.hit) {
                    // Distance of q from p's tangent plane, in footprints
                    // of p per pixel of offset.
                    let off = f32(step) * length(vec2<f32>(f32(dx), f32(dy)));
                    let d = abs(dot(nhat, gq.pos - gp.pos));
                    e += d / (sx * max(gp.footprint, 1e-12) * off);
                }
                // The noise of both pixels: a lone bright sample (a rare
                // path) has a large variance of its own, so it spreads into
                // its neighbours instead of staying a speck; where both
                // sides are quiet, a step in brightness is an edge.
                e += abs(lp - lum(cq.rgb)) / (sl * sqrt(vp + max(cq.w, 0.0)) + 1e-6);
                wt = kw * exp(-e);
            }
            sum += wt * cq.rgb;
            vsum += wt * wt * cq.w;
            wsum += wt;
        }
    }
    dst[i] = vec4<f32>(sum / wsum, vsum / (wsum * wsum));
}

@compute @workgroup_size(8, 8, 1)
fn finish(@builtin(global_invocation_id) id: vec3<u32>) {
    if (id.x >= P.size.x || id.y >= P.size.y) {
        return;
    }
    let i = index(id.x, id.y);
    dst[i] = vec4<f32>(src[i].rgb * divisor(guide(i)), 1.0);
}
