#!/bin/bash
set -e
cd "$(dirname "$0")"

# 1. Bundle the sandbox cad API from TypeScript via Vite library mode.
(cd ../web && pnpm exec vite build --config vite.sandbox.config.ts)

# 2. Build the WASM sandbox component.
pnpm install
pnpm exec jco componentize runtime-wrapper.js --wit ../wit/ --world-name cadview-runtime -o cadview-sandbox.wasm

# 3. Copy to Rust crate for include_bytes!.
cp cadview-sandbox.wasm ../crates/cadview-sandbox/cadview-sandbox.wasm
echo "Component built: $(du -h cadview-sandbox.wasm | cut -f1)"
