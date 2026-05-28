#!/usr/bin/env bash
# Build the browser-side renderer.
#
# Compiles `audiovis-wasm` for `wasm32-unknown-unknown` in release mode, then
# post-processes the artefact with `wasm-bindgen` to produce the JS glue the
# browser loads. Output lands in `webui/wasm-pkg/` (gitignored - regenerate on
# build).
#
# Requires the wasm32 target (`rustup target add wasm32-unknown-unknown`) and
# `wasm-bindgen-cli` matching the `wasm-bindgen` crate version
# (`cargo install wasm-bindgen-cli`).
#
# Use rustup's `cargo` even when a Homebrew rustc is on `PATH` first: the brew
# toolchain may not ship the wasm32 std.

set -euo pipefail

# Resolve repo root from this script's location, then make sure rustup's
# cargo wins regardless of which shell PATH ordering put first.
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
export PATH="$HOME/.cargo/bin:$PATH"

cd "$ROOT"

cargo build --target wasm32-unknown-unknown -p audiovis-wasm --release

OUT="$ROOT/webui/wasm-pkg"
mkdir -p "$OUT"

wasm-bindgen \
    "$ROOT/target/wasm32-unknown-unknown/release/audiovis_wasm.wasm" \
    --target web \
    --out-dir "$OUT" \
    --no-typescript

echo "built: $OUT/audiovis_wasm.js + audiovis_wasm_bg.wasm"
ls -lh "$OUT"
