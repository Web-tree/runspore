# 5 Deterministic kernel contract

Design revision 0.1 — 2026-10-05. Proposed architecture; implementation and production qualification are pending. Numbered research references resolve in [Sources](sources.md).

The reducer exports describe and transition. The host calls these exports. An absent snapshot plus a Started event creates the initial state; a separate init path is unnecessary. A transition request contains graph bytes, prior snapshot, one normalized event, package identity, and the frozen semantic budget. The host resolves and verifies graph references before calling the import-free guest; a graph-embedded component is the alternative. Host caches are keyed by the full graph, kernel, and ABI identity. The result contains the replacement snapshot, an ordered command list, deterministic diagnostics, and a state digest.

The WIT interface is deliberately small. Stable envelopes are typed; graph, state, and payload schemas are independently versioned. An opaque byte field is not permission to accept arbitrary encodings: v1 uses the canonical data rules below. The companion machine.wit is a starting contract, not a substitute for the behavioral requirements here.

## Canonical data

Use RFC 8785 JCS canonical UTF-8 JSON with a narrowed value model. Restrict numeric JSON values to integers within plus or minus 9007199254740991; normalize negative zero to zero. Full-width nonnegative counters use decimal strings matching 0 or a nonzero digit followed by digits, capped at 9223372036854775807 for SQL interoperability. Exact decimals use an explicit tagged coefficient and scale representation, with normalized trailing zeros. Reject duplicate keys, invalid Unicode, NaN, infinities, excessive nesting, and unsupported tags. JCS sorts keys by UTF-16 code units, which is not always Rust UTF-8 lexical order. Preserve strings without implicit Unicode normalization and preserve array order. Hash domain-separated, length-delimited fields, never ambiguous string concatenations. [7]

Large values are immutable BlobRef records containing digest, length, media type, classification, and provider locator. A reducer can compare or forward references, but cannot fetch them. Any calculation requiring blob contents is an activity. Keep snapshots and ordinary event bodies small; initial configurable caps are 256 KiB per state, 64 KiB per event, 128 commands per transition, and 32 parallel activations. These are starting safety defaults to tune with evidence.

## Determinism profile

The portable kernel has no clock, random, network, filesystem, environment, database, or native callback imports. It uses ordered collections. Floating-point arithmetic, relaxed SIMD, threads, and shared memory are excluded from graph expressions. Exact decimal arithmetic can be added as a versioned library with fixtures; v1 branches use strings, booleans, integers, enums, and structural data.

No ambient I/O is necessary but insufficient: Wasmtime documents nondeterminism in NaN representations, relaxed SIMD, memory growth, and interruptions. The admitted kernel uses bounded arenas; allocation failure is a host resource error and never a business branch. Pin the allowed Wasm features and validate the complete import graph. [8]

Explicit state must contain all behavior-relevant continuation data. Fresh instances are the reference for third-party components; cached compiled modules are safe to share, mutable guest instances are not. Reusing the project kernel's instance requires tests proving that dirty memory and prior calls cannot change outputs. Arbitrary custom reducer components are excluded from the cross-host v1 guarantee.

## Semantic budgets and errors

Count graph microsteps and expression operations in the kernel, independently of engine instruction fuel. V1 admits only graphs whose pure run-to-completion segment fits a fixed allowance, initially 1,000 microsteps. Loops must contain a durable boundary and have a total activation limit; an explicit yield node is a normal durable boundary. Exceeding the allowance is a resource-limit fault that pauses the run, not an automatically retried business failure. The compiler rejects statically over-budget segments and the kernel enforces dynamic bounds. Move long computation to a bounded activity. Transparent sliced transitions are deferred: they would require an active-event cursor and must finish the active logical event before delivering later inbox events. This avoids unspecified interleaving halfway through an event.

Wasmtime fuel, deadlines, OS limits, and JS worker termination are watchdogs. A watchdog interruption discards an uncommitted result and reports a host fault. It cannot select a normal failed outcome based on processor speed. Bound encoded input, graph size, fan-out, recursion, expression depth, output size, and memory separately. Standard JS engines do not expose Wasmtime fuel controls. [9]

Classify errors as business outcome, activity failure, malformed input, unsupported profile, resource limit, kernel invariant violation, storage conflict, ambiguous storage commit, and ambiguous external effect. Only declared business or activity failures enter graph error routes. Kernel corruption and incompatible artifacts quarantine the run for investigation.
