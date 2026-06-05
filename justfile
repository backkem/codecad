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

# Build sandbox API setup (cad-api.ts -> IIFE for server-side scripts)
build-sandbox-api:
    cd web && pnpm exec vite build --config vite.sandbox.config.ts

# Build server (rebuilds sandbox API first)
build-server: build-sandbox-api
    cargo build -p cadview-server --release

# Build single packed binary (frontend + WASM baked in)
build-packed: build-wasm build-web build-sandbox-api
    cargo build -p cadview-server --release --features embedded-dist

# Build packed binary with examples too
build-packed-full: build-wasm build-web build-sandbox-api
    cargo build -p cadview-server --release --features embedded-dist,embedded-examples

# Build embeddable viewer (ESM library, no React)
build-embed:
    cd web && pnpm exec vite build --config vite.embed.config.ts

# Build GH Pages site: homepage + app + embed + examples
build-site: build-wasm clean-dist build-web build-embed
    rm -rf _site
    mkdir -p _site/app _site/embed _site/examples _site/assets
    cp site/index.html site/site.css _site/
    cp dist/index.html _site/app/
    cp -r dist/assets _site/app/
    cp dist/cadview-web.js _site/app/
    cp dist/cadview-web_bg.wasm _site/app/
    cp dist-embed/codecad-viewer.js dist-embed/cadview-web.js dist-embed/cadview-web_bg.wasm _site/embed/
    cp examples/*.dwg examples/*.png _site/examples/
    cp docs/brand/icon-mark.svg _site/assets/favicon.svg
    cp docs/brand/logo.svg _site/assets/logo.svg

# Remove stale hashed assets from dist/
clean-dist:
    rm -f dist/assets/cadview-web_bg-*.wasm dist/assets/index-*.js dist/assets/index-*.css

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
