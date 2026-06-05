use crate::document::*;
use crate::hatch::generate_dwg_hatch_fill;
use crate::pdf;
use crate::tessellate::catmull_rom_to_bezpath;
use crate::types::*;
use crate::{
    fill_edges_bbox, flatten_bezpath_adaptive, flatten_fill_edges, geo, signed_polygon_area,
};
use anyhow::{Context, Result};
use kurbo::{Affine, Circle, Line, Point};
use std::collections::HashMap;
use std::f64::consts::PI;

// ── DWG loading ────────────────────────────────────────────────────────

pub(crate) fn aci_to_rgb(index: u8) -> Color {
    match index {
        1 => Color::rgb(255, 0, 0),
        2 => Color::rgb(255, 255, 0),
        3 => Color::rgb(0, 255, 0),
        4 => Color::rgb(0, 255, 255),
        5 => Color::rgb(0, 0, 255),
        6 => Color::rgb(255, 0, 255),
        7 => Color::rgb(255, 255, 255),
        8 => Color::rgb(128, 128, 128),
        9 => Color::rgb(192, 192, 192),
        10..=19 => Color::rgb(255, (index - 10) * 25, 0),
        20..=29 => Color::rgb(255, 128 + (index - 20) * 12, 0),
        30..=39 => Color::rgb(255, 255, (index - 30) * 25),
        40..=49 => Color::rgb(255 - (index - 40) * 12, 255, 0),
        50 | 51 | 53..=59 => Color::rgb(0, 255, (index - 50) * 25),
        52 => Color::rgb(191, 127, 0), // brown-ish override for S-COLS
        60..=69 => Color::rgb(0, 255 - (index - 60) * 12, 255),
        70..=79 => Color::rgb((index - 70) * 25, 0, 255),
        80..=89 => Color::rgb(255, 0, 255 - (index - 80) * 12),
        91 => Color::rgb(128, 0, 128),
        250 => Color::rgb(80, 80, 80),
        _ => Color::rgb(200, 200, 200),
    }
}

pub fn load_dwg(path: &str) -> Result<Document> {
    let mut reader =
        acadrust::DwgReader::from_file(path).with_context(|| format!("opening {path}"))?;
    let cad = reader.read().with_context(|| format!("parsing {path}"))?;
    build_document(cad)
}

pub fn load_dwg_bytes(data: &[u8]) -> Result<Document> {
    let cursor = std::io::Cursor::new(data.to_vec());
    let mut reader = acadrust::DwgReader::from_stream(cursor);
    let cad = reader.read().context("parsing DWG from bytes")?;
    build_document(cad)
}

/// Export a Document as a PDF. Returns the PDF bytes.
/// The caller writes to disk (server) or offers as download (WASM).
pub fn export_pdf_bytes(doc: &Document, opts: &pdf::PdfOptions) -> Vec<u8> {
    pdf::export_pdf(doc, opts)
}

/// Save a Document as a DWG file via acadrust.
/// Converts our Shape types back to acadrust entities.
/// Export a Document as DWG bytes (no filesystem access needed).
pub fn export_dwg_bytes(doc: &Document) -> Result<Vec<u8>> {
    let cad = build_cad_document(doc)?;
    acadrust::DwgWriter::write_to_vec(&cad).with_context(|| "writing DWG to bytes")
}

pub fn save_dwg(doc: &Document, path: &str) -> Result<()> {
    let cad = build_cad_document(doc)?;
    if path.ends_with(".dxf") {
        acadrust::DxfWriter::new(&cad)
            .write_to_file(path)
            .with_context(|| format!("writing DXF to {path}"))
    } else {
        acadrust::DwgWriter::write_to_file(path, &cad)
            .with_context(|| format!("writing DWG to {path}"))
    }
}

fn build_cad_document(doc: &Document) -> Result<acadrust::document::CadDocument> {
    use acadrust::entities::{self as ae, Entity};
    use acadrust::tables::TableEntry;
    use acadrust::types::vector::Vector3;

    let mut cad =
        acadrust::document::CadDocument::with_version(acadrust::types::DxfVersion::AC1015);

    // ── 0. Header fixes for AutoCAD compatibility ───────────────────
    // Set current layer to "0" (AutoCAD rejects null current_layer_handle)
    if let Some(layer0) = cad.layers.get("0") {
        cad.header.current_layer_handle = layer0.handle;
    }
    // Units: millimeters / metric
    cad.header.insertion_units = 4; // 4 = mm
    cad.header.measurement = 1; // 1 = metric
                                // Model space limits (reasonable default for architectural drawings in mm)
    cad.header.model_space_limits_min = acadrust::types::vector::Vector2::new(0.0, 0.0);
    cad.header.model_space_limits_max = acadrust::types::vector::Vector2::new(30000.0, 15000.0);
    for layer in &doc.layers {
        if layer.name == "0" {
            continue;
        } // already exists
        let mut al = acadrust::tables::Layer::new(&layer.name);
        al.color = to_acadrust_color(&layer.color);
        al.set_handle(cad.allocate_handle());
        let _ = cad.layers.add(al);
    }

    // ── 2. Write block definitions as BlockRecords ──────────────────
    // Pattern from acadrust examples: allocate handles for block record,
    // BLOCK entity, and ENDBLK entity; then add sub-entities with
    // owner_handle set to the block record handle.
    for (name, def) in &doc.blocks {
        let mut br = acadrust::tables::BlockRecord::new(name);
        let br_handle = cad.allocate_handle();
        br.set_handle(br_handle);
        br.block_entity_handle = cad.allocate_handle();
        br.block_end_handle = cad.allocate_handle();
        let _ = cad.block_records.add(br);

        // Translate sub-entities by -insert_point so geometry is relative to
        // the block's origin (DWG BLOCK base_point defaults to [0,0]).
        let ip = def.insert_point;
        let offset = Affine::translate((-ip.x, -ip.y));
        for (shape, shape_layer, shape_color) in &def.shapes {
            let translated = if ip.x != 0.0 || ip.y != 0.0 {
                shape.transformed(offset)
            } else {
                shape.clone()
            };
            let layer = if shape_layer.is_empty() {
                &def.default_layer
            } else {
                shape_layer
            };
            if let Some(mut et) =
                shape_to_entity_type(&translated, layer, to_acadrust_color(shape_color))
            {
                et.common_mut().owner_handle = br_handle;
                let _ = cad.add_entity(et);
            }
        }
    }

    // ── 3. Write model-space entities ───────────────────────────────
    for ent in &doc.entities {
        match &ent.shape {
            Shape::BlockInsert {
                block_name,
                position,
                rotation,
                x_scale,
                y_scale,
            } => {
                let mut ins = ae::Insert::new(
                    block_name.as_str(),
                    Vector3::new(position.x, position.y, 0.0),
                );
                ins.set_x_scale(*x_scale);
                ins.set_y_scale(*y_scale);
                ins.rotation = *rotation;
                ins.set_layer(ent.layer.clone());
                ins.set_color(to_acadrust_color(&ent.color));
                cad.add_entity(ae::EntityType::Insert(ins))?;
            }
            shape => {
                if let Some(et) =
                    shape_to_entity_type(shape, &ent.layer, to_acadrust_color(&ent.color))
                {
                    cad.add_entity(et)?;
                }
            }
        }
    }

    // ── 4. Compute model-space extents (AutoCAD rejects sentinel 1e20 values) ──
    let (mut xmin, mut ymin) = (f64::MAX, f64::MAX);
    let (mut xmax, mut ymax) = (f64::MIN, f64::MIN);
    let mut extend = |x: f64, y: f64| {
        if x < xmin {
            xmin = x;
        }
        if y < ymin {
            ymin = y;
        }
        if x > xmax {
            xmax = x;
        }
        if y > ymax {
            ymax = y;
        }
    };
    for ent in &doc.entities {
        match &ent.shape {
            Shape::Line(l) => {
                extend(l.p0.x, l.p0.y);
                extend(l.p1.x, l.p1.y);
            }
            Shape::Circle(c) => {
                extend(c.center.x - c.radius, c.center.y - c.radius);
                extend(c.center.x + c.radius, c.center.y + c.radius);
            }
            Shape::Arc { center, radius, .. } => {
                extend(center.x - radius, center.y - radius);
                extend(center.x + radius, center.y + radius);
            }
            Shape::Polyline { points, .. } => {
                for p in points {
                    extend(p.x, p.y);
                }
            }
            Shape::LwPolyline { vertices, .. } => {
                for v in vertices {
                    extend(v.point.x, v.point.y);
                }
            }
            Shape::Ellipse {
                center, major_axis, ..
            } => {
                let r = (major_axis.0.powi(2) + major_axis.1.powi(2)).sqrt();
                extend(center.x - r, center.y - r);
                extend(center.x + r, center.y + r);
            }
            Shape::Spline { control_points, .. } => {
                for p in control_points {
                    extend(p.x, p.y);
                }
            }
            Shape::Text { position, .. } | Shape::MText { position, .. } => {
                extend(position.x, position.y);
            }
            Shape::SolidFill { boundary, .. } => {
                for edge in boundary {
                    match edge {
                        FillEdge::LineTo(p) => extend(p.x, p.y),
                        FillEdge::ArcTo { center, radius, .. } => {
                            extend(center.x - radius, center.y - radius);
                            extend(center.x + radius, center.y + radius);
                        }
                        _ => {}
                    }
                }
            }
            Shape::BlockInsert {
                block_name,
                position,
                x_scale,
                y_scale,
                ..
            } => {
                // Include the insert point, and expand by block bounds if known
                extend(position.x, position.y);
                if let Some(def) = doc.blocks.get(block_name) {
                    for (shape, _, _) in &def.shapes {
                        match shape {
                            Shape::Line(l) => {
                                extend(
                                    position.x + l.p0.x * x_scale,
                                    position.y + l.p0.y * y_scale,
                                );
                                extend(
                                    position.x + l.p1.x * x_scale,
                                    position.y + l.p1.y * y_scale,
                                );
                            }
                            Shape::Circle(c) => {
                                extend(
                                    position.x + (c.center.x - c.radius) * x_scale,
                                    position.y + (c.center.y - c.radius) * y_scale,
                                );
                                extend(
                                    position.x + (c.center.x + c.radius) * x_scale,
                                    position.y + (c.center.y + c.radius) * y_scale,
                                );
                            }
                            _ => {} // approximate: insert point is sufficient for most cases
                        }
                    }
                }
            }
            Shape::CurvePath { .. } => {} // kurbo BezPath doesn't expose simple min/max
        }
    }
    if xmin < xmax && ymin < ymax {
        cad.header.model_space_extents_min = Vector3::new(xmin, ymin, 0.0);
        cad.header.model_space_extents_max = Vector3::new(xmax, ymax, 0.0);
    }

    Ok(cad)
}

/// Save a Document as a DWG overlay on top of an existing DWG file.
///
/// Loads the original DWG via acadrust (preserving all AutoCAD infrastructure:
/// object dictionaries, layouts, handle maps, class definitions), then adds
/// layers and entities from `doc` that match the given layer prefixes.
/// This produces DWGs that AutoCAD, BricsCAD, and other strict readers accept.
pub fn save_dwg_overlay(
    doc: &Document,
    original_dwg_path: &str,
    output_path: &str,
    overlay_layer_prefixes: &[&str],
) -> Result<()> {
    use acadrust::entities::{self as ae, Entity};
    use acadrust::tables::TableEntry;
    use acadrust::types::vector::Vector3;

    // Load the original DWG preserving all infrastructure
    let mut cad = acadrust::DwgReader::from_file(original_dwg_path)
        .and_then(|mut r| r.read())
        .with_context(|| format!("reading base DWG {original_dwg_path}"))?;

    // Determine which of our entities are overlay entities (by layer prefix)
    let is_overlay = |layer: &str| -> bool {
        overlay_layer_prefixes
            .iter()
            .any(|pfx| layer.starts_with(pfx))
    };

    // Add overlay layers that don't exist yet
    for layer in &doc.layers {
        if !is_overlay(&layer.name) {
            continue;
        }
        if cad.layers.get(&layer.name).is_some() {
            continue;
        }
        let mut al = acadrust::tables::Layer::new(&layer.name);
        al.color = to_acadrust_color(&layer.color);
        al.set_handle(cad.allocate_handle());
        let _ = cad.layers.add(al);
    }

    // Add overlay block definitions
    for (name, def) in &doc.blocks {
        // Only include blocks used by overlay entities
        let block_used = doc.entities.iter().any(|e| {
            if !is_overlay(&e.layer) {
                return false;
            }
            matches!(&e.shape, Shape::BlockInsert { block_name, .. } if block_name == name)
        });
        if !block_used {
            continue;
        }
        if cad.block_records.get(name).is_some() {
            continue;
        }

        let mut br = acadrust::tables::BlockRecord::new(name);
        let br_handle = cad.allocate_handle();
        br.set_handle(br_handle);
        br.block_entity_handle = cad.allocate_handle();
        br.block_end_handle = cad.allocate_handle();
        let _ = cad.block_records.add(br);

        for (shape, shape_layer, shape_color) in &def.shapes {
            let layer = if shape_layer.is_empty() {
                &def.default_layer
            } else {
                shape_layer
            };
            if let Some(mut et) = shape_to_entity_type(shape, layer, to_acadrust_color(shape_color))
            {
                et.common_mut().owner_handle = br_handle;
                let _ = cad.add_entity(et);
            }
        }
    }

    // Add overlay entities
    for ent in &doc.entities {
        if !is_overlay(&ent.layer) {
            continue;
        }
        match &ent.shape {
            Shape::BlockInsert {
                block_name,
                position,
                rotation,
                x_scale,
                y_scale,
            } => {
                let mut ins = ae::Insert::new(
                    block_name.as_str(),
                    Vector3::new(position.x, position.y, 0.0),
                );
                ins.set_x_scale(*x_scale);
                ins.set_y_scale(*y_scale);
                ins.rotation = *rotation;
                ins.set_layer(ent.layer.clone());
                ins.set_color(to_acadrust_color(&ent.color));
                cad.add_entity(ae::EntityType::Insert(ins))?;
            }
            shape => {
                if let Some(et) =
                    shape_to_entity_type(shape, &ent.layer, to_acadrust_color(&ent.color))
                {
                    cad.add_entity(et)?;
                }
            }
        }
    }

    acadrust::DwgWriter::write_to_file(output_path, &cad)
        .with_context(|| format!("writing overlay DWG to {output_path}"))
}

/// Convert a Shape to an acadrust EntityType (returns None for BlockInsert).
#[allow(clippy::field_reassign_with_default)]
pub(crate) fn shape_to_entity_type(
    shape: &Shape,
    layer: &str,
    color: acadrust::types::Color,
) -> Option<acadrust::entities::EntityType> {
    use acadrust::entities::{self as ae, Entity};
    use acadrust::types::vector::{Vector2, Vector3};

    let et = match shape {
        Shape::Line(line) => {
            let mut e = ae::Line::from_coords(line.p0.x, line.p0.y, 0.0, line.p1.x, line.p1.y, 0.0);
            e.set_layer(layer.to_string());
            e.set_color(color);
            ae::EntityType::Line(e)
        }
        Shape::Circle(circle) => {
            let mut e = ae::Circle::from_center_radius(
                Vector3::new(circle.center.x, circle.center.y, 0.0),
                circle.radius,
            );
            e.set_layer(layer.to_string());
            e.set_color(color);
            ae::EntityType::Circle(e)
        }
        Shape::Arc {
            center,
            radius,
            start_angle,
            end_angle,
        } => {
            let mut e = ae::Arc::from_center_radius_angles(
                Vector3::new(center.x, center.y, 0.0),
                *radius,
                *start_angle,
                *end_angle,
            );
            e.set_layer(layer.to_string());
            e.set_color(color);
            ae::EntityType::Arc(e)
        }
        Shape::Polyline { points, closed } => {
            let pts: Vec<Vector2> = points.iter().map(|p| Vector2::new(p.x, p.y)).collect();
            let mut e = ae::LwPolyline::from_points(pts);
            if *closed {
                e.close();
            }
            e.set_layer(layer.to_string());
            e.set_color(color);
            ae::EntityType::LwPolyline(e)
        }
        Shape::LwPolyline { vertices, closed } => {
            let mut e = ae::LwPolyline::new();
            for v in vertices {
                e.add_point_with_bulge(Vector2::new(v.point.x, v.point.y), v.bulge);
            }
            if *closed {
                e.close();
            }
            e.set_layer(layer.to_string());
            e.set_color(color);
            ae::EntityType::LwPolyline(e)
        }
        Shape::Ellipse {
            center,
            major_axis,
            minor_ratio,
            start_param,
            end_param,
        } => {
            let mut e = ae::Ellipse::from_center_axes(
                Vector3::new(center.x, center.y, 0.0),
                Vector3::new(major_axis.0, major_axis.1, 0.0),
                *minor_ratio,
            );
            e.start_parameter = *start_param;
            e.end_parameter = *end_param;
            e.set_layer(layer.to_string());
            e.set_color(color);
            ae::EntityType::Ellipse(e)
        }
        Shape::Spline {
            degree,
            knots,
            control_points,
            closed,
        } => {
            let cps: Vec<Vector3> = control_points
                .iter()
                .map(|p| Vector3::new(p.x, p.y, 0.0))
                .collect();
            let mut e = ae::Spline::from_control_points(*degree, cps);
            e.knots = knots.clone();
            e.flags.closed = *closed;
            e.set_layer(layer.to_string());
            e.set_color(color);
            ae::EntityType::Spline(e)
        }
        Shape::SolidFill { boundary, holes } => {
            let mut hatch = ae::Hatch::solid();
            hatch.pattern.name = "SOLID".to_string();
            hatch.set_layer(layer.to_string());
            hatch.set_color(color);
            let mut path = ae::BoundaryPath::external();
            fill_edges_to_boundary_path(boundary, &mut path);
            hatch.add_path(path);
            for hole in holes {
                let mut hp = ae::BoundaryPath::new();
                fill_edges_to_boundary_path(hole, &mut hp);
                hatch.add_path(hp);
            }
            ae::EntityType::Hatch(hatch)
        }
        Shape::CurvePath { path, closed } => {
            let contours = flatten_bezpath_adaptive(path, 0.1);
            // Write only the first contour as the entity type
            let contour = contours.first()?;
            if contour.len() < 2 {
                return None;
            }
            let pts: Vec<Vector2> =
                contour.iter().map(|p| Vector2::new(p.x, p.y)).collect();
            let mut e = ae::LwPolyline::from_points(pts);
            if *closed {
                e.close();
            }
            e.set_layer(layer.to_string());
            e.set_color(color);
            ae::EntityType::LwPolyline(e)
        }
        Shape::Text {
            text,
            position,
            height,
            rotation,
        } => {
            let mut e = ae::Text::default();
            e.value = text.clone();
            e.insertion_point = Vector3::new(position.x, position.y, 0.0);
            e.height = *height;
            e.rotation = *rotation;
            e.set_layer(layer.to_string());
            e.set_color(color);
            ae::EntityType::Text(e)
        }
        Shape::MText {
            text,
            position,
            height,
            rotation,
            ..
        } => {
            let mut e = ae::MText::default();
            e.value = text.clone();
            e.insertion_point = Vector3::new(position.x, position.y, 0.0);
            e.height = *height;
            e.rotation = *rotation;
            e.set_layer(layer.to_string());
            e.set_color(color);
            ae::EntityType::MText(e)
        }
        Shape::BlockInsert { .. } => return None,
    };
    Some(et)
}

// Old write_shape_to_cad removed — replaced by shape_to_entity_type above.

/// Convert FillEdge sequence to acadrust BoundaryPath edges.
pub(crate) fn fill_edges_to_boundary_path(
    edges: &[FillEdge],
    path: &mut acadrust::entities::BoundaryPath,
) {
    use acadrust::entities as ae;
    use acadrust::types::vector::{Vector2, Vector3};

    // Derive the start point from the last edge (closed boundary wraps around)
    let mut prev = match edges.last() {
        Some(FillEdge::LineTo(p)) => *p,
        Some(FillEdge::ArcTo {
            center,
            radius,
            end_angle,
            ..
        }) => Point::new(
            center.x + radius * end_angle.cos(),
            center.y + radius * end_angle.sin(),
        ),
        _ => Point::ZERO,
    };
    for edge in edges {
        match edge {
            FillEdge::LineTo(p) => {
                path.add_edge(ae::BoundaryEdge::Line(ae::LineEdge {
                    start: Vector2::new(prev.x, prev.y),
                    end: Vector2::new(p.x, p.y),
                }));
                prev = *p;
            }
            FillEdge::ArcTo {
                center,
                radius,
                start_angle,
                end_angle,
            } => {
                let mut sweep = end_angle - start_angle;
                if sweep < 0.0 {
                    sweep += 2.0 * PI;
                }
                path.add_edge(ae::BoundaryEdge::CircularArc(ae::CircularArcEdge {
                    center: Vector2::new(center.x, center.y),
                    radius: *radius,
                    start_angle: *start_angle,
                    end_angle: *end_angle,
                    counter_clockwise: sweep < PI * 2.0 && sweep > 0.0,
                }));
                prev = Point::new(
                    center.x + radius * end_angle.cos(),
                    center.y + radius * end_angle.sin(),
                );
            }
            FillEdge::EllipseArcTo {
                center,
                major_axis,
                minor_ratio,
                start_param,
                end_param,
            } => {
                path.add_edge(ae::BoundaryEdge::EllipticArc(ae::EllipticArcEdge {
                    center: Vector2::new(center.x, center.y),
                    major_axis_endpoint: Vector2::new(major_axis.0, major_axis.1),
                    minor_axis_ratio: *minor_ratio,
                    start_angle: *start_param,
                    end_angle: *end_param,
                    counter_clockwise: true,
                }));
                // Approximate prev point
                let a = (major_axis.0 * major_axis.0 + major_axis.1 * major_axis.1).sqrt();
                let angle = major_axis.1.atan2(major_axis.0);
                let ex = a * end_param.cos();
                let ey = a * minor_ratio * end_param.sin();
                prev = Point::new(
                    center.x + ex * angle.cos() - ey * angle.sin(),
                    center.y + ex * angle.sin() + ey * angle.cos(),
                );
            }
            FillEdge::SplineTo {
                degree,
                knots,
                control_points,
            } => {
                path.add_edge(ae::BoundaryEdge::Spline(ae::SplineEdge {
                    degree: *degree,
                    rational: false,
                    periodic: false,
                    knots: knots.clone(),
                    control_points: control_points
                        .iter()
                        .map(|p| Vector3::new(p.x, p.y, 1.0))
                        .collect(),
                    fit_points: Vec::new(),
                    start_tangent: Vector2::new(0.0, 0.0),
                    end_tangent: Vector2::new(0.0, 0.0),
                }));
                if let Some(last) = control_points.last() {
                    prev = *last;
                }
            }
            FillEdge::PolylineTo(pts) => {
                for p in pts {
                    path.add_edge(ae::BoundaryEdge::Line(ae::LineEdge {
                        start: Vector2::new(prev.x, prev.y),
                        end: Vector2::new(p.x, p.y),
                    }));
                    prev = *p;
                }
            }
        }
    }
}

pub(crate) fn to_acadrust_color(c: &Color) -> acadrust::types::Color {
    if c.r == 255 && c.g == 255 && c.b == 255 {
        acadrust::types::Color::ByLayer
    } else {
        acadrust::types::Color::Rgb {
            r: c.r,
            g: c.g,
            b: c.b,
        }
    }
}

// ── DWG read: entity conversion helpers ───────────────────────────────

/// Convert an LwPolyline to abstract FillEdge sequence, preserving
/// bulge arcs as FillEdge::ArcTo instead of pre-tessellating them.
pub(crate) fn lwpolyline_to_edges(
    vertices: &[acadrust::entities::LwVertex],
    closed: bool,
) -> Vec<FillEdge> {
    let n = vertices.len();
    if n == 0 {
        return Vec::new();
    }

    let mut edges = Vec::new();
    let segments = if closed { n } else { n - 1 };

    // First vertex as the starting point
    let v0 = &vertices[0];
    edges.push(FillEdge::LineTo(Point::new(v0.location.x, v0.location.y)));

    for i in 0..segments {
        let j = (i + 1) % n;
        let vi = &vertices[i];
        let vj = &vertices[j];
        let p0 = Point::new(vi.location.x, vi.location.y);
        let p1 = Point::new(vj.location.x, vj.location.y);

        if vi.bulge.abs() < 1e-10 {
            edges.push(FillEdge::LineTo(p1));
        } else {
            // Bulge arc: compute center and angles
            let bulge = vi.bulge;
            let included = 4.0 * bulge.atan();
            let dx = p1.x - p0.x;
            let dy = p1.y - p0.y;
            let chord = (dx * dx + dy * dy).sqrt();
            if chord < 1e-12 {
                edges.push(FillEdge::LineTo(p1));
                continue;
            }
            let radius = chord / (2.0 * included.sin().abs());
            let sagitta = bulge * chord / 2.0;
            let mx = (p0.x + p1.x) / 2.0;
            let my = (p0.y + p1.y) / 2.0;
            let nx = -dy / chord;
            let ny = dx / chord;
            let d = radius - sagitta.abs();
            let sign = if bulge > 0.0 { 1.0 } else { -1.0 };
            let cx = mx + sign * d * nx;
            let cy = my + sign * d * ny;

            let start_angle = (p0.y - cy).atan2(p0.x - cx);
            let end_angle = (p1.y - cy).atan2(p1.x - cx);

            edges.push(FillEdge::ArcTo {
                center: Point::new(cx, cy),
                radius,
                start_angle,
                end_angle,
            });
        }
    }
    edges
}

/// Tessellate an ellipse (or elliptic arc) into polyline points.
pub(crate) fn ellipse_to_points(
    cx: f64,
    cy: f64,
    major_x: f64,
    major_y: f64,
    minor_ratio: f64,
    start_param: f64,
    end_param: f64,
) -> Vec<Point> {
    let major_len = (major_x * major_x + major_y * major_y).sqrt();
    if major_len < 1e-12 {
        return Vec::new();
    }

    let rot = major_y.atan2(major_x);
    let cos_r = rot.cos();
    let sin_r = rot.sin();
    let a = major_len;
    let b = major_len * minor_ratio;

    let mut sweep = end_param - start_param;
    if sweep <= 0.0 {
        sweep += 2.0 * PI;
    }

    let circumference_approx = PI * (3.0 * (a + b) - ((3.0 * a + b) * (a + 3.0 * b)).sqrt());
    let steps = ((circumference_approx * sweep / (2.0 * PI) / 2.0).ceil() as usize).clamp(16, 128);

    let mut pts = Vec::with_capacity(steps + 1);
    for i in 0..=steps {
        let t = start_param + sweep * i as f64 / steps as f64;
        let lx = a * t.cos();
        let ly = b * t.sin();
        pts.push(Point::new(
            cx + lx * cos_r - ly * sin_r,
            cy + lx * sin_r + ly * cos_r,
        ));
    }
    pts
}

/// Evaluate a B-spline curve and return tessellated points.
pub(crate) fn spline_to_points(
    degree: i32,
    knots: &[f64],
    control_points: &[acadrust::types::Vector3],
    fit_points: &[acadrust::types::Vector3],
    _closed: bool,
) -> Vec<Point> {
    let d = degree as usize;

    // If we have control points and knots, use De Boor evaluation
    if control_points.len() >= 2 && knots.len() > control_points.len() + d {
        let n = control_points.len();
        let _k = d + 1; // order
        let t_min = knots[d];
        let t_max = knots[n];
        if (t_max - t_min).abs() < 1e-12 {
            return Vec::new();
        }

        let steps = (n * 8).clamp(32, 256);
        let mut pts = Vec::with_capacity(steps + 1);
        for i in 0..=steps {
            let t = t_min + (t_max - t_min) * i as f64 / steps as f64;
            let t = t.min(t_max - 1e-10); // avoid endpoint issues
            let p = de_boor(d, knots, control_points, t);
            pts.push(p);
        }
        return pts;
    }

    // Fallback: use fit points as polyline
    if fit_points.len() >= 2 {
        return fit_points.iter().map(|p| Point::new(p.x, p.y)).collect();
    }

    // Last resort: connect control points directly
    control_points
        .iter()
        .map(|p| Point::new(p.x, p.y))
        .collect()
}

/// De Boor's algorithm for B-spline evaluation at parameter t.
pub(crate) fn de_boor(
    degree: usize,
    knots: &[f64],
    cps: &[acadrust::types::Vector3],
    t: f64,
) -> Point {
    // Find knot span
    let n = cps.len();
    let mut k = degree;
    for i in degree..n {
        if t >= knots[i] && t < knots[i + 1] {
            k = i;
            break;
        }
    }

    let mut d: Vec<[f64; 2]> = (0..=degree)
        .map(|j| {
            let idx = (k as isize - degree as isize + j as isize) as usize;
            let idx = idx.min(n - 1);
            [cps[idx].x, cps[idx].y]
        })
        .collect();

    for r in 1..=degree {
        for j in (r..=degree).rev() {
            let idx = k as isize - degree as isize + j as isize;
            let left = knots[idx as usize];
            let right = knots[idx as usize + degree + 1 - r];
            let denom = right - left;
            if denom.abs() < 1e-12 {
                continue;
            }
            let alpha = (t - left) / denom;
            d[j][0] = (1.0 - alpha) * d[j - 1][0] + alpha * d[j][0];
            d[j][1] = (1.0 - alpha) * d[j - 1][1] + alpha * d[j][1];
        }
    }

    Point::new(d[degree][0], d[degree][1])
}

/// Convert a hatch boundary path to abstract FillEdge sequence.
/// Arcs stay parametric; lines and polylines become LineTo/PolylineTo.
/// Convert a hatch boundary path to abstract FillEdge sequence.
/// All edge types stored parametrically for zoom-adaptive rendering.
pub(crate) fn boundary_path_to_edges(path: &acadrust::entities::BoundaryPath) -> Vec<FillEdge> {
    let mut edges: Vec<FillEdge> = Vec::new();

    for edge in &path.edges {
        match edge {
            acadrust::entities::BoundaryEdge::Line(le) => {
                edges.push(FillEdge::LineTo(Point::new(le.start.x, le.start.y)));
                edges.push(FillEdge::LineTo(Point::new(le.end.x, le.end.y)));
            }
            acadrust::entities::BoundaryEdge::Polyline(pe) => {
                let verts: Vec<acadrust::entities::LwVertex> = pe
                    .vertices
                    .iter()
                    .map(|v| acadrust::entities::LwVertex {
                        location: acadrust::types::Vector2::new(v.x, v.y),
                        bulge: v.z,
                        start_width: 0.0,
                        end_width: 0.0,
                    })
                    .collect();
                // Preserve bulge arcs as abstract ArcTo edges
                edges.extend(lwpolyline_to_edges(&verts, pe.is_closed));
            }
            acadrust::entities::BoundaryEdge::CircularArc(ca) => {
                let (start, end) = if ca.counter_clockwise {
                    (ca.start_angle, ca.end_angle)
                } else {
                    (ca.end_angle, ca.start_angle)
                };
                edges.push(FillEdge::ArcTo {
                    center: Point::new(ca.center.x, ca.center.y),
                    radius: ca.radius,
                    start_angle: start,
                    end_angle: end,
                });
            }
            acadrust::entities::BoundaryEdge::EllipticArc(ea) => {
                edges.push(FillEdge::EllipseArcTo {
                    center: Point::new(ea.center.x, ea.center.y),
                    major_axis: (ea.major_axis_endpoint.x, ea.major_axis_endpoint.y),
                    minor_ratio: ea.minor_axis_ratio,
                    start_param: ea.start_angle,
                    end_param: ea.end_angle,
                });
            }
            acadrust::entities::BoundaryEdge::Spline(se) => {
                let cps: Vec<Point> = se
                    .control_points
                    .iter()
                    .map(|p| Point::new(p.x, p.y))
                    .collect();
                if !cps.is_empty() && se.knots.len() > cps.len() {
                    edges.push(FillEdge::SplineTo {
                        degree: se.degree,
                        knots: se.knots.clone(),
                        control_points: cps,
                    });
                } else {
                    // Fallback: fit points as polyline
                    let pts: Vec<Point> =
                        se.fit_points.iter().map(|p| Point::new(p.x, p.y)).collect();
                    if !pts.is_empty() {
                        edges.push(FillEdge::PolylineTo(pts));
                    }
                }
            }
        }
    }

    edges
}

/// Convert a hatch boundary path to renderable shapes (lines, arcs, polylines).
pub(crate) fn hatch_boundary_to_shapes(path: &acadrust::entities::BoundaryPath) -> Vec<Shape> {
    let mut shapes = Vec::new();
    for edge in &path.edges {
        match edge {
            acadrust::entities::BoundaryEdge::Line(le) => {
                shapes.push(Shape::Line(Line::new(
                    Point::new(le.start.x, le.start.y),
                    Point::new(le.end.x, le.end.y),
                )));
            }
            acadrust::entities::BoundaryEdge::CircularArc(ca) => {
                let (start, end) = if ca.counter_clockwise {
                    (ca.start_angle, ca.end_angle)
                } else {
                    (ca.end_angle, ca.start_angle)
                };
                shapes.push(Shape::Arc {
                    center: Point::new(ca.center.x, ca.center.y),
                    radius: ca.radius,
                    start_angle: start,
                    end_angle: end,
                });
            }
            acadrust::entities::BoundaryEdge::Polyline(pe) => {
                // Convert to abstract shapes, preserving bulge arcs
                let verts: Vec<acadrust::entities::LwVertex> = pe
                    .vertices
                    .iter()
                    .map(|v| acadrust::entities::LwVertex {
                        location: acadrust::types::Vector2::new(v.x, v.y),
                        bulge: v.z,
                        start_width: 0.0,
                        end_width: 0.0,
                    })
                    .collect();
                let n = verts.len();
                let segments = if pe.is_closed { n } else { n.saturating_sub(1) };
                for i in 0..segments {
                    let j = (i + 1) % n;
                    let p0 = Point::new(verts[i].location.x, verts[i].location.y);
                    let p1 = Point::new(verts[j].location.x, verts[j].location.y);
                    if verts[i].bulge.abs() < 1e-10 {
                        shapes.push(Shape::Line(Line::new(p0, p1)));
                    } else {
                        let bulge = verts[i].bulge;
                        let included = 4.0 * bulge.atan();
                        let dx = p1.x - p0.x;
                        let dy = p1.y - p0.y;
                        let chord = (dx * dx + dy * dy).sqrt();
                        if chord < 1e-12 {
                            continue;
                        }
                        let radius = chord / (2.0 * included.sin().abs());
                        let sagitta = bulge * chord / 2.0;
                        let mx = (p0.x + p1.x) / 2.0;
                        let my = (p0.y + p1.y) / 2.0;
                        let nx = -dy / chord;
                        let ny = dx / chord;
                        let d = radius - sagitta.abs();
                        let sign = if bulge > 0.0 { 1.0 } else { -1.0 };
                        let cx = mx + sign * d * nx;
                        let cy = my + sign * d * ny;
                        shapes.push(Shape::Arc {
                            center: Point::new(cx, cy),
                            radius,
                            start_angle: (p0.y - cy).atan2(p0.x - cx),
                            end_angle: (p1.y - cy).atan2(p1.x - cx),
                        });
                    }
                }
            }
            acadrust::entities::BoundaryEdge::EllipticArc(ea) => {
                let pts = ellipse_to_points(
                    ea.center.x,
                    ea.center.y,
                    ea.major_axis_endpoint.x,
                    ea.major_axis_endpoint.y,
                    ea.minor_axis_ratio,
                    ea.start_angle,
                    ea.end_angle,
                );
                if pts.len() >= 2 {
                    shapes.push(Shape::Polyline {
                        points: pts,
                        closed: false,
                    });
                }
            }
            acadrust::entities::BoundaryEdge::Spline(se) => {
                // Convert spline edge control points to Vector3 for reuse
                let cps: Vec<acadrust::types::Vector3> = se
                    .control_points
                    .iter()
                    .map(|p| acadrust::types::Vector3::new(p.x, p.y, p.z))
                    .collect();
                let fit: Vec<acadrust::types::Vector3> = se
                    .fit_points
                    .iter()
                    .map(|p| acadrust::types::Vector3::new(p.x, p.y, 0.0))
                    .collect();
                let pts = spline_to_points(se.degree, &se.knots, &cps, &fit, false);
                if pts.len() >= 2 {
                    shapes.push(Shape::Polyline {
                        points: pts,
                        closed: false,
                    });
                }
            }
        }
    }
    shapes
}

pub(crate) struct RawEntity {
    owner: acadrust::Handle,
    layer: String,
    color: acadrust::Color,
    shape: Shape,
}

pub(crate) struct InsertRef {
    block_name: String,
    insert_x: f64,
    insert_y: f64,
    x_scale: f64,
    y_scale: f64,
    rotation: f64,
    layer: String,
    color: acadrust::Color,
    owner: acadrust::Handle,
}

pub(crate) fn build_document(cad: acadrust::CadDocument) -> Result<Document> {
    use acadrust::entities::EntityType;

    let model_space_handle = cad
        .block_records
        .iter()
        .find(|br| br.name == "*Model_Space")
        .map(|br| br.handle);

    let mut handle_to_block: HashMap<u64, String> = HashMap::new();
    for br in cad.block_records.iter() {
        handle_to_block.insert(br.handle.value(), br.name.clone());
    }

    let mut layer_colors: HashMap<String, Color> = HashMap::new();
    let mut layers = Vec::new();
    for layer in cad.layers.iter() {
        let color = resolve_acadrust_color(&layer.color);
        layer_colors.insert(layer.name.clone(), color);
        layers.push(Layer {
            name: layer.name.clone(),
            color,
            visible: true,
        });
    }

    let mut raw_entities: Vec<RawEntity> = Vec::new();
    let mut inserts: Vec<InsertRef> = Vec::new();

    for ent in cad.entities() {
        match ent {
            EntityType::Line(line) => {
                raw_entities.push(RawEntity {
                    owner: line.common.owner_handle,
                    layer: line.common.layer.clone(),
                    color: line.common.color,
                    shape: Shape::Line(Line::new(
                        Point::new(line.start.x, line.start.y),
                        Point::new(line.end.x, line.end.y),
                    )),
                });
            }
            EntityType::Arc(arc) => {
                raw_entities.push(RawEntity {
                    owner: arc.common.owner_handle,
                    layer: arc.common.layer.clone(),
                    color: arc.common.color,
                    shape: Shape::Arc {
                        center: Point::new(arc.center.x, arc.center.y),
                        radius: arc.radius,
                        start_angle: arc.start_angle,
                        end_angle: arc.end_angle,
                    },
                });
            }
            EntityType::Circle(circle) => {
                raw_entities.push(RawEntity {
                    owner: circle.common.owner_handle,
                    layer: circle.common.layer.clone(),
                    color: circle.common.color,
                    shape: Shape::Circle(Circle::new(
                        Point::new(circle.center.x, circle.center.y),
                        circle.radius,
                    )),
                });
            }
            EntityType::LwPolyline(lwp) => {
                let has_bulge = lwp.vertices.iter().any(|v| v.bulge.abs() > 1e-10);
                let shape = if has_bulge {
                    Shape::LwPolyline {
                        vertices: lwp
                            .vertices
                            .iter()
                            .map(|v| LwVertex {
                                point: Point::new(v.location.x, v.location.y),
                                bulge: v.bulge,
                            })
                            .collect(),
                        closed: lwp.is_closed,
                    }
                } else {
                    let pts: Vec<Point> = lwp
                        .vertices
                        .iter()
                        .map(|v| Point::new(v.location.x, v.location.y))
                        .collect();
                    Shape::Polyline {
                        points: pts,
                        closed: lwp.is_closed,
                    }
                };
                raw_entities.push(RawEntity {
                    owner: lwp.common.owner_handle,
                    layer: lwp.common.layer.clone(),
                    color: lwp.common.color,
                    shape,
                });
            }
            EntityType::Ellipse(ell) => {
                raw_entities.push(RawEntity {
                    owner: ell.common.owner_handle,
                    layer: ell.common.layer.clone(),
                    color: ell.common.color,
                    shape: Shape::Ellipse {
                        center: Point::new(ell.center.x, ell.center.y),
                        major_axis: (ell.major_axis.x, ell.major_axis.y),
                        minor_ratio: ell.minor_axis_ratio,
                        start_param: ell.start_parameter,
                        end_param: ell.end_parameter,
                    },
                });
            }
            EntityType::Spline(spl) => {
                let cps: Vec<Point> = spl
                    .control_points
                    .iter()
                    .map(|p| Point::new(p.x, p.y))
                    .collect();
                let shape = if !cps.is_empty() && spl.knots.len() > cps.len() {
                    Shape::Spline {
                        degree: spl.degree,
                        knots: spl.knots.clone(),
                        control_points: cps,
                        closed: spl.flags.closed,
                    }
                } else {
                    // Fit points only: convert to smooth CurvePath via Catmull-Rom
                    let fit_pts: Vec<Point> = spl
                        .fit_points
                        .iter()
                        .map(|p| Point::new(p.x, p.y))
                        .collect();
                    if fit_pts.len() >= 2 {
                        Shape::CurvePath {
                            path: catmull_rom_to_bezpath(&fit_pts, spl.flags.closed),
                            closed: spl.flags.closed,
                        }
                    } else {
                        Shape::Polyline {
                            points: fit_pts,
                            closed: spl.flags.closed,
                        }
                    }
                };
                raw_entities.push(RawEntity {
                    owner: spl.common.owner_handle,
                    layer: spl.common.layer.clone(),
                    color: spl.common.color,
                    shape,
                });
            }
            EntityType::Hatch(hatch) => {
                #[cfg(debug_assertions)]
                {
                    eprintln!(
                        "HATCH layer={} solid={} pattern={:?} scale={} angle={:.2} paths={} lines={}",
                        hatch.common.layer, hatch.is_solid,
                        hatch.pattern.name, hatch.pattern_scale, hatch.pattern_angle,
                        hatch.paths.len(), hatch.pattern.lines.len(),
                    );
                    if !hatch.pattern.lines.is_empty()
                        && (hatch.common.layer.contains("FLOOR")
                            || hatch.common.layer.contains("WALL"))
                    {
                        for (i, line) in hatch.pattern.lines.iter().enumerate() {
                            eprintln!(
                                "  line[{i}] angle={:.4} base=({:.1},{:.1}) offset=({:.1},{:.1}) dashes={:?}",
                                line.angle, line.base_point.x, line.base_point.y,
                                line.offset.x, line.offset.y, line.dash_lengths,
                            );
                        }
                    }
                }

                // Extract ordered boundary polygons directly from acadrust
                // boundary path data (not from our Shape representations).
                let mut path_edge_lists: Vec<Vec<FillEdge>> = Vec::new();
                // Coarse-flattened polygons for containment checks and pattern fill
                let mut path_polygons: Vec<Vec<Point>> = Vec::new();
                for path in &hatch.paths {
                    // Emit boundary geometry for rendering (stroked edges)
                    for shape in hatch_boundary_to_shapes(path) {
                        raw_entities.push(RawEntity {
                            owner: hatch.common.owner_handle,
                            layer: hatch.common.layer.clone(),
                            color: hatch.common.color,
                            shape,
                        });
                    }

                    // Store abstract edges for the fill boundary
                    let edges = boundary_path_to_edges(path);
                    let polygon = flatten_fill_edges(&edges, 1.0);
                    if polygon.len() >= 3 {
                        path_edge_lists.push(edges);
                        path_polygons.push(polygon);
                    }
                }

                // Classify paths by containment: a path whose centroid lies
                // inside a larger path is a hole in that path. Paths that
                // don't contain each other are independent fill regions.
                //
                // For solid fills: fill each independent region, cutting holes.
                // For pattern fills: clip pattern to each region independently.
                let mut areas: Vec<(usize, f64)> = path_polygons
                    .iter()
                    .enumerate()
                    .map(|(i, p)| (i, signed_polygon_area(p).abs()))
                    .collect();
                areas.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

                // Skip truly degenerate paths (< 1 mm² area)
                let valid_paths: Vec<(usize, f64)> =
                    areas.into_iter().filter(|(_, a)| *a > 1.0).collect();

                // For each valid path, check if its centroid is inside any
                // larger valid path. If so, it's a hole in that path.
                let mut parent: Vec<Option<usize>> = vec![None; path_polygons.len()];
                for (idx, (i, _)) in valid_paths.iter().enumerate() {
                    let poly = &path_polygons[*i];
                    let cx = poly.iter().map(|p| p.x).sum::<f64>() / poly.len() as f64;
                    let cy = poly.iter().map(|p| p.y).sum::<f64>() / poly.len() as f64;
                    let centroid = Point::new(cx, cy);
                    // Check larger paths (earlier in sorted list)
                    for (larger_i, _) in &valid_paths[..idx] {
                        if geo::point_in_polygon(centroid, &path_polygons[*larger_i]) {
                            parent[*i] = Some(*larger_i);
                            break; // contained in the smallest enclosing path
                        }
                    }
                }

                // Group: outer paths (no parent) get their holes
                let outer_paths: Vec<usize> = valid_paths
                    .iter()
                    .map(|(i, _)| *i)
                    .filter(|i| parent[*i].is_none())
                    .collect();

                // For each outer path, collect its direct holes (abstract edges)
                let mut regions: Vec<(Vec<FillEdge>, Vec<Vec<FillEdge>>)> = Vec::new();
                for &oi in &outer_paths {
                    let holes: Vec<Vec<FillEdge>> = valid_paths
                        .iter()
                        .filter(|(i, _)| parent[*i] == Some(oi))
                        .map(|(i, _)| path_edge_lists[*i].clone())
                        .collect();
                    regions.push((path_edge_lists[oi].clone(), holes));
                }
                for (i, _) in &valid_paths {
                    if parent[*i].is_some()
                        && parent[*i].map(|p| parent[p].is_some()).unwrap_or(false)
                    {
                        regions.push((path_edge_lists[*i].clone(), Vec::new()));
                    }
                }

                // Use the first region as the primary (for pattern fill clipping).
                // Flatten at coarse tolerance for clipping (not rendering).
                let _boundary_pts = if !regions.is_empty() {
                    flatten_fill_edges(&regions[0].0, 1.0)
                } else {
                    Vec::new()
                };
                let _hole_polygons: Vec<Vec<Point>> = if !regions.is_empty() {
                    regions[0]
                        .1
                        .iter()
                        .map(|h| flatten_fill_edges(h, 1.0))
                        .collect()
                } else {
                    Vec::new()
                };

                // Generate hatch fill lines (non-solid hatches with pattern lines).
                // Each hatch boundary path defines a clip region. Fill each
                // non-degenerate path independently using the original path
                // polygons (before containment classification).
                if !hatch.is_solid && !hatch.pattern.lines.is_empty() {
                    for poly in &path_polygons {
                        if poly.len() < 3 {
                            continue;
                        }
                        let (px0, py0, px1, py1) = geo::bounds_of(poly);
                        let pw = px1 - px0;
                        let ph = py1 - py0;
                        // Skip degenerate: must have meaningful area in BOTH dimensions
                        if pw < 10.0 || ph < 10.0 {
                            continue;
                        }
                        let fill_shapes = generate_dwg_hatch_fill(
                            poly,
                            &hatch.pattern,
                            hatch.pattern_angle,
                            hatch.pattern_scale,
                            hatch.is_double,
                        );
                        for shape in fill_shapes {
                            raw_entities.push(RawEntity {
                                owner: hatch.common.owner_handle,
                                layer: hatch.common.layer.clone(),
                                color: hatch.common.color,
                                shape,
                            });
                        }
                    }
                }

                // Solid fills: triangulated polygon fill for each region.
                // Skip dimension text backgrounds (small mask rectangles).
                let is_dim_bg = hatch.common.layer.contains("DIMENSION");
                if hatch.is_solid && !is_dim_bg {
                    for (region_boundary, region_holes) in &regions {
                        if region_boundary.is_empty() {
                            continue;
                        }
                        let (bx0, by0, bx1, by1) = fill_edges_bbox(region_boundary, region_holes);
                        let bw = bx1 - bx0;
                        let bh = by1 - by0;
                        if bw < 10.0 && bh < 10.0 {
                            continue;
                        }
                        if region_boundary.len() >= 2 {
                            raw_entities.push(RawEntity {
                                owner: hatch.common.owner_handle,
                                layer: hatch.common.layer.clone(),
                                color: hatch.common.color,
                                shape: Shape::SolidFill {
                                    boundary: region_boundary.clone(),
                                    holes: region_holes.clone(),
                                },
                            });
                        }
                    }
                }
            }
            EntityType::MText(mt) => {
                let plain = mtext::parse(&mt.value).plain_text();
                if !plain.is_empty() {
                    raw_entities.push(RawEntity {
                        owner: mt.common.owner_handle,
                        layer: mt.common.layer.clone(),
                        color: mt.common.color,
                        shape: Shape::MText {
                            text: mt.value.clone(),
                            plain_text: plain,
                            position: Point::new(mt.insertion_point.x, mt.insertion_point.y),
                            height: mt.height,
                            rotation: mt.rotation,
                        },
                    });
                }
            }
            EntityType::Text(txt) => {
                if !txt.value.is_empty() {
                    raw_entities.push(RawEntity {
                        owner: txt.common.owner_handle,
                        layer: txt.common.layer.clone(),
                        color: txt.common.color,
                        shape: Shape::Text {
                            text: txt.value.clone(),
                            position: Point::new(txt.insertion_point.x, txt.insertion_point.y),
                            height: txt.height,
                            rotation: txt.rotation,
                        },
                    });
                }
            }
            EntityType::Dimension(dim) => {
                // Dimensions reference anonymous blocks (*D1, *D2, ...) containing
                // their rendered geometry. Treat as insert at origin.
                let block_name = dim.base().block_name.clone();
                if !block_name.is_empty() {
                    inserts.push(InsertRef {
                        block_name,
                        insert_x: 0.0,
                        insert_y: 0.0,
                        x_scale: 1.0,
                        y_scale: 1.0,
                        rotation: 0.0,
                        layer: dim.base().common.layer.clone(),
                        color: dim.base().common.color,
                        owner: dim.base().common.owner_handle,
                    });
                }
            }
            EntityType::Insert(ins) => {
                inserts.push(InsertRef {
                    block_name: ins.block_name.clone(),
                    insert_x: ins.insert_point.x,
                    insert_y: ins.insert_point.y,
                    x_scale: ins.x_scale(),
                    y_scale: ins.y_scale(),
                    rotation: ins.rotation,
                    layer: ins.common.layer.clone(),
                    color: ins.common.color,
                    owner: ins.common.owner_handle,
                });
            }
            _ => continue,
        }
    }

    let mut block_entities: HashMap<String, Vec<usize>> = HashMap::new();
    for (i, raw) in raw_entities.iter().enumerate() {
        if let Some(block_name) = handle_to_block.get(&raw.owner.value()) {
            // Allow named blocks and anonymous dimension blocks (*D*)
            if !block_name.starts_with('*') || block_name.starts_with("*D") {
                block_entities
                    .entry(block_name.clone())
                    .or_default()
                    .push(i);
            }
        }
    }

    let ms = model_space_handle;
    let mut doc = Document::new();
    doc.layers = layers;

    // Model-space geometry
    for raw in &raw_entities {
        if ms.is_some_and(|ms| raw.owner == ms) {
            let color = resolve_entity_color(&raw.color, &raw.layer, &layer_colors);
            let id = doc.alloc_id();
            doc.entities.push(DrawEntity {
                id,
                layer: raw.layer.clone(),
                color,
                shape: raw.shape.clone(),
            });
        }
    }

    // Build block definitions from raw entities
    for (block_name, indices) in &block_entities {
        let shapes: Vec<(Shape, String, Color)> = indices
            .iter()
            .map(|&idx| {
                let raw = &raw_entities[idx];
                let color = resolve_entity_color(&raw.color, &raw.layer, &layer_colors);
                (raw.shape.clone(), raw.layer.clone(), color)
            })
            .collect();
        doc.blocks.insert(
            block_name.clone(),
            BlockDef {
                name: block_name.clone(),
                shapes,
                insert_point: Point::ZERO,
                default_layer: "0".to_string(),
            },
        );
    }

    // Store block inserts as references (not flattened)
    for ins in &inserts {
        if ms.is_none_or(|ms| ins.owner != ms) {
            continue;
        }
        if !block_entities.contains_key(&ins.block_name) {
            continue;
        }
        let color = resolve_entity_color(&ins.color, &ins.layer, &layer_colors);
        let id = doc.alloc_id();
        doc.entities.push(DrawEntity {
            id,
            layer: ins.layer.clone(),
            color,
            shape: Shape::BlockInsert {
                block_name: ins.block_name.clone(),
                position: Point::new(ins.insert_x, ins.insert_y),
                rotation: ins.rotation,
                x_scale: ins.x_scale,
                y_scale: ins.y_scale,
            },
        });
    }

    Ok(doc)
}

pub(crate) fn resolve_entity_color(
    entity_color: &acadrust::Color,
    layer_name: &str,
    layer_colors: &HashMap<String, Color>,
) -> Color {
    match entity_color {
        acadrust::Color::ByLayer => layer_colors
            .get(layer_name)
            .copied()
            .unwrap_or(Color::WHITE),
        acadrust::Color::ByBlock => Color::WHITE,
        acadrust::Color::Index(i) => aci_to_rgb(*i),
        acadrust::Color::Rgb { r, g, b } => Color::rgb(*r, *g, *b),
    }
}

pub(crate) fn resolve_acadrust_color(color: &acadrust::Color) -> Color {
    match color {
        acadrust::Color::ByLayer | acadrust::Color::ByBlock => Color::WHITE,
        acadrust::Color::Index(i) => aci_to_rgb(*i),
        acadrust::Color::Rgb { r, g, b } => Color::rgb(*r, *g, *b),
    }
}
