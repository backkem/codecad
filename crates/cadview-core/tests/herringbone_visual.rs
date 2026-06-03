///! Visual test: call the ACTUAL generate_dwg_hatch_fill and dump as SVG.
///! Run: cargo test -p cadview-core --test herringbone_visual -- --nocapture
///! Then open tests/herringbone_output.svg in a browser.

use cadview_core::*;
use kurbo::Point;

#[test]
fn herringbone_svg() {
    let boundary = vec![
        Point::new(0.0, 0.0),
        Point::new(3000.0, 0.0),
        Point::new(3000.0, 3000.0),
        Point::new(0.0, 3000.0),
    ];

    // FP_7 pattern definition (exact values from the DWG)
    let pattern = acadrust::entities::HatchPattern {
        name: "FP_7".to_string(),
        description: String::new(),
        lines: vec![
            acadrust::entities::HatchPatternLine {
                angle: 0.7854,  // 45°
                base_point: acadrust::types::Vector2::new(0.0, 0.0),
                offset: acadrust::types::Vector2::new(0.0, 265.2),
                dash_lengths: vec![937.5, -562.5],
            },
            acadrust::entities::HatchPatternLine {
                angle: 2.3562,  // 135°
                base_point: acadrust::types::Vector2::new(0.0, 0.0),
                offset: acadrust::types::Vector2::new(0.0, 265.2),
                dash_lengths: vec![750.0, -562.5, 187.5, 0.0],
            },
        ],
    };

    // Call the EXACT same function the renderer uses
    let shapes = generate_dwg_hatch_fill(&boundary, &pattern, 0.0, 1.0, false);

    // Dump as SVG
    let mut svg = String::new();
    svg.push_str(r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="-100 -100 3200 3200">"#);
    svg.push_str(r#"<rect x="0" y="0" width="3000" height="3000" fill="none" stroke="gray" stroke-width="3"/>"#);

    let mut count_45 = 0;
    let mut count_135 = 0;
    let mut oob = 0;

    for shape in &shapes {
        if let Shape::Line(l) = shape {
            // Determine which family based on line angle
            let dx = l.p1.x - l.p0.x;
            let dy = l.p1.y - l.p0.y;
            let angle = dy.atan2(dx);
            let color = if angle.abs() < 1.2 {
                count_45 += 1;
                "red"
            } else {
                count_135 += 1;
                "blue"
            };

            // Check out of bounds
            if l.p0.x < -1.0 || l.p0.x > 3001.0 || l.p0.y < -1.0 || l.p0.y > 3001.0 ||
               l.p1.x < -1.0 || l.p1.x > 3001.0 || l.p1.y < -1.0 || l.p1.y > 3001.0 {
                oob += 1;
            }

            svg.push_str(&format!(
                r#"<line x1="{:.1}" y1="{:.1}" x2="{:.1}" y2="{:.1}" stroke="{}" stroke-width="2"/>"#,
                l.p0.x, 3000.0 - l.p0.y, l.p1.x, 3000.0 - l.p1.y, color
            ));
        }
    }

    svg.push_str("</svg>");
    std::fs::write("tests/herringbone_output.svg", &svg).unwrap();

    println!("Generated {} shapes ({} red/45°, {} blue/135°)", shapes.len(), count_45, count_135);
    println!("Out of bounds: {oob}");
    println!("Wrote tests/herringbone_output.svg");

    assert_eq!(oob, 0, "{oob} lines extend outside boundary");
}
