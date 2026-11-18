//! Kernel-neutral checks on a [`TriMesh`]: whether it is a closed,
//! consistently oriented surface, and its counts for the Euler
//! characteristic.
//!
//! A kernel adapter uses [`topology`] to refuse a tessellation that is not
//! a closed surface, and the test harness uses the same function for its
//! mesh layer, so both read a mesh the same way.

use std::collections::HashMap;

use super::kernel::TriMesh;

/// Summary topology of a mesh.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MeshTopology {
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

impl MeshTopology {
    /// The Euler characteristic `V - E + F`.
    pub fn euler(&self) -> i64 {
        self.vertices as i64 - self.edges as i64 + self.triangles as i64
    }
}

/// The topology of `m`: directed-edge counts, closedness and shells.
pub fn topology(m: &TriMesh) -> MeshTopology {
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
    MeshTopology {
        closed: nt > 0 && bad_edges == 0 && degenerate == 0,
        bad_edges,
        degenerate,
        vertices: used.iter().filter(|u| **u).count(),
        edges: undirected,
        triangles: nt,
        shells,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::solid::types::Point3;

    /// A unit cube with outward triangles.
    fn cube() -> TriMesh {
        let positions = (0..8)
            .map(|i| Point3::new((i & 1) as f64, ((i >> 1) & 1) as f64, ((i >> 2) & 1) as f64))
            .collect();
        let quads = [
            [0, 2, 3, 1],
            [4, 5, 7, 6],
            [0, 1, 5, 4],
            [2, 6, 7, 3],
            [0, 4, 6, 2],
            [1, 3, 7, 5],
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
    fn cube_is_closed_and_flips_open_it() {
        let m = cube();
        let t = topology(&m);
        assert!(t.closed, "{t:?}");
        assert_eq!((t.euler(), t.shells, t.bad_edges), (2, 1, 0));
        let mut flipped = m.clone();
        flipped.triangles[0].swap(1, 2);
        let t = topology(&flipped);
        assert!(!t.closed && t.bad_edges > 0);
        let mut dropped = m;
        dropped.triangles.pop();
        assert!(!topology(&dropped).closed);
        assert!(!topology(&TriMesh::default()).closed);
    }
}
