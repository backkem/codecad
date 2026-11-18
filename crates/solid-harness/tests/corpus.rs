//! The corpus gates on arris through the kernel trait.
//!
//! - 3D0a, prismatic: every part a valid single solid, volume within 1e-9
//!   relative of its closed form, bounding box within 1e-9 mm.
//! - 3D0b, curved: every part a valid single solid with a closed mesh,
//!   volume within 1e-6 relative of its closed form, bounding box within
//!   1e-6 mm.
//!
//! `cargo test -p solid-harness --test corpus -- --nocapture` prints the
//! per-part tables with timings. cad-kernel runs the same corpus for the
//! record (printed, not gated): its gate is its scoreboard.

use std::time::Instant;

use cadview_core::solid::corpus::{curved, measure, prismatic, CorpusEntry};
use cadview_core::solid::SolidKernel;
use solid_harness::kernels::{ArrisKernel, CadKernel};
use solid_harness::mesh::topology;

/// The chord of the closed-mesh check (mm).
const MESH_CHORD: f64 = 1e-2;

struct Gate {
    volume_rel_tol: f64,
    bbox_tol: f64,
    closed_mesh: bool,
}

/// Runs `entries` on `kernel`, prints a table and returns the failures.
fn run_corpus<K: SolidKernel>(
    kernel: &K,
    title: &str,
    entries: Vec<CorpusEntry>,
    gate: &Gate,
) -> Vec<String> {
    let profile = if cfg!(debug_assertions) {
        "debug"
    } else {
        "release"
    };
    println!(
        "\n{title} corpus on {} ({profile} build)\n{:<44} {:>18} {:>18} {:>9} {:>9} {:>5} {:>5} {:>6} {:>10} {:>10}",
        kernel.name(),
        "part",
        "closed form",
        "kernel",
        "rel err",
        "bbox err",
        "valid",
        "faces",
        "mesh",
        "eval ms",
        "total ms"
    );
    let mut failures = Vec::new();
    let mut total_eval = 0.0;
    let all = Instant::now();
    for entry in entries {
        for part in &entry.parts {
            let t0 = Instant::now();
            let evaluated = kernel.evaluate(&part.def, &entry);
            let eval_ms = t0.elapsed().as_secs_f64() * 1e3;
            total_eval += eval_ms;
            let brep = match evaluated {
                Ok(e) => e.brep,
                Err(e) => {
                    println!("{:<44} refused: {e}", part.def.name);
                    failures.push(format!("{} / {}: {e}", entry.name, part.def.name));
                    continue;
                }
            };
            let r = match measure(kernel, part, &brep) {
                Ok(r) => r,
                Err(e) => {
                    println!("{:<44} not measured: {e}", part.def.name);
                    failures.push(format!("{} / {}: {e}", entry.name, part.def.name));
                    continue;
                }
            };
            let mesh = if gate.closed_mesh {
                match kernel.tessellate(&brep, MESH_CHORD) {
                    Ok(m) if !m.triangles.is_empty() && topology(&m).closed => "closed".to_string(),
                    Ok(_) => "OPEN".to_string(),
                    Err(e) => format!("none: {e}"),
                }
            } else {
                "-".to_string()
            };
            println!(
                "{:<44} {:>18.6} {:>18.6} {:>9.1e} {:>9.1e} {:>5} {:>5} {:>6} {:>10.2} {:>10.2}",
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
                mesh,
                eval_ms,
                t0.elapsed().as_secs_f64() * 1e3
            );
            if !r.passes(gate.volume_rel_tol, gate.bbox_tol) {
                failures.push(format!(
                    "{} / {}: valid={} solids={} rel_err={:e} bbox_err={:e} problems={:?}",
                    entry.name, r.part, r.valid, r.solids, r.volume_rel_err, r.bbox_err, r.problems
                ));
            }
            if gate.closed_mesh && mesh != "closed" {
                failures.push(format!(
                    "{} / {}: mesh at chord {MESH_CHORD}: {mesh}",
                    entry.name, r.part
                ));
            }
        }
    }
    println!(
        "evaluation total {:.2} ms, corpus total {:.2} ms",
        total_eval,
        all.elapsed().as_secs_f64() * 1e3
    );
    failures
}

fn prismatic_gate() -> Gate {
    Gate {
        volume_rel_tol: prismatic::VOLUME_REL_TOL,
        bbox_tol: prismatic::BBOX_TOL,
        closed_mesh: false,
    }
}

fn curved_gate() -> Gate {
    Gate {
        volume_rel_tol: curved::VOLUME_REL_TOL,
        bbox_tol: curved::BBOX_TOL,
        closed_mesh: true,
    }
}

#[test]
fn arris_passes_the_prismatic_corpus() {
    let f = run_corpus(
        &ArrisKernel,
        "prismatic",
        prismatic::entries(),
        &prismatic_gate(),
    );
    assert!(f.is_empty(), "corpus gate failed:\n{}", f.join("\n"));
}

#[test]
fn arris_passes_the_curved_corpus() {
    let f = run_corpus(&ArrisKernel, "curved", curved::entries(), &curved_gate());
    assert!(f.is_empty(), "curved corpus gate failed:\n{}", f.join("\n"));
}

/// cad-kernel on the curved corpus, for the record: what it refuses and how
/// close it gets is printed, nothing is asserted.
#[test]
fn cad_kernel_on_the_curved_corpus() {
    let f = run_corpus(&CadKernel, "curved", curved::entries(), &curved_gate());
    for line in &f {
        println!("cad-kernel (not gated): {line}");
    }
}
