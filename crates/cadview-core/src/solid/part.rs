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
    DefId, EdgeSel, FeatId, Feature, PartDef, Path3, Placement, Plane3, Point3, Seg3, ToolRef, Vec3,
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

#[cfg(test)]
mod tests {
    use super::*;

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
