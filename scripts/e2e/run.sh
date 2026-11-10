#!/usr/bin/env bash
# Bout en bout navigateur : un hôte réel (test Rust ignoré `e2e_browser_join`) et Chrome
# sans interface qui crée un joueur, rejoint, recharge la page et analyse une voix en wasm.
# Prérequis : Google Chrome dans /Applications, `npm install` dans ce dossier, `npm run web`.
set -euo pipefail
cd "$(dirname "$0")"
export PATH="$HOME/.cargo/bin:$PATH"
out="${TMPDIR:-/tmp}/youliar-e2e"
rm -rf "$out" && mkdir -p "$out"
[ -d node_modules ] || npm install --silent
(cd ../../src-tauri && YOULIAR_E2E="$out/info.txt" cargo test e2e_browser_join -- --ignored --nocapture > "$out/host.log" 2>&1) &
host=$!
node run.js "$out/info.txt" "$out"
wait "$host"
grep E2E "$out/host.log"
echo "Captures : $out"
