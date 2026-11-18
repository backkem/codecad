//! The reference meaning of a part (docs/3d-test-harness.md section 3).
//!
//! [`Reference::build`] turns a [`PartDef`] into a point set with a
//! certified signed function `f`: `f < 0` inside, `f > 0` outside, and
//! `|f(p)|` a guaranteed lower bound on the distance from `p` to the
//! boundary. Every primitive below gives its exact signed distance (or, for
//! the prism, the maximum of two exact signed distances), each of which is
//! 1-Lipschitz with an exact sign. Features combine with `min` (union),
//! `max` (intersection) and `max(a, -b)` (difference). For sign-exact
//! lower bounds `fa`, `fb` these keep the sign exact, and their magnitude
//! stays a lower bound on the distance to the combined boundary: outside a
//! union the distance is the smaller of the two distances; inside an
//! intersection the depth is the smaller of the two depths; in the other
//! two cases the combined set's depth (or distance) is at least the
//! operand's. The results are 1-Lipschitz too, since `min` and `max` of
//! 1-Lipschitz functions are.
//!
//! Every value is computed in outward-rounded interval arithmetic
//! ([`crate::interval`]), so an interval that does not contain zero is a
//! certified answer and one that does is the boundary band. Evaluated over
//! an interval box, the result encloses `f` over the whole box.
//!
//! This module shares no code with any kernel adapter and calls no kernel.
//! It uses only the plain data types of `cadview_core::solid`; even the
//! plane frame and the quaternion rotation are recomputed here.
//!
//! # Definitions (the specification)
//!
//! **Frame of a sketch plane.** `z = normal / |normal|`,
//! `x = (x_axis - (x_axis . z) z) / |...|`, `y = z x x`. Input whose axes
//! are not unit and perpendicular to 1e-9 is invalid (as `types.rs`
//! says). A point's plane coordinates are `u = (p - origin) . x`,
//! `v = (p - origin) . y`, `w = (p - origin) . z`.
//!
//! **Extrude** (`taper_deg == 0`, `distance = h != 0`). The body is
//! `{p : (u, v) in region(profile), w in [min(0, h), max(0, h)]}`, with
//! `region` as defined in [`region`]. `f = max(f_region(u, v), max(w0 - w,
//! w - w1))`: the projection to `(u, v)` and `w` are 1-Lipschitz, both
//! terms are exact signed distances (to the infinite prism and to the
//! slab), and their maximum is sign-exact for the intersection. A tapered
//! extrude has no reference meaning yet.
//!
//! **Hole** (`dir != 0`, `dia > 0`). With `d = dir / |dir|`, `t = (p - at)
//! . d` and `rho` the distance from `p` to the axis line, the removed set
//! is `{rho < dia / 2, 0 <= t <= depth}` (`depth = +infinity` for
//! `Through`), united with `{rho < cbore_dia / 2, 0 <= t <= cbore_depth}`
//! for a counterbore. Material on the far side of the entry point
//! (`t < 0`) is never removed. `f = max(rho - r, -t, t - depth)` per
//! cylinder, each term an exact signed distance. A counterbore needs
//! `cbore_dia > dia` and `0 < cbore_depth` (`< depth` when blind).
//!
//! **Boolean with an inline tool** `ToolRef::Def(def, placement)`. The tool
//! is the reference of `def`, moved by the placement: `x` is in the placed
//! tool iff `R^T (x - t)` is in the tool, where `R` is the rotation of the
//! normalised quaternion (an exact rotation, so the motion is an isometry
//! and keeps `f` exact). Ops: add = union, subtract = difference,
//! intersect = intersection. A tool that refers back to a definition being
//! built, or is unknown, is invalid. `ToolRef::Part` has no meaning yet.
//!
//! **The part so far** starts empty. A first feature that subtracts or
//! intersects leaves it empty (a kernel should refuse that recipe).
//!
//! **Chamfer** with `d1 == d2 = d`, defined only in this case: the
//! selector is `EdgeSel::Cap` of the extrude `F`, `F` is the part's first
//! feature, it adds, and every feature between `F` and this chamfer is a
//! chamfer of `F` too (so the part so far is `F`'s prism with earlier
//! chamfers). `F`'s outer loop must be all lines and strictly convex, and
//! no selected segment may be on a hole loop. For each selected segment
//! `i` (a line in the profile plane with inward unit normal `n_i`, `s_i(u,
//! v) = n_i . ((u, v) - a_i)` its signed distance inside) and each
//! selected cap with depth `t` into the material from that cap's plane,
//! the chamfer keeps `{s_i + t >= d}`: the half-space bounded by the plane
//! through the cap edge offset `d` inward and the side edge offset `d`
//! down, the 45-degree plane of an equal-distance chamfer. The result is
//! the part so far intersected with every kept half-space; at a convex
//! corner where two chamfered edges meet this is the mitre. `f = (d - s_i -
//! t) / sqrt(2)` is the exact signed distance to that plane (`s_i` and `t`
//! have orthogonal unit gradients). Also required, so that every chamfer
//! face exists and the meaning matches a chamfer of edges: `d > 0`, the
//! chamfers of one cap and of the other together use less than the
//! extrude's height, the same edge is not chamfered twice, and after
//! offsetting the selected edges inward every edge of the cap polygon
//! keeps a length above 1e-6 mm. A concave corner, an arc segment, other
//! selectors, an asymmetric chamfer and a chamfer after any other feature
//! are "not defined".
//!
//! **Revolve** (`0 < angle_deg <= 360`, axis points `p0 != p1` in the
//! profile's plane). With `du` the unit axis direction in the plane and
//! `eu` the unit normal to it towards the profile (the profile must lie in
//! the closed half-plane `s = (q - p0) . eu >= -1e-9`, else "not
//! defined"; anything at `s < 0` is not revolved), the 3D axis runs
//! through `p0` along `d` (both lifted by the plane's frame). A point at
//! axial coordinate `t` and distance `rho` from the axis is in the full
//! revolve iff its meridian point `p0 + t du + rho eu` is in the profile's
//! region. `f = f_region(p0 + t du + rho eu)`: `p -> (t, rho)` is
//! 1-Lipschitz into the half-plane and the meridian map is an isometry, so
//! `f` is 1-Lipschitz, its sign is exact, and it bounds the distance to
//! the surface of revolution (whose meridian is part of the region's
//! boundary: segments on the axis are not surface, so near the axis `|f|`
//! is smaller than the distance, still a lower bound). Below a full turn
//! the solid is also cut to the angles `[0, theta]` about `d`, measured
//! from `e1` (`eu` lifted) towards `d x e1`: the wedge is the intersection
//! (`theta <= 180`) or the union (`theta > 180`) of two half-spaces through
//! the axis, each an exact plane distance; `theta`'s sine and cosine are
//! enclosed from `angle_deg pi / 180`.
//!
//! **Loft**, defined for two sections on planes with the same axes, each
//! one strictly convex polygon of lines with the same number of sides,
//! side `i` of one certainly parallel to and in the direction of side `i`
//! of the other (`ruled` or not: two sections are the ruled loft). Every
//! side face is then a plane and the solid is the convex hull of the two
//! polygons; `f` is the maximum of the exact signed distances to the side
//! planes and the two cap planes (computed in intervals from the vertices).
//! Other lofts are "not defined" (a twisted ruled face has no exact
//! distance here).
//!
//! **Sweep** along a path of lines and arcs that passes `Path3::check`
//! (starts at the plane's origin along its normal, tangent-continuous).
//! The section (the profile in the plane's frame) moves rigidly with no
//! twist: along a line of length `L` it slides `L` along the frame's
//! normal; along an arc it turns about the arc's axis (the circumcentre of
//! its three points and the normal the points run counter-clockwise
//! about, in intervals) by the arc's angle. The swept solid is the union of
//! the pieces: per line the prism of the region over `[0, L]`, per arc the
//! revolve of the section about the arc's axis by its angle (as above,
//! with the axis taken in the section's own coordinates; the section must
//! lie on one side of it). `f` is the minimum over the pieces. An arc of
//! (nearly) half a turn takes the hull of both wedge forms.
//!
//! **Fillet** (rolling ball), defined only for `EdgeSel::All` directly
//! after the part's first feature, an adding, untapered extrude of one
//! strictly convex polygon of lines with no holes, with `0 < r`, `2 r` less
//! than the height by more than 1e-6, and every side of the polygon shrunk
//! by `r` keeping a length above 1e-6. The result is the prism opened by a
//! ball of radius `r`, which for a convex prism is the set of points
//! within `r` of the prism shrunk by `r` (`P'`: the polygon's sides moved
//! in by `r`, the height cut by `r` at both caps). `f = sd(P') - r` with
//! `sd(P')` the exact signed distance to the convex prism `P'`: inside it
//! the larger of the polygon's and the slab's signed distances, outside
//! `sqrt(a+^2 + b+^2)` of the polygon's exact distance (to its sides) and
//! the slab's. `sd` is 1-Lipschitz and exact, so `f` is too. Rounds of
//! other edges, other features before or between, are "not defined".
//!
//! A tapered extrude has no reference meaning yet: "not defined".

pub mod region;

use std::sync::Arc;

use cadview_core::solid::{
    CapSide, DefId, Depth, EdgeSel, FeatId, Feature, Op, PartDef, Placement, Plane3, Point3, Seg2,
    Seg3, ToolRef, ToolResolver,
};

use crate::interval::{Iv, Iv2, Iv3};
use region::{Region, RegionError};

/// Why the reference has no meaning for a recipe.
#[derive(Debug, Clone, PartialEq)]
pub struct NotDefined {
    pub feature: Option<FeatId>,
    /// `true`: malformed input a kernel ought to refuse. `false`:
    /// well-formed but outside what the reference defines.
    pub invalid: bool,
    pub reason: String,
}

impl std::fmt::Display for NotDefined {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let what = if self.invalid {
            "invalid input"
        } else {
            "not defined"
        };
        match self.feature {
            Some(id) => write!(f, "{what} at feature {}: {}", id.0, self.reason),
            None => write!(f, "{what}: {}", self.reason),
        }
    }
}

impl NotDefined {
    /// The same, attributed to `f` if it names no feature yet.
    fn at(mut self, f: Option<FeatId>) -> Self {
        if self.feature.is_none() {
            self.feature = f;
        }
        self
    }
}

fn invalid(feature: Option<FeatId>, reason: impl Into<String>) -> NotDefined {
    NotDefined {
        feature,
        invalid: true,
        reason: reason.into(),
    }
}

fn undefined(feature: Option<FeatId>, reason: impl Into<String>) -> NotDefined {
    NotDefined {
        feature,
        invalid: false,
        reason: reason.into(),
    }
}

/// A certified point classification.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Class {
    /// Certainly inside, at least `margin` mm from the boundary.
    Inside { margin: f64 },
    /// Certainly outside, at least `margin` mm from the boundary.
    Outside { margin: f64 },
    /// The enclosure of `f` holds zero: on or near the boundary, or not
    /// decidable in floating point.
    Band,
}

/// An orthonormal frame with certified axes.
#[derive(Debug, Clone, Copy)]
struct IFrame {
    origin: Iv3,
    x: Iv3,
    y: Iv3,
    z: Iv3,
}

impl IFrame {
    fn local(&self, p: Iv3) -> (Iv, Iv, Iv) {
        let d = p - self.origin;
        (d.dot(self.x), d.dot(self.y), d.dot(self.z))
    }

    /// The linear form `a_u u + a_v v + a_w w + k` in world coordinates:
    /// `g = a_u x + a_v y + a_w z`, `f = g . (p - o) + k = g . p - (g . o -
    /// k)`.
    fn linear(&self, a: [Iv; 3], k: Iv) -> Linear {
        let g = self.x.scale(a[0]) + self.y.scale(a[1]) + self.z.scale(a[2]);
        Linear {
            g,
            c: g.dot(self.origin) - k,
        }
    }

    /// The world box of plane coordinates `(u, v, w)` in a box.
    fn world_box(&self, u: [f64; 2], v: [f64; 2], w: [f64; 2]) -> Bounds {
        let mut b = Bounds::EMPTY;
        for uu in u {
            for vv in v {
                for ww in w {
                    let p = self.origin
                        + self.x.scale(Iv::point(uu))
                        + self.y.scale(Iv::point(vv))
                        + self.z.scale(Iv::point(ww));
                    b.add(p);
                }
            }
        }
        b
    }
}

fn iv3(p: Point3) -> Iv3 {
    Iv3::point(p.x, p.y, p.z)
}

fn frame_of(plane: &Plane3) -> Result<IFrame, String> {
    const EPS: f64 = 1e-9;
    let o = plane.origin;
    let (xa, n) = (plane.x_axis, plane.normal);
    let finite = |v: [f64; 3]| v.iter().all(|c| c.is_finite());
    if !finite([o.x, o.y, o.z]) || !finite([xa.x, xa.y, xa.z]) || !finite([n.x, n.y, n.z]) {
        return Err("plane is not finite".into());
    }
    let len = |v: cadview_core::solid::Vec3| (v.x * v.x + v.y * v.y + v.z * v.z).sqrt();
    if (len(xa) - 1.0).abs() > EPS || (len(n) - 1.0).abs() > EPS {
        return Err("plane x_axis and normal must be unit vectors".into());
    }
    if (xa.x * n.x + xa.y * n.y + xa.z * n.z).abs() > EPS {
        return Err("plane x_axis must be perpendicular to its normal".into());
    }
    let nz = Iv3::point(n.x, n.y, n.z);
    let z = nz.scale(Iv::point(1.0) / nz.norm());
    let xv = Iv3::point(xa.x, xa.y, xa.z);
    let xp = xv - z.scale(xv.dot(z));
    let x = xp.scale(Iv::point(1.0) / xp.norm());
    let y = z.cross(x);
    Ok(IFrame {
        origin: iv3(o),
        x,
        y,
        z,
    })
}

/// A rigid motion `x -> R x + t` with a certified exact rotation.
#[derive(Debug, Clone, Copy)]
struct Motion {
    /// Rows of `R`.
    r: [Iv3; 3],
    t: Iv3,
}

impl Motion {
    fn of(p: &Placement) -> Result<Motion, String> {
        let [w, x, y, z] = p.rotation;
        let n = (w * w + x * x + y * y + z * z).sqrt();
        let t = p.translation;
        if !n.is_finite()
            || (n - 1.0).abs() > 1e-9
            || ![t.x, t.y, t.z].iter().all(|c| c.is_finite())
        {
            return Err("placement rotation must be a finite unit quaternion".into());
        }
        // Normalise exactly (in intervals), then the standard matrix.
        let q = [w, x, y, z].map(Iv::point);
        let norm = (q[0].sqr() + q[1].sqr() + q[2].sqr() + q[3].sqr()).sqrt();
        let inv = Iv::point(1.0) / norm;
        let [w, x, y, z] = q.map(|c| c * inv);
        let two = Iv::point(2.0);
        let one = Iv::point(1.0);
        let r = [
            Iv3::new(
                one - two * (y.sqr() + z.sqr()),
                two * (x * y - w * z),
                two * (x * z + w * y),
            ),
            Iv3::new(
                two * (x * y + w * z),
                one - two * (x.sqr() + z.sqr()),
                two * (y * z - w * x),
            ),
            Iv3::new(
                two * (x * z - w * y),
                two * (y * z + w * x),
                one - two * (x.sqr() + y.sqr()),
            ),
        ];
        Ok(Motion {
            r,
            t: Iv3::point(t.x, t.y, t.z),
        })
    }

    /// `R p + t`: local to world.
    fn to_world(self, p: Iv3) -> Iv3 {
        self.rotate(p) + self.t
    }

    /// `R v`.
    fn rotate(&self, v: Iv3) -> Iv3 {
        Iv3::new(self.r[0].dot(v), self.r[1].dot(v), self.r[2].dot(v))
    }
}

/// A plane's signed distance `g . p - c` with `|g| = 1` in exact
/// arithmetic, negative on the kept side.
#[derive(Debug, Clone, Copy)]
struct Linear {
    g: Iv3,
    c: Iv,
}

impl Linear {
    fn eval(&self, p: Iv3) -> Iv {
        self.g.dot(p) - self.c
    }

    /// The same plane for the set moved by `m`: `f'(x) = f(R^T (x - t)) =
    /// (R g) . x - (c + (R g) . t)`.
    fn moved(&self, m: &Motion) -> Linear {
        let g = m.rotate(self.g);
        Linear {
            g,
            c: self.c + g.dot(m.t),
        }
    }
}

/// A node of the reference tree. Motions are applied when the tree is
/// built (every point, direction and plane is moved), so evaluation never
/// composes boxes through a rotation.
#[derive(Debug, Clone)]
enum Node {
    Empty,
    /// `{(u, v) in region, w in [w0, w1]}` in `frame`, for a region with
    /// no closed form of its own. `w0` and `w1` enclose the exact ends.
    Prism {
        frame: IFrame,
        region: Arc<Region>,
        w0: Iv,
        w1: Iv,
    },
    /// A solid of revolution, optionally cut to a wedge of angles.
    Revolve(Box<Rev>),
    /// A convex prism opened by a ball: the points within `r` of the
    /// prism shrunk by `r`.
    Opening(Box<Opening>),
    /// A convex polytope: the maximum of exact signed plane distances
    /// (a prism over a convex polygon: its edge planes and its two caps).
    /// `frame`, `rb` (the region's box) and `w0`, `w1` only bound it.
    Polytope {
        planes: Vec<Linear>,
        frame: IFrame,
        rb: [f64; 4],
        w0: f64,
        w1: f64,
    },
    /// `{rho < r, t0 <= t <= t1}` about the axis through `a` along unit
    /// `d`; `t1 = None` is unbounded.
    Cylinder {
        a: Iv3,
        d: Iv3,
        r: f64,
        t0: f64,
        t1: Option<f64>,
    },
    /// One half-space (a chamfer's kept side).
    Half(Linear),
    Union(Box<Node>, Box<Node>),
    Inter(Box<Node>, Box<Node>),
    Diff(Box<Node>, Box<Node>),
}

/// A solid of revolution about the line through `a` along the unit `d`.
/// A point `p` has axial coordinate `t = (p - a) . d` and radius `rho`,
/// its distance from the axis; it is in the full revolve when the meridian
/// point `o2 + t du + rho eu` lies in `region` (`du`, `eu` orthonormal in
/// the region's plane: an isometry from the meridian half-plane `rho >= 0`
/// into the region's plane). `wedge` keeps the angles `phi` in `[0,
/// theta]`, measured from `e1` (unit, square to `d`) towards `e2 = d x e1`.
#[derive(Debug, Clone)]
struct Rev {
    /// The profile's plane, for alignment and nothing else.
    frame: IFrame,
    a: Iv3,
    d: Iv3,
    e1: Iv3,
    e2: Iv3,
    o2: Iv2,
    du: Iv2,
    eu: Iv2,
    region: Arc<Region>,
    wedge: Option<Wedge>,
    /// The meridian's extent: `t` in `[tmin, tmax]`, `rho <= rmax`.
    tmin: f64,
    tmax: f64,
    rmax: f64,
}

/// The angles `[0, theta]`: `s = sin theta`, `c = cos theta`; `convex`
/// says `theta <= pi` (`None` when that is not certain: then the hull of
/// both forms, which holds the true one).
#[derive(Debug, Clone, Copy)]
struct Wedge {
    s: Iv,
    c: Iv,
    convex: Option<bool>,
}

impl Rev {
    fn eval(&self, p: Iv3) -> Iv {
        let v = p - self.a;
        let t = v.dot(self.d);
        let rho = (v - self.d.scale(t)).norm();
        let q = self.o2 + self.du.scale(t) + self.eu.scale(rho);
        let mut f = self.region.eval(q);
        if let Some(w) = self.wedge {
            // Half-planes through the axis: `phi <= pi` side of `e1` and
            // the `phi <= theta` side of the end direction, as exact
            // signed distances (negative on the kept side).
            let h1 = -v.dot(self.e2);
            let h2 = -v.dot(self.e1.scale(w.s) - self.e2.scale(w.c));
            let fw = match w.convex {
                Some(true) => h1.max(h2),
                Some(false) => h1.min(h2),
                None => h1.max(h2).hull(h1.min(h2)),
            };
            f = f.max(fw);
        }
        f
    }

    fn bounds(&self) -> Bounds {
        let mut b = Bounds::EMPTY;
        let r = Iv::new(-self.rmax, self.rmax);
        for t in [self.tmin, self.tmax] {
            let c = self.a + self.d.scale(Iv::point(t));
            b.add(Iv3::new(c.x + r, c.y + r, c.z + r));
        }
        b
    }
}

/// A convex prism over the polygon `{q : n_i . q >= c_i}` (unit inward
/// normals; `verts[i]` where line `i - 1` meets line `i`) and `w` in
/// `[w0, w1]`, in `frame`, opened by a ball: the points within `r` of
/// the prism, which itself is the original prism shrunk by `r`. `rb`,
/// `ow0`, `ow1` are the original prism's extent, for bounds.
#[derive(Debug, Clone)]
struct Opening {
    frame: IFrame,
    lines: Vec<(Iv2, Iv)>,
    verts: Vec<Iv2>,
    w0: Iv,
    w1: Iv,
    r: f64,
    rb: [f64; 4],
    ow0: f64,
    ow1: f64,
}

impl Opening {
    /// `sd(P') - r` with `sd(P')` the exact signed distance to the shrunk
    /// prism: inside it the larger of the polygon's and the slab's signed
    /// distances, outside `sqrt(a+^2 + b+^2)` of the two (the distance to
    /// a product of convex sets). That function of `(a, b)` is continuous
    /// and non-decreasing in both, so its interval is its value at the
    /// lower and at the upper ends.
    fn eval(&self, p: Iv3) -> Iv {
        let (u, v, w) = self.frame.local(p);
        let q = Iv2::new(u, v);
        let mut m = Iv::point(f64::NEG_INFINITY);
        for (n, c) in &self.lines {
            m = m.max(*c - n.dot(q));
        }
        let poly = if m.hi < 0.0 {
            m
        } else {
            let k = self.verts.len();
            let mut d = Iv::point(f64::INFINITY);
            for i in 0..k {
                d = d.min(region::segment_distance(
                    self.verts[i],
                    self.verts[(i + 1) % k],
                    q,
                ));
            }
            if m.lo > 0.0 {
                d
            } else {
                m.hull(d)
            }
        };
        let slab = (self.w0 - w).max(w - self.w1);
        let g = |a: f64, b: f64, up: bool| -> f64 {
            if a <= 0.0 && b <= 0.0 {
                a.max(b)
            } else {
                let s = (Iv::point(a.max(0.0)).sqr() + Iv::point(b.max(0.0)).sqr()).sqrt();
                if up {
                    s.hi
                } else {
                    s.lo
                }
            }
        };
        Iv::new(g(poly.lo, slab.lo, false), g(poly.hi, slab.hi, true)) - self.r
    }
}

/// An axis-aligned box, possibly unbounded or empty.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Bounds {
    pub min: [f64; 3],
    pub max: [f64; 3],
}

impl Bounds {
    pub const EMPTY: Bounds = Bounds {
        min: [f64::INFINITY; 3],
        max: [f64::NEG_INFINITY; 3],
    };
    pub const ALL: Bounds = Bounds {
        min: [f64::NEG_INFINITY; 3],
        max: [f64::INFINITY; 3],
    };

    fn add(&mut self, p: Iv3) {
        for (i, c) in [p.x, p.y, p.z].into_iter().enumerate() {
            self.min[i] = self.min[i].min(c.lo);
            self.max[i] = self.max[i].max(c.hi);
        }
    }

    pub fn is_empty(&self) -> bool {
        (0..3).any(|i| self.min[i] > self.max[i])
    }

    pub fn is_bounded(&self) -> bool {
        self.min
            .iter()
            .chain(self.max.iter())
            .all(|c| c.is_finite())
    }

    fn hull(&self, o: &Bounds) -> Bounds {
        Bounds {
            min: [0, 1, 2].map(|i| self.min[i].min(o.min[i])),
            max: [0, 1, 2].map(|i| self.max[i].max(o.max[i])),
        }
    }

    fn meet(&self, o: &Bounds) -> Bounds {
        Bounds {
            min: [0, 1, 2].map(|i| self.min[i].max(o.min[i])),
            max: [0, 1, 2].map(|i| self.max[i].min(o.max[i])),
        }
    }
}

impl Node {
    fn eval(&self, p: Iv3) -> Iv {
        match self {
            Node::Empty => Iv::point(f64::INFINITY),
            Node::Prism {
                frame,
                region,
                w0,
                w1,
            } => {
                let (u, v, w) = frame.local(p);
                let slab = (*w0 - w).max(w - *w1);
                region.eval(Iv2::new(u, v)).max(slab)
            }
            Node::Revolve(rv) => rv.eval(p),
            Node::Opening(o) => o.eval(p),
            Node::Polytope { planes, .. } => {
                let mut f = Iv::point(f64::NEG_INFINITY);
                for pl in planes {
                    f = f.max(pl.eval(p));
                }
                f
            }
            Node::Cylinder { a, d, r, t0, t1 } => {
                let v = p - *a;
                let t = v.dot(*d);
                let rho = v.cross(*d).norm();
                let mut f = (rho - *r).max(Iv::point(*t0) - t);
                if let Some(t1) = t1 {
                    f = f.max(t - *t1);
                }
                f
            }
            Node::Half(l) => l.eval(p),
            Node::Union(a, b) => a.eval(p).min(b.eval(p)),
            Node::Inter(a, b) => a.eval(p).max(b.eval(p)),
            Node::Diff(a, b) => a.eval(p).max(-b.eval(p)),
        }
    }

    /// The frame of the first sketch plane in tree order.
    fn first_frame(&self) -> Option<IFrame> {
        match self {
            Node::Prism { frame, .. } | Node::Polytope { frame, .. } => Some(*frame),
            Node::Revolve(rv) => Some(rv.frame),
            Node::Opening(o) => Some(o.frame),
            Node::Union(a, b) | Node::Inter(a, b) | Node::Diff(a, b) => {
                a.first_frame().or_else(|| b.first_frame())
            }
            _ => None,
        }
    }

    /// The node for the set moved by `m`.
    fn moved(self, m: &Motion) -> Node {
        let frame = |f: IFrame| IFrame {
            origin: m.to_world(f.origin),
            x: m.rotate(f.x),
            y: m.rotate(f.y),
            z: m.rotate(f.z),
        };
        match self {
            Node::Empty => Node::Empty,
            Node::Prism {
                frame: f,
                region,
                w0,
                w1,
            } => Node::Prism {
                frame: frame(f),
                region,
                w0,
                w1,
            },
            Node::Polytope {
                planes,
                frame: f,
                rb,
                w0,
                w1,
            } => Node::Polytope {
                planes: planes.iter().map(|p| p.moved(m)).collect(),
                frame: frame(f),
                rb,
                w0,
                w1,
            },
            Node::Cylinder { a, d, r, t0, t1 } => Node::Cylinder {
                a: m.to_world(a),
                d: m.rotate(d),
                r,
                t0,
                t1,
            },
            Node::Revolve(rv) => Node::Revolve(Box::new(Rev {
                frame: frame(rv.frame),
                a: m.to_world(rv.a),
                d: m.rotate(rv.d),
                e1: m.rotate(rv.e1),
                e2: m.rotate(rv.e2),
                ..*rv
            })),
            Node::Opening(o) => Node::Opening(Box::new(Opening {
                frame: frame(o.frame),
                ..*o
            })),
            Node::Half(l) => Node::Half(l.moved(m)),
            Node::Union(a, b) => Node::Union(Box::new(a.moved(m)), Box::new(b.moved(m))),
            Node::Inter(a, b) => Node::Inter(Box::new(a.moved(m)), Box::new(b.moved(m))),
            Node::Diff(a, b) => Node::Diff(Box::new(a.moved(m)), Box::new(b.moved(m))),
        }
    }

    /// A box that contains the set (conservative).
    fn bounds(&self) -> Bounds {
        match self {
            Node::Empty => Bounds::EMPTY,
            Node::Prism {
                frame,
                region,
                w0,
                w1,
            } => {
                let b = region.bbox;
                frame.world_box([b[0], b[2]], [b[1], b[3]], [w0.lo, w1.hi])
            }
            Node::Polytope {
                frame, rb, w0, w1, ..
            } => frame.world_box([rb[0], rb[2]], [rb[1], rb[3]], [*w0, *w1]),
            Node::Revolve(rv) => rv.bounds(),
            Node::Opening(o) => {
                o.frame
                    .world_box([o.rb[0], o.rb[2]], [o.rb[1], o.rb[3]], [o.ow0, o.ow1])
            }
            Node::Cylinder { .. } | Node::Half(_) => Bounds::ALL,
            Node::Union(a, b) => a.bounds().hull(&b.bounds()),
            Node::Inter(a, b) => a.bounds().meet(&b.bounds()),
            Node::Diff(a, _) => a.bounds(),
        }
    }

    /// An upper bound on the area of this node's boundary inside the
    /// closed box `clip` (mm^2), from the recipe alone (rounded up).
    ///
    /// The boundary of a union, intersection or difference lies in the
    /// union of the operands' boundaries, so the sum over the leaves bounds
    /// it. A leaf: a prism's two caps and its side (`2 A + P h`, area and
    /// perimeter bounded by [`Region::area_bound`] and
    /// [`Region::perimeter_bound`]); a cylinder's side over the part of its
    /// axis the box spans, and its caps; a solid of revolution's surface,
    /// by Pappus at most `2 pi rmax P`, plus two meridian sections. A
    /// convex leaf `C` also has `area(dC n clip) <= area(d(C n clip)) <=
    /// area(d clip)`: a point of `dC` in the box is on the boundary of `C n
    /// clip`, and surface area grows with convex sets. So does its own box,
    /// for a convex leaf inside it. A half-space's plane meets the box in a
    /// flat convex set, whose two sides together have at most the box's
    /// area.
    fn area_bound(&self, clip: &Bounds) -> f64 {
        let box_area = |b: &Bounds| -> Iv {
            let e = [0, 1, 2].map(|i| (Iv::point(b.max[i]) - Iv::point(b.min[i])).max(Iv::ZERO));
            (e[0] * e[1] + e[1] * e[2] + e[2] * e[0]) * 2.0
        };
        let clip_area = box_area(clip).hi;
        let two_pi = Iv::pi() * 2.0;
        match self {
            Node::Empty => 0.0,
            Node::Prism { region, w0, w1, .. } => {
                let h = (*w1 - *w0).abs();
                (Iv::point(region.area_bound()) * 2.0 + Iv::point(region.perimeter_bound()) * h).hi
            }
            Node::Polytope { .. } | Node::Opening(_) => box_area(&self.bounds()).hi.min(clip_area),
            Node::Cylinder { a, d, r, t0, t1 } => {
                let (mut tmin, mut tmax) = (f64::INFINITY, f64::NEG_INFINITY);
                for x in [clip.min[0], clip.max[0]] {
                    for y in [clip.min[1], clip.max[1]] {
                        for z in [clip.min[2], clip.max[2]] {
                            let t = (Iv3::point(x, y, z) - *a).dot(*d);
                            tmin = tmin.min(t.lo);
                            tmax = tmax.max(t.hi);
                        }
                    }
                }
                let lo = t0.max(tmin);
                let hi = t1.unwrap_or(f64::INFINITY).min(tmax);
                let len = (Iv::point(hi) - Iv::point(lo)).max(Iv::ZERO);
                let r = Iv::point(*r);
                let side = two_pi * r * len + two_pi * r * r;
                side.hi.min(clip_area)
            }
            Node::Half(_) => clip_area,
            Node::Revolve(rv) => {
                (two_pi * Iv::point(rv.rmax) * Iv::point(rv.region.perimeter_bound())
                    + Iv::point(rv.region.area_bound()) * 2.0)
                    .hi
            }
            Node::Union(a, b) | Node::Inter(a, b) | Node::Diff(a, b) => {
                (Iv::point(a.area_bound(clip)) + Iv::point(b.area_bound(clip))).hi
            }
        }
    }
}

/// The reference of one part.
#[derive(Debug, Clone)]
pub struct Reference {
    root: Node,
}

/// Bookkeeping for chamfer definedness.
struct ChamferState {
    /// The first feature's id, if it is an adding, untapered extrude.
    base: Option<FeatId>,
    /// Still nothing but `base` and chamfers of it so far.
    pristine: bool,
    /// Selected (segment, cap is the end cap) pairs so far, with distance.
    used: Vec<(usize, bool, f64)>,
}

impl Reference {
    /// The reference of `def`, its tools resolved by `tools`.
    pub fn build(def: &PartDef, tools: &dyn ToolResolver) -> Result<Reference, NotDefined> {
        let mut stack = vec![def.id];
        let root = build_def(def, tools, &mut stack)?;
        Ok(Reference { root })
    }

    /// Encloses the signed function over a box (a point is a degenerate
    /// box).
    pub fn eval(&self, p: Iv3) -> Iv {
        self.root.eval(p)
    }

    /// The certified class of a point.
    pub fn classify(&self, p: Point3) -> Class {
        let f = self.eval(iv3(p));
        if f.hi < 0.0 {
            Class::Inside { margin: -f.hi }
        } else if f.lo > 0.0 {
            Class::Outside { margin: f.lo }
        } else {
            Class::Band
        }
    }

    /// The same solid expressed in the frame of its first sketch plane
    /// (every node moved by the rigid motion from world to that frame, in
    /// interval arithmetic). Volumes are unchanged by a rigid motion, and a
    /// part whose features follow its first plane is axis-aligned there,
    /// which is where a box subdivision is tight.
    pub fn aligned(&self) -> Reference {
        let Some(fr) = self.root.first_frame() else {
            return self.clone();
        };
        let r = [fr.x, fr.y, fr.z];
        let ro = Iv3::new(
            r[0].dot(fr.origin),
            r[1].dot(fr.origin),
            r[2].dot(fr.origin),
        );
        let m = Motion { r, t: -ro };
        Reference {
            root: self.root.clone().moved(&m),
        }
    }

    /// A box that contains the solid, if the reference can bound it.
    pub fn bounds(&self) -> Bounds {
        self.root.bounds()
    }

    /// A certified upper bound on the area of the solid's boundary (mm^2),
    /// from the recipe alone, or `None` when the reference cannot bound
    /// the solid. Leaves are clipped to [`Reference::bounds`]; see
    /// `Node::area_bound` for the argument.
    pub fn area_bound(&self) -> Option<f64> {
        let b = self.bounds();
        if b.is_empty() {
            return Some(0.0);
        }
        b.is_bounded().then(|| self.root.area_bound(&b))
    }
}

fn build_def(
    def: &PartDef,
    tools: &dyn ToolResolver,
    stack: &mut Vec<DefId>,
) -> Result<Node, NotDefined> {
    let mut current = Node::Empty;
    let mut started = false;
    let mut ch = ChamferState {
        base: None,
        pristine: true,
        used: Vec::new(),
    };
    for (i, feature) in def.features.iter().enumerate() {
        let id = feature.id();
        let at = Some(id);
        let is_chamfer = matches!(feature, Feature::Chamfer { .. });
        match feature {
            Feature::Extrude {
                profile,
                plane,
                distance,
                taper_deg,
                op,
                ..
            } => {
                if *taper_deg != 0.0 {
                    return Err(undefined(
                        at,
                        "a tapered extrude has no reference meaning yet",
                    ));
                }
                if !distance.is_finite() || *distance == 0.0 {
                    return Err(invalid(at, "extrude distance must be finite and non-zero"));
                }
                let frame = frame_of(plane).map_err(|e| invalid(at, e))?;
                let region = Region::new(profile).map_err(|e| match e {
                    RegionError::Invalid(s) => invalid(at, s),
                    RegionError::Undefined(s) => undefined(at, s),
                })?;
                let (w0, w1) = (distance.min(0.0), distance.max(0.0));
                let body = match region.convex_outer() {
                    // A convex prism: its edge planes and caps. Outward
                    // distance to edge (a, n): -n . ((u, v) - a).
                    Some(edges) => {
                        let mut planes: Vec<Linear> = edges
                            .iter()
                            .map(|(a, n)| frame.linear([-n.x, -n.y, Iv::ZERO], n.dot(*a)))
                            .collect();
                        planes.push(
                            frame.linear([Iv::ZERO, Iv::ZERO, Iv::point(1.0)], Iv::point(-w1)),
                        );
                        planes.push(
                            frame.linear([Iv::ZERO, Iv::ZERO, Iv::point(-1.0)], Iv::point(w0)),
                        );
                        Node::Polytope {
                            planes,
                            frame,
                            rb: region.bbox,
                            w0,
                            w1,
                        }
                    }
                    None => Node::Prism {
                        frame,
                        region: Arc::new(region),
                        w0: Iv::point(w0),
                        w1: Iv::point(w1),
                    },
                };
                if i == 0 && *op == Op::Add {
                    ch.base = Some(id);
                }
                current = combine(current, body, *op, started);
                started = true;
            }
            Feature::Hole {
                at: entry,
                dir,
                dia,
                depth,
                cbore,
                ..
            } => {
                let d = Iv3::point(dir.x, dir.y, dir.z);
                let len = (dir.x * dir.x + dir.y * dir.y + dir.z * dir.z).sqrt();
                if !entry.is_finite() || !(len.is_finite() && len > 1e-12) {
                    return Err(invalid(
                        at,
                        "hole position and direction must be finite and non-zero",
                    ));
                }
                if !dia.is_finite() || *dia <= 0.0 {
                    return Err(invalid(at, "hole diameter must be finite and positive"));
                }
                let t1 = match depth {
                    Depth::Blind(x) if x.is_finite() && *x > 0.0 => Some(*x),
                    Depth::Blind(_) => {
                        return Err(invalid(at, "hole depth must be finite and positive"))
                    }
                    Depth::Through => None,
                };
                let d = d.scale(Iv::point(1.0) / d.norm());
                let a = iv3(*entry);
                let mut removed = Node::Cylinder {
                    a,
                    d,
                    r: dia / 2.0,
                    t0: 0.0,
                    t1,
                };
                if let Some((cd, cdepth)) = cbore {
                    let too_deep = matches!(t1, Some(x) if *cdepth >= x);
                    if !cd.is_finite()
                        || *cd <= *dia
                        || !cdepth.is_finite()
                        || *cdepth <= 0.0
                        || too_deep
                    {
                        return Err(invalid(
                            at,
                            "a counterbore needs a larger diameter and a positive depth less than the hole's",
                        ));
                    }
                    removed = Node::Union(
                        Box::new(removed),
                        Box::new(Node::Cylinder {
                            a,
                            d,
                            r: cd / 2.0,
                            t0: 0.0,
                            t1: Some(*cdepth),
                        }),
                    );
                }
                current = Node::Diff(Box::new(current), Box::new(removed));
                started = true;
            }
            Feature::Boolean { op, tool, .. } => match tool {
                ToolRef::Def(tool_id, placement) => {
                    let motion = Motion::of(placement).map_err(|e| invalid(at, e))?;
                    if stack.contains(tool_id) {
                        return Err(invalid(
                            at,
                            format!("tool {} refers back to itself", tool_id.0),
                        ));
                    }
                    let Some(tool_def) = tools.def(*tool_id) else {
                        return Err(invalid(at, format!("tool {} is not known", tool_id.0)));
                    };
                    stack.push(*tool_id);
                    let body = build_def(tool_def, tools, stack);
                    stack.pop();
                    let body = body?.moved(&motion);
                    current = combine(current, body, *op, started);
                    started = true;
                }
                ToolRef::Part { .. } => {
                    return Err(undefined(
                        at,
                        "a scene part as a tool has no reference meaning yet",
                    ));
                }
            },
            Feature::Chamfer { edges, d1, d2, .. } => {
                let keep = chamfer(def, i, edges, *d1, *d2, &mut ch)?;
                for k in keep {
                    current = Node::Inter(Box::new(current), Box::new(k));
                }
            }
            Feature::Revolve {
                profile,
                plane,
                axis,
                angle_deg,
                op,
                ..
            } => {
                let body = revolve(profile, plane, axis, *angle_deg).map_err(|e| e.at(at))?;
                current = combine(current, body, *op, started);
                started = true;
            }
            Feature::Loft { sections, op, .. } => {
                let body = loft(sections).map_err(|e| e.at(at))?;
                current = combine(current, body, *op, started);
                started = true;
            }
            Feature::Sweep {
                profile,
                plane,
                path,
                op,
                ..
            } => {
                let body = sweep(profile, plane, path).map_err(|e| e.at(at))?;
                current = combine(current, body, *op, started);
                started = true;
            }
            Feature::Fillet { edges, radius, .. } => {
                current = fillet(def, i, edges, *radius, &ch)?;
            }
        }
        if i > 0 && !is_chamfer {
            ch.pristine = false;
        }
    }
    Ok(current)
}

fn combine(current: Node, body: Node, op: Op, started: bool) -> Node {
    if !started {
        return match op {
            Op::Add => body,
            Op::Subtract | Op::Intersect => Node::Empty,
        };
    }
    match op {
        Op::Add => Node::Union(Box::new(current), Box::new(body)),
        Op::Subtract => Node::Diff(Box::new(current), Box::new(body)),
        Op::Intersect => Node::Inter(Box::new(current), Box::new(body)),
    }
}

/// How far (mm) a profile may reach past a revolve axis and still count as
/// lying on one side of it; the part past the axis is not revolved.
const AXIS_SIDE_TOL: f64 = 1e-9;

fn region_of(profile: &cadview_core::solid::Profile2d) -> Result<Region, NotDefined> {
    Region::new(profile).map_err(|e| match e {
        RegionError::Invalid(s) => invalid(None, s),
        RegionError::Undefined(s) => undefined(None, s),
    })
}

fn neg2(v: Iv2) -> Iv2 {
    Iv2::new(-v.x, -v.y)
}

/// The region's box corners in meridian coordinates `t = (q - o2) . du`,
/// `s = (q - o2) . eu`: `(tmin, tmax, smin, smax)`, rounded outward.
fn meridian_extent(region: &Region, o2: Iv2, du: Iv2, eu: Iv2) -> (f64, f64, f64, f64) {
    let b = region.bbox;
    let mut e = (
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::INFINITY,
        f64::NEG_INFINITY,
    );
    for (x, y) in [(b[0], b[1]), (b[2], b[1]), (b[2], b[3]), (b[0], b[3])] {
        let q = Iv2::point(x, y) - o2;
        let (t, s) = (q.dot(du), q.dot(eu));
        e = (e.0.min(t.lo), e.1.max(t.hi), e.2.min(s.lo), e.3.max(s.hi));
    }
    e
}

/// A revolve's reference: the module note's definition.
fn revolve(
    profile: &cadview_core::solid::Profile2d,
    plane: &Plane3,
    axis: &kurbo::Line,
    angle_deg: f64,
) -> Result<Node, NotDefined> {
    if !(angle_deg.is_finite() && angle_deg > 0.0 && angle_deg <= 360.0) {
        return Err(invalid(None, "revolve angle must be in (0, 360] degrees"));
    }
    let frame = frame_of(plane).map_err(|e| invalid(None, e))?;
    let region = region_of(profile)?;
    let (p0, p1) = (axis.p0, axis.p1);
    if ![p0.x, p0.y, p1.x, p1.y].iter().all(|c| c.is_finite()) || p0 == p1 {
        return Err(invalid(
            None,
            "revolve axis must be two distinct finite points",
        ));
    }
    let o2 = Iv2::point(p0.x, p0.y);
    let dir = Iv2::point(p1.x, p1.y) - o2;
    let du = dir.scale(Iv::point(1.0) / dir.norm());
    let perp = Iv2::new(-du.y, du.x);
    let (_, _, smin, smax) = meridian_extent(&region, o2, du, perp);
    let eu = if smin >= -AXIS_SIDE_TOL {
        perp
    } else if smax <= AXIS_SIDE_TOL {
        neg2(perp)
    } else {
        return Err(undefined(
            None,
            "the profile lies on both sides of the revolve axis",
        ));
    };
    let (tmin, tmax, _, smax) = meridian_extent(&region, o2, du, eu);
    let lift = |v: Iv2| frame.x.scale(v.x) + frame.y.scale(v.y);
    let a = frame.origin + lift(o2);
    let d = lift(du);
    let e1 = lift(eu);
    let wedge = (angle_deg != 360.0).then(|| {
        let theta = Iv::point(angle_deg) * Iv::pi() / Iv::point(180.0);
        Wedge {
            s: theta.sin(),
            c: theta.cos(),
            convex: Some(angle_deg <= 180.0),
        }
    });
    Ok(Node::Revolve(Box::new(Rev {
        frame,
        a,
        d,
        e1,
        e2: d.cross(e1),
        o2,
        du,
        eu,
        region: Arc::new(region),
        wedge,
        tmin,
        tmax,
        rmax: smax.max(0.0),
    })))
}

/// A polygon's vertices and whether it runs counter-clockwise.
type Polygon = (Vec<(f64, f64)>, bool);

/// The start points of a profile that is one convex polygon of lines (no
/// holes), and whether it runs counter-clockwise; or why not.
fn convex_polygon(profile: &cadview_core::solid::Profile2d) -> Result<Polygon, &'static str> {
    if !profile.holes.is_empty() {
        return Err("the profile has a hole");
    }
    let mut pts = Vec::with_capacity(profile.outer.len());
    for s in &profile.outer {
        match s {
            Seg2::Line(l) => pts.push((l.p0.x, l.p0.y)),
            _ => return Err("the profile has a curved segment"),
        }
    }
    let n = pts.len();
    if n < 3 {
        return Err("the profile is not a polygon");
    }
    let area2: f64 = (0..n)
        .map(|k| {
            let (a, b) = (pts[k], pts[(k + 1) % n]);
            a.0 * b.1 - a.1 * b.0
        })
        .sum();
    let ccw = area2 > 0.0;
    for k in 0..n {
        let (a, b, c) = (pts[k], pts[(k + 1) % n], pts[(k + 2) % n]);
        let cr = (b.0 - a.0) * (c.1 - b.1) - (b.1 - a.1) * (c.0 - b.0);
        let scale = ((b.0 - a.0).hypot(b.1 - a.1)) * ((c.0 - b.0).hypot(c.1 - b.1));
        if cr * if ccw { 1.0 } else { -1.0 } <= 1e-9 * scale {
            return Err("the profile has a concave or straight corner");
        }
    }
    Ok((pts, ccw))
}

/// A loft's reference: defined for two sections on planes with the same
/// axes (so parallel, one moved along the other), each one convex polygon
/// of lines with the same number of sides, side `i` of one exactly
/// parallel to and in the same direction as side `i` of the other
/// (certified in interval arithmetic, so the inputs must make it exact).
/// Then every side face of the ruled loft is a plane, and the solid is the
/// convex hull of the two polygons: the intersection of the side planes'
/// and the caps' half-spaces, a convex polytope.
fn loft(sections: &[(cadview_core::solid::Profile2d, Plane3)]) -> Result<Node, NotDefined> {
    let nd = |why: &str| undefined(None, format!("loft: {why}"));
    let [(pa, pla), (pb, plb)] = sections else {
        return Err(nd("defined only for two sections"));
    };
    if pla.x_axis != plb.x_axis || pla.normal != plb.normal {
        return Err(nd("defined only for sections on planes with the same axes"));
    }
    let fa = frame_of(pla).map_err(|e| invalid(None, e))?;
    let fb = frame_of(plb).map_err(|e| invalid(None, e))?;
    let (a, ccw_a) = convex_polygon(pa).map_err(nd)?;
    let (b, ccw_b) = convex_polygon(pb).map_err(nd)?;
    if a.len() != b.len() || ccw_a != ccw_b {
        return Err(nd("the sections differ in side count or in orientation"));
    }
    let n = a.len();
    for i in 0..n {
        let j = (i + 1) % n;
        let ea = (
            Iv::point(a[j].0) - Iv::point(a[i].0),
            Iv::point(a[j].1) - Iv::point(a[i].1),
        );
        let eb = (
            Iv::point(b[j].0) - Iv::point(b[i].0),
            Iv::point(b[j].1) - Iv::point(b[i].1),
        );
        let cross = ea.0 * eb.1 - ea.1 * eb.0;
        let dot = ea.0 * eb.0 + ea.1 * eb.1;
        if cross != Iv::ZERO || !dot.pos() {
            return Err(nd(
                "corresponding sides are not certainly parallel: the side faces are not planes",
            ));
        }
    }
    let lift = |f: &IFrame, p: (f64, f64)| {
        f.origin + f.x.scale(Iv::point(p.0)) + f.y.scale(Iv::point(p.1))
    };
    let va: Vec<Iv3> = a.iter().map(|p| lift(&fa, *p)).collect();
    let vb: Vec<Iv3> = b.iter().map(|p| lift(&fb, *p)).collect();
    let h = (vb[0] - va[0]).dot(fa.z);
    let up = h
        .sign()
        .ok_or_else(|| nd("the sections are not certainly in two planes"))?;
    let mut centre = Iv3::point(0.0, 0.0, 0.0);
    for p in va.iter().chain(&vb) {
        centre = centre + *p;
    }
    let centre = centre.scale(Iv::point(1.0) / Iv::point(2.0 * n as f64));
    let mut planes = Vec::with_capacity(n + 2);
    for i in 0..n {
        let j = (i + 1) % n;
        let nrm = (va[j] - va[i]).cross(vb[i] - va[i]);
        let g = nrm.scale(Iv::point(1.0) / nrm.norm());
        let g = match g.dot(centre - va[i]).sign() {
            Some(true) => -g,
            Some(false) => g,
            None => return Err(nd("a side face is not certainly oriented")),
        };
        planes.push(Linear { g, c: g.dot(va[i]) });
    }
    let z = if up { fa.z } else { -fa.z };
    planes.push(Linear {
        g: -z,
        c: (-z).dot(va[0]),
    });
    planes.push(Linear {
        g: z,
        c: z.dot(vb[0]),
    });
    // Bounds in the first section's frame.
    let (mut rb, mut w0, mut w1) = (
        [
            f64::INFINITY,
            f64::INFINITY,
            f64::NEG_INFINITY,
            f64::NEG_INFINITY,
        ],
        f64::INFINITY,
        f64::NEG_INFINITY,
    );
    for p in va.iter().chain(&vb) {
        let (u, v, w) = fa.local(*p);
        rb = [
            rb[0].min(u.lo),
            rb[1].min(v.lo),
            rb[2].max(u.hi),
            rb[3].max(v.hi),
        ];
        w0 = w0.min(w.lo);
        w1 = w1.max(w.hi);
    }
    Ok(Node::Polytope {
        planes,
        frame: fa,
        rb,
        w0,
        w1,
    })
}

/// A sweep's reference: the module note's definition, piece by piece.
fn sweep(
    profile: &cadview_core::solid::Profile2d,
    plane: &Plane3,
    path: &cadview_core::solid::Path3,
) -> Result<Node, NotDefined> {
    path.check(plane)
        .map_err(|e| invalid(None, e.to_string()))?;
    let region = Arc::new(region_of(profile)?);
    let mut fr = frame_of(plane).map_err(|e| invalid(None, e))?;
    let mut body: Option<Node> = None;
    for seg in &path.segs {
        let piece = match seg {
            Seg3::Line { from, to } => {
                let len = (iv3(*to) - iv3(*from)).norm();
                let n = Node::Prism {
                    frame: fr,
                    region: region.clone(),
                    w0: Iv::ZERO,
                    w1: len,
                };
                fr.origin = fr.origin + fr.z.scale(len);
                n
            }
            Seg3::Arc { from, via, to } => {
                let (n, next) = sweep_arc(&fr, &region, *from, *via, *to)?;
                fr = next;
                n
            }
        };
        body = Some(match body {
            None => piece,
            Some(b) => Node::Union(Box::new(b), Box::new(piece)),
        });
    }
    body.ok_or_else(|| invalid(None, "a sweep path has no segments"))
}

/// One arc of a sweep: the revolve of the section about the arc's axis by
/// the arc's angle, and the section's frame at the arc's end.
fn sweep_arc(
    fr: &IFrame,
    region: &Arc<Region>,
    from: Point3,
    via: Point3,
    to: Point3,
) -> Result<(Node, IFrame), NotDefined> {
    let nd = |why: &str| undefined(None, format!("sweep: {why}"));
    let (pa, pb, pc) = (iv3(from), iv3(via), iv3(to));
    let (u, v) = (pb - pa, pc - pa);
    let w = u.cross(v);
    let w2 = w.norm2();
    if w2.lo <= 0.0 {
        return Err(nd("an arc's points are not certainly off a line"));
    }
    // The circumcentre; the points run counter-clockwise about w.
    let c =
        pa + (v.cross(w).scale(u.norm2()) + w.cross(u).scale(v.norm2())).scale(Iv::point(0.5) / w2);
    let b = w.scale(Iv::point(1.0) / w.norm());
    let flat = |r: Iv3| {
        let p = r - b.scale(r.dot(b));
        p.scale(Iv::point(1.0) / p.norm())
    };
    let (es, ee) = (flat(pa - c), flat(pc - c));
    let (cos_t, sin_t) = (es.dot(ee), es.cross(ee).dot(b));
    // The meridian in the section's own coordinates.
    let co = c - fr.origin;
    let c2 = Iv2::new(co.dot(fr.x), co.dot(fr.y));
    let bu = Iv2::new(b.dot(fr.x), b.dot(fr.y));
    let bu = bu.scale(Iv::point(1.0) / bu.norm());
    let perp = Iv2::new(-bu.y, bu.x);
    // `eu` points from the axis towards the section's origin.
    let eu = match (-c2.dot(perp)).sign() {
        Some(true) => perp,
        Some(false) => neg2(perp),
        None => return Err(nd("the section's origin is on the arc's axis")),
    };
    let (tmin, tmax, smin, smax) = meridian_extent(region, c2, bu, eu);
    if smin < -AXIS_SIDE_TOL {
        return Err(nd(
            "the section reaches across the arc's axis (the bend is tighter than the section)",
        ));
    }
    let e1 = fr.x.scale(eu.x) + fr.y.scale(eu.y);
    let node = Node::Revolve(Box::new(Rev {
        frame: *fr,
        a: c,
        d: b,
        e1,
        e2: b.cross(e1),
        o2: c2,
        du: bu,
        eu,
        region: region.clone(),
        wedge: Some(Wedge {
            s: sin_t,
            c: cos_t,
            convex: sin_t.sign(),
        }),
        tmin,
        tmax,
        rmax: smax.max(0.0),
    }));
    let one = Iv::point(1.0);
    let rot = |x: Iv3| x.scale(cos_t) + b.cross(x).scale(sin_t) + b.scale(b.dot(x) * (one - cos_t));
    let next = IFrame {
        origin: c + rot(fr.origin - c),
        x: rot(fr.x),
        y: rot(fr.y),
        z: rot(fr.z),
    };
    Ok((node, next))
}

/// A fillet's reference: the module note's definition.
fn fillet(
    def: &PartDef,
    index: usize,
    edges: &EdgeSel,
    r: f64,
    st: &ChamferState,
) -> Result<Node, NotDefined> {
    let at = Some(def.features[index].id());
    let nd = |why: &str| undefined(at, format!("fillet: {why}"));
    if !(r.is_finite() && r > 0.0) {
        return Err(invalid(at, "fillet radius must be finite and positive"));
    }
    if !matches!(edges, EdgeSel::All) {
        return Err(nd("only EdgeSel::All has a reference meaning"));
    }
    let Some(Feature::Extrude {
        profile,
        plane,
        distance: h,
        ..
    }) = def.features.first()
    else {
        return Err(nd("defined only on a first, adding extrude"));
    };
    if index != 1 || st.base != Some(def.features[0].id()) {
        return Err(nd(
            "defined only directly after the part's first, adding, untapered extrude",
        ));
    }
    let (pts, ccw) = convex_polygon(profile).map_err(nd)?;
    if 2.0 * r >= h.abs() - 1e-6 {
        return Err(nd("the round uses the whole height of the extrude"));
    }
    let n = pts.len();
    let lens = offset_polygon_edge_lengths(&pts, ccw, &vec![r; n]);
    if lens.iter().any(|&l| l.is_nan() || l <= 1e-6) {
        return Err(nd(
            "the round is too large for the profile: a side of the shrunk polygon vanishes",
        ));
    }
    let frame = frame_of(plane).map_err(|e| invalid(at, e))?;
    let lines: Vec<(Iv2, Iv)> = (0..n)
        .map(|k| {
            let (a, b) = (pts[k], pts[(k + 1) % n]);
            let dir = Iv2::point(b.0, b.1) - Iv2::point(a.0, a.1);
            let nrm = if ccw {
                Iv2::new(-dir.y, dir.x)
            } else {
                Iv2::new(dir.y, -dir.x)
            };
            let nrm = nrm.scale(Iv::point(1.0) / dir.norm());
            (nrm, nrm.dot(Iv2::point(a.0, a.1)) + r)
        })
        .collect();
    // Vertex k: where line k - 1 meets line k (Cramer's rule).
    let mut verts = Vec::with_capacity(n);
    for k in 0..n {
        let (n1, c1) = lines[(k + n - 1) % n];
        let (n2, c2) = lines[k];
        let det = n1.cross(n2);
        if det.contains_zero() {
            return Err(nd(
                "two sides of the shrunk polygon are not certainly apart",
            ));
        }
        verts.push(Iv2::new(
            (c1 * n2.y - c2 * n1.y) / det,
            (n1.x * c2 - n2.x * c1) / det,
        ));
    }
    let (w0, w1) = (h.min(0.0), h.max(0.0));
    let region = Region::new(profile).map_err(|e| match e {
        RegionError::Invalid(s) => invalid(at, s),
        RegionError::Undefined(s) => undefined(at, s),
    })?;
    Ok(Node::Opening(Box::new(Opening {
        frame,
        lines,
        verts,
        w0: Iv::point(w0) + r,
        w1: Iv::point(w1) - r,
        r,
        rb: region.bbox,
        ow0: w0,
        ow1: w1,
    })))
}

/// The kept half-spaces of a chamfer, or why it is not defined.
fn chamfer(
    def: &PartDef,
    index: usize,
    edges: &EdgeSel,
    d1: f64,
    d2: f64,
    st: &mut ChamferState,
) -> Result<Vec<Node>, NotDefined> {
    let id = def.features[index].id();
    let at = Some(id);
    let nd = |why: &str| undefined(at, format!("chamfer: {why}"));
    if d1 != d2 {
        return Err(nd(
            "an asymmetric chamfer (d1 != d2) has no reference meaning",
        ));
    }
    let d = d1;
    if !d.is_finite() || d <= 0.0 {
        return Err(invalid(at, "chamfer distance must be finite and positive"));
    }
    let EdgeSel::Cap {
        feature,
        side,
        segs,
    } = edges
    else {
        return Err(nd("only a Cap selector has a reference meaning"));
    };
    if st.base != Some(*feature) || !st.pristine {
        return Err(nd(
            "defined only on the first (adding) extrude, before any feature other than its chamfers",
        ));
    }
    let Some(Feature::Extrude {
        profile,
        plane,
        distance: h,
        ..
    }) = def.features.first()
    else {
        return Err(nd("the selected feature is not an extrude"));
    };
    let outer_n = profile.outer.len();
    let total = profile.segment_count();
    let selected: Vec<usize> = match segs {
        None => (0..total).collect(),
        Some(list) => list.clone(),
    };
    if selected.iter().any(|&s| s >= total) {
        return Err(invalid(at, "chamfer selects a segment that does not exist"));
    }
    if selected.iter().any(|&s| s >= outer_n) {
        return Err(nd(
            "a chamfer of a hole loop's edge (a concave corner of the material)",
        ));
    }
    // The outer loop: lines only, strictly convex.
    let mut pts = Vec::with_capacity(outer_n);
    for s in &profile.outer {
        match s {
            Seg2::Line(l) => pts.push((l.p0.x, l.p0.y)),
            _ => return Err(nd("the profile has a curved segment")),
        }
    }
    let n = pts.len();
    if n < 3 {
        return Err(nd("the profile is not a polygon"));
    }
    let area2: f64 = (0..n)
        .map(|k| {
            let (a, b) = (pts[k], pts[(k + 1) % n]);
            a.0 * b.1 - a.1 * b.0
        })
        .sum();
    let ccw = area2 > 0.0;
    for k in 0..n {
        let (a, b, c) = (pts[k], pts[(k + 1) % n], pts[(k + 2) % n]);
        let cr = (b.0 - a.0) * (c.1 - b.1) - (b.1 - a.1) * (c.0 - b.0);
        let scale = ((b.0 - a.0).hypot(b.1 - a.1)) * ((c.0 - b.0).hypot(c.1 - b.1));
        let turns_right_way = cr * if ccw { 1.0 } else { -1.0 } > 1e-9 * scale;
        if !turns_right_way {
            return Err(nd("the profile has a concave or straight corner"));
        }
    }
    // Caps and depth direction.
    let sign = h.signum();
    let caps: Vec<(bool, f64, f64)> = match side {
        CapSide::Start => vec![(false, 0.0, sign)],
        CapSide::End => vec![(true, *h, -sign)],
        CapSide::Both => vec![(false, 0.0, sign), (true, *h, -sign)],
    };
    for &(end, _, _) in &caps {
        for &s in &selected {
            if st.used.iter().any(|u| u.0 == s && u.1 == end) {
                return Err(nd("the same edge is chamfered twice"));
            }
            st.used.push((s, end, d));
        }
    }
    let max_on = |end: bool| {
        st.used
            .iter()
            .filter(|u| u.1 == end)
            .map(|u| u.2)
            .fold(0.0, f64::max)
    };
    if max_on(false) + max_on(true) >= h.abs() {
        return Err(nd("the chamfers use the whole height of the extrude"));
    }
    // Every edge of each cap polygon keeps a length after the offsets.
    for end in [false, true] {
        let offs: Vec<f64> = (0..n)
            .map(|k| {
                st.used
                    .iter()
                    .filter(|u| u.0 == k && u.1 == end)
                    .map(|u| u.2)
                    .fold(0.0, f64::max)
            })
            .collect();
        if offs.iter().all(|&o| o == 0.0) {
            continue;
        }
        let lens = offset_polygon_edge_lengths(&pts, ccw, &offs);
        if lens.iter().any(|&l| l.is_nan() || l <= 1e-6) {
            return Err(nd(
                "the chamfer is too large for the profile: an edge of the cap vanishes",
            ));
        }
    }
    let frame = frame_of(plane).map_err(|e| invalid(at, e))?;
    let mut out = Vec::new();
    for &(_, w_cap, sigma) in &caps {
        for &s in &selected {
            let (a, b) = (pts[s], pts[(s + 1) % n]);
            let dir = Iv2::point(b.0, b.1) - Iv2::point(a.0, a.1);
            let len = dir.norm();
            // Inward normal: left of a -> b for a counter-clockwise loop.
            let nrm = if ccw {
                Iv2::new(-dir.y, dir.x)
            } else {
                Iv2::new(dir.y, -dir.x)
            };
            let nrm = nrm.scale(Iv::point(1.0) / len);
            // Kept: s + t >= d with s = n . ((u, v) - a) and t = sigma (w -
            // w_cap); f = (d - s - t) / sqrt(2), a unit-gradient linear form.
            let r2 = Iv::point(2.0).sqrt();
            let k = (nrm.dot(Iv2::point(a.0, a.1)) + Iv::point(sigma * w_cap) + Iv::point(d)) / r2;
            let inv = Iv::point(-1.0) / r2;
            out.push(Node::Half(
                frame.linear([nrm.x * inv, nrm.y * inv, Iv::point(sigma) * inv], k),
            ));
        }
    }
    Ok(out)
}

/// The edge lengths of a convex polygon after moving edge `k` inward by
/// `offs[k]` (floating point; a definedness check with a margin).
fn offset_polygon_edge_lengths(pts: &[(f64, f64)], ccw: bool, offs: &[f64]) -> Vec<f64> {
    let n = pts.len();
    // Each edge as a line: point and unit direction, moved inward.
    let lines: Vec<((f64, f64), (f64, f64))> = (0..n)
        .map(|k| {
            let (a, b) = (pts[k], pts[(k + 1) % n]);
            let len = (b.0 - a.0).hypot(b.1 - a.1);
            let dir = ((b.0 - a.0) / len, (b.1 - a.1) / len);
            let nrm = if ccw {
                (-dir.1, dir.0)
            } else {
                (dir.1, -dir.0)
            };
            ((a.0 + nrm.0 * offs[k], a.1 + nrm.1 * offs[k]), dir)
        })
        .collect();
    let meet = |l1: ((f64, f64), (f64, f64)), l2: ((f64, f64), (f64, f64))| {
        let ((p, r), (q, s)) = (l1, l2);
        let den = r.0 * s.1 - r.1 * s.0;
        let t = ((q.0 - p.0) * s.1 - (q.1 - p.1) * s.0) / den;
        (p.0 + t * r.0, p.1 + t * r.1)
    };
    // Vertex k is where edge k-1 meets edge k.
    let verts: Vec<(f64, f64)> = (0..n)
        .map(|k| meet(lines[(k + n - 1) % n], lines[k]))
        .collect();
    (0..n)
        .map(|k| {
            let (a, b) = (verts[k], verts[(k + 1) % n]);
            // Signed along the edge's own direction: a reversed edge has
            // vanished.
            let dir = lines[k].1;
            (b.0 - a.0) * dir.0 + (b.1 - a.1) * dir.1
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use cadview_core::solid::corpus::prismatic;
    use cadview_core::solid::{NoTools, Vec3};

    fn is_in(r: &Reference, x: f64, y: f64, z: f64) -> bool {
        matches!(r.classify(Point3::new(x, y, z)), Class::Inside { .. })
    }
    fn is_out(r: &Reference, x: f64, y: f64, z: f64) -> bool {
        matches!(r.classify(Point3::new(x, y, z)), Class::Outside { .. })
    }

    #[test]
    fn corpus_points_classify_by_hand() {
        let e = prismatic::board_with_through_hole();
        let r = Reference::build(&e.parts[0].def, &e).unwrap();
        assert!(is_in(&r, 10.0, 10.0, 9.0));
        assert!(is_out(&r, 50.0, 50.0, 9.0)); // in the hole
        assert!(is_out(&r, 50.0, 54.9, 9.0));
        assert!(is_in(&r, 50.0, 55.1, 9.0));
        assert!(is_out(&r, 10.0, 10.0, 18.1));
        match r.classify(Point3::new(10.0, 10.0, 17.0)) {
            Class::Inside { margin } => assert!((margin - 1.0).abs() < 1e-12),
            c => panic!("{c:?}"),
        }
        assert_eq!(r.classify(Point3::new(10.0, 10.0, 18.0)), Class::Band);

        let e = prismatic::dado();
        let r = Reference::build(&e.parts[0].def, &e).unwrap();
        assert!(is_out(&r, 300.0, 150.0, 15.0));
        assert!(is_in(&r, 300.0, 150.0, 11.0));
        assert!(is_in(&r, 290.0, 150.0, 15.0));

        let e = prismatic::half_lap();
        let r = Reference::build(&e.parts[0].def, &e).unwrap();
        assert!(is_out(&r, 200.0, 25.0, 15.0));
        assert!(is_in(&r, 200.0, 25.0, 5.0));
        assert!(is_in(&r, 170.0, 25.0, 15.0));
        assert!(is_in(&r, 230.0, 25.0, 15.0));

        let e = prismatic::counterbored_hole();
        let r = Reference::build(&e.parts[0].def, &e).unwrap();
        assert!(is_out(&r, 50.0, 56.0, 25.0)); // in the counterbore
        assert!(is_in(&r, 50.0, 56.0, 15.0)); // below it, outside the hole
        assert!(is_out(&r, 50.0, 53.0, 1.0));

        let e = prismatic::chamfered_slab();
        let r = Reference::build(&e.parts[0].def, &e).unwrap();
        assert!(is_out(&r, 100.0, 0.5, 19.8));
        assert!(is_in(&r, 100.0, 2.0, 18.5)); // 2 + 1.5 >= 3
        assert!(is_out(&r, 100.0, 2.0, 19.5)); // 2 + 0.5 < 3
        assert!(is_out(&r, 0.5, 0.5, 19.0)); // mitre corner
        assert!(is_in(&r, 0.5, 0.5, 16.0));

        let e = prismatic::rabbet();
        let r = Reference::build(&e.parts[0].def, &e).unwrap();
        assert!(is_out(&r, 300.0, 295.0, 12.0));
        assert!(is_in(&r, 300.0, 295.0, 6.0));

        let e = prismatic::mortise_and_tenon();
        let r = Reference::build(&e.parts[0].def, &e).unwrap();
        assert!(is_out(&r, 200.0, 20.0, 30.0));
        assert!(is_in(&r, 200.0, 20.0, 10.0));
        let r = Reference::build(&e.parts[1].def, &e).unwrap();
        assert!(is_in(&r, 315.0, 30.0, 10.0));
        assert!(is_out(&r, 315.0, 5.0, 10.0));
    }

    #[test]
    fn curved_corpus_points_classify_by_hand() {
        use cadview_core::solid::corpus::curved;
        let build = |e: cadview_core::solid::corpus::CorpusEntry| {
            Reference::build(&e.parts[0].def, &e).expect("defined")
        };
        // Rounded slab: the inner box starts 4 in; a corner column of
        // radius 4 about (4, 4) and a top-edge round about (y, z) = (4, 16).
        let r = build(curved::rounded_slab());
        assert!(is_in(&r, 200.0, 100.0, 10.0));
        assert!(is_out(&r, 0.5, 0.5, 10.0)); // 4.95 from (4, 4)
        assert!(is_in(&r, 2.0, 2.0, 10.0)); // 2.83 from (4, 4)
        assert!(is_out(&r, 200.0, 0.5, 19.5));
        assert!(is_in(&r, 200.0, 2.0, 18.0));
        assert!(is_out(&r, 200.0, 100.0, 20.5));
        // Tapered bar: half width 16 at z = 150.
        let r = build(curved::tapered_square_bar());
        assert!(is_in(&r, 0.0, 0.0, 150.0));
        assert!(is_in(&r, 15.5, -15.5, 150.0));
        assert!(is_out(&r, 16.5, 0.0, 150.0));
        assert!(is_out(&r, 0.0, 0.0, 300.5));
        // Bead (to radius 21 at z = 26) and cove (to radius 11 at z = 56).
        let r = build(curved::revolved_beads_and_coves());
        // On the axis the meridian touches its own axis segment: the band.
        assert_eq!(r.classify(Point3::new(0.0, 0.0, 10.0)), Class::Band);
        assert!(is_in(&r, 0.5, 0.0, 10.0));
        assert!(is_out(&r, 16.0, 0.0, 10.0));
        assert!(is_in(&r, 20.5, 0.0, 26.0));
        assert!(is_in(&r, 0.0, -20.5, 26.0));
        assert!(is_out(&r, 21.5, 0.0, 26.0));
        assert!(is_out(&r, -12.0, 0.0, 56.0));
        assert!(is_in(&r, 0.0, 10.5, 56.0));
        assert!(is_out(&r, 0.0, 0.0, 80.5));
        // Swept bar: legs along x and y, a bend about (100, 40).
        let r = build(curved::swept_round_bar());
        assert!(is_in(&r, 50.0, 5.9, 0.0));
        assert!(is_out(&r, 50.0, 6.1, 0.0));
        assert!(is_out(&r, -0.1, 0.0, 0.0));
        let (c, s) = (
            std::f64::consts::FRAC_1_SQRT_2,
            std::f64::consts::FRAC_1_SQRT_2,
        );
        for (rad, inside) in [
            (40.0, true),
            (45.9, true),
            (46.1, false),
            (34.1, true),
            (33.9, false),
        ] {
            let (x, y) = (100.0 + rad * s, 40.0 - rad * c);
            assert_eq!(is_in(&r, x, y, 0.0), inside, "bend at radius {rad}");
            assert_eq!(is_out(&r, x, y, 0.0), !inside, "bend at radius {rad}");
        }
        assert!(is_in(&r, 140.0, 100.0, 5.9));
        assert!(is_out(&r, 140.0, 140.1, 0.0));
        // Wedges: a quarter and three quarters of a turn from +X to +Y.
        let quarter = Feature::Revolve {
            id: FeatId(1),
            profile: cadview_core::solid::Profile2d::rect(10.0, 0.0, 20.0, 5.0),
            plane: Plane3::XZ,
            axis: kurbo::Line::new((0.0, 0.0), (0.0, 1.0)),
            angle_deg: 90.0,
            op: Op::Add,
        };
        let three = match quarter.clone() {
            Feature::Revolve {
                id,
                profile,
                plane,
                axis,
                op,
                ..
            } => Feature::Revolve {
                id,
                profile,
                plane,
                axis,
                angle_deg: 270.0,
                op,
            },
            _ => unreachable!(),
        };
        let one = |f: Feature| {
            Reference::build(
                &PartDef {
                    id: DefId(1),
                    name: "w".into(),
                    features: vec![f],
                },
                &NoTools,
            )
            .unwrap()
        };
        let (q, t) = (one(quarter), one(three));
        let p = 15.0 * std::f64::consts::FRAC_1_SQRT_2;
        assert!(is_in(&q, p, p, 2.5));
        assert!(is_out(&q, 15.0, -1.0, 2.5));
        assert!(is_out(&q, -1.0, 15.0, 2.5));
        assert!(is_in(&t, -15.0, 0.5, 2.5));
        assert!(is_in(&t, -p, -p, 2.5));
        assert!(is_out(&t, p, -p, 2.5));
        assert!(is_in(&t, p, p, 2.5));
    }

    #[test]
    fn rotated_frames_and_motions_are_isometries() {
        let pl = Placement::axis_angle(Vec3::new(1.0, 2.0, 3.0), 0.7, Vec3::new(5.0, -3.0, 2.0));
        let m = Motion::of(&pl).unwrap();
        let p = Iv3::point(1.0, 2.0, 3.0);
        // R is orthogonal: rows are unit and perpendicular.
        for i in 0..3 {
            for j in 0..3 {
                let d = m.r[i].dot(m.r[j]);
                let want = if i == j { 1.0 } else { 0.0 };
                assert!(d.contains(want) && d.width() < 1e-14, "{d:?}");
            }
        }
        let q = m.to_world(p);
        assert!(!q.norm2().contains(0.0));
        // A moved half-space classifies the moved point the same.
        let l = Linear {
            g: Iv3::point(0.0, 0.0, 1.0),
            c: Iv::point(2.5),
        };
        assert!(l.eval(p).pos() && l.moved(&m).eval(q).pos());
        assert!(Motion::of(&Placement {
            rotation: [2.0, 0.0, 0.0, 0.0],
            translation: Vec3::new(0.0, 0.0, 0.0)
        })
        .is_err());
    }

    #[test]
    fn undefined_cases_say_so() {
        use kurbo::Point;
        let l_profile = cadview_core::solid::Profile2d::polygon(&[
            Point::new(0.0, 0.0),
            Point::new(20.0, 0.0),
            Point::new(20.0, 10.0),
            Point::new(10.0, 10.0),
            Point::new(10.0, 20.0),
            Point::new(0.0, 20.0),
        ]);
        let def = PartDef {
            id: DefId(1),
            name: "l".into(),
            features: vec![
                Feature::Extrude {
                    id: FeatId(1),
                    profile: l_profile,
                    plane: Plane3::XY,
                    distance: 5.0,
                    taper_deg: 0.0,
                    op: Op::Add,
                },
                Feature::Chamfer {
                    id: FeatId(2),
                    edges: EdgeSel::Cap {
                        feature: FeatId(1),
                        side: CapSide::End,
                        segs: None,
                    },
                    d1: 1.0,
                    d2: 1.0,
                },
            ],
        };
        let e = Reference::build(&def, &NoTools).unwrap_err();
        assert!(!e.invalid && e.reason.contains("concave"), "{e}");
    }
}
