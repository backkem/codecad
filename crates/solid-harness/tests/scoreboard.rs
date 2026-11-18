//! The scoreboard gate: runs every case of the catalogue on arris through
//! every layer, prints the report, and compares it with the committed
//! `scoreboards/arris.txt`. Fails if any case is wrong, if any case or
//! layer got worse than the committed file, if a case is new or gone, or
//! if a hand-derived closed form falls outside the reference's certified
//! enclosure (a harness bug).
//!
//! Regenerate the file with `SOLID_HARNESS_BLESS=1`. The run takes minutes
//! in a debug build; use `--release`:
//!
//! `cargo test -p solid-harness --release --test scoreboard -- --nocapture`

use std::path::PathBuf;
use std::time::Instant;

use solid_harness::cases::catalogue;
use solid_harness::kernels::ArrisKernel;
use solid_harness::layers::{enclosures, run_all};
use solid_harness::outcome::{Layer, Outcome};
use solid_harness::scoreboard::{compare, parse, render, BLESS_ENV};

fn path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("scoreboards/arris.txt")
}

#[test]
fn arris_scoreboard() {
    let t0 = Instant::now();
    let cat = catalogue();
    let enc = enclosures(&cat);
    println!(
        "{} parts, {} relations, {} sweeps; enclosures in {:.1} s",
        cat.parts.len(),
        cat.relations.len(),
        cat.sweeps.len(),
        enc.seconds
    );
    let report = run_all(&ArrisKernel, &cat, &enc);
    println!("kernel run {:.1} s", report.seconds);
    for r in &report.rows {
        println!("{:<9} {}", r.overall.word(), r.name);
        for (l, v) in Layer::ALL.iter().zip(&r.verdicts) {
            if v.outcome != Outcome::NotApplicable || !v.detail.is_empty() {
                println!("    {:<10} {:<9} {}", l.name(), v.outcome.word(), v.detail);
            }
        }
    }
    for (l, counts) in report.counts() {
        let c: Vec<String> = counts.iter().map(|(o, n)| format!("{n} {o}")).collect();
        println!("{:<10} {}", l.name(), c.join(", "));
    }
    let text = render(&report);
    if std::env::var_os(BLESS_ENV).is_some() {
        std::fs::write(path(), &text).expect("write the scoreboard");
        println!("wrote {}", path().display());
    }
    let committed = std::fs::read_to_string(path()).unwrap_or_default();
    let committed = parse(&committed).expect("the committed scoreboard parses");
    let diff = compare(&committed, &report);
    for i in &diff.improvements {
        println!("improved: {i}");
    }
    println!("total {:.1} s", t0.elapsed().as_secs_f64());
    let mut problems = diff.problems;
    problems.extend(
        enc.closed_form_violations
            .iter()
            .map(|v| format!("harness: {v}")),
    );
    assert!(
        problems.is_empty(),
        "scoreboard gate failed:\n{}",
        problems.join("\n")
    );
}
