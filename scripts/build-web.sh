#!/bin/sh
# Build the WebAssembly version into web/pkg and copy the AMOS distribution
# files into web/amos-files. Serve the web/ directory with any static file
# server, e.g. `python3 -m http.server -d web 8000`.
set -e
cd "$(dirname "$0")/.."
PROFILE=${PROFILE:-release}
DIR=$PROFILE; [ "$PROFILE" = dev ] && DIR=debug
cargo build -p amos-app --lib --target wasm32-unknown-unknown --profile "$PROFILE"
wasm-bindgen --target web --no-typescript --out-dir web/pkg \
  "${CARGO_TARGET_DIR:-target}/wasm32-unknown-unknown/$DIR/amos_app.wasm"
rm -rf web/amos-files
mkdir -p web/amos-files
(cd AMOS-Professional-365/AMOS && find . -type f ! -name '*.info' | sed 's|^\./||' | sort) > web/amos-files/list.txt
(cd AMOS-Professional-365/AMOS && tar cf - $(cat ../../web/amos-files/list.txt | tr '\n' ' ')) 2>/dev/null | (cd web/amos-files && tar xf -) || \
  rsync -a --exclude '*.info' AMOS-Professional-365/AMOS/ web/amos-files/
python3 -c "import json;print(json.dumps(open('web/amos-files/list.txt').read().split('\n')[:-1]))" > web/amos-files/manifest.json
rm web/amos-files/list.txt
echo "Built web/. Serve with: python3 -m http.server -d web 8000"
