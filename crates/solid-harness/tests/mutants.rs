//! The harness tested by broken adapters (design section 7). Each mutant
//! wraps arris so that its results are subtly wrong while the harness
//! still builds its reference from the original recipe. Every mutant must
//! be flagged as wrong by at least one layer; the test prints which layers
//! caught it and on how many cases.
//!
//! `cargo test -p solid-harness --release --test mutants -- --nocapture`

use std::collections::BTreeMap;

use solid_harness::cases::catalogue;
use solid_harness::kernels::broken::{Broken, Mutation};
use solid_harness::kernels::ArrisKernel;
use solid_harness::layers::{enclosures, run_all};
use solid_harness::outcome::{Layer, Outcome};

fn mutants() -> Vec<Mutation> {
    vec![
        Mutation::ShiftTool(0.01),
        Mutation::HoleDiameter(0.02),
        Mutation::DropLastFeature,
        Mutation::SwapLastOp,
        Mutation::DropCavities,
        Mutation::SnapTools(0.01),
        Mutation::BboxIgnoresRotation,
        Mutation::MassScale(1.0 + 1e-6),
        Mutation::DropTriangle,
        Mutation::FlipTriangle,
        Mutation::MoveVertex(0.01),
    ]
}

#[test]
fn every_mutant_is_caught() {
    let cat = catalogue();
    let enc = enclosures(&cat);
    let base = run_all(&ArrisKernel, &cat, &enc);
    // (row, layer) pairs already wrong on the unbroken kernel do not count.
    let base_wrong: Vec<(String, Layer)> = base
        .rows
        .iter()
        .flat_map(|r| {
            Layer::ALL
                .iter()
                .zip(&r.verdicts)
                .filter(|(_, v)| v.outcome == Outcome::Wrong)
                .map(|(l, _)| (r.name.clone(), *l))
                .collect::<Vec<_>>()
        })
        .collect();
    let mut escaped = Vec::new();
    println!("\n{:<36} caught by (layer: wrong cases)", "mutant");
    for m in mutants() {
        let k = Broken {
            inner: ArrisKernel,
            mutation: m,
        };
        let rep = run_all(&k, &cat, &enc);
        let mut by_layer: BTreeMap<Layer, usize> = BTreeMap::new();
        let mut example: BTreeMap<Layer, String> = BTreeMap::new();
        for r in &rep.rows {
            for (l, v) in Layer::ALL.iter().zip(&r.verdicts) {
                if v.outcome == Outcome::Wrong && !base_wrong.contains(&(r.name.clone(), *l)) {
                    *by_layer.entry(*l).or_insert(0) += 1;
                    example
                        .entry(*l)
                        .or_insert_with(|| format!("{}: {}", r.name, v.detail));
                }
            }
        }
        let caught: Vec<String> = by_layer
            .iter()
            .map(|(l, n)| format!("{} {n}", l.name()))
            .collect();
        let only = if by_layer.len() == 1 {
            "  (only this layer)"
        } else {
            ""
        };
        println!(
            "{:<36} {}{only}",
            m.label(),
            if caught.is_empty() {
                "ESCAPED".to_string()
            } else {
                caught.join(", ")
            }
        );
        for (l, e) in &example {
            let e: String = e.chars().take(220).collect();
            println!("    {:<10} e.g. {e}", l.name());
        }
        if by_layer.is_empty() {
            escaped.push(m.label());
        }
    }
    assert!(escaped.is_empty(), "mutants not caught: {escaped:?}");
}
