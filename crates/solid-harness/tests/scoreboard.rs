//! The scoreboard gate: runs every case of the catalogue on every kernel
//! that has a committed scoreboard (`scoreboards/<kernel>.txt`) through
//! every layer, prints the report, and compares it with the committed
//! file. Fails if any case is wrong where the committed file does not
//! record it as wrong, if any case or layer got worse than the committed
//! file, if a case is new or gone, if a hand-derived closed form falls
//! outside the reference's certified enclosure (a harness bug), or if a
//! scoreboard file has no kernel here to run it. A wrong verdict the file
//! records (a finding against that kernel, from the fuzzer) is printed,
//! not failed: the scoreboard keeps it as evidence, and it may only move
//! toward correct.
//!
//! Regenerate a file with `SOLID_HARNESS_BLESS=1` (filter by test name to
//! regenerate one kernel's). The run takes minutes in a debug build; use
//! `--release`:
//!
//! `cargo test -p solid-harness --release --test scoreboard -- --nocapture`

use std::path::PathBuf;
use std::time::Instant;

use cadview_core::solid::SolidKernel;
use solid_harness::cases::catalogue;
use solid_harness::kernels::{ArrisKernel, CadKernel};
use solid_harness::layers::{band_table, enclosures, run_all};
use solid_harness::outcome::{Layer, Outcome};
use solid_harness::scoreboard::{compare, parse, render, BLESS_ENV};

/// Every kernel with a scoreboard, by its file stem. A test below runs each.
const KERNELS: [&str; 2] = ["arris", "cad-kernel"];

fn dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("scoreboards")
}

fn path(stem: &str) -> PathBuf {
    dir().join(format!("{stem}.txt"))
}

/// Runs the catalogue on `kernel` and compares with `scoreboards/<stem>.txt`.
fn gate<K: SolidKernel>(kernel: &K, stem: &str) {
    let t0 = Instant::now();
    let cat = catalogue();
    let enc = enclosures(&cat);
    println!(
        "{}: {} parts, {} relations, {} sweeps; enclosures in {:.1} s",
        kernel.name(),
        cat.parts.len(),
        cat.relations.len(),
        cat.sweeps.len(),
        enc.seconds
    );
    if stem == KERNELS[0] {
        println!(
            "positional band of the enclosure layer per part:\n{}",
            band_table(&cat, &enc)
        );
    }
    let report = run_all(kernel, &cat, &enc);
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
        std::fs::write(path(stem), &text).expect("write the scoreboard");
        println!("wrote {}", path(stem).display());
    }
    let committed = std::fs::read_to_string(path(stem)).unwrap_or_default();
    let committed = parse(&committed).expect("the committed scoreboard parses");
    let diff = compare(&committed, &report);
    for i in &diff.improvements {
        println!("improved: {i}");
    }
    for w in &diff.recorded_wrong {
        println!("recorded finding (still wrong): {w}");
    }
    println!(
        "{}: {} wrong verdicts recorded as findings",
        kernel.name(),
        diff.recorded_wrong.len()
    );
    println!("total {:.1} s", t0.elapsed().as_secs_f64());
    let mut problems = diff.problems;
    problems.extend(
        enc.closed_form_violations
            .iter()
            .map(|v| format!("harness: {v}")),
    );
    assert!(
        problems.is_empty(),
        "{} scoreboard gate failed:\n{}",
        kernel.name(),
        problems.join("\n")
    );
}

#[test]
fn arris_scoreboard() {
    gate(&ArrisKernel, KERNELS[0]);
}

#[test]
fn cad_kernel_scoreboard() {
    gate(&CadKernel, KERNELS[1]);
}

/// Every committed scoreboard belongs to a kernel this file runs, and every
/// kernel here has one.
#[test]
fn every_scoreboard_has_a_kernel() {
    let mut stems: Vec<String> = std::fs::read_dir(dir())
        .expect("scoreboards/ exists")
        .filter_map(|e| e.ok())
        .filter_map(|e| {
            let p = e.path();
            if p.extension()? != "txt" {
                return None;
            }
            Some(p.file_stem()?.to_string_lossy().into_owned())
        })
        .collect();
    stems.sort();
    let mut known: Vec<String> = KERNELS.iter().map(|s| s.to_string()).collect();
    known.sort();
    assert_eq!(
        stems, known,
        "scoreboards/*.txt and the kernels this test runs must match"
    );
}
