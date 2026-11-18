//! The baseline for the wasm size of arris (rung 3D0b): the same exports
//! as `examples/wasm_corpus.rs` over the same corpus, with no kernel. A
//! development-time tool. Build both for `wasm32-unknown-unknown` and
//! compare their sizes:
//!
//! `cargo build -p solid-harness --release --target wasm32-unknown-unknown --example wasm_corpus --example wasm_corpus_base`
//!
//! `corpus_evaluate` returns the closed-form volume, `corpus_valid` 1 and
//! `corpus_tessellate` 0.

use cadview_core::solid::corpus::{curved, prismatic, CorpusPart};

fn parts() -> Vec<CorpusPart> {
    prismatic::entries()
        .into_iter()
        .chain(curved::entries())
        .flat_map(|e| e.parts)
        .collect()
}

#[no_mangle]
pub extern "C" fn corpus_parts() -> u32 {
    parts().len() as u32
}

#[no_mangle]
pub extern "C" fn corpus_evaluate(i: u32) -> f64 {
    parts().get(i as usize).map_or(f64::NAN, |p| p.volume)
}

#[no_mangle]
pub extern "C" fn corpus_valid() -> u32 {
    1
}

#[no_mangle]
pub extern "C" fn corpus_tessellate(_chord: f64) -> u32 {
    0
}
