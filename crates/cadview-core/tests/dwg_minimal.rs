///! Generate minimal DWGs for AutoCAD compatibility testing.
///! Run: cargo test -p cadview-core --test dwg_minimal -- --nocapture
use cadview_core::*;
use kurbo::Point;

#[test]
fn generate_minimal_dwgs() {
    let out = std::env::temp_dir().join("codecad-test-dwg");
    std::fs::create_dir_all(&out).expect("create output dir");

    // 1. Absolute minimum: one line
    {
        let mut doc = Document::new();
        doc.add_line(
            Point::new(0.0, 0.0),
            Point::new(1000.0, 500.0),
            "0",
            Color::WHITE,
        );
        let path = out.join("_test-1-line.dwg");
        save_dwg(&doc, path.to_str().unwrap()).expect("save failed");
        println!("Wrote: {}", path.display());
    }

    // 2. Multiple entity types + custom layer
    {
        let mut doc = Document::new();
        doc.ensure_layer("TEST");
        doc.add_line(
            Point::new(0.0, 0.0),
            Point::new(1000.0, 0.0),
            "TEST",
            Color::WHITE,
        );
        doc.add_circle(Point::new(500.0, 250.0), 100.0, "TEST", Color::WHITE);
        doc.add_arc(
            Point::new(200.0, 200.0),
            50.0,
            0.0,
            std::f64::consts::FRAC_PI_2,
            "TEST",
            Color::WHITE,
        );
        let path = out.join("_test-2-mixed.dwg");
        save_dwg(&doc, path.to_str().unwrap()).expect("save failed");
        println!("Wrote: {}", path.display());
    }

    // 3. Block insert (like our electrical symbols)
    {
        let mut doc = Document::new();
        doc.ensure_layer("E_TEST");
        // Define a simple block
        let block = BlockDef {
            name: "TEST_SYMBOL".into(),
            shapes: vec![
                (
                    Shape::Circle(kurbo::Circle::new(Point::new(0.0, 0.0), 35.0)),
                    "E_TEST".into(),
                    Color::WHITE,
                ),
                (
                    Shape::Line(kurbo::Line::new(
                        Point::new(-24.5, -24.5),
                        Point::new(24.5, 24.5),
                    )),
                    "E_TEST".into(),
                    Color::WHITE,
                ),
            ],
            insert_point: Point::new(0.0, 0.0),
            default_layer: "E_TEST".into(),
        };
        doc.blocks.insert("TEST_SYMBOL".into(), block);

        // Place the block
        doc.entities.push(DrawEntity {
            id: EntityId(1),
            layer: "E_TEST".into(),
            color: Color::WHITE,
            shape: Shape::BlockInsert {
                block_name: "TEST_SYMBOL".into(),
                position: Point::new(500.0, 250.0),
                rotation: 0.0,
                x_scale: 1.0,
                y_scale: 1.0,
            },
            dash: None,
        });
        doc.add_line(
            Point::new(0.0, 0.0),
            Point::new(1000.0, 0.0),
            "0",
            Color::WHITE,
        );

        let path = out.join("_test-3-block.dwg");
        save_dwg(&doc, path.to_str().unwrap()).expect("save failed");
        println!("Wrote: {}", path.display());
    }

    // 4. Load real DWG if CODECAD_TEST_DWG is set, re-export (pure acadrust roundtrip at AC1027)
    if let Ok(dwg_path) = std::env::var("CODECAD_TEST_DWG") {
        let orig = std::path::PathBuf::from(dwg_path);
        if orig.exists() {
            let mut reader = acadrust::DwgReader::from_file(&orig).expect("open failed");
            let cad = reader.read().expect("read failed");
            println!(
                "Original: version={:?}, entities={}",
                cad.version,
                cad.entity_count()
            );
            let path = out.join("_test-4-roundtrip.dwg");
            acadrust::DwgWriter::write_to_file(path.to_str().unwrap(), &cad).expect("write failed");
            println!("Wrote: {}", path.display());
        }
    }

    println!("\nDone. Test these files on viewer.autodesk.com in order 1-4.");
}
