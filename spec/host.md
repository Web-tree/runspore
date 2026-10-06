# Host 0.1

The host is everything around the kernel: it packages the kernel as a component, runs
it, commits its decisions, and executes activities. It contains no graph logic. It
never decides a branch, a retry, or an outcome route.

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

### Client operations

- `start(workflow_json, input, start_key)`: canonicalize the workflow; reject it if
  any action's `kind` has no registered runner or the runner's `validate` fails; run a
  dry `run.started` transition and reject on failure; then `create_run`. The run ID is
  `ids::run_from_start_key(tenant, start_key)`. Returns the key and whether it was new.
- `signal(key, message_id, name, outcome, data)`: `append_event` with event ID
  `ids::event_for_signal(message_id)`.
- `resolve(key, request_id, invocation_id, resolution)`: `append_event` with event ID
  `ids::event_for_resolution(request_id)`.
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
shutdown it stops claiming, waits for in-flight attempts up to a grace period, and
returns; attempts still running are left to lease expiry.

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
- It runs in its own process group. On timeout or stop the whole group is killed.

Result mapping:

| What happened | Runner output |
| --- | --- |
| Could not be spawned | `Failure`, `command.spawn`, retryable |
| Exit 0, or an exit code listed in `exitOutcomes` | `Success` with outcome `ok` or the mapped name |
| Any other exit code | `Failure`, `command.exit`, retryable, details `{exitCode, stderrTail}` |
| Killed by a signal it did not get from the engine | `Unknown`, `command.signaled` |
| Timed out, or stopped by the engine | `Unknown`, `command.timeout` or `command.stopped` |
| Stdout over 64 KiB | `Failure`, `command.output-too-large`, not retryable |
| `output: "json"` and stdout is not JSON in the canonical domain | `Failure`, `command.malformed-output`, not retryable |

Success output by `output` mode: `text` (default) gives `{"exitCode": n, "stdout":
"<utf-8, lossy>"}`; `json` gives the parsed stdout value; `none` gives `null`.
`validate` checks the shape of every field above and that every `exitOutcomes` value
is a declared outcome.

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
