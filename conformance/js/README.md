# JavaScript host conformance

Replays the compiled golden traces and a generated corpus through the kernel
component transpiled with jco, under Node and under Bun, and requires the same
bytes the native kernel returns.

```sh
conformance/js/check.sh
```

needs `cargo`, `bun` and `node` on `PATH` and does everything from a clean
checkout:

1. `cargo run --release -p runspore-conformance-export -- ../traces build/export`
   writes `component.wasm` (the component embedded in `runspore-wasmtime`), every
   `*.json` of `conformance/traces/` compiled with `runspore_trace::compile`, the
   traces of 500 fixed seeds generated against `NativeReducer`, a
   `<trace>.details.json` beside each trace with the native `Failure.details` per
   step, and `manifest.json`. It replays the whole set natively and through
   Wasmtime first and prints both counts.
2. `bun install --frozen-lockfile` installs the pinned `@bytecodealliance/jco`.
3. `bun run transpile` runs `jco transpile --instantiation async` into
   `build/transpiled/`, and the script stamps it with the component's SHA-256.
4. `node run.mjs` and `bun run.mjs` replay every trace in the manifest.

`build/` and `node_modules/` are build products and are not committed.

## What the runner does and does not do

- It holds no workflow or canonical-JSON logic. Request bytes are the UTF-8 of
  the compiled strings; snapshots, command payloads and diagnostic details are
  compared byte for byte with the UTF-8 of the expected strings. Digests, IDs,
  kinds, codes, node IDs, failure kind, code and details are compared as strings.
- `sequence` and `acceptedAtMs` go from decimal strings straight to `BigInt`; a
  JSON number there is rejected.
- Every transition runs in a fresh instance of the component, as in the Rust
  host. The compiled core module is shared because it holds no state.
- It refuses to run if `component.wasm` does not match the manifest or the
  transpiled output was made from a different component.
- It exits 1 at the first mismatch with the trace, step, event, field and an
  excerpt of expected against actual, and otherwise prints one summary line.
