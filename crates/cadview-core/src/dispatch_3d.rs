//! `cad_call` methods for solid parts (ladder rung 3D1a).
//!
//! A part is a placed instance of a definition; a definition is a feature
//! tree in a general shape language (extrude, revolve, sweep, loft, hole,
//! boolean, fillet, chamfer). Nothing here knows what a part is for: a
//! domain library builds its own vocabulary out of these calls and keeps
//! its meaning in attributes. See docs/api-surface.md, "Solid parts".
//!
//! Conventions:
//!
//! - Coordinates are world millimetres, Z up, angles in degrees. A call
//!   that modifies a part takes positions in world coordinates and moves
//!   them into the part's own frame; `frame: "local"` gives them in that
//!   frame directly.
//! - A modification changes the part's definition, so every part sharing
//!   it changes too (like editing a block definition). `copy3d` with
//!   `unique: true` gives a copy with a definition of its own.
//! - Where a kernel is present (`Document::solids`), every new or changed
//!   definition is evaluated before it is stored. A refusal fails the call
//!   with the part, the feature and the reason, and the document is left
//!   as it was. Without a kernel the definition is checked for shape
//!   (closed profiles, sane numbers) and stored; geometry queries then
//!   fail with `solid::eval::NO_KERNEL`.

use std::collections::{BTreeMap, BTreeSet};

use kurbo::{Line, Point};
use serde_json::{json, Map, Value};

use crate::attrs::{self, Attrs};
use crate::dispatch::parse_entity_id;
use crate::document::Document;
use crate::solid::eval::Solids;
use crate::solid::kernel::{Aabb3, EvalError};
use crate::solid::part::{
    apply_point, compose, def_key, inverse, mirror_features, mirrored_placement, next_feat_id,
    parse_def_id, parse_part_id, part_key, place_feature, rotation_about, tool_defs, Part,
};
use crate::solid::types::{
    CapSide, DefId, Depth, EdgeSel, FeatId, Feature, Op, PartDef, Path3, Placement, Plane3, Point3,
    Profile2d, RoundFallback, Seg2, Seg3, ToolRef, Vec3,
};
use crate::types::{EntityId, LwVertex, Shape};

type R = Result<String, String>;

/// Part methods. `None` for any other method.
pub fn dispatch(doc: &mut Document, method: &str, args: &str) -> Option<R> {
    let r = match method {
        "extrude" | "revolve" | "sweep" | "loft" => create(doc, method, args),
        "cut" => modify_boolean(doc, method, args, Op::Subtract),
        "union" => modify_boolean(doc, method, args, Op::Add),
        "intersect" => modify_boolean(doc, method, args, Op::Intersect),
        "hole" => hole(doc, args),
        "fillet3d" => fillet3d(doc, args),
        "chamfer3d" => chamfer3d(doc, args),
        "move3d" => move3d(doc, args),
        "rotate3d" => rotate3d(doc, args),
        "copy3d" => copy3d(doc, args),
        "mirror3d" => mirror3d(doc, args),
        "remove3d" => remove3d(doc, args),
        "setPartAttrs" => set_part_attrs(doc, args),
        "removePartAttrs" => remove_part_attrs(doc, args),
        "parts" => parts(doc, args),
        "part" => part(doc, args),
        "partDef" => part_def(doc, args),
        _ => return crate::setquery3d::dispatch(doc, method, args),
    };
    Some(r)
}

// ── Argument parsing ───────────────────────────────────────────────────

pub(crate) fn parse(args: &str) -> Result<Value, String> {
    if args.trim().is_empty() {
        return Ok(Value::Object(Map::new()));
    }
    serde_json::from_str(args).map_err(|e| format!("bad args: {e}"))
}

fn finite(v: f64, what: &str) -> Result<f64, String> {
    if v.is_finite() {
        Ok(v)
    } else {
        Err(format!("{what} must be a finite number"))
    }
}

pub(crate) fn num(o: &Value, key: &str) -> Result<Option<f64>, String> {
    match o.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(v) => v
            .as_f64()
            .ok_or_else(|| format!("{key} must be a number"))
            .and_then(|x| finite(x, key))
            .map(Some),
    }
}

fn req_num(o: &Value, key: &str) -> Result<f64, String> {
    num(o, key)?.ok_or_else(|| format!("{key} is required"))
}

fn nums<const N: usize>(v: &Value, what: &str) -> Result<[f64; N], String> {
    let a = v
        .as_array()
        .filter(|a| a.len() == N)
        .ok_or_else(|| format!("{what} must be an array of {N} numbers"))?;
    let mut out = [0.0; N];
    for (o, x) in out.iter_mut().zip(a) {
        *o = finite(
            x.as_f64()
                .ok_or_else(|| format!("{what} must be an array of {N} numbers"))?,
            what,
        )?;
    }
    Ok(out)
}

fn point3(v: &Value, what: &str) -> Result<Point3, String> {
    let [x, y, z] = nums::<3>(v, what)?;
    Ok(Point3::new(x, y, z))
}

fn vec3(v: &Value, what: &str) -> Result<Vec3, String> {
    let [x, y, z] = nums::<3>(v, what)?;
    Ok(Vec3::new(x, y, z))
}

fn opt_point3(o: &Value, key: &str) -> Result<Option<Point3>, String> {
    match o.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(v) => point3(v, key).map(Some),
    }
}

fn opt_vec3(o: &Value, key: &str) -> Result<Option<Vec3>, String> {
    match o.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(v) => vec3(v, key).map(Some),
    }
}

pub(crate) fn p3_json(p: Point3) -> Value {
    json!([p.x, p.y, p.z])
}

pub(crate) fn box_json(b: &Aabb3) -> Value {
    json!({ "min": p3_json(b.min), "max": p3_json(b.max) })
}

/// A sketch plane: `"xy" | "xz" | "yz"` (default `"xy"`) or `{origin, x,
/// normal}`, moved to `at` when given. `xy` has normal +Z, `xz` normal -Y
/// (u along X, v along Z, right-handed) and `yz` normal +X.
fn plane_val(o: &Value) -> Result<Plane3, String> {
    let mut plane = match o.get("plane") {
        None | Some(Value::Null) => Plane3::XY,
        Some(Value::String(s)) => match s.as_str() {
            "xy" => Plane3::XY,
            "xz" => Plane3::XZ,
            "yz" => Plane3::YZ,
            other => {
                return Err(format!(
                    "unknown plane '{other}' (xy|xz|yz or {{origin, x, normal}})"
                ))
            }
        },
        Some(p @ Value::Object(_)) => {
            let origin = opt_point3(p, "origin")?.unwrap_or(Point3::ORIGIN);
            let x =
                opt_vec3(p, "x")?.ok_or("a plane object needs x (its u direction) and normal")?;
            let n = opt_vec3(p, "normal")?
                .ok_or("a plane object needs x (its u direction) and normal")?;
            let x = x.normalized().ok_or("plane x must be a non-zero vector")?;
            let n = n
                .normalized()
                .ok_or("plane normal must be a non-zero vector")?;
            if x.dot(n).abs() > 1e-9 {
                return Err("plane x must be perpendicular to its normal".into());
            }
            Plane3 {
                origin,
                x_axis: x,
                normal: n,
            }
        }
        Some(_) => return Err("plane is \"xy\", \"xz\", \"yz\" or {origin, x, normal}".into()),
    };
    if let Some(at) = opt_point3(o, "at")? {
        plane = plane.at(at);
    }
    plane.frame().map_err(|e| e.to_string())?;
    Ok(plane)
}

fn point2(v: &Value) -> Result<(Point, f64), String> {
    let a = v
        .as_array()
        .filter(|a| a.len() == 2 || a.len() == 3)
        .ok_or("a profile vertex is [x, y] or [x, y, bulge]")?;
    let f = |i: usize| -> Result<f64, String> {
        finite(
            a[i].as_f64().ok_or("profile coordinates must be numbers")?,
            "profile coordinate",
        )
    };
    let bulge = if a.len() == 3 { f(2)? } else { 0.0 };
    Ok((Point::new(f(0)?, f(1)?), bulge))
}

/// One closed loop: vertices (`[x, y]` or `[x, y, bulge]`), a circle
/// `{center, radius}`, a rectangle `{min, max}`, or a closed 2D entity
/// (closed polyline, circle, full ellipse, closed spline) by id or object.
fn loop_val(doc: &Document, v: &Value) -> Result<Vec<Seg2>, String> {
    let invalid = |e: EvalError| e.to_string();
    match v {
        Value::Array(a) => {
            let mut verts = a.iter().map(point2).collect::<Result<Vec<_>, _>>()?;
            // A repeated first vertex at the end closes nothing new.
            if verts.len() > 2 {
                let (first, last) = (verts[0], verts[verts.len() - 1]);
                if first.0.distance(last.0) <= 1e-12 && last.1 == 0.0 {
                    verts.pop();
                }
            }
            let bulged = verts.iter().any(|(_, b)| *b != 0.0);
            if verts.len() < 3 && !bulged {
                return Err("a profile loop needs at least 3 vertices".into());
            }
            let shape = Shape::LwPolyline {
                vertices: verts
                    .into_iter()
                    .map(|(point, bulge)| LwVertex { point, bulge })
                    .collect(),
                closed: true,
            };
            Profile2d::loop_from_shape(&shape).map_err(invalid)
        }
        Value::Object(o) if o.contains_key("id") => entity_loop(doc, v),
        Value::Object(o) if o.contains_key("center") && o.contains_key("radius") => {
            let [cx, cy] = nums::<2>(&o["center"], "center")?;
            let r = req_num(v, "radius")?;
            if r <= 0.0 {
                return Err("circle radius must be > 0".into());
            }
            Ok(Profile2d::circle(Point::new(cx, cy), r).outer)
        }
        Value::Object(o) if o.contains_key("min") && o.contains_key("max") => {
            let [x0, y0] = nums::<2>(&o["min"], "min")?;
            let [x1, y1] = nums::<2>(&o["max"], "max")?;
            if x0 == x1 || y0 == y1 {
                return Err("a rectangle needs a non-zero width and height".into());
            }
            Ok(Profile2d::rect(x0.min(x1), y0.min(y1), x0.max(x1), y0.max(y1)).outer)
        }
        Value::String(_) => entity_loop(doc, v),
        _ => Err(
            "a profile loop is vertices [[x, y, bulge?], ...], {center, radius}, {min, max} \
                  or a closed 2D entity"
                .into(),
        ),
    }
}

fn entity_loop(doc: &Document, v: &Value) -> Result<Vec<Seg2>, String> {
    let id = match v {
        Value::String(s) => parse_entity_id(s)?,
        Value::Object(o) => parse_entity_id(o["id"].as_str().ok_or("entity id must be a string")?)?,
        _ => unreachable!(),
    };
    let ent = doc
        .entity(id)
        .ok_or_else(|| format!("entity e_{} not found", id.0))?;
    Profile2d::loop_from_shape(&ent.shape).map_err(|e| format!("entity e_{}: {e}", id.0))
}

/// A profile: one loop (see [`loop_val`]) or `{outer, holes}` (the shape
/// `regionClip` returns). Kept as exact lines and arcs.
fn profile_val(doc: &Document, v: &Value) -> Result<Profile2d, String> {
    let profile = match v {
        Value::Object(o) if o.contains_key("outer") => {
            let outer = loop_val(doc, &o["outer"])?;
            let holes = match o.get("holes") {
                None | Some(Value::Null) => Vec::new(),
                Some(Value::Array(h)) => h
                    .iter()
                    .map(|x| loop_val(doc, x))
                    .collect::<Result<Vec<_>, _>>()?,
                Some(_) => return Err("profile holes must be an array of loops".into()),
            };
            Profile2d { outer, holes }
        }
        Value::Null => return Err("profile is required".into()),
        other => Profile2d {
            outer: loop_val(doc, other)?,
            holes: Vec::new(),
        },
    };
    profile.check().map_err(|e| e.to_string())?;
    Ok(profile)
}

/// A sweep path: `[{to: [x, y, z]}, {via: [...], to: [...]}, ...]`, lines
/// and arcs starting at the plane's origin.
fn path_val(v: &Value, start: Point3) -> Result<Path3, String> {
    let a = v
        .as_array()
        .filter(|a| !a.is_empty())
        .ok_or("path is a non-empty array of {to} (line) and {via, to} (arc) segments")?;
    let mut from = start;
    let mut segs = Vec::with_capacity(a.len());
    for (i, s) in a.iter().enumerate() {
        let to = opt_point3(s, "to")?.ok_or_else(|| format!("path segment {i} needs to"))?;
        segs.push(match opt_point3(s, "via")? {
            Some(via) => Seg3::Arc { from, via, to },
            None => Seg3::Line { from, to },
        });
        from = to;
    }
    Ok(Path3 { segs })
}

/// A body feature from its kind-specific options: `extrude` (profile,
/// plane, at, distance, taper), `revolve` (profile, plane, at, axis,
/// angle), `sweep` (profile, plane, at, path) or `loft` (sections, ruled).
fn body_feature(
    doc: &Document,
    kind: &str,
    main: &Value,
    o: &Value,
    op: Op,
    id: FeatId,
) -> Result<Feature, String> {
    match kind {
        "extrude" => {
            let profile = profile_val(doc, main)?;
            let plane = plane_val(o)?;
            let distance = req_num(o, "distance")?;
            if distance == 0.0 {
                return Err("extrude distance must be non-zero (negative extrudes against the plane normal)".into());
            }
            let taper_deg = num(o, "taper")?.unwrap_or(0.0);
            Ok(Feature::Extrude {
                id,
                profile,
                plane,
                distance,
                taper_deg,
                op,
            })
        }
        "revolve" => {
            let profile = profile_val(doc, main)?;
            let plane = plane_val(o)?;
            let axis = o
                .get("axis")
                .ok_or("revolve needs axis: [[u0, v0], [u1, v1]] in profile coordinates")?;
            let ends = axis
                .as_array()
                .filter(|a| a.len() == 2)
                .ok_or("axis is [[u0, v0], [u1, v1]]")?;
            let [u0, v0] = nums::<2>(&ends[0], "axis")?;
            let [u1, v1] = nums::<2>(&ends[1], "axis")?;
            if u0 == u1 && v0 == v1 {
                return Err("revolve axis needs two different points".into());
            }
            let angle_deg = num(o, "angle")?.unwrap_or(360.0);
            if !(angle_deg > 0.0 && angle_deg <= 360.0) {
                return Err("revolve angle must be in (0, 360] degrees".into());
            }
            Ok(Feature::Revolve {
                id,
                profile,
                plane,
                axis: Line::new(Point::new(u0, v0), Point::new(u1, v1)),
                angle_deg,
                op,
            })
        }
        "sweep" => {
            let profile = profile_val(doc, main)?;
            let plane = plane_val(o)?;
            let path = path_val(o.get("path").unwrap_or(&Value::Null), plane.origin)?;
            path.check(&plane).map_err(|e| e.to_string())?;
            Ok(Feature::Sweep {
                id,
                profile,
                plane,
                path,
                op,
            })
        }
        "loft" => {
            let a = main
                .as_array()
                .filter(|a| a.len() >= 2)
                .ok_or("loft needs at least two sections [{profile, plane, at}, ...]")?;
            let sections = a
                .iter()
                .enumerate()
                .map(|(i, s)| {
                    let p = profile_val(doc, s.get("profile").unwrap_or(&Value::Null))
                        .map_err(|e| format!("section {i}: {e}"))?;
                    let pl = plane_val(s).map_err(|e| format!("section {i}: {e}"))?;
                    Ok((p, pl))
                })
                .collect::<Result<Vec<_>, String>>()?;
            let ruled = o.get("ruled").and_then(Value::as_bool).unwrap_or(false);
            Ok(Feature::Loft {
                id,
                sections,
                ruled,
                op,
            })
        }
        other => Err(format!("unknown body kind '{other}'")),
    }
}

/// `"world"` (default) or `"local"`.
fn local_frame(o: &Value) -> Result<bool, String> {
    match o.get("frame").and_then(Value::as_str) {
        None | Some("world") => Ok(false),
        Some("local") => Ok(true),
        Some(other) => Err(format!("frame is \"world\" or \"local\", got '{other}'")),
    }
}

// ── Part selection ─────────────────────────────────────────────────────

pub(crate) fn part_id_of(v: &Value) -> Result<EntityId, String> {
    match v {
        Value::String(s) => parse_part_id(s),
        Value::Object(o) => o
            .get("id")
            .and_then(Value::as_str)
            .ok_or_else(|| "part object missing 'id'".to_string())
            .and_then(parse_part_id),
        _ => Err("a part is an id (p_12) or a part object".into()),
    }
}

/// The keys a part selector may have.
const PART_SELECTOR_KEYS: &str = "ids, def, attrs, has, bounds";

/// A part selection: an id, a list of ids or part objects, or a selector
/// `{ids, def, attrs, has, bounds}` (the entity selector's shape; `bounds`
/// is `{min, max}` with 3 coordinates for a box or 2 for the plan).
struct PartSel {
    ids: Option<Vec<EntityId>>,
    def: Option<DefId>,
    attrs: attrs::Updates,
    has: Vec<String>,
    bounds: Option<(Point3, Point3)>,
}

impl PartSel {
    fn parse(v: &Value) -> Result<PartSel, String> {
        let mut s = PartSel {
            ids: None,
            def: None,
            attrs: Vec::new(),
            has: Vec::new(),
            bounds: None,
        };
        match v {
            Value::Null => {}
            Value::String(_) => s.ids = Some(vec![part_id_of(v)?]),
            Value::Array(a) => s.ids = Some(a.iter().map(part_id_of).collect::<Result<_, _>>()?),
            Value::Object(o) if o.contains_key("id") => s.ids = Some(vec![part_id_of(v)?]),
            Value::Object(o) => {
                for (k, x) in o {
                    if x.is_null() && k != "attrs" {
                        continue;
                    }
                    match k.as_str() {
                        "ids" => {
                            let a = x.as_array().ok_or("ids must be an array")?;
                            s.ids = Some(a.iter().map(part_id_of).collect::<Result<_, _>>()?);
                        }
                        "def" => {
                            s.def = Some(parse_def_id(
                                x.as_str().ok_or("def must be a definition id (d_3)")?,
                            )?)
                        }
                        "attrs" => {
                            if !x.is_null() {
                                s.attrs = attrs::parse_updates(x)?;
                            }
                        }
                        "has" => {
                            let a = x.as_array().ok_or("has must be an array of keys")?;
                            for k in a {
                                let k = k.as_str().ok_or("has must be an array of keys")?;
                                attrs::check_key(k)?;
                                s.has.push(k.to_string());
                            }
                        }
                        "bounds" => s.bounds = Some(bounds_val(x)?),
                        other => {
                            return Err(format!(
                                "unknown part selector key '{other}' (use {PART_SELECTOR_KEYS})"
                            ))
                        }
                    }
                }
            }
            _ => {
                return Err(format!(
                    "a part selection is an id, a list of ids, or {{{PART_SELECTOR_KEYS}}}"
                ))
            }
        }
        Ok(s)
    }
}

pub(crate) fn bounds_val(v: &Value) -> Result<(Point3, Point3), String> {
    let corner = |k: &str, open: f64| -> Result<Point3, String> {
        let a = v
            .get(k)
            .and_then(Value::as_array)
            .ok_or("bounds is {min: [x, y, z?], max: [x, y, z?]}")?;
        let c = |i: usize| -> Result<f64, String> {
            match a.get(i) {
                Some(x) => x
                    .as_f64()
                    .ok_or_else(|| "bounds coordinates are numbers".to_string()),
                None if i == 2 => Ok(open),
                None => Err("bounds is {min: [x, y, z?], max: [x, y, z?]}".into()),
            }
        };
        Ok(Point3::new(c(0)?, c(1)?, c(2)?))
    };
    let (a, b) = (
        corner("min", f64::NEG_INFINITY)?,
        corner("max", f64::INFINITY)?,
    );
    Ok((
        Point3::new(a.x.min(b.x), a.y.min(b.y), a.z.min(b.z)),
        Point3::new(a.x.max(b.x), a.y.max(b.y), a.z.max(b.z)),
    ))
}

/// The parts a selection picks, in id order (a list of ids keeps its own
/// order). An id that names no part is an error.
fn select_parts(doc: &Document, sel: &PartSel) -> Result<Vec<EntityId>, String> {
    if let Some(ids) = &sel.ids {
        for id in ids {
            if doc.part(*id).is_none() {
                return Err(format!("part {} not found", part_key(*id)));
            }
        }
    }
    let mut out = Vec::new();
    let candidates: Vec<EntityId> = match &sel.ids {
        Some(ids) => ids.clone(),
        None => doc.parts.iter().map(|p| p.id).collect(),
    };
    for id in candidates {
        let p = doc.part(id).expect("checked");
        if sel.def.is_some_and(|d| d != p.def) {
            continue;
        }
        if !sel.attrs.iter().all(|(k, v)| p.attrs.get(k) == v.as_ref()) {
            continue;
        }
        if !sel.has.iter().all(|k| p.attrs.contains_key(k)) {
            continue;
        }
        if let Some((lo, hi)) = sel.bounds {
            let solids = doc.solids.get().map_err(|e| format!("bounds: {e}"))?;
            let b = world_bbox(doc, solids, p)?;
            if b.max.x < lo.x
                || b.min.x > hi.x
                || b.max.y < lo.y
                || b.min.y > hi.y
                || b.max.z < lo.z
                || b.min.z > hi.z
            {
                continue;
            }
        }
        out.push(id);
    }
    Ok(out)
}

/// The parts a selection value picks (see [`PartSel`]); null picks all.
pub(crate) fn select_parts_value(doc: &Document, v: &Value) -> Result<Vec<EntityId>, String> {
    select_parts(doc, &PartSel::parse(v)?)
}

fn target(doc: &Document, o: &Value) -> Result<Vec<EntityId>, String> {
    let t = o.get("target").ok_or("target is required")?;
    let ids = select_parts(doc, &PartSel::parse(t)?)?;
    Ok(ids)
}

fn one_target(doc: &Document, o: &Value) -> Result<EntityId, String> {
    let ids = target(doc, o)?;
    match ids.as_slice() {
        [id] => Ok(*id),
        [] => Err("target selects no part".into()),
        _ => Err(format!(
            "target selects {} parts; this call modifies one part's definition",
            ids.len()
        )),
    }
}

// ── Evaluation and errors ──────────────────────────────────────────────

/// The reason of an evaluation error without the "feature N:" prefix, and
/// the feature it names.
fn reason_of(e: &EvalError) -> (Option<FeatId>, String) {
    match e {
        EvalError::Unsupported { feature, what } => (
            *feature,
            format!("{what} is not supported by this kernel yet"),
        ),
        EvalError::Invalid { feature, reason } => (*feature, reason.clone()),
        EvalError::Selector { feature, reason } => {
            (Some(*feature), format!("edge selector: {reason}"))
        }
        EvalError::UnknownTool { feature, def } => (
            Some(*feature),
            format!("tool definition {} is not known", def_key(*def)),
        ),
        EvalError::ToolCycle { feature, def } => (
            Some(*feature),
            format!("tool definition {} refers back to this part", def_key(*def)),
        ),
        EvalError::Kernel { feature, message } => (*feature, format!("kernel: {message}")),
    }
}

/// "part p_3 (\"side\"), feature 2 (hole): reason".
pub(crate) fn refusal(def: &PartDef, who: &str, e: &EvalError) -> String {
    let (feature, reason) = reason_of(e);
    let name = if def.name.is_empty() {
        String::new()
    } else {
        format!(" (\"{}\")", def.name)
    };
    let feat = match feature {
        Some(f) => {
            let kind = def
                .features
                .iter()
                .find(|x| x.id() == f)
                .map(Feature::kind_name)
                .unwrap_or("?");
            format!(", feature {} ({kind})", f.0)
        }
        None => String::new(),
    };
    format!("{who}{name}{feat}: {reason}")
}

/// Who a definition stands for in messages: its parts, or the definition.
pub(crate) fn who(doc: &Document, def: DefId) -> String {
    let parts: Vec<String> = doc
        .parts
        .iter()
        .filter(|p| p.def == def)
        .map(|p| part_key(p.id))
        .collect();
    match parts.as_slice() {
        [] => format!("definition {}", def_key(def)),
        [one] => format!("part {one}"),
        many => format!("parts {}", many.join(", ")),
    }
}

/// The definitions that reach `id` through tool references (not `id`).
fn dependents(defs: &BTreeMap<DefId, PartDef>, id: DefId) -> Vec<DefId> {
    let mut out = Vec::new();
    let mut frontier = vec![id];
    while let Some(cur) = frontier.pop() {
        for d in defs.values() {
            if d.id != id && !out.contains(&d.id) && tool_defs(d).any(|t| t == cur) {
                out.push(d.id);
                frontier.push(d.id);
            }
        }
    }
    out
}

/// Store `def` (new or changed) if the kernel accepts it and every
/// definition that uses it as a tool; otherwise leave the definitions as
/// they were and return the refusal.
fn commit_def(doc: &mut Document, method: &str, def: PartDef) -> Result<(), String> {
    let id = def.id;
    // A definition may not reach itself through its tools (checked here
    // too, so a build without a kernel never stores a cycle).
    let mut seen = BTreeSet::new();
    let mut todo: Vec<DefId> = tool_defs(&def).collect();
    while let Some(t) = todo.pop() {
        if t == id {
            return Err(format!(
                "{method}: {}: a tool of this part uses the part itself",
                who(doc, id)
            ));
        }
        if seen.insert(t) {
            if let Some(td) = doc.part_defs.get(&t) {
                todo.extend(tool_defs(td));
            }
        }
    }
    let old = doc.part_defs.insert(id, def);
    if let Ok(solids) = doc.solids.get() {
        let mut check = vec![id];
        check.extend(dependents(&doc.part_defs, id));
        for d in check {
            let def = &doc.part_defs[&d];
            if let Err(e) = solids.solid(&doc.part_defs, def) {
                let msg = format!("{method}: {}", refusal(def, &who(doc, d), &e));
                match old {
                    Some(o) => doc.part_defs.insert(id, o),
                    None => doc.part_defs.remove(&id),
                };
                return Err(msg);
            }
        }
    }
    Ok(())
}

/// Drop definitions no part uses, directly or as a tool.
fn gc_defs(doc: &mut Document) -> Vec<DefId> {
    let mut keep: BTreeSet<DefId> = doc.parts.iter().map(|p| p.def).collect();
    let mut frontier: Vec<DefId> = keep.iter().copied().collect();
    while let Some(d) = frontier.pop() {
        if let Some(def) = doc.part_defs.get(&d) {
            for t in tool_defs(def) {
                if keep.insert(t) {
                    frontier.push(t);
                }
            }
        }
    }
    let gone: Vec<DefId> = doc
        .part_defs
        .keys()
        .filter(|d| !keep.contains(d))
        .copied()
        .collect();
    for d in &gone {
        doc.part_defs.remove(d);
    }
    gone
}

fn world_bbox(doc: &Document, solids: &Solids, p: &Part) -> Result<Aabb3, String> {
    let def = &doc.part_defs[&p.def];
    solids
        .bbox(&doc.part_defs, def, &p.placement)
        .map_err(|e| refusal(def, &format!("part {}", part_key(p.id)), &e))
}

// ── Results ────────────────────────────────────────────────────────────

/// A part as scripts see it. With `geometry`, also its tight box in the
/// world and in its own frame, its volume and its centroid (needs a
/// kernel).
fn part_json(doc: &Document, p: &Part, geometry: bool) -> Result<Value, String> {
    let def = doc.part_defs.get(&p.def).ok_or_else(|| {
        format!(
            "part {}: definition {} is missing",
            part_key(p.id),
            def_key(p.def)
        )
    })?;
    let mut v = json!({
        "id": part_key(p.id),
        "def": def_key(p.def),
        "name": def.name,
        "placement": {
            "translation": [p.placement.translation.x, p.placement.translation.y, p.placement.translation.z],
            "rotation": p.placement.rotation,
        },
        "features": def.features.iter().map(|f| json!({"id": f.id().0, "kind": f.kind_name()})).collect::<Vec<_>>(),
        "instances": doc.parts.iter().filter(|q| q.def == p.def).count(),
    });
    if !p.attrs.is_empty() {
        v["attrs"] = attrs::to_json(&p.attrs);
    }
    if geometry {
        let solids = doc.solids.get()?;
        let who = format!("part {}", part_key(p.id));
        let err = |e: EvalError| refusal(def, &who, &e);
        let world = solids
            .tight_bbox(&doc.part_defs, def, &p.placement)
            .map_err(err)?;
        let local = solids.local_bbox(&doc.part_defs, def).map_err(err)?;
        let mass = solids.mass(&doc.part_defs, def).map_err(err)?;
        v["bbox"] = box_json(&world.aabb);
        v["localBbox"] = box_json(&local.aabb);
        // 0: both boxes are exact. Otherwise each side lies at most this far
        // (mm) outside the solid (a box the kernel cannot make exact).
        v["bboxTol"] = json!(world.tol.max(local.tol));
        v["volume"] = json!(mass.volume);
        v["centroid"] = match mass.centroid {
            Some(c) => p3_json(apply_point(&p.placement, c)),
            None => Value::Null,
        };
    }
    Ok(v)
}

/// The result of a creating or modifying call: the part, with geometry
/// when a kernel is present.
fn result_of(doc: &Document, id: EntityId) -> R {
    let p = doc.part(id).expect("part exists");
    Ok(part_json(doc, p, doc.solids.is_some())?.to_string())
}

// ── Create ─────────────────────────────────────────────────────────────

fn name_of(o: &Value) -> Result<String, String> {
    match o.get("name") {
        None | Some(Value::Null) => Ok(String::new()),
        Some(Value::String(s)) => Ok(s.clone()),
        Some(_) => Err("name must be a string".into()),
    }
}

fn create(doc: &mut Document, method: &str, args: &str) -> R {
    let o = parse(args)?;
    let main = match method {
        "loft" => o.get("sections").cloned().unwrap_or(Value::Null),
        _ => o.get("profile").cloned().unwrap_or(Value::Null),
    };
    let feature = body_feature(doc, method, &main, &o, Op::Add, FeatId(1))
        .map_err(|e| format!("{method}: {e}"))?;
    let attrs = attrs::parse_opt(&o.get("attrs").cloned())?;
    let name = name_of(&o)?;

    let next = doc.next_id();
    let def_id = doc.alloc_def_id();
    let def = PartDef {
        id: def_id,
        name,
        features: vec![feature],
    };
    if let Err(e) = commit_def(doc, method, def) {
        doc.set_next_id(next);
        return Err(e);
    }
    let id = doc.alloc_id();
    doc.parts.push(Part {
        id,
        def: def_id,
        placement: Placement::IDENTITY,
        attrs,
    });
    // The kernel accepted the feature but may still fail to report the
    // part's geometry (an unsupported box or volume): then the call fails
    // as a whole and the document stays as it was.
    result_of(doc, id).map_err(|e| {
        doc.parts.retain(|p| p.id != id);
        doc.part_defs.remove(&def_id);
        doc.set_next_id(next);
        format!("{method}: {e}")
    })
}

// ── Modify ─────────────────────────────────────────────────────────────

/// Add `feature` (given in world coordinates unless `local`) to the
/// definition of part `id`.
fn add_feature(doc: &mut Document, method: &str, id: EntityId, feature: Feature, local: bool) -> R {
    let p = doc.part(id).expect("selected").clone();
    let feature = if local {
        feature
    } else {
        place_feature(&inverse(&p.placement), &feature)
    };
    let old = doc.part_defs[&p.def].clone();
    let mut def = old.clone();
    def.features.push(feature);
    commit_def(doc, method, def)?;
    // As in `create`: a part whose geometry cannot be reported is not kept.
    result_of(doc, id).map_err(|e| {
        doc.part_defs.insert(old.id, old);
        format!("{method}: {e}")
    })
}

fn modify_boolean(doc: &mut Document, method: &str, args: &str, op: Op) -> R {
    let o = parse(args)?;
    let id = one_target(doc, &o).map_err(|e| format!("{method}: {e}"))?;
    let local = local_frame(&o)?;
    let tool = o.get("tool").ok_or_else(|| {
        format!("{method}: tool is required ({{extrude|revolve|sweep|loft: ...}} or {{part}})")
    })?;
    let def_id = doc.part(id).expect("selected").def;
    let fid = next_feat_id(&doc.part_defs[&def_id]);
    let kinds = ["extrude", "revolve", "sweep", "loft", "part"];
    let kind = kinds
        .iter()
        .find(|k| tool.get(**k).is_some())
        .ok_or_else(|| {
            format!("{method}: tool needs one of extrude, revolve, sweep, loft or part")
        })?;
    if *kind == "part" {
        let tid = part_id_of(&tool["part"]).map_err(|e| format!("{method}: tool: {e}"))?;
        let t = doc
            .part(tid)
            .ok_or_else(|| format!("{method}: tool part {} not found", part_key(tid)))?;
        if tid == id {
            return Err(format!("{method}: a part cannot be its own tool"));
        }
        // The tool as placed now, in the world (moved into the target's
        // frame by add_feature). Moving the tool later does not move the
        // cut; editing the tool's definition does change it.
        let feature = Feature::Boolean {
            id: fid,
            op,
            tool: ToolRef::Def(t.def, t.placement),
        };
        return add_feature(doc, method, id, feature, false);
    }
    let main = tool.get(*kind).expect("found");
    let feature =
        body_feature(doc, kind, main, tool, op, fid).map_err(|e| format!("{method}: {e}"))?;
    add_feature(doc, method, id, feature, local)
}

fn hole(doc: &mut Document, args: &str) -> R {
    let o = parse(args)?;
    let id = one_target(doc, &o).map_err(|e| format!("hole: {e}"))?;
    let local = local_frame(&o)?;
    let at = opt_point3(&o, "at")?.ok_or("hole: at is required")?;
    let dir = opt_vec3(&o, "dir")?
        .unwrap_or(Vec3::new(0.0, 0.0, -1.0))
        .normalized()
        .ok_or("hole: dir must be a non-zero vector")?;
    let dia = req_num(&o, "dia").map_err(|e| format!("hole: {e}"))?;
    if dia <= 0.0 {
        return Err("hole: dia must be > 0".into());
    }
    let depth = match o.get("depth") {
        None | Some(Value::Null) => Depth::Through,
        Some(Value::String(s)) if s == "through" => Depth::Through,
        Some(v) => {
            let d = finite(
                v.as_f64().ok_or("hole: depth is a number or \"through\"")?,
                "depth",
            )?;
            if d <= 0.0 {
                return Err("hole: depth must be > 0".into());
            }
            Depth::Blind(d)
        }
    };
    let cbore = match o.get("cbore").or_else(|| o.get("counterbore")) {
        None | Some(Value::Null) => None,
        Some(v) => {
            let [cd, ch] = nums::<2>(v, "counterbore")?;
            if cd <= dia || ch <= 0.0 {
                return Err(
                    "hole: counterbore is [dia, depth] with dia larger than the hole and depth > 0"
                        .into(),
                );
            }
            Some((cd, ch))
        }
    };
    let def_id = doc.part(id).expect("selected").def;
    let feature = Feature::Hole {
        id: next_feat_id(&doc.part_defs[&def_id]),
        at,
        dir,
        dia,
        depth,
        cbore,
    };
    add_feature(doc, "hole", id, feature, local)
}

/// Edges: `"all"`, `{cap: "start"|"end"|"both", feature?, segs?}`,
/// `{lateral: [segs], feature?}` or `{near: [[x, y, z], ...]}`. `feature`
/// defaults to the definition's first feature.
fn edges_val(o: &Value, def: &PartDef) -> Result<EdgeSel, String> {
    let first = def.features.first().map(Feature::id).unwrap_or(FeatId(1));
    let e = o.get("edges").unwrap_or(&Value::Null);
    let feature = |v: &Value| -> Result<FeatId, String> {
        match v.get("feature") {
            None | Some(Value::Null) => Ok(first),
            Some(f) => {
                let n = f
                    .as_u64()
                    .ok_or("edges.feature is a feature id (a number)")?;
                let id = FeatId(n as u32);
                if !def.features.iter().any(|x| x.id() == id) {
                    return Err(format!("edges.feature: the definition has no feature {n}"));
                }
                Ok(id)
            }
        }
    };
    let segs = |v: &Value| -> Result<Vec<usize>, String> {
        v.as_array()
            .ok_or("segment lists are arrays of indices")?
            .iter()
            .map(|x| {
                x.as_u64()
                    .map(|n| n as usize)
                    .ok_or_else(|| "segment indices are whole numbers".to_string())
            })
            .collect()
    };
    match e {
        Value::String(s) if s == "all" => Ok(EdgeSel::All),
        Value::Object(m) if m.contains_key("cap") => {
            let side = match m["cap"].as_str() {
                Some("start") => CapSide::Start,
                Some("end") => CapSide::End,
                Some("both") => CapSide::Both,
                _ => return Err("edges.cap is \"start\", \"end\" or \"both\"".into()),
            };
            let s = match m.get("segs") {
                None | Some(Value::Null) => None,
                Some(v) => Some(segs(v)?),
            };
            Ok(EdgeSel::Cap {
                feature: feature(e)?,
                side,
                segs: s,
            })
        }
        Value::Object(m) if m.contains_key("lateral") => Ok(EdgeSel::Lateral {
            feature: feature(e)?,
            segs: segs(&m["lateral"])?,
        }),
        Value::Object(m) if m.contains_key("near") => {
            let pts = m["near"]
                .as_array()
                .ok_or("edges.near is an array of points")?
                .iter()
                .map(|p| point3(p, "edges.near point"))
                .collect::<Result<Vec<_>, _>>()?;
            if pts.is_empty() {
                return Err("edges.near needs at least one point".into());
            }
            Ok(EdgeSel::Near { points: pts })
        }
        _ => Err(
            "edges is \"all\", {cap, feature?, segs?}, {lateral, feature?} or {near: [points]}"
                .into(),
        ),
    }
}

fn fillet3d(doc: &mut Document, args: &str) -> R {
    let o = parse(args)?;
    let id = one_target(doc, &o).map_err(|e| format!("fillet3d: {e}"))?;
    let local = local_frame(&o)?;
    let def = &doc.part_defs[&doc.part(id).expect("selected").def];
    let edges = edges_val(&o, def).map_err(|e| format!("fillet3d: {e}"))?;
    let radius = req_num(&o, "radius").map_err(|e| format!("fillet3d: {e}"))?;
    if radius <= 0.0 {
        return Err("fillet3d: radius must be > 0".into());
    }
    let fallback = match o.get("fallback").and_then(Value::as_str) {
        None | Some("error") => RoundFallback::Error,
        Some("visual") => RoundFallback::Visual,
        Some(other) => {
            return Err(format!(
                "fillet3d: fallback is \"error\" or \"visual\", got '{other}'"
            ))
        }
    };
    let feature = Feature::Fillet {
        id: next_feat_id(def),
        edges,
        radius,
        fallback,
    };
    add_feature(doc, "fillet3d", id, feature, local)
}

fn chamfer3d(doc: &mut Document, args: &str) -> R {
    let o = parse(args)?;
    let id = one_target(doc, &o).map_err(|e| format!("chamfer3d: {e}"))?;
    let local = local_frame(&o)?;
    let def = &doc.part_defs[&doc.part(id).expect("selected").def];
    let edges = edges_val(&o, def).map_err(|e| format!("chamfer3d: {e}"))?;
    let d1 = req_num(&o, "d").map_err(|e| format!("chamfer3d: {e}"))?;
    let d2 = num(&o, "d2")?.unwrap_or(d1);
    if d1 <= 0.0 || d2 <= 0.0 {
        return Err("chamfer3d: d (and d2) must be > 0".into());
    }
    let feature = Feature::Chamfer {
        id: next_feat_id(def),
        edges,
        d1,
        d2,
    };
    add_feature(doc, "chamfer3d", id, feature, local)
}

// ── Place ──────────────────────────────────────────────────────────────

fn keys(ids: &[EntityId]) -> Vec<String> {
    ids.iter().map(|i| part_key(*i)).collect()
}

fn move3d(doc: &mut Document, args: &str) -> R {
    let o = parse(args)?;
    let ids = target(doc, &o).map_err(|e| format!("move3d: {e}"))?;
    let by = opt_vec3(&o, "by")?.ok_or("move3d: by: [dx, dy, dz] is required")?;
    let t = Placement {
        rotation: Placement::IDENTITY.rotation,
        translation: by,
    };
    for id in &ids {
        let p = doc.part_mut(*id).expect("selected");
        p.placement = compose(&t, &p.placement);
    }
    Ok(json!({ "moved": keys(&ids) }).to_string())
}

/// `{angle, axis = [0, 0, 1], about = [0, 0, 0]}` as a placement.
fn rotation_val(o: &Value) -> Result<Placement, String> {
    let angle = req_num(o, "angle")?;
    let axis = opt_vec3(o, "axis")?.unwrap_or(Vec3::Z);
    let about = opt_point3(o, "about")?.unwrap_or(Point3::ORIGIN);
    rotation_about(axis, angle, about)
}

fn rotate3d(doc: &mut Document, args: &str) -> R {
    let o = parse(args)?;
    let ids = target(doc, &o).map_err(|e| format!("rotate3d: {e}"))?;
    let r = rotation_val(&o).map_err(|e| format!("rotate3d: {e}"))?;
    for id in &ids {
        let p = doc.part_mut(*id).expect("selected");
        p.placement = compose(&r, &p.placement);
    }
    Ok(json!({ "rotated": keys(&ids) }).to_string())
}

fn copy3d(doc: &mut Document, args: &str) -> R {
    let o = parse(args)?;
    let ids = target(doc, &o).map_err(|e| format!("copy3d: {e}"))?;
    let mv = opt_vec3(&o, "move")?.unwrap_or(Vec3::new(0.0, 0.0, 0.0));
    let rot = match o.get("rotate") {
        None | Some(Value::Null) => Placement::IDENTITY,
        Some(r) => rotation_val(r).map_err(|e| format!("copy3d: rotate: {e}"))?,
    };
    let step = compose(
        &Placement {
            rotation: Placement::IDENTITY.rotation,
            translation: mv,
        },
        &rot,
    );
    let count = match o.get("count") {
        None | Some(Value::Null) => 1,
        Some(v) => {
            let n = v.as_u64().filter(|n| (1..=10_000).contains(n));
            n.ok_or("copy3d: count is a whole number from 1 to 10000")? as usize
        }
    };
    let updates = match o.get("attrs") {
        None | Some(Value::Null) => Vec::new(),
        Some(v) => attrs::parse_updates(v)?,
    };
    let unique = o.get("unique").and_then(Value::as_bool).unwrap_or(false);
    let snapshot_next = doc.next_id();
    let mut made = Vec::new();
    let mut new_defs = Vec::new();
    for id in &ids {
        let src = doc.part(*id).expect("selected").clone();
        let mut attrs: Attrs = src.attrs.clone();
        if let Err(e) = attrs::apply(&mut attrs, &updates, false) {
            doc.set_next_id(snapshot_next);
            return Err(format!("copy3d: {e}"));
        }
        let mut placement = src.placement;
        for _ in 0..count {
            placement = compose(&step, &placement);
            let def = if unique {
                let mut d = doc.part_defs[&src.def].clone();
                d.id = doc.alloc_def_id();
                let did = d.id;
                doc.part_defs.insert(did, d);
                new_defs.push(did);
                did
            } else {
                src.def
            };
            let nid = doc.alloc_id();
            doc.parts.push(Part {
                id: nid,
                def,
                placement,
                attrs: attrs.clone(),
            });
            made.push(nid);
        }
    }
    let geometry = doc.solids.is_some();
    let out = match made
        .iter()
        .map(|id| part_json(doc, doc.part(*id).expect("made"), geometry))
        .collect::<Result<Vec<_>, _>>()
    {
        Ok(out) => out,
        Err(e) => {
            // Leave the document as it was: no copies, no new definitions.
            doc.parts.retain(|p| !made.contains(&p.id));
            for d in &new_defs {
                doc.part_defs.remove(d);
            }
            doc.set_next_id(snapshot_next);
            return Err(format!("copy3d: {e}"));
        }
    };
    let single = matches!(o.get("target"), Some(Value::String(_))) && count == 1
        || matches!(o.get("target"), Some(Value::Object(m)) if m.contains_key("id")) && count == 1;
    Ok(if single {
        out.into_iter().next().unwrap_or(Value::Null).to_string()
    } else {
        Value::Array(out).to_string()
    })
}

/// The mirrored definition of `src` (see `solid::part`, "Mirror images"),
/// with its tools mirrored too: an existing definition with exactly those
/// features is reused (so mirroring back finds the original), otherwise a
/// new one is stored and listed in `made`.
fn mirrored_def(
    doc: &mut Document,
    src: DefId,
    memo: &mut BTreeMap<DefId, DefId>,
    made: &mut Vec<DefId>,
) -> Result<DefId, String> {
    if let Some(d) = memo.get(&src) {
        return Ok(*d);
    }
    let def = doc
        .part_defs
        .get(&src)
        .cloned()
        .ok_or_else(|| format!("definition {} is missing", def_key(src)))?;
    let mut tools = BTreeMap::new();
    for t in tool_defs(&def).collect::<Vec<_>>() {
        let m = mirrored_def(doc, t, memo, made)?;
        tools.insert(t, m);
    }
    let features = mirror_features(&def.features, &|d| tools.get(&d).copied().unwrap_or(d))
        .map_err(|e| format!("{}: {e}", who(doc, src)))?;
    let id = match doc.part_defs.values().find(|d| d.features == features) {
        Some(existing) => existing.id,
        None => {
            let id = doc.alloc_def_id();
            doc.part_defs.insert(
                id,
                PartDef {
                    id,
                    name: def.name.clone(),
                    features,
                },
            );
            made.push(id);
            id
        }
    };
    memo.insert(src, id);
    Ok(id)
}

/// `mirror3d({target, point?, normal, copy?, attrs?})`: the parts
/// reflected across the plane through `point` (default the origin) with
/// `normal`. A reflection is not a rigid motion, so a mirrored part places
/// a mirrored definition (made once per definition and shared by all its
/// mirrored parts). In place by default; `copy: true` keeps the originals
/// and adds the images (with `attrs` merged into theirs).
fn mirror3d(doc: &mut Document, args: &str) -> R {
    let o = parse(args)?;
    let ids = target(doc, &o).map_err(|e| format!("mirror3d: {e}"))?;
    let point = opt_point3(&o, "point")?.unwrap_or(Point3::ORIGIN);
    let normal = opt_vec3(&o, "normal")?
        .ok_or("mirror3d: normal: [nx, ny, nz] (the mirror plane's normal) is required")?
        .normalized()
        .ok_or("mirror3d: normal must be a non-zero vector")?;
    let copy = o.get("copy").and_then(Value::as_bool).unwrap_or(false);
    let updates = match o.get("attrs") {
        None | Some(Value::Null) => Vec::new(),
        Some(v) => attrs::parse_updates(v)?,
    };
    let (before_defs, before_parts, before_next) =
        (doc.part_defs.clone(), doc.parts.clone(), doc.next_id());
    let restore = |doc: &mut Document, e: String| {
        doc.part_defs = before_defs.clone();
        doc.parts = before_parts.clone();
        doc.set_next_id(before_next);
        Err(format!("mirror3d: {e}"))
    };
    let mut memo = BTreeMap::new();
    let mut made = Vec::new();
    for id in &ids {
        let src = doc.part(*id).expect("selected").def;
        if let Err(e) = mirrored_def(doc, src, &mut memo, &mut made) {
            return restore(doc, e);
        }
    }
    if let Ok(solids) = doc.solids.get() {
        for d in &made {
            let def = &doc.part_defs[d];
            if let Err(e) = solids.solid(&doc.part_defs, def) {
                let msg = refusal(def, &who(doc, *d), &e);
                return restore(doc, msg);
            }
        }
    }
    let mut out_ids = Vec::new();
    for id in &ids {
        let p = doc.part(*id).expect("selected").clone();
        let placement = mirrored_placement(&p.placement, point, normal);
        let def = memo[&p.def];
        if copy {
            let mut a = p.attrs.clone();
            if let Err(e) = attrs::apply(&mut a, &updates, false) {
                return restore(doc, e);
            }
            let nid = doc.alloc_id();
            doc.parts.push(Part {
                id: nid,
                def,
                placement,
                attrs: a,
            });
            out_ids.push(nid);
        } else {
            let q = doc.part_mut(*id).expect("selected");
            q.def = def;
            q.placement = placement;
            if let Err(e) = attrs::apply(&mut q.attrs, &updates, false) {
                return restore(doc, e);
            }
            out_ids.push(*id);
        }
    }
    let gone = if copy { Vec::new() } else { gc_defs(doc) };
    // As in create and copy3d: parts whose geometry cannot be reported are
    // not kept.
    let geometry = doc.solids.is_some();
    let out = match out_ids
        .iter()
        .map(|id| part_json(doc, doc.part(*id).expect("made"), geometry))
        .collect::<Result<Vec<_>, _>>()
    {
        Ok(out) => out,
        Err(e) => return restore(doc, e),
    };
    let single = matches!(o.get("target"), Some(Value::String(_)))
        || matches!(o.get("target"), Some(Value::Object(m)) if m.contains_key("id"));
    if copy {
        return Ok(if single {
            out.into_iter().next().unwrap_or(Value::Null).to_string()
        } else {
            Value::Array(out).to_string()
        });
    }
    Ok(json!({
        "mirrored": keys(&out_ids),
        "defsMade": made.iter().map(|d| def_key(*d)).collect::<Vec<_>>(),
        "defsRemoved": gone.iter().map(|d| def_key(*d)).collect::<Vec<_>>(),
    })
    .to_string())
}

fn remove3d(doc: &mut Document, args: &str) -> R {
    let o = parse(args)?;
    let ids = target(doc, &o).map_err(|e| format!("remove3d: {e}"))?;
    doc.parts.retain(|p| !ids.contains(&p.id));
    let gone = gc_defs(doc);
    Ok(json!({
        "removed": keys(&ids),
        "defsRemoved": gone.iter().map(|d| def_key(*d)).collect::<Vec<_>>(),
    })
    .to_string())
}

// ── Attributes ─────────────────────────────────────────────────────────

fn update_part_attrs(
    doc: &mut Document,
    target_v: &Value,
    updates: &attrs::Updates,
    replace: bool,
) -> R {
    let ids = select_parts(doc, &PartSel::parse(target_v)?)?;
    let mut next = Vec::with_capacity(ids.len());
    for id in &ids {
        let mut a = doc.part(*id).expect("selected").attrs.clone();
        attrs::apply(&mut a, updates, replace).map_err(|m| format!("{}: {m}", part_key(*id)))?;
        next.push(a);
    }
    for (id, a) in ids.iter().zip(next) {
        doc.part_mut(*id).expect("selected").attrs = a;
    }
    Ok(json!({ "updated": keys(&ids) }).to_string())
}

fn set_part_attrs(doc: &mut Document, args: &str) -> R {
    let o = parse(args)?;
    let t = o.get("target").ok_or("setPartAttrs: target is required")?;
    let updates = attrs::parse_updates(o.get("attrs").ok_or("setPartAttrs: attrs is required")?)?;
    let replace = o.get("replace").and_then(Value::as_bool).unwrap_or(false);
    update_part_attrs(doc, t, &updates, replace)
}

fn remove_part_attrs(doc: &mut Document, args: &str) -> R {
    let o = parse(args)?;
    let t = o
        .get("target")
        .ok_or("removePartAttrs: target is required")?;
    let keys: Vec<String> = o
        .get("keys")
        .and_then(Value::as_array)
        .ok_or("removePartAttrs: keys is an array of attribute keys")?
        .iter()
        .map(|k| {
            k.as_str()
                .map(str::to_string)
                .ok_or_else(|| "removePartAttrs: keys are strings".to_string())
        })
        .collect::<Result<_, _>>()?;
    let updates: attrs::Updates = keys.into_iter().map(|k| (k, None)).collect();
    update_part_attrs(doc, t, &updates, false)
}

// ── Query ──────────────────────────────────────────────────────────────

/// `parts({...selector, geometry?})`: every part the selector picks.
/// `geometry` (default true) adds box, volume and centroid and needs a
/// kernel; `geometry: false` lists ids, definitions, placements and
/// attributes in any build.
fn parts(doc: &mut Document, args: &str) -> R {
    let mut o = parse(args)?;
    let geometry = match o.as_object_mut().and_then(|m| m.remove("geometry")) {
        None | Some(Value::Null) => true,
        Some(Value::Bool(b)) => b,
        Some(_) => return Err("parts: geometry must be a boolean".into()),
    };
    if geometry {
        doc.solids
            .get()
            .map_err(|e| format!("parts: {e} (pass geometry: false to list parts without it)"))?;
    }
    let sel_v = match &o {
        Value::Object(m) if m.is_empty() => Value::Null,
        other => other.clone(),
    };
    let ids = select_parts(doc, &PartSel::parse(&sel_v)?).map_err(|e| format!("parts: {e}"))?;
    let out = ids
        .iter()
        .map(|id| part_json(doc, doc.part(*id).expect("selected"), geometry))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("parts: {e}"))?;
    Ok(Value::Array(out).to_string())
}

fn part(doc: &mut Document, args: &str) -> R {
    let o = parse(args)?;
    let id = part_id_of(o.get("id").ok_or("part: id is required")?)?;
    let p = doc
        .part(id)
        .ok_or_else(|| format!("part {} not found", part_key(id)))?;
    let geometry = o.get("geometry").and_then(Value::as_bool).unwrap_or(true);
    Ok(part_json(doc, p, geometry)
        .map_err(|e| format!("part: {e}"))?
        .to_string())
}

/// The whole feature tree of a definition (by `d_` id, or a part's).
fn part_def(doc: &mut Document, args: &str) -> R {
    let o = parse(args)?;
    let v = o
        .get("def")
        .or_else(|| o.get("id"))
        .ok_or("partDef: def is required")?;
    let s = v
        .as_str()
        .or_else(|| v.get("id").and_then(Value::as_str))
        .ok_or("partDef: def is a definition id (d_3) or a part (p_12)")?;
    let def_id = if s.starts_with("p_") {
        let pid = parse_part_id(s)?;
        doc.part(pid)
            .ok_or_else(|| format!("part {s} not found"))?
            .def
    } else {
        parse_def_id(s)?
    };
    let def = doc
        .part_defs
        .get(&def_id)
        .ok_or_else(|| format!("definition {} not found", def_key(def_id)))?;
    Ok(json!({
        "id": def_key(def.id),
        "name": def.name,
        "features": serde_json::to_value(&def.features).map_err(|e| e.to_string())?,
        "parts": doc.parts.iter().filter(|p| p.def == def.id).map(|p| part_key(p.id)).collect::<Vec<_>>(),
        "tools": tool_defs(def).map(def_key).collect::<Vec<_>>(),
    })
    .to_string())
}

/// What `describe()` says about parts: counts, attribute keys in use, and
/// which kernel (if any) answers geometry queries.
pub fn describe_parts(doc: &Document) -> Value {
    let mut keys: BTreeMap<&str, usize> = BTreeMap::new();
    for p in &doc.parts {
        for k in p.attrs.keys() {
            *keys.entry(k.as_str()).or_default() += 1;
        }
    }
    json!({
        "parts": doc.parts.len(),
        "partDefs": doc.part_defs.len(),
        "partAttrs": keys,
        "solidKernel": doc.solids.kernel_name(),
    })
}

/// What a DWG save leaves out: DWG holds no solids here, so every part is
/// dropped (saveJson keeps them).
pub fn dwg_part_losses(doc: &Document) -> Value {
    json!({
        "dropped": doc.parts.len(),
        "defs": doc.part_defs.len(),
        "note": if doc.parts.is_empty() { "" } else { "DWG export holds 2D only; saveJson keeps parts" },
    })
}
