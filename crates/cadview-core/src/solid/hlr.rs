//! Hidden-line removal on exact curves (docs/3d-design.md 7.2 and 7.3).
//!
//! Input: exact 3D curves (part edges and silhouettes, in world
//! coordinates) and a view frame. Output: exact 2D shapes, each visible or
//! hidden, each tagged with the part and kind of curve it came from.
//!
//! How it works:
//!
//! 1. **Project exactly.** An orthographic projection is affine, so a line
//!    stays a line, a circle or an ellipse becomes an ellipse (a circle when
//!    its plane faces the viewer, a segment when the view runs along its
//!    plane), and a B-spline's points project one by one. Lines along the
//!    view direction are dropped (they are points).
//! 2. **Split where visibility can change.** Seen along a curve, its
//!    visibility only changes where it crosses, in the view, the outline of
//!    something in front: another projected edge or silhouette. So every
//!    curve is split at its crossings with every other curve, at the end
//!    points of other curves that lie on it (a silhouette ends on a rim),
//!    and where a conic seen edge on folds back. Crossings are closed forms
//!    where one curve is straight (a line, or a conic seen edge on), and
//!    Newton's method on the exact parameters otherwise, started from a
//!    flattened copy (used only to find starting points, never output).
//! 3. **Classify each piece** at its middle with an [`Oracle`] (the mesh
//!    ray cast with an exact fallback, see `solid::oracle`). Long pieces
//!    are sampled at a quarter and three quarters too; when the samples
//!    disagree (a crossing was missed), the change is found by bisection on
//!    the exact parameter and the piece is split there.
//! 4. **Merge coincident curves.** Pieces on one 2D carrier (one line, one
//!    circle, one ellipse) are laid on that carrier's parameter, cut into
//!    elementary intervals at every end point, and each interval is visible
//!    if any piece over it is visible, else hidden if any is: so a view
//!    carries no duplicates and visible wins over hidden. Each run of equal
//!    intervals becomes one entity, owned by the first part (in the order
//!    given) of the winning class over it.
//! 5. **Emit** a `Line`, `Arc`, `Circle` or `Ellipse` per run. A B-spline
//!    piece (no exact conic form) is fitted with a cubic spline (Hermite
//!    pieces, to [`Options::fit_tol`]).
//!
//! The split parameters are exact crossings (closed form, or Newton to the
//! rounding of the arithmetic), so pieces of one curve meet exactly, and
//! the pieces of different curves meet where their exact curves cross.

use std::f64::consts::{PI, TAU};

use kurbo::{Point, Vec2};

use super::kernel::Curve3;
use super::types::{Point3, Vec3};
use crate::types::Shape;

/// An orthographic view: the eye looks along `-back`; `right` and `up` are
/// the sheet's x and y; `origin` is the 3D point drawn at the sheet's
/// (0, 0). The three axes are orthonormal and right-handed.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ViewFrame {
    pub origin: Point3,
    pub right: Vec3,
    pub up: Vec3,
    pub back: Vec3,
}

impl ViewFrame {
    /// A 3D point in the view's 2D coordinates.
    pub fn point(&self, p: Point3) -> Point {
        let d = p - self.origin;
        Point::new(d.dot(self.right), d.dot(self.up))
    }

    /// A 3D direction in the view's 2D coordinates.
    pub fn vector(&self, v: Vec3) -> Vec2 {
        Vec2::new(v.dot(self.right), v.dot(self.up))
    }
}

/// What a curve is on its part.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum CurveKind {
    Edge,
    Silhouette,
}

/// One exact curve to draw: `curve` over `range`, on part `owner` (an index
/// the caller understands; a lower index wins a merge).
#[derive(Debug, Clone)]
pub struct Source {
    pub curve: Curve3,
    pub range: [f64; 2],
    pub owner: usize,
    pub kind: CurveKind,
}

/// Decides whether a point on part `owner` is hidden from the eye.
pub trait Oracle {
    fn hidden(&mut self, p: Point3, owner: usize) -> bool;
}

/// One output shape.
#[derive(Debug, Clone)]
pub struct Drawn {
    pub shape: Shape,
    pub hidden: bool,
    pub owner: usize,
    pub kind: CurveKind,
}

#[derive(Debug, Clone, Copy)]
pub struct Options {
    /// Keep hidden pieces.
    pub hidden: bool,
    /// Largest distance (mm) of a fitted spline from its B-spline source.
    pub fit_tol: f64,
    /// Also return every classified piece with its exact curve ([`Traced`]),
    /// for tests that check the oracle.
    pub trace: bool,
}

impl Default for Options {
    fn default() -> Self {
        Options {
            hidden: true,
            fit_tol: 0.01,
            trace: false,
        }
    }
}

/// Counts for reports and tests.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Stats {
    /// Curves after dropping those seen end on.
    pub curves: usize,
    /// Split points found (crossings, end points on curves, folds).
    pub splits: usize,
    /// Pieces classified.
    pub pieces: usize,
    /// Oracle questions asked.
    pub queries: usize,
    /// Pieces whose samples disagreed and were split by bisection.
    pub bisected: usize,
    /// Pieces drawn as fitted splines.
    pub fitted: usize,
    /// Milliseconds spent splitting, classifying, merging (0 on wasm32).
    pub ms: [f64; 3],
}

/// The tolerances of one run, scaled to the drawing's size.
#[derive(Debug, Clone, Copy)]
struct Tols {
    /// A point lies on a curve (mm).
    on: f64,
    /// Two carriers are one (mm).
    merge: f64,
    /// A piece shorter than this takes its neighbour's visibility (mm).
    tiny: f64,
    /// Newton converged (mm).
    newton: f64,
}

#[derive(Debug, Clone, Copy)]
enum Geo {
    Line {
        p: Point,
        d: Vec2,
    },
    /// `c + cos t m1 + sin t m2`; `flat` is the unit direction of the
    /// segment it covers when the view runs along its plane.
    Conic {
        c: Point,
        m1: Vec2,
        m2: Vec2,
        flat: Option<Vec2>,
    },
    General,
}

struct Item {
    curve: Curve3,
    range: [f64; 2],
    owner: usize,
    kind: CurveKind,
    geo: Geo,
    /// Flattened copy: parameters and 2D points, only to find starting
    /// points and candidates.
    samples: Vec<(f64, Point)>,
    /// How far the flattened copy may be from the curve.
    dev: f64,
    lo: Point,
    hi: Point,
    splits: Vec<f64>,
}

fn perp(v: Vec2) -> Vec2 {
    Vec2::new(-v.y, v.x)
}

impl Item {
    fn eval(&self, view: &ViewFrame, t: f64) -> Point {
        match self.geo {
            Geo::Line { p, d } => p + d * t,
            Geo::Conic { c, m1, m2, .. } => {
                let (s, co) = t.sin_cos();
                c + m1 * co + m2 * s
            }
            Geo::General => view.point(self.curve.point(t)),
        }
    }

    fn deriv(&self, view: &ViewFrame, t: f64) -> Vec2 {
        match self.geo {
            Geo::Line { d, .. } => d,
            Geo::Conic { m1, m2, .. } => {
                let (s, co) = t.sin_cos();
                m2 * co - m1 * s
            }
            Geo::General => view.vector(self.curve.deriv(t)),
        }
    }

    /// The carrier line when the curve is straight in the view.
    fn line(&self) -> Option<(Point, Vec2)> {
        match self.geo {
            Geo::Line { p, d } => Some((p, d.normalize())),
            Geo::Conic { c, flat, .. } => flat.map(|w| (c, w)),
            Geo::General => None,
        }
    }

    fn eps(&self) -> f64 {
        1e-12 * (self.range[1] - self.range[0]).abs().max(1.0)
    }

    fn in_range(&self, t: f64) -> bool {
        t >= self.range[0] - self.eps() && t <= self.range[1] + self.eps()
    }

    /// Every `t + 2 pi k` in range.
    fn periodic_in_range(&self, t: f64, out: &mut Vec<f64>) {
        let k0 = ((self.range[0] - t) / TAU).floor() as i64 - 1;
        for k in k0..k0 + 4 {
            let s = t + TAU * k as f64;
            if self.in_range(s) {
                out.push(s.clamp(self.range[0], self.range[1]));
            }
        }
    }

    /// The parameters where the curve meets the line through `q` along the
    /// unit `w`.
    fn roots_on_line(&self, view: &ViewFrame, q: Point, w: Vec2) -> Vec<f64> {
        let n = perp(w);
        let mut out = Vec::new();
        match self.geo {
            Geo::Line { p, d } => {
                let den = n.dot(d);
                if den.abs() > 1e-14 * d.hypot() {
                    let t = -n.dot(p - q) / den;
                    if self.in_range(t) {
                        out.push(t.clamp(self.range[0], self.range[1]));
                    }
                }
            }
            Geo::Conic { c, m1, m2, .. } => {
                let (a, b, cc) = (n.dot(m1), n.dot(m2), n.dot(c - q));
                let r = a.hypot(b);
                if r > 1e-14 * m1.hypot().max(m2.hypot()) {
                    let x = -cc / r;
                    if x.abs() <= 1.0 + 1e-12 {
                        let phi = b.atan2(a);
                        // At |x| = 1 the line touches the conic (or meets a
                        // conic seen edge on at its fold), where the root
                        // is ill-conditioned: acos(1 - 1e-16) is 1.5e-8.
                        // Within 1e-12 of it the root is the touching
                        // point itself.
                        let delta = if x.abs() >= 1.0 - 1e-12 {
                            if x > 0.0 {
                                0.0
                            } else {
                                PI
                            }
                        } else {
                            x.acos()
                        };
                        self.periodic_in_range(phi + delta, &mut out);
                        if delta > 0.0 && delta < PI {
                            self.periodic_in_range(phi - delta, &mut out);
                        }
                    }
                }
            }
            Geo::General => {
                let f = |t: f64| n.dot(self.eval(view, t) - q);
                for w2 in self.samples.windows(2) {
                    let (a, b) = (w2[0].0, w2[1].0);
                    let (fa, fb) = (f(a), f(b));
                    if fa == 0.0 {
                        out.push(a);
                    } else if fa * fb < 0.0 {
                        out.push(bisect_root(&f, a, b, fa));
                    }
                }
                if let Some(&(t, _)) = self.samples.last() {
                    if f(t) == 0.0 {
                        out.push(t);
                    }
                }
            }
        }
        out
    }

    /// The parameters where the curve passes within `tol` of `e`.
    fn params_at(&self, view: &ViewFrame, e: Point, tol: f64) -> Vec<f64> {
        if e.x < self.lo.x - tol
            || e.y < self.lo.y - tol
            || e.x > self.hi.x + tol
            || e.y > self.hi.y + tol
        {
            return Vec::new();
        }
        let mut out = Vec::new();
        match (self.line(), self.geo) {
            (Some((_, w)), _) => {
                out = self.roots_on_line(view, e, perp(w));
            }
            (None, Geo::Conic { c, m1, m2, .. }) => {
                let det = m1.cross(m2);
                if det.abs() > 0.0 {
                    let v = e - c;
                    // (cos t, sin t) = M^-1 v, M = [m1 m2].
                    let ct = (v.x * m2.y - v.y * m2.x) / det;
                    let st = (m1.x * v.y - m1.y * v.x) / det;
                    self.periodic_in_range(st.atan2(ct), &mut out);
                }
            }
            _ => {
                // Nearest sample, then Newton on (X(t) - e) . X'(t) = 0.
                let mut best: Option<(f64, f64)> = None;
                for &(t, p) in &self.samples {
                    let d = (p - e).hypot();
                    if best.is_none_or(|(_, bd)| d < bd) {
                        best = Some((t, d));
                    }
                }
                if let Some((mut t, _)) = best {
                    for _ in 0..30 {
                        let x = self.eval(view, t) - e;
                        let d1 = self.deriv(view, t);
                        let g = x.dot(d1);
                        let h = d1.dot(d1);
                        if h <= 0.0 {
                            break;
                        }
                        let step = g / h;
                        t = (t - step).clamp(self.range[0], self.range[1]);
                        if step.abs() <= 1e-15 * (1.0 + t.abs()) {
                            break;
                        }
                    }
                    out.push(t);
                }
            }
        }
        out.retain(|&t| (self.eval(view, t) - e).hypot() <= tol);
        out
    }
}

fn bisect_root(f: &dyn Fn(f64) -> f64, mut a: f64, mut b: f64, mut fa: f64) -> f64 {
    for _ in 0..80 {
        let m = 0.5 * (a + b);
        if m <= a || m >= b {
            break;
        }
        let fm = f(m);
        if fm == 0.0 {
            return m;
        }
        if (fm < 0.0) == (fa < 0.0) {
            a = m;
            fa = fm;
        } else {
            b = m;
        }
    }
    0.5 * (a + b)
}

/// The closest points of two segments, as fractions along each.
fn seg_closest(p0: Point, p1: Point, q0: Point, q1: Point) -> (f64, f64, f64) {
    let d1 = p1 - p0;
    let d2 = q1 - q0;
    let r = p0 - q0;
    let (a, e, f) = (d1.dot(d1), d2.dot(d2), d2.dot(r));
    let (mut s, mut t);
    if a <= 1e-300 && e <= 1e-300 {
        return (0.0, 0.0, r.hypot());
    }
    if a <= 1e-300 {
        s = 0.0;
        t = (f / e).clamp(0.0, 1.0);
    } else {
        let c = d1.dot(r);
        if e <= 1e-300 {
            t = 0.0;
            s = (-c / a).clamp(0.0, 1.0);
        } else {
            let b = d1.dot(d2);
            let den = a * e - b * b;
            s = if den > 0.0 {
                ((b * f - c * e) / den).clamp(0.0, 1.0)
            } else {
                0.0
            };
            t = (b * s + f) / e;
            if t < 0.0 {
                t = 0.0;
                s = (-c / a).clamp(0.0, 1.0);
            } else if t > 1.0 {
                t = 1.0;
                s = ((b - c) / a).clamp(0.0, 1.0);
            }
        }
    }
    let d = (p0 + d1 * s) - (q0 + d2 * t);
    (s, t, d.hypot())
}

/// Builds the projected item of a source, or None when it is a point.
fn item(view: &ViewFrame, s: Source) -> Option<Item> {
    let [t0, t1] = s.range;
    if t1 <= t0 || !t0.is_finite() || !t1.is_finite() {
        return None;
    }
    let geo = match &s.curve {
        Curve3::Line { origin, dir } => {
            let d = view.vector(*dir);
            if d.hypot() * (t1 - t0) <= 1e-12 * (t1 - t0).max(1.0) || d.hypot() <= 1e-12 {
                return None;
            }
            Geo::Line {
                p: view.point(*origin),
                d,
            }
        }
        Curve3::Circle {
            center,
            x,
            y,
            radius,
        } => conic(
            view.point(*center),
            view.vector(x.scale(*radius)),
            view.vector(y.scale(*radius)),
        )?,
        Curve3::Ellipse { center, x, y, a, b } => conic(
            view.point(*center),
            view.vector(x.scale(*a)),
            view.vector(y.scale(*b)),
        )?,
        Curve3::Nurbs { .. } => Geo::General,
    };
    let mut it = Item {
        curve: s.curve,
        range: s.range,
        owner: s.owner,
        kind: s.kind,
        geo,
        samples: Vec::new(),
        dev: 0.0,
        lo: Point::ORIGIN,
        hi: Point::ORIGIN,
        splits: Vec::new(),
    };
    let n = match geo {
        Geo::Line { .. } => 1,
        Geo::Conic { .. } => ((128.0 * (t1 - t0) / TAU).ceil() as usize).clamp(8, 512),
        Geo::General => 256,
    };
    it.samples = (0..=n)
        .map(|i| {
            let t = if i == n {
                t1
            } else {
                t0 + (t1 - t0) * i as f64 / n as f64
            };
            (t, it.eval(view, t))
        })
        .collect();
    it.dev = match geo {
        Geo::Line { .. } => 0.0,
        Geo::Conic { m1, m2, .. } => {
            let step = (t1 - t0) / n as f64;
            (m1.hypot() + m2.hypot()) * (1.0 - (0.5 * step).cos()) * 1.01
        }
        Geo::General => {
            let mut dev: f64 = 0.0;
            for w in it.samples.windows(2) {
                let mid = it.eval(view, 0.5 * (w[0].0 + w[1].0));
                let (_, _, d) = seg_closest(mid, mid, w[0].1, w[1].1);
                dev = dev.max(d);
            }
            dev * 4.0
        }
    };
    let (mut lo, mut hi) = (it.samples[0].1, it.samples[0].1);
    for &(_, p) in &it.samples {
        lo = Point::new(lo.x.min(p.x), lo.y.min(p.y));
        hi = Point::new(hi.x.max(p.x), hi.y.max(p.y));
    }
    let pad = it.dev;
    it.lo = Point::new(lo.x - pad, lo.y - pad);
    it.hi = Point::new(hi.x + pad, hi.y + pad);
    Some(it)
}

fn conic(c: Point, m1: Vec2, m2: Vec2) -> Option<Geo> {
    let scale = m1.hypot().max(m2.hypot());
    if scale <= 1e-12 {
        return None;
    }
    let area = m1.cross(m2).abs();
    let flat = if area <= 1e-12 * scale * scale {
        // Seen edge on: both columns along one direction.
        let w = if m1.hypot() >= m2.hypot() { m1 } else { m2 };
        Some(w.normalize())
    } else {
        None
    };
    Some(Geo::Conic { c, m1, m2, flat })
}

/// Crossings of two items: parameter pairs.
fn crossings(view: &ViewFrame, a: &Item, b: &Item, tols: &Tols) -> Vec<(f64, f64)> {
    let mut out = Vec::new();
    let (line_item, other, swapped) = match (a.line(), b.line()) {
        (Some(_), _) => (a, b, false),
        (None, Some(_)) => (b, a, true),
        (None, None) => {
            // Two pieces of one conic share every point: no crossing, only
            // their end points (found separately) split them.
            if let (Some(ca), Some(cb)) = (carrier_of(a), carrier_of(b)) {
                if ca.same(&cb, tols.merge) {
                    return out;
                }
            }
            return numeric_crossings(view, a, b, tols);
        }
    };
    let (q, w) = line_item.line().expect("straight");
    for tb in other.roots_on_line(view, q, w) {
        let pt = other.eval(view, tb);
        for ta in line_item.params_at(view, pt, tols.on) {
            out.push(if swapped { (tb, ta) } else { (ta, tb) });
        }
    }
    out
}

/// Crossings of two curves neither of which is straight in the view:
/// candidates where their flattened copies come within their deviations,
/// then Newton on `A(s) = B(t)`.
fn numeric_crossings(view: &ViewFrame, a: &Item, b: &Item, tols: &Tols) -> Vec<(f64, f64)> {
    let reach = a.dev + b.dev + tols.on;
    let near = |p: Point, q: Point, lo: Point, hi: Point| {
        p.x.max(q.x) >= lo.x - reach
            && p.x.min(q.x) <= hi.x + reach
            && p.y.max(q.y) >= lo.y - reach
            && p.y.min(q.y) <= hi.y + reach
    };
    let sa: Vec<usize> = (0..a.samples.len() - 1)
        .filter(|&i| near(a.samples[i].1, a.samples[i + 1].1, b.lo, b.hi))
        .collect();
    let sb: Vec<usize> = (0..b.samples.len() - 1)
        .filter(|&j| near(b.samples[j].1, b.samples[j + 1].1, a.lo, a.hi))
        .collect();
    let mut out: Vec<(f64, f64)> = Vec::new();
    for &i in &sa {
        let (a0, a1) = (a.samples[i], a.samples[i + 1]);
        for &j in &sb {
            let (b0, b1) = (b.samples[j], b.samples[j + 1]);
            let (fs, ft, d) = seg_closest(a0.1, a1.1, b0.1, b1.1);
            if d > reach {
                continue;
            }
            let mut s = a0.0 + (a1.0 - a0.0) * fs;
            let mut t = b0.0 + (b1.0 - b0.0) * ft;
            let mut ok = false;
            for _ in 0..40 {
                let f = a.eval(view, s) - b.eval(view, t);
                if f.hypot() <= tols.newton {
                    ok = true;
                    break;
                }
                let (da, db) = (a.deriv(view, s), b.deriv(view, t));
                // [da, -db] [ds, dt]^T = -f
                let det = -da.x * db.y + db.x * da.y;
                if det.abs() <= 1e-300 {
                    break;
                }
                let ds = (-f.x * -db.y - (-f.y) * -db.x) / det;
                let dt = (da.x * -f.y - da.y * -f.x) / det;
                s += ds;
                t += dt;
                if !s.is_finite() || !t.is_finite() {
                    break;
                }
            }
            // A touch or a stretch where the curves coincide is not a
            // crossing: visibility does not change there (shared end
            // points are split separately).
            let (da, db) = (a.deriv(view, s), b.deriv(view, t));
            let crossing = da.cross(db).abs() > 1e-6 * da.hypot() * db.hypot();
            if ok && crossing && a.in_range(s) && b.in_range(t) {
                let s = s.clamp(a.range[0], a.range[1]);
                let t = t.clamp(b.range[0], b.range[1]);
                if !out
                    .iter()
                    .any(|&(x, y)| (x - s).abs() <= 1e3 * a.eps() && (y - t).abs() <= 1e3 * b.eps())
                {
                    out.push((s, t));
                }
            }
        }
    }
    out
}

/// Runs hidden-line removal. `oracle` decides visibility; `opts` what to
/// keep. Deterministic: the output depends only on the input order.
pub fn run(
    view: &ViewFrame,
    sources: Vec<Source>,
    oracle: &mut dyn Oracle,
    opts: &Options,
) -> (Vec<Drawn>, Stats, Vec<Traced>) {
    let mut stats = Stats::default();
    let mut items: Vec<Item> = sources.into_iter().filter_map(|s| item(view, s)).collect();
    stats.curves = items.len();
    if items.is_empty() {
        return (Vec::new(), stats, Vec::new());
    }
    let (mut lo, mut hi) = (items[0].lo, items[0].hi);
    for it in &items {
        lo = Point::new(lo.x.min(it.lo.x), lo.y.min(it.lo.y));
        hi = Point::new(hi.x.max(it.hi.x), hi.y.max(it.hi.y));
    }
    let extent = (hi - lo).hypot().max(1.0);
    let s = (extent / 100.0).max(1.0);
    let tols = Tols {
        on: 1e-5 * s,
        merge: 1e-6 * s,
        tiny: 1e-6 * s,
        newton: 1e-11 * s,
    };

    let t_split = now();
    // ── 2. Split points ─────────────────────────────────────────────
    let mut order: Vec<usize> = (0..items.len()).collect();
    order.sort_by(|&i, &j| items[i].lo.x.total_cmp(&items[j].lo.x).then(i.cmp(&j)));
    let mut found: Vec<(usize, f64)> = Vec::new();
    for (k, &i) in order.iter().enumerate() {
        for &j in &order[k + 1..] {
            if items[j].lo.x > items[i].hi.x + tols.on {
                break;
            }
            let (a, b) = (&items[i], &items[j]);
            if a.lo.y > b.hi.y + tols.on || b.lo.y > a.hi.y + tols.on {
                continue;
            }
            for (ta, tb) in crossings(view, a, b, &tols) {
                found.push((i, ta));
                found.push((j, tb));
            }
            for (x, y) in [(i, j), (j, i)] {
                let (ex, ey) = (&items[x], &items[y]);
                for t in [ey.range[0], ey.range[1]] {
                    let e = ey.eval(view, t);
                    for tx in ex.params_at(view, e, tols.on) {
                        found.push((x, tx));
                    }
                }
            }
        }
    }
    for (i, it) in items.iter().enumerate() {
        if let Geo::Conic {
            m1,
            m2,
            flat: Some(w),
            ..
        } = it.geo
        {
            let base = w.dot(m2).atan2(w.dot(m1));
            let mut folds = Vec::new();
            it.periodic_in_range(base, &mut folds);
            it.periodic_in_range(base + PI, &mut folds);
            found.extend(folds.into_iter().map(|t| (i, t)));
        }
    }
    for (i, t) in found {
        items[i].splits.push(t);
    }

    stats.ms[0] = since(t_split);
    debug(&format!(
        "{} curves, crossings and end points found in {:.0} ms",
        stats.curves, stats.ms[0]
    ));
    let t_classify = now();
    // ── 3. Pieces and their visibility ──────────────────────────────
    let mut pieces: Vec<Piece> = Vec::new();
    for (ii, it) in items.iter_mut().enumerate() {
        let [t0, t1] = it.range;
        let eps = 1e-10 * (t1 - t0).max(1e-300);
        let mut ts: Vec<f64> = it
            .splits
            .drain(..)
            .filter(|&t| t > t0 + eps && t < t1 - eps)
            .collect();
        ts.sort_by(f64::total_cmp);
        ts.dedup_by(|a, b| (*a - *b).abs() <= eps);
        stats.splits += ts.len();
        let mut bounds = Vec::with_capacity(ts.len() + 2);
        bounds.push(t0);
        bounds.extend(ts);
        bounds.push(t1);
        for w in bounds.windows(2) {
            pieces.push(Piece {
                item: ii,
                t: [w[0], w[1]],
                hidden: None,
                asked: false,
            });
        }
    }
    let items = items;
    let mut classified: Vec<Piece> = Vec::with_capacity(pieces.len());
    let mut i = 0;
    while i < pieces.len() {
        // The pieces of one item, in order.
        let item_id = pieces[i].item;
        let mut j = i;
        while j < pieces.len() && pieces[j].item == item_id {
            j += 1;
        }
        let it = &items[item_id];
        let mut run: Vec<Piece> = Vec::new();
        for p in &pieces[i..j] {
            let len = piece_length(view, it, p.t);
            if len <= tols.tiny {
                run.push(p.clone());
                continue;
            }
            stats.pieces += 1;
            classify(
                view, it, item_id, p.t, len, extent, oracle, &mut stats, &mut run,
            );
        }
        // Tiny pieces take the visibility of a neighbour.
        for k in 0..run.len() {
            if run[k].hidden.is_none() {
                let prev = run[..k].iter().rev().find_map(|p| p.hidden);
                let next = run[k + 1..].iter().find_map(|p| p.hidden);
                run[k].hidden = Some(prev.or(next).unwrap_or(false));
            }
        }
        classified.extend(run);
        i = j;
    }

    stats.ms[1] = since(t_classify);
    debug(&format!(
        "classified {} pieces, {} queries, {} bisected in {:.0} ms",
        stats.pieces, stats.queries, stats.bisected, stats.ms[1]
    ));
    let t_merge = now();
    // ── 4 and 5. Merge on carriers, emit ────────────────────────────
    let out = merge_and_emit(view, &items, &classified, &tols, opts, &mut stats);
    stats.ms[2] = since(t_merge);
    let traced = if opts.trace {
        classified
            .iter()
            .map(|p| Traced {
                curve: items[p.item].curve.clone(),
                t: p.t,
                hidden: p.hidden.unwrap_or(false),
                asked: p.asked,
                owner: items[p.item].owner,
                kind: items[p.item].kind,
            })
            .collect()
    } else {
        Vec::new()
    };
    (out, stats, traced)
}

/// A classified piece before merging: the exact curve, its parameter
/// range, and what the oracle said.
#[derive(Debug, Clone)]
pub struct Traced {
    pub curve: Curve3,
    pub t: [f64; 2],
    pub hidden: bool,
    /// The oracle classified it; false for a piece shorter than the
    /// tolerance, which takes a neighbour's class.
    pub asked: bool,
    pub owner: usize,
    pub kind: CurveKind,
}

#[derive(Debug, Clone)]
struct Piece {
    item: usize,
    t: [f64; 2],
    hidden: Option<bool>,
    /// The oracle classified it (a tiny piece takes a neighbour's class).
    asked: bool,
}

fn piece_length(view: &ViewFrame, it: &Item, t: [f64; 2]) -> f64 {
    let n = 8;
    let mut len = 0.0;
    let mut prev = it.eval(view, t[0]);
    for k in 1..=n {
        let p = it.eval(view, t[0] + (t[1] - t[0]) * k as f64 / n as f64);
        len += (p - prev).hypot();
        prev = p;
    }
    len
}

/// Classifies one piece, splitting it where its samples disagree.
#[allow(clippy::too_many_arguments)]
fn classify(
    view: &ViewFrame,
    it: &Item,
    item: usize,
    t: [f64; 2],
    len: f64,
    extent: f64,
    oracle: &mut dyn Oracle,
    stats: &mut Stats,
    out: &mut Vec<Piece>,
) {
    let mut ask = |u: f64, stats: &mut Stats| {
        stats.queries += 1;
        oracle.hidden(it.curve.point(u), it.owner)
    };
    let fr: &[f64] = if len > extent / 16.0 {
        &[0.25, 0.5, 0.75]
    } else {
        &[0.5]
    };
    let at = |f: f64| t[0] + (t[1] - t[0]) * f;
    let answers: Vec<(f64, bool)> = fr.iter().map(|&f| (at(f), ask(at(f), stats))).collect();
    if answers.iter().all(|a| a.1 == answers[0].1) {
        out.push(Piece {
            item,
            t,
            hidden: Some(answers[0].1),
            asked: true,
        });
        return;
    }
    // A crossing was missed: find each change by bisection.
    stats.bisected += 1;
    let mut cuts = vec![t[0]];
    let mut classes = vec![answers[0].1];
    for w in answers.windows(2) {
        if w[0].1 != w[1].1 {
            let (mut a, mut b) = (w[0].0, w[1].0);
            for _ in 0..50 {
                let m = 0.5 * (a + b);
                if (it.eval(view, a) - it.eval(view, b)).hypot() <= 1e-7 || m <= a || m >= b {
                    break;
                }
                if ask(m, stats) == w[0].1 {
                    a = m;
                } else {
                    b = m;
                }
            }
            cuts.push(0.5 * (a + b));
            classes.push(w[1].1);
        }
    }
    cuts.push(t[1]);
    for (k, h) in classes.iter().enumerate() {
        out.push(Piece {
            item,
            t: [cuts[k], cuts[k + 1]],
            hidden: Some(*h),
            asked: true,
        });
    }
}

// ── Carriers ────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy)]
enum Carrier {
    /// Points `base + s w`, `w` unit.
    Line { base: Point, w: Vec2 },
    /// `center + radius (cos s, sin s)`.
    Circle { center: Point, radius: f64 },
    /// `center + a cos s u + b sin s perp(u)`, `u` unit, `a >= b`.
    Ellipse {
        center: Point,
        u: Vec2,
        a: f64,
        b: f64,
    },
}

impl Carrier {
    fn periodic(&self) -> bool {
        !matches!(self, Carrier::Line { .. })
    }

    /// The carrier parameter of a point on it.
    fn param(&self, p: Point) -> f64 {
        match *self {
            Carrier::Line { base, w } => (p - base).dot(w),
            Carrier::Circle { center, .. } => {
                let v = p - center;
                v.y.atan2(v.x).rem_euclid(TAU)
            }
            Carrier::Ellipse { center, u, a, b } => {
                let v = p - center;
                (v.dot(perp(u)) / b).atan2(v.dot(u) / a).rem_euclid(TAU)
            }
        }
    }

    /// The carrier length per unit parameter (for tolerances).
    fn scale(&self) -> f64 {
        match *self {
            Carrier::Line { .. } => 1.0,
            Carrier::Circle { radius, .. } => radius,
            Carrier::Ellipse { a, .. } => a,
        }
    }

    /// How far a point is from the carrier (mm; for an ellipse, from the
    /// point at the same parameter, which is within a factor a / b of it).
    fn dist(&self, p: Point) -> f64 {
        match *self {
            Carrier::Line { base, w } => perp(w).dot(p - base).abs(),
            Carrier::Circle { center, radius } => ((p - center).hypot() - radius).abs(),
            Carrier::Ellipse { center, u, a, b } => {
                let s = self.param(p);
                let q = center + u * (a * s.cos()) + perp(u) * (b * s.sin());
                (p - q).hypot()
            }
        }
    }

    fn same(&self, o: &Carrier, tol: f64) -> bool {
        match (*self, *o) {
            (Carrier::Line { base, w }, Carrier::Line { base: b2, w: w2 }) => {
                w.cross(w2).abs() <= 1e-12 && perp(w).dot(b2 - base).abs() <= tol
            }
            (
                Carrier::Circle { center, radius },
                Carrier::Circle {
                    center: c2,
                    radius: r2,
                },
            ) => (center - c2).hypot() <= tol && (radius - r2).abs() <= tol,
            (
                Carrier::Ellipse { center, u, a, b },
                Carrier::Ellipse {
                    center: c2,
                    u: u2,
                    a: a2,
                    b: b2,
                },
            ) => {
                (center - c2).hypot() <= tol
                    && (a - a2).abs() <= tol
                    && (b - b2).abs() <= tol
                    && u.cross(u2).abs() * a <= tol
            }
            _ => false,
        }
    }

    /// A sort key that keeps equal carriers close.
    fn key(&self) -> (u8, f64) {
        match *self {
            Carrier::Line { base, w } => (0, perp(w).dot(base.to_vec2())),
            Carrier::Circle { center, .. } => (1, center.x),
            Carrier::Ellipse { center, .. } => (2, center.x),
        }
    }
}

/// The carrier of an item, if it has one.
fn carrier_of(it: &Item) -> Option<Carrier> {
    match it.geo {
        Geo::Line { .. } | Geo::Conic { flat: Some(_), .. } => {
            let (q, w) = it.line()?;
            // One direction per line: pointing up, or right when level.
            let w = if w.y < 0.0 || (w.y == 0.0 && w.x < 0.0) {
                -w
            } else {
                w
            };
            let n = perp(w);
            let base = Point::ORIGIN + n * n.dot(q.to_vec2());
            Some(Carrier::Line { base, w })
        }
        Geo::Conic { c, m1, m2, .. } => {
            let (u, a, b) = principal_axes(m1, m2);
            if (a - b).abs() <= 1e-12 * a {
                Some(Carrier::Circle {
                    center: c,
                    radius: 0.5 * (a + b),
                })
            } else {
                // One major axis per ellipse: pointing up, or right.
                let u = if u.y < 0.0 || (u.y == 0.0 && u.x < 0.0) {
                    -u
                } else {
                    u
                };
                Some(Carrier::Ellipse { center: c, u, a, b })
            }
        }
        Geo::General => None,
    }
}

/// The principal semi-axes of the ellipse `cos t m1 + sin t m2`: unit major
/// direction, major and minor semi-axis.
pub fn principal_axes(m1: Vec2, m2: Vec2) -> (Vec2, f64, f64) {
    // Extremes of |cos t m1 + sin t m2|^2 = p cos^2 + 2 q cos sin + r sin^2.
    let (p, q, r) = (m1.dot(m1), m1.dot(m2), m2.dot(m2));
    let t = 0.5 * (2.0 * q).atan2(p - r);
    let (s, c) = t.sin_cos();
    let major = m1 * c + m2 * s;
    let minor = m2 * c - m1 * s;
    let (a, b) = (major.hypot(), minor.hypot());
    if a >= b {
        (major / a, a, b)
    } else {
        (minor / b, b, a)
    }
}

/// One piece laid on its carrier: `[lo, hi]` with `lo <= hi` (periodic
/// carriers: `lo` in `[0, 2 pi)`, `hi` up to `lo + 2 pi`).
#[derive(Debug, Clone, Copy)]
struct Span {
    lo: f64,
    hi: f64,
    hidden: bool,
    owner: usize,
    kind: CurveKind,
}

fn merge_and_emit(
    view: &ViewFrame,
    items: &[Item],
    pieces: &[Piece],
    tols: &Tols,
    opts: &Options,
    stats: &mut Stats,
) -> Vec<Drawn> {
    let carriers: Vec<Option<Carrier>> = items.iter().map(carrier_of).collect();
    // Cluster equal carriers (sorted by a key, then compared to their
    // neighbours within the tolerance).
    let mut with: Vec<usize> = (0..items.len())
        .filter(|&i| carriers[i].is_some())
        .collect();
    with.sort_by(|&i, &j| {
        let (ki, kj) = (carriers[i].unwrap().key(), carriers[j].unwrap().key());
        ki.0.cmp(&kj.0).then(ki.1.total_cmp(&kj.1)).then(i.cmp(&j))
    });
    let mut cluster_of = vec![usize::MAX; items.len()];
    let mut reps: Vec<Carrier> = Vec::new();
    for (k, &i) in with.iter().enumerate() {
        if cluster_of[i] != usize::MAX {
            continue;
        }
        let c = carriers[i].unwrap();
        let id = reps.len();
        reps.push(c);
        cluster_of[i] = id;
        for &j in &with[k + 1..] {
            let cj = carriers[j].unwrap();
            if cj.key().0 != c.key().0 || cj.key().1 > c.key().1 + tols.merge * 2.0 + 1e-9 {
                break;
            }
            if cluster_of[j] == usize::MAX && c.same(&cj, tols.merge) {
                cluster_of[j] = id;
            }
        }
    }
    let mut spans: Vec<Vec<Span>> = vec![Vec::new(); reps.len()];
    let mut out: Vec<Drawn> = Vec::new();
    for p in pieces {
        let ii = p.item;
        let it = &items[ii];
        let hidden = p.hidden.unwrap_or(false);
        // A general curve that lies on a line, circle or ellipse of the view
        // (a curve on a cylinder seen along its axis lies on its circle) is
        // drawn as that conic.
        let on_carrier = || {
            let pts: Vec<Point> = (0..=8)
                .map(|k| it.eval(view, p.t[0] + (p.t[1] - p.t[0]) * k as f64 / 8.0))
                .collect();
            reps.iter()
                .position(|c| pts.iter().all(|&q| c.dist(q) <= tols.on))
        };
        let ci = match carriers[ii] {
            Some(_) => cluster_of[ii],
            None => match on_carrier() {
                Some(ci) => ci,
                None => {
                    // No carrier: a fitted spline per piece.
                    if hidden && !opts.hidden {
                        continue;
                    }
                    if let Some(shape) = fit_spline(view, it, p.t, opts.fit_tol) {
                        stats.fitted += 1;
                        out.push(Drawn {
                            shape,
                            hidden,
                            owner: it.owner,
                            kind: it.kind,
                        });
                    }
                    continue;
                }
            },
        };
        let c = reps[ci];
        let (pa, pb) = (it.eval(view, p.t[0]), it.eval(view, p.t[1]));
        let (sa, sb) = (c.param(pa), c.param(pb));
        let closed_general = carriers[ii].is_none()
            && (pa - pb).hypot() <= tols.on
            && piece_length(view, it, p.t) > tols.tiny;
        let full = c.periodic() && ((p.t[1] - p.t[0]) >= TAU - 1e-12 || closed_general);
        let (lo, hi) = if full {
            (0.0, TAU)
        } else if c.periodic() {
            // Which way the piece runs on the carrier.
            let mid = c.param(it.eval(view, 0.5 * (p.t[0] + p.t[1])));
            let ccw = (mid - sa).rem_euclid(TAU) <= (sb - sa).rem_euclid(TAU);
            if ccw {
                (sa, sa + (sb - sa).rem_euclid(TAU))
            } else {
                (sb, sb + (sa - sb).rem_euclid(TAU))
            }
        } else {
            let mut lo = sa.min(sb);
            let mut hi = sa.max(sb);
            if carriers[ii].is_none() {
                for k in 1..8 {
                    let s = c.param(it.eval(view, p.t[0] + (p.t[1] - p.t[0]) * k as f64 / 8.0));
                    lo = lo.min(s);
                    hi = hi.max(s);
                }
            }
            (lo, hi)
        };
        spans[ci].push(Span {
            lo,
            hi,
            hidden,
            owner: it.owner,
            kind: it.kind,
        });
    }
    let mut carried: Vec<Drawn> = Vec::new();
    for (ci, list) in spans.iter().enumerate() {
        let c = reps[ci];
        let tol = tols.merge / c.scale();
        for run in elementary_runs(&c, list, tol) {
            if run.hidden && !opts.hidden {
                continue;
            }
            if let Some(shape) = emit(&c, run.lo, run.hi, tols.tiny) {
                carried.push(Drawn {
                    shape,
                    hidden: run.hidden,
                    owner: run.owner,
                    kind: run.kind,
                });
            }
        }
    }
    carried.extend(out);
    carried
}

/// The runs of one carrier: elementary intervals between all span ends,
/// each taking the best class over it, joined while class, owner and kind
/// stay the same.
fn elementary_runs(c: &Carrier, spans: &[Span], tol: f64) -> Vec<Span> {
    let periodic = c.periodic();
    // Periodic spans split at 2 pi into [0, 2 pi].
    let mut parts: Vec<Span> = Vec::new();
    for s in spans {
        if periodic && s.hi > TAU {
            parts.push(Span { hi: TAU, ..*s });
            parts.push(Span {
                lo: 0.0,
                hi: s.hi - TAU,
                ..*s
            });
        } else {
            parts.push(*s);
        }
    }
    let mut cuts: Vec<f64> = parts.iter().flat_map(|s| [s.lo, s.hi]).collect();
    if periodic {
        cuts.push(0.0);
        cuts.push(TAU);
    }
    cuts.sort_by(f64::total_cmp);
    cuts.dedup_by(|a, b| (*a - *b).abs() <= tol);
    let mut runs: Vec<Span> = Vec::new();
    for w in cuts.windows(2) {
        let (lo, hi) = (w[0], w[1]);
        if hi - lo <= tol {
            continue;
        }
        let m = 0.5 * (lo + hi);
        let mut best: Option<Span> = None;
        for s in &parts {
            if s.lo - tol <= m && m <= s.hi + tol {
                let better = match best {
                    None => true,
                    Some(b) => (s.hidden, s.owner, s.kind) < (b.hidden, b.owner, b.kind),
                };
                if better {
                    best = Some(*s);
                }
            }
        }
        let Some(b) = best else { continue };
        let span = Span { lo, hi, ..b };
        match runs.last_mut() {
            Some(r)
                if (r.hi - lo).abs() <= tol
                    && (r.hidden, r.owner, r.kind) == (span.hidden, span.owner, span.kind) =>
            {
                r.hi = hi
            }
            _ => runs.push(span),
        }
    }
    // A periodic run ending at 2 pi continues the one starting at 0.
    if periodic && runs.len() >= 2 {
        let (first, last) = (runs[0], runs[runs.len() - 1]);
        if first.lo <= tol
            && (last.hi - TAU).abs() <= tol
            && (first.hidden, first.owner, first.kind) == (last.hidden, last.owner, last.kind)
        {
            runs.remove(0);
            let l = runs.last_mut().expect("two runs");
            l.hi = TAU + first.hi;
        }
    }
    runs
}

fn emit(c: &Carrier, lo: f64, hi: f64, tiny: f64) -> Option<Shape> {
    let full = c.periodic() && hi - lo >= TAU - 1e-12;
    match *c {
        Carrier::Line { base, w } => {
            if (hi - lo) <= tiny {
                return None;
            }
            Some(Shape::Line(kurbo::Line::new(base + w * lo, base + w * hi)))
        }
        Carrier::Circle { center, radius } => {
            if (hi - lo) * radius <= tiny {
                return None;
            }
            if full {
                Some(Shape::Circle(kurbo::Circle::new(center, radius)))
            } else {
                Some(Shape::Arc {
                    center,
                    radius,
                    start_angle: lo,
                    end_angle: if hi > TAU { hi - TAU } else { hi },
                })
            }
        }
        Carrier::Ellipse { center, u, a, b } => {
            if (hi - lo) * a <= tiny {
                return None;
            }
            let (start, end) = if full {
                (0.0, TAU)
            } else if hi > TAU {
                (lo, hi - TAU)
            } else {
                (lo, hi)
            };
            Some(Shape::Ellipse {
                center,
                major_axis: (u.x * a, u.y * a),
                minor_ratio: b / a,
                start_param: start,
                end_param: end,
            })
        }
    }
}

/// A cubic spline within `tol` of the projected curve over `t`: Hermite
/// pieces (end points and tangents exact), halved until the curve's
/// points at a quarter, a half and three quarters of each piece lie within
/// `tol`, written as a B-spline whose interior knots are triple (a chain
/// of Bezier pieces).
fn fit_spline(view: &ViewFrame, it: &Item, t: [f64; 2], tol: f64) -> Option<Shape> {
    let mut pieces: Vec<[f64; 2]> = Vec::new();
    let mut todo = vec![(t, 0usize)];
    while let Some((r, depth)) = todo.pop() {
        let ok = depth >= 14 || {
            let bez = hermite(view, it, r);
            [0.25, 0.5, 0.75].iter().all(|&f| {
                let q = it.eval(view, r[0] + (r[1] - r[0]) * f);
                (bezier(&bez, f) - q).hypot() <= tol
            })
        };
        if ok {
            pieces.push(r);
        } else {
            let m = 0.5 * (r[0] + r[1]);
            // Pushed so the left half comes off the stack first.
            todo.push(([m, r[1]], depth + 1));
            todo.push(([r[0], m], depth + 1));
        }
    }
    if pieces.is_empty() {
        return None;
    }
    let mut cps: Vec<Point> = Vec::new();
    let mut knots = vec![0.0; 4];
    for (k, r) in pieces.iter().enumerate() {
        let bez = hermite(view, it, *r);
        if k == 0 {
            cps.push(bez[0]);
        }
        cps.extend_from_slice(&bez[1..]);
        let kk = (k + 1) as f64;
        if k + 1 < pieces.len() {
            knots.extend([kk, kk, kk]);
        } else {
            knots.extend([kk, kk, kk, kk]);
        }
    }
    Some(Shape::Spline {
        degree: 3,
        knots,
        control_points: cps,
        closed: false,
    })
}

fn hermite(view: &ViewFrame, it: &Item, r: [f64; 2]) -> [Point; 4] {
    let h = r[1] - r[0];
    let (p0, p1) = (it.eval(view, r[0]), it.eval(view, r[1]));
    let (d0, d1) = (it.deriv(view, r[0]), it.deriv(view, r[1]));
    [p0, p0 + d0 * (h / 3.0), p1 - d1 * (h / 3.0), p1]
}

fn bezier(b: &[Point; 4], t: f64) -> Point {
    let u = 1.0 - t;
    let w = [u * u * u, 3.0 * u * u * t, 3.0 * u * t * t, t * t * t];
    Point::new(
        b.iter().zip(w).map(|(p, w)| p.x * w).sum(),
        b.iter().zip(w).map(|(p, w)| p.y * w).sum(),
    )
}

#[cfg(not(target_arch = "wasm32"))]
fn now() -> Option<std::time::Instant> {
    Some(std::time::Instant::now())
}

#[cfg(target_arch = "wasm32")]
fn now() -> Option<std::time::Instant> {
    None
}

fn since(t: Option<std::time::Instant>) -> f64 {
    t.map_or(0.0, |t| t.elapsed().as_secs_f64() * 1e3)
}

/// Progress on stderr when `CODECAD_HLR_DEBUG` is set (native only).
pub fn debug(msg: &str) {
    #[cfg(not(target_arch = "wasm32"))]
    if std::env::var_os("CODECAD_HLR_DEBUG").is_some() {
        eprintln!("[hlr] {msg}");
    }
    #[cfg(target_arch = "wasm32")]
    let _ = msg;
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Never;
    impl Oracle for Never {
        fn hidden(&mut self, _p: Point3, _owner: usize) -> bool {
            false
        }
    }

    fn front() -> ViewFrame {
        ViewFrame {
            origin: Point3::ORIGIN,
            right: Vec3::X,
            up: Vec3::Z,
            back: Vec3::new(0.0, -1.0, 0.0),
        }
    }

    fn line(a: [f64; 3], b: [f64; 3], owner: usize) -> Source {
        let (p, q) = (Point3::new(a[0], a[1], a[2]), Point3::new(b[0], b[1], b[2]));
        let d = q - p;
        Source {
            curve: Curve3::Line {
                origin: p,
                dir: d.normalized().unwrap(),
            },
            range: [0.0, d.length()],
            owner,
            kind: CurveKind::Edge,
        }
    }

    /// Two crossing lines split each other; a line ending on another
    /// splits it; a circle crossed by a line is split twice.
    #[test]
    fn crossings_and_end_points_split_curves() {
        let s = vec![
            line([0.0, 0.0, 0.0], [10.0, 0.0, 10.0], 0),
            line([0.0, 0.0, 10.0], [10.0, 0.0, 0.0], 0),
            line([5.0, 0.0, -5.0], [5.0, 0.0, 0.0], 1),
            line([0.0, 0.0, -5.0], [10.0, 0.0, -5.0], 1),
        ];
        let (_, stats, _) = run(&front(), s, &mut Never, &Options::default());
        // Diagonals cross each other (2); the vertical ends on the bottom
        // line's middle (1); the diagonals meet at (5, 5), the vertical
        // starts at (5, -5): 3 in all.
        assert_eq!(stats.splits, 3, "{stats:?}");
        let c = Source {
            curve: Curve3::Circle {
                center: Point3::new(0.0, 0.0, 0.0),
                x: Vec3::X,
                y: Vec3::Z,
                radius: 5.0,
            },
            range: [0.0, TAU],
            owner: 0,
            kind: CurveKind::Edge,
        };
        let (_, stats, _) = run(
            &front(),
            vec![c, line([-10.0, 0.0, 1.0], [10.0, 0.0, 1.0], 1)],
            &mut Never,
            &Options::default(),
        );
        assert_eq!(stats.splits, 4, "{stats:?}");
    }

    #[test]
    fn principal_axes_of_an_ellipse() {
        let (u, a, b) = principal_axes(Vec2::new(3.0, 0.0), Vec2::new(0.0, 1.0));
        assert!(
            (a - 3.0).abs() < 1e-15 && (b - 1.0).abs() < 1e-15 && (u.x.abs() - 1.0).abs() < 1e-15
        );
        let (u, a, b) = principal_axes(Vec2::new(0.0, 1.0), Vec2::new(-3.0, 0.0));
        assert!((a - 3.0).abs() < 1e-15 && (b - 1.0).abs() < 1e-15 && u.y.abs() < 1e-15);
    }
}
