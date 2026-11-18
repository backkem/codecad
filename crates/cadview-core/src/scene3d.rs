//! Renderer-neutral 3D scene: meshes, materials, lighting, an orbit camera,
//! and the per-frame description every 3D backend draws.
//!
//! This is the 3D twin of [`crate::render`]. Every decision that must come
//! out the same on every backend is made here, once, in f64:
//!
//! - how normals are derived from a bare triangle mesh ([`RenderMesh`]:
//!   flat facets split at a crease angle, smooth across it);
//! - the camera: orbit parameters, named views, fit to a box, the view and
//!   projection matrices, and the near and far planes;
//! - where the key light points in world space, and the shading and tone
//!   curve ([`shade`], [`tone_map`], [`encode_srgb`]) a backend must
//!   reproduce, so a test can compare a backend's pixel with the formula.
//!
//! Backends (`cadview-render3d`) only upload a [`Frame3d`] and rasterize it.
//! Meshes come in as [`TriMesh`] from a kernel's tessellation or from code;
//! nothing here knows what a mesh depicts.
//!
//! # Conventions
//!
//! - World frame: right-handed, millimetres, **Z up** (design 4.4: parts lie
//!   on the XY plane, which is also the 2D drawing plane).
//! - Eye frame: right-handed, the camera looks down its own -Z, +X is screen
//!   right and +Y is screen up (the OpenGL convention).
//! - Clip space: the WebGPU convention. x and y in [-1, 1] with +y up,
//!   depth z in [0, 1] with 0 at the near plane.
//! - Pixels: origin at the top-left corner of the image, +y down, pixel
//!   centres at half-integers.
//! - Matrices ([`Mat4`]) are column-major, `m[column][row]`, the memory
//!   layout of a WGSL `mat4x4<f32>`, and act on column vectors (`M * v`).
//! - Colours in [`Material`] and [`Frame3d`] are linear-light RGB in [0, 1].

use crate::render::FIT_MARGIN;
use crate::solid::{Aabb3, Placement, Point3, TriMesh, Vec3};
use std::collections::HashMap;
use std::sync::Arc;

/// A 4x4 matrix, column-major: `m[column][row]`.
pub type Mat4 = [[f64; 4]; 4];

/// The identity matrix.
pub const IDENTITY4: Mat4 = [
    [1.0, 0.0, 0.0, 0.0],
    [0.0, 1.0, 0.0, 0.0],
    [0.0, 0.0, 1.0, 0.0],
    [0.0, 0.0, 0.0, 1.0],
];

/// `a * b` (apply `b` first).
pub fn mat_mul(a: &Mat4, b: &Mat4) -> Mat4 {
    let mut out = [[0.0; 4]; 4];
    for (c, col) in out.iter_mut().enumerate() {
        for (r, v) in col.iter_mut().enumerate() {
            *v = (0..4).map(|k| a[k][r] * b[c][k]).sum();
        }
    }
    out
}

/// `m * v` for a homogeneous column vector.
pub fn mat_vec(m: &Mat4, v: [f64; 4]) -> [f64; 4] {
    let mut out = [0.0; 4];
    for (r, o) in out.iter_mut().enumerate() {
        *o = (0..4).map(|c| m[c][r] * v[c]).sum();
    }
    out
}

/// The inverse of `m` (Gauss-Jordan with partial pivoting), or `None` when
/// `m` is singular.
pub fn mat_inverse(m: &Mat4) -> Option<Mat4> {
    // Work on rows: a[r][c] = m[c][r], augmented with the identity.
    let mut a = [[0.0f64; 8]; 4];
    for (r, row) in a.iter_mut().enumerate() {
        for c in 0..4 {
            row[c] = m[c][r];
        }
        row[4 + r] = 1.0;
    }
    for col in 0..4 {
        let pivot = (col..4).max_by(|&i, &j| a[i][col].abs().total_cmp(&a[j][col].abs()))?;
        // Also rejects NaN.
        if a[pivot][col].abs().is_nan() || a[pivot][col].abs() <= 1e-300 {
            return None;
        }
        a.swap(col, pivot);
        let p = a[col][col];
        for v in a[col].iter_mut() {
            *v /= p;
        }
        let pivot_row = a[col];
        for (r, row) in a.iter_mut().enumerate() {
            let f = row[col];
            if r != col && f != 0.0 {
                for (v, pv) in row.iter_mut().zip(pivot_row) {
                    *v -= f * pv;
                }
            }
        }
    }
    let mut out = [[0.0; 4]; 4];
    for (r, row) in a.iter().enumerate() {
        for c in 0..4 {
            out[c][r] = row[4 + c];
        }
    }
    Some(out)
}

/// The 4x4 matrix of a rigid [`Placement`]: `p -> R p + t`. The quaternion
/// is normalised first, so a slightly denormalised one still gives a
/// rotation.
pub fn placement_matrix(p: &Placement) -> Mat4 {
    let [w, x, y, z] = p.rotation;
    let n = (w * w + x * x + y * y + z * z).sqrt();
    let (w, x, y, z) = if n > 0.0 && n.is_finite() {
        (w / n, x / n, y / n, z / n)
    } else {
        (1.0, 0.0, 0.0, 0.0)
    };
    let t = p.translation;
    [
        [
            1.0 - 2.0 * (y * y + z * z),
            2.0 * (x * y + w * z),
            2.0 * (x * z - w * y),
            0.0,
        ],
        [
            2.0 * (x * y - w * z),
            1.0 - 2.0 * (x * x + z * z),
            2.0 * (y * z + w * x),
            0.0,
        ],
        [
            2.0 * (x * z + w * y),
            2.0 * (y * z - w * x),
            1.0 - 2.0 * (x * x + y * y),
            0.0,
        ],
        [t.x, t.y, t.z, 1.0],
    ]
}

fn transform_point(m: &Mat4, p: Point3) -> Point3 {
    let v = mat_vec(m, [p.x, p.y, p.z, 1.0]);
    Point3::new(v[0], v[1], v[2])
}

fn transform_dir(m: &Mat4, d: Vec3) -> Vec3 {
    let v = mat_vec(m, [d.x, d.y, d.z, 0.0]);
    Vec3::new(v[0], v[1], v[2])
}

// ── Meshes with derived normals ─────────────────────────────────────────

/// Default crease angle for [`RenderMesh::from_trimesh`], in degrees.
/// Neighbouring facets that meet at a smaller angle are shaded smooth (a
/// tessellated cylinder or fillet), larger angles stay sharp (box edges,
/// 45-degree chamfers).
pub const DEFAULT_CREASE_DEG: f64 = 40.0;

/// A triangle mesh ready to draw: every vertex carries a position and a
/// unit normal, and triangles index vertices (counter-clockwise from
/// outside, as in [`TriMesh`]). Derived data, never synced.
///
/// It also carries the mesh's edges, classified by the same crease angle
/// as the normals: a sharp edge is drawn in every view, a smooth edge only
/// where it is a silhouette (one neighbour faces the viewer, the other
/// does not; decided per frame in [`build_frame`]).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct RenderMesh {
    /// Vertex positions in the mesh's own frame (mm).
    pub positions: Vec<Point3>,
    /// Unit vertex normals, one per position.
    pub normals: Vec<Vec3>,
    pub triangles: Vec<[u32; 3]>,
    /// Unit facet normal of each triangle.
    pub face_normals: Vec<Vec3>,
    /// Edges drawn in every view: creases (neighbouring facets at more
    /// than the crease angle), boundary edges (one facet) and non-manifold
    /// edges (three or more). Vertex indices.
    pub sharp_edges: Vec<[u32; 2]>,
    /// Edges between two facets within the crease angle: silhouette
    /// candidates. Edges inside a flat face are left out (they can never
    /// be a silhouette).
    pub smooth_edges: Vec<SmoothEdge>,
    /// The box of `positions`, or `None` for an empty mesh.
    pub bounds: Option<Aabb3>,
}

/// An edge shared by two facets that meet within the crease angle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SmoothEdge {
    /// Vertex indices of the two ends.
    pub ends: [u32; 2],
    /// The two triangles that share it.
    pub faces: [u32; 2],
}

impl RenderMesh {
    /// Derive normals and edges for `mesh`.
    ///
    /// Each triangle corner gets the angle-weighted average of the facet
    /// normals around its position, counting only facets whose normal is
    /// within `crease_deg` of the corner's own facet. Corners at one
    /// position with the same resulting normal share a vertex; different
    /// normals split it, so creases stay sharp. Degenerate (zero-area)
    /// triangles are dropped. With `crease_deg = 0` every facet is flat.
    /// Edges are classified with the same angle (see [`RenderMesh`]).
    pub fn from_trimesh(mesh: &TriMesh, crease_deg: f64) -> RenderMesh {
        let pos = &mesh.positions;
        let cos_crease = crease_deg.clamp(0.0, 180.0).to_radians().cos();
        // Facet normals; drop degenerate or out-of-range triangles.
        let mut tris: Vec<([u32; 3], Vec3)> = Vec::with_capacity(mesh.triangles.len());
        for t in &mesh.triangles {
            if t.iter().any(|&i| i as usize >= pos.len()) {
                continue;
            }
            let [a, b, c] = t.map(|i| pos[i as usize]);
            if let Some(n) = (b - a).cross(c - a).normalized() {
                tris.push((*t, n));
            }
        }
        // Interior angle of each corner, and the corners around each position.
        let mut incident: Vec<Vec<(usize, f64)>> = vec![Vec::new(); pos.len()];
        for (ti, (t, _)) in tris.iter().enumerate() {
            for k in 0..3 {
                let p = pos[t[k] as usize];
                let u = pos[t[(k + 1) % 3] as usize] - p;
                let v = pos[t[(k + 2) % 3] as usize] - p;
                let angle = match (u.normalized(), v.normalized()) {
                    (Some(u), Some(v)) => u.dot(v).clamp(-1.0, 1.0).acos(),
                    _ => 0.0,
                };
                incident[t[k] as usize].push((ti, angle));
            }
        }
        let mut out = RenderMesh::default();
        let mut index: HashMap<(u32, [i64; 3]), u32> = HashMap::new();
        // One output vertex per input position, for edge ends.
        let mut any_vertex: Vec<u32> = vec![u32::MAX; pos.len()];
        for (t, n_face) in &tris {
            let mut tri = [0u32; 3];
            for k in 0..3 {
                let vi = t[k];
                let mut sum = Vec3::new(0.0, 0.0, 0.0);
                for &(s, angle) in &incident[vi as usize] {
                    let n_s = tris[s].1;
                    if n_s.dot(*n_face) >= cos_crease - 1e-12 {
                        sum = sum + n_s.scale(angle);
                    }
                }
                let n = sum.normalized().unwrap_or(*n_face);
                // Corners whose normals agree to about 1e-9 share a vertex.
                let q = |v: f64| (v * 1e9).round() as i64;
                let key = (vi, [q(n.x), q(n.y), q(n.z)]);
                tri[k] = *index.entry(key).or_insert_with(|| {
                    out.positions.push(pos[vi as usize]);
                    out.normals.push(n);
                    (out.positions.len() - 1) as u32
                });
                any_vertex[vi as usize] = tri[k];
            }
            out.triangles.push(tri);
            out.face_normals.push(*n_face);
        }
        // Edges by input position pair, in first-seen order (deterministic).
        let mut edge_faces: HashMap<(u32, u32), Vec<u32>> = HashMap::new();
        let mut edge_order: Vec<(u32, u32)> = Vec::new();
        for (ti, (t, _)) in tris.iter().enumerate() {
            for k in 0..3 {
                let (a, b) = (t[k], t[(k + 1) % 3]);
                let key = (a.min(b), a.max(b));
                let faces = edge_faces.entry(key).or_default();
                if faces.is_empty() {
                    edge_order.push(key);
                }
                faces.push(ti as u32);
            }
        }
        for key in edge_order {
            let faces = &edge_faces[&key];
            let ends = [any_vertex[key.0 as usize], any_vertex[key.1 as usize]];
            if let [f, g] = faces[..] {
                let d = out.face_normals[f as usize].dot(out.face_normals[g as usize]);
                if d < cos_crease - 1e-12 {
                    out.sharp_edges.push(ends);
                } else if d < 1.0 - 1e-12 {
                    out.smooth_edges.push(SmoothEdge {
                        ends,
                        faces: [f, g],
                    });
                }
            } else {
                out.sharp_edges.push(ends);
            }
        }
        out.bounds = bounds_of(out.positions.iter().copied());
        out
    }
}

fn bounds_of(points: impl Iterator<Item = Point3>) -> Option<Aabb3> {
    let mut b: Option<Aabb3> = None;
    for p in points.filter(|p| p.is_finite()) {
        b = Some(match b {
            None => Aabb3::new(p, p),
            Some(b) => Aabb3::new(
                Point3::new(b.min.x.min(p.x), b.min.y.min(p.y), b.min.z.min(p.z)),
                Point3::new(b.max.x.max(p.x), b.max.y.max(p.y), b.max.z.max(p.z)),
            ),
        });
    }
    b
}

fn aabb_corners(b: &Aabb3) -> [Point3; 8] {
    let (lo, hi) = (b.min, b.max);
    [
        Point3::new(lo.x, lo.y, lo.z),
        Point3::new(hi.x, lo.y, lo.z),
        Point3::new(lo.x, hi.y, lo.z),
        Point3::new(hi.x, hi.y, lo.z),
        Point3::new(lo.x, lo.y, hi.z),
        Point3::new(hi.x, lo.y, hi.z),
        Point3::new(lo.x, hi.y, hi.z),
        Point3::new(hi.x, hi.y, hi.z),
    ]
}

// ── Materials, lighting ────────────────────────────────────────────────

/// sRGB-encoded component (0..=255) to linear light (0..=1), IEC 61966-2-1.
pub fn srgb_to_linear(c: u8) -> f64 {
    let c = c as f64 / 255.0;
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

/// Linear light (clamped to 0..=1) to an sRGB-encoded component (0..=255),
/// IEC 61966-2-1, rounded to nearest.
pub fn encode_srgb(c: f64) -> u8 {
    let c = c.clamp(0.0, 1.0);
    let e = if c <= 0.003_130_8 {
        c * 12.92
    } else {
        1.055 * c.powf(1.0 / 2.4) - 0.055
    };
    (e * 255.0).round().clamp(0.0, 255.0) as u8
}

/// A simple physically based material.
///
/// The first three fields are what every backend draws. The rest are
/// physical parameters for the presentation render (a path tracer); the
/// working render ignores them and shades `base_color`, `metallic` and
/// `roughness` only. None of them names a material: "oak" or "steel" is a
/// script's word, mapped to these numbers by a render option.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Material {
    /// Linear-light base colour (albedo for dielectrics, reflectance for
    /// metals), each in [0, 1].
    pub base_color: [f64; 3],
    /// 0 = dielectric, 1 = metal.
    pub metallic: f64,
    /// Perceptual roughness, 0 = mirror, 1 = fully rough.
    pub roughness: f64,
    /// Share of light that passes through the surface as through a
    /// dielectric (glass), 0..1. Light that enters is tinted by
    /// `base_color` once.
    pub transmission: f64,
    /// Index of refraction of the dielectric: it sets the reflectance at
    /// normal incidence of dielectric surfaces, `((ior - 1) / (ior + 1))^2`
    /// (0.04 at 1.5), and the bending of transmitted light. 1 gives a
    /// surface with no specular reflection at all (pure Lambert).
    pub ior: f64,
    /// Emitted radiance, linear light (0 for none).
    pub emission: [f64; 3],
    /// A clear dielectric coat over the surface (lacquer, varnish, gloss
    /// paint), 0..1; reflectance 0.04 at normal incidence.
    pub clearcoat: f64,
    /// Perceptual roughness of the coat.
    pub clearcoat_roughness: f64,
    /// A procedural surface pattern that needs no texture coordinates.
    pub pattern: Pattern,
}

impl Material {
    /// A dielectric with an sRGB-encoded base colour and roughness 0.5.
    pub fn from_srgb8(r: u8, g: u8, b: u8) -> Material {
        Material {
            base_color: [r, g, b].map(srgb_to_linear),
            metallic: 0.0,
            roughness: 0.5,
            transmission: 0.0,
            ior: DEFAULT_IOR,
            emission: [0.0; 3],
            clearcoat: 0.0,
            clearcoat_roughness: 0.1,
            pattern: Pattern::Flat,
        }
    }

    /// Reflectance at normal incidence of a dielectric with this
    /// material's index of refraction.
    pub fn dielectric_f0(&self) -> f64 {
        let n = self.ior.max(1.0);
        ((n - 1.0) / (n + 1.0)).powi(2)
    }
}

/// Default index of refraction (common glass, lacquer, plastics).
pub const DEFAULT_IOR: f64 = 1.5;

/// A direction a pattern runs along, in a mesh's own frame.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PatternAxis {
    /// The mesh's longest direction: the principal axis of its surface
    /// with the largest spread (the length of a board or a bar, whatever
    /// way it was modelled).
    Longest,
    /// The principal axis with the smallest spread (through the thickness
    /// of a board).
    Shortest,
    /// The principal axis whose spread differs most from the other two,
    /// which are about equal: the axis of a turned part (a disc, a shaft,
    /// a ring).
    Symmetry,
    /// A direction in the mesh's own frame (need not be unit).
    Vector(Vec3),
}

/// A procedural pattern on a surface, evaluated at the surface point in the
/// mesh's own frame, so it moves with the part. Patterns need no texture
/// coordinates. Only the presentation render draws them.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum Pattern {
    /// One colour.
    #[default]
    Flat,
    /// Rings about an axis, with noise ([`Bands`]): on a face that runs
    /// along the axis they show as bands along it (wood grain).
    Bands(Bands),
    /// Fine streaks along an axis and highlights stretched across it
    /// ([`Brushed`]): brushed or ground metal.
    Brushed(Brushed),
}

/// Rings about an axis: see [`Pattern::Bands`] and [`pattern_albedo`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Bands {
    /// The direction the rings run along.
    pub axis: PatternAxis,
    /// Distance between neighbouring rings (mm).
    pub spacing: f64,
    /// Linear colour at the dark edge of each ring.
    pub color: [f64; 3],
    /// How far toward `color` the dark edge goes, 0..1.
    pub strength: f64,
    /// How far the rings wander (mm, amplitude of the noise).
    pub warp: f64,
    /// Distance of the ring axis from the centre of the mesh's box, toward
    /// its thinnest side (mm). 0 puts the axis through the middle.
    pub distance: f64,
}

/// Streaks along an axis: see [`Pattern::Brushed`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Brushed {
    /// The direction of the streaks.
    pub axis: PatternAxis,
    /// 0 = round highlights, 1 = highlights stretched far across the
    /// streaks (roughness along the streaks falls to a fifth).
    pub anisotropy: f64,
    /// Contrast of the visible streaks, 0..1.
    pub streaks: f64,
    /// Width of a streak (mm).
    pub scale: f64,
    /// Streaks run in circles about the axis (a turned or faced part)
    /// instead of along it.
    pub circular: bool,
}

/// Where a pattern sits on one drawn item: its axis and origin in the
/// mesh's own frame, and a seed so copies of one mesh do not look alike.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PatternFrame {
    /// Unit axis, mesh frame.
    pub axis: Vec3,
    /// A point on the axis, mesh frame.
    pub origin: Point3,
    /// Varies the noise per item.
    pub seed: u32,
}

/// The PCG hash (Jarzynski and Olano, "Hash Functions for GPU Rendering",
/// 2020), the integer hash the patterns and the path tracer's random
/// numbers are built on. Backends reproduce it bit for bit.
pub fn pcg_hash(x: u32) -> u32 {
    let state = x.wrapping_mul(747_796_405).wrapping_add(2_891_336_453);
    let word = ((state >> ((state >> 28).wrapping_add(4))) ^ state).wrapping_mul(277_803_737);
    (word >> 22) ^ word
}

fn lattice(ix: i32, iy: i32, iz: i32, seed: u32) -> f64 {
    let h = pcg_hash(ix as u32 ^ pcg_hash(iy as u32 ^ pcg_hash(iz as u32 ^ pcg_hash(seed))));
    h as f64 / 4_294_967_296.0
}

/// Smooth value noise in [0, 1): random values on the integer lattice,
/// blended with the quintic fade `6t^5 - 15t^4 + 10t^3`.
pub fn value_noise(p: [f64; 3], seed: u32) -> f64 {
    let f = p.map(f64::floor);
    let i = f.map(|v| v as i32);
    let t = [p[0] - f[0], p[1] - f[1], p[2] - f[2]];
    let s = t.map(|t| t * t * t * (t * (t * 6.0 - 15.0) + 10.0));
    let lerp = |a: f64, b: f64, t: f64| a + (b - a) * t;
    let c = |dx: i32, dy: i32, dz: i32| lattice(i[0] + dx, i[1] + dy, i[2] + dz, seed);
    let x00 = lerp(c(0, 0, 0), c(1, 0, 0), s[0]);
    let x10 = lerp(c(0, 1, 0), c(1, 1, 0), s[0]);
    let x01 = lerp(c(0, 0, 1), c(1, 0, 1), s[0]);
    let x11 = lerp(c(0, 1, 1), c(1, 1, 1), s[0]);
    lerp(lerp(x00, x10, s[1]), lerp(x01, x11, s[1]), s[2])
}

/// Coordinates of `p` (mesh frame) relative to a pattern frame: distance
/// along the axis, and the offset across it.
fn pattern_coords(pf: &PatternFrame, p: Point3) -> (f64, Vec3) {
    let q = p - pf.origin;
    let along = q.dot(pf.axis);
    (along, q - pf.axis.scale(along))
}

/// The ring weight of [`Bands`] at `p` (mesh frame), in [0, 1]: 0 in the
/// light part of a ring, rising to 1 at its dark edge, then dropping
/// sharply to 0 (light early wood, a thin dark late-wood edge).
pub fn bands_weight(b: &Bands, pf: &PatternFrame, p: Point3) -> f64 {
    let (along, across) = pattern_coords(pf, p);
    let spacing = b.spacing.max(1e-6);
    // Noise in a frame stretched along the axis, so the rings wander
    // slowly along the grain.
    let k = 1.0 / (4.0 * spacing);
    let n = value_noise(
        [across.x * k, across.y * k, across.z * k + along * k * 0.125],
        pf.seed,
    );
    let r = across.length() + b.warp * (2.0 * n - 1.0);
    let f = (r / spacing).rem_euclid(1.0);
    f * f * f
}

/// Fine streak noise of a pattern at `p`, in [0, 1): long thin streaks
/// along the axis, `scale` mm wide; or, `circular`, circles about the axis
/// (noise of the radius and the height along the axis only).
fn streak_noise(pf: &PatternFrame, p: Point3, scale: f64, circular: bool) -> f64 {
    let (along, across) = pattern_coords(pf, p);
    let k = 1.0 / scale.max(1e-6);
    let q = if circular {
        [across.length() * k, along * k, 0.0]
    } else {
        [across.x * k, across.y * k, across.z * k + along * k * 0.02]
    };
    value_noise(q, pf.seed ^ 0x9e37_79b9)
}

/// The direction of the streaks at `p` (mesh frame, unit): the pattern's
/// axis, or for circular brushing the tangent of the circle about it (the
/// axis itself on the axis).
pub fn streak_direction(m: &Material, pf: &PatternFrame, p: Point3) -> Vec3 {
    match m.pattern {
        Pattern::Brushed(b) if b.circular => {
            pf.axis.cross(p - pf.origin).normalized().unwrap_or(pf.axis)
        }
        _ => pf.axis,
    }
}

/// The base colour of material `m` at `p` (mesh frame) after its pattern:
/// what the presentation render uses as albedo there. Backends reproduce
/// it (the noise to float precision).
///
/// - Bands: `mix(base, color, strength w) (1 - 0.12 strength g)` with the
///   ring weight `w` ([`bands_weight`]) and fine pores `g` (streak noise
///   at a fifth of the ring spacing).
/// - Brushed: `base (1 - 0.25 streaks s)` with streak noise `s`.
pub fn pattern_albedo(m: &Material, pf: &PatternFrame, p: Point3) -> [f64; 3] {
    let base = m.base_color.map(|c| c.clamp(0.0, 1.0));
    match m.pattern {
        Pattern::Flat => base,
        Pattern::Bands(b) => {
            let s = b.strength.clamp(0.0, 1.0);
            let w = s * bands_weight(&b, pf, p);
            let g = streak_noise(pf, p, 0.2 * b.spacing, false);
            let dim = 1.0 - 0.12 * s * g;
            std::array::from_fn(|i| (base[i] + (b.color[i].clamp(0.0, 1.0) - base[i]) * w) * dim)
        }
        Pattern::Brushed(br) => {
            let s = streak_noise(pf, p, br.scale, br.circular);
            let dim = 1.0 - 0.25 * br.streaks.clamp(0.0, 1.0) * s;
            base.map(|c| c * dim)
        }
    }
}

/// The roughness of material `m` at `p` after its pattern, as `(along,
/// across)` the streaks ([`streak_direction`]): equal for no pattern and for bands; a
/// brushed surface is smoother along its streaks (by `1 - 0.8
/// anisotropy`) and its roughness varies with the streaks by up to 30 %.
pub fn pattern_roughness(m: &Material, pf: &PatternFrame, p: Point3) -> (f64, f64) {
    let r = m.roughness.clamp(0.0, 1.0);
    match m.pattern {
        Pattern::Brushed(br) => {
            let s = streak_noise(pf, p, br.scale, br.circular);
            let r = (r * (1.0 + 0.6 * br.streaks.clamp(0.0, 1.0) * (s - 0.5))).clamp(0.0, 1.0);
            (r * (1.0 - 0.8 * br.anisotropy.clamp(0.0, 1.0)), r)
        }
        _ => (r, r),
    }
}

/// The principal axes of a mesh's surface ([`principal_axes`]).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PrincipalAxes {
    /// Unit axes, the largest spread first.
    pub axes: [Vec3; 3],
    /// The variance of the surface along each axis (mm^2).
    pub spreads: [f64; 3],
    /// The area-weighted centroid of the surface.
    pub centroid: Point3,
}

impl PrincipalAxes {
    /// The axis whose spread differs most from the other two, which are
    /// closer to each other: the axis of a turned part.
    pub fn symmetry_axis(&self) -> Vec3 {
        let [a, b, c] = self.spreads.map(|s| s.max(1e-300));
        if (a / b).ln() < (b / c).ln() {
            self.axes[2]
        } else {
            self.axes[0]
        }
    }
}

/// Principal axes of a mesh's surface: unit eigenvectors of the
/// area-weighted covariance of its triangles, the largest spread first.
/// Deterministic (Jacobi rotations); a degenerate mesh gives X, Y, Z.
pub fn principal_axes(mesh: &RenderMesh) -> PrincipalAxes {
    let mut area = 0.0;
    let mut mean = [0.0f64; 3];
    let mut tris = Vec::with_capacity(mesh.triangles.len());
    for t in &mesh.triangles {
        let [a, b, c] = t.map(|i| mesh.positions[i as usize]);
        let w = (b - a).cross(c - a).length() * 0.5;
        if !(w.is_finite() && w > 0.0) {
            continue;
        }
        let cen = [
            (a.x + b.x + c.x) / 3.0,
            (a.y + b.y + c.y) / 3.0,
            (a.z + b.z + c.z) / 3.0,
        ];
        for k in 0..3 {
            mean[k] += w * cen[k];
        }
        area += w;
        tris.push((w, [a, b, c]));
    }
    if area <= 0.0 {
        return PrincipalAxes {
            axes: [Vec3::X, Vec3::Y, Vec3::Z],
            spreads: [0.0; 3],
            centroid: Point3::ORIGIN,
        };
    }
    let mean = mean.map(|v| v / area);
    // Covariance of a uniform density over each triangle about `mean`:
    // (sum of outer products of the three corners plus nine times the
    // centroid's) / 12, weighted by area.
    let mut cov = [[0.0f64; 3]; 3];
    for (w, corners) in &tris {
        let d: Vec<[f64; 3]> = corners
            .iter()
            .map(|p| [p.x - mean[0], p.y - mean[1], p.z - mean[2]])
            .collect();
        let g: [f64; 3] = std::array::from_fn(|k| (d[0][k] + d[1][k] + d[2][k]) / 3.0);
        for i in 0..3 {
            for j in 0..3 {
                let s: f64 = d.iter().map(|v| v[i] * v[j]).sum::<f64>() + 9.0 * g[i] * g[j];
                cov[i][j] += w * s / 12.0;
            }
        }
    }
    let (values, vectors) = jacobi_eigen(cov);
    let mut order = [0usize, 1, 2];
    order.sort_by(|&a, &b| values[b].total_cmp(&values[a]));
    let axes = order.map(|k| {
        let v = Vec3::new(vectors[0][k], vectors[1][k], vectors[2][k]);
        // A fixed sign: the largest component positive.
        let big = [v.x, v.y, v.z]
            .into_iter()
            .max_by(|a, b| a.abs().total_cmp(&b.abs()))
            .unwrap_or(1.0);
        let v = if big < 0.0 { -v } else { v };
        v.normalized().unwrap_or(Vec3::X)
    });
    PrincipalAxes {
        axes,
        spreads: order.map(|k| values[k].max(0.0) / area),
        centroid: Point3::new(mean[0], mean[1], mean[2]),
    }
}

/// Eigenvalues and eigenvectors (columns) of a symmetric 3x3 matrix.
fn jacobi_eigen(mut a: [[f64; 3]; 3]) -> ([f64; 3], [[f64; 3]; 3]) {
    let mut v = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
    for _ in 0..50 {
        let off = a[0][1].abs() + a[0][2].abs() + a[1][2].abs();
        let scale = a[0][0].abs() + a[1][1].abs() + a[2][2].abs();
        if off <= 1e-15 * scale.max(1e-300) {
            break;
        }
        for (p, q) in [(0, 1), (0, 2), (1, 2)] {
            if a[p][q].abs() <= 1e-300 {
                continue;
            }
            let theta = (a[q][q] - a[p][p]) / (2.0 * a[p][q]);
            let t = theta.signum() / (theta.abs() + (theta * theta + 1.0).sqrt());
            let t = if theta == 0.0 { 1.0 } else { t };
            let c = 1.0 / (t * t + 1.0).sqrt();
            let s = t * c;
            for row in a.iter_mut() {
                let (akp, akq) = (row[p], row[q]);
                row[p] = c * akp - s * akq;
                row[q] = s * akp + c * akq;
            }
            let (row_p, row_q) = (a[p], a[q]);
            for (k, (apk, aqk)) in row_p.into_iter().zip(row_q).enumerate() {
                a[p][k] = c * apk - s * aqk;
                a[q][k] = s * apk + c * aqk;
            }
            for row in v.iter_mut() {
                let (vp, vq) = (row[p], row[q]);
                row[p] = c * vp - s * vq;
                row[q] = s * vp + c * vq;
            }
        }
    }
    ([a[0][0], a[1][1], a[2][2]], v)
}

/// The pattern frame of an item: its pattern's axis resolved against the
/// mesh, the origin (the centre of the mesh's box, moved `distance` toward
/// the thinnest side for bands), and a seed from the item's place in the
/// scene.
pub fn pattern_frame(m: &Material, mesh: &RenderMesh, index: usize) -> PatternFrame {
    let seed = pcg_hash(index as u32 ^ 0x5bd1_e995);
    let centre = mesh.bounds.map_or(Point3::ORIGIN, |b| box_center(&b));
    let (axis, distance) = match m.pattern {
        Pattern::Flat => {
            return PatternFrame {
                axis: Vec3::Z,
                origin: centre,
                seed,
            }
        }
        Pattern::Bands(b) => (b.axis, b.distance),
        Pattern::Brushed(b) => (b.axis, 0.0),
    };
    let pa = principal_axes(mesh);
    let axes = pa.axes;
    let axis = match axis {
        PatternAxis::Longest => axes[0],
        PatternAxis::Shortest => axes[2],
        PatternAxis::Symmetry => pa.symmetry_axis(),
        PatternAxis::Vector(v) => v.normalized().unwrap_or(axes[0]),
    };
    // Circles about the axis centre on the surface's centroid, which lies
    // on the axis of a turned part.
    let centre = match m.pattern {
        Pattern::Brushed(b) if b.circular => pa.centroid,
        _ => centre,
    };
    // Move off-centre across the axis, toward the thinnest side that is
    // not the axis itself.
    let thin = if axes[2].dot(axis).abs() < 0.9 {
        axes[2]
    } else {
        axes[1]
    };
    let thin = (thin - axis.scale(thin.dot(axis)))
        .normalized()
        .unwrap_or(thin);
    PatternFrame {
        axis,
        origin: centre + thin.scale(distance),
        seed,
    }
}

impl Default for Material {
    /// Light grey, sRGB (200, 200, 200), dielectric, roughness 0.5, opaque,
    /// index of refraction 1.5, no emission, coat or pattern.
    fn default() -> Self {
        Material::from_srgb8(200, 200, 200)
    }
}

/// Linear-light tint every derived section colour leans toward: a muted
/// brick red, the colour drawings traditionally give cut material.
pub const SECTION_TINT: [f64; 3] = [0.55, 0.10, 0.07];

/// The flat colour a cut face of a part with material `m` gets when its
/// item names none (linear light, unlit): `0.3 base + 0.4 SECTION_TINT`.
/// It keeps the part's own hue in it, so neighbouring cut parts stay
/// apart, while the shift toward red keeps a cut face from passing for a
/// lit face of the same part.
pub fn default_section_color(m: &Material) -> [f64; 3] {
    std::array::from_fn(|i| 0.3 * m.base_color[i].clamp(0.0, 1.0) + 0.4 * SECTION_TINT[i])
}

// ── Palette ────────────────────────────────────────────────────────────

/// sRGB-encoded colour to OKLab `[L, a, b]` (Björn Ottosson's matrices).
/// Euclidean distance in OKLab is a perceptual colour difference: about
/// 0.02 is just noticeable.
pub fn oklab_from_srgb(c: [u8; 3]) -> [f64; 3] {
    let [r, g, b] = c.map(srgb_to_linear);
    let l = 0.412_221_470_8 * r + 0.536_332_536_3 * g + 0.051_445_992_9 * b;
    let m = 0.211_903_498_2 * r + 0.680_699_545_1 * g + 0.107_396_956_6 * b;
    let s = 0.088_302_461_9 * r + 0.281_718_837_6 * g + 0.629_978_700_5 * b;
    let (l, m, s) = (l.cbrt(), m.cbrt(), s.cbrt());
    [
        0.210_454_255_3 * l + 0.793_617_785_0 * m - 0.004_072_046_8 * s,
        1.977_998_495_1 * l - 2.428_592_205_0 * m + 0.450_593_709_9 * s,
        0.025_904_037_1 * l + 0.782_771_766_2 * m - 0.808_675_766_0 * s,
    ]
}

/// OKLab `[L, a, b]` to linear-light RGB (may fall outside [0, 1]).
fn linear_from_oklab([l, a, b]: [f64; 3]) -> [f64; 3] {
    let l_ = l + 0.396_337_777_4 * a + 0.215_803_757_3 * b;
    let m_ = l - 0.105_561_345_8 * a - 0.063_854_172_8 * b;
    let s_ = l - 0.089_484_177_5 * a - 1.291_485_548_0 * b;
    let (l, m, s) = (l_ * l_ * l_, m_ * m_ * m_, s_ * s_ * s_);
    [
        4.076_741_662_1 * l - 3.307_711_591_3 * m + 0.230_969_929_2 * s,
        -1.268_438_004_6 * l + 2.609_757_401_1 * m - 0.341_319_396_5 * s,
        -0.004_196_086_3 * l - 0.703_418_614_7 * m + 1.707_614_701_0 * s,
    ]
}

/// Lightness of the palette's three alternating steps (OKLab L).
pub const PALETTE_LIGHTNESS: [f64; 3] = [0.80, 0.70, 0.62];
/// Chroma of every palette colour (OKLab): muted, so parts read as
/// materials rather than as signal colours.
pub const PALETTE_CHROMA: f64 = 0.075;
/// Hue of the first palette colour, degrees (a warm sand).
pub const PALETTE_START_HUE_DEG: f64 = 70.0;

/// The `i`-th palette colour, sRGB-encoded. Hues step by the golden angle
/// (137.5°), so any run of neighbours is spread around the hue circle, and
/// lightness cycles through the three [`PALETTE_LIGHTNESS`] steps, so
/// colours whose hues come close (8 and 13 apart) differ in lightness.
/// Deterministic, no state: part `i` always gets the same colour.
pub fn palette_color(i: usize) -> [u8; 3] {
    const GOLDEN_ANGLE_DEG: f64 = 137.507_764_050_037_85;
    let hue = (PALETTE_START_HUE_DEG + i as f64 * GOLDEN_ANGLE_DEG).to_radians();
    let l = PALETTE_LIGHTNESS[i % 3];
    let lin = linear_from_oklab([l, PALETTE_CHROMA * hue.cos(), PALETTE_CHROMA * hue.sin()]);
    lin.map(encode_srgb)
}

/// The first `n` palette colours (see [`palette_color`]).
pub fn palette(n: usize) -> Vec<[u8; 3]> {
    (0..n).map(palette_color).collect()
}

/// Which frame [`Lighting::key_direction`] is given in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LightFrame {
    /// Eye frame (x right, y up, z toward the viewer): the light turns with
    /// the camera, so every view is lit the same way (a CAD headlight).
    #[default]
    Camera,
    /// World frame (Z up): the light stays put while the camera orbits.
    World,
}

/// Abstract lighting: a directional key light, a weaker directional fill
/// light (diffuse only) and a uniform ambient term: the working render's
/// light. The presentation render is lit by an [`Environment`] instead.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Lighting {
    /// Direction from the scene toward the key light (need not be unit).
    pub key_direction: Vec3,
    /// Direction from the scene toward the fill light (need not be unit).
    pub fill_direction: Vec3,
    /// The frame both directions are given in.
    pub frame: LightFrame,
    /// Key light radiance scale (linear).
    pub key_intensity: f64,
    /// Fill light radiance scale (linear); 0 turns it off.
    pub fill_intensity: f64,
    /// Uniform ambient light (linear), applied to diffuse and specular
    /// reflectance alike.
    pub ambient: f64,
}

impl Default for Lighting {
    /// A headlight from slightly up and left of the eye, and a weak fill
    /// from the right, so the three visible faces of a box in the
    /// isometric view get three distinct shades and no face is left at
    /// ambient only.
    fn default() -> Self {
        Lighting {
            key_direction: Vec3::new(-0.25, 0.35, 1.0),
            fill_direction: Vec3::new(0.7, -0.2, 0.5),
            frame: LightFrame::Camera,
            key_intensity: 0.85,
            fill_intensity: 0.15,
            ambient: 0.2,
        }
    }
}

/// A distant disc light: the sun outdoors, a soft box in a studio. Its
/// size sets how soft the shadows are.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Sun {
    /// Direction from the scene toward the light (need not be unit).
    pub direction: Vec3,
    /// The frame `direction` is given in: [`LightFrame::Camera`] turns the
    /// light with the camera (every view lit alike), [`LightFrame::World`]
    /// keeps it put.
    pub frame: LightFrame,
    /// Angular diameter in degrees, greater than 0: 0.53 is the real sun
    /// (crisp shadows), 10 to 30 a soft box (soft shadows).
    pub angle_deg: f64,
    /// Irradiance on a surface facing the light, linear and relative to
    /// the environment (it is multiplied by [`Environment::intensity`] too).
    pub irradiance: f64,
    /// Linear colour; multiplies the irradiance.
    pub color: [f64; 3],
}

/// What lights a presentation render and what it shows where nothing is
/// hit: a sky dome of three colours and an optional [`Sun`]. All of it is
/// procedural, so there is no image to ship. The working render ignores it.
///
/// The dome's radiance for a unit direction `d` (world, Z up) is
/// `horizon + (zenith - horizon) sqrt(d.z)` above the horizon and `ground`
/// below it ([`Environment::radiance`]), times `intensity`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Environment {
    /// Linear radiance straight up.
    pub zenith: [f64; 3],
    /// Linear radiance at the horizon, seen from above it.
    pub horizon: [f64; 3],
    /// Linear radiance of everything below the horizon (a scene's ground
    /// plane, when it has one, covers it).
    pub ground: [f64; 3],
    pub sun: Option<Sun>,
    /// Scales the dome and the sun together.
    pub intensity: f64,
}

impl Environment {
    /// A neutral photo studio: a grey dome, bright overhead and dark toward
    /// the horizon (so metal has something to reflect), and a soft key
    /// light (16 degrees) from above and to the left of the camera, which
    /// turns with the camera like the working render's headlight. A
    /// horizontal surface in the open receives about `pi` in the isometric
    /// views (so a white floor comes out near 1 before tone mapping).
    pub fn studio() -> Environment {
        Environment {
            zenith: [0.70; 3],
            horizon: [0.20; 3],
            ground: [0.12; 3],
            sun: Some(Sun {
                direction: Vec3::new(-0.7, 0.6, 0.4),
                frame: LightFrame::Camera,
                angle_deg: 16.0,
                irradiance: 1.9,
                color: [1.0, 0.98, 0.95],
            }),
            intensity: 1.0,
        }
    }

    /// Daylight: a blue sky, a pale horizon and the sun, 0.53 degrees, at 50
    /// degrees elevation from the south-west (world frame).
    pub fn sky() -> Environment {
        Environment {
            zenith: [0.08, 0.17, 0.40],
            horizon: [0.45, 0.52, 0.62],
            ground: [0.25, 0.23, 0.21],
            sun: Some(Sun {
                direction: Vec3::new(-0.45, -0.45, 0.766),
                frame: LightFrame::World,
                angle_deg: 0.53,
                irradiance: 3.2,
                color: [1.0, 0.94, 0.86],
            }),
            intensity: 1.0,
        }
    }

    /// The same radiance `l` from every direction, no sun: the furnace.
    pub fn uniform(l: [f64; 3]) -> Environment {
        Environment {
            zenith: l,
            horizon: l,
            ground: l,
            sun: None,
            intensity: 1.0,
        }
    }

    /// The dome's linear radiance toward unit direction `d` (world), the
    /// sun left out, `intensity` included.
    pub fn radiance(&self, d: Vec3) -> [f64; 3] {
        let k = self.intensity.max(0.0);
        if d.z >= 0.0 {
            let t = d.z.min(1.0).sqrt();
            std::array::from_fn(|i| k * (self.horizon[i] + (self.zenith[i] - self.horizon[i]) * t))
        } else {
            self.ground.map(|g| k * g)
        }
    }

    /// The dome's irradiance on a horizontal surface facing up, closed
    /// form: `pi (horizon + 0.8 (zenith - horizon))`, times `intensity`.
    pub fn dome_horizontal_irradiance(&self) -> [f64; 3] {
        let k = self.intensity.max(0.0) * std::f64::consts::PI;
        std::array::from_fn(|i| k * (self.horizon[i] + 0.8 * (self.zenith[i] - self.horizon[i])))
    }
}

impl Default for Environment {
    /// [`Environment::studio`].
    fn default() -> Self {
        Environment::studio()
    }
}

/// The sun as a backend uses it, in world space.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FrameSun {
    /// Unit direction toward the sun's centre, world frame.
    pub direction: Vec3,
    /// Cosine of the sun's angular radius.
    pub cos_radius: f64,
    /// Linear radiance inside the disc: `irradiance color / (pi sin^2
    /// radius)`, so a surface facing it receives exactly `irradiance color`
    /// (times the environment's intensity).
    pub radiance: [f64; 3],
}

impl FrameSun {
    /// Solid angle of the disc (sr).
    pub fn solid_angle(&self) -> f64 {
        std::f64::consts::TAU * (1.0 - self.cos_radius)
    }
}

/// The environment of a frame: the dome colours with the intensity
/// applied, and the sun resolved into world space.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FrameEnvironment {
    /// The scene's environment, intensity applied to the dome colours
    /// (so `intensity` here is 1).
    pub dome: Environment,
    pub sun: Option<FrameSun>,
}

impl FrameEnvironment {
    /// The radiance seen toward unit direction `d`: dome plus sun.
    pub fn radiance(&self, d: Vec3) -> [f64; 3] {
        let mut l = self.dome.radiance(d);
        if let Some(s) = self.sun {
            if d.dot(s.direction) >= s.cos_radius {
                for (li, si) in l.iter_mut().zip(s.radiance) {
                    *li += si;
                }
            }
        }
        l
    }
}

/// Smallest sun the tracer accepts, degrees of angular diameter.
pub const MIN_SUN_ANGLE_DEG: f64 = 0.05;

/// Exposure in stops: the linear colour is multiplied by `2^ev` before the
/// tone curve.
pub fn exposure_scale(ev: f64) -> f64 {
    if ev.is_finite() {
        2f64.powf(ev.clamp(-20.0, 20.0))
    } else {
        1.0
    }
}

/// How mesh edges are drawn over the shading.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EdgeStyle {
    /// sRGB-encoded line colour (drawn as given, not lit or tone mapped).
    pub color: [u8; 3],
    /// Line width in CSS pixels (scaled by the render's pixel ratio).
    pub width_px: f64,
    /// Draw silhouettes against the background as an outline: the part of
    /// each line that lies over the background (or the ground) is drawn
    /// 1 px wider, in a colour that contrasts at least
    /// [`MIN_OUTLINE_CONTRAST`] with the background (see
    /// [`EdgeStyle::outline`]).
    pub outline: bool,
}

impl Default for EdgeStyle {
    /// Near-black lines, 1.5 px wide, with the outline on.
    fn default() -> Self {
        EdgeStyle {
            color: [20, 20, 20],
            width_px: 1.5,
            outline: true,
        }
    }
}

/// Least contrast ratio between a silhouette line and the background
/// (WCAG 2 contrast for graphical objects, success criterion 1.4.11).
pub const MIN_OUTLINE_CONTRAST: f64 = 3.0;

/// The light outline colour for dark backgrounds (sRGB).
pub const LIGHT_OUTLINE: [u8; 3] = [236, 236, 236];

/// WCAG 2 relative luminance of an sRGB colour, in [0, 1].
pub fn relative_luminance(c: [u8; 3]) -> f64 {
    let [r, g, b] = c.map(srgb_to_linear);
    0.2126 * r + 0.7152 * g + 0.0722 * b
}

/// WCAG 2 contrast ratio of two sRGB colours, in [1, 21].
pub fn contrast_ratio(a: [u8; 3], b: [u8; 3]) -> f64 {
    let (la, lb) = (relative_luminance(a), relative_luminance(b));
    (la.max(lb) + 0.05) / (la.min(lb) + 0.05)
}

/// The outline drawn over the background side of the edge lines.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Outline {
    /// sRGB-encoded colour.
    pub color: [u8; 3],
    /// Total line width in CSS pixels; the part over a part's own pixels
    /// is never drawn, so it shows only on the background side.
    pub width_px: f64,
}

impl EdgeStyle {
    /// The outline for `background`, or `None` when `outline` is off. Its
    /// colour is the line colour when that contrasts at least
    /// [`MIN_OUTLINE_CONTRAST`] with the background, otherwise whichever
    /// of the line colour and [`LIGHT_OUTLINE`] contrasts more. It is 1 px
    /// wider than the line (half a pixel on each side), so a silhouette is
    /// a slightly heavier line than an edge inside a part.
    pub fn outline(&self, background: [u8; 3]) -> Option<Outline> {
        if !self.outline {
            return None;
        }
        let ink = contrast_ratio(self.color, background);
        let color =
            if ink >= MIN_OUTLINE_CONTRAST || ink >= contrast_ratio(LIGHT_OUTLINE, background) {
                self.color
            } else {
                LIGHT_OUTLINE
            };
        Some(Outline {
            color,
            width_px: self.width_px + 1.0,
        })
    }
}

/// Blinn-Phong exponent for a perceptual roughness (`alpha = r^2`,
/// `s = 2 / alpha^2 - 2`), clamped to [1, 4096].
pub fn specular_exponent(roughness: f64) -> f64 {
    let a = (roughness.clamp(0.02, 1.0)).powi(2);
    (2.0 / (a * a) - 2.0).clamp(1.0, 4096.0)
}

/// The reference shading every backend reproduces, in linear light, before
/// tone mapping. `n`: unit surface normal facing the viewer; `v`: unit
/// direction toward the viewer; `l`: unit direction toward the key light;
/// `lf`: unit direction toward the fill light.
///
/// `kd = base (1 - metallic)`, `f0 = mix(0.04, base, metallic)`;
/// `ambient (kd + f0) + fill kd max(n.lf, 0)` plus, when `n.l > 0`,
/// `key (kd + F(v.h) (s + 8) / 8 (n.h)^s) n.l` with Schlick's Fresnel
/// `F(x) = f0 + (1 - f0)(1 - x)^5` and `s` from [`specular_exponent`].
pub fn shade(n: Vec3, v: Vec3, l: Vec3, lf: Vec3, m: &Material, light: &Lighting) -> [f64; 3] {
    let metallic = m.metallic.clamp(0.0, 1.0);
    let n_l = n.dot(l);
    let n_lf = n.dot(lf).max(0.0);
    let s = specular_exponent(m.roughness);
    let h = (v + l).normalized().unwrap_or(n);
    let n_h = n.dot(h).max(0.0);
    let v_h = v.dot(h).clamp(0.0, 1.0);
    let mut out = [0.0; 3];
    for (i, o) in out.iter_mut().enumerate() {
        let base = m.base_color[i].clamp(0.0, 1.0);
        let kd = base * (1.0 - metallic);
        let f0 = 0.04 + (base - 0.04) * metallic;
        let mut c = light.ambient * (kd + f0) + light.fill_intensity * kd * n_lf;
        if n_l > 0.0 {
            let fresnel = f0 + (1.0 - f0) * (1.0 - v_h).powi(5);
            let spec = fresnel * (s + 8.0) / 8.0 * n_h.powf(s);
            c += light.key_intensity * (kd + spec) * n_l;
        }
        *o = c;
    }
    out
}

/// The tone curve: Khronos PBR Neutral (linear in, linear display out, in
/// [0, 1]). Colours below about 0.76 pass through almost unchanged (a small
/// toe offset near black), brighter ones roll off to white. Follow with
/// [`encode_srgb`].
pub fn tone_map(c: [f64; 3]) -> [f64; 3] {
    const START: f64 = 0.8 - 0.04;
    const DESAT: f64 = 0.15;
    let mut c = c.map(|v| v.max(0.0));
    let x = c[0].min(c[1]).min(c[2]);
    let offset = if x < 0.08 { x - 6.25 * x * x } else { 0.04 };
    c = c.map(|v| v - offset);
    let peak = c[0].max(c[1]).max(c[2]);
    if peak < START {
        return c;
    }
    let d = 1.0 - START;
    let new_peak = 1.0 - d * d / (peak + d - START);
    c = c.map(|v| v * new_peak / peak);
    let g = 1.0 - 1.0 / (DESAT * (peak - new_peak) + 1.0);
    c.map(|v| v * (1.0 - g) + new_peak * g)
}

// ── Camera ─────────────────────────────────────────────────────────────

/// Orthographic or perspective projection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Projection {
    #[default]
    Orthographic,
    Perspective,
}

/// A standard view direction. The eye sits on the named side of the
/// target; the screen axes are stated per view.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NamedView {
    /// Eye at -Y looking +Y; X right, Z up.
    Front,
    /// Eye at +Y looking -Y; -X right, Z up.
    Back,
    /// Eye at -X looking +X; -Y right, Z up.
    Left,
    /// Eye at +X looking -X; +Y right, Z up.
    Right,
    /// Eye at +Z looking down; X right, Y up.
    Top,
    /// Eye at -Z looking up; X right, -Y up.
    Bottom,
    /// Eye along (1, -1, 1): front, right and top faces visible, Z up on
    /// screen (the "south-east" isometric).
    Isometric,
    /// Eye along (-1, -1, 1): front, left and top (south-west).
    IsometricSw,
    /// Eye along (1, 1, 1): back, right and top (north-east).
    IsometricNe,
    /// Eye along (-1, 1, 1): back, left and top (north-west).
    IsometricNw,
}

impl NamedView {
    pub const ALL: [NamedView; 10] = [
        NamedView::Front,
        NamedView::Back,
        NamedView::Left,
        NamedView::Right,
        NamedView::Top,
        NamedView::Bottom,
        NamedView::Isometric,
        NamedView::IsometricSw,
        NamedView::IsometricNe,
        NamedView::IsometricNw,
    ];

    /// `(yaw, pitch)` in radians for [`Camera3d`].
    pub fn yaw_pitch(self) -> (f64, f64) {
        use std::f64::consts::{FRAC_PI_2, FRAC_PI_4, PI};
        let iso = (1.0 / 3f64.sqrt()).asin();
        match self {
            NamedView::Front => (-FRAC_PI_2, 0.0),
            NamedView::Back => (FRAC_PI_2, 0.0),
            NamedView::Left => (PI, 0.0),
            NamedView::Right => (0.0, 0.0),
            NamedView::Top => (-FRAC_PI_2, FRAC_PI_2),
            NamedView::Bottom => (-FRAC_PI_2, -FRAC_PI_2),
            NamedView::Isometric => (-FRAC_PI_4, iso),
            NamedView::IsometricSw => (-3.0 * FRAC_PI_4, iso),
            NamedView::IsometricNe => (FRAC_PI_4, iso),
            NamedView::IsometricNw => (3.0 * FRAC_PI_4, iso),
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            NamedView::Front => "front",
            NamedView::Back => "back",
            NamedView::Left => "left",
            NamedView::Right => "right",
            NamedView::Top => "top",
            NamedView::Bottom => "bottom",
            NamedView::Isometric => "isometric",
            NamedView::IsometricSw => "iso-sw",
            NamedView::IsometricNe => "iso-ne",
            NamedView::IsometricNw => "iso-nw",
        }
    }
}

impl std::str::FromStr for NamedView {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, String> {
        NamedView::ALL
            .into_iter()
            .find(|v| v.name() == s)
            .ok_or_else(|| {
                format!(
                    "unknown view '{s}' (front|back|left|right|top|bottom|isometric|iso-sw|iso-ne|iso-nw)"
                )
            })
    }
}

/// Default vertical field of view, degrees.
pub const DEFAULT_FOV_Y_DEG: f64 = 30.0;

/// An orbit camera around `target`.
///
/// The eye sits at `target + distance * d` with
/// `d = (cos pitch cos yaw, cos pitch sin yaw, sin pitch)`: yaw turns about
/// world Z from +X, pitch lifts toward +Z (both radians). Screen right is
/// `(-sin yaw, cos yaw, 0)`, which stays defined at pitch = +-90 degrees,
/// and screen up is `right x forward`.
///
/// Both projections show the same size at the target's depth: the
/// orthographic view is `2 distance tan(fov_y / 2)` high, the perspective
/// view has vertical field of view `fov_y`. Switching projection keeps the
/// framing.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Camera3d {
    pub target: Point3,
    pub yaw: f64,
    pub pitch: f64,
    pub distance: f64,
    pub projection: Projection,
    /// Vertical field of view in radians, in (0, pi).
    pub fov_y: f64,
}

impl Default for Camera3d {
    fn default() -> Self {
        Camera3d::new(NamedView::Isometric, Projection::Orthographic)
    }
}

impl Camera3d {
    /// A camera looking at the origin from `view`, distance 1000 mm.
    pub fn new(view: NamedView, projection: Projection) -> Self {
        let (yaw, pitch) = view.yaw_pitch();
        Camera3d {
            target: Point3::ORIGIN,
            yaw,
            pitch,
            distance: 1000.0,
            projection,
            fov_y: DEFAULT_FOV_Y_DEG.to_radians(),
        }
    }

    /// Turn to `view`, keeping target, distance and projection.
    pub fn set_view(&mut self, view: NamedView) {
        (self.yaw, self.pitch) = view.yaw_pitch();
    }

    /// Unit vector from the target toward the eye (the eye frame's +Z).
    pub fn back(&self) -> Vec3 {
        let (sy, cy) = self.yaw.sin_cos();
        let (sp, cp) = self.pitch.sin_cos();
        Vec3::new(cp * cy, cp * sy, sp)
    }

    /// Unit screen-right vector (the eye frame's +X).
    pub fn right(&self) -> Vec3 {
        let (sy, cy) = self.yaw.sin_cos();
        Vec3::new(-sy, cy, 0.0)
    }

    /// Unit screen-up vector (the eye frame's +Y).
    pub fn up(&self) -> Vec3 {
        self.right().cross(-self.back())
    }

    pub fn eye(&self) -> Point3 {
        self.target + self.back().scale(self.distance)
    }

    /// Half the visible height at the target's depth.
    pub fn half_height(&self) -> f64 {
        self.distance * (self.fov_y * 0.5).tan()
    }

    /// World to eye frame.
    pub fn view_matrix(&self) -> Mat4 {
        let (r, u, b) = (self.right(), self.up(), self.back());
        let e = self.eye();
        let e = Vec3::new(e.x, e.y, e.z);
        [
            [r.x, u.x, b.x, 0.0],
            [r.y, u.y, b.y, 0.0],
            [r.z, u.z, b.z, 0.0],
            [-r.dot(e), -u.dot(e), -b.dot(e), 1.0],
        ]
    }

    /// Eye frame to clip space for an image of `aspect` = width / height,
    /// with depth 0 at eye distance `near` and 1 at `far`. Orthographic
    /// allows `near <= 0`; perspective needs `0 < near < far`.
    pub fn projection_matrix(&self, aspect: f64, near: f64, far: f64) -> Mat4 {
        let depth = far - near;
        match self.projection {
            Projection::Orthographic => {
                let h = self.half_height();
                let w = h * aspect;
                [
                    [1.0 / w, 0.0, 0.0, 0.0],
                    [0.0, 1.0 / h, 0.0, 0.0],
                    [0.0, 0.0, -1.0 / depth, 0.0],
                    [0.0, 0.0, -near / depth, 1.0],
                ]
            }
            Projection::Perspective => {
                let f = 1.0 / (self.fov_y * 0.5).tan();
                [
                    [f / aspect, 0.0, 0.0, 0.0],
                    [0.0, f, 0.0, 0.0],
                    [0.0, 0.0, far / (near - far), -1.0],
                    [0.0, 0.0, near * far / (near - far), 0.0],
                ]
            }
        }
    }

    /// Near and far eye distances that enclose `bounds` (its bounding
    /// sphere, padded by 1 %), or a wide default around the target when
    /// there is nothing to draw. Perspective near stays positive.
    pub fn clip_range(&self, bounds: Option<&Aabb3>) -> (f64, f64) {
        let Some(b) = bounds else {
            let d = self.distance.max(1e-6);
            return (d * 0.01, d * 100.0);
        };
        let c = Point3::new(
            (b.min.x + b.max.x) * 0.5,
            (b.min.y + b.max.y) * 0.5,
            (b.min.z + b.max.z) * 0.5,
        );
        let radius = (b.max - b.min).length() * 0.5 * 1.01 + 1e-9;
        let depth = (self.eye() - c).dot(self.back());
        let (near, far) = (depth - radius, depth + radius);
        match self.projection {
            Projection::Orthographic => (near, far),
            Projection::Perspective => (near.max(far * 1e-4), far),
        }
    }

    /// Re-aim at the centre of `bounds` and set the distance so the whole
    /// box fits an image of `aspect` with `margin` (1.1 = the box spans at
    /// most 1 / 1.1 of the image in each direction), keeping yaw, pitch,
    /// projection and field of view. Returns `false` (camera unchanged) for
    /// a non-finite box or margin.
    ///
    /// Exact for both projections: every corner ends inside the shrunk
    /// frame, and at least one corner touches it.
    pub fn fit(&mut self, bounds: &Aabb3, aspect: f64, margin: f64) -> bool {
        if !(bounds.min.is_finite() && bounds.max.is_finite())
            || !(aspect.is_finite() && aspect > 0.0 && margin.is_finite() && margin > 0.0)
        {
            return false;
        }
        let c = Point3::new(
            (bounds.min.x + bounds.max.x) * 0.5,
            (bounds.min.y + bounds.max.y) * 0.5,
            (bounds.min.z + bounds.max.z) * 0.5,
        );
        let (r, u, b) = (self.right(), self.up(), self.back());
        let tan_v = (self.fov_y * 0.5).tan();
        let tan_h = tan_v * aspect;
        let mut distance: f64 = 0.0;
        for p in aabb_corners(bounds) {
            let d = p - c;
            let (x, y, z) = (d.dot(r).abs(), d.dot(u).abs(), d.dot(b));
            let need = match self.projection {
                // The half-height needed, turned into a distance.
                Projection::Orthographic => (y * margin).max(x * margin / aspect) / tan_v,
                // The eye must be far enough that the corner at depth
                // (distance - z) falls inside the shrunk frustum.
                Projection::Perspective => z + (y * margin / tan_v).max(x * margin / tan_h),
            };
            distance = distance.max(need);
        }
        if !(distance.is_finite()) {
            return false;
        }
        if distance <= 0.0 {
            // A point: keep a usable distance.
            distance = self.distance.max(1.0);
        }
        self.target = c;
        self.distance = distance;
        true
    }

    /// `p` in pixels (top-left origin) and depth in [0, 1], for an image of
    /// `width` x `height` with clip range `(near, far)`. `None` behind a
    /// perspective eye.
    pub fn project(
        &self,
        p: Point3,
        width: u32,
        height: u32,
        (near, far): (f64, f64),
    ) -> Option<(f64, f64, f64)> {
        let (w, h) = (width as f64, height as f64);
        let vp = mat_mul(
            &self.projection_matrix(w / h, near, far),
            &self.view_matrix(),
        );
        let c = mat_vec(&vp, [p.x, p.y, p.z, 1.0]);
        if c[3] <= 0.0 {
            return None;
        }
        let (x, y, z) = (c[0] / c[3], c[1] / c[3], c[2] / c[3]);
        Some(((x + 1.0) * 0.5 * w, (1.0 - y) * 0.5 * h, z))
    }
}

// ── Scene and frame ────────────────────────────────────────────────────

/// One drawable: a mesh placed rigidly in the world with a material.
#[derive(Debug, Clone, PartialEq)]
pub struct SceneItem {
    /// Shared, so many items can draw one mesh and the scene clones cheaply.
    /// Backends may key caches of prepared geometry on this allocation, so
    /// items that repeat a part should clone the `Arc`, not the mesh.
    pub mesh: Arc<RenderMesh>,
    pub placement: Placement,
    pub material: Material,
    /// sRGB colour of this part's cut faces in a cutaway view; `None` for
    /// [`default_section_color`].
    pub section: Option<[u8; 3]>,
}

/// Default background of a 3D render: a light neutral grey (sRGB). Edges,
/// shadows and the ground's contact shadow all read against it.
pub const LIGHT_BACKGROUND: [u8; 3] = [236, 237, 240];

/// The 2D viewer's dark background (sRGB), for 3D views that sit next to
/// it.
pub const VIEWER_BACKGROUND: [u8; 3] = {
    let c = crate::render::DEFAULT_BACKGROUND;
    [c.r, c.g, c.b]
};

/// Background of a 3D view.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Background {
    /// One colour, sRGB-encoded (drawn as given, not tone mapped).
    Solid([u8; 3]),
}

impl Background {
    /// The 2D viewer's dark background ([`VIEWER_BACKGROUND`]).
    pub const VIEWER: Background = Background::Solid(VIEWER_BACKGROUND);

    /// The colour, sRGB-encoded.
    pub fn srgb(&self) -> [u8; 3] {
        let Background::Solid(c) = *self;
        c
    }
}

impl Default for Background {
    /// [`LIGHT_BACKGROUND`].
    fn default() -> Self {
        Background::Solid(LIGHT_BACKGROUND)
    }
}

/// A ground plane under the scene. It is not part of the fit or the
/// scene's bounds and is never cut by clip planes. It receives the
/// ambient-occlusion darkening (the soft contact shadow) but no other
/// lighting.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Ground {
    /// World Z of the plane; `None` for the scene's lowest point.
    pub height: Option<f64>,
    /// sRGB colour; `None` for the background colour, which makes the
    /// ground a shadow catcher: only the contact shadow shows. A colour
    /// fades into the background with distance from the scene.
    pub color: Option<[u8; 3]>,
}

/// Size of the drawn ground square: half its side is this many times the
/// scene's horizontal half-diagonal (plus the scene height, for tall thin
/// scenes).
pub const GROUND_EXTENT: f64 = 6.0;
/// A coloured ground fades into the background between these multiples of
/// the scene's horizontal half-diagonal from its centre.
pub const GROUND_FADE: (f64, f64) = (1.4, 3.0);

/// Ambient occlusion: surfaces that see less of the open sky around them
/// get darker. It is what makes holes, pockets and gaps between parts read
/// darker than the faces around them, and what puts a soft contact shadow
/// on the ground. Screen-space: every backend estimates, for each pixel,
/// the share of [`ao_kernel`] sample points (in the hemisphere around the
/// surface normal) that lie behind a nearer surface, rotated per pixel by
/// [`ao_rotation`] and averaged over each 4 x 4 block of pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Occlusion {
    /// Sampling radius in mm; `None` for [`AO_RADIUS_FRACTION`] of the
    /// scene's diagonal.
    pub radius: Option<f64>,
    /// How dark a fully occluded point gets: the final colour is
    /// multiplied by `1 - strength * occlusion` (see [`apply_occlusion`]).
    pub strength: f64,
}

/// Default ambient-occlusion radius as a share of the scene's diagonal.
pub const AO_RADIUS_FRACTION: f64 = 0.15;
/// Number of sample points in [`ao_kernel`].
pub const AO_SAMPLES: usize = 16;

impl Default for Occlusion {
    fn default() -> Self {
        Occlusion {
            radius: None,
            strength: 1.0,
        }
    }
}

/// How occlusion darkens a final pixel: `color` is the tone-mapped linear
/// colour, `occlusion` the estimated share of the hemisphere that is
/// blocked, in [0, 1].
pub fn apply_occlusion(color: [f64; 3], occlusion: f64, strength: f64) -> [f64; 3] {
    let v = (1.0 - strength.clamp(0.0, 1.0) * occlusion.clamp(0.0, 1.0)).clamp(0.0, 1.0);
    color.map(|c| c * v)
}

/// The radical inverse of `i` in base 2 (van der Corput).
fn radical_inverse(mut i: u32) -> f64 {
    i = i.reverse_bits();
    i as f64 / 4_294_967_296.0
}

/// The ambient-occlusion sample points, in a tangent frame with +Z along
/// the surface normal, inside the unit hemisphere: a Hammersley set mapped
/// to cosine-weighted directions, kept at least 10° above the surface, at
/// lengths spread evenly from 0.25 to 1 (so pockets about as deep as the
/// radius read dark, not only their corners).
pub fn ao_kernel() -> [Vec3; AO_SAMPLES] {
    let n = AO_SAMPLES;
    let min_z = 10f64.to_radians().sin();
    std::array::from_fn(|i| {
        let u1 = (i as f64 + 0.5) / n as f64;
        let u2 = radical_inverse(i as u32);
        let phi = std::f64::consts::TAU * u2;
        let z = (1.0 - u1).sqrt().max(min_z);
        let r = (1.0 - z * z).max(0.0).sqrt();
        let t = i as f64 / (n - 1) as f64;
        let len = 0.25 + 0.75 * t;
        Vec3::new(r * phi.cos(), r * phi.sin(), z).scale(len)
    })
}

/// The 4 x 4 ordered-dither (Bayer) matrix.
const BAYER4: [[u8; 4]; 4] = [[0, 8, 2, 10], [12, 4, 14, 6], [3, 11, 1, 9], [15, 7, 13, 5]];

/// The kernel's rotation about the normal at pixel `(x, y)`, as
/// `(cos, sin)`: one of 16 angles in steps of 22.5°, laid out as a 4 x 4
/// Bayer pattern, so every aligned 4 x 4 block holds each angle once and
/// the backend's 4 x 4 average sees all 256 sample directions.
pub fn ao_rotation(x: u32, y: u32) -> (f64, f64) {
    let k = BAYER4[(y % 4) as usize][(x % 4) as usize] as f64;
    let a = std::f64::consts::TAU * k / 16.0;
    (a.cos(), a.sin())
}

/// A clip plane for cutaway views: it removes the half-space its normal
/// points into. Parts are cut (the ground and nothing else is not); the cut
/// faces of a closed part are filled with its section colour, so it does
/// not look hollow.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ClipPlane {
    /// A point on the plane.
    pub point: Point3,
    /// Points into the removed side (need not be unit).
    pub normal: Vec3,
}

/// Most clip planes a scene may carry.
pub const MAX_CLIP_PLANES: usize = 4;

/// How a scene's clip planes combine.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ClipMode {
    /// Remove what lies beyond any plane: one plane takes off a half, two
    /// keep a quarter.
    #[default]
    Any,
    /// Remove only what lies beyond every plane: two planes take out a
    /// wedge, the classic quarter cutaway.
    All,
}

/// Everything a 3D view shows. Several parts are several items, each with
/// its own placement (an exploded view is a change of placements, see
/// [`Scene3d::explode`]) and material.
#[derive(Debug, Clone, PartialEq)]
pub struct Scene3d {
    pub items: Vec<SceneItem>,
    pub background: Background,
    pub lighting: Lighting,
    pub camera: Camera3d,
    /// Sharp edges and silhouettes drawn over the shading; `None` for
    /// shading only.
    pub edges: Option<EdgeStyle>,
    /// Ambient occlusion (depth cues and the ground's contact shadow);
    /// `None` for none.
    pub occlusion: Option<Occlusion>,
    /// A ground plane; `None` for none.
    pub ground: Option<Ground>,
    /// Cutaway: geometry on the far side of these is removed (at most
    /// [`MAX_CLIP_PLANES`]; [`build_frame`] keeps the first ones), on the
    /// far side of any of them or of all of them as `clip_mode` says.
    pub clip_planes: Vec<ClipPlane>,
    pub clip_mode: ClipMode,
    /// What lights the presentation render (the working render uses
    /// `lighting` instead).
    pub environment: Environment,
    /// Exposure of the presentation render in stops ([`exposure_scale`]);
    /// the working render ignores it.
    pub exposure: f64,
}

impl Default for Scene3d {
    /// Light background, default lighting and edges, ambient occlusion on,
    /// no ground, no clip planes, the studio environment at exposure 0.
    fn default() -> Self {
        Scene3d {
            items: Vec::new(),
            background: Background::default(),
            lighting: Lighting::default(),
            camera: Camera3d::default(),
            edges: Some(EdgeStyle::default()),
            occlusion: Some(Occlusion::default()),
            ground: None,
            clip_planes: Vec::new(),
            clip_mode: ClipMode::Any,
            environment: Environment::default(),
            exposure: 0.0,
        }
    }
}

impl Scene3d {
    /// Add `mesh` at `placement` with `material`, deriving normals at
    /// [`DEFAULT_CREASE_DEG`].
    pub fn add_mesh(&mut self, mesh: &TriMesh, placement: Placement, material: Material) {
        self.add_shared(
            Arc::new(RenderMesh::from_trimesh(mesh, DEFAULT_CREASE_DEG)),
            placement,
            material,
        );
    }

    /// Add one more placement of an already prepared mesh. Repeated parts
    /// share the mesh, and backends prepare it once.
    pub fn add_shared(&mut self, mesh: Arc<RenderMesh>, placement: Placement, material: Material) {
        self.items.push(SceneItem {
            mesh,
            placement,
            material,
            section: None,
        });
    }

    /// The world box of every placed vertex, or `None` for an empty scene.
    pub fn bounds(&self) -> Option<Aabb3> {
        bounds_of(self.items.iter().flat_map(|item| {
            let m = placement_matrix(&item.placement);
            item.mesh
                .positions
                .iter()
                .map(move |&p| transform_point(&m, p))
        }))
    }

    /// The world box of item `i`, or `None` for an empty mesh or a wrong
    /// index.
    pub fn item_bounds(&self, i: usize) -> Option<Aabb3> {
        let item = self.items.get(i)?;
        let m = placement_matrix(&item.placement);
        bounds_of(item.mesh.positions.iter().map(|&p| transform_point(&m, p)))
    }

    /// Fit the camera to the whole scene with the 2D renderer's margin
    /// ([`FIT_MARGIN`]). With clip planes that remove what lies beyond any
    /// of them, it fits the part of the scene's box that the planes keep
    /// (the whole box if they keep nothing); planes that only remove a
    /// wedge ([`ClipMode::All`]) keep the whole box in view.
    /// Returns `false` for an empty scene.
    pub fn fit_camera(&mut self, aspect: f64) -> bool {
        let Some(all) = self.bounds() else {
            return false;
        };
        let kept = match self.clip_mode {
            ClipMode::Any => clip_planes_of(&self.clip_planes)
                .iter()
                .try_fold(all, |b, p| clip_box(&b, p))
                .unwrap_or(all),
            ClipMode::All => all,
        };
        self.camera.fit(&kept, aspect, FIT_MARGIN)
    }

    /// Exploded view: move every item away from the assembly's centre (see
    /// [`explode_offsets`]). Fit the camera afterwards.
    pub fn explode(&mut self, factor: f64, axes: [bool; 3]) {
        let boxes: Vec<Option<Aabb3>> =
            (0..self.items.len()).map(|i| self.item_bounds(i)).collect();
        for (item, d) in self
            .items
            .iter_mut()
            .zip(explode_offsets(&boxes, factor, axes))
        {
            item.placement.translation = item.placement.translation + d;
        }
    }
}

/// Clip planes as `[nx, ny, nz, w]` with unit normals (see
/// [`Frame3d::clip_planes`]); degenerate ones are dropped, at most
/// [`MAX_CLIP_PLANES`] kept.
fn clip_planes_of(planes: &[ClipPlane]) -> Vec<[f64; 4]> {
    planes
        .iter()
        .filter_map(|p| {
            let n = p.normal.normalized()?;
            p.point.is_finite().then(|| {
                [
                    n.x,
                    n.y,
                    n.z,
                    -n.dot(Vec3::new(p.point.x, p.point.y, p.point.z)),
                ]
            })
        })
        .take(MAX_CLIP_PLANES)
        .collect()
}

/// The box of the part of `b` a clip plane keeps (`n . p + w <= 0`), or
/// `None` when it keeps nothing. Exact for one plane: the corners on the
/// kept side plus the points where the box's edges cross the plane.
fn clip_box(b: &Aabb3, [nx, ny, nz, w]: &[f64; 4]) -> Option<Aabb3> {
    let corners = aabb_corners(b);
    let f = |p: Point3| nx * p.x + ny * p.y + nz * p.z + w;
    let mut points: Vec<Point3> = corners.iter().copied().filter(|&p| f(p) <= 0.0).collect();
    // Edges join corners whose index differs in one bit (x = 1, y = 2, z = 4).
    for i in 0..8usize {
        for bit in [1usize, 2, 4] {
            let j = i | bit;
            if j == i {
                continue;
            }
            let (a, c) = (corners[i], corners[j]);
            let (fa, fc) = (f(a), f(c));
            if (fa < 0.0 && fc > 0.0) || (fa > 0.0 && fc < 0.0) {
                let t = fa / (fa - fc);
                points.push(a + (c - a).scale(t));
            }
        }
    }
    bounds_of(points.into_iter())
}

fn box_center(b: &Aabb3) -> Point3 {
    Point3::new(
        (b.min.x + b.max.x) * 0.5,
        (b.min.y + b.max.y) * 0.5,
        (b.min.z + b.max.z) * 0.5,
    )
}

fn union_box(a: &Aabb3, b: &Aabb3) -> Aabb3 {
    Aabb3::new(
        Point3::new(
            a.min.x.min(b.min.x),
            a.min.y.min(b.min.y),
            a.min.z.min(b.min.z),
        ),
        Point3::new(
            a.max.x.max(b.max.x),
            a.max.y.max(b.max.y),
            a.max.z.max(b.max.z),
        ),
    )
}

/// Exploded-view offsets: for each box, `factor` times the vector from the
/// centre of all boxes together to the box's own centre, with the
/// components of the axes not chosen (`[x, y, z]`) set to zero. An item
/// without a box (`None`) stays put. Knows boxes and nothing else.
pub fn explode_offsets(boxes: &[Option<Aabb3>], factor: f64, axes: [bool; 3]) -> Vec<Vec3> {
    let all = boxes.iter().flatten().fold(None, |acc: Option<Aabb3>, b| {
        Some(acc.map_or(*b, |a| union_box(&a, b)))
    });
    let Some(all) = all else {
        return vec![Vec3::new(0.0, 0.0, 0.0); boxes.len()];
    };
    let c = box_center(&all);
    let pick = |on: bool, v: f64| if on { v * factor } else { 0.0 };
    boxes
        .iter()
        .map(|b| match b {
            Some(b) => {
                let d = box_center(b) - c;
                Vec3::new(pick(axes[0], d.x), pick(axes[1], d.y), pick(axes[2], d.z))
            }
            None => Vec3::new(0.0, 0.0, 0.0),
        })
        .collect()
}

/// One item of a [`Frame3d`].
#[derive(Debug, Clone)]
pub struct FrameItem<'a> {
    /// The scene's shared mesh (a backend may key a cache on the `Arc`).
    pub mesh: &'a Arc<RenderMesh>,
    /// Mesh frame to world. Rigid, so it also turns normals.
    pub model: Mat4,
    pub material: Material,
    /// Linear-light flat colour of this item's cut faces.
    pub section_color: [f64; 3],
    /// Edges to draw in this view (vertex index pairs into `mesh`): every
    /// sharp edge plus the smooth edges that are silhouettes from this
    /// eye. Empty when the scene draws no edges. Hidden ones are left to
    /// the depth test.
    pub edges: Vec<[u32; 2]>,
    /// Where the material's pattern sits on this item ([`pattern_frame`]).
    pub pattern: PatternFrame,
}

/// The ground as a backend draws it: a square of side `2 half_size` about
/// `center` at height `z`, flat `color` fading into the frame background
/// between the `fade` radii from `center`. Unlit; darkened only by the
/// ambient occlusion.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FrameGround {
    pub z: f64,
    pub center: [f64; 2],
    pub half_size: f64,
    /// Inner and outer fade radius (mm).
    pub fade: (f64, f64),
    /// Linear-light colour.
    pub color: [f64; 3],
}

impl FrameGround {
    /// The four corners, counter-clockwise seen from above.
    pub fn corners(&self) -> [Point3; 4] {
        let [x, y] = self.center;
        let h = self.half_size;
        [
            Point3::new(x - h, y - h, self.z),
            Point3::new(x + h, y - h, self.z),
            Point3::new(x + h, y + h, self.z),
            Point3::new(x - h, y + h, self.z),
        ]
    }

    /// The linear colour at world point `(x, y)` before occlusion.
    pub fn color_at(&self, x: f64, y: f64, background: [f64; 3]) -> [f64; 3] {
        let d = ((x - self.center[0]).powi(2) + (y - self.center[1]).powi(2)).sqrt();
        let (a, b) = self.fade;
        let t = ((d - a) / (b - a).max(1e-12)).clamp(0.0, 1.0);
        let s = t * t * (3.0 - 2.0 * t);
        std::array::from_fn(|i| self.color[i] + (background[i] - self.color[i]) * s)
    }
}

/// Ambient occlusion as a backend runs it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FrameOcclusion {
    /// Sampling radius, world mm.
    pub radius: f64,
    pub strength: f64,
    /// A nearer surface counts as an occluder only when it is at least
    /// this much (mm) in front of the sample point, so flat and convex
    /// surfaces get none.
    pub bias: f64,
    /// Depth range, in radii: an occluder whose depth differs from the
    /// shaded point's by up to `range.0` radii counts fully, fading to
    /// nothing at `range.1` (so a part far in front of another does not
    /// darken it). The 4 x 4 average only mixes pixels within `range.0`
    /// radii in depth.
    pub range: (f64, f64),
    /// [`ao_kernel`].
    pub kernel: [Vec3; AO_SAMPLES],
}

/// What a backend draws for one image: the 3D twin of
/// [`crate::render::DisplayList`]. All of it in f64; backends cast to f32.
#[derive(Debug, Clone)]
pub struct Frame3d<'a> {
    pub width: u32,
    pub height: u32,
    /// World to eye.
    pub view: Mat4,
    /// Eye to clip (WebGPU depth range).
    pub projection: Mat4,
    /// `projection * view`.
    pub view_projection: Mat4,
    /// Where the viewer is, homogeneous: `(eye, 1)` for perspective, the
    /// unit direction toward the viewer `(back, 0)` for orthographic. The
    /// direction toward the viewer at `p` is `normalize(eye.xyz - p eye.w)`.
    pub eye: [f64; 4],
    /// Unit direction toward the key light, world frame.
    pub key_direction: Vec3,
    /// Unit direction toward the fill light, world frame.
    pub fill_direction: Vec3,
    pub lighting: Lighting,
    /// The edge style, if edges are drawn. Its colour and width also draw
    /// the line where a clip plane cuts a part's faces.
    pub edges: Option<EdgeStyle>,
    /// The contrasting outline for this background, if one is needed
    /// ([`EdgeStyle::outline`]).
    pub outline: Option<Outline>,
    /// Linear-light clear colour.
    pub background: [f64; 3],
    /// The ground, if any.
    pub ground: Option<FrameGround>,
    /// Ambient occlusion, if any.
    pub occlusion: Option<FrameOcclusion>,
    /// Clip planes as `[nx, ny, nz, w]` with a unit normal: a point `p` is
    /// beyond a plane where `n . p + w > 0`. At most [`MAX_CLIP_PLANES`].
    pub clip_planes: Vec<[f64; 4]>,
    /// A point is removed when it is beyond any plane, or beyond all of
    /// them ([`ClipMode`]).
    pub clip_mode: ClipMode,
    /// The clip range used by `projection`.
    pub near_far: (f64, f64),
    /// Device pixels per CSS pixel: scales the edge width. 1 from
    /// [`build_frame`]; a shell sets it.
    pub pixel_ratio: f64,
    /// The world size of one pixel at the target's depth (mm). Backends
    /// pull edge lines this far toward the eye so the faces they lie on do
    /// not hide them.
    pub pixel_world: f64,
    /// The presentation render's light ([`Scene3d::environment`]).
    pub environment: FrameEnvironment,
    /// The presentation render's exposure as a factor ([`exposure_scale`]).
    pub exposure: f64,
    pub items: Vec<FrameItem<'a>>,
}

/// Build the frame for `scene` at `width` x `height` pixels, with the
/// scene's camera as it is (call [`Scene3d::fit_camera`] first to frame
/// everything).
pub fn build_frame(scene: &Scene3d, width: u32, height: u32) -> Frame3d<'_> {
    let cam = &scene.camera;
    let aspect = width.max(1) as f64 / height.max(1) as f64;
    let bounds = scene.bounds();
    let Background::Solid(bg) = scene.background;
    let background = bg.map(srgb_to_linear);
    let ground = match (scene.ground, bounds) {
        (Some(g), Some(b)) => {
            let reach = 0.5 * ((b.max.x - b.min.x).hypot(b.max.y - b.min.y));
            let reach = reach.max(0.5 * (b.max.z - b.min.z)).max(1e-6);
            let c = box_center(&b);
            Some(FrameGround {
                z: g.height.unwrap_or(b.min.z),
                center: [c.x, c.y],
                half_size: GROUND_EXTENT * reach + (b.max.z - b.min.z),
                fade: (GROUND_FADE.0 * reach, GROUND_FADE.1 * reach),
                color: g.color.map_or(background, |c| c.map(srgb_to_linear)),
            })
        }
        _ => None,
    };
    // The clip range also encloses the ground square.
    let depth_bounds = match (bounds, ground) {
        (Some(b), Some(g)) => {
            let c = g.corners();
            Some(union_box(&b, &Aabb3::new(c[0], c[2])))
        }
        (b, _) => b,
    };
    let near_far = cam.clip_range(depth_bounds.as_ref());
    let occlusion = match (scene.occlusion, bounds) {
        (Some(o), Some(b)) => {
            let diagonal = (b.max - b.min).length();
            let radius = o
                .radius
                .filter(|r| r.is_finite() && *r > 0.0)
                .unwrap_or(AO_RADIUS_FRACTION * diagonal)
                .max(1e-9);
            Some(FrameOcclusion {
                radius,
                strength: o.strength.clamp(0.0, 1.0),
                bias: 0.03 * radius,
                range: (0.5, 1.5),
                kernel: ao_kernel(),
            })
        }
        _ => None,
    };
    let clip_planes = clip_planes_of(&scene.clip_planes);
    let view = cam.view_matrix();
    let projection = cam.projection_matrix(aspect, near_far.0, near_far.1);
    let eye = match cam.projection {
        Projection::Perspective => {
            let e = cam.eye();
            [e.x, e.y, e.z, 1.0]
        }
        Projection::Orthographic => {
            let b = cam.back();
            [b.x, b.y, b.z, 0.0]
        }
    };
    let l = scene.lighting;
    let to_world = |d: Vec3| {
        let w = match l.frame {
            LightFrame::World => d,
            LightFrame::Camera => {
                cam.right().scale(d.x) + cam.up().scale(d.y) + cam.back().scale(d.z)
            }
        };
        w.normalized().unwrap_or(cam.back())
    };
    let env = scene.environment;
    let k = env.intensity.max(0.0);
    let environment = FrameEnvironment {
        dome: Environment {
            zenith: env.zenith.map(|v| v.max(0.0) * k),
            horizon: env.horizon.map(|v| v.max(0.0) * k),
            ground: env.ground.map(|v| v.max(0.0) * k),
            sun: None,
            intensity: 1.0,
        },
        sun: env.sun.and_then(|s| {
            let d = match s.frame {
                LightFrame::World => s.direction,
                LightFrame::Camera => {
                    cam.right().scale(s.direction.x)
                        + cam.up().scale(s.direction.y)
                        + cam.back().scale(s.direction.z)
                }
            }
            .normalized()?;
            if !(s.irradiance.is_finite() && s.irradiance > 0.0 && k > 0.0) {
                return None;
            }
            let radius = (0.5 * s.angle_deg.clamp(MIN_SUN_ANGLE_DEG, 90.0)).to_radians();
            let sin2 = radius.sin().powi(2);
            let scale = k * s.irradiance / (std::f64::consts::PI * sin2);
            Some(FrameSun {
                direction: d,
                cos_radius: radius.cos(),
                radiance: s.color.map(|c| c.max(0.0) * scale),
            })
        }),
    };
    Frame3d {
        width,
        height,
        view,
        projection,
        view_projection: mat_mul(&projection, &view),
        eye,
        key_direction: to_world(l.key_direction),
        fill_direction: to_world(l.fill_direction),
        lighting: l,
        edges: scene.edges,
        outline: scene.edges.and_then(|e| e.outline(bg)),
        background,
        ground,
        occlusion,
        clip_planes,
        clip_mode: scene.clip_mode,
        near_far,
        pixel_ratio: 1.0,
        pixel_world: 2.0 * cam.half_height() / height.max(1) as f64,
        environment,
        exposure: exposure_scale(scene.exposure),
        items: scene
            .items
            .iter()
            .enumerate()
            .filter(|(_, it)| !it.mesh.triangles.is_empty())
            .map(|(index, it)| {
                let model = placement_matrix(&it.placement);
                let edges = if scene.edges.is_some() {
                    visible_edges(&it.mesh, &model, eye)
                } else {
                    Vec::new()
                };
                FrameItem {
                    mesh: &it.mesh,
                    model,
                    material: it.material,
                    section_color: it.section.map_or_else(
                        || default_section_color(&it.material),
                        |c| c.map(srgb_to_linear),
                    ),
                    edges,
                    pattern: pattern_frame(&it.material, &it.mesh, index),
                }
            })
            .collect(),
    }
}

/// Sharp edges plus the smooth edges that are silhouettes for `eye`
/// (homogeneous, as in [`Frame3d::eye`]), in the mesh's own frame.
fn visible_edges(mesh: &RenderMesh, model: &Mat4, eye: [f64; 4]) -> Vec<[u32; 2]> {
    // Bring the eye into the mesh frame: the inverse of a rigid transform
    // is the transposed rotation applied after removing the translation.
    let t = Vec3::new(model[3][0], model[3][1], model[3][2]);
    let e = Vec3::new(eye[0], eye[1], eye[2]);
    let e = if eye[3] != 0.0 { e + (-t) } else { e };
    let col = |c: usize| Vec3::new(model[c][0], model[c][1], model[c][2]);
    let local = Vec3::new(col(0).dot(e), col(1).dot(e), col(2).dot(e));
    let facing = |f: u32| {
        let n = mesh.face_normals[f as usize];
        if eye[3] == 0.0 {
            n.dot(local) > 0.0
        } else {
            let p = mesh.positions[mesh.triangles[f as usize][0] as usize];
            n.dot(Vec3::new(local.x - p.x, local.y - p.y, local.z - p.z)) > 0.0
        }
    };
    let mut edges = mesh.sharp_edges.clone();
    edges.extend(
        mesh.smooth_edges
            .iter()
            .filter(|e| facing(e.faces[0]) != facing(e.faces[1]))
            .map(|e| e.ends),
    );
    edges
}

impl Frame3d<'_> {
    /// The colour [`shade`] gives a surface at world point `p` with world
    /// normal `n`, tone mapped and sRGB-encoded: what a backend's pixel
    /// there should be. The normal is flipped toward the viewer first, as
    /// backends do for back faces.
    pub fn expected_srgb(&self, p: Point3, n: Vec3, m: &Material) -> [u8; 3] {
        let e = self.eye;
        let v = Vec3::new(e[0] - p.x * e[3], e[1] - p.y * e[3], e[2] - p.z * e[3])
            .normalized()
            .unwrap_or(Vec3::Z);
        let n = n.normalized().unwrap_or(v);
        let n = if n.dot(v) < 0.0 { -n } else { n };
        tone_map(shade(
            n,
            v,
            self.key_direction,
            self.fill_direction,
            m,
            &self.lighting,
        ))
        .map(encode_srgb)
    }

    /// A world-space normal for a mesh-frame normal of `item`.
    pub fn world_normal(item: &FrameItem<'_>, n: Vec3) -> Vec3 {
        transform_dir(&item.model, n)
    }
}

// ── Procedural meshes (examples and tests) ─────────────────────────────

/// Closed meshes built from triangles, counter-clockwise from outside, for
/// tests and examples. Real parts come from a kernel's tessellation.
pub mod shapes {
    use super::*;

    /// An axis-aligned box from `min` to `max`: 8 positions, 12 triangles.
    pub fn cuboid(min: Point3, max: Point3) -> TriMesh {
        let positions = aabb_corners(&Aabb3::new(min, max)).to_vec();
        // Corner index bits: x = 1, y = 2, z = 4.
        let quads: [[u32; 4]; 6] = [
            [0, 2, 3, 1], // z min (normal -Z)
            [4, 5, 7, 6], // z max
            [0, 1, 5, 4], // y min
            [2, 6, 7, 3], // y max
            [0, 4, 6, 2], // x min
            [1, 3, 7, 5], // x max
        ];
        let mut triangles = Vec::new();
        for [a, b, c, d] in quads {
            triangles.push([a, b, c]);
            triangles.push([a, c, d]);
        }
        TriMesh {
            positions,
            triangles,
        }
    }

    /// A closed cylinder about Z: radius `r`, from z = 0 to `h`, `n`
    /// facets around (an inscribed prism).
    pub fn cylinder(r: f64, h: f64, n: u32) -> TriMesh {
        let n = n.max(3);
        let mut positions = Vec::new();
        for i in 0..n {
            let a = std::f64::consts::TAU * i as f64 / n as f64;
            let (s, c) = a.sin_cos();
            positions.push(Point3::new(r * c, r * s, 0.0));
            positions.push(Point3::new(r * c, r * s, h));
        }
        let (bc, tc) = (2 * n, 2 * n + 1);
        positions.push(Point3::new(0.0, 0.0, 0.0));
        positions.push(Point3::new(0.0, 0.0, h));
        let mut triangles = Vec::new();
        for i in 0..n {
            let j = (i + 1) % n;
            let (b0, t0, b1, t1) = (2 * i, 2 * i + 1, 2 * j, 2 * j + 1);
            triangles.push([b0, b1, t1]);
            triangles.push([b0, t1, t0]);
            triangles.push([bc, b1, b0]);
            triangles.push([tc, t0, t1]);
        }
        TriMesh {
            positions,
            triangles,
        }
    }

    /// A UV sphere of radius `r` about the origin with `rings` latitude
    /// bands and `segments` longitude segments.
    pub fn sphere(r: f64, rings: u32, segments: u32) -> TriMesh {
        let (rings, segments) = (rings.max(2), segments.max(3));
        let mut positions = vec![Point3::new(0.0, 0.0, -r)];
        for i in 1..rings {
            let phi = std::f64::consts::PI * i as f64 / rings as f64;
            let (sp, cp) = phi.sin_cos();
            for j in 0..segments {
                let th = std::f64::consts::TAU * j as f64 / segments as f64;
                let (st, ct) = th.sin_cos();
                positions.push(Point3::new(r * sp * ct, r * sp * st, -r * cp));
            }
        }
        positions.push(Point3::new(0.0, 0.0, r));
        let top = positions.len() as u32 - 1;
        let at = |i: u32, j: u32| 1 + (i - 1) * segments + j % segments;
        let mut triangles = Vec::new();
        for j in 0..segments {
            triangles.push([0, at(1, j + 1), at(1, j)]);
            triangles.push([top, at(rings - 1, j), at(rings - 1, j + 1)]);
        }
        for i in 1..rings - 1 {
            for j in 0..segments {
                let (a, b) = (at(i, j), at(i, j + 1));
                let (c, d) = (at(i + 1, j), at(i + 1, j + 1));
                triangles.push([a, b, d]);
                triangles.push([a, d, c]);
            }
        }
        TriMesh {
            positions,
            triangles,
        }
    }

    /// A square plate `size` x `size` x `thick` (z from 0) with a square
    /// through hole of side `hole` in the middle, built from triangles by
    /// hand: a frame of four boxes would share inner faces, so the faces
    /// are listed directly.
    pub fn plate_with_square_hole(size: f64, hole: f64, thick: f64) -> TriMesh {
        let (o, i) = (size * 0.5, hole * 0.5);
        let ring = |z: f64| {
            [
                Point3::new(-o, -o, z),
                Point3::new(o, -o, z),
                Point3::new(o, o, z),
                Point3::new(-o, o, z),
                Point3::new(-i, -i, z),
                Point3::new(i, -i, z),
                Point3::new(i, i, z),
                Point3::new(-i, i, z),
            ]
        };
        let mut positions = ring(0.0).to_vec();
        positions.extend(ring(thick));
        let mut triangles = Vec::new();
        for k in 0..4u32 {
            let k1 = (k + 1) % 4;
            let (oa, ob, ia, ib) = (k, k1, 4 + k, 4 + k1);
            // Top (z = thick, +Z) and bottom (-Z) of the frame strip.
            triangles.push([8 + oa, 8 + ob, 8 + ib]);
            triangles.push([8 + oa, 8 + ib, 8 + ia]);
            triangles.push([oa, ib, ob]);
            triangles.push([oa, ia, ib]);
            // Outer wall (faces outward) and inner wall (faces the hole).
            triangles.push([oa, ob, 8 + ob]);
            triangles.push([oa, 8 + ob, 8 + oa]);
            triangles.push([ia, 8 + ib, ib]);
            triangles.push([ia, 8 + ia, 8 + ib]);
        }
        TriMesh {
            positions,
            triangles,
        }
    }

    /// A `size` x `size` x `thick` block (z from 0) with a square blind
    /// pocket of side `pocket` and depth `depth` in the middle of its top.
    pub fn block_with_pocket(size: f64, pocket: f64, thick: f64, depth: f64) -> TriMesh {
        let (o, i) = (size * 0.5, pocket * 0.5);
        let square = |h: f64, z: f64| {
            [
                Point3::new(-h, -h, z),
                Point3::new(h, -h, z),
                Point3::new(h, h, z),
                Point3::new(-h, h, z),
            ]
        };
        // 0..4 outer top, 4..8 pocket rim, 8..12 pocket floor, 12..16 outer
        // bottom.
        let mut positions = square(o, thick).to_vec();
        positions.extend(square(i, thick));
        positions.extend(square(i, thick - depth));
        positions.extend(square(o, 0.0));
        let mut triangles = Vec::new();
        let mut quad = |a: u32, b: u32, c: u32, d: u32| {
            triangles.push([a, b, c]);
            triangles.push([a, c, d]);
        };
        for k in 0..4u32 {
            let k1 = (k + 1) % 4;
            quad(k, k1, 4 + k1, 4 + k); // top frame strip (+Z)
            quad(4 + k, 4 + k1, 8 + k1, 8 + k); // pocket wall, facing in
            quad(12 + k, 12 + k1, k1, k); // outer wall
        }
        quad(8, 9, 10, 11); // pocket floor (+Z)
        quad(12, 15, 14, 13); // bottom (-Z)
        TriMesh {
            positions,
            triangles,
        }
    }

    /// A closed box from `min` to `max` with a closed box-shaped cavity
    /// from `cavity_min` to `cavity_max` inside it: the outer box faces
    /// out, the cavity's faces point into the cavity.
    pub fn box_with_cavity(
        min: Point3,
        max: Point3,
        cavity_min: Point3,
        cavity_max: Point3,
    ) -> TriMesh {
        let mut m = cuboid(min, max);
        let inner = cuboid(cavity_min, cavity_max);
        let base = m.positions.len() as u32;
        m.positions.extend(inner.positions);
        m.triangles.extend(
            inner
                .triangles
                .iter()
                .map(|[a, b, c]| [base + a, base + c, base + b]),
        );
        m
    }
}

#[cfg(test)]
mod tests {
    use super::shapes::*;
    use super::*;
    use std::f64::consts::{FRAC_PI_2, PI};

    fn close(a: f64, b: f64, tol: f64) -> bool {
        (a - b).abs() <= tol
    }

    fn vclose(a: Vec3, b: Vec3, tol: f64) -> bool {
        close(a.x, b.x, tol) && close(a.y, b.y, tol) && close(a.z, b.z, tol)
    }

    /// Signed volume of a closed mesh (divergence theorem); positive when
    /// triangles face out.
    fn volume(m: &TriMesh) -> f64 {
        m.triangles
            .iter()
            .map(|t| {
                let [a, b, c] = t.map(|i| m.positions[i as usize]);
                let (a, b, c) = (
                    Vec3::new(a.x, a.y, a.z),
                    Vec3::new(b.x, b.y, b.z),
                    Vec3::new(c.x, c.y, c.z),
                );
                a.dot(b.cross(c)) / 6.0
            })
            .sum()
    }

    #[test]
    fn procedural_meshes_are_closed_and_face_out() {
        let b = cuboid(Point3::new(0.0, 0.0, 0.0), Point3::new(2.0, 3.0, 4.0));
        assert!(close(volume(&b), 24.0, 1e-12));
        let n = 64;
        let c = cylinder(1.0, 2.0, n);
        let prism = 0.5 * n as f64 * (2.0 * PI / n as f64).sin() * 2.0;
        assert!(close(volume(&c), prism, 1e-12));
        let s = sphere(1.0, 32, 64);
        let v = volume(&s);
        assert!(
            v > 0.0 && close(v, 4.0 / 3.0 * PI, 0.03),
            "sphere volume {v}"
        );
        let p = plate_with_square_hole(10.0, 4.0, 1.0);
        assert!(close(volume(&p), 100.0 - 16.0, 1e-12));
    }

    #[test]
    fn named_views_look_down_the_right_axis() {
        let expect = [
            (NamedView::Front, -Vec3::Y, Vec3::X, Vec3::Z),
            (NamedView::Back, Vec3::Y, -Vec3::X, Vec3::Z),
            (NamedView::Left, -Vec3::X, -Vec3::Y, Vec3::Z),
            (NamedView::Right, Vec3::X, Vec3::Y, Vec3::Z),
            (NamedView::Top, Vec3::Z, Vec3::X, Vec3::Y),
            (NamedView::Bottom, -Vec3::Z, Vec3::X, -Vec3::Y),
        ];
        for (view, back, right, up) in expect {
            let cam = Camera3d::new(view, Projection::Orthographic);
            assert!(vclose(cam.back(), back, 1e-12), "{view:?} back");
            assert!(vclose(cam.right(), right, 1e-12), "{view:?} right");
            assert!(vclose(cam.up(), up, 1e-12), "{view:?} up");
            assert_eq!(view.name().parse::<NamedView>().unwrap(), view);
        }
        let iso = Camera3d::new(NamedView::Isometric, Projection::Orthographic);
        let d = 1.0 / 3f64.sqrt();
        assert!(vclose(iso.back(), Vec3::new(d, -d, d), 1e-12));
        // Z projects straight up on screen in the isometric view.
        assert!(close(iso.right().z, 0.0, 1e-15) && iso.up().z > 0.0);
        // Right-handed eye frame: right x up = back.
        for v in NamedView::ALL {
            let c = Camera3d::new(v, Projection::Perspective);
            assert!(vclose(c.right().cross(c.up()), c.back(), 1e-12), "{v:?}");
        }
    }

    #[test]
    fn view_matrix_maps_eye_to_origin_and_target_down_minus_z() {
        let mut cam = Camera3d::new(NamedView::Isometric, Projection::Perspective);
        cam.target = Point3::new(10.0, -5.0, 3.0);
        cam.distance = 250.0;
        let v = cam.view_matrix();
        let e = mat_vec(&v, {
            let e = cam.eye();
            [e.x, e.y, e.z, 1.0]
        });
        assert!(e[..3].iter().all(|c| c.abs() < 1e-9));
        let t = mat_vec(&v, [10.0, -5.0, 3.0, 1.0]);
        assert!(close(t[0], 0.0, 1e-9) && close(t[1], 0.0, 1e-9) && close(t[2], -250.0, 1e-9));
    }

    #[test]
    fn orthographic_projection_matches_closed_form() {
        // Front view, target at the origin, distance 10, fov 60 degrees:
        // half-height 10 tan 30 = 5.7735, half-width that times 1.5.
        let mut cam = Camera3d::new(NamedView::Front, Projection::Orthographic);
        cam.distance = 10.0;
        cam.fov_y = 60f64.to_radians();
        let (w, h) = (300, 200);
        let hh = 10.0 * (30f64).to_radians().tan();
        let (x, y, z) = cam
            .project(Point3::new(1.0, 0.0, 2.0), w, h, (5.0, 15.0))
            .unwrap();
        assert!(close(x, 150.0 + 1.0 / (hh * 1.5) * 150.0, 1e-9));
        assert!(close(y, 100.0 - 2.0 / hh * 100.0, 1e-9));
        // Depth: eye distance 10 in [5, 15] is halfway.
        assert!(close(z, 0.5, 1e-12));
        // Orthographic: depth along the view does not move the point.
        let (x2, y2, z2) = cam
            .project(Point3::new(1.0, 4.0, 2.0), w, h, (5.0, 15.0))
            .unwrap();
        assert!(close(x2, x, 1e-9) && close(y2, y, 1e-9));
        assert!(close(z2, 0.9, 1e-12));
    }

    #[test]
    fn perspective_projection_matches_closed_form() {
        let mut cam = Camera3d::new(NamedView::Front, Projection::Perspective);
        cam.distance = 10.0;
        cam.fov_y = 60f64.to_radians();
        let (w, h) = (300, 200);
        let t = 30f64.to_radians().tan();
        let (near, far) = (2.0, 50.0);
        // The target projects to the centre.
        let (x, y, _) = cam.project(Point3::ORIGIN, w, h, (near, far)).unwrap();
        assert!(close(x, 150.0, 1e-9) && close(y, 100.0, 1e-9));
        // A point 4 behind the target (eye distance 14), offset (1, 2).
        let (x, y, z) = cam
            .project(Point3::new(1.0, 4.0, 2.0), w, h, (near, far))
            .unwrap();
        assert!(close(x, 150.0 + 1.0 / (14.0 * t * 1.5) * 150.0, 1e-9));
        assert!(close(y, 100.0 - 2.0 / (14.0 * t) * 100.0, 1e-9));
        // Depth = far (d - near) / (d (far - near)).
        assert!(close(z, far * (14.0 - near) / (14.0 * (far - near)), 1e-12));
        // Near and far planes map to 0 and 1.
        let zn = cam
            .project(Point3::new(0.0, -8.0, 0.0), w, h, (near, far))
            .unwrap()
            .2;
        let zf = cam
            .project(Point3::new(0.0, 40.0, 0.0), w, h, (near, far))
            .unwrap()
            .2;
        assert!(close(zn, 0.0, 1e-12) && close(zf, 1.0, 1e-12));
        // Behind the eye: no projection.
        assert!(cam
            .project(Point3::new(0.0, -20.0, 0.0), w, h, (near, far))
            .is_none());
    }

    /// Fit puts every corner inside the frame shrunk by the margin, with
    /// at least one on its edge, in every view and both projections.
    #[test]
    fn fit_frames_a_box_with_the_margin() {
        let b = Aabb3::new(Point3::new(-20.0, 5.0, 0.0), Point3::new(100.0, 45.0, 18.0));
        let (w, h) = (960u32, 640u32);
        let margin = 1.1;
        for proj in [Projection::Orthographic, Projection::Perspective] {
            for view in NamedView::ALL {
                let mut cam = Camera3d::new(view, proj);
                cam.yaw += 0.1; // off-axis too
                assert!(cam.fit(&b, w as f64 / h as f64, margin));
                let nf = cam.clip_range(Some(&b));
                let mut worst: f64 = 0.0;
                for p in aabb_corners(&b) {
                    let (x, y, z) = cam.project(p, w, h, nf).unwrap();
                    let nx = (x / w as f64 * 2.0 - 1.0).abs();
                    let ny = (1.0 - y / h as f64 * 2.0).abs();
                    assert!(nx <= 1.0 / margin + 1e-9, "{view:?} {proj:?} x {nx}");
                    assert!(ny <= 1.0 / margin + 1e-9, "{view:?} {proj:?} y {ny}");
                    assert!((0.0..=1.0).contains(&z), "{view:?} {proj:?} depth {z}");
                    worst = worst.max(nx).max(ny);
                }
                assert!(
                    close(worst, 1.0 / margin, 1e-9),
                    "{view:?} {proj:?} touches"
                );
            }
        }
    }

    #[test]
    fn placement_matrix_rotates_then_translates() {
        let p = Placement::axis_angle(Vec3::Z, FRAC_PI_2, Vec3::new(1.0, 2.0, 3.0));
        let m = placement_matrix(&p);
        let q = transform_point(&m, Point3::new(1.0, 0.0, 0.0));
        assert!(q.max_abs_diff(Point3::new(1.0, 3.0, 3.0)) < 1e-12);
        let d = transform_dir(&m, Vec3::Y);
        assert!(vclose(d, -Vec3::X, 1e-12));
    }

    #[test]
    fn box_normals_are_flat_facets() {
        let m = RenderMesh::from_trimesh(
            &cuboid(Point3::ORIGIN, Point3::new(1.0, 1.0, 1.0)),
            DEFAULT_CREASE_DEG,
        );
        // 6 faces x 4 corners, each with its face normal.
        assert_eq!(m.positions.len(), 24);
        assert_eq!(m.triangles.len(), 12);
        for t in &m.triangles {
            let [a, b, c] = t.map(|i| m.positions[i as usize]);
            let face = (b - a).cross(c - a).normalized().unwrap();
            for &i in t {
                assert!(vclose(m.normals[i as usize], face, 1e-12));
            }
        }
    }

    #[test]
    fn cylinder_normals_are_smooth_on_the_side_and_flat_on_the_caps() {
        let n = 24;
        let m = RenderMesh::from_trimesh(&cylinder(5.0, 10.0, n), DEFAULT_CREASE_DEG);
        // Side: one vertex per rim position (2n), with the exact radial
        // normal; caps: n rim vertices + centre each.
        assert_eq!(m.positions.len(), 2 * n as usize + 2 * (n as usize + 1));
        for (p, nrm) in m.positions.iter().zip(&m.normals) {
            let radial = Vec3::new(p.x, p.y, 0.0).normalized();
            let on_side = vclose(*nrm, radial.unwrap_or(Vec3::Z), 1e-9);
            let on_cap = vclose(*nrm, Vec3::Z, 1e-12) || vclose(*nrm, -Vec3::Z, 1e-12);
            assert!(on_side || on_cap, "normal {nrm:?} at {p:?}");
        }
        // Crease 0: every facet flat, no shared side vertices.
        let flat = RenderMesh::from_trimesh(&cylinder(5.0, 10.0, n), 0.0);
        assert_eq!(flat.positions.len(), 4 * n as usize + 2 * (n as usize + 1));
    }

    #[test]
    fn degenerate_triangles_are_dropped() {
        let mesh = TriMesh {
            positions: vec![
                Point3::ORIGIN,
                Point3::new(1.0, 0.0, 0.0),
                Point3::new(2.0, 0.0, 0.0),
                Point3::new(0.0, 1.0, 0.0),
            ],
            triangles: vec![[0, 1, 2], [0, 1, 3], [0, 1, 9]],
        };
        let m = RenderMesh::from_trimesh(&mesh, DEFAULT_CREASE_DEG);
        assert_eq!(m.triangles.len(), 1);
        assert!(vclose(m.normals[0], Vec3::Z, 1e-12));
    }

    #[test]
    fn shading_is_brighter_facing_the_light() {
        let m = Material::default();
        let l = Lighting::default();
        let v = Vec3::Z;
        let key = Vec3::new(0.0, 0.6, 0.8);
        let fill = Vec3::X;
        let lit = shade(Vec3::Z, v, key, fill, &m, &l);
        let grazing = shade(Vec3::new(0.0, -0.6, 0.8), v, key, fill, &m, &l);
        let away = shade(-Vec3::Y, v, key, fill, &m, &l);
        assert!(lit[0] > grazing[0] && grazing[0] > away[0]);
        // Facing away from both lights: ambient only.
        let amb = l.ambient * (m.base_color[0] + 0.04);
        assert!(close(away[0], amb, 1e-12));
        // Facing the fill: ambient plus the fill's diffuse term.
        let filled = shade(Vec3::X, v, key, fill, &m, &l);
        assert!(close(
            filled[0],
            amb + l.fill_intensity * m.base_color[0],
            1e-12
        ));
    }

    #[test]
    fn box_edges_are_all_sharp() {
        let m = RenderMesh::from_trimesh(
            &cuboid(Point3::ORIGIN, Point3::new(1.0, 2.0, 3.0)),
            DEFAULT_CREASE_DEG,
        );
        // 12 box edges; the face diagonals are inside flat faces.
        assert_eq!(m.sharp_edges.len(), 12);
        assert!(m.smooth_edges.is_empty());
        for [a, b] in &m.sharp_edges {
            let d = m.positions[*a as usize] - m.positions[*b as usize];
            let axes = [d.x, d.y, d.z].iter().filter(|c| c.abs() > 0.0).count();
            assert_eq!(axes, 1, "box edge runs along one axis");
        }
    }

    /// A cylinder's rims are sharp; of its side edges only the two on the
    /// outline are drawn, for both projections.
    #[test]
    fn cylinder_draws_rims_and_two_silhouettes() {
        let n = 24;
        let m = RenderMesh::from_trimesh(&cylinder(5.0, 10.0, n), DEFAULT_CREASE_DEG);
        assert_eq!(m.sharp_edges.len(), 2 * n as usize);
        assert_eq!(m.smooth_edges.len(), n as usize);
        for proj in [Projection::Orthographic, Projection::Perspective] {
            let mut scene = Scene3d::default();
            scene.add_mesh(
                &cylinder(5.0, 10.0, n),
                Placement::translation(100.0, 50.0, 0.0),
                Material::default(),
            );
            scene.camera = Camera3d::new(NamedView::Front, proj);
            scene.fit_camera(1.5);
            let f = build_frame(&scene, 300, 200);
            let edges = &f.items[0].edges;
            assert_eq!(edges.len(), 2 * n as usize + 2, "{proj:?}");
            let mesh = f.items[0].mesh;
            let silhouettes: Vec<_> = edges[2 * n as usize..]
                .iter()
                .map(|[a, _]| mesh.positions[*a as usize].x)
                .collect();
            // Ortho: the outline is at x = +-r. Perspective: the tangent
            // from the eye, on a side edge next to x = +-r.
            let tol = match proj {
                Projection::Orthographic => 1e-9,
                Projection::Perspective => 5.0 * (1.0 - 15f64.to_radians().cos()) + 1e-9,
            };
            assert!(
                silhouettes.iter().all(|x| close(x.abs(), 5.0, tol)),
                "{proj:?} {silhouettes:?}"
            );
            assert!(silhouettes[0] * silhouettes[1] < 0.0);
        }
        // No edges when the scene draws none.
        let mut scene = Scene3d {
            edges: None,
            ..Scene3d::default()
        };
        scene.add_mesh(
            &cylinder(5.0, 10.0, n),
            Placement::IDENTITY,
            Material::default(),
        );
        assert!(build_frame(&scene, 30, 20).items[0].edges.is_empty());
    }

    #[test]
    fn tone_curve_and_srgb() {
        assert_eq!(tone_map([0.0; 3]), [0.0; 3]);
        // Mid-tones pass with only the 0.04 toe offset.
        let t = tone_map([0.5, 0.4, 0.3]);
        assert!(close(t[0], 0.46, 1e-12) && close(t[2], 0.26, 1e-12));
        // Highlights roll off below 1, monotonically.
        let a = tone_map([2.0; 3])[0];
        let b = tone_map([8.0; 3])[0];
        assert!(a < b && b < 1.0 && a > 0.76);
        assert_eq!(encode_srgb(0.0), 0);
        assert_eq!(encode_srgb(1.0), 255);
        assert_eq!(encode_srgb(0.215_860_5), 128);
        for c in [0u8, 1, 30, 128, 200, 255] {
            assert_eq!(encode_srgb(srgb_to_linear(c)), c);
        }
    }

    #[test]
    fn pocket_and_cavity_meshes_are_closed_and_face_out() {
        let p = block_with_pocket(60.0, 20.0, 30.0, 18.0);
        assert!(close(
            volume(&p),
            60.0 * 60.0 * 30.0 - 20.0 * 20.0 * 18.0,
            1e-9
        ));
        let m = RenderMesh::from_trimesh(&p, DEFAULT_CREASE_DEG);
        // Every edge has two facets: no boundary edges counted as sharp
        // beyond the 12 + 12 + 4 real ones (outer box, pocket rim, walls
        // and floor).
        assert_eq!(m.sharp_edges.len(), 12 + 4 + 4 + 4);
        let c = box_with_cavity(
            Point3::ORIGIN,
            Point3::new(10.0, 10.0, 10.0),
            Point3::new(2.0, 2.0, 2.0),
            Point3::new(8.0, 8.0, 8.0),
        );
        assert!(close(volume(&c), 1000.0 - 216.0, 1e-9));
    }

    #[test]
    fn matrix_inverse_round_trips() {
        let mut cam = Camera3d::new(NamedView::Isometric, Projection::Perspective);
        cam.distance = 300.0;
        for proj in [Projection::Perspective, Projection::Orthographic] {
            cam.projection = proj;
            let m = mat_mul(&cam.projection_matrix(1.5, 10.0, 900.0), &cam.view_matrix());
            let inv = mat_inverse(&m).unwrap();
            let id = mat_mul(&m, &inv);
            for (c, col) in id.iter().enumerate() {
                for (r, v) in col.iter().enumerate() {
                    let want = if r == c { 1.0 } else { 0.0 };
                    assert!(close(*v, want, 1e-9), "{proj:?} [{c}][{r}] = {v}");
                }
            }
        }
        assert!(mat_inverse(&[[0.0; 4]; 4]).is_none());
    }

    /// The outline keeps the default ink on the light default background,
    /// turns light on the dark viewer background, and contrasts at least
    /// 3:1 with every grey background.
    #[test]
    fn outline_contrasts_with_any_background() {
        let e = EdgeStyle::default();
        let light = e.outline(LIGHT_BACKGROUND).expect("outline on light");
        assert_eq!(light.color, e.color);
        let dark = e.outline(VIEWER_BACKGROUND).expect("outline on dark");
        assert_eq!(dark.color, LIGHT_OUTLINE);
        assert!(close(dark.width_px, e.width_px + 1.0, 1e-12));
        for v in (0..=255).step_by(5) {
            let bg = [v as u8; 3];
            let c = contrast_ratio(e.outline(bg).unwrap().color, bg);
            assert!(c >= MIN_OUTLINE_CONTRAST, "grey {v}: contrast {c:.2}");
        }
        let off = EdgeStyle {
            outline: false,
            ..e
        };
        assert!(off.outline(VIEWER_BACKGROUND).is_none());
        // The frame carries it.
        let mut s = Scene3d {
            background: Background::VIEWER,
            ..Scene3d::default()
        };
        s.add_mesh(
            &cuboid(Point3::ORIGIN, Point3::new(1.0, 1.0, 1.0)),
            Placement::IDENTITY,
            Material::default(),
        );
        assert_eq!(build_frame(&s, 10, 10).outline, Some(dark));
    }

    /// Neighbouring palette colours differ by at least 0.12 in OKLab (six
    /// times a just-noticeable difference), any two of the first 16 by at
    /// least 0.05, and every colour stays muted: its chroma is the
    /// palette's (no gamut clipping) and its lightness one of the three
    /// steps.
    #[test]
    fn palette_neighbours_are_distinct_and_calm() {
        let p = palette(64);
        let lab: Vec<[f64; 3]> = p.iter().map(|&c| oklab_from_srgb(c)).collect();
        let dist = |a: [f64; 3], b: [f64; 3]| {
            ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)).sqrt()
        };
        let neighbour = lab
            .windows(2)
            .map(|w| dist(w[0], w[1]))
            .fold(f64::INFINITY, f64::min);
        let mut any: f64 = f64::INFINITY;
        for i in 0..16 {
            for j in i + 1..16 {
                any = any.min(dist(lab[i], lab[j]));
            }
        }
        eprintln!(
            "palette: least neighbour difference {neighbour:.3}, least of the first 16 {any:.3}"
        );
        assert!(neighbour >= 0.12);
        assert!(any >= 0.05);
        for (i, l) in lab.iter().enumerate() {
            let chroma = l[1].hypot(l[2]);
            assert!(close(chroma, PALETTE_CHROMA, 0.006), "{i}: chroma {chroma}");
            assert!(
                close(l[0], PALETTE_LIGHTNESS[i % 3], 0.006),
                "{i}: L {}",
                l[0]
            );
        }
        assert_eq!(palette_color(5), p[5]);
    }

    #[test]
    fn explode_moves_items_away_from_the_centre() {
        let b = |x0: f64, x1: f64, z0: f64, z1: f64| {
            Some(Aabb3::new(
                Point3::new(x0, 0.0, z0),
                Point3::new(x1, 1.0, z1),
            ))
        };
        // Union box x 0..10, z 0..4: centre (5, 0.5, 2).
        let boxes = [b(0.0, 2.0, 0.0, 1.0), b(8.0, 10.0, 3.0, 4.0), None];
        let d = explode_offsets(&boxes, 0.5, [true, false, true]);
        assert!(vclose(d[0], Vec3::new(-2.0, 0.0, -0.75), 1e-12));
        assert!(vclose(d[1], Vec3::new(2.0, 0.0, 0.75), 1e-12));
        assert!(vclose(d[2], Vec3::new(0.0, 0.0, 0.0), 0.0));
        // On a scene: the translations move by the offsets, and a centred
        // item stays.
        let mut s = Scene3d::default();
        let unit = cuboid(Point3::ORIGIN, Point3::new(1.0, 1.0, 1.0));
        for x in [0.0, 4.5, 9.0] {
            s.add_mesh(
                &unit,
                Placement::translation(x, 0.0, 0.0),
                Material::default(),
            );
        }
        s.explode(1.0, [true, true, true]);
        let xs: Vec<f64> = s.items.iter().map(|i| i.placement.translation.x).collect();
        assert_eq!(xs, vec![-4.5, 4.5, 13.5]);
        assert!(explode_offsets(&[None], 1.0, [true; 3])[0].length() == 0.0);
    }

    #[test]
    fn frame_resolves_ground_occlusion_and_clip_planes() {
        let mut s = Scene3d::default();
        s.add_mesh(
            &cuboid(Point3::new(0.0, 0.0, 5.0), Point3::new(30.0, 40.0, 17.0)),
            Placement::IDENTITY,
            Material::default(),
        );
        s.ground = Some(Ground::default());
        s.clip_planes.push(ClipPlane {
            point: Point3::new(15.0, 0.0, 0.0),
            normal: Vec3::new(2.0, 0.0, 0.0),
        });
        s.fit_camera(1.5);
        let f = build_frame(&s, 300, 200);
        let g = f.ground.unwrap();
        assert_eq!(g.z, 5.0, "ground at the lowest point");
        assert_eq!(g.center, [15.0, 20.0]);
        assert_eq!(g.color, f.background, "shadow catcher by default");
        // The ground square lies inside the clip range.
        for c in g.corners() {
            let (_, _, z) = s.camera.project(c, 300, 200, f.near_far).unwrap();
            assert!((0.0..=1.0).contains(&z), "ground corner depth {z}");
        }
        let o = f.occlusion.unwrap();
        let diag = (30f64 * 30.0 + 40.0 * 40.0 + 12.0 * 12.0).sqrt();
        assert!(close(o.radius, AO_RADIUS_FRACTION * diag, 1e-12));
        assert_eq!(f.clip_planes, vec![[1.0, 0.0, 0.0, -15.0]]);
        // Section colour: derived, or as given.
        assert_eq!(
            f.items[0].section_color,
            default_section_color(&Material::default())
        );
        s.items[0].section = Some([255, 0, 0]);
        s.ground = Some(Ground {
            height: Some(-3.0),
            color: Some([0, 0, 0]),
        });
        s.occlusion = None;
        let f = build_frame(&s, 300, 200);
        assert_eq!(f.items[0].section_color, [1.0, 0.0, 0.0]);
        assert_eq!(f.ground.unwrap().z, -3.0);
        assert!(f.occlusion.is_none());
        // Faded colour: the ground colour inside, the background outside.
        let g = f.ground.unwrap();
        assert_eq!(g.color_at(15.0, 20.0, f.background), [0.0; 3]);
        assert_eq!(
            g.color_at(15.0 + g.fade.1, 20.0, f.background),
            f.background
        );
    }

    #[test]
    fn occlusion_kernel_and_rotations() {
        let k = ao_kernel();
        let min_z = 10f64.to_radians().sin();
        for (i, p) in k.iter().enumerate() {
            let len = p.length();
            assert!(
                (0.2 - 1e-12..=1.0 + 1e-12).contains(&len),
                "{i}: length {len}"
            );
            assert!(p.z / len >= min_z - 1e-12, "{i}: elevation");
        }
        // Every aligned 4 x 4 block holds each of the 16 angles once.
        for (bx, by) in [(0, 0), (4, 8), (12, 4)] {
            let mut seen: Vec<i64> = (0..16)
                .map(|j| {
                    let (c, s) = ao_rotation(bx + j % 4, by + j / 4);
                    (s.atan2(c).rem_euclid(std::f64::consts::TAU) / (PI / 8.0)).round() as i64 % 16
                })
                .collect();
            seen.sort();
            assert_eq!(seen, (0..16).collect::<Vec<_>>());
        }
        let a = apply_occlusion([0.5, 0.4, 1.0], 0.5, 0.8);
        assert!(close(a[0], 0.3, 1e-12) && close(a[1], 0.24, 1e-12) && close(a[2], 0.6, 1e-12));
        assert_eq!(apply_occlusion([0.5; 3], 0.0, 1.0), [0.5; 3]);
    }

    #[test]
    fn frame_resolves_camera_light_into_world() {
        let mut scene = Scene3d::default();
        scene.add_mesh(
            &cuboid(Point3::ORIGIN, Point3::new(1.0, 1.0, 1.0)),
            Placement::translation(10.0, 0.0, 0.0),
            Material::default(),
        );
        let b = scene.bounds().unwrap();
        assert!(b.min.max_abs_diff(Point3::new(10.0, 0.0, 0.0)) < 1e-12);
        scene.camera.set_view(NamedView::Front);
        scene.lighting.key_direction = Vec3::new(0.0, 0.0, 1.0);
        assert!(scene.fit_camera(1.5));
        let f = build_frame(&scene, 300, 200);
        // A camera-frame light along +Z points at the viewer: -Y in world.
        assert!(vclose(f.key_direction, -Vec3::Y, 1e-12));
        assert_eq!(f.eye[3], 0.0);
        assert_eq!(f.items.len(), 1);
        // The front face gets the full key light.
        let c = f.expected_srgb(Point3::new(10.5, 0.0, 0.5), -Vec3::Y, &Material::default());
        let lin = tone_map(shade(
            -Vec3::Y,
            -Vec3::Y,
            -Vec3::Y,
            f.fill_direction,
            &Material::default(),
            &scene.lighting,
        ));
        assert_eq!(c, lin.map(encode_srgb));
    }

    /// The dome's horizontal irradiance closed form against a numeric
    /// integral, and the sun's radiance normalisation: a surface facing a
    /// sun of any size receives its stated irradiance.
    #[test]
    fn environment_irradiance_closed_forms() {
        let env = Environment::studio();
        let e = env.dome_horizontal_irradiance();
        // Midpoint rule over mu = cos(theta): E = 2 pi int L(mu) mu dmu.
        let n = 200_000;
        let mut num = [0.0; 3];
        for k in 0..n {
            let mu = (k as f64 + 0.5) / n as f64;
            let l = env.radiance(Vec3::new((1.0 - mu * mu).sqrt(), 0.0, mu));
            for c in 0..3 {
                num[c] += std::f64::consts::TAU * l[c] * mu / n as f64;
            }
        }
        for c in 0..3 {
            assert!(close(num[c], e[c], 1e-6), "{num:?} vs {e:?}");
        }
        for angle in [0.53, 10.0, 40.0] {
            let mut scene = Scene3d::default();
            scene.environment.sun = Some(Sun {
                direction: Vec3::Z,
                frame: LightFrame::World,
                angle_deg: angle,
                irradiance: 2.0,
                color: [1.0, 0.5, 0.25],
            });
            let f = build_frame(&scene, 10, 10);
            let s = f.environment.sun.unwrap();
            assert!(vclose(s.direction, Vec3::Z, 1e-12));
            // Irradiance of a uniform cap about the normal: L pi sin^2 r.
            let sin2 = 1.0 - s.cos_radius * s.cos_radius;
            for (c, want) in [2.0, 1.0, 0.5].into_iter().enumerate() {
                assert!(close(s.radiance[c] * PI * sin2, want, 1e-9));
            }
            assert!(f.environment.radiance(Vec3::Z)[0] > s.radiance[0]);
        }
        assert_eq!(exposure_scale(1.0), 2.0);
        assert_eq!(exposure_scale(-2.0), 0.25);
        assert_eq!(exposure_scale(f64::NAN), 1.0);
    }

    #[test]
    fn noise_and_patterns_are_deterministic_and_bounded() {
        assert_eq!(pcg_hash(0), 129_708_002);
        let mut lo: f64 = 1.0;
        let mut hi: f64 = 0.0;
        for i in 0..2000 {
            let p = [i as f64 * 0.37 - 300.0, i as f64 * 0.11, -(i as f64) * 0.05];
            let v = value_noise(p, 7);
            assert_eq!(v, value_noise(p, 7));
            lo = lo.min(v);
            hi = hi.max(v);
        }
        assert!(lo >= 0.0 && hi < 1.0 && hi - lo > 0.5, "{lo} {hi}");
        // At lattice points the noise is the lattice value itself.
        assert_eq!(value_noise([3.0, -2.0, 5.0], 1), lattice(3, -2, 5, 1));

        // A board 1000 x 200 x 20: its longest axis is X, shortest Z.
        let board = RenderMesh::from_trimesh(
            &cuboid(Point3::ORIGIN, Point3::new(1000.0, 200.0, 20.0)),
            DEFAULT_CREASE_DEG,
        );
        let axes = principal_axes(&board).axes;
        assert!(vclose(axes[0], Vec3::X, 1e-9), "{axes:?}");
        assert!(vclose(axes[2], Vec3::Z, 1e-9), "{axes:?}");
        let bands = Bands {
            axis: PatternAxis::Longest,
            spacing: 6.0,
            color: [0.2, 0.1, 0.05],
            strength: 0.8,
            warp: 2.0,
            distance: 100.0,
        };
        let m = Material {
            pattern: Pattern::Bands(bands),
            ..Material::from_srgb8(180, 140, 90)
        };
        let pf = pattern_frame(&m, &board, 3);
        assert!(vclose(pf.axis, Vec3::X, 1e-9));
        // The axis sits 100 mm off the centre through the thickness.
        assert!(close((pf.origin.z - 10.0).abs(), 100.0, 1e-9));
        assert_ne!(pf.seed, pattern_frame(&m, &board, 4).seed);
        let mut darkest: f64 = 1.0;
        for i in 0..500 {
            let p = Point3::new(i as f64 * 1.7, 100.0 + (i % 37) as f64, 20.0);
            let w = bands_weight(&bands, &pf, p);
            assert!((0.0..1.0).contains(&w));
            let a = pattern_albedo(&m, &pf, p);
            for c in 0..3 {
                assert!(a[c] >= 0.0 && a[c] <= m.base_color[c] + 1e-12);
            }
            darkest = darkest.min(a[0] / m.base_color[0]);
        }
        assert!(darkest < 0.6, "rings show: {darkest}");
        // Brushed: smoother along the streaks.
        let steel = Material {
            metallic: 1.0,
            roughness: 0.4,
            pattern: Pattern::Brushed(Brushed {
                axis: PatternAxis::Vector(Vec3::Y),
                anisotropy: 1.0,
                streaks: 0.0,
                scale: 0.1,
                circular: false,
            }),
            ..Material::default()
        };
        let pf = pattern_frame(&steel, &board, 0);
        assert!(vclose(pf.axis, Vec3::Y, 1e-12));
        let (along, across) = pattern_roughness(&steel, &pf, Point3::new(1.0, 2.0, 3.0));
        assert!(close(across, 0.4, 1e-12) && close(along, 0.08, 1e-12));
        assert_eq!(
            pattern_albedo(&Material::default(), &pf, Point3::ORIGIN),
            Material::default().base_color
        );
        assert!(close(Material::default().dielectric_f0(), 0.04, 1e-12));

        // Turned parts: a disc and a rod both turn about Z, whichever
        // spread is the odd one out.
        for (r, h) in [(50.0, 10.0), (5.0, 100.0)] {
            let m = RenderMesh::from_trimesh(&cylinder(r, h, 64), DEFAULT_CREASE_DEG);
            let pa = principal_axes(&m);
            assert!(vclose(pa.symmetry_axis(), Vec3::Z, 1e-9), "{pa:?}");
            assert!(pa.centroid.max_abs_diff(Point3::new(0.0, 0.0, h / 2.0)) < 1e-9);
            let turned = Material {
                metallic: 1.0,
                pattern: Pattern::Brushed(Brushed {
                    axis: PatternAxis::Symmetry,
                    anisotropy: 0.5,
                    streaks: 1.0,
                    scale: 0.2,
                    circular: true,
                }),
                ..Material::default()
            };
            let pf = pattern_frame(&turned, &m, 0);
            // Streaks run around the axis: the tangent at +X is +-Y.
            let d = streak_direction(&turned, &pf, Point3::new(r, 0.0, h / 2.0));
            assert!(close(d.y.abs(), 1.0, 1e-9), "{d:?}");
            // Circles: the albedo is the same all the way round.
            let at =
                |a: f64| pattern_albedo(&turned, &pf, Point3::new(r * a.cos(), r * a.sin(), 1.0));
            assert!(close(at(0.3)[0], at(2.0)[0], 1e-12));
        }
    }
}
