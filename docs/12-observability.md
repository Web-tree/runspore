# 12 OpenTelemetry and operational visibility

Design revision 0.1 — 2026-10-05. Proposed architecture; implementation and production qualification are pending. Numbered research references resolve in [Sources](sources.md).

Instrument public API calls, compile and validate, transition computation, storage operations, dispatch, each physical attempt, remote activity calls, signal ingestion, timers, migration, reconciliation, and administrative operations. Export through host SDKs and OTLP. Keep a durable audit log for approvals, commands, and outcomes; sampled traces are not the audit record.

Use short spans rather than a span kept open through days of waiting. Persist allowed trace context with a command, then create attempt spans linked to the scheduling transition and previous attempts. A workflow correlation ID spans trace segments; run and invocation IDs belong in traces and logs, not unbounded metric labels. Replayed transitions are marked replay and do not emit duplicate activity spans or success metrics. [17]

Expose counters for accepted events, committed transitions, attempts, deduplications, stale-fence rejections, unknown effects, retries, and dropped telemetry. Histograms cover queue age, transition duration, commit latency, activity duration, wakeup lateness, and recovery time. Gauges cover pending runs, oldest due work, dead letters, and storage pressure. Metric dimensions are bounded adapter, workflow type, operation, and result code; never user text, raw URLs, or arbitrary node IDs from tenant packages.

## WIT community extension API

Publish a versioned observability interface for activities and host-side plugins: begin span, add bounded attributes or events, end span, emit approved metric, and log structured record. The host supplies opaque context handles and enforces quotas and redaction. Do not let a plugin invent trusted tenant or approval attributes or configure arbitrary exporters. The reducer returns deterministic diagnostic records; the host attaches actual timing after a commit or failed computation.

wasi-otel is an evolving proposal rather than a stable baseline to depend on. Keep a small project-owned interface and an optional version-pinned adapter, with a migration plan when the standard stabilizes. Telemetry failures drop or buffer within a cap and increment a counter; they must not choose graph branches or prevent durable progress. Security audit persistence can be mandatory even when external telemetry is disabled. [19]
