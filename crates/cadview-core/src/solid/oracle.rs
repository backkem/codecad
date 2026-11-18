//! The visibility oracle of hidden-line removal (docs/3d-design.md 7.3).
//!
//! "Is this point on the exact curve hidden from the eye?" A ray from the
//! point toward the eye is cast against a fine triangle mesh of every
//! placed part (a bounding volume hierarchy, double precision). The mesh
//! only ever answers that question: nothing tessellated is stored, synced
//! or drawn.
//!
//! The mesh lies within its chord of the exact faces, so its answer is
//! taken only when it is clear: a hit well inside a triangle, not at a
//! grazing angle, and farther than [`NEAR_CHORDS`] chords from the point
//! (closer hits can be the point's own faces, off by the chord) means
//! hidden; no hit at all, even with the triangles grown by a margin, means
//! visible. Anything else asks the kernel for exact hits on the faces of
//! the parts whose box the ray crosses ([`super::eval::Solid::ray_hits`]):
//! hidden when some hit is farther than [`EXACT_EPS`] and not a graze. A
//! ray that runs inside a face (a face seen edge on) does not count as
//! hidden by that part. Without the kernel capability the mesh's best
//! guess stands, and the counts say how often that happened.

use std::sync::Arc;

use super::eval::Solid;
use super::kernel::{Aabb3, Ray3, TriMesh};
use super::part::{apply_point, apply_vec, inverse};
use super::types::{Placement, Point3, Vec3};

/// Mesh hits nearer than this many chords are left to the exact oracle.
pub const NEAR_CHORDS: f64 = 50.0;

/// An exact hit nearer than this (mm) is the point itself.
pub const EXACT_EPS: f64 = 1e-6;

/// A mesh hit within this many chords of a triangle's side is left to the
/// exact oracle (the mesh's outline can be off the exact one by a chord).
pub const MARGIN_CHORDS: f64 = 2.0;

/// How many chords across the side rays of an unsure point are moved.
pub const SIDE_CHORDS: f64 = 8.0;

/// Relative margin for triangles much larger than the chord.
const BARY: f64 = 1e-9;

/// Below this sine between the ray and a triangle's plane a hit is a graze.
const GRAZE: f64 = 1e-3;

/// One placed part the oracle knows.
pub struct OraclePart {
    pub solid: Arc<dyn Solid>,
    pub placement: Placement,
    /// World box (exact or tight).
    pub bbox: Aabb3,
    /// The part's mesh at the oracle's chord, or None when the kernel
    /// refused to tessellate it (then only the exact oracle sees it).
    pub mesh: Option<Arc<TriMesh>>,
}

/// What the mesh says about one point.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MeshAnswer {
    Visible,
    Hidden,
    Unsure,
}

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct OracleStats {
    pub mesh: usize,
    pub exact: usize,
    /// Exact rays that ran inside a face.
    pub in_face: usize,
    /// Unsure points the kernel could not answer (mesh guess used).
    pub guessed: usize,
    /// Unsure points the mesh decided with four rays beside them.
    pub beside: usize,
    pub triangles: usize,
}

struct Node {
    lo: [f64; 3],
    hi: [f64; 3],
    /// Leaf: first triangle and count; inner: `count == 0`, children at
    /// `first` and `first + 1`.
    first: u32,
    count: u32,
}

pub struct MeshOracle {
    back: Vec3,
    tris: Vec<[Point3; 3]>,
    /// Per triangle, whether each side (v0 v1, v1 v2, v2 v0) lies on the
    /// mesh's outline in this view: an open side, or one between a triangle
    /// facing the eye and one facing away.
    contour: Vec<[bool; 3]>,
    nodes: Vec<Node>,
    parts: Vec<OraclePart>,
    inverse: Vec<Placement>,
    near: f64,
    margin: f64,
    exact: bool,
    pub stats: OracleStats,
}

fn box_of(t: &[Point3; 3]) -> ([f64; 3], [f64; 3]) {
    let mut lo = [f64::INFINITY; 3];
    let mut hi = [f64::NEG_INFINITY; 3];
    for p in t {
        for (i, v) in [p.x, p.y, p.z].into_iter().enumerate() {
            lo[i] = lo[i].min(v);
            hi[i] = hi[i].max(v);
        }
    }
    (lo, hi)
}

impl MeshOracle {
    /// An oracle for a view whose eye lies along the unit `back`, over the
    /// placed parts with their meshes at `chord`. `exact`: ask the kernel
    /// when the mesh is unsure.
    pub fn new(back: Vec3, parts: Vec<OraclePart>, chord: f64, exact: bool) -> MeshOracle {
        let mut tris = Vec::new();
        let mut contour = Vec::new();
        for p in &parts {
            if let Some(m) = &p.mesh {
                let start = tris.len();
                for t in &m.triangles {
                    let q = |i: u32| apply_point(&p.placement, m.positions[i as usize]);
                    tris.push([q(t[0]), q(t[1]), q(t[2])]);
                }
                contour.extend(contour_sides(m, &tris[start..], back));
            }
        }
        let inverse = parts.iter().map(|p| inverse(&p.placement)).collect();
        let mut o = MeshOracle {
            back,
            tris,
            contour,
            nodes: Vec::new(),
            parts,
            inverse,
            near: NEAR_CHORDS * chord,
            margin: MARGIN_CHORDS * chord,
            exact,
            stats: OracleStats::default(),
        };
        o.stats.triangles = o.tris.len();
        o.build();
        o
    }

    fn build(&mut self) {
        if self.tris.is_empty() {
            return;
        }
        let centroids: Vec<[f64; 3]> = self
            .tris
            .iter()
            .map(|t| {
                [
                    (t[0].x + t[1].x + t[2].x) / 3.0,
                    (t[0].y + t[1].y + t[2].y) / 3.0,
                    (t[0].z + t[1].z + t[2].z) / 3.0,
                ]
            })
            .collect();
        let mut idx: Vec<u32> = (0..self.tris.len() as u32).collect();
        self.nodes.push(Node {
            lo: [0.0; 3],
            hi: [0.0; 3],
            first: 0,
            count: 0,
        });
        // (node, start, end) over idx.
        let mut todo = vec![(0usize, 0usize, idx.len())];
        while let Some((node, s, e)) = todo.pop() {
            let mut lo = [f64::INFINITY; 3];
            let mut hi = [f64::NEG_INFINITY; 3];
            for &i in &idx[s..e] {
                let (a, b) = box_of(&self.tris[i as usize]);
                for k in 0..3 {
                    lo[k] = lo[k].min(a[k]);
                    hi[k] = hi[k].max(b[k]);
                }
            }
            self.nodes[node].lo = lo;
            self.nodes[node].hi = hi;
            if e - s <= 4 {
                self.nodes[node].first = s as u32;
                self.nodes[node].count = (e - s) as u32;
                continue;
            }
            let axis = (0..3)
                .max_by(|&a, &b| (hi[a] - lo[a]).total_cmp(&(hi[b] - lo[b])))
                .unwrap_or(0);
            idx[s..e].sort_by(|&a, &b| {
                centroids[a as usize][axis]
                    .total_cmp(&centroids[b as usize][axis])
                    .then(a.cmp(&b))
            });
            let mid = (s + e) / 2;
            let left = self.nodes.len();
            for _ in 0..2 {
                self.nodes.push(Node {
                    lo: [0.0; 3],
                    hi: [0.0; 3],
                    first: 0,
                    count: 0,
                });
            }
            self.nodes[node].first = left as u32;
            self.nodes[node].count = 0;
            todo.push((left, s, mid));
            todo.push((left + 1, mid, e));
        }
        // Reorder the triangles so leaves index them directly.
        let tris = idx.iter().map(|&i| self.tris[i as usize]).collect();
        self.tris = tris;
        let contour = idx.iter().map(|&i| self.contour[i as usize]).collect();
        self.contour = contour;
    }

    /// What the mesh says about `p`.
    pub fn mesh_answer(&mut self, p: Point3) -> MeshAnswer {
        self.stats.mesh += 1;
        if self.tris.is_empty() {
            return MeshAnswer::Visible;
        }
        let o = [p.x, p.y, p.z];
        let d = [self.back.x, self.back.y, self.back.z];
        let inv = d.map(|v| if v == 0.0 { f64::INFINITY } else { 1.0 / v });
        let mut loose = false;
        let mut stack = vec![0usize];
        while let Some(n) = stack.pop() {
            let node = &self.nodes[n];
            // Slab test, t in [0, inf).
            let (mut t0, mut t1) = (0.0f64, f64::INFINITY);
            let mut miss = false;
            for k in 0..3 {
                let pad = self.margin + 1e-9 * (1.0 + node.lo[k].abs().max(node.hi[k].abs()));
                let (lo, hi) = (node.lo[k] - pad, node.hi[k] + pad);
                if d[k] == 0.0 {
                    if o[k] < lo || o[k] > hi {
                        miss = true;
                        break;
                    }
                } else {
                    let (a, b) = ((lo - o[k]) * inv[k], (hi - o[k]) * inv[k]);
                    t0 = t0.max(a.min(b));
                    t1 = t1.min(a.max(b));
                }
            }
            if miss || t0 > t1 {
                continue;
            }
            if node.count == 0 {
                stack.push(node.first as usize);
                stack.push(node.first as usize + 1);
                continue;
            }
            let (lo, hi) = (node.first as usize, (node.first + node.count) as usize);
            for (t, c) in self.tris[lo..hi].iter().zip(&self.contour[lo..hi]) {
                match hit(p, self.back, t, *c, self.margin) {
                    Hit::None => {}
                    Hit::Loose => loose = true,
                    Hit::Clear(tt) => {
                        if tt >= self.near {
                            return MeshAnswer::Hidden;
                        }
                        loose = true;
                    }
                }
            }
        }
        if loose {
            MeshAnswer::Unsure
        } else {
            MeshAnswer::Visible
        }
    }

    /// The exact answer from the kernel, or None when it cannot give one
    /// for some part the ray crosses.
    pub fn exact_answer(&mut self, p: Point3) -> Option<bool> {
        self.stats.exact += 1;
        super::hlr::debug(&format!("exact ray from {p:?}"));
        let mut answered = true;
        for (i, part) in self.parts.iter().enumerate() {
            if !ray_meets_box(p, self.back, &part.bbox) {
                continue;
            }
            let inv = &self.inverse[i];
            let ray = Ray3 {
                origin: apply_point(inv, p),
                dir: apply_vec(inv, self.back),
            };
            match part.solid.ray_hits(&[ray]) {
                Ok(mut r) => match r.pop() {
                    Some(Ok(h)) => {
                        // A ray inside a face, or one whose only hits past its
                        // start are grazes (a graze where a face meets another
                        // at a corner may still cross material), is decided by
                        // the rays beside it.
                        let grazes = !blocks(&h) && h.hits.iter().any(|x| x.t > EXACT_EPS);
                        if h.in_face || grazes {
                            self.stats.in_face += 1;
                            match beside_face(&*part.solid, ray) {
                                Some(true) => return Some(true),
                                Some(false) => continue,
                                None => answered = false,
                            }
                            continue;
                        }
                        if blocks(&h) {
                            return Some(true);
                        }
                    }
                    _ => answered = false,
                },
                Err(_) => answered = false,
            }
        }
        answered.then_some(false)
    }
}

impl super::hlr::Oracle for MeshOracle {
    fn hidden(&mut self, p: Point3, _owner: usize) -> bool {
        // A part without a mesh is seen by the exact oracle only.
        let meshless = self
            .parts
            .iter()
            .any(|q| q.mesh.is_none() && ray_meets_box(p, self.back, &q.bbox));
        let mesh = self.mesh_answer(p);
        if mesh == MeshAnswer::Visible && !meshless {
            return false;
        }
        // Hidden or unsure: four rays moved [`SIDE_CHORDS`] chords across,
        // past the band where the mesh can mislead, decide it the way the
        // exact fallback decides a ray that runs inside a face (visible if
        // any escapes, hidden if all are blocked). That covers a ray that
        // runs along a face seen edge on, or inside a curved face seen end
        // on, or grazes an outline. Only when a side ray is unsure too (and
        // the point's own ray was) is the kernel asked.
        if !meshless {
            let d = self.back;
            let (a, b) = side_axes(d);
            let s = self.margin / MARGIN_CHORDS * SIDE_CHORDS;
            let mut all_hidden = true;
            let mut decided = true;
            'dirs: for o in [a, a.scale(-1.0), b, b.scale(-1.0)] {
                // A side ray that is unsure too gets one more try, four
                // times as far out.
                let mut answer = MeshAnswer::Unsure;
                for k in [1.0, 4.0] {
                    let q = Point3::new(p.x + o.x * s * k, p.y + o.y * s * k, p.z + o.z * s * k);
                    answer = self.mesh_answer(q);
                    if answer != MeshAnswer::Unsure {
                        break;
                    }
                }
                match answer {
                    MeshAnswer::Visible => {
                        all_hidden = false;
                        break 'dirs;
                    }
                    MeshAnswer::Hidden => {}
                    MeshAnswer::Unsure => decided = false,
                }
            }
            if !all_hidden {
                self.stats.beside += 1;
                return false;
            }
            if mesh == MeshAnswer::Hidden {
                return true;
            }
            if decided {
                self.stats.beside += 1;
                return true;
            }
        }
        if self.exact {
            if let Some(h) = self.exact_answer(p) {
                return h;
            }
        }
        self.stats.guessed += 1;
        mesh == MeshAnswer::Hidden
    }
}

/// How far (mm) the side rays of a ray that runs in a face are moved off
/// it.
pub const BESIDE: f64 = 1e-5;

/// Some exact hit beyond the start that crosses the surface.
fn blocks(h: &super::kernel::RayHits) -> bool {
    h.hits.iter().any(|x| x.t > EXACT_EPS && !x.tangent)
}

/// A ray that runs inside one of the solid's faces (a face seen edge on)
/// gets no hits from the kernel. Its answer is taken from four rays moved
/// [`BESIDE`] off it, across the ray in two directions: the point is hidden
/// by this solid only when every one of them that the kernel answers is
/// blocked. So an edge seen along a face that turns away from the eye (the
/// back edge of a box's top in a front view) is visible, and one whose ray
/// then runs into material (the floor edge of a groove seen from below) is
/// hidden. None when no side ray could be answered.
fn beside_face(solid: &dyn Solid, ray: Ray3) -> Option<bool> {
    let d = ray.dir.normalized()?;
    let (a, b) = side_axes(d);
    let rays: Vec<Ray3> = [a, a.scale(-1.0), b, b.scale(-1.0)]
        .iter()
        .map(|o| Ray3 {
            origin: Point3::new(
                ray.origin.x + o.x * BESIDE,
                ray.origin.y + o.y * BESIDE,
                ray.origin.z + o.z * BESIDE,
            ),
            dir: ray.dir,
        })
        .collect();
    let answers = solid.ray_hits(&rays).ok()?;
    let mut seen = false;
    for h in answers.into_iter().flatten() {
        if h.in_face {
            continue;
        }
        seen = true;
        if !blocks(&h) {
            return Some(false);
        }
    }
    seen.then_some(true)
}

enum Hit {
    None,
    /// Near an edge of the triangle, or at a grazing angle.
    Loose,
    Clear(f64),
}

/// Which sides of each triangle (`placed`, the mesh's triangles in world
/// coordinates, in order) lie on the mesh's outline seen along `back`: a
/// side with no neighbour, or whose two triangles face opposite ways (or
/// one of them edge on).
fn contour_sides(m: &TriMesh, placed: &[[Point3; 3]], back: Vec3) -> Vec<[bool; 3]> {
    use std::collections::HashMap;
    let facing: Vec<f64> = placed
        .iter()
        .map(|t| (t[1] - t[0]).cross(t[2] - t[0]).dot(back))
        .collect();
    let key = |a: u32, b: u32| (a.min(b), a.max(b));
    let mut sides: HashMap<(u32, u32), Vec<usize>> = HashMap::new();
    for (i, t) in m.triangles.iter().enumerate() {
        for k in 0..3 {
            sides.entry(key(t[k], t[(k + 1) % 3])).or_default().push(i);
        }
    }
    m.triangles
        .iter()
        .enumerate()
        .map(|(i, t)| {
            let mut c = [true; 3];
            for (k, ck) in c.iter_mut().enumerate() {
                let users = &sides[&key(t[k], t[(k + 1) % 3])];
                if let [a, b] = users[..] {
                    let other = if a == i { b } else { a };
                    let (f, g) = (facing[i], facing[other]);
                    *ck = f * g <= 0.0 || (f * g).is_nan();
                }
            }
            c
        })
        .collect()
}

/// Moller-Trumbore with margins. A ray that passes inside a triangle is a
/// clear hit, unless it passes within `margin` (mm) of a side on the
/// outline (`contour`) or meets the triangle at a grazing angle: then it is
/// loose, and so is one that misses an outline side by less than `margin`.
/// A ray just outside a side shared with a neighbour facing the same way is
/// that neighbour's hit.
fn hit(o: Point3, d: Vec3, t: &[Point3; 3], contour: [bool; 3], margin: f64) -> Hit {
    let e1 = t[1] - t[0];
    let e2 = t[2] - t[0];
    let pv = d.cross(e2);
    let det = e1.dot(pv);
    let twice = e1.cross(e2).length();
    if twice <= 0.0 || det.abs() <= 1e-14 * twice {
        return Hit::None;
    }
    let inv = 1.0 / det;
    let tv = o - t[0];
    let u = tv.dot(pv) * inv;
    let qv = tv.cross(e1);
    let v = d.dot(qv) * inv;
    let tt = e2.dot(qv) * inv;
    if tt <= 1e-9 {
        return Hit::None;
    }
    let w = 1.0 - u - v;
    // Signed distances (mm) of the hit to the three sides.
    let e3 = t[2] - t[1];
    let side = [
        u * twice / e2.length(),
        v * twice / e1.length(),
        w * twice / e3.length(),
    ];
    // Whether each of those sides is on the outline: side[0] is v2 v0,
    // side[1] v0 v1, side[2] v1 v2.
    let outline = [contour[2], contour[0], contour[1]];
    let size = e1.length().max(e2.length());
    let tiny = BARY * size;
    let pad = margin.max(tiny);
    if side
        .iter()
        .zip(outline)
        .any(|(&s, c)| s < -pad || (!c && s < -tiny))
    {
        return Hit::None;
    }
    // det = d . (e1 x e2): |det| / |e1 x e2| is the sine of the ray's
    // angle to the plane (d is unit).
    if side.iter().zip(outline).any(|(&s, c)| c && s < pad) || det.abs() / twice < GRAZE {
        Hit::Loose
    } else {
        Hit::Clear(tt)
    }
}

/// Whether the half-line from `p` along `d` meets the box (grown a little).
pub fn ray_meets_box(p: Point3, d: Vec3, b: &Aabb3) -> bool {
    let o = [p.x, p.y, p.z];
    let dd = [d.x, d.y, d.z];
    let lo = [b.min.x, b.min.y, b.min.z];
    let hi = [b.max.x, b.max.y, b.max.z];
    let (mut t0, mut t1) = (0.0f64, f64::INFINITY);
    for k in 0..3 {
        let pad = 1e-6 * (1.0 + lo[k].abs().max(hi[k].abs()));
        let (l, h) = (lo[k] - pad, hi[k] + pad);
        if dd[k] == 0.0 {
            if o[k] < l || o[k] > h {
                return false;
            }
        } else {
            let (a, c) = ((l - o[k]) / dd[k], (h - o[k]) / dd[k]);
            t0 = t0.max(a.min(c));
            t1 = t1.min(a.max(c));
        }
    }
    t0 <= t1
}

/// Two unit vectors across the unit `d`, turned 0.4 rad off the model's
/// axes so that side rays leave faces that are parallel to an axis plane.
fn side_axes(d: Vec3) -> (Vec3, Vec3) {
    let a0 = super::silhouette::perpendicular(d);
    let b0 = d.cross(a0);
    let (s, c) = 0.4f64.sin_cos();
    (a0.scale(c) + b0.scale(s), b0.scale(c) - a0.scale(s))
}
