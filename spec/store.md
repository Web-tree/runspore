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
audit record of an expiry, the run's base folder, the artifact table with
`get_artifact` and the run's artifact digest, the schema guard that names the version
that wrote a database (§3), an attempt's holder handle and deadline, a renewal capped
at the deadline, the reason of an expiry, the full reference of an invocation's latest
attempt, an invocation's node `params` in every claim, the run's parent, the run's
`updatedAtMs`, `hold_attempt`, the result channel picked for each async action,
`cancel_run` and the run's cancel record, an attempt's needs-person mark with
`set_needs_person`, and scenarios S15 to S35; everything else is as in 0.1.

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

**0.2:** the request may carry the workflow artifact the run is started from
(artifact.md), as its digest and bytes. Predicate: the digest equals
`digest::hash("artifact", [bytes])` (`conflict` / `artifact.digest-mismatch`). Writes,
beside the package: the artifact, content-addressed, if absent, and the artifact
digest on the run. A run started without one has none. The artifact digest is
provenance, not identity: it is not compared when the run already exists, so starting
again with the same start key and package from a rebuilt artifact is a `duplicate`
and keeps the run's first artifact.

Run IDs are chosen by the caller and random (host §2), so `run.start-key-mismatch` is
how a store answers the second of two starts that raced with one start key.

**0.2:** the request may carry the run's parent, `{runId, nodeId}`: the run and node of
the step it was started from (host §2, `parent`), or null. It is stored with the run and
shown by `get_run` and `list_runs`. Like `baseDir`, it is not part of the run's
identity: a repeat with another parent is a `duplicate`, and the run keeps the parent
it was created with. The store does not check that the parent run exists; it may be in
another database.

**0.2:** the request carries `resultChannels`: an object from the ID of each async
action of the package to the result channel `start` picked for it (host §2, The
handshake), `{}` when the package has none. It is stored with the run and shown by
`get_run` and `list_runs`. Like `baseDir`, it is not part of the run's identity: a
repeat with other channels is a `duplicate`, and the run keeps the channels it was
created with.

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

A coherent read. Returns `None` if the run does not exist, is quarantined, (0.2) is
cancelled, or has no pending event (`appliedSequence + 1 == nextSequence`). Otherwise
the run's revision,
applied sequence, package digest, snapshot, and the event at `appliedSequence + 1`.

### get_package

The canonical workflow bytes for a digest, or `None`.

### get_artifact (0.2)

The bytes of the workflow artifact stored under a digest, or `None`. A read: no
request ID and no receipt.

### commit_turn

Predicates, in order: the run exists (`not-found` / `run.not-found`); (0.2) it is not
cancelled (`stale` / `run.cancelled`); it is not quarantined (`stale` /
`run.quarantined`); `expectedRevision` equals the revision
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
    `notBeforeMs`, input, input digest, effect key, node and action IDs, and (0.2) the
    node's `params` when the payload carries them, else null. An existing invocation
    ID is `conflict` / `invocation.exists`.
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

Predicates: the invocation exists (`not-found` / `invocation.not-found`); (0.2) the run
is not cancelled (`stale` / `run.cancelled`); the run is not quarantined (`stale` /
`run.quarantined`); the invocation is `pending` (`conflict`
/ `invocation.not-claimable`); now ≥ `notBeforeMs` (`conflict` / `invocation.not-due`).

Writes: the run's fence counter + 1; an attempt with ID `ids::attempt(invocationId,
attemptNumber)`, the claiming worker as owner, that fence, `leaseUntilMs = now +
leaseDurationMs`, status `running`; the invocation becomes `running`. Returns the
`Claim`. A fence is never reused within a run. **0.2:** the `Claim` carries the
invocation's `params`, canonical JSON as scheduled, or null for a node without them;
every attempt's claim returns the same.

**0.2:** the request also carries `holder` and `deadlineDurationMs`, for an attempt
held outside the worker (host §2, Steps outside the worker):

- `holder` is the **holder handle**: canonical JSON of at most 4 KiB that the store
  keeps and never reads, or null. A larger one is `quota` / `holder.too-large` and
  writes nothing.
- `deadlineDurationMs`, when given, sets the attempt's `deadlineMs = now +
  deadlineDurationMs`, a bound no renewal passes, and `leaseUntilMs` becomes the
  nearer of `now + leaseDurationMs` and `deadlineMs`. Null leaves `deadlineMs` null:
  the attempt is bounded by its lease alone, as in 0.1.

**0.2:** the attempt records when it was claimed, `claimedAtMs`, by the store's
clock; a replayed claim keeps the first time.

The `Claim` returns both. Who the owner is stays the caller's choice: a host gives an
attempt held outside the worker a fresh owner per claim (host §2, Steps outside the worker), so its token names
that claim alone.

### heartbeat

Predicate: the attempt is `running`, its ID, owner, and fence match, and now <
`leaseUntilMs` (`stale` / `lease.lost` for all of these). Writes `leaseUntilMs = now +
extendMs`. A replayed receipt returns the historical deadline and extends nothing.

**0.2:** an attempt with a `deadlineMs` is extended to the nearer of `now + extendMs`
and `deadlineMs`, never past it. `heartbeat` is how a holder renews its lease.

### hold_attempt (0.2)

`hold_attempt(attempt, holder, leaseDurationMs, deadlineDurationMs)` hands a running
attempt to a holder outside the worker: an async command whose start command has
printed its holder handle (host §3, Async commands).

Predicates: as `heartbeat`, the attempt is `running`, its ID, owner, and fence match,
and now < `leaseUntilMs` (`stale` / `lease.lost` for all of these). A holder handle
over 4 KiB is `quota` / `holder.too-large` and writes nothing.

Writes: the attempt's `holder` (canonical JSON, kept and never read), `deadlineMs =
now + deadlineDurationMs`, `leaseUntilMs` the nearer of `now + leaseDurationMs` and
`deadlineMs`, and a null process record. Owner, fence and status are unchanged, so the
attempt's reference still names it; from then on `heartbeat` renews it up to its
deadline, and `finish_attempt` and `expire_attempt` end it as any attempt. Returns
`{holder, leaseUntilMs, deadlineMs}`. A replayed receipt returns the historical value
and writes nothing.

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

### set_needs_person (0.2)

`set_needs_person(attempt, reason)` marks a running attempt as **needing a person**: its
holder waits on someone, such as an agent at a permission prompt, while the attempt
still runs. `reason` is a string of at most 1 KiB, or null to clear the mark.

Predicate: as `heartbeat`, the attempt is `running`, its ID, owner, and fence match,
and now < `leaseUntilMs` (`stale` / `lease.lost` for all of these). A reason over
1 KiB is `quota` / `person.reason-too-large` and writes nothing.

Writes the attempt's mark `{reason, sinceMs}` and nothing else: `sinceMs` is now, or
the time the mark already had when it is set again, so a new reason keeps when the
holder started waiting. A null reason clears the mark; clearing an attempt with none
is applied and changes nothing. The lease is not extended, and the run's
`updatedAtMs` is left. Returns `{mark}`, the mark now held or null. A replayed receipt
returns the historical value and writes nothing.

`finish_attempt`, `expire_attempt` and `cancel_run` clear the mark of the attempt they
end, in the same transaction: only a running attempt has one.

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

**0.2:** the request also carries a `reason`: `lease`, `probe-dead`, `abandoned` or
`cancelled`. Only `lease` requires now ≥ `leaseUntilMs`; the others end a running
attempt at once, before its lease is due, and fence its holder out: a later
`heartbeat` or `finish_attempt` with its reference is `lease.lost`. The appended event
is the same `expired` result whatever the reason. The audit record names the reason,
`{"reason": "<reason>"}`, with the notes above.

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
and are idempotent on an already matching state. **0.2:** a cancelled run is not
quarantined: `quarantine_run` on it is `stale` / `run.cancelled`, checked before the
revision.

### cancel_run (0.2)

`cancel_run(key, reason)` ends an unfinished run on purpose, at once and in one
transaction: Temporal's *terminate*, not its *cancel*. The workflow gets no say and the
kernel is not called; the run keeps its events, invocations and snapshot. `reason` is
a string a person gave, or null. The request ID is the caller's, so a repeat is a
`duplicate`.

Predicate: the run exists (`not-found` / `run.not-found`).

A run that has already ended, `completed`, `failed` or `cancelled`, is left as it is:
the operation writes nothing, no receipt included (as §1 rule 6), and returns the
run's status and, for a cancelled run, its cancel record, disposition `duplicate`.

Otherwise it writes, atomically:

- the run's **cancel record**, `{reason, atMs, requestId}`, with `atMs` now. It lives
  beside the kernel state, as a quarantine does: the snapshot keeps the state it had,
  and the run's status becomes `cancelled`. A cancel is not a kernel event: no event
  is appended for it, and pending events stay unconsumed, as evidence;
- a quarantine, if the run has one, cleared;
- each `running` attempt of the run expired as `expire_attempt` expires it with reason
  `cancelled`: the attempt `expired`, its invocation `settled`, the `expired` result
  appended, and the audit record `{"reason": "cancelled"}`. That holds for an attempt a
  worker runs and for one held outside the worker, whose holder is fenced out;
- the run's `updatedAtMs`, now.

It returns `{status, cancel, expired}`: the run's status, `cancelled` or the one it had
ended with; its cancel record, or null for a run that ended otherwise; and, for an
applied cancel, each attempt it expired, as `{attempt, holder}`: its full reference and
its holder handle (canonical JSON or null), so the caller can run the action's stop
(host §2), and `[]` otherwise. A replay returns the same value.

After it, the run is invisible to `load_turn`, `scan_ready` and `scan_due`;
`commit_turn`, `claim_attempt` and `quarantine_run` are `stale` / `run.cancelled`; the
expired attempts' `heartbeat`, `hold_attempt`, `set_process` and `finish_attempt` are
`stale` / `lease.lost`, and `record_late_evidence` keeps what they say. Events are
still accepted, as for a quarantined run, and never consumed. `release_run` finds no
quarantine. Read views show status `cancelled` and the cancel record.

### Scans and views

- `scan_ready`: keys of runs with a pending event, not quarantined, ascending by
  `(tenant, run)`. The cursor is opaque; `next` is `None` on the last page.
- `scan_due`: first `Lease` items for running attempts with `leaseUntilMs ≤ now`,
  ascending by deadline then attempt ID; then `Outbox` items for `pending` invocations
  with `notBeforeMs ≤ now` in runs that are not quarantined, ascending by
  `notBeforeMs` then invocation ID. **0.2:** a `Lease` item carries the attempt's
  process record, or null, so the sweeping host needs no second read. **0.2:**
  `leaseUntilMs` never passes `deadlineMs`, so every running attempt past its deadline
  is among the `Lease` items, and the ordinary sweep expires it.
- `get_run`, `list_runs` (ascending by run ID within a tenant), `list_events`
  (ascending by sequence, with the consuming revision and that commit's diagnostics),
  `list_invocations` (ascending by invocation ID). **0.2:** an invocation shows the
  process record of its latest attempt, or null. **0.2:** a run shows its artifact
  digest, or null. **0.2:** an invocation also shows its latest attempt's full
  reference (`attemptId`, `owner`, `fence`), its holder handle and its `deadlineMs`, or
  nulls before its first claim, so a host can act on an attempt it did not claim. The
  reference is a credential: a host never prints the fence (host §2, Steps outside the worker).
  **0.2:** a run shows its parent, or null. **0.2:** an invocation shows its latest
  attempt's `claimedAtMs`, or null before its first claim. **0.2:** a run shows
  `updatedAtMs`, the store time of its latest change: its creation, an appended event
  (a signal, a resolution, a result or an expiry), a commit, a claim, a quarantine and
  a release each set it to now; a renewal, `set_process`, `hold_attempt` and (0.2)
  `set_needs_person` leave it.
  **0.2:** an invocation shows its input and its `params` as scheduled, the values
  every claim returns, so a host can probe or watch an attempt it did not claim (host
  §3, Async commands). **0.2:** a run shows its `resultChannels`, `{}` when it has
  none. **0.2:** an invocation shows its latest attempt's needs-person mark
  `{reason, sinceMs}`, or null. **0.2:** a cancelled run shows status `cancelled` and its cancel record
  `{reason, atMs, requestId}`; any other run shows null. A cancel sets `updatedAtMs`.
- **0.2:** `scan_ready` and both kinds of `scan_due` item leave cancelled runs out.

## 3. SQLite adapter

- One database file. Several processes on one host may open it at once.
- **Supported setup (0.2).** One machine, one OS user, one database per checkout.
  Several machines on one database, two repositories sharing one database on
  purpose (`--db` or `RUNSPORE_DB`), and processes of another OS user are not
  supported, and nothing detects them, as for a network filesystem. They share one
  tenant, so one start-key namespace, and a process record they leave cannot be
  checked: its attempt expires by lease with `process.unchecked` (host §3).
- **No authorization (0.2).** Whoever can write the database file may claim any
  label, signal, resolve, abandon or cancel. File permissions are the boundary, as
  for git or SQLite itself.
- `journal_mode = WAL`, `synchronous = FULL`, `foreign_keys = ON`, a busy timeout of at
  least 5 seconds. Never on a network filesystem (documented, not detected).
- Every mutation runs in `BEGIN IMMEDIATE`. `SQLITE_BUSY` after the timeout maps to
  `unavailable` / `store.busy`.
- The schema carries a version, and (**0.2**) the database records which `spore`
  wrote its schema: the version of the binary that initialized or last upgraded it.
  That version, and the one each message below names for this `spore`, is the one
  `spore --version` prints, without the date: on a build that is not a release, the
  pre-release named by its commit, such as `0.4.0-dev+a65d8d1` (release.md §7), as
  rustc's error reports carry its commit-qualified version. An empty file is initialized and records the writer. Opening a file is refused with
  kind `incompatible` when:
  - its schema is newer than this adapter's: `schema.too-new`. The message names the
    version that wrote it, as the file records it, and the fix: finish its runs with
    that `spore`, or move the file aside;
  - its schema is version 1, the schema `spore` 0.1 wrote: `schema.too-old`. The
    message says the file belongs to `spore` 0.1 and names the same fix, with the 0.1
    binary. 0.2 replaces 0.1 outright: nothing upgrades a 0.1 database;
  - (**0.2**) its schema is older, written by an earlier 0.2 `spore`, and the caller
    did not ask to migrate: `schema.needs-migrate`. The message names the version and
    the writer the file records, and the fix: `spore migrate` once no older worker
    uses the file, or keep using that `spore`.

  Opening a file never upgrades it, as a Postgres major version never upgrades a
  data directory by opening it, and Temporal's schema moves only through
  `temporal-sql-tool update-schema`.

  **Migrate (0.2).** Only an open that asks to migrate (`SqliteStore::migrate`) upgrades a
  version written by an earlier 0.2 `spore`, in place in one transaction, and the
  writer is recorded again. Under the write lock, before any upgrade step, it is
  refused with kind `unavailable` / `migrate.workers-live` while a worker of another
  `spore` uses the file, and the file is left as it was. The message names each such
  worker and says to stop it, or let it finish, and migrate again. A worker of
  another `spore` is:
  - a registered worker whose version is not this `spore`'s and whose heartbeat is
    fresh: `until_ms` in the `workers` table is still ahead. A process that works on
    runs (`spore run`, `worker`, `continue`) registers with its worker ID, version,
    host and process ID, renews it every 10 seconds, each time fresh for 30 seconds,
    and removes it when it stops;
  - an attempt still `running` under a live lease whose owner is a worker ID
    (`worker-…`, host.md §3) that is not registered: a worker of a `spore` older than
    the `workers` table. The message names the worker and the step and run it runs.
    An actor's claim of an external step does not count.

  The steps: version 2 to 3 adds the run's base folder, null
  for runs created before; version 3 to 4 adds the artifact table and the run's
  artifact digest; version 4 to 5 adds the writer; version 5 to 6 adds an attempt's
  holder handle and deadline, null for attempts claimed before; version 6 to 7 adds an
  invocation's `params`, null for invocations scheduled before; version 7 to 8 adds the
  run's parent, null for runs created before; version 8 to 9 adds when an attempt was
  claimed, null for attempts claimed before; version 9 to 10 adds the run's result
  channels, `{}` for runs created before; version 10 to 11 adds the run's cancel
  record, null for runs created before; version 11 to 12 adds the `workers` table;
  version 12 to 13 adds an attempt's needs-person mark, none for attempts claimed
  before.
  The refusals happen before any other read or write, so a refused file is left
  exactly as it was.

  This holds for the whole 0.2 line: every schema version a 0.2 `spore` ever wrote is
  upgraded in place by `migrate`, step by step, and only a 0.1 file and a newer one
  are refused. Each schema change ships with its upgrade step from the version
  before, and its scenario in §4. All steps and the new version run in the one
  transaction that opens the file, so an upgrade that stops part way leaves the file
  at its old version, still readable by the `spore` that wrote it. Once upgraded,
  that older `spore` refuses the file as `schema.too-new`. The runs in the file carry
  on as they were: each keeps its package, artifact and snapshot, and the kernel
  continues them (kernel, Versions).
- `capabilities()`: protocol `0.2`, `multiworkerClaims = true` (same host),
  `persistentWakeup = false`, `transactionScope = "database"`,
  `failureModel = "crash-stop"`, `maxRecordBytes = 262144`.
- The schema is internal to the adapter and documented in its README. Unique
  constraints, not application checks alone, back I2, I3, and fence uniqueness.

### Failpoints

Around the transaction commit of each mutating operation:
`store.<op>.before-commit` and `store.<op>.after-commit`, with `<op>` one of
`create-run`, `append-event`, `commit-turn`, `claim-attempt`, `heartbeat`,
`set-process` (0.2), `set-needs-person` (0.2), `finish-attempt`, `expire-attempt`, `cancel-run` (0.2). **0.2:** `store.upgrade.after-step`
after each upgrade step of a migrate, before the transaction commits. Use
`runspore_types::failpoint::hit`.

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
| S18 | `create_run` with an artifact; again with the same start key and another artifact; with an artifact whose digest does not match its bytes (0.2) | The artifact is returned by `get_artifact` and its digest by `get_run`; the second start is `duplicate` and the run keeps the first artifact digest; the third is `artifact.digest-mismatch` and writes nothing; a run started without one shows none |
| S19 | `claim_attempt` with a holder handle, a lease and a deadline; `heartbeat` before and at the deadline; a holder over 4 KiB (0.2) | The claim returns the holder and `deadlineMs`, and `leaseUntilMs` is the nearer bound; `list_invocations` shows the holder, the deadline and the full reference; a renewal stops at the deadline; the large holder is `holder.too-large` and claims nothing |
| S20 | `expire_attempt` with each reason, before the lease is due (0.2) | `abandoned`, `probe-dead` and `cancelled` are applied at once and append one `expired` result; `lease` is `lease.not-due`; the old holder's `heartbeat` and `finish_attempt` are then `lease.lost`, and its result can be kept as late evidence |
| S21 | An attempt claimed with a deadline shorter than its lease; the clock passes the deadline (0.2) | The attempt is a `Lease` item of `scan_due` from its deadline on, and `expire_attempt` with reason `lease` is applied |
| S22 | One commit schedules an invocation with `params` and one without; the first is retried (0.2) | Each claim of the first, the retry's included, returns its `params`; the claim of the second returns null |
| S23 | `create_run` with a parent, repeated with another, and without one (0.2) | `get_run` and `list_runs` show the first parent; the repeat is `duplicate`; a run created without one shows null |
| S24 | An invocation before its claim, after it, after a replay of the claim, and after a retry's claim at a later time (0.2) | `list_invocations` shows `claimedAtMs` null, then the claim's time, the same time after the replay, then the retry's time |
| S25 | A run created, committed, claimed, renewed, given a result, committed again, claimed and abandoned, given a signal, quarantined and released, each at a later time (0.2) | `get_run` shows `updatedAtMs` at the time of each change; the renewal leaves it |
| S26 | `hold_attempt` on a running attempt with a process record; again with its request ID; with another owner, after its lease and after it finished; with a holder over 4 KiB (0.2) | The attempt keeps its reference and shows the holder, the deadline, the nearer lease and no process record in `list_invocations`, with the invocation's input and params; a renewal stops at the deadline and `finish_attempt` with the same reference is applied; the replay is `duplicate`; the others are `lease.lost`, and the large holder `holder.too-large`, writing nothing |
| S27 | `create_run` with result channels, repeated with others, and without any (0.2) | `get_run` and `list_runs` show the first channels; the repeat is `duplicate`; a run created without any shows `{}` |
| S28 | `cancel_run` on a run waiting for a signal, with a reason; then with its request ID again, and with another (0.2) | `applied`; `get_run` shows status `cancelled`, the snapshot it had and the cancel record; the run is gone from `load_turn` and `scan_ready`; a signal is still accepted and stays pending; `commit_turn` and `quarantine_run` are `run.cancelled`; the repeat is `duplicate` with the same value, the other request `duplicate` and writes no receipt |
| S29 | `cancel_run` on a run whose step is stuck (status `needs-intervention`) (0.2) | `applied`, status `cancelled`; a resolution is accepted and never consumed |
| S30 | `cancel_run` on a quarantined run (0.2) | `applied`; the quarantine is cleared and status is `cancelled`; `release_run` finds nothing to release |
| S31 | `cancel_run` while a worker's attempt runs, with a process record (0.2) | The attempt is expired, its invocation `settled`, one `expired` result appended, with an `attempt.expired` audit record of reason `cancelled`; the value lists the attempt with a null holder; the worker's `heartbeat`, `set_process` and `finish_attempt` are `lease.lost`, its result is kept as late evidence; nothing of the run is in `scan_due`, and a pending invocation of it is not claimable (`run.cancelled`) |
| S32 | `cancel_run` while an attempt is held with a holder handle and a deadline (0.2) | The value lists the attempt's reference and its holder; the holder's `heartbeat`, `hold_attempt` and `finish_attempt` are `lease.lost`; the attempt is no `Lease` item of `scan_due` once its deadline passes |
| S33 | Two `cancel_run` calls with two request IDs, from two instances at once (0.2) | Both succeed; exactly one is `applied` and writes a receipt; the run has one cancel record and every running attempt one `expired` result |
| S34 | `cancel_run` on a completed run, a failed run, and a run that does not exist (0.2) | The ended runs are `duplicate`, keep their status and get no cancel record and no receipt; the missing run is `run.not-found` |
| S35 | `set_needs_person` on a running attempt; again with another reason; cleared twice; replayed with the first request ID; that ID with another reason; a reason over 1 KiB; with another owner or fence; once the attempt has finished (0.2) | `list_invocations` shows the mark with its time, and the lease and `updatedAtMs` are unchanged; the second keeps the first time; both clears are applied and show none; the replay is `duplicate` with the historical mark and writes nothing; the other reason is `request.digest-mismatch`; the long reason `person.reason-too-large`; the strangers and every call after the finish are `lease.lost`, and the finished attempt shows no mark |

SQLite-specific, in the adapter crate:

| ID | Scenario | Must hold |
| --- | --- | --- |
| Q01 | Close and reopen the file | Every record is still there |
| Q02 | Two store instances on one file, on different threads | S01 and S03 hold across instances |
| Q03 | A child process aborts at each store failpoint | After reopen the mutation is either wholly absent or wholly present; a retry of the same request converges to one applied mutation |
| Q04 | Open a file with a newer schema version | `schema.too-new`; the message names the writer the file records and the fix |
| Q05 | Expire one attempt plainly and one with `processUnchecked` (0.2) | Each writes one `attempt.expired` audit record with reason `lease`; only the second notes `process.unchecked` |
| Q06 | Open a version 1 file, written by `spore` 0.1 (0.2) | `schema.too-old`; the message names 0.1 and the fix; the file is unchanged |
| Q07 | Migrate a version 2 file (0.2) | Upgraded to the current version; its runs are kept with a null base folder; a new run records its folder |
| Q08 | Migrate a version 3 file (0.2) | Upgraded to the current version; its runs are kept and show no artifact; a new run stores its artifact |
| Q09 | Migrate a version 4 file (0.2) | Upgraded to the current version; its runs are kept; the file records this `spore` as its writer |
| Q10 | Migrate a version 5 file (0.2) | Upgraded to the current version; its attempts are kept with no holder and no deadline; a new claim records both |
| Q11 | A child process aborts at `store.upgrade.after-step` while migrating a version 4 file (0.2) | The file keeps version 4 and none of the upgrade; its runs are kept; the next migrate upgrades it to the current version |
| Q12 | Migrate a version 6 file (0.2) | Upgraded to the current version; its invocations are kept and claim with null `params`; a new schedule records its `params` |
| Q13 | Migrate a version 7 file (0.2) | Upgraded to the current version; its runs are kept with no parent; a new run records its parent |
| Q14 | Migrate a version 8 file (0.2) | Upgraded to the current version; its attempts are kept with no claim time; a new claim records its time |
| Q15 | Migrate a version 9 file (0.2) | Upgraded to the current version; its runs are kept with result channels `{}`; a new run records its own |
| Q16 | Migrate a version 10 file (0.2) | Upgraded to the current version; its runs are kept with no cancel record; one of them can then be cancelled |
| Q17 | Migrate a version 11 file, then migrate it again (0.2) | Upgraded to the current version with the `workers` table; the migration reports version 11 and its writer; its runs are kept; the second migrate changes nothing |
| Q18 | Open a version 11 file without migrating (0.2) | `schema.needs-migrate`; the message names the version, the writer, this `spore` and `spore migrate`; the file is unchanged |
| Q19 | Migrate a version 11 file while one attempt is held under a live lease by `worker-old` and one by an actor; again once the lease has passed (0.2) | `migrate.workers-live` naming `worker-old`, its step and run, not the actor; the file is unchanged; then migrated |
| Q20 | Migrate a version 12 file with a claimed attempt (0.2) | Upgraded to the current version; the migration reports version 12 and its writer; the attempt shows no mark, and can then be marked as needing a person |
