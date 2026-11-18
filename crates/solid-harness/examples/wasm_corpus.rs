//! The corpus on arris, built as a wasm module (rung 3D0b). A
//! development-time tool, not a test. Build it for
//! `wasm32-unknown-unknown`:
//!
//! `cargo build -p solid-harness --release --target wasm32-unknown-unknown --example wasm_corpus`
//!
//! and run it with the native host `examples/wasm_timings.rs`, which loads
//! the module with wasmtime and times every export call from outside (the
//! module itself has no clock and imports nothing).
//!
//! Exports, over the prismatic and then the curved corpus, part by part
//! (the host builds the same list natively, so indices agree):
//!
//! - `corpus_parts() -> u32`: the number of parts;
//! - `corpus_evaluate(i) -> f64`: evaluates part `i` on arris and keeps the
//!   B-rep; returns its volume, or NaN when arris refuses the part;
//! - `corpus_valid() -> u32`: 1 when the kept B-rep is one valid solid;
//! - `corpus_tessellate(chord) -> u32`: tessellates the kept B-rep, returns
//!   the triangle count, or `u32::MAX` when refused.
//!
//! `examples/wasm_corpus_base.rs` exports the same functions with no
//! kernel; the difference in size between the two modules is what arris
//! (with the adapter) adds to a wasm binary.

use std::cell::RefCell;

use cadview_core::solid::corpus::{curved, prismatic, CorpusEntry, CorpusPart};
use cadview_core::solid::SolidKernel;
use solid_harness::kernels::arris::{ArrisBrep, ArrisKernel};

fn parts() -> Vec<(CorpusPart, CorpusEntry)> {
    prismatic::entries()
        .into_iter()
        .chain(curved::entries())
        .flat_map(|e| e.parts.clone().into_iter().map(move |p| (p, e.clone())))
        .collect()
}

thread_local! {
    static KEPT: RefCell<Option<ArrisBrep>> = const { RefCell::new(None) };
}

#[no_mangle]
pub extern "C" fn corpus_parts() -> u32 {
    parts().len() as u32
}

#[no_mangle]
pub extern "C" fn corpus_evaluate(i: u32) -> f64 {
    let all = parts();
    let Some((part, entry)) = all.get(i as usize) else {
        return f64::NAN;
    };
    let k = ArrisKernel;
    let brep = k.evaluate(&part.def, entry).ok().map(|e| e.brep);
    let v = brep
        .as_ref()
        .and_then(|b| k.mass(b).ok())
        .map_or(f64::NAN, |m| m.volume);
    KEPT.with(|kept| *kept.borrow_mut() = brep);
    v
}

#[no_mangle]
pub extern "C" fn corpus_valid() -> u32 {
    KEPT.with(|kept| {
        kept.borrow()
            .as_ref()
            .and_then(|b| ArrisKernel.check(b).ok())
            .map_or(0, |v| u32::from(v.valid && v.solids == 1))
    })
}

#[no_mangle]
pub extern "C" fn corpus_tessellate(chord: f64) -> u32 {
    KEPT.with(|kept| {
        kept.borrow()
            .as_ref()
            .and_then(|b| ArrisKernel.tessellate(b, chord).ok())
            .map_or(u32::MAX, |m| m.triangles.len() as u32)
    })
}
