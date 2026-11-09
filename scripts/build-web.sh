#!/usr/bin/env bash
# Compile le moteur en WebAssembly pour les joueurs qui rejoignent depuis un navigateur.
# Prérequis : rustup target add wasm32-unknown-unknown
#             cargo install wasm-bindgen-cli --version 0.2.130 --locked
set -euo pipefail
cd "$(dirname "$0")/.."
export PATH="$HOME/.cargo/bin:$PATH"
cargo build -p youliar-web --release --target wasm32-unknown-unknown
wasm-bindgen --target web --no-typescript --out-dir ui/web \
  target/wasm32-unknown-unknown/release/youliar_web.wasm
echo "Moteur web généré dans ui/web/"
