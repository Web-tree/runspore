# Runspore documentation

## User guide

Start with the [Runspore user guide](guide/README.md), also published at
[runspore.com/docs](https://runspore.com/docs). It covers the implemented CLI:
installation, workflow authoring, running and recovering work, external actors,
automation, project settings, and command reference.

- [Getting started](guide/getting-started.md)
- [Authoring workflows](guide/authoring-workflows.md)
- [Running workflows](guide/running-workflows.md)
- [External and asynchronous steps](guide/external-steps.md)
- [Automation](guide/automation.md)
- [Projects and settings](guide/project-settings.md)
- [CLI reference](guide/cli-reference.md)

## Reference and standards

The [implementation specifications](../spec/README.md) define the current Runspore
contracts. The [SWL chapter](https://runspore.com/docs/swl) covers the independent
Spore Workflow Language working draft; its notation is not currently a drop-in
input format for `spore`.

## Architecture and design

Design revision 0.1 — 2026-10-05.

This directory defines the proposed production architecture for Runspore: the target, not what `spore` does today. The documents are a coordinated specification: implementation must satisfy the durability, effect, security, and target conformance requirements together. The runtime has not been qualified for production. What is implemented, semantics 0.2, is specified in [spec/](../spec/README.md), which wins where it is narrower; a paragraph here that 0.2 contradicts says so. The decision records behind 0.2 are in [design/](design/), the vocabulary in [CONTEXT.md](../CONTEXT.md).

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
