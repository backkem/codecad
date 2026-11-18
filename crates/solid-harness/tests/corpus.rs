//! The 3D0a gate: the prismatic corpus on arris through the kernel trait.
//! Every part must be a valid single solid whose volume matches its closed
//! form to 1e-9 relative and its bounding box to 1e-9 mm.
//!
//! `cargo test -p solid-harness --test corpus -- --nocapture` prints the
//! per-part table with timings.

use std::time::Instant;

use cadview_core::solid::corpus::measure;
use cadview_core::solid::corpus::prismatic::{entries, BBOX_TOL, VOLUME_REL_TOL};
use cadview_core::solid::SolidKernel;
use solid_harness::kernels::ArrisKernel;

fn run_corpus<K: SolidKernel>(kernel: &K) {
    let profile = if cfg!(debug_assertions) {
        "debug"
    } else {
        "release"
    };
    println!(
        "\nprismatic corpus on {} ({profile} build)\n{:<26} {:<24} {:>18} {:>18} {:>9} {:>9} {:>5} {:>5} {:>10} {:>10}",
        kernel.name(),
        "entry",
        "part",
        "closed form",
        "kernel",
        "rel err",
        "bbox err",
        "valid",
        "faces",
        "eval ms",
        "total ms"
    );
    let mut failures = Vec::new();
    let mut total_eval = 0.0;
    let all = Instant::now();
    for entry in entries() {
        for part in &entry.parts {
            let t0 = Instant::now();
            let evaluated = kernel.evaluate(&part.def, &entry);
            let eval_ms = t0.elapsed().as_secs_f64() * 1e3;
            total_eval += eval_ms;
            let brep = match evaluated {
                Ok(e) => e.brep,
                Err(e) => {
                    failures.push(format!("{} / {}: {e}", entry.name, part.def.name));
                    continue;
                }
            };
            let r = match measure(kernel, part, &brep) {
                Ok(r) => r,
                Err(e) => {
                    failures.push(format!("{} / {}: {e}", entry.name, part.def.name));
                    continue;
                }
            };
            println!(
                "{:<26} {:<24} {:>18.6} {:>18.6} {:>9.1e} {:>9.1e} {:>5} {:>5} {:>10.2} {:>10.2}",
                entry.name,
                r.part,
                r.expected_volume,
                r.volume,
                r.volume_rel_err,
                r.bbox_err,
                if r.valid && r.solids == 1 {
                    "yes"
                } else {
                    "NO"
                },
                r.faces,
                eval_ms,
                t0.elapsed().as_secs_f64() * 1e3
            );
            if !r.passes(VOLUME_REL_TOL, BBOX_TOL) {
                failures.push(format!(
                    "{} / {}: valid={} solids={} rel_err={:e} bbox_err={:e} problems={:?}",
                    entry.name, r.part, r.valid, r.solids, r.volume_rel_err, r.bbox_err, r.problems
                ));
            }
        }
    }
    println!(
        "evaluation total {:.2} ms, corpus total {:.2} ms",
        total_eval,
        all.elapsed().as_secs_f64() * 1e3
    );
    assert!(
        failures.is_empty(),
        "corpus gate failed:\n{}",
        failures.join("\n")
    );
}

#[test]
fn arris_passes_the_prismatic_corpus() {
    run_corpus(&ArrisKernel);
}
