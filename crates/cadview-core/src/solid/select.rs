//! Kernel-neutral resolution of [`EdgeSel`] (docs/3d-design.md 4.3).
//!
//! A selector is turned into exact 3D target curves computed from our own
//! feature tree (an extrude's profile, plane and distance). A kernel then
//! picks the B-rep edges that lie on those curves. Edges split by later
//! features still lie on their target, so they are all picked. A selector
//! that picks nothing is an error, never a silent re-pick.

use super::kernel::EvalError;
use super::types::{CapSide, EdgeSel, FeatId, Feature, Point3, Seg2, Vec3};

/// An exact curve that selected edges must lie on.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum EdgeTarget {
    /// The straight segment from `a` to `b`.
    Line { a: Point3, b: Point3 },
    /// The circle about `center` in the plane with unit `normal`.
    Circle {
        center: Point3,
        normal: Vec3,
        radius: f64,
    },
}

impl EdgeTarget {
    /// Whether `p` lies on the target to within `tol` (mm).
    pub fn contains(&self, p: Point3, tol: f64) -> bool {
        match *self {
            EdgeTarget::Line { a, b } => {
                let ab = b - a;
                let len2 = ab.dot(ab);
                if len2 == 0.0 {
                    return p.distance(a) <= tol;
                }
                let t = (p - a).dot(ab) / len2;
                let slack = tol / len2.sqrt();
                if t < -slack || t > 1.0 + slack {
                    return false;
                }
                let foot = a + ab.scale(t.clamp(0.0, 1.0));
                p.distance(foot) <= tol
            }
            EdgeTarget::Circle {
                center,
                normal,
                radius,
            } => {
                let d = p - center;
                let h = d.dot(normal);
                let radial = (d + normal.scale(-h)).length();
                h.abs() <= tol && (radial - radius).abs() <= tol
            }
        }
    }
}

/// What an [`EdgeSel`] asks the kernel for.
#[derive(Debug, Clone, PartialEq)]
pub enum EdgeQuery {
    /// Every edge lying entirely on one of these curves.
    Targets(Vec<EdgeTarget>),
    /// The edge nearest each point.
    Near(Vec<Point3>),
    /// Every edge.
    All,
}

/// Resolves `sel`, used by feature `at`, against the features before it.
pub fn edge_query(before: &[Feature], sel: &EdgeSel, at: FeatId) -> Result<EdgeQuery, EvalError> {
    match sel {
        EdgeSel::All => Ok(EdgeQuery::All),
        EdgeSel::Near { points } => {
            if points.is_empty() || points.iter().any(|p| !p.is_finite()) {
                return Err(selector(at, "Near needs at least one finite point"));
            }
            Ok(EdgeQuery::Near(points.clone()))
        }
        EdgeSel::Cap {
            feature,
            side,
            segs,
        } => {
            let (profile, frame, distance) = extrude_of(before, *feature, at)?;
            let offsets: &[f64] = match side {
                CapSide::Start => &[0.0],
                CapSide::End => &[distance],
                CapSide::Both => &[0.0, distance],
            };
            let indices: Vec<usize> = match segs {
                None => (0..profile.segment_count()).collect(),
                Some(list) => list.clone(),
            };
            let mut out = Vec::new();
            for &w in offsets {
                for &i in &indices {
                    let Some((seg, _)) = profile.segment(i) else {
                        return Err(selector(
                            at,
                            format!(
                                "segment {i} is out of range for feature {}'s profile",
                                feature.0
                            ),
                        ));
                    };
                    out.push(seg_target(seg, &frame, w).map_err(|why| selector(at, why))?);
                }
            }
            Ok(EdgeQuery::Targets(out))
        }
        EdgeSel::Lateral { feature, segs } => {
            let (profile, frame, distance) = extrude_of(before, *feature, at)?;
            let mut out = Vec::new();
            for &i in segs {
                let Some((seg, _)) = profile.segment(i) else {
                    return Err(selector(
                        at,
                        format!(
                            "segment {i} is out of range for feature {}'s profile",
                            feature.0
                        ),
                    ));
                };
                let Some((p, _)) = seg.ends() else {
                    return Err(selector(at, format!("segment {i} has no start vertex")));
                };
                out.push(EdgeTarget::Line {
                    a: frame.at(p, 0.0),
                    b: frame.at(p, distance),
                });
            }
            Ok(EdgeQuery::Targets(out))
        }
    }
}

fn selector(at: FeatId, reason: impl Into<String>) -> EvalError {
    EvalError::Selector {
        feature: at,
        reason: reason.into(),
    }
}

fn extrude_of(
    before: &[Feature],
    id: FeatId,
    at: FeatId,
) -> Result<(&super::types::Profile2d, super::types::Frame3, f64), EvalError> {
    match before.iter().find(|f| f.id() == id) {
        Some(Feature::Extrude {
            profile,
            plane,
            distance,
            ..
        }) => Ok((profile, plane.frame().map_err(|e| e.at(at))?, *distance)),
        Some(other) => Err(selector(
            at,
            format!(
                "feature {} is a {}; Cap and Lateral name an extrude",
                id.0,
                other.kind_name()
            ),
        )),
        None => Err(selector(
            at,
            format!("feature {} does not come before this one", id.0),
        )),
    }
}

fn seg_target(seg: &Seg2, frame: &super::types::Frame3, w: f64) -> Result<EdgeTarget, String> {
    match seg {
        Seg2::Line(l) => Ok(EdgeTarget::Line {
            a: frame.at(l.p0, w),
            b: frame.at(l.p1, w),
        }),
        Seg2::Circle(c) => Ok(EdgeTarget::Circle {
            center: frame.at(c.center, w),
            normal: frame.z,
            radius: c.radius,
        }),
        Seg2::Arc(a) if a.radii.x == a.radii.y => Ok(EdgeTarget::Circle {
            center: frame.at(a.center, w),
            normal: frame.z,
            radius: a.radii.x,
        }),
        Seg2::Arc(_) => Err("selecting an elliptic cap edge is not supported yet".into()),
        Seg2::Spline { .. } => Err("selecting a spline cap edge is not supported yet".into()),
    }
}
