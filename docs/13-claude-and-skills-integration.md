# 13 Claude Code and the skills epic

Design revision 0.1 — 2026-10-05. Proposed architecture; implementation and production qualification are pending. Numbered research references resolve in [Sources](sources.md).

> **Superseded in part (0.2).** This chapter predates the Claude Code map's decision records (`docs/design/`) and the 0.2 spec. The integration is now the `ml-workflow` plugin in the `max-skills` repository and is specified there; `spore` knows nothing of it and offers it only the `spore` CLI and its versioned JSON. Each paragraph the records replaced starts with *Superseded* and names what replaced it. The cautions about mods, permissions and session resume still stand.

Epic 18 separates graph nodes from reusable Action slots and already has typed boundaries, outcome routing, configurable bindings, progressive disclosure, and agent repair budgets. It also deliberately specifies Bun-only, zero runtime dependencies, agent-driven invocation, DOT topology plus bindings, and a narrow schema dialect. Those are constraints of that plugin, not requirements for this general runtime. Keep the projects separate and integrate through a versioned profile. [18] This stands again: the plugin lives in `max-skills` (C10 L1) and checks `spore capabilities --json` for the contract versions it needs.

*Superseded (C07 L1, C13).* The epic describes DAG topology while also allowing bounded retry backedges. A workflow now draws a rework loop as ordinary arrows, bounds it with a node's `loopLimits` and routes the give-up case along its exhausted arrow; no importer normalizes anything. The cmd, skill, agent and human kinds are the integration's words only: a workflow holds plain actions (`command`, `external`) and node kinds. Named outcomes, the failed-edge error shape and explicit mappings carry over. Its Action naming collision with onboarding remains open in issue 22; ActionContract and ActionBinding here are proposed core terms, not a claim that the repository-wide standard is settled. Engine storage bindings stay outside the graph. Resolve ACS through its CLI contract, snapshot non-secret resolved bindings, and retain secret references only.

*Superseded (C03 L1, C06, C10).* The plugin depends on `spore` and drives it through the CLI; there is no bridge, MCP server or agent-driven planner. Work done outside the worker is an `external` action: an actor claims it with `spore step claim`, gets a token, and reports with `spore step complete` or `spore step fail`. Only the token's bearer can report that attempt, and the worker never claims an external step.

## Claude Code mods

Official Claude Code documentation now describes mods as JavaScript or TypeScript hook functions with UI panes, commands, and tool-call interception. They can coexist with skills and MCP servers. Availability varies by surface; notably the documented Desktop WSL session does not load plugins. [20]

Use the mod for workflow progress, ready actions, evidence links, approvals, cancellation, and resumption. *Superseded (C06 L1–L3).* The plugin uses the `spore` CLI for start, signal, status and claims, unauthenticated (see [11](11-security.md)). Nobody owns a run's loop: any driver may move it, and the plugin follows a run by running `spore continue --wait` in the background. Do not persist authoritative state in the mod's variables, small store, or ordinary file writes. Its API and hook time budgets are not a durable worker contract. [21]

Mods and hooks improve interaction, but they are not the sole authorization boundary. Current mod documentation warns that processes they start are outside Claude's Bash sandbox, and mod behavior can affect approvals. Guard hooks should fail closed when policy checks fail, while merge and deployment credentials remain in the external broker. In the SDK, allowedTools is an auto-approval list rather than an exclusive exposure list, and earlier approvals can skip canUseTool. PreToolUse checks complement broker enforcement. [20, 22]

An agent session identifier is a resume hint, not a workflow commit. SDK transcript mirroring is best-effort and can drop batches; session resume does not restore the repository. File checkpoints omit shell and most subagent edits. Keep accepted structured handoffs and immutable Git snapshots as the authoritative record. Require present structured_output and local schema validation rather than trusting exit zero or a success envelope. Schema repair reshapes recorded evidence without repeating effects; any new tool work uses the broker and preserved operation identities. [23, 24, 25]

## Example delivery workflow

*Out of scope.* The Claude Code map left this eight-step flow out. Its check is the reference loop in [examples/coding-loop/](../examples/coding-loop/): implement, run the tests, go back on red at most three times in a row, then a person approves; it opens no PR. The flow below is kept as a picture of a later, larger workflow.

1. Plan: run an agent against a pinned repository base, save a typed plan and evidence references, and optionally require a human decision on scope.
2. Implement: allocate an isolated worktree or sandbox, run the agent with task-scoped credentials, record session ID and resulting patch or commit identity.
3. Test: execute declared commands with argv and environment references. Capture exit code and artifact references. A failing test routes through a bounded rework loop with a new activation.
4. PR: ensure a pull request exists for the deterministic branch and repository identity. Reconcile by stored external identity after any uncertain response.
5. Validate: collect independent review and required CI checks for the exact head SHA. Reject stale evidence after any push or changed merge result.
6. Authorize merge: bind approval or preauthorized policy to repository, PR, head SHA, required checks, target branch, and expiry. Agent workers do not receive merge credentials.
7. Merge: use the provider's conditional operation or merge queue protection, then reconcile actual merged status and commit SHA on uncertainty. If the provider cannot enforce the required precondition atomically, use a protected merge service or stop; a client-side check alone has a race.
8. Deploy and verify: if deployment is external, wait for its authenticated event. Otherwise dispatch a separately authorized deployment action. Test the exact deployment ID and commit using read-only production probes.

A failed production probe routes to notification and an explicit remediation or rollback policy. Never assume a merge can simply be undone transactionally. The validation agent is useful evidence, not authority to override required checks or branch protection. *Superseded (C06 L3, C15 L1):* closing the Claude session that follows a run pauses the run until someone continues it, and a worker that dies has its leftover command stopped by the guard.
