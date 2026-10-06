# runspore-wasmtime

`WasmtimeReducer` implements `runspore_types::reducer::Reducer` by running the kernel
component (`crates/runspore-component`) through Wasmtime. Normative rules:
`spec/host.md` section 1.

```rust
let reducer = WasmtimeReducer::new()?;          // embedded kernel, default guards
let result = reducer.transition(&request);      // Result<Decision, Failure>
let digest = reducer.kernel_digest();           // digest::hash("kernel", &[component bytes])
```

## How the component is built

`build/main.rs` runs on every `cargo build` / `cargo test` of this crate:

1. A nested `cargo build --locked --release -p runspore-component --target
   wasm32-unknown-unknown` into its own target directory, `$OUT_DIR/guest-target`.
   It uses the workspace release profile (`opt-level = "s"`, LTO, one codegen unit,
   `panic = "abort"`). Inherited `RUSTFLAGS`, wrappers and target-dir settings are
   removed; the only flags are `--remap-path-prefix` for the checkout, the cargo home
   and the target directory, so no local path reaches the bytes.
2. `wit-component` (a pinned build dependency) turns the core module, which carries
   its WIT as a custom section from `wit-bindgen`, into a validated component.
3. The result is written to `$OUT_DIR/kernel.wasm` and embedded as
   `runspore_wasmtime::COMPONENT`.

The same pipeline builds `$OUT_DIR/echo.wasm`: the guest with its test-only `echo`
feature, which replaces the kernel with echo, count, loop, trap and memory
behaviours. Only the integration tests include it; the library never does.

Nothing beyond the pinned toolchain in `rust-toolchain.toml` (with its
`wasm32-unknown-unknown` target) is needed: no `wasm-tools`, `cargo-component` or
`wasmtime` CLI. The build reruns when the guest, kernel, types crate, the WIT,
`Cargo.toml`, `Cargo.lock` or `rust-toolchain.toml` change.

Write the component to a file (for the Node and Bun package):

```sh
cargo run -p runspore-wasmtime --example emit-component -- path/to/kernel.wasm
```

It prints the kernel digest.

### Reproducibility

`tests/component.rs::component_build_is_reproducible` deletes
`$CARGO_TARGET_TMPDIR/reproducibility`, rebuilds the kernel component there from
scratch with the same pipeline, and requires the same digest as the embedded
component, which was built in `$OUT_DIR`. It runs with every `cargo test`.

## Wasmtime configuration

Wasmtime 49.0.2 with default features off; only `std`, `runtime`, `cranelift` and
`component-model` are on. No WASI, no compilation cache, no profiling, no pooling
allocator, no async, no GC, no `wat`, no parallel compilation.

| Setting | Why |
| --- | --- |
| `threads` feature not compiled | Shared memory and atomics wait/notify admit scheduling nondeterminism. Without the feature the proposal cannot be enabled. |
| `wasm_simd(false)` | The kernel needs no vectors; removes the whole SIMD surface. |
| `wasm_relaxed_simd(false)`, `relaxed_simd_deterministic(true)` | Relaxed SIMD results are documented as platform dependent. |
| `cranelift_nan_canonicalization(true)` | NaN bit patterns are documented as nondeterministic; the kernel uses no floats, this closes the gap if one slips in. |
| `wasm_multi_memory(false)`, `wasm_memory64(false)` | Not needed; smaller validated surface. |
| `consume_fuel(true)`, `Guards::fuel` = 2 000 000 000 | Watchdog. Fuel is counted, not timed, so a request stops at the same point on any machine. Running out is `host.watchdog`. A guest looping forever is stopped after about 1.2 s here. |
| `ResourceLimiter`, `Guards::max_memory_bytes` = 64 MiB; tables at most 10 000 elements | Bounds guest memory. Refused growth is recorded; if the call then fails for any reason it is `host.memory`. |
| `wasm_backtrace_details(Disable)` | `Config::new()` otherwise reads `WASMTIME_BACKTRACE_DETAILS` from the environment; failure details must not depend on it. |
| Default `max_wasm_stack` (512 KiB) | Deep recursion traps as `host.trap`. |

Each `transition` creates a fresh `Store` and instance from a cached
`InstancePre`, so no guest memory survives a call; the compiled component and engine
are shared and `WasmtimeReducer` is `Send + Sync`. `describe` is called once at
construction and cached, because the trait method cannot fail.

Any engine error is `Failure { kind: InvariantViolation, code, details }` with code
`host.memory` (growth was refused), `host.watchdog` (out of fuel) or `host.trap`
(everything else, including instantiation errors). The host never panics on a guest
fault.

## Measurements

Measured on this machine (Apple Silicon, macOS, release build) with the **stub
kernel**, whose `transition` returns a fixed failure. These are measurements, not
guarantees, and will change when the real kernel lands.

| Quantity | Value |
| --- | --- |
| Kernel component size | 32 549 bytes |
| Cold first call: engine, compile, `describe`, one transition | 20.1 ms |
| Warm transition, fresh instance per call (mean of 10 000) | 16.3 µs |

Reproduce with `cargo run --release -p runspore-wasmtime --example overhead`.
