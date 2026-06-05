use kurbo::{BezPath, Circle, Line, Point};
use serde::Deserialize;
use std::collections::HashMap;
use std::f64::consts::PI;
use crate::types::*;
use crate::document::*;
use crate::hatch::{clip_line_to_polygon, entity_endpoints};
use crate::dwg::load_dwg;
use crate::geo;
use crate::text;

// ── cad_call dispatcher ────────────────────────────────────────────────

/// The ABI entry point. Routes method + JSON args to Document operations.
/// Returns JSON result string. All coordinates in the drawing's native
/// unit (mm for DWG files, arbitrary for new documents).
pub fn cad_call(doc: &mut Document, method: &str, args: &str) -> Result<String, String> {
    match method {
        "describe" => {
            let bounds = doc.bounds();
            let (min, max) = bounds.map_or(
                ([0.0, 0.0], [0.0, 0.0]),
                |(x0, y0, x1, y1)| ([x0, y0], [x1, y1]),
            );
            let mut counts: HashMap<&str, usize> = HashMap::new();
            for e in &doc.entities {
                let t = match &e.shape {
                    Shape::Line(_) => "line",
                    Shape::Circle(_) => "circle",
                    Shape::Arc { .. } => "arc",
                    Shape::Polyline { .. } => "polyline",
                    Shape::LwPolyline { .. } => "lwpolyline",
                    Shape::SolidFill { .. } => "solid_fill",
                    Shape::CurvePath { .. } => "curve_path",
                    Shape::Ellipse { .. } => "ellipse",
                    Shape::Spline { .. } => "spline",
                    Shape::BlockInsert { .. } => "block_insert",
                    Shape::Text { .. } => "text",
                    Shape::MText { .. } => "mtext",
                };
                *counts.entry(t).or_default() += 1;
            }
            let result = serde_json::json!({
                "bounds": { "min": min, "max": max },
                "entities": doc.entities.len(),
                "layers": doc.layers.iter().map(|l| &l.name).collect::<Vec<_>>(),
                "counts": counts,
            });
            Ok(result.to_string())
        }

        "entities" => {
            #[derive(Deserialize)]
            struct EntArgs {
                #[serde(default)]
                expand: bool,
                #[serde(default)]
                layer: Option<String>,
            }
            let ea: EntArgs = serde_json::from_str(args).unwrap_or(EntArgs { expand: false, layer: None });
            if ea.expand {
                // Flatten block inserts into sub-entities (world coords)
                // Iterate all entities so block inserts on other layers still expand
                let mut result: Vec<EntityJson> = Vec::new();
                for ent in &doc.entities {
                    if let Shape::BlockInsert { block_name, position, rotation, x_scale, y_scale } = &ent.shape {
                        if let Some(def) = doc.blocks.get(block_name) {
                            let xform = kurbo::Affine::translate((position.x, position.y))
                                * kurbo::Affine::rotate(*rotation)
                                * kurbo::Affine::scale_non_uniform(*x_scale, *y_scale)
                                * kurbo::Affine::translate((-def.insert_point.x, -def.insert_point.y));
                            for (shape, shape_layer, shape_color) in &def.shapes {
                                let layer = if shape_layer.is_empty() { &ent.layer } else { shape_layer };
                                let color = if *shape_color == Color::WHITE && ent.color != Color::WHITE {
                                    ent.color
                                } else {
                                    *shape_color
                                };
                                if let Some(ref filter_layer) = ea.layer {
                                    if layer != filter_layer { continue; }
                                }
                                let expanded = DrawEntity {
                                    id: ent.id,
                                    layer: layer.to_string(),
                                    color,
                                    shape: shape.transformed(xform),
                                };
                                result.push(expanded.to_json());
                            }
                        }
                    }
                    // Include the entity itself if it passes the layer filter
                    if ea.layer.as_ref().is_none_or(|l| ent.layer == *l) {
                        result.push(ent.to_json());
                    }
                }
                serde_json::to_string(&result).map_err(|e| e.to_string())
            } else {
                let ents: Vec<EntityJson> = if let Some(ref layer) = ea.layer {
                    doc.entities.iter().filter(|e| e.layer == *layer).map(|e| e.to_json()).collect()
                } else {
                    doc.entities.iter().map(|e| e.to_json()).collect()
                };
                serde_json::to_string(&ents).map_err(|e| e.to_string())
            }
        }

        "children" => {
            #[derive(Deserialize)]
            struct Args { id: String }
            let args: Args = parse_args(args)?;
            let id = parse_entity_id(&args.id)?;
            let ent = doc.entity(id).ok_or(format!("entity {} not found", args.id))?;
            if let Shape::BlockInsert { block_name, position, rotation, x_scale, y_scale } = &ent.shape {
                let layer = ent.layer.clone();
                let ent_color = ent.color;
                let def = doc.blocks.get(block_name)
                    .ok_or(format!("block '{}' not defined", block_name))?;
                let xform = kurbo::Affine::translate((position.x, position.y))
                    * kurbo::Affine::rotate(*rotation)
                    * kurbo::Affine::scale_non_uniform(*x_scale, *y_scale)
                    * kurbo::Affine::translate((-def.insert_point.x, -def.insert_point.y));
                let mut result: Vec<EntityJson> = Vec::new();
                for (shape, shape_layer, shape_color) in &def.shapes {
                    let child_layer = if shape_layer.is_empty() { &layer } else { shape_layer };
                    let color = if *shape_color == Color::WHITE && ent_color != Color::WHITE {
                        ent_color
                    } else {
                        *shape_color
                    };
                    let expanded = DrawEntity {
                        id,
                        layer: child_layer.to_string(),
                        color,
                        shape: shape.transformed(xform),
                    };
                    result.push(expanded.to_json());
                }
                serde_json::to_string(&result).map_err(|e| e.to_string())
            } else {
                Ok("[]".to_string())
            }
        }

        "entity" => {
            #[derive(Deserialize)]
            struct Args { id: String }
            let args: Args = parse_args(args)?;
            let id = parse_entity_id(&args.id)?;
            match doc.entity(id) {
                Some(e) => serde_json::to_string(&e.to_json()).map_err(|e| e.to_string()),
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
                color: Option<[u8; 3]>,
            }
            let a: Args = parse_args(args)?;
            doc.ensure_layer(&a.layer);
            let color = a.color.map_or_else(|| doc.layer_color(&a.layer), |c| Color::rgb(c[0], c[1], c[2]));
            let id = doc.add_line(
                Point::new(a.start[0], a.start[1]),
                Point::new(a.end[0], a.end[1]),
                &a.layer,
                color,
            );
            let ent = doc.entity(id).expect("entity was just added");
            serde_json::to_string(&ent.to_json()).map_err(|e| e.to_string())
        }

        "addCircle" => {
            #[derive(Deserialize)]
            struct Args {
                center: [f64; 2],
                radius: f64,
                #[serde(default = "default_layer")]
                layer: String,
                #[serde(default)]
                color: Option<[u8; 3]>,
            }
            let a: Args = parse_args(args)?;
            doc.ensure_layer(&a.layer);
            let color = a.color.map_or_else(|| doc.layer_color(&a.layer), |c| Color::rgb(c[0], c[1], c[2]));
            let id = doc.add_circle(
                Point::new(a.center[0], a.center[1]),
                a.radius,
                &a.layer,
                color,
            );
            let ent = doc.entity(id).expect("entity was just added");
            serde_json::to_string(&ent.to_json()).map_err(|e| e.to_string())
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
                color: Option<[u8; 3]>,
            }
            let a: Args = parse_args(args)?;
            doc.ensure_layer(&a.layer);
            let color = a.color.map_or_else(|| doc.layer_color(&a.layer), |c| Color::rgb(c[0], c[1], c[2]));
            let center = Point::new(a.center[0], a.center[1]);

            let (mut from_rad, mut to_rad) = if let (Some(p1), Some(p2)) = (a.p1, a.p2) {
                // Point-based: compute angles from tangent points, always shortest
                let a1 = (p1[1] - a.center[1]).atan2(p1[0] - a.center[0]);
                let a2 = (p2[1] - a.center[1]).atan2(p2[0] - a.center[0]);
                // Pick the order that gives the short arc (CCW sweep < PI)
                let mut sweep = a2 - a1;
                if sweep < 0.0 { sweep += 2.0 * PI; }
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
                while sweep < 0.0 { sweep += 2.0 * PI; }
                while sweep >= 2.0 * PI { sweep -= 2.0 * PI; }
                if sweep > PI {
                    std::mem::swap(&mut from_rad, &mut to_rad);
                }
            }

            let id = doc.add_arc(center, a.radius, from_rad, to_rad, &a.layer, color);
            let ent = doc.entity(id).expect("entity was just added");
            serde_json::to_string(&ent.to_json()).map_err(|e| e.to_string())
        }

        "addPolyline" => {
            #[derive(Deserialize)]
            struct Args {
                points: Vec<[f64; 2]>,
                #[serde(default)]
                closed: bool,
                #[serde(default = "default_layer")]
                layer: String,
                #[serde(default)]
                color: Option<[u8; 3]>,
            }
            let a: Args = parse_args(args)?;
            doc.ensure_layer(&a.layer);
            let color = a.color.map_or_else(|| doc.layer_color(&a.layer), |c| Color::rgb(c[0], c[1], c[2]));
            let pts: Vec<Point> = a.points.iter().map(|p| Point::new(p[0], p[1])).collect();
            let id = doc.add_polyline(pts, a.closed, &a.layer, color);
            let ent = doc.entity(id).expect("entity was just added");
            serde_json::to_string(&ent.to_json()).map_err(|e| e.to_string())
        }

        "remove" => {
            #[derive(Deserialize)]
            struct Args { target: serde_json::Value }
            let a: Args = parse_args(args)?;
            let ids = parse_target(&a.target)?;
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
            let ids = parse_target(&a.target)?;
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
            let ids = parse_target(&a.target)?;
            let mut copied = Vec::new();
            for id in ids {
                if let Some(new_id) = doc.copy_entity(id, a.dx, a.dy) {
                    copied.push(doc.entity(new_id).expect("entity was just added").to_json());
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
            let ids = parse_target(&a.target)?;
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
            let ids = parse_target(&a.target)?;
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
                keep: String,  // "start"/"from" or "end"/"to"
            }
            let a: Args = parse_args(args)?;
            let eid = parse_entity_id(&a.id)?;
            let cut = Point::new(a.cut[0], a.cut[1]);
            match doc.trim_entity(eid, cut, &a.keep) {
                Some(new_id) => {
                    let ent = doc.entity(new_id).expect("entity was just added");
                    serde_json::to_string(&ent.to_json()).map_err(|e| e.to_string())
                }
                None => Err(format!("cannot trim entity {}", a.id)),
            }
        }

        "addLayer" => {
            #[derive(Deserialize)]
            struct Args {
                name: String,
                #[serde(default)]
                color: Option<[u8; 3]>,
                #[serde(default)]
                visible: Option<bool>,
            }
            let a: Args = parse_args(args)?;
            let color = a.color.map_or(Color::WHITE, |c| Color::rgb(c[0], c[1], c[2]));
            doc.add_layer(&a.name, color);
            if let Some(false) = a.visible {
                if let Some(layer) = doc.layers.iter_mut().find(|l| l.name == a.name) {
                    layer.visible = false;
                }
            }
            Ok(serde_json::json!({ "name": a.name, "color": [color.r, color.g, color.b] }).to_string())
        }

        "clear" => {
            doc.checkpoint();
            doc.entities.clear();
            doc.layers.clear();
            Ok(r#"{"ok":true}"#.to_string())
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

        "addText" => {
            #[derive(Deserialize)]
            struct Args {
                text: String,
                at: [f64; 2],
                #[serde(default = "default_text_height")]
                height: f64,
                #[serde(default = "default_layer")]
                layer: String,
                #[serde(default)]
                color: Option<[u8; 3]>,
            }
            let a: Args = parse_args(args)?;
            doc.ensure_layer(&a.layer);
            let color = a.color.map_or_else(|| doc.layer_color(&a.layer), |c| Color::rgb(c[0], c[1], c[2]));

            let id = doc.alloc_id();
            doc.entities.push(DrawEntity {
                id,
                layer: a.layer.clone(),
                color,
                shape: Shape::Text {
                    text: a.text.clone(),
                    position: kurbo::Point::new(a.at[0], a.at[1]),
                    height: a.height,
                    rotation: 0.0,
                },
            });

            let width = text::text_width(&a.text, a.height);
            Ok(serde_json::json!({
                "id": format!("e_{}", id.0),
                "width": width,
                "height": a.height,
                "at": a.at,
            }).to_string())
        }

        "measure" => {
            // Dimension line between two points: extension lines, arrows, measurement text
            #[derive(Deserialize)]
            struct Args {
                from: [f64; 2],
                to: [f64; 2],
                #[serde(default = "default_measure_offset")]
                offset: f64,         // perpendicular offset for the dimension line
                #[serde(default = "default_text_height")]
                text_height: f64,
                #[serde(default = "default_layer")]
                layer: String,
                #[serde(default)]
                color: Option<[u8; 3]>,
            }
            let a: Args = parse_args(args)?;
            doc.ensure_layer(&a.layer);
            let color = a.color.map_or_else(|| doc.layer_color(&a.layer), |c| Color::rgb(c[0], c[1], c[2]));

            let from = Point::new(a.from[0], a.from[1]);
            let to = Point::new(a.to[0], a.to[1]);
            let dist = geo::distance(from, to);
            let (nx, ny) = geo::normal(from, to);
            let offset = a.offset;
            let arrow_size = a.text_height * 0.8;

            // Dimension line endpoints (offset from the measured points)
            let d0 = Point::new(from.x + nx * offset, from.y + ny * offset);
            let d1 = Point::new(to.x + nx * offset, to.y + ny * offset);

            // Direction along the dimension line
            let dir_x = (d1.x - d0.x) / dist;
            let dir_y = (d1.y - d0.y) / dist;

            let mut ids = Vec::new();

            // Extension lines (stilts): from measured point to dimension line, with small gap
            let gap = a.text_height * 0.3;
            let ext_start_offset = if offset > 0.0 { gap } else { -gap };
            let ext_end_overshoot = if offset > 0.0 { gap * 0.5 } else { -gap * 0.5 };
            ids.push(doc.add_line(
                Point::new(from.x + nx * ext_start_offset, from.y + ny * ext_start_offset),
                Point::new(d0.x + nx * ext_end_overshoot, d0.y + ny * ext_end_overshoot),
                &a.layer, color,
            ));
            ids.push(doc.add_line(
                Point::new(to.x + nx * ext_start_offset, to.y + ny * ext_start_offset),
                Point::new(d1.x + nx * ext_end_overshoot, d1.y + ny * ext_end_overshoot),
                &a.layer, color,
            ));

            // Dimension line (from d0 to d1)
            ids.push(doc.add_line(d0, d1, &a.layer, color));

            // Arrowheads (small triangles at each end)
            // Arrow at d0, pointing toward d1
            let a0_tip = d0;
            let a0_back = Point::new(d0.x + dir_x * arrow_size, d0.y + dir_y * arrow_size);
            let a0_left = Point::new(a0_back.x + nx * arrow_size * 0.3, a0_back.y + ny * arrow_size * 0.3);
            let a0_right = Point::new(a0_back.x - nx * arrow_size * 0.3, a0_back.y - ny * arrow_size * 0.3);
            ids.push(doc.add_line(a0_tip, a0_left, &a.layer, color));
            ids.push(doc.add_line(a0_tip, a0_right, &a.layer, color));

            // Arrow at d1, pointing toward d0
            let a1_tip = d1;
            let a1_back = Point::new(d1.x - dir_x * arrow_size, d1.y - dir_y * arrow_size);
            let a1_left = Point::new(a1_back.x + nx * arrow_size * 0.3, a1_back.y + ny * arrow_size * 0.3);
            let a1_right = Point::new(a1_back.x - nx * arrow_size * 0.3, a1_back.y - ny * arrow_size * 0.3);
            ids.push(doc.add_line(a1_tip, a1_left, &a.layer, color));
            ids.push(doc.add_line(a1_tip, a1_right, &a.layer, color));

            // Measurement text centered on the dimension line, rotated to follow it
            let label = format!("{:.1}", dist);
            let text_w = text::text_width(&label, a.text_height);
            let mid = geo::midpoint(d0, d1);
            // Text rotation = angle of the dimension line
            let text_rot = dir_y.atan2(dir_x);
            // Anchor: centered along the line, offset perpendicular
            let text_x = mid.x - (text_w / 2.0) * dir_x + nx * a.text_height * 0.3;
            let text_y = mid.y - (text_w / 2.0) * dir_y + ny * a.text_height * 0.3;

            let glyph_paths = text::text_to_paths_rotated(&label, text_x, text_y, a.text_height, text_rot);
            for id in add_glyph_paths(doc, &glyph_paths, &a.layer, color) {
                ids.push(id);
            }

            Ok(serde_json::json!({
                "distance": dist,
                "ids": ids.iter().map(|id| format!("e_{}", id.0)).collect::<Vec<_>>(),
            }).to_string())
        }

        "addHatch" => {
            #[derive(Deserialize)]
            struct Args {
                boundary: Vec<[f64; 2]>,
                #[serde(default = "default_hatch_angle")]
                angle: f64,      // degrees
                #[serde(default = "default_hatch_spacing")]
                spacing: f64,    // distance between lines
                #[serde(default = "default_layer")]
                layer: String,
                #[serde(default)]
                color: Option<[u8; 3]>,
            }
            let a: Args = parse_args(args)?;
            doc.ensure_layer(&a.layer);
            let color = a.color.map_or_else(|| doc.layer_color(&a.layer), |c| Color::rgb(c[0], c[1], c[2]));
            let boundary: Vec<Point> = a.boundary.iter().map(|p| Point::new(p[0], p[1])).collect();
            if boundary.len() < 3 {
                return Err("hatch boundary needs at least 3 points".to_string());
            }

            let ang_rad = a.angle.to_radians();
            let cos_a = ang_rad.cos();
            let sin_a = ang_rad.sin();

            // Project boundary onto the hatch direction perpendicular
            // to find the range of parallel lines needed
            let perp_dists: Vec<f64> = boundary.iter()
                .map(|p| -p.x * sin_a + p.y * cos_a)
                .collect();
            let min_d = perp_dists.iter().cloned().fold(f64::MAX, f64::min);
            let max_d = perp_dists.iter().cloned().fold(f64::MIN, f64::max);

            // Bounding box for line length
            let (bx0, by0, bx1, by1) = geo::bounds_of(&boundary);
            let diag = ((bx1-bx0).powi(2) + (by1-by0).powi(2)).sqrt();

            let mut ids = Vec::new();
            let mut d = min_d + a.spacing;
            while d < max_d {
                // Base point at perpendicular distance d from origin
                // Normal to hatch direction: (-sin_a, cos_a)
                let base = Point::new(-d * sin_a, d * cos_a);
                let dir = Point::new(cos_a, sin_a);
                let p0 = Point::new(base.x - diag * dir.x, base.y - diag * dir.y);
                let p1 = Point::new(base.x + diag * dir.x, base.y + diag * dir.y);

                // Clip this line to the boundary polygon
                let clipped = clip_line_to_polygon(p0, p1, &boundary);
                for (ca, cb) in clipped {
                    let id = doc.add_line(ca, cb, &a.layer, color);
                    ids.push(format!("e_{}", id.0));
                }
                d += a.spacing;
            }
            Ok(serde_json::json!({ "lines": ids.len(), "ids": ids }).to_string())
        }

        "offset" => {
            #[derive(Deserialize)]
            struct Args {
                id: String,
                distance: f64,
            }
            let a: Args = parse_args(args)?;
            let eid = parse_entity_id(&a.id)?;
            let ent = doc.entity(eid).ok_or(format!("entity {} not found", a.id))?;
            let layer = ent.layer.clone();
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
                Shape::Arc { center, radius, start_angle, end_angle, .. } => {
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
                    let ent = doc.entity(id).expect("entity was just added");
                    serde_json::to_string(&ent.to_json()).map_err(|e| e.to_string())
                }
                None => Err(format!("cannot offset entity {}", a.id)),
            }
        }

        "defineBlock" => {
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
                points: Option<Vec<[f64; 2]>>,
                #[serde(default)]
                closed: Option<bool>,
                #[serde(default)]
                layer: Option<String>,
                #[serde(default)]
                color: Option<[u8; 3]>,
            }
            #[derive(Deserialize)]
            struct Args {
                name: String,
                shapes: Vec<ShapeDef>,
                #[serde(default)]
                insert_point: Option<[f64; 2]>,
                #[serde(default = "default_layer")]
                default_layer: String,
            }
            let a: Args = parse_args(args)?;
            let mut shapes = Vec::new();
            for sd in &a.shapes {
                let color = sd.color.map_or(Color::WHITE, |c| Color::rgb(c[0], c[1], c[2]));
                let layer = sd.layer.clone().unwrap_or_default();
                let shape = match sd.shape_type.as_str() {
                    "line" => {
                        let s = sd.start.ok_or("line needs start")?;
                        let e = sd.end.ok_or("line needs end")?;
                        Shape::Line(Line::new(Point::new(s[0], s[1]), Point::new(e[0], e[1])))
                    }
                    "circle" => {
                        let c = sd.center.ok_or("circle needs center")?;
                        let r = sd.radius.ok_or("circle needs radius")?;
                        Shape::Circle(Circle::new(Point::new(c[0], c[1]), r))
                    }
                    "arc" => {
                        let c = sd.center.ok_or("arc needs center")?;
                        let r = sd.radius.ok_or("arc needs radius")?;
                        let from = sd.from.unwrap_or(0.0).to_radians();
                        let to = sd.to.unwrap_or(360.0).to_radians();
                        Shape::Arc {
                            center: Point::new(c[0], c[1]),
                            radius: r,
                            start_angle: from,
                            end_angle: to,
                        }
                    }
                    "polyline" => {
                        let pts = sd.points.as_ref().ok_or("polyline needs points")?;
                        let pts: Vec<Point> = pts.iter().map(|p| Point::new(p[0], p[1])).collect();
                        Shape::Polyline { points: pts, closed: sd.closed.unwrap_or(false) }
                    }
                    other => return Err(format!("unknown shape type in block: {other}")),
                };
                shapes.push((shape, layer, color));
            }
            let ip = a.insert_point.unwrap_or([0.0, 0.0]);
            doc.define_block(BlockDef {
                name: a.name.clone(),
                shapes,
                insert_point: Point::new(ip[0], ip[1]),
                default_layer: a.default_layer,
            });
            Ok(serde_json::json!({ "defined": a.name }).to_string())
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
            let shapes: Vec<(Shape, String, Color)> = tmp_doc.entities
                .into_iter()
                .map(|e| (e.shape, e.layer, e.color))
                .collect();
            let count = shapes.len();
            let ip = a.insert_point.unwrap_or([0.0, 0.0]);
            doc.define_block(BlockDef {
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
            let mut sym = crate::elmt::load_elmt(&a.path)
                .map_err(|e| format!("loadElmt: {e}"))?;
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
                        if x0 < min_x { min_x = x0; }
                        if y0 < min_y { min_y = y0; }
                        if x1 > max_x { max_x = x1; }
                        if y1 > max_y { max_y = y1; }
                    }
                    if min_x.is_finite() {
                        sym.block.insert_point = Point::new(
                            (min_x + max_x) / 2.0,
                            (min_y + max_y) / 2.0,
                        );
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
            }).to_string())
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
            let symbols = crate::elmt::load_elmt_dir(&a.path)
                .map_err(|e| format!("loadElmtDir: {e}"))?;
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
            let source_def = doc.blocks.get(&a.source)
                .ok_or_else(|| format!("block '{}' not defined", a.source))?
                .clone();
            let mut new_def = BlockDef {
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
                        Shape::MText { text, plain_text, .. } => {
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
            }).to_string())
        }

        "place" => {
            #[derive(Deserialize)]
            struct Args {
                block: String,
                at: [f64; 2],
                #[serde(default)]
                rotation: f64,
                #[serde(default)]
                layer: Option<String>,
                #[serde(default = "default_one")]
                scale: f64,
            }
            fn default_one() -> f64 { 1.0 }
            let a: Args = parse_args(args)?;
            let ids = doc.place_block_scaled(
                &a.block,
                Point::new(a.at[0], a.at[1]),
                a.rotation,
                a.scale,
                a.scale,
                a.layer.as_deref(),
            );
            if ids.is_empty() {
                return Err(format!("block '{}' not defined", a.block));
            }
            // First ID is the block_insert entity, return it directly
            let insert_ent = doc.entity(ids[0]).expect("entity was just added");
            serde_json::to_string(&insert_ent.to_json()).map_err(|e| e.to_string())
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
            let ent = doc.entity(eid).ok_or(format!("entity {} not found", a.id))?;
            let tol = a.tolerance;

            // Collect endpoints of the source entity
            let src_pts = entity_endpoints(&ent.shape);
            if src_pts.is_empty() {
                return Ok("[]".to_string());
            }

            // Find all other entities sharing an endpoint within tolerance
            let mut connected: Vec<EntityJson> = Vec::new();
            for other in &doc.entities {
                if other.id == eid { continue; }
                let other_pts = entity_endpoints(&other.shape);
                let shares = src_pts.iter().any(|sp|
                    other_pts.iter().any(|op| geo::distance(*sp, *op) < tol)
                );
                if shares {
                    connected.push(other.to_json());
                }
            }
            serde_json::to_string(&connected).map_err(|e| e.to_string())
        }

        // Geometry helpers (pure queries, no mutation)
        "distance" => {
            #[derive(Deserialize)]
            struct Args { a: [f64; 2], b: [f64; 2] }
            let a: Args = parse_args(args)?;
            let d = geo::distance(
                Point::new(a.a[0], a.a[1]),
                Point::new(a.b[0], a.b[1]),
            );
            Ok(serde_json::json!(d).to_string())
        }

        "midpoint" => {
            #[derive(Deserialize)]
            struct Args { a: [f64; 2], b: [f64; 2] }
            let a: Args = parse_args(args)?;
            let m = geo::midpoint(
                Point::new(a.a[0], a.a[1]),
                Point::new(a.b[0], a.b[1]),
            );
            Ok(serde_json::json!([m.x, m.y]).to_string())
        }

        "direction" => {
            #[derive(Deserialize)]
            struct Args { a: [f64; 2], b: [f64; 2] }
            let a: Args = parse_args(args)?;
            let d = geo::direction(
                Point::new(a.a[0], a.a[1]),
                Point::new(a.b[0], a.b[1]),
            );
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
                c1: [f64; 2], r1: f64,
                c2: [f64; 2], r2: f64,
            }
            let a: Args = parse_args(args)?;
            let pts = geo::circle_circle_intersection(
                Point::new(a.c1[0], a.c1[1]), a.r1,
                Point::new(a.c2[0], a.c2[1]), a.r2,
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

        _ => Err(format!("unknown method: {method}")),
    }
}

fn parse_args<T: serde::de::DeserializeOwned>(args: &str) -> Result<T, String> {
    serde_json::from_str(args).map_err(|e| format!("bad args: {e}"))
}

fn parse_entity_id(s: &str) -> Result<EntityId, String> {
    let num = s.strip_prefix("e_").unwrap_or(s);
    num.parse::<u64>().map(EntityId).map_err(|_| format!("bad entity id: {s}"))
}

/// Parse a target that can be a single ID string, array of ID strings,
/// or array of entity objects (with .id field).
fn parse_target(val: &serde_json::Value) -> Result<Vec<EntityId>, String> {
    match val {
        serde_json::Value::String(s) => Ok(vec![parse_entity_id(s)?]),
        serde_json::Value::Array(arr) => {
            arr.iter().map(|v| {
                match v {
                    serde_json::Value::String(s) => parse_entity_id(s),
                    serde_json::Value::Object(obj) => {
                        obj.get("id")
                            .and_then(|v| v.as_str())
                            .ok_or_else(|| "entity object missing 'id' field".to_string())
                            .and_then(parse_entity_id)
                    }
                    _ => Err("target array elements must be strings or objects".to_string()),
                }
            }).collect()
        }
        _ => Err("target must be a string, array of strings, or array of entity objects".to_string()),
    }
}

/// Store glyph BezPaths as CurvePath entities (abstract, not flattened).
pub(crate) fn add_glyph_paths(
    doc: &mut Document,
    glyph_paths: &[BezPath],
    layer: &str,
    color: Color,
) -> Vec<EntityId> {
    let mut ids = Vec::new();
    for path in glyph_paths {
        if path.elements().is_empty() { continue; }
        // Split multi-contour BezPaths into individual subpaths.
        // Glyphs like "A" have outer + inner contours; each needs
        // its own entity so the renderer draws all of them.
        for subpath in split_subpaths(path) {
            if subpath.elements().is_empty() { continue; }
            let closed = matches!(subpath.elements().last(), Some(kurbo::PathEl::ClosePath));
            let id = doc.alloc_id();
            doc.entities.push(DrawEntity {
                id,
                layer: layer.to_string(),
                color,
                shape: Shape::CurvePath { path: subpath, closed },
            });
            ids.push(id);
        }
    }
    ids
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

fn default_layer() -> String { "0".to_string() }
fn default_arc_to() -> f64 { 360.0 }
fn default_tolerance() -> f64 { 0.01 }
fn default_hatch_angle() -> f64 { 45.0 }
fn default_hatch_spacing() -> f64 { 5.0 }
fn default_text_height() -> f64 { 5.0 }
fn default_measure_offset() -> f64 { 10.0 }
