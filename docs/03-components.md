# 3 Components and ownership

Design revision 0.1 — 2026-10-05. Proposed architecture; implementation and production qualification are pending. Numbered research references resolve in [Sources](sources.md).

The compiler parses source, validates contracts and topology, normalizes defaults, resolves immutable dependencies, and emits a package. The kernel interprets the resulting IR and owns activation identities, branching, joins, loops, retry decisions, and command generation. The host owns I/O and implements the store protocol. Activities execute outside database transactions and return outcomes through the durable inbox.

| Component | Owns | Must not own |
| --- | --- | --- |
| Compiler | Syntax, lint, typed IR, package manifest | Runtime secrets or user approval |
| Kernel | Graph state and deterministic decisions | I/O, ambient time, model calls |
| Coordinator | Read, reduce, validate, commit, wake | A second graph interpreter |
| Store | Atomic state transitions and durable queues | Business branching |
| Dispatcher | Claims, capability checks, execution | Direct workflow-state edits |
| Activity adapter | One effect protocol and result validation | Declaring its own permissions |
| Blob provider | Immutable large values and retention | Run ordering or queue ownership |
| Control API | Authenticated start, signal, inspect, cancel | Bypassing optimistic concurrency |
| Telemetry bridge | Bounded spans, logs, metrics | Authoritative workflow history |

## Repository layout

Use a new repository. Cargo contains workflow-types, workflow-ir, workflow-kernel, workflow-component, workflow-compiler, workflow-host, store-sqlite, store-postgres, and workflow-cli. TypeScript contains runtime, store-node-sqlite, store-bun-sqlite, store-postgres, adapter-browser, adapter-cloudflare, adapter-claude, and sdk. Separate directories contain wit, schemas, conformance, examples, and operational runbooks.

Generated bindings are checked or rebuilt deterministically in CI. Shared serialization, identifiers, retry calculations, decision validation, and canonical hashing belong in Rust and the Wasm kernel; JS host code handles async orchestration and its platform APIs. Reference Rust execution uses Wasmtime and the same component as JS lowering. A native Rust fast path is a later optimization requiring differential testing.

Use a permissive dual MIT or Apache-2.0 license for original code and specifications, subject to dependency review. Avoid copying another engine's implementation without understanding its license. Maintain a security policy, supported-version policy, SBOM, release signatures, and pinned toolchain manifest from the first public release.
