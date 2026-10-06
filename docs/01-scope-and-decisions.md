# 1 Decision and scope

Design revision 0.1 — 2026-10-05. Proposed architecture; implementation and production qualification are pending. Numbered research references resolve in [Sources](sources.md).

Build an embedded durable workflow runtime around one deterministic Rust graph kernel. Compile that kernel to a synchronous, import-free WebAssembly component; expose its contract through WIT. A workflow is an immutable, typed graph package. Host adapters supply transactions, scheduling, activities, authorization, secrets, and telemetry. The invariant is explicit: state plus one recorded event produces a new state and ordered commands. No external effect occurs inside that transition.

Use a prebuilt kernel plus validated graph IR as the default executable representation. An optional pack command can embed the graph in a standalone workflow.wasm component. This keeps the proposed WASM execution model while making new graphs cheap to publish and usable on hosts that restrict dynamic code loading. The portable unit is the package and its semantics; deployment files differ between Wasmtime and JavaScript hosts.

Start with Rust, Node, and Bun on SQLite. Add PostgreSQL before supporting multiple worker machines. Deliver browser and Cloudflare profiles after the same conformance gates pass, with their different durability and availability guarantees stated explicitly. Do not claim that an in-browser process becomes a continuously available worker or that every edge platform has the same capabilities.

This is a proposed architecture, not an implemented or production-certified engine. The contracts in the companion archive are reviewable seeds. Feasibility spikes, adversarial tests, benchmarks, and operational exercises below are release gates, not results claimed by this document. Upstream facts are referenced by source numbers; choices and targets are design recommendations.

## Product boundary

The product provides restartable orchestration, typed graph execution, auditable effects, replaceable storage, and an embedded API. It is useful for local automation, agent pipelines, service workflows, and edge coordination. It does not initially provide a multi-region consensus system, arbitrary-language stack checkpointing, an enterprise workflow UI, a BPMN implementation, or unrestricted hostile-code hosting.

Embedding removes the need for a workflow server. It does not remove the need for a running process: a CLI that exits preserves work but makes no further progress until restarted. Users who require autonomous progress run the same library in a supervised service, a supplied worker executable, or a certified platform adapter.

## Important corrections to the draft

| Draft idea | Production decision |
| --- | --- |
| One arbitrary WASM file loads everywhere | Canonical graph package, Wasmtime component, and trusted generated JS bundles; deployment profile is explicit |
| Compile each graph to fresh executable code | Reuse a bounded interpreter first; optional graph-embedded component |
| WIT defines the workflow runtime | WIT defines types; this specification and conformance tests define behavior |
| The executor loop is almost trivial | Atomic commits, uncertainty, fencing, scheduling, and repair are core product work |
| Non-deterministic workflow flag | Recorded effect activities; reducer remains deterministic |
| Storage can be any key-value implementation | Store must implement semantic atomic operations and declare capabilities |
| The same schema means the same guarantees | Storage, wakeup, isolation, and failure assumptions differ by profile |
| Hashing the WASM solves versioning | Pin graph, kernel, schemas, bindings, policies, and effect implementations |
| Two or three megabyte runtime | No size promise until a reproducible benchmark exists |
