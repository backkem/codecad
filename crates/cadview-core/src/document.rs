use kurbo::{Affine, Circle, Line, Point};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::f64::consts::PI;
use crate::types::*;
use crate::tessellate::mirror_affine;
use crate::geo;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DrawEntity {
    pub id: EntityId,
    pub layer: String,
    pub color: Color,
    pub shape: Shape,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Layer {
    pub name: String,
    pub color: Color,
    pub visible: bool,
}

// ── Document ───────────────────────────────────────────────────────────

/// A block definition: reusable collection of shapes at local origin.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlockDef {
    pub name: String,
    pub shapes: Vec<(Shape, String, Color)>,  // (shape, layer, color)
    pub insert_point: Point,
    pub default_layer: String,
}

/// Serialize a BlockDef to bincode bytes.
pub fn block_to_bytes(block: &BlockDef) -> Vec<u8> {
    bincode::serialize(block).expect("bincode serialize failed")
}

/// Deserialize a BlockDef from bincode bytes.
pub fn block_from_bytes(data: &[u8]) -> Option<BlockDef> {
    bincode::deserialize(data).ok()
}

#[derive(Debug, Clone)]
struct Snapshot {
    layers: Vec<Layer>,
    entities: Vec<DrawEntity>,
    next_id: u64,
}

#[derive(Debug, Clone)]
pub struct Document {
    pub layers: Vec<Layer>,
    pub entities: Vec<DrawEntity>,
    pub blocks: HashMap<String, BlockDef>,
    next_id: u64,
    undo_stack: Vec<Snapshot>,
    redo_stack: Vec<Snapshot>,
}

impl Document {
    /// Empty document.
    pub fn new() -> Self {
        Self {
            layers: Vec::new(),
            entities: Vec::new(),
            blocks: HashMap::new(),
            next_id: 1,
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
        }
    }

    /// Save current state to undo stack. Call before any mutation.
    pub fn checkpoint(&mut self) {
        self.undo_stack.push(Snapshot {
            layers: self.layers.clone(),
            entities: self.entities.clone(),
            next_id: self.next_id,
        });
        self.redo_stack.clear();
        // Cap at 50 snapshots
        if self.undo_stack.len() > 50 {
            self.undo_stack.remove(0);
        }
    }

    pub fn undo_depth(&self) -> usize {
        self.undo_stack.len()
    }

    pub fn undo(&mut self) -> bool {
        if let Some(snap) = self.undo_stack.pop() {
            self.redo_stack.push(Snapshot {
                layers: self.layers.clone(),
                entities: self.entities.clone(),
                next_id: self.next_id,
            });
            self.layers = snap.layers;
            self.entities = snap.entities;
            self.next_id = snap.next_id;
            true
        } else {
            false
        }
    }

    pub fn redo(&mut self) -> bool {
        if let Some(snap) = self.redo_stack.pop() {
            self.undo_stack.push(Snapshot {
                layers: self.layers.clone(),
                entities: self.entities.clone(),
                next_id: self.next_id,
            });
            self.layers = snap.layers;
            self.entities = snap.entities;
            self.next_id = snap.next_id;
            true
        } else {
            false
        }
    }

    /// Set the next entity ID counter (used when restoring from external state).
    pub fn set_next_id(&mut self, id: u64) {
        self.next_id = id;
    }

    pub(crate) fn alloc_id(&mut self) -> EntityId {
        let id = EntityId(self.next_id);
        self.next_id += 1;
        id
    }

    // ── Queries ────────────────────────────────────────────────────

    pub fn bounds(&self) -> Option<(f64, f64, f64, f64)> {
        let mut min_x = f64::MAX;
        let mut min_y = f64::MAX;
        let mut max_x = f64::MIN;
        let mut max_y = f64::MIN;

        for ent in &self.entities {
            let bb = ent.shape.bbox();
            min_x = min_x.min(bb.0);
            min_y = min_y.min(bb.1);
            max_x = max_x.max(bb.2);
            max_y = max_y.max(bb.3);
        }

        if min_x <= max_x { Some((min_x, min_y, max_x, max_y)) } else { None }
    }

    pub fn entity(&self, id: EntityId) -> Option<&DrawEntity> {
        self.entities.iter().find(|e| e.id == id)
    }

    pub fn entity_mut(&mut self, id: EntityId) -> Option<&mut DrawEntity> {
        self.entities.iter_mut().find(|e| e.id == id)
    }

    // ── Mutations ──────────────────────────────────────────────────

    pub fn add_line(
        &mut self,
        p0: Point,
        p1: Point,
        layer: &str,
        color: Color,
    ) -> EntityId {
        let id = self.alloc_id();
        self.entities.push(DrawEntity {
            id,
            layer: layer.to_string(),
            color,
            shape: Shape::Line(Line::new(p0, p1)),
        });
        id
    }

    pub fn add_circle(
        &mut self,
        center: Point,
        radius: f64,
        layer: &str,
        color: Color,
    ) -> EntityId {
        let id = self.alloc_id();
        self.entities.push(DrawEntity {
            id,
            layer: layer.to_string(),
            color,
            shape: Shape::Circle(Circle::new(center, radius)),
        });
        id
    }

    pub fn add_arc(
        &mut self,
        center: Point,
        radius: f64,
        start_angle: f64,
        end_angle: f64,
        layer: &str,
        color: Color,
    ) -> EntityId {
        let id = self.alloc_id();
        self.entities.push(DrawEntity {
            id,
            layer: layer.to_string(),
            color,
            shape: Shape::Arc { center, radius, start_angle, end_angle },
        });
        id
    }

    pub fn add_polyline(
        &mut self,
        points: Vec<Point>,
        closed: bool,
        layer: &str,
        color: Color,
    ) -> EntityId {
        let id = self.alloc_id();
        self.entities.push(DrawEntity {
            id,
            layer: layer.to_string(),
            color,
            shape: Shape::Polyline { points, closed },
        });
        id
    }

    pub fn remove_entity(&mut self, id: EntityId) -> Option<DrawEntity> {
        if let Some(pos) = self.entities.iter().position(|e| e.id == id) {
            Some(self.entities.remove(pos))
        } else {
            None
        }
    }

    pub fn move_entity(&mut self, id: EntityId, dx: f64, dy: f64) -> bool {
        let Some(ent) = self.entity_mut(id) else { return false };
        let xform = Affine::translate((dx, dy));
        ent.shape = ent.shape.transformed(xform);
        true
    }

    /// Copy an entity, offset by (dx, dy). Returns the new entity's ID.
    pub fn copy_entity(&mut self, id: EntityId, dx: f64, dy: f64) -> Option<EntityId> {
        let ent = self.entity(id)?.clone();
        let xform = Affine::translate((dx, dy));
        let new_id = self.alloc_id();
        self.entities.push(DrawEntity {
            id: new_id,
            layer: ent.layer,
            color: ent.color,
            shape: ent.shape.transformed(xform),
        });
        Some(new_id)
    }

    /// Rotate an entity around `center` by `angle_deg` degrees (CCW).
    pub fn rotate_entity(&mut self, id: EntityId, center: Point, angle_deg: f64) -> bool {
        let Some(ent) = self.entity_mut(id) else { return false };
        let angle_rad = angle_deg * PI / 180.0;
        let xform = Affine::translate((center.x, center.y))
            * Affine::rotate(angle_rad)
            * Affine::translate((-center.x, -center.y));
        ent.shape = ent.shape.transformed(xform);
        true
    }

    /// Mirror an entity across the line from `p1` to `p2`. Mutates in place.
    pub fn mirror_entity(&mut self, id: EntityId, p1: Point, p2: Point) -> bool {
        let Some(ent) = self.entity_mut(id) else { return false };
        let xform = mirror_affine(p1, p2);
        ent.shape = ent.shape.transformed(xform);
        true
    }

    /// Trim an entity at cut_point.
    ///
    /// `keep` controls which side survives:
    /// - `"start"` / `"from"`: keep the segment from entity start to cut_point
    /// - `"end"` / `"to"`:     keep the segment from cut_point to entity end
    ///
    /// For lines: "start" keeps p0..cut, "end" keeps cut..p1.
    /// For arcs: "from" keeps from_angle..cut_angle, "to" keeps cut_angle..to_angle.
    ///
    /// Removes the original entity and creates a new shortened one.
    /// Returns the new entity ID, or None if the entity can't be trimmed.
    pub fn trim_entity(
        &mut self,
        id: EntityId,
        cut_point: Point,
        keep: &str,
    ) -> Option<EntityId> {
        let ent = self.entity(id)?;
        let layer = ent.layer.clone();
        let color = ent.color;
        let keep_start = matches!(keep, "start" | "from");

        match &ent.shape {
            Shape::Line(line) => {
                let p0 = line.p0;
                let p1 = line.p1;
                let cut = geo::project_onto(cut_point, p0, p1);
                let (new_start, new_end) = if keep_start {
                    (p0, cut)
                } else {
                    (cut, p1)
                };
                if geo::distance(new_start, new_end) < 1e-6 {
                    return None;
                }
                self.remove_entity(id);
                Some(self.add_line(new_start, new_end, &layer, color))
            }
            Shape::Arc { center, radius, start_angle, end_angle, .. } => {
                let center = *center;
                let radius = *radius;
                let start_angle = *start_angle;
                let end_angle = *end_angle;

                let cut_ang = (cut_point.y - center.y).atan2(cut_point.x - center.x);

                // Normalize cut angle into the arc's sweep
                let mut cut_rel = cut_ang - start_angle;
                while cut_rel < 0.0 { cut_rel += 2.0 * PI; }
                while cut_rel >= 2.0 * PI { cut_rel -= 2.0 * PI; }

                let mut sweep = end_angle - start_angle;
                while sweep <= 0.0 { sweep += 2.0 * PI; }

                let cut_rel = cut_rel.min(sweep);

                let (new_start, new_end) = if keep_start {
                    (start_angle, start_angle + cut_rel)
                } else {
                    (start_angle + cut_rel, start_angle + sweep)
                };

                let new_sweep = new_end - new_start;
                if new_sweep.abs() < 1e-6 {
                    return None;
                }

                self.remove_entity(id);
                Some(self.add_arc(center, radius, new_start, new_end, &layer, color))
            }
            _ => None,
        }
    }

    pub fn add_layer(&mut self, name: &str, color: Color) {
        if let Some(layer) = self.layers.iter_mut().find(|l| l.name == name) {
            layer.color = color; // update existing
        } else {
            self.layers.push(Layer {
                name: name.to_string(),
                color,
                visible: true,
            });
        }
    }

    /// Ensure a layer exists, creating it with default white if missing.
    pub fn ensure_layer(&mut self, name: &str) {
        if !self.layers.iter().any(|l| l.name == name) {
            self.add_layer(name, Color::WHITE);
        }
    }

    /// Get a layer's color. Returns white if layer doesn't exist.
    pub fn layer_color(&self, name: &str) -> Color {
        self.layers.iter().find(|l| l.name == name).map_or(Color::WHITE, |l| l.color)
    }

    // ── Blocks ─────────────────────────────────────────────────────

    pub fn define_block(&mut self, def: BlockDef) {
        self.blocks.insert(def.name.clone(), def);
    }

    /// Place a block instance. Creates a BlockInsert reference entity;
    /// the renderer expands it from the block definition at draw time.
    /// Returns a single-element vec with the BlockInsert entity ID.
    pub fn place_block(
        &mut self,
        block_name: &str,
        position: Point,
        rotation_deg: f64,
        layer_override: Option<&str>,
    ) -> Vec<EntityId> {
        self.place_block_scaled(block_name, position, rotation_deg, 1.0, 1.0, layer_override)
    }

    pub fn place_block_scaled(
        &mut self,
        block_name: &str,
        position: Point,
        rotation_deg: f64,
        x_scale: f64,
        y_scale: f64,
        layer_override: Option<&str>,
    ) -> Vec<EntityId> {
        let def = match self.blocks.get(block_name) {
            Some(d) => d.clone(),
            None => return Vec::new(),
        };

        let place_layer = layer_override
            .unwrap_or(&def.default_layer)
            .to_string();

        self.ensure_layer(&place_layer);
        let color = self.layer_color(&place_layer);
        let insert_id = self.alloc_id();
        self.entities.push(DrawEntity {
            id: insert_id,
            layer: place_layer,
            color,
            shape: Shape::BlockInsert {
                block_name: block_name.to_string(),
                position,
                rotation: rotation_deg.to_radians(),
                x_scale,
                y_scale,
            },
        });

        vec![insert_id]
    }
}

impl Default for Document {
    fn default() -> Self {
        Self::new()
    }
}

// ── JSON serialization ─────────────────────────────────────────────────

/// Serializable entity for the JS API.
#[derive(Serialize, Deserialize)]
pub struct EntityJson {
    pub id: String,
    #[serde(rename = "type")]
    pub entity_type: String,
    pub layer: String,
    pub color: [u8; 3],
    pub bounds: BoundsJson,
    // Type-specific fields (flattened)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start: Option<[f64; 2]>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub end: Option<[f64; 2]>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub center: Option<[f64; 2]>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub radius: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub points: Option<Vec<[f64; 2]>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub closed: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub from: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub to: Option<f64>,
    // Block insert fields
    #[serde(skip_serializing_if = "Option::is_none")]
    pub block_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rotation: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub children: Option<Vec<String>>,
    // Text/MText top-level fields
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub height: Option<f64>,
    /// Abstract boundary edges for SolidFill / CurvePath segments.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub edges: Option<serde_json::Value>,
}

#[derive(Serialize, Deserialize)]
pub struct BoundsJson {
    pub min: [f64; 2],
    pub max: [f64; 2],
}

impl DrawEntity {
    pub fn to_json(&self) -> EntityJson {
        let bb = self.shape.bbox();
        let mut ej = EntityJson {
            id: format!("e_{}", self.id.0),
            entity_type: String::new(),
            layer: self.layer.clone(),
            color: [self.color.r, self.color.g, self.color.b],
            bounds: BoundsJson { min: [bb.0, bb.1], max: [bb.2, bb.3] },
            start: None, end: None, center: None, radius: None,
            points: None, closed: None, from: None, to: None,
            block_name: None, rotation: None, children: None,
            text: None, height: None, edges: None,
        };
        match &self.shape {
            Shape::Line(l) => {
                ej.entity_type = "line".into();
                ej.start = Some([l.p0.x, l.p0.y]);
                ej.end = Some([l.p1.x, l.p1.y]);
            }
            Shape::Circle(c) => {
                ej.entity_type = "circle".into();
                ej.center = Some([c.center.x, c.center.y]);
                ej.radius = Some(c.radius);
            }
            Shape::Arc { center, radius, start_angle, end_angle } => {
                ej.entity_type = "arc".into();
                ej.center = Some([center.x, center.y]);
                ej.radius = Some(*radius);
                ej.from = Some(start_angle.to_degrees());
                ej.to = Some(end_angle.to_degrees());
                ej.start = Some([
                    center.x + radius * start_angle.cos(),
                    center.y + radius * start_angle.sin(),
                ]);
                ej.end = Some([
                    center.x + radius * end_angle.cos(),
                    center.y + radius * end_angle.sin(),
                ]);
            }
            Shape::Polyline { points, closed } => {
                ej.entity_type = "polyline".into();
                ej.points = Some(points.iter().map(|p| [p.x, p.y]).collect());
                ej.closed = Some(*closed);
            }
            Shape::LwPolyline { vertices, closed } => {
                ej.entity_type = "lwpolyline".into();
                ej.closed = Some(*closed);
                ej.edges = Some(serde_json::json!(
                    vertices.iter().map(|v| {
                        serde_json::json!({"x": v.point.x, "y": v.point.y, "bulge": v.bulge})
                    }).collect::<Vec<_>>()
                ));
            }
            Shape::SolidFill { boundary, holes } => {
                ej.entity_type = "solid_fill".into();
                ej.closed = Some(true);
                let edge_to_json = |e: &FillEdge| -> serde_json::Value {
                    match e {
                        FillEdge::LineTo(p) => serde_json::json!({"type": "line", "to": [p.x, p.y]}),
                        FillEdge::ArcTo { center, radius, start_angle, end_angle } => {
                            serde_json::json!({
                                "type": "arc",
                                "center": [center.x, center.y],
                                "radius": radius,
                                "from": start_angle.to_degrees(),
                                "to": end_angle.to_degrees(),
                            })
                        }
                        FillEdge::EllipseArcTo { center, major_axis, minor_ratio, start_param, end_param } => {
                            serde_json::json!({"type": "ellipse_arc", "center": [center.x, center.y], "major_axis": [major_axis.0, major_axis.1], "minor_ratio": minor_ratio, "start": start_param, "end": end_param})
                        }
                        FillEdge::SplineTo { degree, knots, control_points } => {
                            serde_json::json!({"type": "spline", "degree": degree, "knots": knots, "points": control_points.iter().map(|p| [p.x, p.y]).collect::<Vec<_>>()})
                        }
                        FillEdge::PolylineTo(pts) => {
                            serde_json::json!({"type": "polyline", "points": pts.iter().map(|p| [p.x, p.y]).collect::<Vec<_>>()})
                        }
                    }
                };
                let boundary_json: Vec<_> = boundary.iter().map(&edge_to_json).collect();
                let holes_json: Vec<Vec<_>> = holes.iter()
                    .map(|h| h.iter().map(&edge_to_json).collect())
                    .collect();
                ej.edges = Some(serde_json::json!({
                    "boundary": boundary_json,
                    "holes": holes_json,
                }));
            }
            Shape::CurvePath { path, closed } => {
                ej.entity_type = "curve_path".into();
                ej.closed = Some(*closed);
                // Serialize BezPath as abstract path commands
                let cmds: Vec<serde_json::Value> = path.iter().map(|el| match el {
                    kurbo::PathEl::MoveTo(p) => serde_json::json!({"type": "M", "to": [p.x, p.y]}),
                    kurbo::PathEl::LineTo(p) => serde_json::json!({"type": "L", "to": [p.x, p.y]}),
                    kurbo::PathEl::QuadTo(c, p) => serde_json::json!({"type": "Q", "ctrl": [c.x, c.y], "to": [p.x, p.y]}),
                    kurbo::PathEl::CurveTo(c1, c2, p) => serde_json::json!({"type": "C", "ctrl1": [c1.x, c1.y], "ctrl2": [c2.x, c2.y], "to": [p.x, p.y]}),
                    kurbo::PathEl::ClosePath => serde_json::json!({"type": "Z"}),
                }).collect();
                ej.edges = Some(serde_json::json!(cmds));
            }
            Shape::Ellipse { center, major_axis, minor_ratio, start_param, end_param } => {
                ej.entity_type = "ellipse".into();
                ej.center = Some([center.x, center.y]);
                ej.edges = Some(serde_json::json!({
                    "major_axis": [major_axis.0, major_axis.1],
                    "minor_ratio": minor_ratio,
                    "start": start_param,
                    "end": end_param,
                }));
            }
            Shape::Spline { degree, knots, control_points, closed } => {
                ej.entity_type = "spline".into();
                ej.closed = Some(*closed);
                ej.points = Some(control_points.iter().map(|p| [p.x, p.y]).collect());
                ej.edges = Some(serde_json::json!({
                    "degree": degree,
                    "knots": knots,
                }));
            }
            Shape::BlockInsert { block_name, position, rotation, .. } => {
                ej.entity_type = "block_insert".into();
                ej.block_name = Some(block_name.clone());
                ej.center = Some([position.x, position.y]);
                ej.rotation = Some(*rotation);
            }
            Shape::Text { text, position, height, rotation } => {
                ej.entity_type = "text".into();
                ej.start = Some([position.x, position.y]);
                ej.text = Some(text.clone());
                ej.height = Some(*height);
                ej.rotation = Some(rotation.to_degrees());
                ej.edges = Some(serde_json::json!({
                    "text": text, "height": height, "rotation": rotation.to_degrees(),
                }));
            }
            Shape::MText { plain_text, position, height, rotation, .. } => {
                ej.entity_type = "mtext".into();
                ej.start = Some([position.x, position.y]);
                ej.text = Some(plain_text.clone());
                ej.height = Some(*height);
                ej.rotation = Some(rotation.to_degrees());
                ej.edges = Some(serde_json::json!({
                    "text": plain_text, "height": height, "rotation": rotation.to_degrees(),
                }));
            }
        }
        ej
    }
}

// ── Render expansion ──────────────────────────────────────────────────
//
// Flattens BlockInsert and Text/MText entities into renderable shapes.
// Both renderers (Vello and egui) need this; keeping it in core avoids
// duplicating the affine math and text-to-path conversion.

/// Convert a single Text or MText entity into CurvePath entities.
fn text_to_curve_entities(
    layer: &str,
    color: Color,
    text_str: &str,
    position: Point,
    height: f64,
    rotation: f64,
    next_id: &mut u64,
) -> Vec<DrawEntity> {
    let paths = if rotation.abs() > 1e-6 {
        crate::text::text_to_paths_rotated(text_str, position.x, position.y, height, rotation)
    } else {
        crate::text::text_to_paths(text_str, position.x, position.y, height)
    };
    let mut out = Vec::new();
    for path in &paths {
        if path.elements().is_empty() { continue; }
        let closed = matches!(path.elements().last(), Some(kurbo::PathEl::ClosePath));
        *next_id += 1;
        out.push(DrawEntity {
            id: EntityId(*next_id),
            layer: layer.to_string(),
            color,
            shape: Shape::CurvePath { path: path.clone(), closed },
        });
    }
    out
}

/// Expand a Document's entities for rendering. Returns a list of synthetic
/// DrawEntity values that replace BlockInsert, Text, and MText:
///
/// - BlockInsert: each shape in the block definition is transformed to
///   world coordinates (translate, rotate, scale, insert-point offset).
/// - Text / MText: converted to CurvePath glyph outlines.
/// - Text / MText inside blocks: handled in a second pass after block
///   expansion so that block-internal labels become renderable curves.
///
/// The caller should render `doc.entities` (filtering out BlockInsert /
/// Text / MText) plus the returned expanded list.
pub fn expand_for_render(doc: &Document) -> Vec<DrawEntity> {
    let mut expanded = Vec::new();
    let mut next_id = 900_000u64;

    for ent in &doc.entities {
        match &ent.shape {
            Shape::BlockInsert { block_name, position, rotation, x_scale, y_scale } => {
                if let Some(def) = doc.blocks.get(block_name) {
                    let xform = Affine::translate((position.x, position.y))
                        * Affine::rotate(*rotation)
                        * Affine::scale_non_uniform(*x_scale, *y_scale)
                        * Affine::translate((-def.insert_point.x, -def.insert_point.y));
                    for (shape, shape_layer, shape_color) in &def.shapes {
                        let layer = if shape_layer.is_empty() { &ent.layer } else { shape_layer };
                        let color = if *shape_color == Color::WHITE && ent.color != Color::WHITE {
                            ent.color
                        } else {
                            *shape_color
                        };
                        next_id += 1;
                        expanded.push(DrawEntity {
                            id: EntityId(next_id),
                            layer: layer.to_string(),
                            color,
                            shape: shape.transformed(xform),
                        });
                    }
                }
            }
            Shape::Text { text, position, height, rotation } => {
                expanded.extend(text_to_curve_entities(
                    &ent.layer, ent.color, text, *position, *height, *rotation, &mut next_id,
                ));
            }
            Shape::MText { plain_text, position, height, rotation, .. } => {
                expanded.extend(text_to_curve_entities(
                    &ent.layer, ent.color, plain_text, *position, *height, *rotation, &mut next_id,
                ));
            }
            _ => {}
        }
    }

    // Second pass: Text/MText from inside blocks become CurvePaths
    let mut text_curves = Vec::new();
    for ent in expanded.iter() {
        match &ent.shape {
            Shape::Text { text, position, height, rotation } => {
                text_curves.extend(text_to_curve_entities(
                    &ent.layer, ent.color, text, *position, *height, *rotation, &mut next_id,
                ));
            }
            Shape::MText { plain_text, position, height, rotation, .. } => {
                text_curves.extend(text_to_curve_entities(
                    &ent.layer, ent.color, plain_text, *position, *height, *rotation, &mut next_id,
                ));
            }
            _ => {}
        }
    }
    expanded.retain(|e| !matches!(&e.shape, Shape::Text { .. } | Shape::MText { .. }));
    expanded.extend(text_curves);
    expanded
}

// ── Bincode serialization (for Yrs sync) ──────────────────────────────

/// Serialize a DrawEntity to bincode bytes. Lossless, compact.
pub fn entity_to_bytes(ent: &DrawEntity) -> Vec<u8> {
    bincode::serialize(ent).expect("bincode serialize failed")
}

/// Deserialize a DrawEntity from bincode bytes.
pub fn entity_from_bytes(data: &[u8]) -> Option<DrawEntity> {
    bincode::deserialize(data).ok()
}

/// Serialize a Layer to bincode bytes.
pub fn layer_to_bytes(layer: &Layer) -> Vec<u8> {
    bincode::serialize(layer).expect("bincode serialize failed")
}

/// Deserialize a Layer from bincode bytes.
pub fn layer_from_bytes(data: &[u8]) -> Option<Layer> {
    bincode::deserialize(data).ok()
}
