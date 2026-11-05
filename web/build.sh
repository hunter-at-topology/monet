#!/usr/bin/env bash
# Build monet for the browser into web/dist: the same Rust engine as the desktop
# app, compiled to WebAssembly (with SIMD), drawing with WebGPU or WebGL2.
#
#   web/build.sh           # then serve web/dist, e.g. python3 -m http.server -d web/dist
#
# Needs: rustup target add wasm32-unknown-unknown
#        cargo install wasm-bindgen-cli --version 0.2.130   (must match Cargo.lock)
#        wasm-opt from binaryen (optional, makes it smaller and faster)
set -euo pipefail
cd "$(dirname "$0")/.."
out=web/dist
rm -rf "$out" && mkdir -p "$out"

RUSTFLAGS="-C target-feature=+simd128 ${RUSTFLAGS:-}" \
  cargo build --profile web --target wasm32-unknown-unknown --bin monet

wasm-bindgen target/wasm32-unknown-unknown/web/monet.wasm \
  --out-dir "$out" --out-name monet --target web --no-typescript

if command -v wasm-opt >/dev/null; then
  wasm-opt -O3 \
    --enable-simd --enable-bulk-memory --enable-nontrapping-float-to-int \
    --enable-sign-ext --enable-mutable-globals --enable-reference-types --enable-multivalue \
    "$out/monet_bg.wasm" -o "$out/monet_bg.wasm"
else
  echo "wasm-opt not found: skipping (brew install binaryen)" >&2
fi

cp web/index.html web/og.jpg web/icon.png "$out/"
# a content hash in the URLs, so a new deploy is never served from an old cache
hash=$(cat "$out/monet.js" "$out/monet_bg.wasm" | shasum | cut -c1-10)
mv "$out/monet.js" "$out/monet-$hash.js"
mv "$out/monet_bg.wasm" "$out/monet-$hash.wasm"
sed -i '' "s/monet_bg.wasm/monet-$hash.wasm/g" "$out/monet-$hash.js" 2>/dev/null || sed -i "s/monet_bg.wasm/monet-$hash.wasm/g" "$out/monet-$hash.js"
sed -i '' "s/monet.js/monet-$hash.js/g" "$out/index.html" 2>/dev/null || sed -i "s/monet.js/monet-$hash.js/g" "$out/index.html"
ls -la "$out"
gzip -9 -c "$out/monet-$hash.wasm" | wc -c | awk '{printf "wasm gzipped: %.1f MB\n", $1/1048576}'
