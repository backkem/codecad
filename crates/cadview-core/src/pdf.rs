//! PDF export for Document.
//!
//! Renders all visible entities to a single-page PDF. Returns the PDF
//! as `Vec<u8>` so the caller decides what to do with it (write to disk
//! on server, offer as blob download on WASM).

use kurbo::{BezPath, PathEl, Point};
use printpdf::*;

use crate::{Color, Document, Shape};

pub struct PdfOptions {
    pub page_width_mm: f32,
    pub page_height_mm: f32,
    pub margin_mm: f32,
    /// Layers left out of the PDF, by exact name, on top of the layers the
    /// document marks invisible. The script decides: the core gives no
    /// layer name a meaning of its own, except `Defpoints`, which never
    /// plots by DWG convention and is always skipped.
    pub skip_layers: Vec<String>,
    pub line_width_pt: f32,
    pub colors: PdfColorMode,
    /// Stroke entities at their own lineweight (mm) where one is set on
    /// the entity or its layer. Off by default: every stroke then uses
    /// `line_width_pt`.
    pub lineweights: bool,
}

/// How entity colours map onto the white PDF page. Drawings are
/// authored for a dark screen, so stored colours are often too light
/// for paper.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PdfColorMode {
    /// Readable on white: neutrals (white, greys, black) get their
    /// lightness inverted and keep at least 3:1 contrast against white;
    /// chromatic colours keep their hue and are darkened to at least
    /// 4.5:1 (WCAG). Colours that already pass are untouched.
    #[default]
    Paper,
    /// Everything black.
    Mono,
    /// Colours exactly as stored, no remap (white stays white).
    Screen,
}

impl PdfColorMode {
    pub fn parse(s: &str) -> Result<Self, String> {
        match s {
            "paper" => Ok(Self::Paper),
            "mono" => Ok(Self::Mono),
            "screen" => Ok(Self::Screen),
            other => Err(format!(
                "savePdf: unknown colors \"{other}\" (expected \"paper\", \"mono\" or \"screen\")"
            )),
        }
    }
}

impl PdfOptions {
    /// Defaults plus the script-facing options of `cad.savePdf(path, opts)`
    /// (`colors`, `lineweights`, `skipLayers`). Other keys, such as `path`,
    /// are ignored.
    pub fn from_args(args: &serde_json::Value) -> Result<Self, String> {
        let mut opts = Self::default();
        match args.get("colors") {
            None | Some(serde_json::Value::Null) => {}
            Some(serde_json::Value::String(s)) => opts.colors = PdfColorMode::parse(s)?,
            Some(other) => return Err(format!("savePdf: colors must be a string, got {other}")),
        }
        match args.get("lineweights") {
            None | Some(serde_json::Value::Null) => {}
            Some(serde_json::Value::Bool(b)) => opts.lineweights = *b,
            Some(other) => {
                return Err(format!(
                    "savePdf: lineweights must be true or false, got {other}"
                ));
            }
        }
        match args.get("skipLayers") {
            None | Some(serde_json::Value::Null) => {}
            Some(serde_json::Value::Array(names)) => {
                for name in names {
                    match name.as_str() {
                        Some(s) => opts.skip_layers.push(s.to_string()),
                        None => {
                            return Err(format!(
                                "savePdf: skipLayers must be an array of layer names, got {name}"
                            ));
                        }
                    }
                }
            }
            Some(other) => {
                return Err(format!(
                    "savePdf: skipLayers must be an array of layer names, got {other}"
                ));
            }
        }
        Ok(opts)
    }
}

impl Default for PdfOptions {
    fn default() -> Self {
        Self {
            page_width_mm: 297.0,
            page_height_mm: 210.0,
            margin_mm: 10.0,
            skip_layers: Vec::new(),
            line_width_pt: 0.1,
            colors: PdfColorMode::default(),
            lineweights: false,
        }
    }
}

struct ExpandedEntity {
    shape: Shape,
    layer: String,
    color: Color,
    /// Stroke width in mm, when one was set on purpose (see
    /// `explicit_lineweight`).
    lineweight: Option<f64>,
}

const PT_PER_MM: f64 = 72.0 / 25.4;

/// The layer DWG reserves for dimension definition points. It never plots.
const NON_PLOTTING_LAYER: &str = "Defpoints";

/// The stroke width in mm when it was set on purpose: on the entity, or
/// on its layer as anything but the 0.25 default.
fn explicit_lineweight(doc: &Document, entity: Option<f64>, layer: &str) -> Option<f64> {
    let lw = entity.unwrap_or_else(|| doc.layer_lineweight(layer));
    ((lw - 0.25).abs() > 1e-9).then_some(lw)
}

/// Whether entities on `layer` go into the PDF. Left out: the layers the
/// script names in `skip_layers`, the layers the document marks
/// invisible, and `Defpoints`. A layer missing from the layer table is not
/// hidden, so it plots. No other layer name has any meaning here.
fn layer_plots(doc: &Document, opts: &PdfOptions, layer: &str) -> bool {
    layer != NON_PLOTTING_LAYER
        && !opts.skip_layers.iter().any(|s| s == layer)
        && !doc.layers.iter().any(|l| l.name == layer && !l.visible)
}

pub fn export_pdf(doc: &Document, opts: &PdfOptions) -> Vec<u8> {
    let expanded = expand_entities(doc);

    let visible: Vec<&ExpandedEntity> = expanded
        .iter()
        .filter(|e| layer_plots(doc, opts, &e.layer))
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

    let mut ops: Vec<Op> = Vec::new();
    // Paper mapping runs a few hundred colour conversions; drawings use
    // a handful of distinct colours, so map each one once.
    let mut mapped: std::collections::HashMap<[u32; 3], (f32, f32, f32)> =
        std::collections::HashMap::new();

    for ent in &visible {
        // The entity's own colour, or its layer's when it has none: the
        // same ByLayer rule the screen renderers use.
        let stored = color_f32(ent.color);
        let key = [stored.0.to_bits(), stored.1.to_bits(), stored.2.to_bits()];
        let (r, g, b) = *mapped
            .entry(key)
            .or_insert_with(|| map_color(opts.colors, stored));
        let lw = match ent.lineweight.filter(|_| opts.lineweights) {
            Some(mm) => (mm * PT_PER_MM) as f32,
            None => opts.line_width_pt,
        };

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

// ── Colour mapping ────────────────────────────────────────────────────
//
// Contrast is WCAG 2 contrast against white, (1.05) / (Y + 0.05), with Y
// the relative luminance of linear sRGB. Lightness and hue are OKLab /
// OKLCH (Björn Ottosson), which keeps hue stable while darkening.

/// Largest luminance with 4.5:1 contrast against white.
const MAX_LUM_CHROMATIC: f64 = 1.05 / 4.5 - 0.05;
/// Largest luminance with 3:1 contrast against white.
const MAX_LUM_NEUTRAL: f64 = 1.05 / 3.0 - 0.05;
/// OKLCH chroma below which a colour counts as neutral (grey).
const NEUTRAL_CHROMA: f64 = 0.03;

fn map_color(mode: PdfColorMode, rgb: (f32, f32, f32)) -> (f32, f32, f32) {
    match mode {
        PdfColorMode::Screen => rgb,
        PdfColorMode::Mono => (0.0, 0.0, 0.0),
        PdfColorMode::Paper => {
            let [r, g, b] = paper_color([rgb.0 as f64, rgb.1 as f64, rgb.2 as f64]);
            (r as f32, g as f32, b as f32)
        }
    }
}

/// Map one sRGB colour (0..1 per channel) for white paper.
fn paper_color(srgb: [f64; 3]) -> [f64; 3] {
    let lin = srgb.map(srgb_to_linear);
    let [l, a, b] = linear_to_oklab(lin);
    if a.hypot(b) < NEUTRAL_CHROMA {
        // Neutral: invert lightness (white -> black, a dim context grey
        // -> a quiet light grey), keep any faint tint, hold 3:1.
        darken_to(1.0 - l, a, b, MAX_LUM_NEUTRAL)
    } else if luminance(lin) <= MAX_LUM_CHROMATIC {
        srgb
    } else {
        darken_to(l, a, b, MAX_LUM_CHROMATIC)
    }
}

/// The lightest colour at OKLab lightness <= `l` with hue `atan2(b, a)`
/// whose luminance is at most `max_lum`. Chroma is kept, except where
/// the sRGB gamut forces it down. Returns sRGB.
fn darken_to(l: f64, a: f64, b: f64, max_lum: f64) -> [f64; 3] {
    let fits = |l: f64| luminance(oklab_in_gamut(l, a, b)) <= max_lum;
    let l = if fits(l) {
        l
    } else {
        // Black always fits, `l` does not: bisect on lightness.
        let (mut lo, mut hi) = (0.0, l);
        for _ in 0..40 {
            let mid = 0.5 * (lo + hi);
            if fits(mid) {
                lo = mid;
            } else {
                hi = mid;
            }
        }
        lo
    };
    oklab_in_gamut(l, a, b).map(linear_to_srgb)
}

/// OKLab to linear sRGB, scaling chroma down until the colour is inside
/// the sRGB gamut (hue unchanged).
fn oklab_in_gamut(l: f64, a: f64, b: f64) -> [f64; 3] {
    const EPS: f64 = 1e-9;
    let inside = |c: [f64; 3]| c.iter().all(|v| (-EPS..=1.0 + EPS).contains(v));
    let full = oklab_to_linear([l, a, b]);
    let lin = if inside(full) {
        full
    } else {
        let (mut lo, mut hi) = (0.0, 1.0);
        for _ in 0..40 {
            let mid = 0.5 * (lo + hi);
            if inside(oklab_to_linear([l, a * mid, b * mid])) {
                lo = mid;
            } else {
                hi = mid;
            }
        }
        oklab_to_linear([l, a * lo, b * lo])
    };
    lin.map(|v| v.clamp(0.0, 1.0))
}

fn srgb_to_linear(c: f64) -> f64 {
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

fn linear_to_srgb(c: f64) -> f64 {
    if c <= 0.0031308 {
        12.92 * c
    } else {
        1.055 * c.powf(1.0 / 2.4) - 0.055
    }
}

/// WCAG relative luminance of a linear sRGB colour.
fn luminance([r, g, b]: [f64; 3]) -> f64 {
    0.2126 * r + 0.7152 * g + 0.0722 * b
}

fn linear_to_oklab([r, g, b]: [f64; 3]) -> [f64; 3] {
    let l = (0.4122214708 * r + 0.5363325363 * g + 0.0514459929 * b).cbrt();
    let m = (0.2119034982 * r + 0.6806995451 * g + 0.1073969566 * b).cbrt();
    let s = (0.0883024619 * r + 0.2817188376 * g + 0.6299787005 * b).cbrt();
    [
        0.2104542553 * l + 0.7936177850 * m - 0.0040720468 * s,
        1.9779984951 * l - 2.4285922050 * m + 0.4505937099 * s,
        0.0259040371 * l + 0.7827717662 * m - 0.8086757660 * s,
    ]
}

fn oklab_to_linear([l, a, b]: [f64; 3]) -> [f64; 3] {
    let l_ = l + 0.3963377774 * a + 0.2158037573 * b;
    let m_ = l - 0.1055613458 * a - 0.0638541728 * b;
    let s_ = l - 0.0894841775 * a - 1.2914855480 * b;
    let (l3, m3, s3) = (l_ * l_ * l_, m_ * m_ * m_, s_ * s_ * s_);
    [
        4.0767416621 * l3 - 3.3077115913 * m3 + 0.2309699292 * s3,
        -1.2684380046 * l3 + 2.6097574011 * m3 - 0.3413193965 * s3,
        -0.0041960863 * l3 - 0.7034186147 * m3 + 1.7076147010 * s3,
    ]
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
        if matches!(&ent.shape, Shape::BlockInsert { .. }) {
            for placed in crate::document::placed_block_shapes(doc, ent) {
                result.push(ExpandedEntity {
                    lineweight: explicit_lineweight(doc, ent.lineweight, &placed.layer),
                    shape: placed.shape,
                    layer: placed.layer,
                    color: placed.color,
                });
            }
        } else {
            result.push(ExpandedEntity {
                shape: ent.shape.clone(),
                layer: ent.layer.clone(),
                color: ent.color.unwrap_or_else(|| doc.layer_color(&ent.layer)),
                lineweight: explicit_lineweight(doc, ent.lineweight, &ent.layer),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn skip_layers_option_parses_and_defaults_empty() {
        assert!(PdfOptions::default().skip_layers.is_empty());
        let o = PdfOptions::from_args(&serde_json::json!({ "skipLayers": ["A", "B"] })).unwrap();
        assert_eq!(o.skip_layers, vec!["A", "B"]);
        assert!(PdfOptions::from_args(&serde_json::json!({ "skipLayers": "A" })).is_err());
        assert!(PdfOptions::from_args(&serde_json::json!({ "skipLayers": [1] })).is_err());
    }

    #[test]
    fn only_skipped_hidden_and_defpoints_layers_are_left_out() {
        let mut doc = Document::new();
        let layers = ["A", "E_DEBUG", "E_WALLS", "S_WALL", "HIDDEN", "Defpoints"];
        for (i, layer) in layers.into_iter().enumerate() {
            // One line per layer, stacked, so the drawing has an extent.
            let y = i as f64 * 10.0;
            doc.ensure_layer(layer);
            doc.add_line(Point::new(0.0, y), Point::new(100.0, y), layer, None);
        }
        doc.layers
            .iter_mut()
            .find(|l| l.name == "HIDDEN")
            .unwrap()
            .visible = false;
        let defaults = PdfOptions::default();

        // No layer name has a meaning of its own: these all plot.
        for layer in ["A", "E_DEBUG", "E_WALLS", "S_WALL"] {
            assert!(layer_plots(&doc, &defaults, layer), "{layer}");
        }
        // A layer the document hides, and Defpoints, do not.
        assert!(!layer_plots(&doc, &defaults, "HIDDEN"));
        assert!(!layer_plots(&doc, &defaults, "Defpoints"));
        // A layer missing from the layer table is not hidden.
        assert!(layer_plots(&doc, &defaults, "NOT_IN_TABLE"));

        // skipLayers leaves out exactly the named layers.
        let skip = PdfOptions {
            skip_layers: vec!["E_DEBUG".to_string()],
            ..PdfOptions::default()
        };
        assert!(!layer_plots(&doc, &skip, "E_DEBUG"));
        assert!(layer_plots(&doc, &skip, "E_WALLS"));

        // And the export honours it: skipping a layer shrinks the output.
        assert!(export_pdf(&doc, &skip).len() < export_pdf(&doc, &defaults).len());
    }

    #[test]
    fn lineweights_option_parses_and_defaults_off() {
        assert!(
            !PdfOptions::from_args(&serde_json::json!({}))
                .unwrap()
                .lineweights
        );
        assert!(
            PdfOptions::from_args(&serde_json::json!({ "lineweights": true }))
                .unwrap()
                .lineweights
        );
        assert!(PdfOptions::from_args(&serde_json::json!({ "lineweights": "yes" })).is_err());
    }

    #[test]
    fn explicit_lineweight_ignores_the_default() {
        let doc = Document::default();
        // No such layer: the layer default (0.25) is not an explicit weight.
        assert_eq!(explicit_lineweight(&doc, None, "nope"), None);
        assert_eq!(explicit_lineweight(&doc, Some(0.25), "nope"), None);
        assert_eq!(explicit_lineweight(&doc, Some(0.8), "nope"), Some(0.8));
    }

    fn rgb8(r: u8, g: u8, b: u8) -> [f64; 3] {
        [r as f64 / 255.0, g as f64 / 255.0, b as f64 / 255.0]
    }

    fn contrast_on_white(srgb: [f64; 3]) -> f64 {
        1.05 / (luminance(srgb.map(srgb_to_linear)) + 0.05)
    }

    fn hue_deg(srgb: [f64; 3]) -> f64 {
        let [_, a, b] = linear_to_oklab(srgb.map(srgb_to_linear));
        b.atan2(a).to_degrees()
    }

    fn chroma(srgb: [f64; 3]) -> f64 {
        let [_, a, b] = linear_to_oklab(srgb.map(srgb_to_linear));
        a.hypot(b)
    }

    fn hue_diff(a: f64, b: f64) -> f64 {
        let d = (a - b).rem_euclid(360.0);
        d.min(360.0 - d)
    }

    #[test]
    fn white_becomes_black() {
        let out = paper_color([1.0, 1.0, 1.0]);
        assert!(out.iter().all(|c| *c < 0.01), "{out:?}");
    }

    #[test]
    fn light_colours_reach_4_5_and_keep_hue() {
        for src in [
            rgb8(255, 255, 0),
            rgb8(0, 200, 255),
            rgb8(127, 255, 127),
            rgb8(255, 200, 50),
        ] {
            assert!(
                contrast_on_white(src) < 4.5,
                "{src:?} should start below 4.5"
            );
            let out = paper_color(src);
            let c = contrast_on_white(out);
            assert!(c >= 4.5, "{src:?} -> {out:?}: contrast {c:.2}");
            assert!(
                c < 4.6,
                "{src:?} -> {out:?}: darkened further than needed ({c:.2})"
            );
            let dh = hue_diff(hue_deg(src), hue_deg(out));
            assert!(dh < 3.0, "{src:?} -> {out:?}: hue moved {dh:.1} deg");
        }
    }

    #[test]
    fn saturated_red_and_blue_change_little() {
        let blue = rgb8(0, 0, 255);
        assert_eq!(paper_color(blue), blue);

        let red = rgb8(255, 0, 0);
        let out = paper_color(red);
        assert!(contrast_on_white(out) >= 4.5);
        for (o, s) in out.iter().zip(red) {
            assert!((o - s).abs() < 0.1, "{red:?} -> {out:?}");
        }
        assert!(hue_diff(hue_deg(red), hue_deg(out)) < 3.0);
    }

    #[test]
    fn passing_colours_untouched() {
        for src in [rgb8(0, 0, 128), rgb8(128, 0, 0), rgb8(0, 100, 0)] {
            assert_eq!(paper_color(src), src);
        }
    }

    #[test]
    fn greys_stay_grey_with_3_to_1() {
        for src in [rgb8(80, 80, 80), rgb8(192, 192, 192), rgb8(51, 51, 51)] {
            let out = paper_color(src);
            assert!(chroma(out) < 0.01, "{src:?} -> {out:?} not grey");
            let c = contrast_on_white(out);
            assert!(c >= 3.0, "{src:?} -> {out:?}: contrast {c:.2}");
        }
        // Inversion keeps the order reversed: the light grey ends darker.
        assert!(paper_color(rgb8(192, 192, 192))[0] < paper_color(rgb8(80, 80, 80))[0]);
    }

    #[test]
    fn black_does_not_become_white() {
        let out = paper_color([0.0, 0.0, 0.0]);
        assert!(contrast_on_white(out) >= 3.0, "{out:?}");
    }

    #[test]
    fn mono_and_screen() {
        for src in [(1.0, 1.0, 1.0), (1.0, 1.0, 0.0), (0.2, 0.4, 0.6)] {
            assert_eq!(map_color(PdfColorMode::Mono, src), (0.0, 0.0, 0.0));
            assert_eq!(map_color(PdfColorMode::Screen, src), src);
        }
    }

    #[test]
    fn options_from_args() {
        let def = PdfOptions::from_args(&serde_json::json!({"path": "x.pdf"})).unwrap();
        assert_eq!(def.colors, PdfColorMode::Paper);
        for (s, m) in [
            ("paper", PdfColorMode::Paper),
            ("mono", PdfColorMode::Mono),
            ("screen", PdfColorMode::Screen),
        ] {
            let o = PdfOptions::from_args(&serde_json::json!({"colors": s})).unwrap();
            assert_eq!(o.colors, m);
        }
        assert!(PdfOptions::from_args(&serde_json::json!({"colors": "inverse"})).is_err());
        assert!(PdfOptions::from_args(&serde_json::json!({"colors": 1})).is_err());
    }
}
