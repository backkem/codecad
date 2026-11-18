//! The curved corpus (ladder rung 3D0b): solids with curved faces made by
//! the curved features (round, loft, revolve, sweep), each with an exact
//! volume and bounding box.
//!
//! Every closed form is derived in the comment above its entry in exact
//! arithmetic, with pi kept symbolic. Dimensions are mm.
//!
//! Gate: each part is a valid B-rep with one solid and a closed mesh, its
//! volume matches to [`VOLUME_REL_TOL`] relative and its bounding box to
//! [`BBOX_TOL`] mm.

use std::f64::consts::{FRAC_1_SQRT_2, PI};

use kurbo::{Arc, Line, Point, Vec2};

use super::{CorpusEntry, CorpusPart};
use crate::solid::kernel::Aabb3;
use crate::solid::types::{
    DefId, EdgeSel, FeatId, Feature, Op, PartDef, Path3, Plane3, Point3, Profile2d, RoundFallback,
    Seg2, Seg3,
};

/// Relative volume tolerance of the gate.
pub const VOLUME_REL_TOL: f64 = 1e-6;
/// Bounding-box tolerance of the gate (mm).
pub const BBOX_TOL: f64 = 1e-6;

fn single(name: &'static str, def: PartDef, volume: f64, bbox: Aabb3) -> CorpusEntry {
    CorpusEntry {
        name,
        parts: vec![CorpusPart { def, volume, bbox }],
        tools: Vec::new(),
    }
}

fn bbox(min: [f64; 3], max: [f64; 3]) -> Aabb3 {
    Aabb3::new(
        Point3::new(min[0], min[1], min[2]),
        Point3::new(max[0], max[1], max[2]),
    )
}

/// A circular arc about `c` of radius `r` from angle `start` over the
/// signed `sweep` (radians).
pub fn arc(c: (f64, f64), r: f64, start: f64, sweep: f64) -> Seg2 {
    Seg2::Arc(Arc {
        center: Point::new(c.0, c.1),
        radii: Vec2::new(r, r),
        start_angle: start,
        sweep_angle: sweep,
        x_rotation: 0.0,
    })
}

/// A straight profile segment.
pub fn line(a: (f64, f64), b: (f64, f64)) -> Seg2 {
    Seg2::Line(Line::new(a, b))
}

/// Rounded slab: a 400 x 200 x 20 slab with all 12 edges rounded r = 4.
///
/// A ball of radius r rolling inside a convex box reaches exactly the
/// points within r of the inner box B' = [r, L - r] x [r, W - r] x
/// [r, H - r], of sides a' = 392, b' = 192, c' = 12. That set splits into
/// B' itself, six slabs of thickness r on its faces, twelve quarter
/// cylinders of radius r along its edges (four of each length) and eight
/// ball octants at its corners:
/// V = a'b'c' + 2r(a'b' + b'c' + c'a') + pi r^2 (a' + b' + c') + 4/3 pi r^3
///   = 903 168 + 8 * 82 272 + 16 pi * 596 + 256 pi / 3
///   = 1 561 344 + 9 536 pi + 256 pi / 3 = 1 561 344 + 28 864 pi / 3.
/// The faces keep their full extent, so the box is [0, 0, 0] to
/// [400, 200, 20].
pub fn rounded_slab() -> CorpusEntry {
    single(
        "rounded slab",
        PartDef {
            id: DefId(101),
            name: "rounded slab".into(),
            features: vec![
                Feature::Extrude {
                    id: FeatId(1),
                    profile: Profile2d::rect(0.0, 0.0, 400.0, 200.0),
                    plane: Plane3::XY,
                    distance: 20.0,
                    taper_deg: 0.0,
                    op: Op::Add,
                },
                Feature::Fillet {
                    id: FeatId(2),
                    edges: EdgeSel::All,
                    radius: 4.0,
                    fallback: RoundFallback::Error,
                },
            ],
        },
        1_561_344.0 + 28_864.0 * PI / 3.0,
        bbox([0.0, 0.0, 0.0], [400.0, 200.0, 20.0]),
    )
}

/// Tapered square bar: a ruled loft from a 40 x 40 square at z = 0 to a
/// 24 x 24 square at z = 300, both centred on the z axis.
///
/// Corresponding sides are parallel, so every side face is a plane and the
/// solid is a frustum of a square pyramid. Its section at height z is a
/// square of side s(z) = 40 - 16 z / 300, so
/// V = integral of s(z)^2 dz = h / 3 (A1 + A2 + sqrt(A1 A2))
///   = 100 * (1 600 + 576 + 960) = 313 600.
/// The box is the bottom square swept up: [-20, -20, 0] to [20, 20, 300].
pub fn tapered_square_bar() -> CorpusEntry {
    single(
        "tapered square bar",
        PartDef {
            id: DefId(102),
            name: "tapered square bar".into(),
            features: vec![Feature::Loft {
                id: FeatId(1),
                sections: vec![
                    (Profile2d::rect(-20.0, -20.0, 20.0, 20.0), Plane3::XY),
                    (
                        Profile2d::rect(-12.0, -12.0, 12.0, 12.0),
                        Plane3::XY.offset(300.0),
                    ),
                ],
                ruled: true,
                op: Op::Add,
            }],
        },
        313_600.0,
        bbox([-20.0, -20.0, 0.0], [20.0, 20.0, 300.0]),
    )
}

/// The meridian of [`revolved_beads_and_coves`] in the XZ plane (`u` the
/// radius, `v` the height), counter-clockwise from the origin.
pub fn beads_and_coves_profile() -> Profile2d {
    use std::f64::consts::FRAC_PI_2;
    Profile2d {
        outer: vec![
            line((0.0, 0.0), (15.0, 0.0)),
            line((15.0, 0.0), (15.0, 20.0)),
            // The bead: centre (15, 26), radius 6, out to radius 21.
            arc((15.0, 26.0), 6.0, -FRAC_PI_2, PI),
            line((15.0, 32.0), (15.0, 52.0)),
            // The cove: centre (15, 56), radius 4, in to radius 11.
            arc((15.0, 56.0), 4.0, -FRAC_PI_2, -PI),
            line((15.0, 60.0), (15.0, 80.0)),
            line((15.0, 80.0), (0.0, 80.0)),
            line((0.0, 80.0), (0.0, 0.0)),
        ],
        holes: Vec::new(),
    }
}

/// Revolved profile with a bead and a cove: a cylinder of radius R = 15
/// and height 80 about the z axis, with a half-round bead (radius 6,
/// centred at height 26 on the surface) and a half-round cove (radius 4,
/// centred at height 56) turned into it. One full revolve of the meridian
/// [`beads_and_coves_profile`], which runs up the axis.
///
/// By discs: V = pi * integral of rho(z)^2 dz. The plain cylinder gives
/// pi 225 * 80 = 18 000 pi. Over a bead of radius a centred on the
/// surface, rho = R + s with s = sqrt(a^2 - u^2) for u in [-a, a];
/// integral of s du = pi a^2 / 2 and of s^2 du = 4 a^3 / 3, so the bead
/// adds pi (2 R pi a^2 / 2 + 4 a^3 / 3) = pi (pi R a^2 + 4 a^3 / 3). A cove
/// (rho = R - s) removes pi (pi R a^2 - 4 a^3 / 3). With a = 6 for the bead
/// and a = 4 for the cove:
/// bead = pi (540 pi + 288) = 540 pi^2 + 288 pi,
/// cove = pi (240 pi - 256 / 3) = 240 pi^2 - 256 pi / 3,
/// V = 18 000 pi + 540 pi^2 + 288 pi - 240 pi^2 + 256 pi / 3
///   = 300 pi^2 + 55 120 pi / 3.
/// The bead's crest reaches radius 21 all round: box [-21, -21, 0] to
/// [21, 21, 80].
pub fn revolved_beads_and_coves() -> CorpusEntry {
    single(
        "revolved profile with bead and cove",
        PartDef {
            id: DefId(103),
            name: "revolved profile with bead and cove".into(),
            features: vec![Feature::Revolve {
                id: FeatId(1),
                profile: beads_and_coves_profile(),
                plane: Plane3::XZ,
                axis: Line::new((0.0, 0.0), (0.0, 1.0)),
                angle_deg: 360.0,
                op: Op::Add,
            }],
        },
        300.0 * PI * PI + 55_120.0 * PI / 3.0,
        bbox([-21.0, -21.0, 0.0], [21.0, 21.0, 80.0]),
    )
}

/// The path of [`swept_round_bar`]: 100 along +X, a quarter turn of
/// radius 40 to the left (about +Z, centre (100, 40, 0)), 100 along +Y.
pub fn line_arc_line_path() -> Path3 {
    let p = |x, y| Point3::new(x, y, 0.0);
    Path3 {
        segs: vec![
            Seg3::Line {
                from: p(0.0, 0.0),
                to: p(100.0, 0.0),
            },
            Seg3::Arc {
                from: p(100.0, 0.0),
                via: p(100.0 + 40.0 * FRAC_1_SQRT_2, 40.0 - 40.0 * FRAC_1_SQRT_2),
                to: p(140.0, 40.0),
            },
            Seg3::Line {
                from: p(140.0, 40.0),
                to: p(140.0, 140.0),
            },
        ],
    }
}

/// Round bar swept along a path with an arc: a disc of radius r = 6 on
/// the YZ plane at the origin, carried along [`line_arc_line_path`].
///
/// The disc stays square to the path and its centre on it, and the arc's
/// radius (40) exceeds r, so the bar never meets itself. By Pappus's
/// theorem each piece's volume is the disc's area times the length its
/// centre travels: V = pi r^2 (100 + 40 pi / 2 + 100)
///   = 36 pi (200 + 20 pi) = 7 200 pi + 720 pi^2.
/// Box: the start cap is the disc in the plane x = 0; the second leg and
/// the outside of the bend reach x = 140 + 6 = 146; the first leg spans
/// y in [-6, 6] and the end cap lies in y = 140; z in [-6, 6]:
/// [0, -6, -6] to [146, 140, 6].
pub fn swept_round_bar() -> CorpusEntry {
    single(
        "round bar swept along a line, an arc and a line",
        PartDef {
            id: DefId(104),
            name: "swept round bar".into(),
            features: vec![Feature::Sweep {
                id: FeatId(1),
                profile: Profile2d::circle(Point::new(0.0, 0.0), 6.0),
                plane: Plane3::YZ,
                path: line_arc_line_path(),
                op: Op::Add,
            }],
        },
        7_200.0 * PI + 720.0 * PI * PI,
        bbox([0.0, -6.0, -6.0], [146.0, 140.0, 6.0]),
    )
}

/// The whole curved corpus, in ladder order.
pub fn entries() -> Vec<CorpusEntry> {
    vec![
        rounded_slab(),
        tapered_square_bar(),
        revolved_beads_and_coves(),
        swept_round_bar(),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The closed forms against independent evaluations of the same
    /// geometry: Steiner's formula term by term, the prismatoid formula,
    /// a numerical disc integration and the path length.
    #[test]
    fn closed_forms_are_self_consistent() {
        // Rounded slab: the decomposition into box, slabs, quarter
        // cylinders and octants.
        let (r, a, b, c) = (4.0, 392.0, 192.0, 12.0);
        let parts = a * b * c
            + 2.0 * r * (a * b + b * c + c * a)
            + 4.0 * (PI * r * r / 4.0) * (a + b + c)
            + 8.0 * (4.0 / 3.0 * PI * r * r * r / 8.0);
        let v = rounded_slab().parts[0].volume;
        assert!((parts - v).abs() <= 1e-9 * v, "{parts} vs {v}");

        // Tapered bar: Simpson on s(z)^2 (exact for a quadratic).
        let s = |z: f64| 40.0 - 16.0 * z / 300.0;
        let simpson = 300.0 / 6.0 * (s(0.0).powi(2) + 4.0 * s(150.0).powi(2) + s(300.0).powi(2));
        assert!((simpson - tapered_square_bar().parts[0].volume).abs() < 1e-6);

        // Revolved profile: pi * integral of rho^2 by the midpoint rule on
        // a fine grid; the rule's error is far below 1e-6 relative.
        let rho = |z: f64| {
            if (20.0..=32.0).contains(&z) {
                15.0 + (36.0 - (z - 26.0) * (z - 26.0)).max(0.0).sqrt()
            } else if (52.0..=60.0).contains(&z) {
                15.0 - (16.0 - (z - 56.0) * (z - 56.0)).max(0.0).sqrt()
            } else {
                15.0
            }
        };
        let n = 400_000;
        let h = 80.0 / n as f64;
        let num: f64 = (0..n)
            .map(|i| {
                let z = (i as f64 + 0.5) * h;
                rho(z) * rho(z)
            })
            .sum::<f64>()
            * h
            * PI;
        let v = revolved_beads_and_coves().parts[0].volume;
        assert!((num - v).abs() <= 1e-6 * v, "{num} vs {v}");

        // Swept bar: the path's length by its segments.
        let path = line_arc_line_path();
        let mut len = 0.0;
        for s in &path.segs {
            let (a, b) = s.ends();
            len += match s {
                Seg3::Line { .. } => a.distance(b),
                // A quarter of a circle of radius 40 (centre (100, 40)).
                Seg3::Arc { via, .. } => {
                    let c = Point3::new(100.0, 40.0, 0.0);
                    assert!((via.distance(c) - 40.0).abs() < 1e-12);
                    40.0 * PI / 2.0
                }
            };
        }
        let v = swept_round_bar().parts[0].volume;
        assert!((PI * 36.0 * len - v).abs() <= 1e-9 * v);
    }
}
