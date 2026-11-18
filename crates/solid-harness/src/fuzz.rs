//! The fuzzer's library half (docs/3d-test-harness.md section 4, layer 10):
//! a seeded generator of random recipes over the evaluated features, a
//! judge that needs no closed form, a shrinker and a printer that writes
//! a failing recipe as a catalogue case. The driver is
//! `examples/fuzz.rs`; nothing here runs in `cargo test` except a check
//! that generation is reproducible.
//!
//! **Generation.** Case `index` of seed `seed` is drawn from
//! `Rng::new(mix(seed, index))` alone, so every case is reproducible from
//! the pair. A recipe ([`Recipe`]) is a base solid (a box, a convex
//! prism, a cylinder or a washer, each with its box at `[0, L] x [0, W] x
//! [0, H]`), up to four operations (a boolean with a box or cylinder tool,
//! a hole, a cap chamfer right after the base) and an optional motion of
//! the whole part. The draws are biased toward the region where kernels
//! break ([`Gen`]): tool faces flush with, or `1e-12` to `1e-3` mm off, a
//! face of the base; shared edges; holes tangent to a side (inside and
//! outside), breaking out, ending exactly on the far face; pairs of holes
//! tangent to each other; tools a thousandth of the base; tiny and exact
//! quarter-turn rotations; and parts moved `1e4` to `1e7` mm away.
//!
//! **Judging** ([`judge`]) uses the layers that need no closed form: the
//! certified enclosure, the boundary shell test, the mesh invariants, and
//! the kernel's own validity check. Where two kernels both answer,
//! [`cross`] compares their volumes and their meshes' membership; the
//! per-kernel verdicts then say which one the reference finds wrong, or
//! that it cannot decide.
//!
//! **Shrinking** ([`shrink`]) drops operations, the part's motion, tool
//! rotations and counterbores, then simplifies every number (to zero, to
//! an integer, to fewer digits) while the failure persists.

use std::fmt::Write as _;

use cadview_core::solid::{
    CapSide, DefId, Depth, Feature, Op, PartDef, Placement, Plane3, Profile2d, Seg2, SolidKernel,
    ToolRef, Vec3,
};
use kurbo::Point;

use crate::cases::{boolean, chamfer_cap, cuboid, def, extrude, moved_def, PartCase, Tools};
use crate::layers::{enclosure, mesh_invariants, shell};
use crate::outcome::{Outcome, Verdict};
use crate::reference::Reference;
use crate::rng::Rng;
use crate::run::{run_part, Fail, PartRun, MEMBERSHIP_CHORD};

/// The enclosure budget per fuzz case (cell evaluations): a third of the
/// catalogue's, so a case stays well under a second.
pub const ENCLOSURE_BUDGET: usize = 10_000;

/// Relative volume tolerance between two kernels: the harness's.
pub const CROSS_REL_TOL: f64 = 1e-9;

// ── The recipe ────────────────────────────────────────────────────────

/// The base solid, with its box at `[0, L] x [0, W] x [0, H]`.
#[derive(Debug, Clone, PartialEq)]
pub enum Base {
    Box {
        size: [f64; 3],
    },
    /// A convex polygon in `[0, L] x [0, W]`, extruded `h`.
    Prism {
        pts: Vec<[f64; 2]>,
        h: f64,
    },
    /// A circle of radius `r` about `(r, r)`, extruded `h`.
    Cylinder {
        r: f64,
        h: f64,
    },
    /// A circle of radius `r` about `(r, r)` with a hole loop of radius
    /// `r_in`, extruded `h`.
    Washer {
        r: f64,
        r_in: f64,
        h: f64,
    },
}

/// A tool body in its own frame.
#[derive(Debug, Clone, PartialEq)]
pub enum Shape {
    /// `[0, sx] x [0, sy] x [0, sz]`.
    Box([f64; 3]),
    /// Radius `r` about the z axis, `z` in `[0, h]`.
    Cyl { r: f64, h: f64 },
}

/// One operation after the base.
#[derive(Debug, Clone, PartialEq)]
pub enum FOp {
    /// A boolean with a tool, rotated by `angle` about `axis` (through the
    /// tool's origin) and then moved to `at`.
    Tool {
        op: Op,
        shape: Shape,
        axis: [f64; 3],
        angle: f64,
        at: [f64; 3],
    },
    Hole {
        at: [f64; 3],
        dir: [f64; 3],
        dia: f64,
        /// `None`: through.
        depth: Option<f64>,
        cbore: Option<(f64, f64)>,
    },
    /// An equal-distance chamfer of the base's cap edges.
    Chamfer { d: f64, side: CapSide },
}

/// A generated recipe.
#[derive(Debug, Clone, PartialEq)]
pub struct Recipe {
    pub base: Base,
    pub ops: Vec<FOp>,
    /// The whole part moved by this (rotation axis, angle, translation).
    pub motion: Option<([f64; 3], f64, [f64; 3])>,
}

/// The generator seed of case `index` of `seed`.
pub fn mix(seed: u64, index: u64) -> u64 {
    let mut r = Rng::new(seed ^ 0x5eed_0000_0000_0000);
    let a = r.next_u64();
    a ^ index.wrapping_mul(0x9e37_79b9_7f4a_7c15).rotate_left(17)
}

/// The case's name: reproducible from the pair.
pub fn case_name(seed: u64, index: u64) -> String {
    format!("fuzz s{seed} i{index}")
}

impl Base {
    fn size(&self) -> [f64; 3] {
        match self {
            Base::Box { size } => *size,
            Base::Prism { pts, h } => {
                let mx = pts.iter().map(|p| p[0]).fold(0.0, f64::max);
                let my = pts.iter().map(|p| p[1]).fold(0.0, f64::max);
                [mx, my, *h]
            }
            Base::Cylinder { r, h } | Base::Washer { r, h, .. } => [2.0 * r, 2.0 * r, *h],
        }
    }

    fn feature(&self) -> Feature {
        match self {
            Base::Box { size } => cuboid(1, [0.0; 3], *size, Op::Add),
            Base::Prism { pts, h } => {
                let p: Vec<Point> = pts.iter().map(|q| Point::new(q[0], q[1])).collect();
                extrude(1, Profile2d::polygon(&p), Plane3::XY, *h, Op::Add)
            }
            Base::Cylinder { r, h } => extrude(
                1,
                Profile2d::circle(Point::new(*r, *r), *r),
                Plane3::XY,
                *h,
                Op::Add,
            ),
            Base::Washer { r, r_in, h } => extrude(
                1,
                Profile2d::circle(Point::new(*r, *r), *r)
                    .with_hole(vec![Seg2::Circle(kurbo::Circle::new((*r, *r), *r_in))]),
                Plane3::XY,
                *h,
                Op::Add,
            ),
        }
    }
}

impl Recipe {
    /// The recipe as a catalogue case.
    pub fn case(&self, name: &str) -> PartCase {
        let mut features = vec![self.base.feature()];
        let mut tools = Vec::new();
        for (k, op) in self.ops.iter().enumerate() {
            let id = 2 + k as u32;
            features.push(match op {
                FOp::Tool {
                    op,
                    shape,
                    axis,
                    angle,
                    at,
                } => {
                    let tid = 1000 + k as u64;
                    let f = match shape {
                        Shape::Box(s) => cuboid(1, [0.0; 3], *s, Op::Add),
                        Shape::Cyl { r, h } => extrude(
                            1,
                            Profile2d::circle(Point::new(0.0, 0.0), *r),
                            Plane3::XY,
                            *h,
                            Op::Add,
                        ),
                    };
                    tools.push(def(tid, "tool", vec![f]));
                    let p = if *angle == 0.0 {
                        Placement::translation(at[0], at[1], at[2])
                    } else {
                        Placement::axis_angle(
                            Vec3::new(axis[0], axis[1], axis[2]),
                            *angle,
                            Vec3::new(at[0], at[1], at[2]),
                        )
                    };
                    boolean(id, *op, tid, p)
                }
                FOp::Hole {
                    at,
                    dir,
                    dia,
                    depth,
                    cbore,
                } => Feature::Hole {
                    id: cadview_core::solid::FeatId(id),
                    at: cadview_core::solid::Point3::new(at[0], at[1], at[2]),
                    dir: Vec3::new(dir[0], dir[1], dir[2]),
                    dia: *dia,
                    depth: depth.map_or(Depth::Through, Depth::Blind),
                    cbore: *cbore,
                },
                FOp::Chamfer { d, side } => chamfer_cap(id, 1, *side, *d),
            });
        }
        let mut d = def(7_000, name, features);
        if let Some((axis, angle, t)) = self.motion {
            let m = Placement::axis_angle(
                Vec3::new(axis[0], axis[1], axis[2]),
                angle,
                Vec3::new(t[0], t[1], t[2]),
            );
            d = moved_def(&d, &m);
        }
        PartCase {
            name: name.into(),
            def: d,
            tools: Tools(tools),
            volume: None,
            bbox: None,
            euler: None,
        }
    }

    /// The recipe as one line of words and numbers (each number in
    /// Rust's shortest round-trip form), for handing to a child process.
    pub fn encode(&self) -> String {
        let n = |x: &f64| format!("{x:?}");
        let mut w: Vec<String> = Vec::new();
        match &self.base {
            Base::Box { size } => {
                w.push("box".into());
                w.extend(size.iter().map(n));
            }
            Base::Prism { pts, h } => {
                w.push("prism".into());
                w.push(pts.len().to_string());
                for p in pts {
                    w.extend(p.iter().map(n));
                }
                w.push(n(h));
            }
            Base::Cylinder { r, h } => w.extend(["cyl".into(), n(r), n(h)]),
            Base::Washer { r, r_in, h } => w.extend(["washer".into(), n(r), n(r_in), n(h)]),
        }
        for op in &self.ops {
            match op {
                FOp::Tool {
                    op,
                    shape,
                    axis,
                    angle,
                    at,
                } => {
                    w.push("tool".into());
                    w.push(format!("{op:?}"));
                    match shape {
                        Shape::Box(s) => {
                            w.push("b".into());
                            w.extend(s.iter().map(n));
                        }
                        Shape::Cyl { r, h } => w.extend(["c".into(), n(r), n(h)]),
                    }
                    w.extend(axis.iter().map(n));
                    w.push(n(angle));
                    w.extend(at.iter().map(n));
                }
                FOp::Hole {
                    at,
                    dir,
                    dia,
                    depth,
                    cbore,
                } => {
                    w.push("hole".into());
                    w.extend(at.iter().map(n));
                    w.extend(dir.iter().map(n));
                    w.push(n(dia));
                    w.push(depth.as_ref().map_or("through".into(), n));
                    match cbore {
                        None => w.push("nocb".into()),
                        Some((a, b)) => w.extend(["cb".into(), n(a), n(b)]),
                    }
                }
                FOp::Chamfer { d, side } => w.extend(["chamfer".into(), n(d), format!("{side:?}")]),
            }
        }
        if let Some((axis, angle, t)) = &self.motion {
            w.push("motion".into());
            w.extend(axis.iter().map(n));
            w.push(n(angle));
            w.extend(t.iter().map(n));
        }
        w.join(" ")
    }

    /// The inverse of [`Recipe::encode`].
    pub fn decode(s: &str) -> Option<Recipe> {
        let mut it = s.split_whitespace();
        let num = |it: &mut std::str::SplitWhitespace| -> Option<f64> { it.next()?.parse().ok() };
        let base = match it.next()? {
            "box" => Base::Box {
                size: [num(&mut it)?, num(&mut it)?, num(&mut it)?],
            },
            "prism" => {
                let k: usize = it.next()?.parse().ok()?;
                let mut pts = Vec::with_capacity(k);
                for _ in 0..k {
                    pts.push([num(&mut it)?, num(&mut it)?]);
                }
                Base::Prism {
                    pts,
                    h: num(&mut it)?,
                }
            }
            "cyl" => Base::Cylinder {
                r: num(&mut it)?,
                h: num(&mut it)?,
            },
            "washer" => Base::Washer {
                r: num(&mut it)?,
                r_in: num(&mut it)?,
                h: num(&mut it)?,
            },
            _ => return None,
        };
        let mut ops = Vec::new();
        let mut motion = None;
        while let Some(word) = it.next() {
            match word {
                "tool" => {
                    let op = match it.next()? {
                        "Add" => Op::Add,
                        "Subtract" => Op::Subtract,
                        "Intersect" => Op::Intersect,
                        _ => return None,
                    };
                    let shape = match it.next()? {
                        "b" => Shape::Box([num(&mut it)?, num(&mut it)?, num(&mut it)?]),
                        "c" => Shape::Cyl {
                            r: num(&mut it)?,
                            h: num(&mut it)?,
                        },
                        _ => return None,
                    };
                    let axis = [num(&mut it)?, num(&mut it)?, num(&mut it)?];
                    let angle = num(&mut it)?;
                    let at = [num(&mut it)?, num(&mut it)?, num(&mut it)?];
                    ops.push(FOp::Tool {
                        op,
                        shape,
                        axis,
                        angle,
                        at,
                    });
                }
                "hole" => {
                    let at = [num(&mut it)?, num(&mut it)?, num(&mut it)?];
                    let dir = [num(&mut it)?, num(&mut it)?, num(&mut it)?];
                    let dia = num(&mut it)?;
                    let depth = match it.next()? {
                        "through" => None,
                        d => Some(d.parse().ok()?),
                    };
                    let cbore = match it.next()? {
                        "nocb" => None,
                        "cb" => Some((num(&mut it)?, num(&mut it)?)),
                        _ => return None,
                    };
                    ops.push(FOp::Hole {
                        at,
                        dir,
                        dia,
                        depth,
                        cbore,
                    });
                }
                "chamfer" => {
                    let d = num(&mut it)?;
                    let side = match it.next()? {
                        "Start" => CapSide::Start,
                        "End" => CapSide::End,
                        "Both" => CapSide::Both,
                        _ => return None,
                    };
                    ops.push(FOp::Chamfer { d, side });
                }
                "motion" => {
                    let axis = [num(&mut it)?, num(&mut it)?, num(&mut it)?];
                    let angle = num(&mut it)?;
                    let t = [num(&mut it)?, num(&mut it)?, num(&mut it)?];
                    motion = Some((axis, angle, t));
                }
                _ => return None,
            }
        }
        Some(Recipe { base, ops, motion })
    }

    /// Every number of the recipe, for the shrinker.
    fn numbers_mut(&mut self) -> Vec<&mut f64> {
        let mut out: Vec<&mut f64> = Vec::new();
        match &mut self.base {
            Base::Box { size } => out.extend(size.iter_mut()),
            Base::Prism { pts, h } => {
                for p in pts.iter_mut() {
                    out.extend(p.iter_mut());
                }
                out.push(h);
            }
            Base::Cylinder { r, h } => out.extend([r, h]),
            Base::Washer { r, r_in, h } => out.extend([r, r_in, h]),
        }
        for op in &mut self.ops {
            match op {
                FOp::Tool {
                    shape,
                    axis,
                    angle,
                    at,
                    ..
                } => {
                    match shape {
                        Shape::Box(s) => out.extend(s.iter_mut()),
                        Shape::Cyl { r, h } => out.extend([r, h]),
                    }
                    out.extend(at.iter_mut());
                    out.push(angle);
                    out.extend(axis.iter_mut());
                }
                FOp::Hole {
                    at,
                    dir,
                    dia,
                    depth,
                    cbore,
                } => {
                    out.extend(at.iter_mut());
                    out.push(dia);
                    if let Some(d) = depth {
                        out.push(d);
                    }
                    if let Some((a, b)) = cbore {
                        out.extend([a, b]);
                    }
                    out.extend(dir.iter_mut());
                }
                FOp::Chamfer { d, .. } => out.push(d),
            }
        }
        if let Some((axis, angle, t)) = &mut self.motion {
            out.extend(t.iter_mut());
            out.push(angle);
            out.extend(axis.iter_mut());
        }
        out
    }
}

// ── Generation ────────────────────────────────────────────────────────

/// The draws.
struct Gen {
    rng: Rng,
}

impl Gen {
    fn unit(&mut self) -> f64 {
        self.rng.unit()
    }
    fn chance(&mut self, p: f64) -> bool {
        self.rng.unit() < p
    }
    fn pick<T: Copy>(&mut self, xs: &[T]) -> T {
        xs[self.rng.below(xs.len())]
    }
    /// Log-uniform in `[lo, hi]`.
    fn log(&mut self, lo: f64, hi: f64) -> f64 {
        (lo.ln() + (hi.ln() - lo.ln()) * self.unit()).exp()
    }
    /// A size: mostly round numbers in `[1, 200]`, sometimes tiny or huge.
    fn size(&mut self) -> f64 {
        match self.rng.below(10) {
            0 => self.log(1e-2, 1.0),
            1 => self.log(200.0, 5e3),
            2..=5 => (1 + self.rng.below(200)) as f64,
            _ => self.log(1.0, 200.0),
        }
    }
    /// An offset from a contact: zero, a hair (1e-12 to 1e-3 mm) either
    /// way, or a clear amount. Biased to the hard values.
    fn eps(&mut self, scale: f64) -> f64 {
        let s = if self.chance(0.5) { 1.0 } else { -1.0 };
        match self.rng.below(12) {
            0..=3 => 0.0,
            4 => s * 1e-12,
            5 => s * 1e-9,
            6 => s * 1e-7,
            7 => s * 1e-6,
            8 => s * 1e-5,
            9 => s * 1e-3,
            10 => s * self.log(1e-12, 1e-3),
            _ => s * scale * self.unit(),
        }
    }
    /// A rotation angle: none, tiny, a quarter turn, or generic.
    fn angle(&mut self) -> f64 {
        match self.rng.below(10) {
            0..=4 => 0.0,
            5 => self.log(1e-12, 1e-3) * if self.chance(0.5) { 1.0 } else { -1.0 },
            6 => std::f64::consts::FRAC_PI_2,
            7 => std::f64::consts::FRAC_PI_4,
            _ => self.rng.range(-std::f64::consts::PI, std::f64::consts::PI),
        }
    }
    fn axis(&mut self) -> [f64; 3] {
        match self.rng.below(5) {
            0 => [1.0, 0.0, 0.0],
            1 => [0.0, 1.0, 0.0],
            2 => [0.0, 0.0, 1.0],
            _ => {
                let a = [
                    self.rng.range(-1.0, 1.0),
                    self.rng.range(-1.0, 1.0),
                    self.rng.range(-1.0, 1.0),
                ];
                if a.iter().all(|x| x.abs() < 1e-3) {
                    [0.0, 0.0, 1.0]
                } else {
                    a
                }
            }
        }
    }
}

/// A convex polygon in `[0, l] x [0, w]`: points on an ellipse at sorted
/// random angles, then moved so its box starts at the origin.
fn convex_polygon(g: &mut Gen, l: f64, w: f64) -> Vec<[f64; 2]> {
    let n = 3 + g.rng.below(5);
    let mut angles: Vec<f64> = (0..n)
        .map(|k| (k as f64 + 0.15 + 0.7 * g.unit()) * std::f64::consts::TAU / n as f64)
        .collect();
    angles.sort_by(f64::total_cmp);
    let pts: Vec<[f64; 2]> = angles
        .iter()
        .map(|t| [0.5 * l * (1.0 + t.cos()), 0.5 * w * (1.0 + t.sin())])
        .collect();
    let mx = pts.iter().map(|p| p[0]).fold(f64::INFINITY, f64::min);
    let my = pts.iter().map(|p| p[1]).fold(f64::INFINITY, f64::min);
    pts.iter().map(|p| [p[0] - mx, p[1] - my]).collect()
}

/// Case `index` of seed `seed`.
pub fn generate(seed: u64, index: u64) -> Recipe {
    let mut g = Gen {
        rng: Rng::new(mix(seed, index)),
    };
    let (l, w, h) = (g.size(), g.size(), g.size());
    let base = match g.rng.below(10) {
        0..=5 => Base::Box { size: [l, w, h] },
        6 | 7 => Base::Prism {
            pts: convex_polygon(&mut g, l, w),
            h,
        },
        8 => Base::Cylinder { r: 0.5 * l, h },
        _ => Base::Washer {
            r: 0.5 * l,
            r_in: 0.5 * l * g.rng.range(0.05, 0.95),
            h,
        },
    };
    let [bl, bw, bh] = base.size();
    let mut ops = Vec::new();
    // A cap chamfer, only directly after the base and only on polygons.
    if matches!(base, Base::Box { .. } | Base::Prism { .. }) && g.chance(0.2) {
        let m = bl.min(bw).min(bh);
        let d = match g.rng.below(4) {
            0 => g.log(1e-4, 1e-2),
            1 => (0.5 * m * 0.99 + g.eps(1e-3)).max(1e-6),
            _ => m * g.rng.range(0.01, 0.4),
        };
        let side = g.pick(&[CapSide::Start, CapSide::End, CapSide::Both]);
        ops.push(FOp::Chamfer {
            d: if side == CapSide::Both { d * 0.45 } else { d },
            side,
        });
    }
    let n = 1 + g.rng.below(4);
    for _ in 0..n {
        let kind = g.rng.below(10);
        if kind < 6 {
            // A tool with a face flush with (or a hair off) a face of the
            // base, or across it.
            let op = g.pick(&[Op::Subtract, Op::Subtract, Op::Add, Op::Intersect]);
            let tiny = g.chance(0.1);
            let scale = if tiny { 1e-3 } else { 1.0 };
            let shape = if g.chance(0.7) {
                Shape::Box([
                    (g.size() * scale).max(1e-6),
                    (g.size() * scale).max(1e-6),
                    (g.size() * scale).max(1e-6),
                ])
            } else {
                Shape::Cyl {
                    r: (0.5 * g.size() * scale).max(1e-6),
                    h: (g.size() * scale).max(1e-6),
                }
            };
            let ext = match &shape {
                Shape::Box(s) => *s,
                Shape::Cyl { r, h } => [2.0 * r, 2.0 * r, *h],
            };
            let mut at = [0.0; 3];
            for k in 0..3 {
                let size = [bl, bw, bh][k];
                // An intersection with a tool outside the base is empty:
                // keep intersecting tools at a coplanar face or across.
                let pick = if op == Op::Intersect {
                    2 + g.rng.below(4)
                } else {
                    g.rng.below(6)
                };
                at[k] = match pick {
                    // the tool's low face on the base's high face (outside)
                    0 => size + g.eps(size),
                    // its high face on the base's low face
                    1 => -ext[k] + g.eps(size),
                    // its low face on the base's low face (coplanar)
                    2 => g.eps(size),
                    // its high face on the base's high face (coplanar)
                    3 => size - ext[k] + g.eps(size),
                    _ => g.rng.range(-0.5 * ext[k], size),
                };
                if let Shape::Cyl { r, .. } = shape {
                    if k < 2 {
                        // the cylinder's axis is at its local origin
                        at[k] += r;
                    }
                }
            }
            let angle = g.angle();
            let axis = g.axis();
            ops.push(FOp::Tool {
                op,
                shape,
                axis,
                angle,
                at,
            });
        } else {
            // A hole from the top face down (or a side face in), tangent
            // to a side, breaking out, ending on the far face, or a pair
            // of tangent holes.
            let r = match g.rng.below(4) {
                0 => g.log(1e-3, 1e-1),
                _ => 0.5 * g.size().min(0.9 * bl.min(bw)).max(1e-3),
            };
            let along_x = g.chance(0.15);
            let (span_a, span_b, depth_full) = if along_x { (bw, bh, bl) } else { (bl, bw, bh) };
            let place = |g: &mut Gen, span: f64| match g.rng.below(5) {
                0 => r + g.eps(r),        // tangent to the low side
                1 => span - r + g.eps(r), // tangent to the high side
                2 => g.eps(r),            // half out
                _ => g.rng.range(0.0, span),
            };
            let a = place(&mut g, span_a);
            let b = place(&mut g, span_b);
            let depth = match g.rng.below(4) {
                0 => None,
                1 => Some(depth_full + g.eps(depth_full)),
                _ => Some(depth_full * g.rng.range(0.05, 0.95)),
            };
            let depth = depth.filter(|d| *d > 0.0);
            let cbore = if g.chance(0.2) {
                let cd = 2.0 * r * g.rng.range(1.1, 2.5);
                let cdepth = depth.unwrap_or(depth_full) * g.rng.range(0.05, 0.6);
                Some((cd, cdepth))
            } else {
                None
            };
            let (at, dir) = if along_x {
                ([0.0, a, b], [1.0, 0.0, 0.0])
            } else {
                ([a, b, bh], [0.0, 0.0, -1.0])
            };
            ops.push(FOp::Hole {
                at,
                dir,
                dia: 2.0 * r,
                depth,
                cbore,
            });
            if g.chance(0.3) {
                // A second hole tangent to the first.
                let s = 2.0 * r + g.eps(r);
                let at2 = if along_x {
                    [0.0, a + s, b]
                } else {
                    [a + s, b, bh]
                };
                ops.push(FOp::Hole {
                    at: at2,
                    dir,
                    dia: 2.0 * r,
                    depth,
                    cbore: None,
                });
            }
        }
    }
    ops.truncate(5);
    let motion = match g.rng.below(10) {
        0..=5 => None,
        6 | 7 => {
            let far = g.log(1e4, 1e7);
            Some(([0.0, 0.0, 1.0], 0.0, [far, -0.5 * far, 0.25 * far]))
        }
        _ => {
            let axis = g.axis();
            let angle = g.rng.range(-3.0, 3.0);
            let t = [
                g.rng.range(-100.0, 100.0),
                g.rng.range(-100.0, 100.0),
                g.rng.range(-100.0, 100.0),
            ];
            Some((axis, angle, t))
        }
    };
    Recipe { base, ops, motion }
}

// ── Judging ───────────────────────────────────────────────────────────

/// One kernel's result on one case.
#[derive(Debug, Clone)]
pub struct Judgement {
    pub kernel: String,
    pub outcome: Outcome,
    /// `Some` when the kernel returned a solid with a volume.
    pub volume: Option<f64>,
    /// Enclosure, shell, mesh, validity.
    pub verdicts: [Verdict; 4],
    pub panic: Option<String>,
    /// Why the kernel refused, if it did.
    pub refusal: Option<String>,
    pub eval_ms: f64,
}

pub const JUDGED: [&str; 4] = ["enclosure", "shell", "mesh", "validity"];

impl Judgement {
    /// The first layer that says wrong, with its detail.
    pub fn first_wrong(&self) -> Option<(&'static str, &str)> {
        if let Some(p) = &self.panic {
            return Some(("panic", p));
        }
        JUDGED
            .iter()
            .zip(&self.verdicts)
            .find(|(_, v)| v.outcome == Outcome::Wrong)
            .map(|(n, v)| (*n, v.detail.as_str()))
    }
}

/// Judges `case` on `kernel`, with its run (for [`cross`]).
pub fn judge<K: SolidKernel>(kernel: &K, case: &PartCase) -> (Judgement, PartRun<K::Brep>) {
    let run = run_part(kernel, case, true);
    let enc = run
        .reference
        .as_ref()
        .ok()
        .and_then(|r| enclosure::enclose(r, ENCLOSURE_BUDGET, enclosure::REL_TARGET));
    let empty = enc.is_some_and(|e| e.lo == 0.0);
    let verdicts = [
        enclosure::check(&run, enc.as_ref()),
        shell::check(&run),
        mesh_invariants::check(&run),
        crate::layers::validity::check(&run, empty),
    ];
    let mut outcome = crate::outcome::overall(&verdicts, run.refused());
    let panic = run.panicked().map(str::to_string);
    if panic.is_some() {
        outcome = Outcome::Wrong;
    }
    let refusal = match &run.brep {
        Err(Fail::Refused(s) | Fail::Unsupported(s)) => Some(s.clone()),
        _ => None,
    };
    let volume = run.mass.as_ref().ok().map(|m| m.volume);
    (
        Judgement {
            kernel: kernel.name(),
            outcome,
            volume,
            verdicts,
            panic,
            refusal,
            eval_ms: run.eval_ms,
        },
        run,
    )
}

/// How two kernels' answers on one case compare.
#[derive(Debug, Clone, PartialEq)]
pub enum Cross {
    /// Both answered and agree (volume and sampled membership).
    Agree,
    /// One refused (or panicked), the other answered.
    OneAnswered,
    /// Neither answered.
    NeitherAnswered,
    /// They disagree; `wrong` is the kernel the reference finds wrong
    /// (by name), or `None` when the reference cannot decide.
    Disagree { wrong: Vec<String>, detail: String },
}

/// Compares two kernels' runs of one case.
pub fn cross<A, B>(ja: &Judgement, ra: &PartRun<A>, jb: &Judgement, rb: &PartRun<B>) -> Cross {
    let (Some(va), Some(vb)) = (ja.volume, jb.volume) else {
        return if ra.brep.is_ok() || rb.brep.is_ok() {
            Cross::OneAnswered
        } else {
            Cross::NeitherAnswered
        };
    };
    let mut why = Vec::new();
    let scale = va.abs().max(vb.abs());
    if (va - vb).abs() > CROSS_REL_TOL * scale + 1e-12 {
        why.push(format!(
            "volumes {va} and {vb} differ by {:.2e} ({:.1e} relative)",
            va - vb,
            (va - vb).abs() / scale.max(1e-300)
        ));
    }
    let mesh = |r: &Result<cadview_core::solid::TriMesh, Fail>| r.as_ref().ok().cloned();
    if let (Some(Some(ma)), Some(Some(mb))) = (
        ra.mesh(MEMBERSHIP_CHORD).map(mesh),
        rb.mesh(MEMBERSHIP_CHORD).map(mesh),
    ) {
        let v = crate::layers::equivalence::membership_meshes(
            &ma,
            &mb,
            crate::layers::equivalence::Map::Same,
            &ra.case.name,
            (ja.kernel.as_str(), jb.kernel.as_str()),
        );
        if v.outcome == Outcome::Wrong {
            why.push(v.detail);
        }
    }
    if why.is_empty() {
        return Cross::Agree;
    }
    let mut wrong: Vec<String> = [ja, jb]
        .iter()
        .filter(|j| j.outcome == Outcome::Wrong)
        .map(|j| j.kernel.clone())
        .collect();
    if wrong.is_empty() {
        // Neither layer flagged a kernel: ask a finer enclosure.
        if let Ok(r) = &ra.reference {
            if let Some(e) = enclosure::enclose(r, DECIDE_BUDGET, 1e-12) {
                why.push(format!(
                    "finer enclosure [{:.9e}, {:.9e}] (rel width {:.1e})",
                    e.lo,
                    e.hi,
                    e.rel_width()
                ));
                for (j, v) in [(ja, va), (jb, vb)] {
                    let slack = crate::layers::closed_form::VOLUME_REL_TOL * v.abs();
                    if v < e.lo - slack || v > e.hi + slack {
                        wrong.push(format!("{} (finer enclosure)", j.kernel));
                    }
                }
            }
        }
    }
    Cross::Disagree {
        wrong,
        detail: why.join("; "),
    }
}

/// The enclosure budget [`cross`] spends on a disagreement neither
/// kernel's layers decided: forty times a fuzz case's.
pub const DECIDE_BUDGET: usize = 400_000;

// ── Shrinking ─────────────────────────────────────────────────────────

/// Simpler values to try for a number, simplest first.
fn simpler(x: f64) -> Vec<f64> {
    let mut out = vec![0.0];
    if x.is_finite() {
        out.push(x.round());
        for digits in [1, 2, 3, 6] {
            let p = 10f64.powi(digits);
            out.push((x * p).round() / p);
        }
        // Keep a hair as a hair, but a round one.
        if x != 0.0 && x.abs() < 1e-2 {
            let e = x.abs().log10().round();
            out.push(x.signum() * 10f64.powf(e));
        }
    }
    out.retain(|y| *y != x && y.is_finite());
    out.dedup();
    out
}

/// The smallest recipe `fails` still holds for: operations dropped, the
/// motion, rotations and counterbores dropped, numbers simplified.
/// `fails` must hold for `r`. `report` sees every improvement.
pub fn shrink(
    r: &Recipe,
    fails: &mut dyn FnMut(&Recipe) -> bool,
    report: &mut dyn FnMut(&Recipe),
) -> Recipe {
    let mut best = r.clone();
    loop {
        let mut changed = false;
        // Drop operations, last first.
        let mut i = best.ops.len();
        while i > 0 {
            i -= 1;
            let mut t = best.clone();
            t.ops.remove(i);
            if fails(&t) {
                best = t;
                changed = true;
                report(&best);
            }
        }
        // Drop the motion, rotations and counterbores.
        if best.motion.is_some() {
            let mut t = best.clone();
            t.motion = None;
            if fails(&t) {
                best = t;
                changed = true;
                report(&best);
            }
        }
        for k in 0..best.ops.len() {
            let mut t = best.clone();
            let mut any = false;
            match &mut t.ops[k] {
                FOp::Tool { angle, .. } if *angle != 0.0 => {
                    *angle = 0.0;
                    any = true;
                }
                FOp::Hole { cbore, .. } if cbore.is_some() => {
                    *cbore = None;
                    any = true;
                }
                _ => {}
            }
            if any && fails(&t) {
                best = t;
                changed = true;
                report(&best);
            }
        }
        // A simpler base: a prism, cylinder or washer as a box.
        if !matches!(best.base, Base::Box { .. }) {
            let mut t = best.clone();
            t.base = Base::Box {
                size: best.base.size(),
            };
            if fails(&t) {
                best = t;
                changed = true;
                report(&best);
            }
        }
        // Simplify numbers.
        let count = best.clone().numbers_mut().len();
        for k in 0..count {
            let x = *best.clone().numbers_mut()[k];
            for y in simpler(x) {
                let mut t = best.clone();
                *t.numbers_mut()[k] = y;
                if fails(&t) {
                    best = t;
                    changed = true;
                    report(&best);
                    break;
                }
            }
        }
        if !changed {
            return best;
        }
    }
}

// ── Printing a case for the catalogue ─────────────────────────────────

fn f(x: f64) -> String {
    let s = format!("{x:?}");
    if s.contains('.') || s.contains('e') || s.contains("inf") || s.contains("NaN") {
        s
    } else {
        format!("{s}.0")
    }
}

fn pt3(p: cadview_core::solid::Point3) -> String {
    format!("Point3::new({}, {}, {})", f(p.x), f(p.y), f(p.z))
}

fn vec3(v: Vec3) -> String {
    format!("Vec3::new({}, {}, {})", f(v.x), f(v.y), f(v.z))
}

fn seg(s: &Seg2) -> String {
    match s {
        Seg2::Line(l) => format!(
            "Seg2::Line(Line::new(({}, {}), ({}, {})))",
            f(l.p0.x),
            f(l.p0.y),
            f(l.p1.x),
            f(l.p1.y)
        ),
        Seg2::Circle(c) => format!(
            "Seg2::Circle(kurbo::Circle::new(({}, {}), {}))",
            f(c.center.x),
            f(c.center.y),
            f(c.radius)
        ),
        Seg2::Arc(a) => format!(
            "Seg2::Arc(kurbo::Arc {{ center: Point::new({}, {}), radii: kurbo::Vec2::new({}, {}), start_angle: {}, sweep_angle: {}, x_rotation: {} }})",
            f(a.center.x),
            f(a.center.y),
            f(a.radii.x),
            f(a.radii.y),
            f(a.start_angle),
            f(a.sweep_angle),
            f(a.x_rotation)
        ),
        Seg2::Spline { .. } => "/* spline segments are not generated */".into(),
    }
}

fn feature(ft: &Feature) -> String {
    match ft {
        Feature::Extrude {
            id,
            profile,
            plane,
            distance,
            taper_deg,
            op,
        } => {
            let lp = |l: &Vec<Seg2>| {
                format!(
                    "vec![{}]",
                    l.iter().map(seg).collect::<Vec<_>>().join(", ")
                )
            };
            format!(
                "Feature::Extrude {{ id: FeatId({}), profile: Profile2d {{ outer: {}, holes: vec![{}] }}, plane: Plane3 {{ origin: {}, x_axis: {}, normal: {} }}, distance: {}, taper_deg: {}, op: Op::{:?} }}",
                id.0,
                lp(&profile.outer),
                profile.holes.iter().map(lp).collect::<Vec<_>>().join(", "),
                pt3(plane.origin),
                vec3(plane.x_axis),
                vec3(plane.normal),
                f(*distance),
                f(*taper_deg),
                op
            )
        }
        Feature::Hole {
            id,
            at,
            dir,
            dia,
            depth,
            cbore,
        } => format!(
            "Feature::Hole {{ id: FeatId({}), at: {}, dir: {}, dia: {}, depth: {}, cbore: {} }}",
            id.0,
            pt3(*at),
            vec3(*dir),
            f(*dia),
            match depth {
                Depth::Through => "Depth::Through".to_string(),
                Depth::Blind(x) => format!("Depth::Blind({})", f(*x)),
            },
            match cbore {
                None => "None".to_string(),
                Some((a, b)) => format!("Some(({}, {}))", f(*a), f(*b)),
            }
        ),
        Feature::Boolean {
            id,
            op,
            tool: ToolRef::Def(t, p),
        } => format!(
            "boolean({}, Op::{:?}, {}, Placement {{ rotation: [{}, {}, {}, {}], translation: {} }})",
            id.0,
            op,
            t.0,
            f(p.rotation[0]),
            f(p.rotation[1]),
            f(p.rotation[2]),
            f(p.rotation[3]),
            vec3(p.translation)
        ),
        Feature::Chamfer {
            id,
            edges: cadview_core::solid::EdgeSel::Cap { feature, side, .. },
            d1,
            ..
        } => format!(
            "chamfer_cap({}, {}, CapSide::{:?}, {})",
            id.0,
            feature.0,
            side,
            f(*d1)
        ),
        other => format!("/* not printed: {} */", other.kind_name()),
    }
}

fn part_def(d: &PartDef, name_expr: &str) -> String {
    let mut s = String::new();
    let _ = writeln!(s, "def(");
    let _ = writeln!(s, "    {},", d.id.0);
    let _ = writeln!(s, "    {name_expr},");
    let _ = writeln!(s, "    vec![");
    for ft in &d.features {
        let _ = writeln!(s, "        {},", feature(ft));
    }
    let _ = write!(s, "    ],\n)");
    s
}

/// A catalogue case for a recipe, as Rust source for `src/cases.rs`
/// (a function body returning a `PartCase`, `name` its case name).
pub fn emit(case: &PartCase, fn_name: &str, why: &str) -> String {
    let mut s = String::new();
    let _ = writeln!(s, "/// {why}");
    let _ = writeln!(s, "fn {fn_name}() -> PartCase {{");
    let _ = writeln!(s, "    let name = {:?};", case.name);
    let _ = writeln!(s, "    let tools = vec![");
    for t in &case.tools.0 {
        let body = part_def(t, &format!("{:?}", t.name));
        for line in body.lines() {
            let _ = writeln!(s, "        {line}");
        }
        let _ = writeln!(s, "        ,");
    }
    let _ = writeln!(s, "    ];");
    let body = part_def(&case.def, "name");
    let _ = writeln!(s, "    part(");
    let _ = writeln!(s, "        name,");
    for line in body.lines() {
        let _ = writeln!(s, "        {line}");
    }
    let _ = writeln!(s, "        ,\n        tools,\n    )");
    let _ = writeln!(s, "}}");
    s
}

/// The ids of a case's tools, for checking that a printed case is whole.
pub fn tool_ids(case: &PartCase) -> Vec<DefId> {
    case.tools.0.iter().map(|t| t.id).collect()
}

/// Whether the reference defines the recipe (a generated recipe it does
/// not define is skipped, not judged).
pub fn defined(case: &PartCase) -> bool {
    Reference::build(&case.def, &case.tools).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Generation is a function of (seed, index) alone.
    #[test]
    fn reproducible() {
        for i in 0..200 {
            assert_eq!(generate(7, i), generate(7, i));
            let a = generate(7, i).case(&case_name(7, i));
            let b = generate(7, i).case(&case_name(7, i));
            assert_eq!(a.def, b.def);
            assert_eq!(a.tools.0, b.tools.0);
        }
        assert_ne!(generate(7, 1), generate(8, 1));
        for i in 0..500 {
            let r = generate(3, i);
            assert_eq!(Recipe::decode(&r.encode()), Some(r));
        }
    }
}
