//! Native and wasm timings of the corpus on arris (rung 3D0b), side by
//! side on one machine. A development-time tool, not a test; it needs the
//! `wasm-runner` feature (the `wasmtime` crate, the version the workspace
//! already locks through the script sandbox).
//!
//! ```text
//! cargo build -p solid-harness --release --target wasm32-unknown-unknown --example wasm_corpus
//! cargo run -p solid-harness --release --features wasm-runner --example wasm_timings -- \
//!     target/wasm32-unknown-unknown/release/examples/wasm_corpus.wasm
//! ```
//!
//! The module (`examples/wasm_corpus.rs`) is compiled ahead of time by
//! wasmtime's Cranelift, instantiated with no imports, and every export
//! call is timed from the host. Each part is evaluated three times and the
//! fastest kept, natively and in wasm; each tessellation is timed once on
//! that B-rep. The native side runs the same adapter in this process.

use std::time::Instant;

use cadview_core::solid::corpus::{curved, prismatic, CorpusEntry, CorpusPart};
use cadview_core::solid::SolidKernel;
use solid_harness::kernels::ArrisKernel;
use wasmtime::{Engine, ExternType, Linker, Module, Store};

/// Chords of the timed tessellations (mm): a coarse view, a close view and
/// the harness's shell-test neighbour.
const CHORDS: [f64; 3] = [1e-1, 1e-2, 1e-3];

fn ms(t: Instant) -> f64 {
    t.elapsed().as_secs_f64() * 1e3
}

fn parts() -> Vec<(CorpusPart, CorpusEntry)> {
    prismatic::entries()
        .into_iter()
        .chain(curved::entries())
        .flat_map(|e| e.parts.clone().into_iter().map(move |p| (p, e.clone())))
        .collect()
}

/// A module's bytes split into code, data, custom (names, debug info: what
/// a strip removes) and everything else, read from its section headers.
fn sections(b: &[u8]) -> Option<[usize; 4]> {
    let mut out = [0usize; 4];
    let mut i = 8; // magic and version
    while i < b.len() {
        let id = b[i];
        i += 1;
        let (mut size, mut shift) = (0usize, 0);
        loop {
            let byte = *b.get(i)?;
            i += 1;
            size |= ((byte & 0x7f) as usize) << shift;
            shift += 7;
            if byte & 0x80 == 0 {
                break;
            }
        }
        let k = match id {
            10 => 0,
            11 => 1,
            0 => 2,
            _ => 3,
        };
        out[k] += size;
        i += size;
    }
    Some(out)
}

fn main() -> wasmtime::Result<()> {
    let mut args = std::env::args().skip(1);
    let path = args.next().unwrap_or_else(|| {
        "target/wasm32-unknown-unknown/release/examples/wasm_corpus.wasm".into()
    });
    // Further arguments: other modules to print the section sizes of
    // (the size baseline).
    for p in std::iter::once(path.clone()).chain(args) {
        let raw = std::fs::read(&p)?;
        if let Some([code, data, custom, other]) = sections(&raw) {
            println!(
                "{p}: {} bytes: code {code}, data {data}, custom (names, debug) {custom}, other {other}",
                raw.len()
            );
        }
    }
    let bytes = std::fs::metadata(&path)?.len();
    let engine = Engine::default();
    let t = Instant::now();
    let module = Module::from_file(&engine, &path)?;
    let compile_ms = ms(t);
    let mut store = Store::new(&engine, ());
    // Imports the module carries from its dependencies (wasm-bindgen glue
    // that cadview-core links for the browser): each is a stub that traps
    // when called, so a run that needs one fails loudly.
    let mut linker = Linker::new(&engine);
    for imp in module.imports() {
        if let ExternType::Func(ty) = imp.ty() {
            let name = format!("{}::{}", imp.module(), imp.name());
            println!("import stubbed (traps if called): {name}");
            linker.func_new(imp.module(), imp.name(), ty.clone(), move |_, _, _| {
                Err(wasmtime::Error::msg(format!("import {name} called")))
            })?;
        }
    }
    let instance = linker.instantiate(&mut store, &module)?;
    let count = instance.get_typed_func::<(), u32>(&mut store, "corpus_parts")?;
    let eval = instance.get_typed_func::<u32, f64>(&mut store, "corpus_evaluate")?;
    let valid = instance.get_typed_func::<(), u32>(&mut store, "corpus_valid")?;
    let tess = instance.get_typed_func::<f64, u32>(&mut store, "corpus_tessellate")?;

    let all = parts();
    let n = count.call(&mut store, ())? as usize;
    assert_eq!(
        n,
        all.len(),
        "the module and the host see different corpora"
    );
    println!(
        "module {path}: {bytes} bytes, compiled by Cranelift in {compile_ms:.0} ms; {n} parts"
    );
    println!(
        "{:<38} {:>10} {:>10} {:>6} {:>8} | {:>27} | {:>27}",
        "part",
        "native ms",
        "wasm ms",
        "ratio",
        "rel err",
        "native tess ms (0.1/0.01/0.001)",
        "wasm tess ms (0.1/0.01/0.001)"
    );
    let k = ArrisKernel;
    let (mut tn, mut tw) = (0.0, 0.0);
    let mut refused = Vec::new();
    for (i, (part, entry)) in all.iter().enumerate() {
        // Native.
        let mut best_n = f64::INFINITY;
        let mut brep = None;
        for _ in 0..3 {
            let t = Instant::now();
            let r = k.evaluate(&part.def, entry);
            best_n = best_n.min(ms(t));
            brep = r.ok().map(|e| e.brep);
        }
        let mut tess_n = Vec::new();
        if let Some(b) = &brep {
            for c in CHORDS {
                let t = Instant::now();
                let ok = k.tessellate(b, c).is_ok();
                tess_n.push(if ok {
                    format!("{:.1}", ms(t))
                } else {
                    "-".into()
                });
            }
        }
        // Wasm.
        let mut best_w = f64::INFINITY;
        let mut v = f64::NAN;
        let mut trap = None;
        for _ in 0..3 {
            let t = Instant::now();
            match eval.call(&mut store, i as u32) {
                Ok(x) => {
                    best_w = best_w.min(ms(t));
                    v = x;
                }
                Err(e) => {
                    trap = Some(e.to_string());
                    break;
                }
            }
        }
        let mut tess_w = Vec::new();
        if trap.is_none() && v.is_finite() {
            for c in CHORDS {
                let t = Instant::now();
                let r = tess.call(&mut store, c);
                tess_w.push(match r {
                    Ok(x) if x != u32::MAX => format!("{:.1}", ms(t)),
                    _ => "-".into(),
                });
            }
        }
        let ok = trap.is_none() && v.is_finite() && valid.call(&mut store, ())? == 1;
        if !ok {
            refused.push(format!(
                "{}: {}",
                part.def.name,
                trap.clone().unwrap_or_else(|| "refused or invalid".into())
            ));
        }
        if brep.is_some() && ok {
            tn += best_n;
            tw += best_w;
        }
        println!(
            "{:<38} {:>10.2} {:>10.2} {:>6.2} {:>8.1e} | {:>27} | {:>27}",
            part.def.name.chars().take(38).collect::<String>(),
            best_n,
            best_w,
            best_w / best_n,
            (v - part.volume).abs() / part.volume,
            tess_n.join(" / "),
            tess_w.join(" / ")
        );
    }
    println!(
        "evaluation total: native {tn:.1} ms, wasm {tw:.1} ms ({:.2}x)",
        tw / tn
    );
    if refused.is_empty() {
        println!("no part refused in wasm");
    } else {
        for r in refused {
            println!("refused in wasm: {r}");
        }
    }
    Ok(())
}
