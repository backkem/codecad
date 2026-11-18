//! The reference meaning of a 2D profile: a region of the plane, with a
//! certified signed distance to its boundary.
//!
//! # Definition
//!
//! A profile is an outer loop and hole loops (`Profile2d`). Each loop is
//! the closed curve made of its segments in order, where the end of each
//! segment is joined to the start of the next by a straight line (the
//! joins are at most [`JOIN_TOL`] long, the tolerance `types.rs` states; a
//! kernel that closes them another way differs from this by at most that
//! much). The **region** is the set of points inside the outer loop and
//! inside no hole loop, where "inside a loop" means a non-zero winding
//! number. The region is defined only when every loop is simple, the
//! loops do not meet, every hole lies inside the outer loop and no hole
//! lies inside another hole; otherwise [`Region::new`] says "not defined".
//! Under those conditions non-zero winding and even-odd agree, so the
//! choice does not matter.
//!
//! Segments: a line from `p0` to `p1`; an arc of a circle or ellipse
//! `c + R(rot) (rx cos t, ry sin t)` for `t` from `t0` over the signed
//! sweep (a sweep of a full turn is a closed loop); a circle is the full
//! counter-clockwise arc. Splines have no reference meaning yet.
//!
//! # Certified evaluation
//!
//! [`Region::eval`] returns an interval enclosing `f(q)` for every `q` in
//! an interval box, where `f` is the exact signed Euclidean distance to the
//! region's boundary, negative inside. `f` is 1-Lipschitz (a distance
//! function is) and its sign is exact. The magnitude is the minimum over
//! segments of the distance to each segment, each enclosed with interval
//! arithmetic:
//!
//! - line: the perpendicular distance when the foot of the perpendicular
//!   lies on the segment, else the distance to the nearer end;
//! - circular arc: `| |q - c| - r |` when `q`'s direction from the centre
//!   lies within the arc's angular span, else the distance to the nearer
//!   end;
//! - elliptic arc: branch and bound over the parameter. A piece
//!   `[ta, tb]` lies within `max(rx, ry) (tb - ta)^2 / 8` of its chord
//!   (the interpolation error of a C^2 curve, `|e''| <= max(rx, ry)`), so
//!   the distance to the chord minus that bound is a lower bound and the
//!   distance to a piece's end point an upper bound. Pieces are bisected
//!   until the bounds meet to `1e-9 (1 + max(rx, ry))` or the box's own
//!   size, so the interval encloses the exact distance.
//!
//! Wherever an interval test cannot decide which formula applies, the
//! hull of the candidates is taken. The sign comes from the winding number
//! (see [`Region::eval`]) and is used only when the distance is certainly
//! positive; otherwise the result is the band `[-d.hi, d.hi]`.

use std::f64::consts::{PI, TAU};

use cadview_core::solid::{Profile2d, Seg2};

use crate::interval::{Iv, Iv2};

/// How far apart consecutive segment ends may be and still join (mm), as
/// `types.rs` states for profiles.
pub const JOIN_TOL: f64 = 1e-7;

/// How close two non-adjacent segments may come before the profile is
/// "not defined" as a simple region (mm). Above the join tolerance and the
/// harness band, so a profile the reference accepts is unambiguous.
pub const SEP: f64 = 1e-6;

/// One boundary piece with its certified data.
#[derive(Debug, Clone)]
enum Piece {
    /// A straight segment between two (possibly interval) points.
    Line { a: Iv2, b: Iv2, fa: P, fb: P },
    /// A circular or elliptic arc.
    Arc(ArcData),
}

type P = (f64, f64);

#[derive(Debug, Clone)]
struct ArcData {
    c: P,
    rx: f64,
    ry: f64,
    rot: f64,
    t0: f64,
    sweep: f64,
    /// A full turn: a closed loop on its own.
    full: bool,
    /// Certified end points.
    a: Iv2,
    b: Iv2,
    /// Float end points used by the winding computation (the midpoints of
    /// `a` and `b`, so consecutive pieces chain exactly in floats).
    fa: P,
    fb: P,
    /// Start and end directions from the centre (circles only).
    ua: Iv2,
    ub: Iv2,
}

impl ArcData {
    fn circular(&self) -> bool {
        self.rx == self.ry
    }

    /// `c + R(rot) (rx cos t, ry sin t)`, enclosed.
    fn point(&self, t: f64) -> Iv2 {
        let (ct, st) = (Iv::cos_of(t), Iv::sin_of(t));
        let (cr, sr) = (Iv::cos_of(self.rot), Iv::sin_of(self.rot));
        let x = ct * self.rx;
        let y = st * self.ry;
        Iv2::new(
            Iv::point(self.c.0) + x * cr - y * sr,
            Iv::point(self.c.1) + x * sr + y * cr,
        )
    }

    /// The local, unrotated, unit-circle coordinates of `q` minus one:
    /// `(lx / rx)^2 + (ly / ry)^2 - 1`, negative strictly inside.
    fn ellipse_level(&self, q: Iv2) -> Iv {
        let (cr, sr) = (Iv::cos_of(self.rot), Iv::sin_of(self.rot));
        let v = q - Iv2::point(self.c.0, self.c.1);
        let lx = v.x * cr + v.y * sr;
        let ly = -(v.x * sr) + v.y * cr;
        (lx / Iv::point(self.rx)).sqr() + (ly / Iv::point(self.ry)).sqr() - 1.0
    }
}

/// A closed form for one loop's signed distance.
#[derive(Debug, Clone)]
enum Fast {
    /// A convex polygon: each edge's start point and inward unit normal.
    Convex(Vec<(Iv2, Iv2)>),
    /// A full circle: centre and radius.
    Circle(Iv2, f64),
}

/// The closed form of a loop, if it has one.
fn fast_form(lp: &[Piece]) -> Option<Fast> {
    if let [Piece::Arc(a)] = lp {
        if a.full && a.circular() {
            return Some(Fast::Circle(Iv2::point(a.c.0, a.c.1), a.rx));
        }
        return None;
    }
    let mut pts = Vec::with_capacity(lp.len());
    for p in lp {
        match p {
            Piece::Line { a, fa, fb, .. } if dist(*fa, *fb) > SEP => pts.push(*a),
            _ => return None,
        }
    }
    let n = pts.len();
    if n < 3 {
        return None;
    }
    let mut area2 = Iv::ZERO;
    for k in 0..n {
        area2 = area2 + pts[k].cross(pts[(k + 1) % n]);
    }
    let ccw = area2.sign()?;
    for k in 0..n {
        let (a, b, c) = (pts[k], pts[(k + 1) % n], pts[(k + 2) % n]);
        let turn = (b - a).cross(c - b);
        let wrong_way = if ccw { turn.lo < 0.0 } else { turn.hi > 0.0 };
        if wrong_way {
            return None;
        }
    }
    let edges = (0..n)
        .map(|k| {
            let (a, b) = (pts[k], pts[(k + 1) % n]);
            let d = b - a;
            let len = d.norm();
            // Inward: left of a -> b for a counter-clockwise loop.
            let nrm = if ccw {
                Iv2::new(-d.y, d.x)
            } else {
                Iv2::new(d.y, -d.x)
            };
            (a, nrm.scale(Iv::point(1.0) / len))
        })
        .collect();
    Some(Fast::Convex(edges))
}

/// A profile region with certified evaluation.
#[derive(Debug, Clone)]
pub struct Region {
    /// Loop 0 is the outer loop.
    loops: Vec<Vec<Piece>>,
    /// A closed form per loop, where one applies.
    fast: Vec<Option<Fast>>,
    /// Conservative box of the region: `[umin, vmin, umax, vmax]`.
    pub bbox: [f64; 4],
}

/// Why a profile has no reference region.
#[derive(Debug, Clone, PartialEq)]
pub enum RegionError {
    /// Malformed input a kernel should refuse (open loop, zero radius).
    Invalid(String),
    /// Well-formed but outside what the reference defines (a spline, a
    /// self-intersecting loop, an undecidable configuration).
    Undefined(String),
}

fn inv(s: impl Into<String>) -> RegionError {
    RegionError::Invalid(s.into())
}
fn undef(s: impl Into<String>) -> RegionError {
    RegionError::Undefined(s.into())
}

fn fpt(p: kurbo::Point) -> P {
    (p.x, p.y)
}

fn dist(a: P, b: P) -> f64 {
    (a.0 - b.0).hypot(a.1 - b.1)
}

impl Region {
    /// The region of `profile`, checked as the module note requires.
    pub fn new(profile: &Profile2d) -> Result<Region, RegionError> {
        if profile.outer.is_empty() {
            return Err(inv("profile has no outer loop"));
        }
        let mut loops = Vec::new();
        let mut raw = Vec::new();
        for (li, lp) in std::iter::once(&profile.outer)
            .chain(profile.holes.iter())
            .enumerate()
        {
            let pieces = build_loop(lp).map_err(|e| match e {
                RegionError::Invalid(s) => inv(format!("loop {li}: {s}")),
                RegionError::Undefined(s) => undef(format!("loop {li}: {s}")),
            })?;
            raw.push(lp.clone());
            loops.push(pieces);
        }
        // Simplicity: no two pieces of any loops meet except consecutive
        // pieces of one loop at their joint.
        for i in 0..loops.len() {
            check_simple_loop(&loops[i]).map_err(|s| undef(format!("loop {i}: {s}")))?;
            for j in i + 1..loops.len() {
                for p in &loops[i] {
                    for q in &loops[j] {
                        if pieces_meet(p, q, &[])? {
                            return Err(undef(format!("loops {i} and {j} meet")));
                        }
                    }
                }
            }
        }
        let fast = loops.iter().map(|lp| fast_form(lp)).collect();
        let mut region = Region {
            loops,
            fast,
            bbox: [0.0; 4],
        };
        // Every hole inside the outer loop and in no other hole: test the
        // first point of each hole (the loops do not meet, so one point
        // decides).
        for h in 1..region.loops.len() {
            let p = first_point(&region.loops[h]);
            if region.loop_winding(0, p) != Some(true) {
                return Err(undef(format!(
                    "hole loop {} is not inside the outer loop",
                    h - 1
                )));
            }
            for g in 1..region.loops.len() {
                if g != h && region.loop_winding(g, p) != Some(false) {
                    return Err(undef(format!(
                        "hole loop {} lies inside hole loop {}",
                        h - 1,
                        g - 1
                    )));
                }
            }
        }
        region.bbox = bbox_of(&region.loops);
        Ok(region)
    }

    /// An upper bound on the length of the region's boundary: every
    /// piece's length rounded up (a line's chord; an arc's `max(rx, ry)`
    /// times its sweep, which bounds an elliptic arc's length too).
    pub fn perimeter_bound(&self) -> f64 {
        let mut total = Iv::ZERO;
        for lp in &self.loops {
            for p in lp {
                total = total
                    + match p {
                        Piece::Line { a, b, .. } => (*b - *a).norm(),
                        Piece::Arc(arc) => {
                            Iv::point(arc.rx.max(arc.ry)) * Iv::point(arc.sweep.abs())
                        }
                    };
            }
        }
        total.hi
    }

    /// An upper bound on the region's area: its box's.
    pub fn area_bound(&self) -> f64 {
        let b = self.bbox;
        let w = (Iv::point(b[2]) - Iv::point(b[0])).max(Iv::ZERO);
        let h = (Iv::point(b[3]) - Iv::point(b[1])).max(Iv::ZERO);
        (w * h).hi
    }

    /// The edges (start point, inward unit normal) of a region that is one
    /// convex polygon with no holes.
    pub fn convex_outer(&self) -> Option<Vec<(Iv2, Iv2)>> {
        match (&self.fast[0], self.loops.len()) {
            (Some(Fast::Convex(e)), 1) => Some(e.clone()),
            _ => None,
        }
    }

    /// Encloses the region's signed function over the box `q`:
    /// `max(f_outer, -f_hole, ...)`, each `f_loop` the signed distance to
    /// one loop, negative inside it. For sign-exact distance bounds this
    /// `max` is again sign-exact and a lower bound on the distance to the
    /// region's boundary (the argument of the CSG note in the parent
    /// module), and it is 1-Lipschitz. Per loop:
    ///
    /// - a convex polygon of lines (certified convex by interval cross
    ///   products): the maximum of the signed distances to its edge lines,
    ///   which equals the exact signed distance inside and is at most the
    ///   exact distance outside, with the exact sign. Each term is linear,
    ///   so over a box its interval is tight;
    /// - a full circle: `|q - c| - r`, exact;
    /// - any other loop: the minimum distance to its pieces (module note),
    ///   with the sign from the winding number of the box's centre. Each
    ///   line piece contributes the angle its chord subtends; an arc
    ///   contributes its chord's angle plus a full turn (signed like its
    ///   sweep) when the point is inside its ellipse and on the arc's side
    ///   of the chord, the region the arc and its chord enclose. The side
    ///   test reuses the very cross product that sets the chord angle's
    ///   branch, so the two can never disagree, and the in-ellipse test is
    ///   an interval test that reports "undecided" when it cannot tell.
    ///   The sum is a multiple of a full turn in exact arithmetic; it is
    ///   rounded to the nearest one and rejected if more than one radian
    ///   away. The sign is used only when the box is certainly at a
    ///   positive distance from the loop, so membership is constant over
    ///   the box; otherwise the loop gives the band `[-d.hi, d.hi]`.
    pub fn eval(&self, q: Iv2) -> Iv {
        let mut f = self.loop_eval(0, q);
        for h in 1..self.loops.len() {
            f = f.max(-self.loop_eval(h, q));
        }
        f
    }

    fn loop_eval(&self, li: usize, q: Iv2) -> Iv {
        match &self.fast[li] {
            Some(Fast::Convex(edges)) => {
                let mut f = Iv::point(f64::NEG_INFINITY);
                for (a, n) in edges {
                    // Outward distance to the edge line: -n . (q - a).
                    f = f.max(-n.dot(q - *a));
                }
                f
            }
            Some(Fast::Circle(c, r)) => (q - *c).norm() - *r,
            None => {
                let tol_box = q.x.width().hypot(q.y.width());
                let mut d = Iv::point(f64::INFINITY);
                for piece in &self.loops[li] {
                    d = d.min(piece_distance(piece, q, tol_box));
                }
                let d = Iv::new(d.lo.max(0.0), d.hi.max(0.0));
                if d.lo > 0.0 {
                    if let Some(inside) = self.loop_winding(li, q.mid()) {
                        return if inside { -d } else { d };
                    }
                }
                Iv::new(-d.hi, d.hi)
            }
        }
    }

    /// Whether `p` has a non-zero winding number about loop `li`.
    fn loop_winding(&self, li: usize, p: P) -> Option<bool> {
        let mut total = 0.0;
        for piece in &self.loops[li] {
            total += piece_angle(piece, p)?;
        }
        let k = (total / TAU).round();
        if (total - k * TAU).abs() > 1.0 {
            return None;
        }
        Some(k != 0.0)
    }
}

fn first_point(lp: &[Piece]) -> P {
    match &lp[0] {
        Piece::Line { fa, .. } => *fa,
        Piece::Arc(a) => a.fa,
    }
}

/// The angle the chord from `a` to `b` subtends at `p`, and the cross
/// product that fixed its branch (zero normalised to `+0`).
fn chord_angle(a: P, b: P, p: P) -> (f64, f64) {
    let (ax, ay) = (a.0 - p.0, a.1 - p.1);
    let (bx, by) = (b.0 - p.0, b.1 - p.1);
    let mut cr = ax * by - ay * bx;
    if cr == 0.0 {
        cr = 0.0; // turns -0.0 into +0.0
    }
    let dt = ax * bx + ay * by;
    (cr.atan2(dt), cr)
}

fn piece_angle(piece: &Piece, p: P) -> Option<f64> {
    match piece {
        Piece::Line { fa, fb, .. } => Some(chord_angle(*fa, *fb, p).0),
        Piece::Arc(arc) => {
            let level = arc.ellipse_level(Iv2::point(p.0, p.1));
            if arc.full {
                let inside = level.sign().map(|s| !s)?;
                return Some(if inside {
                    TAU * arc.sweep.signum()
                } else {
                    0.0
                });
            }
            let (angle, cr) = chord_angle(arc.fa, arc.fb, p);
            // The arc lies right of its chord when counter-clockwise, left
            // when clockwise; `cr < 0` is "p right of a -> b".
            let arc_side = if arc.sweep > 0.0 { cr < 0.0 } else { cr > 0.0 };
            if !arc_side {
                return Some(angle);
            }
            let inside = level.sign().map(|s| !s)?;
            Some(if inside {
                angle + TAU * arc.sweep.signum()
            } else {
                angle
            })
        }
    }
}

/// Encloses the distance from every point of box `q` to a segment with
/// interval end points. Two enclosures are intersected: the exact formula
/// (perpendicular distance or nearer end, by where the foot falls), and
/// the end-point bound `max(|q - a|, |q - b|) - |b - a| <= dist <=
/// min(|q - a|, |q - b|)` (every point of the segment is within its length
/// of both ends). The second keeps short join segments, whose direction
/// is poorly known, tight.
pub(crate) fn segment_distance(a: Iv2, b: Iv2, q: Iv2) -> Iv {
    let d = b - a;
    let len2 = d.norm2();
    let len = len2.sqrt();
    let qa = (q - a).norm();
    let qb = (q - b).norm();
    let ends_lo = (qa.max(qb) - len).lo.max(0.0);
    let ends_hi = qa.min(qb).hi;
    let ends = Iv::new(ends_lo.min(ends_hi), ends_hi);
    if len2.lo <= 1e-200 {
        return ends;
    }
    let t = (q - a).dot(d) / len2;
    let perp = (d.cross(q - a)).abs() / len;
    let mut out: Option<Iv> = None;
    let mut take = |x: Iv| out = Some(out.map_or(x, |o| o.hull(x)));
    if t.hi >= 0.0 && t.lo <= 1.0 {
        take(perp);
    }
    if t.lo < 0.0 {
        take(qa);
    }
    if t.hi > 1.0 {
        take(qb);
    }
    // When t is certainly inside [0, 1] only `perp` was taken, when
    // certainly outside only an end; otherwise the hull of the candidates,
    // which holds the true value.
    let exact = out.unwrap_or(perp);
    let lo = exact.lo.max(ends.lo);
    let hi = exact.hi.min(ends.hi);
    if lo <= hi {
        Iv::new(lo, hi)
    } else {
        // Both hold the true value, so they overlap in exact arithmetic;
        // an empty meet can only come from a degenerate input.
        exact.hull(ends)
    }
}

fn piece_distance(piece: &Piece, q: Iv2, tol_box: f64) -> Iv {
    match piece {
        Piece::Line { a, b, .. } => segment_distance(*a, *b, q),
        Piece::Arc(arc) if arc.circular() => circle_arc_distance(arc, q),
        Piece::Arc(arc) => ellipse_arc_distance(arc, q, tol_box),
    }
}

fn circle_arc_distance(arc: &ArcData, q: Iv2) -> Iv {
    let v = q - Iv2::point(arc.c.0, arc.c.1);
    let on_circle = (v.norm() - arc.rx).abs();
    if arc.full {
        return on_circle;
    }
    let ends = (q - arc.a).norm().min((q - arc.b).norm());
    // Normalise to a counter-clockwise span from u0 to u1.
    let (u0, u1) = if arc.sweep > 0.0 {
        (arc.ua, arc.ub)
    } else {
        (arc.ub, arc.ua)
    };
    let span = arc.sweep.abs();
    // Tri-state: Some(true) in the span, Some(false) out, None unsure.
    let in_span = if span <= PI {
        let c1 = u0.cross(v);
        let c2 = v.cross(u1);
        if c1.lo >= 0.0 && c2.lo >= 0.0 {
            Some(true)
        } else if c1.hi < 0.0 || c2.hi < 0.0 {
            Some(false)
        } else {
            None
        }
    } else {
        // Out of the span iff strictly inside the complementary sector
        // from u1 to u0 (less than half a turn).
        let c1 = u1.cross(v);
        let c2 = v.cross(u0);
        if c1.lo > 0.0 && c2.lo > 0.0 {
            Some(false)
        } else if c1.hi <= 0.0 || c2.hi <= 0.0 {
            Some(true)
        } else {
            None
        }
    };
    match in_span {
        Some(true) => on_circle,
        Some(false) => ends,
        None => on_circle.hull(ends),
    }
}

fn ellipse_arc_distance(arc: &ArcData, q: Iv2, tol_box: f64) -> Iv {
    let rmax = arc.rx.max(arc.ry);
    let tol = (1e-9 * (1.0 + rmax)).max(tol_box);
    // (ta, tb, point(ta), point(tb), lower bound, upper bound)
    struct Pc {
        ta: f64,
        tb: f64,
        pa: Iv2,
        pb: Iv2,
        lower: f64,
        upper: f64,
    }
    let make = |ta: f64, tb: f64, pa: Iv2, pb: Iv2| -> Pc {
        let dt = Iv::point(tb) - Iv::point(ta);
        let dev = (dt.sqr() * rmax / Iv::point(8.0)).hi;
        let chord = segment_distance(pa, pb, q);
        let upper = (q - pa).norm().hi.min((q - pb).norm().hi);
        Pc {
            ta,
            tb,
            pa,
            pb,
            lower: chord.lo - dev,
            upper,
        }
    };
    let n0 = 16;
    let mut pieces: Vec<Pc> = Vec::with_capacity(64);
    let t_at = |k: usize| arc.t0 + arc.sweep * (k as f64) / (n0 as f64);
    let mut prev_t = t_at(0);
    let mut prev_p = arc.point(prev_t);
    for k in 1..=n0 {
        let t = t_at(k);
        let p = arc.point(t);
        pieces.push(make(prev_t, t, prev_p, p));
        prev_t = t;
        prev_p = p;
    }
    for _ in 0..400 {
        let upper = pieces.iter().map(|p| p.upper).fold(f64::INFINITY, f64::min);
        let (i, lower) = pieces
            .iter()
            .enumerate()
            .map(|(i, p)| (i, p.lower))
            .fold((0, f64::INFINITY), |b, x| if x.1 < b.1 { x } else { b });
        if upper - lower <= tol {
            break;
        }
        let pc = pieces.swap_remove(i);
        let tm = 0.5 * (pc.ta + pc.tb);
        let pm = arc.point(tm);
        pieces.push(make(pc.ta, tm, pc.pa, pm));
        pieces.push(make(tm, pc.tb, pm, pc.pb));
        // Pieces that cannot hold the minimum are dropped.
        let upper = pieces.iter().map(|p| p.upper).fold(f64::INFINITY, f64::min);
        pieces.retain(|p| p.lower <= upper);
    }
    let upper = pieces.iter().map(|p| p.upper).fold(f64::INFINITY, f64::min);
    let lower = pieces.iter().map(|p| p.lower).fold(f64::INFINITY, f64::min);
    Iv::new(lower.max(0.0).min(upper), upper)
}

/// Builds one loop's pieces, with join lines, checking closure.
fn build_loop(lp: &[Seg2]) -> Result<Vec<Piece>, RegionError> {
    if lp.is_empty() {
        return Err(inv("loop is empty"));
    }
    let mut pieces = Vec::new();
    for seg in lp {
        pieces.push(match seg {
            Seg2::Line(l) => {
                let (a, b) = (fpt(l.p0), fpt(l.p1));
                if ![a.0, a.1, b.0, b.1].iter().all(|v| v.is_finite()) {
                    return Err(inv("a line is not finite"));
                }
                if dist(a, b) <= SEP {
                    return Err(undef("a line is shorter than the separation tolerance"));
                }
                Piece::Line {
                    a: Iv2::point(a.0, a.1),
                    b: Iv2::point(b.0, b.1),
                    fa: a,
                    fb: b,
                }
            }
            Seg2::Arc(a) => arc_piece(
                fpt(a.center),
                a.radii.x,
                a.radii.y,
                a.x_rotation,
                a.start_angle,
                a.sweep_angle,
            )?,
            Seg2::Circle(c) => arc_piece(fpt(c.center), c.radius, c.radius, 0.0, 0.0, TAU)?,
            Seg2::Spline { .. } => {
                return Err(undef("a spline segment has no reference meaning yet"));
            }
        });
    }
    let full = pieces.iter().any(|p| matches!(p, Piece::Arc(a) if a.full));
    if full {
        if pieces.len() != 1 {
            return Err(inv("a closed curve must be a loop on its own"));
        }
        return Ok(pieces);
    }
    // Join each end to the next start; check the gap.
    let n = pieces.len();
    let mut out = Vec::with_capacity(2 * n);
    for i in 0..n {
        let (end_iv, end_f) = ends(&pieces[i]).1;
        let (start_iv, start_f) = ends(&pieces[(i + 1) % n]).0;
        out.push(pieces[i].clone());
        let gap = dist(end_f, start_f);
        if gap > JOIN_TOL {
            return Err(inv(format!(
                "segment {i} ends {gap:e} mm from where the next one starts"
            )));
        }
        if end_iv != start_iv {
            out.push(Piece::Line {
                a: end_iv,
                b: start_iv,
                fa: end_f,
                fb: start_f,
            });
        }
    }
    if n == 1 {
        return Err(inv("a loop of one open segment does not close"));
    }
    Ok(out)
}

type End = (Iv2, P);

fn ends(p: &Piece) -> (End, End) {
    match p {
        Piece::Line { a, b, fa, fb } => ((*a, *fa), (*b, *fb)),
        Piece::Arc(arc) => ((arc.a, arc.fa), (arc.b, arc.fb)),
    }
}

fn arc_piece(c: P, rx: f64, ry: f64, rot: f64, t0: f64, sweep: f64) -> Result<Piece, RegionError> {
    if ![c.0, c.1, rx, ry, rot, t0, sweep]
        .iter()
        .all(|v| v.is_finite())
    {
        return Err(inv("an arc is not finite"));
    }
    if !(rx > 0.0 && ry > 0.0) {
        return Err(inv("an arc has a non-positive radius"));
    }
    if sweep.abs() * rx.max(ry) <= SEP {
        return Err(undef("an arc is shorter than the separation tolerance"));
    }
    let full = sweep.abs() >= TAU - 1e-12;
    let mut arc = ArcData {
        c,
        rx,
        ry,
        rot,
        t0,
        sweep,
        full,
        a: Iv2::point(0.0, 0.0),
        b: Iv2::point(0.0, 0.0),
        fa: (0.0, 0.0),
        fb: (0.0, 0.0),
        ua: Iv2::point(0.0, 0.0),
        ub: Iv2::point(0.0, 0.0),
    };
    arc.a = arc.point(t0);
    arc.b = if full { arc.a } else { arc.point(t0 + sweep) };
    arc.fa = arc.a.mid();
    arc.fb = arc.b.mid();
    if rx == ry {
        // Directions from the centre: (a - c) / r, enclosed.
        let r = Iv::point(rx);
        arc.ua = (arc.a - Iv2::point(c.0, c.1)).scale(Iv::point(1.0) / r);
        arc.ub = (arc.b - Iv2::point(c.0, c.1)).scale(Iv::point(1.0) / r);
    }
    Ok(Piece::Arc(arc))
}

fn bbox_of(loops: &[Vec<Piece>]) -> [f64; 4] {
    let mut b = [
        f64::INFINITY,
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::NEG_INFINITY,
    ];
    let mut add = |x: Iv, y: Iv| {
        b[0] = b[0].min(x.lo);
        b[1] = b[1].min(y.lo);
        b[2] = b[2].max(x.hi);
        b[3] = b[3].max(y.hi);
    };
    // The outer loop bounds the region.
    for p in &loops[0] {
        match p {
            Piece::Line { a, b, .. } => {
                add(a.x, a.y);
                add(b.x, b.y);
            }
            Piece::Arc(arc) => {
                // The whole ellipse lies within max radius of the centre.
                let r = arc.rx.max(arc.ry);
                let c = Iv2::point(arc.c.0, arc.c.1);
                add(c.x - r, c.y - r);
                add(c.x + r, c.y + r);
            }
        }
    }
    b
}

// ── Simplicity (a domain check, done in floats with margins) ──────────

fn check_simple_loop(lp: &[Piece]) -> Result<(), String> {
    // Join lines are at most the join tolerance long and stand between two
    // real pieces; adjacency is between real pieces.
    let real: Vec<&Piece> = lp.iter().filter(|p| !is_join(p)).collect();
    let n = real.len();
    if n < 2 {
        return Ok(());
    }
    for i in 0..n {
        for j in i + 1..n {
            let mut joints = Vec::new();
            if j == i + 1 {
                joints.push(ends(real[i]).1 .1);
            }
            if i == 0 && j == n - 1 {
                joints.push(ends(real[0]).0 .1);
            }
            let meet = pieces_meet(real[i], real[j], &joints).map_err(|e| match e {
                RegionError::Invalid(s) | RegionError::Undefined(s) => s,
            })?;
            if meet {
                return Err(format!("pieces {i} and {j} meet: the loop is not simple"));
            }
        }
    }
    Ok(())
}

fn is_join(p: &Piece) -> bool {
    matches!(p, Piece::Line { fa, fb, .. } if dist(*fa, *fb) <= JOIN_TOL * 2.0)
}

/// The float geometry of a piece for the domain checks.
enum G {
    Line(P, P),
    Arc {
        c: P,
        rx: f64,
        ry: f64,
        rot: f64,
        t0: f64,
        sweep: f64,
    },
}

fn geom(p: &Piece) -> G {
    match p {
        Piece::Line { fa, fb, .. } => G::Line(*fa, *fb),
        Piece::Arc(a) => G::Arc {
            c: a.c,
            rx: a.rx,
            ry: a.ry,
            rot: a.rot,
            t0: a.t0,
            sweep: a.sweep,
        },
    }
}

/// Whether two pieces come within [`SEP`] of each other, ignoring points
/// within [`SEP`] of `joint` when they are consecutive. Conservative: a
/// configuration it cannot decide (an ellipse against a curve) is an
/// `Undefined` error.
fn pieces_meet(p: &Piece, q: &Piece, joints: &[P]) -> Result<bool, RegionError> {
    let near_joint = |x: P| joints.iter().any(|j| dist(x, *j) <= 10.0 * SEP + JOIN_TOL);
    // Short join lines are within the join tolerance of their neighbours by
    // construction; their own distance to other pieces is covered by the
    // pieces they join.
    if is_join(p) || is_join(q) {
        return Ok(false);
    }
    let pts = match (geom(p), geom(q)) {
        (G::Line(a, b), G::Line(c, d)) => line_line(a, b, c, d),
        (G::Line(a, b), arc @ G::Arc { .. }) | (arc @ G::Arc { .. }, G::Line(a, b)) => {
            line_arc(a, b, &arc)
        }
        (a1 @ G::Arc { .. }, a2 @ G::Arc { .. }) => arc_arc(&a1, &a2)?,
    };
    Ok(pts.into_iter().any(|x| !near_joint(x)))
}

/// Points where two segments come within SEP (intersections, plus close
/// end points), as candidate points.
fn line_line(a: P, b: P, c: P, d: P) -> Vec<P> {
    let mut out = Vec::new();
    // End points close to the other segment.
    for (x, s0, s1) in [(a, c, d), (b, c, d), (c, a, b), (d, a, b)] {
        if point_seg_dist(x, s0, s1) <= SEP {
            out.push(x);
        }
    }
    let r = (b.0 - a.0, b.1 - a.1);
    let s = (d.0 - c.0, d.1 - c.1);
    let den = r.0 * s.1 - r.1 * s.0;
    if den.abs() > 1e-300 {
        let w = (c.0 - a.0, c.1 - a.1);
        let t = (w.0 * s.1 - w.1 * s.0) / den;
        let u = (w.0 * r.1 - w.1 * r.0) / den;
        if (0.0..=1.0).contains(&t) && (0.0..=1.0).contains(&u) {
            out.push((a.0 + t * r.0, a.1 + t * r.1));
        }
    }
    // Collinear overlap shows up as end points close to the other line.
    out
}

fn point_seg_dist(x: P, a: P, b: P) -> f64 {
    let d = (b.0 - a.0, b.1 - a.1);
    let l2 = d.0 * d.0 + d.1 * d.1;
    let t = if l2 > 0.0 {
        (((x.0 - a.0) * d.0 + (x.1 - a.1) * d.1) / l2).clamp(0.0, 1.0)
    } else {
        0.0
    };
    dist(x, (a.0 + t * d.0, a.1 + t * d.1))
}

/// The arc's parameter span test with an angular margin.
fn on_arc(t: f64, t0: f64, sweep: f64, margin: f64) -> bool {
    if sweep.abs() >= TAU - 1e-12 {
        return true;
    }
    let (lo, len) = if sweep > 0.0 {
        (t0, sweep)
    } else {
        (t0 + sweep, -sweep)
    };
    let rel = (t - lo).rem_euclid(TAU);
    rel <= len + margin || rel >= TAU - margin
}

fn line_arc(a: P, b: P, arc: &G) -> Vec<P> {
    let G::Arc {
        c,
        rx,
        ry,
        rot,
        t0,
        sweep,
    } = *arc
    else {
        return Vec::new();
    };
    // Map to the arc's unit circle: x' = diag(1/rx, 1/ry) R(-rot) (x - c).
    let (s, co) = rot.sin_cos();
    let to = |x: P| {
        let (dx, dy) = (x.0 - c.0, x.1 - c.1);
        ((dx * co + dy * s) / rx, (-dx * s + dy * co) / ry)
    };
    let back = |u: P| {
        let (lx, ly) = (u.0 * rx, u.1 * ry);
        (c.0 + lx * co - ly * s, c.1 + lx * s + ly * co)
    };
    let (pa, pb) = (to(a), to(b));
    let rmin = rx.min(ry);
    // The distance in unit-circle space shrinks by at most 1 / rmin.
    let tol = SEP / rmin;
    let d = (pb.0 - pa.0, pb.1 - pa.1);
    let mut out = Vec::new();
    // Closest approach of the line to the unit circle, and crossings.
    let aa = d.0 * d.0 + d.1 * d.1;
    let bb = 2.0 * (pa.0 * d.0 + pa.1 * d.1);
    let cc = pa.0 * pa.0 + pa.1 * pa.1 - 1.0;
    let mut ts = vec![];
    let disc = bb * bb - 4.0 * aa * cc;
    if disc >= 0.0 {
        let r = disc.sqrt();
        ts.push((-bb - r) / (2.0 * aa));
        ts.push((-bb + r) / (2.0 * aa));
    } else {
        // No crossing: the nearest approach, in case it is within tol.
        ts.push(-bb / (2.0 * aa));
    }
    for t in ts {
        let t = t.clamp(0.0, 1.0);
        let u = (pa.0 + t * d.0, pa.1 + t * d.1);
        let rr = u.0.hypot(u.1);
        if (rr - 1.0).abs() <= tol {
            let ang = u.1.atan2(u.0);
            if on_arc(ang, t0, sweep, tol) {
                out.push(back(u));
            }
        }
    }
    // Arc end points close to the line.
    if sweep.abs() < TAU - 1e-12 {
        for t in [t0, t0 + sweep] {
            let e = back((t.cos(), t.sin()));
            if point_seg_dist(e, a, b) <= SEP {
                out.push(e);
            }
        }
    }
    out
}

fn arc_arc(p: &G, q: &G) -> Result<Vec<P>, RegionError> {
    let (
        G::Arc {
            c: c1,
            rx: rx1,
            ry: ry1,
            t0: s1,
            sweep: w1,
            rot: o1,
        },
        G::Arc {
            c: c2,
            rx: rx2,
            ry: ry2,
            t0: s2,
            sweep: w2,
            rot: o2,
        },
    ) = (p, q)
    else {
        return Ok(Vec::new());
    };
    let (r1, r2) = (rx1.max(*ry1), rx2.max(*ry2));
    let dc = dist(*c1, *c2);
    if rx1 != ry1 || rx2 != ry2 {
        // Conservative for ellipses: only clearly separated discs pass.
        if dc > r1 + r2 + SEP {
            return Ok(Vec::new());
        }
        return Err(undef(
            "an elliptic arc near another curve: the reference cannot decide that the loop is simple",
        ));
    }
    let (t1a, t2a) = (s1 + o1, s2 + o2);
    let mut out = Vec::new();
    let ends1 = [t1a, t1a + w1].map(|t| (c1.0 + r1 * t.cos(), c1.1 + r1 * t.sin()));
    let ends2 = [t2a, t2a + w2].map(|t| (c2.0 + r2 * t.cos(), c2.1 + r2 * t.sin()));
    let near_arc = |x: P, c: P, r: f64, t0: f64, w: f64| {
        let (dx, dy) = (x.0 - c.0, x.1 - c.1);
        let rr = dx.hypot(dy);
        (rr - r).abs() <= SEP && on_arc(dy.atan2(dx), t0, w, SEP / r)
    };
    if w1.abs() < TAU - 1e-12 {
        for e in ends1 {
            if near_arc(e, *c2, r2, t2a, *w2) {
                out.push(e);
            }
        }
    }
    if w2.abs() < TAU - 1e-12 {
        for e in ends2 {
            if near_arc(e, *c1, r1, t1a, *w1) {
                out.push(e);
            }
        }
    }
    if dc <= SEP && (r1 - r2).abs() <= SEP {
        // The same circle: any overlap of spans shows as a near end above,
        // unless one arc contains the other entirely, which also puts its
        // ends on the other.
        return Ok(out);
    }
    if dc > r1 + r2 + SEP || dc < (r1 - r2).abs() - SEP || dc == 0.0 {
        return Ok(out);
    }
    // Circle-circle intersection (clamped for near tangency).
    let a = (r1 * r1 - r2 * r2 + dc * dc) / (2.0 * dc);
    let h = (r1 * r1 - a * a).max(0.0).sqrt();
    let ux = ((c2.0 - c1.0) / dc, (c2.1 - c1.1) / dc);
    let m = (c1.0 + a * ux.0, c1.1 + a * ux.1);
    for sgn in [1.0, -1.0] {
        let x = (m.0 - sgn * h * ux.1, m.1 + sgn * h * ux.0);
        let on1 = on_arc((x.1 - c1.1).atan2(x.0 - c1.0), t1a, *w1, SEP / r1);
        let on2 = on_arc((x.1 - c2.1).atan2(x.0 - c2.0), t2a, *w2, SEP / r2);
        if on1 && on2 {
            out.push(x);
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use kurbo::{Arc, Circle, Line, Point};

    fn at(r: &Region, x: f64, y: f64) -> Iv {
        r.eval(Iv2::point(x, y))
    }

    #[test]
    fn rectangle_distances_are_exact() {
        let r = Region::new(&Profile2d::rect(0.0, 0.0, 10.0, 4.0)).unwrap();
        let f = at(&r, 5.0, 1.0);
        assert!(f.contains(-1.0) && f.width() < 1e-12, "{f:?}");
        let f = at(&r, 13.0, 8.0);
        // Outside a convex corner the bound is the larger edge-line distance
        // (4), below the true distance (5): a lower bound with the right sign.
        assert!(f.contains(4.0) && f.width() < 1e-12, "{f:?}");
        let f = at(&r, 10.0, 2.0);
        assert!(f.contains_zero());
        // Clockwise loops mean the same region.
        let cw = Profile2d::polygon(&[
            Point::new(0.0, 0.0),
            Point::new(0.0, 4.0),
            Point::new(10.0, 4.0),
            Point::new(10.0, 0.0),
        ]);
        let r = Region::new(&cw).unwrap();
        assert!(at(&r, 5.0, 1.0).hi < 0.0);
        assert!(at(&r, -1.0, 1.0).lo > 0.0);
    }

    #[test]
    fn washer_and_slot() {
        let p = Profile2d::circle(Point::new(0.0, 0.0), 10.0)
            .with_hole(vec![Seg2::Circle(Circle::new((0.0, 0.0), 4.0))]);
        let r = Region::new(&p).unwrap();
        let f = at(&r, 7.0, 0.0);
        assert!(f.contains(-3.0) && f.width() < 1e-12, "{f:?}");
        let f = at(&r, 1.0, 0.0);
        assert!(f.contains(3.0), "{f:?}");
        let f = at(&r, 0.0, 12.0);
        assert!(f.contains(2.0), "{f:?}");
        // A stadium: lines and half circles, counter-clockwise.
        let slot = Profile2d {
            outer: vec![
                Seg2::Line(Line::new((0.0, 0.0), (40.0, 0.0))),
                Seg2::Arc(Arc {
                    center: Point::new(40.0, 5.0),
                    radii: kurbo::Vec2::new(5.0, 5.0),
                    start_angle: -PI / 2.0,
                    sweep_angle: PI,
                    x_rotation: 0.0,
                }),
                Seg2::Line(Line::new((40.0, 10.0), (0.0, 10.0))),
                Seg2::Arc(Arc {
                    center: Point::new(0.0, 5.0),
                    radii: kurbo::Vec2::new(5.0, 5.0),
                    start_angle: PI / 2.0,
                    sweep_angle: PI,
                    x_rotation: 0.0,
                }),
            ],
            holes: vec![],
        };
        let r = Region::new(&slot).unwrap();
        let f = at(&r, 44.0, 5.0);
        assert!(f.contains(-1.0) && f.width() < 1e-9, "{f:?}");
        let f = at(&r, -6.0, 5.0);
        assert!(f.contains(1.0) && f.width() < 1e-9, "{f:?}");
        // The point (20, 5) is 5 from both long sides.
        assert!(at(&r, 20.0, 5.0).contains(-5.0));
        // Inside the circle of the right cap but left of its chord: the
        // complementary half circle is not boundary.
        let f = at(&r, 37.0, 5.0);
        assert!(f.contains(-5.0), "{f:?}");
    }

    #[test]
    fn ellipse_distance_brackets_the_truth() {
        let e = Profile2d {
            outer: vec![Seg2::Arc(Arc {
                center: Point::new(0.0, 0.0),
                radii: kurbo::Vec2::new(8.0, 3.0),
                start_angle: 0.0,
                sweep_angle: TAU,
                x_rotation: 0.3,
            })],
            holes: vec![],
        };
        let r = Region::new(&e).unwrap();
        // Along the major axis the nearest point is the vertex.
        let (s, c) = 0.3f64.sin_cos();
        let f = at(&r, 10.0 * c, 10.0 * s);
        assert!(f.lo > 0.0 && f.contains(2.0), "{f:?}");
        assert!(f.width() < 1e-6, "{f:?}");
        assert!(at(&r, 0.0, 0.0).hi < -2.9);
    }

    #[test]
    fn bad_profiles_are_refused() {
        // A bow tie.
        let bow = Profile2d::polygon(&[
            Point::new(0.0, 0.0),
            Point::new(10.0, 10.0),
            Point::new(10.0, 0.0),
            Point::new(0.0, 10.0),
        ]);
        assert!(matches!(Region::new(&bow), Err(RegionError::Undefined(_))));
        // A hole poking out.
        let p = Profile2d::rect(0.0, 0.0, 10.0, 10.0)
            .with_hole(Profile2d::rect(5.0, 5.0, 15.0, 8.0).outer);
        assert!(Region::new(&p).is_err());
        // A hole outside.
        let p = Profile2d::rect(0.0, 0.0, 10.0, 10.0)
            .with_hole(Profile2d::rect(20.0, 5.0, 25.0, 8.0).outer);
        assert!(Region::new(&p).is_err());
        // An open loop.
        let open = Profile2d {
            outer: vec![
                Seg2::Line(Line::new((0.0, 0.0), (1.0, 0.0))),
                Seg2::Line(Line::new((1.0, 0.0), (1.0, 1.0))),
            ],
            holes: vec![],
        };
        assert!(matches!(Region::new(&open), Err(RegionError::Invalid(_))));
    }
}
