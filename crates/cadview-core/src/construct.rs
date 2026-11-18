//! Construction operations: intersections, fillet, chamfer, trim/extend
//! to boundaries, tangents and arrays.
//!
//! Everything works on abstract geometry (lines, arcs, circles). Fillets
//! create real `Shape::Arc` entities; nothing is flattened. The pure
//! geometry functions are public so they can be unit-tested and reused;
//! the `Document`-level operations keep entity IDs stable (they mutate
//! the inputs in place instead of removing and re-adding them).

use crate::document::{Document, DrawEntity};
use crate::types::*;
use kurbo::{Affine, Line, Point, Vec2};
use std::f64::consts::{PI, TAU};

/// Geometric tolerance (drawing units) for "on the curve" tests.
const EPS: f64 = 1e-9;
/// Looser tolerance for extent checks (points computed by intersection).
const TOL: f64 = 1e-7;

// ── Small vector helpers ───────────────────────────────────────────────

fn rot90(v: Vec2) -> Vec2 {
    Vec2::new(-v.y, v.x)
}

fn unit(v: Vec2) -> Vec2 {
    let l = v.hypot();
    if l < 1e-15 {
        Vec2::ZERO
    } else {
        v / l
    }
}

/// Normalize an angle into [0, 2π).
pub fn norm_angle(a: f64) -> f64 {
    let r = a.rem_euclid(TAU);
    if r >= TAU {
        0.0
    } else {
        r
    }
}

fn angle_of(p: Point, c: Point) -> f64 {
    (p.y - c.y).atan2(p.x - c.x)
}

fn polar(c: Point, r: f64, a: f64) -> Point {
    Point::new(c.x + r * a.cos(), c.y + r * a.sin())
}

// ── Curve primitive ────────────────────────────────────────────────────

/// A line segment or circular arc (a full circle is an arc with sweep 2π).
/// Arcs always run counter-clockwise from `start` through `sweep`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Curve {
    Seg {
        p0: Point,
        p1: Point,
    },
    Arc {
        c: Point,
        r: f64,
        start: f64,
        sweep: f64,
    },
}

impl Curve {
    pub fn is_full_circle(&self) -> bool {
        matches!(self, Curve::Arc { sweep, .. } if *sweep >= TAU - 1e-12)
    }

    /// Is `p` (assumed on the carrier) within the finite extent?
    pub fn contains(&self, p: Point, tol: f64) -> bool {
        match *self {
            Curve::Seg { p0, p1 } => {
                let d = p1 - p0;
                let len2 = d.hypot2();
                if len2 < 1e-24 {
                    return (p - p0).hypot() <= tol;
                }
                let t = (p - p0).dot(d) / len2;
                let tl = tol / len2.sqrt();
                t >= -tl && t <= 1.0 + tl
            }
            Curve::Arc {
                c, r, start, sweep, ..
            } => {
                if sweep >= TAU - 1e-12 {
                    return true;
                }
                let rel = norm_angle(angle_of(p, c) - start);
                let at = if r > 0.0 { tol / r } else { 0.0 };
                rel <= sweep + at || rel >= TAU - at
            }
        }
    }

    /// Distance from `p` to the finite curve.
    pub fn distance_to(&self, p: Point) -> f64 {
        match *self {
            Curve::Seg { p0, p1 } => crate::geo::distance_to_segment(p, p0, p1),
            Curve::Arc { c, r, start, sweep } => {
                let a = angle_of(p, c);
                if self.contains(polar(c, r, a), 0.0) {
                    ((p - c).hypot() - r).abs()
                } else {
                    let e0 = polar(c, r, start);
                    let e1 = polar(c, r, start + sweep);
                    (p - e0).hypot().min((p - e1).hypot())
                }
            }
        }
    }

    fn endpoints(&self) -> Option<(Point, Point)> {
        match *self {
            Curve::Seg { p0, p1 } => Some((p0, p1)),
            Curve::Arc { c, r, start, sweep } => {
                if self.is_full_circle() {
                    None
                } else {
                    Some((polar(c, r, start), polar(c, r, start + sweep)))
                }
            }
        }
    }
}

/// Intersections of the *carriers* (infinite line / full circle) of two curves.
pub fn carrier_intersections(a: &Curve, b: &Curve) -> Vec<Point> {
    match (*a, *b) {
        (Curve::Seg { p0, p1 }, Curve::Seg { p0: q0, p1: q1 }) => {
            line_line_intersection(p0, p1, q0, q1, false)
                .into_iter()
                .collect()
        }
        (Curve::Seg { p0, p1 }, Curve::Arc { c, r, .. })
        | (Curve::Arc { c, r, .. }, Curve::Seg { p0, p1 }) => {
            crate::geo::line_circle_intersection(p0, p1, c, r)
        }
        (Curve::Arc { c: c1, r: r1, .. }, Curve::Arc { c: c2, r: r2, .. }) => {
            crate::geo::circle_circle_intersection(c1, r1, c2, r2)
        }
    }
}

/// Intersections of two finite curves.
pub fn curve_intersections(a: &Curve, b: &Curve) -> Vec<Point> {
    carrier_intersections(a, b)
        .into_iter()
        .filter(|p| a.contains(*p, TOL) && b.contains(*p, TOL))
        .collect()
}

/// Decompose a shape into line/arc curves. Returns an empty vec for shapes
/// that have no simple curve representation (text, splines, fills...).
pub fn shape_curves(shape: &Shape) -> Vec<Curve> {
    match shape {
        Shape::Line(l) => vec![Curve::Seg { p0: l.p0, p1: l.p1 }],
        Shape::Circle(c) => vec![Curve::Arc {
            c: c.center,
            r: c.radius,
            start: 0.0,
            sweep: TAU,
        }],
        Shape::Arc {
            center,
            radius,
            start_angle,
            end_angle,
        } => vec![arc_curve(*center, *radius, *start_angle, *end_angle)],
        Shape::Polyline { points, closed } => {
            let mut out = Vec::new();
            let n = points.len();
            let segs = if *closed { n } else { n.saturating_sub(1) };
            for i in 0..segs {
                out.push(Curve::Seg {
                    p0: points[i],
                    p1: points[(i + 1) % n],
                });
            }
            out
        }
        Shape::LwPolyline { vertices, closed } => {
            let mut out = Vec::new();
            let n = vertices.len();
            let segs = if *closed { n } else { n.saturating_sub(1) };
            for i in 0..segs {
                let p0 = vertices[i].point;
                let p1 = vertices[(i + 1) % n].point;
                let b = vertices[i].bulge;
                if b.abs() < 1e-10 {
                    out.push(Curve::Seg { p0, p1 });
                } else if let Some((c, r, s, e)) = bulge_arc(p0, p1, b) {
                    out.push(arc_curve(c, r, s, e));
                }
            }
            out
        }
        _ => Vec::new(),
    }
}

fn arc_curve(c: Point, r: f64, start: f64, end: f64) -> Curve {
    let mut sweep = end - start;
    if sweep <= 0.0 {
        sweep += TAU;
    }
    if sweep > TAU {
        sweep = TAU;
    }
    Curve::Arc {
        c,
        r,
        start: norm_angle(start),
        sweep,
    }
}

/// Center, radius and CCW (start, end) angles of a polyline bulge segment.
pub fn bulge_arc(p0: Point, p1: Point, bulge: f64) -> Option<(Point, f64, f64, f64)> {
    let chord = (p1 - p0).hypot();
    if chord < 1e-12 || bulge.abs() < 1e-12 {
        return None;
    }
    let included = 4.0 * bulge.atan(); // signed, CCW positive
    let r = chord / (2.0 * (included / 2.0).sin().abs());
    let mid = p0.midpoint(p1);
    let n = rot90(unit(p1 - p0));
    // Distance from chord midpoint to center (signed along left normal).
    let h = r * (included / 2.0).cos().abs();
    let sign = if bulge.abs() < 1.0 { 1.0 } else { -1.0 } * bulge.signum();
    let c = mid + n * (h * sign);
    let a0 = angle_of(p0, c);
    let a1 = angle_of(p1, c);
    if bulge > 0.0 {
        Some((c, r, a0, a1))
    } else {
        Some((c, r, a1, a0))
    }
}

// ── Pure geometry ──────────────────────────────────────────────────────

/// Intersection of lines (a0,a1) and (b0,b1). With `segments`, the point
/// must lie within both finite segments. Returns None for parallel lines.
pub fn line_line_intersection(
    a0: Point,
    a1: Point,
    b0: Point,
    b1: Point,
    segments: bool,
) -> Option<Point> {
    let d1 = a1 - a0;
    let d2 = b1 - b0;
    let denom = d1.cross(d2);
    let scale = d1.hypot() * d2.hypot();
    if scale < 1e-24 || denom.abs() < 1e-12 * scale {
        return None;
    }
    let w = b0 - a0;
    let t = w.cross(d2) / denom;
    let u = w.cross(d1) / denom;
    if segments {
        let tt = TOL / d1.hypot();
        let tu = TOL / d2.hypot();
        if t < -tt || t > 1.0 + tt || u < -tu || u > 1.0 + tu {
            return None;
        }
    }
    Some(a0 + d1 * t)
}

/// Lines tangent to two circles. `inner` = crossing (internal) tangents,
/// otherwise outer tangents. Returns up to two `(p_on_c1, p_on_c2)` pairs.
pub fn tangent_lines(c1: Point, r1: f64, c2: Point, r2: f64, inner: bool) -> Vec<(Point, Point)> {
    let d = (c2 - c1).hypot();
    if d < EPS || r1 < 0.0 || r2 < 0.0 {
        return Vec::new();
    }
    let v = (c2 - c1) / d;
    let (k, s2) = if inner {
        ((r1 + r2) / d, -1.0)
    } else {
        ((r1 - r2) / d, 1.0)
    };
    if k.abs() > 1.0 + 1e-12 {
        return Vec::new();
    }
    let k = k.clamp(-1.0, 1.0);
    let h = (1.0 - k * k).max(0.0).sqrt();
    let mut out = Vec::new();
    for sgn in [1.0, -1.0] {
        let n = v * k + rot90(v) * (h * sgn);
        out.push((c1 + n * r1, c2 + n * (r2 * s2)));
        if h < 1e-12 {
            break; // single tangent (touching / internally tangent circles)
        }
    }
    out
}

/// Tangent points on circle (c, r) for lines through external point `p`.
/// Returns 2 points, 1 if `p` lies on the circle, 0 if inside.
pub fn tangent_from_point(p: Point, c: Point, r: f64) -> Vec<Point> {
    let d = (p - c).hypot();
    if d < r - 1e-9 || r <= 0.0 {
        return Vec::new();
    }
    if (d - r).abs() <= 1e-9 {
        return vec![p];
    }
    let base = angle_of(p, c);
    let alpha = (r / d).acos();
    vec![polar(c, r, base + alpha), polar(c, r, base - alpha)]
}

// ── Trimming a curve at a point, keeping one direction ─────────────────

/// Result of shortening/extending a line or arc so it starts (or ends)
/// at `t`, keeping the part that leaves `t` in direction `k`.
struct TrimResult {
    shape: Shape,
    /// How far the entity had to be extended (0 if `t` lay on it).
    extension: f64,
}

/// `k` is the direction (tangent) in which the kept part leaves `t`.
/// `keep`: optional pick point that must lie on the kept part.
fn trim_from(shape: &Shape, t: Point, k: Vec2, keep: Option<Point>) -> Option<TrimResult> {
    match shape {
        Shape::Line(l) => {
            let u = l.p1 - l.p0;
            let len = u.hypot();
            if len < EPS {
                return None;
            }
            let u = u / len;
            if let Some(kp) = keep {
                if (kp - t).dot(k) <= TOL {
                    return None;
                }
            }
            let fwd = k.dot(u) > 0.0;
            let (far, new) = if fwd {
                (l.p1, Line::new(t, l.p1))
            } else {
                (l.p0, Line::new(l.p0, t))
            };
            // The kept far endpoint must lie ahead of t.
            if (far - t).dot(k) <= TOL {
                return None;
            }
            let tp = (t - l.p0).dot(u);
            let extension = if tp < 0.0 {
                -tp
            } else if tp > len {
                tp - len
            } else {
                0.0
            };
            Some(TrimResult {
                shape: Shape::Line(new),
                extension,
            })
        }
        Shape::Arc {
            center,
            radius,
            start_angle,
            end_angle,
        } => {
            let c = *center;
            let r = *radius;
            let Curve::Arc { start, sweep, .. } = arc_curve(c, r, *start_angle, *end_angle) else {
                return None;
            };
            let ccw = (t - c).cross(k) > 0.0;
            let th = angle_of(t, c);
            let rel = norm_angle(th - start);
            let gap = TAU - sweep;
            let inside = rel <= sweep + 1e-9 || rel >= TAU - 1e-9;
            // Extension (in radians) needed and whether t is on the correct side.
            let ext = if inside {
                0.0
            } else if ccw {
                // Keep the end: t must sit before the start, in the half of the gap next to it.
                let before = TAU - rel;
                if before > gap / 2.0 {
                    return None;
                }
                before
            } else {
                let past = rel - sweep;
                if past > gap / 2.0 {
                    return None;
                }
                past
            };
            let (ns, ne) = if ccw {
                (th, start + sweep)
            } else {
                (start, th)
            };
            let new_sweep = norm_angle(ne - ns);
            if new_sweep * r <= TOL || new_sweep >= TAU - 1e-9 {
                return None;
            }
            if let Some(kp) = keep {
                let rk = norm_angle(angle_of(kp, c) - ns);
                if rk > new_sweep + 1e-9 {
                    return None;
                }
            }
            Some(TrimResult {
                shape: Shape::Arc {
                    center: c,
                    radius: r,
                    start_angle: ns,
                    end_angle: ns + new_sweep,
                },
                extension: ext * r,
            })
        }
        Shape::Circle(_) => Some(TrimResult {
            shape: shape.clone(),
            extension: 0.0,
        }),
        _ => None,
    }
}

/// Default "keep" point when the caller gives no pick: the endpoint of the
/// entity farthest from the other entity (corners keep their long legs).
fn default_keep(shape: &Shape, other: &[Curve]) -> Option<Point> {
    let c = shape_curves(shape);
    let (e0, e1) = c.first()?.endpoints()?;
    let dist = |p: Point| {
        other
            .iter()
            .map(|o| o.distance_to(p))
            .fold(f64::INFINITY, f64::min)
    };
    Some(if dist(e0) >= dist(e1) { e0 } else { e1 })
}

/// Offset carriers of a curve by distance r (both sides).
fn offset_carriers(c: &Curve, r: f64) -> Vec<Curve> {
    match *c {
        Curve::Seg { p0, p1 } => {
            let n = rot90(unit(p1 - p0));
            vec![
                Curve::Seg {
                    p0: p0 + n * r,
                    p1: p1 + n * r,
                },
                Curve::Seg {
                    p0: p0 - n * r,
                    p1: p1 - n * r,
                },
            ]
        }
        Curve::Arc { c, r: rc, .. } => {
            let mut v = vec![Curve::Arc {
                c,
                r: rc + r,
                start: 0.0,
                sweep: TAU,
            }];
            if (rc - r).abs() > EPS {
                v.push(Curve::Arc {
                    c,
                    r: (rc - r).abs(),
                    start: 0.0,
                    sweep: TAU,
                });
            }
            v
        }
    }
}

/// Point on curve `c`'s carrier where a circle (center, r) touches it.
fn touch_point(c: &Curve, center: Point, r: f64) -> Option<Point> {
    match *c {
        Curve::Seg { p0, p1 } => Some(crate::geo::project_onto(center, p0, p1)),
        Curve::Arc { c: cc, r: rc, .. } => {
            let d = (center - cc).hypot();
            if d < EPS {
                return None;
            }
            let u = (center - cc) / d;
            let cands = [cc + u * rc, cc - u * rc];
            cands
                .into_iter()
                .min_by(|a, b| {
                    (((*a - center).hypot() - r).abs())
                        .partial_cmp(&(((*b - center).hypot() - r).abs()))
                        .unwrap_or(std::cmp::Ordering::Equal)
                })
                .filter(|t| ((*t - center).hypot() - r).abs() < 1e-6 * (1.0 + r))
        }
    }
}

/// Direction in which a curve continues away from point `t`, toward `keep`
/// (used by zero-radius fillets and chamfers).
fn keep_direction(shape: &Shape, t: Point, keep: Point) -> Vec2 {
    match shape {
        Shape::Line(l) => {
            let u = unit(l.p1 - l.p0);
            if (keep - t).dot(u) >= 0.0 {
                u
            } else {
                -u
            }
        }
        Shape::Arc {
            center,
            radius,
            start_angle,
            end_angle,
        } => {
            let tan = rot90(unit(t - *center));
            // Does keep lie CCW-ahead of t within the arc's span?
            let Curve::Arc { start, sweep, .. } =
                arc_curve(*center, *radius, *start_angle, *end_angle)
            else {
                return tan;
            };
            let rt = norm_angle(angle_of(t, *center) - start);
            let rk = norm_angle(angle_of(keep, *center) - start);
            let rt = if rt > sweep + (TAU - sweep) / 2.0 {
                rt - TAU
            } else {
                rt
            };
            if rk > rt {
                tan
            } else {
                -tan
            }
        }
        _ => Vec2::ZERO,
    }
}

/// Result of a fillet computation (pure: does not touch the document).
#[derive(Debug, Clone)]
pub struct FilletResult {
    /// The fillet arc (None for zero radius).
    pub arc: Option<Shape>,
    pub a: Shape,
    pub b: Shape,
}

/// Fillet two lines / arcs / circles with radius `r`.
///
/// `pick_a` / `pick_b` select which side of each entity is kept (a point on
/// the part that should survive). Without picks, each entity keeps the end
/// farthest from the other one. Circles are never trimmed.
pub fn fillet_shapes(
    a: &Shape,
    b: &Shape,
    r: f64,
    pick_a: Option<Point>,
    pick_b: Option<Point>,
) -> Result<FilletResult, String> {
    if r.is_nan() || r < 0.0 || !r.is_finite() {
        return Err("fillet radius must be >= 0".into());
    }
    let ca = shape_curves(a);
    let cb = shape_curves(b);
    let (Some(ca0), Some(cb0)) = (ca.first().copied(), cb.first().copied()) else {
        return Err("fillet supports lines, arcs and circles".into());
    };
    if ca.len() != 1
        || cb.len() != 1
        || !matches!(a, Shape::Line(_) | Shape::Arc { .. } | Shape::Circle(_))
        || !matches!(b, Shape::Line(_) | Shape::Arc { .. } | Shape::Circle(_))
    {
        return Err("fillet supports lines, arcs and circles".into());
    }
    let keep_a = pick_a.or_else(|| default_keep(a, &cb));
    let keep_b = pick_b.or_else(|| default_keep(b, &ca));

    // Zero radius: trim/extend both to their intersection.
    if r < EPS {
        let xs = carrier_intersections(&ca0, &cb0);
        if xs.is_empty() {
            return Err("entities do not intersect (parallel?)".into());
        }
        let mut best: Option<(f64, FilletResult)> = None;
        for x in xs {
            let ka = keep_a.map_or(Vec2::ZERO, |k| keep_direction(a, x, k));
            let kb = keep_b.map_or(Vec2::ZERO, |k| keep_direction(b, x, k));
            let (Some(ta), Some(tb)) = (trim_from(a, x, ka, keep_a), trim_from(b, x, kb, keep_b))
            else {
                continue;
            };
            let score = ta.extension + tb.extension;
            if best.as_ref().is_none_or(|(s, _)| score < *s) {
                best = Some((
                    score,
                    FilletResult {
                        arc: None,
                        a: ta.shape,
                        b: tb.shape,
                    },
                ));
            }
        }
        return best
            .map(|(_, f)| f)
            .ok_or_else(|| "no valid corner for zero-radius fillet".into());
    }

    let mut centers = Vec::new();
    for oa in offset_carriers(&ca0, r) {
        for ob in offset_carriers(&cb0, r) {
            centers.extend(carrier_intersections(&oa, &ob));
        }
    }
    if centers.is_empty() {
        if let (Curve::Seg { p0, p1 }, Curve::Seg { p0: q0, p1: q1 }) = (ca0, cb0) {
            if line_line_intersection(p0, p1, q0, q1, false).is_none() {
                return Err("cannot fillet parallel lines".into());
            }
        }
        return Err("no fillet solution: radius too large for these entities".into());
    }

    let mut best: Option<(f64, FilletResult)> = None;
    for c in centers {
        let (Some(ta), Some(tb)) = (touch_point(&ca0, c, r), touch_point(&cb0, c, r)) else {
            continue;
        };
        if (ta - tb).hypot() < TOL {
            continue; // the entities are already tangent here
        }
        for sigma in [1.0, -1.0] {
            let tan = |t: Point| rot90((t - c) / r) * sigma;
            let ka = -tan(ta);
            let kb = tan(tb);
            let (Some(ra), Some(rb)) = (trim_from(a, ta, ka, keep_a), trim_from(b, tb, kb, keep_b))
            else {
                continue;
            };
            let a0 = angle_of(ta, c);
            let a1 = angle_of(tb, c);
            let (s, e) = if sigma > 0.0 { (a0, a1) } else { (a1, a0) };
            let sweep = norm_angle(e - s);
            // A fillet never wraps more than half way round: larger sweeps
            // are "outside" solutions that loop around the corner.
            if sweep > PI + 1e-9 {
                continue;
            }
            let mut score = ra.extension + rb.extension + sweep * r * 1e-3;
            // For untrimmed circles, prefer the solution near the pick.
            if matches!(a, Shape::Circle(_)) {
                if let Some(p) = pick_a {
                    score += (ta - p).hypot();
                }
            }
            if matches!(b, Shape::Circle(_)) {
                if let Some(p) = pick_b {
                    score += (tb - p).hypot();
                }
            }
            if best.as_ref().is_none_or(|(bs, _)| score < *bs) {
                best = Some((
                    score,
                    FilletResult {
                        arc: Some(Shape::Arc {
                            center: c,
                            radius: r,
                            start_angle: s,
                            end_angle: s + sweep,
                        }),
                        a: ra.shape,
                        b: rb.shape,
                    },
                ));
            }
        }
    }
    best.map(|(_, f)| f)
        .ok_or_else(|| "no fillet solution: radius too large for these entities".into())
}

/// Chamfer two lines: cut `d1` back from the corner along `a`, `d2` along `b`.
/// Returns (chamfer line or None when both distances are 0, new a, new b).
pub fn chamfer_shapes(
    a: &Shape,
    b: &Shape,
    d1: f64,
    d2: f64,
    pick_a: Option<Point>,
    pick_b: Option<Point>,
) -> Result<(Option<Shape>, Shape, Shape), String> {
    let (Shape::Line(la), Shape::Line(lb)) = (a, b) else {
        return Err("chamfer supports two lines".into());
    };
    if d1 < 0.0 || d2 < 0.0 || !d1.is_finite() || !d2.is_finite() {
        return Err("chamfer distances must be >= 0".into());
    }
    let x = line_line_intersection(la.p0, la.p1, lb.p0, lb.p1, false)
        .ok_or("cannot chamfer parallel lines")?;
    let ca = shape_curves(a);
    let cb = shape_curves(b);
    let keep_a = pick_a.or_else(|| default_keep(a, &cb)).ok_or("bad line")?;
    let keep_b = pick_b.or_else(|| default_keep(b, &ca)).ok_or("bad line")?;
    let ka = keep_direction(a, x, keep_a);
    let kb = keep_direction(b, x, keep_b);
    let pa = x + ka * d1;
    let pb = x + kb * d2;
    let ra =
        trim_from(a, pa, ka, Some(keep_a)).ok_or("chamfer distance too large for first line")?;
    let rb =
        trim_from(b, pb, kb, Some(keep_b)).ok_or("chamfer distance too large for second line")?;
    let line = if (pa - pb).hypot() > EPS {
        Some(Shape::Line(Line::new(pa, pb)))
    } else {
        None
    };
    Ok((line, ra.shape, rb.shape))
}

// ── Trim / extend to boundaries ────────────────────────────────────────

/// Parameter of a point along a target curve: lines use t in [0,1] scaled
/// to length, arcs use angle offset from start (radians).
fn param_on(c: &Curve, p: Point) -> f64 {
    match *c {
        Curve::Seg { p0, p1 } => {
            let d = p1 - p0;
            (p - p0).dot(d) / d.hypot2().max(1e-300)
        }
        Curve::Arc { c, start, .. } => norm_angle(angle_of(p, c) - start),
    }
}

/// Trim `shape` at its intersections with `boundary`, keeping the piece
/// that contains (the projection of) `keep`.
pub fn trim_to_shape(shape: &Shape, boundary: &[Curve], keep: Point) -> Result<Shape, String> {
    let curves = shape_curves(shape);
    let tc = match (shape, curves.first()) {
        (Shape::Line(_) | Shape::Arc { .. } | Shape::Circle(_), Some(c)) => *c,
        _ => return Err("trimTo supports lines, arcs and circles".into()),
    };
    let mut cuts: Vec<f64> = Vec::new();
    for b in boundary {
        for p in carrier_intersections(&tc, b) {
            if b.contains(p, TOL) && tc.contains(p, TOL) {
                cuts.push(param_on(&tc, p));
            }
        }
    }
    match tc {
        Curve::Seg { p0, p1 } => {
            let len = (p1 - p0).hypot();
            let e = TOL / len;
            cuts.retain(|t| *t > e && *t < 1.0 - e);
            if cuts.is_empty() {
                return Err("no intersection with the boundary".into());
            }
            let tk = param_on(&tc, keep).clamp(0.0, 1.0);
            let lo = cuts.iter().copied().filter(|t| *t < tk).fold(0.0, f64::max);
            let hi = cuts.iter().copied().filter(|t| *t > tk).fold(1.0, f64::min);
            let d = p1 - p0;
            Ok(Shape::Line(Line::new(p0 + d * lo, p0 + d * hi)))
        }
        Curve::Arc { c, r, start, sweep } => {
            let full = tc.is_full_circle();
            let e = TOL / r.max(EPS);
            if full {
                cuts.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
                cuts.dedup_by(|a, b| (*a - *b).abs() < e);
                if cuts.len() < 2 {
                    return Err("a circle needs at least two intersections to trim".into());
                }
                let ak = param_on(&tc, keep);
                // Cut just before keep (CCW) and just after.
                let before = cuts
                    .iter()
                    .copied()
                    .filter(|a| *a <= ak)
                    .fold(f64::NAN, f64::max);
                let before = if before.is_nan() {
                    *cuts.last().expect("non-empty")
                } else {
                    before
                };
                let after = cuts
                    .iter()
                    .copied()
                    .filter(|a| *a > ak)
                    .fold(f64::NAN, f64::min);
                let after = if after.is_nan() { cuts[0] } else { after };
                let s = start + before;
                let sw = norm_angle(after - before);
                return Ok(Shape::Arc {
                    center: c,
                    radius: r,
                    start_angle: s,
                    end_angle: s + sw,
                });
            }
            cuts.retain(|a| *a > e && *a < sweep - e);
            if cuts.is_empty() {
                return Err("no intersection with the boundary".into());
            }
            let mut ak = param_on(&tc, keep);
            if ak > sweep {
                // Outside the arc: snap to the nearest end.
                ak = if ak - sweep < TAU - ak { sweep } else { 0.0 };
            }
            let lo = cuts.iter().copied().filter(|t| *t < ak).fold(0.0, f64::max);
            let hi = cuts
                .iter()
                .copied()
                .filter(|t| *t > ak)
                .fold(sweep, f64::min);
            Ok(Shape::Arc {
                center: c,
                radius: r,
                start_angle: start + lo,
                end_angle: start + hi,
            })
        }
    }
}

/// Which end of a line/arc to extend.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ExtendEnd {
    Start,
    End,
    Nearest,
}

/// Extend a line or arc until it meets `boundary`.
pub fn extend_to_shape(
    shape: &Shape,
    boundary: &[Curve],
    which: ExtendEnd,
) -> Result<Shape, String> {
    let curves = shape_curves(shape);
    let tc = match (shape, curves.first()) {
        (Shape::Line(_) | Shape::Arc { .. }, Some(c)) => *c,
        _ => return Err("extendTo supports lines and arcs".into()),
    };
    // Carrier hits that lie on the (finite) boundary.
    let mut hits: Vec<Point> = Vec::new();
    for b in boundary {
        let full = match tc {
            Curve::Seg { p0, p1 } => Curve::Seg { p0, p1 },
            Curve::Arc { c, r, .. } => Curve::Arc {
                c,
                r,
                start: 0.0,
                sweep: TAU,
            },
        };
        for p in carrier_intersections(&full, b) {
            if b.contains(p, TOL) {
                hits.push(p);
            }
        }
    }
    // Distance beyond the start (s) or end (e) for each hit.
    let mut best_start: Option<(f64, Point)> = None;
    let mut best_end: Option<(f64, Point)> = None;
    match tc {
        Curve::Seg { p0, p1 } => {
            let len = (p1 - p0).hypot();
            for p in hits {
                let t = param_on(&tc, p) * len;
                if t > len + TOL {
                    let d = t - len;
                    if best_end.is_none_or(|(bd, _)| d < bd) {
                        best_end = Some((d, p));
                    }
                } else if t < -TOL {
                    let d = -t;
                    if best_start.is_none_or(|(bd, _)| d < bd) {
                        best_start = Some((d, p));
                    }
                }
            }
        }
        Curve::Arc { r, sweep, .. } => {
            for p in hits {
                let a = param_on(&tc, p);
                if a > sweep + TOL / r {
                    let past = (a - sweep) * r;
                    let before = (TAU - a) * r;
                    if best_end.is_none_or(|(bd, _)| past < bd) {
                        best_end = Some((past, p));
                    }
                    if best_start.is_none_or(|(bd, _)| before < bd) {
                        best_start = Some((before, p));
                    }
                }
            }
        }
    }
    let pick_end = match which {
        ExtendEnd::Start => false,
        ExtendEnd::End => true,
        ExtendEnd::Nearest => match (best_start, best_end) {
            (Some((ds, _)), Some((de, _))) => de <= ds,
            (None, Some(_)) => true,
            _ => false,
        },
    };
    let target = if pick_end { best_end } else { best_start };
    let (_, p) = target.ok_or("boundary not reachable by extending this entity")?;
    Ok(match (shape, tc) {
        (Shape::Line(l), _) => {
            if pick_end {
                Shape::Line(Line::new(l.p0, p))
            } else {
                Shape::Line(Line::new(p, l.p1))
            }
        }
        (_, Curve::Arc { c, r, start, sweep }) => {
            let a = angle_of(p, c);
            if pick_end {
                Shape::Arc {
                    center: c,
                    radius: r,
                    start_angle: start,
                    end_angle: start + norm_angle(a - start),
                }
            } else {
                let end = start + sweep;
                Shape::Arc {
                    center: c,
                    radius: r,
                    start_angle: end - norm_angle(end - a),
                    end_angle: end,
                }
            }
        }
        _ => unreachable!("target is a line or arc"),
    })
}

// ── Arrays ─────────────────────────────────────────────────────────────

/// Transforms for the copies of a polar array (excluding the original).
pub fn polar_array_transforms(
    center: Point,
    count: usize,
    angle_deg: f64,
    rotate: bool,
    ref_pt: Point,
) -> Vec<Affine> {
    if count < 2 {
        return Vec::new();
    }
    let full = (angle_deg.abs() - 360.0).abs() < 1e-9 || angle_deg.abs() > 360.0;
    let step = if full {
        angle_deg / count as f64
    } else {
        angle_deg / (count - 1) as f64
    };
    (1..count)
        .map(|i| {
            let a = (step * i as f64).to_radians();
            let rot = Affine::translate(center.to_vec2())
                * Affine::rotate(a)
                * Affine::translate(-center.to_vec2());
            if rotate {
                rot
            } else {
                let moved = rot * ref_pt;
                Affine::translate(moved - ref_pt)
            }
        })
        .collect()
}

/// Transforms for the copies of a rectangular array (excluding the original).
pub fn rect_array_transforms(rows: usize, cols: usize, dx: f64, dy: f64) -> Vec<Affine> {
    let mut out = Vec::new();
    for r in 0..rows.max(1) {
        for c in 0..cols.max(1) {
            if r == 0 && c == 0 {
                continue;
            }
            out.push(Affine::translate((c as f64 * dx, r as f64 * dy)));
        }
    }
    out
}

// ── Document-level helpers ─────────────────────────────────────────────

/// Replace an entity's shape in place (keeps id, layer and style).
pub fn set_shape(doc: &mut Document, id: EntityId, shape: Shape) -> bool {
    match doc.entity_mut(id) {
        Some(e) => {
            e.shape = shape;
            true
        }
        None => false,
    }
}

/// Add a shape as a new entity copying layer/style from `style`.
pub fn add_like(
    doc: &mut Document,
    style: &DrawEntity,
    shape: Shape,
    layer: Option<&str>,
) -> EntityId {
    let id = doc.alloc_id();
    let layer = layer.unwrap_or(&style.layer).to_string();
    doc.ensure_layer(&layer);
    doc.entities.push(DrawEntity {
        attrs: Default::default(),
        id,
        layer,
        color: style.color,
        linetype: style.linetype.clone(),
        lineweight: style.lineweight,
        transparency: style.transparency,
        shape,
    });
    id
}

/// Curves of a set of boundary entities (block inserts are expanded).
pub fn boundary_curves(doc: &Document, ids: &[EntityId]) -> Result<Vec<Curve>, String> {
    let mut out = Vec::new();
    for id in ids {
        let e = doc
            .entity(*id)
            .ok_or_else(|| format!("entity e_{} not found", id.0))?;
        out.extend(shape_curves(&e.shape));
    }
    if out.is_empty() {
        return Err("boundary has no line/arc/circle/polyline geometry".into());
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(x: f64, y: f64) -> Point {
        Point::new(x, y)
    }
    fn close(a: Point, b: Point) -> bool {
        (a - b).hypot() < 1e-6
    }
    fn line(a: Point, b: Point) -> Shape {
        Shape::Line(Line::new(a, b))
    }
    fn arc_ends(s: &Shape) -> (Point, Point) {
        match s {
            Shape::Arc {
                center,
                radius,
                start_angle,
                end_angle,
            } => (
                polar(*center, *radius, *start_angle),
                polar(*center, *radius, *end_angle),
            ),
            _ => panic!("not an arc: {s:?}"),
        }
    }

    #[test]
    fn line_line_basic_and_parallel() {
        let x = line_line_intersection(p(0., 0.), p(10., 0.), p(5., -5.), p(5., 5.), true).unwrap();
        assert!(close(x, p(5., 0.)));
        assert!(
            line_line_intersection(p(0., 0.), p(10., 0.), p(0., 1.), p(10., 1.), false).is_none()
        );
        // Segments that don't reach each other
        assert!(
            line_line_intersection(p(0., 0.), p(1., 0.), p(5., -5.), p(5., 5.), true).is_none()
        );
        // ...but their infinite lines do
        let x = line_line_intersection(p(0., 0.), p(1., 0.), p(5., -5.), p(5., 5.), false).unwrap();
        assert!(close(x, p(5., 0.)));
        // Endpoint touching counts as a segment hit
        assert!(line_line_intersection(p(0., 0.), p(5., 0.), p(5., 0.), p(5., 5.), true).is_some());
    }

    #[test]
    fn fillet_right_angle_corner() {
        // L-corner at origin: horizontal leg to the right, vertical leg up.
        let a = line(p(0., 0.), p(100., 0.));
        let b = line(p(0., 0.), p(0., 50.));
        let f = fillet_shapes(&a, &b, 10.0, None, None).unwrap();
        let arc = f.arc.unwrap();
        match &arc {
            Shape::Arc { center, radius, .. } => {
                assert!(close(*center, p(10., 10.)));
                assert!((radius - 10.0).abs() < 1e-9);
            }
            _ => panic!(),
        }
        let (s, e) = arc_ends(&arc);
        let ends = [s, e];
        assert!(ends.iter().any(|q| close(*q, p(10., 0.))));
        assert!(ends.iter().any(|q| close(*q, p(0., 10.))));
        match (&f.a, &f.b) {
            (Shape::Line(la), Shape::Line(lb)) => {
                assert!(close(la.p0, p(10., 0.)) && close(la.p1, p(100., 0.)));
                assert!(close(lb.p0, p(0., 10.)) && close(lb.p1, p(0., 50.)));
            }
            _ => panic!(),
        }
    }

    #[test]
    fn fillet_lines_that_do_not_meet_extends_them() {
        let a = line(p(20., 0.), p(100., 0.));
        let b = line(p(0., 20.), p(0., 80.));
        let f = fillet_shapes(&a, &b, 5.0, None, None).unwrap();
        match (&f.a, &f.b) {
            (Shape::Line(la), Shape::Line(lb)) => {
                assert!(close(la.p0, p(5., 0.)));
                assert!(close(lb.p0, p(0., 5.)));
            }
            _ => panic!(),
        }
    }

    #[test]
    fn fillet_crossing_lines_uses_picks() {
        // X crossing at origin; keep the +x and +y halves.
        let a = line(p(-50., 0.), p(50., 0.));
        let b = line(p(0., -50.), p(0., 50.));
        let f = fillet_shapes(&a, &b, 5.0, Some(p(40., 0.)), Some(p(0., 40.))).unwrap();
        match f.arc.unwrap() {
            Shape::Arc { center, .. } => assert!(close(center, p(5., 5.))),
            _ => panic!(),
        }
        // Other quadrant
        let f = fillet_shapes(&a, &b, 5.0, Some(p(-40., 0.)), Some(p(0., -40.))).unwrap();
        match f.arc.unwrap() {
            Shape::Arc { center, .. } => assert!(close(center, p(-5., -5.))),
            _ => panic!(),
        }
    }

    #[test]
    fn fillet_parallel_lines_errors() {
        let a = line(p(0., 0.), p(10., 0.));
        let b = line(p(0., 5.), p(10., 5.));
        let e = fillet_shapes(&a, &b, 1.0, None, None).unwrap_err();
        assert!(e.contains("parallel"), "{e}");
    }

    #[test]
    fn fillet_radius_too_large_errors() {
        let a = line(p(0., 0.), p(10., 0.));
        let b = line(p(0., 0.), p(0., 10.));
        let e = fillet_shapes(&a, &b, 50.0, None, None).unwrap_err();
        assert!(e.contains("too large"), "{e}");
    }

    #[test]
    fn fillet_zero_radius_makes_sharp_corner() {
        let a = line(p(5., 0.), p(100., 0.));
        let b = line(p(0., 5.), p(0., 80.));
        let f = fillet_shapes(&a, &b, 0.0, None, None).unwrap();
        assert!(f.arc.is_none());
        match (&f.a, &f.b) {
            (Shape::Line(la), Shape::Line(lb)) => {
                assert!(close(la.p0, p(0., 0.)) && close(la.p1, p(100., 0.)));
                assert!(close(lb.p0, p(0., 0.)) && close(lb.p1, p(0., 80.)));
            }
            _ => panic!(),
        }
        assert!(fillet_shapes(&a, &b, -1.0, None, None).is_err());
    }

    #[test]
    fn fillet_line_and_arc_is_tangent_to_both() {
        // Arc (circle r=20 at origin, upper half) and a horizontal line y=22.
        let arc = Shape::Arc {
            center: p(0., 0.),
            radius: 20.0,
            start_angle: 0.0,
            end_angle: PI,
        };
        let l = line(p(-60., 22.), p(60., 22.));
        let r = 5.0;
        let f = fillet_shapes(&l, &arc, r, Some(p(50., 22.)), Some(p(19., 6.))).unwrap();
        let Shape::Arc { center, radius, .. } = f.arc.clone().unwrap() else {
            panic!()
        };
        // Tangent to the line: distance from center to y=22 is r
        assert!(((22.0 - center.y).abs() - r).abs() < 1e-6);
        // Tangent to the circle externally: |center| = 20 + r
        assert!(((center.to_vec2().hypot()) - (20.0 + radius)).abs() < 1e-6);
        assert!(center.x > 0.0);
        // The line keeps its right part and now starts at the tangent point
        let Shape::Line(nl) = f.a else { panic!() };
        assert!(close(nl.p1, p(60., 22.)));
        assert!((nl.p0.x - center.x).abs() < 1e-6);
        // The arc was trimmed and ends at the fillet tangent point
        let (s, e) = arc_ends(&f.b);
        let (fs, fe) = arc_ends(&f.arc.unwrap());
        assert!([s, e].iter().any(|q| close(*q, fs) || close(*q, fe)));
    }

    #[test]
    fn fillet_line_and_circle_keeps_circle() {
        let circ = Shape::Circle(kurbo::Circle::new(p(0., 0.), 10.0));
        let l = line(p(-50., 15.), p(50., 15.));
        let f = fillet_shapes(&l, &circ, 3.0, Some(p(40., 15.)), Some(p(10., 0.))).unwrap();
        assert!(matches!(f.b, Shape::Circle(_)));
        let Shape::Arc { center, .. } = f.arc.unwrap() else {
            panic!()
        };
        assert!(center.x > 0.0);
    }

    #[test]
    fn chamfer_corner() {
        let a = line(p(0., 0.), p(100., 0.));
        let b = line(p(0., 0.), p(0., 50.));
        let (c, na, nb) = chamfer_shapes(&a, &b, 5.0, 10.0, None, None).unwrap();
        let Some(Shape::Line(cl)) = c else { panic!() };
        assert!(close(cl.p0, p(5., 0.)) && close(cl.p1, p(0., 10.)));
        let (Shape::Line(la), Shape::Line(lb)) = (na, nb) else {
            panic!()
        };
        assert!(close(la.p0, p(5., 0.)));
        assert!(close(lb.p0, p(0., 10.)));
        assert!(chamfer_shapes(&a, &b, 500.0, 5.0, None, None).is_err());
        let par = line(p(0., 5.), p(100., 5.));
        assert!(chamfer_shapes(&a, &par, 1.0, 1.0, None, None).is_err());
    }

    #[test]
    fn trim_line_between_two_boundaries() {
        let target = line(p(0., 0.), p(100., 0.));
        let b = vec![
            Curve::Seg {
                p0: p(30., -10.),
                p1: p(30., 10.),
            },
            Curve::Seg {
                p0: p(60., -10.),
                p1: p(60., 10.),
            },
        ];
        let Shape::Line(l) = trim_to_shape(&target, &b, p(80., 0.)).unwrap() else {
            panic!()
        };
        assert!(close(l.p0, p(60., 0.)) && close(l.p1, p(100., 0.)));
        let Shape::Line(l) = trim_to_shape(&target, &b, p(45., 1.)).unwrap() else {
            panic!()
        };
        assert!(close(l.p0, p(30., 0.)) && close(l.p1, p(60., 0.)));
        // Boundary that doesn't reach the target
        let far = vec![Curve::Seg {
            p0: p(30., 5.),
            p1: p(30., 10.),
        }];
        assert!(trim_to_shape(&target, &far, p(80., 0.)).is_err());
    }

    #[test]
    fn trim_line_against_circle_and_circle_against_line() {
        let target = line(p(-50., 0.), p(50., 0.));
        let circ = shape_curves(&Shape::Circle(kurbo::Circle::new(p(0., 0.), 10.0)));
        let Shape::Line(l) = trim_to_shape(&target, &circ, p(40., 0.)).unwrap() else {
            panic!()
        };
        assert!(close(l.p0, p(10., 0.)) && close(l.p1, p(50., 0.)));

        // Circle cut by a line through it -> upper half arc kept
        let c = Shape::Circle(kurbo::Circle::new(p(0., 0.), 10.0));
        let bl = shape_curves(&target);
        let res = trim_to_shape(&c, &bl, p(0., 10.)).unwrap();
        let (s, e) = arc_ends(&res);
        assert!(close(s, p(10., 0.)) && close(e, p(-10., 0.)));
        // Tangent line touches once: can't trim a circle with one point
        let tangent = shape_curves(&line(p(-50., 10.), p(50., 10.)));
        assert!(trim_to_shape(&c, &tangent, p(0., -10.)).is_err());
    }

    #[test]
    fn trim_arc_against_line() {
        let a = Shape::Arc {
            center: p(0., 0.),
            radius: 10.0,
            start_angle: 0.0,
            end_angle: PI,
        };
        let b = shape_curves(&line(p(0., -20.), p(0., 20.)));
        let res = trim_to_shape(&a, &b, p(-8., 5.)).unwrap();
        let (s, e) = arc_ends(&res);
        assert!(close(s, p(0., 10.)) && close(e, p(-10., 0.)));
    }

    #[test]
    fn extend_line_to_line_and_circle() {
        let l = line(p(0., 0.), p(10., 0.));
        let wall = shape_curves(&line(p(50., -10.), p(50., 10.)));
        let Shape::Line(r) = extend_to_shape(&l, &wall, ExtendEnd::Nearest).unwrap() else {
            panic!()
        };
        assert!(close(r.p0, p(0., 0.)) && close(r.p1, p(50., 0.)));
        // Start side
        let back = shape_curves(&line(p(-20., -10.), p(-20., 10.)));
        let Shape::Line(r) = extend_to_shape(&l, &back, ExtendEnd::Nearest).unwrap() else {
            panic!()
        };
        assert!(close(r.p0, p(-20., 0.)));
        // Circle ahead: nearest hit
        let circ = shape_curves(&Shape::Circle(kurbo::Circle::new(p(40., 0.), 5.0)));
        let Shape::Line(r) = extend_to_shape(&l, &circ, ExtendEnd::End).unwrap() else {
            panic!()
        };
        assert!(close(r.p1, p(35., 0.)));
        // Parallel boundary: nothing reachable
        let par = shape_curves(&line(p(0., 5.), p(10., 5.)));
        assert!(extend_to_shape(&l, &par, ExtendEnd::Nearest).is_err());
    }

    #[test]
    fn extend_arc_to_line() {
        let a = Shape::Arc {
            center: p(0., 0.),
            radius: 10.0,
            start_angle: 0.0,
            end_angle: PI / 4.0,
        };
        let wall2 = shape_curves(&line(p(-20., 5.), p(0., 5.)));
        let r = extend_to_shape(&a, &wall2, ExtendEnd::End).unwrap();
        let (_, e) = arc_ends(&r);
        assert!((e.y - 5.0).abs() < 1e-6 && e.x < 0.0);
    }

    #[test]
    fn tangent_lines_outer_inner() {
        let outer = tangent_lines(p(0., 0.), 10.0, p(100., 0.), 10.0, false);
        assert_eq!(outer.len(), 2);
        for (a, b) in &outer {
            assert!((a.y.abs() - 10.0).abs() < 1e-9 && (b.y - a.y).abs() < 1e-9);
        }
        let inner = tangent_lines(p(0., 0.), 10.0, p(100., 0.), 10.0, true);
        assert_eq!(inner.len(), 2);
        for (a, b) in &inner {
            // crossing tangents pass through the midpoint (50, 0)
            let x = line_line_intersection(*a, *b, p(0., 0.), p(1., 0.), false).unwrap();
            assert!(close(x, p(50., 0.)));
            assert!(((a.to_vec2().hypot()) - 10.0).abs() < 1e-9);
        }
        // Overlapping circles: no inner tangents
        assert!(tangent_lines(p(0., 0.), 10.0, p(15., 0.), 10.0, true).is_empty());
        // One inside the other: no tangents at all
        assert!(tangent_lines(p(0., 0.), 10.0, p(1., 0.), 2.0, false).is_empty());
        // Concentric
        assert!(tangent_lines(p(0., 0.), 10.0, p(0., 0.), 2.0, false).is_empty());
    }

    #[test]
    fn tangent_from_point_cases() {
        let t = tangent_from_point(p(20., 0.), p(0., 0.), 10.0);
        assert_eq!(t.len(), 2);
        for q in &t {
            // radius ⟂ tangent
            let r = q.to_vec2();
            let tv = p(20., 0.) - *q;
            assert!(r.dot(tv).abs() < 1e-9);
        }
        assert_eq!(tangent_from_point(p(10., 0.), p(0., 0.), 10.0).len(), 1);
        assert!(tangent_from_point(p(1., 0.), p(0., 0.), 10.0).is_empty());
    }

    #[test]
    fn polar_and_rect_array_transforms() {
        let xs = polar_array_transforms(p(0., 0.), 4, 360.0, true, p(10., 0.));
        assert_eq!(xs.len(), 3);
        assert!(close(xs[0] * p(10., 0.), p(0., 10.)));
        let xs = polar_array_transforms(p(0., 0.), 3, 90.0, true, p(10., 0.));
        assert!(close(xs[1] * p(10., 0.), p(0., 10.)));
        let xs = rect_array_transforms(2, 3, 10.0, 5.0);
        assert_eq!(xs.len(), 5);
        assert!(close(*xs.last().unwrap() * p(0., 0.), p(20., 5.)));
    }

    #[test]
    fn bulge_arc_semicircle() {
        let (c, r, s, e) = bulge_arc(p(0., 0.), p(10., 0.), 1.0).unwrap();
        assert!(close(c, p(5., 0.)) && (r - 5.0).abs() < 1e-9);
        // CCW from (0,0)→(10,0) with positive bulge goes below the chord
        let mid = polar(c, r, s + norm_angle(e - s) / 2.0);
        assert!(mid.y < 0.0, "{mid:?}");
    }
}
