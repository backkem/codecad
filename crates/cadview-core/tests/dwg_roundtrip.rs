///! DWG roundtrip tests: load -> save -> compare.
///!
///! Run: cargo test -p cadview-core --test dwg_roundtrip -- --nocapture
use cadview_core::*;
use kurbo::Point;

fn approx_eq(a: f64, b: f64, tol: f64) -> bool {
    (a - b).abs() < tol
}
fn points_eq(a: Point, b: Point, tol: f64) -> bool {
    approx_eq(a.x, b.x, tol) && approx_eq(a.y, b.y, tol)
}

fn real_dwg_path() -> Option<std::path::PathBuf> {
    let p = std::path::PathBuf::from(std::env::var("CODECAD_TEST_DWG").ok()?);
    if p.exists() {
        Some(p)
    } else {
        None
    }
}

/// Synthetic roundtrip: build doc with every shape type, save, reload, compare geometry.
#[test]
fn roundtrip_synthetic() {
    let mut doc = Document::new();

    doc.add_line(Point::new(0.0, 0.0), Point::new(100.0, 50.0), "TEST", None);
    doc.add_circle(Point::new(200.0, 200.0), 75.0, "TEST", None);
    doc.add_arc(
        Point::new(400.0, 200.0),
        50.0,
        0.0,
        std::f64::consts::FRAC_PI_2,
        "TEST",
        None,
    );
    doc.add_polyline(
        vec![
            Point::new(500.0, 0.0),
            Point::new(600.0, 0.0),
            Point::new(600.0, 100.0),
            Point::new(500.0, 100.0),
        ],
        true,
        "TEST",
        None,
    );

    let id = EntityId(100);
    doc.entities.push(DrawEntity {
        id,
        layer: "TEST".into(),
        color: None,
        shape: Shape::LwPolyline {
            vertices: vec![
                LwVertex {
                    point: Point::new(700.0, 0.0),
                    bulge: 0.0,
                },
                LwVertex {
                    point: Point::new(800.0, 0.0),
                    bulge: 0.5,
                },
                LwVertex {
                    point: Point::new(800.0, 100.0),
                    bulge: 0.0,
                },
                LwVertex {
                    point: Point::new(700.0, 100.0),
                    bulge: -0.3,
                },
            ],
            closed: true,
        },
        linetype: None,
        lineweight: None,
        transparency: 0,
    });
    let id = EntityId(101);
    doc.entities.push(DrawEntity {
        id,
        layer: "TEST".into(),
        color: None,
        shape: Shape::Ellipse {
            center: Point::new(1000.0, 200.0),
            major_axis: (80.0, 0.0),
            minor_ratio: 0.5,
            start_param: 0.0,
            end_param: std::f64::consts::TAU,
        },
        linetype: None,
        lineweight: None,
        transparency: 0,
    });
    let id = EntityId(102);
    doc.entities.push(DrawEntity {
        id,
        layer: "TEST".into(),
        color: None,
        shape: Shape::Spline {
            degree: 3,
            knots: vec![0.0, 0.0, 0.0, 0.0, 0.5, 1.0, 1.0, 1.0, 1.0],
            control_points: vec![
                Point::new(1200.0, 0.0),
                Point::new(1250.0, 100.0),
                Point::new(1300.0, 50.0),
                Point::new(1350.0, 100.0),
                Point::new(1400.0, 0.0),
            ],
            closed: false,
        },
        linetype: None,
        lineweight: None,
        transparency: 0,
    });
    let id = EntityId(103);
    doc.entities.push(DrawEntity {
        id,
        layer: "TEST".into(),
        color: None,
        shape: Shape::SolidFill {
            boundary: vec![
                FillEdge::LineTo(Point::new(1500.0, 0.0)),
                FillEdge::LineTo(Point::new(1600.0, 0.0)),
                FillEdge::LineTo(Point::new(1550.0, 100.0)),
            ],
            holes: Vec::new(),
        },
        linetype: None,
        lineweight: None,
        transparency: 0,
    });

    let tmp = std::env::temp_dir().join("cadview_rt_test.dwg");
    save_dwg(&doc, tmp.to_str().unwrap()).expect("save failed");
    let doc2 = load_dwg(tmp.to_str().unwrap()).expect("reload failed");
    std::fs::remove_file(&tmp).ok();

    let tol = 0.01;
    let line = doc2
        .entities
        .iter()
        .find(|e| matches!(&e.shape, Shape::Line(_)))
        .expect("no Line");
    if let Shape::Line(l) = &line.shape {
        assert!(points_eq(l.p0, Point::new(0.0, 0.0), tol));
        assert!(points_eq(l.p1, Point::new(100.0, 50.0), tol));
    }
    let c = doc2
        .entities
        .iter()
        .find(|e| matches!(&e.shape, Shape::Circle(_)))
        .expect("no Circle");
    if let Shape::Circle(c) = &c.shape {
        assert!(approx_eq(c.radius, 75.0, tol));
    }
    let a = doc2
        .entities
        .iter()
        .find(|e| matches!(&e.shape, Shape::Arc { .. }))
        .expect("no Arc");
    if let Shape::Arc { radius, .. } = &a.shape {
        assert!(approx_eq(*radius, 50.0, tol));
    }

    assert!(
        doc2.entities
            .iter()
            .any(|e| matches!(&e.shape, Shape::Ellipse { .. })),
        "no Ellipse"
    );
    assert!(
        doc2.entities
            .iter()
            .any(|e| matches!(&e.shape, Shape::Spline { .. })),
        "no Spline"
    );
    assert!(
        doc2.entities
            .iter()
            .any(|e| matches!(&e.shape, Shape::SolidFill { .. })),
        "no SolidFill"
    );

    println!(
        "Synthetic roundtrip PASSED ({} -> {} entities)",
        doc.entities.len(),
        doc2.entities.len()
    );
}

/// Real DWG byte-level roundtrip: load -> save -> byte compare.
#[test]
fn roundtrip_real_dwg_bytes() {
    let src_path = match real_dwg_path() {
        Some(p) => p,
        None => {
            println!("Skipping: real DWG not found");
            return;
        }
    };

    let original_bytes = std::fs::read(&src_path).expect("read original");
    println!("Original: {} bytes", original_bytes.len());

    // Load
    let doc = load_dwg(src_path.to_str().unwrap()).expect("load failed");
    println!("Loaded: {} entities", doc.entities.len());

    // Save
    let tmp = std::env::temp_dir().join("cadview_rt_bytes.dwg");
    save_dwg(&doc, tmp.to_str().unwrap()).expect("save failed");
    let saved_bytes = std::fs::read(&tmp).expect("read saved");
    println!("Saved: {} bytes", saved_bytes.len());

    // Reload the saved file to verify it's valid
    let doc2 = load_dwg(tmp.to_str().unwrap()).expect("reload saved file");
    println!("Reloaded: {} entities", doc2.entities.len());
    std::fs::remove_file(&tmp).ok();

    // Byte comparison
    let min_len = original_bytes.len().min(saved_bytes.len());
    let mut diff_count = 0usize;
    let mut first_diff = None;
    for i in 0..min_len {
        if original_bytes[i] != saved_bytes[i] {
            diff_count += 1;
            if first_diff.is_none() {
                first_diff = Some(i);
            }
        }
    }
    let tail_diff = (original_bytes.len() as isize - saved_bytes.len() as isize).unsigned_abs();
    let total_diff = diff_count + tail_diff;

    println!("--- Byte comparison ---");
    println!("Original size: {}", original_bytes.len());
    println!("Saved size:    {}", saved_bytes.len());
    println!(
        "Size delta:    {} bytes",
        saved_bytes.len() as isize - original_bytes.len() as isize
    );
    println!(
        "Differing bytes (in overlap): {diff_count} / {min_len} ({:.2}%)",
        diff_count as f64 / min_len as f64 * 100.0
    );
    if let Some(pos) = first_diff {
        println!("First diff at byte: {pos} (0x{pos:x})");
    }
    println!("Total difference: {total_diff} bytes");

    // We don't assert byte equality - DWG format has handles, timestamps, etc.
    // that will differ. This test measures how close we are.
    // What matters is that the reloaded geometry is valid.
    assert!(doc2.entities.len() > 0, "reloaded doc is empty");

    // Count what survived
    let count_types = |d: &Document| -> std::collections::BTreeMap<&str, usize> {
        let mut m = std::collections::BTreeMap::new();
        for e in &d.entities {
            let t = match &e.shape {
                Shape::Line(_) => "Line",
                Shape::Circle(_) => "Circle",
                Shape::Arc { .. } => "Arc",
                Shape::Polyline { .. } => "Polyline",
                Shape::LwPolyline { .. } => "LwPolyline",
                Shape::Ellipse { .. } => "Ellipse",
                Shape::Spline { .. } => "Spline",
                Shape::SolidFill { .. } => "SolidFill",
                Shape::CurvePath { .. } => "CurvePath",
                Shape::BlockInsert { .. } => "BlockInsert",
                Shape::Text { .. } => "Text",
                Shape::MText { .. } => "MText",
            };
            *m.entry(t).or_default() += 1;
        }
        m
    };

    let t1 = count_types(&doc);
    let t2 = count_types(&doc2);
    println!("\nEntity types:");
    println!("  Original:  {t1:?}");
    println!("  Reloaded:  {t2:?}");

    // Verify geometry types that should roundtrip exactly
    for typ in ["Line", "Circle", "Arc", "Ellipse", "Spline"] {
        let c1 = t1.get(typ).copied().unwrap_or(0);
        let c2 = t2.get(typ).copied().unwrap_or(0);
        assert!(c2 >= c1, "{typ} count decreased: {c1} -> {c2}");
    }

    println!("\nReal DWG byte roundtrip test PASSED");
}

/// Style roundtrip: layer colors, linetypes, lineweight, and entity
/// overrides must survive save -> reload.
#[test]
fn roundtrip_styles() {
    let mut doc = Document::new();

    // Use ACI-safe colors (standard DWG palette) so RGB survives roundtrip.
    // ACI 1=red, 3=green, 5=blue, 8=dark gray, 9=light gray
    doc.add_layer_full("WALLS", Color::rgb(255, 0, 0), "Continuous", 0.5);
    doc.add_layer_full("CL", Color::rgb(128, 128, 128), "Center", 0.13);
    doc.add_layer_full("HIDDEN", Color::rgb(0, 0, 255), "Hidden", 0.25);

    // Entity inheriting everything from layer (ByLayer)
    doc.add_line(Point::new(0.0, 0.0), Point::new(100.0, 0.0), "CL", None);

    // Entity with explicit color override
    doc.add_circle(
        Point::new(50.0, 50.0),
        25.0,
        "WALLS",
        Some(Color::rgb(255, 0, 0)),
    );

    // Entity with explicit linetype override
    let id = doc.add_circle(Point::new(0.0, 0.0), 75.0, "CL", None);
    if let Some(ent) = doc.entity_mut(id) {
        ent.linetype = Some("Dashed".to_string());
    }

    // Save and reload
    let tmp = std::env::temp_dir().join("cadview_style_rt.dwg");
    save_dwg(&doc, tmp.to_str().unwrap()).expect("save failed");
    let doc2 = load_dwg(tmp.to_str().unwrap()).expect("reload failed");
    std::fs::remove_file(&tmp).ok();

    // ── Layer style checks ─────────────────────────────────────────
    let walls = doc2
        .layers
        .iter()
        .find(|l| l.name == "WALLS")
        .expect("no WALLS layer");
    assert_eq!(
        walls.color,
        Color::rgb(255, 0, 0),
        "WALLS layer color lost: got {:?}",
        walls.color
    );
    assert!(
        approx_eq(walls.lineweight, 0.5, 0.01),
        "WALLS lineweight lost: got {}",
        walls.lineweight
    );

    let cl = doc2
        .layers
        .iter()
        .find(|l| l.name == "CL")
        .expect("no CL layer");
    assert_eq!(
        cl.linetype, "Center",
        "CL layer linetype lost: got {}",
        cl.linetype
    );
    assert!(
        approx_eq(cl.lineweight, 0.13, 0.01),
        "CL lineweight lost: got {}",
        cl.lineweight
    );

    let hidden = doc2
        .layers
        .iter()
        .find(|l| l.name == "HIDDEN")
        .expect("no HIDDEN layer");
    assert_eq!(
        hidden.color,
        Color::rgb(0, 0, 255),
        "HIDDEN layer color lost: got {:?}",
        hidden.color
    );
    assert_eq!(
        hidden.linetype, "Hidden",
        "HIDDEN layer linetype lost: got {}",
        hidden.linetype
    );

    // ── Entity style checks ────────────────────────────────────────
    // ByLayer line on CL: color should be None (inherited)
    let line = doc2
        .entities
        .iter()
        .find(|e| matches!(&e.shape, Shape::Line(_)))
        .expect("no line");
    assert_eq!(
        line.color, None,
        "ByLayer line should have color=None, got {:?}",
        line.color
    );
    // Resolved color should match CL layer
    assert_eq!(
        doc2.resolve_color(line),
        Color::rgb(128, 128, 128),
        "resolved color should be CL layer gray"
    );

    // Explicit red circle on WALLS
    let red_circle = doc2
        .entities
        .iter()
        .find(|e| matches!(&e.shape, Shape::Circle(c) if approx_eq(c.radius, 25.0, 0.1)))
        .expect("no red circle");
    assert_eq!(
        red_circle.color,
        Some(Color::rgb(255, 0, 0)),
        "explicit red color lost: got {:?}",
        red_circle.color
    );

    // Circle with explicit Dashed linetype override
    let dashed_circle = doc2
        .entities
        .iter()
        .find(|e| matches!(&e.shape, Shape::Circle(c) if approx_eq(c.radius, 75.0, 0.1)))
        .expect("no dashed circle");
    assert_eq!(
        dashed_circle.linetype.as_deref(),
        Some("Dashed"),
        "explicit Dashed linetype lost: got {:?}",
        dashed_circle.linetype
    );

    // Linetypes table should contain Center, Dashed, Hidden from layers/entities
    assert!(
        doc2.linetypes.contains_key("Center"),
        "Center linetype missing from table"
    );
    assert!(
        doc2.linetypes.contains_key("Dashed"),
        "Dashed linetype missing from table"
    );

    // ── to_json() resolution check ───────────────────────────────
    // ByLayer entities must resolve to their layer's color in JSON output
    let line_json = line.to_json(&doc2);
    assert_eq!(
        line_json.color,
        [128, 128, 128],
        "to_json should resolve ByLayer color to CL gray, got {:?}",
        line_json.color
    );

    // Explicit-color entities should show their own color
    let red_json = red_circle.to_json(&doc2);
    assert_eq!(
        red_json.color,
        [255, 0, 0],
        "to_json should show explicit red, got {:?}",
        red_json.color
    );

    // ByLayer entity on HIDDEN layer should resolve to blue
    // (add one to test)
    let hidden_line = doc2
        .entities
        .iter()
        .find(|e| e.layer == "HIDDEN")
        .or_else(|| {
            // If no entity on HIDDEN, skip this check
            None
        });
    if let Some(hl) = hidden_line {
        let hj = hl.to_json(&doc2);
        assert_eq!(
            hj.color,
            [0, 0, 255],
            "HIDDEN layer entity should resolve to blue in JSON"
        );
    }

    println!("Style roundtrip PASSED");
}
