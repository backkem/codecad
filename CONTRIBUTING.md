# Contributing to CodeCAD

## Prerequisites

- **Rust** (nightly toolchain, wasm32-unknown-unknown target)
- **wasm-bindgen-cli**: `cargo install wasm-bindgen-cli`
- **Node.js** (20+) and **pnpm**
- **just** (task runner): `cargo install just`

## Building

```bash
just build          # full build: WASM + frontend + server
just build-wasm     # Rust -> WASM + wasm-bindgen
just build-web      # Vite bundles TS/React into dist/
just build-server   # native server binary
```

After changing Rust code in `cadview-web`, you need both `just build-wasm`
AND `just build-web` (Vite hashes the WASM filename).

## Running

```bash
# Serve a DWG file
RUST_LOG=info cargo run -p cadview-server --release -- path/to/file.dwg

# Serve a folder of DWGs
RUST_LOG=info cargo run -p cadview-server --release -- --dir ./drawings/

# Frontend dev mode (Vite HMR, no server)
cd web && pnpm dev
```

## Running tests

```bash
just test           # all crates
cargo test -p cadview-core
cargo test -p cadview-web
cargo test -p cadview-server
```

## Project layout

See [AGENTS.md](AGENTS.md) for the full crate layout and design decisions.

Short version: all CAD logic goes in `cadview-core`. The web and server
crates are thin shells for rendering and I/O. When adding functionality,
default to putting it in core unless it genuinely cannot compile on both
wasm32 and native targets.

## Code style

- `cargo clippy` must pass with zero warnings.
- TypeScript: `pnpm lint` (Biome) must pass.
- Geometry in the document model is always abstract/mathematical (arcs as
  center+radius+angles, curves as BezPaths). Flattening happens at render
  time only.
- The `cad_call` ABI is the universal entry point for mutations and queries.
  Add new methods there, not as separate exports.

## Pull requests

- Keep PRs focused: one feature or fix per PR.
- Include tests for new `cad_call` methods (see `cadview-core/src/dispatch.rs`
  tests for examples).
- Run `just test` before pushing.
