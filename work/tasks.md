# MVP work packages

Parent task: implement the Runspore MVP defined in [spec/README.md](../spec/README.md).
Orchestrated with parallel agents, one package per agent, each in its own git worktree
and branch. Only the orchestrator edits this file and `questions/README.md`.
Questions that need Max collect in [questions/](questions/README.md).

| ID | Package | Owns | Depends on | Status | Branch |
| --- | --- | --- | --- | --- | --- |
| P0 | Contract freeze: spec and `runspore-types` | `spec/`, `crates/runspore-types` | — | done 2026-10-06 | main |
| K | Kernel and golden traces | `crates/runspore-kernel`, `crates/runspore-trace`, `conformance/traces` | P0 | done 2026-10-06, verified and merged | worktree-agent-a515f322a7eeef774 |
| S | SQLite store and store conformance | `crates/runspore-store-sqlite`, `crates/runspore-store-conformance` | P0 | done 2026-10-06, verified and merged | pkg-S |
| C | Component packaging and Wasmtime reducer | `crates/runspore-component`, `crates/runspore-wasmtime`, `contracts/wit/machine/machine.wit` | P0 | done 2026-10-06, verified and merged | worktree-agent-ab051d9354ed47950 |
| H | Engine and activity runners | `crates/runspore-host` | K, S | done 2026-10-06, verified and merged; follow-up (validate_workflow, finish on shutdown) running | worktree-agent-ae3e3822be5e46fe6 |
| X | Cross-host determinism: Wasmtime, Node, Bun | `conformance/js`, tests in `crates/runspore-wasmtime` | K, C | done 2026-10-06, verified and merged | worktree-agent-ae5ea7e843a88261a |
| E | CLI, examples, crash-recovery suite | `crates/runspore-cli`, `examples` | H, C | done 2026-10-06, verified and merged | worktree-agent-a9f736a8f70b610be |

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
- 2026-10-06 13:58: S verified on disk (fmt, clippy, 14 conformance + 3 SQLite tests, code read) and merged to main. Its four spec gaps (payload-invalid code, generic store codes, capability values, natural-key duplicates write no receipt) are folded into spec/store.md.
- 2026-10-06 14:06: C verified on disk (reducer code read, `cargo test --workspace` on main: 14 component and reducer tests, import-free and reproducible build) and merged. Its notes on failure-code precedence, fuel watchdog, cached describe and the versioned export name are folded into spec/host.md. Component is 32.5 KB with the stub kernel.
- 2026-10-06 14:08: H launched early (Opus, no nested agents) against the merged store; it merges main once K lands for the tests that need the real kernel.
- 2026-10-06 14:20: K's kernel and trace crate (commit 47d0bd5) verified in a throwaway checkout (172 tests), kernel state machine read against spec/kernel.md, merged to main; workspace passes with the real kernel inside the component. K keeps adding golden traces on its branch. H handed back its kernel-independent part (25 tests); resumed for H1 and full-run tests. H's three spec notes folded into spec/host.md.
- 2026-10-06 14:24: X launched (Opus, no nested agents) against the merged kernel, trace crate and component.
- 2026-10-06 14:27: H's first agent ran out of context right after merging main; its work is committed on `worktree-agent-ae3e3822be5e46fe6`. A fresh Opus agent (pkg-H2-host-finish) continues in the same worktree: spec alignment, abandon-on-unreadable, shared test helpers, full-run tests with the real kernel.
- 2026-10-06 14:36: K closed. Signal-starvation fix reviewed as a diff; 48 golden traces (9 hand-derived, 39 blessed and read), 30 codes each expected by a trace, 10,000 generated traces. `cargo test --workspace` on main: 225 passed. Open kernel notes kept for later: budgets bound what is committed, not the work; kernel-local codes should move to runspore-types at its next change.
- 2026-10-06 14:48: H verified (engine tick, coordinate and attempt code read; 30 host tests incl. H1 over 20 racing iterations and native-vs-wasm identical bytes) and merged; `cargo test --workspace` on main: 255 passed. Follow-up sent to the H agent: store-free `validate_workflow`, and finishing stopped attempts on shutdown instead of abandoning them. E launched.
- 2026-10-06 15:00: X verified and merged. I re-ran `conformance/js/check.sh` on main: Node 24.21.0 and Bun 1.4.0 each pass 548 traces / 4,451 steps on component sha256:b00d5996…, the same digest the agent got in another checkout path. Agent-reported, not re-run by me: 20,000-trace native-vs-Wasmtime differential (172,455 steps) identical including failure details. Open notes: trace crate should expose a public `replay`; compiled traces carry no failure details (X exports them in side files).
- 2026-10-06 15:05: H follow-up reviewed as a diff, merged; 33 host tests pass on main. Note for later: after a graceful stop the result is stored but the run's status updates only when the next worker coordinates.
- 2026-10-06 15:12: E's first agent ran out of context with the binary, examples, command tests and crash suite committed (crash suite read by me: reference run, 19 failpoints each asserted to fire, 40 SIGKILL points, five recovery assertions). Remaining, handed to a fresh Opus agent in the same worktree: READMEs, strict needs-intervention assertion, Wasmtime-kernel crash subset, exit codes 5 and 10, final verification. H agent adds a public `Engine::shutdown`. Exit code 130 and shutdown semantics added to the spec.
- 2026-10-06 15:20: `Engine::shutdown` reviewed as a diff and merged; 34 host tests pass on main. Known: `run_until_parked` after `shutdown` never returns for a run that still needs an activity.
- 2026-10-06 15:45: E verified and merged. On main: `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace` (43 suites, 273 passed, 0 failed). Crash suite on main: 19 failpoints (53 crash scenarios) and 40 SIGKILL points all recover; an interrupted unsafe step parks the run. README quickstart run by hand against a release build: hello, kill-and-resume, signal, needs-intervention and resolve all behave as documented. All package branches are merged; agent worktrees removed, branches kept.

## State at wrap-up (2026-10-06)

All packages are done. No open questions in `questions/`.

Left for later, in rough priority:
1. Cancellation (spec decision M12).
2. Timers, then fork and join.
3. A JS host and SDK on the already verified kernel bundle; PostgreSQL store on the existing conformance suite.
4. Kernel: budgets bound what is committed, not the work done; move the three kernel-local failure codes into `runspore-types`.
5. Trace crate: a public `replay`; failure details in compiled traces.
6. Engine: `run_until_parked` after `shutdown` never returns for a run that still needs an activity; a missing package fails the whole tick instead of quarantining one run.
7. Store: Q03 finds its crash helper by newest file; test databases are left in the temp directory.

Human-only: licence choice, package and domain name, creating a remote and pushing, security review, soak testing, real browser and Cloudflare targets.
