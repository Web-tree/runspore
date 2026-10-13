# Host 0.1 and 0.2

The host is everything around the kernel: it packages the kernel as a component, runs
it, commits its decisions, and executes activities. It contains no graph logic. It
never decides a branch, a retry, or an outcome route.

Host 0.2 replaces 0.1 one rule at a time. A rule that changed says what 0.1 did, or is
marked **0.2** and names the 0.1 rule it replaces; every other rule is the same in both.

## 1. Kernel packaging

`crates/runspore-component` builds the kernel as a WebAssembly component exporting
world `machine` from `contracts/wit/machine/machine.wit`.

- The component has **no imports**. A test parses the built component and asserts this.
- Records cross the boundary field for field as in `runspore_types::reducer`.
- `cargo build` and `cargo test` work from a clean checkout with only the pinned Rust
  toolchain and its `wasm32-unknown-unknown` target. No globally installed tools.
  Tool crates (`wit-component`, `wit-bindgen`, …) are pinned in `Cargo.lock`.
- The build is reproducible: two builds on one machine give identical bytes.

`crates/runspore-wasmtime` provides `WasmtimeReducer: Reducer`.

- The component is compiled once and cached. Each transition runs in a **fresh
  instance**; no guest memory survives between calls.
- Wasm features that admit nondeterminism are disabled (threads, relaxed SIMD).
  Memory is bounded. A fuel or epoch watchdog stops a runaway guest.
- A trap, a watchdog stop, or an out-of-memory condition is reported as
  `Failure { kind: InvariantViolation, code: "host.trap" | "host.watchdog" | "host.memory" }`.
  It is never turned into a business outcome. If memory growth was refused during the
  call the code is `host.memory`; else running out of fuel is `host.watchdog`; any
  other engine error, including a failed instantiation, is `host.trap`.
- The watchdog is fuel, which is counted, not timed: a request stops at the same point
  on every machine.
- `describe` is called once when the reducer is constructed and cached; a failure
  there fails construction.
- The component exports exactly `runspore:machine/reducer@0.1.0`.
- `kernel_digest()` is `digest::hash("kernel", [component bytes])`.

### Workflow artifacts (0.2)

`runspore_wasmtime::load_artifact(bytes)` loads a workflow artifact by the sequence
of [artifact.md](artifact.md) §5 and returns a `runspore_types::artifact::Artifact`:
its digest and bytes, the parsed manifest, and the package bytes from `blob(main)`.
`ArtifactReader` reads any other blob of a stored artifact.

An artifact is untrusted code. It is instantiated with the kernel's engine settings
(deterministic features only, no imports), in a fresh store with its own fuel
watchdog and memory ceiling, and only its `manifest` and `blob` exports are called. A
trap, a watchdog stop or a refused memory growth is `artifact.invalid`; it never
panics the host. Every entry path is checked on load (`artifact.invalid-path`), and
the package bytes are checked against `main` before anything else reads them.

A composed artifact (artifact.md §7) loads the same way: `load_artifact` finds its
`reader` export beside the `reducer` export and reads nothing else. The kernel nested
in it is never run by this host, which has its own; `runspore_build::compose(kernel,
artifact)` is what `spore build --composed` calls with
`runspore_wasmtime::COMPONENT`, the component of §1, as the kernel.

## 2. Engine

`crates/runspore-host`. Constructed from an `Arc<dyn Store>`, an `Arc<dyn Reducer>`,
an `ActivityRegistry`, and an `EngineConfig`. It opens no listener and spawns no
process on its own.

| Config | Default | Meaning |
| --- | --- | --- |
| `tenant` | `default` | Tenant of every run this engine touches |
| `worker_id` | random per process | Attempt owner |
| `lease_ms` | 10000 | Attempt lease |
| `heartbeat_ms` | 3000 | Lease renewal interval |
| `poll_ms` | 200 | Idle sleep in `serve` |
| `shutdown_grace_ms` | 10000 | How long `serve` waits for in-flight attempts on shutdown |
| `max_concurrent_activities` | 4 | In-flight attempts |
| `turns_per_run` | 32 | Transitions per run per tick |
| `limits` | `Limits::default()` | Frozen kernel budget |
| `base_dir` | current directory | Base for relative `cwd` of command actions. **0.2:** `start` records it as the run's base folder, and dispatch uses the run's recorded folder; it serves only runs that have none |
| `db_path` | none | **0.2.** Absolute path of the database, given to every program the host starts as `RUNSPORE_DB`; unset when none |
| `run_ids` | the operating system's random source | Where the ID of each new run comes from; tests give a fixed sequence |
| `stop_grace_ms` | 5000 | **0.2.** Time between SIGTERM and SIGKILL when a process group is stopped (§3) |
| `guard` | the host's own executable | **0.2.** The executable whose guard mode runs in front of every process (§3) |

### Client operations

- `start(workflow_json, input, start_key)`: a start key names a run only within one
  store. Starting twice with one key gives one run; the same key in another store gives
  another run, which shares no run ID, invocation ID or effect key with the first.
  1. `find_run(tenant, start_key)`. The run ID is the ID of the run found, else a new
     random `run_<32 lowercase hex>`: 128 bits from `run_ids`.
  2. Build the `run.started` evidence with that run ID; canonicalize the workflow;
     reject it if any action's `kind` has no registered runner or the runner's
     `validate` fails; run a dry `run.started` transition and reject on failure.
     **0.2:** then the checks of `validate_workflow` below, and, when the package
     carries `inputSchema`, check `input` against it ([schema.md](schema.md) §4):
     input that does not fit is refused with `invalid-input` / `input.invalid` and
     the errors, and nothing is created. A package without `inputSchema` accepts any
     input.
  3. `create_run` with request ID `start/<start_key>` and, in 0.2, `base_dir` as the
     run's base folder. `applied` is a new run;
     `duplicate` is the run found, not new; `run.exists` (the key names a run with
     another workflow or input) is returned as the error. `run.start-key-mismatch`
     means another start with this key created its run after step 1: go back to
     step 1 once, which finds that run.
  4. Return the run key and whether it was new.

  0.1 derived the run ID from the key, `ids::run_from_start_key(tenant, start_key)`, so
  the same key in two stores gave the same run ID and the same effect keys. Runs
  created that way keep their IDs; step 1 finds them by key like any other run.
- **0.2.** `start_artifact(artifact, input, start_key)`: `start` with the package of
  a loaded artifact. `create_run` carries the artifact's digest and bytes, so the
  store keeps the artifact and the run records its digest (store.md §2). A run found
  by its start key keeps the artifact it was first started from.
- `signal(key, message_id, name, outcome, data)`: `append_event` with event ID
  `ids::event_for_signal(message_id)`. **0.2:** first, the data is checked against
  the run's package: against the schema each await-signal node waiting for `name`
  declares for the signal's outcome (`received` when it names none). W13 makes those
  schemas equal, so the check has one answer, and it holds even when the signal
  arrives before the run reaches the node.
- `resolve(key, request_id, invocation_id, resolution)`: `append_event` with event ID
  `ids::event_for_resolution(request_id)`. **0.2:** a `complete` resolution's output
  is first checked against the schema the invocation's node declares for its outcome.
- **0.2: the output check of a client operation.** Data that does not fit its schema
  ([schema.md](schema.md) §3) is refused with `invalid-input` / `output.invalid`, the
  outcome and the errors, before anything is appended: nothing is recorded, a held
  attempt stays held and the run does not move, so the caller can correct the data
  and send it again. A missing `data` or `output` is `null`. A run that does not
  exist is reported by `append_event` as before. In 0.1 nothing was checked.
- `validate_workflow(reducer, activities, limits, workflow_json)`: the checks of
  `start` up to and including the dry transition, with no store involved. Returns the
  canonical package bytes. `start` uses it. **0.2:** after the dry transition, every
  schema in a node's `outputSchemas` is checked against the subset, its depth counted
  from the package root ([schema.md](schema.md) §1); one outside it is
  `workflow.invalid-schema`, naming the node, the outcome and the keyword's path. The
  package's `inputSchema`, when it has one, is checked the same way and for its
  object root (schema.md §4); the error names `inputSchema`. `validate_workflow` has
  no input to check: only `start` checks one.
- `release(key)`: `release_run`, with a request ID derived from the quarantine it
  lifts, so repeating it is a duplicate.
- Read-through views: `get_run`, `get_package`, `get_artifact` (0.2), `list_runs`,
  `list_events`, `list_invocations`.

Request IDs for these are derived from their idempotency key (start key, message ID,
request ID), so a client retry is a duplicate, not a second event.

### tick

One bounded pass. It never blocks on an activity.

1. **Sweep.** For each `Lease` item of `scan_due`: `expire_attempt`. `lease.lost` and
   `lease.not-due` mean another actor got there first; ignore. **0.2:** an item that
   carries a process record is first settled by the check before a lease expiry (§3),
   which may stop a leftover group, mark the expiry `processUnchecked`, or leave the
   attempt for the next pass. `request.digest-mismatch` means another sweeper expired
   the attempt with the other mark; ignore.
2. **Coordinate.** For each key of `scan_ready`, up to `turns_per_run` times:
   `load_turn`; fetch the package (cached by digest); call `reducer.transition`;
   - on a decision: `commit_turn` with request ID
     `ids::commit_request(revision + 1, event_id)`. `stale`, or a `duplicate` receipt,
     means another coordinator won: drop the decision and move on. `conflict` /
     `request.digest-mismatch` means another coordinator committed a different
     decision for the same event (version skew or a nondeterministic reducer): drop
     the decision and count it in the tick report as `diverged`. `unavailable` or
     `unknown-commit`: retry the identical request. A turn counts only when the
     receipt is `applied`.
   - on a failure: `quarantine_run` with its code and details.
   The decision's commands are **never** acted on directly. Only committed work,
   read back through `scan_due`, is dispatched.
3. **Dispatch.** For each `Outbox` item, while fewer than `max_concurrent_activities`
   attempts are in flight: `claim_attempt`, then run the attempt as a background task.

`serve(shutdown)` repeats `tick`, sleeping `poll_ms` when a tick did nothing. On
shutdown it stops claiming and waits up to `shutdown_grace_ms` for in-flight attempts.
It then signals the rest to stop and waits once more; an attempt whose runner returns
is finished normally with what the runner reported (a stopped command reports
`unknown`), so the next worker does not wait for a lease. Attempts that still have not
returned are aborted and left to lease expiry. A final pass then sweeps and coordinates
without dispatching, so results recorded during the drain are applied before the
process exits. The same sequence is available as `shutdown()`; after it the engine
never claims again.

`run_until_parked(key)` ticks until that run is terminal, `waiting`,
`needs-intervention`, or quarantined, and nothing of it is in flight.

### One attempt

1. `claim_attempt`. If the receipt is `duplicate`, heartbeat once and proceed only if
   that heartbeat is `applied`. A historical claim is not ownership.
2. Resolve the action from the package by `action_id`; pick the runner by `kind`. Find
   the visit number from the invocation's identity (kernel §1): the visit `v` of the
   claimed node, at most the workflow's `limits.maxVisitsPerNode`, for which
   `ids::invocation(tenant, runId, "<nodeId>/<v>")` is the claimed invocation ID. The
   host never reads the kernel's snapshot. If there is none, report `Failure`,
   `host.invocation-unknown`, not retryable.
3. Start a heartbeat every `heartbeat_ms`. If a heartbeat returns `lease.lost`, the
   lease is gone: signal the runner to stop, do not call `finish_attempt`, and store
   what is known through `record_late_evidence`.
4. Run the activity. Map its output to an `ActivityResult`:

   | Runner output | `status` | Other fields |
   | --- | --- | --- |
   | `Success { outcome, output }` | `success` | outcome, output |
   | `Failure { error, retryable }` | `failure` | error, retryable |
   | `Unknown { error }` | `unknown` | error |

   **0.2:** before that, a `Success { outcome, output }` from any runner is checked
   against the schema the claimed node declares for `outcome` ([schema.md](schema.md)
   §3); for a `command`, after its undeclared-outcome check (§3). Output that does not
   fit becomes `Failure` with error `output.invalid`, details
   `{"outcome", "errors"}`, not retryable: the same output would come back from
   another try. The kernel then takes the node's `failed` route, or the step is stuck
   (kernel §7.4). An outcome with no schema is not checked. The same holds for a
   result recorded as late evidence.

5. `finish_attempt` with request ID `finish/<attemptId>`. On `unavailable`, retry the
   identical request. On `lease.lost`, `record_late_evidence`.

The host does not look at the action's `effect`. Reporting `unknown` honestly is its
whole job; the kernel decides what an unknown outcome means.

### Failpoints

| Name | Position |
| --- | --- |
| `host.turn.after-reduce` | decision computed, before `commit_turn` |
| `host.turn.after-commit` | `commit_turn` returned |
| `host.attempt.after-claim` | claim held, activity not started |
| `host.attempt.after-effect` | activity returned, before `finish_attempt` |
| `host.attempt.after-finish` | `finish_attempt` returned |

## 3. Activities

```rust
#[async_trait]
pub trait ActivityRunner: Send + Sync {
    fn validate(&self, action_id: &str, action: &Value) -> Result<(), String>;
    async fn run(&self, ctx: &ActivityContext, action: &Value, input: &Value) -> ActivityOutput;
}
```

`ActivityContext` carries the run key, node ID, action ID, invocation ID, attempt
number, visit number, effect key, and a stop signal that fires when the lease is lost or the engine
shuts down. **0.2:** it also records the attempt's process record (§3,
`record_process`), which the engine stores with `set_process`. The registry maps an
action's `kind` to a runner.

### `command`

```json
{"kind": "command", "argv": ["cargo", "test"], "cwd": ".", "env": {"CI": "1"},
 "timeoutMs": "600000", "output": "text", "exitOutcomes": {"1": "red"},
 "outcomes": ["ok", "red"], "effect": "read-only"}
```

- `argv` is executed directly, never through a shell. `cwd` is resolved against
  the run's base folder (**0.2**; 0.1: `base_dir`), recorded at `start`, so every
  dispatch of a run uses one folder whichever engine runs it. A run with no recorded
  folder uses `base_dir`. The child inherits the engine's environment plus `env`.
- **0.2: `cwd` from input.** `cwd` is a string, as in 0.1, or
  `{"fromInput": "<JSON Pointer>"}` (RFC 6901). That form reads the folder from the
  node's evaluated input at dispatch, the value the command gets on stdin, so a
  workflow maps a folder an earlier step returned with a plain `$get`. A relative
  value resolves against the run's base folder like a string `cwd`; an absolute one
  is used as it is. A pointer that names nothing, a value that is not a string, or a
  path that is not an existing folder is `Failure`, `command.cwd-invalid`, not
  retryable, details `{"pointer"}` plus `"cwd"` when the value was a string: another
  try reads the same input, so the step is stuck or takes its `failed` route
  (kernel §7.4). Nothing is started. A probe's, watch's or stop's `cwd` takes the same
  two forms and reads the same node input. The kernel never reads `cwd`. The run's
  records stay in the database where it started: the command gets `RUNSPORE_DB`
  (below) even when its folder holds a `.spore/` of its own.
- Stdin receives the canonical input JSON, then EOF.
- The child also gets `RUNSPORE_RUN_ID`, `RUNSPORE_NODE_ID`, `RUNSPORE_INVOCATION_ID`,
  `RUNSPORE_ATTEMPT`, `RUNSPORE_VISIT`, and `RUNSPORE_EFFECT_KEY`. The effect key is
  identical on every attempt of one invocation; an idempotent command passes it to
  whatever it calls. `RUNSPORE_VISIT` is the visit number of the invocation's
  activation (`<nodeId>/<visit>`, kernel §1), so a command can tell a first visit of
  its node from a return to it.
- **0.2:** with `db_path` set, every program the host starts gets `RUNSPORE_DB` set to
  it, after `env`, so a workflow cannot point it elsewhere. A `spore` call from inside a
  step reaches the run's own database (cli.md §Project folder).
- **0.2** (replaces 0.1's "It runs in its own process group. On timeout or stop the
  whole group is killed."): it runs under a guard, below.
- A command never learns the action's outcome names. The workflow maps its result to
  an outcome (`exitOutcomes`, or result mode below).

Result mapping, in order:

| What happened | Runner output |
| --- | --- |
| **0.2.** A `cwd` from input that names no existing folder | `Failure`, `command.cwd-invalid`, not retryable, details `{pointer, cwd?}` |
| Could not be spawned, or its program could not be run | `Failure`, `command.spawn`, retryable |
| Killed by a signal it did not get from the engine, or its guard ended without reporting | `Unknown`, `command.signaled` |
| Timed out, or stopped by the engine | `Unknown`, `command.timeout` or `command.stopped` |
| An exit code listed in `exitOutcomes` | `Success` with the mapped outcome |
| Exit 0, `output` other than `result` | `Success` with outcome `ok` |
| Exit 0, `output: "result"` | What stdout says (below) |
| Any other exit code | `Failure`, `command.exit`, retryable, details `{exitCode, stderrTail}` (the last 4 KiB of stderr) |
| Stdout over 64 KiB, `output` other than `text` | `Failure`, `command.output-too-large`, not retryable |
| `output: "json"` and stdout is not JSON in the canonical domain | `Failure`, `command.malformed-output`, not retryable |
| `Success` whose outcome the action does not declare | `Failure`, `command.undeclared-outcome`, not retryable, details `{outcome}` |

The last row holds for every `command` in every output mode, so an undeclared outcome
never reaches the kernel. An action that declares no `outcomes` declares `["ok"]`
(kernel §2).

Success output by `output` mode:

- `text` (default): `{"exitCode": n, "stdout": "…", "stderr": "…", "truncated": b}`,
  each stream decoded as UTF-8, lossy. When the two streams together exceed 64 KiB,
  each keeps its end: stderr its last 16 KiB at most, stdout the last bytes that fit in
  the 64 KiB left beside it. A cut falls on a UTF-8 character boundary. `truncated` is
  `true` when either stream was cut. Text mode never fails with
  `command.output-too-large`: a long log reaches the next step as its end, where test
  runners and compilers print their failures.
- `json`: the parsed stdout value.
- `result`: the output stdout names (below). An exit code listed in `exitOutcomes`
  gives `null`.
- `none`: `null`.

**Result mode.** With `"output": "result"`, a command names its own result. On exit 0,
stdout must be canonical-domain JSON of one of two shapes:

| Stdout | Runner output |
| --- | --- |
| `{"outcome": "<o>", "output": <json>}` | `Success { outcome, output }` |
| `{"error": {"code": "<c>", "message": "<m>"}, "retryable": <bool>}` | `Failure { error, retryable }` |

`error` may also carry `details`, any JSON. Any other stdout on exit 0 (other keys,
wrong types, not JSON) is `Failure`, `command.malformed-output`, not retryable. A
non-zero exit maps through the table above as in every mode, so `exitOutcomes` still
applies to it; a crash that exits 1 is then `command.exit`, never an outcome. These are
the shapes an async `watch` prints.

`validate` checks the shape of every field above, that every `exitOutcomes` value is a
declared outcome, and, in result mode, that `exitOutcomes` does not map 0. A `cwd`
object has the one key `fromInput`, holding a JSON Pointer: empty, or `/` and its
steps, each `~` followed by `0` or `1`.

0.1 had no `result` mode, no `RUNSPORE_VISIT`, no `cwd` from input, and no
undeclared-outcome or `command.cwd-invalid` row. Its
`text` mode gave `{"exitCode": n, "stdout": "<utf-8, lossy>"}` and failed with
`command.output-too-large` when stdout passed 64 KiB, like the other modes.

### The guard (0.2)

Every process the host starts for an attempt runs under a guard. In 0.2 that is a
`command`'s `argv`.

- The guard is a hidden mode of the host's own executable (config `guard`), so there
  is nothing extra to install. It is never named in a workflow and no action opts out.
- The host starts the guard with two pipes: a control pipe the host writes to and a
  report pipe it reads. The guard moves to a session and process group of its own,
  apart from the host's and from the command's, so a signal sent to either group never
  reaches it.
- The guard starts the command's process as the leader of a new process group, with
  default signal dispositions and an empty signal mask. That process waits until the
  host says go; only then does it run the program. The guard reports the process ID
  first, so the host knows the group before anything runs. If the host's end of the
  control pipe closes before go, the guard kills the waiting process and exits: no
  program runs that the host does not know about.
- The command inherits the standard input, output and error pipes directly. The guard
  reports how the command ended (its exit code, or the signal that ended it).
- The guard stops the command's group when the host asks (timeout, lease lost,
  shutdown) and when the host's end of the control pipe closes without a stop: the host
  died, for any reason, including SIGKILL. A dead host's try records nothing; its lease
  runs out as in 0.1 and the sweep expires it.

**Stopping a group** is SIGTERM to the group, a wait of up to `stop_grace_ms` for the
group to be gone, then SIGKILL. It is used on timeout, on stop (lease lost, shutdown)
and when the guard sees the host die, so a command can save its state before it goes.
With the defaults, a dead host's group is gone (5 s at most) well before its lease runs
out (10 s), so the next try never starts beside it.
On a normal ending (the command's process exited) the rest of its group is killed at
once, as in 0.1, so nothing the command left behind outlives it.

A command that starts its own process group or session escapes the guard, as it
escapes 0.1's timeout kill.

### The process record (0.2)

An attempt keeps a **process record** of the process group doing its work, so a worker
that comes later can stop it even when its worker and its guard are both gone. A step
therefore never runs twice at once on one machine.

- The record is `{"host", "boot", "pgid", "started"}`: the machine's host name, an
  identifier of its boot (`/proc/sys/kernel/random/boot_id` on Linux, `kern.boottime`
  on macOS), the process group ID (the ID of the process the guard started, which leads
  the group), and that process's start time as a decimal string in the operating
  system's own unit. The start time is read before the program runs and does not change
  when it does.
- Before a `command` is let run, the host records its process record with
  `set_process` (store §2). A failure to record is `Failure`, `command.spawn`,
  retryable; the guard then kills the waiting process, so no program runs that the
  store does not know about.
- Once the command has ended and the guard has stopped its group, the host clears the
  record (`set_process` with null). A failure to clear is ignored. When the guard ended
  without reporting, the record is kept.
- Only the processes that do a step's work are recorded.

**The check before a lease expiry.** For a `Lease` item that carries a process record,
the sweep settles the record before `expire_attempt`:

| What the record names | The sweep |
| --- | --- |
| Another host name | Expires, with `processUnchecked` (the audit record says `process.unchecked`) |
| This host, another boot | Expires: the group is gone |
| This host and boot, a group that is not ours, or none | Expires |
| This host and boot, our group | Stops the group, waits until no process of it remains, then expires |
| A group the host may not signal (EPERM) or whose members it cannot read | Expires, with `processUnchecked` |

A group is ours when its leader is alive with the recorded start time, or, when it has
lost its leader, when every member started at or after that time: the operating system
does not hand out a group's ID again while any member of it lives. A process with the
group's ID and another start time took the number after our group was gone, and is left
alone. A zombie is not a member: its parent, or init, reaps it. Stopping is the stop
sequence of the guard, and the wait for the group to be gone is bounded by
`stop_grace_ms` plus 1 s. A group still there after that leaves the
attempt unexpired; the sweep tries again on its next pass.

Where the check cannot be made, the host signals nothing. A record from another machine
or another OS user therefore leaves the risk 0.1 had: a leftover there may still run
beside the next try. Several machines on one database are not supported (store §3).

### `native`

`{"kind": "native", "function": "<name>", ...}` calls a Rust closure registered under
that name. For embedding and for tests.

## 4. What recovery is

There is no recovery mode. After a crash, the next `tick` finds the same durable
records any tick finds: pending events are coordinated, due leases are expired, due
invocations are claimed. An attempt that was in flight when the process died holds a
lease nobody renews; when it runs out, the sweep appends an `expired` result and the
kernel either authorizes a new attempt of the same invocation or parks the run in
`needs-intervention`. A resumed run therefore waits up to `lease_ms` before it moves.

## 5. Required evidence

| ID | Scenario | Must hold |
| --- | --- | --- |
| H1 | Two engines on one store tick concurrently through a whole run | The run completes once; every invocation has exactly one accepted result |
| H2 | A commit is lost to a competing coordinator | The loser dispatches nothing from its decision |
| H3 | Lease lost mid-activity | No `finish_attempt`; late evidence stored; the runner was told to stop |
| H4 | Kernel failure (budget exceeded) | Run quarantined, event still pending, no hot loop; `release` resumes |
| H5 | Command runner result table | Every row above, including process-group kill on timeout; both result-mode shapes and a malformed one; a 200 KiB log in text mode arrives as its end with stderr ≤ 16 KiB, cut at UTF-8 boundaries, `truncated: true`; an undeclared outcome in every mode |
| H7 | A node visited twice runs a command each time | `RUNSPORE_VISIT` is 1, then 2: the visit of the invocation's activation |
| H6 | Crash at every host and store failpoint during a multi-step run of a workflow folder, then restart | Final state equals the uninterrupted run's; idempotent steps ran at most `maxAttempts` times; an `unsafe` step interrupted after its effect leaves the run in `needs-intervention`, and resolving it completes the run with the effect performed once |
| H12 | **0.2.** SIGKILL the worker while a command runs | The command's group is gone within `stop_grace_ms` + 1 s; the next try starts only after it |
| H14 | **0.2.** SIGKILL the worker while a command that ignores SIGTERM runs | The guard survives its own SIGTERM to the group and sends SIGKILL after `stop_grace_ms` |
| H15 | **0.2.** SIGKILL the worker while a command that traps SIGTERM runs | The command writes its file before the group goes |
| H13 | **0.2.** SIGKILL the worker and its guard together while the command's group runs, with its leader and without it | The next sweep stops the recorded group before it expires the attempt; the next try starts only after the group is gone |
| H16 | **0.2.** A process record with another host name | The attempt is expired by its lease; the audit record says `process.unchecked` |
| H17 | **0.2.** Output schemas: a command and a native function whose output does not fit; a signal, before and at its node, and a `complete` resolution whose data does not fit; a workflow with a schema outside the subset | The result is `output.invalid`, not retryable, and the step is stuck or takes its `failed` route; the client operations return `output.invalid` with the errors and append nothing; fitting data is accepted; `validate_workflow` refuses the workflow with `workflow.invalid-schema` |
| H19 | **0.2.** A run started by an engine with one `base_dir` is worked by an engine with another and a `db_path` | The command runs in the folder recorded at `start` and sees `RUNSPORE_DB` equal to `db_path`, also when the action's `env` sets it |
| H20 | **0.2.** A step makes a folder holding a `.spore/` of its own and returns its path, absolute or relative; the next step's `cwd` is `{"fromInput": …}` at that path. Then a pointer that names nothing, a value that is not a string, and a folder that does not exist | The second step runs in that folder, and a `spore` call from it reaches the run's database; each invalid case is `command.cwd-invalid`, not retryable, and the step is stuck after one try; `validate` refuses a malformed `cwd` object |
| H21 | **0.2.** Input schema: `start` with input that does not fit the package's `inputSchema`, with input that fits, and with a package whose `inputSchema` root is not an object | The misfit is `input.invalid` with the failing paths and no run exists afterwards; the fit creates the run; `validate_workflow` and `start` refuse the package with `workflow.invalid-schema` |
