# CodeCAD build recipes

# Build everything: WASM + frontend + server
build: build-wasm build-web build-server

# Build WASM (cadview-web -> wasm32)
build-wasm:
    cargo build -p cadview-web --target wasm32-unknown-unknown --release
    wasm-bindgen target/wasm32-unknown-unknown/release/cadview-web.wasm --out-dir dist --web

# Build frontend (Vite bundles TS/React + WASM into dist/)
build-web:
    cd web && pnpm build

# Build server
build-server:
    cargo build -p cadview-server --release

# Full rebuild: WASM + frontend + server (in order)
rebuild: build-wasm build-web build-server

# Run server with a DWG file
run file:
    RUST_LOG=info cargo run -p cadview-server --release -- "{{file}}"

# Run tests
test:
    cargo test -p cadview-core
    cargo test -p cadview-web
    cargo test -p cadview-server

# Dev mode (Vite HMR, no server)
dev:
    cd web && pnpm dev
