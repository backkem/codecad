//! Text-to-geometry: converts strings to kurbo BezPaths via skrifa.
//!
//! Text in a CAD tool IS geometry. Each glyph becomes a set of closed
//! BezPaths that the existing renderer can draw as polylines/curves.

use kurbo::{BezPath, Point};
use skrifa::instance::Size;
use skrifa::outline::DrawSettings;
use skrifa::raw::FontRef;
use skrifa::MetadataProvider;

/// Embedded font: Roboto Mono (OFL license, Google Fonts).
const FONT_BYTES: &[u8] = include_bytes!("../../../assets/RobotoMono.ttf");

/// Pen that converts skrifa outline commands to a kurbo BezPath.
struct KurboPen {
    path: BezPath,
    scale: f64,
    offset_x: f64,
    offset_y: f64,
}

impl skrifa::outline::OutlinePen for KurboPen {
    fn move_to(&mut self, x: f32, y: f32) {
        self.path.move_to(Point::new(
            self.offset_x + x as f64 * self.scale,
            self.offset_y + y as f64 * self.scale,
        ));
    }
    fn line_to(&mut self, x: f32, y: f32) {
        self.path.line_to(Point::new(
            self.offset_x + x as f64 * self.scale,
            self.offset_y + y as f64 * self.scale,
        ));
    }
    fn quad_to(&mut self, cx: f32, cy: f32, x: f32, y: f32) {
        self.path.quad_to(
            Point::new(self.offset_x + cx as f64 * self.scale, self.offset_y + cy as f64 * self.scale),
            Point::new(self.offset_x + x as f64 * self.scale, self.offset_y + y as f64 * self.scale),
        );
    }
    fn curve_to(&mut self, cx0: f32, cy0: f32, cx1: f32, cy1: f32, x: f32, y: f32) {
        self.path.curve_to(
            Point::new(self.offset_x + cx0 as f64 * self.scale, self.offset_y + cy0 as f64 * self.scale),
            Point::new(self.offset_x + cx1 as f64 * self.scale, self.offset_y + cy1 as f64 * self.scale),
            Point::new(self.offset_x + x as f64 * self.scale, self.offset_y + y as f64 * self.scale),
        );
    }
    fn close(&mut self) {
        self.path.close_path();
    }
}

/// Convert a text string to kurbo BezPaths at a given position and height.
///
/// `height` is the text height in drawing units (mm). The font is scaled
/// so that the cap height matches `height`. `rotation` is in radians.
///
/// Returns a Vec of BezPaths (one per glyph that has outlines).
pub fn text_to_paths(text: &str, x: f64, y: f64, height: f64) -> Vec<BezPath> {
    text_to_paths_rotated(text, x, y, height, 0.0)
}

/// Like `text_to_paths` but with rotation (radians) around the anchor point.
pub fn text_to_paths_rotated(text: &str, x: f64, y: f64, height: f64, rotation: f64) -> Vec<BezPath> {
    let font = FontRef::new(FONT_BYTES).expect("embedded font is valid");
    let outlines = font.outline_glyphs();

    // Use a fixed font size and compute scale from that
    let font_size = 1000.0_f32; // arbitrary units-per-em working size
    let size = Size::new(font_size);

    // Get metrics to compute scale
    let metrics = font.metrics(size, skrifa::instance::LocationRef::default());
    let cap_height = metrics.cap_height.unwrap_or(metrics.ascent) as f64;
    let scale = height / cap_height;

    let charmap = font.charmap();
    let glyph_metrics = font.glyph_metrics(size, skrifa::instance::LocationRef::default());

    let loc = skrifa::instance::LocationRef::default();

    // Generate paths at origin (no offset), then transform
    let mut paths = Vec::new();
    let mut cursor_x = 0.0;

    for ch in text.chars() {
        let gid = charmap.map(ch).unwrap_or_default();

        let mut pen = KurboPen {
            path: BezPath::new(),
            scale,
            offset_x: cursor_x,
            offset_y: 0.0,
        };

        if let Some(glyph) = outlines.get(gid) {
            let settings = DrawSettings::unhinted(size, loc);
            let _ = glyph.draw(settings, &mut pen);
            if !pen.path.is_empty() {
                paths.push(pen.path);
            }
        }

        let advance = glyph_metrics.advance_width(gid).unwrap_or(font_size * 0.6) as f64;
        cursor_x += advance * scale;
    }

    // Apply rotation around origin, then translate to (x, y)
    if rotation.abs() > 1e-10 || x.abs() > 1e-10 || y.abs() > 1e-10 {
        let xform = kurbo::Affine::translate((x, y)) * kurbo::Affine::rotate(rotation);
        for path in &mut paths {
            path.apply_affine(xform);
        }
    }

    paths
}

/// Compute the total width of a text string at a given height.
pub fn text_width(text: &str, height: f64) -> f64 {
    let font = FontRef::new(FONT_BYTES).expect("embedded font is valid");
    let font_size = 1000.0_f32;
    let size = Size::new(font_size);
    let metrics = font.metrics(size, skrifa::instance::LocationRef::default());
    let cap_height = metrics.cap_height.unwrap_or(metrics.ascent) as f64;
    let scale = height / cap_height;

    let charmap = font.charmap();
    let glyph_metrics = font.glyph_metrics(size, skrifa::instance::LocationRef::default());

    let mut width = 0.0;
    for ch in text.chars() {
        let gid = charmap.map(ch).unwrap_or_default();
        let advance = glyph_metrics.advance_width(gid).unwrap_or(font_size * 0.6) as f64;
        width += advance * scale;
    }
    width
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_hello() {
        let paths = text_to_paths("Hello", 0.0, 0.0, 10.0);
        assert!(!paths.is_empty(), "should produce glyph paths");
        // H, e, l, l, o = 5 glyphs with outlines
        assert!(paths.len() >= 4, "expected at least 4 glyph paths, got {}", paths.len());
    }

    #[test]
    fn width_scales_with_height() {
        let w1 = text_width("ABC", 10.0);
        let w2 = text_width("ABC", 20.0);
        assert!((w2 / w1 - 2.0).abs() < 0.01, "width should scale linearly with height");
    }
}
