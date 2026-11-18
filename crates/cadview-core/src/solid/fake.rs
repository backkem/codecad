//! A small closed-form kernel for core's tests, so they run without a real
//! one. It knows axis-aligned boxes and cylinders: rectangle and circle
//! extrudes on planes whose axes are world axes, holes along a world axis,
//! and booleans with a tool definition, where every subtracted piece lies
//! inside the material and added pieces do not overlap (the tests build
//! only such parts). Anything else is `Unsupported`, which the tests use
//! as a refusal.

use std::f64::consts::PI;

use super::kernel::{
    Aabb3, Closest, EvalError, Evaluated, Interference, MassProps, Placed, SolidKernel,
    ToolResolver, TriMesh, Validity,
};
use super::part::{apply_point, apply_vec};
use super::types::{Depth, FeatId, Feature, Op, PartDef, Placement, Plane3, Point3, Seg2, ToolRef};

#[derive(Debug, Clone, Copy, Default)]
pub struct FakeKernel;

/// One closed-form piece: its box, volume and centroid, added (+1) or
/// taken away (-1).
#[derive(Debug, Clone, Copy)]
pub struct Piece {
    pub bbox: Aabb3,
    pub volume: f64,
    pub centroid: Point3,
    pub sign: f64,
    /// An axis-aligned box (volume = its bbox), so a subtracted one can be
    /// clipped to the material it overlaps.
    pub boxy: bool,
}

#[derive(Debug, Clone, Default)]
pub struct FakeBrep {
    pub pieces: Vec<Piece>,
}

fn aabb_of(points: impl Iterator<Item = Point3>) -> Aabb3 {
    let mut lo = Point3::new(f64::MAX, f64::MAX, f64::MAX);
    let mut hi = Point3::new(f64::MIN, f64::MIN, f64::MIN);
    for p in points {
        lo = Point3::new(lo.x.min(p.x), lo.y.min(p.y), lo.z.min(p.z));
        hi = Point3::new(hi.x.max(p.x), hi.y.max(p.y), hi.z.max(p.z));
    }
    Aabb3::new(lo, hi)
}

fn corners(b: &Aabb3) -> [Point3; 8] {
    std::array::from_fn(|i| {
        Point3::new(
            if i & 1 == 0 { b.min.x } else { b.max.x },
            if i & 2 == 0 { b.min.y } else { b.max.y },
            if i & 4 == 0 { b.min.z } else { b.max.z },
        )
    })
}

fn place_piece(p: &Placement, pc: &Piece) -> Piece {
    Piece {
        bbox: aabb_of(corners(&pc.bbox).into_iter().map(|c| apply_point(p, c))),
        centroid: apply_point(p, pc.centroid),
        ..*pc
    }
}

fn axis_aligned(v: super::types::Vec3) -> bool {
    let n = [v.x.abs(), v.y.abs(), v.z.abs()];
    n.iter().filter(|x| **x > 1e-12).count() == 1
}

fn body(
    plane: &Plane3,
    profile: &super::types::Profile2d,
    d: f64,
    f: FeatId,
) -> Result<Piece, EvalError> {
    let unsupported = || EvalError::unsupported(Some(f), "this profile in the fake kernel");
    if !profile.holes.is_empty() || !axis_aligned(plane.x_axis) || !axis_aligned(plane.normal) {
        return Err(unsupported());
    }
    let frame = plane.frame()?;
    match profile.outer.as_slice() {
        [Seg2::Circle(c)] => {
            let base = frame.at(c.center, 0.0);
            let r = c.radius;
            let ends = [base, base + frame.z.scale(d)];
            let pts = ends
                .into_iter()
                .flat_map(|e| [frame.x, -frame.x, frame.y, -frame.y].map(|v| e + v.scale(r)));
            Ok(Piece {
                bbox: aabb_of(pts),
                volume: PI * r * r * d.abs(),
                centroid: base + frame.z.scale(d / 2.0),
                sign: 1.0,
                boxy: false,
            })
        }
        segs if segs.len() == 4 && segs.iter().all(|s| matches!(s, Seg2::Line(_))) => {
            let pts: Vec<kurbo::Point> = segs
                .iter()
                .map(|s| match s {
                    Seg2::Line(l) => l.p0,
                    _ => unreachable!(),
                })
                .collect();
            let (u0, u1) = (
                pts.iter().map(|p| p.x).fold(f64::MAX, f64::min),
                pts.iter().map(|p| p.x).fold(f64::MIN, f64::max),
            );
            let (v0, v1) = (
                pts.iter().map(|p| p.y).fold(f64::MAX, f64::min),
                pts.iter().map(|p| p.y).fold(f64::MIN, f64::max),
            );
            // Only axis-aligned rectangles.
            let rect = pts.iter().all(|p| {
                ((p.x - u0).abs() < 1e-12 || (p.x - u1).abs() < 1e-12)
                    && ((p.y - v0).abs() < 1e-12 || (p.y - v1).abs() < 1e-12)
            });
            if !rect {
                return Err(unsupported());
            }
            let b = aabb_of(
                [u0, u1]
                    .into_iter()
                    .flat_map(|u| [v0, v1].map(move |v| (u, v)))
                    .flat_map(|(u, v)| [0.0, d].map(move |w| frame.at(kurbo::Point::new(u, v), w))),
            );
            let c = Point3::new(
                (b.min.x + b.max.x) / 2.0,
                (b.min.y + b.max.y) / 2.0,
                (b.min.z + b.max.z) / 2.0,
            );
            Ok(Piece {
                bbox: b,
                volume: (u1 - u0) * (v1 - v0) * d.abs(),
                centroid: c,
                sign: 1.0,
                boxy: true,
            })
        }
        _ => Err(unsupported()),
    }
}

fn eval(def: &PartDef, tools: &dyn ToolResolver, depth: usize) -> Result<FakeBrep, EvalError> {
    eval_from(def, tools, depth, FakeBrep::default(), 0)
}

/// `def` from feature `start` on, on top of `b` (its first `start`
/// features).
fn eval_from(
    def: &PartDef,
    tools: &dyn ToolResolver,
    depth: usize,
    mut b: FakeBrep,
    start: usize,
) -> Result<FakeBrep, EvalError> {
    if depth > 16 {
        return Err(EvalError::invalid(None, "tools nest too deep"));
    }
    for f in def.features.iter().skip(start) {
        let id = f.id();
        let combine = |b: &mut FakeBrep, mut pieces: Vec<Piece>, op: Op| -> Result<(), EvalError> {
            match op {
                Op::Add => {}
                Op::Subtract => {
                    if b.pieces.is_empty() {
                        return Err(EvalError::invalid(Some(id), "nothing to subtract from"));
                    }
                    let hull = b.hull();
                    for p in &mut pieces {
                        p.sign = -p.sign;
                        if p.boxy && p.sign < 0.0 {
                            // Only the overlap with the material is taken away.
                            let lo = Point3::new(
                                p.bbox.min.x.max(hull.min.x),
                                p.bbox.min.y.max(hull.min.y),
                                p.bbox.min.z.max(hull.min.z),
                            );
                            let hi = Point3::new(
                                p.bbox.max.x.min(hull.max.x),
                                p.bbox.max.y.min(hull.max.y),
                                p.bbox.max.z.min(hull.max.z),
                            );
                            let e = [hi.x - lo.x, hi.y - lo.y, hi.z - lo.z].map(|v| v.max(0.0));
                            p.bbox = Aabb3::new(lo, hi);
                            p.volume = e[0] * e[1] * e[2];
                            p.centroid = Point3::new(
                                (lo.x + hi.x) / 2.0,
                                (lo.y + hi.y) / 2.0,
                                (lo.z + hi.z) / 2.0,
                            );
                        }
                    }
                }
                Op::Intersect => return Err(EvalError::unsupported(Some(id), "intersect")),
            }
            b.pieces.extend(pieces);
            Ok(())
        };
        match f {
            Feature::Extrude {
                profile,
                plane,
                distance,
                taper_deg,
                op,
                ..
            } => {
                if *taper_deg != 0.0 {
                    return Err(EvalError::unsupported(Some(id), "a tapered extrude"));
                }
                let piece = body(plane, profile, *distance, id)?;
                combine(&mut b, vec![piece], *op)?;
            }
            Feature::Hole {
                at,
                dir,
                dia,
                depth,
                cbore,
                ..
            } => {
                if cbore.is_some() {
                    return Err(EvalError::unsupported(Some(id), "a counterbore"));
                }
                if !axis_aligned(*dir) {
                    return Err(EvalError::unsupported(Some(id), "an oblique hole"));
                }
                let d = dir.normalized().expect("checked");
                let len = match depth {
                    Depth::Blind(x) => *x,
                    Depth::Through => {
                        let all = aabb_of(
                            b.pieces
                                .iter()
                                .filter(|p| p.sign > 0.0)
                                .flat_map(|p| corners(&p.bbox)),
                        );
                        corners(&all)
                            .iter()
                            .map(|c| (*c - *at).dot(d))
                            .fold(0.0, f64::max)
                    }
                };
                let r = dia / 2.0;
                let (x, y) = if d.x.abs() > 0.5 {
                    (super::types::Vec3::Y, super::types::Vec3::Z)
                } else if d.y.abs() > 0.5 {
                    (super::types::Vec3::Z, super::types::Vec3::X)
                } else {
                    (super::types::Vec3::X, super::types::Vec3::Y)
                };
                let pts = [*at, *at + d.scale(len)]
                    .into_iter()
                    .flat_map(|e| [x, -x, y, -y].map(|v| e + v.scale(r)));
                let piece = Piece {
                    bbox: aabb_of(pts),
                    volume: PI * r * r * len,
                    centroid: *at + d.scale(len / 2.0),
                    sign: 1.0,
                    boxy: false,
                };
                combine(&mut b, vec![piece], Op::Subtract)?;
            }
            Feature::Boolean { op, tool, .. } => match tool {
                ToolRef::Def(t, placement) => {
                    let Some(td) = tools.def(*t) else {
                        return Err(EvalError::UnknownTool {
                            feature: id,
                            def: *t,
                        });
                    };
                    if !axis_aligned(apply_vec(placement, super::types::Vec3::X))
                        || !axis_aligned(apply_vec(placement, super::types::Vec3::Y))
                    {
                        return Err(EvalError::unsupported(Some(id), "a rotated tool"));
                    }
                    let tb = eval(td, tools, depth + 1)?;
                    let pieces = tb
                        .pieces
                        .iter()
                        .map(|p| place_piece(placement, p))
                        .collect();
                    combine(&mut b, pieces, *op)?;
                }
                ToolRef::Part { .. } => {
                    return Err(EvalError::unsupported(Some(id), "ToolRef::Part"))
                }
            },
            other => {
                return Err(EvalError::unsupported(Some(id), other.kind_name()));
            }
        }
    }
    if b.pieces.is_empty() {
        return Err(EvalError::invalid(None, "the part has no body"));
    }
    Ok(b)
}

impl FakeBrep {
    fn hull(&self) -> Aabb3 {
        aabb_of(
            self.pieces
                .iter()
                .filter(|p| p.sign > 0.0)
                .flat_map(|p| corners(&p.bbox)),
        )
    }
}

impl SolidKernel for FakeKernel {
    type Brep = FakeBrep;

    fn name(&self) -> String {
        "fake (closed forms)".into()
    }

    fn evaluate(
        &self,
        def: &PartDef,
        tools: &dyn ToolResolver,
    ) -> Result<Evaluated<FakeBrep>, EvalError> {
        Ok(Evaluated {
            brep: eval(def, tools, 0)?,
        })
    }

    fn mass(&self, b: &FakeBrep) -> Result<MassProps, EvalError> {
        let v: f64 = b.pieces.iter().map(|p| p.sign * p.volume).sum();
        let m = b
            .pieces
            .iter()
            .fold(super::types::Vec3::new(0.0, 0.0, 0.0), |acc, p| {
                acc + super::types::Vec3::new(p.centroid.x, p.centroid.y, p.centroid.z)
                    .scale(p.sign * p.volume)
            });
        let c = m.scale(1.0 / v);
        Ok(MassProps {
            volume: v,
            centroid: Some(Point3::new(c.x, c.y, c.z)),
            exact: true,
        })
    }

    fn bbox(&self, b: &FakeBrep, frame: &Placement) -> Result<Aabb3, EvalError> {
        Ok(aabb_of(
            corners(&b.hull())
                .into_iter()
                .map(|c| apply_point(frame, c)),
        ))
    }

    fn check(&self, _b: &FakeBrep) -> Result<Validity, EvalError> {
        Ok(Validity {
            valid: true,
            solids: 1,
            faces: 6,
            problems: Vec::new(),
        })
    }

    fn tessellate(&self, b: &FakeBrep, _chord: f64) -> Result<TriMesh, EvalError> {
        let h = b.hull();
        Ok(crate::scene3d::shapes::cuboid(h.min, h.max))
    }

    fn evaluate_from(
        &self,
        def: &PartDef,
        tools: &dyn ToolResolver,
        base: &FakeBrep,
        done: usize,
    ) -> Result<Evaluated<FakeBrep>, EvalError> {
        Ok(Evaluated {
            brep: eval_from(def, tools, 0, base.clone(), done)?,
        })
    }

    /// Between the placed hulls (exact for parts that are one box).
    fn distances(
        &self,
        solids: &[Placed<'_, FakeBrep>],
        pairs: &[(usize, usize)],
    ) -> Result<Vec<Result<Closest, EvalError>>, EvalError> {
        Ok(pairs
            .iter()
            .map(|&(i, j)| {
                let (a, b) = (placed_hull(&solids[i]), placed_hull(&solids[j]));
                Ok(box_closest(&a, &b))
            })
            .collect())
    }

    /// Between the placed hulls (exact for parts that are one box).
    fn interferences(
        &self,
        solids: &[Placed<'_, FakeBrep>],
        pairs: &[(usize, usize)],
    ) -> Result<Vec<Result<Interference, EvalError>>, EvalError> {
        Ok(pairs
            .iter()
            .map(|&(i, j)| {
                let (a, b) = (placed_hull(&solids[i]), placed_hull(&solids[j]));
                let c = box_closest(&a, &b);
                if c.distance > 0.0 {
                    return Ok(Interference::Apart(c));
                }
                let e = [
                    a.max.x.min(b.max.x) - a.min.x.max(b.min.x),
                    a.max.y.min(b.max.y) - a.min.y.max(b.min.y),
                    a.max.z.min(b.max.z) - a.min.z.max(b.min.z),
                ];
                let volume = e[0] * e[1] * e[2];
                if volume <= 0.0 {
                    return Ok(Interference::Touching { at: c.a });
                }
                let vol =
                    |x: &Aabb3| (x.max.x - x.min.x) * (x.max.y - x.min.y) * (x.max.z - x.min.z);
                Ok(Interference::Overlapping {
                    volume,
                    a_in_b: volume == vol(&a),
                    b_in_a: volume == vol(&b),
                })
            })
            .collect())
    }
}

fn placed_hull((b, p): &Placed<'_, FakeBrep>) -> Aabb3 {
    aabb_of(corners(&b.hull()).into_iter().map(|c| apply_point(p, c)))
}

/// The closest points of two boxes: per axis, the gap between the ranges
/// (or the middle of their overlap).
fn box_closest(a: &Aabb3, b: &Aabb3) -> Closest {
    let axis = |a0: f64, a1: f64, b0: f64, b1: f64| -> (f64, f64) {
        if a1 < b0 {
            (a1, b0)
        } else if b1 < a0 {
            (a0, b1)
        } else {
            let m = (a0.max(b0) + a1.min(b1)) / 2.0;
            (m, m)
        }
    };
    let (x, y, z) = (
        axis(a.min.x, a.max.x, b.min.x, b.max.x),
        axis(a.min.y, a.max.y, b.min.y, b.max.y),
        axis(a.min.z, a.max.z, b.min.z, b.max.z),
    );
    let (pa, pb) = (Point3::new(x.0, y.0, z.0), Point3::new(x.1, y.1, z.1));
    Closest {
        distance: pa.distance(pb),
        a: pa,
        b: pb,
    }
}
