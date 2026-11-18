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

# Package an all-in-one release archive for the host platform.
# Mirrors .github/workflows/release.yml; tag defaults to "dev".
package tag="dev": build-wasm clean-dist build-web build-sandbox-api build-packed
    #!/usr/bin/env bash
    set -euo pipefail
    target=$(rustc -vV | sed -n 's/^host: //p')
    ext=""
    if [[ "$target" == *windows* ]]; then ext=".exe"; fi
    dir="codecad-{{tag}}-${target}"
    rm -rf "release/$dir" && mkdir -p "release/$dir"
    cp "target/release/cadview-server${ext}" "release/$dir/codecad${ext}"
    cp README.md LICENSE "release/$dir/"
    cd release
    if [[ "$target" == *windows* ]]; then
      rm -f "$dir.zip" && 7z a "$dir.zip" "$dir" >/dev/null && archive="$dir.zip"
    else
      tar czf "$dir.tar.gz" "$dir" && archive="$dir.tar.gz"
    fi
    if command -v sha256sum >/dev/null; then
      sha256sum "$archive" > "$archive.sha256"
    else
      shasum -a 256 "$archive" > "$archive.sha256"
    fi
    echo "release/$archive"

# Build the solid kernel module the browser loads on demand (dist/kernel/:
# a content-hashed .wasm and manifest.json with its wire version). Optional:
# without it the page shows prebaked scenes and server bakes only.
build-kernel:
    #!/usr/bin/env bash
    set -euo pipefail
    # link.sh keeps only the module's own exports, so the linker drops the
    # code of the dependency tree the kernel never calls.
    cargo rustc -p cadview-kernel-wasm --lib --crate-type cdylib --target wasm32-unknown-unknown --release -- -C strip=symbols -C linker="$PWD/crates/cadview-kernel-wasm/link.sh"
    src=target/wasm32-unknown-unknown/release/cadview_kernel_wasm.wasm
    hash=$(sha256sum "$src" | cut -c1-64)
    file="cadview-kernel-${hash:0:16}.wasm"
    wire=$(sed -n 's/^pub const WIRE_VERSION: u32 = \([0-9]*\);/\1/p' crates/cadview-core/src/solid/remote.rs)
    mkdir -p dist/kernel
    rm -f dist/kernel/cadview-kernel-*.wasm
    cp "$src" "dist/kernel/$file"
    printf '{"file":"%s","wire":%s,"bytes":%s,"sha256":"%s"}\n' "$file" "$wire" "$(wc -c < "$src" | tr -d ' ')" "$hash" > dist/kernel/manifest.json
    echo "dist/kernel/$file ($(wc -c < "$src" | tr -d ' ') bytes, wire $wire)"

# Build embeddable viewer (ESM library, no React)
build-embed:
    cd web && pnpm exec vite build --config vite.embed.config.ts

# Build GH Pages site: homepage + gallery + example pages + app + embed +
# examples. The example files are copied as committed (drawings, scripts,
# renders, prebaked scenes): no kernel, GPU or server step.
build-site: build-wasm clean-dist build-web build-embed
    rm -rf _site
    mkdir -p _site/app _site/embed _site/examples/lib _site/examples/3d/lib _site/assets
    cp site/*.html site/*.css site/*.js _site/
    cp dist/index.html _site/app/
    cp -r dist/assets _site/app/
    cp dist/cadview-web.js _site/app/
    cp dist/cadview-web_bg.wasm _site/app/
    cp dist-embed/codecad-viewer.js dist-embed/cadview-web.js dist-embed/cadview-web_bg.wasm _site/embed/
    cp examples/*.dwg examples/*.png examples/*.js _site/examples/
    cp examples/lib/*.js _site/examples/lib/
    cp examples/3d/*.js examples/3d/*.png examples/3d/*.dwg examples/3d/*.bake3d _site/examples/3d/
    cp examples/3d/lib/*.js _site/examples/3d/lib/
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
    cargo test -p cadview-render
    cargo test -p cadview-web
    cargo test -p cadview-server

# Dev mode (Vite HMR, no server)
dev:
    cd web && pnpm dev
