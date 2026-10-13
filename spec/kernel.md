# Kernel semantics 0.1 and 0.2

The kernel is one pure function:

```text
transition(identity, graph, snapshot?, event, limits) -> decision | failure
```

It performs no I/O, reads no clock, and uses no randomness, floating point, threads,
or unordered collections. The same request yields the same bytes on every host. Wire
types are in `crates/runspore-types` (`model`, `reducer`); this file specifies behavior.

## Versions

The kernel speaks semantics `0.1` and `0.2` and accepts workflow documents in format
`runspore.workflow/0.1` and `runspore.workflow/0.2`. A request names its semantics in
`identity.semanticsVersion` (check 1), and the state records the semantics it was
created under (check 5). The two are independent: a request of either semantics may
carry a document of either format.

Version 0.2 is being added beside 0.1 one rule at a time. A rule that applies to one
version only says so, and is keyed by the document's `format`; every other rule
applies to both. Outside those rules, a `runspore.workflow/0.2` document is validated
and run exactly as the same document marked `runspore.workflow/0.1`: the same
failures, and decisions identical byte for byte. `describe` reports semantics `0.1`
and format `runspore.workflow/0.1`, the versions a host uses when it does not choose.
The rules marked 0.2 so far:

| Rule | Where | Applies to |
| --- | --- | --- |
| `from` as a path's first step; `default` beside `$get` | §4, W10 | documents in format `runspore.workflow/0.2`, under either semantics |
| A step that cannot recover is stuck and waits for a person instead of failing the run | §3, §7.3, §7.4 | documents in format `runspore.workflow/0.2`, under either semantics |
| Nodes may carry `outputSchemas`, which no transition reads | §2, W01, W13 | documents in format `runspore.workflow/0.2`, under either semantics |
| Loop limits: `loopLimits`, the reserved outcome `exhausted`, `streaks`, *Route* | §2 (W07–W09, W12), §3, §7.4 | documents in format `runspore.workflow/0.2`, under either semantics |
| Every node is reachable from `start`, and an end from every node | §2 (W14, W15) | documents in format `runspore.workflow/0.2`, under either semantics |

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
  "format": "runspore.workflow/0.2",
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
                "outcomes": {"ok": "approve", "red": "build", "failed": "broken",
                             "exhausted": "broken"},
                "loopLimits": {"red": 3}},
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
| W01 | `format` is `runspore.workflow/0.1` or `runspore.workflow/0.2` |
| W02 | `name` matches `[A-Za-z0-9_.-]{1,128}` |
| W03 | 1 to 256 nodes; node IDs match `[A-Za-z0-9_-]{1,64}` |
| W04 | Action IDs match `[A-Za-z0-9_-]{1,64}` |
| W05 | `start` names a node |
| W06 | `maxVisitsPerNode` in 1..=1000; `maxActivations` in 1..=10000 |
| W07 | Each action parses as `ActionPolicy`; outcomes are non-empty, unique, match `[a-z0-9][a-z0-9-]{0,63}`, and none is `failed` (0.2: nor `exhausted`); `maxAttempts` in 1..=100 |
| W08 | Activity node: its action exists; `outcomes` has a key for every action outcome; the only other keys allowed are `failed`, and (0.2) `exhausted` when the node has `loopLimits`; every target names a node |
| W09 | Await-signal node: `signal` matches `[A-Za-z0-9_.-]{1,64}`; `outcomes` is non-empty; keys match the outcome pattern; (0.2) the key `exhausted` only when the node has `loopLimits`; every target names a node |
| W10 | Mappings are well formed (section 4) |
| W11 | Fail node: `error.code` matches `[a-z0-9][a-z0-9.-]{0,63}`; `error.nodeId` is null |
| W12 | `loopLimits` (0.2 only; no node of a 0.1 document has it) appears only on activity and await-signal nodes and is non-empty. Each key is a key of the node's `outcomes` other than `exhausted`; each value is an integer with value + 1 ≤ `maxVisitsPerNode` (so at most 999). A node with `loopLimits` routes `exhausted` |
| W13 | 0.2: `outputSchemas` (below) is non-empty; each key is an outcome the node produces, one of its action's `outcomes` for an activity node or a key of its `outcomes` for an await-signal node, and never `failed` or `exhausted`; each value is a JSON object. Await-signal nodes that wait for the same `signal` declare equal schemas, or none, for every outcome they both route |
| W14 | (0.2) Every node is reachable from `start` by following `outcomes` |
| W15 | (0.2) From every node, following `outcomes` reaches a `complete` or `fail` node |

A violation is `invalid-input` / `workflow.invalid`; its details name the node. An
action with effect `reconcilable` is `incompatible-version` /
`workflow.unsupported-effect`, checked as part of W07. In a `runspore.workflow/0.1`
document, a node that carries `outputSchemas` violates W01.

`loopLimits` is an object from outcome to integer: `"loopLimits": {"red": 3}` lets the
node route `red` at most 3 times in a row; the next `red` takes the node's `exhausted`
route instead (*Route*, §7.4). In a 0.2 document `exhausted` is a reserved outcome,
like `failed`: no action declares it and no signal takes it. A 0.1 document keeps
0.1's rules, so an await-signal node of a 0.1 document may route an outcome named
`exhausted` as any other.

W14 and W15 look only at routes. A stuck step (§7.4) is not a route, and `loopLimits`
exempts no loop: a node's `exhausted` target must reach an end like every other node,
so `a -exhausted-> b -> a` with no other way out fails W15. In W14 the first node in
byte order that `start` does not reach is reported; in W15 the first that reaches no
end.

**Output schemas (0.2).** An `activity` or `await-signal` node of a
`runspore.workflow/0.2` document may carry `outputSchemas`: an object from outcome to
the schema that outcome's output must fit ([schema.md](schema.md)). The kernel checks
only the field's shape (W13) and never reads it in a transition: a result or a signal
whose data does not fit is decided exactly as one that fits. Hosts check the data
before it is recorded ([host.md](host.md) §2, §3) and check the schemas themselves
against the subset (`workflow.invalid-schema`). `{"$outputSchemas": {}}` left in a
mapping is a `$` key, so W10 already refuses it. `exhausted` takes no schema: the
output it records is the real result's, which fits that outcome's schema.

## 3. State

Type: `model::State`. `status` is one of:

| Status | Meaning | `position` | `invocation` |
| --- | --- | --- | --- |
| `running` | An attempt of an activity is authorized | the activity node | `scheduled` |
| `waiting` | Parked at an await-signal node | that node | null |
| `needs-intervention` | An unsafe invocation has an unknown outcome, or (0.2) a step is stuck | the activity node | `unknown` or `stuck` |
| `completed` | Terminal success | null | null |
| `failed` | Terminal failure | null | null |

An invocation's `state` is `scheduled`, `unknown`, or (0.2 only) `stuck`. A stuck
invocation keeps the error that made it stuck in `error`; no other invocation has an
`error`, and the field is absent when it has none. A stuck invocation in the snapshot
of a 0.1 document is `state.invalid`.

`nodes[id]` holds the latest result of a node: `{visit, outcome, output}`. A later
visit replaces it. `visits[id]` counts admitted entries.

`streaks` (0.2) maps a node ID to `{outcome, count}`: the limited outcome the node
routed last and how many times in a row (*Route*, §7.4). An entry exists only for a
node with `loopLimits` whose last routed outcome is one it limits, with `count` in
1..=its limit; the field is absent when it has no entry. A snapshot whose streak names
a node without `loopLimits`, an outcome the node does not limit, or a count outside
1..=limit is `state.invalid`. `activations` counts admitted
entries across all nodes. `signals` is the buffer of accepted, unconsumed signals in
acceptance order. `lastSequence` is the sequence of the last event applied.
`lastAcceptedAtMs` is the maximum accepted time seen: logical time never decreases.

## 4. Mappings

A mapping is a JSON value evaluated against the run context:

```json
{"input": <run input>, "nodes": {"<id>": {"visit": 1, "outcome": "ok", "output": ...}},
 "run": {"id": "<runId>", "tenant": "<tenant>"},
 "from": {"node": "<id>", "outcome": "red", "output": ...} | null}
```

- An object whose only key is `$get` is replaced by the value at that path. The path
  is an array of one or more steps; a string step selects an object member, a
  non-negative integer step selects an array element. The first step must be `input`,
  `nodes`, or `run`; in a 0.2 document it may also be `from`.
- In a 0.2 document, a `$get` object may carry one more key, `default`. When the path
  does not resolve, the object is replaced by the value of `default`, unevaluated (as
  `$literal`). When the path resolves, `default` is ignored.
- An object whose only key is `$literal` is replaced by its value, unevaluated.
- Any other object key starting with `$`, a `$literal` object with more keys, or a
  `$get` object with a key other than `$get` and (0.2) `default`, is a validation
  error (W10), as is a malformed path.
- Other objects and arrays are evaluated member by member; scalars are themselves.

`from` (0.2) is the arrow the run took into the node whose mapping is being evaluated:
the node whose result routed there, the outcome it routed on, and the output recorded
with it, all as `nodes[node]` holds them at that moment. A `failed` arrow shows
outcome `failed` and output `{"error": ErrorInfo}` (*fail the invocation*); a signal's
arrow shows the waiter, the signal's outcome and its data. An `exhausted` arrow shows
the outcome the node really returned, as `nodes` records it (*Route*). When the node is entered
as the start node, `from` is `null`. It exists only while the entered node's mappings
are evaluated; the state gains no field. `{"$get": ["from"]}` always resolves; a deeper
path into a `null` `from` does not.

A path that does not resolve, on a `$get` without `default`, fails the run: terminal
`failed` with error `mapping.missing-path`, `nodeId` set, `details` `{"path": [...]}`.

Each mapping value visited costs one expression operation; each path step costs one,
for the whole path even when it does not resolve. A `$get` that falls back to its
`default` costs one more. `$literal` costs one and an absent mapping costs one. Members
of an object are evaluated in canonical key order, which fixes which missing path is
reported when several are missing.

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

1. `identity.semanticsVersion` is `0.1` or `0.2`, else `incompatible-version` /
   `version.semantics`.
2. `identity.codecVersion` is `jcs-int53/1`, else `incompatible-version` / `version.codec`.
3. `graph` decodes as a canonical `Workflow` and passes validation.
4. `inputEvent.kind` is known, else `invalid-input` / `event.unknown-kind`. The payload
   decodes canonically as that kind's type, else `invalid-input` / `event.invalid`.
5. Snapshot: absent with a kind other than `run.started` is `invalid-input` /
   `state.missing`; present with `run.started` is `invalid-input` /
   `state.unexpected-start`; undecodable is `invalid-input` / `state.invalid`; a
   `format` other than `runspore.state/0.1`, or a `semantics` other than
   `identity.semanticsVersion`, is `incompatible-version` / `version.state-format`.
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
  buffered signals, a streak §3 does not allow) is `state.invalid`.

### 7.2 Bookkeeping

Before dispatch: `lastSequence = event.sequence`;
`lastAcceptedAtMs = max(lastAcceptedAtMs, event.acceptedAtMs)`. "Now" below means the
updated `lastAcceptedAtMs`.

### 7.3 Dispatch by event kind

If the state is terminal, every event is consumed with diagnostic
`event.ignored-terminal` (details `{"kind": <event kind>}`) and no other change.

**`run.started`** — create the state: status `running`, `semantics` from
`identity.semanticsVersion`, the given tenant, run ID and input, empty `nodes`,
`visits`, `signals`, zero `activations`, no result. Then *enter* the start node.

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
  `{"outcome"}`). Otherwise clear the invocation and *route* the node with that
  outcome and `result.output`.
- `failure`: if `result.retryable` and `inv.attempt < maxAttempts`: *retry*. Otherwise
  *fail the invocation* with `result.error`, or with `activity.failed` when it is absent.
- `unknown` or `expired`: if the action's effect is repeatable (`pure`, `read-only`,
  `idempotent`): *retry* when `inv.attempt < maxAttempts`, otherwise *fail the
  invocation* with `activity.attempts-exhausted` (details `{"lastStatus"}`). If the
  effect is `unsafe`: set `inv.state = unknown`, status `needs-intervention`, emit
  diagnostic `invocation.unknown` (details `{"invocationId", "attempt", "status"}`).

**`invocation.resolved`** — unless `state.invocation` has that ID and is `unknown` or
`stuck`: diagnostic `resolve.not-applicable` (details `{"invocationId", "reason"}`),
no other change. Otherwise by `resolution.action`:

- `complete`: if the action does not declare the outcome, diagnostic
  `resolve.not-applicable` and no other change. Otherwise as a `success` result with
  that outcome and output.
- `retry`: *retry* with no delay and regardless of `maxAttempts`. A stuck invocation
  drops its kept error. The new attempt has the same invocation ID and effect key.
- `fail`: for an `unknown` invocation, *fail the invocation* with the given error,
  but never park: with no `failed` route the run ends `failed` even under 0.2. For a
  `stuck` invocation, terminal `failed` with the kept error as the result; the given
  error stays in the event only. A person's fail is final.

The `reason` of `resolve.not-applicable` is `invocation-mismatch` (no invocation, or
another ID), `invocation-not-unknown` (the ID matches but its phase is `scheduled`),
or `outcome-undeclared`.

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
does not route that outcome, or (0.2) the outcome is `exhausted`: diagnostic
`signal.outcome-unrouted` (details `{"eventId", "outcome"}`) and look for the next
matching signal. Otherwise *route* the node with that outcome and `signal.data`.

**Route(node, outcome, output)** — every place that records a node's result and moves
on goes through it: a `success` result (also by `invocation.resolved` with
`complete`), *consume*, and *fail the invocation* at a node that routes `failed`.

1. Record `nodes[node] = {visit, outcome, output}`.
2. If the node has `loopLimits` (0.2):
   - If `outcome` is a key of `loopLimits`: let `n` be `streaks[node].count` when
     `streaks[node].outcome` equals `outcome`, else 0. If `n + 1 > loopLimits[outcome]`:
     delete `streaks[node]`, emit diagnostic `loop.exhausted` (details `{"outcome",
     "limit"}`), *enter* `node.outcomes.exhausted`, and stop. Otherwise set
     `streaks[node] = {outcome, count: n + 1}`.
   - Otherwise delete `streaks[node]`.
3. *Enter* `node.outcomes[outcome]`.

The recorded result stays the real one: after the limit is used up, `nodes[node]`
holds the outcome the node returned and its output, so the node behind the
`exhausted` arrow can map them. *Enter*'s limits (the safety net) and mapping errors
are unchanged and stay terminal; a node entered through `exhausted` counts its visit
like any other.

**Retry** — increment `inv.attempt`, set `inv.state = scheduled`, status `running`.
Emit `activity.retry` with the new attempt as command ordinal and
`notBeforeMs = now + retry.delayAfter(previous attempt)` (zero delay when resolving).
Emit diagnostic `activity.retry-scheduled` (details `{"attempt"}`).

**Fail the invocation(error)** — set `error.nodeId` to the node. If the node routes
`failed`: clear the invocation and *route* the node with outcome `failed` and output
`{"error": error}`. Otherwise, for a 0.1 document,
clear the invocation and end terminal `failed` with `result = {error}`.

For a 0.2 document with no `failed` route the step is **stuck**: set `inv.state =
stuck` and `inv.error = error`, keep the invocation and the position, set status
`needs-intervention`, and emit diagnostic `invocation.stuck` (details
`{"invocationId", "attempt", "error"}`). This covers every way an attempt gives up: a
non-retryable failure, a retryable failure on the last attempt, the attempts of a
repeatable action exhausted by `unknown` or `expired` results
(`activity.attempts-exhausted`), and an undeclared outcome
(`activity.undeclared-outcome`). No command is emitted; a person answers with
`invocation.resolved`.

**Terminal `failed` from a limit or mapping error** — status `failed`, `result =
{error}`, clear `position` and `invocation`. Limit and mapping errors are not a step
failing: they never take a `failed` route and never make a step stuck, in either
version.

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
| `invocation.unknown`, `invocation.stuck`, `activity.retry-scheduled` | the activity node |
| `loop.exhausted` | the node that used up its loop limit |

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
| K6 | An unknown outcome on an `unsafe` action never produces a command (C04); nor does a step becoming stuck |
| K7 | Exceeding a budget commits nothing (C16) |
| K8 | Keys with supplementary-plane characters keep JCS order and digest (C15) |
| K9 | Output does not depend on the insertion order of maps in the input documents |
| K10 | A limited outcome is never routed more than its limit times in a row (0.2) |

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

Every trace's document is `runspore.workflow/0.2`, except the W01 trace, whose format
no version defines, and the three traces where a step gives up with no `failed` route,
which pin the 0.1 rule; the traces include requests of both semantics. The Rust kernel
tests also replay every trace with its `format` switched between `runspore.workflow/0.1`
and `runspore.workflow/0.2`, and require the same expectations, so a 0.1 document stays
accepted. The exceptions are the traces of a rule that names a format, which must change:
the three 0.1 traces above, where a 0.2 step is stuck, which the `14*` traces pin, and
the 0.2 traces of `from`, `$get` `default`, the stuck step, `outputSchemas`, loop limits,
W12, W14 and W15.

The loop-limit traces: the limit used up takes `exhausted` with the real result
recorded; another outcome resets the streak; taking `exhausted` resets it; a signal
with outcome `exhausted` is unrouted; and W07, W08, W09, W12, W14 and W15 each
refuse a document.

The 0.2 mapping rules have their own traces: the start node sees `from` `null`; a
loop back sees the judging step's outcome and output; a `failed` arrow shows as
`failed` with the error; a signal's arrow shows its outcome and data; a `default` is
used, unevaluated, while its path is missing and ignored once it resolves; a `$get`
object with a key besides `default` is `workflow.invalid`.

`outputSchemas` has two: a result and a signal whose data do not fit their
schemas route as if no schema were declared, and two await-signal nodes that wait for
one signal with different schemas for an outcome both route are `workflow.invalid`
(W13).
