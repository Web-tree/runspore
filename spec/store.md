# Store protocol 0.1 and 0.2

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
6. Where an operation returns an earlier value as `duplicate` because of a natural key
   (a start key in `create_run`, an event ID in `append_event`), it writes nothing,
   including no receipt for the new request ID.

Codes any operation may return: `unavailable` / `store.busy` or `store.io` (retry the
identical request), `corrupt` / `store.corrupt`, and `conflict` / `cursor.invalid` for
a scan cursor the store did not issue.

Failure codes are part of the contract. `kind` is in parentheses.

Protocol 0.2 adds `find_run`, an attempt's process record with `set_process`, the
audit record of an expiry, the run's base folder, and scenarios S15 to S17; everything
else is as in 0.1.

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

**0.2:** the request carries `baseDir`, the run's base folder (host §2), a string or
null. It is stored with the run and shown by `get_run` and `list_runs`. It is not part
of the run's identity: a repeat with another `baseDir` is a `duplicate`, and the run
keeps the folder it was created with.

Run IDs are chosen by the caller and random (host §2), so `run.start-key-mismatch` is
how a store answers the second of two starts that raced with one start key.

### find_run (0.2)

`find_run(tenant, startKey)`: the key of the run created with that start key in that
tenant, or `None`. A read, like `get_run`: no request ID and no receipt.

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
  - a payload that does not decode as its kind's type: `conflict` /
    `command.payload-invalid`.

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

### set_process (0.2)

`set_process(attempt, process)` stores the attempt's **process record**, the process
group doing its work (host §3), or clears it with null. A record is canonical JSON
`{"host": "<name>", "boot": "<id>", "pgid": <int>, "started": "<decimal>"}`.

Predicate: as `heartbeat`, the attempt is `running`, its ID, owner, and fence match, and
now < `leaseUntilMs` (`stale` / `lease.lost` for all of these). Writes the record on the
attempt and nothing else; the lease is not extended. Returns the stored record. A
replayed receipt returns the historical record and writes nothing.

The record is the store's to keep and the host's to read. An attempt that ends keeps
whatever record it had.

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

**0.2:** the request carries `processUnchecked`, true when the expiring host could not
check the attempt's process record (host §3). In the same transaction the store writes
one audit record for the attempt: `{"reason": "lease"}`, with `"notes":
["process.unchecked"]` when `processUnchecked` is true.

### record_late_evidence

Predicate: the attempt exists (`not-found` / `attempt.not-found`). Writes one audit
record holding the evidence and the time, of kind `late-evidence` (an expiry's audit
record is of kind `attempt.expired`). It changes no run, invocation, attempt, or
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
  `notBeforeMs` then invocation ID. **0.2:** a `Lease` item carries the attempt's
  process record, or null, so the sweeping host needs no second read.
- `get_run`, `list_runs` (ascending by run ID within a tenant), `list_events`
  (ascending by sequence, with the consuming revision and that commit's diagnostics),
  `list_invocations` (ascending by invocation ID). **0.2:** an invocation shows the
  process record of its latest attempt, or null.

## 3. SQLite adapter

- One database file. Several processes on one host may open it at once.
- `journal_mode = WAL`, `synchronous = FULL`, `foreign_keys = ON`, a busy timeout of at
  least 5 seconds. Never on a network filesystem (documented, not detected).
- Every mutation runs in `BEGIN IMMEDIATE`. `SQLITE_BUSY` after the timeout maps to
  `unavailable` / `store.busy`.
- The schema carries a version. Opening a newer version fails with `incompatible` /
  `schema.too-new`. An empty file is initialized. An older version is upgraded in place
  in one transaction: version 1 to 2 adds the attempt's process record and the audit
  record's kind; version 2 to 3 adds the run's base folder, null for runs created
  before.
- `capabilities()`: protocol `0.2`, `multiworkerClaims = true` (same host),
  `persistentWakeup = false`, `transactionScope = "database"`,
  `failureModel = "crash-stop"`, `maxRecordBytes = 262144`.
- The schema is internal to the adapter and documented in its README. Unique
  constraints, not application checks alone, back I2, I3, and fence uniqueness.

### Failpoints

Around the transaction commit of each mutating operation:
`store.<op>.before-commit` and `store.<op>.after-commit`, with `<op>` one of
`create-run`, `append-event`, `commit-turn`, `claim-attempt`, `heartbeat`,
`set-process` (0.2), `finish-attempt`, `expire-attempt`. Use `runspore_types::failpoint::hit`.

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
| S15 | Two `create_run` calls carry one start key and two run IDs, from two instances at once (0.2) | One is `applied`, the other `run.start-key-mismatch`; `find_run` returns the winner; `find_run` of an unknown key, or of the key in another tenant, is `None` |
| S16 | `set_process` on a running attempt, then with another owner or fence, at and after the deadline, and after expiry (0.2) | The record shows in `list_invocations` and in the `Lease` item of `scan_due`; null clears it; the lease is not extended; a replay is `duplicate` with the historical record; every other call is `lease.lost` and changes nothing; an expiry with `processUnchecked` is applied |
| S17 | `create_run` with a base folder, repeated with another, and without one (0.2) | `get_run` and `list_runs` show the first folder; the repeat is `duplicate`; a run created with null shows null |

SQLite-specific, in the adapter crate:

| ID | Scenario | Must hold |
| --- | --- | --- |
| Q01 | Close and reopen the file | Every record is still there |
| Q02 | Two store instances on one file, on different threads | S01 and S03 hold across instances |
| Q03 | A child process aborts at each store failpoint | After reopen the mutation is either wholly absent or wholly present; a retry of the same request converges to one applied mutation |
| Q04 | Open a file with a newer schema version | `schema.too-new` |
| Q05 | Expire one attempt plainly and one with `processUnchecked` (0.2) | Each writes one `attempt.expired` audit record with reason `lease`; only the second notes `process.unchecked` |
| Q06 | Open a version 1 file (0.2) | Upgraded to version 3; its records are kept; process records and audit kinds work |
| Q07 | Open a version 2 file (0.2) | Upgraded to version 3; its runs are kept with a null base folder; a new run records its folder |
