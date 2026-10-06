# 9 Replaceable storage and WIT

Design revision 0.1 — 2026-10-05. Proposed architecture; implementation and production qualification are pending. Numbered research references resolve in [Sources](sources.md).

Define a semantic Store interface with capabilities, create_run, append_event, load_transition_input, commit_transition, get_commit, claim_attempt, heartbeat_attempt, finish_attempt, expire_attempt, due_timers, fire_timer, scan_pending, and administrative export operations. Each mutating request has an identity and payload digest. Each operation specifies its consistency and atomicity; passing a type checker is insufficient.

The minimum production store supports atomic conditional updates across all records of one run, unique insertion, consistent readback after an uncertain write, durable acknowledgements, and recovery scans. Multiworker stores additionally support competing claims with fences. Wakeup support is a separate capability. Reject a profile at startup if these requirements are missing; do not emulate atomic batches with a series of puts.

## Adapter choices

SQLite is the default local store. Enable foreign keys, WAL, FULL synchronous mode for the stated power-loss durability profile, bounded busy retries, and short writer transactions. Use BEGIN IMMEDIATE for claims and commits where appropriate. WAL requires same-host shared-memory behavior and has one writer; it is unsuitable for a database file on shared network storage. Pin a patched SQLite build and test the actual driver version. Use the online backup API or a documented consistent snapshot procedure. [10]

PostgreSQL is the shared-worker store. Use row-level locking or optimistic revision updates, unique constraints, and conditional claims; SKIP LOCKED is suitable for queue-like work selection. Notifications are hints, with polling as fallback. Durability settings and replication configuration determine the acknowledged-loss guarantee. Do not advertise zero-loss failover independently of database configuration. [11]

IndexedDB is a browser store with transactions across its run, event, and outbox object stores. Compute Wasm outside the transaction, then perform a short read-check-write transaction. Do not await arbitrary promises while an IndexedDB transaction is active. Web Locks can reduce duplicate local workers, but revision checks remain authoritative. Request persistence where available and offer explicit export; browser eviction, user deletion, and a closed browser limit guarantees. [12]

Cloudflare uses one Durable Object per run initially, with SQLite-backed storage and atomic local updates. Parent-child messages and activity dispatch use outbox plus idempotent delivery, not cross-object transactions. Maintain one alarm for the earliest deadline or reconciliation wakeup; the durable table holds the full timer set. Alarms are at-least-once and automatic retries are finite, so rearm on recoverable failures and use a supervised scheduled reconciler over a durable enumerable registry. [13, 14]

Cloudflare run creation is registry-first. Atomically reserve tenant plus start key to deterministic run ID, complete immutable start request and digest, and initializing status before contacting the run object. The registry outbox and reconciler retry idempotent initialization. The run object atomically writes Started, work records, and its recovery alarm, then returns a receipt. Only then is the registry marked active; lost acknowledgement permits safe query or retry. Reject first initialization without a valid reservation. Retain registry entries until terminal effects are settled and deduplication retention expires. This permits abandoned reservations to be recovered without allowing unindexed live runs. Disable unconfirmed writes in the engine's storage path.

## Where WIT fits

A WIT store provider is possible as a host-side extension. The workflow component never imports it. Initially use native async Rust traits and TypeScript interfaces generated or checked against a common protocol. The companion store.ts describes request and receipt operations; a later WIT provider can map this protocol through explicit async or polling integration after its target toolchain is certified.

A storage component still needs access to a database or host storage API. Moving the code into Wasm does not make its data durable, its transactions atomic, or its platform universally supported. Async WIT and WASI evolution should not delay the import-free reducer. Trusted store adapters have high privilege and require review and the same fault-injection suite as built-ins.

## Live replacement and migration

Replaceable means interchangeable between deployments under a documented migration procedure. It does not mean hot-swapping providers mid-transaction. Drain dispatch, fence the old owner, snapshot all logical records and artifact references, restore, validate digests and pending-work counts, reconcile uncertain external operations, then activate one new owner. Preserve event, invocation, command, approval, and idempotency identities. Do not run old and new stores independently against the same external effects.
