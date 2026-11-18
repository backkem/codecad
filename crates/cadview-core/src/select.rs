//! Selections: which entities a call works on.
//!
//! A selection is an id, a list of ids or entity objects, or a selector
//! object `{ids, layer, type, attrs, has, bounds, expand}` whose conditions
//! must all hold. `expand` turns block inserts into their child shapes, each
//! matched on its own layer and attributes. The same selection works for
//! `entities()`, for every mutation that takes a target, and for the set
//! queries in `setquery.rs`.

use crate::attrs::{self, AttrValue, Attrs};
use crate::dispatch::parse_entity_id;
use crate::document::{placed_block_shapes, Document, DrawEntity};
use crate::types::*;
use serde_json::Value;
use std::collections::HashSet;

/// The keys a selector object may have.
const SELECTOR_KEYS: &str = "ids, layer, type, attrs, has, bounds, expand";

/// A parsed selection. Every condition that is set must hold.
#[derive(Debug, Clone, Default)]
pub struct Selector {
    /// Only these entities (for an expanded insert: its children).
    pub ids: Option<HashSet<EntityId>>,
    /// The ids in the order given, for mutations.
    id_list: Vec<EntityId>,
    /// Layer names (any of them).
    pub layers: Option<Vec<String>>,
    /// Entity types as `entities()` reports them (any of them).
    pub types: Option<Vec<String>>,
    /// Attribute equality; `None` means the key must be absent.
    pub attrs: Vec<(String, Option<AttrValue>)>,
    /// Keys that must be present, whatever their value.
    pub has: Vec<String>,
    /// The entity's bounding box must overlap this rectangle.
    pub bounds: Option<(f64, f64, f64, f64)>,
    /// Match block inserts child by child.
    pub expand: bool,
}

fn strings(v: &Value, what: &str) -> Result<Vec<String>, String> {
    match v {
        Value::String(s) => Ok(vec![s.clone()]),
        Value::Array(a) => a
            .iter()
            .map(|x| {
                x.as_str()
                    .map(str::to_string)
                    .ok_or_else(|| format!("{what} must be a string or an array of strings"))
            })
            .collect(),
        _ => Err(format!("{what} must be a string or an array of strings")),
    }
}

fn id_of(v: &Value) -> Result<EntityId, String> {
    match v {
        Value::String(s) => parse_entity_id(s),
        Value::Object(o) => o
            .get("id")
            .and_then(|i| i.as_str())
            .ok_or_else(|| "entity object missing 'id' field".to_string())
            .and_then(parse_entity_id),
        _ => Err("ids are strings or entity objects".to_string()),
    }
}

fn rect_of(v: &Value) -> Result<(f64, f64, f64, f64), String> {
    let p = |k: &str| -> Result<(f64, f64), String> {
        let a = v
            .get(k)
            .and_then(|x| x.as_array())
            .filter(|a| a.len() >= 2)
            .ok_or("bounds is {min: [x, y], max: [x, y]}")?;
        Ok((
            a[0].as_f64().ok_or("bounds coordinates are numbers")?,
            a[1].as_f64().ok_or("bounds coordinates are numbers")?,
        ))
    };
    let (x0, y0) = p("min")?;
    let (x1, y1) = p("max")?;
    Ok((x0.min(x1), y0.min(y1), x0.max(x1), y0.max(y1)))
}

/// Is this value a selector object (rather than an id or entity object)?
pub fn is_selector_object(v: &Value) -> bool {
    matches!(v, Value::Object(o) if !o.contains_key("id"))
}

impl Selector {
    /// Everything.
    pub fn all() -> Self {
        Self::default()
    }

    /// Parse a selection. Null selects everything.
    pub fn parse(v: &Value) -> Result<Self, String> {
        let mut s = Selector::default();
        match v {
            Value::Null => {}
            Value::String(_) => s.set_ids(vec![id_of(v)?]),
            Value::Array(a) => s.set_ids(a.iter().map(id_of).collect::<Result<_, _>>()?),
            Value::Object(o) if o.contains_key("id") => s.set_ids(vec![id_of(v)?]),
            Value::Object(o) => {
                for (k, v) in o {
                    if v.is_null() && k != "attrs" {
                        continue;
                    }
                    match k.as_str() {
                        "ids" => {
                            let a = v.as_array().ok_or("ids must be an array")?;
                            s.set_ids(a.iter().map(id_of).collect::<Result<_, _>>()?);
                        }
                        "layer" | "layers" => s.layers = Some(strings(v, "layer")?),
                        "type" | "types" => s.types = Some(strings(v, "type")?),
                        "attrs" => {
                            if !v.is_null() {
                                s.attrs = attrs::parse_updates(v)?;
                            }
                        }
                        "has" => {
                            s.has = strings(v, "has")?;
                            for k in &s.has {
                                attrs::check_key(k)?;
                            }
                        }
                        "bounds" => s.bounds = Some(rect_of(v)?),
                        "expand" => s.expand = v.as_bool().ok_or("expand must be a boolean")?,
                        other => {
                            return Err(format!(
                                "unknown selector key '{other}' (use {SELECTOR_KEYS})"
                            ))
                        }
                    }
                }
            }
            _ => {
                return Err(format!(
                    "a selection is an id, a list of ids, or {{{SELECTOR_KEYS}}}"
                ))
            }
        }
        Ok(s)
    }

    fn set_ids(&mut self, ids: Vec<EntityId>) {
        self.ids = Some(ids.iter().copied().collect());
        self.id_list = ids;
    }

    /// Does an entity or child with these properties match? `bbox` is only
    /// computed when the selector has bounds.
    pub fn matches(
        &self,
        id: EntityId,
        layer: &str,
        kind: &str,
        attrs: &Attrs,
        bbox: impl FnOnce() -> (f64, f64, f64, f64),
    ) -> bool {
        if self.ids.as_ref().is_some_and(|ids| !ids.contains(&id)) {
            return false;
        }
        if self
            .layers
            .as_ref()
            .is_some_and(|ls| !ls.iter().any(|l| l == layer))
        {
            return false;
        }
        if self
            .types
            .as_ref()
            .is_some_and(|ts| !ts.iter().any(|t| t == kind))
        {
            return false;
        }
        if !self.attrs.iter().all(|(k, v)| attrs.get(k) == v.as_ref()) {
            return false;
        }
        if !self.has.iter().all(|k| attrs.contains_key(k)) {
            return false;
        }
        if let Some((x0, y0, x1, y1)) = self.bounds {
            let b = bbox();
            if b.2 < x0 || b.0 > x1 || b.3 < y0 || b.1 > y1 {
                return false;
            }
        }
        true
    }
}

/// The type name `entities()` reports for a shape.
pub fn shape_type(shape: &Shape) -> &'static str {
    match shape {
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
    }
}

/// The attributes a child of an insert carries: the definition's, then the
/// shape's own, then the insert's (a later one wins on equal keys).
pub fn child_attrs(def: &crate::document::BlockDef, i: usize, insert: &DrawEntity) -> Attrs {
    attrs::layered(&[&def.attrs, def.shape_attrs(i), &insert.attrs])
}

/// The children of a block insert as world-space entities, the way
/// `entities({expand: true})` and `children()` report them: each keeps the
/// insert's id and takes its index in the definition.
pub fn insert_children(doc: &Document, ent: &DrawEntity) -> Vec<(usize, DrawEntity)> {
    let Shape::BlockInsert {
        block_name,
        position,
        rotation,
        x_scale,
        y_scale,
    } = &ent.shape
    else {
        return Vec::new();
    };
    let Some(def) = doc.blocks.get(block_name) else {
        return Vec::new();
    };
    let xform = kurbo::Affine::translate((position.x, position.y))
        * kurbo::Affine::rotate(*rotation)
        * kurbo::Affine::scale_non_uniform(*x_scale, *y_scale)
        * kurbo::Affine::translate((-def.insert_point.x, -def.insert_point.y));
    def.shapes
        .iter()
        .enumerate()
        .map(|(i, (shape, shape_layer, shape_color))| {
            let layer = if shape_layer.is_empty() {
                &ent.layer
            } else {
                shape_layer
            };
            let color = if shape_color.is_none() || *shape_color == Some(Color::WHITE) {
                // Inherit from the insert; if that is ByLayer too, the
                // insert's layer colour.
                ent.color.or_else(|| Some(doc.layer_color(&ent.layer)))
            } else {
                *shape_color
            };
            (
                i,
                DrawEntity {
                    id: ent.id,
                    layer: layer.to_string(),
                    color,
                    linetype: None,
                    lineweight: None,
                    transparency: 0,
                    shape: shape.transformed(xform),
                    attrs: child_attrs(def, i, ent),
                },
            )
        })
        .collect()
}

/// One selected thing with world-space geometry: an entity, or (with
/// `expand`) one child of a block insert.
#[derive(Debug, Clone)]
pub struct Item {
    pub id: EntityId,
    /// Index of the child in its block definition, for an expanded child.
    pub child: Option<usize>,
    pub layer: String,
    pub attrs: Attrs,
    /// World-space shapes, each with its child index: one shape for an
    /// entity or a child; every placed child for an unexpanded insert.
    pub shapes: Vec<(Option<usize>, Shape)>,
}

fn union_bbox(shapes: &[(Option<usize>, Shape)], fallback: &Shape) -> (f64, f64, f64, f64) {
    let mut b = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
    for (_, s) in shapes {
        let s = s.bbox();
        b = (b.0.min(s.0), b.1.min(s.1), b.2.max(s.2), b.3.max(s.3));
    }
    if b.0 > b.2 {
        fallback.bbox()
    } else {
        b
    }
}

/// An entity's bounding box; for a block insert, that of its placed children.
pub fn entity_bbox(doc: &Document, ent: &DrawEntity) -> (f64, f64, f64, f64) {
    if matches!(ent.shape, Shape::BlockInsert { .. }) {
        let shapes: Vec<(Option<usize>, Shape)> = placed_block_shapes(doc, ent)
            .into_iter()
            .map(|p| (None, p.shape))
            .collect();
        union_bbox(&shapes, &ent.shape)
    } else {
        ent.shape.bbox()
    }
}

/// The items a selection picks, in document order (children in definition
/// order).
pub fn collect(doc: &Document, sel: &Selector) -> Vec<Item> {
    let mut out = Vec::new();
    for ent in &doc.entities {
        let is_insert = matches!(ent.shape, Shape::BlockInsert { .. });
        if is_insert && sel.expand {
            let Shape::BlockInsert { block_name, .. } = &ent.shape else {
                unreachable!()
            };
            let Some(def) = doc.blocks.get(block_name) else {
                continue;
            };
            for (i, placed) in placed_block_shapes(doc, ent).into_iter().enumerate() {
                let child_attrs = child_attrs(def, i, ent);
                let shape = placed.shape;
                if sel.matches(
                    ent.id,
                    &placed.layer,
                    shape_type(&shape),
                    &child_attrs,
                    || shape.bbox(),
                ) {
                    out.push(Item {
                        id: ent.id,
                        child: Some(i),
                        layer: placed.layer,
                        attrs: child_attrs,
                        shapes: vec![(Some(i), shape)],
                    });
                }
            }
            continue;
        }
        let shapes: Vec<(Option<usize>, Shape)> = if is_insert {
            placed_block_shapes(doc, ent)
                .into_iter()
                .enumerate()
                .map(|(i, p)| (Some(i), p.shape))
                .collect()
        } else {
            vec![(None, ent.shape.clone())]
        };
        if sel.matches(
            ent.id,
            &ent.layer,
            shape_type(&ent.shape),
            &ent.attrs,
            || union_bbox(&shapes, &ent.shape),
        ) {
            out.push(Item {
                id: ent.id,
                child: None,
                layer: ent.layer.clone(),
                attrs: ent.attrs.clone(),
                shapes,
            });
        }
    }
    out
}

/// The top-level entities a selector object picks, for mutations.
pub fn select_ids(doc: &Document, sel: &Selector) -> Result<Vec<EntityId>, String> {
    if sel.expand {
        return Err("expand selects children of block inserts; a mutation needs whole entities (leave out expand)".into());
    }
    if !sel.id_list.is_empty()
        && sel.layers.is_none()
        && sel.types.is_none()
        && sel.attrs.is_empty()
        && sel.has.is_empty()
        && sel.bounds.is_none()
    {
        return Ok(sel.id_list.clone());
    }
    Ok(collect(doc, sel).into_iter().map(|i| i.id).collect())
}
