# 2 Research and build versus adopt

Design revision 0.1 — 2026-10-05. Proposed architecture; implementation and production qualification are pending. Numbered research references resolve in [Sources](sources.md).

Obelisk is the closest reference: it uses the Component Model for deterministic workflows and records execution in SQLite or PostgreSQL. Its repository explicitly describes a prerelease surface. The runtime is AGPL-3.0, while specified interface directories have different licenses. It validates the architecture direction but is not evidence of a ready embedded Rust and JavaScript library with this graph and storage contract. Treat it as a design reference or an alternative deployment, and review licenses before reusing code. [1]

Open Workflow Specification, formerly Serverless Workflow, already supplies a declarative ecosystem and links Rust and TypeScript SDKs. Its DSL spans integrations, scripts, events, branches, and waits. Adopt a narrow import adapter only after the native IR is stable; reject unsupported constructs rather than claim full conformance. Those SDK links do not by themselves establish durable cross-runtime execution. [2]

SCXML provides mature statechart semantics. Its run-to-completion approach is useful for ordered microsteps, but full SCXML compatibility would add a large language and executable-content surface. DOT is appropriate for visualization and the existing skills profile, but it does not define durable execution semantics. [3]

DBOS remains a sensible alternative when embedding in one supported application language is enough. Temporal, Restate, and Obelisk remain preferable when their established execution models meet the need. This project is justified specifically by the combination of portable graph semantics, a small embedded host, interchangeable certified persistence, and resumable human or agent activities. Do not justify it merely as a smaller Temporal.

## Decisions to freeze before implementing the core

| ID | Decision | Consequence |
| --- | --- | --- |
| A01 | One Rust implementation of graph semantics | TS adapters cannot independently interpret branches or retries |
| A02 | Bounded graph IR is the semantic source | DOT and YAML are frontends, not execution standards |
| A03 | Pure synchronous WIT reducer | No WASI, clock, database, or telemetry exporter imports in the reducer |
| A04 | Atomic event plus state plus command commit | Database transactions are a required capability |
| A05 | Effects are potentially repeated | Idempotency and reconciliation are explicit contracts |
| A06 | Package and run versions are immutable | In-flight changes require migration, never hot replacement |
| A07 | Native storage adapters first | WIT storage plugins are optional extensions with the same tests |
| A08 | Local SQLite first, PostgreSQL for shared workers | No SQLite database over a network filesystem |
| A09 | Agent control is a host integration | Skill text is data; model judgment becomes recorded output |
| A10 | Permissions are enforced at dispatch | A workflow or model cannot grant itself capabilities |
| A11 | Telemetry is separate from audit state | Export outages do not corrupt or block execution |
| A12 | Target certification is explicit | No undifferentiated promise of all browsers and all edge hosts |
