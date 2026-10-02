#!/bin/sh
# Build the WebAssembly version into web/pkg. Serve the web/ directory with
# any static file server, e.g. `python3 -m http.server -d web 8000`.
set -e
cd "$(dirname "$0")/.."
PROFILE=${PROFILE:-release}
DIR=$PROFILE; [ "$PROFILE" = dev ] && DIR=debug
cargo build -p amos-app --lib --target wasm32-unknown-unknown --profile "$PROFILE"
wasm-bindgen --target web --no-typescript --out-dir web/pkg \
  "target/wasm32-unknown-unknown/$DIR/amos_app.wasm"
echo "Built web/pkg. Serve with: python3 -m http.server -d web 8000"
