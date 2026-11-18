//! [`SolidKernel`] on the cad-kernel B-rep kernel (crates.io `cad-kernel`,
//! pinned `=0.15.0`, from the geomsolver repository).
//!
//! This is the only file that names cad-kernel. Everything it builds stays
//! in a [`CadBrep`], derived data: never synced, never stored.
//!
//! What each feature maps to:
//!
//! | Feature | cad-kernel |
//! |---|---|
//! | `Extrude` (no taper) | a `build::Profile` on the sketch plane (lines as `ProfileEdge::Line`, circular arcs and circles as `ProfileEdge::Arc` on a frame in the plane, elliptic arcs as exact rational quadratic B-splines, `ProfileEdge::Spline`), then `build::prism` from `min(0, d)` to `max(0, d)` along the plane normal, then the op |
//! | `Hole` | `build::prism` of `build::disc` at the entry point along the direction (blind: the depth; through: past the far side of the part's box), `boolean::combined(.., Op::Cut, 0)`; a counterbore is a second, wider disc prism from the entry point, cut the same way |
//! | `Boolean` with `ToolRef::Def` | the tool definition evaluated on its own, moved by `Brep::moved` (a rigid motion), then the op |
//! | `Chamfer` (`d1 == d2`) | edges picked by [`edge_query`], then for each picked convex edge, one at a time on the current solid: `fillet::flat_along` (the flat between the two legs, a plane between two planes) and `boolean::combined(.., Op::Cut, 0)` |
//! | op `Add` / `Subtract` / `Intersect` | `boolean::combined` with `Op::Union` / `Op::Cut` / `Op::Common`, floor 0 (the kernel's own tolerance, `1e-9` of the operands' size) |
//!
//! `combined` is the plain Boolean. The kernel also has `combined_written`,
//! which first moves a tool's planar faces that are "all but flush" with
//! the solid's onto them; that would change the recipe, so it is not used.
//!
//! Everything else (`Revolve`, `Sweep`, `Loft`, `Fillet`, tapered extrude,
//! asymmetric chamfer, `ToolRef::Part`, spline profile segments, which have
//! no reference meaning yet, a chamfer of a concave edge, and a chamfer
//! whose flat on one edge cuts away another picked edge) returns
//! [`EvalError::Unsupported`].
//!
//! Volumes come from `props::volume` (Green's theorem over each exact face,
//! no mesh). cad-kernel computes no centroid, so [`MassProps::centroid`] is
//! `None` and the harness's centroid comparisons are not applicable.
//! The box comes from the exact geometry: `Brep::bounds` on the moved
//! B-rep, which is every vertex, the exact box of every line, circle and
//! ellipse edge, and nothing more for ruled faces (planes, cylinders,
//! extrusions take their extremes on their edges). Any other edge or face
//! kind (B-spline, traced) has only a conservative box there, so the
//! adapter refuses a tight box for it.
//!
//! Validity is `Brep::check` at ten times the kernel's tolerance (what
//! `combined` checks its own results to); solids are counted as the
//! shells (face components over shared edges) with positive volume.
//!
//! Tessellation is `mesh::mesh(b, chord, MESH_ANGLE)`: every edge sampled
//! once and shared by its two faces, triangles turning with the outward
//! normal, each face refined until its measured sag is within the chord.

use std::fmt;
use std::sync::Arc as Shared;

use cad_kernel::boolean::{combined, Op as KOp};
use cad_kernel::build::{disc, prism, Profile, ProfileEdge};
use cad_kernel::geom::{tolerance, Curve, Frame, Rigid, Surface, V};
use cad_kernel::nurbs::BSpline;
use cad_kernel::topo::{Brep, EdgeCurve};

use cadview_core::solid::kernel::{
    Aabb3, EvalError, Evaluated, MassProps, SolidKernel, ToolResolver, TriMesh, Validity,
};
use cadview_core::solid::select::{edge_query, EdgeQuery};
use cadview_core::solid::types::{
    arc_point, DefId, Depth, FeatId, Feature, Frame3, Op, PartDef, Placement, Plane3, Point3,
    Profile2d, Seg2, ToolRef, Vec3,
};

/// How far (mm) a picked edge may sit from its selector's target curve.
const PICK_TOL: f64 = 1e-6;

/// How deep tool definitions may nest.
const MAX_TOOL_DEPTH: usize = 16;

/// The mesher's angular limit per edge sample and facet (radians). The
/// chord governs the mesh contract; this only keeps closed curves from
/// starting too coarse.
const MESH_ANGLE: f64 = 0.5;

/// The cad-kernel backend. Stateless: each evaluation builds its own solids.
#[derive(Debug, Clone, Copy, Default)]
pub struct CadKernel;

/// An evaluated part.
#[derive(Clone)]
pub struct CadBrep {
    brep: Shared<Brep>,
}

impl fmt::Debug for CadBrep {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CadBrep")
            .field("faces", &self.brep.faces.len())
            .finish_non_exhaustive()
    }
}

impl CadBrep {
    /// The kernel's B-rep, for the forward-look tests.
    pub fn brep(&self) -> &Brep {
        &self.brep
    }
}

impl CadKernel {
    /// The version compiled in: the exact pin in solid-harness's
    /// Cargo.toml (cad-kernel exports no version constant).
    pub const VERSION: &'static str = "0.15.0";
}

fn kernel_err<E: fmt::Display>(feature: Option<FeatId>) -> impl Fn(E) -> EvalError {
    move |e| EvalError::Kernel {
        feature,
        message: e.to_string(),
    }
}

fn v3(p: Point3) -> V {
    [p.x, p.y, p.z]
}

fn d3(v: Vec3) -> V {
    [v.x, v.y, v.z]
}

impl SolidKernel for CadKernel {
    type Brep = CadBrep;

    fn name(&self) -> String {
        format!("cad-kernel {}", Self::VERSION)
    }

    fn evaluate(
        &self,
        def: &PartDef,
        tools: &dyn ToolResolver,
    ) -> Result<Evaluated<CadBrep>, EvalError> {
        let mut stack = vec![def.id];
        let mut names = Names::default();
        let brep = eval_def(def, tools, &mut stack, &mut names)?;
        Ok(Evaluated {
            brep: CadBrep {
                brep: Shared::new(brep),
            },
        })
    }

    fn mass(&self, b: &CadBrep) -> Result<MassProps, EvalError> {
        let volume = cad_kernel::props::volume(&b.brep);
        if !volume.is_finite() {
            return Err(EvalError::Kernel {
                feature: None,
                message: format!("volume is {volume}"),
            });
        }
        Ok(MassProps {
            volume,
            // cad-kernel computes volume and area, no first moments.
            centroid: None,
            // Green's theorem over the exact faces: no mesh is involved.
            exact: true,
        })
    }

    fn bbox(&self, b: &CadBrep, frame: &Placement) -> Result<Aabb3, EvalError> {
        frame.check()?;
        tight_bounds(&b.brep.moved(&rigid(frame)))
    }

    fn check(&self, b: &CadBrep) -> Result<Validity, EvalError> {
        let brep = &b.brep;
        let tol = 10.0 * tolerance(brep.size());
        let mut problems = Vec::new();
        let valid = match brep.check(tol) {
            Ok(()) => true,
            Err(e) => {
                problems.push(format!("{:?}: {}", e.kind, e.msg));
                false
            }
        };
        Ok(Validity {
            valid,
            solids: solids(brep),
            faces: brep.faces.len(),
            problems,
        })
    }

    fn tessellate(&self, b: &CadBrep, chord: f64) -> Result<TriMesh, EvalError> {
        if !chord.is_finite() || chord <= 0.0 {
            return Err(EvalError::invalid(
                None,
                "chord must be finite and positive",
            ));
        }
        let m = cad_kernel::mesh::mesh(&b.brep, chord, MESH_ANGLE).map_err(kernel_err(None))?;
        Ok(TriMesh {
            positions: m
                .pts
                .iter()
                .map(|p| Point3::new(p[0], p[1], p[2]))
                .collect(),
            triangles: m.tris,
        })
    }
}

/// The rotation matrix (row-major) and translation of a checked placement.
fn rigid(p: &Placement) -> Rigid {
    let [w, x, y, z] = p.rotation;
    let n = (w * w + x * x + y * y + z * z).sqrt();
    let (w, x, y, z) = (w / n, x / n, y / n, z / n);
    Rigid {
        r: [
            [
                1.0 - 2.0 * (y * y + z * z),
                2.0 * (x * y - w * z),
                2.0 * (x * z + w * y),
            ],
            [
                2.0 * (x * y + w * z),
                1.0 - 2.0 * (x * x + z * z),
                2.0 * (y * z - w * x),
            ],
            [
                2.0 * (x * z - w * y),
                2.0 * (y * z + w * x),
                1.0 - 2.0 * (x * x + y * y),
            ],
        ],
        t: d3(p.translation),
    }
}

/// Evaluates `def`. `stack` holds the definitions being evaluated,
/// outermost first, to catch tool cycles.
fn eval_def(
    def: &PartDef,
    tools: &dyn ToolResolver,
    stack: &mut Vec<DefId>,
    names: &mut Names,
) -> Result<Brep, EvalError> {
    let mut current: Option<Brep> = None;
    for (i, feature) in def.features.iter().enumerate() {
        let id = feature.id();
        let next = match feature {
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
                if !distance.is_finite() || *distance == 0.0 {
                    return Err(EvalError::invalid(
                        Some(id),
                        "extrude distance must be finite and non-zero",
                    ));
                }
                let body = extrude(profile, plane, *distance, names).map_err(|e| e.at(id))?;
                combine(current.take(), body, *op, id)?
            }
            Feature::Hole {
                at,
                dir,
                dia,
                depth,
                cbore,
                ..
            } => {
                let Some(cur) = current.take() else {
                    return Err(EvalError::invalid(
                        Some(id),
                        "a hole needs material to cut: add a body first",
                    ));
                };
                hole(&cur, *at, *dir, *dia, *depth, *cbore, id, names)?
            }
            Feature::Boolean { op, tool, .. } => match tool {
                ToolRef::Def(tool_id, placement) => {
                    placement.check().map_err(|e| e.at(id))?;
                    if stack.contains(tool_id) {
                        return Err(EvalError::ToolCycle {
                            feature: id,
                            def: *tool_id,
                        });
                    }
                    if stack.len() >= MAX_TOOL_DEPTH {
                        return Err(EvalError::invalid(
                            Some(id),
                            format!("tool definitions nest deeper than {MAX_TOOL_DEPTH}"),
                        ));
                    }
                    let Some(tool_def) = tools.def(*tool_id) else {
                        return Err(EvalError::UnknownTool {
                            feature: id,
                            def: *tool_id,
                        });
                    };
                    stack.push(*tool_id);
                    let body = eval_def(tool_def, tools, stack, names);
                    stack.pop();
                    let body = body?;
                    let placed = if placement.is_identity() {
                        body
                    } else {
                        body.moved(&rigid(placement))
                    };
                    combine(current.take(), placed, *op, id)?
                }
                ToolRef::Part { .. } => {
                    return Err(EvalError::unsupported(
                        Some(id),
                        "a boolean with a placed scene part as its tool (ToolRef::Part)",
                    ));
                }
            },
            Feature::Chamfer { edges, d1, d2, .. } => {
                let Some(cur) = current.take() else {
                    return Err(EvalError::invalid(Some(id), "nothing to chamfer yet"));
                };
                if d1 != d2 {
                    return Err(EvalError::unsupported(
                        Some(id),
                        "an asymmetric chamfer (d1 != d2)",
                    ));
                }
                if !d1.is_finite() || *d1 <= 0.0 {
                    return Err(EvalError::invalid(
                        Some(id),
                        "chamfer distance must be finite and positive",
                    ));
                }
                let query = edge_query(&def.features[..i], edges, id)?;
                chamfer(cur, &query, *d1, id)?
            }
            Feature::Revolve { .. }
            | Feature::Sweep { .. }
            | Feature::Loft { .. }
            | Feature::Fillet { .. } => {
                return Err(EvalError::unsupported(
                    Some(id),
                    format!("the {} feature", feature.kind_name()),
                ));
            }
        };
        current = Some(next);
    }
    current.ok_or_else(|| {
        EvalError::invalid(
            None,
            format!(
                "part '{}' has no material: no feature adds a body",
                def.name
            ),
        )
    })
}

/// Combines a new body with the part so far.
fn combine(current: Option<Brep>, body: Brep, op: Op, id: FeatId) -> Result<Brep, EvalError> {
    let (cur, kop) = match (current, op) {
        (None, Op::Add) => return Ok(body),
        (None, Op::Subtract) => {
            return Err(EvalError::invalid(
                Some(id),
                "nothing to subtract from: the first body must be added",
            ));
        }
        (None, Op::Intersect) => {
            return Err(EvalError::invalid(
                Some(id),
                "nothing to intersect with: the first body must be added",
            ));
        }
        (Some(c), Op::Add) => (c, KOp::Union),
        (Some(c), Op::Subtract) => (c, KOp::Cut),
        (Some(c), Op::Intersect) => (c, KOp::Common),
    };
    boolean(&cur, &body, kop, Some(id))
}

/// One Boolean at the kernel's own tolerance.
fn boolean(a: &Brep, b: &Brep, op: KOp, id: Option<FeatId>) -> Result<Brep, EvalError> {
    combined(a, b, op, 0.0).map_err(|e| EvalError::Kernel {
        feature: id,
        message: format!("{:?}: {}", e.kind, e.msg),
    })
}

/// Face names, unique within one evaluation. cad-kernel reads two faces
/// with the same name as pieces of one document face (its flat and fillet
/// code refuses a run that "turns where each face's pieces meet"), and its
/// own front end names every primitive's faces under a key of their own.
/// The adapter does the same: every prism's faces are `p<n>.<face>`, with
/// `<face>` the side's segment (`l<loop>s<segment>`) or the kernel's cap
/// name (`near`, `far`).
#[derive(Debug, Default)]
struct Names {
    next: u64,
}

impl Names {
    /// The primitive's faces renamed under a fresh key.
    fn own(&mut self, mut b: Brep) -> Brep {
        let key = self.next;
        self.next += 1;
        for f in &mut b.faces {
            f.name = format!("p{key}.{}", f.name);
        }
        b
    }
}

/// A profile on a sketch plane, swept `distance` along the plane normal.
fn extrude(
    profile: &Profile2d,
    plane: &Plane3,
    distance: f64,
    names: &mut Names,
) -> Result<Brep, EvalError> {
    profile.check()?;
    let frame = plane.frame()?;
    let loops = profile
        .loops()
        .map(|(_, lp)| profile_loop(lp, &frame))
        .collect::<Result<Vec<_>, _>>()?;
    let face_names = loops
        .iter()
        .enumerate()
        .map(|(li, lp)| (0..lp.len()).map(|k| format!("l{li}s{k}")).collect())
        .collect();
    let sketch = Profile {
        origin: v3(frame.origin),
        normal: d3(frame.z),
        loops,
        names: face_names,
    };
    let b = prism(&sketch, distance.min(0.0), distance.max(0.0)).map_err(kernel_err(None))?;
    Ok(names.own(b))
}

/// The kernel frame at profile point `c` with the plane's axes.
fn frame_at(f: &Frame3, c: kurbo::Point) -> Frame {
    Frame {
        o: v3(f.at(c, 0.0)),
        x: d3(f.x),
        y: d3(f.y),
        z: d3(f.z),
    }
}

/// One closed profile loop as kernel profile edges, in 3D. Each line runs
/// from its own start to the next segment's start (the loop's join points),
/// so the joins checked to [`cadview_core::solid::PROFILE_JOIN_TOL`] close.
fn profile_loop(lp: &[Seg2], f: &Frame3) -> Result<Vec<ProfileEdge>, EvalError> {
    let starts: Vec<kurbo::Point> = lp
        .iter()
        .map(|s| s.ends().map(|(a, _)| a))
        .collect::<Option<_>>()
        .ok_or_else(|| EvalError::invalid(None, "profile segment has no end points"))?;
    let n = lp.len();
    let mut out = Vec::with_capacity(n);
    for (i, seg) in lp.iter().enumerate() {
        out.push(match seg {
            Seg2::Line(_) => ProfileEdge::Line {
                a: v3(f.at(starts[i], 0.0)),
                b: v3(f.at(starts[(i + 1) % n], 0.0)),
            },
            Seg2::Circle(c) => ProfileEdge::Arc {
                frame: frame_at(f, c.center),
                r: c.radius,
                span: None,
            },
            Seg2::Arc(a) if a.radii.x == a.radii.y => {
                // The point at parameter t is c + r (cos(t + rot), sin(t + rot)).
                let t0 = a.start_angle + a.x_rotation;
                let (lo, hi) = if a.sweep_angle >= 0.0 {
                    (t0, t0 + a.sweep_angle)
                } else {
                    (t0 + a.sweep_angle, t0)
                };
                let span = if seg.is_closed_loop() {
                    None
                } else {
                    let tau = std::f64::consts::TAU;
                    let shift = lo.div_euclid(tau) * tau;
                    Some([lo - shift, hi - shift])
                };
                ProfileEdge::Arc {
                    frame: frame_at(f, a.center),
                    r: a.radii.x,
                    span,
                }
            }
            Seg2::Arc(a) => ProfileEdge::Spline(Shared::new(elliptic_arc(a, f)?)),
            Seg2::Spline { .. } => {
                return Err(EvalError::unsupported(
                    None,
                    "a spline profile segment (no reference meaning yet)",
                ));
            }
        });
    }
    Ok(out)
}

/// An elliptic arc as the exact rational quadratic B-spline: pieces of at
/// most a quarter turn, each the affine image of a circular arc's
/// rational Bezier form (end poles on the arc, the middle pole where the
/// end tangents meet, weight `cos(h / 2)` for a piece turning `h`), joined
/// with double knots. No approximation: every point of it lies on the
/// ellipse (up to rounding of the poles).
fn elliptic_arc(a: &kurbo::Arc, f: &Frame3) -> Result<BSpline, EvalError> {
    let sweep = a
        .sweep_angle
        .clamp(-std::f64::consts::TAU, std::f64::consts::TAU);
    let pieces = (sweep.abs() / std::f64::consts::FRAC_PI_2).ceil().max(1.0) as usize;
    let h = sweep / pieces as f64;
    let w = (0.5 * h).cos();
    let (rs, rc) = a.x_rotation.sin_cos();
    let at = |x: f64, y: f64| {
        let p = kurbo::Point::new(a.center.x + x * rc - y * rs, a.center.y + x * rs + y * rc);
        v3(f.at(p, 0.0))
    };
    let mut poles = Vec::with_capacity(2 * pieces + 1);
    let mut weights = Vec::with_capacity(2 * pieces + 1);
    let mut knots = vec![0.0; 3];
    for k in 0..pieces {
        let t0 = a.start_angle + h * k as f64;
        let m = t0 + 0.5 * h;
        if k == 0 {
            let p = arc_point(a, t0);
            poles.push(v3(f.at(p, 0.0)));
            weights.push(1.0);
        }
        poles.push(at(a.radii.x * m.cos() / w, a.radii.y * m.sin() / w));
        weights.push(w);
        let p = arc_point(a, t0 + h);
        poles.push(v3(f.at(p, 0.0)));
        weights.push(1.0);
        let kk = (k + 1) as f64;
        if k + 1 < pieces {
            knots.extend([kk, kk]);
        } else {
            knots.extend([kk, kk, kk]);
        }
    }
    BSpline::rational(2, knots, poles, weights).map_err(kernel_err(None))
}

#[allow(clippy::too_many_arguments)]
fn hole(
    current: &Brep,
    at: Point3,
    d: Vec3,
    dia: f64,
    depth: Depth,
    cbore: Option<(f64, f64)>,
    id: FeatId,
    names: &mut Names,
) -> Result<Brep, EvalError> {
    let invalid = |why: &str| EvalError::invalid(Some(id), why);
    if !at.is_finite() {
        return Err(invalid("hole position is not finite"));
    }
    let Some(d) = d.normalized() else {
        return Err(invalid("hole direction must be a non-zero vector"));
    };
    if !dia.is_finite() || dia <= 0.0 {
        return Err(invalid("hole diameter must be finite and positive"));
    }
    // The reference meaning: the hole removes only `t` in [0, depth] from
    // the entry point along `d`, never material behind it. The drill
    // starts exactly at `at`. A through hole runs past the far side of the
    // part's (conservative) box by a margin, so no drill face is coplanar
    // with the face it leaves through.
    let margin = dia.max(1.0);
    let length = match depth {
        Depth::Blind(x) if x.is_finite() && x > 0.0 => x,
        Depth::Blind(_) => return Err(invalid("hole depth must be finite and positive")),
        Depth::Through => {
            let (lo, hi) = current.bounds();
            let mut reach = f64::NEG_INFINITY;
            for x in [lo[0], hi[0]] {
                for y in [lo[1], hi[1]] {
                    for z in [lo[2], hi[2]] {
                        reach = reach.max((Point3::new(x, y, z) - at).dot(d));
                    }
                }
            }
            if !reach.is_finite() || reach <= 0.0 {
                return Err(invalid("a through hole points away from the part"));
            }
            reach + margin
        }
    };
    if let Some((cdia, cdepth)) = cbore {
        if !cdia.is_finite() || cdia <= dia {
            return Err(invalid(
                "counterbore diameter must be larger than the hole's",
            ));
        }
        let too_deep = matches!(depth, Depth::Blind(x) if cdepth >= x);
        if !cdepth.is_finite() || cdepth <= 0.0 || too_deep {
            return Err(invalid(
                "counterbore depth must be positive and less than the hole's",
            ));
        }
    }
    let k = kernel_err::<String>(Some(id));
    let drill = names.own(prism(&disc(v3(at), d3(d), dia / 2.0), 0.0, length).map_err(&k)?);
    let mut out = boolean(current, &drill, KOp::Cut, Some(id))?;
    if let Some((cdia, cdepth)) = cbore {
        let bore = names.own(prism(&disc(v3(at), d3(d), cdia / 2.0), 0.0, cdepth).map_err(&k)?);
        out = boolean(&out, &bore, KOp::Cut, Some(id))?;
    }
    Ok(out)
}

fn p3(v: V) -> Point3 {
    Point3::new(v[0], v[1], v[2])
}

/// Start, middle and end of an edge, or `None` for a degenerate one.
fn samples(b: &Brep, e: usize) -> Option<[Point3; 3]> {
    let edge = &b.edges[e];
    match &edge.curve {
        EdgeCurve::Curve(c) => {
            let [t0, t1] = edge.t;
            Some([t0, 0.5 * (t0 + t1), t1].map(|t| p3(c.point(t))))
        }
        EdgeCurve::Degenerate => None,
    }
}

/// The distance from `p` to edge `e`: the curve's closest point, clamped
/// to the edge's span.
fn edge_distance(b: &Brep, e: usize, p: Point3) -> Option<f64> {
    let edge = &b.edges[e];
    let EdgeCurve::Curve(c) = &edge.curve else {
        return None;
    };
    let mut t = c.inverse(v3(p));
    if let Some(period) = c.period() {
        t = edge.t[0] + (t - edge.t[0]).rem_euclid(period);
    }
    let t = t.clamp(edge.t[0], edge.t[1]);
    let q = p3(c.point(t));
    let d = q.distance(p);
    // The ends too: a periodic curve's clamp can land on the far end.
    let ends = [edge.t[0], edge.t[1]].map(|t| p3(c.point(t)).distance(p));
    Some(d.min(ends[0]).min(ends[1]))
}

/// The edges a query picks on `b`, as `(edge, target index)`; the target
/// index is `None` for `All` and `Near`.
fn picked(
    b: &Brep,
    query: &EdgeQuery,
    id: FeatId,
) -> Result<Vec<(usize, Option<usize>)>, EvalError> {
    let selector = |reason: String| EvalError::Selector {
        feature: id,
        reason,
    };
    let mut out = Vec::new();
    match query {
        EdgeQuery::All => {
            for e in 0..b.edges.len() {
                if samples(b, e).is_some() {
                    out.push((e, None));
                }
            }
        }
        EdgeQuery::Targets(targets) => {
            for (ti, target) in targets.iter().enumerate() {
                let mut any = false;
                for e in 0..b.edges.len() {
                    if out.iter().any(|(x, _)| *x == e) {
                        continue;
                    }
                    if let Some(s) = samples(b, e) {
                        if s.iter().all(|p| target.contains(*p, PICK_TOL)) {
                            out.push((e, Some(ti)));
                            any = true;
                        }
                    }
                }
                if !any {
                    return Err(selector(format!(
                        "target {ti} ({target:?}) matches no edge of the part"
                    )));
                }
            }
        }
        EdgeQuery::Near(points) => {
            for p in points {
                let best = (0..b.edges.len())
                    .filter_map(|e| edge_distance(b, e, *p).map(|d| (d, e)))
                    .min_by(|x, y| x.0.total_cmp(&y.0));
                let Some((_, e)) = best else {
                    return Err(selector("the part has no edges".into()));
                };
                if !out.iter().any(|(x, _)| *x == e) {
                    out.push((e, None));
                }
            }
        }
    }
    if out.is_empty() {
        return Err(selector("no edge selected".into()));
    }
    Ok(out)
}

/// Whether edge `e` of `b` is convex: with face A walking the edge along
/// `t` and outward normals `nA`, `nB` of its two faces at the middle,
/// `(nA x nB) . t > 0`. `None` when that cannot be read (not two faces, a
/// pole, a tangent meeting).
fn convex(b: &Brep, e: usize) -> Option<bool> {
    let uses = b.uses();
    let [(fa, la, ca), (fb, _, _)] = uses[e].as_slice() else {
        return None;
    };
    let edge = &b.edges[e];
    let EdgeCurve::Curve(c) = &edge.curve else {
        return None;
    };
    let tm = 0.5 * (edge.t[0] + edge.t[1]);
    let p = c.point(tm);
    let mut t = c.tangent(tm);
    if b.faces[*fa].loops[*la][*ca].reversed {
        t = t.map(|x| -x);
    }
    let na = b.faces[*fa].outward(p)?;
    let nb = b.faces[*fb].outward(p)?;
    let cr = [
        na[1] * nb[2] - na[2] * nb[1],
        na[2] * nb[0] - na[0] * nb[2],
        na[0] * nb[1] - na[1] * nb[0],
    ];
    let s = cr[0] * t[0] + cr[1] * t[1] + cr[2] * t[2];
    let scale = (t[0] * t[0] + t[1] * t[1] + t[2] * t[2]).sqrt();
    if s.abs() <= 1e-9 * scale {
        return None;
    }
    Some(s > 0.0)
}

/// An equal-distance chamfer of the picked edges, one edge at a time: the
/// kernel's flat along the edge (`fillet::flat_along`, legs `d` on both
/// faces), cut from the solid. cad-kernel has no operation that chamfers
/// several edges at once, and one flat after another equals the
/// reference's intersection of half-spaces only while every picked edge
/// is still an edge of the solid when its turn comes. So:
///
/// - a picked edge that an earlier flat has cut away (its middle is no
///   longer on an edge) is refused: its own half-space would still remove
///   material the earlier one did not, and no edge is left to cut it
///   along. (Until the fuzzer found it, such an edge was skipped, which
///   was wrong at a convex corner turning less than the chamfer: a
///   heptagon with both caps chamfered 0.3 kept 0.3 mm^3 too much.)
/// - an edge still there after its own cut is refused.
fn chamfer(mut body: Brep, query: &EdgeQuery, d: f64, id: FeatId) -> Result<Brep, EvalError> {
    let first = picked(&body, query, id)?;
    // The points the work list is keyed by: an edge's middle.
    let todo: Vec<Point3> = first
        .iter()
        .filter_map(|(e, _)| samples(&body, *e).map(|s| s[1]))
        .collect();
    for at in todo {
        // The edge of the current solid through that point.
        let Some((dist, e)) = (0..body.edges.len())
            .filter_map(|e| edge_distance(&body, e, at).map(|x| (x, e)))
            .min_by(|x, y| x.0.total_cmp(&y.0))
        else {
            return Err(EvalError::Kernel {
                feature: Some(id),
                message: "the solid has no edges left to chamfer".into(),
            });
        };
        if dist > PICK_TOL {
            return Err(EvalError::unsupported(
                Some(id),
                "a chamfer whose flat on one edge cuts away another picked edge \
                 (cad-kernel has no chamfer of several edges at once)",
            ));
        }
        match convex(&body, e) {
            Some(true) => {}
            Some(false) => {
                return Err(EvalError::unsupported(
                    Some(id),
                    "a chamfer of a concave edge",
                ))
            }
            None => {
                return Err(EvalError::Kernel {
                    feature: Some(id),
                    message: "cannot tell whether a picked edge is convex".into(),
                })
            }
        }
        let tol = tolerance(body.size());
        let piece = cad_kernel::fillet::flat_along(&body, v3(at), [d, d], None, tol)
            .map_err(kernel_err(Some(id)))?;
        body = boolean(&body, &piece, KOp::Cut, Some(id))?;
        let still = (0..body.edges.len())
            .any(|e| edge_distance(&body, e, at).is_some_and(|x| x <= PICK_TOL));
        if still {
            return Err(EvalError::Kernel {
                feature: Some(id),
                message: "a chamfered edge is still there after its cut".into(),
            });
        }
    }
    Ok(body)
}

/// The number of solids: shells (faces connected through shared edges)
/// whose enclosed volume is positive. A void's shell faces inward and has
/// negative volume.
fn solids(b: &Brep) -> usize {
    let n = b.faces.len();
    let mut parent: Vec<usize> = (0..n).collect();
    fn find(p: &mut [usize], mut x: usize) -> usize {
        while p[x] != x {
            p[x] = p[p[x]];
            x = p[x];
        }
        x
    }
    let mut by_edge: Vec<Option<usize>> = vec![None; b.edges.len()];
    for (fi, f) in b.faces.iter().enumerate() {
        for c in f.loops.iter().flatten() {
            let slot = &mut by_edge[c.edge as usize];
            match *slot {
                Some(g) => {
                    let (a, r) = (find(&mut parent, fi), find(&mut parent, g));
                    parent[a] = r;
                }
                None => *slot = Some(fi),
            }
        }
    }
    let mut shells: std::collections::BTreeMap<usize, Vec<usize>> = Default::default();
    for fi in 0..n {
        let r = find(&mut parent, fi);
        shells.entry(r).or_default().push(fi);
    }
    shells
        .values()
        .filter(|faces| {
            let sub = Brep {
                vertices: b.vertices.clone(),
                edges: b.edges.clone(),
                faces: faces.iter().map(|&f| b.faces[f].clone()).collect(),
            };
            cad_kernel::props::volume(&sub) > 0.0
        })
        .count()
}

/// The tight box of `b` from its exact geometry (see the module docs).
fn tight_bounds(b: &Brep) -> Result<Aabb3, EvalError> {
    let unsupported =
        |what: &str| EvalError::unsupported(None, format!("a tight bounding box of {what}"));
    for e in &b.edges {
        if let EdgeCurve::Curve(c) = &e.curve {
            if !matches!(
                c,
                Curve::Line { .. } | Curve::Circle(..) | Curve::Ellipse(..)
            ) {
                return Err(unsupported(&format!("a {} edge", c.kind())));
            }
        }
    }
    for f in &b.faces {
        if !matches!(
            f.surface,
            Surface::Plane(_) | Surface::Cylinder(..) | Surface::Cone(..)
        ) {
            return Err(unsupported(&format!("a {} face", f.surface.kind())));
        }
    }
    let (lo, hi) = b.bounds();
    if !(lo.iter().chain(&hi).all(|v| v.is_finite())) {
        return Err(EvalError::Kernel {
            feature: None,
            message: "the shape has an empty bounding box".into(),
        });
    }
    Ok(Aabb3::new(p3(lo), p3(hi)))
}

/// Evidence for the rungs after 3D0a, on cad-kernel directly (not through
/// the trait): the rounded smoke part, tessellation, a planar section,
/// revolve, sweep, loft, offsets and shells, point classification and
/// face names through a Boolean. Each test checks a closed form. Run with
/// `cargo test -p solid-harness --release kernels::cad_kernel::forward_look
/// -- --nocapture` to see the numbers.
#[cfg(test)]
mod forward_look {
    use cad_kernel::boolean::{combined, Op};
    use cad_kernel::build::{block, carried, disc, lofted, prism, revolve, Profile, ProfileEdge};
    use cad_kernel::geom::{tolerance, Carry, Surface};
    use cad_kernel::nurbs::Net;
    use cad_kernel::offset::{grown, shelled, Join, Side};
    use cad_kernel::props::{area, volume};
    use cad_kernel::query::{Located, Place};
    use cad_kernel::topo::Brep;
    use std::f64::consts::{PI, TAU};
    use std::time::Instant;

    fn ms(t: Instant) -> f64 {
        t.elapsed().as_secs_f64() * 1e3
    }

    fn close(got: f64, want: f64, rel: f64) -> bool {
        (got - want).abs() <= rel * want.abs()
    }

    /// The design's smoke part: 600 x 300 x 18 board, optionally all 12
    /// edges rounded r = 3 (`rounds::rounded`), minus a 10 diameter
    /// through hole at (50, 50), minus a 12 x 6 dado along the length.
    fn smoke(rounded: bool) -> Result<Brep, String> {
        let mut b = block([0.0; 3], [600.0, 300.0, 18.0])?;
        if rounded {
            let edges: Vec<usize> = (0..b.edges.len()).collect();
            b = cad_kernel::rounds::rounded(&b, &edges, 3.0, tolerance(b.size()))?;
        }
        let hole = prism(&disc([50.0, 50.0, -1.0], [0.0, 0.0, 1.0], 5.0), 0.0, 20.0)?;
        b = combined(&b, &hole, Op::Cut, 0.0)?;
        let dado = block([-1.0, 144.0, 12.0], [601.0, 156.0, 19.0])?;
        Ok(combined(&b, &dado, Op::Cut, 0.0)?)
    }

    #[test]
    fn smoke_part_plain_and_rounded() {
        for (rounded, want) in [(false, 3_195_386.283), (true, 3_188_376.635)] {
            let t = Instant::now();
            let b = smoke(rounded).expect("smoke part builds");
            let build_ms = ms(t);
            let check = b.check(10.0 * tolerance(b.size()));
            let v = volume(&b);
            println!(
                "smoke rounded={rounded}: build {build_ms:.1} ms, faces {}, check {:?}, volume {v:.6} \
                 (closed form {want}), diff {:.2e}",
                b.faces.len(),
                check.as_ref().map_err(|e| e.msg.clone()),
                v - want
            );
            assert!(check.is_ok());
            assert!((v - want).abs() < 1e-3, "{v} vs {want}");
        }
    }

    #[test]
    fn tessellation_to_a_chord() {
        let b = smoke(true).expect("smoke part builds");
        let exact = volume(&b);
        for chord in [0.1, 0.01, 0.001] {
            let t = Instant::now();
            let m = cad_kernel::mesh::mesh(&b, chord, super::MESH_ANGLE).expect("meshes");
            let mesh_ms = ms(t);
            let tm = cadview_core::solid::TriMesh {
                positions: m
                    .pts
                    .iter()
                    .map(|p| cadview_core::solid::Point3::new(p[0], p[1], p[2]))
                    .collect(),
                triangles: m.tris.clone(),
            };
            let closed = crate::mesh::topology(&tm).closed;
            let v = crate::mesh::signed_volume(&tm);
            // The mesh contract's bound (the mesh layer's Steiner tube),
            // not the arris test's rule of thumb (2e-3 x chord relative),
            // which cad-kernel exceeds at chord 1e-3 (3.1e-6 relative).
            let mut tube = 0.0;
            for i in 0..tm.triangles.len() {
                let [a, b2, c] = crate::mesh::corners(&tm, i);
                let ar = 0.5
                    * crate::mesh::norm(crate::mesh::cross(
                        crate::mesh::sub(b2, a),
                        crate::mesh::sub(c, a),
                    ));
                let per = crate::mesh::norm(crate::mesh::sub(b2, a))
                    + crate::mesh::norm(crate::mesh::sub(c, b2))
                    + crate::mesh::norm(crate::mesh::sub(a, c));
                tube +=
                    2.0 * chord * ar + PI * chord * chord * per + 4.0 / 3.0 * PI * chord.powi(3);
            }
            println!(
                "tessellate chord {chord}: {mesh_ms:.1} ms, {} triangles, closed {closed}, measured sag \
                 {:.2e}, mesh volume {v:.4}, rel err {:.1e} (rule of thumb {:.1e}, tube bound {:.1e})",
                m.tris.len(),
                m.sag,
                (v - exact).abs() / exact,
                chord * 2e-3,
                tube / exact
            );
            assert!(closed);
            assert!((v - exact).abs() <= tube);
        }
    }

    /// A planar section through a Boolean with a half-space box: the cut
    /// face at z = 9 of the rounded board is the plan outline with r = 3
    /// corners minus the hole, A = 179 964 - 16 pi.
    #[test]
    fn planar_section_by_a_boolean() {
        let b = smoke(true).expect("smoke part builds");
        let below = block([-10.0, -10.0, -10.0], [610.0, 310.0, 9.0]).unwrap();
        let t = Instant::now();
        let lower = combined(&b, &below, Op::Common, 0.0).expect("common");
        let on_cut = |s: &Surface| match s {
            Surface::Plane(fr) => (fr.o[2] - 9.0).abs() < 1e-9 && fr.z[2].abs() > 1.0 - 1e-12,
            _ => false,
        };
        let a: f64 = lower
            .faces
            .iter()
            .filter(|f| on_cut(&f.surface))
            .map(|f| area(&lower, f))
            .sum();
        let want = 179_964.0 - 16.0 * PI;
        println!(
            "section z = 9 by a Boolean: {:.1} ms, area {a:.9} (closed form {want:.9})",
            ms(t)
        );
        assert!(close(a, want, 1e-9));
    }

    /// A rectangle r in [1, 2], z in [-1, 1] turned a whole turn about z:
    /// V = pi (4 - 1) 2 = 6 pi.
    #[test]
    fn revolve_a_tube() {
        let pts = [
            [1.0, 0.0, -1.0],
            [2.0, 0.0, -1.0],
            [2.0, 0.0, 1.0],
            [1.0, 0.0, 1.0],
        ];
        let edges = (0..4)
            .map(|k| ProfileEdge::Line {
                a: pts[k],
                b: pts[(k + 1) % 4],
            })
            .collect();
        let p = Profile {
            origin: [0.0; 3],
            normal: [0.0, -1.0, 0.0],
            loops: vec![edges],
            names: Vec::new(),
        };
        let b = revolve(&p, [0.0; 3], [0.0, 0.0, 1.0], TAU).expect("revolve");
        let v = volume(&b);
        println!("revolve tube: volume {v} (closed form {})", 6.0 * PI);
        assert!(close(v, 6.0 * PI, 1e-9));
    }

    /// A disc r = 0.5 in a plane through the z axis, centred 5 from it,
    /// screwed one turn with a pitch of 3 (`build::carried`, a constant
    /// twist): by Pappus for a screw, V = 2 pi 5 pi 0.25 = 2.5 pi^2. A
    /// general path has no sweep in 0.15.0 (README, "What comes next").
    #[test]
    fn sweep_along_a_helix() {
        let p = disc([5.0, 0.0, 0.0], [0.0, 1.0, 0.0], 0.5);
        let t = Instant::now();
        let b = carried(
            &p,
            Carry::Screw {
                center: [0.0; 3],
                axis: [0.0, 0.0, 1.0],
                angle: TAU,
                advance: 3.0,
            },
        )
        .expect("screwed sweep");
        let v = volume(&b);
        let want = 2.5 * PI * PI;
        println!(
            "helical sweep: {:.1} ms, faces {}, volume {v} (closed form {want}), rel err {:.1e}",
            ms(t),
            b.faces.len(),
            (v - want).abs() / want
        );
        assert!(close(v, want, 1e-6));
    }

    /// A ruled loft from a 4 x 4 square at z = 0 to a 2 x 2 square at
    /// z = 3, as four bilinear B-spline sheets (`build::lofted`): the
    /// prismatoid V = h / 3 (A1 + A2 + sqrt(A1 A2)) = 28.
    #[test]
    fn ruled_loft_of_two_squares() {
        let lo = [[0.0, 0.0], [4.0, 0.0], [4.0, 4.0], [0.0, 4.0]];
        let hi = [[1.0, 1.0], [3.0, 1.0], [3.0, 3.0], [1.0, 3.0]];
        let nets: Vec<Net> = (0..4)
            .map(|k| {
                let k1 = (k + 1) % 4;
                Net {
                    du: 1,
                    dv: 1,
                    uknots: vec![0.0, 0.0, 1.0, 1.0],
                    vknots: vec![0.0, 0.0, 1.0, 1.0],
                    poles: vec![
                        vec![[lo[k][0], lo[k][1], 0.0], [hi[k][0], hi[k][1], 3.0]],
                        vec![[lo[k1][0], lo[k1][1], 0.0], [hi[k1][0], hi[k1][1], 3.0]],
                    ],
                    weights: None,
                }
            })
            .collect();
        let b = lofted([0.0; 3], [0.0, 0.0, 1.0], [0.0, 3.0], vec![nets]).expect("loft");
        let v = volume(&b);
        println!(
            "ruled loft: faces {}, volume {v} (closed form 28)",
            b.faces.len()
        );
        assert!(close(v, 28.0, 1e-9));
    }

    /// A 10 x 20 x 30 box grown by 2 with round joins (Steiner): V = abc +
    /// 2 (ab + bc + ca) d + pi (a + b + c) d^2 + 4/3 pi d^3; and a 10 mm
    /// cube hollowed to a 1 mm wall open at the top: V = 1000 - 8 * 8 * 9.
    #[test]
    fn offset_and_shell() {
        let b = block([0.0; 3], [10.0, 20.0, 30.0]).unwrap();
        let t = Instant::now();
        let g = grown(&b, 2.0, Join::Round).expect("grown");
        let v = volume(&g);
        let want = 6000.0 + 4400.0 + 240.0 * PI + 32.0 * PI / 3.0;
        println!(
            "grown 2 round: {:.1} ms, volume {v} (closed form {want}), rel err {:.1e}",
            ms(t),
            (v - want).abs() / want
        );
        assert!(close(v, want, 1e-9));
        let cube = block([0.0; 3], [10.0; 3]).unwrap();
        // The cap at the top is the prism's last face ("near").
        let top = cube.faces.len() - 1;
        assert_eq!(cube.faces[top].name, "near");
        let cup = shelled(&cube, &[top], 1.0, Side::In, Join::Sharp).expect("shell");
        let v = volume(&cup);
        println!("shelled cube: volume {v} (closed form 424)");
        assert!(close(v, 424.0, 1e-9));
    }

    /// Point classification and face names: a point in the board is in,
    /// one in the hole is out, one on the hole's wall is on; the board's
    /// top face, cut by the dado, keeps its name ("near") on both pieces.
    #[test]
    fn classification_and_face_names() {
        let b = smoke(false).expect("smoke part builds");
        let at = Located::new(&b, tolerance(b.size()));
        assert_eq!(at.solid_place([300.0, 50.0, 9.0]), Place::In);
        assert_eq!(at.solid_place([50.0, 50.0, 9.0]), Place::Out);
        assert_eq!(at.solid_place([50.0, 55.0, 9.0]), Place::On);
        let tops = b.faces.iter().filter(|f| f.name == "near").count();
        println!("faces named `near` after the dado: {tops}");
        assert_eq!(tops, 2);
    }
}
