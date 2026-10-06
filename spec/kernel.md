# Kernel semantics 0.1

The kernel is one pure function:

```text
transition(identity, graph, snapshot?, event, limits) -> decision | failure
```

It performs no I/O, reads no clock, and uses no randomness, floating point, threads,
or unordered collections. The same request yields the same bytes on every host. Wire
types are in `crates/runspore-types` (`model`, `reducer`); this file specifies behavior.

## 1. Canonical data

All payloads, snapshots, and workflow documents handed to the kernel are canonical
JSON (`runspore_types::canonical`): RFC 8785 key order by UTF-16 code units, no
whitespace, integers within ±(2^53 − 1), no other numbers, no duplicate keys, depth at
most 32. Counters and timestamps are decimal strings up to 2^63 − 1. The kernel rejects
input that is valid JSON but not in canonical form. Everything the kernel emits is
canonical.

Digests and identities are in `runspore_types::digest`. The kernel never invents an ID.

| Identity | Derivation |
| --- | --- |
| Activation | `<nodeId>/<visit>`; visit counts entries into that node from 1 |
| Invocation | `ids::invocation(tenant, runId, activationId)`; stable across attempts |
| Effect key | equals the invocation ID |
| Command | `ids::command(tenant, runId, activationId, ordinal)`; ordinal is the attempt the command authorizes |
| Snapshot digest | `digest::state(snapshot bytes)` |

## 2. Workflow document

Type: `model::Workflow`. One document holds nodes and action bindings. Its canonical
bytes are the package; `digest::package` of them is the package digest.

```json
{
  "format": "runspore.workflow/0.1",
  "name": "review-loop",
  "start": "build",
  "limits": {"maxVisitsPerNode": 16, "maxActivations": 256},
  "actions": {
    "build": {"kind": "command", "argv": ["make"], "effect": "idempotent",
              "outcomes": ["ok", "red"], "retry": {"maxAttempts": 3, "backoffMs": "500"}}
  },
  "nodes": {
    "build":   {"kind": "activity", "action": "build",
                "input": {"repo": {"$get": ["input", "repo"]}},
                "outcomes": {"ok": "approve", "red": "build", "failed": "broken"}},
    "approve": {"kind": "await-signal", "signal": "approval",
                "outcomes": {"approved": "done", "rejected": "build"}},
    "done":    {"kind": "complete", "output": {"$get": ["nodes", "build", "output"]}},
    "broken":  {"kind": "fail", "error": {"code": "build.broken", "message": "never passed"}}
  }
}
```

From an action the kernel reads `outcomes` (default `["ok"]`), `effect` (required), and
`retry` (default one attempt). It ignores every other action field.

### Validation

Validation runs on every transition, before anything else that depends on the graph.
Rules are checked in this order; nodes and actions are visited in ascending byte order
of their IDs; the first violation is reported.

| Rule | Requirement |
| --- | --- |
| W01 | `format` is `runspore.workflow/0.1` |
| W02 | `name` matches `[A-Za-z0-9_.-]{1,128}` |
| W03 | 1 to 256 nodes; node IDs match `[A-Za-z0-9_-]{1,64}` |
| W04 | Action IDs match `[A-Za-z0-9_-]{1,64}` |
| W05 | `start` names a node |
| W06 | `maxVisitsPerNode` in 1..=1000; `maxActivations` in 1..=10000 |
| W07 | Each action parses as `ActionPolicy`; outcomes are non-empty, unique, match `[a-z0-9][a-z0-9-]{0,63}`, and none is `failed`; `maxAttempts` in 1..=100 |
| W08 | Activity node: its action exists; `outcomes` has a key for every action outcome; the only other key allowed is `failed`; every target names a node |
| W09 | Await-signal node: `signal` matches `[A-Za-z0-9_.-]{1,64}`; `outcomes` is non-empty; keys match the outcome pattern; every target names a node |
| W10 | Mappings are well formed (section 4) |
| W11 | Fail node: `error.code` matches `[a-z0-9][a-z0-9.-]{0,63}`; `error.nodeId` is null |

A violation is `invalid-input` / `workflow.invalid`. An action with effect
`reconcilable` is `incompatible-version` / `workflow.unsupported-effect`, checked as
part of W07.

## 3. State

Type: `model::State`. `status` is one of:

| Status | Meaning | `position` | `invocation` |
| --- | --- | --- | --- |
| `running` | An attempt of an activity is authorized | the activity node | `scheduled` |
| `waiting` | Parked at an await-signal node | that node | null |
| `needs-intervention` | An unsafe invocation has an unknown outcome | the activity node | `unknown` |
| `completed` | Terminal success | null | null |
| `failed` | Terminal failure | null | null |

`nodes[id]` holds the latest result of a node: `{visit, outcome, output}`. A later
visit replaces it. `visits[id]` counts admitted entries. `activations` counts admitted
entries across all nodes. `signals` is the buffer of accepted, unconsumed signals in
acceptance order. `lastSequence` is the sequence of the last event applied.
`lastAcceptedAtMs` is the maximum accepted time seen: logical time never decreases.

## 4. Mappings

A mapping is a JSON value evaluated against the run context:

```json
{"input": <run input>, "nodes": {"<id>": {"visit": 1, "outcome": "ok", "output": ...}},
 "run": {"id": "<runId>", "tenant": "<tenant>"}}
```

- An object whose only key is `$get` is replaced by the value at that path. The path
  is an array of one or more steps; a string step selects an object member, a
  non-negative integer step selects an array element. The first step must be `input`,
  `nodes`, or `run`.
- An object whose only key is `$literal` is replaced by its value, unevaluated.
- Any other object key starting with `$`, or a `$get`/`$literal` object with more keys,
  is a validation error (W10), as is a malformed path.
- Other objects and arrays are evaluated member by member; scalars are themselves.

A path that does not resolve fails the run: terminal `failed` with error
`mapping.missing-path`, `nodeId` set, `details` `{"path": [...]}`.

Each mapping value visited costs one expression operation; each path step costs one,
for the whole path even when it does not resolve. `$literal` costs one and an absent
mapping costs one. Members of an object are evaluated in canonical key order, which
fixes which missing path is reported when several are missing.

## 5. Events

`Envelope.kind` and payload type:

| Kind | Payload | Appended by |
| --- | --- | --- |
| `run.started` | `RunStarted` | `create_run`, sequence 1, event ID `start` |
| `activity.result` | `ActivityResult` | `finish_attempt`, `expire_attempt` |
| `signal.received` | `SignalReceived` | `append_event` |
| `invocation.resolved` | `InvocationResolved` | `append_event` |

## 6. Commands

| Kind | Payload | Meaning |
| --- | --- | --- |
| `activity.schedule` | `ScheduleActivity` | Create the invocation and authorize attempt 1 |
| `activity.retry` | `RetryActivity` | Authorize attempt `n` of an existing invocation, same input and effect key |

`Command.activationId` is the activation that owns the invocation.

## 7. Transition algorithm

### 7.1 Request checks

In order; the first failing check returns its failure and nothing else happens.

1. `identity.semanticsVersion` is `0.1`, else `incompatible-version` / `version.semantics`.
2. `identity.codecVersion` is `jcs-int53/1`, else `incompatible-version` / `version.codec`.
3. `graph` decodes as a canonical `Workflow` and passes validation.
4. `inputEvent.kind` is known, else `invalid-input` / `event.unknown-kind`. The payload
   decodes canonically as that kind's type, else `invalid-input` / `event.invalid`.
5. Snapshot: absent with a kind other than `run.started` is `invalid-input` /
   `state.missing`; present with `run.started` is `invalid-input` /
   `state.unexpected-start`; undecodable is `invalid-input` / `state.invalid`; a
   `format` or `semantics` other than this version's is `incompatible-version` /
   `version.state-format`.
6. Sequence: `run.started` must have sequence 1; any other event must have sequence
   `lastSequence + 1`. Else `invalid-input` / `event.out-of-order`.
7. `RunStarted.tenant` and `runId` match `[A-Za-z0-9_.-]{1,128}`, else `event.invalid`.

`Failure.details` is explanatory text and is not part of conformance; `kind` and `code` are.

Refinements of these checks:

- In check 3, a graph that does not decode at all is `workflow.invalid`. Within W07 the
  order for one action is: it parses, its effect is supported, its outcomes are valid,
  its `maxAttempts` is in range.
- In check 4, an envelope `sequence` or `acceptedAtMs` above 2^63 − 1 is `event.invalid`.
- In check 5, `format` and `semantics` are read from the raw snapshot before the typed
  decode, so a future format is `version.state-format`, not `state.invalid`. A snapshot
  that decodes but contradicts itself or the graph (status against position and
  invocation, a position at a node of the wrong kind, a visit count below the
  position's visit, an invocation ID that is not the derived one, more than 64
  buffered signals) is `state.invalid`.

### 7.2 Bookkeeping

Before dispatch: `lastSequence = event.sequence`;
`lastAcceptedAtMs = max(lastAcceptedAtMs, event.acceptedAtMs)`. "Now" below means the
updated `lastAcceptedAtMs`.

### 7.3 Dispatch by event kind

If the state is terminal, every event is consumed with diagnostic
`event.ignored-terminal` (details `{"kind": <event kind>}`) and no other change.

**`run.started`** — create the state: status `running`, the given tenant, run ID and
input, empty `nodes`, `visits`, `signals`, zero `activations`, no result. Then *enter*
the start node.

**`signal.received`** — if status is `waiting` and the signal's name equals the
parked node's `signal`, the signal is taken directly, exactly as *consume* would take
it from the buffer (it routes, or it is dropped with `signal.outcome-unrouted`); the
buffer is not involved, so a full buffer can never starve a waiter. Otherwise, if the
buffer holds 64 signals, drop this one with diagnostic `signal.buffer-overflow`
(details `{"eventId": ...}`); else append `{eventId, sequence, name, outcome, data}`.

**`activity.result`** — let `inv` be `state.invocation`. Unless `inv` exists, has the
same `invocationId` and `attempt`, and is `scheduled`: diagnostic `result.stale`
(details `{"invocationId", "attempt"}`), no other change. Otherwise by `status`:

- `success`: the outcome is `result.outcome`, or `ok` when absent. If the action does
  not declare it: *fail the invocation* with `activity.undeclared-outcome` (details
  `{"outcome"}`). Otherwise record `nodes[node] = {visit, outcome, output}`, clear the
  invocation, and *enter* `node.outcomes[outcome]`.
- `failure`: if `result.retryable` and `inv.attempt < maxAttempts`: *retry*. Otherwise
  *fail the invocation* with `result.error`, or with `activity.failed` when it is absent.
- `unknown` or `expired`: if the action's effect is repeatable (`pure`, `read-only`,
  `idempotent`): *retry* when `inv.attempt < maxAttempts`, otherwise *fail the
  invocation* with `activity.attempts-exhausted` (details `{"lastStatus"}`). If the
  effect is `unsafe`: set `inv.state = unknown`, status `needs-intervention`, emit
  diagnostic `invocation.unknown` (details `{"invocationId", "attempt", "status"}`).

**`invocation.resolved`** — unless `state.invocation` has that ID and is `unknown`:
diagnostic `resolve.not-applicable` (details `{"invocationId", "reason"}`), no other
change. Otherwise by `resolution.action`:

- `complete`: if the action does not declare the outcome, diagnostic
  `resolve.not-applicable` and no other change. Otherwise as a `success` result with
  that outcome and output.

The `reason` of `resolve.not-applicable` is `invocation-mismatch` (no invocation, or
another ID), `invocation-not-unknown` (the ID matches but its phase is `scheduled`),
or `outcome-undeclared`.
- `retry`: *retry* with no delay and regardless of `maxAttempts`.
- `fail`: *fail the invocation* with the given error.

### 7.4 Procedures

**Enter(node)** — repeat:

1. Count one microstep. Over `limits.microsteps`: failure `resource-limit` /
   `budget.microsteps`.
2. If `visits[node] + 1 > maxVisitsPerNode`: terminal `failed` with
   `limit.visits-exceeded`. Else if `activations + 1 > maxActivations`: terminal
   `failed` with `limit.activations-exceeded`. Both set `nodeId`; counters are not
   incremented.
3. Increment `visits[node]` and `activations`. The activation ID is `<node>/<visit>`.
4. By node kind:
   - `activity`: evaluate `input`. Set `position`, status `running`, and
     `invocation = {invocationId, activationId, nodeId, actionId, attempt: 1, state:
     scheduled, inputDigest: digest::body(canonical input)}`. Emit `activity.schedule`
     with `notBeforeMs` = now and command ordinal 1. Stop.
   - `await-signal`: set `position`, status `waiting`, clear `invocation`. *Consume*;
     if it routed, continue this loop with the target node, else stop.
   - `complete`: evaluate `output`; status `completed`, `result = {output}`, clear
     `position`. Stop.
   - `fail`: status `failed`, `result = {error}` with `nodeId` set to this node, clear
     `position`. Stop.

**Consume** (at an await-signal node) — find the earliest buffered signal whose name
equals `node.signal`. None: stay `waiting`. Otherwise remove it and count one
microstep (over the budget is the same `budget.microsteps` failure as in *enter*). Its outcome is `signal.outcome`, or `received` when absent. If the node
does not route that outcome: diagnostic `signal.outcome-unrouted` (details
`{"eventId", "outcome"}`) and look for the next matching signal. Otherwise record
`nodes[node] = {visit, outcome, output: signal.data}` and *enter* the target.

**Retry** — increment `inv.attempt`, set `inv.state = scheduled`, status `running`.
Emit `activity.retry` with the new attempt as command ordinal and
`notBeforeMs = now + retry.delayAfter(previous attempt)` (zero delay when resolving).
Emit diagnostic `activity.retry-scheduled` (details `{"attempt"}`).

**Fail the invocation(error)** — set `error.nodeId` to the node and clear the
invocation. If the node routes `failed`: record `nodes[node] = {visit, outcome:
"failed", output: {"error": error}}` and *enter* the target. Otherwise terminal
`failed` with `result = {error}`.

**Terminal `failed` from a limit or mapping error** — status `failed`, `result =
{error}`, clear `position` and `invocation`.

### 7.5 Result checks

After dispatch: more than `limits.maxCommandCount` commands is `resource-limit` /
`budget.command-count`; a snapshot over `limits.maxStateBytes` is `resource-limit` /
`budget.state-bytes`; more than `limits.expressionOperations` operations is
`resource-limit` / `budget.expression-operations`. A failure discards the whole
decision: no partial state, no commands.

Three further failures exist for conditions the checks above cannot rule out; their
codes are defined by the kernel crate (`runspore_kernel::code`): `resource-limit` /
`budget.value-depth` when a value the kernel must emit would nest deeper than 32;
`resource-limit` / `budget.counter-range` when an attempt number or a retry time would
leave its counter domain; `invariant-violation` / `kernel.invariant` for a state the
request checks should have made unreachable.

Diagnostics appear in the order they were produced. `details` is canonical JSON.
`Diagnostic.nodeId` by code:

| Code | `nodeId` |
| --- | --- |
| `event.ignored-terminal`, `signal.buffer-overflow` | null |
| `signal.outcome-unrouted` | the await-signal node |
| `result.stale`, `resolve.not-applicable` | the activity node when the event names the current invocation, else null |
| `invocation.unknown`, `activity.retry-scheduled` | the activity node |

`activity.retry-scheduled` details carry the newly authorized attempt and follow the
retry command. The `message` of an `ErrorInfo` the kernel creates is fixed text and is
part of the snapshot bytes; the kernel's source is the reference for it.

Known limit of 0.1: the expression-operation and state-size budgets are checked after
dispatch, so they bound what is committed, not the work done to find out. Work is
bounded in practice by the size of the workflow document and of the state.

## 8. Properties the tests must demonstrate

| ID | Property |
| --- | --- |
| K1 | Replaying the accepted events of a run from sequence 1 reproduces every snapshot and command byte for byte (C20) |
| K2 | A result is applied at most once; duplicates and stale attempts change nothing (I4) |
| K3 | A terminal state never changes status (I6) |
| K4 | Each visit of a node has a distinct activation and invocation ID; retries keep the invocation ID and change the command ID |
| K5 | A signal accepted before its waiter is consumed exactly once, earliest first (C08) |
| K6 | An unknown outcome on an `unsafe` action never produces a command (C04) |
| K7 | Exceeding a budget commits nothing (C16) |
| K8 | Keys with supplementary-plane characters keep JCS order and digest (C15) |
| K9 | Output does not depend on the insertion order of maps in the input documents |

## 9. Golden traces

`conformance/traces/*.json`, format `runspore.trace/0.1`:

```json
{
  "format": "runspore.trace/0.1",
  "name": "retry-then-success",
  "description": "one line",
  "workflow": { },
  "limits": {"microsteps": 1000},
  "steps": [
    {"event": {"eventId": "start", "sequence": "1", "acceptedAtMs": "1000",
               "kind": "run.started", "payload": { }},
     "expect": {"snapshot": { }, "snapshotDigest": "sha256:…",
                "commands": [{"commandId": "…", "activationId": "…", "kind": "…", "payload": { }}],
                "diagnostics": [{"code": "…", "nodeId": null, "details": { }}],
                "decisionDigest": "sha256:…"}},
    {"event": { }, "expect": {"failure": {"kind": "resource-limit", "code": "budget.microsteps"}}}
  ]
}
```

`workflow`, payloads, snapshots, and details are written as plain JSON; a runner
canonicalizes them before use and compares bytes. `limits` is optional and overrides
individual fields of the default budget. A step that expects a failure leaves the
snapshot unchanged for the next step. `payloadRaw` (a string) may replace `payload` to
feed bytes that are not canonical. Every host's runner must pass every trace.
