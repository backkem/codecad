//! Annotation: aligned text, ISO-style dimensions (linear, radius,
//! diameter, angle) built from real entities.
//!
//! Dimensions are emitted as ordinary entities so they survive DWG
//! roundtrips without a dimension-style table:
//! - extension / dimension lines: `Shape::Line` (angle dims: `Shape::Arc`)
//! - arrowheads: filled triangles (`Shape::SolidFill`, written to DWG as
//!   SOLID entities)
//! - the measurement: a real `Shape::Text` entity (DWG TEXT), not glyph
//!   outlines, so it stays editable/searchable.

use crate::document::{Document, DrawEntity};
use crate::text;
use crate::types::*;
use kurbo::{Line, Point, Vec2};
use std::f64::consts::{PI, TAU};

// ── Text alignment ─────────────────────────────────────────────────────

/// Horizontal text alignment relative to the anchor point.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum HAlign {
    #[default]
    Left,
    Center,
    Right,
}

/// Vertical text alignment relative to the anchor point. Text height is
/// the cap height, so `Top` puts capitals' tops on the anchor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum VAlign {
    #[default]
    Baseline,
    Middle,
    Top,
}

impl HAlign {
    pub fn parse(s: Option<&str>) -> Result<Self, String> {
        match s.unwrap_or("left") {
            "left" | "l" | "start" => Ok(HAlign::Left),
            "center" | "centre" | "c" | "middle" => Ok(HAlign::Center),
            "right" | "r" | "end" => Ok(HAlign::Right),
            o => Err(format!("unknown align '{o}' (left|center|right)")),
        }
    }
}

impl VAlign {
    pub fn parse(s: Option<&str>) -> Result<Self, String> {
        match s.unwrap_or("baseline") {
            "baseline" | "bottom" | "b" => Ok(VAlign::Baseline),
            "middle" | "center" | "centre" | "m" => Ok(VAlign::Middle),
            "top" | "t" => Ok(VAlign::Top),
            o => Err(format!("unknown valign '{o}' (baseline|middle|top)")),
        }
    }
}

/// Left-baseline insertion point for text anchored at `at` with the given
/// alignment and rotation (radians). Widths come from the embedded font.
pub fn aligned_text_position(
    s: &str,
    at: Point,
    height: f64,
    rotation: f64,
    h: HAlign,
    v: VAlign,
) -> Point {
    let w = text::text_width(s, height);
    let dx = match h {
        HAlign::Left => 0.0,
        HAlign::Center => -w / 2.0,
        HAlign::Right => -w,
    };
    let dy = match v {
        VAlign::Baseline => 0.0,
        VAlign::Middle => -height / 2.0,
        VAlign::Top => -height,
    };
    let (c, sn) = (rotation.cos(), rotation.sin());
    Point::new(at.x + dx * c - dy * sn, at.y + dx * sn + dy * c)
}

// ── Number formatting ──────────────────────────────────────────────────

/// Format a measured value. With `decimals`, fixed precision; otherwise
/// rounded to 0.1 with a trailing ".0" dropped (100 -> "100", 12.34 -> "12.3").
pub fn format_value(v: f64, decimals: Option<usize>) -> String {
    match decimals {
        Some(d) => format!("{:.*}", d, v),
        None => {
            let r = (v * 10.0).round() / 10.0;
            if (r - r.round()).abs() < 1e-9 {
                format!("{}", r.round() as i64)
            } else {
                format!("{r:.1}")
            }
        }
    }
}

/// Replace "<>" in a label template with the formatted value.
pub fn apply_label(template: &str, value: &str) -> String {
    if template.contains("<>") {
        template.replace("<>", value)
    } else {
        template.to_string()
    }
}

// ── Emitter ────────────────────────────────────────────────────────────

/// Style shared by all entities of one annotation.
#[derive(Debug, Clone)]
pub struct DimStyle {
    pub layer: String,
    pub color: Option<Color>,
    /// Text (cap) height.
    pub text_height: f64,
    /// Arrowhead length.
    pub arrow: f64,
}

impl DimStyle {
    pub fn new(layer: &str, color: Option<Color>, text_height: f64, arrow: Option<f64>) -> Self {
        DimStyle {
            layer: layer.to_string(),
            color,
            text_height,
            arrow: arrow.unwrap_or(text_height * 0.85),
        }
    }
    /// Gap between the measured object and the start of an extension line.
    fn gap(&self) -> f64 {
        self.text_height * 0.3
    }
    /// Extension-line overshoot past the dimension line.
    fn overshoot(&self) -> f64 {
        self.text_height * 0.6
    }
    /// Distance from dimension line to the text baseline.
    fn text_lift(&self) -> f64 {
        self.text_height * 0.3
    }
}

/// Collects the entities of one annotation.
pub struct Emitter<'a> {
    doc: &'a mut Document,
    style: DimStyle,
    pub ids: Vec<EntityId>,
    pub text_id: Option<EntityId>,
}

impl<'a> Emitter<'a> {
    pub fn new(doc: &'a mut Document, style: DimStyle) -> Self {
        doc.ensure_layer(&style.layer);
        Emitter {
            doc,
            style,
            ids: Vec::new(),
            text_id: None,
        }
    }

    fn push(&mut self, shape: Shape) -> EntityId {
        let id = self.doc.alloc_id();
        self.doc.entities.push(DrawEntity {
            attrs: Default::default(),
            id,
            layer: self.style.layer.clone(),
            color: self.style.color,
            linetype: None,
            lineweight: None,
            transparency: 0,
            shape,
        });
        self.ids.push(id);
        id
    }

    pub fn line(&mut self, a: Point, b: Point) {
        if (b - a).hypot() > 1e-12 {
            self.push(Shape::Line(Line::new(a, b)));
        }
    }

    pub fn arc(&mut self, c: Point, r: f64, start: f64, end: f64) {
        self.push(Shape::Arc {
            center: c,
            radius: r,
            start_angle: start,
            end_angle: end,
        });
    }

    /// Filled arrowhead with its tip at `tip`, pointing along `dir`.
    pub fn arrow(&mut self, tip: Point, dir: Vec2) {
        let d = unit(dir);
        if d == Vec2::ZERO {
            return;
        }
        let l = self.style.arrow;
        let w = l / 6.0;
        let n = Vec2::new(-d.y, d.x);
        let back = tip - d * l;
        self.push(Shape::SolidFill {
            boundary: vec![
                FillEdge::LineTo(tip),
                FillEdge::LineTo(back + n * w),
                FillEdge::LineTo(back - n * w),
            ],
            holes: Vec::new(),
        });
    }

    /// Text centred on `mid` along direction `dir`, lifted above the line.
    /// The direction is flipped if needed so text always reads left-to-right
    /// or bottom-to-top. Returns the text width.
    pub fn label(&mut self, s: &str, mid: Point, dir: Vec2) -> f64 {
        let mut d = unit(dir);
        if d == Vec2::ZERO {
            d = Vec2::new(1.0, 0.0);
        }
        let ang = d.y.atan2(d.x);
        if ang > PI / 2.0 + 1e-9 || ang <= -PI / 2.0 + 1e-9 {
            d = -d;
        }
        let rot = d.y.atan2(d.x);
        let h = self.style.text_height;
        let w = text::text_width(s, h);
        let up = Vec2::new(-d.y, d.x);
        let pos = mid - d * (w / 2.0) + up * self.style.text_lift();
        let id = self.push(Shape::Text {
            text: s.to_string(),
            position: pos,
            height: h,
            rotation: rot,
        });
        self.text_id = Some(id);
        w
    }

    pub fn style(&self) -> &DimStyle {
        &self.style
    }
}

fn unit(v: Vec2) -> Vec2 {
    let l = v.hypot();
    if l < 1e-15 {
        Vec2::ZERO
    } else {
        v / l
    }
}

// ── Dimension builders ─────────────────────────────────────────────────

/// Direction of a linear dimension.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DimDir {
    Aligned,
    Horizontal,
    Vertical,
}

impl DimDir {
    pub fn parse(s: Option<&str>) -> Result<Self, String> {
        match s.unwrap_or("aligned") {
            "aligned" | "a" => Ok(DimDir::Aligned),
            "h" | "horizontal" | "x" => Ok(DimDir::Horizontal),
            "v" | "vertical" | "y" => Ok(DimDir::Vertical),
            o => Err(format!("unknown dim dir '{o}' (h|v|aligned)")),
        }
    }
}

#[allow(clippy::too_many_arguments)]
/// Linear dimension between `a` and `b`. `offset` is the signed distance of
/// the dimension line from the outermost measured point along the left
/// normal of the dimension direction. Returns the measured value.
pub fn linear_dimension(
    em: &mut Emitter,
    a: Point,
    b: Point,
    offset: f64,
    dir: DimDir,
    label_tpl: &str,
    decimals: Option<usize>,
    shift: f64,
) -> Result<f64, String> {
    let d = match dir {
        DimDir::Horizontal => Vec2::new(if b.x < a.x { -1.0 } else { 1.0 }, 0.0),
        DimDir::Vertical => Vec2::new(0.0, if b.y < a.y { -1.0 } else { 1.0 }),
        DimDir::Aligned => unit(b - a),
    };
    if d == Vec2::ZERO {
        return Err("cannot dimension two coincident points".into());
    }
    let n = Vec2::new(-d.y, d.x);
    let value = (b - a).dot(d).abs();
    let na = a.to_vec2().dot(n);
    let nb = b.to_vec2().dot(n);
    let line_n = if offset >= 0.0 {
        na.max(nb) + offset
    } else {
        na.min(nb) + offset
    };
    let on_line = |p: Point| p + n * (line_n - p.to_vec2().dot(n));
    let da = on_line(a);
    let db = on_line(b);
    let st = em.style().clone();
    // Extension lines (from a small gap off the object to just past the line)
    for (p, q) in [(a, da), (b, db)] {
        let v = q - p;
        let len = v.hypot();
        if len > st.gap() {
            let u = v / len;
            em.line(p + u * st.gap(), q + u * st.overshoot());
        }
    }
    let dd = unit(db - da);
    let span = (db - da).hypot();
    let dd = if dd == Vec2::ZERO { d } else { dd };
    if span < st.arrow * 3.0 {
        // Too short for inside arrows: arrows outside pointing in.
        em.line(da - dd * (st.arrow * 2.0), db + dd * (st.arrow * 2.0));
        em.arrow(da, dd);
        em.arrow(db, -dd);
    } else {
        em.line(da, db);
        em.arrow(da, -dd);
        em.arrow(db, dd);
    }
    let label = apply_label(label_tpl, &format_value(value, decimals));
    let mid = da.midpoint(db) + dd * shift;
    em.label(&label, mid, dd);
    Ok(value)
}

/// Radius dimension of a circle/arc at `angle` (radians).
pub fn radius_dimension(
    em: &mut Emitter,
    c: Point,
    r: f64,
    angle: f64,
    label_tpl: &str,
    decimals: Option<usize>,
) -> Result<f64, String> {
    if r <= 0.0 {
        return Err("radius must be > 0".into());
    }
    let st = em.style().clone();
    let u = Vec2::new(angle.cos(), angle.sin());
    let p = c + u * r;
    let label = apply_label(label_tpl, &format_value(r, decimals));
    let w = text::text_width(&label, st.text_height);
    if w + st.arrow * 2.0 < r {
        // Inside: line from the centre, arrow at the arc pointing out.
        em.line(c, p);
        em.arrow(p, u);
        em.label(&label, c.midpoint(p), u);
    } else {
        // Outside: leader from beyond the arc, arrow pointing at the centre.
        let tail = st.arrow + st.gap() * 2.0 + w;
        let q = p + u * tail;
        em.line(p, q);
        em.arrow(p, -u);
        em.label(&label, p + u * (st.arrow + st.gap() + w / 2.0), u);
    }
    Ok(r)
}

/// Diameter dimension across a circle at `angle` (radians).
pub fn diameter_dimension(
    em: &mut Emitter,
    c: Point,
    r: f64,
    angle: f64,
    label_tpl: &str,
    decimals: Option<usize>,
) -> Result<f64, String> {
    if r <= 0.0 {
        return Err("radius must be > 0".into());
    }
    let st = em.style().clone();
    let u = Vec2::new(angle.cos(), angle.sin());
    let p1 = c - u * r;
    let p2 = c + u * r;
    let label = apply_label(label_tpl, &format_value(2.0 * r, decimals));
    let w = text::text_width(&label, st.text_height);
    em.arrow(p1, -u);
    em.arrow(p2, u);
    if w + st.arrow * 3.0 < 2.0 * r {
        em.line(p1, p2);
        em.label(&label, c, u);
    } else {
        let tail = st.gap() * 2.0 + w;
        em.line(p1, p2 + u * tail);
        em.label(&label, p2 + u * (st.gap() + w / 2.0), u);
    }
    Ok(2.0 * r)
}

/// Angle dimension at `vertex`, measured counter-clockwise from the ray
/// through `p1` to the ray through `p2`. The dimension arc has radius
/// `radius`. Returns the angle in degrees.
pub fn angle_dimension(
    em: &mut Emitter,
    vertex: Point,
    p1: Point,
    p2: Point,
    radius: Option<f64>,
    label_tpl: &str,
    decimals: Option<usize>,
) -> Result<f64, String> {
    let d1 = (p1 - vertex).hypot();
    let d2 = (p2 - vertex).hypot();
    if d1 < 1e-12 || d2 < 1e-12 {
        return Err("angle rays must not start at the vertex point".into());
    }
    let a1 = (p1.y - vertex.y).atan2(p1.x - vertex.x);
    let a2 = (p2.y - vertex.y).atan2(p2.x - vertex.x);
    let sweep = (a2 - a1).rem_euclid(TAU);
    if sweep < 1e-9 {
        return Err("rays are collinear (zero angle)".into());
    }
    let st = em.style().clone();
    let r = radius.unwrap_or(d1.min(d2) * 0.75);
    if r <= 0.0 {
        return Err("dimension radius must be > 0".into());
    }
    // Extension lines along each ray when the measured point is inside the arc.
    for (a, d) in [(a1, d1), (a2, d2)] {
        let u = Vec2::new(a.cos(), a.sin());
        if d < r - st.gap() {
            em.line(
                vertex + u * (d + st.gap()),
                vertex + u * (r + st.overshoot()),
            );
        }
    }
    em.arc(vertex, r, a1, a1 + sweep);
    let e1 = vertex + Vec2::new(a1.cos(), a1.sin()) * r;
    let e2 = vertex + Vec2::new((a1 + sweep).cos(), (a1 + sweep).sin()) * r;
    // Tangent directions at the ends (CCW).
    let t1 = Vec2::new(-a1.sin(), a1.cos());
    let t2 = Vec2::new(-(a1 + sweep).sin(), (a1 + sweep).cos());
    em.arrow(e1, -t1);
    em.arrow(e2, t2);
    let deg = sweep.to_degrees();
    let label = apply_label(label_tpl, &format_value(deg, decimals));
    let am = a1 + sweep / 2.0;
    let mid = vertex + Vec2::new(am.cos(), am.sin()) * r;
    // Text tangent to the arc at its midpoint, on the outside.
    let tang = Vec2::new(-am.sin(), am.cos());
    // `label` lifts text to the left of the reading direction; make sure
    // that is away from the vertex.
    let outward = Vec2::new(am.cos(), am.sin());
    let mut dir = tang;
    let ang = dir.y.atan2(dir.x);
    if ang > PI / 2.0 + 1e-9 || ang <= -PI / 2.0 + 1e-9 {
        dir = -dir;
    }
    let up = Vec2::new(-dir.y, dir.x);
    let mid = if up.dot(outward) >= 0.0 {
        mid
    } else {
        // Text would sit inside the arc: push the anchor outward by a line.
        mid + outward * (st.text_height + 2.0 * st.text_lift())
    };
    em.label(&label, mid, dir);
    Ok(deg)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn count(doc: &Document, pred: impl Fn(&Shape) -> bool) -> usize {
        doc.entities.iter().filter(|e| pred(&e.shape)).count()
    }

    #[test]
    fn format_values() {
        assert_eq!(format_value(100.0, None), "100");
        assert_eq!(format_value(12.34, None), "12.3");
        assert_eq!(format_value(12.0, Some(2)), "12.00");
        assert_eq!(apply_label("Ø<>", "20"), "Ø20");
        assert_eq!(apply_label("fixed", "20"), "fixed");
    }

    #[test]
    fn aligned_text_center_and_rotation() {
        let w = text::text_width("ABC", 10.0);
        let p = aligned_text_position(
            "ABC",
            Point::new(0., 0.),
            10.0,
            0.0,
            HAlign::Center,
            VAlign::Middle,
        );
        assert!((p.x + w / 2.0).abs() < 1e-9 && (p.y + 5.0).abs() < 1e-9);
        // Rotated 90°: offsets rotate too
        let p = aligned_text_position(
            "ABC",
            Point::new(0., 0.),
            10.0,
            PI / 2.0,
            HAlign::Right,
            VAlign::Baseline,
        );
        assert!(p.x.abs() < 1e-9 && (p.y + w).abs() < 1e-9);
    }

    #[test]
    fn linear_dimension_horizontal_emits_text_and_filled_arrows() {
        let mut doc = Document::new();
        let mut em = Emitter::new(&mut doc, DimStyle::new("DIM", None, 3.5, None));
        let v = linear_dimension(
            &mut em,
            Point::new(0., 0.),
            Point::new(100., 20.),
            10.0,
            DimDir::Horizontal,
            "<>",
            None,
            0.0,
        )
        .unwrap();
        assert!((v - 100.0).abs() < 1e-9);
        let text_id = em.text_id.unwrap();
        assert_eq!(count(&doc, |s| matches!(s, Shape::SolidFill { .. })), 2);
        assert_eq!(count(&doc, |s| matches!(s, Shape::Line(_))), 3);
        let t = doc.entity(text_id).unwrap();
        match &t.shape {
            Shape::Text {
                text,
                position,
                rotation,
                ..
            } => {
                assert_eq!(text, "100");
                // Dimension line at y = 20 + 10, text just above it
                assert!(position.y > 30.0 && position.y < 32.0);
                assert!(rotation.abs() < 1e-9);
            }
            _ => panic!(),
        }
    }

    #[test]
    fn vertical_dimension_text_reads_bottom_to_top() {
        let mut doc = Document::new();
        let mut em = Emitter::new(&mut doc, DimStyle::new("DIM", None, 3.5, None));
        linear_dimension(
            &mut em,
            Point::new(0., 50.),
            Point::new(0., 0.),
            10.0,
            DimDir::Vertical,
            "<>",
            Some(1),
            0.0,
        )
        .unwrap();
        let tid = em.text_id.unwrap();
        let t = doc.entity(tid).unwrap();
        let Shape::Text { text, rotation, .. } = &t.shape else {
            panic!()
        };
        assert_eq!(text, "50.0");
        assert!((rotation - PI / 2.0).abs() < 1e-9);
    }

    #[test]
    fn coincident_points_error() {
        let mut doc = Document::new();
        let mut em = Emitter::new(&mut doc, DimStyle::new("DIM", None, 3.5, None));
        let p = Point::new(1., 1.);
        assert!(linear_dimension(&mut em, p, p, 5.0, DimDir::Aligned, "<>", None, 0.0).is_err());
    }

    #[test]
    fn radius_and_diameter_labels() {
        let mut doc = Document::new();
        let mut em = Emitter::new(&mut doc, DimStyle::new("DIM", None, 2.5, None));
        radius_dimension(&mut em, Point::new(0., 0.), 40.0, 0.5, "R<>", None).unwrap();
        let t = em.text_id.unwrap();
        let mut em = Emitter::new(&mut doc, DimStyle::new("DIM", None, 2.5, None));
        diameter_dimension(&mut em, Point::new(0., 0.), 3.0, 0.0, "Ø<>", None).unwrap();
        let t2 = em.text_id.unwrap();
        let txt = |id| match &doc.entity(id).unwrap().shape {
            Shape::Text { text, .. } => text.clone(),
            _ => panic!(),
        };
        assert_eq!(txt(t), "R40");
        assert_eq!(txt(t2), "Ø6");
        let mut em = Emitter::new(&mut doc, DimStyle::new("DIM", None, 2.5, None));
        assert!(radius_dimension(&mut em, Point::new(0., 0.), 0.0, 0.0, "R<>", None).is_err());
    }

    #[test]
    fn angle_dimension_ninety() {
        let mut doc = Document::new();
        let mut em = Emitter::new(&mut doc, DimStyle::new("DIM", None, 2.5, None));
        let deg = angle_dimension(
            &mut em,
            Point::new(0., 0.),
            Point::new(50., 0.),
            Point::new(0., 50.),
            Some(30.0),
            "<>°",
            None,
        )
        .unwrap();
        assert!((deg - 90.0).abs() < 1e-9);
        assert_eq!(count(&doc, |s| matches!(s, Shape::Arc { .. })), 1);
        let mut em = Emitter::new(&mut doc, DimStyle::new("DIM", None, 2.5, None));
        assert!(angle_dimension(
            &mut em,
            Point::new(0., 0.),
            Point::new(5., 0.),
            Point::new(10., 0.),
            None,
            "<>",
            None
        )
        .is_err());
    }
}
