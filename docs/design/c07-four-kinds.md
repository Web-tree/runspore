# C07 — The four kinds as Runspore actions

Design record of the grill held from 2026-10-07 to 2026-10-10 for the map ticket **C07 — The four kinds as Runspore actions** (Web-tree/runspore#12, map #5). It settles how the profile's step kinds (`cmd`, `skill`, `agent`, `human`, inherited from the old skills-repo map) become Runspore workflows. The kinds are vocabulary only. Workflow files hold plain Runspore actions, and each step has two halves: the `settings.json` action (the Runspore half: kind, effect, outcomes, retry, async) and the instructions file's frontmatter (the Claude half: `skill`, `model`, `tools`), which only the plugin reads. The grill also moved a decision the map had locked. A Claude step never runs in the person's own session; it runs only in a session the Claude Code integration starts for that step, either a **background step** (`claude -p`) or an **interactive step** (a herdr-managed Claude session). The integration chooses each session's id (`claude --session-id <uuid>`), so a step killed mid-way reruns on its own and continues the same conversation (`--resume`), up to three tries. When a step cannot recover, it waits for the person's decision instead of failing the run. That last rule is a change to Runspore's core (kernel semantics 0.2), and it applies to every step with no `failed` path. `human` is an `await-signal` node for a decision and an `external` step for manual work. A command never has to learn outcome names: the workflow maps its result, the host checks it, and an opt-in `output: "result"` mode lets an implementation name its outcome. The visual is at [c07-four-kinds-visual.html](c07-four-kinds-visual.html): the kind-to-action map, both halves of a step, the per-form lanes, and the kill lane with the "can't recover" fork.

Scope rules that held throughout: Runspore stays generic (the spec never names Claude, agents or AI; Claude-specific behaviour lives in the plugin); decisions locked in C02 and C03 were not relitigated except where Max changed them explicitly, and every such change is listed under [Changes outside C07](#changes-outside-c07); this is a spec, not code.

## Terms

- **Profile kind** — One of `cmd`, `skill`, `agent`, `human`: the plugin's words for a step, used in its authoring interview, docs and menu. Never written in workflow files (L1). *Avoid:* action kind, node kind, step type.
- **Action kind** — The `kind` field of an action in `settings.json`; it tells the host how to run the step: `command`, `external` (C03) or `native`. *Avoid:* binding kind, step kind.
- **Node kind** — The DOT `kind` attribute: `activity` (runs an action), `await-signal` (waits for a signal), `complete`, `fail` (C02). *Avoid:* step kind.
- **Background step** — A Claude step run by `claude -p`, started by the worker through the plugin's wrapper command, with nobody watching live. In Runspore it is a `command`. *Avoid:* headless step (spec word only), subprocess step.
- **Interactive step** — A Claude step run in an interactive Claude session that the integration starts for it in a herdr pane (`claude --session-id <uuid>`), so a person can watch and type into it. In Runspore it is an async `command` (C03). Replaces the old "in-session step" (L2). *Avoid:* in-session step, the person's own session.
- **External step** — An `activity` node bound to an `external` action: the worker never runs it; an actor claims it with `spore step claim` and reports with `spore step complete|fail` (C03). After L2 the profile uses it only for a person's manual work. *Avoid:* manual step (that is one use), agent step.
- **Actor label** — The opaque `actor` name on an external action; `spore step claim --actor <label>` takes only steps with that label (C03 R1). *Avoid:* queue, role, assignee.
- **Signal** — A named message to a run (`spore signal <run> approval --outcome approved`); an `await-signal` node parks the run as `waiting` until it arrives. *Avoid:* event, callback.
- **Effect class** — Required on every action: `pure`, `read-only`, `idempotent` or `unsafe`. When a try is lost, a repeatable class is retried automatically; `unsafe` waits for a person. In the grill: "rerun on its own" versus "ask me first". *Avoid:* side-effect flag.
- **Effect key** — An identity shared by every try (attempt) of one step visit (invocation); commands get it as `RUNSPORE_EFFECT_KEY`, so work can be keyed to it and continued or deduplicated across tries. *Avoid:* attempt id, idempotency token.
- **Instructions file** — A Markdown file a node names with `instructions` in `settings.json` (C02), built into the artifact and handed to the step (delivery is C14). Its frontmatter is the step's Claude half (Q7). *Avoid:* prompt file, node file.
- **Holder handle** — The recorded identity of whoever holds an async step (a PID, a session id, a herdr agent name); its form belongs to the action, and spore stores it without reading it (C03). *Avoid:* job id, token.
- **Probe** — An action's own check of a holder handle, answering alive, dead or unknown; only a certain `dead` frees the step early (C03). Implemented by a command or, later, a component function (Q5). *Avoid:* healthcheck, ping.
- **Watch** — An action's own wait on a holder handle that ends when the job ends and may deliver its result (C03). *Avoid:* await, poll.
- **Result mode** — The opt-in `command` output mode `"output": "result"`: stdout names the outcome, `{"outcome", "output"}` (Q4). *Avoid:* outcome-from-stdout, JSON mode (that is `"output": "json"`).
- **Stuck step** — A step that cannot recover: its tries ran out, or it reported a failure it cannot continue from, and its node has no `failed` path. After L3 it waits for the person's decision. *Avoid:* failed step (the run has not failed), blocked step (herdr's word for a session waiting at a question).

## Why

The map's destination is a profile in which an agent implements, a command runs the tests, a failed test loops back a bounded number of times and a human approves, and which survives killing the session mid-step. C07 asked how the old map's four kinds map onto Runspore's generic action and node kinds, and what each declares: instructions, input, output schema, effect class, timeout, retry. In Max's words over the session:

- On probes (Q5): "We should support both a probe command and an optional probe function exposed through the action implementation's WIT interface and implemented by a WebAssembly component. Both receive the holder handle and return alive, dead, or unknown, with the same Runspore recovery rules. The interface must remain generic; Claude-specific checking belongs to the integration or action implementation."
- On outcome names (Q8): "Ordinary commands should receive their normal input parameters or generic context and return their own results. They should not be required to receive or understand workflow-specific outcome names. A directory-listing program, for example, should run unchanged and return its listing and exit status. The workflow declares the results it expects and how to route them. The host or action implementation consumes the program's output, interprets it through an explicit mapping or wrapper where needed, validates the resulting outcome, and lets the graph choose the next step. Passing Claude a list such as "ok, changes" is not enough to explain what those labels mean; the task instructions and implementation must supply that meaning."
- On crashes (Q12): "claude code session can be "resumable", even if process failed/killed, it has it's session state." Then the answer: "rerun automatically until we can't recover".
- On session ids (Q15): "we should start claude with claude --session-id "UUID", that uuid is generated by the claude code integration."
- On the person's own session (Q17): "not in scope. RunSpore manages it's own sessions, started specifically for a concreate step, not manually started."
- On which sessions (Q19): "claude -p, or herdr managed session only. it's impossible to control sessions otherwise"
- On a stuck step (Q18): "if a step can't recover, only that step waits for your decision; independent steps continue, dependants wait"

## The mapping

| Profile kind | Node kind | Action kind | What the step declares |
| --- | --- | --- | --- |
| `cmd` | `activity` | `command` (sync) | `argv`, `effect` (author's call), `outcomes`; the result maps through `exitOutcomes` or opt-in `output: "result"` (Q4, Q8) |
| `agent`, background | `activity` | `command` (sync) running the plugin's wrapper, which runs `claude -p` | `effect: "idempotent"`, `retry.maxAttempts: 3`, `timeoutMs` 1 h, `output: "result"`, `instructions` on the node; frontmatter `model`, `tools` (Q7, Q12–Q14) |
| `agent`, interactive | `activity` | `command` with `async`: a start command that opens a herdr pane running Claude | `effect: "idempotent"`, `retry.maxAttempts: 3`, `async.timeoutMs` 4 h, `async.probe`, `async.watch`, `instructions`; frontmatter as above (Q12–Q14, Q19–Q21) |
| `skill` | as `agent` (either form) | as `agent` | as `agent`, plus frontmatter `skill: <name>`; the plugin starts the session with that skill as a slash command (Q3) |
| `human`, a decision | `await-signal` | none | `signal`, the outcomes on the node (`approved`, `rejected`), optional `instructions` (Q2) |
| `human`, manual work | `activity` | `external` with a person's actor label | `actor` (e.g. `person`), `effect` (author's call), `async.timeoutMs` 7 days, no probe (Q2, Q13) |

What every kind can also declare comes from the existing package: an `input` mapping (`$get` / `$literal`, kernel §4), `outcomes`, and `retry`. Output schemas are C12, and how instruction and asset bytes reach the step is C14.

## Locked decisions

Each of these passed all three gates: hard to reverse, surprising without context, a real trade-off.

### L1. The four kinds are words, not file contents (Q1 → A)

**Decision.** `cmd`, `skill`, `agent` and `human` never appear in workflow files. `settings.json` holds plain Runspore actions (`command`, `external`) and DOT holds plain node kinds (`activity`, `await-signal`). The plugin's authoring interview (C09), docs and menu use the profile words, and the plugin recognises what a step is from its action: the wrapper or start command it runs, or its actor label. The cost is a few more JSON lines per step, which the authoring interview writes anyway.

**Rejected.**
- *B — plugin shorthand: `"kind": "agent"` in `settings.json`, rewritten by the plugin into Runspore actions before `spore` builds.* Lost because C02 L6 already rejected a friendlier second schema that compiles to the package, and because `spore` could no longer read the plugin's workflow folders directly.
- *C — generic presets in spore: `"preset": "agent"`, filled from a preset file the plugin installs.* Lost because a run would depend on a file outside the workflow folder, which would then need pinning too.

### L2. A Claude step runs only in a session the integration starts for it (Q19, Max's words; Q17)

**Decision.** Every Claude step runs in one of two forms, both started by the Claude Code integration specifically for that step: a **background step** (`claude -p`) or an **interactive step** (a Claude session that herdr manages, in a herdr pane, which a person can watch and type into). The person's own Claude session never does step work; it runs the menu (C08) and nothing else. In Runspore both forms are `command` actions: the background form runs and waits, and the interactive form is an async command whose start command opens the session, prints its holder handle and exits (C03 L4). `external` stays for a person's manual work and for generic actors outside this profile. Max's reason: "it's impossible to control sessions otherwise". Only a session the integration started has an id chosen in advance (Q15), a process the integration can check, and a lifecycle herdr reports on.

This replaces the map's locked bullet "Agent steps come in two forms, both in scope: in-session (the person's own Claude session does the step and reports back) and headless (the worker starts a separate `claude -p`)", and CONTEXT.md's **In-session step**. See [Changes outside C07](#changes-outside-c07).

**Rejected.**
- *B — as L2, but the person's own session may still take a step by hand on request.* Lost to Max's Q17 ruling ("not in scope"): a manually started session cannot be resumed under a known id or reliably checked.
- *C — background only, nobody watches a step live.* Lost because Max keeps herdr-managed interactive sessions.
- *Q17's options (how a retry continues the person's own killed session: tell the person to `claude --resume <id>`, continue it in the background, or start fresh).* All moot: the case does not exist after L2.

### L3. A step that cannot recover waits for the person's decision (Q16 → B, Q18 in Max's words, Q22 → A)

**Decision.** When a step cannot recover, the run pauses at that step and waits for the person's decision instead of ending as failed. A step cannot recover when its automatic tries ran out, or it reported a failure that leaves no tries (a non-retryable failure, or an undeclared outcome). The person answers with `spore resolve`: `--retry` (a new try of the same step visit, so a Claude step resumes its conversation), `--complete <outcome>` (mark it done), or `--fail` (fail it for good). This is the default for **every** step whose node has no `failed` path. A `failed` path the author drew still wins: the failure takes it, exactly as today. Max's scope rule: "only that step waits for your decision; independent steps continue, dependants wait". A run is at exactly one position in semantics 0.1 and parallel steps are out of this map, so today that means the run pauses at that step while other runs carry on. The rule applies as worded once parallel steps exist.

This is a change to the kernel (semantics 0.2), the first in this map: C03 kept the kernel at 0.1. Spelling in [§Spec](#kernel-semantics-02-a-stuck-step-pauses).

**Rejected.**
- *Q16 A — as today: the `failed` path, or the run ends as failed.* Lost because a long run would end on a crash loop or a failed resume, and a failed run cannot be picked up again.
- *Q16 C — the workflow sends the failure to a "what now?" `await-signal` node drawn in the graph.* No core change, but every Claude step grows an extra node, and going back through an edge is a new step visit: it gets a new effect key, so it starts a fresh conversation, and it uses up `maxVisitsPerNode`.
- *Q22 B — opt-in per step, with the plugin switching it on for Claude steps.* Lost because most steps would keep the harsher behaviour, and Max's rule reads as general.
- *Q22 C — every step, even one with a `failed` path, which is taken only when the person chooses `--fail`.* Lost because it ignores failure routes the author drew on purpose.
- *Q18's drafted options (a per-step `retry.whenStuck` setting, A/B; or a rule for steps with no `failed` path, C).* Superseded by Max's text answer and Q22. The coverage of option A survives: both "tries ran out" and "reported it can't continue" count as stuck.

## Routine choices

- **R1. `human` (Q2 → A).** By purpose. A decision (approve or reject) is an `await-signal` node: no work is done, so it needs no claim, no effect class and no deadline, and it is the kernel's own form for it (`examples/review-loop.json`). Manual work by a person is an `external` step with a person's actor label, because it has real effects and a claim keeps two people from both doing it. Rejected: B (always `await-signal`), C (always `external` with actor `person`).
- **R2. `skill` (Q3 → B).** The instructions file's frontmatter names the skill: `skill: tdd`. The plugin starts the step's session with that skill as a slash command, with the prose as context, so the skill loads for certain instead of relying on Claude to pick it up. The name sits in a file only the plugin reads, so `spore` is untouched. Rejected: A (prose only), C (a `$literal` in the node's input, which puts a Claude word into the step's data).
- **R3. Result mode for commands (Q4 → B).** A new generic `command` output mode, `"output": "result"`: stdout is `{"outcome": "<o>", "output": <json>}`, or `{"error": {...}, "retryable": <bool>}` for a failure, the same shapes a watch prints (C03). With exit codes, a crash that exits 1 reads as the outcome mapped to 1; a named outcome cannot be confused with an error. It is opt-in for implementations built for it (Max, Q8). Rejected: A (exit codes only, with the plugin's wrapper turning Claude's answer into a number), C (`outcomeFrom: "/verdict"`, a JSON pointer into each step's own output layout).
- **R4. Where a probe is declared (Q5 → A, rewritten at Max's request).** On the action, tagged by implementation: `"probe": {"command": {"argv": [...]}}` or `"probe": {"component": {"ref": "...", "export": "probe"}}`. Both receive the holder handle and answer alive, dead or unknown, under the same recovery rules. The command contract is specified (C03 §Probe); the component interface is planned, not specified here (see Deferred). The same tag applies to `watch`. The tag makes the component form a new value rather than a new field, at the cost of C03's spelling `"probe": {"argv": …}` becoming `"probe": {"command": {"argv": …}}`. Rejected: B (a host actor registry that pins the resolved probe at `start`), C (both, the action's probe winning over the registry).
- **R5. Who does the work in a step's session (Q6 → A, reread after L2).** Q6 was drafted for the person's own session: main conversation or background subagent. After L2 it reads: the main conversation of the session started for the step does the work, not a subagent it spawns. A separate context is a separate step. Rejected: B (per-step `run: subagent` frontmatter), C (always a subagent; `$.agent.spawn` is early access).
- **R6. A step's Claude settings (Q7 → B).** In the instructions file's frontmatter, next to `skill`: `model`, `tools`, and other Claude-only settings. The instructions file is the step's whole Claude half and the `settings.json` action its whole Runspore half, so switching a step between background and interactive changes only the action. Rejected: A (Claude flags in the action's `argv`), C (split between frontmatter and `argv`).
- **R7. Commands never learn outcome names (Q8 → A, rewritten after Max's clarification).** A command gets its input on stdin and generic context in its environment, and returns its own output and exit status. The workflow maps that result to an outcome (`exitOutcomes`, a wrapper, or R3's opt-in result mode), and the host checks that the outcome is declared before the kernel sees it: an undeclared outcome is a non-retryable failure, `command.undeclared-outcome`. The meaning of each outcome comes from the step's instructions. An external actor (C03 `step claim`) speaks Runspore's protocol on purpose, so its claim still returns the declared outcomes. Rejected: B (opt-in `"passOutcomes": true` setting `RUNSPORE_OUTCOMES`), C (always set `RUNSPORE_OUTCOMES`).
- **R8. Whether a probe's implementation is bundled (Q9 → C).** The reference form decides. A folder path (`./probes/check`, `./probes/check.wasm`) is built into the artifact as an asset owned by the action and pinned by digest. A bare name (`claude-spore`) is looked up on the machine when the probe runs, and the run pins only the reference. A probe is not part of the run's meaning (the kernel only ever sees `expired`), so an integration's probe can be fixed under running runs while a workflow's own probe travels with it. Rejected: A (always looked up), B (always bundled).
- **R9. The holder handle of a detached background step (Q10 → A).** `{"pid": …, "started": "<process start time>", "session": "<session id>"}`; the probe answers `dead` when no process has that PID and start time. Drafted for the person's own session; after L2 it applies to a background step run as an async (detached) `command`. Interactive steps use herdr's own identity instead (Q20, deferred). Rejected: B (session id only, judged from file activity, which can never answer `dead` safely), C (leave the format to the plugin).
- **R10. Checking frontmatter (Q11 → B).** The plugin's check command (`claude-spore check <folder>`, name provisional, C10) validates the frontmatter keys before a run starts: the authoring interview and the menu's "start a run" both run it. The step also checks them when it starts, as a backstop. This keeps C03's rule that a missing capability fails at start with a fix, without teaching `spore` what frontmatter is. Rejected: A (only when the step starts, possibly an hour into the run), C (a generic build hook in `spore` for one profile's need).
- **R11. After a crash, Claude steps rerun on their own (Q12, Max's words: "rerun automatically until we can't recover").** The authoring interview declares Claude steps `idempotent`, so a lost try (session killed, deadline passed) gets a new try automatically. Each new try continues the same conversation (R14), and the working tree keeps the edits already made. Steps whose effects leave the machine (push, deploy, messages) stay a question for the author. This changes C03 L2's example: C03 used the reference `implement` step as an `unsafe` step that asks the person after a kill. C03's rule itself (the effect class decides) stands. Drafted and rejected: A (ask per step, suggesting `unsafe` for steps that change code), B (always `unsafe`), C (derived from `tools`).
- **R12. Time limits per kind (Q13 → B).** The authoring interview writes a default per kind: interactive 4 h (`async.timeoutMs` `"14400000"`), background 1 h (`timeoutMs` `"3600000"`), a person's manual work 7 days (`async.timeoutMs` `"604800000"`). Every try stays bounded (C03 R4), with a sensible bound per kind. Rejected: A (spore's 1 h default everywhere, which expires a person's afternoon of work), C (no limit for manual work).
- **R13. Three tries (Q14 → A).** Claude steps get `"retry": {"maxAttempts": 3}`. Each rerun continues the old conversation. If the plugin cannot resume it, the try reports a non-retryable failure at once, and L3 applies. A step killed three times in a row is probably stuck on something a rerun will not fix, and the cap stops a loop that burns tokens. A failing test is not a retry: it is the `red` outcome and the bounded loop (C13). Rejected: B (effectively unlimited), C (ask per step).
- **R14. Session ids (Q15, Max's words).** The integration generates a UUID and starts Claude with `claude --session-id <uuid>` (background: `claude -p --session-id <uuid>`; interactive: the herdr pane's Claude). A retry runs `claude --resume <uuid>` (`claude -p --resume <uuid>`). How the UUID is generated is my default, not ruled on (see Open threads): a name-based UUID made from the step's effect key (`RUNSPORE_EFFECT_KEY`), so every try of one step visit computes the same UUID and nothing needs storing. Drafted and superseded: A (`step claim` returns `effectKey` and the plugin keeps an effect key → session map), B (`step claim` returns the previous holder handle), C (no resume).
- **R15. How an interactive step reports (Q21 → B; my recommendation was A).** Through a watch: it waits until herdr shows the session quiet **and** a result file exists, then returns that file's result. Quiet with no file means the session is waiting for the person, so the watch keeps waiting. The step's instructions tell Claude to write the file as its last act, at a path named after the step's session id. A crash between the file being written and the watch reading it is safe: the next watch reads the same file. Rejected: A (Claude reports itself with `spore step complete` through a plugin tool; recommended because it works without a running worker), C (both, the first to arrive counting).
- **R16. Background steps are sync by default (my default, stated in the grill's closing note and not objected to).** A background step is a plain `command` the worker runs and waits on. If the worker dies, the try's lease runs out, the try ends as `expired`, and R11 reruns it, resuming the session. A detached async background step (R9's handle) remains possible for runs that must outlive the worker.

## Spec

Normative wording to carry into `spec/kernel.md` 0.2, `spec/host.md` 0.2, and the profile spec (`spec/claude-code.md` or the plugin's own docs; the split is fog on the map). Examples are illustrative, not the reference loop (C11). Command names `claude-spore run|start|check|probe|watch` are provisional (C10).

### `settings.json`: the Runspore half

```json
"actions": {
  "test": {
    "kind": "command", "argv": ["cargo", "test"], "effect": "read-only",
    "outcomes": ["ok", "red"], "exitOutcomes": { "101": "red" }
  },
  "review": {
    "kind": "command", "argv": ["claude-spore", "run"], "output": "result",
    "effect": "idempotent", "outcomes": ["ok", "changes"],
    "retry": { "maxAttempts": 3 }, "timeoutMs": "3600000"
  },
  "implement": {
    "kind": "command", "argv": ["claude-spore", "start"],
    "effect": "idempotent", "outcomes": ["ok"], "retry": { "maxAttempts": 3 },
    "async": {
      "timeoutMs": "14400000",
      "probe": { "command": { "argv": ["claude-spore", "probe"] } },
      "watch": { "command": { "argv": ["claude-spore", "watch"] } }
    }
  },
  "deploy-check": {
    "kind": "external", "actor": "person", "effect": "unsafe", "outcomes": ["ok"],
    "async": { "timeoutMs": "604800000" }
  }
},
"nodes": {
  "review":    { "action": "review",    "instructions": "instructions/review.md" },
  "implement": { "action": "implement", "instructions": "instructions/implement.md",
                 "input": { "failures": { "$get": ["nodes", "test", "output", "stdout"] } } },
  "approve":   { "signal": "approval", "instructions": "instructions/approve.md" }
}
```

`approve` is an `await-signal` node in the DOT (`approve [kind="await-signal"]`, outcomes `approved`, `rejected` on its edges). `test` is `cmd`, `review` a background `agent`, `implement` an interactive `agent`, `deploy-check` a person's manual work.

### Instructions frontmatter: the Claude half

```markdown
---
skill: tdd
model: opus
tools: [Read, Edit, Bash]
---
Fix the failing tests listed in the input. Do not change the tests themselves.
Outcomes: `ok` when every test you were given passes locally.
```

Only the plugin reads the frontmatter; `spore` carries the file as bytes (C02 R4, delivery C14). The key set is closed. An unknown key is an error from `claude-spore check` (R10) and from the step when it starts. Keys named in the grill: `skill`, `model`, `tools`; the final list (permission mode and others) is settled with C09/C10. The prose explains what each declared outcome means (R7).

### Host 0.2: `command` result mode and outcome check

- `"output": "result"`: on exit 0, stdout must be canonical JSON, either `{"outcome": "<o>", "output": <json>}` (runner output `Success`) or `{"error": {"code", "message"}, "retryable": <bool>}` (runner output `Failure`). Any other stdout on exit 0 is `Failure`, `command.malformed-output`, not retryable. A non-zero exit maps through the existing `command` table (host 0.1 §3), so `exitOutcomes` still applies. (Exact interplay with `exitOutcomes` is my default.)
- For every `command`, in any output mode: a success outcome the action does not declare becomes `Failure`, `command.undeclared-outcome` (details `{"outcome"}`), not retryable, before it reaches the kernel. This mirrors C03 R9 for `step complete`.
- No outcome names are passed to commands (R7).

### Host 0.2: tagged probe and watch references, and bundling

- `async.probe` and `async.watch` are each `{"command": {argv, cwd?, env?, timeoutMs?}}` (C03's contract, unchanged inside the tag) or `{"component": {"ref": "<reference>", "export": "<name>"}}` (planned; validation rejects it until a host advertises component probes in its capability set, C03 L4).
- Reference forms (R8): an `argv[0]` or component `ref` that starts with `./` names a file in the workflow folder. The build adds it to the artifact as an `asset` owned by the action (C02 R4/R9 rules, so a path escaping the folder is a build error), and it is pinned. A bare name is resolved on the host when the probe or watch runs.

### Kernel semantics 0.2: a stuck step pauses

Changes to spec/kernel.md §7.3–7.4. Everything else stays as in 0.1:

- **Fail the invocation(error)**: if the node routes `failed`, as 0.1. Otherwise, instead of terminal `failed`: set `inv.state = stuck`, keep the invocation and its error, status `needs-intervention`, and emit diagnostic `invocation.stuck` (details `{"invocationId", "attempt", "error"}`). This covers a non-retryable failure, retryable failures past `maxAttempts`, `activity.attempts-exhausted`, and `activity.undeclared-outcome`.
- **`invocation.resolved`** accepts an invocation that is `unknown` (0.1) or `stuck` (new). `retry`: a new try with no delay, regardless of `maxAttempts`, same invocation and effect key (so a Claude step resumes its conversation, R14). `complete`: as 0.1. `fail`: for a `stuck` invocation, terminal `failed` with the kept error, so the person's "fail" is final and never parks again. For an `unknown` one, as 0.1.
- Limit and mapping errors (`limit.visits-exceeded`, `limit.activations-exceeded`, mapping failures) stay terminal: they are not a step failing.
- `spore run` returns exit code 4 ("the run needs intervention", cli.md) at a stuck step, as for an unsafe unknown one. `status --json` shows the kept error.
- New golden traces: stuck after a non-retryable failure, after exhausted retries, after exhausted expiries of a repeatable step; resolve `retry`, `complete` and `fail` from stuck; a `failed` route still taken when drawn.

### Plugin side: background step (`claude-spore run`)

1. Reads the step's input (stdin), instructions and frontmatter (delivery C14), and `RUNSPORE_EFFECT_KEY`.
2. Computes the session UUID (R14). If no session with that id exists, it runs `claude -p --session-id <uuid>` with the frontmatter's model and tools, and with the skill as a slash command when `skill` is set. Otherwise it runs `claude -p --resume <uuid>`, telling Claude to finish the step.
3. Prints a result-mode JSON (R3): the outcome Claude chose (checked against the instructions' outcome list by the host, R7), and its output. If it cannot resume, it prints a non-retryable error, and L3 applies.

### Plugin side: interactive step (`claude-spore start`, `probe`, `watch`)

1. **Start** (async start command, C03): opens a herdr pane and starts Claude in it with `--session-id <uuid>` (first try) or `--resume <uuid>` (later tries), seeded with the instructions (and `/skill` when set). It prints the holder handle and exits 0. The handle's content is deferred (Q20); the proposal is `{herdr agent name, session uuid, machine}`.
2. **Probe**: answers `dead` only when certain the session is gone (proposal: the herdr agent name no longer exists); herdr unreachable is `unknown`, so the deadline decides (C03 L3).
3. **Watch** (R15): waits until herdr reports the session ready for input (quiet) and the result file `<runtime dir>/<session uuid>.result.json` exists, then prints its contents in the watch contract (C03). Quiet without the file means it keeps waiting.

### Kill and resume, step by step (an interactive `implement`)

1. The worker reaches `implement` and runs `claude-spore start`. Claude opens in a herdr pane with session id U (derived from the effect key). The start command prints the handle and exits, and the worker's watch waits.
2. The session or its machine is killed mid-step. The watch loses sight of the job (the attempt stays held). The next probe answers `dead`, or the 4 h deadline passes, and the try ends as `expired`.
3. `implement` is `idempotent` with `maxAttempts: 3`, so the kernel schedules try 2 of the same invocation (same effect key).
4. The start command runs again: session U exists, so it opens `claude --resume U` in a new pane. The conversation continues on the tree as it was left.
5. Claude finishes and writes the result file. The watch returns `ok` and the run moves on to `test`.
6. If try 3 is also lost, or the session cannot be resumed, the step is stuck (L3): the run pauses with exit code 4 and the person chooses `resolve --retry` (try 4, resuming U again), `--complete ok`, or `--fail`.

A background `review` follows the same path. The worker running `claude -p` dies, the lease expires, and try 2 runs `claude -p --resume U`.

## Verified facts

Established by exploring the repo and tools, not by asking:

- The kernel reads only `outcomes` (default `["ok"]`), `effect` (required) and `retry` (default one attempt) from an action (spec/kernel.md §2, line 60). `maxAttempts` is 1..=100 (W07).
- Kernel 0.1 result rules (spec/kernel.md §7.3): `success` with an undeclared outcome fails the invocation with `activity.undeclared-outcome`. `failure` retries when `retryable` and `attempt < maxAttempts`, and otherwise fails the invocation. `unknown`/`expired` retries a repeatable effect up to `maxAttempts`, then fails the invocation with `activity.attempts-exhausted`; an `unsafe` one parks in `needs-intervention`. `resolve retry` retries "with no delay and regardless of `maxAttempts`".
- *Fail the invocation* enters the node's `failed` target when it routes one, and otherwise ends the run as terminal `failed` (spec/kernel.md §7.4, line 272). A failed run is terminal; `release` lifts only a quarantine.
- CLI exit codes: 3 waiting for a signal, 4 needs intervention, 1 failed (spec/cli.md).
- `command` runner (spec/host.md §3): any exit code outside 0 and `exitOutcomes` is `Failure`, `command.exit`, **retryable**; output modes `text`, `json`, `none`; the effect key `RUNSPORE_EFFECT_KEY` is identical on every attempt of one invocation.
- C02's settings example already puts `instructions` on an `await-signal` node (`"review": {"signal": "review", "instructions": "instructions/review.md"}`).
- In the old skills-repo map (skills#18), `skill` meant the agent "invokes the skill" itself and `agent` meant it "spawns the subagent"; the four kinds were "exactly one kind per node" (L11). docs/13 asked to preserve them; docs/08 lists "Agent or skill: agent runner plus pinned instruction assets" and "Human: durable request and authenticated response".
- herdr (read from `herdr --skill` on 2026-10-10): agent names match `[a-z][a-z0-9_-]{0,31}`, are unique among live agents, and are "cleared when that agent exits, is released, or is replaced". Pane ids (`w1:p1`) are opaque, never reused, and change when a pane is moved to another workspace. Lifecycle states are `idle`, `working`, `blocked`, `done`, `unknown`; "`idle` and `done` both mean the agent is ready for input", and `unknown` "does not prove completion". `agent start --pane <p> -- <agent-args>` passes arguments to the agent. `agent restart` closes and reopens a session with `--resume`. `--machine` drives saved SSH machines, and ids are scoped to one server.
- Mods (docs/research/claude-code-mods.md): `$.agent.spawn` is early access (d.ts from 2.1.277); whether `$.prompt.submit` and `$.agent.spawn` work under `claude -p` is undocumented.

## Changes outside C07

Max's answers in this grill change decisions recorded elsewhere. Each must be carried over when the spec lands:

- **Map #5, "Locked during charting".** The bullet "Agent steps come in two forms, both in scope: in-session (the person's own Claude session does the step and reports back) and headless (the worker starts a separate `claude -p`)" becomes: two forms, both started by the integration for the step: interactive (a herdr-managed Claude session) and background (`claude -p`). The person's own session runs the menu only (L2).
- **CONTEXT.md.** **In-session step** is replaced by **Interactive step** (above); **Headless step** becomes **Background step** (keeping "headless" out of the profile's words); **Agent step** stays.
- **C03 (docs/design/c03-in-session-hand-off.md).** (1) Its reference `implement` is `unsafe` (L2 example and the kill-and-resume walkthrough); Claude steps are now `idempotent` and resume (R11, R14), while the effect-class rule itself stands. (2) `async.probe` / `async.watch` take the tagged form (R4). (3) The `external` claim flow (`step claim --actor coding-session --holder …`) no longer serves Claude steps; it serves a person's manual work and generic actors. (4) Its open thread "the plugin's holder handle and probe for an in-session step" is answered by R9 (background) and deferred Q20 (interactive).
- **Kernel.** Semantics 0.2 for L3; C03 had kept 0.1.
- **Tickets.** C05 (spike mod): measuring whether a mod's process outlives the person's session matters less; what matters now is `claude --session-id` / `--resume` behaviour in `-p` and in herdr panes. C06 (who owns the loop): R15's watch needs a running worker for interactive steps. C08 (menu): no "claim this step here" in the person's session. C11 (reference loop): use this mapping. C12 (output schemas): an interactive step's output arrives in the result file, a background one's through `claude -p` (where `--json-schema` enforces). C14 (delivery): the wrapper and start commands need the instructions file and frontmatter at dispatch.

## Risks

- **Automatic reruns act without a person.** An `idempotent` Claude step reruns after a kill with nobody asked (R11). Resume restores the conversation, not the files, and the transcript can miss its last batch, so a resumed Claude may repeat its last tool call. Three tries (R13) and the stuck pause (L3) bound it; steps with effects beyond the machine must stay `unsafe`.
- **A wrong "dead" puts two sessions on one conversation.** If a probe says `dead` while the session still runs, the next try resumes the same session id beside the live one. herdr also clears an agent's name on a manual release (Q20, deferred), so a name check alone is not certain.
- **`--session-id` on an existing session.** The wrapper must tell "first try" from "rerun" by whether session U exists. What `claude --session-id` does with an existing id, and whether `-p --resume` honours it, is unmeasured (C04/C05).
- **Interactive steps depend on herdr.** No herdr, no interactive steps. The handshake (C03 L4) and `claude-spore check` must report it at start. herdr's command output becomes a contract the probe and watch rely on.
- **Watches need a running worker.** An interactive step's result arrives only through a watch (R15), which runs only while a worker runs (C03). Who runs it is C06.
- **Quiet but unfinished.** If Claude never writes the result file, the step sits until the 4 h deadline, then reruns and resumes.
- **L3 changes every workflow.** A run whose step fails with no `failed` path now waits (exit 4) instead of failing (exit 1). Scripts and existing golden traces change.
- **Frontmatter is invisible to `spore`.** A run started with plain `spore start` (not from the menu) meets a bad key only when the step starts (R10).
- **A probe looked up by name can change mid-run** (R8). A buggy integration update that says `dead` wrongly hits running runs too.
- **A loop back is a new conversation.** Going back through an edge (test `red` → `implement`) is a new step visit with a new effect key, so its session starts fresh. Context from the previous visit reaches it only through the input mapping (e.g. the failing tests).

## Deferred

- **Q20 — how Runspore checks on an interactive step.** Max deferred it after asking me to research herdr. Proposal on record: the start command saves the herdr agent name (made from the step), the session UUID and the machine; the probe asks herdr whether that name still exists (gone = dead, herdr unreachable = unknown). The name beats the pane id because a moved pane gets a new id. It reopens when the interactive start command is built, or when C05 measures herdr-managed Claude sessions. The risk above (a manual release clears the name) must be answered first.
- **The component probe interface.** A WIT interface through which a WebAssembly component exports `probe` (and later `watch`, `start`), taking the holder handle and returning alive/dead/unknown (Max, Q5). R4 fixes the reference shape now; the interface itself arrives with the action kind that runs WebAssembly modules, which is out of this map's scope (map #5, C03 Deferred). A sandboxed component can check a holder only through imports the host grants it.

## Open threads

- **How the session UUID is generated.** Max: "generated by the claude code integration". The derivation from the effect key (R14) is my default. Its alternative, a random UUID stored by the plugin, needs a store that survives the crash.
- **Background steps sync or detached.** R16 is my default; detaching serves runs that must outlive the worker and brings R9's handle and probe into play.
- **Labels and names.** The actor label for a person (`person`), the plugin's command names (`claude-spore …`) and the result file path are provisional (C10).
- **The frontmatter key list** beyond `skill`, `model`, `tools`: settled with C09/C10.
- **Result mode and `exitOutcomes` together** (§Host 0.2): my default is that a non-zero exit still maps through `exitOutcomes`.
- **Q18 as literally worded** ("independent steps continue, dependants wait") applies once parallel steps exist; this map keeps them out of scope.
- **Process notes for later grills** (from Max, saved as feedback): write questions in plain language from what happens to the person, and research tool details (herdr) myself instead of asking.
