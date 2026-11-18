//! Plain floating-point geometry on a kernel's [`TriMesh`]: the harness's
//! own, never the kernel's. These describe what the kernel claims; the
//! certified side of every comparison is the reference.

use std::collections::HashMap;

use cadview_core::solid::{Point3, TriMesh};

pub type V3 = [f64; 3];

pub fn v(p: Point3) -> V3 {
    [p.x, p.y, p.z]
}
pub fn sub(a: V3, b: V3) -> V3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
pub fn add(a: V3, b: V3) -> V3 {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}
pub fn scale(a: V3, s: f64) -> V3 {
    [a[0] * s, a[1] * s, a[2] * s]
}
pub fn dot(a: V3, b: V3) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
pub fn cross(a: V3, b: V3) -> V3 {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}
pub fn norm(a: V3) -> f64 {
    dot(a, a).sqrt()
}

/// The three corners of triangle `i`.
pub fn corners(m: &TriMesh, i: usize) -> [V3; 3] {
    m.triangles[i].map(|k| v(m.positions[k as usize]))
}

/// Twice the vector area of a triangle (outward for a counter-clockwise
/// triangle seen from outside).
pub fn area_vector2(t: [V3; 3]) -> V3 {
    cross(sub(t[1], t[0]), sub(t[2], t[0]))
}

/// Summary topology of a mesh.
#[derive(Debug, Clone, PartialEq)]
pub struct Topology {
    /// Every directed edge occurs once and its opposite once, and no
    /// triangle repeats an index: a closed, consistently oriented,
    /// edge-manifold surface.
    pub closed: bool,
    /// Directed edges used more than once, or without their opposite.
    pub bad_edges: usize,
    /// Degenerate triangles (a repeated index).
    pub degenerate: usize,
    /// Vertices used by triangles.
    pub vertices: usize,
    /// Undirected edges.
    pub edges: usize,
    pub triangles: usize,
    /// Connected components of triangles (through shared edges).
    pub shells: usize,
}

impl Topology {
    pub fn euler(&self) -> i64 {
        self.vertices as i64 - self.edges as i64 + self.triangles as i64
    }
}

pub fn topology(m: &TriMesh) -> Topology {
    let mut directed: HashMap<(u32, u32), u32> = HashMap::new();
    let mut degenerate = 0;
    let mut used = vec![false; m.positions.len()];
    for t in &m.triangles {
        if t[0] == t[1] || t[1] == t[2] || t[2] == t[0] {
            degenerate += 1;
        }
        for k in 0..3 {
            let (a, b) = (t[k], t[(k + 1) % 3]);
            *directed.entry((a, b)).or_insert(0) += 1;
            used[a as usize] = true;
        }
    }
    let mut bad_edges = 0;
    let mut undirected = 0;
    for (&(a, b), &n) in &directed {
        let back = directed.get(&(b, a)).copied().unwrap_or(0);
        if n != 1 || back != 1 {
            bad_edges += 1;
        }
        if a < b || back == 0 {
            undirected += 1;
        }
    }
    // Shells: union-find over triangles sharing an undirected edge.
    let nt = m.triangles.len();
    let mut parent: Vec<usize> = (0..nt).collect();
    fn find(p: &mut [usize], mut x: usize) -> usize {
        while p[x] != x {
            p[x] = p[p[x]];
            x = p[x];
        }
        x
    }
    let mut owner: HashMap<(u32, u32), usize> = HashMap::new();
    for (i, t) in m.triangles.iter().enumerate() {
        for k in 0..3 {
            let (a, b) = (t[k], t[(k + 1) % 3]);
            let key = (a.min(b), a.max(b));
            if let Some(&j) = owner.get(&key) {
                let (ri, rj) = (find(&mut parent, i), find(&mut parent, j));
                parent[ri] = rj;
            } else {
                owner.insert(key, i);
            }
        }
    }
    let shells = (0..nt).filter(|&i| find(&mut parent, i) == i).count();
    Topology {
        closed: nt > 0 && bad_edges == 0 && degenerate == 0,
        bad_edges,
        degenerate,
        vertices: used.iter().filter(|u| **u).count(),
        edges: undirected,
        triangles: nt,
        shells,
    }
}

/// The total vector area and the total (scalar) area.
pub fn vector_area(m: &TriMesh) -> (V3, f64) {
    let mut s = [0.0; 3];
    let mut a = 0.0;
    for i in 0..m.triangles.len() {
        let w = area_vector2(corners(m, i));
        s = add(s, scale(w, 0.5));
        a += 0.5 * norm(w);
    }
    (s, a)
}

/// The enclosed volume by the divergence theorem, `sum a . (b x c) / 6`,
/// taken about the centre of the positions' box (the sum is independent
/// of the centre for a closed mesh, and a nearby centre loses less to
/// cancellation).
pub fn signed_volume(m: &TriMesh) -> f64 {
    let c = centre(m);
    let mut s = 0.0;
    for i in 0..m.triangles.len() {
        let [a, b, d] = corners(m, i).map(|p| sub(p, c));
        s += dot(a, cross(b, d));
    }
    s / 6.0
}

fn centre(m: &TriMesh) -> V3 {
    let (lo, hi) = aabb(m);
    [0, 1, 2].map(|i| 0.5 * (lo[i] + hi[i]))
}

pub fn aabb(m: &TriMesh) -> (V3, V3) {
    let mut lo = [f64::INFINITY; 3];
    let mut hi = [f64::NEG_INFINITY; 3];
    for p in &m.positions {
        let p = v(*p);
        for i in 0..3 {
            lo[i] = lo[i].min(p[i]);
            hi[i] = hi[i].max(p[i]);
        }
    }
    (lo, hi)
}

/// The distance from `p` to triangle `t` (Ericson, Real-Time Collision
/// Detection, 5.1.5).
pub fn point_triangle_distance(p: V3, t: [V3; 3]) -> f64 {
    let [a, b, c] = t;
    let ab = sub(b, a);
    let ac = sub(c, a);
    let ap = sub(p, a);
    let d1 = dot(ab, ap);
    let d2 = dot(ac, ap);
    if d1 <= 0.0 && d2 <= 0.0 {
        return norm(ap);
    }
    let bp = sub(p, b);
    let d3 = dot(ab, bp);
    let d4 = dot(ac, bp);
    if d3 >= 0.0 && d4 <= d3 {
        return norm(bp);
    }
    let vc = d1 * d4 - d3 * d2;
    if vc <= 0.0 && d1 >= 0.0 && d3 <= 0.0 {
        let w = d1 / (d1 - d3);
        return norm(sub(p, add(a, scale(ab, w))));
    }
    let cp = sub(p, c);
    let d5 = dot(ab, cp);
    let d6 = dot(ac, cp);
    if d6 >= 0.0 && d5 <= d6 {
        return norm(cp);
    }
    let vb = d5 * d2 - d1 * d6;
    if vb <= 0.0 && d2 >= 0.0 && d6 <= 0.0 {
        let w = d2 / (d2 - d6);
        return norm(sub(p, add(a, scale(ac, w))));
    }
    let va = d3 * d6 - d5 * d4;
    if va <= 0.0 && (d4 - d3) >= 0.0 && (d5 - d6) >= 0.0 {
        let w = (d4 - d3) / ((d4 - d3) + (d5 - d6));
        return norm(sub(p, add(b, scale(sub(c, b), w))));
    }
    let n = cross(ab, ac);
    let nn = norm(n);
    if nn == 0.0 {
        // Degenerate: the nearest of the three edges.
        let seg = |a: V3, b: V3| {
            let d = sub(b, a);
            let l2 = dot(d, d);
            let t = if l2 > 0.0 {
                (dot(sub(p, a), d) / l2).clamp(0.0, 1.0)
            } else {
                0.0
            };
            norm(sub(p, add(a, scale(d, t))))
        };
        return seg(a, b).min(seg(b, c)).min(seg(c, a));
    }
    (dot(ap, n) / nn).abs()
}

/// A uniform grid over a mesh's triangles, for "is any triangle within
/// `r` of `p`" queries.
pub struct Grid {
    lo: V3,
    cell: f64,
    dims: [usize; 3],
    cells: HashMap<[usize; 3], Vec<u32>>,
    tris: Vec<[V3; 3]>,
}

impl Grid {
    pub fn new(m: &TriMesh) -> Grid {
        let (lo, hi) = aabb(m);
        let ext = [0, 1, 2].map(|i| (hi[i] - lo[i]).max(1e-9));
        let n = m.triangles.len().max(1) as f64;
        // About n cells over the box's volume, at least 1e-3 of the
        // largest extent, so a huge triangle does not fill millions.
        let vol = ext[0] * ext[1] * ext[2];
        let big = ext.iter().cloned().fold(0.0, f64::max);
        let cell = (vol / n).cbrt().max(big / 256.0).max(1e-9);
        let dims = ext.map(|e| ((e / cell).ceil() as usize).max(1));
        let tris: Vec<[V3; 3]> = (0..m.triangles.len()).map(|i| corners(m, i)).collect();
        let mut cells: HashMap<[usize; 3], Vec<u32>> = HashMap::new();
        for (i, t) in tris.iter().enumerate() {
            let mut tlo = [f64::INFINITY; 3];
            let mut thi = [f64::NEG_INFINITY; 3];
            for p in t {
                for k in 0..3 {
                    tlo[k] = tlo[k].min(p[k]);
                    thi[k] = thi[k].max(p[k]);
                }
            }
            let a = Self::index_in(lo, cell, dims, tlo);
            let b = Self::index_in(lo, cell, dims, thi);
            for x in a[0]..=b[0] {
                for y in a[1]..=b[1] {
                    for z in a[2]..=b[2] {
                        cells.entry([x, y, z]).or_default().push(i as u32);
                    }
                }
            }
        }
        Grid {
            lo,
            cell,
            dims,
            cells,
            tris,
        }
    }

    fn index_in(lo: V3, cell: f64, dims: [usize; 3], p: V3) -> [usize; 3] {
        [0, 1, 2].map(|k| {
            let f = ((p[k] - lo[k]) / cell).floor();
            (f.max(0.0) as usize).min(dims[k] - 1)
        })
    }

    /// The distance from `p` to the nearest triangle, capped at `r`: any
    /// value below `r` is exact, `r` means "at least `r`".
    pub fn distance_capped(&self, p: V3, r: f64) -> f64 {
        let a = Self::index_in(self.lo, self.cell, self.dims, sub(p, [r; 3]));
        let b = Self::index_in(self.lo, self.cell, self.dims, add(p, [r; 3]));
        let mut best = r;
        // Points outside the grid's box clamp to its border cells, which
        // still hold every triangle that could be within r.
        let mut seen = std::collections::HashSet::new();
        for x in a[0]..=b[0] {
            for y in a[1]..=b[1] {
                for z in a[2]..=b[2] {
                    if let Some(list) = self.cells.get(&[x, y, z]) {
                        for &i in list {
                            if seen.insert(i) {
                                best = best.min(point_triangle_distance(p, self.tris[i as usize]));
                            }
                        }
                    }
                }
            }
        }
        best
    }
}

/// The generalised winding number of a closed mesh about `p` (the sum of
/// the triangles' signed solid angles over 4 pi; Van Oosterom and
/// Strackee). An integer for a closed mesh and a point off the surface:
/// 1 inside, 0 outside.
pub fn winding(m: &TriMesh, p: V3) -> f64 {
    let mut total = 0.0;
    for i in 0..m.triangles.len() {
        let [a, b, c] = corners(m, i).map(|q| sub(q, p));
        let (la, lb, lc) = (norm(a), norm(b), norm(c));
        let num = dot(a, cross(b, c));
        let den = la * lb * lc + dot(a, b) * lc + dot(a, c) * lb + dot(b, c) * la;
        total += 2.0 * num.atan2(den);
    }
    total / (4.0 * std::f64::consts::PI)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A unit cube with outward triangles.
    pub fn cube() -> TriMesh {
        let positions = (0..8)
            .map(|i| Point3::new((i & 1) as f64, ((i >> 1) & 1) as f64, ((i >> 2) & 1) as f64))
            .collect();
        let quads = [
            [0, 2, 3, 1], // z = 0, normal -z
            [4, 5, 7, 6], // z = 1
            [0, 1, 5, 4], // y = 0
            [2, 6, 7, 3], // y = 1
            [0, 4, 6, 2], // x = 0
            [1, 3, 7, 5], // x = 1
        ];
        let mut triangles = Vec::new();
        for q in quads {
            triangles.push([q[0], q[1], q[2]]);
            triangles.push([q[0], q[2], q[3]]);
        }
        TriMesh {
            positions,
            triangles,
        }
    }

    #[test]
    fn cube_invariants() {
        let m = cube();
        let t = topology(&m);
        assert!(t.closed, "{t:?}");
        assert_eq!(t.euler(), 2);
        assert_eq!(t.shells, 1);
        assert!((signed_volume(&m) - 1.0).abs() < 1e-15);
        let (va, a) = vector_area(&m);
        assert!(norm(va) < 1e-15 && (a - 6.0).abs() < 1e-15);
        assert!((winding(&m, [0.5, 0.5, 0.5]) - 1.0).abs() < 1e-12);
        assert!(winding(&m, [1.5, 0.5, 0.5]).abs() < 1e-12);
        let g = Grid::new(&m);
        assert!((g.distance_capped([0.5, 0.5, 1.25], 1.0) - 0.25).abs() < 1e-15);
        assert_eq!(g.distance_capped([5.0, 5.0, 5.0], 1.0), 1.0);
        // A flipped triangle opens the surface.
        let mut f = m.clone();
        f.triangles[0].swap(1, 2);
        assert!(!topology(&f).closed);
    }
}
