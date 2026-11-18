//! The fuzzer (docs/3d-test-harness.md section 4, layer 10). A
//! development-time tool: it never runs in `cargo test` or in CI. What it
//! finds enters the gate as named cases in `src/cases.rs`.
//!
//! ```text
//! cargo run -p solid-harness --release --example fuzz -- run \
//!     --seed 1 --cases 2000 --kernels arris,cad-kernel [--start 0] [--timeout 120] [--shrink 6]
//! cargo run -p solid-harness --release --example fuzz -- one --seed 1 --index 17 --kernels arris,cad-kernel
//! cargo run -p solid-harness --release --example fuzz -- shrink --seed 1 --index 17 --kernels cad-kernel --why wrong
//! cargo run -p solid-harness --release --example fuzz -- shrink --seed 4 --index 195 --kernels cad-kernel --why hang --timeout 30
//! cargo run -p solid-harness --release --example fuzz -- steps --seed 4 --index 195 --kernels cad-kernel
//! ```
//!
//! `--why` for `shrink`: `wrong` (judged wrong), `wrong:<layer>` (wrong
//! first at that layer, or `panic`), `disagree`, `disagree-undecided`, or
//! `hang` (no answer within `--timeout` seconds; each try runs in a child
//! process, `check`). `steps` times evaluation, mass and each tessellation
//! of one case.
//!
//! Kernels: `arris`, `cad-kernel`, and broken ones for testing the fuzzer
//! itself: `broken-arris:<m>` or `broken-cad-kernel:<m>` with `<m>` one of
//! `shift` (tools moved 0.01 mm), `hole` (holes 0.02 mm wider), `snap`
//! (tool positions snapped to 0.01 mm), `swap` (last op swapped), `drop`
//! (last feature dropped), `mass` (volume times 1 + 1e-6).
//!
//! `run` drives child processes (`batch`) so that a hang or an abort costs
//! one case, not the run: a case that prints nothing for `--timeout`
//! seconds is killed and recorded as a hang. Every case is reproducible
//! from (seed, index): `one` reruns it and prints every layer. At the end
//! `run` shrinks the first case of each distinct failure (`--shrink`
//! of them per kernel) and prints the smallest recipe as a function to
//! paste into `src/cases.rs`.

use std::collections::BTreeMap;
use std::io::{BufRead, BufReader};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use cadview_core::solid::SolidKernel;
use solid_harness::fuzz::{
    case_name, cross, defined, emit, generate, judge, shrink, Cross, Judgement, Recipe, JUDGED,
};
use solid_harness::kernels::broken::{Broken, Mutation};
use solid_harness::kernels::{ArrisKernel, CadKernel};
use solid_harness::outcome::Outcome;
use solid_harness::run::PartRun;

// ── Kernels by name ───────────────────────────────────────────────────

#[derive(Debug, Clone, Copy)]
enum Kn {
    Arris,
    Cad,
    BrokenArris(Mutation),
    BrokenCad(Mutation),
}

fn mutation(s: &str) -> Mutation {
    match s {
        "shift" => Mutation::ShiftTool(0.01),
        "hole" => Mutation::HoleDiameter(0.02),
        "snap" => Mutation::SnapTools(0.01),
        "swap" => Mutation::SwapLastOp,
        "drop" => Mutation::DropLastFeature,
        "mass" => Mutation::MassScale(1.0 + 1e-6),
        _ => panic!("unknown mutation {s}"),
    }
}

fn kernel(s: &str) -> Kn {
    match s {
        "arris" => Kn::Arris,
        "cad-kernel" => Kn::Cad,
        _ => {
            if let Some(m) = s.strip_prefix("broken-arris:") {
                Kn::BrokenArris(mutation(m))
            } else if let Some(m) = s.strip_prefix("broken-cad-kernel:") {
                Kn::BrokenCad(mutation(m))
            } else {
                panic!("unknown kernel {s}")
            }
        }
    }
}

/// Calls `$body` with `$k` bound to the concrete kernel.
macro_rules! with_kernel {
    ($kn:expr, |$k:ident| $body:expr) => {
        match $kn {
            Kn::Arris => {
                let $k = &ArrisKernel;
                $body
            }
            Kn::Cad => {
                let $k = &CadKernel;
                $body
            }
            Kn::BrokenArris(m) => {
                let $k = &Broken {
                    inner: ArrisKernel,
                    mutation: m,
                };
                $body
            }
            Kn::BrokenCad(m) => {
                let $k = &Broken {
                    inner: CadKernel,
                    mutation: m,
                };
                $body
            }
        }
    };
}

// ── Arguments ─────────────────────────────────────────────────────────

struct Args {
    mode: String,
    seed: u64,
    cases: u64,
    start: u64,
    index: u64,
    to: u64,
    kernels: Vec<String>,
    timeout: u64,
    shrink: usize,
    why: String,
    recipe: String,
}

fn args() -> Args {
    let v: Vec<String> = std::env::args().skip(1).collect();
    let mut a = Args {
        mode: v.first().cloned().unwrap_or_else(|| "run".into()),
        seed: 1,
        cases: 1000,
        start: 0,
        index: 0,
        to: 0,
        kernels: vec!["arris".into(), "cad-kernel".into()],
        timeout: 120,
        shrink: 6,
        why: "wrong".into(),
        recipe: String::new(),
    };
    let mut i = 1;
    while i < v.len() {
        let val = v.get(i + 1).cloned().unwrap_or_default();
        match v[i].as_str() {
            "--seed" => a.seed = val.parse().expect("--seed"),
            "--cases" => a.cases = val.parse().expect("--cases"),
            "--start" => a.start = val.parse().expect("--start"),
            "--index" => a.index = val.parse().expect("--index"),
            "--to" => a.to = val.parse().expect("--to"),
            "--kernels" => a.kernels = val.split(',').map(str::to_string).collect(),
            "--timeout" => a.timeout = val.parse().expect("--timeout"),
            "--shrink" => a.shrink = val.parse().expect("--shrink"),
            "--why" => a.why = val,
            "--recipe" => a.recipe = val,
            other => panic!("unknown argument {other}"),
        }
        i += 2;
    }
    a
}

// ── Judging one case ──────────────────────────────────────────────────

/// A short, number-free signature of a failure or refusal, for grouping.
fn signature(s: &str) -> String {
    let mut out = String::new();
    let mut last_hash = false;
    for c in s.chars() {
        if c.is_ascii_digit() || c == '.' || c == '-' || c == 'e' && last_hash {
            if !last_hash {
                out.push('#');
            }
            last_hash = true;
        } else {
            out.push(c);
            last_hash = false;
        }
    }
    out.chars().take(90).collect()
}

fn describe(j: &Judgement) -> (String, String) {
    if let Some((layer, d)) = j.first_wrong() {
        return (format!("{layer}: {}", signature(d)), d.to_string());
    }
    if let Some(r) = &j.refusal {
        return (format!("refused: {}", signature(r)), r.clone());
    }
    let refused: Vec<String> = JUDGED
        .iter()
        .zip(&j.verdicts)
        .filter(|(_, v)| v.outcome == Outcome::Refused)
        .map(|(n, v)| format!("{n}: {}", v.detail))
        .collect();
    if j.outcome == Outcome::Refused && !refused.is_empty() {
        let s = refused.join("; ");
        return (format!("refused after evaluation: {}", signature(&s)), s);
    }
    let undecided: Vec<String> = JUDGED
        .iter()
        .zip(&j.verdicts)
        .filter(|(_, v)| v.outcome == Outcome::Undecided)
        .map(|(n, v)| format!("{n}: {}", v.detail))
        .collect();
    if !undecided.is_empty() {
        let s = undecided.join("; ");
        return (format!("undecided: {}", signature(&s)), s);
    }
    (String::new(), String::new())
}

fn clean(s: &str) -> String {
    s.replace(['\t', '\n', '\r'], " ")
        .chars()
        .take(600)
        .collect()
}

/// Judges case `index` on one kernel, printing its `K` line.
fn judge_one<K: SolidKernel>(
    k: &K,
    recipe: &Recipe,
    name: &str,
    index: u64,
) -> (Judgement, PartRun<K::Brep>) {
    println!("RUN\t{index}\t{}", k.name());
    let case = recipe.case(name);
    let (j, run) = judge(k, &case);
    let (sig, detail) = describe(&j);
    println!(
        "K\t{index}\t{}\t{}\t{}\t{:.2}\t{}\t{}",
        j.kernel,
        j.outcome.word(),
        j.volume.map_or("-".to_string(), |v| format!("{v:?}")),
        j.eval_ms,
        clean(&sig),
        clean(&detail)
    );
    (j, run)
}

fn cross_line(index: u64, c: &Cross) {
    match c {
        Cross::Agree => println!("X\t{index}\tagree\t"),
        Cross::OneAnswered => println!("X\t{index}\tone\t"),
        Cross::NeitherAnswered => println!("X\t{index}\tneither\t"),
        Cross::Disagree { wrong, detail } => println!(
            "X\t{index}\tdisagree\t{}\t{}",
            if wrong.is_empty() {
                "undecided".to_string()
            } else {
                wrong.join(",")
            },
            clean(detail)
        ),
    }
}

/// Judges one case on one or two kernels and prints the protocol lines.
fn case_lines(seed: u64, index: u64, kernels: &[Kn]) {
    println!("BEGIN\t{index}");
    let recipe = generate(seed, index);
    let name = case_name(seed, index);
    if !defined(&recipe.case(&name)) {
        println!("SKIP\t{index}\treference not defined");
        println!("END\t{index}");
        return;
    }
    match kernels {
        [a] => {
            with_kernel!(*a, |k| {
                judge_one(k, &recipe, &name, index);
            });
        }
        [a, b] => {
            with_kernel!(*a, |ka| {
                let (ja, ra) = judge_one(ka, &recipe, &name, index);
                with_kernel!(*b, |kb| {
                    let (jb, rb) = judge_one(kb, &recipe, &name, index);
                    cross_line(index, &cross(&ja, &ra, &jb, &rb));
                });
            });
        }
        _ => panic!("one or two kernels"),
    }
    println!("END\t{index}");
}

// ── Modes ─────────────────────────────────────────────────────────────

fn batch(a: &Args) {
    let kernels: Vec<Kn> = a.kernels.iter().map(|s| kernel(s)).collect();
    for i in a.start..a.to {
        case_lines(a.seed, i, &kernels);
    }
}

fn one(a: &Args) {
    let recipe = if a.recipe.is_empty() {
        generate(a.seed, a.index)
    } else {
        Recipe::decode(&a.recipe).expect("a recipe from Recipe::encode")
    };
    let name = case_name(a.seed, a.index);
    println!("{recipe:#?}");
    let case = recipe.case(&name);
    println!("{}", emit(&case, "case", "as generated"));
    let kernels: Vec<Kn> = a.kernels.iter().map(|s| kernel(s)).collect();
    for kn in kernels {
        with_kernel!(kn, |k| {
            let (j, _) = judge(k, &case);
            println!(
                "{}: {} volume {:?} eval {:.2} ms",
                j.kernel,
                j.outcome.word(),
                j.volume,
                j.eval_ms
            );
            if let Some(r) = &j.refusal {
                println!("    refused: {r}");
            }
            if let Some(p) = &j.panic {
                println!("    PANIC: {p}");
            }
            for (n, v) in JUDGED.iter().zip(&j.verdicts) {
                println!("    {n:<10} {:<9} {}", v.outcome.word(), v.detail);
            }
        });
    }
    if let [ka, kb] = a.kernels.iter().map(|s| kernel(s)).collect::<Vec<_>>()[..] {
        with_kernel!(ka, |x| {
            let (ja, ra) = judge(x, &case);
            with_kernel!(kb, |y| {
                let (jb, rb) = judge(y, &case);
                println!("cross: {:?}", cross(&ja, &ra, &jb, &rb));
            });
        });
    }
}

/// Whether `r` still fails the way `why` says. `why` is `wrong` (the
/// single kernel is judged wrong), `wrong:<layer>` (wrong first at that
/// layer, or `panic`), `disagree` (two kernels disagree) or
/// `disagree-undecided` (they disagree and the reference finds neither
/// wrong).
fn still_fails(r: &Recipe, name: &str, kernels: &[Kn], why: &str) -> bool {
    let case = r.case(name);
    if !defined(&case) {
        return false;
    }
    match kernels {
        [a] => with_kernel!(*a, |k| {
            let (j, _) = judge(k, &case);
            match why.strip_prefix("wrong:") {
                Some(layer) => j.first_wrong().is_some_and(|(l, _)| l == layer),
                None => j.outcome == Outcome::Wrong,
            }
        }),
        [a, b] => with_kernel!(*a, |ka| {
            let (ja, ra) = judge(ka, &case);
            with_kernel!(*b, |kb| {
                let (jb, rb) = judge(kb, &case);
                match cross(&ja, &ra, &jb, &rb) {
                    Cross::Disagree { wrong, .. } => why == "disagree" || wrong.is_empty(),
                    _ => false,
                }
            })
        }),
        _ => false,
    }
}

/// Judges an encoded recipe (`--recipe`) and prints `DONE`: the child a
/// hang check runs.
fn check(a: &Args) {
    let r = Recipe::decode(&a.recipe).expect("a recipe from Recipe::encode");
    let case = r.case("check");
    for s in &a.kernels {
        with_kernel!(kernel(s), |k| {
            let _ = judge(k, &case);
        });
    }
    println!("DONE");
}

/// Whether judging `r` on `kernels` takes longer than `secs` (run in a
/// child process, killed when it does).
fn hangs(r: &Recipe, kernels: &[String], secs: u64) -> bool {
    if !defined(&r.case("check")) {
        return false;
    }
    let argv = vec![
        "check".to_string(),
        "--kernels".into(),
        kernels.join(","),
        "--recipe".into(),
        r.encode(),
    ];
    let (mut child, rx) = spawn(&argv);
    let deadline = Instant::now() + Duration::from_secs(secs);
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        match rx.recv_timeout(left.max(Duration::from_millis(1))) {
            Ok(Some(l)) if l == "DONE" => {
                let _ = child.wait();
                return false;
            }
            Ok(Some(_)) => {}
            // Ended without DONE: a crash, not a hang.
            Ok(None) => {
                let _ = child.wait();
                return false;
            }
            Err(_) => {
                let _ = child.kill();
                let _ = child.wait();
                return true;
            }
        }
    }
}

fn shrink_mode(a: &Args) {
    let kernels: Vec<Kn> = a.kernels.iter().map(|s| kernel(s)).collect();
    let name = case_name(a.seed, a.index);
    let r = generate(a.seed, a.index);
    if a.why == "hang" {
        // A hang cannot be judged in this process: each try is a child
        // given `--timeout` seconds.
        let ks = a.kernels.clone();
        let secs = a.timeout;
        let print = |best: &Recipe| {
            println!("BEST-BEGIN");
            println!("{best:?}");
            println!("encoded: {}", best.encode());
            print!(
                "{}",
                emit(
                    &best.case(&name),
                    &format!("fuzz_s{}_i{}", a.seed, a.index),
                    &format!(
                        "Found by the fuzzer (seed {}, index {}): {} gives no answer in {} s, shrunk.",
                        a.seed,
                        a.index,
                        a.kernels.join(" and "),
                        secs
                    )
                )
            );
            println!("BEST-END");
        };
        if !hangs(&r, &ks, secs) {
            println!("NOFAIL");
            return;
        }
        print(&r);
        let best = shrink(&r, &mut |t| hangs(t, &ks, secs), &mut |b| print(b));
        print(&best);
        println!("DONE");
        return;
    }
    if !still_fails(&r, &name, &kernels, &a.why) {
        println!("NOFAIL");
        return;
    }
    let print = |best: &Recipe| {
        let fn_name = format!("fuzz_s{}_i{}", a.seed, a.index);
        let why = format!(
            "Found by the fuzzer (seed {}, index {}) on {}: {}, shrunk.",
            a.seed,
            a.index,
            a.kernels.join(" and "),
            a.why
        );
        println!("BEST-BEGIN");
        println!("{best:?}");
        println!(
            "fuzz_case({:?}, {:?}),",
            format!("fuzz s{} i{}", a.seed, a.index),
            best.encode()
        );
        print!("{}", emit(&best.case(&name), &fn_name, &why));
        println!("BEST-END");
    };
    print(&r);
    let best = shrink(
        &r,
        &mut |t| still_fails(t, &name, &kernels, &a.why),
        &mut |b| print(b),
    );
    print(&best);
    println!("DONE");
}

// ── The driver ────────────────────────────────────────────────────────

#[derive(Default)]
struct Tally {
    run: usize,
    by_outcome: BTreeMap<String, usize>,
    /// signature -> (count, first index, detail)
    wrong: BTreeMap<String, (usize, u64, String)>,
    refused: BTreeMap<String, (usize, u64, String)>,
    undecided: BTreeMap<String, (usize, u64, String)>,
    hangs: Vec<u64>,
    crashes: Vec<u64>,
    ms: Vec<f64>,
}

fn add(map: &mut BTreeMap<String, (usize, u64, String)>, sig: &str, i: u64, detail: &str) {
    let e = map
        .entry(sig.to_string())
        .or_insert((0, i, detail.to_string()));
    e.0 += 1;
}

/// Runs `fuzz <args>` as a child, sending its stdout lines; `None` on EOF.
fn spawn(args: &[String]) -> (std::process::Child, mpsc::Receiver<Option<String>>) {
    let exe = std::env::current_exe().expect("current exe");
    let mut child = Command::new(exe)
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn a fuzz child");
    let out = child.stdout.take().expect("stdout");
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        for line in BufReader::new(out).lines() {
            match line {
                Ok(l) => {
                    if tx.send(Some(l)).is_err() {
                        return;
                    }
                }
                Err(_) => break,
            }
        }
        let _ = tx.send(None);
    });
    (child, rx)
}

fn run(a: &Args) {
    let t0 = Instant::now();
    let end = a.start + a.cases;
    let mut tallies: BTreeMap<String, Tally> = BTreeMap::new();
    let mut skipped = 0usize;
    let mut cross_counts: BTreeMap<String, usize> = BTreeMap::new();
    let mut disagreements: Vec<(u64, String, String)> = Vec::new();
    let mut next = a.start;
    let timeout = Duration::from_secs(a.timeout);
    while next < end {
        let to = end.min(next + 250);
        let argv: Vec<String> = vec![
            "batch".into(),
            "--seed".into(),
            a.seed.to_string(),
            "--start".into(),
            next.to_string(),
            "--to".into(),
            to.to_string(),
            "--kernels".into(),
            a.kernels.join(","),
        ];
        let (mut child, rx) = spawn(&argv);
        let mut current: Option<(u64, String)> = None;
        let mut finished = next;
        loop {
            match rx.recv_timeout(timeout) {
                Ok(Some(line)) => {
                    let c: Vec<&str> = line.split('\t').collect();
                    match c[0] {
                        "BEGIN" => current = Some((c[1].parse().unwrap(), String::new())),
                        "RUN" => {
                            if let Some(cur) = &mut current {
                                cur.1 = c[2].to_string();
                            }
                        }
                        "SKIP" => skipped += 1,
                        "K" => {
                            let i: u64 = c[1].parse().unwrap();
                            let t = tallies.entry(c[2].to_string()).or_default();
                            t.run += 1;
                            *t.by_outcome.entry(c[3].to_string()).or_insert(0) += 1;
                            t.ms.push(c[5].parse().unwrap_or(0.0));
                            let (sig, detail) = (
                                c.get(6).copied().unwrap_or(""),
                                c.get(7).copied().unwrap_or(""),
                            );
                            match c[3] {
                                "WRONG" => add(&mut t.wrong, sig, i, detail),
                                "refused" => add(&mut t.refused, sig, i, detail),
                                "undecided" => add(&mut t.undecided, sig, i, detail),
                                _ => {}
                            }
                        }
                        "X" => {
                            let i: u64 = c[1].parse().unwrap();
                            *cross_counts.entry(c[2].to_string()).or_insert(0) += 1;
                            if c[2] == "disagree" {
                                disagreements.push((
                                    i,
                                    c.get(3).copied().unwrap_or("").to_string(),
                                    c.get(4).copied().unwrap_or("").to_string(),
                                ));
                            }
                        }
                        "END" => {
                            finished = c[1].parse::<u64>().unwrap() + 1;
                            current = None;
                            if finished.is_multiple_of(100) {
                                eprintln!(
                                    "seed {} case {finished}/{end} ({:.0} s)",
                                    a.seed,
                                    t0.elapsed().as_secs_f64()
                                );
                            }
                        }
                        _ => {}
                    }
                }
                Ok(None) => {
                    // The child ended. If a case was in flight, it crashed.
                    let _ = child.wait();
                    if let Some((i, k)) = current.take() {
                        tallies.entry(k).or_default().crashes.push(i);
                        finished = i + 1;
                    }
                    break;
                }
                Err(_) => {
                    let _ = child.kill();
                    let _ = child.wait();
                    if let Some((i, k)) = current.take() {
                        eprintln!(
                            "seed {} case {i}: no answer from {k} in {} s",
                            a.seed, a.timeout
                        );
                        tallies.entry(k).or_default().hangs.push(i);
                        finished = i + 1;
                    }
                    break;
                }
            }
        }
        next = finished.max(next + if finished == next { 1 } else { 0 });
    }
    // ── Report ──
    println!(
        "\nfuzz seed {} cases {}..{} kernels {} ({:.0} s, {} not defined by the reference and skipped)",
        a.seed,
        a.start,
        end,
        a.kernels.join(","),
        t0.elapsed().as_secs_f64(),
        skipped
    );
    for (k, t) in &tallies {
        let mut ms = t.ms.clone();
        ms.sort_by(f64::total_cmp);
        let pct = |p: f64| {
            ms.get(((ms.len() as f64 - 1.0) * p) as usize)
                .copied()
                .unwrap_or(0.0)
        };
        println!(
            "{k}: {} judged: {:?}; {} hangs {:?}; {} crashes {:?}; eval ms median {:.2} p95 {:.2} max {:.2}",
            t.run,
            t.by_outcome,
            t.hangs.len(),
            t.hangs,
            t.crashes.len(),
            t.crashes,
            pct(0.5),
            pct(0.95),
            pct(1.0)
        );
        for (title, map) in [
            ("WRONG", &t.wrong),
            ("refused", &t.refused),
            ("undecided", &t.undecided),
        ] {
            for (sig, (n, i, d)) in map {
                println!(
                    "  {title} x{n} (first index {i}): {sig}\n      {}",
                    d.chars().take(300).collect::<String>()
                );
            }
        }
    }
    println!("cross: {cross_counts:?}");
    for (i, who, d) in &disagreements {
        println!("  disagree at index {i}: reference says wrong: {who}; {d}");
    }
    // ── Shrink the first case of each distinct failure ──
    let mut jobs: Vec<(u64, Vec<String>, String, String)> = Vec::new();
    for (k, t) in &tallies {
        for (sig, (_, i, _)) in t.wrong.iter().take(a.shrink) {
            let why = match sig.split(':').next() {
                Some(l) if JUDGED.contains(&l) || l == "panic" => format!("wrong:{l}"),
                _ => "wrong".into(),
            };
            jobs.push((*i, vec![k.clone()], why, sig.clone()));
        }
    }
    let names: Vec<(String, String)> = a
        .kernels
        .iter()
        .map(|s| (s.clone(), with_kernel!(kernel(s), |k| k.name())))
        .collect();
    let arg_of = |display: &str| {
        names
            .iter()
            .find(|(_, n)| n == display)
            .map(|(a, _)| a.clone())
            .unwrap_or_else(|| display.to_string())
    };
    let mut seen_disagree: Vec<String> = Vec::new();
    for (i, who, d) in &disagreements {
        let sig = format!(
            "{who}: {}",
            signature(d).chars().take(40).collect::<String>()
        );
        if seen_disagree.contains(&sig) || seen_disagree.len() >= a.shrink {
            continue;
        }
        seen_disagree.push(sig.clone());
        let why = if who == "undecided" {
            "disagree-undecided"
        } else {
            "disagree"
        };
        jobs.push((*i, a.kernels.clone(), why.into(), sig));
    }
    for (i, ks, why, sig) in jobs {
        let ks: Vec<String> = ks.iter().map(|k| arg_of(k)).collect();
        println!(
            "\n== shrinking index {i} on {} ({why}): {sig}",
            ks.join(",")
        );
        let argv: Vec<String> = vec![
            "shrink".into(),
            "--seed".into(),
            a.seed.to_string(),
            "--index".into(),
            i.to_string(),
            "--kernels".into(),
            ks.join(","),
            "--why".into(),
            why,
        ];
        let (mut child, rx) = spawn(&argv);
        let deadline = Instant::now() + Duration::from_secs(10 * a.timeout);
        let (mut block, mut best) = (Vec::new(), Vec::new());
        let mut inside = false;
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            match rx.recv_timeout(left.max(Duration::from_millis(1))) {
                Ok(Some(l)) if l == "BEST-BEGIN" => {
                    inside = true;
                    block.clear();
                }
                Ok(Some(l)) if l == "BEST-END" => {
                    inside = false;
                    best = block.clone();
                }
                Ok(Some(l)) if l == "NOFAIL" => {
                    println!("(does not fail when rerun on its own)");
                }
                Ok(Some(l)) => {
                    if inside {
                        block.push(l);
                    }
                }
                Ok(None) => break,
                Err(_) => {
                    let _ = child.kill();
                    println!(
                        "(shrinking stopped after {} s; best so far below)",
                        10 * a.timeout
                    );
                    break;
                }
            }
        }
        let _ = child.wait();
        for l in best {
            println!("{l}");
        }
    }
}

/// Evaluation, mass and each tessellation of one case, timed step by
/// step (to see where a slow case spends its time).
fn steps(a: &Args) {
    let case = generate(a.seed, a.index).case(&case_name(a.seed, a.index));
    for s in &a.kernels {
        with_kernel!(kernel(s), |k| {
            let t = Instant::now();
            let b =
                solid_harness::run::guard(|| k.evaluate(&case.def, &case.tools).map(|e| e.brep));
            println!(
                "{}: evaluate {:.1} ms: {}",
                k.name(),
                t.elapsed().as_secs_f64() * 1e3,
                b.is_ok()
            );
            if let Ok(b) = b {
                let t = Instant::now();
                let m = solid_harness::run::guard(|| k.mass(&b));
                println!(
                    "    mass {:.1} ms: {:?}",
                    t.elapsed().as_secs_f64() * 1e3,
                    m.map(|m| m.volume)
                );
                for c in solid_harness::run::CHORDS {
                    let t = Instant::now();
                    let m = solid_harness::run::guard(|| k.tessellate(&b, c));
                    println!(
                        "    tessellate {c:e}: {:.1} ms: {:?}",
                        t.elapsed().as_secs_f64() * 1e3,
                        m.map(|m| m.triangles.len())
                    );
                }
            }
        });
    }
}

fn main() {
    let a = args();
    match a.mode.as_str() {
        "run" => run(&a),
        "batch" => batch(&a),
        "one" => one(&a),
        "steps" => steps(&a),
        "shrink" => shrink_mode(&a),
        "check" => check(&a),
        m => panic!("unknown mode {m} (run, one, shrink)"),
    }
}
