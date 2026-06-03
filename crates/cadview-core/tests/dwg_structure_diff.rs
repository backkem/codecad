///! Compare structure of a working DWG (acadrust roundtrip) vs our from-scratch DWG.
///! Run: cargo test -p cadview-core --test dwg_structure_diff -- --nocapture

use acadrust::TableEntry;
use acadrust::entities::Entity;

#[test]
fn compare_structures() {
    let electrical_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..").join("..").join("..").join("..")
        .join("design").join("electrical");

    let roundtrip_path = electrical_dir.join("_test-4-roundtrip.dwg");
    let scratch_path = electrical_dir.join("_first-floor-electrical.dwg");

    if !roundtrip_path.exists() || !scratch_path.exists() {
        println!("SKIP: need both _test-4-roundtrip.dwg and _first-floor-electrical.dwg");
        return;
    }

    let mut r1 = acadrust::DwgReader::from_file(&roundtrip_path).expect("open roundtrip");
    let good = r1.read().expect("read roundtrip");

    let mut r2 = acadrust::DwgReader::from_file(&scratch_path).expect("open scratch");
    let bad = r2.read().expect("read scratch");

    println!("=== HEADER DIFFS ===");
    macro_rules! cmp_hdr {
        ($field:ident) => {
            let g = format!("{:?}", good.header.$field);
            let b = format!("{:?}", bad.header.$field);
            if g != b {
                println!("  {:<40} GOOD={}", stringify!($field), &g[..g.len().min(60)]);
                println!("  {:<40} BAD ={}", "", &b[..b.len().min(60)]);
            }
        };
    }

    cmp_hdr!(handle_seed);
    cmp_hdr!(model_space_extents_min);
    cmp_hdr!(model_space_extents_max);
    cmp_hdr!(model_space_limits_min);
    cmp_hdr!(model_space_limits_max);
    cmp_hdr!(model_space_insertion_base);
    cmp_hdr!(current_layer_handle);
    cmp_hdr!(block_control_handle);
    cmp_hdr!(layer_control_handle);
    cmp_hdr!(style_control_handle);
    cmp_hdr!(linetype_control_handle);
    cmp_hdr!(dimstyle_control_handle);
    cmp_hdr!(named_objects_dict_handle);
    cmp_hdr!(measurement);
    cmp_hdr!(insertion_units);
    cmp_hdr!(linear_unit_format);
    cmp_hdr!(linear_unit_precision);
    cmp_hdr!(angular_unit_format);
    cmp_hdr!(angular_unit_precision);
    cmp_hdr!(continuous_linetype_handle);
    cmp_hdr!(bylayer_linetype_handle);
    cmp_hdr!(byblock_linetype_handle);
    cmp_hdr!(current_linetype_handle);

    println!("\n=== TABLES ===");
    println!("{:<30} {:>10} {:>10}", "Table", "GOOD", "BAD");
    println!("{}", "-".repeat(52));
    println!("{:<30} {:>10} {:>10}", "Layers", good.layers.len(), bad.layers.len());
    println!("{:<30} {:>10} {:>10}", "LineTypes", good.line_types.len(), bad.line_types.len());
    println!("{:<30} {:>10} {:>10}", "TextStyles", good.text_styles.len(), bad.text_styles.len());
    println!("{:<30} {:>10} {:>10}", "DimStyles", good.dim_styles.len(), bad.dim_styles.len());
    println!("{:<30} {:>10} {:>10}", "BlockRecords", good.block_records.len(), bad.block_records.len());
    println!("{:<30} {:>10} {:>10}", "AppIds", good.app_ids.len(), bad.app_ids.len());
    println!("{:<30} {:>10} {:>10}", "Views", good.views.len(), bad.views.len());
    println!("{:<30} {:>10} {:>10}", "VPorts", good.vports.len(), bad.vports.len());
    println!("{:<30} {:>10} {:>10}", "UCSs", good.ucss.len(), bad.ucss.len());

    println!("\n=== LAYERS ===");
    for gl in good.layers.iter() {
        if bad.layers.get(&gl.name).is_none() {
            println!("  GOOD only: {}", gl.name);
        }
    }
    for bl in bad.layers.iter() {
        if good.layers.get(&bl.name).is_none() {
            println!("  BAD only:  {}", bl.name);
        }
    }

    println!("\n=== BLOCK RECORDS ===");
    for gb in good.block_records.iter() {
        if bad.block_records.get(&gb.name).is_none() {
            println!("  GOOD only: {} ({} ent)", gb.name, gb.entity_handles.len());
        }
    }
    for bb in bad.block_records.iter() {
        if good.block_records.get(&bb.name).is_none() {
            println!("  BAD only:  {} ({} ent)", bb.name, bb.entity_handles.len());
        }
    }
    for gb in good.block_records.iter() {
        if let Some(bb) = bad.block_records.get(&gb.name) {
            let ge = gb.entity_handles.len();
            let be = bb.entity_handles.len();
            if ge != be {
                println!("  DIFF:      {} entities: {} vs {}", gb.name, ge, be);
            }
        }
    }

    println!("\n=== ENTITY TYPES ===");
    use std::collections::BTreeMap;
    let count_entities = |doc: &acadrust::CadDocument| -> BTreeMap<String, usize> {
        let mut m = BTreeMap::new();
        for e in doc.entities() {
            let name = entity_variant(e);
            *m.entry(name).or_default() += 1;
        }
        m
    };
    let gc = count_entities(&good);
    let bc = count_entities(&bad);
    let all_types: std::collections::BTreeSet<_> = gc.keys().chain(bc.keys()).collect();
    println!("{:<25} {:>10} {:>10} {:>10}", "Type", "GOOD", "BAD", "DIFF");
    println!("{}", "-".repeat(57));
    for t in &all_types {
        let g = gc.get(*t).copied().unwrap_or(0);
        let b = bc.get(*t).copied().unwrap_or(0);
        let diff = b as i64 - g as i64;
        if diff != 0 {
            println!("{:<25} {:>10} {:>10} {:>+10}", t, g, b, diff);
        } else {
            println!("{:<25} {:>10} {:>10}", t, g, b);
        }
    }

    println!("\n=== OBJECTS ===");
    println!("  GOOD: {} objects", good.objects.len());
    println!("  BAD:  {} objects", bad.objects.len());

    println!("\n=== CLASSES ===");
    println!("  GOOD: {} classes", good.classes.len());
    println!("  BAD:  {} classes", bad.classes.len());

    println!("\n=== FILE SIZE ===");
    println!("  GOOD: {} bytes", std::fs::metadata(&roundtrip_path).unwrap().len());
    println!("  BAD:  {} bytes", std::fs::metadata(&scratch_path).unwrap().len());
}

fn entity_variant(e: &acadrust::entities::EntityType) -> String {
    use acadrust::entities::EntityType::*;
    match e {
        Line(_) => "Line", Circle(_) => "Circle", Arc(_) => "Arc",
        Ellipse(_) => "Ellipse", Spline(_) => "Spline", Point(_) => "Point",
        LwPolyline(_) => "LwPolyline", Polyline2D(_) => "Polyline2D",
        Polyline3D(_) => "Polyline3D", Text(_) => "Text", MText(_) => "MText",
        Insert(_) => "Insert", Hatch(_) => "Hatch", Dimension(_) => "Dimension",
        Leader(_) => "Leader", Block(_) => "Block", BlockEnd(_) => "BlockEnd",
        Solid(_) => "Solid", Face3D(_) => "Face3D",
        _ => "Other",
    }.to_string()
}
