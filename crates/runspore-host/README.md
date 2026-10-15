# runspore-host

The engine between the store and the kernel. It holds no graph logic: it never
decides a branch, a retry or an outcome route. Unix only (process groups).

`validate_workflow` checks a document without a store: canonical form, every action
against the runner its `kind` names, and a dry `run.started` transition (null input).
`start` runs the same checks, with the dry transition fed the real run ID and input.

## The loop

`tick()` is one bounded pass and never waits for an activity:

1. **Sweep**: `expire_attempt` for each due lease. `lease.lost` / `lease.not-due` mean
   someone else got there first. A lease with a process record is settled first (the
   `process` module): a live group of ours on this machine is stopped and the expiry
   waits until it is gone; a record from another host, or a group the host may not
   signal, is expired with `processUnchecked`; a group still there after SIGKILL leaves
   the attempt for the next pass.
2. **Coordinate**: for each ready run, up to `turns_per_run` times: `load_turn`, call
   the reducer, `commit_turn`. A turn counts only when the receipt is `applied`. A
   `stale` failure or a `duplicate` receipt means another coordinator won; the
   decision is dropped. `conflict` / `request.digest-mismatch` means another
   coordinator committed a different decision for the same event (version skew or a
   nondeterministic reducer); the decision is dropped and counted as `diverged`. A
   kernel `Failure` quarantines the run with its code and details; a quarantined run
   is invisible to `scan_ready`, so it is never retried until `release`.
3. **Dispatch**: for each due invocation, while fewer than
   `max_concurrent_activities` are in flight: `claim_attempt`, then run the attempt
   as a background task. Commands in a decision are never acted on directly.

An attempt heartbeats every `heartbeat_ms`. A heartbeat answered `lease.lost` fires
the runner's stop signal; the attempt then records its result with
`record_late_evidence` and never calls `finish_attempt`. A `duplicate` claim receipt
is only used after a heartbeat on it is `applied`. An attempt whose run or package
cannot be read is abandoned: nothing is reported, the lease expires and the kernel
decides. A missing action or runner, or an input that does not parse, is reported as
a non-retryable `failure`.

`serve(shutdown)` repeats `tick` until `shutdown` resolves, then calls
`shutdown()`. `shutdown()` can also be called on its own, e.g. on SIGINT while
`run_until_parked` is being dropped. From its first call no attempt is ever claimed
again, by it or by any later `tick`. It waits up to `shutdown_grace_ms` for in-flight
attempts, then fires every remaining attempt's stop signal and waits up to
`shutdown_grace_ms` again. An attempt whose runner returns is finished with what the
runner reported (a stopped `command` reports `unknown` / `command.stopped`), so the
kernel decides at once instead of after a lease. Attempts that still have not
returned are aborted, unfinished: their leases expire. A last tick then sweeps and
coordinates (dispatch is off), so the stored run status reflects the results just
recorded; its errors are ignored. Calling `shutdown()` again is harmless. Only a lost
lease skips `finish_attempt`.

## Failpoints

| Name | Brackets |
| --- | --- |
| `host.turn.after-reduce` | decision computed, not yet committed |
| `host.turn.after-commit` | `commit_turn` returned, next turn not started |
| `host.attempt.after-claim` | claim held, activity not started |
| `host.attempt.after-effect` | activity returned, result not reported |
| `host.attempt.after-finish` | `finish_attempt` returned |

## Request IDs

| Operation | Request ID |
| --- | --- |
| `create_run` | `start/<startKey>` |
| `append_event` (signal) | `signal/<messageId>`, event ID `ids::event_for_signal` |
| `append_event` (resolve) | `resolve/<requestId>`, event ID `ids::event_for_resolution` |
| `release_run` | `release/<revision>/<quarantine at_ms>`: names the quarantine it lifts |
| `commit_turn` | `ids::commit_request(revision + 1, eventId)` |
| `expire_attempt` | `expire/<attemptId>`; after a dead probe `probe-dead/<attemptId>` |
| `finish_attempt` | `finish/<attemptId>`; a watch's result `watch/<attemptId>/<result digest>` |
| `hold_attempt` | `hold/<attemptId>` |
| `claim_attempt`, `heartbeat`, `set_process`, `quarantine_run` | `<op>/<subject>/<engine nonce>/<counter>` |
| `record_late_evidence` | `late/<attemptId>/<engine nonce>` |

Client operations derive their IDs from the client's idempotency key, so a retry is a
duplicate. Engine-internal operations that may legitimately repeat (a second claim of
the same invocation, a second quarantine at the same revision) get a unique ID per
engine (the nonce is unique per `Engine::new`, also within one process); only a retry of the identical request reuses one. Every request is retried
unchanged on `unavailable` and, where the store may have committed, `unknown-commit`.

## Limits

- Unix only: `command` runs each child in its own process group under a guard (the
  `guard` module, a hidden mode of the host's executable; spec/host.md §3). The guard
  stops the group with SIGTERM, `stop_grace_ms`, SIGKILL on timeout, on stop, and when
  the host dies; after the child exits it kills the rest of the group at once. A
  binary that embeds the engine calls `guard::main_if_requested()` first thing in
  `main`, or sets `EngineConfig::guard` to an executable that does, such as the
  `runspore-guard` binary of this crate.
- Before a command runs, its process record (host name, boot, process group, start
  time) is stored with `set_process`; it is cleared once the group has been stopped,
  or by `hold_attempt` when an async command's start hands over its holder handle.
  Probes and watches run under the guard without a record.
  Reading start times and group members needs Linux (`/proc`) or macOS (libproc);
  elsewhere a command cannot be recorded and fails with `command.spawn`.
- The reducer is called inline on the tick's task; use a multi-thread runtime when a
  transition may be slow.
- A release that follows two quarantines in the same millisecond at the same revision
  would be a duplicate of the first release.
