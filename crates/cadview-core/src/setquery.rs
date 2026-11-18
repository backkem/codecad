//! Set-level geometric queries: questions about whole selections, answered
//! in one call with exact geometry.
//!
//! - `pairsWithin(a, b, distance)`: every pair closer than a distance, with
//!   the distance and the two closest points.
//! - `overlapping(a, b)`: every pair whose outlines meet or (with `areas`)
//!   whose closed areas contain one another, with the meeting points.
//! - `inside(a, region)`: which entities lie inside a rectangle, a ring, or
//!   each closed entity of a set.
//! - `nearest(a, b, {k})`: the k nearest entities of b for each entity of a.
//! - `near(point, radius)` and `inBounds(rect, mode)`: the one-point and
//!   one-rectangle versions.
//!
//! Lines, arcs, circles and (bulge) polylines are used exactly: an arc is a
//! centre, radius and angles, never a sampled polyline. Fills count with
//! their line and arc edges. Shapes without an exact distance here
//! (splines, ellipses, text, curve paths) are left out and reported in
//! `skipped`, so a rule never passes silently on geometry it did not see.
//! A static bounding-volume hierarchy keeps the queries near n log n.

use crate::attrs::{self, Attrs};
use crate::construct::{curve_intersections, norm_angle, shape_curves, Curve};
use crate::dispatch::parse_args;
use crate::document::Document;
use crate::select::{self, Item, Selector};
use crate::types::*;
use kurbo::{Point, Vec2};
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::f64::consts::{FRAC_PI_2, TAU};

type R = Result<String, String>;

/// The closest approach of two items: distance, the point on each, and
/// the child each point lies on.
type Closest = (f64, Point, Point, Option<usize>, Option<usize>);

/// Most pairs one call returns unless `limit` says otherwise.
const DEFAULT_LIMIT: usize = 100_000;

// ── Bounding boxes ─────────────────────────────────────────────────────

/// An axis-aligned box. Empty when `x0 > x1`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BBox {
    pub x0: f64,
    pub y0: f64,
    pub x1: f64,
    pub y1: f64,
}

impl BBox {
    pub const EMPTY: BBox = BBox {
        x0: f64::INFINITY,
        y0: f64::INFINITY,
        x1: f64::NEG_INFINITY,
        y1: f64::NEG_INFINITY,
    };

    fn add(&mut self, p: Point) {
        self.x0 = self.x0.min(p.x);
        self.y0 = self.y0.min(p.y);
        self.x1 = self.x1.max(p.x);
        self.y1 = self.y1.max(p.y);
    }

    fn union(self, o: BBox) -> BBox {
        BBox {
            x0: self.x0.min(o.x0),
            y0: self.y0.min(o.y0),
            x1: self.x1.max(o.x1),
            y1: self.y1.max(o.y1),
        }
    }

    fn inflate(self, d: f64) -> BBox {
        BBox {
            x0: self.x0 - d,
            y0: self.y0 - d,
            x1: self.x1 + d,
            y1: self.y1 + d,
        }
    }

    fn is_empty(&self) -> bool {
        self.x0 > self.x1 || self.y0 > self.y1
    }

    fn overlaps(&self, o: &BBox) -> bool {
        self.x0 <= o.x1 && o.x0 <= self.x1 && self.y0 <= o.y1 && o.y0 <= self.y1
    }

    fn contains(&self, o: &BBox) -> bool {
        o.x0 >= self.x0 && o.x1 <= self.x1 && o.y0 >= self.y0 && o.y1 <= self.y1
    }

    fn center(&self, axis: usize) -> f64 {
        if axis == 0 {
            (self.x0 + self.x1) / 2.0
        } else {
            (self.y0 + self.y1) / 2.0
        }
    }

    /// Largest absolute coordinate, for a scale-aware tolerance.
    fn magnitude(&self) -> f64 {
        if self.is_empty() {
            return 0.0;
        }
        self.x0
            .abs()
            .max(self.x1.abs())
            .max(self.y0.abs())
            .max(self.y1.abs())
    }

    fn diagonal(&self) -> f64 {
        if self.is_empty() {
            0.0
        } else {
            (self.x1 - self.x0).hypot(self.y1 - self.y0)
        }
    }
}

// ── Curve helpers ──────────────────────────────────────────────────────

fn polar(c: Point, r: f64, a: f64) -> Point {
    Point::new(c.x + r * a.cos(), c.y + r * a.sin())
}

fn angle_of(p: Point, c: Point) -> f64 {
    (p.y - c.y).atan2(p.x - c.x)
}

/// A counter-clockwise arc from `s` to `e` (equal angles: a full circle).
fn arc_ccw(c: Point, r: f64, s: f64, e: f64) -> Curve {
    let mut sweep = e - s;
    if sweep <= 0.0 {
        sweep += TAU;
    }
    Curve::Arc {
        c,
        r,
        start: norm_angle(s),
        sweep: sweep.min(TAU),
    }
}

/// Is angle `a` within the arc's sweep (no tolerance)?
fn on_sweep(start: f64, sweep: f64, a: f64) -> bool {
    sweep >= TAU - 1e-12 || norm_angle(a - start) <= sweep
}

fn curve_start(c: &Curve) -> Point {
    match *c {
        Curve::Seg { p0, .. } => p0,
        Curve::Arc { c, r, start, .. } => polar(c, r, start),
    }
}

fn curve_end(c: &Curve) -> Point {
    match *c {
        Curve::Seg { p1, .. } => p1,
        Curve::Arc { c, r, start, sweep } => polar(c, r, start + sweep),
    }
}

/// The point at parameter `t` in [0, 1].
fn point_at(c: &Curve, t: f64) -> Point {
    match *c {
        Curve::Seg { p0, p1 } => p0 + (p1 - p0) * t,
        Curve::Arc { c, r, start, sweep } => polar(c, r, start + sweep * t),
    }
}

/// The parameter in [0, 1] of a point on (or within tolerance of) a curve.
fn param_of(c: &Curve, p: Point) -> f64 {
    match *c {
        Curve::Seg { p0, p1 } => {
            let d = p1 - p0;
            let l2 = d.hypot2();
            if l2 < 1e-300 {
                0.0
            } else {
                ((p - p0).dot(d) / l2).clamp(0.0, 1.0)
            }
        }
        Curve::Arc {
            c, start, sweep, ..
        } => {
            let rel = norm_angle(angle_of(p, c) - start);
            if rel <= sweep {
                rel / sweep
            } else if rel - sweep < TAU - rel {
                1.0
            } else {
                0.0
            }
        }
    }
}

/// Exact bounding box of a curve.
pub fn curve_bbox(cv: &Curve) -> BBox {
    let mut b = BBox::EMPTY;
    match *cv {
        Curve::Seg { p0, p1 } => {
            b.add(p0);
            b.add(p1);
        }
        Curve::Arc { c, r, start, sweep } => {
            b.add(polar(c, r, start));
            b.add(polar(c, r, start + sweep));
            for k in 0..4 {
                let a = k as f64 * FRAC_PI_2;
                if on_sweep(start, sweep, a) {
                    b.add(polar(c, r, a));
                }
            }
        }
    }
    b
}

/// The point of a curve closest to `p`.
fn closest_on(cv: &Curve, p: Point) -> Point {
    match *cv {
        Curve::Seg { p0, p1 } => {
            let d = p1 - p0;
            let l2 = d.hypot2();
            if l2 < 1e-300 {
                return p0;
            }
            p0 + d * ((p - p0).dot(d) / l2).clamp(0.0, 1.0)
        }
        Curve::Arc { c, r, start, sweep } => {
            let v = p - c;
            if v.hypot() < 1e-300 {
                return polar(c, r, start);
            }
            let a = v.y.atan2(v.x);
            if on_sweep(start, sweep, a) {
                return c + v * (r / v.hypot());
            }
            let e0 = polar(c, r, start);
            let e1 = polar(c, r, start + sweep);
            if (p - e0).hypot2() <= (p - e1).hypot2() {
                e0
            } else {
                e1
            }
        }
    }
}

/// Exact distance between two curves, with the closest point on each.
///
/// The minimum is either an intersection (distance 0), or lies at an
/// endpoint of one curve, or at an interior point of both, where the
/// connecting line is normal to both curves. For a segment and an arc those
/// interior points lie on the radius normal to the segment; for two arcs, on
/// the line through both centres. Every candidate is a real pair of points,
/// so the smallest is the distance.
pub fn curve_distance(a: &Curve, b: &Curve) -> (f64, Point, Point) {
    if let Some(p) = curve_intersections(a, b).first() {
        return (0.0, *p, *p);
    }
    let mut best = (f64::INFINITY, Point::ZERO, Point::ZERO);
    let mut try_ab = |pa: Point, pb: Point| {
        let d = (pa - pb).hypot();
        if d < best.0 {
            best = (d, pa, pb);
        }
    };
    // Endpoints (a full circle offers its start point: harmless, and it
    // covers two concentric circles).
    for pa in [curve_start(a), curve_end(a)] {
        try_ab(pa, closest_on(b, pa));
    }
    for pb in [curve_start(b), curve_end(b)] {
        try_ab(closest_on(a, pb), pb);
    }
    // Interior points of both.
    let radial = |arc: &Curve, dir: Vec2| -> Vec<Point> {
        let Curve::Arc { c, r, start, sweep } = *arc else {
            return Vec::new();
        };
        let l = dir.hypot();
        if l < 1e-300 {
            return Vec::new();
        }
        let u = dir / l;
        [u, -u]
            .into_iter()
            .filter(|u| on_sweep(start, sweep, u.y.atan2(u.x)))
            .map(|u| c + u * r)
            .collect()
    };
    match (*a, *b) {
        (Curve::Seg { p0, p1 }, Curve::Arc { .. }) => {
            let d = p1 - p0;
            for q in radial(b, Vec2::new(-d.y, d.x)) {
                try_ab(closest_on(a, q), q);
            }
        }
        (Curve::Arc { .. }, Curve::Seg { p0, p1 }) => {
            let d = p1 - p0;
            for q in radial(a, Vec2::new(-d.y, d.x)) {
                try_ab(q, closest_on(b, q));
            }
        }
        (Curve::Arc { c: c1, .. }, Curve::Arc { c: c2, .. }) => {
            let w = c2 - c1;
            for q in radial(a, w) {
                try_ab(q, closest_on(b, q));
            }
            for q in radial(b, w) {
                try_ab(closest_on(a, q), q);
            }
        }
        _ => {}
    }
    best
}

/// Pieces of `a` and `b` that coincide along a stretch (collinear
/// overlapping segments, or arcs of one circle with overlapping sweeps), as
/// the end points of each shared stretch.
fn shared_stretches(a: &Curve, b: &Curve, eps: f64) -> Vec<(Point, Point)> {
    match (*a, *b) {
        (Curve::Seg { p0, p1 }, Curve::Seg { p0: q0, p1: q1 }) => {
            let d = p1 - p0;
            let len = d.hypot();
            if len <= eps || (q1 - q0).hypot() <= eps {
                return Vec::new();
            }
            let u = d / len;
            let off = |q: Point| u.cross(q - p0).abs();
            if off(q0) > eps || off(q1) > eps {
                return Vec::new();
            }
            let t0 = (q0 - p0).dot(u);
            let t1 = (q1 - p0).dot(u);
            let lo = t0.min(t1).max(0.0);
            let hi = t0.max(t1).min(len);
            if hi - lo > eps {
                vec![(p0 + u * lo, p0 + u * hi)]
            } else {
                Vec::new()
            }
        }
        (
            Curve::Arc {
                c: c1,
                r: r1,
                start: s1,
                sweep: w1,
            },
            Curve::Arc {
                c: c2,
                r: r2,
                start: s2,
                sweep: w2,
            },
        ) => {
            if (c1 - c2).hypot() > eps || (r1 - r2).abs() > eps || r1 <= eps {
                return Vec::new();
            }
            let ang_eps = eps / r1;
            let full1 = w1 >= TAU - 1e-12;
            let full2 = w2 >= TAU - 1e-12;
            if full1 && full2 {
                // Two equal circles share everything: report the circle's
                // start point twice (a closed stretch).
                let p = polar(c1, r1, s1);
                return vec![(p, p)];
            }
            // Intersect [0, w1] with b's sweep shifted relative to a's start.
            let d = norm_angle(s2 - s1);
            let mut out = Vec::new();
            for shift in [d - TAU, d] {
                let lo = shift.max(0.0);
                let hi = (shift + w2).min(w1);
                if hi - lo > ang_eps {
                    out.push((polar(c1, r1, s1 + lo), polar(c1, r1, s1 + hi)));
                }
            }
            out
        }
        _ => Vec::new(),
    }
}

// ── Geometry of a shape ────────────────────────────────────────────────

/// A closed area: rings of curves, combined even-odd (a hole is a ring
/// inside another).
type Area = Vec<Vec<Curve>>;

/// The exact outline of a shape and, for a closed shape, its area. None
/// when the shape has no exact form here.
fn shape_geometry(shape: &Shape) -> Option<(Vec<Curve>, Option<Area>)> {
    match shape {
        Shape::Line(_) | Shape::Arc { .. } => Some((shape_curves(shape), None)),
        Shape::Circle(c) => {
            let ring = vec![Curve::Arc {
                c: c.center,
                r: c.radius,
                start: 0.0,
                sweep: TAU,
            }];
            Some((ring.clone(), Some(vec![ring])))
        }
        Shape::Polyline { points, closed } => {
            let curves = shape_curves(shape);
            let area = (*closed && points.len() >= 3).then(|| vec![curves.clone()]);
            Some((curves, area))
        }
        Shape::LwPolyline { vertices, closed } => {
            let curves = shape_curves(shape);
            let area = (*closed && vertices.len() >= 2).then(|| vec![curves.clone()]);
            Some((curves, area))
        }
        Shape::SolidFill { boundary, holes } => {
            let mut rings = vec![fill_ring(boundary)?];
            for h in holes {
                rings.push(fill_ring(h)?);
            }
            let curves = rings.iter().flatten().copied().collect();
            Some((curves, Some(rings)))
        }
        _ => None,
    }
}

/// The curves of one fill boundary, closed. None for ellipse or spline
/// edges.
fn fill_ring(edges: &[FillEdge]) -> Option<Vec<Curve>> {
    let mut out = Vec::new();
    let mut first: Option<Point> = None;
    let mut cur: Option<Point> = None;
    let line_to = |out: &mut Vec<Curve>, cur: &mut Option<Point>, p: Point| {
        if let Some(c) = *cur {
            if (p - c).hypot() > 0.0 {
                out.push(Curve::Seg { p0: c, p1: p });
            }
        }
        *cur = Some(p);
    };
    for e in edges {
        match e {
            FillEdge::LineTo(p) => {
                first.get_or_insert(*p);
                line_to(&mut out, &mut cur, *p);
            }
            FillEdge::PolylineTo(pts) => {
                for p in pts {
                    first.get_or_insert(*p);
                    line_to(&mut out, &mut cur, *p);
                }
            }
            FillEdge::ArcTo {
                center,
                radius,
                start_angle,
                end_angle,
            } => {
                let arc = arc_ccw(*center, *radius, *start_angle, *end_angle);
                let s = curve_start(&arc);
                first.get_or_insert(s);
                line_to(&mut out, &mut cur, s);
                out.push(arc);
                cur = Some(curve_end(&arc));
            }
            FillEdge::EllipseArcTo { .. } | FillEdge::SplineTo { .. } => return None,
        }
    }
    if let (Some(f), Some(c)) = (first, cur) {
        if (f - c).hypot() > 0.0 {
            out.push(Curve::Seg { p0: c, p1: f });
        }
    }
    Some(out)
}

/// Where a point is relative to a region.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Side {
    In,
    On,
    Out,
}

/// The angle a curve sweeps as seen from `p` (p not on the curve).
fn subtended(cv: &Curve, p: Point) -> f64 {
    match *cv {
        Curve::Seg { p0, p1 } => {
            let a = p0 - p;
            let b = p1 - p;
            a.cross(b).atan2(a.dot(b))
        }
        Curve::Arc { c, r, start, sweep } => {
            let q0 = polar(c, r, start) - p;
            let q1 = polar(c, r, start + sweep) - p;
            let turn = q0.cross(q1).atan2(q0.dot(q1));
            if (p - c).hypot() < r {
                // From inside the circle the direction turns counter-
                // clockwise all the way, between 0 and 2π.
                if sweep >= TAU - 1e-12 {
                    TAU
                } else if turn < 0.0 {
                    turn + TAU
                } else {
                    turn
                }
            } else {
                // From outside, the whole arc lies within half a turn.
                turn
            }
        }
    }
}

/// Is `p` inside an area (even-odd over its rings), on its boundary (within
/// `eps`), or outside?
fn classify(p: Point, area: &Area, eps: f64) -> Side {
    if area
        .iter()
        .flatten()
        .any(|cv| (closest_on(cv, p) - p).hypot() <= eps)
    {
        return Side::On;
    }
    let mut inside = false;
    for ring in area {
        let w: f64 = ring.iter().map(|cv| subtended(cv, p)).sum();
        if (w / TAU).round() as i64 % 2 != 0 {
            inside = !inside;
        }
    }
    if inside {
        Side::In
    } else {
        Side::Out
    }
}

/// `classify` over a union of areas.
fn classify_union(p: Point, areas: &[Area], eps: f64) -> Side {
    let mut on = false;
    for a in areas {
        match classify(p, a, eps) {
            Side::In => return Side::In,
            Side::On => on = true,
            Side::Out => {}
        }
    }
    if on {
        Side::On
    } else {
        Side::Out
    }
}

// ── Items with geometry ────────────────────────────────────────────────

/// A selected item, decomposed.
struct Geo {
    item: Item,
    /// Outline curves with the child they belong to.
    curves: Vec<(Option<usize>, Curve)>,
    /// The closed shapes of the item (their union is its region).
    areas: Vec<Area>,
    /// Shapes left out, with their child index and type.
    skipped: Vec<(Option<usize>, &'static str)>,
    bbox: BBox,
}

impl Geo {
    fn new(item: Item) -> Geo {
        let mut curves = Vec::new();
        let mut areas = Vec::new();
        let mut skipped = Vec::new();
        let mut bbox = BBox::EMPTY;
        for (child, shape) in &item.shapes {
            match shape_geometry(shape) {
                Some((cs, area)) => {
                    for c in cs {
                        bbox = bbox.union(curve_bbox(&c));
                        curves.push((*child, c));
                    }
                    if let Some(a) = area {
                        areas.push(a);
                    }
                }
                None => skipped.push((*child, select::shape_type(shape))),
            }
        }
        Geo {
            item,
            curves,
            areas,
            skipped,
            bbox,
        }
    }

    fn key(&self) -> (EntityId, Option<usize>) {
        (self.item.id, self.item.child)
    }
}

fn geos(doc: &Document, sel: &Selector) -> Vec<Geo> {
    select::collect(doc, sel)
        .into_iter()
        .map(Geo::new)
        .collect()
}

/// A tolerance in drawing units, relative to how far from the origin the
/// geometry lies.
fn tolerance(sets: &[&[Geo]]) -> f64 {
    let m = sets
        .iter()
        .flat_map(|s| s.iter())
        .map(|g| g.bbox.magnitude())
        .fold(1.0, f64::max);
    1e-9 * m
}

fn skipped_json(sets: &[&[Geo]]) -> Value {
    let mut out = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for s in sets {
        for g in s.iter() {
            for (child, kind) in &g.skipped {
                if seen.insert((g.item.id, *child)) {
                    let mut v = json!({ "id": format!("e_{}", g.item.id.0), "type": kind });
                    if let Some(c) = child {
                        v["child"] = json!(c);
                    }
                    out.push(v);
                }
            }
        }
    }
    Value::Array(out)
}

// ── Spatial index ──────────────────────────────────────────────────────

/// A static bounding-volume hierarchy over boxes (median split on the
/// longer axis, up to 8 boxes per leaf).
struct Bvh {
    nodes: Vec<Node>,
    order: Vec<u32>,
    boxes: Vec<BBox>,
}

struct Node {
    bbox: BBox,
    /// Leaf: `order[start..end]`; inner: children `left` and `left + 1`
    /// are not adjacent, so both are stored.
    start: u32,
    end: u32,
    left: u32,
    right: u32,
}

const LEAF: u32 = u32::MAX;

impl Bvh {
    fn build(boxes: Vec<BBox>) -> Bvh {
        let mut bvh = Bvh {
            nodes: Vec::new(),
            order: (0..boxes.len() as u32).collect(),
            boxes,
        };
        if !bvh.order.is_empty() {
            bvh.build_range(0, bvh.order.len());
        }
        bvh
    }

    fn build_range(&mut self, lo: usize, hi: usize) -> u32 {
        let mut bbox = BBox::EMPTY;
        let mut cbox = BBox::EMPTY;
        for &i in &self.order[lo..hi] {
            let b = self.boxes[i as usize];
            bbox = bbox.union(b);
            cbox.add(Point::new(b.center(0), b.center(1)));
        }
        let id = self.nodes.len() as u32;
        self.nodes.push(Node {
            bbox,
            start: lo as u32,
            end: hi as u32,
            left: LEAF,
            right: LEAF,
        });
        if hi - lo <= 8 {
            return id;
        }
        let axis = if cbox.x1 - cbox.x0 >= cbox.y1 - cbox.y0 {
            0
        } else {
            1
        };
        let mid = (lo + hi) / 2;
        let boxes = &self.boxes;
        self.order[lo..hi].select_nth_unstable_by(mid - lo, |a, b| {
            boxes[*a as usize]
                .center(axis)
                .total_cmp(&boxes[*b as usize].center(axis))
        });
        let left = self.build_range(lo, mid);
        let right = self.build_range(mid, hi);
        let n = &mut self.nodes[id as usize];
        n.left = left;
        n.right = right;
        id
    }

    /// Call `f` with every box index whose box overlaps `q`.
    fn query(&self, q: &BBox, mut f: impl FnMut(usize)) {
        if self.nodes.is_empty() {
            return;
        }
        let mut stack = vec![0u32];
        while let Some(n) = stack.pop() {
            let node = &self.nodes[n as usize];
            if !node.bbox.overlaps(q) {
                continue;
            }
            if node.left == LEAF {
                for &i in &self.order[node.start as usize..node.end as usize] {
                    if self.boxes[i as usize].overlaps(q) {
                        f(i as usize);
                    }
                }
            } else {
                stack.push(node.left);
                stack.push(node.right);
            }
        }
    }
}

/// Every curve of a set of items, flattened, with an index over them.
struct Pieces {
    /// (item index, child, curve)
    list: Vec<(usize, Option<usize>, Curve)>,
    bvh: Bvh,
}

impl Pieces {
    fn of(geos: &[Geo]) -> Pieces {
        let mut list = Vec::new();
        let mut boxes = Vec::new();
        for (i, g) in geos.iter().enumerate() {
            for (child, c) in &g.curves {
                list.push((i, *child, *c));
                boxes.push(curve_bbox(c));
            }
        }
        Pieces {
            list,
            bvh: Bvh::build(boxes),
        }
    }
}

// ── Arguments and results ──────────────────────────────────────────────

fn id_str(id: EntityId) -> String {
    format!("e_{}", id.0)
}

fn pt_json(p: Point) -> Value {
    json!([p.x, p.y])
}

/// Do the two items hold equal values (missing counts as a value) for every
/// key in `keys`?
fn same_values(a: &Attrs, b: &Attrs, keys: &[String]) -> bool {
    keys.iter().all(|k| a.get(k) == b.get(k))
}

/// Options shared by the pair queries.
#[derive(Deserialize, Default)]
struct PairOpts {
    /// Leave out pairs whose values for all these keys are equal.
    #[serde(default)]
    differ: Vec<String>,
    /// Include each entity's attributes in the pair.
    #[serde(default)]
    attrs: bool,
    #[serde(default)]
    limit: Option<usize>,
}

/// The two sets of a pair query: `b` absent or null means pairs within `a`.
fn two_sets(
    doc: &Document,
    a: &Value,
    b: &Option<Value>,
) -> Result<(Vec<Geo>, Option<Vec<Geo>>), String> {
    let ga = geos(doc, &Selector::parse(a)?);
    let gb = match b {
        None | Some(Value::Null) => None,
        Some(v) => Some(geos(doc, &Selector::parse(v)?)),
    };
    Ok((ga, gb))
}

fn pair_head(a: &Geo, ac: Option<usize>, b: &Geo, bc: Option<usize>, opts: &PairOpts) -> Value {
    let mut v = json!({ "a": id_str(a.item.id), "b": id_str(b.item.id) });
    if let Some(c) = ac {
        v["aChild"] = json!(c);
    }
    if let Some(c) = bc {
        v["bChild"] = json!(c);
    }
    if opts.attrs {
        v["aAttrs"] = attrs::to_json(&a.item.attrs);
        v["bAttrs"] = attrs::to_json(&b.item.attrs);
    }
    v
}

/// For every curve of `ga[i]`, the candidate pieces of `pb` within `reach`,
/// skipping the item itself and (within one set) items at or before `i`.
fn for_candidates(
    ga: &[Geo],
    gb: &[Geo],
    pb: &Pieces,
    same_set: bool,
    reach: f64,
    mut f: impl FnMut(usize, Option<usize>, &Curve, usize, Option<usize>, &Curve),
) {
    for (i, g) in ga.iter().enumerate() {
        for (ac, cv) in &g.curves {
            let q = curve_bbox(cv).inflate(reach);
            pb.bvh.query(&q, |k| {
                let (j, bc, bcv) = &pb.list[k];
                if same_set {
                    if *j <= i {
                        return;
                    }
                } else if gb[*j].key() == g.key() {
                    return;
                }
                f(i, *ac, cv, *j, *bc, bcv);
            });
        }
    }
}

// ── pairsWithin ────────────────────────────────────────────────────────

fn pairs_within(doc: &Document, args: &str) -> R {
    #[derive(Deserialize)]
    struct A {
        a: Value,
        #[serde(default)]
        b: Option<Value>,
        distance: f64,
        #[serde(flatten)]
        opts: PairOpts,
    }
    let a: A = parse_args(args)?;
    if !(a.distance.is_finite() && a.distance >= 0.0) {
        return Err("distance must be a finite number >= 0".into());
    }
    let (ga, gb) = two_sets(doc, &a.a, &a.b)?;
    let same_set = gb.is_none();
    let gb_ref: &[Geo] = gb.as_deref().unwrap_or(&ga);
    let pb = Pieces::of(gb_ref);
    // Best (distance, pa, pb, aChild, bChild) per item pair.
    let mut best: HashMap<(usize, usize), Closest> = HashMap::new();
    for_candidates(
        &ga,
        gb_ref,
        &pb,
        same_set,
        a.distance,
        |i, ac, acv, j, bc, bcv| {
            if !a.opts.differ.is_empty()
                && same_values(&ga[i].item.attrs, &gb_ref[j].item.attrs, &a.opts.differ)
            {
                return;
            }
            let (d, p, q) = curve_distance(acv, bcv);
            if d < a.distance {
                let e = best.entry((i, j)).or_insert((f64::INFINITY, p, q, ac, bc));
                if d < e.0 {
                    *e = (d, p, q, ac, bc);
                }
            }
        },
    );
    let mut keys: Vec<_> = best.keys().copied().collect();
    keys.sort_unstable();
    let limit = a.opts.limit.unwrap_or(DEFAULT_LIMIT);
    let truncated = keys.len() > limit;
    let pairs: Vec<Value> = keys
        .into_iter()
        .take(limit)
        .map(|(i, j)| {
            let (d, p, q, ac, bc) = best[&(i, j)];
            let mut v = pair_head(&ga[i], ac, &gb_ref[j], bc, &a.opts);
            v["distance"] = json!(d);
            v["pa"] = pt_json(p);
            v["pb"] = pt_json(q);
            v
        })
        .collect();
    let mut out = json!({ "pairs": pairs, "skipped": skipped_json(&[&ga, gb_ref]) });
    if truncated {
        out["truncated"] = json!(true);
    }
    Ok(out.to_string())
}

// ── overlapping ────────────────────────────────────────────────────────

#[derive(Default)]
struct Contact {
    points: Vec<Point>,
    shared: bool,
    a_child: Option<usize>,
    b_child: Option<usize>,
}

fn push_point(points: &mut Vec<Point>, p: Point, eps: f64) {
    if !points.iter().any(|q| (*q - p).hypot() <= eps) {
        points.push(p);
    }
}

/// Does some part of `b` lie inside an area of `a` (b's outline not
/// meeting a's)? One point per curve of b decides for that curve.
fn some_part_inside(a: &Geo, b: &Geo, eps: f64) -> bool {
    !a.areas.is_empty()
        && b.bbox.overlaps(&a.bbox)
        && b.curves
            .iter()
            .any(|(_, cv)| classify_union(point_at(cv, 0.5), &a.areas, eps) == Side::In)
}

fn overlapping(doc: &Document, args: &str) -> R {
    #[derive(Deserialize)]
    struct A {
        a: Value,
        #[serde(default)]
        b: Option<Value>,
        /// Closed shapes count as filled areas.
        #[serde(default)]
        areas: bool,
        #[serde(flatten)]
        opts: PairOpts,
    }
    let a: A = parse_args(args)?;
    let (ga, gb) = two_sets(doc, &a.a, &a.b)?;
    let same_set = gb.is_none();
    let gb_ref: &[Geo] = gb.as_deref().unwrap_or(&ga);
    let eps = tolerance(&[&ga, gb_ref]);
    let pb = Pieces::of(gb_ref);
    let mut contacts: HashMap<(usize, usize), Contact> = HashMap::new();
    for_candidates(&ga, gb_ref, &pb, same_set, eps, |i, ac, acv, j, bc, bcv| {
        if !a.opts.differ.is_empty()
            && same_values(&ga[i].item.attrs, &gb_ref[j].item.attrs, &a.opts.differ)
        {
            return;
        }
        let stretches = shared_stretches(acv, bcv, eps);
        let mut points = curve_intersections(acv, bcv);
        if points.is_empty() && stretches.is_empty() {
            // Touching within the tolerance (an end on the other curve).
            let (d, p, _) = curve_distance(acv, bcv);
            if d <= eps {
                points.push(p);
            }
        }
        if points.is_empty() && stretches.is_empty() {
            return;
        }
        let c = contacts.entry((i, j)).or_insert_with(|| Contact {
            a_child: ac,
            b_child: bc,
            ..Default::default()
        });
        for p in points {
            push_point(&mut c.points, p, eps);
        }
        for (p, q) in stretches {
            c.shared = true;
            push_point(&mut c.points, p, eps);
            push_point(&mut c.points, q, eps);
        }
    });
    // Containment without contact, between the closed areas.
    let mut contains: HashMap<(usize, usize), &'static str> = HashMap::new();
    if a.areas {
        let ib = Bvh::build(gb_ref.iter().map(|g| g.bbox).collect());
        for (i, g) in ga.iter().enumerate() {
            ib.query(&g.bbox, |j| {
                if (same_set && j <= i) || (!same_set && gb_ref[j].key() == g.key()) {
                    return;
                }
                if contacts.contains_key(&(i, j)) {
                    return;
                }
                let h = &gb_ref[j];
                if !a.opts.differ.is_empty()
                    && same_values(&g.item.attrs, &h.item.attrs, &a.opts.differ)
                {
                    return;
                }
                if some_part_inside(g, h, eps) {
                    contains.insert((i, j), "a");
                } else if some_part_inside(h, g, eps) {
                    contains.insert((i, j), "b");
                }
            });
        }
    }
    let mut keys: Vec<_> = contacts.keys().chain(contains.keys()).copied().collect();
    keys.sort_unstable();
    keys.dedup();
    let limit = a.opts.limit.unwrap_or(DEFAULT_LIMIT);
    let truncated = keys.len() > limit;
    let pairs: Vec<Value> = keys
        .into_iter()
        .take(limit)
        .map(|(i, j)| match contacts.get(&(i, j)) {
            Some(c) => {
                let mut v = pair_head(&ga[i], c.a_child, &gb_ref[j], c.b_child, &a.opts);
                v["points"] = Value::Array(c.points.iter().map(|p| pt_json(*p)).collect());
                v["shared"] = json!(c.shared);
                v
            }
            None => {
                let mut v = pair_head(&ga[i], None, &gb_ref[j], None, &a.opts);
                v["points"] = json!([]);
                v["shared"] = json!(false);
                v["contains"] = json!(contains[&(i, j)]);
                v
            }
        })
        .collect();
    let mut out = json!({ "pairs": pairs, "skipped": skipped_json(&[&ga, gb_ref]) });
    if truncated {
        out["truncated"] = json!(true);
    }
    Ok(out.to_string())
}

// ── inside ─────────────────────────────────────────────────────────────

/// Does item `a` lie inside the region `areas` (with `rings` all their
/// rings)? Returns None when it does not, else whether it touches the
/// boundary.
fn lies_inside(a: &Geo, areas: &[Area], region_box: &BBox, eps: f64) -> Option<bool> {
    if a.curves.is_empty() || !region_box.inflate(eps).contains(&a.bbox) {
        return None;
    }
    let mut touch = false;
    for (_, cv) in &a.curves {
        let mut ts = vec![0.0, 1.0];
        for area in areas {
            for rc in area.iter().flatten() {
                if !curve_bbox(rc).inflate(eps).overlaps(&curve_bbox(cv)) {
                    continue;
                }
                for p in curve_intersections(cv, rc) {
                    touch = true;
                    ts.push(param_of(cv, p));
                }
                for (p, q) in shared_stretches(cv, rc, eps) {
                    touch = true;
                    ts.push(param_of(cv, p));
                    ts.push(param_of(cv, q));
                }
            }
        }
        ts.sort_by(f64::total_cmp);
        ts.dedup_by(|x, y| (*x - *y).abs() < 1e-12);
        let probes: Vec<f64> = if ts.len() < 2 {
            vec![0.5]
        } else {
            ts.windows(2).map(|w| (w[0] + w[1]) / 2.0).collect()
        };
        for t in probes {
            match classify_union(point_at(cv, t), areas, eps) {
                Side::Out => return None,
                Side::On => touch = true,
                Side::In => {}
            }
        }
    }
    // A closed item must not cover part of the region's boundary (a hole).
    if !a.areas.is_empty() {
        for rc in areas.iter().flatten().flatten() {
            if classify_union(curve_start(rc), &a.areas, eps) == Side::In {
                return None;
            }
        }
    }
    Some(touch)
}

/// A region given inline: a rectangle `{min, max}` or a ring of points.
fn inline_region(v: &Value) -> Result<Option<Area>, String> {
    let pts: Vec<Point> = match v {
        Value::Object(o) if o.contains_key("min") && o.contains_key("max") => {
            let p = |k: &str| -> Result<Point, String> {
                let a = o[k]
                    .as_array()
                    .filter(|a| a.len() >= 2)
                    .ok_or("a rectangle is {min: [x, y], max: [x, y]}")?;
                Ok(Point::new(
                    a[0].as_f64().ok_or("coordinates are numbers")?,
                    a[1].as_f64().ok_or("coordinates are numbers")?,
                ))
            };
            let (lo, hi) = (p("min")?, p("max")?);
            vec![
                Point::new(lo.x, lo.y),
                Point::new(hi.x, lo.y),
                Point::new(hi.x, hi.y),
                Point::new(lo.x, hi.y),
            ]
        }
        Value::Array(a)
            if a.first().is_some_and(|f| {
                f.as_array()
                    .is_some_and(|f| f.first().is_some_and(Value::is_number))
            }) =>
        {
            a.iter()
                .map(|p| {
                    let a = p
                        .as_array()
                        .filter(|a| a.len() >= 2)
                        .ok_or("a ring is a list of [x, y] points")?;
                    Ok(Point::new(
                        a[0].as_f64().ok_or("coordinates are numbers")?,
                        a[1].as_f64().ok_or("coordinates are numbers")?,
                    ))
                })
                .collect::<Result<_, String>>()?
        }
        _ => return Ok(None),
    };
    if pts.len() < 3 {
        return Err("a ring needs at least 3 points".into());
    }
    let n = pts.len();
    Ok(Some(vec![(0..n)
        .map(|i| Curve::Seg {
            p0: pts[i],
            p1: pts[(i + 1) % n],
        })
        .collect()]))
}

fn inside(doc: &Document, args: &str) -> R {
    #[derive(Deserialize)]
    struct A {
        a: Value,
        /// A rectangle, a ring of points, or a selection of closed entities.
        region: Value,
        /// Touching the boundary does not count as inside.
        #[serde(default)]
        strict: bool,
        #[serde(flatten)]
        opts: PairOpts,
    }
    let a: A = parse_args(args)?;
    let ga = geos(doc, &Selector::parse(&a.a)?);
    let mut pairs = Vec::new();
    let mut skipped_regions = Vec::new();
    let gb;
    if let Some(area) = inline_region(&a.region)? {
        let mut rb = BBox::EMPTY;
        for c in area.iter().flatten() {
            rb = rb.union(curve_bbox(c));
        }
        let eps = tolerance(&[&ga]).max(1e-9 * rb.magnitude());
        let areas = vec![area];
        for g in &ga {
            if let Some(touch) = lies_inside(g, &areas, &rb, eps) {
                if a.strict && touch {
                    continue;
                }
                let mut v = json!({ "a": id_str(g.item.id), "b": Value::Null });
                if let Some(c) = g.item.child {
                    v["aChild"] = json!(c);
                }
                if a.opts.attrs {
                    v["aAttrs"] = attrs::to_json(&g.item.attrs);
                }
                v["touches"] = json!(touch);
                pairs.push(v);
            }
        }
        gb = Vec::new();
    } else {
        gb = geos(doc, &Selector::parse(&a.region)?);
        let eps = tolerance(&[&ga, &gb]);
        let index = Bvh::build(gb.iter().map(|g| g.bbox).collect());
        let mut found = Vec::new();
        for (i, g) in ga.iter().enumerate() {
            index.query(&g.bbox.inflate(eps), |j| {
                let h = &gb[j];
                if h.areas.is_empty() || h.key() == g.key() {
                    return;
                }
                if !a.opts.differ.is_empty()
                    && same_values(&g.item.attrs, &h.item.attrs, &a.opts.differ)
                {
                    return;
                }
                if let Some(touch) = lies_inside(g, &h.areas, &h.bbox, eps) {
                    if !(a.strict && touch) {
                        found.push((i, j, touch));
                    }
                }
            });
        }
        found.sort_unstable_by_key(|(i, j, _)| (*i, *j));
        for (i, j, touch) in found {
            let mut v = pair_head(&ga[i], ga[i].item.child, &gb[j], gb[j].item.child, &a.opts);
            v["touches"] = json!(touch);
            pairs.push(v);
        }
        for h in &gb {
            if h.areas.is_empty() && !h.curves.is_empty() {
                let mut v = json!({ "id": id_str(h.item.id), "type": "not a closed area" });
                if let Some(c) = h.item.child {
                    v["child"] = json!(c);
                }
                skipped_regions.push(v);
            }
        }
    }
    let mut skipped = skipped_json(&[&ga, &gb]);
    if let Value::Array(s) = &mut skipped {
        s.extend(skipped_regions);
    }
    Ok(json!({ "pairs": pairs, "skipped": skipped }).to_string())
}

// ── nearest ────────────────────────────────────────────────────────────

fn nearest(doc: &Document, args: &str) -> R {
    #[derive(Deserialize)]
    struct A {
        a: Value,
        #[serde(default)]
        b: Option<Value>,
        /// How many neighbours per entity (default 1).
        #[serde(default)]
        k: Option<usize>,
        /// Only neighbours closer than this.
        #[serde(default)]
        within: Option<f64>,
        #[serde(flatten)]
        opts: PairOpts,
    }
    let a: A = parse_args(args)?;
    let k = a.k.unwrap_or(1);
    if k == 0 {
        return Err("k must be at least 1".into());
    }
    let (ga, gb) = two_sets(doc, &a.a, &a.b)?;
    let same_set = gb.is_none();
    let gb_ref: &[Geo] = gb.as_deref().unwrap_or(&ga);
    let pb = Pieces::of(gb_ref);
    let all = gb_ref
        .iter()
        .chain(ga.iter())
        .fold(BBox::EMPTY, |b, g| b.union(g.bbox));
    let diag = all.diagonal();
    let r_max = a.within.unwrap_or(f64::INFINITY).min(diag * 2.0 + 1.0);
    let r0 = (diag / (pb.list.len().max(1) as f64).sqrt()).max(1e-9);
    let mut pairs = Vec::new();
    for (i, g) in ga.iter().enumerate() {
        let mut r = r0.min(r_max);
        let found = loop {
            let mut best: HashMap<usize, Closest> = HashMap::new();
            for (ac, cv) in &g.curves {
                pb.bvh.query(&curve_bbox(cv).inflate(r), |q| {
                    let (j, bc, bcv) = &pb.list[q];
                    if (same_set && *j == i) || (!same_set && gb_ref[*j].key() == g.key()) {
                        return;
                    }
                    if !a.opts.differ.is_empty()
                        && same_values(&g.item.attrs, &gb_ref[*j].item.attrs, &a.opts.differ)
                    {
                        return;
                    }
                    let (d, p, q) = curve_distance(cv, bcv);
                    let e = best.entry(*j).or_insert((f64::INFINITY, p, q, *ac, *bc));
                    if d < e.0 {
                        *e = (d, p, q, *ac, *bc);
                    }
                });
            }
            let mut within: Vec<_> = best.into_iter().filter(|(_, v)| v.0 <= r).collect();
            if within.len() >= k || r >= r_max {
                within.sort_by(|x, y| x.1 .0.total_cmp(&y.1 .0).then(x.0.cmp(&y.0)));
                within.truncate(k);
                break within;
            }
            r = (r * 4.0).min(r_max);
        };
        for (j, (d, p, q, ac, bc)) in found {
            if a.within.is_some_and(|w| d >= w) {
                continue;
            }
            let mut v = pair_head(g, ac, &gb_ref[j], bc, &a.opts);
            v["distance"] = json!(d);
            v["pa"] = pt_json(p);
            v["pb"] = pt_json(q);
            pairs.push(v);
        }
    }
    Ok(json!({ "pairs": pairs, "skipped": skipped_json(&[&ga, gb_ref]) }).to_string())
}

// ── near and inBounds ──────────────────────────────────────────────────

/// An item as `entities()` reports it.
fn item_json(doc: &Document, item: &Item) -> Value {
    let Some(ent) = doc.entity(item.id) else {
        return Value::Null;
    };
    let j = match item.child {
        Some(c) => select::insert_children(doc, ent)
            .into_iter()
            .nth(c)
            .map(|(i, child)| {
                let mut j = child.to_json(doc);
                j.child = Some(i);
                j
            }),
        None => Some(ent.to_json(doc)),
    };
    j.and_then(|j| serde_json::to_value(j).ok())
        .unwrap_or(Value::Null)
}

fn bbox_of(g: &Geo) -> BBox {
    if !g.curves.is_empty() && g.skipped.is_empty() {
        return g.bbox;
    }
    // Shapes without exact geometry: their stored bounding boxes.
    let mut b = g.bbox;
    for (_, s) in &g.item.shapes {
        let (x0, y0, x1, y1) = s.bbox();
        b = b.union(BBox { x0, y0, x1, y1 });
    }
    b
}

fn near(doc: &Document, args: &str) -> R {
    #[derive(Deserialize)]
    struct A {
        point: [f64; 2],
        radius: f64,
        #[serde(default)]
        select: Option<Value>,
    }
    let a: A = parse_args(args)?;
    let p = Point::new(a.point[0], a.point[1]);
    let sel = match &a.select {
        Some(v) => Selector::parse(v)?,
        None => Selector::all(),
    };
    let mut found: Vec<(f64, bool, Value)> = Vec::new();
    for g in geos(doc, &sel) {
        let (d, approx) = if g.skipped.is_empty() && !g.curves.is_empty() {
            let d = g
                .curves
                .iter()
                .map(|(_, c)| (closest_on(c, p) - p).hypot())
                .fold(f64::INFINITY, f64::min);
            (d, false)
        } else {
            // Exact where possible, the box for the rest.
            let b = bbox_of(&g);
            let dx = (b.x0 - p.x).max(0.0).max(p.x - b.x1);
            let dy = (b.y0 - p.y).max(0.0).max(p.y - b.y1);
            let exact = g
                .curves
                .iter()
                .map(|(_, c)| (closest_on(c, p) - p).hypot())
                .fold(f64::INFINITY, f64::min);
            (exact.min(dx.hypot(dy)), true)
        };
        if d <= a.radius {
            found.push((d, approx, item_json(doc, &g.item)));
        }
    }
    found.sort_by(|x, y| x.0.total_cmp(&y.0));
    let out: Vec<Value> = found
        .into_iter()
        .map(|(d, approx, mut v)| {
            v["distance"] = json!(d);
            if approx {
                v["approx"] = json!(true);
            }
            v
        })
        .collect();
    Ok(Value::Array(out).to_string())
}

fn in_bounds(doc: &Document, args: &str) -> R {
    #[derive(Deserialize)]
    struct A {
        rect: Value,
        #[serde(default)]
        mode: Option<String>,
        #[serde(default)]
        select: Option<Value>,
    }
    let a: A = parse_args(args)?;
    let Some(area) = inline_region(&a.rect)? else {
        return Err("rect is {min: [x, y], max: [x, y]}".into());
    };
    let mut r = BBox::EMPTY;
    for c in area.iter().flatten() {
        r = r.union(curve_bbox(c));
    }
    let contain = match a.mode.as_deref() {
        None | Some("intersect") => false,
        Some("contain") => true,
        Some(o) => return Err(format!("unknown mode '{o}' (intersect|contain)")),
    };
    let sel = match &a.select {
        Some(v) => Selector::parse(v)?,
        None => Selector::all(),
    };
    let out: Vec<Value> = geos(doc, &sel)
        .iter()
        .filter(|g| {
            let b = bbox_of(g);
            !b.is_empty()
                && if contain {
                    r.contains(&b)
                } else {
                    r.overlaps(&b)
                }
        })
        .map(|g| item_json(doc, &g.item))
        .collect();
    Ok(Value::Array(out).to_string())
}

/// Set-query methods of `cad_call`; None for any other method.
pub fn dispatch(doc: &mut Document, method: &str, args: &str) -> Option<R> {
    let r = match method {
        "pairsWithin" => pairs_within(doc, args),
        "overlapping" => overlapping(doc, args),
        "inside" => inside(doc, args),
        "nearest" => nearest(doc, args),
        "near" => near(doc, args),
        "inBounds" => in_bounds(doc, args),
        _ => return None,
    };
    Some(r)
}
