#!/bin/sh
# Puts the kernel components into crates/runspore-wasmtime/prebuilt/, which the
# published runspore-wasmtime crate carries in place of the workspace it cannot
# build them from (spec/release.md section 7). Run it from the checkout of the
# commit being published, before `cargo publish`.
#
#   scripts/stage-crates.sh
set -eu

root=$(cd "$(dirname "$0")/.." && pwd)
cd "$root"
dest=crates/runspore-wasmtime/prebuilt

# The build script's OUT_DIR holds the components this checkout builds. The guest
# build is release and path-remapped whatever the outer profile, so the bytes do not
# depend on it.
out_dir=$(cargo build --locked -p runspore-wasmtime --message-format=json |
	sed -n 's/.*"reason":"build-script-executed","package_id":"[^"]*runspore-wasmtime[^"]*".*"out_dir":"\([^"]*\)".*/\1/p' |
	tail -n 1)
[ -n "$out_dir" ] || { echo "stage-crates: no OUT_DIR for runspore-wasmtime" >&2; exit 1; }

rm -rf "$dest"
mkdir -p "$dest"
for name in kernel test-runner; do
	cp "$out_dir/$name.wasm" "$dest/$name.wasm"
done
( cd "$dest" && if command -v sha256sum >/dev/null 2>&1; then sha256sum ./*.wasm; else shasum -a 256 ./*.wasm; fi )
