//! Silhouettes of curved surfaces in closed form (docs/3d-design.md 7.3).
//!
//! A silhouette is where a surface turns away from the viewer: its normal
//! is perpendicular to the view direction. For the surfaces a kernel
//! reports in closed form ([`Surface3`]) these curves are:
//!
//! - cylinder, elliptic cylinder, cone: two rulings (lines), none when the
//!   axis points at the viewer (or, for a cone, when the viewer looks from
//!   inside its opening angle);
//! - sphere: the great circle perpendicular to the view direction;
//! - torus, with its axis along the view direction: the outer and the
//!   inner equator (circles of radius `major + minor` and `major - minor`);
//!   with its axis across the view direction: the top and bottom circles
//!   (radius `major`, seen edge on) and the two meridian circles across the
//!   view (radius `minor`). In any other direction a torus's silhouette is
//!   not a conic, and it is reported as having no closed form here.
//!
//! These are whole curves, not yet clipped to a face: a line is unbounded
//! (`range` None) and a circle runs a full turn. A kernel clips them to its
//! faces ([`super::kernel::SolidKernel::silhouettes`]). Plane faces have no
//! silhouette. Kernel-neutral: no kernel is named here.

use std::f64::consts::{FRAC_PI_2, TAU};

use super::kernel::{Curve3, Surface3};
use super::types::{Point3, Vec3};

/// Below this, a direction counts as parallel (or perpendicular) to an
/// axis (sine or cosine of the angle).
pub const ALIGN_TOL: f64 = 1e-12;

/// One silhouette curve of a whole surface. `range` is None for a line (it
/// is unbounded; the face decides its ends).
#[derive(Debug, Clone, PartialEq)]
pub struct Candidate {
    pub curve: Curve3,
    pub range: Option<[f64; 2]>,
}

fn add(p: Point3, v: Vec3) -> Point3 {
    Point3::new(p.x + v.x, p.y + v.y, p.z + v.z)
}

fn line(origin: Point3, dir: Vec3) -> Candidate {
    Candidate {
        curve: Curve3::Line { origin, dir },
        range: None,
    }
}

fn circle(center: Point3, x: Vec3, y: Vec3, radius: f64) -> Candidate {
    Candidate {
        curve: Curve3::Circle {
            center,
            x,
            y,
            radius,
        },
        range: Some([0.0, TAU]),
    }
}

/// A unit vector perpendicular to the unit vector `d`.
pub fn perpendicular(d: Vec3) -> Vec3 {
    let helper = if d.x.abs() <= d.y.abs() && d.x.abs() <= d.z.abs() {
        Vec3::X
    } else if d.y.abs() <= d.z.abs() {
        Vec3::Y
    } else {
        Vec3::Z
    };
    d.cross(helper).normalized().unwrap_or(Vec3::X)
}

/// The silhouette curves of surface `s` for a viewer looking along
/// `-view` (`view` points toward the eye; any length but zero). `Err`
/// names a surface whose silhouette has no closed form here.
pub fn candidates(s: &Surface3, view: Vec3) -> Result<Vec<Candidate>, String> {
    let Some(d) = view.normalized() else {
        return Err("the view direction is zero".into());
    };
    Ok(match s {
        Surface3::Plane { .. } => Vec::new(),
        Surface3::Cylinder {
            origin,
            axis,
            radius,
            ..
        } => {
            // The normal is radial: perpendicular to both the axis and d.
            match axis.cross(d).normalized() {
                Some(w) if axis.cross(d).length() > ALIGN_TOL => vec![
                    line(add(*origin, w.scale(*radius)), *axis),
                    line(add(*origin, w.scale(-radius)), *axis),
                ],
                _ => Vec::new(),
            }
        }
        Surface3::EllipticCylinder {
            origin,
            axis,
            x,
            a,
            b,
        } => {
            // P(u) = O + a cos u X + b sin u Y; the normal is along
            // b cos u X + a sin u Y, perpendicular to d where
            // b cos u (d.X) + a sin u (d.Y) = 0.
            let y = axis.cross(*x);
            let (dx, dy) = (d.dot(*x), d.dot(y));
            if dx.hypot(dy) <= ALIGN_TOL {
                return Ok(Vec::new());
            }
            let u = (-b * dx).atan2(a * dy);
            [u, u + std::f64::consts::PI]
                .into_iter()
                .map(|u| {
                    let (s, c) = u.sin_cos();
                    line(add(*origin, x.scale(a * c) + y.scale(b * s)), *axis)
                })
                .collect()
        }
        Surface3::Cone {
            origin,
            axis,
            x,
            radius,
            half_angle,
        } => {
            // The normal along cos h e(u) - sin h axis is perpendicular to
            // d where e(u).d = tan h (axis.d).
            let y = axis.cross(*x);
            let (dx, dy) = (d.dot(*x), d.dot(y));
            let rho = dx.hypot(dy);
            if rho <= ALIGN_TOL {
                return Ok(Vec::new());
            }
            let (sh, ch) = half_angle.sin_cos();
            let c = half_angle.tan() * axis.dot(d) / rho;
            if c.abs() > 1.0 {
                return Ok(Vec::new());
            }
            let phi = dy.atan2(dx);
            let delta = c.acos();
            let mut out = Vec::new();
            for u in [phi + delta, phi - delta] {
                let (s, co) = u.sin_cos();
                let e = x.scale(co) + y.scale(s);
                out.push(line(
                    add(*origin, e.scale(*radius)),
                    (e.scale(sh) + axis.scale(ch)).normalized().unwrap_or(*axis),
                ));
                if delta == 0.0 {
                    break;
                }
            }
            out
        }
        Surface3::Sphere { center, radius } => {
            let x = perpendicular(d);
            let y = d.cross(x);
            vec![circle(*center, x, y, *radius)]
        }
        Surface3::Torus {
            origin,
            axis,
            x,
            major,
            minor,
        } => {
            let along = axis.dot(d);
            let y = axis.cross(*x);
            if (along.abs() - 1.0).abs() <= ALIGN_TOL || axis.cross(d).length() <= ALIGN_TOL {
                // Axis toward the viewer: n.d = sin v (axis.d) = 0 at
                // v = 0 and pi, the outer and inner equators.
                let mut out = vec![circle(*origin, *x, y, major + minor)];
                if major - minor > 0.0 {
                    out.push(circle(*origin, *x, y, major - minor));
                }
                out
            } else if along.abs() <= ALIGN_TOL {
                // Axis across the view: n = cos v e(u) + sin v axis, and
                // n.d = cos v (e(u).d) = 0 at v = +-pi/2 (the circles of
                // radius `major` at heights +-minor) or at e(u) across d
                // (two meridian circles in the plane perpendicular to d).
                let mut out = vec![
                    circle(add(*origin, axis.scale(*minor)), *x, y, *major),
                    circle(add(*origin, axis.scale(-minor)), *x, y, *major),
                ];
                let phi = d.dot(y).atan2(d.dot(*x));
                for u in [phi + FRAC_PI_2, phi - FRAC_PI_2] {
                    let (s, c) = u.sin_cos();
                    let e = x.scale(c) + y.scale(s);
                    out.push(circle(add(*origin, e.scale(*major)), e, *axis, *minor));
                }
                out
            } else {
                return Err(
                    "a torus seen obliquely: its silhouette is not a conic, and no \
                     closed form is implemented"
                        .into(),
                );
            }
        }
        Surface3::Other { kind } => {
            return Err(format!(
                "a {kind} face: its silhouette has no closed form here"
            ))
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn normal_at(s: &Surface3, p: Point3) -> Vec3 {
        match s {
            Surface3::Cylinder { origin, axis, .. } => {
                let r = p - *origin;
                (r - axis.scale(r.dot(*axis))).normalized().unwrap()
            }
            Surface3::Sphere { center, .. } => (p - *center).normalized().unwrap(),
            Surface3::Torus {
                origin,
                axis,
                major,
                ..
            } => {
                let r = p - *origin;
                let radial = (r - axis.scale(r.dot(*axis))).normalized().unwrap();
                let tube = add(*origin, radial.scale(*major));
                (p - tube).normalized().unwrap()
            }
            _ => unreachable!(),
        }
    }

    /// Every candidate point lies where the normal is across the view.
    #[test]
    fn candidates_are_where_the_normal_is_across_the_view() {
        let iso = Vec3::new(1.0, -1.0, 1.0);
        let surfaces = [
            Surface3::Cylinder {
                origin: Point3::new(1.0, 2.0, 3.0),
                axis: Vec3::Z,
                x: Vec3::X,
                radius: 5.0,
            },
            Surface3::Sphere {
                center: Point3::ORIGIN,
                radius: 4.0,
            },
        ];
        for s in &surfaces {
            for c in candidates(s, iso).unwrap() {
                for t in [0.0, 0.3, 1.7, 4.0] {
                    let p = c.curve.point(t);
                    let n = normal_at(s, p);
                    assert!(n.dot(iso).abs() < 1e-14, "{s:?} at {t}");
                }
            }
        }
        // A torus with its axis across the view (front view of a ring
        // lying flat): four circles, each with normals across the view.
        let ring = Surface3::Torus {
            origin: Point3::ORIGIN,
            axis: Vec3::Z,
            x: Vec3::X,
            major: 10.0,
            minor: 2.0,
        };
        let front = Vec3::new(0.0, -1.0, 0.0);
        let cs = candidates(&ring, front).unwrap();
        assert_eq!(cs.len(), 4);
        for c in &cs {
            for t in [0.0, 1.0, 2.0, 3.0] {
                let p = c.curve.point(t);
                assert!(normal_at(&ring, p).dot(front).abs() < 1e-14);
            }
        }
        assert!(candidates(&ring, iso).is_err());
        // A cylinder seen end on has none.
        let end_on = candidates(&surfaces[0], Vec3::Z).unwrap();
        assert!(end_on.is_empty());
    }

    /// A cone's rulings: for a cone of half angle 30 degrees seen from the
    /// side, the rulings make 30 degrees with the axis on both sides.
    #[test]
    fn cone_silhouette_rulings() {
        let h = 30f64.to_radians();
        let s = Surface3::Cone {
            origin: Point3::ORIGIN,
            axis: Vec3::Z,
            x: Vec3::X,
            radius: 10.0,
            half_angle: h,
        };
        let cs = candidates(&s, Vec3::new(0.0, -1.0, 0.0)).unwrap();
        assert_eq!(cs.len(), 2);
        for c in cs {
            let Curve3::Line { origin, dir } = c.curve else {
                panic!()
            };
            assert!(origin.y.abs() < 1e-14 && (origin.x.abs() - 10.0).abs() < 1e-14);
            assert!((dir.z - h.cos()).abs() < 1e-15 && dir.y.abs() < 1e-15);
        }
    }
}
