# C03 — The in-session hand-off

Design record of the grill held on 2026-10-07 for the map ticket **C03 — The in-session hand-off** (Web-tree/runspore#8, map #5). It settles how a run hands a step to an actor outside the worker and takes the outcome back. An **external step** is an `activity` node bound to a new action kind, `external`, that the worker never executes: an **actor** claims its attempt through `spore`, does the work, and reports with `spore step complete` or `spore step fail`. The grill widened the question into a general model of **async actions**: an action whose result does not come back to the worker that dispatched it, but later through a **result channel**. An async action is either `external` (nothing to start; the actor claims it) or a `command` with an `async` object (the worker runs a short start command that prints a **holder handle** and exits). Spore stores the handle without reading it and asks the action's own **probe** whether the holder is still alive; an optional **watch** lets a running worker wait for the job instead of polling, and can deliver the result itself. A hard deadline always bounds the attempt and an opt-in lease bounds holders that renew; when the holder is gone, the attempt ends as `expired` and the action's effect class decides, exactly as kernel 0.1 already does. The kernel stays at semantics 0.1; the changes land in host, store and CLI 0.2. No visual was drawn.

Scope rules that held throughout: Runspore stays generic (the spec never names Claude, agents or AI; the Claude session is one actor among others); decisions locked on the map and in C02 were not relitigated; this is a spec, not code.

## Terms

- **Worker** — The `spore` process that moves runs forward: it asks the kernel for the next step, commits the decision, and executes the actions it can run itself. *Avoid:* engine (the library inside it), runner (that executes one action kind), daemon.
- **Actor** — Whoever does an external step's work and reports it to `spore` (claims, then completes or fails it): a person, a script, a Claude session. *Avoid:* agent (one kind of actor), client, user.
- **External step** — A step whose work is done by an actor outside the worker and reported back to the run; an `activity` node bound to an `external` action. *Avoid:* manual step, agent step (one use of it).
- **In-session step** — An external step done by the Claude session the person is working in. Profile vocabulary; the spec never uses it. *Avoid:* controlled mode, agent-driven step.
- **Async action** — An action whose result is not returned to the worker that dispatched it but reported later through the result channel. Every other action is sync: the worker runs it and waits. *Avoid:* background step, detached step.
- **Result channel** — The way an async action's result reaches the run: `spore step complete|fail` writing to the store, or a watch's output. Message queues may be added later. *Avoid:* callback, webhook.
- **Holder** — Whoever an open attempt is waiting on: the worker for a sync action, the claiming actor or the started process for an async one. *Avoid:* owner (the store's lease field), assignee.
- **Holder handle** — The serialisable identity of an async attempt's holder (a PID, a session id, a pod name); its form belongs to the action's implementation, and spore stores it without reading it. *Avoid:* job id, token (the claim's credential).
- **Probe** — An optional operation of an async action's implementation that checks a holder handle and answers alive, dead or unknown. *Avoid:* healthcheck, ping.
- **Watch** — An optional operation of an async action's implementation that blocks until the job behind a holder handle ends, and may return its result. *Avoid:* await, poll.
- **Actor label** — An optional opaque name on an external action saying which kind of actor should claim it; spore only matches it against `step claim --actor`. *Avoid:* queue, assignee, role.
- **Token** — The credential `step claim` returns: the attempt's identity, owner and fence. Only its bearer can renew, complete or fail that attempt. *Avoid:* handle (that is the holder's identity), ticket.

## Why

The map's destination requires a run that survives killing the session mid-step and resuming in a new one, with in-session agent steps done by the person's own Claude session. Spore 0.1 runs only `command` and native actions, inside the worker, under 10-second leases the worker renews; it has no way to give a step to someone else. In Max's words over the session:

- "the state should be saved after each node execution. so when we start workflow again, it resumes from the same state. moreover, I wound say, that we need to run the node, and we can exit from the process until we got a result."
- "do we need some kind of "healthcheck" for our nodes wit api? for example, if it's cli process, it can be PID, if it's AI session in herdr, we can store the session id. Like, if node implement the healthcheck, then it returns the serializable id object (for example, if it contain IP, port, k8s pod where it runs, some id, etc for each run). Then it should be able to check health by this id. runspore just save the id, and call healthcheck method, when needed. where it's not implemented, we can use timeout, if no results."
- "I think that our nodes should have the ability to run synchronously or asynchronously. so, for example, if our node is synchronous, it means it can be called quickly and return the result immediately and it fails or succeeds. and if not, if it's asynchronous, we'll need to think about the communication way, how we'll find the result. for example, for the very beginning, we can tell that the process will record the result into the SQLite database, but for the future, we can think about some queues like NATS or something like this."
- "it should be kind of handshake and agreement between the node code and the runSpore, because for example, if some node doesn't support some concrete output, it means that we need to configure it on the runSpore to make it supported."
- "runSpore calls the node's implemented endpoint to check the status of it. the implementation and the form of serializable ID is the responsibility of the node itself, so the spore doesn't know anything about that."
- "another thing that probably we should have in the API is an optional implementation of a method like `watch`/`await` or something like this, so if we have the spore running, we will be able to wait when the process will finish in job to avoid calling it within some loop or something"

Max asked during Q1 what an external step is (not a sub-workflow; one node whose work an outside actor does), where the term came from (the map's Notes, `CONTEXT.md`, this ticket), and what the worker is. The answers are in Terms.

## Locked decisions

Each of these passed all three gates: hard to reverse, surprising without context, a real trade-off.

### L1. An external step is an `external` action kind on an `activity` node (Q1 → A)

**Decision.** A new action kind, `external`, bound to an ordinary `activity` node. The kernel schedules its invocation as for any activity; the worker never claims or executes it. An actor claims the attempt through `spore step claim`, receives a token, and ends it with `spore step complete` or `spore step fail`. Because the kernel reads only `outcomes`, `effect` and `retry` from an action (M02), the kernel, its golden traces and its certified component are unchanged. Attempts, fencing, effect classes, retry and `needs-intervention` all apply to external steps as they are.

In DOT and settings (C02's format), the node is named for the work, not for who does it:

```dot
plan -> implement -> test;
test -> implement [on="red"];
```

```json
"actions": { "implement": { "kind": "external", "actor": "coding-session", "effect": "unsafe", "outcomes": ["ok"] } },
"nodes":   { "implement": { "action": "implement", "instructions": "instructions/implement.md" } }
```

**Rejected.**
- *B — `await-signal` plus an input mapping; the actor answers with `spore signal`.* Lost because a signal wait has no claim, so two sessions can both take the step, and no effect class, so a step killed midway cannot be told from one never started. It would also need a kernel change (an input mapping on a signal node), where A needs none.

### L2. When the holder is gone, the effect class decides (Q2 → A)

**Decision.** When an async attempt's holder is gone (its probe says dead, its deadline or lease ran out, or a person abandoned it), the attempt ends as `expired` and kernel 0.1's existing rule applies: a repeatable effect (`pure`, `read-only`, `idempotent`) gets a new attempt of the same invocation automatically, up to `maxAttempts`; an `unsafe` effect parks the run in `needs-intervention`, and a person answers with `spore resolve` (`--retry` redoes the step, `--complete <outcome>` marks it done, `--fail` fails it). Finished steps are never rerun. The reference loop's `implement` step is declared `unsafe`: an agent's edits to the working tree are real effects and not provably idempotent, so after a kill the person is asked.

**Rejected.**
- *B — always ask a person, whatever the effect class.* Lost because it ignores steps that are safe to repeat, such as a read-only review.
- *C — always start a new attempt.* Lost because it silently reruns unsafe steps.

### L3. Liveness is the action's own probe of an opaque holder handle (Q3 → D)

**Decision.** An async attempt records a holder handle: from the claim (`external`) or from the start command's stdout (async `command`). Spore stores it as opaque canonical JSON and never interprets it; its form and its check belong to the action's implementation. When spore needs to know whether the holder is still there, it calls the action's `probe` with the handle. A certain `dead` frees the step at once (the attempt ends as `expired`, L2 applies); `alive` leaves it; `unknown`, a failing probe, or no probe at all falls back to the deadline and the lease (R4). A probe answering `alive` past the deadline still loses: alive is not progressing. The kill-and-resume case therefore costs one probe, not a lease's worth of waiting: a new session asks, the dead session's process is gone, the step is freed.

**Rejected.**
- *A — a per-action lease the actor renews (`leaseMs`, `spore renew`).* Not rejected outright: it survives as the opt-in lease of R4. As the only mechanism it lost because a killed session holds the step until its lease runs out.
- *B — no lease; the claim holds until reported or freed by hand.* Lost because a dead session's step is blocked forever.
- *C — keep the 10 s lease, renewed by a background process the mod starts.* Lost because it depends on a process outliving the session, which is C05's open measurement.
- *A built-in `pid` probe in spore.* Proposed during the grill and withdrawn on Max's correction: spore must not know what a handle is.

### L4. A handshake between the action and the host; `command` can be async (Q6 → A)

**Decision.** The host advertises what it supports: action kinds, which kinds may run async, and result channels (`store` now; `watch` when the action declares one; a message-queue channel later). Each action declares what its implementation supports, in its `async` object (R6). At `validate` and `start`, spore checks every action against the host and rejects the workflow when they do not agree, naming what to configure (for example, "action `review` needs result channel `nats`; this host offers `store`, `watch`"). The agreement is pinned with the run: the declaration lives in the pinned package, and every later dispatch, claim, probe and watch rechecks it. A host that has lost a capability refuses to act on that step and reports it in `status`; it never switches to another way mid-run and never turns its own missing capability into a business failure. In this version there is one store channel, so the handshake is mostly a capability check plus the pinned declaration; the shape admits more channels without a format change.

Async `command` is part of this decision: a `command` with an `async` object runs its `argv` as a short start command that prints the holder handle and exits; the work it started reports later through the result channel. It lets a detached `claude -p`, a herdr pane or a Kubernetes job outlive a killed worker and report back.

**Rejected.**
- *B — the same handshake, async only for `external`; async `command` later.* Lost because async attempts must exist for `external` anyway, so async `command` adds only "run a start command and keep its stdout as the handle", and it serves the destination's kill-and-resume for headless steps.
- *C — no handshake; the action states one mode and spore runs it or rejects it.* Lost to Max's rule that a missing capability fails at start with a fix, never mid-run, and that a resume reuses the agreed way.

## Routine choices

- **R1. Actor label (Q4 → A).** An external action may carry `"actor": "<label>"`, an opaque name the profile chooses (for example the plugin's `claude-session`). `spore step claim --actor <label>` takes only steps with that label. Default, stated in the grill and not objected to: `step claim` without `--actor` takes only unlabelled steps, so a labelled step is never grabbed by a generic claimer by accident. The label is the natural hook for the map's "who may claim what" fog. Rejected: B (no label: the file cannot say who does a step), C (leave it to the fog). The label was renamed from `queue` so it does not clash with message queues as a future result channel.
- **R2. What the actor gets (Q5 → A).** `spore status --json` shows that the run is at an external or async step, whether it is ready or held, the holder's label and handle, and the deadline and lease, but not the input. `spore step claim` returns everything needed to start work: the token, run, node and action IDs, attempt number, the evaluated input, the declared outcomes, the deadline, and references to the step's instruction files (how their bytes are delivered is C14). `spore run` and `spore worker --run --until-parked` return when the run reaches an async step with nothing for the worker to do, with a new exit code 6, "waiting for an async step". The kernel's status stays `running`; "waiting for an async step" is a host view. Rejected: B (`status` shows everything including input, `claim` returns only the token), because input and instructions belong to the holder and `status` should stay a cheap view for menus and panes.
- **R3. Watch returns the result (Q7 → A).** `watch(handle)` blocks until the job ends and returns its result, which spore records like a sync result: a second result channel beside `step complete`. The store keeps the first result from either channel; the other is a stale duplicate kept as late evidence. A worker that dies mid-watch loses nothing: the job continues and the next worker watches the stored handle again. Default, stated in the grill: a watch that exits 0 prints `{"outcome", "output"}` or `{"error", "retryable"}`; any other exit means only that spore lost sight of the job, never that the step failed, and spore falls back to the probe and the deadline. Rejected: B (watch only says "ended"; the result must come through `step complete`), C (no watch; poll the probe on a timer).
- **R4. Deadline and lease (Q8 → C).** Both exist. `async.timeoutMs` (default 1 hour, counted from the claim or the start) always bounds the attempt. `async.leaseMs` is opt-in: an action that sets it expects its holder to call `spore step renew <token>` within that time; without it nobody renews (a person working by hand cannot ping). Whichever bound comes first ends the attempt as `expired` and L2 applies. Max chose C over the recommended A (hard timeout only), so holders with no probe still have a short liveness bound when they can renew.
- **R5. Freeing a step by hand (Q9 → A).** `spore step abandon <runId>` expires the run's held async attempt at once, fences out the old holder (a late `complete` from it fails with `lease.lost` and is kept as late evidence), and then L2 applies. It is a person's act, never automatic, because the old holder may still be alive and editing the same tree. Named `abandon` because `release` already lifts a quarantine. Rejected: B (wait for the probe or the deadline), because a step held by an actor with no probe would be stuck until the deadline.
- **R6. The `async` object (Q10 → A).** An action is async when it carries an `async` object; `external` implies one and may carry it to set its fields. The object is the action's half of the handshake: `timeoutMs`, `leaseMs`, `probe`, `watch`. A sync action cannot carry a stray `probe`. Rejected: B (flat `mode`, `probe`, `watch`, `timeoutMs`, `leaseMs` beside `argv`), because it scatters the async contract across the action. Spelling in §Spec.
- **R7. CLI verbs (Q11 → B).** Grouped under `spore step`: `claim`, `renew`, `complete`, `fail`, `abandon`. Rejected: A (top level), because `spore complete` beside `spore resolve --complete` invites the wrong one; scripts and the mod type these, so the extra word costs little.
- **R8. The first result channel is the store (Max's call).** `spore step complete|fail` writes the result through the store's existing `finish_attempt`. A message-queue channel such as NATS can come later as another caller of the same store operation, so the run model does not change.
- **R9. Outcomes are checked before they reach the kernel (my default).** `spore step complete` refuses an outcome the action does not declare (exit 2) instead of appending it, so a typo by an actor never fails a step through kernel 0.1's `activity.undeclared-outcome` rule. `step complete` also requires its `--output` to be in the canonical JSON domain (integers only, depth at most 32) and the result body to be at most 64 KiB.

## Spec

Normative wording to carry into `spec/host.md`, `spec/store.md` and `spec/cli.md` 0.2. Examples are illustrative, not the reference loop.

### Action fields

```json
"actions": {
  "implement": {
    "kind": "external", "actor": "coding-session",
    "effect": "unsafe", "outcomes": ["ok"],
    "async": { "timeoutMs": "7200000",
               "probe": { "argv": ["./bin/probe-holder"] } }
  },
  "review": {
    "kind": "command", "argv": ["./bin/start-review"],
    "effect": "read-only", "outcomes": ["ok", "changes"],
    "async": { "timeoutMs": "3600000", "leaseMs": "120000",
               "probe": { "argv": ["./bin/probe-review"] },
               "watch": { "argv": ["./bin/wait-review"] } }
  }
}
```

- `kind: "external"`: the worker never claims or runs it. Allowed fields: `actor` (optional, matches `[a-z0-9][a-z0-9.-]{0,63}`), `async` (optional; absent means all defaults), plus the kernel fields `effect` (required), `outcomes`, `retry`.
- `async` on `command`: `argv` is the start command. `async` on any other kind the host cannot run async is a validation error.
- `async.timeoutMs`: decimal string, default `"3600000"`. `async.leaseMs`: decimal string, optional, smaller than `timeoutMs`. `async.probe`, `async.watch`: optional, each `{argv, cwd?, env?, timeoutMs?}` with the `command` rules (argv, no shell; `cwd` against `base_dir`; own process group). A probe's own `timeoutMs` defaults to 10 s, a watch's to the attempt's remaining deadline.

### The async attempt

1. **Start.** For `external`, the invocation stays pending until an actor claims it: `step claim` claims the attempt with the actor as owner, records the holder handle given with `--holder` (or null), and sets `leaseUntilMs` to the nearer of the deadline and, when `leaseMs` is set, `now + leaseMs`. For an async `command`, the worker claims the attempt under its own short lease and runs the start command with the canonical input on stdin and, beside the existing `RUNSPORE_*` variables, the attempt's token in `RUNSPORE_TOKEN`. Exit 0 means started: stdout, canonical JSON of at most 4 KiB, is the holder handle, and the attempt passes to the async bounds as above; from then on the worker holds nothing. Any other result maps through the `command` table of host 0.1 §3 (for example a non-zero exit is a retryable `command.exit` failure).
2. **Held.** The holder works. `step renew <token>` extends `leaseUntilMs` by `leaseMs`, never past the deadline. Spore probes on demand: when `step claim` targets a held step, on `status` and `continue`, and in the worker's sweep at most every 30 s per attempt. When a worker runs and the action declares `watch`, the worker runs one watch per held attempt, outside the `max_concurrent_activities` slots and under its own limit, and stops it on shutdown.
3. **End.** The attempt ends at the first of:
   - `step complete <token> --outcome <o> [--output <json>]` or `step fail <token> --error <message> [--code <c>] [--retryable]`: `finish_attempt` with the token's attempt, owner and fence (a `success` or `failure` result);
   - a watch that exits 0 with a result: `finish_attempt` the same way;
   - a probe answering `dead`, the deadline, the lease, or `step abandon`: the attempt ends as `expired`.

   Kernel 0.1 then routes the result, retries, or parks the run in `needs-intervention` (L2). A second report for the same attempt is stale and kept as late evidence.

### Probe and watch contracts

- **Probe**: stdin is the holder handle. Exit 0 with stdout `{"holder": "alive"}`, `{"holder": "dead"}` or `{"holder": "unknown"}`. Any other exit, a timeout, or other output reads as `unknown`, so a crashing probe never frees a step. Only `dead` frees it early.
- **Watch**: stdin is the holder handle. Exit 0 with stdout `{"outcome": "<o>", "output": <json>}` (success) or `{"error": {"code", "message"}, "retryable": <bool>}` (failure). Any other exit or output means spore lost sight of the job; the attempt stays held.

### The handshake

The host's capability set, printed by `spore capabilities --json`, lists the action kinds it runs, the kinds it runs async, and its result channels (`store`, and `watch` for actions that declare one). `validate` and `start` reject an action whose kind, async use or declared operations the host does not offer, naming the action and what to configure. Before dispatching, claiming, probing or watching, the host checks the pinned action again; when it falls short it leaves the attempt untouched and reports `capability-missing` for that step in `status`.

### Store 0.2

- An attempt carries `holder` (opaque canonical JSON, at most 4 KiB, or null) and `deadlineMs`. `claim_attempt` accepts both; for an external step the owner is a fresh ID per claim. A new `hold_attempt` records the handle and moves `leaseUntilMs` to the async bounds for an async `command` whose start command exited 0; it has the same owner, attempt and fence predicates as `heartbeat`. `leaseUntilMs` keeps its meaning (the nearer bound), so `heartbeat` serves as `renew` capped at `deadlineMs`, and `finish_attempt` is unchanged.
- An attempt can be expired before its `leaseUntilMs`: `expire_attempt` gains a `reason` (`lease`, `probe-dead`, `abandoned`), and only `lease` requires `now ≥ leaseUntilMs`. The appended event is the same `expired` `ActivityResult`; the reason goes to the audit record.
- The ordinary sweep (`scan_due`) also expires attempts whose `deadlineMs` has passed.

### CLI 0.2

| Command | Does |
| --- | --- |
| `step claim [<runId>] [--actor <label>] [--holder <json>]` | Claims the ready external step (of that run, or the first ready one), probing a held step first. Prints the claim as JSON (R2). Without `--actor`, only unlabelled steps |
| `step renew <token>` | Extends the lease (R4) |
| `step complete <token> --outcome <o> [--output <json>]` | Reports success (R9) |
| `step fail <token> --error <message> [--code <c>] [--retryable]` | Reports failure; `--retryable` lets the kernel retry up to `maxAttempts` |
| `step abandon <runId>` | Expires the run's held async attempt now (R5) |
| `capabilities` | Prints the host's capability set |

`status --json` gains a `step` object when the position is an async step: `{kind, actor, state: "ready"|"held"|"capability-missing", holder, deadlineMs, leaseUntilMs, attempt}`. Exit code 6: the run is waiting for an async step.

### Kill and resume, step by step (the reference `implement` step)

1. The worker reaches `implement` (`external`, `unsafe`) and exits with code 6. The session's mod runs `spore step claim --actor coding-session --holder '{…}'`, gets the token, input and instruction references, and Claude works.
2. The session is killed mid-step. Nothing reports; the store still shows the attempt held by that session's handle.
3. A new session opens; `spore continue` (C08) or `step claim` probes the handle. The probe answers `dead`, so the attempt is expired with reason `probe-dead`. (No probe, or `unknown`: the lease or the deadline does the same later, or the person runs `step abandon`.)
4. The kernel sees `expired` on an `unsafe` action and parks the run in `needs-intervention`.
5. The person chooses: redo (`resolve --retry`, a new attempt on the half-edited tree, same effect key), mark done (`resolve --complete ok`), or fail (`resolve --fail`). `plan` and every other finished step are not rerun.

## Verified facts

Established by exploring the repo, not by asking:

- Kernel 0.1 reads only `outcomes`, `effect` and `retry` from an action (spec/kernel.md §2, decision M02); the action's `kind` is a host concern. An `expired` or `unknown` result retries a repeatable effect up to `maxAttempts` and parks an `unsafe` one in `needs-intervention`, with diagnostic `invocation.unknown` (spec/kernel.md §7.3). `resolve` answers it with `complete`, `retry` (regardless of `maxAttempts`) or `fail`.
- `claim_attempt` already takes the owner and the lease length per call (`ClaimAttempt { worker, lease_duration_ms }`, `crates/runspore-types/src/store.rs`); the claim returns `AttemptRef { attempt_id, owner, fence }`. `heartbeat` and `finish_attempt` require the matching owner, attempt and fence and `now < leaseUntilMs`; `expire_attempt` requires `now ≥ leaseUntilMs` (`lease.not-due`); finish and expire compete and exactly one wins; `record_late_evidence` keeps a result that lost (spec/store.md §2).
- Engine defaults: `lease_ms` 10 000, `heartbeat_ms` 3 000, `max_concurrent_activities` 4 (spec/host.md §2). After a crash, an in-flight attempt waits for its lease to run out before the sweep expires it (spec/host.md §4).
- `start` rejects an action whose `kind` has no registered runner (`action.kind-unregistered`, `crates/runspore-host/src/engine.rs:238`). At dispatch, a missing runner fails the attempt non-retryably with `host.runner-missing` (`engine.rs:951`).
- `run_until_parked` stops on a terminal or quarantined run, or on `waiting` / `needs-intervention` with no pending event (`engine.rs:182`). A run at an activity nobody executes is `running` and would never park, hence exit code 6.
- CLI 0.1 exit codes are 0–5, 10 and 130 (spec/cli.md); 6 is free. `release` (lifts a quarantine) and `resolve --complete|--retry|--fail` exist.
- The `command` runner passes `RUNSPORE_RUN_ID`, `RUNSPORE_NODE_ID`, `RUNSPORE_INVOCATION_ID`, `RUNSPORE_ATTEMPT` and `RUNSPORE_EFFECT_KEY`, gives stdin the canonical input, runs the child in its own process group, and caps stdout at 64 KiB (spec/host.md §3). `append_event` caps a body at 64 KiB (spec/store.md §2).
- Mods (docs/research/claude-code-mods.md): a timer started from `session.start` keeps running between turns but stops on module reload; `$.process.run` runs outside the Bash sandbox; whether a process a mod starts outlives the session is not documented (C05 measures it).
- docs/13 already sketched a "controlled execution mode" with `get_ready`, `claim_action`, `complete_action`, `fail_action` and attempt tokens, and the rule that an agent cannot complete an activation it did not claim; L1 is that mode, made generic.

## Risks

- **A false "dead" lets two actors edit one tree.** Only an explicit `{"holder": "dead"}` frees a step early; a crash, timeout or garbled output reads as `unknown`. A probe must be written so that "dead" is certain (for a PID, compare the process start time, not just the number).
- **The in-session probe depends on C05.** What identifies a live Claude Code session, and whether a PID check is enough, is measured there; until then the reference step relies on the deadline, the lease, or `abandon`.
- **The 1-hour default deadline can expire real work.** A long agent step past its deadline ends as `expired` and, being `unsafe`, waits for a person, who can mark it done. Authors of long steps set `timeoutMs`.
- **Probe, watch and start commands are code the workflow names.** They run with the user's rights, outside any sandbox, like `command` actions; the local trust model of docs/11 applies. A workflow from another author can name them.
- **Two result channels.** A watch and a `step complete` can both report; the store keeps the first and the second becomes late evidence. Implementations must not treat the loser's report as lost work.
- **An async start that exits 0 without starting anything** is caught only by the probe or the deadline.
- **Watches hold worker resources** for as long as their jobs run; they need their own concurrency limit and must stop on shutdown.
- **Exit code 6 is new.** Scripts that treat anything outside 0–5 as an error will misread a run waiting for an async step.

## Deferred

- **Message-queue result channels (NATS or similar).** Max's "for the future". R8 keeps the store operation as the one entry point so a queue bridge is additive. Reopens as its own effort when a deployment needs a channel the CLI cannot reach.
- **A WIT interface for async action implementations** (`start`, `probe`, `watch` as exports of a WASM action). It arrives with the action kind that runs WebAssembly modules, already out of this map's scope.

## Open threads

- **Two defaults Max did not rule on.** `step claim` without `--actor` takes only unlabelled steps (R1); the probe answers through stdout JSON rather than exit codes (§Probe). Both were stated in the grill and drew no objection.
- **0.1's `host.runner-missing` contradicts L4.** Host 0.1 fails a step non-retryably when its runner is missing at dispatch, turning a host fault into a business outcome; L4 says the host leaves the attempt untouched and reports `capability-missing`. Host 0.2 should change the 0.1 behaviour.
- **The plugin's holder handle and probe** for an in-session step (what it records at claim, what its probe checks) are plugin concerns, settled with C07's mapping of the `agent` kind and C05's measurements.
- **Who runs the worker, sweeps and watches while the person works in Claude Code** is C06. This record only fixes that the worker exits at an async step (code 6), that probes also run on demand from `status`, `continue` and `step claim`, and that watches need a running worker.
- **Instruction delivery.** `step claim` returns references to the step's instruction files; how the actor gets the bytes is C14.
- **Output schemas.** If `spore` validates an external step's output, `step complete` is where it happens; that is C12.
- **docs/13 paragraph 3** ("a controlled execution mode … exposes get_ready, claim_action, …") is realised by L1 and should be rewritten when the spec lands, together with the paragraphs the map already marks as superseded.
