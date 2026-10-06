# 14 Public API and operations

Design revision 0.1 — 2026-10-05. Proposed architecture; implementation and production qualification are pending. Numbered research references resolve in [Sources](sources.md).

Expose register_package, start, get_run, list_runs, signal, cancel, events, resume, and export. Administrative reconcile, migrate, and abandon operations require stronger permissions and an expected revision. Start accepts a client idempotency key; signal accepts a message ID; retries with conflicting payloads return a conflict. Return receipts containing run ID, accepted sequence or revision, and whether the operation was newly applied or deduplicated.

The Rust and TS SDKs provide embedded Engine construction with a Store, ActivityRegistry, PolicyProvider, BlobProvider, TelemetryProvider, and host limits. Engine.tick performs bounded work; Engine.serve runs until cancellation with graceful drain. Neither starts a hidden network listener. Add a CLI worker and a separately enabled control server for users who need them.

CLI operations include validate, compile, inspect-package, run, worker, status, events, signal, cancel, replay, reconcile, export, import, and doctor. Replay is read-only by default and uses recorded events and artifacts with all effects disabled. A repair creates explicit events or a new run; it never silently edits past history. JSON output is versioned for plugin integration.

## Recovery and operations

On startup validate store schema and adapter capabilities, recover expired owners, scan pending events and tasks, reconcile unknown operations, and rebuild wakeup hints. Graceful shutdown stops new claims, finishes or checkpoints bounded work, releases ownership where safe, and preserves uncertain activities for recovery. A process crash follows the same durable records.

Back up the database, blobs, artifact registry, and encryption key references as a consistent recoverable set. A restore to an older database can forget effects that already happened externally. Before re-enabling dispatch, reconcile potentially repeated effects and preserved external idempotency identities. A backup RPO is also a side-effect replay risk, not only lost history.

Database upgrades use versioned migrations and compatibility checks. Prefer expand-then-contract when rolling workers; reject incompatible simultaneous readers and writers. Pin old package and kernel versions while runs depend on them. Deleting or revoking a vulnerable artifact blocks affected runs and invokes an explicit remediation path; it must not silently run new code against old state.

## Run migration

By default, old runs finish on their original package. Migration requires a quiescent boundary, no unresolved effects, compatible pending signals and timers, an explicit old-to-new state transformation, validated output, and a committed migration event. First release may forbid migration while any invocation is outstanding. Test the migration against archived fixtures and offer dry-run output. Preserve original history and include both artifact digests in the audit record.

Cross-host resume works when both hosts support the exact package, store protocol, effects, and limits. Quiesce and fence the previous owner before moving a SQLite file or importing an export. Matching reducer semantics alone does not make a browser's local run safe to execute simultaneously on a server.
