//! PDF export for Document.
//!
//! Renders all visible entities to a single-page PDF. Returns the PDF
//! as `Vec<u8>` so the caller decides what to do with it (write to disk
//! on server, offer as blob download on WASM).

use kurbo::{Affine, BezPath, PathEl, Point};
use printpdf::*;

use crate::{Color, Document, Shape};

pub struct PdfOptions {
    pub page_width_mm: f32,
    pub page_height_mm: f32,
    pub margin_mm: f32,
    pub skip_layers: Vec<String>,
    pub line_width_pt: f32,
}

impl Default for PdfOptions {
    fn default() -> Self {
        Self {
            page_width_mm: 297.0,
            page_height_mm: 210.0,
            margin_mm: 10.0,
            skip_layers: [
                "E_DEBUG",
                "E_RAY_HIT",
                "E_RAY_EP",
                "E_RAY_MISS",
                "E_RAY_DOOR",
                "E_DOORS",
                "E_RAW",
                "E_VIS",
                "E_WALLS",
                "Defpoints",
            ]
            .iter()
            .map(|s| s.to_string())
            .collect(),
            line_width_pt: 0.1,
        }
    }
}

struct ExpandedEntity {
    shape: Shape,
    layer: String,
    color: Color,
}

pub fn export_pdf(doc: &Document, opts: &PdfOptions) -> Vec<u8> {
    let skip: std::collections::HashSet<&str> =
        opts.skip_layers.iter().map(|s| s.as_str()).collect();

    let expanded = expand_entities(doc);

    let visible: Vec<&ExpandedEntity> = expanded
        .iter()
        .filter(|e| !skip.contains(e.layer.as_str()))
        .filter(|e| {
            doc.layers.iter().any(|l| l.name == e.layer && l.visible) || e.layer.starts_with("E_")
        })
        .collect();

    // Bounding box.
    let (mut min_x, mut min_y) = (f64::MAX, f64::MAX);
    let (mut max_x, mut max_y) = (f64::MIN, f64::MIN);
    for ent in &visible {
        update_bounds(&ent.shape, &mut min_x, &mut min_y, &mut max_x, &mut max_y);
    }

    if min_x >= max_x || min_y >= max_y {
        let doc = PdfDocument::default();
        let mut warnings = Vec::new();
        return doc.save(&PdfSaveOptions::default(), &mut warnings);
    }

    let draw_w = max_x - min_x;
    let draw_h = max_y - min_y;
    let avail_w = (opts.page_width_mm - 2.0 * opts.margin_mm) as f64;
    let avail_h = (opts.page_height_mm - 2.0 * opts.margin_mm) as f64;
    let scale = (avail_w / draw_w).min(avail_h / draw_h);
    let off_x = opts.margin_mm as f64 + (avail_w - draw_w * scale) / 2.0;
    let off_y = opts.margin_mm as f64 + (avail_h - draw_h * scale) / 2.0;
    let tol = 1.0; // 1mm in DWG space

    // 1mm = 2.8346 pt
    let mm_to_pt = |mm: f64| -> Pt { Pt(Mm(mm as f32).into_pt().0) };

    let to_pdf = |p: Point| -> printpdf::Point {
        let x_mm = off_x + (p.x - min_x) * scale;
        let y_mm = off_y + (p.y - min_y) * scale;
        printpdf::Point {
            x: mm_to_pt(x_mm),
            y: mm_to_pt(y_mm),
        }
    };

    let layer_color = |name: &str| -> (f32, f32, f32) {
        doc.layers
            .iter()
            .find(|l| l.name == name)
            .map(|l| color_f32(l.color))
            .unwrap_or((0.0, 0.0, 0.0))
    };

    let mut ops: Vec<Op> = Vec::new();

    for ent in &visible {
        let (r, g, b) = resolve_color(ent, &layer_color);
        let lw = line_weight(&ent.layer, opts.line_width_pt);

        match &ent.shape {
            Shape::Text {
                text,
                position,
                height,
                rotation,
            } => {
                let paths = crate::text::text_to_paths_rotated(
                    text, position.x, position.y, *height, *rotation,
                );
                set_stroke(&mut ops, r, g, b, lw * 0.5);
                for path in &paths {
                    emit_path(&mut ops, path, &to_pdf);
                }
            }
            Shape::MText {
                plain_text,
                position,
                height,
                rotation,
                ..
            } => {
                let paths = crate::text::text_to_paths_rotated(
                    plain_text, position.x, position.y, *height, *rotation,
                );
                set_stroke(&mut ops, r, g, b, lw * 0.5);
                for path in &paths {
                    emit_path(&mut ops, path, &to_pdf);
                }
            }
            _ => {
                if let Some(path) = ent.shape.to_bezpath_tol(tol) {
                    set_stroke(&mut ops, r, g, b, lw);
                    emit_path(&mut ops, &path, &to_pdf);
                }
            }
        }
    }

    let page = PdfPage::new(Mm(opts.page_width_mm), Mm(opts.page_height_mm), ops);
    let mut pdf_doc = PdfDocument::default();
    PdfDocument::with_pages(&mut pdf_doc, vec![page]);

    let mut warnings = Vec::new();
    pdf_doc.save(&PdfSaveOptions::default(), &mut warnings)
}

fn set_stroke(ops: &mut Vec<Op>, r: f32, g: f32, b: f32, lw: f32) {
    ops.push(Op::SetOutlineColor {
        col: printpdf::Color::Rgb(Rgb {
            r,
            g,
            b,
            icc_profile: None,
        }),
    });
    ops.push(Op::SetOutlineThickness { pt: Pt(lw) });
}

fn emit_path(ops: &mut Vec<Op>, path: &BezPath, to_pdf: &dyn Fn(Point) -> printpdf::Point) {
    let mut points: Vec<LinePoint> = Vec::new();
    let mut segments: Vec<Vec<LinePoint>> = Vec::new();

    for el in path.iter() {
        match el {
            PathEl::MoveTo(p) => {
                if points.len() >= 2 {
                    segments.push(std::mem::take(&mut points));
                }
                points.push(LinePoint {
                    p: to_pdf(p),
                    bezier: false,
                });
            }
            PathEl::LineTo(p) => {
                points.push(LinePoint {
                    p: to_pdf(p),
                    bezier: false,
                });
            }
            PathEl::CurveTo(p1, p2, p3) => {
                points.push(LinePoint {
                    p: to_pdf(p1),
                    bezier: true,
                });
                points.push(LinePoint {
                    p: to_pdf(p2),
                    bezier: true,
                });
                points.push(LinePoint {
                    p: to_pdf(p3),
                    bezier: false,
                });
            }
            PathEl::QuadTo(p1, p2) => {
                // Elevate to cubic using last point.
                let last = points.last().map(|lp| lp.p).unwrap_or_default();
                let pp1 = to_pdf(p1);
                let pp2 = to_pdf(p2);
                let cp1 = printpdf::Point {
                    x: Pt(last.x.0 + 2.0 / 3.0 * (pp1.x.0 - last.x.0)),
                    y: Pt(last.y.0 + 2.0 / 3.0 * (pp1.y.0 - last.y.0)),
                };
                let cp2 = printpdf::Point {
                    x: Pt(pp2.x.0 + 2.0 / 3.0 * (pp1.x.0 - pp2.x.0)),
                    y: Pt(pp2.y.0 + 2.0 / 3.0 * (pp1.y.0 - pp2.y.0)),
                };
                points.push(LinePoint {
                    p: cp1,
                    bezier: true,
                });
                points.push(LinePoint {
                    p: cp2,
                    bezier: true,
                });
                points.push(LinePoint {
                    p: pp2,
                    bezier: false,
                });
            }
            PathEl::ClosePath => {
                if let Some(first) = points.first() {
                    points.push(LinePoint {
                        p: first.p,
                        bezier: false,
                    });
                }
            }
        }
    }
    if points.len() >= 2 {
        segments.push(points);
    }

    for seg in segments {
        if seg.len() < 2 {
            continue;
        }
        ops.push(Op::DrawLine {
            line: Line {
                points: seg,
                is_closed: false,
            },
        });
    }
}

fn resolve_color(
    ent: &ExpandedEntity,
    layer_color: &dyn Fn(&str) -> (f32, f32, f32),
) -> (f32, f32, f32) {
    let (r, g, b) = if ent.color != Color::WHITE && ent.color != Color::rgb(0, 0, 0) {
        color_f32(ent.color)
    } else {
        layer_color(&ent.layer)
    };
    if r > 0.9 && g > 0.9 && b > 0.9 {
        (0.6, 0.6, 0.6)
    } else {
        (r, g, b)
    }
}

fn line_weight(layer: &str, base: f32) -> f32 {
    if layer.starts_with("E_") {
        base * 2.5
    } else if layer.starts_with("S_WALL") {
        base * 1.5
    } else {
        base
    }
}

fn update_bounds(
    shape: &Shape,
    min_x: &mut f64,
    min_y: &mut f64,
    max_x: &mut f64,
    max_y: &mut f64,
) {
    match shape {
        Shape::Text {
            position,
            height,
            text,
            ..
        }
        | Shape::MText {
            position,
            height,
            plain_text: text,
            ..
        } => {
            *min_x = min_x.min(position.x);
            *min_y = min_y.min(position.y);
            *max_x = max_x.max(position.x + text.len() as f64 * height * 0.6);
            *max_y = max_y.max(position.y + *height);
        }
        _ => {
            if let Some(path) = shape.to_bezpath() {
                for_each_point(&path, |p| {
                    *min_x = min_x.min(p.x);
                    *min_y = min_y.min(p.y);
                    *max_x = max_x.max(p.x);
                    *max_y = max_y.max(p.y);
                });
            }
        }
    }
}

fn expand_entities(doc: &Document) -> Vec<ExpandedEntity> {
    let mut result = Vec::new();
    for ent in &doc.entities {
        if let Shape::BlockInsert {
            block_name,
            position,
            rotation,
            x_scale,
            y_scale,
        } = &ent.shape
        {
            if let Some(def) = doc.blocks.get(block_name) {
                let xform = Affine::translate((position.x, position.y))
                    * Affine::rotate(*rotation)
                    * Affine::scale_non_uniform(*x_scale, *y_scale)
                    * Affine::translate((-def.insert_point.x, -def.insert_point.y));
                for (shape, shape_layer, shape_color) in &def.shapes {
                    let layer = if shape_layer.is_empty() {
                        &ent.layer
                    } else {
                        shape_layer
                    };
                    let color = if *shape_color == Color::WHITE && ent.color != Color::WHITE {
                        ent.color
                    } else {
                        *shape_color
                    };
                    result.push(ExpandedEntity {
                        shape: shape.transformed(xform),
                        layer: layer.clone(),
                        color,
                    });
                }
            }
        } else {
            result.push(ExpandedEntity {
                shape: ent.shape.clone(),
                layer: ent.layer.clone(),
                color: ent.color,
            });
        }
    }
    result
}

fn color_f32(c: Color) -> (f32, f32, f32) {
    (c.r as f32 / 255.0, c.g as f32 / 255.0, c.b as f32 / 255.0)
}

fn for_each_point(path: &BezPath, mut f: impl FnMut(Point)) {
    for el in path.iter() {
        match el {
            PathEl::MoveTo(p) | PathEl::LineTo(p) => f(p),
            PathEl::QuadTo(p1, p2) => {
                f(p1);
                f(p2);
            }
            PathEl::CurveTo(p1, p2, p3) => {
                f(p1);
                f(p2);
                f(p3);
            }
            PathEl::ClosePath => {}
        }
    }
}
