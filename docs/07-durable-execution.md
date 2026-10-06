# 7 Durable execution protocol

Design revision 0.1 — 2026-10-05. Proposed architecture; implementation and production qualification are pending. Numbered research references resolve in [Sources](sources.md).

Persistence is the heart of the engine. Every acknowledged state change must survive the advertised failure model. The store is authoritative for event order and ownership. In-memory queues, notifications, timers, UI state, and traces are accelerators only.

## Invariants

I1. A run revision advances only with the atomic consumption of one pending event and insertion of its complete command set. Administrative migration is also a recorded event. I2. A stable event ID cannot refer to two different payload digests. I3. A deterministic command ID cannot refer to different arguments. I4. An activity result affects the graph at most once. I5. A stale lease cannot commit a result as the current attempt. I6. Terminal graph states do not resume from late messages. I7. Every external dispatch has an already committed intent. I8. Every run can resolve its exact package and bindings. I9. An unknown commit outcome is resolved using the same operation identity. I10. Permission changes can stop future effects even for a pinned run.

## Logical records

| Record | Required fields and constraints |
| --- | --- |
| Run | tenant, run ID, package and binding digests, status, revision, last processed event, next event sequence, snapshot, owner epoch |
| Inbox event | tenant and run, sequence, event ID, kind, payload digest and bytes or reference, accepted time, correlation; unique event ID per run |
| Transition commit | commit ID, expected revision, consumed sequence, decision digest, resulting revision; unique commit ID |
| Outbox command | command ID, run and activation, kind, payload digest, dispatch state; unique command ID |
| Invocation | logical invocation ID, effect key, immutable input and implementation, status, retry budget |
| Attempt | invocation ID, attempt number, owner, fence, lease deadline, start and result evidence |
| Timer | run, timer ID and generation, due time, status; unique generation |
| Artifact | content digest, size, trusted publisher and locator, retention references |
| Audit entry | actor, operation, target revision, approval or policy reference, recorded time |

These are logical records, not a promise that all providers use the same tables. SQLite and PostgreSQL should share one logical SQL schema where possible. IndexedDB and Durable Objects preserve the protocol through provider-specific representation. Revision counts committed transitions; event sequence counts accepted inbox events, which may be pending. Never conflate them.

## One transition

1. Append an external event with an idempotency key in a short transaction. Assign sequence and accepted time atomically with the run's inbox counter. An identical retry returns the original receipt; same ID with different payload is a conflict.
2. Load a coherent run snapshot and its next pending event. A run lease reduces wasted work, but a compare-and-swap on revision is the correctness barrier. Validate artifacts and request limits.
3. Compute the pure transition outside the database transaction. Other events may arrive meanwhile. They cannot reorder the event currently being consumed.
4. Validate the decision, command identities, limits, and expected input digest. Begin a short transaction; check revision, owner epoch when used, lifecycle generation, and next-event sequence.
5. Insert the commit receipt, new snapshot and digest, commands, timer intents, and any continuation; mark the event consumed and advance revision together. Commit all or none.
6. A dispatcher reads committed work. Notifications can wake it sooner; periodic recovery scans ensure a missed notification loses no work.

Concurrent coordinators may both calculate a decision, but only one commits. The loser reloads. It must not execute commands from the discarded calculation. A database transaction is never held while calling the kernel, an activity, a model, or a network service. SQL adapters use one lock order: run first, then subordinate records in stable ID order. Queue discovery must release discovery locks before acquiring a run in reverse order. Retry serialization failures through the same request identity, never by rerunning an external effect inside the transaction.

## Ambiguous database acknowledgement

The caller chooses commit_id before invoking commit_transition. If acknowledgement is lost, query get_commit(commit_id). A matching receipt means success. An absent receipt does not prove rollback: the first request can still be in flight. Resubmit only the identical operation ID and digest. The store serializes receipt lookup, mutation, and receipt insertion in the same transaction; repeated requests return the stored outcome before checking now-stale revision predicates. Conflicting bodies are rejected. Do not allocate a new claim or attempt merely because a receipt lookup was absent. Apply this protocol to start, signal, claim, completion, and all other acknowledged mutations.

## Activity execution and fencing

Claiming a job atomically creates its attempt and increments a monotonically increasing fence. The claim records a lease deadline and owner. V1 uses strict leases: heartbeat and completion require matching owner, attempt, fence, running status, and authority_now less than lease_until, plus the operation deadline when applicable. Expiry requires authority_now at or after the deadline and atomically revokes the fence, changes status, and records the observation. Sample time after acquiring the authoritative lock, not from a stale transaction-start timestamp. Heartbeats cannot revive expired ownership. A replayed claim receipt may be historical: verify current ownership again before dispatch. Before the actual effect, the dispatcher rechecks dispatch generation, effective permissions, binding identity, and approval validity.

finish_attempt and expire_attempt compete through one conditional state transition. A result is accepted only for the live current attempt and valid fence; completion atomically stores its evidence, changes attempt status, and appends the result event. Expiry wins only once and appends an uncertainty or timeout event. Results arriving after expiry are retained separately, without masquerading as the successful current attempt. A reconciler may adopt verified late evidence through a new authenticated reconciliation event.

Leases cannot prevent a paused or partitioned worker from continuing to act on an external system. Fences protect storage commits; downstream fencing is effective only when the external service enforces it. Therefore reassigning a task is safe only under its declared repeat or reconciliation contract. A worker stops initiating new effects when renewal fails or its acknowledgement is unknown, until current ownership is verified. Reconciliation must account for an old attempt that can still act; a transient not-found response is not proof that reissue is safe.

## Remote worker protocol

The source coordinator owns the attempt and fence. Its committed dispatch envelope carries tenant, run, invocation, attempt, fence, dispatch ID, payload and binding digests, deadline, and a scoped credential. A remote runner durably deduplicates that dispatch ID before execution and rejects conflicting bodies. Acceptance acknowledges delivery, not completion. The runner heartbeats against the source authority, records the result in its own durable result outbox, and retries finish_attempt with the same identity. The source accepts only a live current fence; stale results become evidence. A disconnected HTTP call cannot trigger a second unguarded process. Dispatch IDs, attempt IDs, and provider effect keys have separate roles.
