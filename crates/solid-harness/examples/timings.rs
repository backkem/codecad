//! Evaluation and tessellation times of every catalogue part on every
//! kernel, for the kernel comparison (docs/research/kernel-comparison.md).
//! A development-time tool; run it in a release build on an idle machine:
//!
//! `cargo run -p solid-harness --release --example timings`
//!
//! Each part is evaluated three times and the fastest run is kept; the
//! tessellation at each chord of the harness (`run::CHORDS`) is timed once
//! on that B-rep. Refused parts count in neither total. The last lines
//! compare the two kernels on the parts both evaluate.

use std::collections::BTreeMap;
use std::time::Instant;

use cadview_core::solid::SolidKernel;
use solid_harness::cases::catalogue;
use solid_harness::kernels::{ArrisKernel, CadKernel};
use solid_harness::run::{guard, CHORDS};

fn ms(t: Instant) -> f64 {
    t.elapsed().as_secs_f64() * 1e3
}

fn pct(v: &[f64], p: f64) -> f64 {
    let mut s = v.to_vec();
    s.sort_by(f64::total_cmp);
    s.get(((s.len() as f64 - 1.0) * p).round() as usize)
        .copied()
        .unwrap_or(0.0)
}

/// One part's evaluation time and the time of each tessellation.
type PartTimes = (f64, Vec<Option<f64>>);

/// Every part's, by name.
type Times = BTreeMap<String, PartTimes>;

fn time<K: SolidKernel>(k: &K) -> Times {
    let cat = catalogue();
    let mut out = Times::new();
    let mut eval = Vec::new();
    let mut mesh: Vec<Vec<f64>> = vec![Vec::new(); CHORDS.len()];
    let mut tris: Vec<usize> = vec![0; CHORDS.len()];
    let mut slow: Vec<(f64, String)> = Vec::new();
    let mut refused = 0;
    for p in &cat.parts {
        let mut best = f64::INFINITY;
        let mut brep = None;
        for _ in 0..3 {
            let t = Instant::now();
            let r = guard(|| k.evaluate(&p.def, &p.tools).map(|e| e.brep));
            best = best.min(ms(t));
            brep = r.ok();
        }
        let Some(b) = brep else {
            refused += 1;
            continue;
        };
        eval.push(best);
        slow.push((best, p.name.clone()));
        let mut per = Vec::new();
        for (i, &c) in CHORDS.iter().enumerate() {
            let t = Instant::now();
            if let Ok(m) = guard(|| k.tessellate(&b, c)) {
                let dt = ms(t);
                mesh[i].push(dt);
                tris[i] += m.triangles.len();
                per.push(Some(dt));
            } else {
                per.push(None);
            }
        }
        out.insert(p.name.clone(), (best, per));
    }
    slow.sort_by(|a, b| b.0.total_cmp(&a.0));
    println!(
        "{}: {} parts evaluated ({} refused or panicked): total {:.1} ms, median {:.3} ms, p95 {:.2} ms, max {:.2} ms",
        k.name(),
        eval.len(),
        refused,
        eval.iter().sum::<f64>(),
        pct(&eval, 0.5),
        pct(&eval, 0.95),
        pct(&eval, 1.0)
    );
    for (i, c) in CHORDS.iter().enumerate() {
        println!(
            "    tessellate chord {c:e}: total {:.1} ms over {} parts, median {:.2} ms, max {:.1} ms, {} triangles",
            mesh[i].iter().sum::<f64>(),
            mesh[i].len(),
            pct(&mesh[i], 0.5),
            pct(&mesh[i], 1.0),
            tris[i]
        );
    }
    for (t, n) in slow.iter().take(5) {
        println!("    slowest: {t:.2} ms {n}");
    }
    out
}

fn main() {
    let profile = if cfg!(debug_assertions) {
        "debug"
    } else {
        "release"
    };
    println!("catalogue timings ({profile} build)");
    let a = time(&ArrisKernel);
    let c = time(&CadKernel);
    let common: Vec<&String> = a.keys().filter(|n| c.contains_key(*n)).collect();
    let sum = |t: &Times, f: &dyn Fn(&PartTimes) -> Option<f64>| -> f64 {
        common.iter().filter_map(|n| f(&t[*n])).sum()
    };
    println!("on the {} parts both evaluate:", common.len());
    println!(
        "    evaluation: arris {:.1} ms, cad-kernel {:.1} ms",
        sum(&a, &|x| Some(x.0)),
        sum(&c, &|x| Some(x.0))
    );
    for (i, ch) in CHORDS.iter().enumerate() {
        println!(
            "    tessellate chord {ch:e}: arris {:.1} ms, cad-kernel {:.1} ms",
            sum(&a, &|x| x.1[i]),
            sum(&c, &|x| x.1[i])
        );
    }
}
