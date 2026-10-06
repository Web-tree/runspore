#!/bin/sh
# From a clean checkout: export the component and the compiled traces, install
# the pinned jco, transpile, and replay every trace under Node and under Bun.
# Needs cargo, bun and node on PATH. Exits non-zero on the first failure.
set -eu
cd "$(dirname "$0")"

rm -rf build
cargo run --release --quiet -p runspore-conformance-export -- ../traces build/export
bun install --frozen-lockfile
bun run transpile
sha=$(shasum -a 256 build/export/component.wasm | cut -d' ' -f1)
echo "sha256:$sha" > build/transpiled/component.sha256
node run.mjs
bun run.mjs
