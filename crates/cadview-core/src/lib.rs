//! cadview-core: 2D CAD document model with f64 geometry.
//!
//! Read-write document model for AI-native 2D CAD. Supports:
//! - DWG import via acadrust (Lines, Arcs, Circles, LwPolylines, Ellipses,
//!   Splines, Hatches, MText, Dimensions, Block Inserts)
//! - Programmatic entity creation/mutation
//! - JSON serialization for the JS sandbox ABI
//! - Geometry helper functions (distance, projection, polygon tests)

pub mod dispatch;
pub mod document;
pub mod dwg;
pub mod hatch;
pub mod tessellate;
pub mod types;

pub mod elmt;
pub mod geo;
pub mod pdf;
pub mod sync;
pub mod text;

// Re-export all public items so `use cadview_core::*` keeps working.
pub use dispatch::*;
pub use document::*;
pub use dwg::*;
pub use hatch::*;
pub use tessellate::*;
pub use types::*;

#[cfg(test)]
mod tests {
    use super::*;
    use kurbo::{BezPath, Circle, Line, ParamCurve, Point};
    use std::f64::consts::PI;

    #[test]
    fn empty_document() {
        let doc = Document::new();
        assert_eq!(doc.entities.len(), 0);
        assert!(doc.bounds().is_none());
    }

    #[test]
    fn add_and_remove_line() {
        let mut doc = Document::new();
        let id = doc.add_line(
            Point::new(0.0, 0.0),
            Point::new(5.0, 0.0),
            "TEST",
            Color::WHITE,
        );
        assert_eq!(doc.entities.len(), 1);
        assert_eq!(doc.entity(id).unwrap().id, id);

        let removed = doc.remove_entity(id);
        assert!(removed.is_some());
        assert_eq!(doc.entities.len(), 0);
    }

    #[test]
    fn entity_ids_are_unique() {
        let mut doc = Document::new();
        let id1 = doc.add_line(
            Point::new(0.0, 0.0),
            Point::new(1.0, 0.0),
            "0",
            Color::WHITE,
        );
        let id2 = doc.add_line(
            Point::new(0.0, 0.0),
            Point::new(0.0, 1.0),
            "0",
            Color::WHITE,
        );
        let id3 = doc.add_circle(Point::new(0.0, 0.0), 1.0, "0", Color::WHITE);
        assert_ne!(id1, id2);
        assert_ne!(id2, id3);
    }

    #[test]
    fn move_entity() {
        let mut doc = Document::new();
        let id = doc.add_line(
            Point::new(0.0, 0.0),
            Point::new(5.0, 0.0),
            "0",
            Color::WHITE,
        );
        assert!(doc.move_entity(id, 10.0, 20.0));
        let ent = doc.entity(id).unwrap();
        if let Shape::Line(l) = &ent.shape {
            assert!((l.p0.x - 10.0).abs() < 1e-10);
            assert!((l.p0.y - 20.0).abs() < 1e-10);
            assert!((l.p1.x - 15.0).abs() < 1e-10);
            assert!((l.p1.y - 20.0).abs() < 1e-10);
        } else {
            panic!("expected line");
        }
    }

    #[test]
    fn copy_entity() {
        let mut doc = Document::new();
        let id = doc.add_line(
            Point::new(0.0, 0.0),
            Point::new(5.0, 0.0),
            "0",
            Color::WHITE,
        );
        let new_id = doc.copy_entity(id, 10.0, 20.0).unwrap();
        assert_ne!(id, new_id);
        if let Shape::Line(l) = &doc.entity(id).unwrap().shape {
            assert!((l.p0.x).abs() < 1e-10);
        } else {
            panic!("expected line");
        }
        if let Shape::Line(l) = &doc.entity(new_id).unwrap().shape {
            assert!((l.p0.x - 10.0).abs() < 1e-10);
            assert!((l.p0.y - 20.0).abs() < 1e-10);
            assert!((l.p1.x - 15.0).abs() < 1e-10);
        } else {
            panic!("expected line");
        }
        assert_eq!(doc.entities.len(), 2);
    }

    #[test]
    fn rotate_entity() {
        let mut doc = Document::new();
        let id = doc.add_line(
            Point::new(1.0, 0.0),
            Point::new(2.0, 0.0),
            "0",
            Color::WHITE,
        );
        assert!(doc.rotate_entity(id, Point::new(0.0, 0.0), 90.0));
        if let Shape::Line(l) = &doc.entity(id).unwrap().shape {
            assert!((l.p0.x).abs() < 1e-10);
            assert!((l.p0.y - 1.0).abs() < 1e-10);
            assert!((l.p1.x).abs() < 1e-10);
            assert!((l.p1.y - 2.0).abs() < 1e-10);
        } else {
            panic!("expected line");
        }
    }

    #[test]
    fn mirror_entity_x_axis() {
        let mut doc = Document::new();
        let id = doc.add_line(
            Point::new(1.0, 2.0),
            Point::new(3.0, 4.0),
            "0",
            Color::WHITE,
        );
        assert!(doc.mirror_entity(id, Point::new(0.0, 0.0), Point::new(1.0, 0.0)));
        if let Shape::Line(l) = &doc.entity(id).unwrap().shape {
            assert!((l.p0.x - 1.0).abs() < 1e-10);
            assert!((l.p0.y + 2.0).abs() < 1e-10);
            assert!((l.p1.x - 3.0).abs() < 1e-10);
            assert!((l.p1.y + 4.0).abs() < 1e-10);
        } else {
            panic!("expected line");
        }
    }

    #[test]
    fn mirror_entity_vertical() {
        let mut doc = Document::new();
        let id = doc.add_line(
            Point::new(1.0, 0.0),
            Point::new(3.0, 0.0),
            "0",
            Color::WHITE,
        );
        assert!(doc.mirror_entity(id, Point::new(5.0, 0.0), Point::new(5.0, 1.0)));
        if let Shape::Line(l) = &doc.entity(id).unwrap().shape {
            assert!((l.p0.x - 9.0).abs() < 1e-10);
            assert!((l.p0.y).abs() < 1e-10);
            assert!((l.p1.x - 7.0).abs() < 1e-10);
        } else {
            panic!("expected line");
        }
    }

    #[test]
    fn cad_call_copy() {
        let mut doc = Document::new();
        cad_call(&mut doc, "addLine", r#"{"start":[0,0],"end":[5,0]}"#).unwrap();
        let result = cad_call(&mut doc, "copy", r#"{"target":"e_1","dx":10,"dy":0}"#).unwrap();
        let v: serde_json::Value = serde_json::from_str(&result).unwrap();
        assert_eq!(v.as_array().unwrap().len(), 1);
        assert_eq!(doc.entities.len(), 2);
    }

    #[test]
    fn cad_call_rotate() {
        let mut doc = Document::new();
        cad_call(&mut doc, "addLine", r#"{"start":[1,0],"end":[2,0]}"#).unwrap();
        let result = cad_call(
            &mut doc,
            "rotate",
            r#"{"target":"e_1","center":[0,0],"angle":90}"#,
        )
        .unwrap();
        let v: serde_json::Value = serde_json::from_str(&result).unwrap();
        assert_eq!(v["rotated"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn cad_call_mirror() {
        let mut doc = Document::new();
        cad_call(&mut doc, "addLine", r#"{"start":[1,2],"end":[3,4]}"#).unwrap();
        let result = cad_call(
            &mut doc,
            "mirror",
            r#"{"target":"e_1","p1":[0,0],"p2":[1,0]}"#,
        )
        .unwrap();
        let v: serde_json::Value = serde_json::from_str(&result).unwrap();
        assert_eq!(v["mirrored"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn bounds_update() {
        let mut doc = Document::new();
        doc.add_line(
            Point::new(0.0, 0.0),
            Point::new(10.0, 5.0),
            "0",
            Color::WHITE,
        );
        doc.add_circle(Point::new(20.0, 20.0), 3.0, "0", Color::WHITE);
        let (x0, y0, x1, y1) = doc.bounds().unwrap();
        assert!((x0 - 0.0).abs() < 1e-10);
        assert!((y0 - 0.0).abs() < 1e-10);
        assert!((x1 - 23.0).abs() < 1e-10);
        assert!((y1 - 23.0).abs() < 1e-10);
    }

    #[test]
    fn cad_call_describe() {
        let mut doc = Document::new();
        doc.add_line(
            Point::new(0.0, 0.0),
            Point::new(5.0, 0.0),
            "S_WALL",
            Color::WHITE,
        );
        doc.add_circle(Point::new(3.0, 3.0), 1.0, "E_LITE", Color::WHITE);

        let result = cad_call(&mut doc, "describe", "{}").unwrap();
        let v: serde_json::Value = serde_json::from_str(&result).unwrap();
        assert_eq!(v["entities"], 2);
    }

    #[test]
    fn cad_call_add_and_query() {
        let mut doc = Document::new();

        let result = cad_call(&mut doc, "addLine", r#"{"start":[0,0],"end":[5,3]}"#).unwrap();
        let v: serde_json::Value = serde_json::from_str(&result).unwrap();
        assert_eq!(v["type"], "line");
        assert!(v["id"].as_str().unwrap().starts_with("e_"));

        let id = v["id"].as_str().unwrap();
        let result = cad_call(&mut doc, "entity", &format!(r#"{{"id":"{id}"}}"#)).unwrap();
        let v2: serde_json::Value = serde_json::from_str(&result).unwrap();
        assert_eq!(v2["id"], id);
    }

    #[test]
    fn cad_call_remove_with_array() {
        let mut doc = Document::new();
        cad_call(&mut doc, "addLine", r#"{"start":[0,0],"end":[1,0]}"#).unwrap();
        cad_call(&mut doc, "addLine", r#"{"start":[0,0],"end":[0,1]}"#).unwrap();
        assert_eq!(doc.entities.len(), 2);

        cad_call(&mut doc, "remove", r#"{"target":["e_1","e_2"]}"#).unwrap();
        assert_eq!(doc.entities.len(), 0);
    }

    #[test]
    fn cad_call_move() {
        let mut doc = Document::new();
        cad_call(&mut doc, "addLine", r#"{"start":[0,0],"end":[5,0]}"#).unwrap();
        cad_call(&mut doc, "move", r#"{"target":"e_1","dx":10,"dy":20}"#).unwrap();

        let ent = doc.entity(EntityId(1)).unwrap();
        if let Shape::Line(l) = &ent.shape {
            assert!((l.p0.x - 10.0).abs() < 1e-10);
            assert!((l.p0.y - 20.0).abs() < 1e-10);
        }
    }

    #[test]
    fn add_polyline() {
        let mut doc = Document::new();
        let id = doc.add_polyline(
            vec![
                Point::new(0.0, 0.0),
                Point::new(5.0, 0.0),
                Point::new(5.0, 3.0),
            ],
            true,
            "S_WALL",
            Color::WHITE,
        );
        let ent = doc.entity(id).unwrap();
        if let Shape::Polyline { points, closed } = &ent.shape {
            assert_eq!(points.len(), 3);
            assert!(*closed);
        } else {
            panic!("expected polyline");
        }
    }

    #[test]
    fn ensure_layer_creates_once() {
        let mut doc = Document::new();
        doc.ensure_layer("TEST");
        doc.ensure_layer("TEST");
        assert_eq!(doc.layers.iter().filter(|l| l.name == "TEST").count(), 1);
    }

    #[test]
    fn trim_line_keep_start() {
        let mut doc = Document::new();
        let id = doc.add_line(
            Point::new(0.0, 0.0),
            Point::new(100.0, 0.0),
            "0",
            Color::WHITE,
        );
        let new_id = doc.trim_entity(id, Point::new(60.0, 0.0), "start").unwrap();
        assert!(doc.entity(id).is_none());
        let ent = doc.entity(new_id).unwrap();
        if let Shape::Line(l) = &ent.shape {
            assert!((l.p0.x - 0.0).abs() < 1e-6);
            assert!((l.p1.x - 60.0).abs() < 1e-6);
        } else {
            panic!("expected line");
        }
    }

    #[test]
    fn trim_line_keep_end() {
        let mut doc = Document::new();
        let id = doc.add_line(
            Point::new(0.0, 0.0),
            Point::new(100.0, 0.0),
            "0",
            Color::WHITE,
        );
        let new_id = doc.trim_entity(id, Point::new(40.0, 0.0), "end").unwrap();
        let ent = doc.entity(new_id).unwrap();
        if let Shape::Line(l) = &ent.shape {
            assert!((l.p0.x - 40.0).abs() < 1e-6);
            assert!((l.p1.x - 100.0).abs() < 1e-6);
        } else {
            panic!("expected line");
        }
    }

    #[test]
    fn trim_arc_keep_from() {
        let mut doc = Document::new();
        let id = doc.add_arc(
            Point::new(0.0, 0.0),
            50.0,
            0.0_f64.to_radians(),
            90.0_f64.to_radians(),
            "0",
            Color::WHITE,
        );
        let cut = Point::new(
            50.0 * 45.0_f64.to_radians().cos(),
            50.0 * 45.0_f64.to_radians().sin(),
        );
        let new_id = doc.trim_entity(id, cut, "from").unwrap();
        let ent = doc.entity(new_id).unwrap();
        if let Shape::Arc {
            start_angle,
            end_angle,
            ..
        } = &ent.shape
        {
            assert!((start_angle.to_degrees() - 0.0).abs() < 1.0);
            assert!((end_angle.to_degrees() - 45.0).abs() < 1.0);
        } else {
            panic!("expected arc");
        }
    }

    #[test]
    fn trim_arc_keep_to() {
        let mut doc = Document::new();
        let id = doc.add_arc(
            Point::new(0.0, 0.0),
            50.0,
            0.0_f64.to_radians(),
            90.0_f64.to_radians(),
            "0",
            Color::WHITE,
        );
        let cut = Point::new(
            50.0 * 45.0_f64.to_radians().cos(),
            50.0 * 45.0_f64.to_radians().sin(),
        );
        let new_id = doc.trim_entity(id, cut, "to").unwrap();
        let ent = doc.entity(new_id).unwrap();
        if let Shape::Arc {
            start_angle,
            end_angle,
            ..
        } = &ent.shape
        {
            assert!((start_angle.to_degrees() - 45.0).abs() < 1.0);
            assert!((end_angle.to_degrees() - 90.0).abs() < 1.0);
        } else {
            panic!("expected arc");
        }
    }

    #[test]
    fn trim_via_cad_call() {
        let mut doc = Document::new();
        cad_call(&mut doc, "addLine", r#"{"start":[0,0],"end":[100,0]}"#).unwrap();
        let result = cad_call(
            &mut doc,
            "trim",
            r#"{"id":"e_1","cut":[70,0],"keep":"start"}"#,
        )
        .unwrap();
        let v: serde_json::Value = serde_json::from_str(&result).unwrap();
        assert_eq!(v["type"], "line");
        let end = v["end"].as_array().unwrap();
        assert!((end[0].as_f64().unwrap() - 70.0).abs() < 1e-6);
    }

    #[test]
    fn clone_block_with_text_replace() {
        let mut doc = Document::new();
        doc.define_block(BlockDef {
            name: "SOCKET_3".into(),
            shapes: vec![
                (
                    Shape::Circle(Circle::new(Point::new(0.0, 0.0), 10.0)),
                    String::new(),
                    Color::WHITE,
                ),
                (
                    Shape::Text {
                        text: "3".into(),
                        position: Point::new(5.0, 0.0),
                        height: 7.0,
                        rotation: 0.0,
                    },
                    String::new(),
                    Color::WHITE,
                ),
            ],
            insert_point: Point::ZERO,
            default_layer: "E_POWR".into(),
        });

        let result = cad_call(
            &mut doc,
            "clone",
            r#"{"source":"SOCKET_3","name":"SOCKET_5","replaceText":{"3":"5"}}"#,
        )
        .unwrap();
        let v: serde_json::Value = serde_json::from_str(&result).unwrap();
        assert_eq!(v["name"], "SOCKET_5");
        assert_eq!(v["clonedFrom"], "SOCKET_3");
        assert_eq!(v["shapeCount"], 2);

        let cloned = doc.blocks.get("SOCKET_5").unwrap();
        let text_shape = cloned
            .shapes
            .iter()
            .find(|(s, _, _)| matches!(s, Shape::Text { .. }))
            .unwrap();
        if let Shape::Text { text, .. } = &text_shape.0 {
            assert_eq!(text, "5");
        } else {
            panic!("expected text shape");
        }

        let orig = doc.blocks.get("SOCKET_3").unwrap();
        let orig_text = orig
            .shapes
            .iter()
            .find(|(s, _, _)| matches!(s, Shape::Text { .. }))
            .unwrap();
        if let Shape::Text { text, .. } = &orig_text.0 {
            assert_eq!(text, "3");
        }

        let ids1 = doc.place_block("SOCKET_3", Point::new(0.0, 0.0), 0.0, None);
        let ids2 = doc.place_block("SOCKET_5", Point::new(100.0, 0.0), 0.0, None);
        assert!(!ids1.is_empty());
        assert!(!ids2.is_empty());
    }

    // ── to_bezpath tests ──────────────────────────────────────────

    #[test]
    fn line_to_bezpath() {
        let s = Shape::Line(Line::new(Point::new(0.0, 0.0), Point::new(10.0, 5.0)));
        let path = s.to_bezpath().unwrap();
        let els: Vec<_> = path.elements().to_vec();
        assert_eq!(els.len(), 2);
        assert!(matches!(els[0], kurbo::PathEl::MoveTo(_)));
        assert!(matches!(els[1], kurbo::PathEl::LineTo(_)));
    }

    #[test]
    fn circle_to_bezpath() {
        let s = Shape::Circle(Circle::new(Point::new(5.0, 5.0), 3.0));
        let path = s.to_bezpath().unwrap();
        let els: Vec<_> = path.elements().to_vec();
        assert!(els.len() >= 4);
        assert!(matches!(els[0], kurbo::PathEl::MoveTo(_)));
        assert!(matches!(els.last().unwrap(), kurbo::PathEl::ClosePath));
    }

    #[test]
    fn arc_to_bezpath_endpoints() {
        let center = Point::new(0.0, 0.0);
        let r = 10.0;
        let s = Shape::Arc {
            center,
            radius: r,
            start_angle: 0.0,
            end_angle: PI / 2.0,
        };
        let path = s.to_bezpath().unwrap();
        let els: Vec<_> = path.elements().to_vec();
        if let kurbo::PathEl::MoveTo(p) = els[0] {
            assert!((p.x - r).abs() < 0.5, "start x should be ~{r}, got {}", p.x);
            assert!(p.y.abs() < 0.5, "start y should be ~0, got {}", p.y);
        } else {
            panic!("expected MoveTo");
        }
        let end = path.segments().last().unwrap().end();
        assert!(end.x.abs() < 0.5, "end x should be ~0, got {}", end.x);
        assert!(
            (end.y - r).abs() < 0.5,
            "end y should be ~{r}, got {}",
            end.y
        );
    }

    #[test]
    fn polyline_to_bezpath() {
        let s = Shape::Polyline {
            points: vec![
                Point::new(0.0, 0.0),
                Point::new(1.0, 0.0),
                Point::new(1.0, 1.0),
            ],
            closed: true,
        };
        let path = s.to_bezpath().unwrap();
        let els: Vec<_> = path.elements().to_vec();
        assert_eq!(els.len(), 4);
        assert!(matches!(els.last().unwrap(), kurbo::PathEl::ClosePath));
    }

    #[test]
    fn lwpolyline_straight_to_bezpath() {
        let s = Shape::LwPolyline {
            vertices: vec![
                LwVertex {
                    point: Point::new(0.0, 0.0),
                    bulge: 0.0,
                },
                LwVertex {
                    point: Point::new(10.0, 0.0),
                    bulge: 0.0,
                },
                LwVertex {
                    point: Point::new(10.0, 10.0),
                    bulge: 0.0,
                },
            ],
            closed: false,
        };
        let path = s.to_bezpath().unwrap();
        let els: Vec<_> = path.elements().to_vec();
        assert_eq!(els.len(), 3);
    }

    #[test]
    fn lwpolyline_bulge_to_bezpath() {
        let s = Shape::LwPolyline {
            vertices: vec![
                LwVertex {
                    point: Point::new(0.0, 0.0),
                    bulge: 1.0,
                },
                LwVertex {
                    point: Point::new(10.0, 0.0),
                    bulge: 0.0,
                },
            ],
            closed: false,
        };
        let path = s.to_bezpath().unwrap();
        let els: Vec<_> = path.elements().to_vec();
        assert!(els.len() >= 2);
        assert!(
            els.iter().any(|e| matches!(e, kurbo::PathEl::CurveTo(..))),
            "bulge arc should produce CurveTo elements"
        );
    }

    #[test]
    fn curve_path_to_bezpath() {
        let mut bp = BezPath::new();
        bp.move_to(Point::new(0.0, 0.0));
        bp.curve_to(
            Point::new(1.0, 2.0),
            Point::new(3.0, 2.0),
            Point::new(4.0, 0.0),
        );
        let s = Shape::CurvePath {
            path: bp.clone(),
            closed: false,
        };
        let path = s.to_bezpath().unwrap();
        assert_eq!(path.elements().len(), bp.elements().len());
    }

    #[test]
    fn ellipse_to_bezpath() {
        let s = Shape::Ellipse {
            center: Point::new(0.0, 0.0),
            major_axis: (10.0, 0.0),
            minor_ratio: 0.5,
            start_param: 0.0,
            end_param: 2.0 * PI,
        };
        let path = s.to_bezpath().unwrap();
        assert!(path.elements().len() >= 4);
    }

    #[test]
    fn block_insert_returns_none() {
        let s = Shape::BlockInsert {
            block_name: "X".to_string(),
            position: Point::ZERO,
            rotation: 0.0,
            x_scale: 1.0,
            y_scale: 1.0,
        };
        assert!(s.to_bezpath().is_none());
    }

    #[test]
    fn text_returns_none() {
        let s = Shape::Text {
            text: "hello".to_string(),
            position: Point::ZERO,
            height: 10.0,
            rotation: 0.0,
        };
        assert!(s.to_bezpath().is_none());
    }

    #[test]
    fn solid_fill_to_bezpath() {
        let s = Shape::SolidFill {
            boundary: vec![
                FillEdge::LineTo(Point::new(0.0, 0.0)),
                FillEdge::LineTo(Point::new(10.0, 0.0)),
                FillEdge::LineTo(Point::new(10.0, 10.0)),
                FillEdge::LineTo(Point::new(0.0, 10.0)),
            ],
            holes: vec![vec![
                FillEdge::LineTo(Point::new(2.0, 2.0)),
                FillEdge::LineTo(Point::new(8.0, 2.0)),
                FillEdge::LineTo(Point::new(8.0, 8.0)),
                FillEdge::LineTo(Point::new(2.0, 8.0)),
            ]],
        };
        let path = s.to_bezpath().unwrap();
        let moves = path
            .elements()
            .iter()
            .filter(|e| matches!(e, kurbo::PathEl::MoveTo(_)))
            .count();
        assert_eq!(moves, 2, "should have 2 MoveTo elements (boundary + hole)");
    }

    // ── block expansion tests ────────────────────────────────────

    fn make_block_doc() -> Document {
        let mut doc = Document::new();
        doc.define_block(BlockDef {
            name: "BOX".into(),
            shapes: vec![
                (
                    Shape::Line(Line::new(Point::new(0.0, 0.0), Point::new(10.0, 0.0))),
                    String::new(),
                    Color::WHITE,
                ),
                (
                    Shape::Circle(Circle::new(Point::new(5.0, 5.0), 2.0)),
                    "inner".to_string(),
                    Color::rgb(255, 0, 0),
                ),
            ],
            insert_point: Point::ZERO,
            default_layer: "0".into(),
        });
        doc
    }

    #[test]
    fn entities_expand_basic() {
        let mut doc = make_block_doc();
        cad_call(&mut doc, "place", r#"{"block":"BOX","at":[100,200]}"#).unwrap();
        let result = cad_call(&mut doc, "entities", r#"{"expand":true}"#).unwrap();
        let ents: Vec<serde_json::Value> = serde_json::from_str(&result).unwrap();
        // Should have 2 expanded sub-entities + 1 block_insert = 3
        assert_eq!(ents.len(), 3);
        let types: Vec<&str> = ents.iter().map(|e| e["type"].as_str().unwrap()).collect();
        assert!(types.contains(&"line"));
        assert!(types.contains(&"circle"));
        assert!(types.contains(&"block_insert"));
    }

    #[test]
    fn entities_expand_transforms_position() {
        let mut doc = make_block_doc();
        cad_call(&mut doc, "place", r#"{"block":"BOX","at":[100,200]}"#).unwrap();
        let result = cad_call(&mut doc, "entities", r#"{"expand":true}"#).unwrap();
        let ents: Vec<serde_json::Value> = serde_json::from_str(&result).unwrap();
        // The expanded line should be translated to [100,200]->[110,200]
        let line = ents.iter().find(|e| e["type"] == "line").unwrap();
        let start = line["start"].as_array().unwrap();
        assert!((start[0].as_f64().unwrap() - 100.0).abs() < 1e-6);
        assert!((start[1].as_f64().unwrap() - 200.0).abs() < 1e-6);
        let end = line["end"].as_array().unwrap();
        assert!((end[0].as_f64().unwrap() - 110.0).abs() < 1e-6);
    }

    #[test]
    fn entities_expand_with_rotation() {
        let mut doc = make_block_doc();
        // Place at origin with 90-degree rotation
        cad_call(
            &mut doc,
            "place",
            r#"{"block":"BOX","at":[0,0],"rotation":90}"#,
        )
        .unwrap();
        let result = cad_call(&mut doc, "entities", r#"{"expand":true}"#).unwrap();
        let ents: Vec<serde_json::Value> = serde_json::from_str(&result).unwrap();
        let line = ents.iter().find(|e| e["type"] == "line").unwrap();
        // Original line (0,0)->(10,0) rotated 90° CCW => (0,0)->(0,10)
        let end = line["end"].as_array().unwrap();
        assert!(
            end[0].as_f64().unwrap().abs() < 1e-6,
            "rotated end x should be ~0"
        );
        assert!(
            (end[1].as_f64().unwrap() - 10.0).abs() < 1e-6,
            "rotated end y should be ~10"
        );
    }

    #[test]
    fn entities_expand_with_scale() {
        let mut doc = make_block_doc();
        cad_call(&mut doc, "place", r#"{"block":"BOX","at":[0,0],"scale":2}"#).unwrap();
        let result = cad_call(&mut doc, "entities", r#"{"expand":true}"#).unwrap();
        let ents: Vec<serde_json::Value> = serde_json::from_str(&result).unwrap();
        let line = ents.iter().find(|e| e["type"] == "line").unwrap();
        let end = line["end"].as_array().unwrap();
        assert!(
            (end[0].as_f64().unwrap() - 20.0).abs() < 1e-6,
            "scaled end x should be 20"
        );
    }

    #[test]
    fn entities_expand_layer_inheritance() {
        let mut doc = make_block_doc();
        cad_call(&mut doc, "place", r#"{"block":"BOX","at":[0,0]}"#).unwrap();
        let result = cad_call(&mut doc, "entities", r#"{"expand":true}"#).unwrap();
        let ents: Vec<serde_json::Value> = serde_json::from_str(&result).unwrap();
        // Shape with empty layer inherits from the block_insert's layer
        let line = ents.iter().find(|e| e["type"] == "line").unwrap();
        assert_eq!(line["layer"], "0");
        // Shape with explicit layer keeps it
        let circle = ents.iter().find(|e| e["type"] == "circle").unwrap();
        assert_eq!(circle["layer"], "inner");
    }

    #[test]
    fn entities_expand_color_inheritance() {
        let mut doc = make_block_doc();
        // Place with a non-white layer color
        cad_call(
            &mut doc,
            "addLayer",
            r#"{"name":"colored","color":[0,128,255]}"#,
        )
        .unwrap();
        cad_call(
            &mut doc,
            "place",
            r#"{"block":"BOX","at":[0,0],"layer":"colored"}"#,
        )
        .unwrap();
        let result = cad_call(&mut doc, "entities", r#"{"expand":true}"#).unwrap();
        let ents: Vec<serde_json::Value> = serde_json::from_str(&result).unwrap();
        // The line shape has Color::WHITE, so it should inherit the insert's color
        let line = ents.iter().find(|e| e["type"] == "line").unwrap();
        let c = line["color"].as_array().unwrap();
        assert_eq!(c[0].as_u64().unwrap(), 0);
        assert_eq!(c[1].as_u64().unwrap(), 128);
        assert_eq!(c[2].as_u64().unwrap(), 255);
        // The circle has explicit red, should keep it
        let circle = ents.iter().find(|e| e["type"] == "circle").unwrap();
        let c = circle["color"].as_array().unwrap();
        assert_eq!(c[0].as_u64().unwrap(), 255);
        assert_eq!(c[1].as_u64().unwrap(), 0);
    }

    // ── children method tests ────────────────────────────────────

    #[test]
    fn children_of_block_insert() {
        let mut doc = make_block_doc();
        let result = cad_call(&mut doc, "place", r#"{"block":"BOX","at":[50,60]}"#).unwrap();
        let placed: serde_json::Value = serde_json::from_str(&result).unwrap();
        let id = placed["id"].as_str().unwrap();

        let result = cad_call(&mut doc, "children", &format!(r#"{{"id":"{}"}}"#, id)).unwrap();
        let children: Vec<serde_json::Value> = serde_json::from_str(&result).unwrap();
        assert_eq!(children.len(), 2);
        let types: Vec<&str> = children
            .iter()
            .map(|e| e["type"].as_str().unwrap())
            .collect();
        assert!(types.contains(&"line"));
        assert!(types.contains(&"circle"));
    }

    #[test]
    fn children_of_non_block_returns_empty() {
        let mut doc = Document::new();
        cad_call(&mut doc, "addLine", r#"{"start":[0,0],"end":[10,0]}"#).unwrap();
        let result = cad_call(&mut doc, "children", r#"{"id":"e_1"}"#).unwrap();
        let children: Vec<serde_json::Value> = serde_json::from_str(&result).unwrap();
        assert!(children.is_empty());
    }

    #[test]
    fn children_transforms_match_expand() {
        let mut doc = make_block_doc();
        let result = cad_call(
            &mut doc,
            "place",
            r#"{"block":"BOX","at":[10,20],"rotation":45}"#,
        )
        .unwrap();
        let placed: serde_json::Value = serde_json::from_str(&result).unwrap();
        let id = placed["id"].as_str().unwrap();

        let children_result =
            cad_call(&mut doc, "children", &format!(r#"{{"id":"{}"}}"#, id)).unwrap();
        let children: Vec<serde_json::Value> = serde_json::from_str(&children_result).unwrap();

        let expand_result = cad_call(&mut doc, "entities", r#"{"expand":true}"#).unwrap();
        let expanded: Vec<serde_json::Value> = serde_json::from_str(&expand_result).unwrap();
        let expanded_line = expanded.iter().find(|e| e["type"] == "line").unwrap();
        let children_line = children.iter().find(|e| e["type"] == "line").unwrap();

        // Same transform applied, so positions should match
        assert_eq!(expanded_line["start"], children_line["start"]);
        assert_eq!(expanded_line["end"], children_line["end"]);
    }

    // ── layer filter tests ───────────────────────────────────────

    #[test]
    fn entities_layer_filter() {
        let mut doc = Document::new();
        cad_call(
            &mut doc,
            "addLayer",
            r#"{"name":"walls","color":[255,255,255]}"#,
        )
        .unwrap();
        cad_call(
            &mut doc,
            "addLayer",
            r#"{"name":"doors","color":[0,255,0]}"#,
        )
        .unwrap();
        cad_call(
            &mut doc,
            "addLine",
            r#"{"start":[0,0],"end":[10,0],"layer":"walls"}"#,
        )
        .unwrap();
        cad_call(
            &mut doc,
            "addLine",
            r#"{"start":[0,0],"end":[0,10],"layer":"walls"}"#,
        )
        .unwrap();
        cad_call(
            &mut doc,
            "addCircle",
            r#"{"center":[5,5],"radius":1,"layer":"doors"}"#,
        )
        .unwrap();

        let all = cad_call(&mut doc, "entities", "{}").unwrap();
        let all: Vec<serde_json::Value> = serde_json::from_str(&all).unwrap();
        assert_eq!(all.len(), 3);

        let walls = cad_call(&mut doc, "entities", r#"{"layer":"walls"}"#).unwrap();
        let walls: Vec<serde_json::Value> = serde_json::from_str(&walls).unwrap();
        assert_eq!(walls.len(), 2);
        assert!(walls.iter().all(|e| e["layer"] == "walls"));

        let doors = cad_call(&mut doc, "entities", r#"{"layer":"doors"}"#).unwrap();
        let doors: Vec<serde_json::Value> = serde_json::from_str(&doors).unwrap();
        assert_eq!(doors.len(), 1);
        assert_eq!(doors[0]["type"], "circle");
    }

    #[test]
    fn entities_layer_filter_with_expand() {
        let mut doc = make_block_doc();
        cad_call(
            &mut doc,
            "addLine",
            r#"{"start":[0,0],"end":[1,0],"layer":"inner"}"#,
        )
        .unwrap();
        cad_call(&mut doc, "place", r#"{"block":"BOX","at":[0,0]}"#).unwrap();

        // Filter to "inner" layer with expand: should get the standalone line
        // + the expanded circle (which has layer "inner" in the block def)
        let result = cad_call(&mut doc, "entities", r#"{"layer":"inner","expand":true}"#).unwrap();
        let ents: Vec<serde_json::Value> = serde_json::from_str(&result).unwrap();
        assert!(ents.iter().all(|e| e["layer"] == "inner"));
        assert_eq!(ents.len(), 2); // standalone line + expanded circle
    }

    // ── mtext/text summary field tests ───────────────────────────

    #[test]
    fn text_entity_has_top_level_fields() {
        let mut doc = Document::new();
        doc.entities.push(DrawEntity {
            id: EntityId(1),
            layer: "0".to_string(),
            color: Color::WHITE,
            shape: Shape::Text {
                text: "Hello".into(),
                position: Point::new(10.0, 20.0),
                height: 5.0,
                rotation: 0.0,
            },
            dash: None,
        });
        let result = cad_call(&mut doc, "entities", "{}").unwrap();
        let ents: Vec<serde_json::Value> = serde_json::from_str(&result).unwrap();
        assert_eq!(ents[0]["text"], "Hello");
        assert_eq!(ents[0]["height"], 5.0);
        let start = ents[0]["start"].as_array().unwrap();
        assert_eq!(start[0].as_f64().unwrap(), 10.0);
        assert_eq!(start[1].as_f64().unwrap(), 20.0);
    }

    #[test]
    fn mtext_entity_has_top_level_fields() {
        let mut doc = Document::new();
        doc.entities.push(DrawEntity {
            id: EntityId(1),
            layer: "0".to_string(),
            color: Color::WHITE,
            shape: Shape::MText {
                text: "\\Arich\\P".into(),
                plain_text: "Hello World".into(),
                position: Point::new(30.0, 40.0),
                height: 7.5,
                rotation: 0.5,
            },
            dash: None,
        });
        let result = cad_call(&mut doc, "entities", "{}").unwrap();
        let ents: Vec<serde_json::Value> = serde_json::from_str(&result).unwrap();
        assert_eq!(ents[0]["text"], "Hello World");
        assert_eq!(ents[0]["height"], 7.5);
        assert!((ents[0]["rotation"].as_f64().unwrap() - 0.5_f64.to_degrees()).abs() < 1e-6);
    }

    // ── expand_for_render tests ──────────────────────────────────

    #[test]
    fn expand_for_render_blocks() {
        let mut doc = make_block_doc();
        doc.place_block("BOX", Point::new(10.0, 20.0), 0.0, None);
        let expanded = expand_for_render(&doc);
        // Should have expanded sub-entities from the block (line + circle)
        assert!(expanded.iter().any(|e| matches!(&e.shape, Shape::Line(_))));
        assert!(expanded
            .iter()
            .any(|e| matches!(&e.shape, Shape::Circle(_))));
        // No BlockInsert in expanded output
        assert!(!expanded
            .iter()
            .any(|e| matches!(&e.shape, Shape::BlockInsert { .. })));
    }

    #[test]
    fn expand_for_render_text() {
        let mut doc = Document::new();
        doc.entities.push(DrawEntity {
            id: EntityId(1),
            layer: "0".into(),
            color: Color::WHITE,
            shape: Shape::Text {
                text: "Hi".into(),
                position: Point::new(0.0, 0.0),
                height: 10.0,
                rotation: 0.0,
            },
            dash: None,
        });
        let expanded = expand_for_render(&doc);
        // Text should become CurvePath entities (glyph outlines)
        assert!(!expanded.is_empty());
        assert!(expanded
            .iter()
            .all(|e| matches!(&e.shape, Shape::CurvePath { .. })));
        // No raw Text in output
        assert!(!expanded
            .iter()
            .any(|e| matches!(&e.shape, Shape::Text { .. })));
    }

    #[test]
    fn expand_for_render_text_in_blocks() {
        let mut doc = Document::new();
        doc.define_block(BlockDef {
            name: "LABEL".into(),
            shapes: vec![(
                Shape::Text {
                    text: "A".into(),
                    position: Point::new(0.0, 0.0),
                    height: 5.0,
                    rotation: 0.0,
                },
                String::new(),
                Color::WHITE,
            )],
            insert_point: Point::ZERO,
            default_layer: "0".into(),
        });
        doc.place_block("LABEL", Point::new(0.0, 0.0), 0.0, None);
        let expanded = expand_for_render(&doc);
        // Block-internal Text should be expanded to CurvePaths (second pass)
        assert!(!expanded.is_empty());
        assert!(expanded
            .iter()
            .all(|e| matches!(&e.shape, Shape::CurvePath { .. })));
        assert!(!expanded
            .iter()
            .any(|e| matches!(&e.shape, Shape::Text { .. })));
    }
}

#[cfg(test)]
mod fill_tests {
    use super::*;
    use kurbo::Point;

    #[test]
    fn simple_rect_with_hole() {
        let outer = vec![
            Point::new(0.0, 0.0),
            Point::new(10.0, 0.0),
            Point::new(10.0, 10.0),
            Point::new(0.0, 10.0),
        ];
        let hole = vec![
            Point::new(3.0, 3.0),
            Point::new(7.0, 3.0),
            Point::new(7.0, 7.0),
            Point::new(3.0, 7.0),
        ];
        let (triangles, tri_indices) = triangulate_polygon(&outer, &[hole]);
        assert!(!tri_indices.is_empty(), "should produce triangles");
        let center = [5.0f32, 5.0f32];
        let mut inside_count = 0;
        for chunk in tri_indices.chunks(3) {
            let a = triangles[chunk[0] as usize];
            let b = triangles[chunk[1] as usize];
            let c = triangles[chunk[2] as usize];
            if point_in_triangle(center, a, b, c) {
                inside_count += 1;
            }
        }
        assert_eq!(
            inside_count, 0,
            "center (5,5) should be in the hole, not covered by any triangle"
        );

        let corner = [1.0f32, 1.0f32];
        let mut corner_inside = 0;
        for chunk in tri_indices.chunks(3) {
            let a = triangles[chunk[0] as usize];
            let b = triangles[chunk[1] as usize];
            let c = triangles[chunk[2] as usize];
            if point_in_triangle(corner, a, b, c) {
                corner_inside += 1;
            }
        }
        assert!(corner_inside > 0, "corner (1,1) should be covered by fill");
    }

    fn point_in_triangle(p: [f32; 2], a: [f32; 2], b: [f32; 2], c: [f32; 2]) -> bool {
        let d1 = sign(p, a, b);
        let d2 = sign(p, b, c);
        let d3 = sign(p, c, a);
        let has_neg = (d1 < 0.0) || (d2 < 0.0) || (d3 < 0.0);
        let has_pos = (d1 > 0.0) || (d2 > 0.0) || (d3 > 0.0);
        !(has_neg && has_pos)
    }

    fn sign(p1: [f32; 2], p2: [f32; 2], p3: [f32; 2]) -> f32 {
        (p1[0] - p3[0]) * (p2[1] - p3[1]) - (p2[0] - p3[0]) * (p1[1] - p3[1])
    }

    #[test]
    fn hatch_grid_pattern() {
        let boundary = vec![
            Point::new(0.0, 0.0),
            Point::new(2000.0, 0.0),
            Point::new(2000.0, 2000.0),
            Point::new(0.0, 2000.0),
        ];
        let pattern = acadrust::entities::HatchPattern {
            name: "GRID".to_string(),
            description: String::new(),
            lines: vec![
                acadrust::entities::HatchPatternLine {
                    angle: 0.0,
                    base_point: acadrust::types::Vector2::new(0.0, 0.0),
                    offset: acadrust::types::Vector2::new(0.0, 600.0),
                    dash_lengths: vec![],
                },
                acadrust::entities::HatchPatternLine {
                    angle: std::f64::consts::FRAC_PI_2,
                    base_point: acadrust::types::Vector2::new(0.0, 0.0),
                    offset: acadrust::types::Vector2::new(-600.0, 0.0),
                    dash_lengths: vec![],
                },
            ],
        };
        let shapes = generate_dwg_hatch_fill(&boundary, &pattern, 0.0, 1.0, false);

        let mut horiz = 0;
        let mut vert = 0;
        for s in &shapes {
            if let Shape::Line(l) = s {
                let dx = (l.p1.x - l.p0.x).abs();
                let dy = (l.p1.y - l.p0.y).abs();
                if dx > dy * 10.0 {
                    horiz += 1;
                }
                if dy > dx * 10.0 {
                    vert += 1;
                }
            }
        }
        assert!(horiz >= 2, "should have horizontal lines, got {horiz}");
        assert!(vert >= 2, "should have vertical lines, got {vert}");
    }

    #[test]
    fn hatch_dashed_pattern() {
        let boundary = vec![
            Point::new(0.0, 0.0),
            Point::new(3000.0, 0.0),
            Point::new(3000.0, 3000.0),
            Point::new(0.0, 3000.0),
        ];
        let pattern = acadrust::entities::HatchPattern {
            name: "HERRING".to_string(),
            description: String::new(),
            lines: vec![
                acadrust::entities::HatchPatternLine {
                    angle: 0.7854,
                    base_point: acadrust::types::Vector2::new(0.0, 0.0),
                    offset: acadrust::types::Vector2::new(0.0, 265.2),
                    dash_lengths: vec![937.5, -562.5],
                },
                acadrust::entities::HatchPatternLine {
                    angle: 2.3562,
                    base_point: acadrust::types::Vector2::new(0.0, 0.0),
                    offset: acadrust::types::Vector2::new(0.0, 265.2),
                    dash_lengths: vec![750.0, -562.5, 187.5, 0.0],
                },
            ],
        };
        let shapes = generate_dwg_hatch_fill(&boundary, &pattern, 0.0, 1.0, false);

        let mut short = 0;
        let mut long = 0;
        for s in &shapes {
            if let Shape::Line(l) = s {
                let len = ((l.p1.x - l.p0.x).powi(2) + (l.p1.y - l.p0.y).powi(2)).sqrt();
                if len < 1000.0 {
                    short += 1;
                } else {
                    long += 1;
                }
            }
        }
        assert!(short > long, "dashed pattern should have more short segments than long, got {short} short vs {long} long");
        assert!(
            shapes.len() > 20,
            "should have many segments, got {}",
            shapes.len()
        );
    }

    #[test]
    fn herringbone_far_from_origin() {
        let pattern = acadrust::entities::HatchPattern {
            name: "FP_7".to_string(),
            description: String::new(),
            lines: vec![
                acadrust::entities::HatchPatternLine {
                    angle: 0.7854,
                    base_point: acadrust::types::Vector2::new(0.0, 0.0),
                    offset: acadrust::types::Vector2::new(0.0, 265.2),
                    dash_lengths: vec![937.5, -562.5],
                },
                acadrust::entities::HatchPatternLine {
                    angle: 2.3562,
                    base_point: acadrust::types::Vector2::new(0.0, 0.0),
                    offset: acadrust::types::Vector2::new(0.0, 265.2),
                    dash_lengths: vec![750.0, -562.5, 187.5, 0.0],
                },
            ],
        };

        let near_boundary = vec![
            Point::new(0.0, 0.0),
            Point::new(3000.0, 0.0),
            Point::new(3000.0, 3000.0),
            Point::new(0.0, 3000.0),
        ];
        let near_shapes = generate_dwg_hatch_fill(&near_boundary, &pattern, 0.0, 1.0, false);
        let (near_45, near_135) = count_families(&near_shapes);

        let far_boundary = vec![
            Point::new(19000.0, 0.0),
            Point::new(22000.0, 0.0),
            Point::new(22000.0, 3000.0),
            Point::new(19000.0, 3000.0),
        ];
        let far_shapes = generate_dwg_hatch_fill(&far_boundary, &pattern, 0.0, 1.0, false);
        let (far_45, far_135) = count_families(&far_shapes);

        assert!(near_45 > 0, "near boundary should have 45deg lines");
        assert!(near_135 > 0, "near boundary should have 135deg lines");
        assert!(far_45 > 0, "far boundary should have 45deg lines, got 0");
        assert!(far_135 > 0, "far boundary should have 135deg lines, got 0");

        let ratio = far_45.min(far_135) as f64 / far_45.max(far_135) as f64;
        assert!(
            ratio > 0.3,
            "far families should be roughly balanced, ratio={ratio:.2}"
        );
    }

    fn count_families(shapes: &[Shape]) -> (usize, usize) {
        let mut n45 = 0;
        let mut n135 = 0;
        for s in shapes {
            if let Shape::Line(l) = s {
                let angle = (l.p1.y - l.p0.y).atan2(l.p1.x - l.p0.x);
                if angle.abs() < 1.2 {
                    n45 += 1;
                } else {
                    n135 += 1;
                }
            }
        }
        (n45, n135)
    }

    #[test]
    fn herringbone_135_isolated() {
        let boundary = vec![
            Point::new(0.0, 0.0),
            Point::new(3000.0, 0.0),
            Point::new(3000.0, 3000.0),
            Point::new(0.0, 3000.0),
        ];
        let pat = acadrust::entities::HatchPattern {
            name: "TEST".to_string(),
            description: String::new(),
            lines: vec![acadrust::entities::HatchPatternLine {
                angle: 2.3562,
                base_point: acadrust::types::Vector2::new(0.0, 0.0),
                offset: acadrust::types::Vector2::new(0.0, 265.2),
                dash_lengths: vec![],
            }],
        };
        let shapes = generate_dwg_hatch_fill(&boundary, &pat, 0.0, 1.0, false);
        assert!(
            shapes.len() > 0,
            "135deg family should produce lines, got 0"
        );
    }
}
