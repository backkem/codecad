//! The feature-tree types of a solid part (docs/3d-design.md section 4.1).
//!
//! Everything here is kernel-neutral plain data: no kernel type appears in
//! these definitions, and nothing is flattened. Profiles use the same exact
//! 2D primitives as the document model (kurbo lines and arcs, circles,
//! B-splines), so an outline drawn in 2D can become a profile unchanged.
//!
//! Units are document units (mm), f64 throughout. The world frame has Z up.

use kurbo::{Arc, Circle, Line, Point};

use crate::types::{EntityId, LwVertex, Shape};

use super::kernel::EvalError;

// ── Ids ───────────────────────────────────────────────────────────────

/// Identity of a [`PartDef`]. Stable across edits and peers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct DefId(pub u64);

/// Identity of a [`Feature`] within its [`PartDef`]. Edge selectors name
/// features by this id, so it must stay stable when features are edited.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct FeatId(pub u32);

// ── 3D points, vectors, planes, placements ────────────────────────────

/// A point in 3D (mm).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Point3 {
    pub x: f64,
    pub y: f64,
    pub z: f64,
}

/// A vector in 3D (mm, or unitless for a direction).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Vec3 {
    pub x: f64,
    pub y: f64,
    pub z: f64,
}

impl Point3 {
    pub const ORIGIN: Point3 = Point3::new(0.0, 0.0, 0.0);

    pub const fn new(x: f64, y: f64, z: f64) -> Self {
        Self { x, y, z }
    }

    pub fn distance(self, other: Point3) -> f64 {
        (self - other).length()
    }

    pub fn is_finite(self) -> bool {
        self.x.is_finite() && self.y.is_finite() && self.z.is_finite()
    }

    /// The largest coordinate difference (the L-infinity distance).
    pub fn max_abs_diff(self, o: Point3) -> f64 {
        (self.x - o.x)
            .abs()
            .max((self.y - o.y).abs())
            .max((self.z - o.z).abs())
    }
}

impl Vec3 {
    pub const X: Vec3 = Vec3::new(1.0, 0.0, 0.0);
    pub const Y: Vec3 = Vec3::new(0.0, 1.0, 0.0);
    pub const Z: Vec3 = Vec3::new(0.0, 0.0, 1.0);

    pub const fn new(x: f64, y: f64, z: f64) -> Self {
        Self { x, y, z }
    }

    pub fn scale(self, s: f64) -> Vec3 {
        Vec3::new(self.x * s, self.y * s, self.z * s)
    }

    pub fn dot(self, o: Vec3) -> f64 {
        self.x * o.x + self.y * o.y + self.z * o.z
    }

    pub fn cross(self, o: Vec3) -> Vec3 {
        Vec3::new(
            self.y * o.z - self.z * o.y,
            self.z * o.x - self.x * o.z,
            self.x * o.y - self.y * o.x,
        )
    }

    pub fn length(self) -> f64 {
        self.dot(self).sqrt()
    }

    pub fn is_finite(self) -> bool {
        self.x.is_finite() && self.y.is_finite() && self.z.is_finite()
    }

    /// The unit vector along `self`, or `None` for a zero or non-finite
    /// vector.
    pub fn normalized(self) -> Option<Vec3> {
        let len = self.length();
        if !len.is_finite() || len <= 1e-12 {
            return None;
        }
        Some(self.scale(1.0 / len))
    }
}

impl std::ops::Add<Vec3> for Point3 {
    type Output = Point3;
    fn add(self, v: Vec3) -> Point3 {
        Point3::new(self.x + v.x, self.y + v.y, self.z + v.z)
    }
}

impl std::ops::Sub for Point3 {
    type Output = Vec3;
    fn sub(self, o: Point3) -> Vec3 {
        Vec3::new(self.x - o.x, self.y - o.y, self.z - o.z)
    }
}

impl std::ops::Add for Vec3 {
    type Output = Vec3;
    fn add(self, o: Vec3) -> Vec3 {
        Vec3::new(self.x + o.x, self.y + o.y, self.z + o.z)
    }
}

impl std::ops::Neg for Vec3 {
    type Output = Vec3;
    fn neg(self) -> Vec3 {
        Vec3::new(-self.x, -self.y, -self.z)
    }
}

/// A sketch plane: where a [`Profile2d`] lives in 3D.
///
/// Profile coordinates `(u, v)` map to `origin + u * x_axis + v * y_axis`,
/// where `y_axis = normal x x_axis`. `x_axis` and `normal` must be unit
/// length and perpendicular ([`Plane3::frame`] checks this).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Plane3 {
    pub origin: Point3,
    pub x_axis: Vec3,
    pub normal: Vec3,
}

/// An orthonormal right-handed frame resolved from a [`Plane3`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Frame3 {
    pub origin: Point3,
    pub x: Vec3,
    pub y: Vec3,
    pub z: Vec3,
}

impl Frame3 {
    /// The 3D point at profile coordinates `p`, offset `w` along the normal.
    pub fn at(&self, p: Point, w: f64) -> Point3 {
        self.origin + self.x.scale(p.x) + self.y.scale(p.y) + self.z.scale(w)
    }
}

impl Plane3 {
    /// World XY (plan view): `u` along X, `v` along Y, normal +Z.
    pub const XY: Plane3 = Plane3 {
        origin: Point3::ORIGIN,
        x_axis: Vec3::X,
        normal: Vec3::Z,
    };
    /// World XZ (front view): `u` along X, `v` along Z, normal -Y.
    pub const XZ: Plane3 = Plane3 {
        origin: Point3::ORIGIN,
        x_axis: Vec3::X,
        normal: Vec3::new(0.0, -1.0, 0.0),
    };
    /// World YZ (side view): `u` along Y, `v` along Z, normal +X.
    pub const YZ: Plane3 = Plane3 {
        origin: Point3::ORIGIN,
        x_axis: Vec3::Y,
        normal: Vec3::X,
    };

    /// The same plane moved by `d` along its normal.
    pub fn offset(self, d: f64) -> Plane3 {
        Plane3 {
            origin: self.origin + self.normal.scale(d),
            ..self
        }
    }

    /// The same orientation with a new origin.
    pub fn at(self, origin: Point3) -> Plane3 {
        Plane3 { origin, ..self }
    }

    /// The orthonormal frame of the plane, checked.
    pub fn frame(&self) -> Result<Frame3, EvalError> {
        const EPS: f64 = 1e-9;
        if !self.origin.is_finite() {
            return Err(EvalError::invalid(None, "plane origin is not finite"));
        }
        let unit = |v: Vec3| v.is_finite() && (v.length() - 1.0).abs() <= EPS;
        if !unit(self.x_axis) || !unit(self.normal) {
            return Err(EvalError::invalid(
                None,
                "plane x_axis and normal must be unit vectors",
            ));
        }
        if self.x_axis.dot(self.normal).abs() > EPS {
            return Err(EvalError::invalid(
                None,
                "plane x_axis must be perpendicular to its normal",
            ));
        }
        Ok(Frame3 {
            origin: self.origin,
            x: self.x_axis,
            y: self.normal.cross(self.x_axis),
            z: self.normal,
        })
    }
}

/// A rigid transform: rotation (unit quaternion `w, x, y, z`) then
/// translation. Maps a local point `p` to `R p + t`. No scale: parts use
/// explicit dimensions (design 4.4).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Placement {
    pub rotation: [f64; 4],
    pub translation: Vec3,
}

impl Placement {
    pub const IDENTITY: Placement = Placement {
        rotation: [1.0, 0.0, 0.0, 0.0],
        translation: Vec3::new(0.0, 0.0, 0.0),
    };

    pub const fn translation(x: f64, y: f64, z: f64) -> Placement {
        Placement {
            rotation: [1.0, 0.0, 0.0, 0.0],
            translation: Vec3::new(x, y, z),
        }
    }

    /// A rotation of `angle` radians about the unit `axis` (right-handed)
    /// through the origin, followed by `translation`.
    pub fn axis_angle(axis: Vec3, angle: f64, translation: Vec3) -> Placement {
        let a = axis.normalized().unwrap_or(Vec3::Z);
        let (s, c) = (angle * 0.5).sin_cos();
        Placement {
            rotation: [c, a.x * s, a.y * s, a.z * s],
            translation,
        }
    }

    pub fn is_identity(&self) -> bool {
        *self == Placement::IDENTITY
    }

    /// Checks the quaternion is finite and unit length (to 1e-9).
    pub fn check(&self) -> Result<(), EvalError> {
        let [w, x, y, z] = self.rotation;
        let n = (w * w + x * x + y * y + z * z).sqrt();
        if !n.is_finite() || (n - 1.0).abs() > 1e-9 || !self.translation.is_finite() {
            return Err(EvalError::invalid(
                None,
                "placement rotation must be a finite unit quaternion",
            ));
        }
        Ok(())
    }
}

// ── 2D profiles ───────────────────────────────────────────────────────

/// One exact segment of a profile loop. A loop is walked segment by
/// segment, each starting where the previous one ends.
#[derive(Debug, Clone, PartialEq)]
pub enum Seg2 {
    /// A straight segment from `p0` to `p1`.
    Line(Line),
    /// A circular (`radii.x == radii.y`) or elliptic arc. The sweep is
    /// signed: positive runs counter-clockwise. A sweep of a full turn is a
    /// closed loop on its own.
    Arc(Arc),
    /// A full circle, a closed loop on its own (counter-clockwise from
    /// angle 0).
    Circle(Circle),
    /// A non-rational B-spline, as `Shape::Spline` stores it.
    Spline {
        degree: i32,
        knots: Vec<f64>,
        control_points: Vec<Point>,
    },
}

/// The 2D section of a feature (design 4.1): one outer loop and any number
/// of hole loops, each a closed chain of exact segments. Never flattened.
///
/// Segment indices used by [`EdgeSel`] count the outer loop first, then
/// each hole loop in order.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Profile2d {
    pub outer: Vec<Seg2>,
    pub holes: Vec<Vec<Seg2>>,
}

/// How far apart two loop points may be and still meet (mm).
pub const PROFILE_JOIN_TOL: f64 = 1e-7;

impl Seg2 {
    /// Start and end points. A closed segment (full circle) starts and ends
    /// at the same point.
    pub fn ends(&self) -> Option<(Point, Point)> {
        match self {
            Seg2::Line(l) => Some((l.p0, l.p1)),
            Seg2::Arc(a) => Some((arc_point(a, a.start_angle), arc_end(a))),
            Seg2::Circle(c) => {
                let p = Point::new(c.center.x + c.radius, c.center.y);
                Some((p, p))
            }
            Seg2::Spline { control_points, .. } => {
                Some((*control_points.first()?, *control_points.last()?))
            }
        }
    }

    /// Whether the segment closes on itself (a full circle or full arc).
    pub fn is_closed_loop(&self) -> bool {
        match self {
            Seg2::Circle(_) => true,
            Seg2::Arc(a) => a.sweep_angle.abs() >= std::f64::consts::TAU - 1e-12,
            _ => false,
        }
    }
}

/// The point at parameter angle `t` of a kurbo arc (ellipse parameter).
pub fn arc_point(a: &Arc, t: f64) -> Point {
    let (s, c) = t.sin_cos();
    let (rs, rc) = a.x_rotation.sin_cos();
    let (x, y) = (a.radii.x * c, a.radii.y * s);
    Point::new(a.center.x + x * rc - y * rs, a.center.y + x * rs + y * rc)
}

fn arc_end(a: &Arc) -> Point {
    arc_point(a, a.start_angle + a.sweep_angle)
}

impl Profile2d {
    /// An axis-aligned rectangle `[x0, x1] x [y0, y1]`, walked
    /// counter-clockwise from `(x0, y0)`. Segments 0..4 are the bottom,
    /// right, top and left sides.
    pub fn rect(x0: f64, y0: f64, x1: f64, y1: f64) -> Profile2d {
        Profile2d::polygon(&[
            Point::new(x0, y0),
            Point::new(x1, y0),
            Point::new(x1, y1),
            Point::new(x0, y1),
        ])
    }

    /// A closed polygon through `points` (no repeated last point).
    pub fn polygon(points: &[Point]) -> Profile2d {
        let n = points.len();
        let outer = (0..n)
            .map(|i| Seg2::Line(Line::new(points[i], points[(i + 1) % n])))
            .collect();
        Profile2d {
            outer,
            holes: Vec::new(),
        }
    }

    /// A full circle.
    pub fn circle(center: Point, radius: f64) -> Profile2d {
        Profile2d {
            outer: vec![Seg2::Circle(Circle::new(center, radius))],
            holes: Vec::new(),
        }
    }

    /// The same profile with one more hole loop.
    pub fn with_hole(mut self, hole: Vec<Seg2>) -> Profile2d {
        self.holes.push(hole);
        self
    }

    /// One closed loop from a closed 2D shape of the document model:
    /// `LwPolyline` (bulges become exact arcs), `Polyline`, `Circle`, a
    /// full `Ellipse` or a closed `Spline`.
    pub fn loop_from_shape(shape: &Shape) -> Result<Vec<Seg2>, EvalError> {
        match shape {
            Shape::LwPolyline {
                vertices,
                closed: true,
            } => loop_from_bulges(vertices),
            Shape::Polyline {
                points,
                closed: true,
            } if points.len() >= 3 => Ok(Profile2d::polygon(points).outer),
            Shape::Circle(c) => Ok(vec![Seg2::Circle(*c)]),
            Shape::Ellipse {
                center,
                major_axis,
                minor_ratio,
                start_param,
                end_param,
            } if (end_param - start_param).abs() >= std::f64::consts::TAU - 1e-12 => {
                let major = major_axis.0.hypot(major_axis.1);
                Ok(vec![Seg2::Arc(Arc {
                    center: *center,
                    radii: kurbo::Vec2::new(major, major * minor_ratio),
                    start_angle: 0.0,
                    sweep_angle: std::f64::consts::TAU,
                    x_rotation: major_axis.1.atan2(major_axis.0),
                })])
            }
            Shape::Spline {
                degree,
                knots,
                control_points,
                closed: true,
            } => Ok(vec![Seg2::Spline {
                degree: *degree,
                knots: knots.clone(),
                control_points: control_points.clone(),
            }]),
            _ => Err(EvalError::invalid(
                None,
                "a profile loop must be a closed LwPolyline, Polyline, Circle, \
                 full Ellipse or closed Spline",
            )),
        }
    }

    /// Checks every loop is non-empty, finite and closed: each segment
    /// starts where the previous one ends, to [`PROFILE_JOIN_TOL`].
    pub fn check(&self) -> Result<(), EvalError> {
        if self.outer.is_empty() {
            return Err(EvalError::invalid(None, "profile has no outer loop"));
        }
        for (i, lp) in std::iter::once(&self.outer)
            .chain(self.holes.iter())
            .enumerate()
        {
            check_loop(lp).map_err(|why| {
                let which = if i == 0 {
                    "outer loop".to_string()
                } else {
                    format!("hole loop {}", i - 1)
                };
                EvalError::invalid(None, format!("profile {which}: {why}"))
            })?;
        }
        Ok(())
    }

    /// All loops, outer first, with the global index of each loop's first
    /// segment.
    pub fn loops(&self) -> impl Iterator<Item = (usize, &Vec<Seg2>)> {
        let mut base = 0;
        std::iter::once(&self.outer)
            .chain(self.holes.iter())
            .map(move |lp| {
                let start = base;
                base += lp.len();
                (start, lp)
            })
    }

    /// The segment with global index `i` (outer loop first, then holes),
    /// and the segment before it in the same loop.
    pub fn segment(&self, i: usize) -> Option<(&Seg2, &Seg2)> {
        for (start, lp) in self.loops() {
            if i < start + lp.len() {
                let k = i - start;
                let prev = (k + lp.len() - 1) % lp.len();
                return Some((&lp[k], &lp[prev]));
            }
        }
        None
    }

    /// The number of segments over all loops.
    pub fn segment_count(&self) -> usize {
        self.outer.len() + self.holes.iter().map(Vec::len).sum::<usize>()
    }
}

fn check_loop(lp: &[Seg2]) -> Result<(), String> {
    if lp.is_empty() {
        return Err("loop is empty".into());
    }
    for (i, seg) in lp.iter().enumerate() {
        let finite = |p: Point| p.x.is_finite() && p.y.is_finite();
        let Some((a, b)) = seg.ends() else {
            return Err(format!("segment {i} has no end points"));
        };
        if !finite(a) || !finite(b) {
            return Err(format!("segment {i} is not finite"));
        }
        match seg {
            Seg2::Arc(arc) if !(arc.radii.x > 0.0 && arc.radii.y > 0.0) => {
                return Err(format!("segment {i} has a non-positive radius"));
            }
            Seg2::Circle(c) if c.radius.partial_cmp(&0.0) != Some(std::cmp::Ordering::Greater) => {
                return Err(format!("segment {i} has a non-positive radius"));
            }
            _ => {}
        }
        if seg.is_closed_loop() && lp.len() > 1 {
            return Err(format!(
                "segment {i} is a closed curve and must be a loop on its own"
            ));
        }
    }
    for i in 0..lp.len() {
        let (_, end) = lp[i].ends().unwrap_or_default();
        let (start, _) = lp[(i + 1) % lp.len()].ends().unwrap_or_default();
        if end.distance(start) > PROFILE_JOIN_TOL {
            return Err(format!(
                "segment {i} ends at ({}, {}) but the next starts at ({}, {})",
                end.x, end.y, start.x, start.y
            ));
        }
    }
    Ok(())
}

/// Converts a closed bulge polyline to lines and exact circular arcs.
fn loop_from_bulges(vertices: &[LwVertex]) -> Result<Vec<Seg2>, EvalError> {
    let n = vertices.len();
    if n < 2 {
        return Err(EvalError::invalid(
            None,
            "a closed LwPolyline needs at least two vertices",
        ));
    }
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        let (p0, p1) = (vertices[i].point, vertices[(i + 1) % n].point);
        let bulge = vertices[i].bulge;
        if bulge == 0.0 {
            out.push(Seg2::Line(Line::new(p0, p1)));
            continue;
        }
        // Bulge b = tan(theta / 4), theta the signed included angle.
        // Chord c, radius r = c (1 + b^2) / (4 |b|); the centre sits on the
        // chord's perpendicular bisector at distance s = c (1 - b^2) / (4 b)
        // to the left of the chord direction (signed by b).
        let chord = p1 - p0;
        let c = chord.hypot();
        if c == 0.0 {
            return Err(EvalError::invalid(None, "LwPolyline has a zero-length arc"));
        }
        let theta = 4.0 * bulge.atan();
        let r = c * (1.0 + bulge * bulge) / (4.0 * bulge.abs());
        let s = c * (1.0 - bulge * bulge) / (4.0 * bulge);
        let mid = p0.midpoint(p1);
        let left = kurbo::Vec2::new(-chord.y, chord.x) / c;
        let center = mid + left * s;
        let start_angle = (p0.y - center.y).atan2(p0.x - center.x);
        out.push(Seg2::Arc(Arc {
            center,
            radii: kurbo::Vec2::new(r, r),
            start_angle,
            sweep_angle: theta,
            x_rotation: 0.0,
        }));
    }
    Ok(out)
}

/// A 3D path for a sweep: a chain of exact segments.
#[derive(Debug, Clone, PartialEq)]
pub struct Path3 {
    pub segs: Vec<Seg3>,
}

/// One segment of a [`Path3`].
#[derive(Debug, Clone, PartialEq)]
pub enum Seg3 {
    Line {
        from: Point3,
        to: Point3,
    },
    /// A circular arc through three points.
    Arc {
        from: Point3,
        via: Point3,
        to: Point3,
    },
}

// ── Features ──────────────────────────────────────────────────────────

/// How a feature's body combines with the part so far.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Op {
    Add,
    Subtract,
    Intersect,
}

/// How deep a hole goes.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Depth {
    /// To this depth (mm) from `at`, flat bottom.
    Blind(f64),
    /// Through everything along `dir`.
    Through,
}

/// Which cap of an extrude an [`EdgeSel::Cap`] names.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CapSide {
    /// The cap on the sketch plane.
    Start,
    /// The cap at `distance` along the normal.
    End,
    Both,
}

/// What happens when the kernel refuses an exact roundover (design 5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RoundFallback {
    Error,
    /// Render-only bevel plus a 2D note.
    Visual,
}

/// Edge selection by meaning, never by kernel topology (design 4.3).
#[derive(Debug, Clone, PartialEq)]
pub enum EdgeSel {
    /// Edges of an extrude's start or end cap, by profile segment index
    /// (`None` = every segment).
    Cap {
        feature: FeatId,
        side: CapSide,
        segs: Option<Vec<usize>>,
    },
    /// Edges swept from profile vertices: index `i` is the vertex where
    /// segment `i` starts.
    Lateral {
        feature: FeatId,
        segs: Vec<usize>,
    },
    /// The edge nearest each point, at evaluation time.
    Near {
        points: Vec<Point3>,
    },
    All,
}

/// A tool body for a boolean (joinery).
#[derive(Debug, Clone, PartialEq)]
pub enum ToolRef {
    /// Another part as placed in the scene, grown by `clearance`.
    Part { part: EntityId, clearance: f64 },
    /// An inline definition at a placement in this part's frame.
    Def(DefId, Placement),
}

/// One step of a part's feature tree, evaluated top to bottom.
#[derive(Debug, Clone, PartialEq)]
pub enum Feature {
    Extrude {
        id: FeatId,
        profile: Profile2d,
        plane: Plane3,
        /// Signed: negative extrudes against the plane normal.
        distance: f64,
        taper_deg: f64,
        op: Op,
    },
    Revolve {
        id: FeatId,
        profile: Profile2d,
        plane: Plane3,
        /// The axis in profile coordinates.
        axis: Line,
        angle_deg: f64,
        op: Op,
    },
    Sweep {
        id: FeatId,
        profile: Profile2d,
        path: Path3,
        op: Op,
    },
    Loft {
        id: FeatId,
        sections: Vec<(Profile2d, Plane3)>,
        ruled: bool,
        op: Op,
    },
    /// Always subtracts. `at` is the entry point, `dir` points into the
    /// material. `cbore` is `(diameter, depth)` of a counterbore.
    Hole {
        id: FeatId,
        at: Point3,
        dir: Vec3,
        dia: f64,
        depth: Depth,
        cbore: Option<(f64, f64)>,
    },
    Boolean {
        id: FeatId,
        op: Op,
        tool: ToolRef,
    },
    Fillet {
        id: FeatId,
        edges: EdgeSel,
        radius: f64,
        fallback: RoundFallback,
    },
    /// `d1 == d2` is the symmetric chamfer.
    Chamfer {
        id: FeatId,
        edges: EdgeSel,
        d1: f64,
        d2: f64,
    },
}

impl Feature {
    pub fn id(&self) -> FeatId {
        match self {
            Feature::Extrude { id, .. }
            | Feature::Revolve { id, .. }
            | Feature::Sweep { id, .. }
            | Feature::Loft { id, .. }
            | Feature::Hole { id, .. }
            | Feature::Boolean { id, .. }
            | Feature::Fillet { id, .. }
            | Feature::Chamfer { id, .. } => *id,
        }
    }

    /// The feature's kind as a word, for messages.
    pub fn kind_name(&self) -> &'static str {
        match self {
            Feature::Extrude { .. } => "extrude",
            Feature::Revolve { .. } => "revolve",
            Feature::Sweep { .. } => "sweep",
            Feature::Loft { .. } => "loft",
            Feature::Hole { .. } => "hole",
            Feature::Boolean { .. } => "boolean",
            Feature::Fillet { .. } => "fillet",
            Feature::Chamfer { .. } => "chamfer",
        }
    }
}

// ── Parts ─────────────────────────────────────────────────────────────

/// Grain direction of a flat part, in its local frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GrainDir {
    X,
    Y,
    /// Sheet goods with no grain (MDF, plastics).
    None,
}

/// What kind of part a definition is: decides the cut-list fast path.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PartKind {
    Flat { thickness: f64, grain: GrainDir },
    Turned,
    General,
}

/// A reusable solid definition in its own local frame (mm, f64).
#[derive(Debug, Clone, PartialEq)]
pub struct PartDef {
    pub id: DefId,
    pub name: String,
    pub kind: PartKind,
    /// Ordered, evaluated top to bottom.
    pub features: Vec<Feature>,
}
