/// Count ALL entity types in a DWG file (raw acadrust, before cadview filtering).
use std::collections::HashMap;
use std::env;

fn main() {
    let path = env::args().nth(1).expect("usage: entity_census <file.dwg>");
    let mut reader = acadrust::DwgReader::from_file(&path).expect("failed to open");
    let cad = reader.read().expect("failed to parse");

    let model_space_handle = cad
        .block_records
        .iter()
        .find(|br| br.name == "*Model_Space")
        .map(|br| br.handle);

    // Build block handle -> name map
    let mut handle_to_block: HashMap<u64, String> = HashMap::new();
    for br in cad.block_records.iter() {
        handle_to_block.insert(br.handle.value(), br.name.clone());
    }

    let mut total_counts: HashMap<String, usize> = HashMap::new();
    let mut model_counts: HashMap<String, usize> = HashMap::new();
    let mut block_counts: HashMap<String, usize> = HashMap::new();

    for ent in cad.entities() {
        let type_name = ent.as_entity().entity_type().to_string();
        let common = ent.common();
        let owner = common.owner_handle;

        *total_counts.entry(type_name.clone()).or_default() += 1;

        let in_model = model_space_handle.is_some_and(|ms| owner == ms);
        let in_named_block = handle_to_block
            .get(&owner.value())
            .is_some_and(|name| !name.starts_with('*'));

        if in_model {
            *model_counts.entry(type_name.clone()).or_default() += 1;
        }
        if in_named_block {
            *block_counts.entry(type_name).or_default() += 1;
        }
    }

    let total: usize = total_counts.values().sum();
    let model_total: usize = model_counts.values().sum();
    let block_total: usize = block_counts.values().sum();

    println!("=== ALL entities ({total}) ===");
    print_sorted(&total_counts, total);

    println!("\n=== Model Space entities ({model_total}) ===");
    print_sorted(&model_counts, model_total);

    println!("\n=== Named Block entities ({block_total}) ===");
    print_sorted(&block_counts, block_total);

    // What cadview currently handles
    let handled = [
        "LINE",
        "ARC",
        "CIRCLE",
        "INSERT",
        "LWPOLYLINE",
        "ELLIPSE",
        "SPLINE",
        "HATCH",
        "MTEXT",
        "TEXT",
        "DIMENSION_LINEAR",
    ];
    let handled_count: usize = handled.iter().filter_map(|t| model_counts.get(*t)).sum();
    let handled_block: usize = handled.iter().filter_map(|t| block_counts.get(*t)).sum();
    println!("\n=== Coverage ===");
    if model_total > 0 {
        println!(
            "Model space: {handled_count}/{model_total} = {:.1}%",
            handled_count as f64 / model_total as f64 * 100.0
        );
    }
    if block_total > 0 {
        println!(
            "Block defs:  {handled_block}/{block_total} = {:.1}%",
            handled_block as f64 / block_total as f64 * 100.0
        );
    }
}

fn print_sorted(counts: &HashMap<String, usize>, total: usize) {
    let mut sorted: Vec<_> = counts.iter().collect();
    sorted.sort_by(|a, b| b.1.cmp(a.1));
    for (name, count) in sorted {
        println!(
            "  {name:20} {count:6}  ({:.1}%)",
            *count as f64 / total as f64 * 100.0
        );
    }
}
