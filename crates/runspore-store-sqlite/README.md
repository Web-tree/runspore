# runspore-store-sqlite

`SqliteStore` implements `runspore_types::store::Store` (protocol 0.1, `spec/store.md`)
on one SQLite file. `SqliteStore::open(path)` uses `SystemClock`;
`SqliteStore::open_with_clock(path, clock)` injects the authority clock.

## Durability settings

- `journal_mode = WAL`, `synchronous = FULL`, `foreign_keys = ON`, busy timeout 5 s.
- Every mutation is one `BEGIN IMMEDIATE` transaction: receipt lookup, predicates,
  writes and the receipt insert commit together or not at all. "Now" is read from the
  clock after `BEGIN IMMEDIATE` has taken the write lock.
- `SQLITE_BUSY` after the timeout is `unavailable` / `store.busy`.
- Failpoints `store.<op>.before-commit` and `store.<op>.after-commit` wrap the commit
  of the seven operations named in spec section 3.

## Schema (version 1, in `PRAGMA user_version`)

An empty file is initialized from `src/schema.sql`. A file with a newer version is
refused with `incompatible` / `schema.too-new`.

| Table | Key | Invariant the constraints enforce |
| --- | --- | --- |
| `packages` | `digest` | packages are content-addressed |
| `runs` | `(tenant, run_id)`; unique `(tenant, start_key)` | one run per start key; holds revision, applied and next sequence, last accepted time, fence counter, quarantine |
| `events` | `(tenant, run_id, sequence)`; unique `(tenant, run_id, event_id)` | one event per ID per run (I2); one event per sequence; one `activity.result` per attempt, since its ID derives from the attempt ID |
| `transitions` | `(tenant, run_id, revision)`; unique `(tenant, run_id, consumed_sequence)` | one commit per revision and per consumed event |
| `commands` | `(tenant, run_id, command_id)` | a command ID exists once per run (I3) |
| `invocations` | `(tenant, run_id, invocation_id)` | an invocation is scheduled once |
| `attempts` | `(tenant, run_id, attempt_id)`; unique `(tenant, run_id, invocation_id, attempt_number)`; unique `(tenant, run_id, fence)` | one attempt per number; a fence is never reused within a run |
| `receipts` | `(tenant, run_id, request_id)` | one applied mutation per request ID |
| `audit` | `audit_id` | late evidence, append-only |

A receipt stores the operation name, request digest, and the canonical JSON
(`canonical::encode`) of the returned value. A replay decodes those bytes, so it returns
the same typed value. Two cases answer `duplicate` without writing a receipt: a
`create_run` with identical values under another request ID, and an `append_event`
whose event ID, kind and body digest already exist.

Cursors are JSON arrays holding the sort key of the last item returned:
`[tenant, run]` for `scan_ready`, `[run]` for `list_runs`, and
`["lease"|"outbox", ms, id, tenant, run]` for `scan_due`. Clients treat them as opaque.

## Concurrency trade-off

Each `SqliteStore` owns one connection behind a `std::sync::Mutex`. The async methods
run their short synchronous SQLite work inline on the calling task; the mutex is never
held across an `.await`. A call that waits for another process's write lock blocks its
runtime worker thread for up to the 5 s busy timeout. In exchange there are no thread
hand-offs and the code stays simple. Several instances, in one process or in several,
may share a file; SQLite's file locks serialize their writers.

## Limits

- Single host only. Never put the file on a network filesystem: WAL needs shared
  memory and correct POSIX locks. This is documented, not detected.
- One writer at a time per file; throughput is bounded by `fsync` per commit.
- Event bodies are limited to 64 KiB and snapshots to 256 KiB.

## Tests

`tests/conformance.rs` runs S01–S14 from `runspore-store-conformance`. S01 and S03
race two instances on one file, which covers Q02. `tests/sqlite.rs` covers Q01, Q03
and Q04. Q03 runs `examples/crash.rs` as a child process at every failpoint.
