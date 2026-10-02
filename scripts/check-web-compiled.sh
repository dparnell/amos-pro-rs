#!/bin/sh
# Checks compiled programs in the real web runtime without a browser: builds
# web/pkg (scripts/build-web.sh must have run), writes a compiled bundle of
# every example program to a temporary directory (the distribution is only
# read), then runs each bundle headless in node, compiled and interpreted,
# and compares state, log and display. Usage: scripts/check-web-compiled.sh [FRAMES]
set -e
cd "$(dirname "$0")/.."
FRAMES=${1:-100}
TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT
cargo run --release -q -p amos-build --example bundle_examples -- "$TMP"
node scripts/web-headless-compare.mjs "$PWD/web/pkg" "$FRAMES" "$TMP"/*.amospak | grep -v '^SAME'
