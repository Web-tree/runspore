# runspore-host

The engine between the store and the kernel. It holds no graph logic: it never
decides a branch, a retry or an outcome route. Unix only (process groups).

## The loop

`tick()` is one bounded pass and never waits for an activity:

1. **Sweep**: `expire_attempt` for each due lease. `lease.lost` / `lease.not-due` mean
   someone else got there first.
2. **Coordinate**: for each ready run, up to `turns_per_run` times: `load_turn`, call
   the reducer, `commit_turn`. A `stale` commit (or `request.digest-mismatch`, see
   below) means another coordinator won; the decision is dropped. A kernel `Failure`
   quarantines the run with its code and details; a quarantined run is invisible to
   `scan_ready`, so it is never retried until `release`.
3. **Dispatch**: for each due invocation, while fewer than
   `max_concurrent_activities` are in flight: `claim_attempt`, then run the attempt
   as a background task. Commands in a decision are never acted on directly.

An attempt heartbeats every `heartbeat_ms`. A heartbeat answered `lease.lost` fires
the runner's stop signal; the attempt then records its result with
`record_late_evidence` and never calls `finish_attempt`. A `duplicate` claim receipt
is only used after a heartbeat on it is `applied`.

`serve(shutdown)` repeats `tick`. On shutdown it stops claiming, waits up to
`lease_ms` for in-flight attempts, then fires every remaining attempt's stop signal,
waits up to `lease_ms` again and returns. Attempts stopped this way are not
finished: their leases expire and the kernel decides.

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
| `expire_attempt` | `expire/<attemptId>` |
| `finish_attempt` | `finish/<attemptId>` |
| `claim_attempt`, `heartbeat`, `quarantine_run` | `<op>/<subject>/<process nonce>/<counter>` |
| `record_late_evidence` | `late/<attemptId>/<process nonce>` |

Client operations derive their IDs from the client's idempotency key, so a retry is a
duplicate. Engine-internal operations that may legitimately repeat (a second claim of
the same invocation, a second quarantine at the same revision) get a unique ID per
process; only a retry of the identical request reuses one. Every request is retried
unchanged on `unavailable` and, where the store may have committed, `unknown-commit`.

## Limits

- Unix only: `command` runs each child in its own process group and kills the group
  on timeout, stop, and after the child exits.
- The reducer is called inline on the tick's task; use a multi-thread runtime when a
  transition may be slow.
- A release that follows two quarantines in the same millisecond at the same revision
  would be a duplicate of the first release.
