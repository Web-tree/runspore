# Contract seeds

Runspore — portable durable workflow runtime
Architecture contract seeds v0.1 — 2026-10-05

Read [the architecture documentation](../docs/README.md) first.

This archive contains design inputs for implementation, not a functioning engine.
No Rust/WASM runtime, database adapter, production benchmark, or Claude integration
was implemented or executed in this architecture task.

Files
  wit/machine/machine.wit     Import-free synchronous reducer boundary
  wit/telemetry/telemetry.wit Activity/host telemetry extension boundary
  store.ts                   Semantic store operation and receipt contracts
  action-binding.example.json Example effectful Action binding
  conformance-cases.json     Required adversarial scenarios and assertions
  VALIDATION.txt             Checks actually performed on these seeds

Normative design choices
  - One Rust graph semantics implementation; JS hosts use its Wasm build.
  - WIT exports types, not the behavioral or atomicity proof.
  - Use RFC 8785/JCS plus the narrowed value domain described in the document.
  - UInt64 wire strings are capped at 2^63-1 for SQL interoperability.
  - Pure run-to-completion segments are bounded; no transparent sliced reducer
    continuation in v1. Explicit graph yield is a normal durable boundary.
  - Commit snapshot, consumed inbox event, and commands atomically.
  - Deduplicate each mutation's operation ID plus body digest transactionally,
    before evaluating revision predicates. Unknown acknowledgements reuse the ID.
  - Keep stable logical invocation/effect IDs distinct from physical attempts.
  - Strict leases cannot be resurrected by late heartbeats.
  - Completion/expiry races are resolved by the authoritative store transaction.
  - External side effects are not guaranteed exactly once by this runtime.
  - Approval consumption binds to one logical operation, allowing its authorized
    reconciliation without granting a second operation.

Before freezing v1
  1. Finalize and generate graph/state/event/command JSON schemas and validators.
  2. Parse WIT with pinned wasm-tools and generate Rust/TS bindings.
  3. Build the component portability spike and run byte-equality trace fixtures.
  4. Implement the protocol model and database fault injector.
  5. Turn conformance-cases.json scenarios into executable adapter tests.
  6. Publish tested target versions and capability limits.

The Store interface is a native async contract for v1. A future WIT provider must
implement these same semantic operations and tests; generic get/put is insufficient.
The telemetry WIT world is an optional effectful extension, not an import of the
pure reducer. Its opaque live handles must never be serialized into run snapshots.

The project name is Runspore and the WIT package namespace is `runspore:`. The domain is
runspore.com; registry names are not yet reserved. The existing skills repository is a separate
integration target.
