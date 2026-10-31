#!/bin/sh
# Build both examples to WebAssembly into www/pkg. Needs the wasm32 target and wasm-bindgen-cli 0.2.130.
set -eu
cd "$(dirname "$0")"
cargo build --release --target wasm32-unknown-unknown -p reagera-example-todo -p reagera-example-latency
for name in todo latency; do
  wasm-bindgen --target web --no-typescript --out-dir "www/pkg/$name" "target/wasm32-unknown-unknown/release/reagera_example_$name.wasm"
done
ls -la www/pkg/*/*.wasm
