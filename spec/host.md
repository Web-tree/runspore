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
| `base_dir` | current directory | Base for relative `cwd` of command actions |
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
  3. `create_run` with request ID `start/<start_key>`. `applied` is a new run;
     `duplicate` is the run found, not new; `run.exists` (the key names a run with
     another workflow or input) is returned as the error. `run.start-key-mismatch`
     means another start with this key created its run after step 1: go back to
     step 1 once, which finds that run.
  4. Return the run key and whether it was new.

  0.1 derived the run ID from the key, `ids::run_from_start_key(tenant, start_key)`, so
  the same key in two stores gave the same run ID and the same effect keys. Runs
  created that way keep their IDs; step 1 finds them by key like any other run.
- `signal(key, message_id, name, outcome, data)`: `append_event` with event ID
  `ids::event_for_signal(message_id)`.
- `resolve(key, request_id, invocation_id, resolution)`: `append_event` with event ID
  `ids::event_for_resolution(request_id)`.
- `validate_workflow(reducer, activities, limits, workflow_json)`: the checks of
  `start` up to and including the dry transition, with no store involved. Returns the
  canonical package bytes. `start` uses it.
- `release(key)`: `release_run`, with a request ID derived from the quarantine it
  lifts, so repeating it is a duplicate.
- Read-through views: `get_run`, `list_runs`, `list_events`, `list_invocations`.

Request IDs for these are derived from their idempotency key (start key, message ID,
request ID), so a client retry is a duplicate, not a second event.

### tick

One bounded pass. It never blocks on an activity.

1. **Sweep.** For each `Lease` item of `scan_due`: `expire_attempt`. `lease.lost` and
   `lease.not-due` mean another actor got there first; ignore.
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
2. Resolve the action from the package by `action_id`; pick the runner by `kind`.
3. Start a heartbeat every `heartbeat_ms`. If a heartbeat returns `lease.lost`, the
   lease is gone: signal the runner to stop, do not call `finish_attempt`, and store
   what is known through `record_late_evidence`.
4. Run the activity. Map its output to an `ActivityResult`:

   | Runner output | `status` | Other fields |
   | --- | --- | --- |
   | `Success { outcome, output }` | `success` | outcome, output |
   | `Failure { error, retryable }` | `failure` | error, retryable |
   | `Unknown { error }` | `unknown` | error |

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
number, effect key, and a stop signal that fires when the lease is lost or the engine
shuts down. The registry maps an action's `kind` to a runner.

### `command`

```json
{"kind": "command", "argv": ["cargo", "test"], "cwd": ".", "env": {"CI": "1"},
 "timeoutMs": "600000", "output": "text", "exitOutcomes": {"1": "red"},
 "outcomes": ["ok", "red"], "effect": "read-only"}
```

- `argv` is executed directly, never through a shell. `cwd` is resolved against
  `base_dir`. The child inherits the engine's environment plus `env`.
- Stdin receives the canonical input JSON, then EOF.
- The child also gets `RUNSPORE_RUN_ID`, `RUNSPORE_NODE_ID`, `RUNSPORE_INVOCATION_ID`,
  `RUNSPORE_ATTEMPT`, and `RUNSPORE_EFFECT_KEY`. The effect key is identical on every
  attempt of one invocation; an idempotent command passes it to whatever it calls.
- **0.2** (replaces 0.1's "It runs in its own process group. On timeout or stop the
  whole group is killed."): it runs under a guard, below.

Result mapping:

| What happened | Runner output |
| --- | --- |
| Could not be spawned, or its program could not be run | `Failure`, `command.spawn`, retryable |
| Exit 0, or an exit code listed in `exitOutcomes` | `Success` with outcome `ok` or the mapped name |
| Any other exit code | `Failure`, `command.exit`, retryable, details `{exitCode, stderrTail}` |
| Killed by a signal it did not get from the engine, or its guard ended without reporting | `Unknown`, `command.signaled` |
| Timed out, or stopped by the engine | `Unknown`, `command.timeout` or `command.stopped` |
| Stdout over 64 KiB | `Failure`, `command.output-too-large`, not retryable |
| `output: "json"` and stdout is not JSON in the canonical domain | `Failure`, `command.malformed-output`, not retryable |

Success output by `output` mode: `text` (default) gives `{"exitCode": n, "stdout":
"<utf-8, lossy>"}`; `json` gives the parsed stdout value; `none` gives `null`.
`validate` checks the shape of every field above and that every `exitOutcomes` value
is a declared outcome.

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
| H5 | Command runner result table | Every row above, including process-group kill on timeout |
| H6 | Crash at every host and store failpoint during a multi-step run, then restart | Final state equals the uninterrupted run's; idempotent steps ran at most `maxAttempts` times; an `unsafe` step interrupted after its effect leaves the run in `needs-intervention`, and resolving it completes the run with the effect performed once |
| H12 | **0.2.** SIGKILL the worker while a command runs | The command's group is gone within `stop_grace_ms` + 1 s; the next try starts only after it |
| H14 | **0.2.** SIGKILL the worker while a command that ignores SIGTERM runs | The guard survives its own SIGTERM to the group and sends SIGKILL after `stop_grace_ms` |
| H15 | **0.2.** SIGKILL the worker while a command that traps SIGTERM runs | The command writes its file before the group goes |
