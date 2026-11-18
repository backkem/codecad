use crate::document::*;
use crate::dwg::load_dwg;
use crate::geo;
use crate::hatch::entity_endpoints;
use crate::types::*;
use kurbo::{BezPath, Point};
use serde::Deserialize;
use std::collections::HashMap;
use std::f64::consts::PI;

// ── Color argument: accepts [r,g,b] array or ACI index number ─────────

#[derive(Debug, Clone)]
pub(crate) enum ColorArg {
    Rgb([u8; 3]),
    Aci(u8),
}

impl ColorArg {
    pub(crate) fn resolve(self) -> Color {
        match self {
            ColorArg::Rgb(c) => Color::rgb(c[0], c[1], c[2]),
            ColorArg::Aci(i) => crate::dwg::aci_to_rgb(i),
        }
    }
}

impl<'de> Deserialize<'de> for ColorArg {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let v = serde_json::Value::deserialize(d)?;
        match &v {
            serde_json::Value::Array(arr) if arr.len() == 3 => {
                let r = arr[0].as_u64().ok_or(serde::de::Error::custom("bad r"))? as u8;
                let g = arr[1].as_u64().ok_or(serde::de::Error::custom("bad g"))? as u8;
                let b = arr[2].as_u64().ok_or(serde::de::Error::custom("bad b"))? as u8;
                Ok(ColorArg::Rgb([r, g, b]))
            }
            serde_json::Value::Number(n) => {
                let i = n
                    .as_u64()
                    .ok_or(serde::de::Error::custom("bad ACI index"))?
                    as u8;
                Ok(ColorArg::Aci(i))
            }
            _ => Err(serde::de::Error::custom(
                "color must be [r,g,b] array or ACI index number",
            )),
        }
    }
}

// ── cad_call dispatcher ────────────────────────────────────────────────

/// The ABI entry point. Routes method + JSON args to Document operations.
/// Returns JSON result string. All coordinates in the drawing's native
/// unit (mm for DWG files, arbitrary for new documents).
pub fn cad_call(doc: &mut Document, method: &str, args: &str) -> Result<String, String> {
    match method {
        "describe" => {
            let bounds = doc.bounds();
            let (min, max) = bounds.map_or(([0.0, 0.0], [0.0, 0.0]), |(x0, y0, x1, y1)| {
                ([x0, y0], [x1, y1])
            });
            let mut counts: HashMap<&str, usize> = HashMap::new();
            // Attribute keys in use, with how many entities carry each.
            let mut attr_keys: std::collections::BTreeMap<&str, usize> = Default::default();
            for e in &doc.entities {
                *counts
                    .entry(crate::select::shape_type(&e.shape))
                    .or_default() += 1;
                for k in e.attrs.keys() {
                    *attr_keys.entry(k.as_str()).or_default() += 1;
                }
            }
            let mut result = serde_json::json!({
                "bounds": { "min": min, "max": max },
                "entities": doc.entities.len(),
                "layers": doc.layers.iter().map(|l| &l.name).collect::<Vec<_>>(),
                "linetypes": doc.linetypes.len(),
                "counts": counts,
                "attrs": attr_keys,
            });
            // Solid parts: counts, attribute keys, the kernel (if any).
            if let (Some(m), serde_json::Value::Object(p)) = (
                result.as_object_mut(),
                crate::dispatch_3d::describe_parts(doc),
            ) {
                m.extend(p);
            }
            Ok(result.to_string())
        }

        "entities" => {
            // The whole argument object is a selection (see `select.rs`):
            // {expand?, layer?, type?, attrs?, has?, ids?, bounds?}.
            let v: serde_json::Value = if args.trim().is_empty() {
                serde_json::Value::Null
            } else {
                parse_args(args)?
            };
            let sel = crate::select::Selector::parse(&v)?;
            let mut result: Vec<EntityJson> = Vec::new();
            for ent in &doc.entities {
                if sel.expand {
                    // Children of block inserts, each matched on its own
                    // layer and attributes, whatever the insert's layer.
                    for (i, child) in crate::select::insert_children(doc, ent) {
                        if sel.matches(
                            child.id,
                            &child.layer,
                            crate::select::shape_type(&child.shape),
                            &child.attrs,
                            || child.shape.bbox(),
                        ) {
                            let mut j = child.to_json(doc);
                            j.child = Some(i);
                            result.push(j);
                        }
                    }
                }
                if sel.matches(
                    ent.id,
                    &ent.layer,
                    crate::select::shape_type(&ent.shape),
                    &ent.attrs,
                    || crate::select::entity_bbox(doc, ent),
                ) {
                    result.push(ent.to_json(doc));
                }
            }
            serde_json::to_string(&result).map_err(|e| e.to_string())
        }

        "children" => {
            #[derive(Deserialize)]
            struct Args {
                id: String,
            }
            let args: Args = parse_args(args)?;
            let id = parse_entity_id(&args.id)?;
            let ent = doc
                .entity(id)
                .ok_or(format!("entity {} not found", args.id))?;
            if let Shape::BlockInsert { block_name, .. } = &ent.shape {
                if !doc.blocks.contains_key(block_name) {
                    return Err(format!("block '{}' not defined", block_name));
                }
                let result: Vec<EntityJson> = crate::select::insert_children(doc, ent)
                    .into_iter()
                    .map(|(i, child)| {
                        let mut j = child.to_json(doc);
                        j.child = Some(i);
                        j
                    })
                    .collect();
                serde_json::to_string(&result).map_err(|e| e.to_string())
            } else {
                Ok("[]".to_string())
            }
        }

        "entity" => {
            #[derive(Deserialize)]
            struct Args {
                id: String,
            }
            let args: Args = parse_args(args)?;
            let id = parse_entity_id(&args.id)?;
            match doc.entity(id) {
                Some(e) => serde_json::to_string(&e.to_json(doc)).map_err(|e| e.to_string()),
                None => Err(format!("entity {} not found", args.id)),
            }
        }

        "addLine" => {
            #[derive(Deserialize)]
            struct Args {
                start: [f64; 2],
                end: [f64; 2],
                #[serde(default = "default_layer")]
                layer: String,
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
            let a: Args = parse_args(args)?;
            doc.ensure_layer(&a.layer);
            let color = a.color.map(ColorArg::resolve);
            let id = doc.add_line(
                Point::new(a.start[0], a.start[1]),
                Point::new(a.end[0], a.end[1]),
                &a.layer,
                color,
            );
            if let Some(ent) = doc.entity_mut(id) {
                ent.linetype = a.linetype;
                ent.lineweight = a.lineweight;
                ent.transparency = a.transparency.unwrap_or(0);
                ent.attrs = a.attrs;
            }
            let ent = doc.entity(id).expect("entity was just added");
            serde_json::to_string(&ent.to_json(doc)).map_err(|e| e.to_string())
        }

        "addCircle" => {
            #[derive(Deserialize)]
            struct Args {
                center: [f64; 2],
                radius: f64,
                #[serde(default = "default_layer")]
                layer: String,
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
            let a: Args = parse_args(args)?;
            doc.ensure_layer(&a.layer);
            let color = a.color.map(ColorArg::resolve);
            let id = doc.add_circle(
                Point::new(a.center[0], a.center[1]),
                a.radius,
                &a.layer,
                color,
            );
            if let Some(ent) = doc.entity_mut(id) {
                ent.linetype = a.linetype;
                ent.lineweight = a.lineweight;
                ent.transparency = a.transparency.unwrap_or(0);
                ent.attrs = a.attrs;
            }
            let ent = doc.entity(id).expect("entity was just added");
            serde_json::to_string(&ent.to_json(doc)).map_err(|e| e.to_string())
        }

        "addArc" => {
            #[derive(Deserialize)]
            struct Args {
                center: [f64; 2],
                radius: f64,
                #[serde(default)]
                from: f64,
                #[serde(default = "default_arc_to")]
                to: f64,
                #[serde(default)]
                shortest: bool,
                // Point-based alternative: pass p1/p2 instead of from/to
                #[serde(default)]
                p1: Option<[f64; 2]>,
                #[serde(default)]
                p2: Option<[f64; 2]>,
                #[serde(default = "default_layer")]
                layer: String,
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
            let a: Args = parse_args(args)?;
            doc.ensure_layer(&a.layer);
            let color = a.color.map(ColorArg::resolve);
            let center = Point::new(a.center[0], a.center[1]);

            let (mut from_rad, mut to_rad) = if let (Some(p1), Some(p2)) = (a.p1, a.p2) {
                // Point-based: compute angles from tangent points, always shortest
                let a1 = (p1[1] - a.center[1]).atan2(p1[0] - a.center[0]);
                let a2 = (p2[1] - a.center[1]).atan2(p2[0] - a.center[0]);
                // Pick the order that gives the short arc (CCW sweep < PI)
                let mut sweep = a2 - a1;
                if sweep < 0.0 {
                    sweep += 2.0 * PI;
                }
                if sweep > PI {
                    (a2, a1) // swap to get the short arc
                } else {
                    (a1, a2)
                }
            } else {
                (a.from.to_radians(), a.to.to_radians())
            };

            if a.shortest {
                let mut sweep = to_rad - from_rad;
                while sweep < 0.0 {
                    sweep += 2.0 * PI;
                }
                while sweep >= 2.0 * PI {
                    sweep -= 2.0 * PI;
                }
                if sweep > PI {
                    std::mem::swap(&mut from_rad, &mut to_rad);
                }
            }

            let id = doc.add_arc(center, a.radius, from_rad, to_rad, &a.layer, color);
            if let Some(ent) = doc.entity_mut(id) {
                ent.linetype = a.linetype;
                ent.lineweight = a.lineweight;
                ent.transparency = a.transparency.unwrap_or(0);
                ent.attrs = a.attrs;
            }
            let ent = doc.entity(id).expect("entity was just added");
            serde_json::to_string(&ent.to_json(doc)).map_err(|e| e.to_string())
        }

        "remove" => {
            #[derive(Deserialize)]
            struct Args {
                target: serde_json::Value,
            }
            let a: Args = parse_args(args)?;
            let ids = parse_target(doc, &a.target)?;
            let mut removed = Vec::new();
            for id in ids {
                if doc.remove_entity(id).is_some() {
                    removed.push(format!("e_{}", id.0));
                }
            }
            Ok(serde_json::json!({ "removed": removed }).to_string())
        }

        "move" => {
            #[derive(Deserialize)]
            struct Args {
                target: serde_json::Value,
                dx: f64,
                dy: f64,
            }
            let a: Args = parse_args(args)?;
            let ids = parse_target(doc, &a.target)?;
            let mut moved = Vec::new();
            for id in ids {
                if doc.move_entity(id, a.dx, a.dy) {
                    moved.push(format!("e_{}", id.0));
                }
            }
            Ok(serde_json::json!({ "moved": moved }).to_string())
        }

        "copy" => {
            #[derive(Deserialize)]
            struct Args {
                target: serde_json::Value,
                dx: f64,
                dy: f64,
            }
            let a: Args = parse_args(args)?;
            let ids = parse_target(doc, &a.target)?;
            let mut copied = Vec::new();
            for id in ids {
                if let Some(new_id) = doc.copy_entity(id, a.dx, a.dy) {
                    copied.push(
                        doc.entity(new_id)
                            .expect("entity was just added")
                            .to_json(doc),
                    );
                }
            }
            Ok(serde_json::json!(copied).to_string())
        }

        "rotate" => {
            #[derive(Deserialize)]
            struct Args {
                target: serde_json::Value,
                center: [f64; 2],
                angle: f64, // degrees, CCW
            }
            let a: Args = parse_args(args)?;
            let ids = parse_target(doc, &a.target)?;
            let center = Point::new(a.center[0], a.center[1]);
            let mut rotated = Vec::new();
            for id in ids {
                if doc.rotate_entity(id, center, a.angle) {
                    rotated.push(format!("e_{}", id.0));
                }
            }
            Ok(serde_json::json!({ "rotated": rotated }).to_string())
        }

        "mirror" => {
            #[derive(Deserialize)]
            struct Args {
                target: serde_json::Value,
                p1: [f64; 2],
                p2: [f64; 2],
            }
            let a: Args = parse_args(args)?;
            let ids = parse_target(doc, &a.target)?;
            let p1 = Point::new(a.p1[0], a.p1[1]);
            let p2 = Point::new(a.p2[0], a.p2[1]);
            let mut mirrored = Vec::new();
            for id in ids {
                if doc.mirror_entity(id, p1, p2) {
                    mirrored.push(format!("e_{}", id.0));
                }
            }
            Ok(serde_json::json!({ "mirrored": mirrored }).to_string())
        }

        "trim" => {
            #[derive(Deserialize)]
            struct Args {
                id: String,
                cut: [f64; 2],
                keep: String, // "start"/"from" or "end"/"to"
            }
            let a: Args = parse_args(args)?;
            let eid = parse_entity_id(&a.id)?;
            let cut = Point::new(a.cut[0], a.cut[1]);
            match doc.trim_entity(eid, cut, &a.keep) {
                Some(new_id) => {
                    let ent = doc.entity(new_id).expect("entity was just added");
                    serde_json::to_string(&ent.to_json(doc)).map_err(|e| e.to_string())
                }
                None => Err(format!("cannot trim entity {}", a.id)),
            }
        }

        "addLayer" => {
            #[derive(Deserialize)]
            struct Args {
                name: String,
                #[serde(default)]
                color: Option<ColorArg>,
                #[serde(default)]
                visible: Option<bool>,
                #[serde(default)]
                linetype: Option<String>,
                #[serde(default)]
                lineweight: Option<f64>,
            }
            let a: Args = parse_args(args)?;
            let color = a.color.map_or(Color::WHITE, ColorArg::resolve);
            doc.add_layer(&a.name, color);
            if let Some(false) = a.visible {
                if let Some(layer) = doc.layers.iter_mut().find(|l| l.name == a.name) {
                    layer.visible = false;
                }
            }
            if let Some(lt) = &a.linetype {
                if let Some(layer) = doc.layers.iter_mut().find(|l| l.name == a.name) {
                    layer.linetype = lt.clone();
                }
            }
            if let Some(lw) = a.lineweight {
                if let Some(layer) = doc.layers.iter_mut().find(|l| l.name == a.name) {
                    layer.lineweight = lw;
                }
            }
            Ok(serde_json::json!({
                "name": a.name,
                "color": [color.r, color.g, color.b],
                "linetype": doc.layers.iter().find(|l| l.name == a.name).map(|l| &l.linetype),
                "lineweight": doc.layers.iter().find(|l| l.name == a.name).map(|l| l.lineweight),
            })
            .to_string())
        }

        "checkpoint" => {
            doc.checkpoint();
            Ok(serde_json::json!({ "undoDepth": doc.undo_depth() }).to_string())
        }

        "undo" => {
            let ok = doc.undo();
            Ok(serde_json::json!({ "ok": ok, "entities": doc.entities.len() }).to_string())
        }

        "redo" => {
            let ok = doc.redo();
            Ok(serde_json::json!({ "ok": ok, "entities": doc.entities.len() }).to_string())
        }

        "offset" => {
            #[derive(Deserialize)]
            struct Args {
                id: String,
                distance: f64,
            }
            let a: Args = parse_args(args)?;
            let eid = parse_entity_id(&a.id)?;
            let ent = doc
                .entity(eid)
                .ok_or(format!("entity {} not found", a.id))?;
            let layer = ent.layer.clone();
            let src_attrs = ent.attrs.clone();
            let color = ent.color;
            let d = a.distance;

            let new_id = match &ent.shape {
                Shape::Line(line) => {
                    let (nx, ny) = geo::normal(line.p0, line.p1);
                    let p0 = Point::new(line.p0.x + nx * d, line.p0.y + ny * d);
                    let p1 = Point::new(line.p1.x + nx * d, line.p1.y + ny * d);
                    Some(doc.add_line(p0, p1, &layer, color))
                }
                Shape::Circle(circle) => {
                    let new_r = circle.radius + d;
                    if new_r > 0.0 {
                        Some(doc.add_circle(circle.center, new_r, &layer, color))
                    } else {
                        None
                    }
                }
                Shape::Arc {
                    center,
                    radius,
                    start_angle,
                    end_angle,
                    ..
                } => {
                    let new_r = radius + d;
                    if new_r > 0.0 {
                        Some(doc.add_arc(*center, new_r, *start_angle, *end_angle, &layer, color))
                    } else {
                        None
                    }
                }
                _ => None,
            };
            match new_id {
                Some(id) => {
                    if let Some(e) = doc.entity_mut(id) {
                        e.attrs = src_attrs;
                    }
                    let ent = doc.entity(id).expect("entity was just added");
                    serde_json::to_string(&ent.to_json(doc)).map_err(|e| e.to_string())
                }
                None => Err(format!("cannot offset entity {}", a.id)),
            }
        }

        // Load a DWG file and register all its entities as a named block definition.
        // Server-only: requires filesystem access to read the DWG file.
        "loadDwgAsBlock" => {
            #[derive(Deserialize)]
            struct Args {
                path: String,
                name: String,
                #[serde(default, rename = "insertPoint")]
                insert_point: Option<[f64; 2]>,
                #[serde(default = "default_layer", rename = "defaultLayer")]
                default_layer: String,
            }
            let a: Args = parse_args(args)?;
            let tmp_doc = load_dwg(&a.path).map_err(|e| format!("loadDwgAsBlock: {e}"))?;
            let shapes: Vec<(Shape, String, Option<Color>)> = tmp_doc
                .entities
                .into_iter()
                .map(|e| (e.shape, e.layer, e.color))
                .collect();
            let count = shapes.len();
            let ip = a.insert_point.unwrap_or([0.0, 0.0]);
            doc.define_block(BlockDef {
                attrs: Default::default(),
                shape_attrs: Vec::new(),
                name: a.name.clone(),
                shapes,
                insert_point: Point::new(ip[0], ip[1]),
                default_layer: a.default_layer,
            });
            Ok(serde_json::json!({ "name": a.name, "shapeCount": count }).to_string())
        }

        // Load a QElectroTech .elmt file as a block definition.
        // anchor: "terminal" (first terminal) or "hotspot" (default, element origin)
        "loadElmt" => {
            #[derive(Deserialize)]
            struct Args {
                path: String,
                #[serde(default)]
                name: Option<String>,
                #[serde(default = "default_layer", rename = "defaultLayer")]
                default_layer: String,
                #[serde(default)]
                anchor: Option<String>,
            }
            let a: Args = parse_args(args)?;
            let mut sym = crate::elmt::load_elmt(&a.path).map_err(|e| format!("loadElmt: {e}"))?;
            if let Some(name) = a.name {
                sym.block.name = name;
            }
            if !a.default_layer.is_empty() {
                sym.block.default_layer = a.default_layer;
            }
            // Set anchor point based on mode
            match a.anchor.as_deref() {
                Some("terminal") => {
                    if let Some(t) = sym.terminals.first() {
                        sym.block.insert_point = t.position;
                    }
                }
                Some("center") => {
                    // Compute bounding box center of all shapes
                    let mut min_x = f64::INFINITY;
                    let mut min_y = f64::INFINITY;
                    let mut max_x = f64::NEG_INFINITY;
                    let mut max_y = f64::NEG_INFINITY;
                    for (shape, _, _) in &sym.block.shapes {
                        let (x0, y0, x1, y1) = shape.bbox();
                        // shape bbox used for center computation
                        if x0 < min_x {
                            min_x = x0;
                        }
                        if y0 < min_y {
                            min_y = y0;
                        }
                        if x1 > max_x {
                            max_x = x1;
                        }
                        if y1 > max_y {
                            max_y = y1;
                        }
                    }
                    if min_x.is_finite() {
                        sym.block.insert_point =
                            Point::new((min_x + max_x) / 2.0, (min_y + max_y) / 2.0);
                    }
                }
                _ => {} // default: hotspot (already at origin)
            }
            let block_name = sym.block.name.clone();
            let shape_count = sym.block.shapes.len();
            let insert_pt = sym.block.insert_point;
            doc.define_block(sym.block);
            Ok(serde_json::json!({
                "name": block_name,
                "shapeCount": shape_count,
                "insertPoint": [insert_pt.x, insert_pt.y],
                "terminals": sym.terminals.iter().map(|t| {
                    serde_json::json!({
                        "x": t.position.x, "y": t.position.y,
                        "orientation": t.orientation,
                    })
                }).collect::<Vec<_>>(),
                "en": sym.en_standard,
            })
            .to_string())
        }

        // Load all .elmt files from a directory as block definitions.
        "loadElmtDir" => {
            #[derive(Deserialize)]
            struct Args {
                path: String,
                #[serde(default = "default_layer", rename = "defaultLayer")]
                default_layer: String,
            }
            let a: Args = parse_args(args)?;
            let symbols =
                crate::elmt::load_elmt_dir(&a.path).map_err(|e| format!("loadElmtDir: {e}"))?;
            let mut loaded = Vec::new();
            for mut sym in symbols {
                if !a.default_layer.is_empty() {
                    sym.block.default_layer = a.default_layer.clone();
                }
                let name = sym.block.name.clone();
                let count = sym.block.shapes.len();
                doc.define_block(sym.block);
                loaded.push(serde_json::json!({
                    "name": name,
                    "shapeCount": count,
                }));
            }
            Ok(serde_json::json!({ "loaded": loaded }).to_string())
        }

        // Clone a block (or entity) definition under a new name, with optional
        // text replacements. Useful for creating socket count variants (SOCKET_3 -> SOCKET_4).
        "clone" => {
            #[derive(Deserialize)]
            struct Args {
                source: String,
                name: String,
                #[serde(default, rename = "replaceText")]
                replace_text: Option<HashMap<String, String>>,
            }
            let a: Args = parse_args(args)?;
            let source_def = doc
                .blocks
                .get(&a.source)
                .ok_or_else(|| format!("block '{}' not defined", a.source))?
                .clone();
            let mut new_def = BlockDef {
                attrs: source_def.attrs.clone(),
                shape_attrs: source_def.shape_attrs.clone(),
                name: a.name.clone(),
                shapes: source_def.shapes.clone(),
                insert_point: source_def.insert_point,
                default_layer: source_def.default_layer.clone(),
            };
            // Apply text replacements across all Text and MText shapes
            if let Some(replacements) = &a.replace_text {
                for (shape, _, _) in &mut new_def.shapes {
                    match shape {
                        Shape::Text { text, .. } => {
                            for (from, to) in replacements {
                                if text.contains(from.as_str()) {
                                    *text = text.replace(from.as_str(), to.as_str());
                                }
                            }
                        }
                        Shape::MText {
                            text, plain_text, ..
                        } => {
                            for (from, to) in replacements {
                                if text.contains(from.as_str()) {
                                    *text = text.replace(from.as_str(), to.as_str());
                                }
                                if plain_text.contains(from.as_str()) {
                                    *plain_text = plain_text.replace(from.as_str(), to.as_str());
                                }
                            }
                        }
                        _ => {}
                    }
                }
            }
            let shape_count = new_def.shapes.len();
            doc.define_block(new_def);
            Ok(serde_json::json!({
                "name": a.name,
                "clonedFrom": a.source,
                "shapeCount": shape_count,
            })
            .to_string())
        }

        "connectedTo" => {
            #[derive(Deserialize)]
            struct Args {
                id: String,
                #[serde(default = "default_tolerance")]
                tolerance: f64,
            }
            let a: Args = parse_args(args)?;
            let eid = parse_entity_id(&a.id)?;
            let ent = doc
                .entity(eid)
                .ok_or(format!("entity {} not found", a.id))?;
            let tol = a.tolerance;

            // Collect endpoints of the source entity
            let src_pts = entity_endpoints(&ent.shape);
            if src_pts.is_empty() {
                return Ok("[]".to_string());
            }

            // Find all other entities sharing an endpoint within tolerance
            let mut connected: Vec<EntityJson> = Vec::new();
            for other in &doc.entities {
                if other.id == eid {
                    continue;
                }
                let other_pts = entity_endpoints(&other.shape);
                let shares = src_pts
                    .iter()
                    .any(|sp| other_pts.iter().any(|op| geo::distance(*sp, *op) < tol));
                if shares {
                    connected.push(other.to_json(doc));
                }
            }
            serde_json::to_string(&connected).map_err(|e| e.to_string())
        }

        // Geometry helpers (pure queries, no mutation)
        "distance" => {
            #[derive(Deserialize)]
            struct Args {
                a: [f64; 2],
                b: [f64; 2],
            }
            let a: Args = parse_args(args)?;
            let d = geo::distance(Point::new(a.a[0], a.a[1]), Point::new(a.b[0], a.b[1]));
            Ok(serde_json::json!(d).to_string())
        }

        "midpoint" => {
            #[derive(Deserialize)]
            struct Args {
                a: [f64; 2],
                b: [f64; 2],
            }
            let a: Args = parse_args(args)?;
            let m = geo::midpoint(Point::new(a.a[0], a.a[1]), Point::new(a.b[0], a.b[1]));
            Ok(serde_json::json!([m.x, m.y]).to_string())
        }

        "direction" => {
            #[derive(Deserialize)]
            struct Args {
                a: [f64; 2],
                b: [f64; 2],
            }
            let a: Args = parse_args(args)?;
            let d = geo::direction(Point::new(a.a[0], a.a[1]), Point::new(a.b[0], a.b[1]));
            Ok(serde_json::json!(d).to_string())
        }

        "lineCircleIntersection" => {
            #[derive(Deserialize)]
            struct Args {
                line: [[f64; 2]; 2],
                center: [f64; 2],
                radius: f64,
            }
            let a: Args = parse_args(args)?;
            let pts = geo::line_circle_intersection(
                Point::new(a.line[0][0], a.line[0][1]),
                Point::new(a.line[1][0], a.line[1][1]),
                Point::new(a.center[0], a.center[1]),
                a.radius,
            );
            let result: Vec<[f64; 2]> = pts.iter().map(|p| [p.x, p.y]).collect();
            Ok(serde_json::json!(result).to_string())
        }

        "circleCircleIntersection" => {
            #[derive(Deserialize)]
            struct Args {
                c1: [f64; 2],
                r1: f64,
                c2: [f64; 2],
                r2: f64,
            }
            let a: Args = parse_args(args)?;
            let pts = geo::circle_circle_intersection(
                Point::new(a.c1[0], a.c1[1]),
                a.r1,
                Point::new(a.c2[0], a.c2[1]),
                a.r2,
            );
            let result: Vec<[f64; 2]> = pts.iter().map(|p| [p.x, p.y]).collect();
            Ok(serde_json::json!(result).to_string())
        }

        "projectOntoCircle" => {
            #[derive(Deserialize)]
            struct Args {
                point: [f64; 2],
                center: [f64; 2],
                radius: f64,
            }
            let a: Args = parse_args(args)?;
            let p = geo::project_onto_circle(
                Point::new(a.point[0], a.point[1]),
                Point::new(a.center[0], a.center[1]),
                a.radius,
            );
            Ok(serde_json::json!([p.x, p.y]).to_string())
        }

        "angleOf" => {
            #[derive(Deserialize)]
            struct Args {
                point: [f64; 2],
                center: [f64; 2],
            }
            let a: Args = parse_args(args)?;
            let deg = geo::angle_of(
                Point::new(a.point[0], a.point[1]),
                Point::new(a.center[0], a.center[1]),
            );
            Ok(serde_json::json!(deg).to_string())
        }

        "projectOnto" => {
            #[derive(Deserialize)]
            struct Args {
                point: [f64; 2],
                line: [[f64; 2]; 2],
            }
            let a: Args = parse_args(args)?;
            let p = geo::project_onto(
                Point::new(a.point[0], a.point[1]),
                Point::new(a.line[0][0], a.line[0][1]),
                Point::new(a.line[1][0], a.line[1][1]),
            );
            Ok(serde_json::json!([p.x, p.y]).to_string())
        }

        "addLinetype" => {
            #[derive(Deserialize)]
            struct Args {
                name: String,
                pattern: Vec<f64>,
                #[serde(default)]
                description: Option<String>,
            }
            let a: Args = parse_args(args)?;
            doc.linetypes.insert(
                a.name.clone(),
                crate::types::LinetypeDef {
                    name: a.name.clone(),
                    description: a.description.unwrap_or_default(),
                    pattern: a.pattern.clone(),
                },
            );
            Ok(serde_json::json!({"name": a.name, "pattern": a.pattern}).to_string())
        }

        "linetypes" => {
            let lts: Vec<serde_json::Value> = doc
                .linetypes
                .values()
                .map(|lt| {
                    serde_json::json!({
                        "name": lt.name,
                        "description": lt.description,
                        "pattern": lt.pattern,
                    })
                })
                .collect();
            serde_json::to_string(&lts).map_err(|e| e.to_string())
        }

        "aciColors" => {
            let colors: Vec<serde_json::Value> = (1u16..=255)
                .map(|i| {
                    let (r, g, b) = acadrust::types::ACI_COLORS[i as usize];
                    serde_json::json!({"index": i, "color": [r, g, b]})
                })
                .collect();
            serde_json::to_string(&colors).map_err(|e| e.to_string())
        }

        _ => crate::dispatch_ext::cad_call_ext(doc, method, args)
            .unwrap_or_else(|| Err(format!("unknown method: {method}"))),
    }
}

pub(crate) fn parse_args<T: serde::de::DeserializeOwned>(args: &str) -> Result<T, String> {
    serde_json::from_str(args).map_err(|e| format!("bad args: {e}"))
}

pub(crate) fn parse_entity_id(s: &str) -> Result<EntityId, String> {
    let num = s.strip_prefix("e_").unwrap_or(s);
    num.parse::<u64>()
        .map(EntityId)
        .map_err(|_| format!("bad entity id: {s}"))
}

/// Parse a target that can be a single ID string, array of ID strings,
/// array of entity objects (with .id field), or a selector object
/// (`{layer, attrs, has, type, bounds, ids}`, see `select.rs`).
pub(crate) fn parse_target(
    doc: &Document,
    val: &serde_json::Value,
) -> Result<Vec<EntityId>, String> {
    if crate::select::is_selector_object(val) {
        let sel = crate::select::Selector::parse(val)?;
        return crate::select::select_ids(doc, &sel);
    }
    match val {
        serde_json::Value::String(s) => Ok(vec![parse_entity_id(s)?]),
        serde_json::Value::Array(arr) => arr
            .iter()
            .map(|v| match v {
                serde_json::Value::String(s) => parse_entity_id(s),
                serde_json::Value::Object(obj) => obj
                    .get("id")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| "entity object missing 'id' field".to_string())
                    .and_then(parse_entity_id),
                _ => Err("target array elements must be strings or objects".to_string()),
            })
            .collect(),
        _ => Err(
            "target must be a string, array of strings, array of entity objects, or a selector"
                .to_string(),
        ),
    }
}

/// Split a BezPath with multiple MoveTo's into separate subpaths.
pub fn split_subpaths(path: &BezPath) -> Vec<BezPath> {
    let mut subpaths = Vec::new();
    let mut current = BezPath::new();
    for el in path.elements() {
        if matches!(el, kurbo::PathEl::MoveTo(_)) && !current.elements().is_empty() {
            subpaths.push(std::mem::replace(&mut current, BezPath::new()));
        }
        current.push(*el);
    }
    if !current.elements().is_empty() {
        subpaths.push(current);
    }
    subpaths
}

pub(crate) fn default_layer() -> String {
    "0".to_string()
}
fn default_arc_to() -> f64 {
    360.0
}
fn default_tolerance() -> f64 {
    0.01
}
