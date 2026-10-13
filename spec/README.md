# MVP specification

Semantics 0.1 — frozen 2026-10-06. Semantics 0.2 replaces it outright: `spore` 0.2
reads only 0.2 workflow folders and the artifacts built from them, and refuses a
database written by 0.1 or by a newer `spore` with the fix named ([cli.md](cli.md),
[store.md](store.md)). The kernel still speaks both semantics, rule by rule;
[kernel.md](kernel.md) says which rules each version follows. This directory is
normative for the first implementation. Where it is narrower than `docs/`, this
directory wins for the MVP; `docs/` remains the target architecture.

## What the MVP is

A single binary, `spore`, and the Rust library behind it. A user writes a workflow
as a folder holding `workflow.dot` and `settings.json`, starts a run, and can kill the process at any moment. Restarting
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
| Output schemas | 0.2: a schema per outcome on a node; every result, signal and resolution checked against it |
| Effects | `pure`, `read-only`, `idempotent` repeat after an unknown outcome; `unsafe` pauses for an operator |
| Stuck steps | 0.2: a step that gives up with no `failed` route pauses the run for a person, who retries, completes or fails it |
| Loop limits | 0.2: `loopLimits` bounds how many times in a row a step routes an outcome; the next one takes the step's `exhausted` edge. Every node is reachable from the start, and an end from every node |
| Retry | Kernel-decided delivery retries with exponential backoff, same invocation and effect key |
| Actions | `command` (argv, no shell) and registered native Rust functions |
| Store | SQLite, one file, several processes on one host |
| Recovery | Receipts by request ID, revision compare-and-swap, fenced strict leases, lease expiry |
| Hosts | Rust via Wasmtime; Node and Bun verified at the kernel-trace level only |
| Evidence | Golden kernel traces, store conformance tests, crash injection at every failpoint |

## Out of scope for the MVP

Timers, fork and join, child workflows, compensation, `reconcilable` effects, JSON
Schema beyond the subset of [schema.md](schema.md), a YAML frontend, PostgreSQL, a JS
host or SDK, browser and Cloudflare profiles, approvals and capability brokering,
secrets, telemetry, blobs, history compaction, run migration, package signing. Each is
additive under a later semantics version. 0.2 took cancellation, input and output
schema checks, and the DOT frontend off this list; 0.1 had none of them.

## Decisions taken at the freeze

| ID | Decision | Reason |
| --- | --- | --- |
| M01 | One workflow document holds graph and action bindings | No compiler yet; the package digest covers both. **Superseded in 0.2:** a workflow is a folder, `workflow.dot` and `settings.json`, built into an artifact ([dot.md](dot.md), [artifact.md](artifact.md)); `spore` no longer reads a JSON document ([cli.md](cli.md)). The package inside the artifact is still one document, and its digest is still the run's identity |
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
| M12 | No cancellation yet | It needs the dispatch gate and settled-effects tracking done properly. **Superseded in 0.2:** a run can be cancelled (`spore cancel`, [cli.md](cli.md)): it ends as `cancelled`, its history is kept, and a step in progress is stopped |

## Files

- [kernel.md](kernel.md): workflow document, state, events, commands, the transition algorithm.
- [store.md](store.md): store operations, atomicity, failure codes, required tests.
- [host.md](host.md): engine loop, dispatch, activities, failpoints, wasm packaging.
- [cli.md](cli.md): the `spore` command surface and exit codes.
- [dot.md](dot.md): workflow folders, the DOT dialect, settings and the build.
- [schema.md](schema.md): output schemas: the JSON Schema subset and how data is checked.
- [artifact.md](artifact.md): the workflow artifact, its manifest, and how a host loads it.
- [release.md](release.md): release assets, the release key and signature, `install.sh`.

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
crates/runspore-build         Build: workflow folder (DOT and settings) to package and workflow artifact
crates/runspore-schema        Output schemas: the subset, its checks, data checks
crates/runspore-cli           the spore binary and crash-recovery tests
conformance/traces            golden kernel traces
conformance/js                Node and Bun trace runners
examples                      sample workflows
install.sh                    installer that checks a release's signature and checksums
scripts                       release key setup
```
