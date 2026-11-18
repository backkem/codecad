# The solid corpus in wasm: size and speed (rung 3D0b)

Measured on 2026-11-18 on one Windows x64 machine (24 hardware threads),
release builds, the workspace's default release profile (no LTO, no
`wasm-opt`). The kernel is arris `=0.11.1` through the harness's adapter
(`crates/solid-harness/src/kernels/arris.rs`), with its `paranoid`
feature (its checker runs on every operation's output).

## How

The corpus (the 9 prismatic and the 4 curved parts) is compiled into a
wasm module, `examples/wasm_corpus.rs` in `solid-harness`, a `cdylib`
for `wasm32-unknown-unknown` that exports four functions (count,
evaluate one part and keep its B-rep, check it, tessellate it). A native
host, `examples/wasm_timings.rs`, loads the module with the `wasmtime`
crate (50.0.1, the version the workspace already locks through the script
sandbox; Cranelift compiles it ahead of time), calls every export and
times each call from outside, and runs the same adapter natively in the
same process for comparison. No Node and nothing installed globally.

The module imports six functions: wasm-bindgen glue that cadview-core's
browser dependencies bring in. The host stubs each with a function that
traps when called; no run called one.

```bash
cargo build -p solid-harness --release --target wasm32-unknown-unknown \
    --example wasm_corpus --example wasm_corpus_base
cargo run -p solid-harness --release --features wasm-runner --example wasm_timings -- \
    target/wasm32-unknown-unknown/release/examples/wasm_corpus.wasm \
    target/wasm32-unknown-unknown/release/examples/wasm_corpus_base.wasm
gzip -9 -c target/wasm32-unknown-unknown/release/examples/wasm_corpus.wasm | wc -c
```

Each part is evaluated three times and the fastest run kept, natively and
in wasm; each tessellation is timed once, on that B-rep.

## Size

`wasm_corpus_base.rs` is the same module with no kernel: the same
exports over the same corpus, returning the closed forms. The difference
is what arris and the adapter add.

| module | total bytes | code | data | custom (names, debug) | gzip -9 |
|---|---:|---:|---:|---:|---:|
| corpus, no kernel | 10 424 341 | 4 966 119 | 4 352 168 | 1 091 624 | 4 039 722 |
| corpus on arris | 15 277 993 | 9 032 325 | 4 432 672 | 1 792 424 | 5 486 092 |
| **arris adds** | **4 853 652** | **4 066 206** | **80 504** | **700 800** | **1 446 370** |

So arris adds about 4.1 MB of code and data (4.9 MB with its name and
debug sections, which a strip removes) and 1.4 MB gzipped. The baseline
itself is large because cadview-core's linked code and embedded data
(fonts among them) are in it; it is not a measure of the web bundle.
Not measured: the size with LTO or after `wasm-opt`, and the delta in the
real `cadview-web` bundle (no shipped crate links a kernel yet).

## Timings (ms)

| part | native | wasm | wasm / native | native tessellation at 0.1 / 0.01 / 0.001 mm | wasm tessellation |
|---|---:|---:|---:|---|---|
| board | 0.37 | 2.03 | 5.6 | 0.2 / 0.1 / 0.1 | 0.1 / 0.1 / 0.1 |
| board with hole | 19.79 | 30.87 | 1.6 | 0.3 / 0.5 / 1.5 | 0.3 / 0.6 / 1.6 |
| side with dado | 1.38 | 5.11 | 3.7 | 0.1 / 0.1 / 0.1 | 0.2 / 0.2 / 0.1 |
| back with rabbet | 1.09 | 3.90 | 3.6 | 0.1 / 0.1 / 0.1 | 0.1 / 0.1 / 0.1 |
| stile with mortise | 1.16 | 4.78 | 4.1 | 0.1 / 0.1 / 0.1 | 0.2 / 0.1 / 0.1 |
| rail with tenon | 1.19 | 4.74 | 4.0 | 0.1 / 0.1 / 0.1 | 0.2 / 0.1 / 0.1 |
| rail with half-lap | 1.38 | 5.09 | 3.7 | 0.1 / 0.1 / 0.1 | 0.2 / 0.2 / 0.2 |
| block with counterbore | 50.52 | 68.63 | 1.4 | 0.5 / 1.2 / 3.5 | 0.6 / 1.5 / 4.3 |
| chamfered slab | 0.92 | 3.99 | 4.4 | 0.1 / 0.1 / 0.1 | 0.1 / 0.1 / 0.1 |
| rounded slab | 3.04 | 15.36 | 5.1 | 0.9 / 5.4 / 68.8 | 1.2 / 7.1 / 89.6 |
| tapered square bar | 0.43 | 2.11 | 4.9 | 0.1 / 0.1 / 0.1 | 0.1 / 0.1 / 0.1 |
| revolved profile with bead and cove | 6.58 | 13.22 | 2.0 | 3.5 / 41.7 / 754.7 | 4.4 / 55.0 / 1103.3 |
| swept round bar | 195.15 | 282.50 | 1.4 | 1.7 / 16.4 / 262.0 | 2.3 / 21.3 / 366.2 |
| **total** | **283.0** | **442.4** | **1.6** | | |

Every part evaluates in wasm to the same volume as natively (relative
error to the closed form 0 to 7.4e-16 in both), as one valid solid. **No
corpus entry is refused**, natively or in wasm.

Reading the table:

- The slowest part is the swept round bar: 195 ms natively, 283 ms in
  wasm, almost all of it arris's sweep along the arc. Everything else
  evaluates in under 70 ms in wasm, the rounded slab in 15 ms.
- Small parts cost 4 to 6 times more in wasm, but they take a few ms;
  the slow parts are 1.4 to 2 times slower, so the totals differ by 1.6x.
- Tessellation at render chords (0.1 and 0.01 mm) stays under 60 ms in
  wasm. At 0.001 mm the revolved profile takes 1.1 s; the design's zoom
  buckets (section 6) stop at 0.005 mm.
- These are wasmtime's Cranelift numbers, not a browser's. A browser's
  optimising compiler may be faster or slower. The slowest evaluation
  (283 ms) spans many frames, so evaluation stays off the input path as
  the design plans (cached by content hash, time-sliced, or a worker),
  and so does tessellation of curved parts at fine chords.
