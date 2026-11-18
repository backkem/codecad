//! Layer 4: mesh invariants, at every chord of [`CHORDS`].
//!
//! - **Closed and manifold**: every directed edge is used exactly once and
//!   its opposite exactly once, no triangle repeats an index.
//! - **Total vector area zero**: `|sum of (b - a) x (c - a) / 2| <= 1e-10
//!   A`, `A` the total area. For a closed surface the sum is exactly zero;
//!   each term is rounded at about 1e-16 of its size, so 1e-10 of the
//!   total leaves a millionfold margin for meshes of up to 10^4 triangles
//!   and still flags one flipped triangle of more than 1e-10 of the area.
//! - **Euler characteristic** `V - E + F` equal to what the recipe implies
//!   (2 per shell minus 2 per through hole), where the case states it.
//! - **Volume convergence**: the mesh volume by the divergence theorem
//!   within `T(c)` of the kernel's stated volume, where `T(c)` is the
//!   volume of the `c`-neighbourhood of the mesh, bounded triangle by
//!   triangle (Steiner): `sum (2 c a_i + pi c^2 p_i + 4/3 pi c^3)` with
//!   `a_i` and `p_i` each triangle's area and perimeter, plus `1e-9 V` for
//!   rounding. The bound holds because the mesh contract puts the exact
//!   boundary within `c` of the mesh both ways, so the solid and the mesh's
//!   interior differ only inside that neighbourhood. `T(c)` shrinks
//!   linearly with `c`, so passing at every chord is convergence. The
//!   mesh volume must also be positive (outward triangles).

use std::f64::consts::PI;

use crate::mesh::{corners, norm, signed_volume, sub, topology, vector_area};
use crate::outcome::Verdict;
use crate::run::{Fail, PartRun};

pub fn check<B>(run: &PartRun<B>) -> Verdict {
    if let Err(f) = &run.brep {
        return super::closed_form::fail_verdict(f);
    }
    if run.meshes.is_empty() {
        return Verdict::na("no mesh requested");
    }
    let stated = match &run.mass {
        Ok(m) => Some(m.volume),
        Err(_) => None,
    };
    let mut parts = Vec::new();
    for (chord, mesh) in &run.meshes {
        let mesh = match mesh {
            Ok(m) => m,
            Err(Fail::Unsupported(s)) => {
                parts.push(Verdict::na(format!("no tessellation: {s}")));
                continue;
            }
            Err(f) => {
                parts.push(super::closed_form::fail_verdict(f));
                continue;
            }
        };
        if mesh.triangles.is_empty() {
            parts.push(Verdict::na(format!("chord {chord:e}: no triangles")));
            continue;
        }
        let topo = topology(mesh);
        if !topo.closed {
            parts.push(Verdict::wrong(format!(
                "chord {chord:e}: not closed ({} bad directed edges, {} degenerate triangles)",
                topo.bad_edges, topo.degenerate
            )));
            continue;
        }
        let (va, area) = vector_area(mesh);
        if norm(va) > 1e-10 * area {
            parts.push(Verdict::wrong(format!(
                "chord {chord:e}: total vector area {:.1e} of {area:.1e}",
                norm(va)
            )));
        }
        if let Some(chi) = run.case.euler {
            if topo.euler() != chi {
                parts.push(Verdict::wrong(format!(
                    "chord {chord:e}: Euler characteristic {} (V {} E {} F {}), the recipe implies {chi}",
                    topo.euler(),
                    topo.vertices,
                    topo.edges,
                    topo.triangles
                )));
            }
        }
        let v = signed_volume(mesh);
        if v <= 0.0 {
            parts.push(Verdict::wrong(format!(
                "chord {chord:e}: mesh volume {v} is not positive"
            )));
        }
        if let Some(stated) = stated {
            let mut tube = 0.0;
            for i in 0..mesh.triangles.len() {
                let [a, b, c] = corners(mesh, i);
                let ar = 0.5 * norm(crate::mesh::cross(sub(b, a), sub(c, a)));
                let per = norm(sub(b, a)) + norm(sub(c, b)) + norm(sub(a, c));
                tube +=
                    2.0 * chord * ar + PI * chord * chord * per + 4.0 / 3.0 * PI * chord.powi(3);
            }
            let bound = tube + 1e-9 * stated.abs();
            let err = (v - stated).abs();
            if err > bound {
                parts.push(Verdict::wrong(format!(
                    "chord {chord:e}: mesh volume {v} vs stated {stated}: off by {err:.3e}, bound {bound:.3e}"
                )));
            } else {
                parts.push(Verdict::correct(""));
            }
        } else {
            parts.push(Verdict::correct("no stated volume to converge on"));
        }
    }
    let mut v = Verdict::combine(parts);
    if v.outcome == crate::outcome::Outcome::Correct {
        v.detail = format!(
            "{} chords closed{}",
            run.meshes.len(),
            if run.case.euler.is_some() {
                ", Euler characteristic as implied"
            } else {
                ""
            }
        );
    }
    v
}
