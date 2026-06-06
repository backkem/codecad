///! DWG layer roundtrip test
use cadview_core::*;
use kurbo::Point;

#[test]
fn roundtrip_layers() {
    let mut doc = Document::new();

    // Add layers
    doc.ensure_layer("TEST_RED");
    doc.ensure_layer("TEST_BLUE");

    // Add entities on specific layers
    doc.add_line(
        Point::new(0.0, 0.0),
        Point::new(100.0, 0.0),
        "TEST_RED",
        None,
    );
    doc.add_line(
        Point::new(0.0, 50.0),
        Point::new(100.0, 50.0),
        "TEST_BLUE",
        None,
    );
    doc.add_circle(Point::new(50.0, 25.0), 10.0, "0", None);

    println!(
        "Before save: {} layers, {} entities",
        doc.layers.len(),
        doc.entities.len()
    );
    for l in &doc.layers {
        println!(
            "  layer: {} color=({},{},{})",
            l.name, l.color.r, l.color.g, l.color.b
        );
    }
    for e in &doc.entities {
        println!("  entity: layer={}", e.layer);
    }

    let tmp = std::env::temp_dir().join("cadview_layer_test.dwg");
    save_dwg(&doc, tmp.to_str().unwrap()).expect("save failed");

    let doc2 = load_dwg(tmp.to_str().unwrap()).expect("reload failed");
    std::fs::remove_file(&tmp).ok();

    println!(
        "\nAfter reload: {} layers, {} entities",
        doc2.layers.len(),
        doc2.entities.len()
    );
    for l in &doc2.layers {
        println!(
            "  layer: {} color=({},{},{})",
            l.name, l.color.r, l.color.g, l.color.b
        );
    }
    for e in &doc2.entities {
        println!("  entity: layer={}", e.layer);
    }

    // Verify layers survived
    assert!(
        doc2.layers.iter().any(|l| l.name == "TEST_RED"),
        "TEST_RED layer missing"
    );
    assert!(
        doc2.layers.iter().any(|l| l.name == "TEST_BLUE"),
        "TEST_BLUE layer missing"
    );

    // Verify entities kept their layer assignments
    let red_ents: Vec<_> = doc2
        .entities
        .iter()
        .filter(|e| e.layer == "TEST_RED")
        .collect();
    let blue_ents: Vec<_> = doc2
        .entities
        .iter()
        .filter(|e| e.layer == "TEST_BLUE")
        .collect();
    assert_eq!(
        red_ents.len(),
        1,
        "expected 1 entity on TEST_RED, got {}",
        red_ents.len()
    );
    assert_eq!(
        blue_ents.len(),
        1,
        "expected 1 entity on TEST_BLUE, got {}",
        blue_ents.len()
    );

    println!("\nLayer roundtrip PASSED");
}
