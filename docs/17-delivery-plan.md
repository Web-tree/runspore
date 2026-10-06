# 17 Delivery plan and work packages

Design revision 0.1 — 2026-10-05. Proposed architecture; implementation and production qualification are pending. Numbered research references resolve in [Sources](sources.md).

The dependency chain is protocol and fixtures, then kernel and packaging, then SQLite durability, then action adapters, then shared-worker storage and production controls. Browser, edge, and rich agent UI can develop against the frozen contracts, but their release cannot bypass conformance.

| Milestone | Deliverables | Exit gate |
| --- | --- | --- |
| M0 Contract and feasibility | ADRs, IR schema, WIT, store protocol, component portability spike | Same small graph runs on every proposed target |
| M1 Local engine | Compiler, kernel, SQLite, Rust and Node SDKs, CLI, timers and signals | Crash recovery and cross-host resume pass |
| M2 Effects and Bun | Binding registry, HTTP and command adapters, approvals, Bun driver | Ambiguous-effect suite and independent Bun CI pass |
| M3 Production server | PostgreSQL, remote claims, child runs, compensation, audit and OTel | Model review, soak, restore and security gates pass |
| M4 Agent integration | Epic profile, CLI or MCP bridge, Claude mod, coding pipeline example | Kill and resume demo, exact-revision approval, no duplicate PR |
| M5 Browser and edge | IndexedDB and Cloudflare profiles, export and handover | Real target tests and documented liveness limits |
| M6 Ecosystem | WIT store plugins, activity SDK, registry and import frontends | Third-party implementation passes public conformance |

## Tickets that should exist before implementation starts

P01 freezes canonical values, envelopes, identity derivation, schema subset, and error taxonomy. P02 freezes structured IR and activation semantics. P03 produces reference traces and the bounded protocol model. P04 builds the Rust interpreter and semantic budget. P05 builds WIT packaging and JS lowering. P06 implements the SQLite semantic store and crash injector. P07 builds the host loop and sweeper. P08 implements activity policies and reconciliation. P09 adds capability broker, approvals, secrets, and native isolation. P10 implements SDKs and CLI. P11 adds PostgreSQL and multiworker tests. P12 adds telemetry, backup, upgrade, and incident tools. P13 implements the skills and Claude profile. P14 certifies browser and Cloudflare. P15 publishes plugin contracts and conformance tooling.

Each ticket needs a concrete acceptance fixture, including failures, before implementation. In particular, P01 through P03 are blocking design work: the companion contracts narrow them but do not claim their schema and protocol are already frozen. Do not split across language teams by asking them to implement the same semantics independently.

## Release scope and estimates

Plan roughly 6 to 10 engineer-weeks for the spikes and useful local alpha, and 20 to 35 engineer-weeks total for a narrowly supported production release with PostgreSQL, security controls, conformance, and operations. Browser and Cloudflare plus a polished mod are additional work, plausibly 8 to 14 engineer-weeks. These are planning estimates for experienced Rust, TypeScript, and database engineers, not schedule commitments. Re-estimate after M0; audit and platform surprises can dominate.

The first public promise should be small: one graph semantics implementation, SQLite local recovery, explicit effect guarantees, Rust and JS APIs, and a conformance suite. Multi-region execution, arbitrary custom reducers everywhere, full OWS or SCXML import, live provider swapping, generalized runtime code generation, and a marketplace remain out of the first release.
