# MVP work packages

Parent task: implement the Runspore MVP defined in [spec/README.md](../spec/README.md).
Orchestrated with parallel agents, one package per agent, each in its own git worktree
and branch. Only the orchestrator edits this file and `questions/README.md`.
Questions that need Max collect in [questions/](questions/README.md).

| ID | Package | Owns | Depends on | Status | Branch |
| --- | --- | --- | --- | --- | --- |
| P0 | Contract freeze: spec and `runspore-types` | `spec/`, `crates/runspore-types` | — | done 2026-10-06 | main |
| K | Kernel and golden traces | `crates/runspore-kernel`, `crates/runspore-trace`, `conformance/traces` | P0 | running since 2026-10-06 | |
| S | SQLite store and store conformance | `crates/runspore-store-sqlite`, `crates/runspore-store-conformance` | P0 | running since 2026-10-06 | pkg-S |
| C | Component packaging and Wasmtime reducer | `crates/runspore-component`, `crates/runspore-wasmtime`, `contracts/wit/machine/machine.wit` | P0 | running since 2026-10-06 | |
| H | Engine and activity runners | `crates/runspore-host` | K, S | pending | |
| X | Cross-host determinism: Wasmtime, Node, Bun | `conformance/js`, tests in `crates/runspore-wasmtime` | K, C | pending | |
| E | CLI, examples, crash-recovery suite | `crates/runspore-cli`, `examples` | H, C | pending | |

## Done-when

- **K**: every rule of spec/kernel.md section 7 is implemented; golden traces cover
  properties K1 to K9 and every failure code; 10,000 generated traces replay
  identically; the crate builds for `wasm32-unknown-unknown`.
- **S**: every operation of spec/store.md behaves as specified; conformance tests S01
  to S14 and SQLite tests Q01 to Q04 pass.
- **C**: `cargo build` from a clean checkout produces an import-free component;
  `WasmtimeReducer` round-trips every field of the ABI; the build is reproducible.
- **H**: spec/host.md sections 2 and 3 are implemented; evidence H1 to H5 passes.
- **X**: all golden traces and a generated corpus give identical bytes natively,
  through Wasmtime, in Node, and in Bun.
- **E**: every command of spec/cli.md works; evidence H6 passes against the real binary;
  the README quickstart runs as written.

## Log
- 2026-10-06: P0 frozen on main (7f42cf8). Wave 1 launched: K, S, C, each an Opus agent in its own worktree.
- 2026-10-06 13:44: K and S each fanned out to nested sub-agents and stopped; neither left a file, and S's worktree was removed as unchanged. Max: new agents start only on Opus. S worktree recreated on branch `pkg-S`; K and S resumed with "do it yourself, start no sub-agents, do not hand back before committing". C continues.
