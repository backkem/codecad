//! Parts in the document: placed instances of part definitions.
//!
//! A [`PartDef`] is shape only: an id, a name and an ordered feature list
//! in its own frame (docs/3d-design.md section 4.0). A [`Part`] places one
//! definition in the world and carries the script's own attributes. Several
//! parts may share one definition (copies, arrays); they differ only in
//! placement and attributes, and the definition is evaluated once.
//!
//! Ids: parts take their numbers from the document's one id counter, the
//! same one 2D entities use, so a number is never both an entity and a
//! part. They print as `p_<n>`; definitions print as `d_<n>`; entities stay
//! `e_<n>`. A part id given where an entity is expected (or the other way
//! round) fails to parse instead of picking something else.
//!
//! This file also holds the rigid-placement maths the API needs (compose,
//! invert, move points, vectors and planes between frames).

use serde::{Deserialize, Serialize};

use crate::attrs::Attrs;
use crate::types::EntityId;

use super::types::{
    DefId, EdgeSel, FeatId, Feature, PartDef, Path3, Placement, Plane3, Point3, Profile2d, Seg2,
    Seg3, ToolRef, Vec3,
};

/// A placed occurrence of a definition: what a script selects, moves and
/// asks about.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Part {
    pub id: EntityId,
    pub def: DefId,
    /// Definition frame to world.
    pub placement: Placement,
    /// Free-form attributes the script sets; stored, synced and filtered
    /// on, never interpreted (see `attrs.rs`).
    #[serde(default)]
    pub attrs: Attrs,
}

/// `p_<n>` for a part id.
pub fn part_key(id: EntityId) -> String {
    format!("p_{}", id.0)
}

/// `d_<n>` for a definition id.
pub fn def_key(id: DefId) -> String {
    format!("d_{}", id.0)
}

/// Parses `p_<n>`.
pub fn parse_part_id(s: &str) -> Result<EntityId, String> {
    s.strip_prefix("p_")
        .and_then(|n| n.parse::<u64>().ok())
        .map(EntityId)
        .ok_or_else(|| format!("bad part id: {s} (part ids look like p_12)"))
}

/// Parses `d_<n>`.
pub fn parse_def_id(s: &str) -> Result<DefId, String> {
    s.strip_prefix("d_")
        .and_then(|n| n.parse::<u64>().ok())
        .map(DefId)
        .ok_or_else(|| format!("bad definition id: {s} (definition ids look like d_3)"))
}

/// Bincode bytes of a part, for Yrs sync.
pub fn part_to_bytes(p: &Part) -> Vec<u8> {
    bincode::serialize(p).expect("bincode serialize failed")
}

pub fn part_from_bytes(data: &[u8]) -> Option<Part> {
    bincode::deserialize(data).ok()
}

/// Bincode bytes of a definition, for Yrs sync and the evaluation cache.
pub fn def_to_bytes(d: &PartDef) -> Vec<u8> {
    bincode::serialize(d).expect("bincode serialize failed")
}

pub fn def_from_bytes(data: &[u8]) -> Option<PartDef> {
    bincode::deserialize(data).ok()
}

/// The definitions `def` uses as boolean tools, directly.
pub fn tool_defs(def: &PartDef) -> impl Iterator<Item = DefId> + '_ {
    def.features.iter().filter_map(|f| match f {
        Feature::Boolean {
            tool: ToolRef::Def(id, _),
            ..
        } => Some(*id),
        _ => None,
    })
}

/// The next free feature id of a definition (one more than the largest).
pub fn next_feat_id(def: &PartDef) -> FeatId {
    FeatId(def.features.iter().map(|f| f.id().0).max().unwrap_or(0) + 1)
}

// ── Rigid placements ──────────────────────────────────────────────────

/// Hamilton product `a * b` of quaternions `[w, x, y, z]`.
pub fn quat_mul(a: [f64; 4], b: [f64; 4]) -> [f64; 4] {
    let [aw, ax, ay, az] = a;
    let [bw, bx, by, bz] = b;
    [
        aw * bw - ax * bx - ay * by - az * bz,
        aw * bx + ax * bw + ay * bz - az * by,
        aw * by - ax * bz + ay * bw + az * bx,
        aw * bz + ax * by - ay * bx + az * bw,
    ]
}

fn quat_normalize(q: [f64; 4]) -> [f64; 4] {
    let n = q.iter().map(|v| v * v).sum::<f64>().sqrt();
    if n > 0.0 && n.is_finite() {
        q.map(|v| v / n)
    } else {
        [1.0, 0.0, 0.0, 0.0]
    }
}

/// `v` turned by the unit quaternion `q`.
pub fn rotate_vec(q: [f64; 4], v: Vec3) -> Vec3 {
    // v' = v + 2 w (u x v) + 2 u x (u x v), u the vector part.
    let [w, x, y, z] = q;
    let u = Vec3::new(x, y, z);
    let t = u.cross(v).scale(2.0);
    v + t.scale(w) + u.cross(t)
}

/// Where a placement puts a local point.
pub fn apply_point(p: &Placement, pt: Point3) -> Point3 {
    let v = rotate_vec(p.rotation, Vec3::new(pt.x, pt.y, pt.z)) + p.translation;
    Point3::new(v.x, v.y, v.z)
}

/// Where a placement turns a local direction (no translation).
pub fn apply_vec(p: &Placement, v: Vec3) -> Vec3 {
    rotate_vec(p.rotation, v)
}

/// `a` after `b`: the placement that applies `b` first, then `a`.
pub fn compose(a: &Placement, b: &Placement) -> Placement {
    Placement {
        rotation: quat_normalize(quat_mul(a.rotation, b.rotation)),
        translation: rotate_vec(a.rotation, b.translation) + a.translation,
    }
}

/// The inverse placement: world back to the local frame.
pub fn inverse(p: &Placement) -> Placement {
    let [w, x, y, z] = p.rotation;
    let conj = [w, -x, -y, -z];
    Placement {
        rotation: conj,
        translation: -rotate_vec(conj, p.translation),
    }
}

/// A rotation of `angle_deg` degrees (right-handed) about the line through
/// `about` along `axis`, as a placement.
pub fn rotation_about(axis: Vec3, angle_deg: f64, about: Point3) -> Result<Placement, String> {
    let a = axis
        .normalized()
        .ok_or("rotation axis must be a non-zero vector")?;
    if !angle_deg.is_finite() || !about.is_finite() {
        return Err("rotation angle and centre must be finite".into());
    }
    let r = Placement::axis_angle(a, angle_deg.to_radians(), Vec3::new(0.0, 0.0, 0.0));
    let c = Vec3::new(about.x, about.y, about.z);
    // x -> about + R (x - about) = R x + (about - R about)
    Ok(Placement {
        rotation: r.rotation,
        translation: c - rotate_vec(r.rotation, c),
    })
}

/// A plane moved by a placement.
pub fn place_plane(p: &Placement, plane: &Plane3) -> Plane3 {
    Plane3 {
        origin: apply_point(p, plane.origin),
        x_axis: apply_vec(p, plane.x_axis),
        normal: apply_vec(p, plane.normal),
    }
}

fn place_path(p: &Placement, path: &Path3) -> Path3 {
    Path3 {
        segs: path
            .segs
            .iter()
            .map(|s| match s {
                Seg3::Line { from, to } => Seg3::Line {
                    from: apply_point(p, *from),
                    to: apply_point(p, *to),
                },
                Seg3::Arc { from, via, to } => Seg3::Arc {
                    from: apply_point(p, *from),
                    via: apply_point(p, *via),
                    to: apply_point(p, *to),
                },
            })
            .collect(),
    }
}

/// A feature with every position and direction in it moved by `p` (used to
/// bring world-coordinate input into a part's own frame). Profiles stay as
/// they are: they live on their plane, and the plane moves.
pub fn place_feature(p: &Placement, f: &Feature) -> Feature {
    let mut f = f.clone();
    match &mut f {
        Feature::Extrude { plane, .. } | Feature::Revolve { plane, .. } => {
            *plane = place_plane(p, plane)
        }
        Feature::Sweep { plane, path, .. } => {
            *plane = place_plane(p, plane);
            *path = place_path(p, path);
        }
        Feature::Loft { sections, .. } => {
            for (_, plane) in sections.iter_mut() {
                *plane = place_plane(p, plane);
            }
        }
        Feature::Hole { at, dir, .. } => {
            *at = apply_point(p, *at);
            *dir = apply_vec(p, *dir);
        }
        Feature::Boolean { tool, .. } => {
            if let ToolRef::Def(_, placement) = tool {
                *placement = compose(p, placement);
            }
        }
        Feature::Fillet { edges, .. } | Feature::Chamfer { edges, .. } => {
            if let EdgeSel::Near { points } = edges {
                for q in points.iter_mut() {
                    *q = apply_point(p, *q);
                }
            }
        }
    }
    f
}

// ── Mirror images ─────────────────────────────────────────────────────
//
// A mirror image is not a rigid motion of the original, so it cannot be a
// placement: it needs a definition of its own. Every definition is
// mirrored the same way, across its own frame's YZ plane (x -> -x, the
// reflection `S`), so the mirrored definition depends only on the
// original: a part placed by `P` and mirrored across a world plane `M`
// becomes the mirrored definition placed by `M P S`, a rigid placement
// (two reflections and a rotation). Parts that share a definition mirror
// to parts that share its mirrored definition.

/// `p` reflected by `S` (x -> -x).
fn mirror_point(p: Point3) -> Point3 {
    Point3::new(-p.x, p.y, p.z)
}

fn mirror_vec(v: Vec3) -> Vec3 {
    Vec3::new(-v.x, v.y, v.z)
}

/// A sketch plane and its profile reflected by `S`. The plane's origin, u
/// axis and normal are reflected; its v axis (`normal x u`) then comes out
/// reversed (a reflection turns handedness), so the profile's v
/// coordinates are negated to keep every point where the reflection puts
/// it.
fn mirror_sketch(plane: &Plane3, profile: &Profile2d) -> (Plane3, Profile2d) {
    let p = Plane3 {
        origin: mirror_point(plane.origin),
        x_axis: mirror_vec(plane.x_axis),
        normal: mirror_vec(plane.normal),
    };
    let flip = |lp: &Vec<Seg2>| lp.iter().map(mirror_seg).collect::<Vec<_>>();
    (
        p,
        Profile2d {
            outer: flip(&profile.outer),
            holes: profile.holes.iter().map(flip).collect(),
        },
    )
}

/// A profile segment with its v coordinates negated: an arc's centre,
/// start angle, sweep and axis rotation are negated, so the point at
/// parameter `-t` is the reflection of the point at `t`.
fn mirror_seg(s: &Seg2) -> Seg2 {
    let m = |p: kurbo::Point| kurbo::Point::new(p.x, -p.y);
    match s {
        Seg2::Line(l) => Seg2::Line(kurbo::Line::new(m(l.p0), m(l.p1))),
        Seg2::Arc(a) => Seg2::Arc(kurbo::Arc {
            center: m(a.center),
            radii: a.radii,
            start_angle: -a.start_angle,
            sweep_angle: -a.sweep_angle,
            x_rotation: -a.x_rotation,
        }),
        Seg2::Circle(c) => Seg2::Circle(kurbo::Circle::new(m(c.center), c.radius)),
        Seg2::Spline {
            degree,
            knots,
            control_points,
        } => Seg2::Spline {
            degree: *degree,
            knots: knots.clone(),
            control_points: control_points.iter().map(|p| m(*p)).collect(),
        },
    }
}

/// `S P S` for a placement `P`: the same motion seen in the mirror. The
/// rotation part conjugated by `S` is the quaternion `[w, x, -y, -z]`.
fn mirror_placement(p: &Placement) -> Placement {
    let [w, x, y, z] = p.rotation;
    Placement {
        rotation: [w, x, -y, -z],
        translation: mirror_vec(p.translation),
    }
}

/// The features of a definition reflected by `S` (x -> -x in its own
/// frame). Tool definitions are renamed by `tool` (to their own mirrored
/// definitions). Feature ids and profile segment order are kept, so edge
/// selectors still name the same edges. A revolve's axis is reversed: a
/// reflection turns a right-handed turn into a left-handed one. Mirroring
/// twice gives the same features back, bit for bit.
pub fn mirror_features(
    features: &[Feature],
    tool: &dyn Fn(DefId) -> DefId,
) -> Result<Vec<Feature>, String> {
    features
        .iter()
        .map(|f| {
            Ok(match f {
                Feature::Extrude {
                    id,
                    profile,
                    plane,
                    distance,
                    taper_deg,
                    op,
                } => {
                    let (plane, profile) = mirror_sketch(plane, profile);
                    Feature::Extrude {
                        id: *id,
                        profile,
                        plane,
                        distance: *distance,
                        taper_deg: *taper_deg,
                        op: *op,
                    }
                }
                Feature::Revolve {
                    id,
                    profile,
                    plane,
                    axis,
                    angle_deg,
                    op,
                } => {
                    let (plane, profile) = mirror_sketch(plane, profile);
                    let m = |p: kurbo::Point| kurbo::Point::new(p.x, -p.y);
                    Feature::Revolve {
                        id: *id,
                        profile,
                        plane,
                        axis: kurbo::Line::new(m(axis.p1), m(axis.p0)),
                        angle_deg: *angle_deg,
                        op: *op,
                    }
                }
                Feature::Sweep {
                    id,
                    profile,
                    plane,
                    path,
                    op,
                } => {
                    let (plane, profile) = mirror_sketch(plane, profile);
                    let path = Path3 {
                        segs: path
                            .segs
                            .iter()
                            .map(|s| match s {
                                Seg3::Line { from, to } => Seg3::Line {
                                    from: mirror_point(*from),
                                    to: mirror_point(*to),
                                },
                                Seg3::Arc { from, via, to } => Seg3::Arc {
                                    from: mirror_point(*from),
                                    via: mirror_point(*via),
                                    to: mirror_point(*to),
                                },
                            })
                            .collect(),
                    };
                    Feature::Sweep {
                        id: *id,
                        profile,
                        plane,
                        path,
                        op: *op,
                    }
                }
                Feature::Loft {
                    id,
                    sections,
                    ruled,
                    op,
                } => Feature::Loft {
                    id: *id,
                    sections: sections
                        .iter()
                        .map(|(profile, plane)| {
                            let (plane, profile) = mirror_sketch(plane, profile);
                            (profile, plane)
                        })
                        .collect(),
                    ruled: *ruled,
                    op: *op,
                },
                Feature::Hole {
                    id,
                    at,
                    dir,
                    dia,
                    depth,
                    cbore,
                } => Feature::Hole {
                    id: *id,
                    at: mirror_point(*at),
                    dir: mirror_vec(*dir),
                    dia: *dia,
                    depth: *depth,
                    cbore: *cbore,
                },
                Feature::Boolean { id, op, tool: t } => match t {
                    ToolRef::Def(d, p) => Feature::Boolean {
                        id: *id,
                        op: *op,
                        tool: ToolRef::Def(tool(*d), mirror_placement(p)),
                    },
                    ToolRef::Part { .. } => {
                        let why = "uses a live link to a placed part, which has no mirror image";
                        return Err(format!("feature {} {why}", id.0));
                    }
                },
                Feature::Fillet {
                    id,
                    edges,
                    radius,
                    fallback,
                } => Feature::Fillet {
                    id: *id,
                    edges: mirror_edges(edges),
                    radius: *radius,
                    fallback: *fallback,
                },
                Feature::Chamfer { id, edges, d1, d2 } => Feature::Chamfer {
                    id: *id,
                    edges: mirror_edges(edges),
                    d1: *d1,
                    d2: *d2,
                },
            })
        })
        .collect()
}

fn mirror_edges(e: &EdgeSel) -> EdgeSel {
    match e {
        EdgeSel::Near { points } => EdgeSel::Near {
            points: points.iter().map(|p| mirror_point(*p)).collect(),
        },
        other => other.clone(),
    }
}

/// The 3 x 3 matrix of a unit quaternion `[w, x, y, z]`, by rows.
fn quat_matrix(q: [f64; 4]) -> [[f64; 3]; 3] {
    let c = |v: Vec3| rotate_vec(q, v);
    let (a, b, d) = (c(Vec3::X), c(Vec3::Y), c(Vec3::Z));
    [[a.x, b.x, d.x], [a.y, b.y, d.y], [a.z, b.z, d.z]]
}

/// The unit quaternion of a proper rotation matrix (Shepperd's method:
/// divide by the largest of the four candidate components).
fn matrix_quat(m: [[f64; 3]; 3]) -> [f64; 4] {
    let t = m[0][0] + m[1][1] + m[2][2];
    let q = if t >= m[0][0] && t >= m[1][1] && t >= m[2][2] {
        let s = (1.0 + t).sqrt() * 2.0;
        [
            s / 4.0,
            (m[2][1] - m[1][2]) / s,
            (m[0][2] - m[2][0]) / s,
            (m[1][0] - m[0][1]) / s,
        ]
    } else if m[0][0] >= m[1][1] && m[0][0] >= m[2][2] {
        let s = (1.0 + m[0][0] - m[1][1] - m[2][2]).sqrt() * 2.0;
        [
            (m[2][1] - m[1][2]) / s,
            s / 4.0,
            (m[0][1] + m[1][0]) / s,
            (m[0][2] + m[2][0]) / s,
        ]
    } else if m[1][1] >= m[2][2] {
        let s = (1.0 + m[1][1] - m[0][0] - m[2][2]).sqrt() * 2.0;
        [
            (m[0][2] - m[2][0]) / s,
            (m[0][1] + m[1][0]) / s,
            s / 4.0,
            (m[1][2] + m[2][1]) / s,
        ]
    } else {
        let s = (1.0 + m[2][2] - m[0][0] - m[1][1]).sqrt() * 2.0;
        [
            (m[1][0] - m[0][1]) / s,
            (m[0][2] + m[2][0]) / s,
            (m[1][2] + m[2][1]) / s,
            s / 4.0,
        ]
    };
    quat_normalize(q)
}

/// The placement of a mirrored definition that puts its part where the
/// mirror across the world plane through `point` with unit `normal` puts
/// a part placed by `p`: `M P S`, rigid.
pub fn mirrored_placement(p: &Placement, point: Point3, normal: Vec3) -> Placement {
    // M x = H x + 2 (n . q) n with H = I - 2 n n^T.
    let n = normal;
    let h = |v: Vec3| v + n.scale(-2.0 * n.dot(v));
    let r = quat_matrix(p.rotation);
    // Columns of R S: -R e_x, R e_y, R e_z; then H applied to each.
    let col = |j: usize| Vec3::new(r[0][j], r[1][j], r[2][j]);
    let (c0, c1, c2) = (h(-col(0)), h(col(1)), h(col(2)));
    let m = [[c0.x, c1.x, c2.x], [c0.y, c1.y, c2.y], [c0.z, c1.z, c2.z]];
    let q = Vec3::new(point.x, point.y, point.z);
    Placement {
        rotation: matrix_quat(m),
        translation: h(p.translation) + n.scale(2.0 * n.dot(q)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mirrored_placement_puts_points_where_the_mirror_does() {
        let p = compose(
            &Placement::translation(10.0, -4.0, 3.0),
            &rotation_about(Vec3::new(1.0, 2.0, 0.5), 33.0, Point3::new(1.0, 0.0, 2.0)).unwrap(),
        );
        let n = Vec3::new(0.3, -0.5, 0.8).normalized().unwrap();
        let q = Point3::new(5.0, 1.0, -2.0);
        let m = mirrored_placement(&p, q, n);
        assert!(m.check().is_ok());
        for local in [
            Point3::new(1.0, 2.0, 3.0),
            Point3::new(-4.0, 0.5, 7.0),
            Point3::ORIGIN,
        ] {
            // M (P x) against (M P S) (S x).
            let w = apply_point(&p, local);
            let d = (w - q).dot(n);
            let want = w + n.scale(-2.0 * d);
            let got = apply_point(&m, mirror_point(local));
            assert!(close(got, want), "{got:?} vs {want:?}");
        }
        // Mirroring twice puts it back.
        let back = mirrored_placement(&m, q, n);
        let x = Point3::new(2.0, -3.0, 1.0);
        assert!(close(apply_point(&back, x), apply_point(&p, x)));
    }

    #[test]
    fn mirroring_features_twice_gives_them_back() {
        let f = vec![
            Feature::Extrude {
                id: FeatId(1),
                profile: Profile2d::rect(0.0, 1.0, 5.0, 4.0),
                plane: Plane3::XZ.at(Point3::new(1.0, 2.0, 3.0)),
                distance: 4.0,
                taper_deg: 0.0,
                op: super::super::types::Op::Add,
            },
            Feature::Hole {
                id: FeatId(2),
                at: Point3::new(2.0, 0.0, 1.0),
                dir: Vec3::Y,
                dia: 1.0,
                depth: super::super::types::Depth::Through,
                cbore: None,
            },
        ];
        let id = |d: DefId| d;
        let once = mirror_features(&f, &id).unwrap();
        assert_ne!(once, f);
        assert_eq!(mirror_features(&once, &id).unwrap(), f);
    }

    fn close(a: Point3, b: Point3) -> bool {
        a.max_abs_diff(b) < 1e-12
    }

    #[test]
    fn compose_and_inverse_agree_with_points() {
        let a =
            rotation_about(Vec3::new(1.0, 2.0, 3.0), 37.0, Point3::new(5.0, -1.0, 2.0)).unwrap();
        let b = Placement::translation(10.0, 20.0, -3.0);
        let p = Point3::new(1.5, -2.0, 7.0);
        let ab = compose(&a, &b);
        assert!(close(
            apply_point(&ab, p),
            apply_point(&a, apply_point(&b, p))
        ));
        let back = apply_point(&inverse(&ab), apply_point(&ab, p));
        assert!(close(back, p), "{back:?}");
        assert!(ab.check().is_ok());
    }

    #[test]
    fn quarter_turn_about_a_point() {
        let r = rotation_about(Vec3::Z, 90.0, Point3::new(1.0, 0.0, 0.0)).unwrap();
        assert!(close(
            apply_point(&r, Point3::new(2.0, 0.0, 0.0)),
            Point3::new(1.0, 1.0, 0.0)
        ));
        assert!(rotation_about(Vec3::new(0.0, 0.0, 0.0), 10.0, Point3::ORIGIN).is_err());
    }

    #[test]
    fn ids_parse_and_print() {
        assert_eq!(parse_part_id("p_12"), Ok(EntityId(12)));
        assert!(parse_part_id("e_12").is_err());
        assert_eq!(parse_def_id(&def_key(DefId(3))), Ok(DefId(3)));
        assert!(parse_def_id("p_3").is_err());
    }
}
