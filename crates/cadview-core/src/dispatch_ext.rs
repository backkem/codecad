//! cad_call methods for construction, annotation, fills, splines and
//! region booleans. `dispatch::cad_call` falls through to
//! [`cad_call_ext`] for any method it doesn't handle itself.

use crate::annotate::{self, DimDir, DimStyle, Emitter, HAlign, VAlign};
use crate::construct::{self, Curve, ExtendEnd};
use crate::dispatch::{default_layer, parse_args, parse_entity_id, parse_target, ColorArg};
use crate::document::*;
use crate::hatch;
use crate::region;
use crate::spline;
use crate::types::*;
use kurbo::{Affine, Circle, Line, Point};
use serde::Deserialize;
use serde_json::{json, Value};
use std::f64::consts::{PI, TAU};

type R = Result<String, String>;

// ── Shared argument helpers ────────────────────────────────────────────

/// Style options accepted by every creation call.
#[derive(Deserialize, Default, Clone)]
pub(crate) struct Style {
    #[serde(default)]
    layer: Option<String>,
    #[serde(default)]
    color: Option<ColorArg>,
    #[serde(default)]
    linetype: Option<String>,
    #[serde(default)]
    lineweight: Option<f64>,
    #[serde(default)]
    transparency: Option<u8>,
    #[serde(default, deserialize_with = "crate::attrs::deserialize")]
    attrs: crate::attrs::Attrs,
}

impl Style {
    fn layer(&self) -> String {
        self.layer.clone().unwrap_or_else(default_layer)
    }
    fn color(&self) -> Option<Color> {
        self.color.clone().map(ColorArg::resolve)
    }
}

fn add_styled(doc: &mut Document, shape: Shape, style: &Style) -> EntityId {
    let layer = style.layer();
    doc.ensure_layer(&layer);
    let id = doc.alloc_id();
    doc.entities.push(DrawEntity {
        id,
        layer,
        color: style.color(),
        linetype: style.linetype.clone(),
        lineweight: style.lineweight,
        transparency: style.transparency.unwrap_or(0),
        shape,
        attrs: style.attrs.clone(),
    });
    id
}

/// A new entity that joins two others (a fillet arc, a chamfer line) gets
/// the attributes both of them have with equal values.
fn joined_attrs(doc: &mut Document, id: EntityId, a: EntityId, b: EntityId) {
    let (Some(ea), Some(eb)) = (doc.entity(a), doc.entity(b)) else {
        return;
    };
    let shared = crate::attrs::common(&ea.attrs, &eb.attrs);
    if let Some(e) = doc.entity_mut(id) {
        e.attrs = shared;
    }
}

fn ent_json(doc: &Document, id: EntityId) -> Value {
    doc.entity(id)
        .map(|e| serde_json::to_value(e.to_json(doc)).unwrap_or(Value::Null))
        .unwrap_or(Value::Null)
}

fn ok(v: Value) -> R {
    Ok(v.to_string())
}

fn pt(v: [f64; 2]) -> Point {
    Point::new(v[0], v[1])
}

fn point_val(v: &Value) -> Result<Point, String> {
    let a = v.as_array().ok_or("expected a point [x, y]")?;
    if a.len() < 2 {
        return Err("expected a point [x, y]".into());
    }
    let x = a[0].as_f64().ok_or("point x must be a number")?;
    let y = a[1].as_f64().ok_or("point y must be a number")?;
    Ok(Point::new(x, y))
}

/// Entity reference: "e_12" or an entity object with `id`.
fn entity_ref(v: &Value) -> Option<Result<EntityId, String>> {
    match v {
        Value::String(s) => Some(parse_entity_id(s)),
        Value::Object(o) => o.get("id").and_then(|i| i.as_str()).map(parse_entity_id),
        _ => None,
    }
}

fn entity_id_val(v: &Value) -> Result<EntityId, String> {
    entity_ref(v).unwrap_or_else(|| Err("expected an entity id or entity object".into()))
}

/// A line given as [[x,y],[x,y]], {start, end}, or a line entity reference.
fn line_val(doc: &Document, v: &Value) -> Result<(Point, Point), String> {
    if let Value::Array(a) = v {
        if a.len() == 2 {
            return Ok((point_val(&a[0])?, point_val(&a[1])?));
        }
        return Err("a line is [[x1,y1],[x2,y2]]".into());
    }
    if let Value::Object(o) = v {
        if let (Some(s), Some(e), None) = (o.get("start"), o.get("end"), o.get("id")) {
            return Ok((point_val(s)?, point_val(e)?));
        }
    }
    let id = entity_id_val(v)?;
    match doc.entity(id).map(|e| &e.shape) {
        Some(Shape::Line(l)) => Ok((l.p0, l.p1)),
        Some(_) => Err(format!("entity e_{} is not a line", id.0)),
        None => Err(format!("entity e_{} not found", id.0)),
    }
}

fn shape_of(doc: &Document, id: EntityId) -> Result<Shape, String> {
    doc.entity(id)
        .map(|e| e.shape.clone())
        .ok_or_else(|| format!("entity e_{} not found", id.0))
}

/// Sample an arc into points (for polygon-only consumers: hatch clipping,
/// region booleans). Chord error ~ r/2000.
fn sample_arc(c: Point, r: f64, start: f64, sweep: f64, out: &mut Vec<Point>, include_start: bool) {
    let step = 2.0 * (1.0 - 5e-4f64).acos();
    let n = ((sweep.abs() / step).ceil() as usize).clamp(4, 2048);
    let first = if include_start { 0 } else { 1 };
    for i in first..=n {
        let a = start + sweep * i as f64 / n as f64;
        out.push(Point::new(c.x + r * a.cos(), c.y + r * a.sin()));
    }
}

/// A closed ring as abstract fill edges plus a sampled polygon.
struct Ring {
    edges: Vec<FillEdge>,
    poly: Vec<Point>,
}

fn ring_from_points(pts: Vec<Point>) -> Result<Ring, String> {
    if pts.len() < 3 {
        return Err("a boundary ring needs at least 3 points".into());
    }
    Ok(Ring {
        edges: pts.iter().map(|p| FillEdge::LineTo(*p)).collect(),
        poly: pts,
    })
}

fn ring_circle(c: Point, r: f64) -> Result<Ring, String> {
    if r <= 0.0 {
        return Err("circle ring radius must be > 0".into());
    }
    let mut poly = Vec::new();
    sample_arc(c, r, 0.0, TAU, &mut poly, true);
    poly.pop(); // last == first
    Ok(Ring {
        edges: vec![FillEdge::ArcTo {
            center: c,
            radius: r,
            start_angle: 0.0,
            end_angle: TAU,
        }],
        poly,
    })
}

/// Parse a ring: array of points, {center, radius}, or an entity reference
/// (circle, closed polyline, polyline with bulges).
fn ring_val(doc: &Document, v: &Value) -> Result<Ring, String> {
    match v {
        Value::Array(a) => {
            let pts = a.iter().map(point_val).collect::<Result<Vec<_>, _>>()?;
            ring_from_points(pts)
        }
        Value::Object(o)
            if o.contains_key("center") && o.contains_key("radius") && !o.contains_key("id") =>
        {
            let c = point_val(&o["center"])?;
            let r = o["radius"].as_f64().ok_or("radius must be a number")?;
            ring_circle(c, r)
        }
        _ => {
            let id = entity_id_val(v)?;
            match shape_of(doc, id)? {
                Shape::Circle(c) => ring_circle(c.center, c.radius),
                Shape::Polyline { points, .. } => ring_from_points(points),
                Shape::LwPolyline { vertices, .. } => lw_ring(vertices),
                _ => Err(format!(
                    "entity e_{} can't be used as a boundary (use a circle or polyline)",
                    id.0
                )),
            }
        }
    }
}

/// Fill ring from bulge vertices. FillEdge arcs are counter-clockwise, so
/// a ring whose arcs are all clockwise is reversed first.
fn lw_ring(mut v: Vec<LwVertex>) -> Result<Ring, String> {
    let n = v.len();
    if n < 2 {
        return Err("polyline boundary needs at least 2 vertices".into());
    }
    let has_ccw = v.iter().any(|x| x.bulge > 1e-12);
    let has_cw = v.iter().any(|x| x.bulge < -1e-12);
    if has_ccw && has_cw {
        return Err(
            "polyline boundaries with both CW and CCW arc segments are not supported".into(),
        );
    }
    if has_cw {
        // Reverse: segment i (v[i]->v[i+1], bulge b_i) becomes v[i+1]->v[i] with -b_i.
        let pts: Vec<Point> = v.iter().rev().map(|x| x.point).collect();
        let mut bulges: Vec<f64> = Vec::with_capacity(n);
        for i in 0..n {
            // new segment i goes from pts[i] = v[n-1-i] to pts[i+1] = v[n-2-i]
            let orig = (2 * n - 2 - i) % n; // segment v[n-2-i] -> v[n-1-i]
            bulges.push(-v[orig].bulge);
        }
        v = pts
            .into_iter()
            .zip(bulges)
            .map(|(point, bulge)| LwVertex { point, bulge })
            .collect();
    }
    let mut edges = Vec::new();
    let mut poly = Vec::new();
    for i in 0..n {
        let p0 = v[i].point;
        let p1 = v[(i + 1) % n].point;
        match construct::bulge_arc(p0, p1, v[i].bulge) {
            Some((c, r, s, e)) => {
                let sweep = (e - s).rem_euclid(TAU);
                edges.push(FillEdge::ArcTo {
                    center: c,
                    radius: r,
                    start_angle: s,
                    end_angle: s + sweep,
                });
                let first = poly.is_empty();
                sample_arc(c, r, s, sweep, &mut poly, first);
            }
            None => {
                edges.push(FillEdge::LineTo(p1));
                if poly.is_empty() {
                    poly.push(p0);
                }
                poly.push(p1);
            }
        }
    }
    if poly.len() > 1 && (poly[0] - *poly.last().expect("non-empty")).hypot() < 1e-9 {
        poly.pop();
    }
    Ok(Ring { edges, poly })
}

/// List of rings for region booleans: a single ring (array of points), an
/// array of rings / entity references, or a single entity reference.
fn rings_val(doc: &Document, v: &Value) -> Result<Vec<Vec<Point>>, String> {
    if let Value::Array(a) = v {
        let single = a
            .first()
            .and_then(|f| f.as_array())
            .is_some_and(|f| f.first().is_some_and(|x| x.is_number()));
        if single {
            return Ok(vec![ring_val(doc, v)?.poly]);
        }
        return a.iter().map(|r| ring_val(doc, r).map(|x| x.poly)).collect();
    }
    Ok(vec![ring_val(doc, v)?.poly])
}

fn deg_opt(v: Option<f64>) -> Option<f64> {
    v.map(|d| d.to_radians())
}

fn d_text_height() -> f64 {
    5.0
}
fn d_offset() -> f64 {
    10.0
}
fn d_hatch_angle() -> f64 {
    45.0
}
fn d_hatch_spacing() -> f64 {
    5.0
}
fn d_three() -> usize {
    3
}
fn d_360() -> f64 {
    360.0
}
fn d_true() -> bool {
    true
}

// ── Dispatcher ─────────────────────────────────────────────────────────

/// Extension methods. Returns None for unknown methods.
pub fn cad_call_ext(doc: &mut Document, method: &str, args: &str) -> Option<R> {
    let r = match method {
        "lineLineIntersection" => line_line_intersection(doc, args),
        "fillet" => fillet(doc, args),
        "chamfer" => chamfer(doc, args),
        "trimTo" => trim_to(doc, args),
        "extendTo" => extend_to(doc, args),
        "tangentLines" => tangent_lines(args),
        "tangentFromPoint" => tangent_from_point(args),
        "array" => array(doc, args),
        "clear" => clear(doc, args),
        "addText" => add_text(doc, args),
        "measure" => measure(doc, args),
        "dimRadius" => dim_circle(doc, args, false),
        "dimDiameter" => dim_circle(doc, args, true),
        "dimAngle" => dim_angle(doc, args),
        "addHatch" => add_hatch(doc, args),
        "addFill" => add_fill(doc, args),
        "addSpline" => add_spline(doc, args),
        "addPolyline" => add_polyline(doc, args),
        "regionClip" => region_clip(doc, args),
        "defineBlock" => define_block(doc, args),
        "place" => place(doc, args),
        "setAttrs" => set_attrs(doc, args),
        "removeAttrs" => remove_attrs(doc, args),
        "attrs" => get_attrs(doc, args),
        "setBlockAttrs" => set_block_attrs(doc, args),
        "blockAttrs" => block_attrs(doc, args),
        "methods" => methods(),
        _ => {
            return crate::dispatch_3d::dispatch(doc, method, args)
                .or_else(|| crate::setquery::dispatch(doc, method, args))
        }
    };
    Some(r)
}

// ── Attributes ─────────────────────────────────────────────────────────

/// Apply one update to every target, all or nothing.
fn update_attrs(
    doc: &mut Document,
    target: &Value,
    updates: &crate::attrs::Updates,
    replace: bool,
) -> R {
    let ids = parse_target(doc, target)?;
    let mut next = Vec::with_capacity(ids.len());
    for id in &ids {
        let e = doc
            .entity(*id)
            .ok_or_else(|| format!("entity e_{} not found", id.0))?;
        let mut a = e.attrs.clone();
        crate::attrs::apply(&mut a, updates, replace).map_err(|m| format!("e_{}: {m}", id.0))?;
        next.push(a);
    }
    let mut updated = Vec::with_capacity(ids.len());
    for (id, a) in ids.iter().zip(next) {
        if let Some(e) = doc.entity_mut(*id) {
            e.attrs = a;
            updated.push(format!("e_{}", id.0));
        }
    }
    ok(json!({ "updated": updated }))
}

fn set_attrs(doc: &mut Document, args: &str) -> R {
    #[derive(Deserialize)]
    struct A {
        target: Value,
        attrs: Value,
        #[serde(default)]
        replace: bool,
    }
    let a: A = parse_args(args)?;
    let updates = crate::attrs::parse_updates(&a.attrs)?;
    update_attrs(doc, &a.target, &updates, a.replace)
}

fn remove_attrs(doc: &mut Document, args: &str) -> R {
    #[derive(Deserialize)]
    struct A {
        target: Value,
        keys: Vec<String>,
    }
    let a: A = parse_args(args)?;
    let updates: crate::attrs::Updates = a.keys.into_iter().map(|k| (k, None)).collect();
    update_attrs(doc, &a.target, &updates, false)
}

fn get_attrs(doc: &mut Document, args: &str) -> R {
    #[derive(Deserialize)]
    struct A {
        id: Value,
    }
    let a: A = parse_args(args)?;
    let id = entity_id_val(&a.id)?;
    let e = doc
        .entity(id)
        .ok_or_else(|| format!("entity e_{} not found", id.0))?;
    ok(crate::attrs::to_json(&e.attrs))
}

fn set_block_attrs(doc: &mut Document, args: &str) -> R {
    #[derive(Deserialize)]
    struct A {
        name: String,
        attrs: Value,
        #[serde(default)]
        replace: bool,
        /// Index of one shape of the definition; absent: the definition.
        #[serde(default)]
        shape: Option<usize>,
    }
    let a: A = parse_args(args)?;
    let updates = crate::attrs::parse_updates(&a.attrs)?;
    let def = doc
        .blocks
        .get_mut(&a.name)
        .ok_or_else(|| format!("block '{}' not defined", a.name))?;
    let target = match a.shape {
        None => &mut def.attrs,
        Some(i) => {
            if i >= def.shapes.len() {
                return Err(format!(
                    "block '{}' has {} shapes (no shape {i})",
                    a.name,
                    def.shapes.len()
                ));
            }
            if def.shape_attrs.len() <= i {
                def.shape_attrs.resize(i + 1, crate::attrs::Attrs::new());
            }
            &mut def.shape_attrs[i]
        }
    };
    crate::attrs::apply(target, &updates, a.replace)?;
    let out = crate::attrs::to_json(target);
    ok(out)
}

fn block_attrs(doc: &mut Document, args: &str) -> R {
    #[derive(Deserialize)]
    struct A {
        name: String,
    }
    let a: A = parse_args(args)?;
    let def = doc
        .blocks
        .get(&a.name)
        .ok_or_else(|| format!("block '{}' not defined", a.name))?;
    let shapes: Vec<Value> = (0..def.shapes.len())
        .map(|i| crate::attrs::to_json(def.shape_attrs(i)))
        .collect();
    ok(json!({ "attrs": crate::attrs::to_json(&def.attrs), "shapes": shapes }))
}

// ── Construction ───────────────────────────────────────────────────────

fn line_line_intersection(doc: &mut Document, args: &str) -> R {
    #[derive(Deserialize)]
    struct A {
        l1: Value,
        l2: Value,
        #[serde(default)]
        segments: bool,
    }
    let a: A = parse_args(args)?;
    let (p0, p1) = line_val(doc, &a.l1)?;
    let (q0, q1) = line_val(doc, &a.l2)?;
    ok(
        match construct::line_line_intersection(p0, p1, q0, q1, a.segments) {
            Some(p) => json!([p.x, p.y]),
            None => Value::Null,
        },
    )
}

/// Picks: `pick: [[x,y],[x,y]]` or `pickA` / `pickB`.
fn picks(
    pick: &Option<Value>,
    pa: Option<[f64; 2]>,
    pb: Option<[f64; 2]>,
) -> Result<(Option<Point>, Option<Point>), String> {
    let mut a = pa.map(pt);
    let mut b = pb.map(pt);
    if let Some(Value::Array(arr)) = pick {
        if let Some(v) = arr.first() {
            if !v.is_null() {
                a = Some(point_val(v)?);
            }
        }
        if let Some(v) = arr.get(1) {
            if !v.is_null() {
                b = Some(point_val(v)?);
            }
        }
    }
    Ok((a, b))
}

fn fillet(doc: &mut Document, args: &str) -> R {
    #[derive(Deserialize)]
    struct A {
        #[serde(alias = "id1")]
        a: Value,
        #[serde(alias = "id2")]
        b: Value,
        radius: f64,
        #[serde(default)]
        pick: Option<Value>,
        #[serde(default, rename = "pickA")]
        pick_a: Option<[f64; 2]>,
        #[serde(default, rename = "pickB")]
        pick_b: Option<[f64; 2]>,
        #[serde(default)]
        layer: Option<String>,
    }
    let a: A = parse_args(args)?;
    let ia = entity_id_val(&a.a)?;
    let ib = entity_id_val(&a.b)?;
    if ia == ib {
        return Err("fillet needs two different entities".into());
    }
    let (pa, pb) = picks(&a.pick, a.pick_a, a.pick_b)?;
    let sa = shape_of(doc, ia)?;
    let sb = shape_of(doc, ib)?;
    let res = construct::fillet_shapes(&sa, &sb, a.radius, pa, pb)?;
    construct::set_shape(doc, ia, res.a);
    construct::set_shape(doc, ib, res.b);
    let arc = match res.arc {
        Some(shape) => {
            let style = doc.entity(ia).expect("checked above").clone();
            let id = construct::add_like(doc, &style, shape, a.layer.as_deref());
            joined_attrs(doc, id, ia, ib);
            ent_json(doc, id)
        }
        None => Value::Null,
    };
    ok(json!({ "arc": arc, "a": ent_json(doc, ia), "b": ent_json(doc, ib) }))
}

fn chamfer(doc: &mut Document, args: &str) -> R {
    #[derive(Deserialize)]
    struct A {
        a: Value,
        b: Value,
        d1: f64,
        #[serde(default)]
        d2: Option<f64>,
        #[serde(default)]
        pick: Option<Value>,
        #[serde(default, rename = "pickA")]
        pick_a: Option<[f64; 2]>,
        #[serde(default, rename = "pickB")]
        pick_b: Option<[f64; 2]>,
        #[serde(default)]
        layer: Option<String>,
    }
    let a: A = parse_args(args)?;
    let ia = entity_id_val(&a.a)?;
    let ib = entity_id_val(&a.b)?;
    if ia == ib {
        return Err("chamfer needs two different entities".into());
    }
    let (pa, pb) = picks(&a.pick, a.pick_a, a.pick_b)?;
    let sa = shape_of(doc, ia)?;
    let sb = shape_of(doc, ib)?;
    let (line, na, nb) = construct::chamfer_shapes(&sa, &sb, a.d1, a.d2.unwrap_or(a.d1), pa, pb)?;
    construct::set_shape(doc, ia, na);
    construct::set_shape(doc, ib, nb);
    let line = match line {
        Some(shape) => {
            let style = doc.entity(ia).expect("checked above").clone();
            let id = construct::add_like(doc, &style, shape, a.layer.as_deref());
            joined_attrs(doc, id, ia, ib);
            ent_json(doc, id)
        }
        None => Value::Null,
    };
    ok(json!({ "line": line, "a": ent_json(doc, ia), "b": ent_json(doc, ib) }))
}

fn boundary_of(doc: &Document, v: &Value, exclude: EntityId) -> Result<Vec<Curve>, String> {
    let mut ids = parse_target(doc, v)?;
    ids.retain(|i| *i != exclude);
    construct::boundary_curves(doc, &ids)
}

fn trim_to(doc: &mut Document, args: &str) -> R {
    #[derive(Deserialize)]
    struct A {
        id: Value,
        boundary: Value,
        keep: [f64; 2],
    }
    let a: A = parse_args(args)?;
    let id = entity_id_val(&a.id)?;
    let shape = shape_of(doc, id)?;
    let b = boundary_of(doc, &a.boundary, id)?;
    let new = construct::trim_to_shape(&shape, &b, pt(a.keep))?;
    construct::set_shape(doc, id, new);
    ok(ent_json(doc, id))
}

fn extend_to(doc: &mut Document, args: &str) -> R {
    #[derive(Deserialize)]
    struct A {
        id: Value,
        boundary: Value,
        #[serde(default)]
        end: Option<String>,
    }
    let a: A = parse_args(args)?;
    let id = entity_id_val(&a.id)?;
    let shape = shape_of(doc, id)?;
    let b = boundary_of(doc, &a.boundary, id)?;
    let which = match a.end.as_deref() {
        None | Some("nearest") | Some("auto") => ExtendEnd::Nearest,
        Some("start") | Some("from") => ExtendEnd::Start,
        Some("end") | Some("to") => ExtendEnd::End,
        Some(o) => return Err(format!("unknown end '{o}' (start|end|nearest)")),
    };
    let new = construct::extend_to_shape(&shape, &b, which)?;
    construct::set_shape(doc, id, new);
    ok(ent_json(doc, id))
}

fn tangent_lines(args: &str) -> R {
    #[derive(Deserialize)]
    struct A {
        c1: [f64; 2],
        r1: f64,
        c2: [f64; 2],
        r2: f64,
        #[serde(default)]
        kind: Option<String>,
    }
    let a: A = parse_args(args)?;
    let inner = match a.kind.as_deref().unwrap_or("outer") {
        "outer" | "external" => false,
        "inner" | "internal" | "cross" => true,
        o => return Err(format!("unknown tangent kind '{o}' (outer|inner)")),
    };
    let lines = construct::tangent_lines(pt(a.c1), a.r1, pt(a.c2), a.r2, inner);
    let v: Vec<[[f64; 2]; 2]> = lines
        .iter()
        .map(|(p, q)| [[p.x, p.y], [q.x, q.y]])
        .collect();
    ok(json!(v))
}

fn tangent_from_point(args: &str) -> R {
    #[derive(Deserialize)]
    struct A {
        point: [f64; 2],
        center: [f64; 2],
        radius: f64,
    }
    let a: A = parse_args(args)?;
    let pts = construct::tangent_from_point(pt(a.point), pt(a.center), a.radius);
    let v: Vec<[f64; 2]> = pts.iter().map(|p| [p.x, p.y]).collect();
    ok(json!(v))
}

fn array(doc: &mut Document, args: &str) -> R {
    #[derive(Deserialize)]
    struct Polar {
        center: [f64; 2],
        count: usize,
        #[serde(default = "d_360")]
        angle: f64,
        #[serde(default = "d_true")]
        rotate: bool,
    }
    #[derive(Deserialize)]
    struct Rect {
        rows: usize,
        cols: usize,
        #[serde(default)]
        dx: f64,
        #[serde(default)]
        dy: f64,
    }
    #[derive(Deserialize)]
    struct A {
        target: Value,
        #[serde(default)]
        polar: Option<Polar>,
        #[serde(default)]
        rect: Option<Rect>,
    }
    let a: A = parse_args(args)?;
    let ids = parse_target(doc, &a.target)?;
    let mut sources = Vec::new();
    for id in &ids {
        sources.push(
            doc.entity(*id)
                .cloned()
                .ok_or_else(|| format!("entity e_{} not found", id.0))?,
        );
    }
    let xforms: Vec<Affine> = match (a.polar, a.rect) {
        (Some(p), None) => {
            if p.count < 1 {
                return Err("polar count must be >= 1".into());
            }
            // Reference point for non-rotating copies: centre of the selection.
            let (mut x0, mut y0, mut x1, mut y1) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
            for e in &sources {
                let b = e.shape.bbox();
                x0 = x0.min(b.0);
                y0 = y0.min(b.1);
                x1 = x1.max(b.2);
                y1 = y1.max(b.3);
            }
            let rp = Point::new((x0 + x1) / 2.0, (y0 + y1) / 2.0);
            construct::polar_array_transforms(pt(p.center), p.count, p.angle, p.rotate, rp)
        }
        (None, Some(r)) => {
            if r.rows < 1 || r.cols < 1 {
                return Err("rect rows and cols must be >= 1".into());
            }
            construct::rect_array_transforms(r.rows, r.cols, r.dx, r.dy)
        }
        _ => return Err("array needs exactly one of {polar} or {rect}".into()),
    };
    let mut out = Vec::new();
    for xf in xforms {
        for src in &sources {
            let id = doc.alloc_id();
            let mut e = src.clone();
            e.id = id;
            e.shape = e.shape.transformed(xf);
            doc.entities.push(e);
            out.push(ent_json(doc, id));
        }
    }
    ok(json!(out))
}

fn clear(doc: &mut Document, args: &str) -> R {
    #[derive(Deserialize, Default)]
    struct A {
        #[serde(default)]
        all: bool,
    }
    let a: A = if args.trim().is_empty() {
        A::default()
    } else {
        parse_args(args).unwrap_or_default()
    };
    doc.checkpoint();
    doc.entities.clear();
    doc.layers.clear();
    doc.parts.clear();
    doc.part_defs.clear();
    if a.all {
        doc.blocks.clear();
        doc.linetypes = crate::document::standard_linetypes();
    }
    ok(json!({ "ok": true, "all": a.all }))
}

// ── Text & dimensions ──────────────────────────────────────────────────

fn add_text(doc: &mut Document, args: &str) -> R {
    #[derive(Deserialize)]
    struct A {
        text: String,
        at: [f64; 2],
        #[serde(default = "d_text_height")]
        height: f64,
        /// Degrees, CCW.
        #[serde(default)]
        rotation: f64,
        #[serde(default)]
        align: Option<String>,
        #[serde(default)]
        valign: Option<String>,
        #[serde(flatten)]
        style: Style,
    }
    let a: A = parse_args(args)?;
    if a.height <= 0.0 {
        return Err("text height must be > 0".into());
    }
    let h = HAlign::parse(a.align.as_deref())?;
    let v = VAlign::parse(a.valign.as_deref())?;
    let rot = a.rotation.to_radians();
    let pos = annotate::aligned_text_position(&a.text, pt(a.at), a.height, rot, h, v);
    let id = add_styled(
        doc,
        Shape::Text {
            text: a.text.clone(),
            position: pos,
            height: a.height,
            rotation: rot,
        },
        &a.style,
    );
    let width = crate::text::text_width(&a.text, a.height);
    ok(json!({
        "id": format!("e_{}", id.0),
        "width": width,
        "height": a.height,
        "at": a.at,
        "position": [pos.x, pos.y],
        "rotation": a.rotation,
    }))
}

#[derive(Deserialize)]
struct DimCommon {
    #[serde(default = "d_text_height")]
    text_height: f64,
    #[serde(default)]
    arrow: Option<f64>,
    #[serde(default)]
    label: Option<String>,
    #[serde(default)]
    decimals: Option<usize>,
    #[serde(default)]
    layer: Option<String>,
    #[serde(default)]
    color: Option<ColorArg>,
}

impl DimCommon {
    fn style(&self) -> Result<DimStyle, String> {
        if self.text_height <= 0.0 {
            return Err("text_height must be > 0".into());
        }
        Ok(DimStyle::new(
            &self.layer.clone().unwrap_or_else(default_layer),
            self.color.clone().map(ColorArg::resolve),
            self.text_height,
            self.arrow,
        ))
    }
}

fn dim_result(
    doc: &Document,
    value: f64,
    label: &str,
    em_ids: &[EntityId],
    text: Option<EntityId>,
) -> R {
    let _ = doc;
    ok(json!({
        "value": value,
        "distance": value,
        "label": label,
        "ids": em_ids.iter().map(|i| format!("e_{}", i.0)).collect::<Vec<_>>(),
        "text": text.map(|i| format!("e_{}", i.0)),
    }))
}

fn label_of(doc: &Document, text: Option<EntityId>) -> String {
    text.and_then(|t| doc.entity(t))
        .and_then(|e| match &e.shape {
            Shape::Text { text, .. } => Some(text.clone()),
            _ => None,
        })
        .unwrap_or_default()
}

fn measure(doc: &mut Document, args: &str) -> R {
    #[derive(Deserialize)]
    struct A {
        from: [f64; 2],
        to: [f64; 2],
        #[serde(default = "d_offset")]
        offset: f64,
        #[serde(default)]
        dir: Option<String>,
        #[serde(default)]
        shift: f64,
        #[serde(flatten)]
        c: DimCommon,
    }
    let a: A = parse_args(args)?;
    let dir = DimDir::parse(a.dir.as_deref())?;
    let tpl = a.c.label.clone().unwrap_or_else(|| "<>".into());
    let mut em = Emitter::new(doc, a.c.style()?);
    let v = annotate::linear_dimension(
        &mut em,
        pt(a.from),
        pt(a.to),
        a.offset,
        dir,
        &tpl,
        a.c.decimals,
        a.shift,
    )?;
    let (ids, text) = (em.ids.clone(), em.text_id);
    let label = label_of(doc, text);
    dim_result(doc, v, &label, &ids, text)
}

fn dim_circle(doc: &mut Document, args: &str, diameter: bool) -> R {
    #[derive(Deserialize)]
    struct A {
        #[serde(default)]
        id: Option<Value>,
        #[serde(default)]
        center: Option<[f64; 2]>,
        #[serde(default)]
        radius: Option<f64>,
        /// Degrees; default 45 (or the arc's mid-angle).
        #[serde(default)]
        angle: Option<f64>,
        #[serde(flatten)]
        c: DimCommon,
    }
    let a: A = parse_args(args)?;
    let (c, r, default_ang) = if let Some(idv) = &a.id {
        let id = entity_id_val(idv)?;
        match shape_of(doc, id)? {
            Shape::Circle(ci) => (ci.center, ci.radius, PI / 4.0),
            Shape::Arc {
                center,
                radius,
                start_angle,
                end_angle,
            } => {
                let sw = (end_angle - start_angle).rem_euclid(TAU);
                (center, radius, start_angle + sw / 2.0)
            }
            _ => return Err(format!("entity e_{} is not a circle or arc", id.0)),
        }
    } else {
        (
            pt(a.center.ok_or("need id or center + radius")?),
            a.radius.ok_or("need id or center + radius")?,
            PI / 4.0,
        )
    };
    let ang = deg_opt(a.angle).unwrap_or(default_ang);
    let tpl =
        a.c.label
            .clone()
            .unwrap_or_else(|| if diameter { "Ø<>" } else { "R<>" }.into());
    let mut em = Emitter::new(doc, a.c.style()?);
    let v = if diameter {
        annotate::diameter_dimension(&mut em, c, r, ang, &tpl, a.c.decimals)?
    } else {
        annotate::radius_dimension(&mut em, c, r, ang, &tpl, a.c.decimals)?
    };
    let (ids, text) = (em.ids.clone(), em.text_id);
    let label = label_of(doc, text);
    dim_result(doc, v, &label, &ids, text)
}

fn dim_angle(doc: &mut Document, args: &str) -> R {
    #[derive(Deserialize)]
    struct A {
        #[serde(default)]
        center: Option<[f64; 2]>,
        #[serde(default)]
        p1: Option<[f64; 2]>,
        #[serde(default)]
        p2: Option<[f64; 2]>,
        #[serde(default)]
        l1: Option<Value>,
        #[serde(default)]
        l2: Option<Value>,
        #[serde(default)]
        radius: Option<f64>,
        #[serde(flatten)]
        c: DimCommon,
    }
    let a: A = parse_args(args)?;
    let (v, p1, p2) = match (a.center, a.p1, a.p2, &a.l1, &a.l2) {
        (Some(c), Some(p1), Some(p2), _, _) => (pt(c), pt(p1), pt(p2)),
        (_, _, _, Some(l1), Some(l2)) => {
            let (a0, a1) = line_val(doc, l1)?;
            let (b0, b1) = line_val(doc, l2)?;
            let x = construct::line_line_intersection(a0, a1, b0, b1, false)
                .ok_or("lines are parallel: no angle")?;
            let far = |p: Point, q: Point| {
                if (p - x).hypot() >= (q - x).hypot() {
                    p
                } else {
                    q
                }
            };
            (x, far(a0, a1), far(b0, b1))
        }
        _ => return Err("dimAngle needs {center, p1, p2} or {l1, l2}".into()),
    };
    let tpl = a.c.label.clone().unwrap_or_else(|| "<>°".into());
    let mut em = Emitter::new(doc, a.c.style()?);
    let deg = annotate::angle_dimension(&mut em, v, p1, p2, a.radius, &tpl, a.c.decimals)?;
    let (ids, text) = (em.ids.clone(), em.text_id);
    let label = label_of(doc, text);
    dim_result(doc, deg, &label, &ids, text)
}

// ── Fills ──────────────────────────────────────────────────────────────

fn parse_region(
    doc: &Document,
    boundary: &Value,
    holes: &Option<Vec<Value>>,
) -> Result<(Ring, Vec<Ring>), String> {
    let outer = ring_val(doc, boundary)?;
    let holes = holes
        .iter()
        .flatten()
        .map(|h| ring_val(doc, h))
        .collect::<Result<Vec<_>, _>>()?;
    Ok((outer, holes))
}

fn add_fill(doc: &mut Document, args: &str) -> R {
    #[derive(Deserialize)]
    struct A {
        boundary: Value,
        #[serde(default)]
        holes: Option<Vec<Value>>,
        #[serde(flatten)]
        style: Style,
    }
    let a: A = parse_args(args)?;
    let (outer, holes) = parse_region(doc, &a.boundary, &a.holes)?;
    let id = add_styled(
        doc,
        Shape::SolidFill {
            boundary: outer.edges,
            holes: holes.into_iter().map(|h| h.edges).collect(),
        },
        &a.style,
    );
    ok(ent_json(doc, id))
}

fn add_hatch(doc: &mut Document, args: &str) -> R {
    #[derive(Deserialize)]
    struct A {
        boundary: Value,
        #[serde(default)]
        holes: Option<Vec<Value>>,
        #[serde(default = "d_hatch_angle")]
        angle: f64,
        #[serde(default = "d_hatch_spacing")]
        spacing: f64,
        #[serde(default)]
        pattern: Option<String>,
        #[serde(flatten)]
        style: Style,
    }
    let a: A = parse_args(args)?;
    let pattern = a
        .pattern
        .clone()
        .unwrap_or_else(|| "lines".into())
        .to_ascii_uppercase();
    if pattern == "SOLID" {
        return add_fill(doc, args);
    }
    if a.spacing.is_nan() || a.spacing <= 0.0 {
        return Err("hatch spacing must be > 0".into());
    }
    let (outer, holes) = parse_region(doc, &a.boundary, &a.holes)?;
    let mut rings = vec![outer.poly];
    rings.extend(holes.into_iter().map(|h| h.poly));
    let angles: Vec<f64> = match pattern.as_str() {
        "LINES" | "ANSI31" => vec![a.angle],
        "CROSS" | "ANSI37" | "NET" => vec![a.angle, a.angle + 90.0],
        o => return Err(format!("unknown hatch pattern '{o}' (lines|cross|solid)")),
    };
    let mut ids = Vec::new();
    for ang in angles {
        for (p, q) in hatch::hatch_lines_for_rings(&rings, ang.to_radians(), a.spacing) {
            let id = add_styled(doc, Shape::Line(Line::new(p, q)), &a.style);
            ids.push(format!("e_{}", id.0));
        }
    }
    ok(json!({ "lines": ids.len(), "ids": ids }))
}

// ── Curves ─────────────────────────────────────────────────────────────

fn add_spline(doc: &mut Document, args: &str) -> R {
    #[derive(Deserialize)]
    struct A {
        points: Vec<[f64; 2]>,
        #[serde(default)]
        fit: bool,
        #[serde(default = "d_three")]
        degree: usize,
        #[serde(default)]
        closed: bool,
        #[serde(flatten)]
        style: Style,
    }
    let a: A = parse_args(args)?;
    let mut pts: Vec<Point> = a.points.iter().map(|p| pt(*p)).collect();
    if pts.len() < 2 {
        return Err("spline needs at least 2 points".into());
    }
    if a.degree < 1 {
        return Err("spline degree must be >= 1".into());
    }
    if a.closed && (pts[0] - *pts.last().expect("non-empty")).hypot() > 1e-12 {
        pts.push(pts[0]);
    }
    let (degree, knots, ctrl) = if a.fit {
        spline::interpolate(&pts, a.degree)?
    } else {
        let p = a.degree.min(pts.len() - 1);
        (p, spline::clamped_knots(pts.len(), p), pts)
    };
    let id = add_styled(
        doc,
        Shape::Spline {
            degree: degree as i32,
            knots,
            control_points: ctrl,
            closed: false,
        },
        &a.style,
    );
    ok(ent_json(doc, id))
}

/// Polyline vertices as [x, y] or [x, y, bulge].
fn parse_vertices(points: &[Vec<f64>], bulges: &Option<Vec<f64>>) -> Result<Vec<LwVertex>, String> {
    points
        .iter()
        .enumerate()
        .map(|(i, p)| {
            if p.len() < 2 || p.len() > 3 {
                return Err(format!("vertex {i} must be [x, y] or [x, y, bulge]"));
            }
            let b = bulges
                .as_ref()
                .and_then(|b| b.get(i).copied())
                .unwrap_or_else(|| p.get(2).copied().unwrap_or(0.0));
            Ok(LwVertex {
                point: Point::new(p[0], p[1]),
                bulge: b,
            })
        })
        .collect()
}

fn verts_to_shape(v: Vec<LwVertex>, closed: bool) -> Shape {
    if v.iter().any(|x| x.bulge.abs() > 1e-12) {
        Shape::LwPolyline {
            vertices: v,
            closed,
        }
    } else {
        Shape::Polyline {
            points: v.into_iter().map(|x| x.point).collect(),
            closed,
        }
    }
}

fn add_polyline(doc: &mut Document, args: &str) -> R {
    #[derive(Deserialize)]
    struct A {
        points: Vec<Vec<f64>>,
        #[serde(default)]
        closed: bool,
        #[serde(default)]
        bulges: Option<Vec<f64>>,
        #[serde(flatten)]
        style: Style,
    }
    let a: A = parse_args(args)?;
    let v = parse_vertices(&a.points, &a.bulges)?;
    let id = add_styled(doc, verts_to_shape(v, a.closed), &a.style);
    ok(ent_json(doc, id))
}

// ── Regions ────────────────────────────────────────────────────────────

fn region_clip(doc: &mut Document, args: &str) -> R {
    #[derive(Deserialize)]
    struct A {
        subject: Value,
        clip: Value,
        #[serde(default)]
        op: Option<String>,
    }
    let a: A = parse_args(args)?;
    let op = region::BoolOp::parse(a.op.as_deref())?;
    let conv = |rs: Vec<Vec<Point>>| -> Vec<Vec<[f64; 2]>> {
        rs.into_iter()
            .map(|r| r.into_iter().map(|p| [p.x, p.y]).collect())
            .collect()
    };
    let s = conv(rings_val(doc, &a.subject)?);
    let c = conv(rings_val(doc, &a.clip)?);
    let out: Vec<Value> = region::boolean(&s, &c, op)
        .iter()
        .map(|r| json!({ "outer": r.outer, "holes": r.holes, "area": region::region_area(r) }))
        .collect();
    ok(json!(out))
}

// ── Blocks ─────────────────────────────────────────────────────────────

fn define_block(doc: &mut Document, args: &str) -> R {
    #[derive(Deserialize)]
    struct ShapeDef {
        #[serde(rename = "type")]
        shape_type: String,
        #[serde(default)]
        start: Option<[f64; 2]>,
        #[serde(default)]
        end: Option<[f64; 2]>,
        #[serde(default)]
        center: Option<[f64; 2]>,
        #[serde(default)]
        radius: Option<f64>,
        #[serde(default)]
        from: Option<f64>,
        #[serde(default)]
        to: Option<f64>,
        #[serde(default)]
        points: Option<Vec<Vec<f64>>>,
        #[serde(default)]
        closed: Option<bool>,
        #[serde(default)]
        text: Option<String>,
        #[serde(default, alias = "position")]
        at: Option<[f64; 2]>,
        #[serde(default)]
        height: Option<f64>,
        #[serde(default)]
        rotation: Option<f64>,
        #[serde(default)]
        align: Option<String>,
        #[serde(default)]
        valign: Option<String>,
        #[serde(default)]
        layer: Option<String>,
        #[serde(default, deserialize_with = "crate::attrs::deserialize")]
        attrs: crate::attrs::Attrs,
        #[serde(default)]
        color: Option<ColorArg>,
    }
    #[derive(Deserialize)]
    struct A {
        name: String,
        shapes: Vec<ShapeDef>,
        #[serde(default, alias = "insertPoint")]
        insert_point: Option<[f64; 2]>,
        #[serde(default = "default_layer", alias = "defaultLayer")]
        default_layer: String,
        #[serde(default, deserialize_with = "crate::attrs::deserialize")]
        attrs: crate::attrs::Attrs,
    }
    let a: A = parse_args(args)?;
    let mut shapes = Vec::new();
    let mut shape_attrs = Vec::new();
    for (i, sd) in a.shapes.iter().enumerate() {
        let color = sd.color.clone().map(ColorArg::resolve);
        let layer = sd.layer.clone().unwrap_or_default();
        let shape = match sd.shape_type.as_str() {
            "line" => {
                let s = sd.start.ok_or("line needs start")?;
                let e = sd.end.ok_or("line needs end")?;
                Shape::Line(Line::new(pt(s), pt(e)))
            }
            "circle" => {
                let c = sd.center.ok_or("circle needs center")?;
                let r = sd.radius.ok_or("circle needs radius")?;
                Shape::Circle(Circle::new(pt(c), r))
            }
            "arc" => {
                let c = sd.center.ok_or("arc needs center")?;
                let r = sd.radius.ok_or("arc needs radius")?;
                Shape::Arc {
                    center: pt(c),
                    radius: r,
                    start_angle: sd.from.unwrap_or(0.0).to_radians(),
                    end_angle: sd.to.unwrap_or(360.0).to_radians(),
                }
            }
            "polyline" => {
                let pts = sd.points.as_ref().ok_or("polyline needs points")?;
                verts_to_shape(parse_vertices(pts, &None)?, sd.closed.unwrap_or(false))
            }
            "text" => {
                let t = sd.text.clone().ok_or("text shape needs text")?;
                let at = sd.at.ok_or("text shape needs at")?;
                let h = sd.height.unwrap_or(2.5);
                let rot = sd.rotation.unwrap_or(0.0).to_radians();
                let pos = annotate::aligned_text_position(
                    &t,
                    pt(at),
                    h,
                    rot,
                    HAlign::parse(sd.align.as_deref())?,
                    VAlign::parse(sd.valign.as_deref())?,
                );
                Shape::Text {
                    text: t,
                    position: pos,
                    height: h,
                    rotation: rot,
                }
            }
            other => return Err(format!("unknown block shape type (shape {i}): {other}")),
        };
        shapes.push((shape, layer, color));
        shape_attrs.push(sd.attrs.clone());
    }
    let ip = a.insert_point.unwrap_or([0.0, 0.0]);
    doc.define_block(BlockDef {
        attrs: a.attrs,
        shape_attrs,
        name: a.name.clone(),
        shapes,
        insert_point: pt(ip),
        default_layer: a.default_layer,
    });
    ok(json!({ "defined": a.name }))
}

fn place(doc: &mut Document, args: &str) -> R {
    #[derive(Deserialize)]
    struct A {
        block: String,
        at: [f64; 2],
        #[serde(default)]
        rotation: f64,
        #[serde(default)]
        layer: Option<String>,
        /// number or [sx, sy]
        #[serde(default)]
        scale: Option<Value>,
        #[serde(default, rename = "xScale")]
        x_scale: Option<f64>,
        #[serde(default, rename = "yScale")]
        y_scale: Option<f64>,
        #[serde(default, deserialize_with = "crate::attrs::deserialize")]
        attrs: crate::attrs::Attrs,
    }
    let a: A = parse_args(args)?;
    let (mut sx, mut sy) = match &a.scale {
        None | Some(Value::Null) => (1.0, 1.0),
        Some(Value::Number(n)) => {
            let s = n.as_f64().unwrap_or(1.0);
            (s, s)
        }
        Some(v @ Value::Array(_)) => {
            let p = point_val(v)?;
            (p.x, p.y)
        }
        _ => return Err("scale must be a number or [sx, sy]".into()),
    };
    if let Some(x) = a.x_scale {
        sx = x;
    }
    if let Some(y) = a.y_scale {
        sy = y;
    }
    let ids = doc.place_block_scaled(&a.block, pt(a.at), a.rotation, sx, sy, a.layer.as_deref());
    if ids.is_empty() {
        return Err(format!("block '{}' not defined", a.block));
    }
    if let Some(e) = doc.entity_mut(ids[0]) {
        e.attrs = a.attrs;
    }
    ok(ent_json(doc, ids[0]))
}

// ── Introspection ──────────────────────────────────────────────────────

fn methods() -> R {
    let m = json!([
        {"name": "describe", "args": "{}", "desc": "Scene summary: bounds, entity counts, layers"},
        {"name": "entities", "args": "{expand?, layer?, type?, attrs?, has?, ids?, bounds?}", "desc": "Entities matching a selector (all without one). expand=true adds block children, matched one by one"},
        {"name": "entity", "args": "{id}", "desc": "Get one entity by ID"},
        {"name": "children", "args": "{id}", "desc": "Expanded child shapes of a block insert"},
        {"name": "addLine", "args": "{start, end, layer?, color?, linetype?, lineweight?, transparency?}", "desc": "Add line"},
        {"name": "addCircle", "args": "{center, radius, layer?, color?, linetype?, lineweight?, transparency?}", "desc": "Add circle"},
        {"name": "addArc", "args": "{center, radius, from?, to?, p1?, p2?, shortest?, layer?, color?, linetype?, lineweight?, transparency?}", "desc": "Add arc (degrees or point-based)"},
        {"name": "addPolyline", "args": "{points: [x,y] | [x,y,bulge], closed?, bulges?, layer?, color?, ...}", "desc": "Add polyline; nonzero bulge makes that segment an arc"},
        {"name": "addSpline", "args": "{points, fit?, degree?, closed?, layer?, color?, ...}", "desc": "Add B-spline: control points, or fit=true to pass through the points"},
        {"name": "addText", "args": "{text, at, height?, rotation?, align?, valign?, layer?, color?}", "desc": "Add single-line TEXT. rotation in degrees; align left|center|right; valign baseline|middle|top"},
        {"name": "addLayer", "args": "{name, color?, linetype?, lineweight?}", "desc": "Create/update layer"},
        {"name": "addLinetype", "args": "{name, pattern, description?}", "desc": "Define named linetype. pattern=[dash,gap,...] in drawing units"},
        {"name": "linetypes", "args": "{}", "desc": "List all defined linetypes"},
        {"name": "aciColors", "args": "{}", "desc": "ACI palette: 255 entries of {index, color: [r,g,b]}"},
        {"name": "measure", "args": "{from, to, offset?, dir?: h|v|aligned, label?, decimals?, text_height?, arrow?, shift?, layer?, color?}", "desc": "Linear dimension: extension lines, filled arrows, TEXT label ('<>' = value)"},
        {"name": "dimRadius", "args": "{id | center+radius, angle?, label?, decimals?, text_height?, layer?}", "desc": "Radius dimension (default label 'R<>')"},
        {"name": "dimDiameter", "args": "{id | center+radius, angle?, label?, decimals?, text_height?, layer?}", "desc": "Diameter dimension (default label 'Ø<>')"},
        {"name": "dimAngle", "args": "{center, p1, p2 | l1, l2, radius?, label?, decimals?, text_height?, layer?}", "desc": "Angle dimension, CCW from p1 to p2 (degrees)"},
        {"name": "addHatch", "args": "{boundary, holes?, angle?, spacing?, pattern?: lines|cross|solid, layer?}", "desc": "Line hatch clipped to boundary minus holes (rings: points, {center,radius} or entity id)"},
        {"name": "addFill", "args": "{boundary, holes?, layer?, color?, transparency?}", "desc": "Solid fill (DWG SOLID hatch). Circle/arc rings stay exact"},
        {"name": "regionClip", "args": "{subject, clip, op?: intersect|union|subtract|xor}", "desc": "Polygon boolean. Returns [{outer, holes, area}]"},
        {"name": "defineBlock", "args": "{name, shapes, insert_point?, default_layer?, attrs?}", "desc": "Define block from shapes: line|circle|arc|polyline|text"},
        {"name": "place", "args": "{block, at, rotation?, scale? (n | [sx,sy]), xScale?, yScale?, layer?, attrs?}", "desc": "Place block instance"},
        {"name": "clone", "args": "{source, name, replaceText?}", "desc": "Clone block def with text substitution"},
        {"name": "remove", "args": "{target}", "desc": "Delete entities by ID(s) or selector"},
        {"name": "move", "args": "{target, dx, dy}", "desc": "Translate entities"},
        {"name": "copy", "args": "{target, dx, dy}", "desc": "Duplicate entities with offset"},
        {"name": "rotate", "args": "{target, center, angle}", "desc": "Rotate entities (degrees)"},
        {"name": "mirror", "args": "{target, p1, p2}", "desc": "Mirror entities across a line"},
        {"name": "array", "args": "{target, polar?: {center, count, angle?, rotate?} | rect?: {rows, cols, dx, dy}}", "desc": "Polar or rectangular array; returns the new copies"},
        {"name": "offset", "args": "{id, distance}", "desc": "Parallel offset (lines, arcs, circles)"},
        {"name": "trim", "args": "{id, cut, keep}", "desc": "Trim entity at point. keep='start'|'end'"},
        {"name": "trimTo", "args": "{id, boundary, keep}", "desc": "Trim line/arc/circle at its intersections with boundary entities, keeping the piece containing 'keep'"},
        {"name": "extendTo", "args": "{id, boundary, end?: start|end|nearest}", "desc": "Extend a line/arc until it meets the boundary"},
        {"name": "fillet", "args": "{a, b, radius, pick?: [pa, pb]}", "desc": "Tangent arc between lines/arcs/circles; trims/extends both. Returns {arc, a, b}"},
        {"name": "chamfer", "args": "{a, b, d1, d2?, pick?}", "desc": "Bevel two lines. Returns {line, a, b}"},
        {"name": "lineLineIntersection", "args": "{l1, l2, segments?}", "desc": "Intersection of two lines (point pairs or line ids); null if parallel/missing"},
        {"name": "lineCircleIntersection", "args": "{line, center, radius}", "desc": "Line/circle intersections"},
        {"name": "circleCircleIntersection", "args": "{c1, r1, c2, r2}", "desc": "Circle/circle intersections"},
        {"name": "tangentLines", "args": "{c1, r1, c2, r2, kind?: outer|inner}", "desc": "Tangent lines to two circles: [[p1, p2], ...]"},
        {"name": "tangentFromPoint", "args": "{point, center, radius}", "desc": "Tangent points on a circle seen from a point"},
        {"name": "projectOntoCircle", "args": "{point, center, radius}", "desc": "Nearest point on circle"},
        {"name": "projectOnto", "args": "{point, line}", "desc": "Perpendicular projection onto line"},
        {"name": "angleOf", "args": "{point, center}", "desc": "Angle in degrees from center to point"},
        {"name": "distance", "args": "{a, b}", "desc": "Distance between points"},
        {"name": "midpoint", "args": "{a, b}", "desc": "Midpoint of two points"},
        {"name": "direction", "args": "{a, b}", "desc": "Direction a->b in degrees"},
        {"name": "connectedTo", "args": "{id, tolerance?}", "desc": "Entities sharing an endpoint"},
        {"name": "clear", "args": "{all?}", "desc": "Remove all entities and layers; all=true also clears blocks and resets linetypes"},
        {"name": "checkpoint", "args": "{}", "desc": "Save undo snapshot"},
        {"name": "undo", "args": "{}", "desc": "Undo last checkpoint"},
        {"name": "redo", "args": "{}", "desc": "Redo last undo"},
        {"name": "methods", "args": "{}", "desc": "List all available cad_call methods"},
    ]);
    // Attributes and set queries (a second list: one json! array this long
    // exceeds the macro recursion limit).
    let more = json!([
        {"name": "setAttrs", "args": "{target, attrs, replace?}", "desc": "Merge attributes into entities (null removes a key); all or nothing"},
        {"name": "removeAttrs", "args": "{target, keys}", "desc": "Remove attribute keys from entities"},
        {"name": "attrs", "args": "{id}", "desc": "One entity's attributes"},
        {"name": "setBlockAttrs", "args": "{name, attrs, replace?, shape?}", "desc": "Attributes of a block definition, or of one of its shapes"},
        {"name": "blockAttrs", "args": "{name}", "desc": "A block definition's attributes and its shapes' attributes"},
        {"name": "pairsWithin", "args": "{a, b?, distance, differ?, attrs?, limit?}", "desc": "All pairs closer than distance (b absent: within a), exact, with closest points"},
        {"name": "overlapping", "args": "{a, b?, areas?, differ?, attrs?, limit?}", "desc": "All pairs whose outlines meet (areas: or one lies inside the other), with the points"},
        {"name": "inside", "args": "{a, region, strict?, differ?, attrs?}", "desc": "Entities of a inside a rectangle, a ring, or each closed entity of a selection"},
        {"name": "nearest", "args": "{a, b?, k?, within?, differ?, attrs?}", "desc": "The k nearest entities of b for each entity of a"},
        {"name": "near", "args": "{point, radius, select?}", "desc": "Entities within radius of a point, nearest first, with distance"},
        {"name": "inBounds", "args": "{rect, mode?: intersect|contain, select?}", "desc": "Entities whose bounding box overlaps or lies in a rectangle"},
    ]);
    // Solid parts (dispatch_3d.rs).
    let solids = json!([
        {"name": "extrude", "args": "{profile, plane?, at?, distance, taper?, attrs?, name?}", "desc": "New part: a profile extruded along its plane normal"},
        {"name": "revolve", "args": "{profile, plane?, at?, axis, angle?, attrs?, name?}", "desc": "New part: a profile turned about an axis in its plane"},
        {"name": "sweep", "args": "{profile, plane?, at?, path, attrs?, name?}", "desc": "New part: a profile carried along a path of lines and arcs"},
        {"name": "loft", "args": "{sections, ruled?, attrs?, name?}", "desc": "New part: a solid through profiles on planes"},
        {"name": "cut", "args": "{target, tool, frame?}", "desc": "Subtract a tool (extrude|revolve|sweep|loft body, or another part) from a part"},
        {"name": "union", "args": "{target, tool, frame?}", "desc": "Add a tool body to a part"},
        {"name": "intersect", "args": "{target, tool, frame?}", "desc": "Keep what a part and a tool body share"},
        {"name": "hole", "args": "{target, at, dir?, dia, depth?, cbore?, frame?}", "desc": "Drill a hole (blind, through, counterbored)"},
        {"name": "fillet3d", "args": "{target, edges, radius, fallback?, frame?}", "desc": "Round selected edges of a part"},
        {"name": "chamfer3d", "args": "{target, edges, d, d2?, frame?}", "desc": "Bevel selected edges of a part"},
        {"name": "move3d", "args": "{target, by}", "desc": "Move parts by [dx, dy, dz]"},
        {"name": "rotate3d", "args": "{target, angle, axis?, about?}", "desc": "Rotate parts about an axis (degrees)"},
        {"name": "copy3d", "args": "{target, move?, rotate?, count?, attrs?, unique?}", "desc": "Placed copies sharing the definition (unique: own definitions)"},
        {"name": "remove3d", "args": "{target}", "desc": "Remove parts; unused definitions go too"},
        {"name": "setPartAttrs", "args": "{target, attrs, replace?}", "desc": "Merge attributes into parts"},
        {"name": "removePartAttrs", "args": "{target, keys}", "desc": "Remove attribute keys from parts"},
        {"name": "parts", "args": "{ids?, def?, attrs?, has?, bounds?, geometry?}", "desc": "Parts with placement, attributes, box, volume, centroid"},
        {"name": "part", "args": "{id, geometry?}", "desc": "One part"},
        {"name": "partDef", "args": "{def}", "desc": "A definition's whole feature tree"},
    ]);
    // Between parts (setquery3d.rs) and mirror images.
    let checks = json!([
        {"name": "mirror3d", "args": "{target, point?, normal, copy?, attrs?}", "desc": "Mirror parts across a plane (a mirrored definition, shared)"},
        {"name": "overlapping3d", "args": "{a, b?, tolerance?, touching?, differ?, attrs?, limit?}", "desc": "All pairs of parts that share volume, with the shared volume"},
        {"name": "pairsWithin3d", "args": "{a, b?, distance, differ?, attrs?, limit?}", "desc": "All pairs of parts closer than distance, with closest points"},
        {"name": "inside3d", "args": "{a, region, strict?, attrs?}", "desc": "Parts inside a box, or inside the solid of each part of a selection"},
        {"name": "distance3d", "args": "{a, b}", "desc": "Distance and closest points of two parts"},
    ]);
    let mut m = m;
    for extra in [more, solids, checks] {
        if let (Some(all), Value::Array(extra)) = (m.as_array_mut(), extra) {
            all.extend(extra);
        }
    }
    ok(m)
}

#[cfg(test)]
mod tests {
    use crate::dispatch::cad_call;
    use crate::document::Document;
    use crate::types::*;
    use serde_json::{json, Value};

    fn call(doc: &mut Document, m: &str, a: Value) -> Value {
        let r = cad_call(doc, m, &a.to_string()).unwrap_or_else(|e| panic!("{m}: {e}"));
        serde_json::from_str(&r).unwrap()
    }
    fn call_err(doc: &mut Document, m: &str, a: Value) -> String {
        cad_call(doc, m, &a.to_string()).expect_err("expected an error")
    }
    fn id(v: &Value) -> String {
        v["id"].as_str().unwrap().to_string()
    }
    fn close(a: &Value, x: f64, y: f64) -> bool {
        (a[0].as_f64().unwrap() - x).abs() < 1e-6 && (a[1].as_f64().unwrap() - y).abs() < 1e-6
    }

    #[test]
    fn fillet_via_cad_call_keeps_ids_and_adds_arc() {
        let mut d = Document::new();
        let a = call(
            &mut d,
            "addLine",
            json!({"start": [0, 0], "end": [100, 0], "layer": "OUT"}),
        );
        let b = call(
            &mut d,
            "addLine",
            json!({"start": [0, 0], "end": [0, 60], "layer": "OUT"}),
        );
        let r = call(
            &mut d,
            "fillet",
            json!({"a": id(&a), "b": id(&b), "radius": 10}),
        );
        assert_eq!(r["a"]["id"], a["id"]);
        assert_eq!(r["b"]["id"], b["id"]);
        assert_eq!(r["arc"]["type"], "arc");
        assert_eq!(r["arc"]["layer"], "OUT");
        assert!(close(&r["arc"]["center"], 10.0, 10.0));
        assert!(close(&r["a"]["start"], 10.0, 0.0));
        assert!(close(&r["b"]["start"], 0.0, 10.0));
        assert_eq!(d.entities.len(), 3);
        // Parallel / too large / same entity / zero radius
        let c = call(&mut d, "addLine", json!({"start": [0, 5], "end": [100, 5]}));
        assert!(call_err(
            &mut d,
            "fillet",
            json!({"a": id(&a), "b": id(&c), "radius": 1})
        )
        .contains("parallel"));
        let e = call(
            &mut d,
            "addLine",
            json!({"start": [200, 0], "end": [200, 3]}),
        );
        let f = call(
            &mut d,
            "addLine",
            json!({"start": [200, 0], "end": [203, 0]}),
        );
        assert!(call_err(
            &mut d,
            "fillet",
            json!({"a": id(&e), "b": id(&f), "radius": 50})
        )
        .contains("too large"));
        assert!(call_err(
            &mut d,
            "fillet",
            json!({"a": id(&e), "b": id(&e), "radius": 1})
        )
        .contains("different"));
        let z = call(
            &mut d,
            "fillet",
            json!({"a": id(&e), "b": id(&f), "radius": 0}),
        );
        assert!(z["arc"].is_null());
    }

    #[test]
    fn fillet_with_pick_points_and_arc() {
        let mut d = Document::new();
        let l = call(
            &mut d,
            "addLine",
            json!({"start": [-60, 22], "end": [60, 22]}),
        );
        let a = call(
            &mut d,
            "addArc",
            json!({"center": [0, 0], "radius": 20, "from": 0, "to": 180}),
        );
        let r = call(
            &mut d,
            "fillet",
            json!({"a": id(&l), "b": a, "radius": 5, "pick": [[50, 22], [19, 6]]}),
        );
        assert_eq!(r["arc"]["radius"], 5.0);
        assert!(close(&r["a"]["end"], 60.0, 22.0));
    }

    #[test]
    fn chamfer_via_cad_call() {
        let mut d = Document::new();
        let a = call(&mut d, "addLine", json!({"start": [0, 0], "end": [50, 0]}));
        let b = call(&mut d, "addLine", json!({"start": [0, 0], "end": [0, 50]}));
        let r = call(
            &mut d,
            "chamfer",
            json!({"a": id(&a), "b": id(&b), "d1": 5}),
        );
        assert!(close(&r["line"]["start"], 5.0, 0.0) && close(&r["line"]["end"], 0.0, 5.0));
        assert!(call_err(
            &mut d,
            "chamfer",
            json!({"a": id(&a), "b": id(&b), "d1": 500})
        )
        .contains("too large"));
    }

    #[test]
    fn trim_and_extend_to_boundaries() {
        let mut d = Document::new();
        let l = call(&mut d, "addLine", json!({"start": [0, 0], "end": [100, 0]}));
        let w = call(
            &mut d,
            "addLine",
            json!({"start": [40, -10], "end": [40, 10]}),
        );
        let c = call(
            &mut d,
            "addCircle",
            json!({"center": [70, 0], "radius": 10}),
        );
        let r = call(
            &mut d,
            "trimTo",
            json!({"id": id(&l), "boundary": [id(&w), id(&c)], "keep": [50, 0]}),
        );
        assert_eq!(r["id"], l["id"]);
        assert!(close(&r["start"], 40.0, 0.0) && close(&r["end"], 60.0, 0.0));
        let s = call(
            &mut d,
            "addLine",
            json!({"start": [0, 30], "end": [10, 30]}),
        );
        let wall = call(
            &mut d,
            "addLine",
            json!({"start": [80, 0], "end": [80, 50]}),
        );
        let r = call(
            &mut d,
            "extendTo",
            json!({"id": id(&s), "boundary": id(&wall)}),
        );
        assert!(close(&r["end"], 80.0, 30.0));
        assert!(call_err(
            &mut d,
            "extendTo",
            json!({"id": id(&s), "boundary": id(&wall), "end": "start"})
        )
        .contains("not reachable"));
        // Trimming the circle with the wall line through it
        let cut = call(
            &mut d,
            "addLine",
            json!({"start": [70, -20], "end": [70, 20]}),
        );
        let r = call(
            &mut d,
            "trimTo",
            json!({"id": id(&c), "boundary": id(&cut), "keep": [80, 0]}),
        );
        assert_eq!(r["type"], "arc");
        assert!(close(&r["start"], 70.0, -10.0) && close(&r["end"], 70.0, 10.0));
    }

    #[test]
    fn intersections_and_tangents() {
        let mut d = Document::new();
        let a = call(&mut d, "addLine", json!({"start": [0, 0], "end": [10, 10]}));
        let p = call(
            &mut d,
            "lineLineIntersection",
            json!({"l1": id(&a), "l2": [[0, 10], [10, 0]]}),
        );
        assert!(close(&p, 5.0, 5.0));
        let n = call(
            &mut d,
            "lineLineIntersection",
            json!({"l1": [[0, 0], [1, 0]], "l2": [[0, 1], [1, 1]]}),
        );
        assert!(n.is_null());
        let n = call(
            &mut d,
            "lineLineIntersection",
            json!({"l1": [[0, 0], [1, 0]], "l2": [[5, -1], [5, 1]], "segments": true}),
        );
        assert!(n.is_null());
        let t = call(
            &mut d,
            "tangentLines",
            json!({"c1": [0, 0], "r1": 10, "c2": [50, 0], "r2": 5, "kind": "outer"}),
        );
        assert_eq!(t.as_array().unwrap().len(), 2);
        let t = call(
            &mut d,
            "tangentLines",
            json!({"c1": [0, 0], "r1": 10, "c2": [12, 0], "r2": 5, "kind": "inner"}),
        );
        assert_eq!(t.as_array().unwrap().len(), 0);
        let t = call(
            &mut d,
            "tangentFromPoint",
            json!({"point": [20, 0], "center": [0, 0], "radius": 10}),
        );
        assert_eq!(t.as_array().unwrap().len(), 2);
    }

    #[test]
    fn arrays() {
        let mut d = Document::new();
        let c = call(&mut d, "addCircle", json!({"center": [50, 0], "radius": 4}));
        let r = call(
            &mut d,
            "array",
            json!({"target": [id(&c)], "polar": {"center": [0, 0], "count": 6}}),
        );
        assert_eq!(r.as_array().unwrap().len(), 5);
        assert!(close(
            &r[0]["center"],
            25.0,
            50.0 * (60f64).to_radians().sin()
        ));
        let l = call(&mut d, "addLine", json!({"start": [0, 0], "end": [1, 0]}));
        let r = call(
            &mut d,
            "array",
            json!({"target": id(&l), "rect": {"rows": 2, "cols": 3, "dx": 10, "dy": 5}}),
        );
        assert_eq!(r.as_array().unwrap().len(), 5);
        assert!(call_err(&mut d, "array", json!({"target": id(&l)})).contains("exactly one"));
    }

    #[test]
    fn clear_all_resets_blocks_and_linetypes_and_undo_restores() {
        let mut d = Document::new();
        call(
            &mut d,
            "addLinetype",
            json!({"name": "MY", "pattern": [1, 1]}),
        );
        call(
            &mut d,
            "defineBlock",
            json!({"name": "B", "shapes": [{"type": "line", "start": [0, 0], "end": [1, 0]}]}),
        );
        call(&mut d, "addLine", json!({"start": [0, 0], "end": [1, 0]}));
        call(&mut d, "clear", json!({}));
        assert!(d.blocks.contains_key("B") && d.linetypes.contains_key("MY"));
        call(&mut d, "clear", json!({"all": true}));
        assert!(d.blocks.is_empty());
        assert!(!d.linetypes.contains_key("MY") && d.linetypes.contains_key("Dashed"));
        assert!(d.undo());
        assert!(d.blocks.contains_key("B"));
    }

    #[test]
    fn add_text_alignment_and_rotation() {
        let mut d = Document::new();
        let t = call(
            &mut d,
            "addText",
            json!({"text": "AB", "at": [100, 50], "height": 10, "align": "center", "valign": "middle", "rotation": 0}),
        );
        let w = t["width"].as_f64().unwrap();
        assert!(close(&t["position"], 100.0 - w / 2.0, 45.0));
        let t = call(
            &mut d,
            "addText",
            json!({"text": "AB", "at": [0, 0], "height": 10, "align": "right", "rotation": 90}),
        );
        assert!(close(&t["position"], 0.0, -w));
        let e = d.entities.last().unwrap();
        match &e.shape {
            Shape::Text { rotation, .. } => {
                assert!((rotation - std::f64::consts::FRAC_PI_2).abs() < 1e-9)
            }
            _ => panic!(),
        }
        assert!(call_err(
            &mut d,
            "addText",
            json!({"text": "x", "at": [0, 0], "align": "diagonal"})
        )
        .contains("align"));
    }

    #[test]
    fn measure_emits_text_entity_and_filled_arrows() {
        let mut d = Document::new();
        let r = call(
            &mut d,
            "measure",
            json!({"from": [0, 0], "to": [80, 0], "offset": 10, "text_height": 3.5, "label": "<> mm", "layer": "DIM"}),
        );
        assert_eq!(r["label"], "80 mm");
        assert_eq!(r["value"], 80.0);
        let text_id = r["text"].as_str().unwrap();
        let te = call(&mut d, "entity", json!({"id": text_id}));
        assert_eq!(te["type"], "text");
        assert_eq!(te["text"], "80 mm");
        assert!(!d
            .entities
            .iter()
            .any(|e| matches!(e.shape, Shape::CurvePath { .. })));
        assert_eq!(
            d.entities
                .iter()
                .filter(|e| matches!(e.shape, Shape::SolidFill { .. }))
                .count(),
            2
        );
        let r = call(
            &mut d,
            "measure",
            json!({"from": [0, 0], "to": [30, 40], "dir": "v", "decimals": 2}),
        );
        assert_eq!(r["label"], "40.00");
    }

    #[test]
    fn radius_diameter_angle_dims() {
        let mut d = Document::new();
        let c = call(&mut d, "addCircle", json!({"center": [0, 0], "radius": 25}));
        let r = call(&mut d, "dimRadius", json!({"id": id(&c), "text_height": 3}));
        assert_eq!(r["label"], "R25");
        let r = call(
            &mut d,
            "dimDiameter",
            json!({"center": [0, 0], "radius": 25, "angle": 30, "text_height": 3}),
        );
        assert_eq!(r["label"], "Ø50");
        let a = call(&mut d, "addLine", json!({"start": [0, 0], "end": [50, 0]}));
        let b = call(&mut d, "addLine", json!({"start": [0, 0], "end": [0, 50]}));
        let r = call(
            &mut d,
            "dimAngle",
            json!({"l1": id(&a), "l2": id(&b), "radius": 20, "text_height": 3}),
        );
        assert_eq!(r["label"], "90°");
        assert!(call_err(&mut d, "dimRadius", json!({"id": id(&a)})).contains("not a circle"));
    }

    fn hatch_lines(d: &Document) -> Vec<kurbo::Line> {
        d.entities
            .iter()
            .filter_map(|e| match e.shape {
                Shape::Line(l) => Some(l),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn hatch_with_hole_keeps_all_scanlines() {
        let mut d = Document::new();
        let r = call(
            &mut d,
            "addHatch",
            json!({"boundary": [[0, 0], [10, 0], [10, 10], [0, 10]],
                   "holes": [[[3, 3], [7, 3], [7, 7], [3, 7]]],
                   "angle": 0, "spacing": 1}),
        );
        // y = 1,2,8,9 -> one span; y = 3..6 -> two spans each; y = 7 lies
        // exactly on the hole's top edge and (half-open rule) stays one span
        // drawn along that edge, never through the hole's interior.
        assert_eq!(r["lines"], 13);
        for l in hatch_lines(&d) {
            let m = l.p0.midpoint(l.p1);
            assert!(
                !(m.x > 3.0 && m.x < 7.0 && m.y > 3.0 && m.y < 7.0),
                "line inside hole: {l:?}"
            );
        }
    }

    #[test]
    fn hatch_through_touching_vertex_is_not_dropped() {
        // Notched square: the scanline y=5 touches the notch tip (5,5).
        let mut d = Document::new();
        call(
            &mut d,
            "addHatch",
            json!({"boundary": [[0, 0], [10, 0], [10, 10], [5, 5], [0, 10]], "angle": 0, "spacing": 5}),
        );
        let total: f64 = hatch_lines(&d).iter().map(|l| (l.p1 - l.p0).hypot()).sum();
        assert!((total - 10.0).abs() < 1e-9, "covered length {total}");
    }

    #[test]
    fn hatch_circle_hole_and_cross_pattern() {
        let mut d = Document::new();
        let h = call(
            &mut d,
            "addCircle",
            json!({"center": [50, 50], "radius": 20}),
        );
        let r = call(
            &mut d,
            "addHatch",
            json!({"boundary": [[0, 0], [100, 0], [100, 100], [0, 100]], "holes": [id(&h)], "pattern": "cross", "spacing": 10}),
        );
        assert!(r["lines"].as_u64().unwrap() > 20);
        for l in hatch_lines(&d) {
            let m = l.p0.midpoint(l.p1);
            assert!(
                (m - kurbo::Point::new(50.0, 50.0)).hypot() > 19.0 || (l.p1 - l.p0).hypot() < 1e-9
            );
        }
    }

    #[test]
    fn add_fill_keeps_arc_holes_abstract() {
        let mut d = Document::new();
        let f = call(
            &mut d,
            "addFill",
            json!({"boundary": [[0, 0], [100, 0], [100, 60], [0, 60]], "holes": [{"center": [30, 30], "radius": 8}], "color": 8}),
        );
        assert_eq!(f["type"], "solid_fill");
        let e = d.entities.last().unwrap();
        let Shape::SolidFill { holes, .. } = &e.shape else {
            panic!()
        };
        assert!(matches!(holes[0][0], FillEdge::ArcTo { .. }));
        assert!(
            call_err(&mut d, "addFill", json!({"boundary": [[0, 0], [1, 0]]})).contains("3 points")
        );
        let s = call(
            &mut d,
            "addHatch",
            json!({"boundary": [[0, 0], [10, 0], [0, 10]], "pattern": "SOLID"}),
        );
        assert_eq!(s["type"], "solid_fill");
    }

    #[test]
    fn spline_and_bulge_polyline() {
        let mut d = Document::new();
        let s = call(
            &mut d,
            "addSpline",
            json!({"points": [[0, 0], [10, 10], [20, 0], [30, 10]], "fit": true}),
        );
        assert_eq!(s["type"], "spline");
        let s2 = call(
            &mut d,
            "addSpline",
            json!({"points": [[0, 0], [10, 10], [20, 0]]}),
        );
        assert_eq!(s2["points"].as_array().unwrap().len(), 3);
        let p = call(
            &mut d,
            "addPolyline",
            json!({"points": [[0, 0, 1], [10, 0], [10, 10]], "closed": true}),
        );
        assert_eq!(p["type"], "lwpolyline");
        let p = call(&mut d, "addPolyline", json!({"points": [[0, 0], [10, 0]]}));
        assert_eq!(p["type"], "polyline");
        assert!(call_err(&mut d, "addSpline", json!({"points": [[0, 0]]})).contains("at least 2"));
    }

    #[test]
    fn region_clip_ops() {
        let mut d = Document::new();
        let sq = |x: f64, y: f64, s: f64| json!([[x, y], [x + s, y], [x + s, y + s], [x, y + s]]);
        let r = call(
            &mut d,
            "regionClip",
            json!({"subject": sq(0., 0., 10.), "clip": sq(5., 5., 10.), "op": "intersect"}),
        );
        assert!((r[0]["area"].as_f64().unwrap() - 25.0).abs() < 1e-9);
        let c = call(&mut d, "addCircle", json!({"center": [5, 5], "radius": 2}));
        let r = call(
            &mut d,
            "regionClip",
            json!({"subject": [sq(0., 0., 10.)], "clip": id(&c), "op": "subtract"}),
        );
        assert_eq!(r[0]["holes"].as_array().unwrap().len(), 1);
        let a = r[0]["area"].as_f64().unwrap();
        assert!(
            (a - (100.0 - std::f64::consts::PI * 4.0)).abs() < 0.05,
            "{a}"
        );
    }

    #[test]
    fn define_block_with_text_and_scaled_place() {
        let mut d = Document::new();
        call(
            &mut d,
            "defineBlock",
            json!({"name": "TAG", "shapes": [
                {"type": "circle", "center": [0, 0], "radius": 5},
                {"type": "text", "text": "A1", "at": [0, 0], "height": 3, "align": "center", "valign": "middle"},
                {"type": "polyline", "points": [[0, 0, 0.5], [5, 0]]}
            ]}),
        );
        assert!(matches!(d.blocks["TAG"].shapes[1].0, Shape::Text { .. }));
        assert!(matches!(
            d.blocks["TAG"].shapes[2].0,
            Shape::LwPolyline { .. }
        ));
        call(
            &mut d,
            "place",
            json!({"block": "TAG", "at": [10, 10], "scale": [2, 3]}),
        );
        match d.entities.last().unwrap().shape {
            Shape::BlockInsert {
                x_scale, y_scale, ..
            } => assert!(x_scale == 2.0 && y_scale == 3.0),
            _ => panic!(),
        }
        assert!(
            call_err(&mut d, "place", json!({"block": "NOPE", "at": [0, 0]}))
                .contains("not defined")
        );
    }

    #[test]
    fn methods_list_has_new_calls() {
        let mut d = Document::new();
        let m = call(&mut d, "methods", json!({}));
        let names: Vec<&str> = m
            .as_array()
            .unwrap()
            .iter()
            .map(|x| x["name"].as_str().unwrap())
            .collect();
        for n in [
            "fillet",
            "chamfer",
            "trimTo",
            "extendTo",
            "array",
            "lineLineIntersection",
            "tangentLines",
            "tangentFromPoint",
            "addFill",
            "addSpline",
            "regionClip",
            "dimRadius",
            "dimDiameter",
            "dimAngle",
        ] {
            assert!(names.contains(&n), "{n} missing");
        }
        // Every listed method is actually dispatchable (no "unknown method").
        for n in names {
            if let Err(e) = cad_call(&mut d, n, "{}") {
                assert!(!e.starts_with("unknown method"), "{n}: {e}");
            }
        }
    }

    #[test]
    fn dwg_roundtrip_of_new_entities() {
        let mut d = Document::new();
        let a = call(
            &mut d,
            "addLine",
            json!({"start": [0, 0], "end": [100, 0], "layer": "OUT"}),
        );
        let b = call(
            &mut d,
            "addLine",
            json!({"start": [0, 0], "end": [0, 60], "layer": "OUT"}),
        );
        call(
            &mut d,
            "fillet",
            json!({"a": id(&a), "b": id(&b), "radius": 10}),
        );
        call(
            &mut d,
            "measure",
            json!({"from": [0, 0], "to": [100, 0], "offset": -15, "text_height": 3.5, "layer": "DIM"}),
        );
        call(
            &mut d,
            "addFill",
            json!({"boundary": [[200, 0], [300, 0], [300, 60], [200, 60]], "holes": [{"center": [250, 30], "radius": 15}], "layer": "FILL"}),
        );
        call(
            &mut d,
            "addPolyline",
            json!({"points": [[0, 100, 0.5], [50, 100], [50, 150]], "layer": "OUT"}),
        );
        let bytes = crate::dwg::export_dwg_bytes(&d).expect("export");
        let back = crate::dwg::load_dwg_bytes(&bytes).expect("reload");
        let count =
            |f: &dyn Fn(&Shape) -> bool| back.entities.iter().filter(|e| f(&e.shape)).count();
        assert_eq!(
            count(&|s| matches!(s, Shape::Arc { radius, .. } if (*radius - 10.0).abs() < 1e-9)),
            1
        );
        let texts: Vec<String> = back
            .entities
            .iter()
            .filter_map(|e| match &e.shape {
                Shape::Text { text, .. } => Some(text.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(texts, vec!["100".to_string()]);
        // 2 arrowheads (SOLID) + the filled region with its hole
        assert!(count(&|s| matches!(s, Shape::SolidFill { holes, .. } if holes.is_empty())) >= 2);
        assert_eq!(
            count(&|s| matches!(s, Shape::SolidFill { holes, .. } if holes.len() == 1)),
            1
        );
        assert_eq!(count(&|s| matches!(s, Shape::LwPolyline { .. })), 1);
    }
}
