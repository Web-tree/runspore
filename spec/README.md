# MVP specification

Semantics 0.1 — frozen 2026-10-06. This directory is normative for the first
implementation. Where it is narrower than `docs/`, this directory wins for the MVP;
`docs/` remains the target architecture.

## What the MVP is

A single binary, `spore`, and the Rust library behind it. A user writes a workflow
as one JSON document, starts a run, and can kill the process at any moment. Restarting
resumes the run from its last committed step. No completed step runs again. A step
that was interrupted mid-effect is either repeated (when the workflow declares that
safe) or the run pauses and says so.

The same kernel artifact, a WebAssembly component, is executed through Wasmtime by the
binary and is shown to produce byte-identical decisions in Node and Bun.

## In scope

| Area | MVP content |
| --- | --- |
| Graph | `activity`, `await-signal`, `complete`, `fail`; routing by named outcome; bounded loops through back edges |
| Data | Run input, latest result per node, `$get` projection and `$literal` in mappings |
| Effects | `pure`, `read-only`, `idempotent` repeat after an unknown outcome; `unsafe` pauses for an operator |
| Retry | Kernel-decided delivery retries with exponential backoff, same invocation and effect key |
| Actions | `command` (argv, no shell) and registered native Rust functions |
| Store | SQLite, one file, several processes on one host |
| Recovery | Receipts by request ID, revision compare-and-swap, fenced strict leases, lease expiry |
| Hosts | Rust via Wasmtime; Node and Bun verified at the kernel-trace level only |
| Evidence | Golden kernel traces, store conformance tests, crash injection at every failpoint |

## Out of scope for the MVP

Timers, fork and join, child workflows, compensation, cancellation, `reconcilable`
effects, JSON Schema validation of inputs and outputs, a compiler or YAML/DOT
frontends, PostgreSQL, a JS host or SDK, browser and Cloudflare profiles, approvals
and capability brokering, secrets, telemetry, blobs, history compaction, run migration,
package signing. Each is additive under a later semantics version.

## Decisions taken at the freeze

| ID | Decision | Reason |
| --- | --- | --- |
| M01 | One workflow document holds graph and action bindings | No compiler yet; the package digest covers both |
| M02 | The kernel reads only `outcomes`, `effect`, `retry` from an action | Implementation fields stay a host concern |
| M03 | `effect` is required on every action | The author must state what a repeat would do |
| M04 | Retries are kernel decisions carried by an `activity.retry` command | One implementation of retry semantics (ADR A01) |
| M05 | Backoff is a `notBeforeMs` on the invocation, derived from accepted event time | Deterministic without a timer subsystem |
| M06 | One token: a run is at exactly one node | Fork and join wait for their cancellation tests |
| M07 | Activation IDs are readable (`node/visit`); invocation and command IDs are hashes | Readable history, collision-free external keys |
| M08 | The Rust host runs the kernel through Wasmtime by default | The certified artifact is the component, not a native build |
| M09 | The store interface is async from the start | PostgreSQL and remote stores must not force a rewrite |
| M10 | A kernel failure quarantines the run; the event stays pending | A resource or version fault is never a business branch |
| M11 | Commit request IDs are deterministic per revision and event | Competing coordinators collapse into one receipt |
| M12 | No cancellation yet | It needs the dispatch gate and settled-effects tracking done properly |

## Files

- [kernel.md](kernel.md): workflow document, state, events, commands, the transition algorithm.
- [store.md](store.md): store operations, atomicity, failure codes, required tests.
- [host.md](host.md): engine loop, dispatch, activities, failpoints, wasm packaging.
- [cli.md](cli.md): the `spore` command surface and exit codes.

The wire types are code: `crates/runspore-types`. Where prose and that crate disagree,
the crate wins and the prose is a bug.

## Repository layout

```text
crates/runspore-types         frozen contract (canonical JSON, digests, model, traits)
crates/runspore-kernel        pure reducer
crates/runspore-component     kernel as a WebAssembly component (guest)
crates/runspore-wasmtime      Reducer implemented by running that component
crates/runspore-store-sqlite  Store on SQLite
crates/runspore-host          Engine: coordinator, dispatcher, sweeper, activity runners
crates/runspore-cli           the spore binary and crash-recovery tests
conformance/traces            golden kernel traces
conformance/js                Node and Bun trace runners
examples                      sample workflows
```
