use cadview_core::load_dwg;
use std::env;

fn main() {
    let path = env::args().nth(1).expect("usage: load_dwg <file.dwg>");
    let doc = load_dwg(&path).expect("failed to load");

    println!("Layers: {}", doc.layers.len());
    println!("Entities: {}", doc.entities.len());

    if let Some((x0, y0, x1, y1)) = doc.bounds() {
        println!("Bounds: ({x0:.1}, {y0:.1}) - ({x1:.1}, {y1:.1})");
        println!("Size: {:.1} x {:.1}", x1 - x0, y1 - y0);
    }

    // Count by shape type
    let (mut lines, mut arcs, mut circles, mut polylines, mut blocks) = (0, 0, 0, 0, 0);
    for ent in &doc.entities {
        match &ent.shape {
            cadview_core::Shape::Line(_) => lines += 1,
            cadview_core::Shape::Arc { .. } => arcs += 1,
            cadview_core::Shape::Circle(_) => circles += 1,
            cadview_core::Shape::Polyline { .. } => polylines += 1,
            cadview_core::Shape::BlockInsert { .. } => blocks += 1,
            _ => {} // SolidFill, CurvePath, Ellipse, Spline, LwPolyline, Text, MText
        }
    }
    println!("Lines: {lines}, Arcs: {arcs}, Circles: {circles}, Polylines: {polylines}, Blocks: {blocks}");

    // Count by layer
    let mut layer_counts = std::collections::HashMap::new();
    for ent in &doc.entities {
        *layer_counts.entry(ent.layer.as_str()).or_insert(0u32) += 1;
    }
    let mut lc: Vec<_> = layer_counts.iter().collect();
    lc.sort_by(|a, b| b.1.cmp(a.1));
    println!("\nEntities per layer:");
    for (name, count) in lc {
        println!("  {name}: {count}");
    }
}
