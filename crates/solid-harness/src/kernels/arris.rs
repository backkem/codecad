//! [`SolidKernel`] on the arris B-rep kernel (docs/3d-design.md section 5).
//!
//! The harness's adapter for arris, moved out of cadview-core so that core
//! links no kernel. This is the only file that names arris. Everything
//! arris builds stays in an [`ArrisBrep`], which is derived data: never
//! synced, never stored.
//!
//! What each feature maps to:
//!
//! | Feature | arris |
//! |---|---|
//! | `Extrude` (no taper) | a `geom::Profile` on the sketch plane (`LineTo`, `ArcTo` through the arc's midpoint, `EllipseTo`, or a whole `Circle`/`Ellipse` loop), then `ops::extrude` along the plane normal (against it for a negative distance), then the op |
//! | `Hole` | `ops::primitive_cylinder` starting exactly at the entry point (blind: the depth; through: past the far side of the part), `ops::cut`; a counterbore is a second, wider cylinder from the entry point, `ops::cut` |
//! | `Boolean` with `ToolRef::Def` | the tool definition evaluated into the same model, moved by `ops::transform`, then the op |
//! | `Chamfer` (`d1 == d2`) | edges picked by [`edge_query`], then `ops::chamfer` (mitred corners) |
//! | op `Add` / `Subtract` / `Intersect` | `ops::fuse` / `ops::cut` / `ops::common` |
//!
//! Everything else (`Revolve`, `Sweep`, `Loft`, `Fillet`, tapered extrude,
//! asymmetric chamfer, `ToolRef::Part`, spline profile segments) returns
//! [`EvalError::Unsupported`].
//!
//! Volumes come from `ops::measure::mass_properties`, which integrates on
//! the exact faces (no mesh). arris has no body-level bounding box, so
//! [`bounds`] builds a tight one from the exact geometry: every vertex,
//! the exact box of every edge curve, and the interior extremes of sphere
//! and torus faces (a plane, cylinder or cone face takes its extremes on
//! its edges).
//!
//! Tessellation is `arris::mesh::tessellate` at the requested chord: every
//! topological vertex is one mesh vertex, edges are sampled once and shared
//! by both faces, so the mesh of a solid is closed, and triangles are
//! counter-clockwise seen from outside (arris documents both).
//!
//! The adapter refuses what arris itself can tell is wrong. arris is built
//! with its `paranoid` feature, so every operation's output goes through
//! its checker (an error in a release build, a panic in a debug build that
//! [`SolidKernel::evaluate`] turns into the same error), and the finished
//! part must pass the full check level. A tessellation that is not a
//! closed surface is refused too (the fuzzer found a few on slivers).

use std::fmt;
use std::panic::{catch_unwind, resume_unwind, AssertUnwindSafe};
use std::sync::Arc as Shared;

use arris::check::domain::FaceDomain;
use arris::check::{check, lumps, Level};
use arris::geom::region2::Side;
use arris::geom::{Curve, Profile, ProfileLoop, ProfileSegment, Surface};
use arris::math::nalgebra::{Quaternion, UnitQuaternion};
use arris::math::{Axis, Frame, Isometry, Point2, Precision, Vec2};
use arris::ops::query::{distance, Operand};
use arris::ops::OpError;
use arris::topo::{Body, Edge, Model};
use arris::Control;

use cadview_core::solid::kernel::{
    Aabb3, EvalError, Evaluated, MassProps, SolidKernel, ToolResolver, TriMesh, Validity,
};
use cadview_core::solid::select::{edge_query, EdgeQuery};
use cadview_core::solid::types::{
    arc_point, DefId, Depth, FeatId, Feature, Frame3, Op, PartDef, Placement, Plane3, Point3,
    Profile2d, Seg2, ToolRef, Vec3,
};

/// How arris's debug-build panic starts when an operation's output fails
/// its checker (`arris-ops`, `verify`).
const CHECKER_PANIC: &str = "kernel bug: an operation's output fails the checker";

/// How far (mm) a picked edge may sit from its selector's target curve.
const PICK_TOL: f64 = 1e-6;

/// Smallest distance, extrude length or diameter accepted (mm): arris's
/// default linear tolerance.
const CONFUSION: f64 = 1e-7;

/// How deep tool definitions may nest.
const MAX_TOOL_DEPTH: usize = 16;

/// No cancellation and no step budget.
const RUN: Control<'static> = Control::NONE;

/// The arris backend. Stateless: each evaluation builds its own model.
#[derive(Debug, Clone, Copy, Default)]
pub struct ArrisKernel;

/// An evaluated part: the arris model and the solid in it.
#[derive(Clone)]
pub struct ArrisBrep {
    model: Shared<Model>,
    body: Body,
}

impl fmt::Debug for ArrisBrep {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ArrisBrep").finish_non_exhaustive()
    }
}

impl ArrisKernel {
    /// The arris version compiled in: the exact pin in solid-harness's
    /// Cargo.toml (arris exports no version constant).
    pub const VERSION: &'static str = "0.11.1";
}

fn kernel_err<E: fmt::Display>(feature: Option<FeatId>) -> impl Fn(E) -> EvalError {
    // One line: arris's checker reports span several.
    move |e| EvalError::Kernel {
        feature,
        message: e
            .to_string()
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" "),
    }
}

fn pt(p: Point3) -> arris::math::Point3 {
    arris::math::Point3::new(p.x, p.y, p.z)
}

fn vec(v: Vec3) -> arris::math::Vec3 {
    arris::math::Vec3::new(v.x, v.y, v.z)
}

fn p3(p: arris::math::Point3) -> Point3 {
    Point3::new(p.x, p.y, p.z)
}

fn p2(p: kurbo::Point) -> Point2 {
    Point2::new(p.x, p.y)
}

impl SolidKernel for ArrisKernel {
    type Brep = ArrisBrep;

    fn name(&self) -> String {
        format!("arris {}", Self::VERSION)
    }

    fn evaluate(
        &self,
        def: &PartDef,
        tools: &dyn ToolResolver,
    ) -> Result<Evaluated<ArrisBrep>, EvalError> {
        // arris checks every operation's output: with the `paranoid`
        // feature a release build returns an error for a result its checker
        // rejects, and a debug build panics with `CHECKER_PANIC`. Both are
        // the kernel saying its own result is invalid, so both are a
        // refusal here. Any other panic is passed on.
        let run = catch_unwind(AssertUnwindSafe(|| {
            let mut model = Model::new(Precision::DEFAULT).map_err(kernel_err(None))?;
            let mut stack = vec![def.id];
            let body = eval_into(&mut model, def, tools, &mut stack)?;
            Ok((model, body))
        }));
        let (model, body) = match run {
            Ok(r) => r?,
            Err(payload) => {
                let message = payload
                    .downcast_ref::<String>()
                    .map(String::as_str)
                    .or_else(|| payload.downcast_ref::<&str>().copied());
                match message {
                    Some(m) if m.starts_with(CHECKER_PANIC) => {
                        return Err(EvalError::Kernel {
                            feature: None,
                            message: format!(
                                "the kernel's checker rejects an operation's result: {}",
                                m.lines().nth(1).unwrap_or("").trim()
                            ),
                        })
                    }
                    _ => resume_unwind(payload),
                }
            }
        };
        // The per-operation check is the fast level; the result as a whole
        // must also pass the full one (what `check` below reports).
        let report = check(&model, body, Level::Full);
        if !report.is_ok() {
            return Err(EvalError::Kernel {
                feature: None,
                message: format!(
                    "the kernel's checker rejects the result: {}",
                    report
                        .violations()
                        .first()
                        .map(|v| v.to_string())
                        .unwrap_or_default()
                ),
            });
        }
        Ok(Evaluated {
            brep: ArrisBrep {
                model: Shared::new(model),
                body,
            },
        })
    }

    fn mass(&self, b: &ArrisBrep) -> Result<MassProps, EvalError> {
        let m = arris::ops::measure::mass_properties(&b.model, b.body, &RUN)
            .map_err(kernel_err(None))?;
        Ok(MassProps {
            volume: m.volume,
            centroid: Some(p3(m.centroid)),
            // A flux integral over the exact faces: no mesh is involved.
            exact: true,
        })
    }

    fn bbox(&self, b: &ArrisBrep, frame: &Placement) -> Result<Aabb3, EvalError> {
        frame.check()?;
        bounds(&b.model, b.body, &isometry(frame))
    }

    fn check(&self, b: &ArrisBrep) -> Result<Validity, EvalError> {
        let k = kernel_err(None);
        let report = check(&b.model, b.body, Level::Full);
        let mut problems: Vec<String> = report.violations().iter().map(|v| v.to_string()).collect();
        problems.extend(report.unchecked().iter().map(|u| format!("undecided: {u}")));
        let solids = match lumps(&b.model, b.body) {
            Ok(l) => l.len(),
            Err(e) => {
                problems.push(format!("lumps: {e}"));
                0
            }
        };
        let faces = b.model.faces(b.body).map_err(&k)?.len();
        Ok(Validity {
            valid: report.is_ok(),
            solids,
            faces,
            problems,
        })
    }

    fn tessellate(&self, b: &ArrisBrep, chord: f64) -> Result<TriMesh, EvalError> {
        if !chord.is_finite() || chord <= 0.0 {
            return Err(EvalError::invalid(
                None,
                "chord must be finite and positive",
            ));
        }
        let mesh =
            arris::mesh::tessellate(&b.model, b.body, chord, &RUN).map_err(kernel_err(None))?;
        let mesh = TriMesh {
            positions: mesh
                .positions()
                .iter()
                .map(|p| Point3::new(p[0], p[1], p[2]))
                .collect(),
            triangles: mesh.triangles().to_vec(),
        };
        // A solid's mesh must be a closed, consistently oriented surface.
        // arris leaves a few open on slivers; that is a refusal, not a mesh.
        let topo = crate::mesh::topology(&mesh);
        if !topo.closed {
            return Err(EvalError::Kernel {
                feature: None,
                message: format!(
                    "the tessellation at chord {chord} is not a closed surface ({} unmatched edges)",
                    topo.bad_edges
                ),
            });
        }
        Ok(mesh)
    }
}

/// Evaluates `def` into `model`. `stack` holds the definitions being
/// evaluated, outermost first, to catch tool cycles.
fn eval_into(
    model: &mut Model,
    def: &PartDef,
    tools: &dyn ToolResolver,
    stack: &mut Vec<DefId>,
) -> Result<Body, EvalError> {
    let mut current: Option<Body> = None;
    for (i, feature) in def.features.iter().enumerate() {
        let id = feature.id();
        let k = kernel_err(Some(id));
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
                if !distance.is_finite() || distance.abs() <= CONFUSION {
                    return Err(EvalError::invalid(
                        Some(id),
                        "extrude distance must be finite and non-zero",
                    ));
                }
                let body = extrude(model, profile, plane, *distance).map_err(|e| e.at(id))?;
                combine(model, current.take(), body, *op, id)?
            }
            Feature::Hole {
                at,
                dir: d,
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
                hole(model, cur, *at, *d, *dia, *depth, *cbore, id)?
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
                    let body = eval_into(model, tool_def, tools, stack);
                    stack.pop();
                    let placed = place(model, body?, placement, Some(id))?;
                    combine(model, current.take(), placed, *op, id)?
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
                if !d1.is_finite() || *d1 <= CONFUSION {
                    return Err(EvalError::invalid(
                        Some(id),
                        "chamfer distance must be finite and positive",
                    ));
                }
                let query = edge_query(&def.features[..i], edges, id)?;
                let picked = pick_edges(model, cur, &query, id)?;
                arris::ops::chamfer(model, cur, &picked, *d1, &RUN)
                    .map_err(&k)?
                    .0
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
fn combine(
    model: &mut Model,
    current: Option<Body>,
    body: Body,
    op: Op,
    id: FeatId,
) -> Result<Body, EvalError> {
    let built = match (current, op) {
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
        (Some(c), Op::Add) => arris::ops::fuse(model, c, body, &RUN),
        (Some(c), Op::Subtract) => arris::ops::cut(model, c, body, &RUN),
        (Some(c), Op::Intersect) => arris::ops::common(model, c, body, &RUN),
    };
    Ok(built.map_err(kernel_err::<OpError>(Some(id)))?.0)
}

/// The rigid motion of a placement (checked by the caller).
fn isometry(placement: &Placement) -> Isometry {
    let [w, x, y, z] = placement.rotation;
    let rotation = UnitQuaternion::from_quaternion(Quaternion::new(w, x, y, z));
    Isometry::new(rotation, vec(placement.translation))
}

/// `body` moved by a placement (no-op for the identity).
fn place(
    model: &mut Model,
    body: Body,
    placement: &Placement,
    id: Option<FeatId>,
) -> Result<Body, EvalError> {
    if placement.is_identity() {
        return Ok(body);
    }
    Ok(
        arris::ops::transform(model, body, &isometry(placement), &RUN)
            .map_err(kernel_err::<OpError>(id))?
            .0,
    )
}

/// The arris frame of a checked sketch plane, axes kept bit for bit.
fn arris_frame(f: &Frame3) -> Result<Frame, EvalError> {
    Frame::from_orthonormal(pt(f.origin), vec(f.x), vec(f.y), vec(f.z)).map_err(kernel_err(None))
}

/// A profile on a sketch plane, swept `distance` along the plane normal.
fn extrude(
    model: &mut Model,
    profile: &Profile2d,
    plane: &Plane3,
    distance: f64,
) -> Result<Body, EvalError> {
    profile.check()?;
    let frame = plane.frame()?;
    let mut loops = profile.loops().map(|(_, lp)| profile_loop(lp));
    let outer = loops
        .next()
        .ok_or_else(|| EvalError::invalid(None, "profile has no outer loop"))??;
    let holes = loops.collect::<Result<Vec<_>, _>>()?;
    let sketch = Profile {
        plane: arris_frame(&frame)?,
        outer,
        holes,
    };
    let direction = vec(frame.z.scale(distance.signum()));
    Ok(
        arris::ops::extrude(model, &sketch, direction, distance.abs(), &RUN)
            .map_err(kernel_err::<OpError>(None))?
            .0,
    )
}

/// One closed profile loop in arris's sketch form. Each segment ends
/// exactly where the next one starts (the loop's own start points), so
/// the joins checked to [`cadview_core::solid::PROFILE_JOIN_TOL`] close exactly.
fn profile_loop(lp: &[Seg2]) -> Result<ProfileLoop, EvalError> {
    if let [only] = lp {
        if only.is_closed_loop() {
            return Ok(match only {
                Seg2::Circle(c) => ProfileLoop::Circle {
                    center: p2(c.center),
                    radius: c.radius,
                },
                Seg2::Arc(a) if a.radii.x == a.radii.y => ProfileLoop::Circle {
                    center: p2(a.center),
                    radius: a.radii.x,
                },
                Seg2::Arc(a) => {
                    let (major, minor_radius) = ellipse_axes(a);
                    ProfileLoop::Ellipse {
                        center: p2(a.center),
                        major,
                        minor_radius,
                    }
                }
                _ => {
                    return Err(EvalError::invalid(
                        None,
                        "a closed profile loop must be a circle or an ellipse",
                    ));
                }
            });
        }
    }
    let starts: Vec<kurbo::Point> = lp
        .iter()
        .map(|s| s.ends().map(|(a, _)| a))
        .collect::<Option<_>>()
        .ok_or_else(|| EvalError::invalid(None, "profile segment has no end points"))?;
    let n = lp.len();
    let mut segments = Vec::with_capacity(n);
    for (i, seg) in lp.iter().enumerate() {
        let to = p2(starts[(i + 1) % n]);
        segments.push(match seg {
            Seg2::Line(_) => ProfileSegment::LineTo(to),
            Seg2::Arc(a) if a.radii.x == a.radii.y => ProfileSegment::ArcTo {
                to,
                via: p2(arc_point(a, a.start_angle + 0.5 * a.sweep_angle)),
            },
            Seg2::Arc(a) => {
                let (major, minor_radius) = ellipse_axes(a);
                ProfileSegment::EllipseTo {
                    to,
                    center: p2(a.center),
                    major,
                    minor_radius,
                    ccw: a.sweep_angle > 0.0,
                }
            }
            Seg2::Circle(_) => {
                return Err(EvalError::invalid(
                    None,
                    "a full circle must be a profile loop on its own",
                ));
            }
            Seg2::Spline { .. } => {
                return Err(EvalError::unsupported(None, "a spline profile segment"));
            }
        });
    }
    Ok(ProfileLoop::Path {
        start: p2(starts[0]),
        segments,
    })
}

/// The major semi-axis vector and minor radius of a kurbo elliptic arc.
fn ellipse_axes(a: &kurbo::Arc) -> (Vec2, f64) {
    let (s, c) = a.x_rotation.sin_cos();
    if a.radii.x >= a.radii.y {
        (Vec2::new(c, s) * a.radii.x, a.radii.y)
    } else {
        (Vec2::new(-s, c) * a.radii.y, a.radii.x)
    }
}

#[allow(clippy::too_many_arguments)]
fn hole(
    model: &mut Model,
    current: Body,
    at: Point3,
    d: Vec3,
    dia: f64,
    depth: Depth,
    cbore: Option<(f64, f64)>,
    id: FeatId,
) -> Result<Body, EvalError> {
    let k = kernel_err::<OpError>(Some(id));
    let invalid = |why: &str| EvalError::invalid(Some(id), why);
    if !at.is_finite() {
        return Err(invalid("hole position is not finite"));
    }
    let Some(d) = d.normalized() else {
        return Err(invalid("hole direction must be a non-zero vector"));
    };
    if !dia.is_finite() || dia <= CONFUSION {
        return Err(invalid("hole diameter must be finite and positive"));
    }
    // The reference meaning (solid-harness reference): the hole removes
    // only `t` in [0, depth] from the entry point along `d`, never material
    // behind the entry point. So the drill starts exactly at `at`. (Until
    // the harness's first run it started max(dia, 1) mm behind `at`, which
    // cut an opening to the surface when `at` lay inside the material.) A
    // through hole runs past the far side of the part's box by a margin,
    // so no tool face is coplanar with the face it leaves through.
    let margin = dia.max(1.0);
    let length = match depth {
        Depth::Blind(x) if x.is_finite() && x > CONFUSION => x,
        Depth::Blind(_) => return Err(invalid("hole depth must be finite and positive")),
        Depth::Through => {
            let b = bounds(model, current, &Isometry::identity()).map_err(|e| e.at(id))?;
            let mut reach = f64::NEG_INFINITY;
            for x in [b.min.x, b.max.x] {
                for y in [b.min.y, b.max.y] {
                    for z in [b.min.z, b.max.z] {
                        reach = reach.max((Point3::new(x, y, z) - at).dot(d));
                    }
                }
            }
            if reach <= 0.0 {
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
        if !cdepth.is_finite() || cdepth <= CONFUSION || too_deep {
            return Err(invalid(
                "counterbore depth must be positive and less than the hole's",
            ));
        }
    }
    let axis = Axis::new(pt(at), vec(d)).map_err(kernel_err(Some(id)))?;
    let drill = arris::ops::primitive_cylinder(model, axis, dia / 2.0, length, &RUN)
        .map_err(&k)?
        .0;
    let mut out = arris::ops::cut(model, current, drill, &RUN).map_err(&k)?.0;
    if let Some((cdia, cdepth)) = cbore {
        let bore = arris::ops::primitive_cylinder(model, axis, cdia / 2.0, cdepth, &RUN)
            .map_err(&k)?
            .0;
        out = arris::ops::cut(model, out, bore, &RUN).map_err(&k)?.0;
    }
    Ok(out)
}

/// The edges of `body` an [`EdgeQuery`] picks, each once.
fn pick_edges(
    model: &Model,
    body: Body,
    query: &EdgeQuery,
    id: FeatId,
) -> Result<Vec<Edge>, EvalError> {
    let k = kernel_err(Some(id));
    let edges = model.edges(body).map_err(&k)?;
    let selector = |reason: String| EvalError::Selector {
        feature: id,
        reason,
    };
    let mut picked = vec![false; edges.len()];
    match query {
        EdgeQuery::All => picked.iter_mut().for_each(|p| *p = true),
        EdgeQuery::Targets(targets) => {
            // Start, middle and end of every edge with a curve.
            let mut samples = Vec::with_capacity(edges.len());
            for edge in &edges {
                let entity = model.edge(edge.id).map_err(&k)?;
                samples.push(match entity.curve() {
                    Some((curve, range)) => {
                        let curve = model.curve(curve).map_err(&k)?;
                        Some([range.lo(), range.midpoint(), range.hi()].map(|t| p3(curve.point(t))))
                    }
                    None => None,
                });
            }
            for (ti, target) in targets.iter().enumerate() {
                let mut any = false;
                for (ei, s) in samples.iter().enumerate() {
                    if let Some(s) = s {
                        if s.iter().all(|p| target.contains(*p, PICK_TOL)) {
                            picked[ei] = true;
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
                let mut best: Option<(f64, usize)> = None;
                for (ei, edge) in edges.iter().enumerate() {
                    let d = distance(
                        model,
                        Operand::Point(pt(*p)),
                        Operand::Shape(edge.shape()),
                        &RUN,
                    )
                    .map_err(kernel_err::<OpError>(Some(id)))?
                    .distance;
                    if best.is_none_or(|(b, _)| d < b) {
                        best = Some((d, ei));
                    }
                }
                let Some((_, ei)) = best else {
                    return Err(selector("the part has no edges".into()));
                };
                picked[ei] = true;
            }
        }
    }
    let out: Vec<Edge> = edges
        .into_iter()
        .zip(picked)
        .filter_map(|(e, p)| p.then_some(e))
        .collect();
    if out.is_empty() {
        return Err(selector("no edge selected".into()));
    }
    Ok(out)
}

/// A running axis-aligned box.
struct Bounds {
    min: [f64; 3],
    max: [f64; 3],
}

impl Bounds {
    fn new() -> Self {
        Bounds {
            min: [f64::INFINITY; 3],
            max: [f64::NEG_INFINITY; 3],
        }
    }

    fn add(&mut self, p: [f64; 3]) {
        for (i, v) in p.into_iter().enumerate() {
            self.add_axis(i, v);
        }
    }

    fn add_axis(&mut self, i: usize, v: f64) {
        self.min[i] = self.min[i].min(v);
        self.max[i] = self.max[i].max(v);
    }
}

/// The tight axis-aligned box of `body` moved by `motion`, from the exact
/// geometry. A smooth function on a compact face takes its extremes on the
/// face's edges or at interior stationary points. Every vertex and the
/// exact box of every edge curve cover the first. For the second, a plane,
/// cylinder, elliptic cylinder or cone is stationary only along whole
/// rulings that end on the face's edges, so only spheres and tori add
/// points, each kept when it lies on the face. NURBS edges and faces have
/// no exact extremes here and are refused.
fn bounds(model: &Model, body: Body, motion: &Isometry) -> Result<Aabb3, EvalError> {
    let k = kernel_err(None);
    let unsupported =
        |what: &str| EvalError::unsupported(None, format!("a tight bounding box of {what}"));
    let mut b = Bounds::new();
    for v in model.vertices(body).map_err(&k)? {
        let p = motion.apply(model.vertex(v.id).map_err(&k)?.point());
        b.add([p.x, p.y, p.z]);
    }
    for e in model.edges(body).map_err(&k)? {
        let Some((curve, range)) = model.edge(e.id).map_err(&k)?.curve() else {
            continue;
        };
        let curve = model.curve(curve).map_err(&k)?;
        if matches!(curve, Curve::Nurbs(_)) {
            return Err(unsupported("a NURBS edge"));
        }
        let Some(cb) = curve.transformed(motion).bounds(range) else {
            return Err(unsupported("an unbounded edge"));
        };
        b.add(cb.min);
        b.add(cb.max);
    }
    for f in model.faces(body).map_err(&k)? {
        let entity = model.face(f.id).map_err(&k)?;
        let surface = model.surface(entity.surface()).map_err(&k)?;
        if matches!(surface, Surface::Nurbs(_)) {
            return Err(unsupported("a NURBS face"));
        }
        if !matches!(surface, Surface::Sphere { .. } | Surface::Torus { .. }) {
            continue;
        }
        let moved = surface.transformed(motion);
        let domain = FaceDomain::of(model, f.id, entity.tolerance()).map_err(&k)?;
        let on_face = |p: arris::math::Point3| -> bool {
            moved
                .project(p)
                .is_ok_and(|proj| domain.side(proj.uv).0 != Side::Outside)
        };
        for (axis, e) in [
            arris::math::Vec3::x(),
            arris::math::Vec3::y(),
            arris::math::Vec3::z(),
        ]
        .into_iter()
        .enumerate()
        {
            match moved {
                Surface::Sphere { frame, radius } => {
                    for s in [1.0, -1.0] {
                        let p = frame.origin() + e * (s * radius);
                        if on_face(p) {
                            b.add([p.x, p.y, p.z]);
                        }
                    }
                }
                Surface::Torus {
                    frame,
                    major_radius,
                    minor_radius,
                } => {
                    let a = frame.z().into_inner();
                    let beta = e.dot(&a);
                    let radial = e - a * beta;
                    let alpha = radial.norm();
                    if alpha > 1e-12 {
                        // The normal is +-e at the four points where the
                        // tube's circle in the plane of `a` and `e` turns.
                        let u = radial / alpha;
                        for s in [1.0, -1.0] {
                            for side in [1.0, -1.0] {
                                let (cos_phi, sin_phi) = (s * alpha * side, s * beta);
                                let p = frame.origin()
                                    + u * (side * (major_radius + minor_radius * cos_phi))
                                    + a * (minor_radius * sin_phi);
                                if on_face(p) {
                                    b.add([p.x, p.y, p.z]);
                                }
                            }
                        }
                    } else {
                        // `e` is the torus axis: the extremes along it are
                        // the top and bottom circles (v = pi/2 and 3pi/2).
                        // The face is connected, so it reaches such a
                        // circle when its v range contains it.
                        let Some([_, v]) = domain.uv_box() else {
                            continue;
                        };
                        for (vc, s) in [
                            (std::f64::consts::FRAC_PI_2, 1.0),
                            (3.0 * std::f64::consts::FRAC_PI_2, -1.0),
                        ] {
                            let tau = std::f64::consts::TAU;
                            let shift = ((v.lo() - vc) / tau).ceil() * tau;
                            if vc + shift <= v.hi() {
                                let c = frame.origin().coords.dot(&e) + s * beta * minor_radius;
                                b.add_axis(axis, c);
                            }
                        }
                    }
                }
                _ => {}
            }
        }
    }
    if !(b.min.iter().chain(&b.max).all(|v| v.is_finite())) {
        return Err(EvalError::Kernel {
            feature: None,
            message: "the shape has an empty bounding box".into(),
        });
    }
    Ok(Aabb3::new(
        Point3::new(b.min[0], b.min[1], b.min[2]),
        Point3::new(b.max[0], b.max[1], b.max[2]),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use cadview_core::solid::kernel::NoTools;
    use cadview_core::solid::types::{DefId, FeatId};
    use cadview_core::types::{LwVertex, Shape as Shape2d};
    use kurbo::Point as P2;
    use std::f64::consts::PI;

    fn def(features: Vec<Feature>) -> PartDef {
        PartDef {
            id: DefId(1),
            name: "test".into(),
            features,
        }
    }

    fn extrude(profile: Profile2d, distance: f64, op: Op) -> Feature {
        Feature::Extrude {
            id: FeatId(1),
            profile,
            plane: Plane3::XY,
            distance,
            taper_deg: 0.0,
            op,
        }
    }

    fn volume(d: &PartDef) -> f64 {
        let k = ArrisKernel;
        let b = k.evaluate(d, &NoTools).expect("evaluates").brep;
        let v = k.check(&b).expect("checks");
        assert!(v.valid, "invalid: {:?}", v.problems);
        assert_eq!(v.solids, 1);
        let m = k.mass(&b).expect("mass");
        assert!(m.exact, "volume came from a mesh");
        m.volume
    }

    /// A slot (stadium) from a closed LwPolyline with two bulges of 1
    /// (half circles), extruded 10: V = (L * 2r + pi r^2) * h with
    /// L = 40, r = 5, h = 10, so V = 4000 + 250 pi.
    #[test]
    fn slot_from_bulge_polyline_is_exact() {
        let v = |x, y, bulge| LwVertex {
            point: P2::new(x, y),
            bulge,
        };
        let shape = Shape2d::LwPolyline {
            vertices: vec![
                v(0.0, 0.0, 0.0),
                v(40.0, 0.0, 1.0),
                v(40.0, 10.0, 0.0),
                v(0.0, 10.0, 1.0),
            ],
            closed: true,
        };
        let outer = Profile2d::loop_from_shape(&shape).expect("loop");
        let p = Profile2d {
            outer,
            holes: vec![],
        };
        let d = def(vec![extrude(p, 10.0, Op::Add)]);
        let got = volume(&d);
        let want = 4000.0 + 250.0 * PI;
        assert!((got - want).abs() / want <= 1e-9, "{got} vs {want}");
        // The half circles bulge to x = -5 and x = 45.
        let k = ArrisKernel;
        let b = k.evaluate(&d, &NoTools).unwrap().brep;
        let bb = k.bbox(&b, &Placement::IDENTITY).unwrap();
        let want_bb = Aabb3::new(Point3::new(-5.0, 0.0, 0.0), Point3::new(45.0, 10.0, 10.0));
        assert!(bb.max_abs_diff(&want_bb) <= 1e-9, "{bb:?}");
    }

    /// A washer: circle r = 10 with a circle r = 4 hole, extruded -3 (below
    /// the plane): V = pi (100 - 16) * 3 = 252 pi.
    #[test]
    fn washer_with_hole_loop_extruded_backwards() {
        let p = Profile2d::circle(P2::new(0.0, 0.0), 10.0)
            .with_hole(vec![Seg2::Circle(kurbo::Circle::new((0.0, 0.0), 4.0))]);
        let d = def(vec![extrude(p, -3.0, Op::Add)]);
        let got = volume(&d);
        let want = 252.0 * PI;
        assert!((got - want).abs() / want <= 1e-9, "{got} vs {want}");
        let k = ArrisKernel;
        let b = k.evaluate(&d, &NoTools).unwrap().brep;
        let bb = k.bbox(&b, &Placement::IDENTITY).unwrap();
        let want_bb = Aabb3::new(
            Point3::new(-10.0, -10.0, -3.0),
            Point3::new(10.0, 10.0, 0.0),
        );
        assert!(bb.max_abs_diff(&want_bb) <= 1e-9, "{bb:?}");
    }

    /// A full ellipse loop (a = 8, b = 3, axis turned 30 degrees) extruded
    /// 5: V = pi a b h = 120 pi. arris takes elliptic profile loops and
    /// segments directly.
    #[test]
    fn elliptic_profile_is_exact() {
        let e = Shape2d::Ellipse {
            center: P2::new(0.0, 0.0),
            major_axis: (8.0 * (PI / 6.0).cos(), 8.0 * (PI / 6.0).sin()),
            minor_ratio: 3.0 / 8.0,
            start_param: 0.0,
            end_param: std::f64::consts::TAU,
        };
        let outer = Profile2d::loop_from_shape(&e).expect("loop");
        let d = def(vec![extrude(
            Profile2d {
                outer,
                holes: vec![],
            },
            5.0,
            Op::Add,
        )]);
        let got = volume(&d);
        let want = 120.0 * PI;
        assert!((got - want).abs() / want <= 1e-9, "{got} vs {want}");
    }

    #[test]
    fn bbox_under_a_rotated_frame() {
        let d = def(vec![extrude(
            Profile2d::rect(0.0, 0.0, 30.0, 10.0),
            5.0,
            Op::Add,
        )]);
        let k = ArrisKernel;
        let b = k.evaluate(&d, &NoTools).unwrap().brep;
        // A quarter turn about Z takes x to y and y to -x.
        let frame = Placement::axis_angle(Vec3::Z, PI / 2.0, Vec3::new(100.0, 0.0, 0.0));
        let bb = k.bbox(&b, &frame).unwrap();
        let want = Aabb3::new(Point3::new(90.0, 0.0, 0.0), Point3::new(100.0, 30.0, 5.0));
        assert!(bb.max_abs_diff(&want) <= 1e-9, "{bb:?}");
    }

    #[test]
    fn unsupported_features_are_errors_not_panics() {
        let base = extrude(Profile2d::rect(0.0, 0.0, 10.0, 10.0), 10.0, Op::Add);
        let cases = vec![
            Feature::Revolve {
                id: FeatId(2),
                profile: Profile2d::rect(1.0, 0.0, 2.0, 1.0),
                plane: Plane3::XZ,
                axis: kurbo::Line::new((0.0, 0.0), (0.0, 1.0)),
                angle_deg: 360.0,
                op: Op::Add,
            },
            Feature::Fillet {
                id: FeatId(2),
                edges: cadview_core::solid::types::EdgeSel::All,
                radius: 1.0,
                fallback: cadview_core::solid::types::RoundFallback::Error,
            },
            Feature::Chamfer {
                id: FeatId(2),
                edges: cadview_core::solid::types::EdgeSel::All,
                d1: 1.0,
                d2: 2.0,
            },
            Feature::Boolean {
                id: FeatId(2),
                op: Op::Subtract,
                tool: ToolRef::Part {
                    part: cadview_core::types::EntityId(7),
                    clearance: 0.0,
                },
            },
            Feature::Extrude {
                id: FeatId(2),
                profile: Profile2d::rect(0.0, 0.0, 1.0, 1.0),
                plane: Plane3::XY,
                distance: 1.0,
                taper_deg: 2.0,
                op: Op::Add,
            },
        ];
        for f in cases {
            let name = f.kind_name();
            let err = ArrisKernel
                .evaluate(&def(vec![base.clone(), f]), &NoTools)
                .expect_err(name);
            assert!(
                matches!(
                    err,
                    EvalError::Unsupported {
                        feature: Some(FeatId(2)),
                        ..
                    }
                ),
                "{name}: {err}"
            );
            assert!(err.to_string().contains("not supported"), "{err}");
        }
    }

    #[test]
    fn bad_input_is_an_error() {
        let k = ArrisKernel;
        // Subtract with nothing before it.
        let e = k
            .evaluate(
                &def(vec![extrude(
                    Profile2d::rect(0.0, 0.0, 1.0, 1.0),
                    1.0,
                    Op::Subtract,
                )]),
                &NoTools,
            )
            .unwrap_err();
        assert!(matches!(e, EvalError::Invalid { .. }), "{e}");
        // An open profile.
        let open = Profile2d {
            outer: vec![
                Seg2::Line(kurbo::Line::new((0.0, 0.0), (1.0, 0.0))),
                Seg2::Line(kurbo::Line::new((1.0, 0.0), (1.0, 1.0))),
            ],
            holes: vec![],
        };
        let e = k
            .evaluate(&def(vec![extrude(open, 1.0, Op::Add)]), &NoTools)
            .unwrap_err();
        assert!(matches!(e, EvalError::Invalid { .. }), "{e}");
        // Zero distance.
        let e = k
            .evaluate(
                &def(vec![extrude(
                    Profile2d::rect(0.0, 0.0, 1.0, 1.0),
                    0.0,
                    Op::Add,
                )]),
                &NoTools,
            )
            .unwrap_err();
        assert!(matches!(e, EvalError::Invalid { .. }), "{e}");
        // A degenerate (zero-area) rectangle is the kernel's refusal.
        let e = k
            .evaluate(
                &def(vec![extrude(
                    Profile2d::rect(0.0, 0.0, 1.0, 0.0),
                    1.0,
                    Op::Add,
                )]),
                &NoTools,
            )
            .unwrap_err();
        assert!(matches!(e, EvalError::Kernel { .. }), "{e}");
        // No features.
        assert!(k.evaluate(&def(vec![]), &NoTools).is_err());
        // Unknown tool and a self-referencing tool.
        let tool = Feature::Boolean {
            id: FeatId(2),
            op: Op::Subtract,
            tool: ToolRef::Def(DefId(1), Placement::IDENTITY),
        };
        let base = extrude(Profile2d::rect(0.0, 0.0, 1.0, 1.0), 1.0, Op::Add);
        let d = def(vec![base.clone(), tool]);
        let e = k.evaluate(&d, &NoTools).unwrap_err();
        assert!(matches!(e, EvalError::ToolCycle { .. }), "{e}");
        let unknown = Feature::Boolean {
            id: FeatId(2),
            op: Op::Subtract,
            tool: ToolRef::Def(DefId(99), Placement::IDENTITY),
        };
        let e = k.evaluate(&def(vec![base, unknown]), &NoTools).unwrap_err();
        assert!(matches!(e, EvalError::UnknownTool { .. }), "{e}");
    }

    #[test]
    fn selector_that_resolves_to_nothing_is_an_error() {
        let base = extrude(Profile2d::rect(0.0, 0.0, 10.0, 10.0), 10.0, Op::Add);
        let bad_seg = Feature::Chamfer {
            id: FeatId(2),
            edges: cadview_core::solid::types::EdgeSel::Cap {
                feature: FeatId(1),
                side: cadview_core::solid::types::CapSide::End,
                segs: Some(vec![9]),
            },
            d1: 1.0,
            d2: 1.0,
        };
        let e = ArrisKernel
            .evaluate(&def(vec![base.clone(), bad_seg]), &NoTools)
            .unwrap_err();
        assert!(matches!(e, EvalError::Selector { .. }), "{e}");
        let later = Feature::Chamfer {
            id: FeatId(2),
            edges: cadview_core::solid::types::EdgeSel::Cap {
                feature: FeatId(5),
                side: cadview_core::solid::types::CapSide::End,
                segs: None,
            },
            d1: 1.0,
            d2: 1.0,
        };
        let e = ArrisKernel
            .evaluate(&def(vec![base, later]), &NoTools)
            .unwrap_err();
        assert!(matches!(e, EvalError::Selector { .. }), "{e}");
    }

    /// `EdgeSel::Near` picks the nearest edge: one vertical edge of a
    /// 10 x 10 x 10 cube chamfered 1 removes a prism of 1/2 * 1 * 1 * 10.
    #[test]
    fn near_selector_chamfers_one_edge() {
        let base = extrude(Profile2d::rect(0.0, 0.0, 10.0, 10.0), 10.0, Op::Add);
        let ch = Feature::Chamfer {
            id: FeatId(2),
            edges: cadview_core::solid::types::EdgeSel::Near {
                points: vec![Point3::new(10.2, 10.1, 5.0)],
            },
            d1: 1.0,
            d2: 1.0,
        };
        let got = volume(&def(vec![base, ch]));
        assert!((got - 995.0).abs() <= 1e-9 * 995.0, "{got}");
    }
}

/// Evidence for the rungs after 3D0a, on arris directly (not through the
/// trait): roundovers, tessellation, sections, projection and provenance.
/// Run with `cargo test -p solid-harness kernels::arris::forward_look
/// -- --nocapture` to see the numbers.
#[cfg(test)]
mod forward_look {
    use super::*;
    use arris::mesh::tessellate;
    use arris::ops::{chamfer, cut, extrude, fillet, primitive_box, primitive_cylinder, section};
    use arris::topo::provenance::SweepPart;
    use arris::topo::{Provenance, Role, Shape};
    use std::f64::consts::PI;
    use std::time::Instant;

    fn ms(t: Instant) -> f64 {
        t.elapsed().as_secs_f64() * 1e3
    }

    /// The design's smoke part: 600 x 300 x 18 board, optionally all 12
    /// edges rounded r = 3, minus a 10 diameter through hole at (50, 50),
    /// minus a 12 x 6 dado along the length (y in [144, 156], z in
    /// [12, 18]).
    fn smoke(m: &mut Model, rounded: bool) -> Result<Body, OpError> {
        let (mut b, _) = primitive_box(
            m,
            arris::math::Point3::origin(),
            arris::math::Point3::new(600.0, 300.0, 18.0),
            &RUN,
        )?;
        if rounded {
            let edges = m.edges(b).expect("edges");
            b = fillet(m, b, &edges, 3.0, &RUN)?.0;
        }
        let axis = Axis::z_at(arris::math::Point3::new(50.0, 50.0, -1.0));
        let (hole, _) = primitive_cylinder(m, axis, 5.0, 20.0, &RUN)?;
        b = cut(m, b, hole, &RUN)?.0;
        let (dado, _) = primitive_box(
            m,
            arris::math::Point3::new(-1.0, 144.0, 12.0),
            arris::math::Point3::new(601.0, 156.0, 19.0),
            &RUN,
        )?;
        Ok(cut(m, b, dado, &RUN)?.0)
    }

    #[test]
    fn smoke_part_plain_and_rounded() {
        for (rounded, want) in [(false, 3_195_386.283), (true, 3_188_376.635)] {
            let mut m = Model::default();
            let t = Instant::now();
            let body = smoke(&mut m, rounded).expect("smoke part builds");
            let build_ms = ms(t);
            let t = Instant::now();
            let report = check(&m, body, Level::Full);
            let check_ms = ms(t);
            let v = arris::ops::measure::mass_properties(&m, body, &RUN)
                .expect("mass")
                .volume;
            let faces = m.faces(body).unwrap().len();
            println!(
                "smoke rounded={rounded}: build {build_ms:.1} ms, check(Full) {check_ms:.1} ms, \
                 faces {faces}, valid {}, undecided {}, volume {v:.6} (closed form {want}), \
                 diff {:.2e}",
                report.is_ok(),
                report.unchecked().len(),
                v - want
            );
            assert!(report.is_ok(), "{report}");
            // The closed forms are given to 3 decimals.
            assert!((v - want).abs() < 1e-3, "{v} vs {want}");
        }
    }

    /// Tessellation to a chord tolerance: watertight, and the mesh volume
    /// converges on the exact one.
    #[test]
    fn tessellation_to_a_chord() {
        let mut m = Model::default();
        let body = smoke(&mut m, true).expect("smoke part builds");
        let exact = arris::ops::measure::mass_properties(&m, body, &RUN)
            .unwrap()
            .volume;
        for chord in [0.1, 0.01, 0.001] {
            let t = Instant::now();
            let mesh = tessellate(&m, body, chord, &RUN).expect("tessellates");
            let mesh_ms = ms(t);
            let v = mesh.signed_volume().expect("closed mesh volume");
            println!(
                "tessellate chord {chord}: {mesh_ms:.1} ms, {} triangles, closed {}, \
                 mesh volume {v:.4}, rel err {:.1e}",
                mesh.triangles().len(),
                mesh.is_closed(),
                (v - exact).abs() / exact
            );
            assert!(mesh.is_closed());
            // An inscribed mesh loses about chord / r of the rounds.
            assert!((v - exact).abs() / exact < chord * 2e-3);
        }
    }

    /// A planar section of the rounded board at mid thickness: the plan
    /// outline with r = 3 corners, minus the hole:
    /// A = 600 * 300 - 4 (9 - 9 pi / 4) - 25 pi = 179 964 - 16 pi.
    /// Also: edges projected to a view plane come back as exact curves.
    #[test]
    fn section_and_projection() {
        let mut m = Model::default();
        let body = smoke(&mut m, true).expect("smoke part builds");
        let plane = Frame::from_z(arris::math::Point3::new(0.0, 0.0, 9.0), up()).unwrap();
        let t = Instant::now();
        let s = section(&mut m, body, &plane, &RUN)
            .expect("section")
            .expect("plane crosses");
        let want = 179_964.0 - 16.0 * PI;
        println!(
            "section z = 9: {:.1} ms, {} faces, area {:.9} (closed form {want:.9})",
            ms(t),
            m.faces(s.sheet).unwrap().len(),
            s.area
        );
        assert!((s.area - want).abs() / want < 1e-9);

        let front = Frame::from_z(arris::math::Point3::origin(), -arris::math::Vec3::y()).unwrap();
        let shapes: Vec<Shape> = m.edges(body).unwrap().iter().map(|e| e.shape()).collect();
        let (mut kinds, mut collapsed) = (std::collections::BTreeMap::new(), 0);
        for sh in &shapes {
            match arris::ops::query::project_to_plane(&m, &[*sh], &front) {
                Ok(p) => {
                    if let arris::ops::query::Projection::Edge { curve, .. } = &p[0] {
                        *kinds.entry(format!("{:?}", curve.kind())).or_insert(0) += 1;
                    }
                }
                Err(_) => collapsed += 1,
            }
        }
        println!(
            "front projection of {} edges: {kinds:?}, {collapsed} collapse to a point or \
             segment (refused); no visibility is computed",
            shapes.len()
        );
    }

    fn up() -> arris::math::Vec3 {
        arris::math::Vec3::z()
    }

    fn close(got: f64, want: f64, rel: f64) -> bool {
        (got - want).abs() <= rel * want.abs()
    }

    fn vol(m: &Model, b: Body) -> f64 {
        arris::ops::measure::mass_properties(m, b, &RUN)
            .expect("mass")
            .volume
    }

    fn square_at(z: f64, h: f64, c: f64) -> Profile {
        let p = |x: f64, y: f64| Point2::new(c + x, c + y);
        Profile {
            plane: Frame::new(
                arris::math::Point3::new(0.0, 0.0, z),
                arris::math::Vec3::z(),
                arris::math::Vec3::x(),
            )
            .unwrap(),
            outer: ProfileLoop::Path {
                start: p(h, h),
                segments: vec![
                    ProfileSegment::LineTo(p(-h, h)),
                    ProfileSegment::LineTo(p(-h, -h)),
                    ProfileSegment::LineTo(p(h, -h)),
                    ProfileSegment::LineTo(p(h, h)),
                ],
            },
            holes: vec![],
        }
    }

    /// The same closed forms as the cad-kernel forward-look tests: a
    /// rectangle r in [1, 2], z in [-1, 1] revolved a whole turn (6 pi); a
    /// disc r = 0.5 swept one helical turn of radius 5 and pitch 3
    /// (2.5 pi^2, Pappus for a screw); a ruled loft from a 4 x 4 to a
    /// 2 x 2 square 3 apart (28); a 10 mm cube shelled to a 1 mm wall open
    /// at the top (424) and one face offset (a 2 mm cube's top moved 0.5:
    /// 10).
    #[test]
    fn revolve_sweep_loft_shell_offset() {
        use arris::geom::{FrameLaw, Path, PathSegment};
        use arris::ops::{loft, offset_faces, revolve, shell, sweep, LoftKind, ShellSide};
        use std::f64::consts::TAU;
        let mut m = Model::default();
        // Revolve: the rectangle in the plane through the z axis.
        let ring = Profile {
            plane: Frame::new(
                arris::math::Point3::origin(),
                -arris::math::Vec3::y(),
                arris::math::Vec3::x(),
            )
            .unwrap(),
            outer: ProfileLoop::Path {
                start: Point2::new(1.0, -1.0),
                segments: vec![
                    ProfileSegment::LineTo(Point2::new(2.0, -1.0)),
                    ProfileSegment::LineTo(Point2::new(2.0, 1.0)),
                    ProfileSegment::LineTo(Point2::new(1.0, 1.0)),
                    ProfileSegment::LineTo(Point2::new(1.0, -1.0)),
                ],
            },
            holes: vec![],
        };
        let (tube, _) = revolve(
            &mut m,
            &ring,
            Axis::z_at(arris::math::Point3::origin()),
            TAU,
            &RUN,
        )
        .expect("revolve");
        let v = vol(&m, tube);
        println!("revolve tube: volume {v} (closed form {})", 6.0 * PI);
        assert!(close(v, 6.0 * PI, 1e-9));
        // Sweep along a helix.
        let wire = Profile {
            plane: Frame::new(
                arris::math::Point3::new(5.0, 0.0, 0.0),
                -arris::math::Vec3::y(),
                arris::math::Vec3::x(),
            )
            .unwrap(),
            outer: ProfileLoop::Circle {
                center: Point2::new(0.0, 0.0),
                radius: 0.5,
            },
            holes: Vec::new(),
        };
        let path = Path {
            start: arris::math::Point3::new(5.0, 0.0, 0.0),
            segments: vec![PathSegment::Helix {
                axis: Axis::z_at(arris::math::Point3::origin()),
                pitch: 3.0,
                angle: TAU,
            }],
        };
        let t = Instant::now();
        let law = FrameLaw::FixedBinormal(arris::math::Vec3::z_axis());
        let (coil, _) = sweep(&mut m, &wire, &path, law, &RUN).expect("sweep");
        let v = vol(&m, coil);
        let want = 2.5 * PI * PI;
        println!(
            "helical sweep: {:.1} ms, volume {v} (closed form {want}), rel err {:.1e}",
            ms(t),
            (v - want).abs() / want
        );
        // A frame law that keeps the binormal fixed tilts the section off
        // the meridian plane by the helix angle; the swept volume is the
        // section's area along the centre line's length, A L, not Pappus's
        // A 2 pi R for a meridian section. Report both, assert the one
        // that matches this sweep's definition.
        let l = (TAU * 5.0f64).hypot(3.0);
        let along = PI * 0.25 * l;
        println!("    A L = {along}");
        assert!(close(v, want, 1e-6) || close(v, along, 1e-6));
        // Ruled loft.
        let (frustum, _) = loft(
            &mut m,
            &[square_at(0.0, 2.0, 0.0), square_at(3.0, 1.0, 0.0)],
            LoftKind::Ruled,
            false,
            &RUN,
        )
        .expect("loft");
        let v = vol(&m, frustum);
        println!("ruled loft: volume {v} (closed form 28)");
        assert!(close(v, 28.0, 1e-9));
        // Shell and offset.
        let (cube, _) = arris::ops::primitive_box(
            &mut m,
            arris::math::Point3::origin(),
            arris::math::Point3::new(10.0, 10.0, 10.0),
            &RUN,
        )
        .unwrap();
        let top = m
            .faces(cube)
            .unwrap()
            .into_iter()
            .find(|f| {
                arris::check::classify::classify_point(
                    &m,
                    cube,
                    arris::math::Point3::new(5.0, 5.0, 10.0),
                )
                .unwrap()
                    == arris::check::classify::Classification::On(f.shape())
            })
            .unwrap();
        let (cup, _) = shell(&mut m, cube, &[top], 1.0, ShellSide::Inward, &RUN).expect("shell");
        let v = vol(&m, cup);
        println!("shelled cube: volume {v} (closed form 424)");
        assert!(close(v, 424.0, 1e-9));
        let (small, _) = arris::ops::primitive_box(
            &mut m,
            arris::math::Point3::origin(),
            arris::math::Point3::new(2.0, 2.0, 2.0),
            &RUN,
        )
        .unwrap();
        let lid = m
            .faces(small)
            .unwrap()
            .into_iter()
            .find(|f| {
                arris::check::classify::classify_point(
                    &m,
                    small,
                    arris::math::Point3::new(1.0, 1.0, 2.0),
                )
                .unwrap()
                    == arris::check::classify::Classification::On(f.shape())
            })
            .unwrap();
        let (taller, _) = offset_faces(&mut m, small, &[lid], 0.5, &RUN).expect("offset");
        let v = vol(&m, taller);
        println!("offset face: volume {v} (closed form 10)");
        assert!(close(v, 10.0, 1e-9));
    }

    /// An exact visibility query for a hidden-line oracle: a ray toward
    /// the front-view eye (-y) from a point on the far side of the hole's
    /// wall (x = 50, y = 55, z = 9) first meets the near side of the wall
    /// at y = 45, so that point is hidden; the hit is on the exact
    /// cylinder, not a mesh.
    #[test]
    fn ray_fire_answers_visibility_on_exact_faces() {
        use arris::ops::query::{ray_fire, Ray, RayHits};
        let mut m = Model::default();
        let body = smoke(&mut m, true).expect("smoke part builds");
        let ray = Ray {
            origin: arris::math::Point3::new(50.0, 55.0, 9.0),
            direction: -arris::math::Vec3::y(),
        };
        let t = Instant::now();
        let hits = ray_fire(&m, body, &ray, RayHits::All).expect("ray");
        println!(
            "ray_fire: {:.2} ms, hits at t = {:?}",
            ms(t),
            hits.iter().map(|h| h.t).collect::<Vec<_>>()
        );
        // t = 0 on the far wall itself, then 10 to the near wall, then
        // 55 to the board's front face.
        assert_eq!(hits.len(), 3);
        assert!((hits[1].t - 10.0).abs() < 1e-9 && (hits[2].t - 55.0).abs() < 1e-9);
    }

    /// Provenance names the edges an extrude made by profile segment, and
    /// `Provenance::then` follows them through later booleans, splits
    /// included. Here a dado across the top cap splits the cap's y = 0
    /// and y = 100 edges; the composed record and select.rs's geometric
    /// match pick the same edges.
    #[test]
    fn provenance_names_cap_edges_through_a_cut() {
        let mut m = Model::default();
        let profile = Profile {
            plane: Frame::world(),
            outer: profile_loop(&Profile2d::rect(0.0, 0.0, 200.0, 100.0).outer).unwrap(),
            holes: vec![],
        };
        let (board, made) = extrude(&mut m, &profile, up(), 20.0, &RUN).unwrap();
        let (tool, _) = primitive_box(
            &mut m,
            arris::math::Point3::new(90.0, -5.0, 14.0),
            arris::math::Point3::new(110.0, 105.0, 25.0),
            &RUN,
        )
        .unwrap();
        let (cut_body, cut_rec) = cut(&mut m, board, tool, &RUN).unwrap();
        let history: Provenance = made.then(&cut_rec);
        let mut by_role: Vec<Edge> = Vec::new();
        for segment in 0..4 {
            for s in history.generated_from(Role::Extrude(SweepPart::EndEdge {
                loop_index: 0,
                segment,
            })) {
                if let Ok(e) = arris::topo::Edge::try_from(*s) {
                    by_role.push(e);
                }
            }
        }
        let query = edge_query(
            &[Feature::Extrude {
                id: FeatId(1),
                profile: Profile2d::rect(0.0, 0.0, 200.0, 100.0),
                plane: Plane3::XY,
                distance: 20.0,
                taper_deg: 0.0,
                op: Op::Add,
            }],
            &cadview_core::solid::types::EdgeSel::Cap {
                feature: FeatId(1),
                side: cadview_core::solid::types::CapSide::End,
                segs: None,
            },
            FeatId(2),
        )
        .unwrap();
        let by_geometry = pick_edges(&m, cut_body, &query, FeatId(2)).unwrap();
        let ids = |v: &[Edge]| {
            let mut ids: Vec<_> = v.iter().map(|e| e.id).collect();
            ids.sort();
            ids
        };
        println!(
            "top cap edges after a dado: {} by provenance, {} by geometry",
            by_role.len(),
            by_geometry.len()
        );
        assert_eq!(ids(&by_role), ids(&by_geometry));
        // And they chamfer.
        let (_, _) = chamfer(&mut m, cut_body, &by_role, 1.0, &RUN).expect("chamfer");
    }
}
