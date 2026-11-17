//! The prismatic corpus (ladder rung 3D0a): flat woodworking parts made of
//! extrudes, holes, booleans and chamfers, each with an exact volume and
//! bounding box.
//!
//! Every closed form is derived in the comment above its entry in exact
//! arithmetic, with pi kept symbolic. Dimensions are mm. All parts sit in
//! their own frame with one corner at the origin.
//!
//! Gate: each part is a valid B-rep with one solid, its volume matches to
//! [`VOLUME_REL_TOL`] relative and its bounding box to [`BBOX_TOL`] mm.

use std::f64::consts::PI;

use super::{CorpusEntry, CorpusPart};
use crate::solid::kernel::Aabb3;
use crate::solid::types::{
    CapSide, DefId, Depth, EdgeSel, FeatId, Feature, GrainDir, Op, PartDef, PartKind, Placement,
    Plane3, Point3, Profile2d, ToolRef, Vec3,
};

/// Relative volume tolerance of the gate.
pub const VOLUME_REL_TOL: f64 = 1e-9;
/// Bounding-box tolerance of the gate (mm).
pub const BBOX_TOL: f64 = 1e-9;

fn flat(thickness: f64) -> PartKind {
    PartKind::Flat {
        thickness,
        grain: GrainDir::X,
    }
}

fn part_def(id: u64, name: &str, kind: PartKind, features: Vec<Feature>) -> PartDef {
    PartDef {
        id: DefId(id),
        name: name.into(),
        kind,
        features,
    }
}

/// An extrude of `profile` on `plane` by `distance`, no taper.
fn extrude(id: u32, profile: Profile2d, plane: Plane3, distance: f64, op: Op) -> Feature {
    Feature::Extrude {
        id: FeatId(id),
        profile,
        plane,
        distance,
        taper_deg: 0.0,
        op,
    }
}

/// The base board `[0, l] x [0, w] x [0, t]`: a rectangle on XY extruded
/// up by `t`. Feature id 1.
fn board(l: f64, w: f64, t: f64) -> Feature {
    extrude(1, Profile2d::rect(0.0, 0.0, l, w), Plane3::XY, t, Op::Add)
}

fn bbox(min: [f64; 3], max: [f64; 3]) -> Aabb3 {
    Aabb3::new(
        Point3::new(min[0], min[1], min[2]),
        Point3::new(max[0], max[1], max[2]),
    )
}

fn single(name: &'static str, def: PartDef, volume: f64, bbox: Aabb3) -> CorpusEntry {
    CorpusEntry {
        name,
        parts: vec![CorpusPart { def, volume, bbox }],
        tools: Vec::new(),
    }
}

/// Board: 600 x 300 x 18.
///
/// V = 600 * 300 * 18 = 3 240 000.
/// Box [0, 0, 0] to [600, 300, 18].
pub fn board_plain() -> CorpusEntry {
    single(
        "board",
        part_def(1, "board", flat(18.0), vec![board(600.0, 300.0, 18.0)]),
        3_240_000.0,
        bbox([0.0, 0.0, 0.0], [600.0, 300.0, 18.0]),
    )
}

/// Board with a through hole: the board above, minus a Ø10 hole drilled
/// down from (50, 50, 18) through the 18 thickness.
///
/// The hole is a cylinder of radius 5 and height 18, fully inside the board
/// in plan (50 > 5 from every edge).
/// V = 3 240 000 - pi * 5^2 * 18 = 3 240 000 - 450 pi.
/// The box is unchanged: [0, 0, 0] to [600, 300, 18].
pub fn board_with_through_hole() -> CorpusEntry {
    single(
        "board with through hole",
        part_def(
            2,
            "board with hole",
            flat(18.0),
            vec![
                board(600.0, 300.0, 18.0),
                Feature::Hole {
                    id: FeatId(2),
                    at: Point3::new(50.0, 50.0, 18.0),
                    dir: Vec3::new(0.0, 0.0, -1.0),
                    dia: 10.0,
                    depth: Depth::Through,
                    cbore: None,
                },
            ],
        ),
        3_240_000.0 - 450.0 * PI,
        bbox([0.0, 0.0, 0.0], [600.0, 300.0, 18.0]),
    )
}

/// Dado: an 18 wide, 6 deep housing across the full 300 width of the
/// board, for an 18 shelf, cut by a boolean with an inline tool definition.
///
/// The tool (def 31) is a box 18 x 310 x 10 placed at (291, -5, 12), so it
/// spans x in [291, 309], y in [-5, 305], z in [12, 22]. Inside the board
/// that is x in [291, 309], y in [0, 300], z in [12, 18]:
/// removed = 18 * 300 * 6 = 32 400.
/// V = 3 240 000 - 32 400 = 3 207 600.
/// The dado does not reach the outer faces' extremes, so the box is
/// unchanged: [0, 0, 0] to [600, 300, 18].
pub fn dado() -> CorpusEntry {
    let tool = part_def(
        31,
        "dado cutter",
        PartKind::General,
        vec![extrude(
            1,
            Profile2d::rect(0.0, 0.0, 18.0, 310.0),
            Plane3::XY,
            10.0,
            Op::Add,
        )],
    );
    let side = part_def(
        3,
        "side with dado",
        flat(18.0),
        vec![
            board(600.0, 300.0, 18.0),
            Feature::Boolean {
                id: FeatId(2),
                op: Op::Subtract,
                tool: ToolRef::Def(DefId(31), Placement::translation(291.0, -5.0, 12.0)),
            },
        ],
    );
    CorpusEntry {
        name: "dado",
        parts: vec![CorpusPart {
            def: side,
            volume: 3_207_600.0,
            bbox: bbox([0.0, 0.0, 0.0], [600.0, 300.0, 18.0]),
        }],
        tools: vec![tool],
    }
}

/// Rabbet: a 12 wide, 9 deep rebate along the back long edge, cut by a
/// subtracting extrude sketched on the YZ plane and run along X.
///
/// On the YZ plane `u` is Y and `v` is Z. The profile u in [288, 310],
/// v in [9, 25], on the plane at x = -5 extruded 610 along +X, spans
/// x in [-5, 605], y in [288, 310], z in [9, 25]. Inside the board that is
/// x in [0, 600], y in [288, 300], z in [9, 18]:
/// removed = 600 * 12 * 9 = 64 800.
/// V = 3 240 000 - 64 800 = 3 175 200.
/// The bottom half of the back edge (z in [0, 9] at y = 300) remains, so
/// the box is unchanged: [0, 0, 0] to [600, 300, 18].
pub fn rabbet() -> CorpusEntry {
    single(
        "rabbet",
        part_def(
            4,
            "back with rabbet",
            flat(18.0),
            vec![
                board(600.0, 300.0, 18.0),
                extrude(
                    2,
                    Profile2d::rect(288.0, 9.0, 310.0, 25.0),
                    Plane3::YZ.at(Point3::new(-5.0, 0.0, 0.0)),
                    610.0,
                    Op::Subtract,
                ),
            ],
        ),
        3_175_200.0,
        bbox([0.0, 0.0, 0.0], [600.0, 300.0, 18.0]),
    )
}

/// Mortise and tenon pair.
///
/// Stile: a 45 x 45 square section on the YZ plane extruded 400 along X,
/// so [0, 400] x [0, 45] x [0, 45], volume 45 * 45 * 400 = 810 000.
/// The mortise is sketched on the top face itself (plane z = 45, so the
/// tool's top is coplanar with the stile's top) and extruded -30: the
/// rectangle x in [175, 225], y in [15, 30] gives a pocket
/// 50 x 15 x 30 = 22 500.
/// V(stile) = 810 000 - 22 500 = 787 500. Box [0, 0, 0] to [400, 45, 45].
///
/// Rail: a 300 x 70 x 25 board, volume 525 000, with a tenon added on its
/// end face x = 300 (sketched on that face, so the tenon's base touches
/// the rail face to face): the YZ rectangle y in [10, 60], z in [5, 20]
/// extruded 30 along +X, so 30 x 50 x 15 = 22 500.
/// V(rail) = 525 000 + 22 500 = 547 500. Box [0, 0, 0] to [330, 70, 25].
///
/// The pair fits: the tenon's length 30, width 50 and thickness 15 match
/// the mortise's depth 30, length 50 and width 15, so both volumes are
/// 22 500.
pub fn mortise_and_tenon() -> CorpusEntry {
    let stile = part_def(
        51,
        "stile with mortise",
        PartKind::General,
        vec![
            extrude(
                1,
                Profile2d::rect(0.0, 0.0, 45.0, 45.0),
                Plane3::YZ,
                400.0,
                Op::Add,
            ),
            extrude(
                2,
                Profile2d::rect(175.0, 15.0, 225.0, 30.0),
                Plane3::XY.offset(45.0),
                -30.0,
                Op::Subtract,
            ),
        ],
    );
    let rail = part_def(
        52,
        "rail with tenon",
        flat(25.0),
        vec![
            board(300.0, 70.0, 25.0),
            extrude(
                2,
                Profile2d::rect(10.0, 5.0, 60.0, 20.0),
                Plane3::YZ.at(Point3::new(300.0, 0.0, 0.0)),
                30.0,
                Op::Add,
            ),
        ],
    );
    CorpusEntry {
        name: "mortise and tenon pair",
        parts: vec![
            CorpusPart {
                def: stile,
                volume: 787_500.0,
                bbox: bbox([0.0, 0.0, 0.0], [400.0, 45.0, 45.0]),
            },
            CorpusPart {
                def: rail,
                volume: 547_500.0,
                bbox: bbox([0.0, 0.0, 0.0], [330.0, 70.0, 25.0]),
            },
        ],
        tools: Vec::new(),
    }
}

/// Half-lap: a 400 x 50 x 20 rail with a notch half its thickness deep and
/// one rail width (50) long at the middle, cut by a rotated inline tool.
///
/// The tool (def 61) is the local box [0, 70] x [0, 50] x [0, 15], turned a
/// quarter turn about +Z ((x, y) goes to (-y, x)) and moved by
/// (225, -10, 10). Local x in [0, 70] becomes world y in [0, 70] and local
/// y in [0, 50] becomes world x in [-50, 0], so after the move the tool
/// spans x in [175, 225], y in [-10, 60], z in [10, 25]. Inside the rail:
/// x in [175, 225], y in [0, 50], z in [10, 20], removed 50 * 50 * 10 =
/// 25 000.
/// V = 400 * 50 * 20 - 25 000 = 400 000 - 25 000 = 375 000.
/// Box [0, 0, 0] to [400, 50, 20].
pub fn half_lap() -> CorpusEntry {
    let tool = part_def(
        61,
        "half-lap cutter",
        PartKind::General,
        vec![extrude(
            1,
            Profile2d::rect(0.0, 0.0, 70.0, 50.0),
            Plane3::XY,
            15.0,
            Op::Add,
        )],
    );
    let rail = part_def(
        6,
        "rail with half-lap",
        flat(20.0),
        vec![
            board(400.0, 50.0, 20.0),
            Feature::Boolean {
                id: FeatId(2),
                op: Op::Subtract,
                tool: ToolRef::Def(
                    DefId(61),
                    Placement::axis_angle(Vec3::Z, PI / 2.0, Vec3::new(225.0, -10.0, 10.0)),
                ),
            },
        ],
    );
    CorpusEntry {
        name: "half-lap",
        parts: vec![CorpusPart {
            def: rail,
            volume: 375_000.0,
            bbox: bbox([0.0, 0.0, 0.0], [400.0, 50.0, 20.0]),
        }],
        tools: vec![tool],
    }
}

/// Counterbored hole: a 100 x 100 x 30 block with a Ø9 through hole at its
/// centre, counterbored Ø15 x 9 deep from the top.
///
/// The removed volume is the counterbore, radius 7.5 over the top 9, plus
/// the hole, radius 4.5 over the remaining 30 - 9 = 21:
/// removed = pi * 7.5^2 * 9 + pi * 4.5^2 * 21
///         = pi * (56.25 * 9 + 20.25 * 21) = pi * (506.25 + 425.25)
///         = 931.5 pi.
/// V = 100 * 100 * 30 - 931.5 pi = 300 000 - 931.5 pi.
/// Box [0, 0, 0] to [100, 100, 30].
pub fn counterbored_hole() -> CorpusEntry {
    single(
        "counterbored hole",
        part_def(
            7,
            "block with counterbore",
            PartKind::General,
            vec![
                board(100.0, 100.0, 30.0),
                Feature::Hole {
                    id: FeatId(2),
                    at: Point3::new(50.0, 50.0, 30.0),
                    dir: Vec3::new(0.0, 0.0, -1.0),
                    dia: 9.0,
                    depth: Depth::Through,
                    cbore: Some((15.0, 9.0)),
                },
            ],
        ),
        300_000.0 - 931.5 * PI,
        bbox([0.0, 0.0, 0.0], [100.0, 100.0, 30.0]),
    )
}

/// Chamfered slab: a 200 x 100 x 20 slab with its four top edges (the end
/// cap of the base extrude) chamfered d = 3, mitred at the corners.
///
/// Below z = H - d the slab is untouched: L W (H - d). The top layer
/// z in [H - d, H] that remains is a prismatoid (its side faces are the
/// four bevel planes, which meet in mitres at the corners) with base
/// L x W, top (L - 2d) x (W - 2d) and mid-section (L - d) x (W - d). The
/// prismatoid formula is exact here, so its volume is
/// d / 6 * (L W + 4 (L - d)(W - d) + (L - 2d)(W - 2d))
/// = d / 6 * (6 L W - 6 L d - 6 W d + 8 d^2)
/// = L W d - (L + W) d^2 + 4 d^3 / 3.
/// V = L W (H - d) + L W d - (L + W) d^2 + 4 d^3 / 3
///   = L W H - (L + W) d^2 + 4 d^3 / 3.
/// Cross-check: the four edge wedges remove (L + W) d^2 (triangles of legs
/// d along a total length 2 (L + W)), and at each of the 4 corners two
/// wedges overlap in a pyramid of volume d^3 / 3, counted twice.
/// With L = 200, W = 100, H = 20, d = 3:
/// V = 400 000 - 300 * 9 + 4 * 27 / 3 = 400 000 - 2 700 + 36 = 397 336.
/// The bottom face keeps its full outline, so the box is unchanged:
/// [0, 0, 0] to [200, 100, 20].
pub fn chamfered_slab() -> CorpusEntry {
    single(
        "chamfered slab",
        part_def(
            8,
            "chamfered slab",
            flat(20.0),
            vec![
                board(200.0, 100.0, 20.0),
                Feature::Chamfer {
                    id: FeatId(2),
                    edges: EdgeSel::Cap {
                        feature: FeatId(1),
                        side: CapSide::End,
                        segs: None,
                    },
                    d1: 3.0,
                    d2: 3.0,
                },
            ],
        ),
        397_336.0,
        bbox([0.0, 0.0, 0.0], [200.0, 100.0, 20.0]),
    )
}

/// The whole prismatic corpus, in ladder order.
pub fn entries() -> Vec<CorpusEntry> {
    vec![
        board_plain(),
        board_with_through_hole(),
        dado(),
        rabbet(),
        mortise_and_tenon(),
        half_lap(),
        counterbored_hole(),
        chamfered_slab(),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::solid::arris_kernel::ArrisKernel;
    use crate::solid::corpus::measure;
    use crate::solid::kernel::SolidKernel;
    use std::time::Instant;

    /// Runs every entry on `kernel` through the trait, prints a table and
    /// fails on any part that misses the gate.
    fn run_corpus<K: SolidKernel>(kernel: &K) {
        let profile = if cfg!(debug_assertions) {
            "debug"
        } else {
            "release"
        };
        println!(
            "\nprismatic corpus on {} ({profile} build)\n{:<26} {:<24} {:>18} {:>18} {:>9} {:>9} {:>5} {:>5} {:>10} {:>10}",
            kernel.name(),
            "entry",
            "part",
            "closed form",
            "kernel",
            "rel err",
            "bbox err",
            "valid",
            "faces",
            "eval ms",
            "total ms"
        );
        let mut failures = Vec::new();
        let mut total_eval = 0.0;
        let all = Instant::now();
        for entry in entries() {
            for part in &entry.parts {
                let t0 = Instant::now();
                let evaluated = kernel.evaluate(&part.def, &entry);
                let eval_ms = t0.elapsed().as_secs_f64() * 1e3;
                total_eval += eval_ms;
                let brep = match evaluated {
                    Ok(e) => e.brep,
                    Err(e) => {
                        println!(
                            "{:<26} {:<24} EVALUATION FAILED: {e}",
                            entry.name, part.def.name
                        );
                        failures.push(format!("{} / {}: {e}", entry.name, part.def.name));
                        continue;
                    }
                };
                let r = match measure(kernel, part, &brep) {
                    Ok(r) => r,
                    Err(e) => {
                        println!(
                            "{:<26} {:<24} MEASURE FAILED: {e}",
                            entry.name, part.def.name
                        );
                        failures.push(format!("{} / {}: {e}", entry.name, part.def.name));
                        continue;
                    }
                };
                let total_ms = t0.elapsed().as_secs_f64() * 1e3;
                println!(
                    "{:<26} {:<24} {:>18.6} {:>18.6} {:>9.1e} {:>9.1e} {:>5} {:>5} {:>10.2} {:>10.2}",
                    entry.name,
                    r.part,
                    r.expected_volume,
                    r.volume,
                    r.volume_rel_err,
                    r.bbox_err,
                    if r.valid && r.solids == 1 { "yes" } else { "NO" },
                    r.faces,
                    eval_ms,
                    total_ms
                );
                if !r.passes(VOLUME_REL_TOL, BBOX_TOL) {
                    failures.push(format!(
                        "{} / {}: valid={} solids={} exact={} rel_err={:e} bbox_err={:e} bbox={:?} problems={:?}",
                        entry.name,
                        r.part,
                        r.valid,
                        r.solids,
                        r.exact_volume,
                        r.volume_rel_err,
                        r.bbox_err,
                        r.bbox,
                        r.problems
                    ));
                }
            }
        }
        println!(
            "evaluation total {:.2} ms, corpus total {:.2} ms",
            total_eval,
            all.elapsed().as_secs_f64() * 1e3
        );
        assert!(
            failures.is_empty(),
            "corpus gate failed:\n{}",
            failures.join("\n")
        );
    }

    #[test]
    fn arris_passes_the_prismatic_corpus() {
        run_corpus(&ArrisKernel);
    }

    #[test]
    fn corpus_closed_forms_are_self_consistent() {
        // The mortise and tenon pair fit: both are 22 500 mm^3.
        let pair = mortise_and_tenon();
        let stile_solid = 45.0 * 45.0 * 400.0;
        let rail_body = 300.0 * 70.0 * 25.0;
        assert_eq!(stile_solid - pair.parts[0].volume, 22_500.0);
        assert_eq!(pair.parts[1].volume - rail_body, 22_500.0);
        // The chamfer closed form agrees with the prismatoid evaluated
        // directly.
        let (l, w, h, d): (f64, f64, f64, f64) = (200.0, 100.0, 20.0, 3.0);
        let frustum = d / 6.0 * (l * w + 4.0 * (l - d) * (w - d) + (l - 2.0 * d) * (w - 2.0 * d));
        assert!((l * w * (h - d) + frustum - chamfered_slab().parts[0].volume).abs() < 1e-9);
        // Every entry's parts and tools have unique ids.
        for e in entries() {
            let mut ids: Vec<u64> = e
                .parts
                .iter()
                .map(|p| p.def.id.0)
                .chain(e.tools.iter().map(|t| t.id.0))
                .collect();
            let n = ids.len();
            ids.sort_unstable();
            ids.dedup();
            assert_eq!(ids.len(), n, "{}", e.name);
        }
    }
}
