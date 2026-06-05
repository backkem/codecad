use kurbo::{Affine, BezPath, Circle, Line, Point, Shape as KurboShape};
use serde::{Deserialize, Serialize};
use std::f64::consts::PI;

use crate::tessellate::tessellate_spline;

// ── Types ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct EntityId(pub u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl Color {
    pub const WHITE: Color = Color {
        r: 255,
        g: 255,
        b: 255,
        a: 255,
    };

    pub fn rgb(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b, a: 255 }
    }

    pub fn to_f32_array(self) -> [f32; 4] {
        [
            self.r as f32 / 255.0,
            self.g as f32 / 255.0,
            self.b as f32 / 255.0,
            self.a as f32 / 255.0,
        ]
    }
}

/// An edge in a fill boundary path. Stores abstract geometry so the
/// renderer can flatten at zoom-adaptive tolerance.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum FillEdge {
    LineTo(Point),
    ArcTo {
        center: Point,
        radius: f64,
        start_angle: f64,
        end_angle: f64,
    },
    EllipseArcTo {
        center: Point,
        major_axis: (f64, f64),
        minor_ratio: f64,
        start_param: f64,
        end_param: f64,
    },
    SplineTo {
        degree: i32,
        knots: Vec<f64>,
        control_points: Vec<Point>,
    },
    PolylineTo(Vec<Point>),
}

/// LwPolyline vertex: position + bulge (0 = straight, nonzero = arc).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LwVertex {
    pub point: Point,
    pub bulge: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Shape {
    Line(Line),
    Circle(Circle),
    Arc {
        center: Point,
        radius: f64,
        start_angle: f64, // radians
        end_angle: f64,   // radians
    },
    Polyline {
        points: Vec<Point>,
        closed: bool,
    },
    /// LwPolyline with bulge arcs. Source data from DWG, tessellated at render time.
    LwPolyline {
        vertices: Vec<LwVertex>,
        closed: bool,
    },
    /// Filled polygon (from DWG solid hatches). Boundary and holes are
    /// abstract edge sequences; triangulation is deferred to render time.
    SolidFill {
        boundary: Vec<FillEdge>,
        holes: Vec<Vec<FillEdge>>,
    },
    /// BezPath curves (text glyph outlines, splines, ellipses, etc).
    /// Flattened to polylines at render time with zoom-adaptive tolerance.
    CurvePath {
        path: BezPath,
        closed: bool,
    },
    /// Ellipse or elliptic arc (parametric).
    Ellipse {
        center: Point,
        major_axis: (f64, f64), // endpoint relative to center
        minor_ratio: f64,
        start_param: f64, // radians
        end_param: f64,   // radians
    },
    /// B-spline curve (De Boor evaluation at render time).
    Spline {
        degree: i32,
        knots: Vec<f64>,
        control_points: Vec<Point>,
        closed: bool,
    },
    /// Block insert reference. Not flattened at load time; expanded
    /// at render time with cached transforms.
    BlockInsert {
        block_name: String,
        position: Point,
        rotation: f64, // radians
        x_scale: f64,
        y_scale: f64,
    },
    /// Text entity (single-line). Rendered to glyphs at render time.
    Text {
        text: String,
        position: Point,
        height: f64,
        rotation: f64, // radians
    },
    /// MText entity (multi-line, formatted). Rendered to glyphs at render time.
    MText {
        text: String,       // raw MText with formatting codes
        plain_text: String, // stripped text for rendering
        position: Point,
        height: f64,
        rotation: f64, // radians
    },
}

impl Shape {
    pub fn transformed(&self, xform: Affine) -> Shape {
        match self {
            Shape::Line(l) => Shape::Line(Line::new(xform * l.p0, xform * l.p1)),
            Shape::Circle(c) => {
                let center = xform * c.center;
                let coeffs = xform.as_coeffs();
                let sx = (coeffs[0] * coeffs[0] + coeffs[1] * coeffs[1]).sqrt();
                Shape::Circle(Circle::new(center, c.radius * sx))
            }
            Shape::Arc {
                center,
                radius,
                start_angle,
                end_angle,
            } => {
                let new_center = xform * *center;
                let coeffs = xform.as_coeffs();
                let sx = (coeffs[0] * coeffs[0] + coeffs[1] * coeffs[1]).sqrt();
                // Recalculate angles from transformed arc endpoints
                let start_pt = Point::new(
                    center.x + radius * start_angle.cos(),
                    center.y + radius * start_angle.sin(),
                );
                let end_pt = Point::new(
                    center.x + radius * end_angle.cos(),
                    center.y + radius * end_angle.sin(),
                );
                let new_start_pt = xform * start_pt;
                let new_end_pt = xform * end_pt;
                let mut new_start =
                    (new_start_pt.y - new_center.y).atan2(new_start_pt.x - new_center.x);
                let mut new_end = (new_end_pt.y - new_center.y).atan2(new_end_pt.x - new_center.x);
                // Reflections (negative determinant) reverse winding
                let det = coeffs[0] * coeffs[3] - coeffs[1] * coeffs[2];
                if det < 0.0 {
                    std::mem::swap(&mut new_start, &mut new_end);
                }
                Shape::Arc {
                    center: new_center,
                    radius: radius * sx,
                    start_angle: new_start,
                    end_angle: new_end,
                }
            }
            Shape::Polyline { points, closed } => {
                let pts = points.iter().map(|&p| xform * p).collect();
                Shape::Polyline {
                    points: pts,
                    closed: *closed,
                }
            }
            Shape::LwPolyline { vertices, closed } => Shape::LwPolyline {
                vertices: vertices
                    .iter()
                    .map(|v| LwVertex {
                        point: xform * v.point,
                        bulge: v.bulge,
                    })
                    .collect(),
                closed: *closed,
            },
            Shape::SolidFill { boundary, holes } => {
                let coeffs = xform.as_coeffs();
                let sx = (coeffs[0] * coeffs[0] + coeffs[1] * coeffs[1]).sqrt();
                let det = coeffs[0] * coeffs[3] - coeffs[1] * coeffs[2];
                let transform_edges = |edges: &[FillEdge]| -> Vec<FillEdge> {
                    edges
                        .iter()
                        .map(|e| match e {
                            FillEdge::LineTo(p) => FillEdge::LineTo(xform * *p),
                            FillEdge::ArcTo {
                                center,
                                radius,
                                start_angle,
                                end_angle,
                            } => {
                                let new_center = xform * *center;
                                let sp = Point::new(
                                    center.x + radius * start_angle.cos(),
                                    center.y + radius * start_angle.sin(),
                                );
                                let ep = Point::new(
                                    center.x + radius * end_angle.cos(),
                                    center.y + radius * end_angle.sin(),
                                );
                                let nsp = xform * sp;
                                let nep = xform * ep;
                                let mut ns = (nsp.y - new_center.y).atan2(nsp.x - new_center.x);
                                let mut ne = (nep.y - new_center.y).atan2(nep.x - new_center.x);
                                if det < 0.0 {
                                    std::mem::swap(&mut ns, &mut ne);
                                }
                                FillEdge::ArcTo {
                                    center: new_center,
                                    radius: radius * sx,
                                    start_angle: ns,
                                    end_angle: ne,
                                }
                            }
                            FillEdge::EllipseArcTo {
                                center,
                                major_axis,
                                minor_ratio,
                                start_param,
                                end_param,
                            } => {
                                let nc = xform * *center;
                                let ma_pt =
                                    Point::new(center.x + major_axis.0, center.y + major_axis.1);
                                let nma = xform * ma_pt;
                                FillEdge::EllipseArcTo {
                                    center: nc,
                                    major_axis: (nma.x - nc.x, nma.y - nc.y),
                                    minor_ratio: *minor_ratio,
                                    start_param: *start_param,
                                    end_param: *end_param,
                                }
                            }
                            FillEdge::SplineTo {
                                degree,
                                knots,
                                control_points,
                            } => FillEdge::SplineTo {
                                degree: *degree,
                                knots: knots.clone(),
                                control_points: control_points.iter().map(|&p| xform * p).collect(),
                            },
                            FillEdge::PolylineTo(pts) => {
                                FillEdge::PolylineTo(pts.iter().map(|&p| xform * p).collect())
                            }
                        })
                        .collect()
                };
                Shape::SolidFill {
                    boundary: transform_edges(boundary),
                    holes: holes.iter().map(|h| transform_edges(h)).collect(),
                }
            }
            Shape::CurvePath { path, closed } => {
                let mut t = path.clone();
                t.apply_affine(xform);
                Shape::CurvePath {
                    path: t,
                    closed: *closed,
                }
            }
            Shape::Ellipse {
                center,
                major_axis,
                minor_ratio,
                start_param,
                end_param,
            } => {
                let new_center = xform * *center;
                let ma_pt = Point::new(center.x + major_axis.0, center.y + major_axis.1);
                let new_ma_pt = xform * ma_pt;
                Shape::Ellipse {
                    center: new_center,
                    major_axis: (new_ma_pt.x - new_center.x, new_ma_pt.y - new_center.y),
                    minor_ratio: *minor_ratio,
                    start_param: *start_param,
                    end_param: *end_param,
                }
            }
            Shape::Spline {
                degree,
                knots,
                control_points,
                closed,
            } => Shape::Spline {
                degree: *degree,
                knots: knots.clone(),
                control_points: control_points.iter().map(|&p| xform * p).collect(),
                closed: *closed,
            },
            Shape::BlockInsert {
                block_name,
                position,
                rotation,
                x_scale,
                y_scale,
            } => {
                // Compose the insert transform with the applied transform
                let new_pos = xform * *position;
                Shape::BlockInsert {
                    block_name: block_name.clone(),
                    position: new_pos,
                    rotation: *rotation,
                    x_scale: *x_scale,
                    y_scale: *y_scale,
                }
            }
            Shape::Text {
                text,
                position,
                height,
                rotation,
            } => {
                let coeffs = xform.as_coeffs();
                let sx = (coeffs[0] * coeffs[0] + coeffs[1] * coeffs[1]).sqrt();
                Shape::Text {
                    text: text.clone(),
                    position: xform * *position,
                    height: height * sx,
                    rotation: *rotation,
                }
            }
            Shape::MText {
                text,
                plain_text,
                position,
                height,
                rotation,
            } => {
                let coeffs = xform.as_coeffs();
                let sx = (coeffs[0] * coeffs[0] + coeffs[1] * coeffs[1]).sqrt();
                Shape::MText {
                    text: text.clone(),
                    plain_text: plain_text.clone(),
                    position: xform * *position,
                    height: height * sx,
                    rotation: *rotation,
                }
            }
        }
    }

    /// Bounding box as (min_x, min_y, max_x, max_y).
    pub fn bbox(&self) -> (f64, f64, f64, f64) {
        match self {
            Shape::Line(l) => (
                l.p0.x.min(l.p1.x),
                l.p0.y.min(l.p1.y),
                l.p0.x.max(l.p1.x),
                l.p0.y.max(l.p1.y),
            ),
            Shape::Circle(c) => (
                c.center.x - c.radius,
                c.center.y - c.radius,
                c.center.x + c.radius,
                c.center.y + c.radius,
            ),
            Shape::Arc {
                center,
                radius,
                start_angle,
                end_angle,
            } => arc_bbox(*center, *radius, *start_angle, *end_angle),
            Shape::BlockInsert { position, .. } => {
                // Approximate: the actual bbox depends on block def contents.
                // The renderer computes the real bbox from expanded shapes.
                (position.x, position.y, position.x, position.y)
            }
            Shape::Text {
                position,
                height,
                text,
                ..
            } => {
                // Approximate: width ~ height * char_count * 0.6
                let w = *height * text.len() as f64 * 0.6;
                (position.x, position.y, position.x + w, position.y + height)
            }
            Shape::MText {
                position,
                height,
                plain_text,
                ..
            } => {
                let w = *height * plain_text.len() as f64 * 0.6;
                (position.x, position.y, position.x + w, position.y + height)
            }
            Shape::Polyline { points, .. } => {
                let mut min_x = f64::MAX;
                let mut min_y = f64::MAX;
                let mut max_x = f64::MIN;
                let mut max_y = f64::MIN;
                for p in points {
                    min_x = min_x.min(p.x);
                    min_y = min_y.min(p.y);
                    max_x = max_x.max(p.x);
                    max_y = max_y.max(p.y);
                }
                (min_x, min_y, max_x, max_y)
            }
            Shape::LwPolyline { vertices, .. } => {
                // Use vertex positions + arc extent for bulge arcs
                let mut min_x = f64::MAX;
                let mut min_y = f64::MAX;
                let mut max_x = f64::MIN;
                let mut max_y = f64::MIN;
                for v in vertices {
                    min_x = min_x.min(v.point.x);
                    min_y = min_y.min(v.point.y);
                    max_x = max_x.max(v.point.x);
                    max_y = max_y.max(v.point.y);
                }
                (min_x, min_y, max_x, max_y)
            }
            Shape::SolidFill { boundary, holes } => fill_edges_bbox(boundary, holes),
            Shape::CurvePath { path, .. } => {
                let bb = path.bounding_box();
                (bb.x0, bb.y0, bb.x1, bb.y1)
            }
            Shape::Ellipse {
                center,
                major_axis,
                minor_ratio,
                ..
            } => {
                let ma_len = (major_axis.0 * major_axis.0 + major_axis.1 * major_axis.1).sqrt();
                let r = ma_len.max(ma_len * minor_ratio);
                (center.x - r, center.y - r, center.x + r, center.y + r)
            }
            Shape::Spline { control_points, .. } => {
                let mut min_x = f64::MAX;
                let mut min_y = f64::MAX;
                let mut max_x = f64::MIN;
                let mut max_y = f64::MIN;
                for p in control_points {
                    min_x = min_x.min(p.x);
                    min_y = min_y.min(p.y);
                    max_x = max_x.max(p.x);
                    max_y = max_y.max(p.y);
                }
                (min_x, min_y, max_x, max_y)
            }
        }
    }

    /// Convert to a `kurbo::BezPath` with configurable tolerance.
    /// `tol` is in drawing units. Use ~0.1 for GPU, ~50 for PDF export.
    /// Returns `None` for BlockInsert/Text/MText.
    pub fn to_bezpath_tol(&self, tol: f64) -> Option<BezPath> {
        match self {
            Shape::Circle(c) => Some(kurbo::Circle::new(c.center, c.radius).to_path(tol)),
            Shape::Arc {
                center,
                radius,
                start_angle,
                end_angle,
            } => {
                let mut sweep = end_angle - start_angle;
                if sweep < 0.0 {
                    sweep += 2.0 * PI;
                }
                if sweep > 2.0 * PI {
                    sweep = 2.0 * PI;
                }
                let arc = kurbo::Arc::new(*center, (*radius, *radius), *start_angle, sweep, 0.0);
                let start_pt = Point::new(
                    center.x + radius * start_angle.cos(),
                    center.y + radius * start_angle.sin(),
                );
                let mut path = BezPath::new();
                path.move_to(start_pt);
                arc.to_path(tol).iter().for_each(|el| {
                    if !matches!(el, kurbo::PathEl::MoveTo(_)) {
                        path.push(el);
                    }
                });
                Some(path)
            }
            Shape::Ellipse {
                center,
                major_axis,
                minor_ratio,
                start_param,
                end_param,
            } => {
                let a = (major_axis.0 * major_axis.0 + major_axis.1 * major_axis.1).sqrt();
                let b = a * minor_ratio;
                let rotation = major_axis.1.atan2(major_axis.0);
                let mut sweep = end_param - start_param;
                if sweep < 0.0 {
                    sweep += 2.0 * PI;
                }
                if sweep > 2.0 * PI {
                    sweep = 2.0 * PI;
                }
                let arc = kurbo::Arc::new(*center, (a, b), *start_param, sweep, rotation);
                let cos_r = rotation.cos();
                let sin_r = rotation.sin();
                let ex = a * start_param.cos();
                let ey = b * start_param.sin();
                let sx = center.x + ex * cos_r - ey * sin_r;
                let sy = center.y + ex * sin_r + ey * cos_r;
                let mut path = BezPath::new();
                path.move_to(Point::new(sx, sy));
                arc.to_path(tol).iter().for_each(|el| {
                    if !matches!(el, kurbo::PathEl::MoveTo(_)) {
                        path.push(el);
                    }
                });
                Some(path)
            }
            // For non-curved shapes, delegate to the standard method.
            _ => self.to_bezpath(),
        }
    }

    /// Convert to a `kurbo::BezPath` for GPU rendering (Vello).
    ///
    /// Returns `None` for BlockInsert/Text/MText, which must be expanded
    /// by the caller before rendering.
    pub fn to_bezpath(&self) -> Option<BezPath> {
        match self {
            Shape::Line(l) => {
                let mut path = BezPath::new();
                path.move_to(l.p0);
                path.line_to(l.p1);
                Some(path)
            }
            Shape::Circle(c) => Some(kurbo::Circle::new(c.center, c.radius).to_path(0.1)),
            Shape::Arc {
                center,
                radius,
                start_angle,
                end_angle,
            } => {
                let mut sweep = end_angle - start_angle;
                if sweep < 0.0 {
                    sweep += 2.0 * PI;
                }
                if sweep > 2.0 * PI {
                    sweep = 2.0 * PI;
                }
                let arc = kurbo::Arc::new(*center, (*radius, *radius), *start_angle, sweep, 0.0);
                let start_pt = Point::new(
                    center.x + radius * start_angle.cos(),
                    center.y + radius * start_angle.sin(),
                );
                let mut path = BezPath::new();
                path.move_to(start_pt);
                arc.to_path(0.1).iter().for_each(|el| {
                    if !matches!(el, kurbo::PathEl::MoveTo(_)) {
                        path.push(el);
                    }
                });
                Some(path)
            }
            Shape::Polyline { points, closed } => {
                if points.len() < 2 {
                    return None;
                }
                let mut path = BezPath::new();
                path.move_to(points[0]);
                for p in &points[1..] {
                    path.line_to(*p);
                }
                if *closed {
                    path.close_path();
                }
                Some(path)
            }
            Shape::LwPolyline { vertices, closed } => {
                if vertices.is_empty() {
                    return None;
                }
                let mut path = BezPath::new();
                path.move_to(vertices[0].point);
                let n = vertices.len();
                let segments = if *closed { n } else { n.saturating_sub(1) };
                for i in 0..segments {
                    let j = (i + 1) % n;
                    let p0 = vertices[i].point;
                    let p1 = vertices[j].point;
                    let bulge = vertices[i].bulge;
                    if bulge.abs() < 1e-10 {
                        path.line_to(p1);
                    } else {
                        lwpolyline_bulge_arc_to_path(&mut path, p0, p1, bulge);
                    }
                }
                if *closed {
                    path.close_path();
                }
                Some(path)
            }
            Shape::SolidFill { boundary, holes } => {
                if boundary.is_empty() {
                    return None;
                }
                let mut path = fill_edges_to_bezpath(boundary);
                path.close_path();
                for hole in holes {
                    if hole.is_empty() {
                        continue;
                    }
                    let mut hole_path = fill_edges_to_bezpath(hole);
                    hole_path.close_path();
                    for el in hole_path.iter() {
                        path.push(el);
                    }
                }
                Some(path)
            }
            Shape::CurvePath { path, closed } => {
                let mut p = path.clone();
                if *closed {
                    let els: Vec<_> = p.elements().to_vec();
                    if !els.is_empty() && !matches!(els.last(), Some(kurbo::PathEl::ClosePath)) {
                        p.close_path();
                    }
                }
                Some(p)
            }
            Shape::Ellipse {
                center,
                major_axis,
                minor_ratio,
                start_param,
                end_param,
            } => {
                let a = (major_axis.0 * major_axis.0 + major_axis.1 * major_axis.1).sqrt();
                let b = a * minor_ratio;
                let rotation = major_axis.1.atan2(major_axis.0);
                let mut sweep = end_param - start_param;
                if sweep < 0.0 {
                    sweep += 2.0 * PI;
                }
                if sweep > 2.0 * PI {
                    sweep = 2.0 * PI;
                }
                let arc = kurbo::Arc::new(*center, (a, b), *start_param, sweep, rotation);
                let cos_r = rotation.cos();
                let sin_r = rotation.sin();
                let ex = a * start_param.cos();
                let ey = b * start_param.sin();
                let sx = center.x + ex * cos_r - ey * sin_r;
                let sy = center.y + ex * sin_r + ey * cos_r;
                let mut path = BezPath::new();
                path.move_to(Point::new(sx, sy));
                arc.to_path(0.1).iter().for_each(|el| {
                    if !matches!(el, kurbo::PathEl::MoveTo(_)) {
                        path.push(el);
                    }
                });
                Some(path)
            }
            Shape::Spline {
                degree,
                knots,
                control_points,
                closed,
            } => {
                let pts = tessellate_spline(*degree, knots, control_points, 0.5);
                if pts.len() < 2 {
                    return None;
                }
                let mut path = BezPath::new();
                path.move_to(pts[0]);
                for p in &pts[1..] {
                    path.line_to(*p);
                }
                if *closed {
                    path.close_path();
                }
                Some(path)
            }
            Shape::BlockInsert { .. } | Shape::Text { .. } | Shape::MText { .. } => None,
        }
    }
}

/// Convert a LwPolyline bulge arc segment to cubic bezier curves.
pub(crate) fn lwpolyline_bulge_arc_to_path(path: &mut BezPath, p0: Point, p1: Point, bulge: f64) {
    let included = 4.0 * bulge.atan();
    let dx = p1.x - p0.x;
    let dy = p1.y - p0.y;
    let chord = (dx * dx + dy * dy).sqrt();
    if chord < 1e-12 {
        path.line_to(p1);
        return;
    }
    let radius = (chord / (2.0 * included.sin().abs())).abs();
    let sagitta = bulge * chord / 2.0;
    let mx = (p0.x + p1.x) / 2.0;
    let my = (p0.y + p1.y) / 2.0;
    let nx = -dy / chord;
    let ny = dx / chord;
    let d = radius - sagitta.abs();
    let sign = if bulge > 0.0 { 1.0 } else { -1.0 };
    let cx = mx + sign * d * nx;
    let cy = my + sign * d * ny;
    let center = Point::new(cx, cy);
    let start_angle = (p0.y - cy).atan2(p0.x - cx);
    let arc = kurbo::Arc::new(center, (radius, radius), start_angle, included, 0.0);
    arc.to_path(0.1).iter().for_each(|el| {
        if !matches!(el, kurbo::PathEl::MoveTo(_)) {
            path.push(el);
        }
    });
}

/// Convert FillEdge sequence to a BezPath (without closing).
pub(crate) fn fill_edges_to_bezpath(edges: &[FillEdge]) -> BezPath {
    let mut path = BezPath::new();
    let mut first = true;
    for edge in edges {
        match edge {
            FillEdge::LineTo(p) => {
                if first {
                    path.move_to(*p);
                    first = false;
                } else {
                    path.line_to(*p);
                }
            }
            FillEdge::ArcTo {
                center,
                radius,
                start_angle,
                end_angle,
            } => {
                let start_pt = Point::new(
                    center.x + radius * start_angle.cos(),
                    center.y + radius * start_angle.sin(),
                );
                if first {
                    path.move_to(start_pt);
                    first = false;
                }
                let mut sweep = end_angle - start_angle;
                if sweep < 0.0 {
                    sweep += 2.0 * PI;
                }
                if sweep > 2.0 * PI {
                    sweep = 2.0 * PI;
                }
                let arc = kurbo::Arc::new(*center, (*radius, *radius), *start_angle, sweep, 0.0);
                arc.to_path(0.1).iter().for_each(|el| {
                    if !matches!(el, kurbo::PathEl::MoveTo(_)) {
                        path.push(el);
                    }
                });
            }
            FillEdge::EllipseArcTo {
                center,
                major_axis,
                minor_ratio,
                start_param,
                end_param,
            } => {
                let a = (major_axis.0 * major_axis.0 + major_axis.1 * major_axis.1).sqrt();
                let b = a * minor_ratio;
                let rotation = major_axis.1.atan2(major_axis.0);
                let cos_r = rotation.cos();
                let sin_r = rotation.sin();
                let ex = a * start_param.cos();
                let ey = b * start_param.sin();
                let sx = center.x + ex * cos_r - ey * sin_r;
                let sy = center.y + ex * sin_r + ey * cos_r;
                if first {
                    path.move_to(Point::new(sx, sy));
                    first = false;
                }
                let mut sweep = end_param - start_param;
                if sweep < 0.0 {
                    sweep += 2.0 * PI;
                }
                if sweep > 2.0 * PI {
                    sweep = 2.0 * PI;
                }
                let arc = kurbo::Arc::new(*center, (a, b), *start_param, sweep, rotation);
                arc.to_path(0.1).iter().for_each(|el| {
                    if !matches!(el, kurbo::PathEl::MoveTo(_)) {
                        path.push(el);
                    }
                });
            }
            FillEdge::SplineTo {
                degree,
                knots,
                control_points,
            } => {
                let pts = tessellate_spline(*degree, knots, control_points, 0.5);
                for (i, p) in pts.iter().enumerate() {
                    if i == 0 && first {
                        path.move_to(*p);
                        first = false;
                    } else {
                        path.line_to(*p);
                    }
                }
            }
            FillEdge::PolylineTo(pts) => {
                for (i, p) in pts.iter().enumerate() {
                    if i == 0 && first {
                        path.move_to(*p);
                        first = false;
                    } else {
                        path.line_to(*p);
                    }
                }
            }
        }
    }
    path
}

/// Exact bounding box for a circular arc. Checks the arc endpoints plus
/// any axis-aligned extrema (0, 90, 180, 270) that fall within the sweep.
pub(crate) fn arc_bbox(center: Point, radius: f64, start: f64, end: f64) -> (f64, f64, f64, f64) {
    let mut sweep = end - start;
    if sweep < 0.0 {
        sweep += 2.0 * PI;
    }
    if sweep >= 2.0 * PI {
        // Full circle
        return (
            center.x - radius,
            center.y - radius,
            center.x + radius,
            center.y + radius,
        );
    }

    let p0 = Point::new(
        center.x + radius * start.cos(),
        center.y + radius * start.sin(),
    );
    let p1 = Point::new(center.x + radius * end.cos(), center.y + radius * end.sin());
    let mut min_x = p0.x.min(p1.x);
    let mut min_y = p0.y.min(p1.y);
    let mut max_x = p0.x.max(p1.x);
    let mut max_y = p0.y.max(p1.y);

    // Check cardinal directions: 0, PI/2, PI, 3PI/2
    let cardinals = [0.0, PI / 2.0, PI, 3.0 * PI / 2.0];
    let extremes = [
        (radius, 0.0),  // right
        (0.0, radius),  // top
        (-radius, 0.0), // left
        (0.0, -radius), // bottom
    ];
    for (card, (dx, dy)) in cardinals.iter().zip(extremes.iter()) {
        // Normalize cardinal relative to start angle
        let mut a = card - start;
        while a < 0.0 {
            a += 2.0 * PI;
        }
        if a < sweep {
            let ex = center.x + dx;
            let ey = center.y + dy;
            min_x = min_x.min(ex);
            min_y = min_y.min(ey);
            max_x = max_x.max(ex);
            max_y = max_y.max(ey);
        }
    }
    (min_x, min_y, max_x, max_y)
}

/// Bounding box for a fill boundary (FillEdge sequences).
pub(crate) fn fill_edges_bbox(
    boundary: &[FillEdge],
    holes: &[Vec<FillEdge>],
) -> (f64, f64, f64, f64) {
    let mut min_x = f64::MAX;
    let mut min_y = f64::MAX;
    let mut max_x = f64::MIN;
    let mut max_y = f64::MIN;
    let mut merge = |x0: f64, y0: f64, x1: f64, y1: f64| {
        min_x = min_x.min(x0);
        min_y = min_y.min(y0);
        max_x = max_x.max(x1);
        max_y = max_y.max(y1);
    };
    for edges in std::iter::once(boundary).chain(holes.iter().map(|h| h.as_slice())) {
        for edge in edges {
            match edge {
                FillEdge::LineTo(p) => merge(p.x, p.y, p.x, p.y),
                FillEdge::ArcTo {
                    center,
                    radius,
                    start_angle,
                    end_angle,
                } => {
                    let (x0, y0, x1, y1) = arc_bbox(*center, *radius, *start_angle, *end_angle);
                    merge(x0, y0, x1, y1);
                }
                FillEdge::EllipseArcTo {
                    center,
                    major_axis,
                    minor_ratio,
                    ..
                } => {
                    let ma_len = (major_axis.0 * major_axis.0 + major_axis.1 * major_axis.1).sqrt();
                    let r = ma_len.max(ma_len * minor_ratio);
                    merge(center.x - r, center.y - r, center.x + r, center.y + r);
                }
                FillEdge::SplineTo { control_points, .. } => {
                    for p in control_points {
                        merge(p.x, p.y, p.x, p.y);
                    }
                }
                FillEdge::PolylineTo(pts) => {
                    for p in pts {
                        merge(p.x, p.y, p.x, p.y);
                    }
                }
            }
        }
    }
    (min_x, min_y, max_x, max_y)
}
