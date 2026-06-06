//! QElectroTech .elmt file parser.
//!
//! Parses `.elmt` XML into [`BlockDef`] by mapping QET drawing primitives
//! to cadview's mathematical [`Shape`] types:
//!
//! | .elmt element | Shape |
//! |---|---|
//! | `<line>` | `Line` |
//! | `<ellipse>` (square) | `Circle` |
//! | `<ellipse>` (rect) | `Ellipse` |
//! | `<arc>` (square) | `Arc` (circular) |
//! | `<arc>` (rect) | `Ellipse` (elliptic arc) |
//! | `<rect>` | `Polyline { closed: true }` |
//! | `<polygon>` | `Polyline` (open/closed per attribute) |
//! | `<text>` | `Text` |
//!
//! Coordinates are translated so the hotspot becomes the origin.
//! QET uses a Y-down screen coordinate system; we flip to Y-up CAD.

use crate::{BlockDef, Color, Shape};
use kurbo::{Circle, Line, Point};
use quick_xml::events::{BytesStart, Event};
use quick_xml::Reader;
use std::f64::consts::PI;

/// Terminal (connection point) from an .elmt file.
#[derive(Debug, Clone)]
pub struct Terminal {
    pub position: Point,
    pub orientation: String, // "n", "s", "e", "w"
}

/// Result of parsing an .elmt file.
#[derive(Debug, Clone)]
pub struct ElmtSymbol {
    pub block: BlockDef,
    pub terminals: Vec<Terminal>,
    pub en_standard: String,          // e.g. "EN 60617: 11-15-03"
    pub names: Vec<(String, String)>, // (lang, name)
}

impl ElmtSymbol {
    /// English name, falling back to first available.
    pub fn name_en(&self) -> &str {
        self.names
            .iter()
            .find(|(lang, _)| lang == "en")
            .or_else(|| self.names.first())
            .map(|(_, n)| n.as_str())
            .unwrap_or("unnamed")
    }
}

// ── Attribute helpers ─────────────────────────────────────────────────

fn attr_f64(e: &BytesStart, name: &[u8]) -> Option<f64> {
    e.attributes()
        .filter_map(|a| a.ok())
        .find(|a| a.key.as_ref() == name)
        .and_then(|a| std::str::from_utf8(a.value.as_ref()).ok()?.parse().ok())
}

fn attr_str(e: &BytesStart, name: &[u8]) -> Option<String> {
    e.attributes()
        .filter_map(|a| a.ok())
        .find(|a| a.key.as_ref() == name)
        .and_then(|a| String::from_utf8(a.value.as_ref().to_vec()).ok())
}

/// Parse a QElectroTech .elmt file from XML string into an ElmtSymbol.
pub fn parse_elmt(xml: &str) -> Result<ElmtSymbol, String> {
    let mut reader = Reader::from_str(xml);

    let mut hotspot_x: f64 = 0.0;
    let mut hotspot_y: f64 = 0.0;
    let mut shapes: Vec<(Shape, String, Option<Color>)> = Vec::new();
    let mut terminals: Vec<Terminal> = Vec::new();
    let mut names: Vec<(String, String)> = Vec::new();
    let mut en_standard = String::new();
    let mut in_description = false;
    // State for accumulating text inside <name> and <informations>
    let mut current_name_lang: Option<String> = None;
    let mut text_buf = String::new();
    let mut in_informations = false;

    loop {
        match reader.read_event() {
            Ok(Event::Eof) => break,
            Ok(Event::Start(ref e)) | Ok(Event::Empty(ref e)) => {
                let tag = reader
                    .decoder()
                    .decode(e.name().as_ref())
                    .unwrap_or_default()
                    .to_string();

                match tag.as_str() {
                    "definition" => {
                        hotspot_x = attr_f64(e, b"hotspot_x").unwrap_or(0.0);
                        hotspot_y = attr_f64(e, b"hotspot_y").unwrap_or(0.0);
                    }
                    "name" => {
                        current_name_lang = Some(attr_str(e, b"lang").unwrap_or_default());
                        text_buf.clear();
                    }
                    "informations" => {
                        in_informations = true;
                        text_buf.clear();
                    }
                    "description" => {
                        in_description = true;
                    }

                    "line" if in_description => {
                        if let Some(shape) = parse_line(e, hotspot_x, hotspot_y) {
                            shapes.push((shape, String::new(), Some(Color::WHITE)));
                        }
                    }
                    "ellipse" if in_description => {
                        if let Some(shape) = parse_ellipse(e, hotspot_x, hotspot_y) {
                            shapes.push((shape, String::new(), Some(Color::WHITE)));
                        }
                    }
                    "arc" if in_description => {
                        if let Some(shape) = parse_arc(e, hotspot_x, hotspot_y) {
                            shapes.push((shape, String::new(), Some(Color::WHITE)));
                        }
                    }
                    "rect" if in_description => {
                        if let Some(shape) = parse_rect(e, hotspot_x, hotspot_y) {
                            shapes.push((shape, String::new(), Some(Color::WHITE)));
                        }
                    }
                    "polygon" if in_description => {
                        if let Some(shape) = parse_polygon(e, hotspot_x, hotspot_y) {
                            shapes.push((shape, String::new(), Some(Color::WHITE)));
                        }
                    }
                    "text" if in_description => {
                        if let Some(shape) = parse_text(e, hotspot_x, hotspot_y) {
                            shapes.push((shape, String::new(), Some(Color::WHITE)));
                        }
                    }
                    "terminal" if in_description => {
                        let x = attr_f64(e, b"x").unwrap_or(0.0);
                        let y = attr_f64(e, b"y").unwrap_or(0.0);
                        let orientation = attr_str(e, b"orientation").unwrap_or_default();
                        terminals.push(Terminal {
                            position: qet_to_cad(x, y, hotspot_x, hotspot_y),
                            orientation,
                        });
                    }
                    _ => {}
                }
            }
            Ok(Event::End(ref e)) => {
                let tag = reader
                    .decoder()
                    .decode(e.name().as_ref())
                    .unwrap_or_default()
                    .to_string();
                match tag.as_str() {
                    "name" => {
                        if let Some(lang) = current_name_lang.take() {
                            let txt = text_buf.trim().to_string();
                            if !txt.is_empty() {
                                names.push((lang, txt));
                            }
                        }
                        text_buf.clear();
                    }
                    "description" => {
                        in_description = false;
                    }
                    "informations" => {
                        in_informations = false;
                        en_standard = text_buf.trim().to_string();
                        text_buf.clear();
                    }
                    _ => {}
                }
            }
            Ok(Event::Text(ref t)) => {
                if current_name_lang.is_some() || in_informations {
                    let decoded = reader.decoder().decode(t.as_ref()).unwrap_or_default();
                    text_buf.push_str(&decoded);
                }
            }
            Err(e) => return Err(format!("XML parse error: {e}")),
            _ => {}
        }
    }

    // Derive block name from English name (or first name)
    let block_name = names
        .iter()
        .find(|(lang, _)| lang == "en")
        .or_else(|| names.first())
        .map(|(_, n)| n.clone())
        .unwrap_or_else(|| "unnamed".to_string());

    // Hotspot is already at origin (0,0) after coordinate transform.
    // This is the QET-intended insertion anchor.
    let insert_point = Point::ZERO;

    Ok(ElmtSymbol {
        block: BlockDef {
            name: block_name,
            shapes,
            insert_point,
            default_layer: String::new(),
        },
        terminals,
        en_standard,
        names,
    })
}

/// Load an .elmt file from disk.
pub fn load_elmt(path: &str) -> Result<ElmtSymbol, String> {
    let xml = std::fs::read_to_string(path).map_err(|e| format!("read {path}: {e}"))?;
    parse_elmt(&xml)
}

/// Load all .elmt files from a directory (non-recursive).
pub fn load_elmt_dir(dir: &str) -> Result<Vec<ElmtSymbol>, String> {
    let entries = std::fs::read_dir(dir).map_err(|e| format!("read dir {dir}: {e}"))?;
    let mut symbols = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|e| format!("dir entry: {e}"))?;
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) == Some("elmt") {
            match load_elmt(&path.to_string_lossy()) {
                Ok(sym) => symbols.push(sym),
                Err(e) => eprintln!("skip {}: {e}", path.display()),
            }
        }
    }
    Ok(symbols)
}

// ── Coordinate transform ──────────────────────────────────────────────

/// Convert QET screen coords (Y-down, hotspot = origin) to CAD coords (Y-up).
fn qet_to_cad(x: f64, y: f64, hx: f64, hy: f64) -> Point {
    Point::new(x - hx, -(y - hy))
}

// ── Shape parsers ─────────────────────────────────────────────────────

fn parse_line(e: &BytesStart, hx: f64, hy: f64) -> Option<Shape> {
    let x1 = attr_f64(e, b"x1")?;
    let y1 = attr_f64(e, b"y1")?;
    let x2 = attr_f64(e, b"x2")?;
    let y2 = attr_f64(e, b"y2")?;
    let p0 = qet_to_cad(x1, y1, hx, hy);
    let p1 = qet_to_cad(x2, y2, hx, hy);
    Some(Shape::Line(Line::new(p0, p1)))
}

fn parse_ellipse(e: &BytesStart, hx: f64, hy: f64) -> Option<Shape> {
    let x = attr_f64(e, b"x")?;
    let y = attr_f64(e, b"y")?;
    let w = attr_f64(e, b"width")?;
    let h = attr_f64(e, b"height")?;

    // QET ellipse: x,y is top-left of bounding box (screen coords, Y-down)
    // Center in QET coords:
    let cx = x + w / 2.0;
    let cy = y + h / 2.0;
    let center = qet_to_cad(cx, cy, hx, hy);

    if (w - h).abs() < 0.01 {
        // Circle
        Some(Shape::Circle(Circle::new(center, w / 2.0)))
    } else {
        // Full ellipse: major axis along the longer dimension
        let (major_axis, minor_ratio) = if w >= h {
            ((w / 2.0, 0.0), h / w)
        } else {
            ((0.0, h / 2.0), w / h) // Y-up: major axis vertical
        };
        Some(Shape::Ellipse {
            center,
            major_axis,
            minor_ratio,
            start_param: 0.0,
            end_param: 2.0 * PI,
        })
    }
}

fn parse_arc(e: &BytesStart, hx: f64, hy: f64) -> Option<Shape> {
    let x = attr_f64(e, b"x")?;
    let y = attr_f64(e, b"y")?;
    let w = attr_f64(e, b"width")?;
    let h = attr_f64(e, b"height")?;
    let start_deg = attr_f64(e, b"start")?; // QET: degrees, 0 = 3 o'clock, CCW
    let span_deg = attr_f64(e, b"angle")?; // sweep in degrees (positive = CCW)

    let cx = x + w / 2.0;
    let cy = y + h / 2.0;
    let center = qet_to_cad(cx, cy, hx, hy);

    // QET uses y = cy - ry*sin(theta): angles are standard math convention.
    // Keep as-is; the Y-flip in coordinate transform handles orientation.
    let sa = start_deg.to_radians();
    let ea = (start_deg + span_deg).to_radians();

    if (w - h).abs() < 0.01 {
        // Circular arc
        Some(Shape::Arc {
            center,
            radius: w / 2.0,
            start_angle: sa,
            end_angle: ea,
        })
    } else {
        // Elliptic arc
        let (major_axis, minor_ratio) = if w >= h {
            ((w / 2.0, 0.0), h / w)
        } else {
            ((0.0, h / 2.0), w / h)
        };
        Some(Shape::Ellipse {
            center,
            major_axis,
            minor_ratio,
            start_param: sa,
            end_param: ea,
        })
    }
}

fn parse_rect(e: &BytesStart, hx: f64, hy: f64) -> Option<Shape> {
    let x = attr_f64(e, b"x")?;
    let y = attr_f64(e, b"y")?;
    let w = attr_f64(e, b"width")?;
    let h = attr_f64(e, b"height")?;

    let p0 = qet_to_cad(x, y, hx, hy);
    let p1 = qet_to_cad(x + w, y, hx, hy);
    let p2 = qet_to_cad(x + w, y + h, hx, hy);
    let p3 = qet_to_cad(x, y + h, hx, hy);

    Some(Shape::Polyline {
        points: vec![p0, p1, p2, p3],
        closed: true,
    })
}

fn parse_polygon(e: &BytesStart, hx: f64, hy: f64) -> Option<Shape> {
    // QET polygons use x1,y1, x2,y2, x3,y3, ... (up to ~8 points seen)
    let mut points = Vec::new();
    for i in 1..=20 {
        let xk = format!("x{i}");
        let yk = format!("y{i}");
        match (attr_f64(e, xk.as_bytes()), attr_f64(e, yk.as_bytes())) {
            (Some(x), Some(y)) => points.push(qet_to_cad(x, y, hx, hy)),
            _ => break,
        }
    }
    if points.len() < 2 {
        return None;
    }

    let closed = attr_str(e, b"closed").map(|s| s != "false").unwrap_or(true); // QET default: closed unless explicitly "false"

    Some(Shape::Polyline { points, closed })
}

fn parse_text(e: &BytesStart, hx: f64, hy: f64) -> Option<Shape> {
    let text = attr_str(e, b"text")?;
    if text.is_empty() {
        return None;
    }
    let x = attr_f64(e, b"x").unwrap_or(0.0);
    let y = attr_f64(e, b"y").unwrap_or(0.0);
    let rotation = attr_f64(e, b"rotation").unwrap_or(0.0);
    let position = qet_to_cad(x, y, hx, hy);

    // Extract font size from the font string if present, else default 7pt
    let height = attr_str(e, b"font")
        .and_then(|f| {
            // Font string format: "Liberation Sans,7,-1,5,50,..."
            f.split(',').nth(1)?.parse::<f64>().ok()
        })
        .unwrap_or(7.0);

    Some(Shape::Text {
        text,
        position,
        height,
        rotation: rotation.to_radians(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_lamp() {
        let xml = include_str!("../../../vendor/qelectrotech-elements/lampe.elmt");
        let sym = parse_elmt(xml).unwrap();
        assert_eq!(sym.name_en(), "Light");
        // Should have: 1 ellipse (circle) + 2 lines
        let circles: Vec<_> = sym
            .block
            .shapes
            .iter()
            .filter(|(s, _, _)| matches!(s, Shape::Circle(_)))
            .collect();
        let lines: Vec<_> = sym
            .block
            .shapes
            .iter()
            .filter(|(s, _, _)| matches!(s, Shape::Line(_)))
            .collect();
        assert_eq!(circles.len(), 1, "expected 1 circle for lamp");
        assert_eq!(lines.len(), 2, "expected 2 lines (X cross) for lamp");
        assert_eq!(sym.terminals.len(), 1);
    }

    #[test]
    fn parse_switch() {
        let xml = include_str!("../../../vendor/qelectrotech-elements/interrupteur.elmt");
        let sym = parse_elmt(xml).unwrap();
        assert_eq!(sym.name_en(), "Switch");
        let circles: Vec<_> = sym
            .block
            .shapes
            .iter()
            .filter(|(s, _, _)| matches!(s, Shape::Circle(_)))
            .collect();
        let lines: Vec<_> = sym
            .block
            .shapes
            .iter()
            .filter(|(s, _, _)| matches!(s, Shape::Line(_)))
            .collect();
        assert_eq!(circles.len(), 1, "expected 1 circle for switch base");
        assert_eq!(lines.len(), 1, "expected 1 line (lever) for switch");
    }

    #[test]
    fn parse_socket() {
        let xml = include_str!(
            "../../../vendor/qelectrotech-elements/electrical_socket_11-13-01_en60617.elmt"
        );
        let sym = parse_elmt(xml).unwrap();
        assert!(sym.name_en().to_lowercase().contains("socket"));
        let arcs: Vec<_> = sym
            .block
            .shapes
            .iter()
            .filter(|(s, _, _)| matches!(s, Shape::Arc { .. }))
            .collect();
        let lines: Vec<_> = sym
            .block
            .shapes
            .iter()
            .filter(|(s, _, _)| matches!(s, Shape::Line(_)))
            .collect();
        assert_eq!(arcs.len(), 1, "expected 1 arc (semicircle) for socket");
        assert_eq!(lines.len(), 1, "expected 1 line (stem) for socket");
    }

    #[test]
    fn parse_double_socket() {
        let xml = include_str!(
            "../../../vendor/qelectrotech-elements/2_electrical_sockets_11-13-02_en60617.elmt"
        );
        let sym = parse_elmt(xml).unwrap();
        // Should have: 1 arc, 2 lines, 1 text ("2")
        let arcs: Vec<_> = sym
            .block
            .shapes
            .iter()
            .filter(|(s, _, _)| matches!(s, Shape::Arc { .. }))
            .collect();
        let texts: Vec<_> = sym
            .block
            .shapes
            .iter()
            .filter(|(s, _, _)| matches!(s, Shape::Text { .. }))
            .collect();
        assert_eq!(arcs.len(), 1);
        assert_eq!(texts.len(), 1);
    }

    #[test]
    fn parse_push_button() {
        let xml = include_str!("../../../vendor/qelectrotech-elements/bouton_poussoir.elmt");
        let sym = parse_elmt(xml).unwrap();
        assert_eq!(sym.name_en(), "Push button");
        // Two concentric circles
        let circles: Vec<_> = sym
            .block
            .shapes
            .iter()
            .filter(|(s, _, _)| matches!(s, Shape::Circle(_)))
            .collect();
        assert_eq!(circles.len(), 2, "expected 2 circles for push button");
    }

    #[test]
    fn parse_two_way_switch() {
        let xml = include_str!(
            "../../../vendor/qelectrotech-elements/interrupteur_unipolaire_va_et_vient.elmt"
        );
        let sym = parse_elmt(xml).unwrap();
        // Circle + 2 polygon lever arms
        let circles: Vec<_> = sym
            .block
            .shapes
            .iter()
            .filter(|(s, _, _)| matches!(s, Shape::Circle(_)))
            .collect();
        let polylines: Vec<_> = sym
            .block
            .shapes
            .iter()
            .filter(|(s, _, _)| matches!(s, Shape::Polyline { .. }))
            .collect();
        assert_eq!(circles.len(), 1);
        assert_eq!(polylines.len(), 2, "expected 2 polylines (lever arms)");
    }
}
