# 10 Time signals ordering and retention

Design revision 0.1 — 2026-10-05. Proposed architecture; implementation and production qualification are pending. Numbered research references resolve in [Sources](sources.md).

Every accepted event receives a per-run sequence. Process that order even if remote timestamps disagree. Timestamps from clients are evidence fields, not scheduling authority. Store-accepted wall time is recorded; the kernel's logical time never decreases. Timers record absolute UTC deadlines once, derived from an accepted event or an explicit sampled-time activity. A timer fires no earlier than its deadline according to the adapter's clock, and may fire late after downtime.

A timer generation distinguishes cancellation and replacement. Firing atomically checks status and generation, marks it fired, and appends a unique event. Duplicate wakeups are harmless. A signal accepted before a waiter exists is buffered subject to declared size and retention; the later waiter consumes the earliest eligible unconsumed item. Correlation, expiry, schema, and authorization are validated at ingress. Overflow is an explicit rejection or dead-letter policy, never silent loss.

For signal-versus-timeout races, first durable event acceptance wins in v1. A late scheduler can therefore accept a signal before it records an overdue timeout. If a business rule requires a strict arrival deadline, declare a deadline predicate using the authoritative accepted timestamp and test it separately. Do not imply real-time ordering from transport timestamps or thread scheduling.

Retention must preserve deduplication at least as long as the maximum replay, redelivery, and external retry windows. Archive payloads independently of compact idempotency tombstones. Preserve a terminal run tombstone when deleting bulky history. Durable continuations start a new linked history segment with explicit carried state and counters; do not drop unmatched signals, outstanding effects, or approval state during compaction.

Blob writes finish before committing a reference. Content-addressed unreferenced uploads are harmless orphans for later collection. After a reference is committed, availability and retention are required. Verify digest and size on fetch. Exclude secrets and sensitive payloads from exported traces; apply encryption and access controls to durable data according to classification.
