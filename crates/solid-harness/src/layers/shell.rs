//! Layer 2: the boundary shell test.
//!
//! Points are sampled on the kernel's mesh at the finest chord
//! ([`CHORD`]). From each sample `p` on a triangle with outward unit normal
//! `n`, the points `p - s n` and `p + s n` must be certified inside and
//! outside by the reference.
//!
//! The step is `s = 10 * CHORD`. Why that makes a correct kernel never
//! fail and never sit in the band: the mesh contract
//! (`cadview_core::solid::TriMesh`) puts every mesh point within the chord
//! of the exact boundary and every boundary point within the chord of the
//! mesh. A stepped point is used only when no part of the mesh is closer
//! to it than `s` (another sheet nearby, at an acute edge or a thin wall,
//! skips the sample), so the segment from `p` to it crosses no other
//! part of the mesh and the exact boundary is at least `s - CHORD = 9
//! CHORD` away. For a correct kernel the point is therefore strictly on
//! the expected side, at least 9e-4 mm from the boundary. The reference's
//! certified bound is a lower bound on that distance: exact for faces,
//! within a factor of sqrt(3) at box corners for intersections of
//! half-spaces, and the interval rounding is near 1e-13 mm, so a correct
//! kernel's points certify. A wrong answer at a certified point means the
//! kernel's surface is not where the recipe says, by more than the step
//! (1e-3 mm): that is this layer's resolution.
//!
//! Sampling: 1000 points area-weighted (big faces) and 1000 on triangles
//! picked uniformly (small and curved faces, which have many small
//! triangles), seeded from the case name.

use crate::mesh::{area_vector2, corners, norm, scale, Grid, V3};
use crate::outcome::Verdict;
use crate::reference::Class;
use crate::rng::Rng;
use crate::run::{Fail, PartRun, CHORDS};

/// The chord of the sampled mesh (mm).
pub const CHORD: f64 = CHORDS[3];
/// The step off the surface (mm).
pub const STEP: f64 = 10.0 * CHORD;
const AREA_SAMPLES: usize = 1000;
const TRIANGLE_SAMPLES: usize = 1000;
/// Fewer checked steps than this is not evidence.
const MIN_CHECKED: usize = 200;

pub fn check<B>(run: &PartRun<B>) -> Verdict {
    let reference = match &run.reference {
        Ok(r) => r,
        Err(e) => return Verdict::na(format!("reference {e}")),
    };
    if let Err(f) = &run.brep {
        return super::closed_form::fail_verdict(f);
    }
    let mesh = match run.mesh(CHORD) {
        Some(Ok(m)) => m,
        Some(Err(Fail::Unsupported(s))) => return Verdict::na(format!("no tessellation: {s}")),
        Some(Err(f)) => return super::closed_form::fail_verdict(f),
        None => return Verdict::na("no mesh requested"),
    };
    if mesh.triangles.is_empty() {
        return Verdict::na("the mesh has no triangles");
    }
    let n = mesh.triangles.len();
    let mut cum = Vec::with_capacity(n);
    let mut total = 0.0;
    let mut normals: Vec<Option<V3>> = Vec::with_capacity(n);
    for i in 0..n {
        let w = area_vector2(corners(mesh, i));
        let a = 0.5 * norm(w);
        total += a;
        cum.push(total);
        normals.push((a > 1e-18).then(|| scale(w, 1.0 / (2.0 * a))));
    }
    let grid = Grid::new(mesh);
    let mut rng = Rng::named(&run.case.name, 2);
    let (mut checked, mut band, mut skipped) = (0usize, 0usize, 0usize);
    let mut wrong: Vec<String> = Vec::new();
    let mut wrong_count = 0usize;
    for k in 0..(AREA_SAMPLES + TRIANGLE_SAMPLES) {
        let ti = if k < AREA_SAMPLES {
            let x = rng.unit() * total;
            cum.partition_point(|&c| c < x).min(n - 1)
        } else {
            rng.below(n)
        };
        let Some(nrm) = normals[ti] else {
            skipped += 1;
            continue;
        };
        let [a, b, c] = corners(mesh, ti);
        let (r1, r2) = (rng.unit().sqrt(), rng.unit());
        let p = [0, 1, 2].map(|i| (1.0 - r1) * a[i] + r1 * (1.0 - r2) * b[i] + r1 * r2 * c[i]);
        for (sign, want_inside) in [(-1.0, true), (1.0, false)] {
            let q = [0, 1, 2].map(|i| p[i] + sign * STEP * nrm[i]);
            if grid.distance_capped(q, STEP) < STEP * (1.0 - 1e-6) {
                skipped += 1;
                continue;
            }
            let cls = reference.classify(cadview_core::solid::Point3::new(q[0], q[1], q[2]));
            match (cls, want_inside) {
                (Class::Inside { .. }, true) | (Class::Outside { .. }, false) => checked += 1,
                (Class::Band, _) => band += 1,
                (got, _) => {
                    wrong_count += 1;
                    if wrong.len() < 2 {
                        wrong.push(format!(
                            "step {} at ({:.4}, {:.4}, {:.4}) along normal ({:.3}, {:.3}, {:.3}): reference says {:?}",
                            if want_inside { "in" } else { "out" },
                            q[0], q[1], q[2], nrm[0], nrm[1], nrm[2], got
                        ));
                    }
                }
            }
        }
    }
    let summary = format!("{checked} certified, {band} band, {skipped} skipped");
    if wrong_count > 0 {
        Verdict::wrong(format!(
            "{wrong_count} steps on the wrong side ({summary}); {}",
            wrong.join("; ")
        ))
    } else if band > 0 {
        Verdict::undecided(format!("steps left in the boundary band ({summary})"))
    } else if checked < MIN_CHECKED {
        Verdict::undecided(format!("too few certified steps ({summary})"))
    } else {
        Verdict::correct(summary)
    }
}
