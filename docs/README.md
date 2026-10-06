# Architecture documentation

Design revision 0.1 — 2026-10-05.

This directory defines the proposed production architecture for Runspore. The documents are a coordinated specification: implementation must satisfy the durability, effect, security, and target conformance requirements together. The runtime has not yet been implemented or qualified for production.

Start with scope and decisions, then components, kernel, graph semantics, and durable execution. Sources describe upstream behavior; design choices are project recommendations.

- [1 Decision and scope](01-scope-and-decisions.md)
- [2 Research and build versus adopt](02-research-and-adrs.md)
- [3 Components and ownership](03-components.md)
- [4 Artifact and portability model](04-artifacts-and-portability.md)
- [5 Deterministic kernel contract](05-kernel-contract.md)
- [6 Workflow Graph semantics](06-graph-semantics.md)
- [7 Durable execution protocol](07-durable-execution.md)
- [8 Effects and non deterministic activities](08-activities-and-effects.md)
- [9 Replaceable storage and WIT](09-storage.md)
- [10 Time signals ordering and retention](10-time-signals-and-retention.md)
- [11 Security model](11-security.md)
- [12 OpenTelemetry and operational visibility](12-observability.md)
- [13 Claude Code and the skills epic](13-claude-and-skills-integration.md)
- [14 Public API and operations](14-api-and-operations.md)
- [15 Efficiency and feasibility gates](15-efficiency-and-feasibility.md)
- [16 Conformance and production qualification](16-conformance-and-production-gates.md)
- [17 Delivery plan and work packages](17-delivery-plan.md)
- [18 Final design judgment](18-design-judgment.md)
- [Sources](sources.md)

## Starter contracts

Review [contract seeds](../contracts/README.md), [store protocol](../contracts/store.ts), [conformance scenarios](../contracts/conformance-cases.json), and [validation status](../contracts/VALIDATION.txt) alongside the architecture. These are draft interfaces and scenario definitions, not implemented tests.
