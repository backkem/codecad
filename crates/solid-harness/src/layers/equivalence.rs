//! Layer 5: equivalent constructions. Two recipes for the same solid must
//! give the same volume, bounding box, centroid and sampled membership.
//! The comparison here is shared with layer 6 (motion, mirroring, `A + A`,
//! operand order), which compares a solid with a moved, mirrored or
//! rebuilt copy.
//!
//! Tolerances, all with a kernel that integrates on exact surfaces in
//! mind (its errors are rounding, about 1e-16 relative):
//!
//! - volume: `1e-9` relative, as the closed-form layer;
//! - bounding box: `1e-9 (1 + D)` mm per coordinate, `D` the larger box
//!   diagonal (rotated geometry carries rounding proportional to its
//!   size);
//! - centroid: the same `1e-9 (1 + D)` mm;
//! - membership: 400 points uniform in the solid's box grown by 5 %, each
//!   classified by the winding number of both kernels' meshes at
//!   [`MEMBERSHIP_CHORD`]. A point within `2 MEMBERSHIP_CHORD + 1e-6` mm
//!   of either mesh is skipped: there the mesh, which may be off the exact
//!   surface by the chord, cannot speak for the solid. A winding number
//!   more than 0.25 from an integer is undecided. Any other disagreement
//!   is wrong. Membership needs closed meshes; without them it is not
//!   applicable (the mesh layer reports the open mesh).

use cadview_core::solid::{Aabb3, Placement, Point3, SolidKernel, TriMesh, Vec3};

use crate::cases::qrot;
use crate::mesh::{topology, winding, Grid};
use crate::outcome::{Outcome, Verdict};
use crate::rng::Rng;
use crate::run::{guard, Fail, PartRun, MEMBERSHIP_CHORD};

use super::closed_form::fail_verdict;

pub const VOLUME_REL_TOL: f64 = 1e-9;
pub const POSITION_REL_TOL: f64 = 1e-9;
const MEMBERSHIP_SAMPLES: usize = 400;

/// How the second solid relates to the first.
#[derive(Debug, Clone, Copy)]
pub enum Map {
    /// The same solid.
    Same,
    /// The first moved by a placement.
    Motion(Placement),
    /// The first mirrored in the plane `x = x0`.
    Mirror(f64),
}

impl Map {
    /// From the first solid's space to the second's.
    fn forward(&self, p: Point3) -> Point3 {
        match self {
            Map::Same => p,
            Map::Motion(m) => {
                let r = qrot(m.rotation, Vec3::new(p.x, p.y, p.z)) + m.translation;
                Point3::new(r.x, r.y, r.z)
            }
            Map::Mirror(x0) => Point3::new(2.0 * x0 - p.x, p.y, p.z),
        }
    }

    /// From the second solid's space back to the first's.
    fn back(&self, p: Point3) -> Point3 {
        match self {
            Map::Same => p,
            Map::Motion(m) => {
                let [w, x, y, z] = m.rotation;
                let inv = [w, -x, -y, -z];
                let d = Vec3::new(
                    p.x - m.translation.x,
                    p.y - m.translation.y,
                    p.z - m.translation.z,
                );
                let r = qrot(inv, d);
                Point3::new(r.x, r.y, r.z)
            }
            Map::Mirror(x0) => Point3::new(2.0 * x0 - p.x, p.y, p.z),
        }
    }
}

fn diag(b: &Aabb3) -> f64 {
    b.max.distance(b.min)
}

/// The verdict when either solid has no B-rep.
pub fn eval_failures<B>(runs: &[&PartRun<B>]) -> Option<Verdict> {
    let mut parts = Vec::new();
    for r in runs {
        if let Err(f) = &r.brep {
            let mut v = fail_verdict(f);
            v.detail = format!("{}: {}", r.case.name, v.detail);
            parts.push(v);
        }
    }
    (!parts.is_empty()).then(|| Verdict::combine(parts))
}

/// Compares `b` with `a` mapped by `map`.
pub fn compare<K: SolidKernel>(
    kernel: &K,
    a: &PartRun<K::Brep>,
    b: &PartRun<K::Brep>,
    map: Map,
    seed_name: &str,
) -> Verdict {
    if let Some(v) = eval_failures(&[a, b]) {
        return v;
    }
    let mut parts = Vec::new();
    // Volume.
    match (&a.mass, &b.mass) {
        (Ok(ma), Ok(mb)) => {
            let scale = ma.volume.abs().max(mb.volume.abs());
            let d = (ma.volume - mb.volume).abs();
            if d <= VOLUME_REL_TOL * scale {
                parts.push(Verdict::correct(""));
            } else {
                parts.push(Verdict::wrong(format!(
                    "volumes {} and {} differ by {:.1e} relative",
                    ma.volume,
                    mb.volume,
                    d / scale
                )));
            }
            // Centroid.
            let ca = map.forward(ma.centroid);
            let tol = match &b.bbox {
                Ok(bb) => POSITION_REL_TOL * (1.0 + diag(bb)),
                Err(_) => POSITION_REL_TOL,
            };
            let e = ca.max_abs_diff(mb.centroid);
            if e > tol {
                parts.push(Verdict::wrong(format!(
                    "centroids differ by {e:.1e} mm: {:?} mapped vs {:?}",
                    ca, mb.centroid
                )));
            }
        }
        (Err(f), _) | (_, Err(f)) => parts.push(fail_verdict(f)),
    }
    // Bounding box.
    let want = match map {
        Map::Same => a.bbox.clone(),
        Map::Motion(m) => match &a.brep {
            Ok(brep) => guard(|| kernel.bbox(brep, &m)),
            Err(f) => Err(f.clone()),
        },
        Map::Mirror(x0) => a.bbox.clone().map(|bb| {
            Aabb3::new(
                Point3::new(2.0 * x0 - bb.max.x, bb.min.y, bb.min.z),
                Point3::new(2.0 * x0 - bb.min.x, bb.max.y, bb.max.z),
            )
        }),
    };
    match (&want, &b.bbox) {
        (Ok(wa), Ok(bb)) => {
            let e = wa.max_abs_diff(bb);
            let tol = POSITION_REL_TOL * (1.0 + diag(bb).max(diag(wa)));
            if e > tol {
                parts.push(Verdict::wrong(format!(
                    "bounding boxes differ by {e:.1e} mm: {wa:?} vs {bb:?}"
                )));
            }
        }
        (Err(f), _) | (_, Err(f)) => parts.push(fail_verdict(f)),
    }
    parts.push(membership(a, b, map, seed_name));
    Verdict::combine(parts)
}

fn closed_mesh<B>(r: &PartRun<B>) -> Result<&TriMesh, Verdict> {
    match r.mesh(MEMBERSHIP_CHORD) {
        Some(Ok(m)) if !m.triangles.is_empty() && topology(m).closed => Ok(m),
        Some(Ok(_)) => Err(Verdict::na(format!(
            "{}: the mesh is not closed, membership not compared",
            r.case.name
        ))),
        Some(Err(Fail::Unsupported(s))) => Err(Verdict::na(format!("no tessellation: {s}"))),
        Some(Err(f)) => Err(fail_verdict(f)),
        None => Err(Verdict::na("no mesh requested")),
    }
}

/// Sampled membership of `b` against `a` mapped by `map`.
pub fn membership<B>(a: &PartRun<B>, b: &PartRun<B>, map: Map, seed_name: &str) -> Verdict {
    let (ma, mb) = match (closed_mesh(a), closed_mesh(b)) {
        (Ok(x), Ok(y)) => (x, y),
        (Err(v), _) | (_, Err(v)) => return v,
    };
    let (lo, hi) = crate::mesh::aabb(mb);
    let grow = [0, 1, 2].map(|i| 0.05 * (hi[i] - lo[i]) + 1e-3);
    let (ga, gb) = (Grid::new(ma), Grid::new(mb));
    let band = 2.0 * MEMBERSHIP_CHORD + 1e-6;
    let mut rng = Rng::named(seed_name, 5);
    let (mut agree, mut skipped, mut fuzzy) = (0, 0, 0);
    let mut bad: Vec<String> = Vec::new();
    let mut bad_count = 0;
    for _ in 0..MEMBERSHIP_SAMPLES {
        let x = [0, 1, 2].map(|i| rng.range(lo[i] - grow[i], hi[i] + grow[i]));
        let pb = Point3::new(x[0], x[1], x[2]);
        let pa = map.back(pb);
        let xa = [pa.x, pa.y, pa.z];
        if gb.distance_capped(x, band) < band || ga.distance_capped(xa, band) < band {
            skipped += 1;
            continue;
        }
        let (wa, wb) = (winding(ma, xa), winding(mb, x));
        let (ra, rb) = (wa.round(), wb.round());
        if (wa - ra).abs() > 0.25 || (wb - rb).abs() > 0.25 {
            fuzzy += 1;
            continue;
        }
        if (ra != 0.0) == (rb != 0.0) {
            agree += 1;
        } else {
            bad_count += 1;
            if bad.len() < 2 {
                bad.push(format!(
                    "at ({:.3}, {:.3}, {:.3}): {} says {}, {} says {}",
                    x[0],
                    x[1],
                    x[2],
                    a.case.name,
                    if ra != 0.0 { "inside" } else { "outside" },
                    b.case.name,
                    if rb != 0.0 { "inside" } else { "outside" }
                ));
            }
        }
    }
    let summary = format!("membership: {agree} agree, {skipped} near a surface");
    if bad_count > 0 {
        Verdict::wrong(format!(
            "{bad_count} sampled points disagree ({summary}); {}",
            bad.join("; ")
        ))
    } else if fuzzy > 0 {
        Verdict::undecided(format!(
            "{fuzzy} winding numbers not near an integer ({summary})"
        ))
    } else {
        Verdict::correct(summary)
    }
}

/// Layer 5 for one pair.
pub fn check<K: SolidKernel>(
    kernel: &K,
    name: &str,
    a: &PartRun<K::Brep>,
    b: &PartRun<K::Brep>,
) -> Verdict {
    let v = compare(kernel, a, b, Map::Same, name);
    tidy(v)
}

/// Keeps the detail short for a passing comparison.
pub fn tidy(mut v: Verdict) -> Verdict {
    if v.outcome == Outcome::Correct {
        v.detail = v
            .detail
            .split("; ")
            .find(|s| s.starts_with("membership"))
            .unwrap_or("")
            .to_string();
    }
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_round_trip() {
        let m = Map::Motion(crate::cases::generic_motion());
        let p = Point3::new(1.0, -2.0, 3.5);
        let q = m.back(m.forward(p));
        assert!(q.max_abs_diff(p) < 1e-13);
        let r = Map::Mirror(37.0);
        assert_eq!(
            r.forward(Point3::new(30.0, 1.0, 2.0)),
            Point3::new(44.0, 1.0, 2.0)
        );
        assert_eq!(r.back(r.forward(p)), p);
    }
}
