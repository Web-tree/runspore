# 16 Conformance and production qualification

Design revision 0.1 — 2026-10-05. Proposed architecture; implementation and production qualification are pending. Numbered research references resolve in [Sources](sources.md).

The conformance suite is a first-class package, used by every store, host, and community extension. Separate kernel semantic tests from persistence fault tests and platform liveness tests. A provider is certified only for the capabilities tested. Run real services in CI where emulators cannot establish the guarantee.

| Test family | Required cases |
| --- | --- |
| Determinism | Different map insertion order, Unicode, integer boundaries, dirty instances, arm64 and x64, engine versions |
| Graph semantics | Loop activation IDs, duplicate outcomes, fork joins, bounded rework, cancellation, terminal late events |
| Inbox | Same ID same payload, same ID different payload, signal before wait, ordered concurrent append |
| Commit | Crash before commit, lost success acknowledgement, competing revisions, commit digest mismatch |
| Attempts | Expired worker, stale completion, heartbeat race, completion versus expiry, duplicate callback |
| Effects | Idempotent retry, expired dedupe window, reconcilable PR, unknown shell effect, late evidence |
| Timers | Clock jumps, downtime, duplicate wakeup, cancellation generation, signal-timeout acceptance ordering |
| Storage | Full disk, read-only database, corruption detection, network partition, failed failover, restore |
| Security | Import violation, SSRF redirects and DNS changes, argv injection, symlink escape, approval replay |
| Extensions | Malformed result, over-budget telemetry, incompatible ABI, unavailable driver, revoked capability |
| Lifecycle | Schema upgrade, artifact revocation, state migration, history compaction, blob retention |

Model-check a bounded version of the commit and attempt protocol before declaring PostgreSQL multiworker GA. A small TLA+ or equivalent model should explore competing coordinators, lost acknowledgements, duplicate deliveries, expired fences, and cancellation. The model proves properties of its assumptions, not the correctness of drivers or external services; retain fault injection and integration tests.

Require a reproducible release build, cross-platform fixtures, 72-hour fault-injection soak, tested backup and restore, security review, documented unknown-effect handling, and an operator runbook. Establish an incident process and release rollback policy. Targeted production pilots should start with low-impact automation; promote privileged merge or deployment only after the broker and reconciliation gates pass.
