# 15 Efficiency and feasibility gates

Design revision 0.1 — 2026-10-05. Proposed architecture; implementation and production qualification are pending. Numbered research references resolve in [Sources](sources.md).

Optimize around persistence and I/O, which are likely to dominate orchestration. Coalesce bounded pure steps into one commit, cache validated immutable graph data and compiled modules, pass blob references, and use indexed pending-work scans. Never batch across externally visible effects in a way that loses the intent or accepted-result boundary. Maintain fairness with per-run and per-tenant queue budgets.

Do not preload every workflow instance. Waiting runs occupy persistent records, not live stacks or open spans. A kernel instance executes a bounded transition and becomes reusable after a verified reset, or is discarded. Worker concurrency is limited by store throughput, memory, activity capacity, and tenant quotas. Backpressure rejects or delays new work explicitly rather than allowing an unbounded queue.

## First feasibility spikes

| Spike | Evidence required | Failure response |
| --- | --- | --- |
| WIT portability | Same fixture decisions in Wasmtime, Node, Bun, real browsers, deployed Cloudflare | Adjust packaging or lower target support |
| Shared kernel | 10,000 generated event traces have identical canonical decisions | Block release and fix semantics |
| Atomic storage | Inject failures around every write and recover all invariants | Redesign adapter or commit protocol |
| Cross-host resume | Node to Rust and reverse through SQLite and PostgreSQL | Do not advertise interchangeable executors |
| Effect uncertainty | Crash before and after external success; no blind unsafe reissue | Require reconciler or intervention |
| Resource safety | Adversarial IR, payloads, imports, hidden state, infinite loops | Restrict profile further |
| Claude integration | Kill and resume plan to PR flow without duplicate PR or unauthorized merge | Keep integration experimental |
| Provider wakeups | Browser close and Cloudflare alarm exhaustion behave as documented | Add reconciliation or narrow liveness claim |

Benchmark cold and warm startup, package size, idle RSS, per-run stored bytes, ABI copying, transition time, transaction latency, pending-scan latency, and throughput. Publish hardware, filesystem, database settings, versions, graph size, and p50, p95, and p99. Initial engineering goals are sub-10 ms warm pure transitions for 100-node graphs with 64 KiB state and sub-second local recovery discovery under a modest pending queue. These are test targets, not guarantees or measured results. End-to-end effect latency remains adapter-dependent.
