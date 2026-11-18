//! The two kernels side by side on the catalogue, for the kernel
//! comparison (docs/research/kernel-comparison.md). A development-time
//! tool:
//!
//! `cargo run -p solid-harness --release --example compare`
//!
//! 1. Every catalogue part on both kernels, judged as the fuzzer judges
//!    (enclosure, shell test, mesh invariants, validity) and compared
//!    (volume to 1e-9 relative, sampled membership of the two meshes).
//!    Prints every part where they disagree or only one answers, and what
//!    the reference says.
//! 2. Every continuity sweep, step by step: each kernel's volume minus the
//!    exact volume of the recipe at that step (closed forms below), or its
//!    refusal. This shows where a kernel snaps a near contact to an exact
//!    one, refuses, or jumps.

use std::f64::consts::PI;

use cadview_core::solid::SolidKernel;
use solid_harness::cases::{catalogue, Sweep};
use solid_harness::fuzz::{cross, judge, Cross};
use solid_harness::kernels::{ArrisKernel, CadKernel};
use solid_harness::outcome::Outcome;
use solid_harness::run::{guard, Fail};

/// The area of a circular segment of depth `h` cut from a circle of
/// radius `r` (0 for `h <= 0`, the whole circle for `h >= 2r`).
fn segment(r: f64, h: f64) -> f64 {
    if h <= 0.0 {
        0.0
    } else if h >= 2.0 * r {
        PI * r * r
    } else {
        r * r * ((r - h) / r).acos() - (r - h) * (2.0 * r * h - h * h).sqrt()
    }
}

/// The overlap of two discs of radius `r` with centres `d` apart.
fn lens(r: f64, d: f64) -> f64 {
    if d >= 2.0 * r {
        0.0
    } else {
        2.0 * r * r * (d / (2.0 * r)).acos() - 0.5 * d * (4.0 * r * r - d * d).sqrt()
    }
}

/// The exact volume of each sweep's recipe at parameter `s` (the
/// catalogue's sweeps, in order; see `cases::sweeps`).
fn exact(name: &str, s: f64) -> Option<f64> {
    let plate = 200.0 * 100.0 * 18.0;
    Some(match name {
        "box tool through a flush top face" => 600.0 * 300.0 * 18.0 - 400.0 * s.max(0.0),
        "dado tool through a flush end face" => {
            600.0 * 300.0 * 18.0 - 300.0 * 6.0 * (18.0 - s.max(0.0))
        }
        "cylinder tool through tangency with a face" => plate - 18.0 * segment(5.0, s),
        "hole breaking out through a face" => plate - 18.0 * (PI * 25.0 - segment(5.0, s)),
        "two holes through tangency" => plate - 18.0 * (2.0 * PI * 25.0 - lens(5.0, 10.0 - s)),
        "fused box through face contact" => 100.0 * 100.0 * 18.0 + 4000.0 - 400.0 * (-s).max(0.0),
        _ => return None,
    })
}

fn sweep_row<K: SolidKernel>(k: &K, sw: &Sweep) -> Vec<String> {
    sw.steps
        .iter()
        .map(|(s, case)| {
            let v = guard(|| {
                let b = k.evaluate(&case.def, &case.tools)?.brep;
                Ok(k.mass(&b)?.volume)
            });
            match (v, exact(&sw.name, *s)) {
                (Ok(v), Some(e)) => format!("{:+.3e}", v - e),
                (Ok(v), None) => format!("{v}"),
                (Err(Fail::Panic(_)), _) => "PANIC".into(),
                (Err(_), _) => "refused".into(),
            }
        })
        .collect()
}

fn main() {
    let cat = catalogue();
    let (a, c) = (ArrisKernel, CadKernel);
    println!("== catalogue parts: {} ==", cat.parts.len());
    let mut counts = std::collections::BTreeMap::<String, usize>::new();
    for p in &cat.parts {
        let (ja, ra) = judge(&a, p);
        let (jc, rc) = judge(&c, p);
        let x = cross(&ja, &ra, &jc, &rc);
        let key = match &x {
            Cross::Agree => "agree",
            Cross::OneAnswered => "one answered",
            Cross::NeitherAnswered => "neither answered",
            Cross::Disagree { .. } => "disagree",
        };
        *counts.entry(key.into()).or_insert(0) += 1;
        let line = |j: &solid_harness::fuzz::Judgement| {
            format!(
                "{} {} {}",
                j.kernel,
                j.outcome.word(),
                j.volume
                    .map(|v| format!("V = {v:?}"))
                    .unwrap_or_else(|| j.refusal.clone().unwrap_or_default())
            )
        };
        match &x {
            Cross::Agree | Cross::NeitherAnswered => {}
            Cross::OneAnswered => {
                println!(
                    "{}: only one answers\n    {}\n    {}",
                    p.name,
                    line(&ja),
                    line(&jc)
                );
            }
            Cross::Disagree { wrong, detail } => {
                println!(
                    "{}: DISAGREE ({}): {detail}\n    {}\n    {}",
                    p.name,
                    if wrong.is_empty() {
                        "the reference finds neither wrong".to_string()
                    } else {
                        format!("the reference finds wrong: {}", wrong.join(", "))
                    },
                    line(&ja),
                    line(&jc)
                );
            }
        }
        for j in [&ja, &jc] {
            if j.outcome == Outcome::Wrong {
                println!(
                    "    {} WRONG on {}: {:?}",
                    p.name,
                    j.kernel,
                    j.first_wrong()
                );
            }
        }
    }
    println!("{counts:?}");
    println!("\n== sweeps: kernel volume minus the exact volume at each step ==");
    for sw in &cat.sweeps {
        println!("{}", sw.name);
        let ra = sweep_row(&a, sw);
        let rc = sweep_row(&c, sw);
        println!("    {:>10} {:>14} {:>14}", "s", "arris", "cad-kernel");
        for (i, (s, _)) in sw.steps.iter().enumerate() {
            println!("    {:>10.0e} {:>14} {:>14}", s, ra[i], rc[i]);
        }
    }
}
