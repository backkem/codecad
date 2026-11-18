//! `cad.project` and `cad.viewPoints` (ladder rung 3D3a): 2D views of the
//! document's parts as ordinary 2D entities. See docs/api-surface.md,
//! "Projected views", and docs/3d-design.md section 7.
//!
//! A view is one block definition holding the view's exact lines, arcs,
//! circles and ellipses in true size (millimetres in the view plane, the
//! view's origin at the block's (0, 0)), plus one insert that places it on
//! the sheet at `at` with the view's `scale`. Each shape carries the
//! attributes `partId` (the part it came from), `edge` (`"visible"` or
//! `"hidden"`) and `curve` (`"edge"` or `"silhouette"`), and sits on the
//! layer the script named for its class. No layer name means anything
//! here; a layer this call creates gets plain defaults.

use std::sync::Arc;

use kurbo::Point;
use serde_json::{json, Value};

use super::eval::Solid;
use super::hlr::{self, CurveKind, Options, Source, ViewFrame};
use super::kernel::{Aabb3, EvalError};
use super::oracle::{MeshOracle, OraclePart};
use super::part::{apply_vec, inverse, part_key};
use super::types::{Point3, Vec3};
use crate::attrs::{AttrValue, Attrs};
use crate::document::{BlockDef, Document};
use crate::types::{Color, EntityId, Shape};

/// The default chord of the visibility oracle's mesh (mm), and its share
/// of the parts' size when that is larger.
pub const ORACLE_CHORD: f64 = 0.01;
pub const ORACLE_CHORD_SHARE: f64 = 1e-5;

/// The named views, with the same names and directions as `cad.render`:
/// `(name, back, right, up)`, `back` pointing from the parts to the eye.
pub fn named_view(name: &str) -> Option<(Vec3, Vec3, Vec3)> {
    let s = 1.0 / 3f64.sqrt();
    let h = std::f64::consts::FRAC_1_SQRT_2;
    let iso = |bx: f64, by: f64, rx: f64, ry: f64| {
        let back = Vec3::new(bx * s, by * s, s);
        let right = Vec3::new(rx * h, ry * h, 0.0);
        (back, right, back.cross(right))
    };
    Some(match name {
        "front" => (Vec3::new(0.0, -1.0, 0.0), Vec3::X, Vec3::Z),
        "back" => (Vec3::Y, Vec3::new(-1.0, 0.0, 0.0), Vec3::Z),
        "left" => (
            Vec3::new(-1.0, 0.0, 0.0),
            Vec3::new(0.0, -1.0, 0.0),
            Vec3::Z,
        ),
        "right" => (Vec3::X, Vec3::Y, Vec3::Z),
        "top" => (Vec3::Z, Vec3::X, Vec3::Y),
        "bottom" => (
            Vec3::new(0.0, 0.0, -1.0),
            Vec3::X,
            Vec3::new(0.0, -1.0, 0.0),
        ),
        "iso" | "iso-se" | "isometric" => iso(1.0, -1.0, 1.0, 1.0),
        "iso-sw" => iso(-1.0, -1.0, 1.0, -1.0),
        "iso-ne" => iso(1.0, 1.0, -1.0, 1.0),
        "iso-nw" => iso(-1.0, 1.0, -1.0, -1.0),
        _ => return None,
    })
}

const VIEW_NAMES: &str = "front|back|left|right|top|bottom|iso|iso-se|iso-sw|iso-ne|iso-nw";

fn p3(v: &Value, what: &str) -> Result<Vec3, String> {
    let a = v
        .as_array()
        .filter(|a| a.len() == 3)
        .ok_or_else(|| format!("{what} is [x, y, z]"))?;
    let f = |i: usize| {
        a[i].as_f64()
            .filter(|x| x.is_finite())
            .ok_or_else(|| format!("{what} is [x, y, z] of finite numbers"))
    };
    Ok(Vec3::new(f(0)?, f(1)?, f(2)?))
}

fn p2(v: &Value, what: &str) -> Result<Point, String> {
    let a = v
        .as_array()
        .filter(|a| a.len() == 2)
        .ok_or_else(|| format!("{what} is [x, y]"))?;
    let f = |i: usize| {
        a[i].as_f64()
            .filter(|x| x.is_finite())
            .ok_or_else(|| format!("{what} is [x, y] of finite numbers"))
    };
    Ok(Point::new(f(0)?, f(1)?))
}

/// A view's direction: its name (or `"view"` for a direction), and the
/// unit vectors toward the eye, to the sheet's right and up.
pub struct Axes {
    pub name: String,
    pub back: Vec3,
    pub right: Vec3,
    pub up: Vec3,
}

/// `view` (a name) or `from` (a direction toward the eye) with `up`
/// (default world Z; world Y when looking along Z).
pub fn axes(o: &Value) -> Result<Axes, String> {
    let view = o.get("view").filter(|v| !v.is_null());
    let from = o.get("from").filter(|v| !v.is_null());
    match (view, from) {
        (Some(_), Some(_)) => Err("give view or from, not both".into()),
        (Some(v), None) => {
            let name = v
                .as_str()
                .ok_or_else(|| format!("view is a name ({VIEW_NAMES})"))?;
            let (back, right, up) =
                named_view(name).ok_or_else(|| format!("unknown view '{name}' ({VIEW_NAMES})"))?;
            if o.get("up").is_some_and(|u| !u.is_null()) {
                return Err("up goes with from; a named view has its own".into());
            }
            Ok(Axes {
                name: name.to_string(),
                back,
                right,
                up,
            })
        }
        (None, Some(f)) => {
            let back = p3(f, "from")?
                .normalized()
                .ok_or("from must be a non-zero direction")?;
            let up_hint = match o.get("up").filter(|u| !u.is_null()) {
                Some(u) => p3(u, "up")?,
                None if back.cross(Vec3::Z).length() <= 1e-9 => Vec3::Y,
                None => Vec3::Z,
            };
            let right = up_hint
                .cross(back)
                .normalized()
                .ok_or("up must not point along from")?;
            if up_hint.cross(back).length() <= 1e-9 * up_hint.length() {
                return Err("up must not point along from".into());
            }
            let up = back.cross(right);
            Ok(Axes {
                name: "view".into(),
                back,
                right,
                up,
            })
        }
        (None, None) => Err(format!(
            "a view needs view ({VIEW_NAMES}) or from: [x, y, z]"
        )),
    }
}

/// One selected part with what the projection needs from the kernel.
struct Sel {
    id: EntityId,
    attrs: Attrs,
    solid: Arc<dyn Solid>,
    placement: super::types::Placement,
    bbox: Aabb3,
    topology: Arc<super::kernel::BrepTopology>,
    def: super::types::PartDef,
}

fn select(doc: &Document, o: &Value, need_topology: bool) -> Result<Vec<Sel>, String> {
    let ids = crate::dispatch_3d::select_parts_value(doc, o.get("parts").unwrap_or(&Value::Null))?;
    let solids = doc.solids.get()?;
    let mut out = Vec::with_capacity(ids.len());
    for id in ids {
        let p = doc.part(id).expect("selected");
        let def = doc
            .part_defs
            .get(&p.def)
            .ok_or_else(|| format!("part {}: its definition is missing", part_key(id)))?;
        let who = format!("part {}", part_key(id));
        let err = |e: &EvalError| crate::dispatch_3d::refusal(def, &who, e);
        let solid = solids.solid(&doc.part_defs, def).map_err(|e| err(&e))?;
        let bbox = solids
            .tight_bbox(&doc.part_defs, def, &p.placement)
            .map_err(|e| err(&e))?
            .aabb;
        let topology = if need_topology {
            solids.topology(&doc.part_defs, def).map_err(|e| err(&e))?
        } else {
            Arc::new(Default::default())
        };
        out.push(Sel {
            id,
            attrs: p.attrs.clone(),
            solid,
            placement: p.placement,
            bbox,
            topology,
            def: def.clone(),
        });
    }
    Ok(out)
}

fn union_box(sel: &[Sel]) -> Option<Aabb3> {
    let first = sel.first()?.bbox;
    Some(sel.iter().fold(first, |a, s| {
        Aabb3::new(
            Point3::new(
                a.min.x.min(s.bbox.min.x),
                a.min.y.min(s.bbox.min.y),
                a.min.z.min(s.bbox.min.z),
            ),
            Point3::new(
                a.max.x.max(s.bbox.max.x),
                a.max.y.max(s.bbox.max.y),
                a.max.z.max(s.bbox.max.z),
            ),
        )
    }))
}

/// Where the view goes: `origin` (3D, default the centre of the parts'
/// box), `at` (sheet, default [0, 0]) and `scale` (default 1).
struct Placing {
    origin: Point3,
    at: Point,
    scale: f64,
}

fn placing(o: &Value, sel: &[Sel]) -> Result<Placing, String> {
    let origin = match o.get("origin").filter(|v| !v.is_null()) {
        Some(v) => {
            let p = p3(v, "origin")?;
            Point3::new(p.x, p.y, p.z)
        }
        None => {
            let b = union_box(sel).ok_or("no parts selected")?;
            Point3::new(
                0.5 * (b.min.x + b.max.x),
                0.5 * (b.min.y + b.max.y),
                0.5 * (b.min.z + b.max.z),
            )
        }
    };
    let at = match o.get("at").filter(|v| !v.is_null()) {
        Some(v) => p2(v, "at")?,
        None => Point::ORIGIN,
    };
    let scale = match o.get("scale").filter(|v| !v.is_null()) {
        Some(v) => v
            .as_f64()
            .filter(|s| s.is_finite() && *s > 0.0)
            .ok_or("scale must be a positive number")?,
        None => 1.0,
    };
    Ok(Placing { origin, at, scale })
}

fn bool_opt(o: &Value, key: &str, default: bool) -> Result<bool, String> {
    match o.get(key) {
        None | Some(Value::Null) => Ok(default),
        Some(Value::Bool(b)) => Ok(*b),
        Some(_) => Err(format!("{key} must be true or false")),
    }
}

/// The layers each class of curve goes on.
struct Layers {
    visible: String,
    hidden: String,
    silhouette: String,
    hidden_silhouette: String,
}

fn layers(o: &Value) -> Result<Layers, String> {
    let l = o.get("layers").filter(|v| !v.is_null());
    if let Some(l) = l {
        let obj = l
            .as_object()
            .ok_or("layers is {visible?, hidden?, silhouette?, hiddenSilhouette?}")?;
        for k in obj.keys() {
            if !["visible", "hidden", "silhouette", "hiddenSilhouette"].contains(&k.as_str()) {
                return Err(format!(
                    "unknown layers key '{k}' (visible, hidden, silhouette, hiddenSilhouette)"
                ));
            }
        }
    }
    let get = |k: &str| -> Result<Option<String>, String> {
        match l.and_then(|l| l.get(k)) {
            None | Some(Value::Null) => Ok(None),
            Some(Value::String(s)) if !s.is_empty() => Ok(Some(s.clone())),
            Some(_) => Err(format!("layers.{k} is a layer name")),
        }
    };
    let visible = get("visible")?.unwrap_or_else(|| "visible".into());
    let hidden = get("hidden")?.unwrap_or_else(|| "hidden".into());
    let silhouette = get("silhouette")?.unwrap_or_else(|| visible.clone());
    let hidden_silhouette = get("hiddenSilhouette")?.unwrap_or_else(|| hidden.clone());
    Ok(Layers {
        visible,
        hidden,
        silhouette,
        hidden_silhouette,
    })
}

fn ensure(doc: &mut Document, name: &str, linetype: &str, lineweight: f64) {
    if !doc.layers.iter().any(|l| l.name == name) {
        doc.add_layer_full(name, Color::WHITE, linetype, lineweight);
    }
}

fn now_ms(t: Option<std::time::Instant>) -> f64 {
    t.map_or(0.0, |t| t.elapsed().as_secs_f64() * 1e3)
}

#[cfg(not(target_arch = "wasm32"))]
fn clock() -> Option<std::time::Instant> {
    Some(std::time::Instant::now())
}

#[cfg(target_arch = "wasm32")]
fn clock() -> Option<std::time::Instant> {
    None
}

/// What one projection made, before it is written to the document.
pub struct Projected {
    pub axes: Axes,
    pub frame: ViewFrame,
    pub drawn: Vec<hlr::Drawn>,
    pub stats: hlr::Stats,
    pub oracle: MeshOracle,
    pub chord: f64,
    pub owners: Vec<(EntityId, Attrs)>,
    pub open: Vec<Value>,
    pub times: [f64; 3],
    pub at: Point,
    pub scale: f64,
    /// Every classified piece, when the caller asked for a trace.
    pub traced: Vec<hlr::Traced>,
}

/// Runs the projection of the selected parts (no document change).
pub fn compute(doc: &Document, o: &Value) -> Result<Projected, String> {
    let t0 = clock();
    let axes = axes(o)?;
    let sel = select(doc, o, true)?;
    if sel.is_empty() {
        return Err("no parts selected".into());
    }
    let pl = placing(o, &sel)?;
    let frame = ViewFrame {
        origin: pl.origin,
        right: axes.right,
        up: axes.up,
        back: axes.back,
    };
    let hidden = bool_opt(o, "hidden", true)?;
    let smooth = bool_opt(o, "smooth", false)?;
    let silhouettes = bool_opt(o, "silhouettes", true)?;
    let exact = bool_opt(o, "exact", true)?;
    let ub = union_box(&sel).expect("parts");
    let size = (ub.max - ub.min).length();
    let chord = match o.get("chord").filter(|v| !v.is_null()) {
        Some(v) => v
            .as_f64()
            .filter(|c| c.is_finite() && *c > 0.0)
            .ok_or("chord must be a positive number")?,
        None => ORACLE_CHORD.max(ORACLE_CHORD_SHARE * size),
    };
    let solids = doc.solids.get()?;
    let mut sources: Vec<Source> = Vec::new();
    let mut open: Vec<Value> = Vec::new();
    let mut parts: Vec<OraclePart> = Vec::with_capacity(sel.len());
    for (owner, s) in sel.iter().enumerate() {
        let local = apply_vec(&inverse(&s.placement), axes.back);
        for e in &s.topology.edges {
            if e.seam || (e.smooth && !smooth && !on_outline(&s.topology, e, local)) {
                continue;
            }
            sources.push(Source {
                curve: e.curve.placed(&s.placement),
                range: e.range,
                owner,
                kind: CurveKind::Edge,
            });
        }
        if silhouettes {
            match s.solid.silhouettes(local) {
                Ok(sil) => {
                    for c in sil.curves {
                        sources.push(Source {
                            curve: c.curve.placed(&s.placement),
                            range: c.range,
                            owner,
                            kind: CurveKind::Silhouette,
                        });
                    }
                    for (face, why) in sil.skipped {
                        open.push(json!({"part": part_key(s.id), "face": face, "reason": why}));
                    }
                }
                Err(e) => open.push(json!({"part": part_key(s.id), "reason": e.to_string()})),
            }
        }
        let mesh = match solids.trimesh(&doc.part_defs, &s.def, chord) {
            Ok(m) => Some(m),
            Err(e) => {
                open.push(
                    json!({"part": part_key(s.id), "reason": format!("no oracle mesh: {e}")}),
                );
                None
            }
        };
        parts.push(OraclePart {
            solid: s.solid.clone(),
            placement: s.placement,
            bbox: s.bbox,
            mesh,
        });
    }
    let t1 = clock();
    hlr::debug(&format!(
        "gathered {} curves of {} parts in {:.0} ms",
        sources.len(),
        sel.len(),
        now_ms(t0)
    ));
    let mut oracle = MeshOracle::new(axes.back, parts, chord, exact);
    let t2 = clock();
    let trace = bool_opt(o, "trace", false)?;
    let (drawn, stats, traced) = hlr::run(
        &frame,
        sources,
        &mut oracle,
        &Options {
            hidden,
            trace,
            ..Options::default()
        },
    );
    let times = [
        t0.map_or(0.0, |t| (t1.unwrap() - t).as_secs_f64() * 1e3),
        t1.map_or(0.0, |t| (t2.unwrap() - t).as_secs_f64() * 1e3),
        now_ms(t2),
    ];
    Ok(Projected {
        axes,
        frame,
        drawn,
        stats,
        oracle,
        chord,
        owners: sel.iter().map(|s| (s.id, s.attrs.clone())).collect(),
        open,
        times,
        at: pl.at,
        scale: pl.scale,
        traced,
    })
}

/// Whether a smooth (tangent) edge lies on the outline of this view: the
/// faces' common normal is across the view at its start, middle and end
/// (part frame), so it is drawn as a silhouette would be.
fn on_outline(t: &super::kernel::BrepTopology, e: &super::kernel::BrepEdge, view: Vec3) -> bool {
    let Some(face) = e.faces.first().and_then(|&f| t.faces.get(f)) else {
        return false;
    };
    let [t0, t1] = e.range;
    [t0, 0.5 * (t0 + t1), t1].iter().all(|&u| {
        face.surface
            .normal_at(e.curve.point(u))
            .is_some_and(|n| n.dot(view).abs() <= 1e-9)
    })
}

fn shape_kind(s: &Shape) -> &'static str {
    match s {
        Shape::Line(_) => "line",
        Shape::Arc { .. } => "arc",
        Shape::Circle(_) => "circle",
        Shape::Ellipse { .. } => "ellipse",
        Shape::Spline { .. } => "spline",
        _ => "other",
    }
}

/// `cad.project(opts)`: see docs/api-surface.md, "Projected views".
pub fn project(doc: &mut Document, o: &Value) -> Result<Value, String> {
    let pr = compute(doc, o)?;
    let lay = layers(o)?;
    let part_attrs = bool_opt(o, "partAttrs", false)?;
    let place = bool_opt(o, "place", true)?;
    let pl = Placing {
        origin: pr.frame.origin,
        at: pr.at,
        scale: pr.scale,
    };
    let name = match o.get("name").filter(|v| !v.is_null()) {
        Some(Value::String(s)) if !s.is_empty() => s.clone(),
        Some(_) => return Err("name is a block name".into()),
        None => format!("view-{}", pr.axes.name),
    };
    ensure(doc, &lay.visible, "Continuous", 0.5);
    ensure(doc, &lay.silhouette, "Continuous", 0.5);
    ensure(doc, &lay.hidden, "Hidden", 0.25);
    ensure(doc, &lay.hidden_silhouette, "Hidden", 0.25);
    let mut shapes = Vec::with_capacity(pr.drawn.len());
    let mut shape_attrs = Vec::with_capacity(pr.drawn.len());
    let mut counts = std::collections::BTreeMap::<&str, usize>::new();
    let (mut lo, mut hi) = (
        Point::new(f64::INFINITY, f64::INFINITY),
        Point::new(f64::NEG_INFINITY, f64::NEG_INFINITY),
    );
    for d in &pr.drawn {
        let layer = match (d.hidden, d.kind) {
            (false, CurveKind::Edge) => &lay.visible,
            (false, CurveKind::Silhouette) => &lay.silhouette,
            (true, CurveKind::Edge) => &lay.hidden,
            (true, CurveKind::Silhouette) => &lay.hidden_silhouette,
        };
        let (id, pattrs) = &pr.owners[d.owner];
        let mut a: Attrs = if part_attrs {
            pattrs.clone()
        } else {
            Attrs::new()
        };
        a.insert("partId".into(), AttrValue::Str(part_key(*id)));
        a.insert(
            "edge".into(),
            AttrValue::Str(if d.hidden { "hidden" } else { "visible" }.into()),
        );
        a.insert(
            "curve".into(),
            AttrValue::Str(
                match d.kind {
                    CurveKind::Edge => "edge",
                    CurveKind::Silhouette => "silhouette",
                }
                .into(),
            ),
        );
        if a.len() > crate::attrs::MAX_KEYS {
            return Err(format!(
                "part {} has too many attributes to copy onto its view (partAttrs)",
                part_key(*id)
            ));
        }
        *counts.entry(shape_kind(&d.shape)).or_default() += 1;
        *counts
            .entry(if d.hidden { "hidden" } else { "visible" })
            .or_default() += 1;
        let (x0, y0, x1, y1) = d.shape.bbox();
        lo = Point::new(lo.x.min(x0), lo.y.min(y0));
        hi = Point::new(hi.x.max(x1), hi.y.max(y1));
        shapes.push((d.shape.clone(), layer.clone(), None));
        shape_attrs.push(a);
    }
    let n = shapes.len();
    let mut def = BlockDef::new(name.clone(), shapes, Point::ORIGIN, lay.visible.clone());
    def.shape_attrs = shape_attrs;
    doc.define_block(def);
    let existing = doc.entities.iter().position(
        |e| matches!(&e.shape, Shape::BlockInsert { block_name, .. } if *block_name == name),
    );
    let insert = match existing {
        Some(i) => {
            if let Shape::BlockInsert {
                position,
                x_scale,
                y_scale,
                ..
            } = &mut doc.entities[i].shape
            {
                *position = pl.at;
                *x_scale = pl.scale;
                *y_scale = pl.scale;
            }
            Some(doc.entities[i].id)
        }
        None if place => doc
            .place_block_scaled(&name, pl.at, 0.0, pl.scale, pl.scale, Some(&lay.visible))
            .first()
            .copied(),
        None => None,
    };
    let sheet = |p: Point| [pl.at.x + pl.scale * p.x, pl.at.y + pl.scale * p.y];
    let f = &pr.frame;
    let v3 = |v: Vec3| json!([v.x, v.y, v.z]);
    Ok(json!({
        "block": name,
        "insert": insert.map(|id| format!("e_{}", id.0)),
        "view": pr.axes.name,
        "from": v3(f.back),
        "right": v3(f.right),
        "up": v3(f.up),
        "origin": [f.origin.x, f.origin.y, f.origin.z],
        "at": [pl.at.x, pl.at.y],
        "scale": pl.scale,
        "parts": pr.owners.len(),
        "entities": n,
        "counts": counts,
        "bounds": if n > 0 { json!({"min": sheet(lo), "max": sheet(hi)}) } else { Value::Null },
        "open": pr.open,
        "stats": {
            "curves": pr.stats.curves,
            "splits": pr.stats.splits,
            "pieces": pr.stats.pieces,
            "queries": pr.stats.queries,
            "bisected": pr.stats.bisected,
            "fitted": pr.stats.fitted,
        },
        "oracle": {
            "chord": pr.chord,
            "triangles": pr.oracle.stats.triangles,
            "mesh": pr.oracle.stats.mesh,
            "exact": pr.oracle.stats.exact,
            "inFace": pr.oracle.stats.in_face,
            "guessed": pr.oracle.stats.guessed,
            "beside": pr.oracle.stats.beside,
        },
        "ms": {
            "gather": round1(pr.times[0]),
            "mesh": round1(pr.times[1]),
            "hlr": round1(pr.times[2]),
            "split": round1(pr.stats.ms[0]),
            "classify": round1(pr.stats.ms[1]),
            "merge": round1(pr.stats.ms[2]),
        },
    }))
}

fn round1(v: f64) -> f64 {
    (v * 10.0).round() / 10.0
}

/// `cad.viewPoints(opts)`: where 3D points land on the sheet in the view
/// that `cad.project` with the same view and placing options draws.
pub fn view_points(doc: &Document, o: &Value) -> Result<Value, String> {
    let axes = axes(o)?;
    let needs_parts = o.get("origin").is_none_or(Value::is_null);
    let sel = if needs_parts {
        select(doc, o, false)?
    } else {
        Vec::new()
    };
    let pl = placing(o, &sel)?;
    let frame = ViewFrame {
        origin: pl.origin,
        right: axes.right,
        up: axes.up,
        back: axes.back,
    };
    let pts = o
        .get("points")
        .and_then(Value::as_array)
        .ok_or("points is a list of [x, y, z]")?;
    let out: Vec<Value> = pts
        .iter()
        .map(|p| {
            let v = p3(p, "a point")?;
            let q = frame.point(Point3::new(v.x, v.y, v.z));
            Ok(json!([pl.at.x + pl.scale * q.x, pl.at.y + pl.scale * q.y]))
        })
        .collect::<Result<_, String>>()?;
    Ok(Value::Array(out))
}
