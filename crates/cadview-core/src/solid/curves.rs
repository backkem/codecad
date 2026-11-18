//! Evaluating and placing the exact curves and surfaces a kernel reports
//! ([`Curve3`], [`Surface3`]). Plain closed forms; a B-spline is evaluated
//! with de Boor's algorithm on homogeneous points.

use super::kernel::{Curve3, Surface3};
use super::part::{apply_point, apply_vec};
use super::types::{Placement, Point3, Vec3};

fn add(p: Point3, v: Vec3) -> Point3 {
    Point3::new(p.x + v.x, p.y + v.y, p.z + v.z)
}

impl Curve3 {
    /// The point at parameter `t`.
    pub fn point(&self, t: f64) -> Point3 {
        match self {
            Curve3::Line { origin, dir } => add(*origin, dir.scale(t)),
            Curve3::Circle {
                center,
                x,
                y,
                radius,
            } => {
                let (s, c) = t.sin_cos();
                add(*center, x.scale(radius * c) + y.scale(radius * s))
            }
            Curve3::Ellipse { center, x, y, a, b } => {
                let (s, c) = t.sin_cos();
                add(*center, x.scale(a * c) + y.scale(b * s))
            }
            Curve3::Nurbs {
                degree,
                knots,
                points,
                weights,
            } => nurbs_point(*degree, knots, points, weights, t),
        }
    }

    /// The derivative `dP/dt` at `t`.
    pub fn deriv(&self, t: f64) -> Vec3 {
        match self {
            Curve3::Line { dir, .. } => *dir,
            Curve3::Circle { x, y, radius, .. } => {
                let (s, c) = t.sin_cos();
                x.scale(-radius * s) + y.scale(radius * c)
            }
            Curve3::Ellipse { x, y, a, b, .. } => {
                let (s, c) = t.sin_cos();
                x.scale(-a * s) + y.scale(b * c)
            }
            Curve3::Nurbs { knots, .. } => {
                // A central difference on the curve's own scale: the
                // derivative only steers Newton steps and tangents.
                let (lo, hi) = (knots[0], knots[knots.len() - 1]);
                let h = 1e-7 * (hi - lo).abs().max(1e-12);
                let (a, b) = ((t - h).max(lo), (t + h).min(hi));
                let (pa, pb) = (self.point(a), self.point(b));
                (pb - pa).scale(1.0 / (b - a))
            }
        }
    }

    /// The curve moved by a placement (rigid: parameters keep their
    /// meaning).
    pub fn placed(&self, p: &Placement) -> Curve3 {
        if p.is_identity() {
            return self.clone();
        }
        match self {
            Curve3::Line { origin, dir } => Curve3::Line {
                origin: apply_point(p, *origin),
                dir: apply_vec(p, *dir),
            },
            Curve3::Circle {
                center,
                x,
                y,
                radius,
            } => Curve3::Circle {
                center: apply_point(p, *center),
                x: apply_vec(p, *x),
                y: apply_vec(p, *y),
                radius: *radius,
            },
            Curve3::Ellipse { center, x, y, a, b } => Curve3::Ellipse {
                center: apply_point(p, *center),
                x: apply_vec(p, *x),
                y: apply_vec(p, *y),
                a: *a,
                b: *b,
            },
            Curve3::Nurbs {
                degree,
                knots,
                points,
                weights,
            } => Curve3::Nurbs {
                degree: *degree,
                knots: knots.clone(),
                points: points.iter().map(|q| apply_point(p, *q)).collect(),
                weights: weights.clone(),
            },
        }
    }

    /// A short name for reports.
    pub fn kind(&self) -> &'static str {
        match self {
            Curve3::Line { .. } => "line",
            Curve3::Circle { .. } => "circle",
            Curve3::Ellipse { .. } => "ellipse",
            Curve3::Nurbs { .. } => "nurbs",
        }
    }
}

impl Surface3 {
    /// The surface moved by a placement.
    pub fn placed(&self, p: &Placement) -> Surface3 {
        let pt = |q: &Point3| apply_point(p, *q);
        let v = |q: &Vec3| apply_vec(p, *q);
        match self {
            Surface3::Plane { origin, normal } => Surface3::Plane {
                origin: pt(origin),
                normal: v(normal),
            },
            Surface3::Cylinder {
                origin,
                axis,
                x,
                radius,
            } => Surface3::Cylinder {
                origin: pt(origin),
                axis: v(axis),
                x: v(x),
                radius: *radius,
            },
            Surface3::EllipticCylinder {
                origin,
                axis,
                x,
                a,
                b,
            } => Surface3::EllipticCylinder {
                origin: pt(origin),
                axis: v(axis),
                x: v(x),
                a: *a,
                b: *b,
            },
            Surface3::Cone {
                origin,
                axis,
                x,
                radius,
                half_angle,
            } => Surface3::Cone {
                origin: pt(origin),
                axis: v(axis),
                x: v(x),
                radius: *radius,
                half_angle: *half_angle,
            },
            Surface3::Sphere { center, radius } => Surface3::Sphere {
                center: pt(center),
                radius: *radius,
            },
            Surface3::Torus {
                origin,
                axis,
                x,
                major,
                minor,
            } => Surface3::Torus {
                origin: pt(origin),
                axis: v(axis),
                x: v(x),
                major: *major,
                minor: *minor,
            },
            Surface3::Other { kind } => Surface3::Other { kind: kind.clone() },
        }
    }

    /// The unit normal (either sign) at a point on the surface, in closed
    /// form; None for a surface without one here, or a point on an axis.
    pub fn normal_at(&self, p: Point3) -> Option<Vec3> {
        let radial = |origin: &Point3, axis: &Vec3| {
            let r = p - *origin;
            r - axis.scale(r.dot(*axis))
        };
        match self {
            Surface3::Plane { normal, .. } => normal.normalized(),
            Surface3::Cylinder { origin, axis, .. } => radial(origin, axis).normalized(),
            Surface3::EllipticCylinder {
                origin,
                axis,
                x,
                a,
                b,
            } => {
                let r = p - *origin;
                let y = axis.cross(*x);
                let (c, s) = (r.dot(*x) / a, r.dot(y) / b);
                (x.scale(b * c) + y.scale(a * s)).normalized()
            }
            Surface3::Cone {
                origin,
                axis,
                half_angle,
                ..
            } => {
                let e = radial(origin, axis).normalized()?;
                let (sh, ch) = half_angle.sin_cos();
                (e.scale(ch) - axis.scale(sh)).normalized()
            }
            Surface3::Sphere { center, .. } => (p - *center).normalized(),
            Surface3::Torus {
                origin,
                axis,
                major,
                ..
            } => {
                let e = radial(origin, axis).normalized()?;
                let tube = add(*origin, e.scale(*major));
                (p - tube).normalized()
            }
            Surface3::Other { .. } => None,
        }
    }

    /// A short name for reports.
    pub fn kind(&self) -> &str {
        match self {
            Surface3::Plane { .. } => "plane",
            Surface3::Cylinder { .. } => "cylinder",
            Surface3::EllipticCylinder { .. } => "elliptic cylinder",
            Surface3::Cone { .. } => "cone",
            Surface3::Sphere { .. } => "sphere",
            Surface3::Torus { .. } => "torus",
            Surface3::Other { kind } => kind,
        }
    }
}

/// The knot span `k` with `knots[k] <= t < knots[k + 1]`, clamped to the
/// curve's valid spans `degree ..= n` (`n` the last control point).
pub(crate) fn span(degree: usize, knots: &[f64], n: usize, t: f64) -> usize {
    if t >= knots[n + 1] {
        // The last span that is not empty.
        let mut k = n;
        while k > degree && knots[k] >= knots[n + 1] {
            k -= 1;
        }
        return k;
    }
    if t <= knots[degree] {
        return degree;
    }
    let (mut lo, mut hi) = (degree, n + 1);
    while hi - lo > 1 {
        let mid = (lo + hi) / 2;
        if t < knots[mid] {
            hi = mid;
        } else {
            lo = mid;
        }
    }
    lo
}

/// A rational B-spline at `t` (de Boor on homogeneous points).
fn nurbs_point(degree: usize, knots: &[f64], points: &[Point3], weights: &[f64], t: f64) -> Point3 {
    let n = points.len() - 1;
    let p = degree;
    let k = span(p, knots, n, t);
    let mut d: Vec<[f64; 4]> = (0..=p)
        .map(|j| {
            let q = points[j + k - p];
            let w = weights[j + k - p];
            [q.x * w, q.y * w, q.z * w, w]
        })
        .collect();
    for r in 1..=p {
        for j in (r..=p).rev() {
            let i = j + k - p;
            let den = knots[i + p + 1 - r] - knots[i];
            let a = if den == 0.0 {
                0.0
            } else {
                (t - knots[i]) / den
            };
            let prev = d[j - 1];
            for (v, p) in d[j].iter_mut().zip(prev) {
                *v = (1.0 - a) * p + a * *v;
            }
        }
    }
    let [x, y, z, w] = d[p];
    Point3::new(x / w, y / w, z / w)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f64::consts::FRAC_1_SQRT_2;

    /// A quarter circle as a rational quadratic: exact points on the
    /// circle, the midpoint at 45 degrees.
    #[test]
    fn rational_quarter_circle() {
        let c = Curve3::Nurbs {
            degree: 2,
            knots: vec![0.0, 0.0, 0.0, 1.0, 1.0, 1.0],
            points: vec![
                Point3::new(1.0, 0.0, 0.0),
                Point3::new(1.0, 1.0, 0.0),
                Point3::new(0.0, 1.0, 0.0),
            ],
            weights: vec![1.0, FRAC_1_SQRT_2, 1.0],
        };
        for i in 0..=10 {
            let p = c.point(i as f64 / 10.0);
            assert!((p.x.hypot(p.y) - 1.0).abs() < 1e-15, "{p:?}");
        }
        let m = c.point(0.5);
        assert!((m.x - FRAC_1_SQRT_2).abs() < 1e-15 && (m.y - FRAC_1_SQRT_2).abs() < 1e-15);
        assert_eq!(c.point(1.0), Point3::new(0.0, 1.0, 0.0));
    }

    #[test]
    fn placed_circle_keeps_its_parameter() {
        let c = Curve3::Circle {
            center: Point3::ORIGIN,
            x: Vec3::X,
            y: Vec3::Y,
            radius: 2.0,
        };
        let p = Placement::translation(1.0, 2.0, 3.0);
        assert_eq!(c.placed(&p).point(0.0), Point3::new(3.0, 2.0, 3.0));
    }
}
