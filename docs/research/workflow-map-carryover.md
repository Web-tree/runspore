# Carry-over from the skills-repo workflow map

Answers [C01, runspore#6][c01]: what happens to [Map: workflow, skills#18][m18] now that Runspore takes over its role ([runspore#5][r5]). Read on 2026-10-06. The categories below are evidence for the Runspore tickets named in each note. They decide nothing on [runspore#5][r5].

## 1. Summary

1. The old map has 41 items: 27 locked decisions and 14 tickets. 1 carries over as-is, 17 carry over amended, 13 are already covered by Runspore and 10 are dropped.
2. Locked decisions: 0 as-is, 9 amended, 10 covered, 8 dropped. Tickets: 1 as-is (W01), 8 amended, 3 covered, 2 dropped (W03, W12).
3. Runspore's design contradicts seven locked decisions outright: `.dot` as source of truth (L03, A02), agent-driven execution (L05), a DAG with several ready nodes (L06, M06), "retry is never an engine policy" (L08, M04), "the consumer trusts" (L16), merge-by-key (L19) and one state structure per run (L23).
4. What is still open is the authoring layer: the DOT subset and node files (C02), the four kinds (C07), agent-output schemas and repair (C12), and what the session is told to read (C03).
5. The ACS layer (rebinding, engine bindings, `init`/`upgrade`, config) and the typing lint (width subtyping, fan-in) are dropped, because [runspore#5][r5] puts them out of scope.
6. Recommendation: (a) close skills#18 with a pointer to runspore#5 and this file. Do it as the separate act runspore#5 schedules for after its spec is locked.
7. Not (b): 17 of the 27 locked bullets would need rewriting or deleting, and so would the Bun and zero-dependency hard constraints. Swapping the engine also removes the old map's core decision, agent-driven execution.
8. Under (a), the standalone Action standard (`standards/agent-actions.md`) loses its home. If it is still wanted, file it again on [skills#7][acsv2].

## 2. Item-by-item table

L01 to L27 are the bullets of the "Locked during charting" block of [skills#18][m18], in order. W01 to W14 are its tickets.

| Item | Category | Note |
| --- | --- | --- |
| **L01** [Shape] New sibling plugin `workflow`; `orchestrate` is not touched ([skills#18][m18]) | carries over amended | What is being planned is now Runspore's Claude Code profile: a mod with `spore` doing the work ([runspore#5][r5], Standing preferences and Locked). It is no longer a `workflow` plugin in max-skills. Which repository ships the plugin is open in [C10][c10]. The `orchestrate` half still holds: nothing on runspore#5 touches it. |
| **L02** [Shape] A definition is distinct from a run; one definition, many concurrent runs ([skills#18][m18]) | already covered by Runspore | **Workflow** and **Run** in [CONTEXT.md][ctx]. A run is created from a package by `start` with a start key, and `list` shows every run ([cli Commands][cli-c]). The versioned unit is the canonical JSON package identified by its digest ([kernel §2][k2]), not the `.dot` file. Package and run versions are immutable (A06, [docs/02][adr]). |
| **L03** [Shape] `.dot` is the source of truth for topology and bindings only; contracts and prose live in the node's markdown file; nothing structured goes into a DOT attribute string ([skills#18][m18]) | carries over amended | **Contradicted in part.** A02 makes the bounded graph IR the semantic source and DOT only a frontend ([docs/02][adr]), and runspore#5 locks this. `.dot` is therefore the authoring source, not the source of truth. What runs is the package that one document of graph plus bindings compiles to (M01, [spec README][sr-dec]). The other two rules (prose and contracts in a per-node markdown file, nothing structured in an attribute string) remain candidates for [C02][c02], which names this row. The case for the attribute rule is stronger here than on the old map. Runspore actions carry arrays and maps (`argv`, `env`, `exitOutcomes`, `retry`; [host §3][hcmd]), and W01 found DOT attribute grammar flat, `ID '=' ID` ([skills#19][w01]). |
| **L04** [Shape] Own minimal DOT-subset parser in Bun; Graphviz optional, for rendering only ([skills#18][m18]) | carries over amended | The parser is Rust inside `spore`, not Bun (runspore#5 Locked: "the DOT front end lives in `spore`"). "The `dot` binary is never required" carries over unchanged, and [C02][c02] restates it. docs/06 adds that DOT export is always available ([docs/06][d06]). |
| **L05** [Execution] Agent-driven: scripts are graph math, the agent performs every step and reports back, a script never spawns an agent ([skills#18][m18]) | dropped | **Contradicted.** In Runspore the worker owns the loop. The engine claims attempts and runs `command` actions itself ([host §2][h2], [host §3][hcmd]), and A09 makes agent control a host integration ([docs/02][adr]). Headless steps, where "the worker starts a separate `claude -p`", are locked on runspore#5, and they are exactly what "a script never spawns an agent" forbade. A narrower idea survives and is already locked as the **in-session step** ([CONTEXT.md][ctx]). Who drives the loop between in-session steps is [C06][c06]. |
| **L06** [Execution] DAG topology, per-node status, outcome-labelled edges; an FSM is the degenerate case ([skills#18][m18]) | already covered by Runspore | Graph semantics belong to the kernel (A01, one Rust implementation; [docs/02][adr]). Outcome-labelled edges exist: each node maps outcomes to targets ([kernel §2][k2]). **A DAG with several ready nodes contradicts M06** ("a run is at exactly one node", [spec README][sr-dec]). A Runspore graph has bounded back edges, so it is not a DAG, and 0.1 is the FSM case. Per-node status becomes run status plus the latest result per node ([kernel §3][k3]). Parallel steps are out of scope on [runspore#5][r5]. |
| **L07** [Execution] A node declares outcomes; edges match on them; dispatch is dumb; guards are not a language ([skills#18][m18]) | already covered by Runspore | Kernel validation rules W07 and W08: an action declares its outcomes, and an activity node must route every one of them and may add only `failed` ([kernel §2][k2]). docs/06 keeps JavaScript eval, jq and user functions out of routing ([docs/06][d06]). |
| **L08** [Execution] `failed` is an ordinary outcome; retry is an edge back to the node with a limit, never an engine policy ([skills#18][m18]) | carries over amended | `failed` can be routed: "fail the invocation" enters the node's `failed` target when it has one ([kernel §7.4][k74]). **"Never an engine policy" is contradicted.** Delivery retries are kernel decisions with backoff, on the same invocation and effect key (M04, [spec README][sr-dec]; `retry` in [kernel §2][k2]). Runspore keeps delivery retries in the engine and rework in a visible loop ([docs/06 Retry][d06r]). **0.1 has no per-edge limit.** The bound is the workflow-wide `maxVisitsPerNode`, and exceeding it ends the run as `failed` with `limit.visits-exceeded`, an outcome no edge can route ([kernel §7.4][k74]). So "after N red test runs, go to a person" cannot be drawn in 0.1 unless the compiler unrolls the loop (inferred). [C02][c02] and [C11][c11] must deal with this. |
| **L09** [Execution] An error handler is just a node reached by `[on="failed"]`; `halted` for unhandled failure; unreached nodes `skipped` ([skills#18][m18]) | already covered by Runspore | A handler is a node reached by `failed` ([kernel §7.4][k74]). The names differ: `halted` is run status `failed` with `result.error`, and Runspore has no `skipped`, since only visited nodes have entries ([kernel §3][k3]). "A handler with no outgoing edges ends the run normally" cannot be drawn. Every path ends at an explicit `complete` or `fail` node, and an activity node must route every outcome (W08, [kernel §2][k2]). |
| **L10** [Actions] The reusable abstraction is an Action, not a node; an action reference sits beside ACS's credential reference ([skills#18][m18]) | already covered by Runspore | The workflow document splits nodes from actions: it has an `actions` map, and an activity node names one with `action` ([kernel §2][k2]). docs/08 calls this "an Action binding used by a graph node" ([docs/08][d08]). The ACS half is dropped, because rebinding through ACS is out of scope ([runspore#5][r5]). Gap: [CONTEXT.md][ctx] does not define *Action* yet (inferred to be a `/domain-modeling` item). |
| **L11** [Actions] Four kinds in v1: `cmd`, `skill`, `agent`, `human`; exactly one kind per node ([skills#18][m18]) | carries over amended | docs/13 says to keep the four kinds ([docs/13][d13]). They become profile vocabulary on top of Runspore's own kinds. Its action kinds are `command` and `native` ([host §3][hcmd]), and its node kinds are `activity`, `await-signal`, `complete` and `fail` ([kernel §2][k2]). `human` maps to a node kind, `await-signal`, not to an action kind, so "one kind per node" needs restating. The mapping is decided in [C07][c07]. |
| **L12** [Actions] Named slots with a default binding; a repo or user ACS layer can rebind a name, including its kind ([skills#18][m18]) | dropped | Out of scope on [runspore#5][r5] ("Rebinding actions through configuration layers (ACS)"). It also cuts against M01 and A06: one document holds the graph and the bindings, the package digest covers both, and a package is immutable ([spec README][sr-dec], [docs/02][adr]). If rebinding comes back, it has to be resolved before compilation so it lands in the digest. docs/13 sketches that path: "snapshot non-secret resolved bindings" ([docs/13][d13]). |
| **L13** [Actions] One Action interface for graph steps, engine bindings (state, notification) and lifecycle hooks (`init`, `upgrade`); engine bindings are not nodes ([skills#18][m18]) | dropped | Storage is the Store interface, not an Action ([store.md][st]). `init` and `upgrade` are out of scope ([runspore#5][r5]). A notification is just an ordinary node. The one sub-claim that survives, that engine bindings are not graph nodes, holds by construction: "Engine storage bindings stay outside the graph" ([docs/13][d13]). |
| **L14** [Typing] Every boundary declares a schema, including `agent`; prose only as an extra layer ([skills#18][m18]) | carries over amended | 0.1 has no JSON Schema validation of inputs or outputs ([spec README][sr-out]). In the target design every binding still declares contracts ([docs/08][d08]), and producer output is validated when it is ingested ([docs/06 Types][d06t]). Whether the profile validates an agent step's output in 0.1, and where, is [C12][c12]. A `command` step's output in 0.1 is `{exitCode, stdout}` or parsed JSON ([host §3][hcmd]), so "every boundary" would mean schemas on these too. |
| **L15** [Typing] A narrow JSON Schema subset plus a shorthand; chosen because headless `claude -p --json-schema` can enforce it and the validator is about 143 lines ([skills#18][m18]) | carries over amended | Runspore's target subset is wider. It adds "bounded lengths" and local `$ref` resolved and bundled at compile time ([docs/06 Types][d06t]). Bundling inlines the references, so the schema that reaches a boundary still has none (inferred). Bounded-length keywords are outside Claude Code's strict allowlist, so they fall back to Ajv with retry and are still enforced ([W02 notes][w02doc], forced-violation runs). **New constraint:** canonical JSON allows only integers within ±(2^53 − 1) ([kernel §1][k1]), so `number` cannot mean a float. [C04][c04] already lists non-integer numbers in the `claude -p` envelope. The subset goes to [C12][c12] and the shorthand to [C02][c02]. |
| **L16** [Typing] Lint compares declared to declared before a run; runtime checks actual against declared at the producer only; the consumer trusts ([skills#18][m18]) | dropped | The lint half is out of scope ("type checks on edges between steps", [runspore#5][r5]). **"The consumer trusts" is contradicted.** Runspore validates producer output when it is ingested and the mapped input before dispatch, because compile-time compatibility "does not replace runtime boundary checks" ([docs/06 Types][d06t]). Whether to check at the producer is part of [C12][c12]. |
| **L17** [Typing] Edge compatibility is width subtyping ([skills#18][m18]) | dropped | Edge type-lint is out of scope ([runspore#5][r5]). docs/06 keeps compile-time width compatibility as a later compiler check ([docs/06 Types][d06t]). W02's corollary applies when it returns: strict enforcement forces `additionalProperties: false`, so an enforced agent step cannot pass extra keys through ([skills#20][w02]). |
| **L18** [Typing] No implicit name mapping, no coercion; renames are explicit on the edge ([skills#18][m18]) | already covered by Runspore | Nothing reaches a node unless it is mapped. An activity's `input` is an explicit mapping of `$get` paths and `$literal` values, with no coercion ([kernel §4][k4]). A rename goes in the consumer's input mapping, not on the edge, because Runspore edges carry no data. |
| **L19** [Typing] Fan-in merges inputs by key; a key with two types is a lint error ([skills#18][m18]) | dropped | 0.1 has no fan-in: there is one token (M06), and fork-all is out of scope ([spec README][sr-out]; parallel steps, [runspore#5][r5]). The target design contradicts merge-by-key. A join builds its result from declared mappings, and key collisions are compile errors ([docs/06 Activations][d06a]). That is closer to the named ports W01 found in Dagster and Argo ([skills#19][w01]). |
| **L20** [Typing] A `[on="failed"]` edge carries a standard error shape, not the producer's output schema ([skills#18][m18]) | already covered by Runspore | A routed failure records `output: {"error": ErrorInfo}` with `code`, `message`, `nodeId` and `details` ([kernel §7.4][k74]). That is a standard shape, not the producer's output. Distinguishing the two in lint no longer matters, because there is no type-lint. The missing / invalid / exhausted distinction from W02 and W06 becomes error codes in [C04][c04] and [C12][c12]. Limit and mapping errors end the run and never reach a `failed` edge. |
| **L21** [Repair] Schema repair is engine-internal, agent nodes only, with context intact; budget 2; an exhausted budget becomes `failed` ([skills#18][m18]) | carries over amended | "An exhausted budget becomes `failed`" survives. W02 found that `omp` does the opposite, and the old map's choice is the better signal for a graph ([W02 notes][w02doc]). Two amendments. First, Runspore makes a repair a new bounded activation linked to the kept invalid output, not a retry with identical input ([docs/06 Types][d06t]); a repair "reshapes recorded evidence without repeating effects" ([docs/13][d13]). Second, on a headless step the harness owns the loop (5 attempts), and Runspore sees only a present or null `structured_output` ([skills#20][w02]). Whether a budget exists, and its default, is [C12][c12]. |
| **L22** [Repair] Commands get no repair ([skills#18][m18]) | already covered by Runspore | "Commands receive no model-based repair by default" ([docs/06 Types][d06t]). "By default" leaves an opening the old map had closed, and [C12][c12] should say whether it is ever allowed. |
| **L23** [State] Run state is one machine-readable structure per run, path set through ACS, gitignored; small values inline, large by reference; human view rendered on demand ([skills#18][m18]) | already covered by Runspore | Run state lives in the store: one SQLite file with events and snapshots, chosen with `--db` or `RUNSPORE_DB` ([cli Global options][cli-g], [store §3][st3]). **It contradicts the old decision in three places.** It is one database for all runs, not one structure per run. The path comes from a flag or an environment variable, not ACS. And nothing is stored by reference in 0.1: blobs are out of scope ([spec README][sr-out]), stdout is capped at 64 KiB ([host §3][hcmd]), and records at 256 KiB ([store §3][st3]). The text output of `spore status` and `spore events` is the human view rendered on demand ([cli Commands][cli-c]). Where the database lives in a repository, and whether it is gitignored, is [C08][c08]. |
| **L24** [State] One entry command always runs and reports ready / needs-init / needs-upgrade; `upgrade` is an ordinary Action ([skills#18][m18]) | dropped | `init` and `upgrade` are out of scope ([runspore#5][r5]). What is left is a check that always runs and gives a machine-readable reason, and that fits the mod-to-`spore` version handshake in [C10][c10] (inferred). The store already refuses a newer schema with `schema.too-new` ([store §3][st3]). |
| **L25** [State] Config, run state and the workflow definition carry versions independently ([skills#18][m18]) | already covered by Runspore | Runspore versions more than three things: the workflow `format` and package digest ([kernel §2][k2]), the semantics and codec versions ([kernel §7.1][k71]), the state format, the store schema ([store §3][st3]) and the CLI JSON `runspore.cli/0.1` ([cli JSON output][cli-j]). Package and run versions are immutable (A06), and run migration is out of scope ([spec README][sr-out]). Config has no version, because there is no config layer. The profile will add a version for the DOT dialect and the mod handshake ([C02][c02], [C10][c10]; inferred). |
| **L26** [State] `init` is not special machinery; it is a node ([skills#18][m18]) | dropped | `init` is out of scope ([runspore#5][r5]). |
| **L27** [State] Progressive disclosure in three tiers; the script, not the agent, decides what to read next ([skills#18][m18]) | carries over amended | The skill tiers (SKILL.md, `references/`) are plugin packaging and belong to [C10][c10]. The core rule, that the runtime decides what the agent reads, becomes the payload of the in-session hand-off ("what `spore status --json` must expose so the session knows what to do", [C03][c03]). It also becomes the pinned instruction assets of an agent step ([docs/08][d08]; A09, "Skill text is data", [docs/02][adr]). The per-step instructions file is in [C07][c07]. |
| **W01** [Prior art, closed][w01] | carries over as-is | Its findings are evidence, not decisions, and none of them depends on Bun. Two remain relevant. §5: no surveyed engine uses DOT as its source, and DOT attributes are flat, which supports A02 and the attribute rule ([C02][c02]). §3: Temporal, Step Functions and Airflow document failures caused by retries nobody could see, and Argo had to create a node for each attempt to keep retries inspectable. Both support docs/06's split between delivery retry and rework. §4 and §6 (fan-in, dynamic fan-out) do not matter while parallel steps are out of scope. Its addendum on Claude Code's own workflow runtime (`agent(prompt, {schema})`) is prior art for [C09][c09] and [C12][c12]. |
| **W02** [Agent-boundary typing, closed][w02] | carries over amended | Its facts underpin [C04][c04] and [C12][c12]. A Task-tool subagent enforces nothing. `claude -p --json-schema` enforces, with 5 attempts. A null `structured_output` with exit 0 and `subtype: success` is the only failure signal ([W02 notes][w02doc]). Amendments: the findings are pinned to Claude Code 2.1.269 while runspore#5 targets 2.1.287 or later, so C04 measures again. The 143-line validator is Bun and TypeScript, written because of a zero-dependency rule that does not bind Runspore ([docs/13][d13]), so the Rust side may use a crate (inferred). |
| **W03** [Orchestrate as a workflow][w03] | dropped | The old map had already put migrating `orchestrate` out of scope. W03's pressure points are dynamic fan-out, waves and a non-blocking question protocol. [runspore#5][r5] rules out parallel steps and says the coding loop is all the profile needs. |
| **W04** [The Action standard][w04] | carries over amended | Only the per-kind reference shape carries over, into [C07][c07]. There the workflow document already fixes the fields the kernel reads (`outcomes`, `effect`, `retry`; M02) and leaves the rest to runners ([kernel §2][k2], [host §3][hcmd]). Dropped: the reserved `actions` ACS key and rebinding. Inside Runspore the naming collision goes away, because Runspore's spec owns the word *action*. It returns only if the plugin ships from max-skills ([C10][c10]) beside ACS v2's onboarding actions, which [D2][d2] settled as files found by convention in `actions/` (inferred). |
| **W05** [DOT subset and vocabulary][w05] | carries over amended | It becomes [C02][c02], with three changes. The parser is Rust in `spore`. The vocabulary must spell Runspore's fields (`effect`, `retry`, `exitOutcomes`, signal names, `limits`), not ACS binding names. And the question of writing `.dot` back is half answered: export from the package is always available ([docs/06][d06]). |
| **W06** [I/O contracts][w06] | carries over amended | It is narrowed to the schema subset and the missing / invalid / exhausted distinction, which go to [C12][c12] and [C04][c04]. Compatibility and fan-in conflicts are dropped along with edge type-lint. Renames and the error shape are already in the kernel ([kernel §4][k4], [kernel §7.4][k74]). Its two follow-up comments after W02 are the most reusable text on the old map for C12: the desugaring rules, the `oneOf` citation caveat, and the impossibility of pass-through. |
| **W07** [The node markdown file][w07] | carries over amended | Where node prose and contracts live is [C02][c02]; the fields per kind are [C07][c07]; [C11][c11] writes the node files. The field list changes: `effect` is required on every action (M03, [spec README][sr-dec]), retry is an action field, and outcomes are declared on the action. A disagreement between DOT and the node file is partly settled by kernel rule W08: after compilation, an unrouted outcome is a validation error ([kernel §2][k2]). |
| **W08** [Run state, concurrency, claiming, migration][w08] | already covered by Runspore | Structure: state and events in the store ([kernel §3][k3], [store.md][st]). Identity: the run ID derives from the start key ([host §2][h2]). Concurrency and claiming: `claim_attempt` with fenced leases, and two engines sharing one store is evidence item H1 ([store claim_attempt][st-claim], [host §5][h5]). Migration: A06, out of scope in 0.1. Still open: how a session finds its run ([C08][c08]), who renews a lease while the session works ([C03][c03]), and two sessions or repositories sharing one database ([runspore#5][r5], Not yet specified). |
| **W09** [The CLI contract][w09] | carries over amended | `spore` already has a command surface, exit codes and `--json` ([cli Commands][cli-c]). `lint` is `validate` and `status` is `status`. The central pair, `next` and `advance`, has no equivalent. It becomes the in-session hand-off in [C03][c03], where one candidate is claim, complete and fail through `spore`, with the attempt as a token. `check` becomes the version handshake in [C10][c10], `continue` is [C08][c08], and `init`/`upgrade` are dropped. |
| **W10** [The lint rules][w10] | already covered by Runspore | Most of its structural seed rules are already kernel validation, run by `spore validate`: an outcome with no route, a route on an undeclared outcome, a missing action, an unknown target (W08), and a cycle without a limit, which cannot happen because `maxVisitsPerNode` always applies (W06; it has a default when omitted, `crates/runspore-types/src/model.rs`) ([kernel §2][k2]). The type seeds are dropped with edge type-lint. 0.1 does not check for unreachable nodes or for a graph with no terminal node (inferred from the rule list). A missing node file only becomes possible with C02. These are candidates for the compiler in [C02][c02]. |
| **W11** [The `human` kind][w11] | already covered by Runspore | An `await-signal` node parks the run as `waiting`, and the session may end. `spore signal <runId> <name> --outcome … --data …` resumes it, and `spore run` exits with 3 while the run waits ([kernel §3][k3], [cli Commands][cli-c]). The kernel's own example is the reference loop's approval gate ([kernel §2][k2]; `examples/review-loop.json`). Still open: whether `human` remains a kind ([C07][c07]), how the mod shows an approval ([C08][c08]), and validating the answer's `data` ([C12][c12]; 0.1 has no schema checks). |
| **W12** [Config schema, `init`, `upgrade`][w12] | dropped | ACS config, `init` and `upgrade` are out of scope ([runspore#5][r5]). Only one key has a successor, the state path: where the database lives in a repository is [C08][c08]. |
| **W13** [Progressive disclosure][w13] | carries over amended | The read-list becomes what `spore status --json` exposes for an in-session step ([C03][c03]) plus each step's instructions file ([C07][c07]). How the skill's files are split is a packaging question for [C10][c10]. The failure mode W13 names, an agent reasoning from a file it has not read, still applies to in-session steps (inferred). |
| **W14** [How an `agent` node's schema is enforced][w14] | carries over amended | The either/or no longer exists, because runspore#5 locks both forms. What remains is enforcement for each form ([C12][c12]) and how a null payload maps to an outcome. [C04][c04] asks exactly that: "how `structured_output: null` with exit 0 should map to an outcome". Portability to non-Claude harnesses is dropped, since the target is terminal Claude Code. There is also an option the old map lacked. An in-session step can report through a tool the mod registers with structured input ([mods research §6][mods6]). The docs do not say whether Claude Code enforces that tool's input schema, which is a question for [C05][c05] (inferred). |

**Hard constraints from the old map's Notes.** These are not in the locked block, but they decide hypothesis (b). Zero runtime dependencies and "Bun only" are **dropped**: docs/13 calls them "constraints of that plugin, not requirements for this general runtime" ([docs/13][d13]), and Runspore is Rust on Wasmtime ([spec README][sr]). "The `dot` binary must never be required" **carries over as-is** ([C02][c02]). The ACS config constraint is **dropped** ([runspore#5][r5]). The Precedence note ("decisions on this map outrank every existing implementation") does not transfer. Here the frozen 0.1 spec is normative ([spec README][sr]).

**Two inconsistencies found on the way.**
- [docs/13][d13] still describes the plan that runspore#5 replaced. It says "Keep the projects separate and integrate through a versioned profile", "The original Bun planner can remain agent-driven", and the runtime "should not become an accidental dependency of the zero-dependency plugin". Its first and third paragraphs need revising when the profile spec is written (inferred).
- The old map is out of date. Its Notes point to `plugins/herdr`, `plugins/orchestrate` and `plugins/agent-config`. Those moved on 2026-09-20 to `plugins/ml-subagents/skills/{herdr,orchestrate}` and `plugins/ml-agent-config` (commits [1e4bc106][mv1] and [ad124d68][mv2]; [current tree][subagents]). It also calls [D2][d2] open, but D2 closed on 2026-08-21. The issue body was last updated on 2026-09-11 ([docs/sources.md][src], entry 18).

## 3. What the old map's open tickets become here

Runspore map children, from `gh issue list --repo Web-tree/runspore`: [C01 #6][c01], [C02 #7][c02], [C03 #8][c03], [C04 #9][c04], [C05 #10][c05], [C06 #11][c06], [C07 #12][c07], [C08 #13][c08], [C09 #14][c09], [C10 #15][c10], [C11 #16][c11], [C12 #17][c12].

| Old ticket | Feeds | What it brings |
| --- | --- | --- |
| [W01][w01] (closed) | [C02][c02], [C11][c11], [C12][c12] | Evidence: no surveyed engine uses DOT as its source, retries should be visible, and Claude Code's `agent({schema})` is prior art |
| [W02][w02] (closed) | [C04][c04], [C12][c12] | Enforcement facts, the null-payload trap, and the validator as a reference |
| [W03][w03] | no counterpart | `orchestrate`, fan-out and waves are out of scope on both maps. The coding loop needs none of them |
| [W04][w04] | [C07][c07] | The per-kind field shape. The standard document and the ACS key have no counterpart (see section 4) |
| [W05][w05] | [C02][c02] | The subset posture ("reject, don't ignore"), the treatment of foreign attributes, and round-tripping |
| [W06][w06] | [C12][c12], [C04][c04] | The schema subset, the desugaring rules, and missing / invalid / exhausted |
| [W07][w07] | [C02][c02], [C07][c07], [C11][c11] | Node-file location, the split between frontmatter and body, and the rule for disagreements |
| [W08][w08] | [C08][c08], [C03][c03] | Finding "the run I should continue" and lease renewal. Sharing one database between sessions is listed under "Not yet specified" on [runspore#5][r5] |
| [W09][w09] | [C03][c03], [C08][c08], [C10][c10] | `next`/`advance` become the hand-off, `continue` goes to C08, and `check` becomes the handshake |
| [W10][w10] | [C02][c02] | Checks beyond kernel validation, such as unreachable nodes, a missing terminal node, or a missing node file (inferred) |
| [W11][w11] | [C07][c07], [C08][c08], [C11][c11] | Whether `human` stays a kind, how the approval is shown, and the approval step of the reference loop |
| [W12][w12] | no counterpart | ACS config, `init` and `upgrade` are out of scope. Its only surviving question, the database location, is already in C08 |
| [W13][w13] | [C03][c03], [C07][c07], [C10][c10] | The read-list, the instructions file per step, and the skill's file layout |
| [W14][w14] | [C12][c12], [C04][c04], [C05][c05] | Enforcement for each form, mapping a null payload, and whether a mod tool enforces its input schema |

Three Runspore tickets have no predecessor on the old map: [C05][c05] (the spike mod), [C06][c06] (who drives the loop) and [C09][c09] (authoring by interview). The mod surface is new ground.

## 4. The old map's fate

**Recommendation: (a). Close skills#18 with a pointer to [runspore#5][r5] and to this file.** The pointer comment is the migration note, so no rewrite is needed. The timing follows runspore#5: "Closing or migrating the skills-repo map" is "a separate act after this spec is locked". Until then, a single comment on skills#18 that links runspore#5 would stop anyone working the frontier tickets W04–W14 in parallel (inferred risk; the owner decides).

The reasons, drawn from the table above:

1. **Ownership has already moved.** runspore#5 says Runspore "takes over the role of the skills-repo workflow plugin". Its tickets already ask what 10 of the old map's 12 open tickets asked (section 3).
2. **(b) is a rewrite, not a swap.** The engine is not a detail of the old map. It is the map's main execution decision (L05: agent-driven, "a script never spawns an agent"), and Runspore contradicts it. Of 27 locked bullets, 9 would need amending and 8 deleting. None carries over as-is, and the Bun and zero-dependency hard constraints would go too. What is left would be a new map in a repository that holds none of the specs it changes.
3. **Two planning surfaces for one spec would split decisions.** The old map's Precedence note claims to outrank every implementation. The Runspore spec is normative and frozen ([spec README][sr]). Under (b), one of the two would have to give way on every overlap.
4. **The old map has not been maintained since mid-September**: plugin paths are stale and D2 is listed as open although it is closed. Keeping it alive adds upkeep for a map nobody is updating.

**What (a) loses, and where each piece should go:**
- **The standalone Action standard** (Destination 1, `standards/agent-actions.md`, "usable outside this plugin"). Runspore defines actions in its own spec and does not need a cross-component standard. The only other consumer named was ACS onboarding, and D2 and D3 have since settled onboarding actions their own way ([D2][d2]). If the owner still wants the standard, the right home is the agent-config v2 map ([skills#7][acsv2]), where W04's "one standard, two profiles" question actually belongs. Keeping skills#18 open for it would be the wrong home.
- **Groundwork for features runspore#5 defers**: ACS rebinding, `init`/`upgrade`, edge type-lint, fan-in, and a workflow library. Closing deletes nothing, and the closed issues stay readable as reference for when Runspore takes these up. The W01 findings and the W02 validator live only on branches `research/w01-prior-art` and `research/w02-agent-boundary-typing`. Those branches should be kept or tagged before anyone tidies branches (inferred).
- **Open items with a home here**: evals for the skill, a README and a marketplace entry go with [C10][c10]. How workflows are found partly goes with [C08][c08]. Sharing workflows between repositories has no home yet and should be added to runspore#5's "Not yet specified" list if it matters.

**What (b) would lose:** one planning surface next to the normative spec, and the C-tickets' claim to be the place where decisions are made. Every decision would also need syncing across two repositories. Its one advantage is keeping the planning in the marketplace where the plugin might ship. That depends on [C10][c10], and a pointer from skills#18 keeps the link either way.

**(c) Split** means closing the plugin half and keeping skills#18 open for the Action standard. It would lose little, but it would keep open a map where only the four Actions bullets (L10–L13) bear on the remaining half, and two of those are dropped here. Moving that one question to skills#7 gets the same result without keeping the map open.

## 5. Sources

Skills repository (Max-Levitskiy/skills):
- [Map: workflow, skills#18][m18]: the issue body, its Notes and its "Locked during charting" block.
- [W01, skills#19][w01] with its resolution and addendum comments.
- [W02, skills#20][w02] with its answer and addendum comments, and the [W02 notes][w02doc] on branch `research/w02-agent-boundary-typing`.
- W03 to W13 are issues [#21][w03], [#22][w04], [#23][w05], [#24][w06] (with two follow-up comments after W02), [#25][w07], [#26][w08], [#27][w09], [#28][w10], [#29][w11], [#30][w12] and [#31][w13]. W14 is [#44][w14].
- [Map: agent-config v2, skills#7][acsv2] and [D2, skills#12][d2], closed 2026-08-21.
- [`standards/agent-config.md`](https://github.com/Max-Levitskiy/skills/blob/main/standards/agent-config.md) (ACS v1: three layers, vendoring, `cacheVar`).
- [Current location of `herdr` and `orchestrate`][subagents], moved in commits [1e4bc106][mv1] and [ad124d68][mv2].

Runspore (Web-tree/runspore):
- [Map: Claude Code profile, runspore#5][r5]. Its children C01–C12 are issues #6–#17.
- [CONTEXT.md][ctx] and [claude-code-mods.md][mods6] were not yet committed when this was written. The links resolve once they are.
- Specs: [spec README][sr] ([decisions][sr-dec], [out of scope][sr-out]), [kernel.md][k2], [host.md][h2], [store.md][st], [cli.md][cli-c].
- Docs: [02 ADRs][adr], [06 graph semantics][d06], [08 activities and effects][d08], [13 Claude and skills][d13], [sources][src], `examples/review-loop.json`.

[m18]: https://github.com/Max-Levitskiy/skills/issues/18
[w01]: https://github.com/Max-Levitskiy/skills/issues/19
[w02]: https://github.com/Max-Levitskiy/skills/issues/20
[w02doc]: https://github.com/Max-Levitskiy/skills/blob/research/w02-agent-boundary-typing/docs/research/2026-09-11-w02-agent-boundary-typing.md
[w03]: https://github.com/Max-Levitskiy/skills/issues/21
[w04]: https://github.com/Max-Levitskiy/skills/issues/22
[w05]: https://github.com/Max-Levitskiy/skills/issues/23
[w06]: https://github.com/Max-Levitskiy/skills/issues/24
[w07]: https://github.com/Max-Levitskiy/skills/issues/25
[w08]: https://github.com/Max-Levitskiy/skills/issues/26
[w09]: https://github.com/Max-Levitskiy/skills/issues/27
[w10]: https://github.com/Max-Levitskiy/skills/issues/28
[w11]: https://github.com/Max-Levitskiy/skills/issues/29
[w12]: https://github.com/Max-Levitskiy/skills/issues/30
[w13]: https://github.com/Max-Levitskiy/skills/issues/31
[w14]: https://github.com/Max-Levitskiy/skills/issues/44
[acsv2]: https://github.com/Max-Levitskiy/skills/issues/7
[d2]: https://github.com/Max-Levitskiy/skills/issues/12
[subagents]: https://github.com/Max-Levitskiy/skills/tree/main/plugins/ml-subagents/skills
[mv1]: https://github.com/Max-Levitskiy/skills/commit/1e4bc106
[mv2]: https://github.com/Max-Levitskiy/skills/commit/ad124d68
[r5]: https://github.com/Web-tree/runspore/issues/5
[c01]: https://github.com/Web-tree/runspore/issues/6
[c02]: https://github.com/Web-tree/runspore/issues/7
[c03]: https://github.com/Web-tree/runspore/issues/8
[c04]: https://github.com/Web-tree/runspore/issues/9
[c05]: https://github.com/Web-tree/runspore/issues/10
[c06]: https://github.com/Web-tree/runspore/issues/11
[c07]: https://github.com/Web-tree/runspore/issues/12
[c08]: https://github.com/Web-tree/runspore/issues/13
[c09]: https://github.com/Web-tree/runspore/issues/14
[c10]: https://github.com/Web-tree/runspore/issues/15
[c11]: https://github.com/Web-tree/runspore/issues/16
[c12]: https://github.com/Web-tree/runspore/issues/17
[ctx]: ../../CONTEXT.md
[mods6]: claude-code-mods.md#6-can-a-mod-make-claude-do-work-and-learn-when-it-finished
[sr]: ../../spec/README.md
[sr-dec]: ../../spec/README.md#decisions-taken-at-the-freeze
[sr-out]: ../../spec/README.md#out-of-scope-for-the-mvp
[k1]: ../../spec/kernel.md#1-canonical-data
[k2]: ../../spec/kernel.md#2-workflow-document
[k3]: ../../spec/kernel.md#3-state
[k4]: ../../spec/kernel.md#4-mappings
[k71]: ../../spec/kernel.md#71-request-checks
[k74]: ../../spec/kernel.md#74-procedures
[h2]: ../../spec/host.md#2-engine
[h5]: ../../spec/host.md#5-required-evidence
[hcmd]: ../../spec/host.md#command
[st]: ../../spec/store.md
[st3]: ../../spec/store.md#3-sqlite-adapter
[st-claim]: ../../spec/store.md#claim_attempt
[cli-g]: ../../spec/cli.md#global-options
[cli-c]: ../../spec/cli.md#commands
[cli-j]: ../../spec/cli.md#json-output
[adr]: ../02-research-and-adrs.md#decisions-to-freeze-before-implementing-the-core
[d06]: ../06-graph-semantics.md
[d06a]: ../06-graph-semantics.md#activations-and-identity
[d06r]: ../06-graph-semantics.md#retry-and-cancellation
[d06t]: ../06-graph-semantics.md#types-and-validation
[d08]: ../08-activities-and-effects.md
[d13]: ../13-claude-and-skills-integration.md
[src]: ../sources.md
