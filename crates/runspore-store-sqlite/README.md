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
  of the nine operations named in spec section 3.

## Schema (version 12, in `PRAGMA user_version`)

An empty file is initialized from `src/schema.sql`, and `meta` records the crate
version that wrote the schema (`writer`). Opening a file an earlier 0.2 `spore` wrote
is refused as `schema.needs-migrate`. `SqliteStore::migrate(path)` (or
`migrate_with_clock`) upgrades it in place, in one transaction, records the writer
again and returns the `Migration` it made. It refuses as `migrate.workers-live` while
a row of `workers` of another version has `until_ms` ahead, or a `running` attempt
owned by an unregistered `worker-…` ID holds a live lease. The steps: version 2 to
3 adds `runs.base_dir`, version 3 to 4 adds the `artifacts` table and
`runs.artifact_digest`, version 4 to 5 adds `meta`, version 5 to 6 adds
`attempts.holder` and `attempts.deadline_ms`, version 6 to 7 adds
`invocations.params`, version 7 to 8 adds `runs.parent_run_id` and
`runs.parent_node_id`, version 8 to 9 adds `attempts.claimed_at_ms`, version 9 to 10
adds `runs.result_channels`, version 10 to 11 adds `runs.cancel_reason`,
`runs.cancel_at_ms` and `runs.cancel_request_id`, version 11 to 12 adds `workers`.
Two more files are refused with kind
`incompatible` before anything else is read or written: version 1, which `spore` 0.1
wrote, as `schema.too-old`, and a newer version as `schema.too-new`; both messages
name the version that wrote the file and the fix (spec/store.md §3).

| Table | Key | Invariant the constraints enforce |
| --- | --- | --- |
| `meta` | `key` | `writer` is the version of the `spore` that initialized or last upgraded the schema |
| `packages` | `digest` | packages are content-addressed |
| `artifacts` | `digest` | workflow artifacts are content-addressed |
| `runs` | `(tenant, run_id)`; unique `(tenant, start_key)` | one run per start key; holds revision, applied and next sequence, last accepted time, fence counter, quarantine, base folder, the digest of the artifact it was started from or null, the run and node it was started from (`parent_run_id`, `parent_node_id`) or nulls, and the result channel picked for each async action as a JSON object (`result_channels`), null for none, and the cancel record (`cancel_reason`, `cancel_at_ms`, `cancel_request_id`), `cancel_at_ms` null unless the run is cancelled; a cancel sets `status` to `cancelled` |
| `events` | `(tenant, run_id, sequence)`; unique `(tenant, run_id, event_id)` | one event per ID per run (I2); one event per sequence; one `activity.result` per attempt, since its ID derives from the attempt ID |
| `transitions` | `(tenant, run_id, revision)`; unique `(tenant, run_id, consumed_sequence)` | one commit per revision and per consumed event |
| `commands` | `(tenant, run_id, command_id)` | a command ID exists once per run (I3) |
| `invocations` | `(tenant, run_id, invocation_id)` | an invocation is scheduled once; `params` holds the node's canonical evaluated params, or null when it has none |
| `attempts` | `(tenant, run_id, attempt_id)`; unique `(tenant, run_id, invocation_id, attempt_number)`; unique `(tenant, run_id, fence)` | one attempt per number; a fence is never reused within a run; `process` holds the canonical process record or null; `holder` the canonical holder handle or null; `deadline_ms` the deadline `lease_until_ms` never passes, or null |
| `receipts` | `(tenant, run_id, request_id)` | one applied mutation per request ID |
| `audit` | `audit_id` | append-only; `kind` is `late-evidence` or `attempt.expired` |
| `workers` | `worker_id` | one row per worker that uses the file: its `spore` version, host, process ID, last heartbeat (`beat_at_ms`) and the time it stays fresh until (`until_ms`); written by `beat_worker` with `synchronous = OFF`, since the record is advisory and its fsync would hold the write lock while workers renew leases; removed by `end_worker` |

A receipt stores the operation name, request digest, and the canonical JSON
(`canonical::encode`) of the returned value. A replay decodes those bytes, so it returns
the same typed value. Two cases answer `duplicate` without writing a receipt: a
`create_run` with identical values under another request ID, and an `append_event`
whose event ID, kind and body digest already exist; and a `cancel_run` of a run that
has already ended.

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

`tests/conformance.rs` runs S01–S34 from `runspore-store-conformance`. S01, S03 and S15
race two instances on one file, which covers Q02. `tests/sqlite.rs` covers Q01 and
Q03–Q19, and a unit test the registered half of `migrate.workers-live`. Q03 and Q11 run `examples/crash.rs` as a child process at a failpoint.
