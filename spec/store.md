# Store protocol 0.1

The store is authoritative for event order, revisions, and attempt ownership. The
interface is `runspore_types::store::Store`. This file specifies what each operation
must do; `docs/07-durable-execution.md` gives the reasoning and invariants I1 to I10.

## 1. Rules for every mutation

1. **Receipt first.** Look up `(tenant, run, requestId)`. If a receipt exists with the
   same digest, return its stored value with disposition `duplicate` and evaluate no
   other predicate, even if the state has since moved on. Same ID with a different
   digest: `conflict` / `request.digest-mismatch`.
2. **One transaction.** Receipt lookup, every predicate, every write, and the receipt
   insert share one serializable write transaction. All or nothing.
3. **Authority time.** "Now" is the injected `Clock`, sampled after the write lock is
   held. Client timestamps are never used.
4. **A failure leaves no trace**, including no receipt.
5. Digests are verified: `Evidence.digest` must equal `digest::body(body)`, else
   `conflict` / `evidence.digest-mismatch`.

Failure codes are part of the contract. `kind` is in parentheses.

## 2. Operations

### create_run

Predicates: `packageDigest == digest::package(package)` (`conflict` /
`package.digest-mismatch`). If `(tenant, startKey)` already names another run:
`conflict` / `run.start-key-mismatch`. If the run exists with a different start key,
package digest, or started digest: `conflict` / `run.exists`. If it exists with
identical values but was created by another request ID: return that original value,
disposition `duplicate`.

Writes: the package, content-addressed, if absent; the run at revision 0, applied
sequence 0, status `created`; event sequence 1 with ID `start`, kind `run.started`,
body `started.body`, accepted at now. Returns `{key, startedSequence: 1}`.

### append_event

Predicates: the run exists (`not-found` / `run.not-found`); `kind` is not
`run.started` or `activity.result` (`conflict` / `event.reserved-kind`); the body is
at most 64 KiB (`quota` / `event.too-large`). If the event ID exists in this run with
the same kind and body digest: return the original `{sequence, acceptedAtMs}`,
disposition `duplicate`. Same ID with anything different: `conflict` /
`event.id-conflict`, and the stored event is unchanged.

Writes: the event with the next sequence (gapless, starting after the last accepted
event) and `acceptedAtMs = max(now, previous event's acceptedAtMs)`.

### load_turn

A coherent read. Returns `None` if the run does not exist, is quarantined, or has no
pending event (`appliedSequence + 1 == nextSequence`). Otherwise the run's revision,
applied sequence, package digest, snapshot, and the event at `appliedSequence + 1`.

### get_package

The canonical workflow bytes for a digest, or `None`.

### commit_turn

Predicates, in order: the run exists (`not-found` / `run.not-found`); it is not
quarantined (`stale` / `run.quarantined`); `expectedRevision` equals the revision
(`stale` / `revision.mismatch`); the event at `appliedSequence + 1` has `eventId` and
`eventSequence` (`stale` / `event.mismatch`); `snapshotDigest ==
digest::state(snapshot)` (`conflict` / `snapshot.digest-mismatch`); the snapshot is at
most 256 KiB (`quota` / `snapshot.too-large`); no command ID already exists in the run
(`conflict` / `command.id-conflict`).

Writes, atomically:

- a transition record: resulting revision, consumed sequence, decision digest,
  snapshot digest, diagnostics, commit time;
- the run: revision + 1, applied sequence, snapshot, snapshot digest, status;
- the event marked as consumed by that revision;
- each command, in order, and its effect:
  - `activity.schedule`: insert the invocation as `pending` with attempt number 1,
    `notBeforeMs`, input, input digest, effect key, node and action IDs. An existing
    invocation ID is `conflict` / `invocation.exists`.
  - `activity.retry`: the invocation must exist, be `settled`, and have attempt
    number `attempt − 1` (`conflict` / `invocation.retry-invalid`). Set it `pending`
    with the new attempt number and `notBeforeMs`.
  - any other kind: `incompatible` / `command.unknown-kind`.

Returns `{key, revision, appliedSequence, decisionDigest}`.

### get_receipt

The stored receipt for a request ID, or `None`. Absence does not prove the original
request aborted.

### claim_attempt

Predicates: the invocation exists (`not-found` / `invocation.not-found`); the run is
not quarantined (`stale` / `run.quarantined`); the invocation is `pending` (`conflict`
/ `invocation.not-claimable`); now ≥ `notBeforeMs` (`conflict` / `invocation.not-due`).

Writes: the run's fence counter + 1; an attempt with ID `ids::attempt(invocationId,
attemptNumber)`, the claiming worker as owner, that fence, `leaseUntilMs = now +
leaseDurationMs`, status `running`; the invocation becomes `running`. Returns the
`Claim`. A fence is never reused within a run.

### heartbeat

Predicate: the attempt is `running`, its ID, owner, and fence match, and now <
`leaseUntilMs` (`stale` / `lease.lost` for all of these). Writes `leaseUntilMs = now +
extendMs`. A replayed receipt returns the historical deadline and extends nothing.

### finish_attempt

Predicates: `result.body` decodes as `ActivityResult` whose `invocationId`,
`attemptId`, and `attempt` match the attempt and whose status is not `expired`
(`conflict` / `result.invalid`); the attempt is `running`, the tuple matches, and now <
`leaseUntilMs` (`stale` / `lease.lost`).

Writes: the attempt becomes `finished` with the result stored; the invocation becomes
`settled`; an event with ID `ids::event_for_attempt(attemptId)`, kind
`activity.result`, and the result body is appended under the `append_event` ordering
rules. Returns its sequence.

### expire_attempt

Predicates: the attempt exists and the tuple matches (`not-found` /
`attempt.not-found`); it is `running` (`stale` / `lease.lost`); now ≥ `leaseUntilMs`
(`conflict` / `lease.not-due`).

Writes: the attempt becomes `expired`; the invocation becomes `settled`; an event with
ID `ids::event_for_attempt(attemptId)`, kind `activity.result`, and body
`canonical::encode(ActivityResult::expired(invocationId, attemptNumber))`. Returns its
sequence. `finish_attempt` and `expire_attempt` compete: exactly one succeeds.

### record_late_evidence

Predicate: the attempt exists (`not-found` / `attempt.not-found`). Writes one audit
record holding the evidence and the time. It changes no run, invocation, attempt, or
event. This is where a result that lost to expiry is kept.

### quarantine_run, release_run

`quarantine_run`: `expectedRevision` must match (`stale` / `revision.mismatch`).
Records `{code, details, at}` on the run. While quarantined the run is invisible to
`load_turn` and `scan_ready`, and `commit_turn` and `claim_attempt` refuse it.
Accepting events continues. `release_run` clears it. Both return the resulting flag
and are idempotent on an already matching state.

### Scans and views

- `scan_ready`: keys of runs with a pending event, not quarantined, ascending by
  `(tenant, run)`. The cursor is opaque; `next` is `None` on the last page.
- `scan_due`: first `Lease` items for running attempts with `leaseUntilMs ≤ now`,
  ascending by deadline then attempt ID; then `Outbox` items for `pending` invocations
  with `notBeforeMs ≤ now` in runs that are not quarantined, ascending by
  `notBeforeMs` then invocation ID.
- `get_run`, `list_runs` (ascending by run ID within a tenant), `list_events`
  (ascending by sequence, with the consuming revision and that commit's diagnostics),
  `list_invocations` (ascending by invocation ID).

## 3. SQLite adapter

- One database file. Several processes on one host may open it at once.
- `journal_mode = WAL`, `synchronous = FULL`, `foreign_keys = ON`, a busy timeout of at
  least 5 seconds. Never on a network filesystem (documented, not detected).
- Every mutation runs in `BEGIN IMMEDIATE`. `SQLITE_BUSY` after the timeout maps to
  `unavailable` / `store.busy`.
- The schema carries a version. Opening a newer version fails with `incompatible` /
  `schema.too-new`. An empty file is initialized.
- `capabilities()`: protocol `0.1`, `multiworkerClaims = true` (same host),
  `persistentWakeup = false`, `transactionScope = "database"`.
- The schema is internal to the adapter and documented in its README. Unique
  constraints, not application checks alone, back I2, I3, and fence uniqueness.

### Failpoints

Around the transaction commit of each mutating operation:
`store.<op>.before-commit` and `store.<op>.after-commit`, with `<op>` one of
`create-run`, `append-event`, `commit-turn`, `claim-attempt`, `heartbeat`,
`finish-attempt`, `expire-attempt`. Use `runspore_types::failpoint::hit`.

## 4. Conformance tests

Written once, generic over a store factory and a manual clock, in
`crates/runspore-store-conformance`. Every adapter runs all of them.

| ID | Scenario | Must hold |
| --- | --- | --- |
| S01 | Two coordinators commit against one revision (C01) | One applies. The other gets `duplicate` (same request) or `stale` (different request). Commands and invocations exist once |
| S02 | Every mutation is replayed after the state moved on (C02) | Same receipt value, disposition `duplicate`, no new writes |
| S03 | Same request ID, different digest; identical requests submitted concurrently (C03) | `request.digest-mismatch`; exactly one mutation survives |
| S04 | Result arrives after expiry (C05) | `finish_attempt` is `stale`; the inbox holds only the `expired` result; late evidence is retained |
| S05 | Heartbeat before, at, and after the deadline (C06) | Extends only strictly before; a lease is never resurrected |
| S06 | Same event ID, different body (C07) | `event.id-conflict`; original body and sequence unchanged |
| S07 | Concurrent appends | Gapless sequences; accepted times never decrease, also when the clock steps back |
| S08 | A commit whose second command is invalid | Nothing is written: no transition, no first command, no receipt |
| S09 | `notBeforeMs` in the future | Not in `scan_due`; `claim_attempt` is `invocation.not-due`; claimable once due |
| S10 | Several attempts in one run | Fences strictly increase; an old fence cannot finish or heartbeat |
| S11 | Quarantine and release | Hidden and refused while quarantined; events still accepted; restored after release |
| S12 | Finish and expire race from two tasks | Exactly one wins; exactly one result event |
| S13 | `create_run` repeated with the same start key, and with a changed package | `duplicate` with the original value; `run.exists` |
| S14 | Reserved kinds and oversize bodies in `append_event` | `event.reserved-kind`; `event.too-large` |

SQLite-specific, in the adapter crate:

| ID | Scenario | Must hold |
| --- | --- | --- |
| Q01 | Close and reopen the file | Every record is still there |
| Q02 | Two store instances on one file, on different threads | S01 and S03 hold across instances |
| Q03 | A child process aborts at each store failpoint | After reopen the mutation is either wholly absent or wholly present; a retry of the same request converges to one applied mutation |
| Q04 | Open a file with a newer schema version | `schema.too-new` |
