#!/usr/bin/env bash
# Linker for the kernel module (`just build-kernel` passes it as
# `-C linker`): rust-lld with every `--export NAME` dropped except the
# module's own `ck_*` functions.
#
# rustc exports every `#[no_mangle]` symbol of the whole dependency tree
# from a wasm module. cadview-core links printpdf, whose wasm-bindgen API
# functions are exported on wasm unconditionally; as exports they are roots
# the linker keeps, with everything they reach (about 10 MB of code and
# data the kernel never calls). Without them the linker drops it.
set -euo pipefail
host=$(rustc -vV | sed -n 's/^host: //p')
lld="$(rustc --print sysroot)/lib/rustlib/$host/bin/rust-lld"
args=()
while [[ $# -gt 0 ]]; do
    if [[ $1 == --export && $# -gt 1 ]]; then
        [[ $2 == ck_* ]] && args+=("$1" "$2")
        shift 2
    else
        args+=("$1")
        shift
    fi
done
exec "$lld" "${args[@]}"
