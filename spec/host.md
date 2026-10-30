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
| `max_concurrent_activities` | 4 | In-flight attempts: the worker's slots. **0.2:** `spore` takes it from the project's settings, `worker.slots` (cli.md §Project folder) |
| `turns_per_run` | 32 | Transitions per run per tick |
| `limits` | `Limits::default()` | Frozen kernel budget |
| `base_dir` | current directory | Base for relative `cwd` of command actions. **0.2:** `start` records it as the run's base folder, and dispatch uses the run's recorded folder; it serves only runs that have none |
| `db_path` | none | **0.2.** Absolute path of the database, given to every program the host starts as `RUNSPORE_DB`; unset when none |
| `run_ids` | the operating system's random source | Where the ID of each new run comes from; tests give a fixed sequence |
| `stop_grace_ms` | 5000 | **0.2.** Time between SIGTERM and SIGKILL when a process group is stopped (§3) |
| `guard` | the host's own executable | **0.2.** The executable whose guard mode runs in front of every process (§3) |
| `files_dir` | none | **0.2.** Folder of step files and work folders (§3, Step files and work folders). Required for a workflow artifact with `instructions` or `asset` entries |
| `parent` | none | **0.2.** The run and node of the step this engine is started from, `{run_id, node_id}`; `start` records it as the parent of every run it creates. `spore` sets it from `RUNSPORE_RUN_ID` and `RUNSPORE_NODE_ID` (cli.md, `list`) |
| `max_watches` | 16 | **0.2.** Watches the engine runs at once (§3, Async commands), apart from `max_concurrent_activities`. `spore` takes it from `--max-watches` |
| `disk_floor` | 1 GiB | Free bytes the disk of `files_dir`, else of `base_dir`, must keep for a step to be claimed (Disk pressure, below); 0 never pauses. `spore` takes it from the project's settings, `worker.diskFloorMb` |
| `disk` | the operating system's `statvfs` | How that disk's free space and mount point are read; tests give their own |

### Client operations

- `start(workflow_json, input, start_key)`: a start key names a run only within one
  store. Starting twice with one key gives one run; the same key in another store gives
  another run, which shares no run ID, invocation ID or effect key with the first.
  1. `find_run(tenant, start_key)`. The run ID is the ID of the run found, else a new
     random `run_<32 lowercase hex>`: 128 bits from `run_ids`.
  2. Build the `run.started` evidence with that run ID; canonicalize the workflow;
     reject it if any action's `kind` has no registered runner or the runner's
     `validate` fails (**0.2:** or the action asks for anything else this host does
     not offer: The handshake, below); run a dry `run.started` transition and reject
     on failure.
     **0.2:** then the checks of `validate_workflow` below, and, when the package
     carries `inputSchema`, check `input` against it ([schema.md](schema.md) §4):
     input that does not fit is refused with `invalid-input` / `input.invalid` and
     the errors, and nothing is created. A package without `inputSchema` accepts any
     input.
  3. `create_run` with request ID `start/<start_key>` and, in 0.2, `base_dir` as the
     run's base folder, `parent` as its parent, and the result channel picked for each
     async action (The handshake, below). `applied` is a new run;
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
  by its start key keeps the artifact it was first started from. An artifact with an
  `instructions` or `asset` entry is refused with `host.files-dir-missing`, before
  anything is stored, when `files_dir` is unset, whatever its action kinds: its steps
  could not be given their files.
- `signal(key, message_id, name, outcome, data)`: `append_event` with event ID
  `ids::event_for_signal(message_id)`. **0.2:** first, the data is checked against
  the run's package: against the schema each await-signal node waiting for `name`
  declares for the signal's outcome (`received` when it names none). W13 makes those
  schemas equal, so the check has one answer, and it holds even when the signal
  arrives before the run reaches the node. **0.2:** before that, when the run's
  committed state is `waiting` at an await-signal node whose `signal` is `name` and
  no event is pending, the signal's outcome must be one the node routes, as *consume*
  decides (kernel.md §7.4: not `exhausted` in a 0.2 workflow). Otherwise it is refused
  with `invalid-input` / `signal.outcome-unrouted`, details `{nodeId, signal, outcome,
  outcomes}` (the outcomes the node accepts), before anything is appended. A signal
  sent while the run does not wait for it is appended as before, and the kernel may
  drop it later with the diagnostic of the same code.
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
- **0.2.** `cancel(key, request_id, reason)`: ends an unfinished run as `cancelled`
  (store.md §2 `cancel_run`), with request ID `cancel/<request_id>`; the same request
  ID with another reason is `conflict` / `request.digest-mismatch`. The store expires
  every running attempt of the run with reason `cancelled`, so:
  - a worker running a command of the run gets `lease.lost` at its next heartbeat and
    stops the command's group as on any lost lease (One attempt, below): the command
    stops within `heartbeat_ms` plus `stop_grace_ms`, and what it reports is kept as
    late evidence;
  - for each expired attempt that was held outside the worker (it has a deadline)
    and whose action declares `async.stop`, the host then runs that stop once (§3,
    Async commands) and keeps its output with `record_late_evidence`; its failure is
    ignored. The stop runs only when the cancel is `applied`: a replay, a second
    cancel, and a cancel of a run that has already ended run nothing.

  Returns the run's status, its cancel record, the disposition, and for each stop that
  ran, the invocation, the attempt number and how the stop exited. A cancelled run
  parks `run_until_parked` (below). A run that does not exist is `run.not-found`.
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
   the attempt with the other mark; ignore. **0.2:** with `files_dir` set, the sweep
   also removes step files no unfinished run needs (§3, Step files and work folders),
   at most once every 30 s per engine, starting with its first tick. **0.2:** at the
   same pace, starting with its first tick, it looks after every held async attempt
   of the tenant, whoever started it (§3, Async commands): it probes the attempt when
   its action declares a probe, and starts a watch when its action declares a watch
   and this engine does not watch it yet.
2. **Coordinate.** For each key of `scan_ready`, up to `turns_per_run` times:
   `load_turn`; fetch the package (cached by digest); call `reducer.transition` with
   the semantics the run's snapshot records, not the kernel's default, so a newer
   kernel continues an older kernel's runs (kernel, Versions). A run's first turn,
   with no snapshot, takes the semantics `describe` reports. The arrow `status` shows
   is found by replaying the run's events under the same recorded semantics;
   - on a decision: `commit_turn` with request ID
     `ids::commit_request(revision + 1, event_id)`. `stale`, or a `duplicate` receipt,
     means another coordinator won: drop the decision and move on. `conflict` /
     `request.digest-mismatch` means another coordinator committed a different
     decision for the same event (version skew or a nondeterministic reducer): drop
     the decision and count it in the tick report as `diverged`. `unavailable` or
     `unknown-commit`: retry the identical request. A turn counts only when the
     receipt is `applied`. **0.2:** an `applied` turn that moved the run into
     `needs-intervention` or `failed` starts the workflow's `onIntervention` action
     (Telling a person, below).
   - on a failure: `quarantine_run` with its code and details.
   The decision's commands are **never** acted on directly. Only committed work,
   read back through `scan_due`, is dispatched.
3. **Dispatch.** For each `Outbox` item, while fewer than `max_concurrent_activities`
   attempts are in flight: `claim_attempt`, then run the attempt as a background task.
   **0.2:** the items are taken in _queue order_: the highest `priority` of their
   action first (dot.md §4, default 0), and among equals in `scan_due` order, the
   one due longest first. Temporal's task queues hand out tasks in the order they
   were scheduled; Kubernetes schedules a pod of a higher priority class first.
   0.1 took them in `scan_due` order alone.
   **0.2:** an invocation of an `external` action is skipped: an actor claims it.
   **0.2:** so is one whose action this host falls short of (The handshake, below):
   nothing is claimed, and the step is `capability-missing` until a host that has the
   capability serves the run. 0.1 claimed it and failed the attempt with
   `host.runner-missing`, not retryable.
   **0.2:** a due invocation left unclaimed because every slot is taken is _queued_:
   it waits for a worker with a free slot. `held_slots(now)` lists the attempts that
   take a slot, those a worker claimed at a step that is not `external`, unfinished
   and with a lease that has not run out, each with its run, node, attempt, worker,
   claim time and the pid of its program; `spore status` shows them (cli.md,
   `slots`). **0.2:** `cut_off(invocation, now)` says whether such a claimed try is
   cut off, and why: on the machine its process record names, its group, its worker
   or the boot it started in is gone, looked at as the check before a lease expiry
   does (§3) but stopping nothing; or its lease has run out. `spore status` and `list`
   show it (cli.md, What a run needs).
   **0.2:** with `files_dir` set, before the claim, the host makes or checks the
   step's files and its work folder (§3). When it cannot, it claims nothing and quarantines the run instead, so
   no attempt is spent and no lease runs out on a try that never started.
   While the disk is full (Disk pressure, below), nothing is claimed, and a step whose
   files cannot be written then is not quarantined.

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
`needs-intervention`, or quarantined, and nothing of it is in flight. **0.2:** or
cancelled, by any process, once its in-flight attempts have stopped. **0.2:** or at
an async step (Steps outside the worker, below) that this engine does not watch, or
whose watch has said that its holder waits for a person (§3, Async commands).
**0.2:** its ticks coordinate and dispatch that run only; the sweep still covers every
run. Another run takes no turn and has no attempt claimed, so a caller that exits when
its run parks leaves no other run's try cut off, or claimed and never started. As in
Temporal's worker shutdown, a worker stops taking work before it lets go, and never
fails work it did not start.
**0.2:** "that run" is the run with the runs below it: the runs it started (their
recorded parent is the run, cli.md §Runs that start runs), and theirs, looked up again
on every tick so a run started meanwhile joins. Their ticks coordinate and dispatch
all of them, and it returns only when nothing of them is in flight and each of them
has stopped as above: a parent waiting for the signal its child sends goes on while the
child can be worked, as Temporal's worker runs a workflow's child workflows with it.
Once the run itself has ended, nothing below it is taken up anew, but the runs below
still take turns, so a step of theirs that finishes is applied; it returns when nothing
of them is in flight and none of them has an event left to apply.

**0.2:** `follow(key)` ticks as `run_until_parked` does, but goes on past a held async
step whose holder does not wait for a person: it returns only when the run is
terminal, `waiting`, `needs-intervention` or quarantined, or at an async step that
needs someone: a ready external step, a step this host falls short of (The
handshake, below), or a held step whose watch has said that its holder waits for a
person, or (0.2) whose attempt carries a needs-person mark it did not carry when
`follow` began, by its attempt and `sinceMs`. A mark `follow` found when it began does
not stop it, so a caller that follows again right after telling the person waits for
the next change instead of returning at once. It works the runs below the run as `run_until_parked` does, each of them
stopping as the run does. While the step is held it keeps ticking, sleeping `poll_ms` after a tick that
did nothing, so a report from elsewhere is applied within one poll; its sweep expires
the attempt at its lease or deadline, and probes and watches it at most every 30 s
(§2 `tick`). `async_step` reports a held step whose watch said so with that watch's
message.

**0.2:** shutdown stops every watch the engine runs with its process group; the
attempts they watched stay held for the next worker.

**0.2: progress.** `observe(observer)` gives the engine a callback it calls with each
change of each run it commits from then on, in commit order: after every `applied`
`commit_turn`, the changes `progress::changes` finds from the snapshot before the
turn, the snapshot after it and the event it consumed (a step started, a try failed,
an outcome and its arrow, waiting, needs intervention, completed, failed), and after
every `applied` `quarantine_run`, the quarantine. Each change carries the run key, its
start key and the wall-clock time. The changes are read from committed state only, so a
reader of the store finds the same ones by replaying a run's events. An engine has at
most one observer; without one it finds no changes. `spore` prints them (cli.md,
Progress).

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

The host does not look at the action's `effect`, but to set aside a try that ended on a
full disk (Disk pressure, below). Reporting `unknown` honestly is its whole job; the
kernel decides what an unknown outcome means.

### Disk pressure

A try that fails because its machine is out of disk says nothing about the step. Like
a kubelet that marks its node unhealthy on disk pressure and takes no new pod, the
engine stops taking steps while its disk is full.

`Engine::disk_full()` reads the free space of the disk holding `files_dir`, else
`base_dir` (its nearest folder that exists), and its mount point, the topmost folder
above it on the same device. The disk is **full** while the free space, the bytes a
process that is not root may still write, is under `disk_floor`; it returns the free
space, the mount and the floor, shown `disk full (<free> free on <mount>)`. A disk
whose space cannot be read is not full. Then:

1. **Dispatch** claims no invocation (§2 `tick`), and tells the observer `paused` once
   per run per pause. A step whose files cannot be written while the disk is full is
   left due rather than quarantined; out of space with the free space over the floor
   (out of inodes, say) is still `host.files-unwritable`.
2. **A try that ends** while the disk is full, by its runner's return with any output
   but a holder handle, and with its lease kept and the engine not shutting down, is
   not reported. The engine tells the observer `paused` at the node. When the
   action's effect is repeatable, the engine keeps heartbeating the attempt, checks
   the disk every `poll_ms`, and once it is no longer full runs the try again in the
   same attempt: no attempt is spent and no result is recorded for the try set aside.
   A lost lease or a shutdown meanwhile ends the wait, and the attempt is left to its
   lease. An `unsafe` action's try is reported `Unknown`, code `host.disk-full`,
   details `{"freeBytes", "floorBytes", "mount"}`: the kernel asks a person (kernel
   §7.4), and never repeats an unsafe step itself.


### Steps outside the worker (0.2)

An **external step** is an `activity` node bound to an action of kind `external`. Its
work is done by an **actor** outside the worker, a person or a program the workflow
does not start, who claims the step's attempt, does the work, and reports the result.
The worker never claims or runs it. The kernel schedules its invocation like any
other, since it reads only an action's `outcomes`, `effect` and `retry` (kernel §2);
attempts, fences, effect classes, retries and stuck steps apply unchanged.

```json
{"kind": "external", "actor": "reviewer", "effect": "unsafe",
 "outcomes": ["approved", "changes"],
 "async": {"timeoutMs": "7200000", "leaseMs": "600000"}}
```

- `actor`: optional, `[a-z0-9][a-z0-9.-]{0,63}`, an **actor label**. A claim that
  names a label takes only steps with that label; a claim that names none takes only
  unlabelled steps, so a generic claimer never takes a step meant for someone else.
  The label is a filter, not a permission: whoever can write the database can claim
  any label.
- `async`: optional; absent means every default.
  - `timeoutMs`: a positive decimal string, default `"3600000"` (1 hour). Every claim
    sets the attempt's deadline that long after it, and no renewal passes it.
  - `leaseMs`: a positive decimal string smaller than `timeoutMs`, optional. When set,
    the holder must renew within it (`renew_step`); when absent, nobody renews and the
    deadline alone bounds the attempt, since a person working by hand cannot.

  - `probe` and `watch`: optional, the programs that look after the holder, as for an
    async command (§3, Async commands): the host probes a held external step and
    watches it the same way, with the holder handle given at the claim.
  - `channels`: the result channels the actor may report through, in order of
    preference, default `["store"]`, as for an async command (The handshake, below).
  - `stop`: optional, the program that tells the holder to stop when the run is
    cancelled (`cancel`, above), run as for an async command (§3, Async commands).

  This host refuses the `component` form of `probe`, `watch` and `stop` (The
  handshake).

`validate_workflow` and `start` check these fields and refuse a bad one with
`action.invalid`, like a runner's `validate`. An external action needs no runner.

An **async step** is an external step, or an async command or a `component` action
(§3) whose start has handed over its holder handle. Both are held outside the worker until a result
comes back through a result channel; the rules below that name an async step hold for
both.

**Dispatch.** `tick` never claims an invocation of an external action: dispatch reads
the action's kind from the run's package first and leaves it pending for an actor.
The sweep treats its attempts like any other: once the lease or the deadline has
passed, `expire_attempt` with reason `lease` (store §2), and the kernel retries a
repeatable effect or parks an `unsafe` one in `needs-intervention` (kernel §7.3).

**Waiting on a step.** A run is **at an async step** when it is `running`, has no
pending event, and its outstanding invocation (kernel §3) is `scheduled` with an
external action, or with an async command or a `component` action whose attempt is held (its holder handle
recorded by `hold_attempt`). An external step is `ready` while that invocation is
pending and `held` while an attempt of it runs; an async command's step is only ever
`held`. `run_until_parked` also returns there, since the worker has nothing to do until
an actor or the work reports or the attempt expires, unless it watches the attempt.
`Engine::async_step(view)` describes it: the action's kind (`external`, `command` or
`component`)
and actor, `ready` or `held`, the attempt number, and while held the holder handle,
`deadlineMs`, `leaseUntilMs` and (0.2) the attempt's needs-person mark, `{reason,
sinceMs}` or none. It never shows the input, the owner or the fence.
**0.2:** it also describes a step that is `capability-missing` (The handshake).

**Describing a step (0.2).** `Engine::describe_step(view)` describes the step an
unfinished run is at, an activity or an await-signal node, without claiming, probing
or recording anything: its node, kind and activation, its action or signal, its
outcomes and `outputSchemas`, its step files and instructions file (§3), and the arrow
into it. It checks, or makes again, the step files as a dispatch does; when they cannot
be made it returns the code a dispatch would quarantine with, and leaves the run as it
is. It is `None` for a finished run or any other node.

**Client operations.** Each takes or returns a **token**: the claimed attempt's
reference (run, invocation, attempt number, owner and fence) as one string. Only its
bearer can renew, complete or fail the attempt. An async command's start command gets
its attempt's token as `RUNSPORE_TOKEN` (§3), so the work it starts reports the same
way.

- `probe_step(key)`: when the run is at a held async step whose action declares a
  probe, runs the probe (§3, Async commands); a `dead` answer expires the attempt with
  reason `probe-dead`. Returns the answer, or none when nothing was probed. `status`
  calls it unless asked not to (cli.md).
- `claim_step(run, actor, holder)`: claims a ready external step whose action's label
  equals `actor` (both absent counts as equal). With a run, that run's step; without
  one, the oldest matching ready step of the tenant: the first in queue order (§2
  `tick`, Dispatch), highest action `priority` first, then oldest `notBeforeMs`.
  With a run whose external step is held, the host first calls `probe_step`; after a
  `dead` answer it takes the run's next turns (§2 `tick`, Coordinate), so a step the
  kernel makes ready again is claimed at once.
  None is `Ok(None)`. The `holder` handle is
  canonical JSON of at most 4 KiB (`invalid-input` / `holder.invalid` otherwise). Before
  the claim, the host makes or checks the step's files and work folder as for any
  dispatch (§3, Step files and work folders), and quarantines the run instead when it
  cannot. The claim goes to `claim_attempt` with a fresh owner, the holder, `leaseMs`
  (or `timeoutMs` when there is none) as the lease, and `timeoutMs` as the deadline. It
  returns the token, the run, node, action and invocation IDs, the attempt number, the
  evaluated input, (0.2) the node's evaluated `params` (`{}` when it has none), the
  declared outcomes, the node's `outputSchemas`, `deadlineMs`,
  `leaseUntilMs`, the result channel the run recorded for the action (The handshake;
  `store` for a run that recorded none), and the paths of the step files, its
  instructions file and its work folder (null without `files_dir`). A step whose
  action this host falls short of is not claimed (The handshake).
- `renew_step(token)`: `heartbeat` by `leaseMs` (or `timeoutMs`), capped at the
  deadline (store §2). Returns the new `leaseUntilMs`.
- `complete_step(token, outcome, output)`: refuses, with nothing recorded, an outcome
  the action does not declare (`invalid-input` / `step.undeclared-outcome`), output
  outside the canonical JSON domain (integers only, depth at most 32;
  `step.output-not-canonical`), a result body over 64 KiB (`step.output-too-large`),
  and output that does not fit the node's schema for the outcome (`output.invalid`, as
  the other client operations, above). Then `finish_attempt` with a `success` result.
  The kernel never sees an undeclared outcome from an actor.
- `fail_step(token, error, retryable)`: `finish_attempt` with a `failure` result: the
  error (code `step.failed` unless the actor names one of the pattern
  `[a-z0-9][a-z0-9.-]{0,63}`) and whether another try could help, which the kernel
  then honours up to `maxAttempts`.
- `abandon_step(key)`: expires the run's held async attempt at once with reason
  `abandoned` (store §2), whatever its lease; the effect class then decides. A run with
  no held async step is `invalid-input` / `step.not-held`. It is a person's act,
  never automatic: the old holder may still be at work.
- `needs_person(token, reason, request_id)` (0.2): sets the attempt's needs-person
  mark with `reason`, or clears it with none, through `set_needs_person` (store §2)
  under the request ID `needs-person/<attemptId>/<request_id>`, so an ID names one
  request of one attempt; the caller generates a `request_id` when none is given. A reason that is empty or over 1024 bytes is
  `invalid-input` / `step.reason-invalid`, with nothing recorded. It returns the
  node, the attempt, the disposition and the mark afterwards. It calls no kernel and
  changes nothing the kernel reads, so it never moves the run: what it says is for the
  people and programs that read the step (cli.md, `step needs-person`).

A token that does not name an attempt of an external step or an async command of its
run is `invalid-input` / `step.token-invalid`. A report whose attempt has already ended
(abandoned, expired, or reported before, through `complete_step`, `fail_step` or a
watch) is refused with `stale` / `lease.lost`, and its result is kept with
`record_late_evidence`: a stale report never moves the run, and nothing it says is
lost. The first result from any channel wins.

### The handshake (0.2)

A host and a workflow agree, before a run starts, on how each action runs and how an
async action's result comes back. The run keeps that agreement: a host never switches
to another way mid-run, and never turns a capability it lacks into a step's failure.

**The capability set.** `Engine::capabilities()` says what the host offers, each list
sorted:

| Field | Value |
| --- | --- |
| `actionKinds` | The kinds it runs: the kind of every runner in its registry, and `external` |
| `asyncKinds` | The kinds it runs async: `external`, `command` when its registry runs commands (§3, Async commands), and `component` when it runs step runners (§3, `component`) |
| `resultChannels` | The result channels it offers: `store`, where `complete_step` and `fail_step` write the result through the store's `finish_attempt` |
| `componentWorlds` | **0.2.** The WIT worlds of the step runners it runs, `runspore:runner/runner@0.1.0` when it runs kind `component`; empty when it runs none |

It offers no `probe`, `watch` or `stop` in their `component` form. A kind, a channel
or a form offered later is a new value of these lists, not a new field.

**Async use.** An action is async when its kind is `external` or `component`, or it
carries an `async` object. Its result channels are `async.channels`, in order of preference, default
`["store"]`.

**At `validate_workflow` and `start`.** Every action is checked against the set. The
first shortfall refuses the workflow with `invalid-input` and its code, and a message
that names the action and what to configure; nothing is created:

| Shortfall | Code | The message names |
| --- | --- | --- |
| Its kind is not in `actionKinds` | `action.kind-unregistered` | The kind and the kinds the host runs |
| It carries `async` and its kind is not in `asyncKinds` | `action.async-unsupported` | The kind, the kinds the host runs async, and the fix: remove `async` |
| Its `async` `probe`, `watch` or `stop` has the `component` form | `action.component-unsupported` | The program, and the fix: the `command` form |
| None of its result channels is in `resultChannels` | `action.no-result-channel` | Its channels and the host's, for example "action `review` lists result channels `nats`; this host offers `store`" |

For each async action, `start` picks the first of its result channels the host offers
and records the picks with the run (store §2, `create_run`, `resultChannels`). A run
found by its start key keeps the channels it was created with.

**Before acting on a step.** Before `tick` claims an invocation and before
`claim_step` claims one, the host checks the invocation's action again, with the
channel the run recorded rather than a new pick: the action's kind is in
`actionKinds`; an async action's kind is in `asyncKinds` and its recorded channel in
`resultChannels` (a run that recorded none counts `store`); no `probe`, `watch` or
`stop` has the `component` form; and the runner of its kind accepts it, as its
`validate` did at `start` (a `native` action's function is registered). When the
check falls short, the host leaves the invocation untouched: it claims nothing, so no
attempt is spent and no lease starts. The step is **capability-missing** until a host
that has the capability serves the run, which then runs it as usual.

`async_step(view)` describes such a step when the run is `running`, has no pending
event, and its outstanding invocation is `scheduled`, pending, with an action this
host falls short of: the action's kind and actor, the state `capability-missing`, the
attempt number, and `missing`, a sentence that says what the host lacks and what to
restore, for example "this host has no runner for kind `native`; it runs `command`,
`external`: use a host that registers one, or one of those kinds". A held async step
stays `held`: its attempt is its holder's, and the host does not touch it.
`run_until_parked` returns at a capability-missing step, as at an async one.

### Telling a person (0.2)

A run that needs intervention or has failed waits for a person, and a person finds it
only by asking. A workflow says who to tell with `onIntervention` (kernel.md §2): an
action of the workflow and its `params`. n8n's Error Workflow, Argo's `onExit` and Step
Functions' top-level `Catch` do the same in their engines.

```json
"onIntervention": {"action": "notify",
                   "params": {"issue": {"$get": ["input", "ticket", "issue"]}}}
```

- **When.** Each time a run *enters* `needs-intervention` or ends `failed`: the
  engine whose `commit_turn` was `applied` for a turn whose status differs from the
  status before it runs the action once, in the background, beside the attempts it
  dispatches (it counts against `max_concurrent_activities`). A run that stays in
  `needs-intervention` while other events are consumed is not told again; one that is
  resolved and gets stuck again is. Only one engine's commit is applied, so two
  engines on one store tell once. A completed run, a waiting run and a quarantine tell
  nobody.
- **Input.** The action gets, on stdin for a command:

  | Field | Value |
  | --- | --- |
  | `run` | `{"id", "tenant"}` |
  | `node` | The node the run is at, or for `failed` the node its error names (`result.error.nodeId`); null when it names none |
  | `status` | `needs-intervention` or `failed` |
  | `reason` | `stuck` (a step gave up with no `failed` route), `unknown` (an `unsafe` step's outcome is unknown), or `failed` |
  | `error` | A stuck step's kept error; for `unknown`, the error of the result that made it unknown, or null (an expired lease has none); the run's error for `failed` |

- **Params.** `params` is a mapping (kernel.md §4) evaluated with the kernel's
  evaluator over `input` (the run's input), `nodes` (the latest result of each node)
  and `run` (`{"id", "tenant"}`). The action gets it as a node's params: under
  `params` for its `argv` mappings, in `ActivityContext::params` for a native function.
  Without `params` it gets `{}`.
- **How it runs.** By the runner of its kind, like an attempt, but not as one: no
  invocation, attempt or lease is stored and no heartbeat is sent. Its context names
  the node above (`RUNSPORE_NODE_ID`), the action, attempt 1, visit 1, and its own
  invocation ID, `ids::invocation(tenant, runId, "onIntervention/<revision>")` with
  the revision of the turn that moved the run, which is also its effect key
  (`RUNSPORE_EFFECT_KEY`), the same on every engine. It runs from the run's base
  folder and gets `RUNSPORE_DB` like any command, and no step files or work folder.
  Shutdown stops it like an attempt.
- **Its result never changes the run.** A success records nothing. A `Failure` or
  `Unknown` (a command's non-zero exit, timeout, a runner this host lacks, code
  `capability-missing`, which 0.1 called `host.runner-missing`, params that do not
  evaluate) is appended as an event of kind `on-intervention.failed` with event and
  request ID `on-intervention/<revision>`, body `{"action", "status", "error"}`, and
  nothing more: it is not retried, and the kernel consumes the event with no change
  (kernel.md §7.3), so it shows in the run's events and nowhere else.
- **Checks.** `validate_workflow` and `start` refuse, with `workflow.invalid`, an
  `onIntervention` whose action the workflow does not define, an `external` action
  (the worker runs `onIntervention`; nobody would claim it), an async command (its
  result would come back to no attempt), and `params` that are not a mapping over
  those roots.
- **Known limit.** Telling is at most once: an engine that stops before the action
  ends (a crash, or a shutdown whose last tick commits the turn) does not run it
  again, and the next engine does not either. The run's status still says it needs a
  person; `onIntervention` only says so sooner.

### Failpoints

| Name | Position |
| --- | --- |
| `host.turn.after-reduce` | decision computed, before `commit_turn` |
| `host.turn.after-commit` | `commit_turn` returned |
| `host.attempt.after-claim` | claim held, activity not started |
| `host.attempt.after-effect` | activity returned, before `finish_attempt` |
| `host.attempt.after-finish` | `finish_attempt` returned |
| `host.component.after-spawn` | **0.2.** A step runner's `start-process` started its program (§3, `component`) |
| `host.component.after-start` | **0.2.** A step runner's `start` returned a handle, before `hold_attempt` |

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
`record_process`), which the engine stores with `set_process`, and the step's files and
work folder (below), when it has them. **0.2:** it carries the node's `params`
(kernel.md §2, §4) as `params`: the value the kernel evaluated with the input, from
the `activity.schedule` command, which the store keeps with the invocation and returns
in every claim, so every attempt of the invocation gets the same params. A node without
`params` gives `{}`. The registry maps an action's `kind` to a runner.

A host that evaluates mappings of its own at dispatch (an action's `argv`, a command's
`result`) uses the kernel's evaluator, `runspore_kernel::mapping`, over its own roots:
the same operators and the same error codes as in the kernel (kernel.md §4).

### `command`

```json
{"kind": "command", "argv": ["cargo", "test"], "cwd": ".", "env": {"CI": "1"},
 "timeoutMs": "600000", "output": "text", "exitOutcomes": {"1": "red"},
 "outcomes": ["ok", "red"], "effect": "read-only"}
```

- `argv` is executed directly, never through a shell. **0.2:** an item may be a
  mapping (below). `cwd` is resolved against
  the run's base folder (**0.2**; 0.1: `base_dir`), recorded at `start`, so every
  dispatch of a run uses one folder whichever engine runs it. A run with no recorded
  folder uses `base_dir`. The child inherits the engine's environment plus `env`.
  **0.2:** when the folder the command would run in is under the run's base folder
  and that base folder no longer exists, as a removed git worktree's does, the step is
  `Failure`, `command.base-dir-missing`, not retryable, details `{"baseDir"}`, with a
  message naming the folder. Nothing is started. An absolute `cwd` outside the base
  folder does not need it.
- **0.2: `cwd` from input.** `cwd` is a string, as in 0.1, or
  `{"fromInput": "<JSON Pointer>"}` (RFC 6901); the build refuses one that is not a
  pointer (`command.cwd-not-pointer`, dot.md §6). That form reads the folder from the
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
- **0.2: `argv` from mappings.** An item of `argv` is a string, kept as it is, or a
  mapping (kernel.md §4). The host evaluates every mapping item before each try, at
  dispatch, with the kernel's evaluator, over these roots:

  | Root | Value |
  | --- | --- |
  | `input` | The node's evaluated input, the value the command gets on stdin |
  | `params` | The node's evaluated `params` (§3), `{}` when it has none |
  | `run` | `{"id", "tenant"}` of the run |
  | `step` | `{"node", "visit", "attempt", "resumed"}`: the node ID, the visit (`RUNSPORE_VISIT`), the attempt number (`RUNSPORE_ATTEMPT`), and `resumed` |

  `step.resumed` is `true` when this node already had a try earlier in this run: an
  earlier try of this invocation (one that failed, timed out or was stopped), or an
  earlier visit of the node. It is `false` on the first try of the first visit. So a
  program that keeps a session starts it on the first try and resumes it on every
  later one:

  ```jsonc
  "argv": [
    "coder", "-p",
    { "$if": [{ "$get": ["step", "resumed"] }, "--resume", "--session-id"] },
    { "$uuid": [{ "$get": ["run", "id"] }, { "$get": ["step", "node"] }] },
    "--tools", { "$join": [{ "$get": ["params", "tools"] }, ","] }
  ]
  ```

  An item must evaluate to a string. One that evaluates to anything else is
  `Failure`, `command.argv-invalid`, not retryable, details `{"index", "value"}`: the
  item's index in `argv` and the value it gave. One that does not evaluate keeps the
  kernel's code and details (`mapping.missing-path`, `mapping.invalid-operand` with a
  path that starts `["argv", <index>]`, or `budget.value-depth`), not retryable.
  Either way nothing is started, and since another try evaluates the same, the step
  is stuck or takes its `failed` route (kernel §7.4). An `argv[0]` asset path
  (below) is looked for after evaluation. `validate` checks every mapping item as a
  mapping (W10) and refuses a `$get` path that starts with anything but `input`,
  `params`, `run` or `step`, an item that is neither a string nor an object, and a path
  into `input` or `params` that a step running the action surely does not give
  (dot.md §4, `mapping.missing-path`). A
  probe's, watch's or stop's `command.argv` takes the same items, which the build
  checks the same way, and, once the host runs it, gets the same roots, `step`
  included, for the try it looks after. `ActivityContext::scope(input)` gives these
  roots to any runner.

  **Known limit.** `resumed` says that a try happened, not how far its program got.
  Measured with one such program: a try stopped within about 1 second of starting has
  not created its session yet, so the next try gets `resumed: true` and `--resume`
  fails because there is no session to resume; a try stopped after 3 seconds or more resumes
  correctly. A try that ended before its program ran (`command.spawn`) counts too. The
  fix is a step reporting its progress, as Temporal's heartbeat details do, and
  `resumed` reading that report; it is left to a later version.
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
- **0.2:** it also gets `RUNSPORE_STEP_FILES`, `RUNSPORE_INSTRUCTIONS` and
  `RUNSPORE_WORK_DIR` (Step files and work folders, below), after `env`. One of them
  the step does not have is removed from the environment, so a value the worker
  inherited never reaches the step.
- **0.2:** in a run with a parent recorded (store.md §2 `create_run`; cli.md §Runs
  that start runs), it also gets `RUNSPORE_PARENT_RUN_ID`, the parent's run ID, after
  `env`, so a child's step can signal its parent with no input naming it, as Temporal
  gives a child workflow its parent's execution. In a run with no parent it is
  removed, like the three above.
- **0.2:** an `argv[0]` of the form `./<p>`, where `<p>` is the path of an asset of the
  action, runs the step files' copy: the host replaces it with the absolute path of
  `<p>` in the step files. Any other `./…` keeps its 0.1 meaning, relative to `cwd`, so
  a project's own `./test.sh` still runs the project's script. Other `argv` entries are
  never rewritten.
- **0.2** (replaces 0.1's "It runs in its own process group. On timeout or stop the
  whole group is killed."): it runs under a guard, below.
- A command never learns the action's outcome names. The workflow maps its result to
  an outcome (`exitOutcomes`, or result mode below).

Result mapping, in order:

| What happened | Runner output |
| --- | --- |
| **0.2.** The run's base folder is gone, and the command's folder is under it | `Failure`, `command.base-dir-missing`, not retryable, details `{baseDir}` |
| **0.2.** A `cwd` from input that names no existing folder | `Failure`, `command.cwd-invalid`, not retryable, details `{pointer, cwd?}` |
| **0.2.** An `argv` mapping that gives no string, or does not evaluate | `Failure`, `command.argv-invalid`, not retryable, details `{index, value}`; or the kernel's mapping code |
| Could not be spawned, or its program could not be run | `Failure`, `command.spawn`, retryable |
| Killed by a signal it did not get from the engine, or its guard ended without reporting | `Unknown`, `command.signaled` |
| Timed out, or stopped by the engine | `Unknown`, `command.timeout` or `command.stopped` |
| An exit code listed in `exitOutcomes` | `Success` with the mapped outcome |
| Exit 0, `output` other than `result`, no `result` block | `Success` with outcome `ok` |
| Exit 0, `output: "result"` | What stdout says (below) |
| **0.2.** Exit 0, `output: "json"` with a `result` block | What the block maps from stdout (below) |
| Any other exit code | `Failure`, `command.exit`, retryable, details `{exitCode, stdoutTail, stderrTail}` (below) |
| Stdout over 64 KiB, `output` other than `text` | `Failure`, `command.output-too-large`, not retryable |
| `output: "json"` and stdout is not JSON in the canonical domain (with a `result` block: not JSON) | `Failure`, `command.malformed-output`, not retryable |
| `Success` whose outcome the action does not declare | `Failure`, `command.undeclared-outcome`, not retryable, details `{outcome}` |

The last row holds for every `command` in every output mode, so an undeclared outcome
never reaches the kernel.

**0.2: the ends of the streams.** Every `Failure` and `Unknown` above of a program that
ran (every row but `base-dir-missing`, `cwd-invalid`, `argv-invalid` and `spawn`) carries in its details
`stdoutTail` and `stderrTail`, the last 4 KiB of each stream, decoded as UTF-8, lossy,
and cut on a character boundary. Stdout's end is kept even when the stream passes
64 KiB. Some programs write their errors to stdout (one prints
`{"is_error": true, …}` there and leaves stderr empty), so the cause shows whichever
stream it went to. A failure a command names itself in result mode (below) is its own
and gets neither. 0.1 kept only `stderrTail`, in `command.exit`, and gave the other
rows no details. An action that declares no `outcomes` declares `["ok"]`
(kernel §2).

Success output by `output` mode:

- `text` (default): `{"exitCode": n, "stdout": "…", "stderr": "…", "truncated": b}`,
  each stream decoded as UTF-8, lossy. When the two streams together exceed 64 KiB,
  each keeps its end: stderr its last 16 KiB at most, stdout the last bytes that fit in
  the 64 KiB left beside it. A cut falls on a UTF-8 character boundary. `truncated` is
  `true` when either stream was cut. Text mode never fails with
  `command.output-too-large`: a long log reaches the next step as its end, where test
  runners and compilers print their failures.
- `json`: the parsed stdout value, or with a `result` block on exit 0 what its
  `output` maps (below).
- `result`: the output stdout names (below). An exit code listed in `exitOutcomes`
  gives `null`.
- `none`: `null`.

**Result mode.** With `"output": "result"`, a command names its own result. On exit 0,
stdout must be canonical-domain JSON of one of two shapes:

| Stdout | Runner output |
| --- | --- |
| `{"outcome": "<o>", "output": <json>}` | `Success { outcome, output }` |
| `{"error": {"code": "<c>", "message": "<m>"}, "retryable": <bool>}` | `Failure { error, retryable }` |

`output` may be left out: `{"outcome": "<o>"}` is `Success { outcome, output: {} }`.
`error` may also carry `details`, any JSON. Any other stdout on exit 0 (other keys,
wrong types, not JSON) is `Failure`, `command.malformed-output`, not retryable. A
non-zero exit maps through the table above as in every mode, so `exitOutcomes` still
applies to it; a crash that exits 1 is then `command.exit`, never an outcome. These are
the shapes an async `watch` prints (Async commands, below).

**0.2: the `result` block.** Most programs print JSON of their own shape, not
`{"outcome", "output"}`. With `"output": "json"`, a `result` block maps the outcome
and the output out of it, so no wrapper script is needed (AWS Step Functions'
`ResultSelector` and `Choice` do the same in two states):

```jsonc
{"kind": "command", "argv": ["coder", "-p", "--output-format", "json", "…"],
 "output": "json", "outcomes": ["approved", "changes"],
 "result": {
   "outcome": {"$get": ["stdout", "structured_output", "outcome"]},
   "output":  {"$get": ["stdout", "structured_output", "output"]}
 }}
```

- `outcome` and `output` are both required; each is a mapping (kernel.md §4, every
  0.2 operator) whose `$get` paths start with one of `stdout`, the parsed stdout;
  `input`, the node's evaluated input; `params`, its params (`{}` without);
  `run`, `{"id", "tenant"}`; or `step`, `{"node", "visit", "attempt", "resumed"}`, as
  for `argv` mappings above.
- On exit 0 the host parses stdout as JSON (`command.output-too-large` as above;
  stdout that is not JSON is `command.malformed-output`), then evaluates `outcome`,
  then `output`, with the kernel's evaluator. Unlike `json` mode without a block,
  stdout itself need not be in the canonical domain: a program may print its cost
  as a fraction beside the answer. Only the mapped `output` must be;
  one that is not (a fraction, a nesting too deep) is `command.result-invalid`,
  details `{"field": "output", "reason": <the canonical code>}`, such as
  `canonical.non-integer`. The result is checked as in result mode: the outcome must be
  a declared one, and the engine checks the output against the node's `outputSchemas`
  (`output.invalid`).
- An `outcome` or `output` that does not evaluate (a `$get` that selects nothing and
  has no `default`, an operator given the wrong operand), or an `outcome` that is not
  a string, is `Failure`, `command.result-invalid`, not retryable, details
  `{"field": "outcome" | "output"}` plus the mapping error's `path` (and `operator`
  and `value` for an operand), or `value` for a non-string outcome. An undeclared
  outcome is `command.undeclared-outcome`, details `{outcome}`. Each also carries
  `stdoutTail` and `stderrTail` (above), so the program's real answer shows in the
  error. Another try would read the same kind of answer, so
  the step is stuck or takes its `failed` route.
- An exit code listed in `exitOutcomes` gives its outcome and the parsed stdout as
  output, as without the block; the block is not evaluated. A non-zero exit maps
  through the table above.
- Without a `result` block, `json` mode gives the whole parsed stdout as outcome `ok`,
  as in 0.1.

`validate` checks the shape of every field above, that every `exitOutcomes` value is a
declared outcome, and, in result mode or with a `result` block, that `exitOutcomes`
does not map 0. A `result` block goes with `"output": "json"` only
(`command.result-not-json`, dot.md §6), and its mappings are checked as mappings with
its own roots (`mapping.path-root` names them). A `cwd`
object has the one key `fromInput`, holding a JSON Pointer: empty, or `/` and its
steps, each `~` followed by `0` or `1`.

0.1 had no `result` mode, no `result` block, no `RUNSPORE_VISIT`, no `cwd` from input,
no `argv` mappings, and no undeclared-outcome, `command.cwd-invalid` or
`command.argv-invalid` row. Its
`text` mode gave `{"exitCode": n, "stdout": "<utf-8, lossy>"}` and failed with
`command.output-too-large` when stdout passed 64 KiB, like the other modes.

### Async commands (0.2)

A `command` with an `async` object is an **async command**. Its `argv` is a short
**start command**: it starts the work somewhere else, prints a **holder handle** that
names it, and exits. The work reports later, through a result channel. A job that must
outlive the worker that started it is declared this way.

```json
{"kind": "command", "argv": ["./bin/start-review"], "effect": "idempotent",
 "outcomes": ["ok", "changes"],
 "async": {"timeoutMs": "3600000", "leaseMs": "120000", "channels": ["store"],
           "probe": {"command": {"argv": ["./bin/probe-review"]}},
           "watch": {"command": {"argv": ["review-tool", "wait"], "timeoutMs": "600000"}}}}
```

- `async` has the fields of an external action's (§2): `timeoutMs` (default
  `"3600000"`), `leaseMs` (smaller than `timeoutMs`; the work must renew within it
  with `renew_step`), `probe`, `watch`, `stop` and `channels`. `stop` runs when the
  run is cancelled (Stop, below).
- `probe`, `watch` and `stop` are each tagged by implementation:
  `{"command": {"argv", "cwd", "env", "timeoutMs"}}`. Only `argv` is required. `argv`,
  `cwd` and `env` take the forms of a command's own (above): `argv` items are strings
  or mappings over the same roots, evaluated for the try the program looks after, and
  `cwd` from input reads that try's input. An `argv[0]` of the form `./<p>` names a
  file bundled as the action's asset (dot.md §4) and runs the step files' copy; a bare
  name is looked up on the `PATH` when the program runs. `timeoutMs`, a positive
  decimal string, defaults to 10 s for a probe and a stop and to the time left before
  the attempt's deadline for a watch. The tag leaves room for a `component` form; this host
  offers none and refuses one with `action.component-unsupported` (§2, The
  handshake); any other shape is `action.invalid`.
- `channels`: the result channels the work can report through, in order of
  preference, default `["store"]`. This host offers one, `store` (`complete_step` and
  `fail_step`, §2), and refuses a list without it with `action.no-result-channel` (§2,
  The handshake). A declared watch is a channel of its own and is not listed.
- The start's stdout is always its holder handle: `output`, `exitOutcomes` for exit 0
  and a `result` block do not apply to it.

**Start.** The worker claims the attempt as any command's, under its own lease, and
runs the start command as a sync command: under the guard, with its process record, on
the node's input, with the same variables, and also:

| Variable | Value |
| --- | --- |
| `RUNSPORE_TOKEN` | The attempt's token (§2, Client operations): the work renews, completes or fails the step with it |
| `RUNSPORE_RESULT_CHANNEL` | The result channel the run recorded for the action (§2, The handshake), `store` for a run that recorded none |

On exit 0, stdout is the holder handle: canonical-domain JSON of at most 4 KiB, which
the host stores and never reads. The host then calls `hold_attempt` (store §2) with the
handle, `leaseMs` (or `timeoutMs`) as the lease and `timeoutMs` as the deadline, both
counted from then; the call clears the process record. From then on the worker holds
nothing, takes no slot for the step, and sends no heartbeat. The attempt is **held**
until the work reports, a probe answers `dead`, its lease or deadline passes, or a
person abandons it (§2). A `hold_attempt` refused with `lease.lost` means the work has
already reported, or the attempt has ended otherwise; the host records nothing more.

Stdout on exit 0 that is not canonical-domain JSON is `Failure`,
`command.malformed-output`; over 4 KiB, `command.output-too-large`; neither is
retryable. Every other ending maps through the table above as for a sync command: a
non-zero exit is a retryable `command.exit`, an exit code `exitOutcomes` lists ends the
step at once with its outcome, a timeout or stop is `unknown`.

On a normal ending the rest of the start command's process group is killed at once (The
guard, below). The work must therefore leave that group: it runs in a session or
process group of its own (`setsid`), with its standard output and error elsewhere than
the start's, which the host reads to their end. A start command still running when its
worker and its guard die is a leftover like any command's, and the next sweep stops its
recorded group before the next try (The process record, below).

The start command of a repeatable action (`pure`, `read-only` or `idempotent`) must be
repeatable by `RUNSPORE_EFFECT_KEY`. A worker that dies after the start has exited and
before `hold_attempt` leaves no handle in the store, and the next try runs the start
again: it must find the job it already started under that key and print its handle
instead of starting a second. A start that cannot do this belongs to an `unsafe`
action, whose expired attempt waits for a person.

**Probe.** The action's own check of a holder handle. It runs under the guard, without
a process record, with the handle on stdin and the variables of the start command but
`RUNSPORE_TOKEN`. It answers with exit 0 and stdout exactly one of
`{"holder": "alive"}`, `{"holder": "dead"}` and `{"holder": "unknown"}`. Anything else,
a non-zero exit, a timeout, a program that cannot be started, other output, reads as
`unknown`, so a crashing probe never frees a step. Only `dead` acts: the host expires
the attempt at once with reason `probe-dead` (store §2), and the action's effect
decides, as for any expiry. `alive` and `unknown` leave the attempt to its lease and its
deadline: `alive` past the deadline still loses. A probe must say `dead` only when it is
certain: a process ID is checked with its start time, not the number alone.

Probes run on demand, never on a timer of their own: in the sweep, at most every 30 s
per engine (§2 `tick`); in `probe_step`, which `status` and `claim_step` call (§2); and
in `continue` (cli.md). An action without a probe is never probed.

**Watch.** The action's own wait on a holder handle: it ends when the job ends and may
deliver its result. It runs under the guard, without a process record, with the handle
on stdin and the probe's variables.

- A worker runs at most one watch per held attempt and at most `max_watches` in all,
  apart from its slots. It starts one at once when its own start command hands over a
  handle, and in the sweep for the held attempts of the tenant it does not watch yet,
  whoever started them; several workers may each watch one attempt.
- While it runs, a watch may print a line `{"needs": "person", "message": "<text>"}`
  (`message` optional): its holder waits for a person, and the job is alive. The line
  is not a result. The engine keeps the latest message while the watch runs
  (`Engine::holder_waits(key)`), and `run_until_parked` returns, so a driver that
  follows the run can tell the person.
- On exit 0, the rest of stdout, the `needs` lines left out, is one result in result
  mode's shapes (Result mode, above): `{"outcome", "output"}` or `{"error",
  "retryable"}`. It is recorded like a sync command's result: an outcome the action
  does not declare is `command.undeclared-outcome`, not retryable, and the output is
  checked against the node's `outputSchemas` (`output.invalid`, §2, One attempt). The
  host then calls `finish_attempt` with the attempt's reference read from the store
  (`list_invocations`), so a worker that took over the watch from a dead one reports
  it. A result that loses (the work's own `complete_step` came first, or another
  worker's watch) is kept with `record_late_evidence`; a later `complete_step` is
  refused and kept the same way (§2).
- Any other ending (a non-zero exit, a signal, a timeout, other output) means only that
  the host lost sight of the job, never that the step failed: nothing is recorded, the
  attempt stays held, and the next sweep starts a watch again.
- The host stops a watch with its process group when the attempt stops being held (it
  checks every `heartbeat_ms`) and when the engine shuts down.

**Stop.** The action's own way to end the work behind a holder handle, run once when
the run is cancelled (§2, `cancel`), after the store has expired the attempt with
reason `cancelled`, so the holder is already fenced out. It runs under the guard and
the stop sequence, without a process record, with the handle on stdin (`null` for a
claim that gave none) and the probe's variables: the try's input and params, the step
files and work folder, and `RUNSPORE_DB`. Its default timeout is 10 s. Its exit code and
stdout are kept as late evidence of the attempt, `{"stop": {"exit", "stdout"}}`
(`exit` null when it timed out, was killed, or could not be started; `stdout` as text,
its last 16 KiB). Whatever it does, the run stays cancelled: a stop that fails, hangs or
cannot be started is ignored. It is best effort, as Temporal's terminate is: a host
that dies between the cancel and the stop does not run it later.

### `component` (0.2)

A **step runner** is a WebAssembly component that starts, probes and stops an async
step's work, as an async command's start command, probe and stop do. It is sealed into
the workflow artifact, so a run keeps the runner it started with, and it can do only
what the workflow grants it. Wasmtime runs host-defined WIT worlds this way; Extism's
host functions and the capability grants of Spin and wasmCloud do the same.

```json
{"kind": "component", "module": "runner.wasm", "effect": "idempotent",
 "outcomes": ["ok", "changes"],
 "grants": {"programs": ["herdr", "./bin/job"], "sockets": ["~/.herdr/api.sock"],
            "folders": ["."]},
 "async": {"timeoutMs": "3600000", "leaseMs": "120000"}}
```

- `module`: the path of the runner inside the workflow folder, sealed as an `asset`
  entry owned by the action (dot.md §4). The host loads it from the run's step files
  (Step files and work folders, below), whose digest it checks before every use, so
  replacing the file in the folder never changes a run in flight.
- `grants`: what the runner may do, by kind. Each list is optional, default empty.
  - `programs`: the programs `start-process` may start, each an `argv[0]` as written:
    a bare name looked up on the `PATH` when it starts, an absolute path, or `./<p>`,
    a file of the folder bundled as the action's asset whose step files' copy runs.
    `process-alive` and `signal-process` take only a process whose `program` is one.
  - `sockets`: the Unix sockets `request` may reach.
  - `folders`: the folders `read-file` and `list-folder` may read, each with what is
    below it.

  A socket or folder is an absolute path, `~/<p>` under the worker's home folder, or
  a path relative to the run's base folder. A folder is compared once symbolic links
  are resolved, so a link never leads out of it. A grant of any other kind is
  `component.grant-unsupported`.
- `async`: as an external action's (§2), and optional: a `component` action is always
  async. `probe`, `watch` and `stop` may each be given in the `command` form; one that
  is given is run instead of the runner's own export. The `component` form of those
  three stays refused (`action.component-unsupported`).
- `timeoutMs`: how long `start` may run, a positive decimal string, default
  `"60000"`. A probe and a stop get 10 s.

**The world.** A runner is a component of world `runner` of
`contracts/wit/runner/runner.wit`, `runspore:runner@0.1.0`. It exports `step`:
`start`, `probe` and `stop`, each given the try it looks after as a `context` (run,
node, action, invocation, attempt, visit, effect key, input and params as canonical
JSON, the base folder, the work folder, the step files, the result channel, and to
`start` only the attempt's token). It may import only `host`:

| Import | What it does |
| --- | --- |
| `start-process` | Starts a granted program detached: argv, working folder and environment, in a process group of its own, its standard streams on the null device. Returns a handle: program, host name, boot, process group ID and start time |
| `process-alive` | `alive`, `dead` or `unknown`, as the check before a lease expiry looks at a process record (The process record, below): the leader checked by its start time, not its ID alone; a zombie is dead |
| `signal-process` | Sends `term`, `kill`, `interrupt` or `hangup` to the process's group when it is alive; says whether it did |
| `request` | One request and response over a granted Unix socket: writes the body, reads to the first newline or the end, at most 1 MiB |
| `sleep` | Waits, at most until the call's deadline |
| `read-file`, `list-folder` | Read-only access to the granted folders: a file of at most 16 MiB, the sorted names in a folder |

A started program inherits the worker's environment, then the spawn's `env`, then the
variables of an async command's start command (§3: `RUNSPORE_TOKEN`,
`RUNSPORE_RESULT_CHANNEL`, `RUNSPORE_DB`, the step files and the rest), so it reports
the step with `complete_step` or `fail_step` as an async command's work does. It does
not run under the guard: like that work, it must outlive the worker. A runner that
leaves no handle to its process, or loses it, leaves it running.

**Sandbox.** The runner runs as the kernel does (§1): Wasmtime's deterministic
features, a memory ceiling of 64 MiB, a fuel watchdog, a fresh instance for every
call, and no WASI. The host compiles each module once per process, by digest. A call
also has a deadline: `timeoutMs` for `start`, 10 s for a probe or a stop. A `sleep` or
`request` past it traps.

**Grants are enforced, never asked.** A host call outside the action's grants traps the
runner. The host records which grant was missing, and the call ends with it, whatever
the runner does next:

| Call | Ends with |
| --- | --- |
| `start` | `Failure`, `component.not-granted`, not retryable, details `{"grant": "programs" \| "sockets" \| "folders", "value"}`; the message names the grant and the value, such as "action `review` is not granted program `curl`: add it to `grants.programs`" |
| `probe` | `unknown`, so a probe never frees a step by breaking its grants |
| `stop` | Late evidence `{"stop": {"exit": null, "stdout": "<the message>"}}` |

The other endings of a call:

| What happened | `start` | `probe` | `stop` |
| --- | --- | --- | --- |
| It returned | `ok(handle)`: held, as an async start's handle (§3) | Its answer | `ok(text)`: exit 0; `err(text)`: exit 1; the text as `stdout` |
| `start` returned `err(failure)` | `Failure` with its code (`component.failed` when it is not a label), message and `retryable` | | |
| A handle that is not canonical-domain JSON, or over 4 KiB | `Failure`, `component.malformed-output` or `component.output-too-large`, not retryable | | |
| A trap, the watchdog, refused memory growth, the deadline | `Unknown`, `component.trap`, `component.watchdog`, `component.memory` or `component.timeout`: a process may have started | `unknown` | exit null |
| The module is not in the step files, or does not load as a `runner` | `Failure`, `component.module-missing` or `component.invalid`, not retryable | `unknown` | exit null |
| The lease is lost or the engine shuts down, and the runner then calls the host | `Unknown`, `component.stopped`: that call traps | `unknown` | exit null |

**Known limit.** `process-alive` and `signal-process` check that a handle's `program`
is granted and that its group's leader has the recorded start time; they cannot tell
that the runner started it. A runner that forges a handle naming a granted program can
signal any process group of the worker's user whose leader's start time it knows.
Signing handles is left to a later version.

The start of a repeatable action must be repeatable by its effect key, as an async
start command must (§3): a worker that dies after the runner started its process and
before `hold_attempt` leaves no handle, and the next try runs `start` again. The work
folder, which is the same for every try, is where it can keep what it started; grant
it as a folder to read it back.

**Checks.** `validate_workflow` and `start` refuse, with `action.invalid`, a `module`
that is not a relative path and `grants` that are not lists of strings by the kinds
above (`component.grant-unsupported` for another kind). `start` of a package without
an artifact refuses a `component` action with `component.no-artifact`; `start_artifact`
loads each runner from the artifact and refuses one it does not hold
(`component.module-missing`) or that is not a component of world `runner`, imports
anything else or lacks an export (`component.invalid`, naming the import or the
export). Nothing is created. `spore validate` checks the same of a folder (dot.md §6).

**Recovery** is an async command's. The handle is stored with `hold_attempt`; the
worker then holds nothing. A worker that died is replaced by any other: its sweep, and
`probe_step`, probe the held attempt with the runner's `probe`, and a `dead` answer
expires it (§3, Probe); a cancel runs the runner's `stop` once (§3, Stop).

**Failpoints.**

| Name | Position |
| --- | --- |
| `host.component.after-spawn` | `start-process` started the program, before the runner gets its handle |
| `host.component.after-start` | `start` returned a handle, before `hold_attempt` |

### The guard (0.2)

Every process the host starts for an attempt runs under a guard. In 0.2 that is a
`command`'s `argv`, an async command's start command included, and an async action's
probe, watch and stop. A step runner's `start-process` is not: it starts work that
must outlive the worker (§3, `component`).

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
- Only the processes that do a step's work are recorded: a command's, and an async
  command's start command, whose record `hold_attempt` clears when it hands over its
  handle. A probe and a watch are not recorded.

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

### Step files and work folders (0.2)

A workflow artifact seals each node's `instructions` file and each action's assets
(artifact.md §2). The host delivers them to the step that declares them, the same way
for every kind of action, from the run's stored artifact and never from the folder it
was built from. Each visit of a step also gets a writable work folder.

**Step files.** For a run of artifact `A` at activity node `N`, the step files are the
folder `S = <files_dir>/steps/<hex>/<N>/`, where `<hex>` is the hexadecimal part of
`A`'s digest. `S` holds exactly the manifest entries owned by `N` (its `instructions`
entry) and those owned by `N`'s action (its `asset` entries, bundled probe, watch and
stop programs included), each once, at its entry path. It holds nothing of any other
node, so a step cannot quietly depend on a file it never declared. `S` exists, empty,
for a node that declares no files.

1. **Making `S`.** The host reads the stored artifact (store.md `get_artifact`), checks
   that its bytes match the run's artifact digest, loads it in the sandbox (§1) and
   reads each entry with `blob(digest)`, checking the bytes against the entry's digest.
   It checks every path again (artifact.md §3) and writes each file into a fresh
   sibling folder, mode `0444`, or `0555` when the entry is `executable`. It then sets
   every folder in it to `0555` and renames it to `S`. When `S` appeared meanwhile, it
   discards its own folder and uses `S` after checking it.
2. **Checking `S`, before every use.** Before each dispatch that touches `N`, the files
   under `S` must be exactly the entries of `N`: each a regular file, not a symbolic
   link, with mode `0444` (`0555` when `executable`) and a matching digest, in folders
   of mode `0555`. Otherwise the host moves `S` aside, removes it, and makes it again.
   A hand-edited or extra file therefore never reaches a step. Read-only modes do not
   stop a program that changes them during its own try; the check before the next use
   repairs what it changed.
3. **Failure.** When the stored artifact is missing, does not match its digest or does
   not load, or an entry's blob is absent or does not match, the host does not claim
   the attempt. It quarantines the run with code `artifact.unreadable`. A broken store
   is not a business failure: no attempt is spent, and the host never falls back to the
   source folder. `release` (§2) lets the next tick try again, after the store has
   been repaired. When `S` or `W` cannot be written, the quarantine code is
   `host.files-unwritable`. An engine without `files_dir` does none of this before
   the claim; when it claims a step of a run whose artifact has files (a run started
   by another engine), it quarantines the run with `host.files-dir-missing`, starts
   nothing, and leaves the attempt to its lease, as after a crash.

**Work folder.** Each invocation gets `W = <files_dir>/work/<runId>/<invocationId>/`,
created with mode `0700` before its first try. Every try of the invocation gets the same
folder, so a step's state survives between its tries. The host never removes it in this
version; pruning finished runs and their folders is left to a later version.

**Variables.** Every program the host runs for an invocation gets:

| Variable | Value |
| --- | --- |
| `RUNSPORE_STEP_FILES` | Absolute path of `S`; unset for a run with no artifact, or when `files_dir` is unset |
| `RUNSPORE_INSTRUCTIONS` | Absolute path of `N`'s instructions file in `S`; unset when `N` has none |
| `RUNSPORE_WORK_DIR` | Absolute path of `W`; unset when `files_dir` is unset |

A `native` function finds the same paths in its `ActivityContext`.

**The sweep, under a lock.** Each artifact has a lock file,
`<files_dir>/steps/<hex>.lock`. Checking and making `S` and the whole try that uses it
hold the lock shared. The sweep (§2 `tick`) takes it exclusive without waiting, skipping
an artifact whose lock is held. While it holds the lock, it reads the engine's runs and,
when no unfinished run (status other than `completed` and `failed`) has the artifact's
digest, renames `<files_dir>/steps/<hex>/` to a hidden trash name. It deletes the trash
after releasing the lock. A run exists before anything is dispatched for it, so either
the dispatch holds the lock first and the sweep then finds the run unfinished, or the
sweep moves the folder first and the dispatch makes it again. Lock files are kept.

`files_dir` is an engine setting. The CLI sets it to `<database path>.files`
(cli.md §Project folder). An embedder must set it for any workflow artifact with
`instructions` or `asset` entries (§2 `start_artifact`).

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
| H22 | **0.2.** A run of an artifact whose node has instructions and whose action has an executable asset, run as `./<asset>`, visited twice, with a try that fails | The command runs the step files' copy and sees the three variables; the step files hold only that node's files, modes `0444`/`0555`; both tries of a visit share one work folder of mode `0700`, the second visit gets another, and both are kept after the run ends; another node's step files hold none of them |
| H23 | **0.2.** A step file edited, a file added and a mode changed between two visits | The next visit sees the built bytes and modes again |
| H24 | **0.2.** `start_artifact` with files and no `files_dir` | Refused with `host.files-dir-missing`; nothing stored |
| H25 | **0.2.** The stored artifact missing when a step is due, then restored | The run is quarantined `artifact.unreadable` with no attempt claimed; after `release` it completes. An artifact whose entry blob is absent quarantines the same way |
| H26 | **0.2.** The sweep with one finished and one unfinished run of different artifacts, and with a finished run's lock held | The finished run's step files are removed and the unfinished run's kept; a held lock keeps them until the next sweep |
| H27 | **0.2.** A run at an external step, worked by an engine, then claimed with and without the step's label, completed, failed and abandoned | The engine never claims it and `run_until_parked` returns there; only a claim naming the label takes it; a completion with an undeclared outcome, misfitting or oversize output records nothing; a completion routes the run; a retryable failure brings a new attempt; an abandoned `unsafe` step needs intervention and the old holder's report is `lease.lost` and kept as late evidence |
| H29 | **0.2.** `onIntervention` on a run whose step gets stuck, is resolved with `retry` and gets stuck again; on a run that ends at a fail node; on a run that completes; with an action that fails; and `validate_workflow` with an unknown or `external` action and `params` rooted at `from` | Each entry into `needs-intervention` and the failure tell once, with `{run, node, status, reason, error}` and the evaluated params, and further ticks tell nobody again; a completed run tells nobody; the failing action's error is one `on-intervention.failed` event, consumed with no change; each bad `onIntervention` is `workflow.invalid` |
| H30 | **0.2.** A signal for the node a run waits at with an outcome the node does not route, with `exhausted`, and with none; another signal name; then the same unrouted outcome sent before the run waits | Each unrouted one is `signal.outcome-unrouted` with the node's outcomes and appends nothing; the other name and a routed outcome are applied; the early signal is applied and dropped with the diagnostic `signal.outcome-unrouted` |
| H28 | **0.2.** External attempts past their lease and past their deadline, with a repeatable and an `unsafe` effect | The sweep expires each; the repeatable step is ready again with the next attempt, the `unsafe` one needs intervention; renewals never pass the deadline |
| H31 | **0.2.** An async command whose start command detaches a job and prints its handle, the job reporting with `RUNSPORE_TOKEN`; one whose watch prints a `needs` line, then a result; one whose watch result does not fit its schema; then a `complete_step` after the watch's result; and a start that prints no JSON or over 4 KiB | The start's input, token and result channel reach it; the attempt is held with the handle and the async deadline, takes no slot, and `run_until_parked` returns there; the job's report completes the run; the `needs` message is kept and `run_until_parked` returns while the watch runs, then the watch's result completes the run; the misfit is `output.invalid`; the late report is `lease.lost` and kept as late evidence; the bad starts are `command.malformed-output` and `command.output-too-large` |
| H32 | **0.2.** A held async step probed by the sweep, by `probe_step` and by `claim_step`, with a probe that answers `alive`, `dead`, `unknown`, crashes, times out and prints something else | Only `dead` expires the attempt, with reason `probe-dead`; a repeatable step then gets its next try and an `unsafe` one needs intervention; every other answer leaves it held |
| H33 | **0.2.** SIGKILL the worker after `hold_attempt`, while the job its async start command detached runs | The job survives; its report reaches the run, which the next worker completes |
| H34 | **0.2.** SIGKILL the worker and its guard together while an async start command runs | The next sweep stops the start command's recorded group before it expires the attempt; the next try starts only after the group is gone |
| H36 | **0.2.** `cancel` while a worker's command runs; while an async command is held, with a stop that writes its stdin to a file, and with one that exits non-zero; while an external step is held; on a completed run; twice with one request ID | The command's group is gone within `heartbeat_ms` + `stop_grace_ms` + 1 s and `run_until_parked` returns the run `cancelled`; the stop ran once with the holder handle on stdin and `RUNSPORE_DB`, and its output is late evidence; the failing stop leaves the run cancelled; the holder's later `complete_step` is `lease.lost` and kept; the completed run is unchanged; the repeat runs no stop |
| H37 | **0.2.** The reference coding loop (cli.md §Coming back after a kill) through the binary, with stand-in programs for `implement`'s command and `test`'s, driven by `continue --wait --exit-status` and killed at every row of that table: SIGKILL of the driver's process group and SIGHUP during a try, failpoints `host.attempt.after-effect` and `host.attempt.after-finish`, before the first turn, at a wait and after a signal; then four red tests in a row, keep-going, four more, a rejection and an approval; and `cancel` at `approve`, at `tests_stuck`, at a stuck step and after a kill | Each row's run state, `needs`, `continue` output and exit, and what runs again match the table; the fourth red takes `exhausted` to `tests_stuck`, keep-going allows three reds again, and the re-entered `implement` reads the hint and the rejection's reason through `from` and the last failing test through `nodes`; each cancelled run is left out of `list --unfinished`, `continue` on it exits 7, and no command runs again |
| H35 | **0.2.** The handshake: `validate_workflow` and `start` with an action of an unregistered kind, an action of a kind the host does not run async carrying `async`, a `component` probe, and result channels the host does not offer; a run started with an external action listing `["nats", "store"]`; a run started by an engine that registers a runner and served by one that does not | Each is refused with its code, a message naming the action and the fix, and no run; the run records `store` and a claim returns it; the second engine claims nothing, the invocation stays pending with no attempt, `async_step` reports `capability-missing` with what to restore, and `run_until_parked` returns; an engine with the runner then completes the run |
| H38 | **0.2.** Two runs with a step due each; `run_until_parked` on one, then on the other; and `spore continue <runId>` on one of two started runs | The first call runs only its run's step: the other's invocation stays pending with no attempt, and runs once on its own call; `continue` prints no change of the other run, which is still `created`, and its command has not run |
| H39 | **0.2.** A waiting run that started a run, which started a third, each with a step due, and a run outside them with a step due; `run_until_parked` on the waiting run; and `spore continue <runId> --wait` on a build whose step starts a ticket run that waits for a person, then signals the build | The two runs below it complete and the call returns the run `waiting`; the run outside them is still `created`; `continue` returns with the ticket run at its wait and the build's `needs` `child`, with the ticket's commands and none of the build's; after the ticket's signal, `continue --wait` completes both |
| H40 | **0.2.** A worker with one slot and three runs of `implement` then `ship`, started a moment apart, the first run's `implement` holding the slot; once with no `priority` and once with `ship` at priority 1 | `status` shows the other two runs queued 1st and 2nd of 2; without priority every `implement` runs before any `ship`, oldest run first; with it each run's `ship` runs before the next run's `implement` |
| H41 | **0.2.** A `component` action whose runner starts a detached process and returns its handle; the worker SIGKILLed after `hold_attempt`; a second worker | The attempt is held with the handle; the process survives the worker; the second worker's `probe_step` answers `alive` through the runner, its `cancel` runs the runner's `stop`, which ends the process, and a probe then answers `dead` |
| H42 | **0.2.** Runners that start a program, reach a socket and read a file outside their grants; one granted each | Each ungranted call fails the step with `component.not-granted` naming the grant and the value, not retryable, with nothing started; a granted one works; an ungranted probe answers `unknown` |
| H43 | **0.2.** `validate_workflow`, `start` and `start_artifact` with a `component` action whose grants name another kind, without an artifact, whose module the artifact lacks, that is not a component, and that imports an interface outside `runner` | Each is refused with its code and no run |
| H44 | **0.2.** Crash at `host.component.after-spawn` and `host.component.after-start` during a run whose `component` step follows a command step, then restart | The run completes; the command step ran once; the runner's start ran at most once per try, and the step completed once |
| H45 | **0.2.** A held external step marked as needing a person, twice with one request ID, with another reason, cleared twice; an async command's held attempt marked through its `RUNSPORE_TOKEN`, then completed; a claimed step marked, then abandoned, then claimed again; `follow` begun before a mark and begun after one | The mark shows in `async_step` with the first `sinceMs`; the repeat and the second clear are `duplicate`; the run's revision, the lease and the run's events are unchanged; the completion and the abandon clear it and the next attempt is unmarked; the first `follow` returns at the mark, the second waits on until the mark is set anew |
